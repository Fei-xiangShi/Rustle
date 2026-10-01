use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{self, SyncSender};
use std::thread::{self, JoinHandle};

use pipewire::{
    channel,
    context::ContextRc,
    main_loop::MainLoopRc,
    metadata::{Metadata, MetadataListener},
    node::{Node, NodeChangeMask, NodeListener},
    registry::RegistryRc,
    types::ObjectType,
};

use crate::player::{PlaybackError, PlaybackResult};

use super::{DeviceChangedCallback, DeviceTopologyChange, PlatformBackend};

enum WatchCommand {
    Stop,
}

struct SinkMonitor {
    _listener: NodeListener,
    _node: Node,
}

struct MetadataMonitor {
    _listener: MetadataListener,
    _metadata: Metadata,
}

struct WatchState {
    initialized: bool,
    sink_ids: HashSet<u32>,
    sinks: HashMap<u32, SinkMonitor>,
    metadata: HashMap<u32, MetadataMonitor>,
}

impl WatchState {
    fn new() -> Self {
        Self {
            initialized: false,
            sink_ids: HashSet::new(),
            sinks: HashMap::new(),
            metadata: HashMap::new(),
        }
    }
}

fn is_audio_sink(
    global: &pipewire::registry::GlobalObject<&pipewire::spa::utils::dict::DictRef>,
) -> bool {
    global.type_ == ObjectType::Node
        && global
            .props
            .as_ref()
            .and_then(|props| props.as_ref().get("media.class"))
            .is_some_and(|class| class == "Audio/Sink" || class == "Audio/Duplex")
}

fn is_default_sink_property(key: Option<&str>) -> bool {
    matches!(
        key,
        Some("default.audio.sink") | Some("default.configured.audio.sink")
    )
}

fn is_relevant_sink_change(change_mask: NodeChangeMask) -> bool {
    change_mask.intersects(
        NodeChangeMask::INPUT_PORTS
            | NodeChangeMask::OUTPUT_PORTS
            | NodeChangeMask::PROPS
            | NodeChangeMask::PARAMS,
    )
}

fn notify_if_ready(
    state: &Rc<RefCell<WatchState>>,
    changed: bool,
    default_changed: bool,
    notifications: &channel::Sender<bool>,
) {
    if changed && state.borrow().initialized {
        let _ = notifications.send(default_changed);
    }
}

fn bind_metadata_listener(
    registry: &RegistryRc,
    global: &pipewire::registry::GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    state: &Rc<RefCell<WatchState>>,
    notifications: &channel::Sender<bool>,
) -> Option<MetadataMonitor> {
    let Ok(metadata) = registry.bind::<Metadata, _>(global) else {
        return None;
    };
    let state_for_event = Rc::clone(state);
    let notifications = notifications.clone();
    let listener = metadata
        .add_listener_local()
        .property(move |_subject, key, _type, _value| {
            notify_if_ready(
                &state_for_event,
                is_default_sink_property(key),
                true,
                &notifications,
            );
            0
        })
        .register();
    Some(MetadataMonitor {
        _listener: listener,
        _metadata: metadata,
    })
}

fn bind_sink_listener(
    registry: &RegistryRc,
    global: &pipewire::registry::GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    state: &Rc<RefCell<WatchState>>,
    notifications: &channel::Sender<bool>,
) -> Option<SinkMonitor> {
    let Ok(node) = registry.bind::<Node, _>(global) else {
        return None;
    };
    let received_initial_info = Rc::new(Cell::new(false));
    let state_for_event = Rc::clone(state);
    let notifications = notifications.clone();
    let listener = node
        .add_listener_local()
        .info(move |info| {
            if !received_initial_info.replace(true) {
                return;
            }
            notify_if_ready(
                &state_for_event,
                is_relevant_sink_change(info.change_mask()),
                false,
                &notifications,
            );
        })
        .register();
    Some(SinkMonitor {
        _listener: listener,
        _node: node,
    })
}

pub(super) struct Backend {
    commands: channel::Sender<WatchCommand>,
    thread: Option<JoinHandle<()>>,
}

impl PlatformBackend for Backend {
    const SUPPORTED: bool = true;

    fn new(callback: DeviceChangedCallback) -> PlaybackResult<Self> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (commands, command_rx) = channel::channel();

        let thread = thread::Builder::new()
            .name("pipewire-device-watcher".to_string())
            .spawn(move || {
                pipewire::init();
                if let Err(error) = run_watcher(callback, command_rx, ready_tx) {
                    tracing::warn!(error = %error, "PipeWire audio device watcher exited");
                }
            })
            .map_err(|error| {
                PlaybackError::ControlUnavailable(format!(
                    "failed to start PipeWire device watcher: {error}"
                ))
            })?;

        match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                commands,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(PlaybackError::DeviceUnavailable(error))
            }
            Err(error) => {
                let _ = commands.send(WatchCommand::Stop);
                let _ = thread.join();
                Err(PlaybackError::ControlUnavailable(format!(
                    "PipeWire device watcher failed during startup: {error}"
                )))
            }
        }
    }

    fn stop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        let _ = self.commands.send(WatchCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_watcher(
    callback: DeviceChangedCallback,
    commands: channel::Receiver<WatchCommand>,
    ready_tx: SyncSender<Result<(), String>>,
) -> PlaybackResult<()> {
    let mainloop = MainLoopRc::new(None)
        .map_err(|error| PlaybackError::DeviceUnavailable(error.to_string()))?;
    let context = ContextRc::new(&mainloop, None)
        .map_err(|error| PlaybackError::DeviceUnavailable(error.to_string()))?;
    let core = context
        .connect_rc(None)
        .map_err(|error| PlaybackError::DeviceUnavailable(error.to_string()))?;
    let registry = core
        .get_registry_rc()
        .map_err(|error| PlaybackError::DeviceUnavailable(error.to_string()))?;
    let state = Rc::new(RefCell::new(WatchState::new()));
    let (notifications, notification_rx) = channel::channel();

    let command_source = commands.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |WatchCommand::Stop| mainloop.quit()
    });
    let notification_source = notification_rx.attach(mainloop.loop_(), move |default_changed| {
        callback(DeviceTopologyChange { default_changed });
    });

    let state_for_global = Rc::clone(&state);
    let registry_for_global = registry.clone();
    let notifications_for_global = notifications.clone();
    let registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ == ObjectType::Metadata {
                if let Some(monitor) = bind_metadata_listener(
                    &registry_for_global,
                    global,
                    &state_for_global,
                    &notifications_for_global,
                ) {
                    state_for_global
                        .borrow_mut()
                        .metadata
                        .insert(global.id, monitor);
                }
                return;
            }
            if is_audio_sink(global) {
                let monitor = bind_sink_listener(
                    &registry_for_global,
                    global,
                    &state_for_global,
                    &notifications_for_global,
                );
                let mut state = state_for_global.borrow_mut();
                let changed = state.sink_ids.insert(global.id);
                if let Some(monitor) = monitor {
                    state.sinks.insert(global.id, monitor);
                }
                drop(state);
                notify_if_ready(&state_for_global, changed, false, &notifications_for_global);
            }
        })
        .global_remove({
            let state = Rc::clone(&state);
            move |id| {
                let mut state_mut = state.borrow_mut();
                state_mut.metadata.remove(&id);
                state_mut.sinks.remove(&id);
                let changed = state_mut.sink_ids.remove(&id);
                drop(state_mut);
                notify_if_ready(&state, changed, false, &notifications);
            }
        })
        .register();

    let ready = Rc::new(Cell::new(Some(ready_tx)));
    let ready_for_done = Rc::clone(&ready);
    let loop_for_error = mainloop.clone();
    let state_for_done = Rc::clone(&state);
    let core_listener = core
        .add_listener_local()
        .done(move |_id, _seq| {
            state_for_done.borrow_mut().initialized = true;
            if let Some(sender) = ready_for_done.take() {
                let _ = sender.send(Ok(()));
            }
        })
        .error(move |_id, _seq, code, message| {
            if let Some(sender) = ready.take() {
                let _ = sender.send(Err(format!(
                    "PipeWire connection failed ({code}): {message}"
                )));
            }
            tracing::warn!(code, "PipeWire device watcher connection failed");
            loop_for_error.quit();
        })
        .register();
    core.sync(0)
        .map_err(|error| PlaybackError::DeviceUnavailable(error.to_string()))?;
    mainloop.run();

    drop(core_listener);
    drop(registry_listener);
    drop(notification_source);
    drop(command_source);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_default_sink_metadata_keys() {
        assert!(is_default_sink_property(Some("default.audio.sink")));
        assert!(is_default_sink_property(Some(
            "default.configured.audio.sink"
        )));
        assert!(!is_default_sink_property(Some("default.audio.source")));
    }

    #[test]
    fn ignores_sink_runtime_state_changes() {
        assert!(is_relevant_sink_change(NodeChangeMask::PROPS));
        assert!(is_relevant_sink_change(NodeChangeMask::PARAMS));
        assert!(!is_relevant_sink_change(NodeChangeMask::STATE));
    }
}

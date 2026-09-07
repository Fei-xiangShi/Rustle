//! macOS menu-bar implementation.
//!
//! This remains a `tray-icon`/NSStatusItem backend, but all native objects are
//! owned and updated by the active AppKit/Winit UI thread. The deeper macOS UX
//! work (opt-in status item and dedicated template artwork) is intentionally a
//! separate task.

use super::{TrayCommand, TrayHandle, TrayPresentation, TrayWindowCommand};
use crate::domain::playback::PlayMode;
use anyhow::{Context, anyhow};
use std::cell::RefCell;
use tokio::sync::mpsc;
use tray_icon::{
    TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu},
};

const PLAY_PAUSE_ID: &str = "play_pause";
const PREV_TRACK_ID: &str = "prev_track";
const NEXT_TRACK_ID: &str = "next_track";
const TOGGLE_FAVORITE_ID: &str = "toggle_favorite";
const SEQUENTIAL_ID: &str = "sequential";
const LOOP_ALL_ID: &str = "loop_all";
const LOOP_ONE_ID: &str = "loop_one";
const SHUFFLE_ID: &str = "shuffle";
const TOGGLE_WINDOW_ID: &str = "toggle_window";
const QUIT_ID: &str = "quit";

thread_local! {
    static MACOS_TRAY: RefCell<Option<MacosTray>> = const { RefCell::new(None) };
}

pub fn start_macos_tray(
    presentation: TrayPresentation,
    command_capacity: usize,
) -> anyhow::Result<(TrayHandle, mpsc::Receiver<TrayCommand>)> {
    let (command_tx, command_rx) = mpsc::channel(command_capacity);
    install_menu_handler(command_tx.clone());

    let (menu, items) = build_menu(&presentation)?;
    let icon = load_icon()?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .with_menu_on_right_click(true)
        .with_tooltip(&presentation.tooltip)
        .with_icon(icon)
        .with_icon_as_template(true)
        .build()
        .map_err(|error| anyhow!("Failed to create macOS status item: {error}"))?;

    let owner = MacosTray {
        tray,
        items,
        presentation,
    };
    MACOS_TRAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return Err(anyhow!("macOS status item is already initialized"));
        }
        *slot = Some(owner);
        Ok(())
    })?;

    Ok((TrayHandle { _private: () }, command_rx))
}

pub fn update_state(presentation: TrayPresentation) -> anyhow::Result<()> {
    MACOS_TRAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        let owner = slot
            .as_mut()
            .ok_or_else(|| anyhow!("macOS status item is not initialized"))?;
        owner.apply_state(presentation)
    })
}

pub fn shutdown() {
    MACOS_TRAY.with(|slot| {
        let _ = slot.borrow_mut().take();
    });
}

pub fn is_available() -> bool {
    MACOS_TRAY.with(|slot| slot.try_borrow().ok().is_some_and(|slot| slot.is_some()))
}

struct MacosTray {
    tray: TrayIcon,
    items: MenuItems,
    presentation: TrayPresentation,
}

impl MacosTray {
    fn apply_state(&mut self, presentation: TrayPresentation) -> anyhow::Result<()> {
        self.items.now_playing.set_text(&presentation.now_playing);
        self.items.play_pause.set_text(presentation.play_pause);
        self.items.previous.set_text(presentation.previous);
        self.items.next.set_text(presentation.next);
        self.items.favorite.set_text(presentation.favorite);
        self.items
            .favorite
            .set_enabled(presentation.favorite_enabled);
        self.items
            .play_mode_menu
            .set_text(presentation.play_mode_label);
        self.items.sequential.set_text(presentation.sequential);
        self.items.loop_all.set_text(presentation.loop_all);
        self.items.loop_one.set_text(presentation.loop_one);
        self.items.shuffle.set_text(presentation.shuffle);
        self.items
            .toggle_window
            .set_text(presentation.toggle_window);
        self.items.quit.set_text(presentation.quit);
        self.items
            .sequential
            .set_checked(presentation.play_mode == PlayMode::Sequential);
        self.items
            .loop_all
            .set_checked(presentation.play_mode == PlayMode::LoopAll);
        self.items
            .loop_one
            .set_checked(presentation.play_mode == PlayMode::LoopOne);
        self.items
            .shuffle
            .set_checked(presentation.play_mode == PlayMode::Shuffle);
        self.tray
            .set_tooltip(Some(&presentation.tooltip))
            .context("Failed to update macOS status item tooltip")?;
        self.presentation = presentation;
        Ok(())
    }
}

struct MenuItems {
    now_playing: MenuItem,
    play_pause: MenuItem,
    previous: MenuItem,
    next: MenuItem,
    favorite: MenuItem,
    play_mode_menu: Submenu,
    sequential: CheckMenuItem,
    loop_all: CheckMenuItem,
    loop_one: CheckMenuItem,
    shuffle: CheckMenuItem,
    toggle_window: MenuItem,
    quit: MenuItem,
}

fn install_menu_handler(command_tx: mpsc::Sender<TrayCommand>) {
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = match event.id.0.as_str() {
            PLAY_PAUSE_ID => Some(TrayCommand::PlayPause),
            PREV_TRACK_ID => Some(TrayCommand::PrevTrack),
            NEXT_TRACK_ID => Some(TrayCommand::NextTrack),
            TOGGLE_FAVORITE_ID => Some(TrayCommand::ToggleFavorite),
            SEQUENTIAL_ID => Some(TrayCommand::SetPlayMode(PlayMode::Sequential)),
            LOOP_ALL_ID => Some(TrayCommand::SetPlayMode(PlayMode::LoopAll)),
            LOOP_ONE_ID => Some(TrayCommand::SetPlayMode(PlayMode::LoopOne)),
            SHUFFLE_ID => Some(TrayCommand::SetPlayMode(PlayMode::Shuffle)),
            TOGGLE_WINDOW_ID => Some(TrayCommand::Window(TrayWindowCommand::Toggle)),
            QUIT_ID => Some(TrayCommand::Quit),
            _ => None,
        };
        if let Some(command) = command {
            match command_tx.try_send(command) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!("macOS status item command channel is full; dropping input");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!("macOS status item command receiver is closed");
                }
            }
        }
    }));
}

fn build_menu(presentation: &TrayPresentation) -> anyhow::Result<(Menu, MenuItems)> {
    let menu = Menu::new();
    let now_playing = MenuItem::with_id(
        MenuId::new("now_playing"),
        &presentation.now_playing,
        false,
        None,
    );
    let play_pause = MenuItem::with_id(
        MenuId::new(PLAY_PAUSE_ID),
        presentation.play_pause,
        true,
        None,
    );
    let previous = MenuItem::with_id(
        MenuId::new(PREV_TRACK_ID),
        presentation.previous,
        true,
        None,
    );
    let next = MenuItem::with_id(MenuId::new(NEXT_TRACK_ID), presentation.next, true, None);
    let favorite = MenuItem::with_id(
        MenuId::new(TOGGLE_FAVORITE_ID),
        presentation.favorite,
        presentation.favorite_enabled,
        None,
    );
    let play_mode_menu = Submenu::new(presentation.play_mode_label, true);
    let sequential = CheckMenuItem::with_id(
        MenuId::new(SEQUENTIAL_ID),
        presentation.sequential,
        true,
        true,
        None,
    );
    let loop_all = CheckMenuItem::with_id(
        MenuId::new(LOOP_ALL_ID),
        presentation.loop_all,
        true,
        false,
        None,
    );
    let loop_one = CheckMenuItem::with_id(
        MenuId::new(LOOP_ONE_ID),
        presentation.loop_one,
        true,
        false,
        None,
    );
    let shuffle = CheckMenuItem::with_id(
        MenuId::new(SHUFFLE_ID),
        presentation.shuffle,
        true,
        false,
        None,
    );
    let toggle_window = MenuItem::with_id(
        MenuId::new(TOGGLE_WINDOW_ID),
        presentation.toggle_window,
        true,
        None,
    );
    let quit = MenuItem::with_id(MenuId::new(QUIT_ID), presentation.quit, true, None);

    menu.append(&now_playing)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&play_pause)?;
    menu.append(&previous)?;
    menu.append(&next)?;
    menu.append(&favorite)?;
    menu.append(&PredefinedMenuItem::separator())?;
    play_mode_menu.append(&sequential)?;
    play_mode_menu.append(&loop_all)?;
    play_mode_menu.append(&loop_one)?;
    play_mode_menu.append(&shuffle)?;
    menu.append(&play_mode_menu)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&toggle_window)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit)?;

    Ok((
        menu,
        MenuItems {
            now_playing,
            play_pause,
            previous,
            next,
            favorite,
            play_mode_menu,
            sequential,
            loop_all,
            loop_one,
            shuffle,
            toggle_window,
            quit,
        },
    ))
}

fn load_icon() -> anyhow::Result<tray_icon::Icon> {
    static ICON_DATA: &[u8] = include_bytes!("../../../assets/icons/icon_256.png");
    let image = image::load_from_memory(ICON_DATA).context("Failed to load status item icon")?;
    let rgba = image
        .resize(36, 36, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    tray_icon::Icon::from_rgba(rgba.into_raw(), width, height)
        .map_err(|error| anyhow!("Failed to create status item icon: {error}"))
}

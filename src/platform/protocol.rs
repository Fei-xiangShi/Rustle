//! Platform-level URL scheme handling.

/// Setup macOS URL event handler to capture `kAEGetURL` Apple Events.
///
/// On macOS, `rustle://` links arrive as Apple Events rather than CLI args.
/// This handler intercepts them and forwards URLs through the IPC channel.
#[cfg(target_os = "macos")]
pub fn setup_macos_url_handler(tx: crate::protocol::ipc::IpcSender) {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};
    use std::cell::RefCell;

    tracing::info!("Setting up macOS URL handler for rustle://");

    let Some(main_thread) = MainThreadMarker::new() else {
        tracing::error!("macOS URL handler must be installed on the AppKit main thread");
        return;
    };

    // Apple Events are delivered on AppKit's main thread. Keeping both values
    // in TLS makes that ownership part of the Rust type/lifetime boundary.
    thread_local! {
        static SENDER: RefCell<Option<crate::protocol::ipc::IpcSender>> = const {
            RefCell::new(None)
        };
    }
    SENDER.with(|sender| {
        *sender.borrow_mut() = Some(tx);
    });

    // Define a custom ObjC class whose method will be called for GURL events.
    define_class!(
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        struct RustleURLHandler;

        // SAFETY: define_class! creates a valid NSObject subclass and this
        // implementation adds no ivars or custom memory-management behavior.
        // MainThreadOnly prevents the object from becoming Send or Sync.
        unsafe impl NSObjectProtocol for RustleURLHandler {}

        impl RustleURLHandler {
            #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
            fn handle_get_url_event(
                &self,
                event: &NSAppleEventDescriptor,
                _reply: &NSAppleEventDescriptor,
            ) {
                // keyDirectObject = '----' (0x2d2d2d2d)
                // SAFETY: event is a live descriptor borrowed from AppKit for
                // this callback, and the selector returns an Objective-C
                // object represented by the declared retained optional type.
                let desc: Option<Retained<NSAppleEventDescriptor>> = unsafe {
                    msg_send![event, paramDescriptorForKeyword: 0x2d2d2d2du32]
                };
                if let Some(desc) = desc {
                    if let Some(url) = desc.stringValue() {
                        let url = url.to_string();
                        tracing::info!("macOS URL handler received: {}", url);
                        SENDER.with(|sender| {
                            let Ok(sender) = sender.try_borrow() else {
                                tracing::warn!("macOS URL sender is already borrowed");
                                return;
                            };
                            if let Some(sender) = sender.as_ref() {
                                let _ = sender
                                    .send(crate::protocol::ipc::IpcMessage::Uri(url));
                            }
                        });
                    }
                }
            }
        }
    );

    // Constructor: alloc → set_ivars → super init
    impl RustleURLHandler {
        fn new(main_thread: MainThreadMarker) -> Retained<Self> {
            let this = main_thread.alloc().set_ivars(());
            // SAFETY: this is a freshly allocated RustleURLHandler whose ivars
            // are initialized. NSObject's init returns the initialized object
            // under Objective-C's retained init-family ownership convention.
            unsafe { msg_send![super(this), init] }
        }
    }

    // NSAppleEventManager does NOT retain its handler — we hold it alive.
    thread_local! {
        static HANDLER: RefCell<Option<Retained<RustleURLHandler>>> = const {
            RefCell::new(None)
        };
    }
    let handler = RustleURLHandler::new(main_thread);

    // kAEGetURL: eventClass = 'GURL', eventID = 'GURL'
    let manager = NSAppleEventManager::sharedAppleEventManager();

    // SAFETY: manager and handler are live for this synchronous registration;
    // the class implements the exact two-argument selector ABI below. HANDLER
    // retains the object for every later callback, and all access is main-thread
    // bound by the class type and TLS owner.
    unsafe {
        let _: () = msg_send![
            &manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleGetURLEvent:withReplyEvent:),
            forEventClass: 0x4755524cu32,
            andEventID: 0x4755524cu32,
        ];
    }

    HANDLER.with(|slot| {
        *slot.borrow_mut() = Some(handler);
    });
    tracing::info!("macOS URL handler installed");
}

#[cfg(not(target_os = "macos"))]
pub fn setup_macos_url_handler(_tx: crate::protocol::ipc::IpcSender) {}

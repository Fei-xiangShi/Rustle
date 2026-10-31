//! Rustle application library and desktop composition entry point.

mod api;
mod app;
pub use rustle_application as application;
mod audio;
mod cache;
pub use rustle_storage::database;
#[cfg(feature = "diagnostics")]
pub mod diagnostics;
pub use rustle_domain as domain;
mod download;
pub mod error;
mod features;
mod i18n;
mod image;
mod metadata;
mod observability;
mod platform;
mod protocol;
mod runtime;
mod ui;
mod utils;

/// Starts the Rustle desktop application.
///
/// The startup order intentionally matches the historical binary entry point.
/// Infrastructure work can now evolve the composition root without making
/// every business module part of the executable target.
pub fn run() -> iced::Result {
    let _observability = observability::initialize();
    observability::set_runtime_phase(observability::RuntimePhase::SingleInstance);

    // Launchers pass a rustle:// URI, audio file paths ("Open with" / file
    // association / `file://` URIs from desktop entries), or nothing.
    let current_dir = std::env::current_dir().ok();
    let request =
        protocol::launch::parse_launch_args(std::env::args_os().skip(1), current_dir.as_deref());

    // Single-instance enforcement: hand the request to a running instance and
    // exit, or claim the primary role and keep the request for startup.
    if protocol::ipc::negotiate_startup(&request) == protocol::ipc::StartupRole::Forwarded {
        tracing::info!("Launch request forwarded to primary instance, exiting");
        return Ok(());
    }
    match request {
        protocol::launch::LaunchRequest::Normal => {}
        protocol::launch::LaunchRequest::Uri(uri) => protocol::ipc::set_pending_startup_uri(uri),
        protocol::launch::LaunchRequest::OpenFiles(paths) => {
            protocol::ipc::set_pending_startup_files(paths);
        }
    }

    observability::set_runtime_phase(observability::RuntimePhase::Platform);
    platform::init();

    // Run the application as a daemon (keeps running when windows are closed).
    // This allows the app to run in the background with system tray.
    observability::set_runtime_phase(observability::RuntimePhase::UiRuntime);
    iced::daemon(app::App::new, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .antialiasing(true)
        .run()
}

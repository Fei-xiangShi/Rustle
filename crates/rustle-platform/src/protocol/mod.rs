//! Desktop protocol parsing, single-instance IPC, and native URL delivery.

pub mod ipc;
pub mod launch;
mod macos;
pub mod uri;

pub use macos::setup_macos_url_handler;

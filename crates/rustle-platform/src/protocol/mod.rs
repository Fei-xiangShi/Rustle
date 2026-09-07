//! Desktop protocol parsing, single-instance IPC, and native URL delivery.

pub mod ipc;
mod macos;
pub mod uri;

pub use macos::setup_macos_url_handler;

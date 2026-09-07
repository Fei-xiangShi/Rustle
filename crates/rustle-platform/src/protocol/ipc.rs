//! IPC (Inter-Process Communication) for single-instance URL forwarding
//!
//! When a second instance is launched with a rustle:// URI, it forwards the URI
//! to the primary instance via a local socket and exits.
//!
//! On normal launch, a new instance first checks whether a primary instance is
//! already running — if so, it sends a Focus command and exits, enforcing
//! single-instance behavior.

use std::error::Error;
use std::fmt;
use std::sync::Mutex;

/// Well-known socket name for Rustle single-instance IPC
const IPC_SOCKET_NAME: &str = "rustle_instance";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcErrorKind {
    Namespace,
    Connect,
    Write,
}

type BoxError = Box<dyn Error + Send + Sync + 'static>;

#[derive(Debug)]
pub struct IpcError {
    kind: IpcErrorKind,
    source: BoxError,
}

impl IpcError {
    fn namespace<E>(source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind: IpcErrorKind::Namespace,
            source: Box::new(source),
        }
    }

    fn connect<E>(source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind: IpcErrorKind::Connect,
            source: Box::new(source),
        }
    }

    fn write<E>(source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind: IpcErrorKind::Write,
            source: Box::new(source),
        }
    }

    pub const fn kind(&self) -> IpcErrorKind {
        self.kind
    }

    pub const fn code(&self) -> rustle_domain::error::ErrorCode {
        rustle_domain::error::ErrorCode::PlatformUnavailable
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            IpcErrorKind::Namespace => "The local IPC endpoint could not be prepared",
            IpcErrorKind::Connect => "No reachable primary Rustle instance was found",
            IpcErrorKind::Write => "The local IPC message could not be delivered",
        })
    }
}

impl Error for IpcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl From<IpcError> for rustle_application::error::AppError {
    fn from(error: IpcError) -> Self {
        let code = error.code();
        let summary = error.to_string();
        rustle_application::error::AppError::with_source(code, summary, error)
    }
}

/// Messages exchanged between instances via the IPC socket
#[derive(Debug, Clone)]
pub enum IpcMessage {
    /// A rustle:// URI to process
    Uri(String),
    /// Show and focus the main window
    Focus,
}

/// Channel types for IPC message forwarding
pub type IpcSender = tokio::sync::mpsc::UnboundedSender<IpcMessage>;
pub type IpcReceiver = tokio::sync::mpsc::UnboundedReceiver<IpcMessage>;

/// Create a new IPC channel
pub fn ipc_channel() -> (IpcSender, IpcReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Stores a URI from CLI args that should be processed on startup
static PENDING_STARTUP_URI: Mutex<Option<String>> = Mutex::new(None);

/// Store a URI to be processed when the application starts
pub fn set_pending_startup_uri(uri: String) {
    if let Ok(mut guard) = PENDING_STARTUP_URI.lock() {
        *guard = Some(uri);
    }
}

/// Take and clear the pending startup URI
pub fn take_pending_startup_uri() -> Option<String> {
    PENDING_STARTUP_URI.lock().ok()?.take()
}

/// Wire format prefixes
const PREFIX_FOCUS: &[u8] = b"FOCUS";
const PREFIX_URI: &[u8] = b"URI:";

/// Spawn an IPC listener in a tokio background task.
///
/// Binds a local socket and forwards received messages to the application
/// via the provided mpsc sender.
pub fn spawn_ipc_listener(tx: IpcSender) -> tokio::task::JoinHandle<()> {
    use interprocess::local_socket::ToNsName;
    use interprocess::local_socket::traits::tokio::Listener;
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions};
    use tokio::io::AsyncReadExt;

    tokio::spawn(async move {
        let name = match IPC_SOCKET_NAME.to_ns_name::<GenericNamespaced>() {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("Failed to create IPC socket name: {}", e);
                return;
            }
        };

        let listener = match ListenerOptions::new().name(name).create_tokio() {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("Failed to bind IPC socket: {}", e);
                return;
            }
        };

        tracing::info!("IPC listener started on '{}'", IPC_SOCKET_NAME);

        loop {
            match listener.accept().await {
                Ok(mut stream) => {
                    let mut buf = vec![0u8; 4096];
                    match stream.read(&mut buf).await {
                        Ok(n) if n > 0 => {
                            buf.truncate(n);
                            let msg = if buf.starts_with(PREFIX_FOCUS) {
                                IpcMessage::Focus
                            } else if buf.starts_with(PREFIX_URI) {
                                match String::from_utf8(buf[PREFIX_URI.len()..].to_vec()) {
                                    Ok(uri) => {
                                        let uri = uri.trim().to_string();
                                        IpcMessage::Uri(uri)
                                    }
                                    Err(e) => {
                                        tracing::warn!("IPC received invalid UTF-8: {}", e);
                                        continue;
                                    }
                                }
                            } else {
                                // Backward compat: raw URI without prefix
                                match String::from_utf8(buf) {
                                    Ok(uri) => {
                                        let uri = uri.trim().to_string();
                                        IpcMessage::Uri(uri)
                                    }
                                    Err(e) => {
                                        tracing::warn!("IPC received invalid UTF-8: {}", e);
                                        continue;
                                    }
                                }
                            };
                            let message_kind = match &msg {
                                IpcMessage::Uri(_) => "uri",
                                IpcMessage::Focus => "focus",
                            };
                            tracing::info!(message_kind, "IPC message received");
                            let _ = tx.send(msg);
                        }
                        Ok(_) => {
                            tracing::warn!("IPC received empty message");
                        }
                        Err(e) => {
                            tracing::warn!("IPC read error: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("IPC accept error: {}", e);
                }
            }
        }
    })
}

/// Forward a URI to the primary instance via local socket.
///
/// Called by secondary instances. Returns Ok(()) if the URI was
/// successfully forwarded, or Err if no primary instance is running.
pub fn forward_uri_to_primary(uri: &str) -> Result<(), IpcError> {
    let payload = format!("URI:{}", uri);
    write_to_socket(&payload, "uri")
}

/// Send a focus command to the primary instance via local socket.
///
/// Called by secondary instances on normal launch. Returns Ok(())
/// if the focus command was forwarded, or Err if no primary instance is running.
pub fn forward_focus_to_primary() -> Result<(), IpcError> {
    write_to_socket("FOCUS", "focus")
}

fn write_to_socket(payload: &str, message_kind: &'static str) -> Result<(), IpcError> {
    use interprocess::local_socket::ConnectOptions;
    use interprocess::local_socket::GenericNamespaced;
    use interprocess::local_socket::ToNsName;
    use std::io::Write;

    let name = IPC_SOCKET_NAME
        .to_ns_name::<GenericNamespaced>()
        .map_err(IpcError::namespace)?;

    let mut stream = ConnectOptions::new()
        .name(name)
        .connect_sync()
        .map_err(IpcError::connect)?;

    stream
        .write_all(payload.as_bytes())
        .map_err(IpcError::write)?;

    tracing::info!(
        message_kind,
        payload_bytes = payload.len(),
        "IPC message forwarded"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_errors_expose_stable_platform_policy_without_source_text() {
        let error = IpcError::connect(std::io::Error::other("private endpoint detail"));

        assert_eq!(error.kind(), IpcErrorKind::Connect);
        assert_eq!(
            error.code(),
            rustle_domain::error::ErrorCode::PlatformUnavailable
        );
        assert!(!error.to_string().contains("private endpoint detail"));
        assert_eq!(
            error.source().expect("native source").to_string(),
            "private endpoint detail"
        );
    }
}

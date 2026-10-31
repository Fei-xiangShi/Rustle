//! IPC (Inter-Process Communication) for single-instance launch forwarding
//!
//! When a second instance is launched with a rustle:// URI or with audio file
//! paths, it forwards the request to the primary instance via a local socket
//! and exits.
//!
//! On normal launch, a new instance first checks whether a primary instance is
//! already running — if so, it sends a Focus command and exits, enforcing
//! single-instance behavior.
//!
//! The primary instance claims the socket name synchronously during startup,
//! before the UI runtime exists. Shells that open several files at once may
//! start several processes simultaneously; claiming early lets exactly one of
//! them win while the others retry forwarding to it.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::launch::LaunchRequest;

/// Well-known socket name for Rustle single-instance IPC
const IPC_SOCKET_NAME: &str = "rustle_instance";

/// Upper bound for a single IPC message (file lists can be long).
const MAX_MESSAGE_BYTES: u64 = 1024 * 1024;

/// How long a starting process keeps trying to reach a primary instance that
/// is concurrently claiming the socket name.
const STARTUP_NEGOTIATION_ATTEMPTS: u32 = 20;
const STARTUP_NEGOTIATION_DELAY: Duration = Duration::from_millis(100);

/// Failed attempts after which an unreachable but taken name is treated as
/// stale. A live Unix listener accepts connections into its backlog as soon as
/// it is bound, so only a just-binding peer can look unreachable this long.
const STALE_RECLAIM_AFTER_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcErrorKind {
    Namespace,
    Connect,
    Write,
    Listen,
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

    fn listen<E>(source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind: IpcErrorKind::Listen,
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
            IpcErrorKind::Listen => "The local IPC endpoint could not be claimed",
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcMessage {
    /// A rustle:// URI to process
    Uri(String),
    /// Local files to open for playback, in launcher order
    OpenFiles(Vec<PathBuf>),
    /// Show and focus the main window
    Focus,
}

impl IpcMessage {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Uri(_) => "uri",
            Self::OpenFiles(_) => "open_files",
            Self::Focus => "focus",
        }
    }
}

impl From<LaunchRequest> for IpcMessage {
    fn from(request: LaunchRequest) -> Self {
        match request {
            LaunchRequest::Normal => Self::Focus,
            LaunchRequest::Uri(uri) => Self::Uri(uri),
            LaunchRequest::OpenFiles(paths) => Self::OpenFiles(paths),
        }
    }
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

/// Stores file paths from CLI args that should be opened on startup
static PENDING_STARTUP_FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Channel already fed by the primary-instance listener claimed at startup
static PRIMARY_CHANNEL: Mutex<Option<(IpcSender, IpcReceiver)>> = Mutex::new(None);

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

/// Store file paths to be opened when the application starts
pub fn set_pending_startup_files(paths: Vec<PathBuf>) {
    if let Ok(mut guard) = PENDING_STARTUP_FILES.lock() {
        *guard = paths;
    }
}

/// Take and clear the pending startup files
pub fn take_pending_startup_files() -> Vec<PathBuf> {
    PENDING_STARTUP_FILES
        .lock()
        .map(|mut guard| std::mem::take(&mut *guard))
        .unwrap_or_default()
}

/// Take the channel served by the listener claimed during startup, if any.
///
/// `None` means this process runs standalone without a listener (the endpoint
/// could not be claimed); the caller should use a local-only channel.
pub fn take_primary_channel() -> Option<(IpcSender, IpcReceiver)> {
    PRIMARY_CHANNEL.lock().ok()?.take()
}

/// Outcome of startup single-instance negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupRole {
    /// The request was delivered to an existing instance; exit now.
    Forwarded,
    /// This process is the primary instance and should start the UI.
    Primary,
}

/// Deliver `request` to an existing instance or become the primary instance.
///
/// Concurrent launches (for example, Explorer opening several selected files
/// as separate processes) race to claim the socket name. Losers retry
/// forwarding until the winner starts accepting connections. If no instance
/// can be reached or claimed, the process starts standalone without IPC.
pub fn negotiate_startup(request: &LaunchRequest) -> StartupRole {
    let message = IpcMessage::from(request.clone());

    for attempt in 0..STARTUP_NEGOTIATION_ATTEMPTS {
        if forward_to_primary(&message).is_ok() {
            return StartupRole::Forwarded;
        }

        // A socket file left behind by a crashed process (Unix filesystem
        // names) never becomes reachable; reclaim it instead of waiting out
        // every attempt. Overwriting has no effect on Windows named pipes.
        let reclaim_stale = attempt + 1 >= STALE_RECLAIM_AFTER_ATTEMPTS;
        match claim_primary_listener(reclaim_stale) {
            Ok(listener) => {
                let (tx, rx) = ipc_channel();
                match serve_listener(listener, tx.clone()) {
                    Ok(()) => {
                        if let Ok(mut guard) = PRIMARY_CHANNEL.lock() {
                            *guard = Some((tx, rx));
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "Failed to start IPC listener thread");
                    }
                }
                return StartupRole::Primary;
            }
            Err(error) => {
                tracing::debug!(
                    attempt,
                    error = %error,
                    "IPC endpoint busy, retrying forward to primary instance"
                );
                std::thread::sleep(STARTUP_NEGOTIATION_DELAY);
            }
        }
    }

    tracing::warn!("Could not reach or claim the single-instance endpoint, starting standalone");
    StartupRole::Primary
}

fn claim_primary_listener(
    reclaim_stale: bool,
) -> Result<interprocess::local_socket::Listener, IpcError> {
    use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName};

    let name = IPC_SOCKET_NAME
        .to_ns_name::<GenericNamespaced>()
        .map_err(IpcError::namespace)?;
    ListenerOptions::new()
        .name(name)
        .try_overwrite(reclaim_stale)
        .create_sync()
        .map_err(IpcError::listen)
}

fn serve_listener(
    listener: interprocess::local_socket::Listener,
    tx: IpcSender,
) -> Result<(), IpcError> {
    use interprocess::local_socket::traits::Listener as _;

    std::thread::Builder::new()
        .name("rustle-ipc".to_string())
        .spawn(move || {
            tracing::info!("IPC listener started on '{}'", IPC_SOCKET_NAME);
            loop {
                let mut stream = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) => {
                        tracing::warn!("IPC accept error: {}", error);
                        continue;
                    }
                };
                let mut buf = Vec::new();
                let read = std::io::Read::read_to_end(
                    &mut std::io::Read::take(&mut stream, MAX_MESSAGE_BYTES),
                    &mut buf,
                );
                // Some named-pipe peers disconnect right after writing; keep
                // whatever arrived before the error.
                if let Err(error) = read
                    && buf.is_empty()
                {
                    tracing::warn!("IPC read error: {}", error);
                    continue;
                }
                if !dispatch_received(&buf, &tx) {
                    break;
                }
            }
        })
        .map(|_| ())
        .map_err(IpcError::listen)
}

/// Decode and forward one received message. Returns false once the
/// application side of the channel has gone away.
fn dispatch_received(buf: &[u8], tx: &IpcSender) -> bool {
    let Some(message) = decode_message(buf) else {
        tracing::warn!(bytes = buf.len(), "IPC received an unrecognized message");
        return true;
    };
    tracing::info!(message_kind = message.kind(), "IPC message received");
    tx.send(message).is_ok()
}

/// Wire format prefixes
const PREFIX_FOCUS: &[u8] = b"FOCUS";
const PREFIX_URI: &[u8] = b"URI:";
const PREFIX_FILES: &[u8] = b"FILES:";

/// Separator between paths in a FILES message (paths cannot contain NUL).
const PATH_SEPARATOR: u8 = 0;

fn encode_message(message: &IpcMessage) -> Vec<u8> {
    match message {
        IpcMessage::Focus => PREFIX_FOCUS.to_vec(),
        IpcMessage::Uri(uri) => [PREFIX_URI, uri.as_bytes()].concat(),
        IpcMessage::OpenFiles(paths) => {
            let mut buf = PREFIX_FILES.to_vec();
            for (index, path) in paths.iter().enumerate() {
                if index > 0 {
                    buf.push(PATH_SEPARATOR);
                }
                buf.extend_from_slice(&path_to_bytes(path));
            }
            buf
        }
    }
}

fn decode_message(buf: &[u8]) -> Option<IpcMessage> {
    if buf.starts_with(PREFIX_FOCUS) {
        return Some(IpcMessage::Focus);
    }
    if let Some(rest) = buf.strip_prefix(PREFIX_FILES) {
        let paths: Vec<PathBuf> = rest
            .split(|byte| *byte == PATH_SEPARATOR)
            .filter(|segment| !segment.is_empty())
            .map(bytes_to_path)
            .collect();
        return (!paths.is_empty()).then_some(IpcMessage::OpenFiles(paths));
    }
    // Backward compat: raw URI without prefix
    let uri = buf.strip_prefix(PREFIX_URI).unwrap_or(buf);
    match std::str::from_utf8(uri) {
        Ok(uri) => Some(IpcMessage::Uri(uri.trim().to_string())),
        Err(error) => {
            tracing::warn!("IPC received invalid UTF-8: {}", error);
            None
        }
    }
}

#[cfg(unix)]
fn path_to_bytes(path: &Path) -> std::borrow::Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    std::borrow::Cow::Borrowed(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn path_to_bytes(path: &Path) -> std::borrow::Cow<'_, [u8]> {
    match path.to_string_lossy() {
        std::borrow::Cow::Borrowed(text) => std::borrow::Cow::Borrowed(text.as_bytes()),
        std::borrow::Cow::Owned(text) => std::borrow::Cow::Owned(text.into_bytes()),
    }
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// Forward a message to the primary instance via local socket.
///
/// Returns Ok(()) if the message was delivered, or Err if no primary
/// instance is reachable.
fn forward_to_primary(message: &IpcMessage) -> Result<(), IpcError> {
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

    let payload = encode_message(message);
    stream.write_all(&payload).map_err(IpcError::write)?;
    stream.flush().map_err(IpcError::write)?;

    tracing::info!(
        message_kind = message.kind(),
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

    #[test]
    fn messages_round_trip_through_wire_format() {
        let files = IpcMessage::OpenFiles(vec![
            std::env::temp_dir().join("a b.mp3"),
            std::env::temp_dir().join("歌曲\n换行.flac"),
        ]);
        for message in [
            IpcMessage::Focus,
            IpcMessage::Uri("rustle://play/1".to_string()),
            files,
        ] {
            assert_eq!(decode_message(&encode_message(&message)), Some(message));
        }
    }

    #[test]
    fn unprefixed_payload_is_treated_as_uri_for_backward_compatibility() {
        assert_eq!(
            decode_message(b"rustle://search/x\n"),
            Some(IpcMessage::Uri("rustle://search/x".to_string()))
        );
    }

    #[test]
    fn empty_file_list_is_rejected() {
        assert_eq!(decode_message(b"FILES:"), None);
    }

    #[test]
    fn launch_requests_map_to_ipc_messages() {
        assert_eq!(IpcMessage::from(LaunchRequest::Normal), IpcMessage::Focus);
        assert_eq!(
            IpcMessage::from(LaunchRequest::Uri("rustle://x".to_string())),
            IpcMessage::Uri("rustle://x".to_string())
        );
    }
}

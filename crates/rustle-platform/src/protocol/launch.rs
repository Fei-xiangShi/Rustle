//! Command-line launch request parsing.
//!
//! Desktop shells start Rustle with either a `rustle://` URI (protocol
//! handler), one or more audio file paths ("Open with" / file association),
//! or no arguments at all. Linux desktop entries using `%U` deliver files as
//! `file://` URIs, so those are normalized into plain paths here.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What the current process was asked to do by its launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchRequest {
    /// Plain launch: show the application.
    Normal,
    /// Handle a `rustle://` protocol URI.
    Uri(String),
    /// Open local files for playback, in launcher order.
    OpenFiles(Vec<PathBuf>),
}

/// Returns true when `text` is a Rustle protocol URI.
pub fn is_protocol_uri(text: &str) -> bool {
    text.starts_with("rustle://") || text.starts_with("rustle-dev://")
}

/// Parse launcher arguments (excluding the program name).
///
/// Relative paths are resolved against `current_dir` because the primary
/// instance that eventually opens them runs with a different working
/// directory. Option-like arguments (such as macOS `-psn_*`) are ignored.
pub fn parse_launch_args<I>(args: I, current_dir: Option<&Path>) -> LaunchRequest
where
    I: IntoIterator<Item = OsString>,
{
    let mut files = Vec::new();

    for arg in args {
        let Some(text) = arg.to_str() else {
            // Non-UTF-8 arguments can only be native paths.
            files.push(absolutize(PathBuf::from(arg), current_dir));
            continue;
        };

        if is_protocol_uri(text) {
            if files.is_empty() {
                return LaunchRequest::Uri(text.to_string());
            }
            continue;
        }

        if text.starts_with("file://") {
            match url::Url::parse(text)
                .ok()
                .and_then(|url| url.to_file_path().ok())
            {
                Some(path) => files.push(path),
                None => tracing::warn!("Ignoring unparseable file URI launch argument"),
            }
            continue;
        }

        if text.is_empty() || text.starts_with('-') {
            continue;
        }

        files.push(absolutize(PathBuf::from(text), current_dir));
    }

    if files.is_empty() {
        LaunchRequest::Normal
    } else {
        LaunchRequest::OpenFiles(files)
    }
}

fn absolutize(path: PathBuf, current_dir: Option<&Path>) -> PathBuf {
    match current_dir {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_arguments_is_a_normal_launch() {
        assert_eq!(parse_launch_args(args(&[]), None), LaunchRequest::Normal);
        assert_eq!(
            parse_launch_args(args(&["-psn_0_12345"]), None),
            LaunchRequest::Normal
        );
    }

    #[test]
    fn protocol_uri_takes_precedence_when_first() {
        assert_eq!(
            parse_launch_args(args(&["rustle://play/1"]), None),
            LaunchRequest::Uri("rustle://play/1".to_string())
        );
        assert_eq!(
            parse_launch_args(args(&["rustle-dev://search/x"]), None),
            LaunchRequest::Uri("rustle-dev://search/x".to_string())
        );
    }

    #[test]
    fn relative_paths_resolve_against_launch_directory() {
        let cwd = std::env::temp_dir();
        let request = parse_launch_args(args(&["a.mp3", "sub/b.flac"]), Some(&cwd));
        assert_eq!(
            request,
            LaunchRequest::OpenFiles(vec![cwd.join("a.mp3"), cwd.join("sub/b.flac")])
        );
    }

    #[test]
    fn absolute_paths_are_kept_in_order() {
        let first = std::env::temp_dir().join("first.mp3");
        let second = std::env::temp_dir().join("second.mp3");
        let request = parse_launch_args(
            vec![
                first.clone().into_os_string(),
                second.clone().into_os_string(),
            ],
            Some(Path::new("/unused")),
        );
        assert_eq!(request, LaunchRequest::OpenFiles(vec![first, second]));
    }

    #[test]
    fn file_uris_are_decoded_into_paths() {
        let path = std::env::temp_dir().join("My Song 测试.mp3");
        let uri = url::Url::from_file_path(&path).expect("absolute path converts to URI");
        let request = parse_launch_args(args(&[uri.as_str()]), None);
        assert_eq!(request, LaunchRequest::OpenFiles(vec![path]));
    }

    #[test]
    fn protocol_uri_after_files_is_ignored() {
        let cwd = std::env::temp_dir();
        let request = parse_launch_args(args(&["a.mp3", "rustle://play/1"]), Some(&cwd));
        assert_eq!(request, LaunchRequest::OpenFiles(vec![cwd.join("a.mp3")]));
    }
}

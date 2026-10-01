use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::NcmError;

const SESSION_VERSION: u8 = 2;
static SESSION_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SessionState {
    #[serde(default)]
    version: u8,
    #[serde(default)]
    cookie: String,
    #[serde(default)]
    device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anonymous_token: Option<String>,
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState")
            .field("version", &self.version)
            .field("has_cookie", &!self.cookie.trim().is_empty())
            .field("has_login", &self.has_login())
            .field("has_anonymous_token", &self.anonymous_token.is_some())
            .field("has_device_id", &is_valid_device_id(&self.device_id))
            .finish()
    }
}

impl SessionState {
    pub(super) fn new(cookie: Option<String>) -> Self {
        Self {
            version: SESSION_VERSION,
            cookie: cookie.unwrap_or_default(),
            device_id: ncm_api_rs::util::device::generate_device_id(),
            anonymous_token: None,
        }
        .normalized()
    }

    pub(super) fn load(path: &Path) -> Result<Option<Self>, NcmError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let state = serde_json::from_slice::<Self>(&bytes)?.normalized();
        Ok(Some(state))
    }

    pub(super) fn save(&self, path: &Path) -> Result<(), NcmError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec(self)?;
        let temp_path = session_temp_path(path);
        let write_result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            replace_closed_file(&temp_path, path)
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        write_result?;
        Ok(())
    }

    pub(super) fn device_id(&self) -> &str {
        &self.device_id
    }

    pub(super) fn anonymous_token(&self) -> Option<&str> {
        self.anonymous_token.as_deref()
    }

    pub(super) fn has_login(&self) -> bool {
        cookie_value(&self.cookie, "MUSIC_U").is_some()
    }

    pub(super) fn has_session_identity(&self) -> bool {
        self.has_login()
            || self.anonymous_token.is_some()
            || cookie_value(&self.cookie, "MUSIC_A").is_some()
    }

    pub(super) fn request_cookie(&self) -> String {
        self.request_cookie_with_os(None)
    }

    pub(super) fn request_cookie_for_os(&self, os: &str) -> String {
        self.request_cookie_with_os(Some(os))
    }

    fn request_cookie_with_os(&self, os: Option<&str>) -> String {
        let mut pairs = cookie_pairs(&self.cookie);
        if self.has_login() {
            pairs.retain(|(name, _)| name != "MUSIC_A");
        }
        upsert_cookie(&mut pairs, "deviceId", self.device_id.clone());
        if !self.has_login()
            && let Some(token) = self
                .anonymous_token
                .clone()
                .or_else(|| cookie_value(&self.cookie, "MUSIC_A"))
        {
            upsert_cookie(&mut pairs, "MUSIC_A", token);
        }
        if let Some(os) = os {
            upsert_cookie(&mut pairs, "os", os.to_string());
        }
        serialize_cookie_pairs(pairs)
    }

    pub(super) fn merge_response_cookies(&mut self, cookies: Vec<String>) {
        if cookies.is_empty() {
            return;
        }
        let mut pairs = cookie_pairs(&self.cookie);
        for raw in cookies {
            if let Some((name, value)) = cookie_pair(&raw) {
                upsert_cookie(&mut pairs, &name, value);
            }
        }
        self.cookie = serialize_cookie_pairs(pairs);
        if self.anonymous_token.is_none() {
            self.anonymous_token = cookie_value(&self.cookie, "MUSIC_A");
        }
    }

    pub(super) fn set_anonymous_token(&mut self, token: String) {
        let token = token.trim().to_string();
        if token.is_empty() {
            return;
        }
        self.anonymous_token = Some(token.clone());
        let mut pairs = cookie_pairs(&self.cookie);
        upsert_cookie(&mut pairs, "MUSIC_A", token);
        self.cookie = serialize_cookie_pairs(pairs);
    }

    pub(super) fn clear_authenticated_session(&mut self) {
        let anonymous_token = self
            .anonymous_token
            .clone()
            .or_else(|| cookie_value(&self.cookie, "MUSIC_A"));
        self.cookie.clear();
        if let Some(token) = anonymous_token {
            self.set_anonymous_token(token);
        }
    }

    fn normalized(mut self) -> Self {
        self.version = SESSION_VERSION;
        if !is_valid_device_id(&self.device_id) {
            self.device_id = ncm_api_rs::util::device::generate_device_id();
        }
        if self.anonymous_token.is_none() {
            self.anonymous_token = cookie_value(&self.cookie, "MUSIC_A");
        }
        self
    }
}

fn cookie_pairs(cookie: &str) -> Vec<(String, String)> {
    cookie.split(';').filter_map(cookie_pair).collect()
}

fn cookie_pair(raw: &str) -> Option<(String, String)> {
    let pair = raw.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), value.trim().to_string()))
}

fn cookie_value(cookie: &str, name: &str) -> Option<String> {
    cookie_pairs(cookie)
        .into_iter()
        .find_map(|(key, value)| (key == name && !value.is_empty()).then_some(value))
}

fn upsert_cookie(pairs: &mut Vec<(String, String)>, name: &str, value: String) {
    let mut first = None;
    let mut index = 0;
    while index < pairs.len() {
        if pairs[index].0 == name {
            if first.is_none() {
                first = Some(index);
                index += 1;
            } else {
                pairs.remove(index);
            }
        } else {
            index += 1;
        }
    }
    if let Some(index) = first {
        pairs[index].1 = value;
        return;
    }
    pairs.push((name.to_string(), value));
}

fn serialize_cookie_pairs(pairs: Vec<(String, String)>) -> String {
    pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn is_valid_device_id(device_id: &str) -> bool {
    device_id.len() == 52 && device_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn session_temp_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("ncm-session");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = SESSION_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{name}.{}.{}.{}.tmp",
        std::process::id(),
        nonce,
        counter
    ))
}

#[cfg(target_os = "windows")]
fn replace_closed_file(temp_path: &Path, final_path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temp_wide = temp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let final_wide = final_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both buffers are NUL-terminated and remain valid for the
    // synchronous call. Both files are closed before replacement, and native
    // failure is returned to the caller without exposing either path.
    let moved = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            final_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_closed_file(temp_path: &Path, final_path: &Path) -> io::Result<()> {
    fs::rename(temp_path, final_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        let nonce = SESSION_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rustle-ncm-session-test-{}-{nonce}-{name}.json",
            std::process::id()
        ))
    }

    #[test]
    fn legacy_cookie_file_migrates_without_losing_login() {
        let path = test_path("legacy");
        fs::write(&path, br#"{"cookie":"MUSIC_U=login; __csrf=token"}"#).unwrap();

        let state = SessionState::load(&path).unwrap().unwrap();
        assert!(state.has_login());
        assert!(is_valid_device_id(state.device_id()));
        state.save(&path).unwrap();

        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], SESSION_VERSION);
        assert_eq!(saved["cookie"], "MUSIC_U=login; __csrf=token");
        assert_eq!(saved["device_id"].as_str().unwrap().len(), 52);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn stable_device_id_survives_restart() {
        let path = test_path("stable-device");
        let state = SessionState::new(None);
        let expected = state.device_id().to_string();
        state.save(&path).unwrap();

        let reloaded = SessionState::load(&path).unwrap().unwrap();
        assert_eq!(reloaded.device_id(), expected);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn request_cookie_uses_stable_device_and_only_anonymous_identity_when_logged_out() {
        let mut state = SessionState::new(Some("deviceId=OLD".to_string()));
        let stable = state.device_id().to_string();
        state.set_anonymous_token("anonymous".to_string());
        let anonymous_cookie = state.request_cookie();
        assert!(anonymous_cookie.contains(&format!("deviceId={stable}")));
        assert!(anonymous_cookie.contains("MUSIC_A=anonymous"));

        state.merge_response_cookies(vec!["MUSIC_U=login; Path=/".to_string()]);
        let logged_in_cookie = state.request_cookie();
        assert!(logged_in_cookie.contains("MUSIC_U=login"));
        assert!(!logged_in_cookie.contains("MUSIC_A=anonymous"));
    }

    #[test]
    fn logout_preserves_only_anonymous_identity_and_stable_device() {
        let mut state = SessionState::new(Some(
            "MUSIC_A=anonymous; MUSIC_U=login; __csrf=csrf; os=pc".to_string(),
        ));
        let device_id = state.device_id().to_string();

        state.clear_authenticated_session();

        assert!(!state.has_login());
        assert_eq!(state.device_id(), device_id);
        assert_eq!(state.cookie, "MUSIC_A=anonymous");
        assert_eq!(state.anonymous_token(), Some("anonymous"));
        assert!(state.request_cookie().contains("MUSIC_A=anonymous"));
        assert!(
            state
                .request_cookie()
                .contains(&format!("deviceId={device_id}"))
        );
    }

    #[test]
    fn request_os_override_does_not_mutate_the_persisted_session() {
        let state = SessionState::new(Some("MUSIC_U=login; os=pc".to_string()));

        assert!(state.request_cookie_for_os("osx").contains("os=osx"));
        assert!(state.request_cookie().contains("os=pc"));
    }

    #[test]
    fn atomic_save_replaces_existing_file_without_leaving_temporary_files() {
        let path = test_path("atomic-replace");
        fs::write(&path, br#"{"cookie":"old"}"#).unwrap();
        let state = SessionState::new(Some("MUSIC_A=anonymous".to_string()));

        state.save(&path).unwrap();

        let saved = SessionState::load(&path).unwrap().unwrap();
        assert_eq!(saved.anonymous_token(), Some("anonymous"));
        let prefix = format!(".{}.", path.file_name().unwrap().to_string_lossy());
        let temporary_count = path
            .parent()
            .unwrap()
            .read_dir()
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .count();
        assert_eq!(temporary_count, 0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn debug_projection_contains_no_session_values() {
        let mut state = SessionState::new(Some("MUSIC_U=super-secret".to_string()));
        state.set_anonymous_token("anonymous-secret".to_string());
        let debug = format!("{state:?}");
        assert!(!debug.contains("super-secret"));
        assert!(!debug.contains("anonymous-secret"));
        assert!(!debug.contains(state.device_id()));
    }
}

//! MSI uninstall helper for removing Rustle-owned user data.
//!
//! This binary is only invoked by the MSI when the user explicitly selects
//! data removal (or passes `REMOVE_RUSTLE_DATA=1` to msiexec). It deliberately
//! uses a fixed allow-list of Rustle-owned paths and never follows paths from
//! settings, the database, or user-selected storage directories.

#[cfg(windows)]
use std::{fs, io, path::PathBuf, process::Command};

#[cfg(windows)]
fn app_is_running() -> io::Result<bool> {
    let output = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq rustle.exe", "/NH"])
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(
            "tasklist could not inspect running processes",
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.to_ascii_lowercase().contains("rustle.exe")))
}

#[cfg(windows)]
fn roaming_paths() -> io::Result<Vec<PathBuf>> {
    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("APPDATA is not available"))?;

    let rustle_data = app_data.join("rustle").join("Rustle");
    let ncm_data = app_data.join("fxs").join("rustle").join("data");

    Ok([
        rustle_data.join("config").join("settings.json"),
        rustle_data.join("data").join("rustle.db"),
        rustle_data.join("data").join("rustle.db-wal"),
        rustle_data.join("data").join("rustle.db-shm"),
        ncm_data.join("cookies.json"),
    ]
    .into_iter()
    .collect())
}

#[cfg(windows)]
fn owned_directories() -> io::Result<Vec<PathBuf>> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("LOCALAPPDATA is not available"))?;
    let temp = std::env::var_os("TEMP")
        .or_else(|| std::env::var_os("TMP"))
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("TEMP is not available"))?;

    Ok(vec![
        local_app_data.join("fxs").join("rustle").join("cache"),
        temp.join("rustle_covers"),
    ])
}

#[cfg(windows)]
fn remove_data() -> io::Result<()> {
    if app_is_running()? {
        return Err(io::Error::other(
            "Rustle is still running; close it before removing Rustle data",
        ));
    }

    let mut failures = Vec::new();

    for path in roaming_paths()? {
        if path.exists() && fs::remove_file(&path).is_err() && path.exists() {
            failures.push(path);
        }
    }

    for path in owned_directories()? {
        if path.exists() && fs::remove_dir_all(&path).is_err() && path.exists() {
            failures.push(path);
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "failed to remove Rustle-owned paths: {}",
            failures
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

#[cfg(windows)]
fn main() {
    if std::env::args().any(|arg| arg == "--remove-data")
        && let Err(error) = remove_data()
    {
        eprintln!("Rustle data removal failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {}

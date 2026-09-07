//! User-invoked CLI for exporting Rustle's privacy-safe diagnostic bundle.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let destination = match arguments.next() {
        Some(argument) if argument == "--help" || argument == "-h" => {
            println!("Usage: rustle-diagnostics [OUTPUT.zip]");
            return;
        }
        Some(path) => PathBuf::from(path),
        None => default_destination(),
    };
    if arguments.next().is_some() {
        eprintln!("rustle-diagnostics accepts at most one output path");
        std::process::exit(2);
    }

    match rustle::diagnostics::export(&destination) {
        Ok(summary) => {
            println!(
                "Created {} with {} diagnostic files ({} bytes, {} truncated, {} omitted)",
                summary.destination.display(),
                summary.included_files,
                summary.included_bytes,
                summary.truncated_files,
                summary.omitted_files
            );
        }
        Err(error) => {
            eprintln!("Rustle diagnostic export failed: {error}");
            std::process::exit(1);
        }
    }
}

fn default_destination() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    PathBuf::from(format!("rustle-diagnostics-{timestamp}.zip"))
}

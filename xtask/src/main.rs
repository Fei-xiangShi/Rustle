use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

type XtaskResult<T> = Result<T, Box<dyn Error>>;

const SUPPLY_CHAIN_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
];
const FORBIDDEN_PRODUCTION_PACKAGES: &[&str] = &["iced_beacon", "iced_devtools"];
const RELEASE_METADATA_SCHEMA_VERSION: u32 = 1;
const RELEASE_MANIFEST_SCHEMA_VERSION: u32 = 1;
const RELEASE_PACKAGE: &str = "rustle";
const RELEASE_METADATA_FILE: &str = "release-metadata.json";
const RELEASE_MANIFEST_FILE: &str = "release-manifest.json";
const RELEASE_CHECKSUMS_FILE: &str = "SHA256SUMS.txt";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReleaseMetadata {
    schema_version: u32,
    package: String,
    version: String,
    tag: Option<String>,
    channel: String,
    commit: String,
    commit_timestamp: String,
    toolchain: String,
    cargo_lock_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReleaseArtifact {
    name: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReleaseManifest {
    schema_version: u32,
    package: String,
    version: String,
    tag: Option<String>,
    channel: String,
    commit: String,
    artifacts: Vec<ReleaseArtifact>,
}

struct SourceContractRule {
    path: &'static str,
    forbidden: &'static str,
    rationale: &'static str,
}

const ERROR_SOURCE_CONTRACTS: &[SourceContractRule] = &[
    SourceContractRule {
        path: "crates/rustle-audio/src/player.rs",
        forbidden: "classify_playback_error",
        rationale: "playback failures must be classified by the producer, never by parsing text",
    },
    SourceContractRule {
        path: "apps/rustle-desktop/src/app/update/song_resolver.rs",
        forbidden: "Result<ResolvedAudioSource, String>",
        rationale: "audio-source resolution is a stable typed application boundary",
    },
    SourceContractRule {
        path: "apps/rustle-desktop/src/app/update/song_resolver.rs",
        forbidden: "Result<ResolvedSong, String>",
        rationale: "song resolution must preserve stable codes and source chains",
    },
    SourceContractRule {
        path: "crates/rustle-platform/src/global_hotkeys.rs",
        forbidden: "Result<(), String>",
        rationale: "native registration and rollback failures require typed semantics",
    },
    SourceContractRule {
        path: "crates/rustle-platform/src/global_hotkeys.rs",
        forbidden: "GlobalHotkeyError::new",
        rationale: "platform errors must select an explicit typed kind",
    },
    SourceContractRule {
        path: "apps/rustle-desktop/src/app/message.rs",
        forbidden: "DatabaseError(String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "apps/rustle-desktop/src/app/message.rs",
        forbidden: "SongResolveFailed(PlaybackContext, String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "apps/rustle-desktop/src/app/message.rs",
        forbidden: "DownloadError(i64, String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "apps/rustle-desktop/src/app/message.rs",
        forbidden: "LyricsLoadFailed(i64, String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "apps/rustle-desktop/src/app/message.rs",
        forbidden: "NcmPlaylistLoadFailed(u64, i64, String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "crates/rustle-audio/src/events.rs",
        forbidden: "error: String",
        rationale: "audio events must transport PlaybackError without a parallel text classifier",
    },
    SourceContractRule {
        path: "crates/rustle-audio/src/streaming.rs",
        forbidden: "Error(String)",
        rationale: "streaming terminal events must retain PlaybackError",
    },
    SourceContractRule {
        path: "crates/rustle-audio/src/streaming.rs",
        forbidden: "    Failed(String),",
        rationale: "streaming health must retain PlaybackError",
    },
    SourceContractRule {
        path: "crates/rustle-audio/src/streaming.rs",
        forbidden: "Fatal(String)",
        rationale: "range failures must be classified at production",
    },
    SourceContractRule {
        path: "crates/rustle-platform/src/protocol/ipc.rs",
        forbidden: "Result<(), String>",
        rationale: "single-instance forwarding failures require typed semantics",
    },
];

fn main() {
    if let Err(error) = run() {
        eprintln!("xtask error: {error}");
        std::process::exit(1);
    }
}

fn run() -> XtaskResult<()> {
    let root = workspace_root()?;
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("check") => {
            reject_extra_args(args)?;
            check(&root)
        }
        Some("check-native") => {
            reject_extra_args(args)?;
            check_native(&root)
        }
        Some("check-production") => {
            reject_extra_args(args)?;
            check_production(&root)
        }
        Some("supply-chain") => {
            reject_extra_args(args)?;
            supply_chain(&root)
        }
        Some("metadata") => {
            reject_extra_args(args)?;
            print_metadata(&root)
        }
        Some("release-preflight") => release_preflight(&root, args),
        Some("release-manifest") => release_manifest(&root, args),
        Some("help" | "-h" | "--help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => Err(failure(format!("unknown command `{other}`\n\n{}", usage()))),
    }
}

fn workspace_root() -> XtaskResult<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| failure("xtask manifest has no workspace parent"))
}

fn check(root: &Path) -> XtaskResult<()> {
    verify_toolchain(root)?;
    verify_architecture_source_contracts(root)?;
    verify_architecture_dependency_graph(root)?;
    verify_error_source_contracts(root)?;
    verify_observability_source_contract(root)?;
    verify_panic_boundary_contracts(root)?;
    run_cargo(root, &["fmt", "--all", "--check"])?;
    check_production_workspace(root)?;
    check_workspace(root)?;
    clippy_workspace(root)?;
    test_workspace(root)?;
    doc_workspace(root)
}

fn check_production(root: &Path) -> XtaskResult<()> {
    verify_toolchain(root)?;
    verify_architecture_source_contracts(root)?;
    verify_architecture_dependency_graph(root)?;
    verify_error_source_contracts(root)?;
    verify_observability_source_contract(root)?;
    verify_panic_boundary_contracts(root)?;
    check_production_workspace(root)
}

fn check_production_workspace(root: &Path) -> XtaskResult<()> {
    run_cargo(root, &["check", "--locked", "--workspace", "--all-targets"])?;
    verify_production_dependency_graph(root)
}

fn verify_production_dependency_graph(root: &Path) -> XtaskResult<()> {
    let tree = capture_cargo(
        root,
        &["tree", "--locked", "--workspace", "--edges", "normal"],
    )?;
    let forbidden = forbidden_packages_in_tree(&tree);
    if forbidden.is_empty() {
        println!("production dependency graph: ok");
        Ok(())
    } else {
        Err(failure(format!(
            "production dependency graph contains forbidden debug/devtools packages: {}",
            forbidden.join(", ")
        )))
    }
}

fn forbidden_packages_in_tree(tree: &str) -> Vec<&'static str> {
    FORBIDDEN_PRODUCTION_PACKAGES
        .iter()
        .copied()
        .filter(|package| {
            tree.lines()
                .any(|line| line.contains(&format!("{package} v")))
        })
        .collect()
}

fn verify_error_source_contracts(root: &Path) -> XtaskResult<()> {
    let mut violations = Vec::new();
    for rule in ERROR_SOURCE_CONTRACTS {
        let path = root.join(rule.path);
        let contents = fs::read_to_string(&path)?;
        violations.extend(source_contract_violations(rule.path, &contents));
    }

    violations.sort();
    violations.dedup();
    if violations.is_empty() {
        println!("typed error source contracts: ok");
        Ok(())
    } else {
        Err(failure(format!(
            "typed error source contract violations:\n{}",
            violations.join("\n")
        )))
    }
}

fn source_contract_violations(path: &str, contents: &str) -> Vec<String> {
    ERROR_SOURCE_CONTRACTS
        .iter()
        .filter(|rule| rule.path == path)
        .flat_map(|rule| {
            contents
                .lines()
                .enumerate()
                .filter(move |(_, line)| line.contains(rule.forbidden))
                .map(move |(line_index, _)| {
                    format!(
                        "{}:{} contains forbidden `{}`: {}",
                        rule.path,
                        line_index + 1,
                        rule.forbidden,
                        rule.rationale
                    )
                })
        })
        .collect()
}

fn verify_observability_source_contract(root: &Path) -> XtaskResult<()> {
    let mut source_paths = Vec::new();
    for source_root in [root.join("apps"), root.join("src"), root.join("crates")] {
        if source_root.is_dir() {
            collect_rust_source_paths(&source_root, &mut source_paths)?;
        }
    }
    source_paths.sort();

    let mut violations = Vec::new();
    for path in source_paths {
        let relative_path = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read_to_string(&path)?;
        violations.extend(observability_source_contract_violations(
            &relative_path,
            &contents,
        ));
    }

    if violations.is_empty() {
        println!("observability source contract: ok");
        Ok(())
    } else {
        Err(failure(format!(
            "observability source contract violations:\n{}",
            violations.join("\n")
        )))
    }
}

fn collect_rust_source_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_rust_source_paths(&path, paths)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
    Ok(())
}

fn observability_source_contract_violations(path: &str, contents: &str) -> Vec<String> {
    if path == "crates/rustle-observability/src/lib.rs" {
        return Vec::new();
    }

    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("tracing_subscriber"))
        .map(|(line_index, _)| {
            format!(
                "{path}:{} references `tracing_subscriber`; subscriber configuration belongs to crates/rustle-observability/src/lib.rs",
                line_index + 1
            )
        })
        .collect()
}

fn verify_panic_boundary_contracts(root: &Path) -> XtaskResult<()> {
    let mut source_paths = Vec::new();
    for source_root in [root.join("apps"), root.join("src"), root.join("crates")] {
        if source_root.is_dir() {
            collect_rust_source_paths(&source_root, &mut source_paths)?;
        }
    }
    source_paths.sort();

    let mut violations = Vec::new();
    for path in source_paths {
        let relative_path = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read_to_string(&path)?;
        violations.extend(ffi_panic_boundary_violations(&relative_path, &contents));
    }
    violations.extend(release_panic_contract_violations(&fs::read_to_string(
        root.join("Cargo.toml"),
    )?));

    if violations.is_empty() {
        println!("panic boundary source contracts: ok");
        Ok(())
    } else {
        Err(failure(format!(
            "panic boundary source contract violations:\n{}",
            violations.join("\n")
        )))
    }
}

fn ffi_panic_boundary_violations(path: &str, contents: &str) -> Vec<String> {
    let mut callbacks = contents
        .match_indices("unsafe extern \"system\" fn")
        .chain(contents.match_indices("unsafe extern \"C\" fn"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    callbacks.sort_unstable();

    let mut violations = Vec::new();
    for (callback_index, start) in callbacks.iter().copied().enumerate() {
        let end = callbacks
            .get(callback_index + 1)
            .copied()
            .unwrap_or(contents.len());
        let callback = &contents[start..end];
        if !callback.contains("crate::runtime::catch_ffi_unwind(") {
            let line = contents[..start].lines().count() + 1;
            violations.push(format!(
                "{path}:{line} owns a Rust FFI callback without `crate::runtime::catch_ffi_unwind`"
            ));
        }
    }
    violations
}

fn release_panic_contract_violations(contents: &str) -> Vec<String> {
    let mut in_release_profile = false;
    let mut settings = Vec::new();
    for line in contents.lines() {
        let line = line.split_once('#').map_or(line, |(value, _)| value).trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_release_profile = line == "[profile.release]";
            continue;
        }
        if !in_release_profile {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "panic" {
            settings.push(value.trim());
        }
    }

    match settings.as_slice() {
        ["\"unwind\""] => Vec::new(),
        ["\"abort\""] => {
            vec!["Cargo.toml restores forbidden release `panic = \"abort\"`".to_string()]
        }
        _ => vec![
            "Cargo.toml [profile.release] must declare exactly one `panic = \"unwind\"`"
                .to_string(),
        ],
    }
}

fn check_native(root: &Path) -> XtaskResult<()> {
    verify_toolchain(root)?;
    verify_architecture_source_contracts(root)?;
    verify_architecture_dependency_graph(root)?;
    verify_error_source_contracts(root)?;
    verify_observability_source_contract(root)?;
    verify_panic_boundary_contracts(root)?;
    check_workspace(root)?;
    clippy_workspace(root)?;
    test_workspace(root)
}

fn verify_architecture_source_contracts(root: &Path) -> XtaskResult<()> {
    let mut source_paths = Vec::new();
    for source_root in [root.join("apps"), root.join("src"), root.join("crates")] {
        if source_root.is_dir() {
            collect_rust_source_paths(&source_root, &mut source_paths)?;
        }
    }
    source_paths.sort();

    let mut violations = Vec::new();
    for path in source_paths {
        let relative_path = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read_to_string(&path)?;
        violations.extend(architecture_source_contract_violations(
            &relative_path,
            &contents,
        ));
    }

    if violations.is_empty() {
        println!("architecture source contracts: ok");
        Ok(())
    } else {
        Err(failure(format!(
            "architecture source contract violations:\n{}",
            violations.join("\n")
        )))
    }
}

fn verify_architecture_dependency_graph(root: &Path) -> XtaskResult<()> {
    let metadata = load_metadata(root)?;
    let violations = architecture_dependency_violations(&metadata)?;

    if violations.is_empty() {
        println!("architecture dependency graph: ok");
        Ok(())
    } else {
        Err(failure(format!(
            "architecture dependency graph violations:\n{}",
            violations.join("\n")
        )))
    }
}

fn architecture_dependency_violations(metadata: &Value) -> XtaskResult<Vec<String>> {
    let packages = metadata_array(metadata, "packages")?;
    let workspace_members = metadata_array(metadata, "workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let mut violations = Vec::new();

    for package_name in [
        "rustle-domain",
        "rustle-application",
        "rustle-audio",
        "rustle-cleanup",
        "rustle-media",
        "rustle-observability",
        "rustle-platform",
        "rustle-storage",
        "rustle-ncm",
        "rustle-ui",
    ] {
        let Some(package) = packages
            .iter()
            .find(|package| package.get("name").and_then(Value::as_str) == Some(package_name))
        else {
            violations.push(format!("workspace package `{package_name}` is missing"));
            continue;
        };

        let package_id = metadata_string(package, "id")?;
        if !workspace_members.contains(&package_id) {
            violations.push(format!(
                "package `{package_name}` is not a workspace member"
            ));
        }

        let dependencies = metadata_array(package, "dependencies")?
            .iter()
            .filter_map(|dependency| dependency.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        let allowed: &[&str] = match package_name {
            "rustle-domain" => &["regex", "serde", "serde_json"],
            "rustle-application" => &["rustle-domain"],
            "rustle-audio" => &[
                "parking_lot",
                "reqwest",
                "rodio",
                "rustle-application",
                "rustle-domain",
                "serde",
                "serde_json",
                "spectrum-analyzer",
                "tokio",
                "tracing",
            ],
            "rustle-cleanup" => &[],
            "rustle-media" => &[
                "encoding_rs",
                "image",
                "lofty",
                "notify",
                "quick-xml",
                "rayon",
                "rodio",
                "rustle-application",
                "rustle-domain",
                "thiserror",
                "tokio",
                "tracing",
                "walkdir",
                "xxhash-rust",
            ],
            "rustle-platform" => &[
                "cosmic-text",
                "discord-rich-presence",
                "global-hotkey",
                "iced",
                "image",
                "interprocess",
                "ksni",
                "mpris-server",
                "objc2",
                "objc2-foundation",
                "rustle-application",
                "rustle-domain",
                "souvlaki",
                "thiserror",
                "tokio",
                "tracing",
                "tray-icon",
                "windows-sys",
                "x11rb",
            ],
            "rustle-observability" => &[
                "directories",
                "serde",
                "serde_json",
                "thiserror",
                "tracing",
                "tracing-appender",
                "tracing-subscriber",
                "zip",
            ],
            "rustle-storage" => &[
                "anyhow",
                "directories",
                "fs4",
                "futures-util",
                "rustle-application",
                "rustle-domain",
                "serde",
                "serde_json",
                "sqlx",
                "thiserror",
                "tokio",
                "tracing",
                "windows-sys",
                "xxhash-rust",
            ],
            "rustle-ncm" => &[
                "directories",
                "futures-util",
                "ncm-api-rs",
                "parking_lot",
                "qrcode-generator",
                "reqwest",
                "rustle-application",
                "rustle-domain",
                "serde",
                "serde_json",
                "thiserror",
                "tracing",
            ],
            "rustle-ui" => &[
                "bytemuck",
                "iced",
                "iced_runtime",
                "image",
                "rand",
                "tracing",
            ],
            _ => unreachable!(),
        };

        for dependency in dependencies {
            if !allowed.contains(&dependency) {
                violations.push(format!(
                    "package `{package_name}` has forbidden direct dependency `{dependency}`"
                ));
            }
        }

        if package_name == "rustle-application"
            && !metadata_array(package, "dependencies")?
                .iter()
                .any(|dependency| {
                    dependency.get("name").and_then(Value::as_str) == Some("rustle-domain")
                })
        {
            violations
                .push("package `rustle-application` must depend on `rustle-domain`".to_string());
        }
        if matches!(
            package_name,
            "rustle-audio" | "rustle-media" | "rustle-platform" | "rustle-storage"
        ) {
            for required_dependency in ["rustle-domain", "rustle-application"] {
                if !metadata_array(package, "dependencies")?
                    .iter()
                    .any(|dependency| {
                        dependency.get("name").and_then(Value::as_str) == Some(required_dependency)
                    })
                {
                    violations.push(format!(
                        "package `{package_name}` must depend on `{required_dependency}`"
                    ));
                }
            }
        }
        if package_name == "rustle-ncm" {
            for required_dependency in ["rustle-domain", "rustle-application"] {
                if !metadata_array(package, "dependencies")?
                    .iter()
                    .any(|dependency| {
                        dependency.get("name").and_then(Value::as_str) == Some(required_dependency)
                    })
                {
                    violations.push(format!(
                        "package `rustle-ncm` must depend on `{required_dependency}`"
                    ));
                }
            }
        }
    }

    let Some(root_package) = packages
        .iter()
        .find(|package| package.get("name").and_then(Value::as_str) == Some("rustle"))
    else {
        violations.push("workspace package `rustle` is missing".to_string());
        return Ok(violations);
    };
    let root_dependencies = metadata_array(root_package, "dependencies")?;
    for dependency in [
        "rustle-domain",
        "rustle-application",
        "rustle-audio",
        "rustle-media",
        "rustle-observability",
        "rustle-platform",
        "rustle-storage",
        "rustle-ncm",
        "rustle-ui",
    ] {
        if !root_dependencies
            .iter()
            .any(|candidate| candidate.get("name").and_then(Value::as_str) == Some(dependency))
        {
            violations.push(format!(
                "root package `rustle` must depend on `{dependency}`"
            ));
        }
    }

    Ok(violations)
}

fn architecture_source_contract_violations(path: &str, contents: &str) -> Vec<String> {
    let path = path.strip_prefix("apps/rustle-desktop/").unwrap_or(path);
    let mut forbidden = Vec::new();

    if path.starts_with("src/features/lyrics/") || path == "src/features/media/lyrics.rs" {
        forbidden.push((
            "crate::ui",
            "lyrics parsing/media must return domain models",
        ));
    }
    if path.starts_with("src/audio/") || path == "src/audio.rs" {
        forbidden.push(("crate::api", "audio must not depend on the NCM adapter"));
        forbidden.push((
            "crate::cache",
            "audio must consume the application cache port",
        ));
    }
    if path.starts_with("src/domain/")
        || path == "src/domain.rs"
        || path.starts_with("crates/rustle-domain/src/")
    {
        forbidden.extend([
            ("iced::", "domain must be UI-framework free"),
            ("tokio::", "domain must be runtime free"),
            ("sqlx::", "domain must be persistence-adapter free"),
            ("reqwest::", "domain must be transport-adapter free"),
            ("rodio::", "domain must be audio-backend free"),
            ("crate::application::", "domain must not depend outward"),
            ("crate::platform", "domain must not depend outward"),
            ("crate::api", "domain must not depend outward"),
            ("crate::cache", "domain must not depend outward"),
            ("crate::database", "domain must not depend outward"),
            ("crate::ui", "domain must not depend outward"),
            ("crate::app::", "domain must not depend outward"),
            ("rustle_application", "domain must not depend outward"),
        ]);
    }
    if path.starts_with("src/application/")
        || path == "src/application.rs"
        || path.starts_with("crates/rustle-application/src/")
    {
        forbidden.extend([
            ("iced::", "application contracts must be UI-framework free"),
            (
                "sqlx::",
                "application contracts must not own storage adapters",
            ),
            (
                "reqwest::",
                "application contracts must not own network adapters",
            ),
            (
                "rodio::",
                "application contracts must not own audio backends",
            ),
            (
                "crate::platform",
                "application must not depend on concrete adapters",
            ),
            (
                "crate::api",
                "application must not depend on concrete adapters",
            ),
            (
                "crate::cache",
                "application must not depend on concrete adapters",
            ),
            (
                "crate::database",
                "application must not depend on concrete adapters",
            ),
            (
                "crate::ui",
                "application must not depend on concrete adapters",
            ),
            (
                "crate::app::",
                "application must not depend on the composition root",
            ),
            (
                "rustle_storage",
                "application must not depend on concrete adapters",
            ),
            (
                "rustle_ncm",
                "application must not depend on concrete adapters",
            ),
            (
                "rustle_audio",
                "application must not depend on concrete adapters",
            ),
            (
                "rustle_platform",
                "application must not depend on concrete adapters",
            ),
            (
                "rustle_ui",
                "application must not depend on concrete adapters",
            ),
        ]);
    }
    if path.starts_with("crates/rustle-storage/src/") {
        forbidden.extend([
            ("iced::", "storage must be UI-framework free"),
            ("reqwest::", "storage must not own network transport"),
            ("rodio::", "storage must not own audio backends"),
            (
                "ncm_api_rs",
                "storage must not depend on NCM protocol types",
            ),
            ("crate::api", "storage must not depend on the NCM adapter"),
            (
                "crate::features",
                "storage must not depend on root features",
            ),
            (
                "crate::audio",
                "storage must not depend on the audio adapter",
            ),
            (
                "crate::platform",
                "storage must not depend on platform adapters",
            ),
            ("crate::ui", "storage must not depend on UI adapters"),
            (
                "crate::app::",
                "storage must not depend on the composition root",
            ),
            (
                "crate::error",
                "storage must use application error contracts directly",
            ),
            ("rustle_ncm", "storage must not depend on sibling adapters"),
            (
                "rustle_media",
                "storage must not depend on sibling adapters",
            ),
            (
                "rustle_audio",
                "storage must not depend on sibling adapters",
            ),
            (
                "rustle_platform",
                "storage must not depend on sibling adapters",
            ),
            (
                "rustle_observability",
                "storage must not depend on sibling adapters",
            ),
            ("rustle_ui", "storage must not depend on sibling adapters"),
        ]);
    }
    if path.starts_with("crates/rustle-ncm/src/") {
        forbidden.extend([
            ("iced::", "NCM must be UI-framework free"),
            ("sqlx::", "NCM must not own database adapters"),
            ("rodio::", "NCM must not own audio backends"),
            ("crate::api", "NCM must not depend on the root API facade"),
            ("crate::features", "NCM must not depend on root features"),
            ("crate::audio", "NCM must not depend on the audio adapter"),
            (
                "crate::platform",
                "NCM must not depend on platform adapters",
            ),
            ("crate::ui", "NCM must not depend on UI adapters"),
            (
                "crate::app::",
                "NCM must not depend on the composition root",
            ),
            (
                "crate::error",
                "NCM must use application error contracts directly",
            ),
            ("crate::domain", "NCM must use domain contracts directly"),
            ("rustle_storage", "NCM must not depend on sibling adapters"),
            ("rustle_media", "NCM must not depend on sibling adapters"),
            ("rustle_audio", "NCM must not depend on sibling adapters"),
            ("rustle_platform", "NCM must not depend on sibling adapters"),
            (
                "rustle_observability",
                "NCM must not depend on sibling adapters",
            ),
            ("rustle_ui", "NCM must not depend on sibling adapters"),
        ]);
    }
    if path.starts_with("crates/rustle-media/src/") {
        forbidden.extend([
            ("iced::", "media must be UI-framework free"),
            ("sqlx::", "media must not own database adapters"),
            ("reqwest::", "media must not own network transport"),
            ("ncm_api_rs", "media must not own NCM protocol types"),
            ("crate::features", "media must not depend on root features"),
            (
                "crate::database",
                "media must not depend on root storage facades",
            ),
            ("crate::api", "media must not depend on the root NCM facade"),
            ("crate::audio", "media must not depend on audio playback"),
            (
                "crate::platform",
                "media must not depend on platform adapters",
            ),
            ("crate::ui", "media must not depend on UI adapters"),
            (
                "crate::app::",
                "media must not depend on the composition root",
            ),
            ("crate::domain", "media must use domain contracts directly"),
            (
                "rustle_storage",
                "media must not depend on sibling adapters",
            ),
            ("rustle_ncm", "media must not depend on sibling adapters"),
            ("rustle_audio", "media must not depend on sibling adapters"),
            (
                "rustle_platform",
                "media must not depend on sibling adapters",
            ),
            (
                "rustle_observability",
                "media must not depend on sibling adapters",
            ),
            ("rustle_ui", "media must not depend on sibling adapters"),
        ]);
    }
    if path.starts_with("crates/rustle-audio/src/") {
        forbidden.extend([
            ("iced::", "audio must be UI-framework free"),
            ("sqlx::", "audio must not own database adapters"),
            ("ncm_api_rs", "audio must not own NCM protocol types"),
            ("crate::api", "audio must not depend on the root NCM facade"),
            ("crate::cache", "audio must consume application cache ports"),
            (
                "crate::database",
                "audio must not depend on root storage facades",
            ),
            ("crate::features", "audio must not depend on root features"),
            (
                "crate::platform",
                "audio must not depend on platform adapters",
            ),
            ("crate::ui", "audio must not depend on UI adapters"),
            (
                "crate::app::",
                "audio must not depend on the composition root",
            ),
            ("crate::runtime", "audio worker spawning must be injected"),
            ("crate::error", "audio must use application errors directly"),
            ("crate::domain", "audio must use domain contracts directly"),
            (
                "crate::application",
                "audio must use application contracts directly",
            ),
            (
                "rustle_storage",
                "audio must not depend on sibling adapters",
            ),
            ("rustle_ncm", "audio must not depend on sibling adapters"),
            ("rustle_media", "audio must not depend on sibling adapters"),
            (
                "rustle_platform",
                "audio must not depend on sibling adapters",
            ),
            (
                "rustle_observability",
                "audio must not depend on sibling adapters",
            ),
            ("rustle_ui", "audio must not depend on sibling adapters"),
        ]);
    }
    if path.starts_with("crates/rustle-platform/src/") {
        forbidden.extend([
            (
                "anyhow",
                "platform public and internal boundaries must use typed errors",
            ),
            (
                "Result<(), String>",
                "platform boundaries must not return bare strings",
            ),
            ("crate::app", "platform adapters must emit pure commands"),
            (
                "crate::i18n",
                "platform adapters must consume localized presentation",
            ),
            (
                "crate::features",
                "platform adapters must use domain/application contracts directly",
            ),
            (
                "crate::domain",
                "platform must use domain contracts directly",
            ),
            (
                "crate::application",
                "platform must use application contracts directly",
            ),
            (
                "crate::error",
                "platform must use application errors directly",
            ),
            (
                "crate::platform",
                "platform must not depend on a root facade",
            ),
            ("crate::audio", "platform must not depend on audio adapters"),
            ("crate::api", "platform must not depend on NCM adapters"),
            (
                "crate::cache",
                "platform must not depend on storage facades",
            ),
            (
                "crate::database",
                "platform must not depend on storage facades",
            ),
            (
                "rustle_audio",
                "platform must not depend on sibling adapters",
            ),
            (
                "rustle_media",
                "platform must not depend on sibling adapters",
            ),
            ("rustle_ncm", "platform must not depend on sibling adapters"),
            (
                "rustle_storage",
                "platform must not depend on sibling adapters",
            ),
            (
                "rustle_observability",
                "platform panic containment must be injected",
            ),
            ("rustle_ui", "platform must not depend on UI adapters"),
            (
                "tracing_subscriber",
                "platform must not initialize observability",
            ),
        ]);
    }
    if path.starts_with("crates/rustle-observability/src/") {
        forbidden.extend([
            ("iced::", "observability must be UI-framework free"),
            (
                "crate::app",
                "observability must not depend on the composition root",
            ),
            ("crate::ui", "observability must not depend on UI adapters"),
            ("rustle_domain", "observability must not depend on domain"),
            (
                "rustle_application",
                "observability must not depend on application contracts",
            ),
            (
                "rustle_audio",
                "observability must not depend on audio adapters",
            ),
            (
                "rustle_media",
                "observability must not depend on media adapters",
            ),
            (
                "rustle_ncm",
                "observability must not depend on NCM adapters",
            ),
            (
                "rustle_platform",
                "observability must not depend on platform adapters",
            ),
            (
                "rustle_storage",
                "observability must not depend on storage adapters",
            ),
            ("rustle_ui", "observability must not depend on UI adapters"),
        ]);
    }
    if path.starts_with("crates/rustle-ui/src/") {
        forbidden.extend([
            (
                "crate::app",
                "UI infrastructure must not depend on the desktop app",
            ),
            (
                "crate::features",
                "UI infrastructure must not depend on desktop feature owners",
            ),
            (
                "crate::platform",
                "UI infrastructure must not depend on platform adapters",
            ),
            (
                "rustle_domain",
                "UI infrastructure must remain business-neutral",
            ),
            (
                "rustle_application",
                "UI infrastructure must remain presentation infrastructure",
            ),
            (
                "rustle_audio",
                "UI infrastructure must not depend on audio adapters",
            ),
            (
                "rustle_media",
                "UI infrastructure must not depend on media adapters",
            ),
            (
                "rustle_ncm",
                "UI infrastructure must not depend on NCM adapters",
            ),
            (
                "rustle_observability",
                "UI infrastructure must not initialize observability",
            ),
            (
                "rustle_platform",
                "UI infrastructure must not depend on platform adapters",
            ),
            (
                "rustle_storage",
                "UI infrastructure must not depend on storage adapters",
            ),
        ]);
    }

    forbidden
        .into_iter()
        .flat_map(|(token, rationale)| {
            contents
                .lines()
                .enumerate()
                .filter(move |(_, line)| line.contains(token))
                .map(move |(line_index, _)| {
                    format!(
                        "{path}:{} contains forbidden `{token}`: {rationale}",
                        line_index + 1
                    )
                })
        })
        .collect()
}

fn check_workspace(root: &Path) -> XtaskResult<()> {
    run_cargo(
        root,
        &[
            "check",
            "--locked",
            "--workspace",
            "--all-targets",
            "--all-features",
        ],
    )
}

fn clippy_workspace(root: &Path) -> XtaskResult<()> {
    run_cargo(
        root,
        &[
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    )
}

fn test_workspace(root: &Path) -> XtaskResult<()> {
    run_cargo(
        root,
        &[
            "test",
            "--locked",
            "--workspace",
            "--all-targets",
            "--all-features",
        ],
    )
}

fn doc_workspace(root: &Path) -> XtaskResult<()> {
    let args = [
        "doc",
        "--locked",
        "--workspace",
        "--all-features",
        "--no-deps",
    ];
    eprintln!("+ RUSTDOCFLAGS=-D warnings cargo {}", args.join(" "));
    let status = cargo(root)
        .env("RUSTDOCFLAGS", "-D warnings")
        .args(args)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "`RUSTDOCFLAGS=-D warnings cargo {}` failed with {status}",
            args.join(" ")
        )))
    }
}

fn supply_chain(root: &Path) -> XtaskResult<()> {
    verify_toolchain(root)?;
    verify_manifest_git_revisions(root)?;
    let expected = pinned_tool_version(root, ".cargo-deny-version", "cargo-deny")?;

    let output = capture_cargo(root, &["deny", "--version"])?;
    let actual = tool_version(&output, "cargo-deny").ok_or_else(|| {
        failure(format!(
            "unexpected cargo-deny version output `{output}`; install {expected} with `cargo install cargo-deny --version {expected} --locked`"
        ))
    })?;
    if actual != expected {
        return Err(failure(format!(
            "cargo-deny version `{actual}` does not match pinned version `{expected}`; install it with `cargo install cargo-deny --version {expected} --locked --force`"
        )));
    }

    let machete_expected = pinned_tool_version(root, ".cargo-machete-version", "cargo-machete")?;
    let machete_output = capture_program(root, "cargo-machete", &["--version"])?;
    if machete_output != machete_expected {
        return Err(failure(format!(
            "cargo-machete version `{machete_output}` does not match pinned version `{machete_expected}`; install it with `cargo install cargo-machete --version {machete_expected} --locked --force`"
        )));
    }
    run_program(root, "cargo-machete", &["--skip-target-dir"])?;

    for target in SUPPLY_CHAIN_TARGETS {
        println!("supply-chain target: {target}");
        run_cargo(
            root,
            &[
                "deny",
                "--locked",
                "--workspace",
                "--all-features",
                "--target",
                target,
                "check",
                "--allow",
                "license-not-encountered",
                "--allow",
                "unmatched-source",
                "--allow",
                "unmatched-skip",
                "--allow",
                "unnecessary-skip",
                "--hide-inclusion-graph",
                "advisories",
                "licenses",
                "bans",
                "sources",
            ],
        )?;
    }
    Ok(())
}

fn pinned_tool_version(root: &Path, file_name: &str, tool: &str) -> XtaskResult<String> {
    let version_file = root.join(file_name);
    let version = fs::read_to_string(&version_file)?.trim().to_owned();
    if version.is_empty() {
        Err(failure(format!(
            "{tool} version file is empty: {}",
            version_file.display()
        )))
    } else {
        Ok(version)
    }
}

fn verify_manifest_git_revisions(root: &Path) -> XtaskResult<()> {
    let mut manifests = Vec::new();
    collect_manifests(root, &mut manifests)?;
    manifests.sort();

    for manifest in manifests {
        let contents = fs::read_to_string(&manifest)?;
        for (line_index, line) in contents.lines().enumerate() {
            if inline_quoted_setting(line, "git").is_none() {
                continue;
            }
            let revision = inline_quoted_setting(line, "rev").filter(|revision| {
                revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
            if revision.is_none()
                || inline_quoted_setting(line, "branch").is_some()
                || inline_quoted_setting(line, "tag").is_some()
            {
                return Err(failure(format!(
                    "{}:{} git dependencies and patches must use one inline table with a full 40-character `rev` and no branch/tag selector",
                    manifest.display(),
                    line_index + 1
                )));
            }
        }
    }

    Ok(())
}

fn collect_manifests(directory: &Path, manifests: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let name = entry.file_name();
            if matches!(name.to_str(), Some(".git" | ".trellis" | "target")) {
                continue;
            }
            collect_manifests(&path, manifests)?;
        } else if file_type.is_file() && entry.file_name() == "Cargo.toml" {
            manifests.push(path);
        }
    }
    Ok(())
}

fn print_metadata(root: &Path) -> XtaskResult<()> {
    let toolchain = verify_toolchain(root)?;
    let metadata = load_metadata(root)?;

    println!("toolchain: {toolchain}");
    println!(
        "workspace root: {}",
        metadata_string(&metadata, "workspace_root")?
    );
    println!(
        "target directory: {}",
        metadata_string(&metadata, "target_directory")?
    );

    let members = metadata_array(&metadata, "workspace_members")?;
    let packages = metadata_array(&metadata, "packages")?;
    println!("workspace members: {}", members.len());

    for package in packages.iter().filter(|package| {
        package
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| members.iter().any(|member| member.as_str() == Some(id)))
    }) {
        let name = value_string(package, "name")?;
        let version = value_string(package, "version")?;
        let targets = metadata_array(package, "targets")?;
        let target_summary = targets
            .iter()
            .map(target_description)
            .collect::<XtaskResult<Vec<_>>>()?
            .join(", ");
        println!("- {name} {version}: {target_summary}");
    }

    Ok(())
}

struct ReleasePreflightArgs {
    commit: String,
    tag: Option<String>,
    output: PathBuf,
}

struct ReleaseManifestArgs {
    metadata: PathBuf,
    artifacts_dir: PathBuf,
    output: PathBuf,
    checksums_output: PathBuf,
}

fn release_preflight(root: &Path, args: impl Iterator<Item = String>) -> XtaskResult<()> {
    let args = parse_release_preflight_args(root, args)?;
    let toolchain = verify_toolchain(root)?;
    verify_production_dependency_graph(root)?;
    verify_required_lockfile(root)?;

    let metadata = build_release_metadata(root, &args.commit, args.tag.as_deref(), &toolchain)?;
    write_json_atomic(&args.output, &metadata)?;

    println!("release package: {}", metadata.package);
    println!("release version: {}", metadata.version);
    println!(
        "release tag: {}",
        metadata.tag.as_deref().unwrap_or("not supplied")
    );
    println!("release channel: {}", metadata.channel);
    println!("release commit: {}", metadata.commit);
    println!("release metadata: {}", args.output.display());
    println!("release preflight: ok");
    Ok(())
}

fn release_manifest(root: &Path, args: impl Iterator<Item = String>) -> XtaskResult<()> {
    let args = parse_release_manifest_args(root, args)?;
    let metadata: ReleaseMetadata = serde_json::from_slice(&fs::read(&args.metadata)?)?;
    validate_release_metadata_against_checkout(root, &metadata)?;

    let manifest = build_release_manifest(&metadata, &args.artifacts_dir)?;
    write_json_atomic(&args.output, &manifest)?;
    write_release_checksums(
        &manifest,
        &args.artifacts_dir,
        &args.metadata,
        &args.output,
        &args.checksums_output,
    )?;

    println!("release manifest: {}", args.output.display());
    println!("release checksums: {}", args.checksums_output.display());
    println!("release artifact count: {}", manifest.artifacts.len());
    println!("release manifest: ok");
    Ok(())
}

fn parse_release_preflight_args(
    root: &Path,
    mut args: impl Iterator<Item = String>,
) -> XtaskResult<ReleasePreflightArgs> {
    let mut commit = None;
    let mut tag = None;
    let mut output = None;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--commit" => set_once(&mut commit, "--commit", next_value(&mut args, "--commit")?)?,
            "--tag" => set_once(&mut tag, "--tag", next_value(&mut args, "--tag")?)?,
            "--output" => set_once(
                &mut output,
                "--output",
                resolve_cli_path(root, next_value(&mut args, "--output")?),
            )?,
            _ => {
                return Err(failure(format!(
                    "unexpected release-preflight argument `{flag}`"
                )));
            }
        }
    }

    Ok(ReleasePreflightArgs {
        commit: commit.ok_or_else(|| failure("release-preflight requires `--commit`"))?,
        tag,
        output: output.unwrap_or_else(|| root.join("target").join(RELEASE_METADATA_FILE)),
    })
}

fn parse_release_manifest_args(
    root: &Path,
    mut args: impl Iterator<Item = String>,
) -> XtaskResult<ReleaseManifestArgs> {
    let mut metadata = None;
    let mut artifacts_dir = None;
    let mut output = None;
    let mut checksums_output = None;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--metadata" => set_once(
                &mut metadata,
                "--metadata",
                resolve_cli_path(root, next_value(&mut args, "--metadata")?),
            )?,
            "--artifacts-dir" => set_once(
                &mut artifacts_dir,
                "--artifacts-dir",
                resolve_cli_path(root, next_value(&mut args, "--artifacts-dir")?),
            )?,
            "--output" => set_once(
                &mut output,
                "--output",
                resolve_cli_path(root, next_value(&mut args, "--output")?),
            )?,
            "--checksums-output" => set_once(
                &mut checksums_output,
                "--checksums-output",
                resolve_cli_path(root, next_value(&mut args, "--checksums-output")?),
            )?,
            _ => {
                return Err(failure(format!(
                    "unexpected release-manifest argument `{flag}`"
                )));
            }
        }
    }

    Ok(ReleaseManifestArgs {
        metadata: metadata.ok_or_else(|| failure("release-manifest requires `--metadata`"))?,
        artifacts_dir: artifacts_dir
            .ok_or_else(|| failure("release-manifest requires `--artifacts-dir`"))?,
        output: output.unwrap_or_else(|| root.join("target").join(RELEASE_MANIFEST_FILE)),
        checksums_output: checksums_output
            .unwrap_or_else(|| root.join("target").join(RELEASE_CHECKSUMS_FILE)),
    })
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> XtaskResult<String> {
    args.next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| failure(format!("`{flag}` requires a value")))
}

fn set_once<T>(slot: &mut Option<T>, flag: &str, value: T) -> XtaskResult<()> {
    if slot.replace(value).is_some() {
        Err(failure(format!("`{flag}` may be supplied only once")))
    } else {
        Ok(())
    }
}

fn resolve_cli_path(root: &Path, value: String) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn build_release_metadata(
    root: &Path,
    expected_commit: &str,
    tag: Option<&str>,
    toolchain: &str,
) -> XtaskResult<ReleaseMetadata> {
    validate_commit_sha(expected_commit)?;
    let actual_commit = capture_program(root, "git", &["rev-parse", "HEAD"])?;
    if actual_commit != expected_commit {
        return Err(failure(format!(
            "checked-out commit `{actual_commit}` does not match expected release commit `{expected_commit}`"
        )));
    }
    verify_tracked_worktree_clean(root)?;

    let cargo_metadata = load_metadata(root)?;
    let package_version = package_version(&cargo_metadata, RELEASE_PACKAGE)?;
    let version = Version::parse(package_version)?;
    if version.to_string() != package_version {
        return Err(failure(format!(
            "Cargo version `{package_version}` is not canonical semver"
        )));
    }
    if let Some(tag) = tag {
        let tag_commit = resolve_release_tag(root, tag)?;
        validate_release_tag_identity(tag, package_version, &tag_commit, expected_commit)?;
    }

    let commit_timestamp = capture_program(
        root,
        "git",
        &["show", "-s", "--format=%cI", expected_commit],
    )?;
    let cargo_lock_sha256 = sha256_file(&verify_required_lockfile(root)?)?;

    Ok(ReleaseMetadata {
        schema_version: RELEASE_METADATA_SCHEMA_VERSION,
        package: RELEASE_PACKAGE.to_owned(),
        version: package_version.to_owned(),
        tag: tag.map(str::to_owned),
        channel: release_channel(&version).to_owned(),
        commit: expected_commit.to_owned(),
        commit_timestamp,
        toolchain: toolchain.to_owned(),
        cargo_lock_sha256,
    })
}

fn validate_release_metadata_against_checkout(
    root: &Path,
    metadata: &ReleaseMetadata,
) -> XtaskResult<()> {
    if metadata.schema_version != RELEASE_METADATA_SCHEMA_VERSION {
        return Err(failure(format!(
            "unsupported release metadata schema {}; expected {}",
            metadata.schema_version, RELEASE_METADATA_SCHEMA_VERSION
        )));
    }
    if metadata.package != RELEASE_PACKAGE {
        return Err(failure(format!(
            "release metadata package `{}` is not `{RELEASE_PACKAGE}`",
            metadata.package
        )));
    }
    validate_commit_sha(&metadata.commit)?;
    let actual_commit = capture_program(root, "git", &["rev-parse", "HEAD"])?;
    if metadata.commit != actual_commit {
        return Err(failure(format!(
            "release metadata commit `{}` does not match checkout `{actual_commit}`",
            metadata.commit
        )));
    }
    verify_tracked_worktree_clean(root)?;

    let toolchain = verify_toolchain(root)?;
    if metadata.toolchain != toolchain {
        return Err(failure(format!(
            "release metadata toolchain `{}` does not match checkout `{toolchain}`",
            metadata.toolchain
        )));
    }

    let cargo_metadata = load_metadata(root)?;
    let package_version = package_version(&cargo_metadata, RELEASE_PACKAGE)?;
    if metadata.version != package_version {
        return Err(failure(format!(
            "release metadata version `{}` does not match Cargo version `{package_version}`",
            metadata.version
        )));
    }
    let version = Version::parse(package_version)?;
    let expected_channel = release_channel(&version);
    if metadata.channel != expected_channel {
        return Err(failure(format!(
            "release metadata channel `{}` does not match semver channel `{expected_channel}`",
            metadata.channel
        )));
    }

    if let Some(tag) = metadata.tag.as_deref() {
        let tag_commit = resolve_release_tag(root, tag)?;
        validate_release_tag_identity(tag, package_version, &tag_commit, &metadata.commit)?;
    }

    let commit_timestamp = capture_program(
        root,
        "git",
        &["show", "-s", "--format=%cI", &metadata.commit],
    )?;
    if metadata.commit_timestamp != commit_timestamp {
        return Err(failure(format!(
            "release metadata timestamp `{}` does not match checkout `{commit_timestamp}`",
            metadata.commit_timestamp
        )));
    }

    let lock_hash = sha256_file(&verify_required_lockfile(root)?)?;
    if metadata.cargo_lock_sha256 != lock_hash {
        return Err(failure(
            "release metadata Cargo.lock SHA-256 does not match checkout",
        ));
    }
    Ok(())
}

fn release_channel(version: &Version) -> &'static str {
    if version.pre.is_empty() {
        "stable"
    } else {
        "preview"
    }
}

fn validate_commit_sha(commit: &str) -> XtaskResult<()> {
    if commit.len() == 40
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(failure(format!(
            "release commit must be exactly 40 lowercase hexadecimal characters: `{commit}`"
        )))
    }
}

fn resolve_release_tag(root: &Path, tag: &str) -> XtaskResult<String> {
    let reference = format!("refs/tags/{tag}^{{commit}}");
    capture_program(root, "git", &["rev-parse", "--verify", &reference])
        .map_err(|_| failure(format!("release tag `{tag}` does not resolve to a commit")))
}

fn validate_release_tag_identity(
    tag: &str,
    package_version: &str,
    tag_commit: &str,
    expected_commit: &str,
) -> XtaskResult<()> {
    let version_text = tag
        .strip_prefix('v')
        .ok_or_else(|| failure(format!("release tag `{tag}` must start with `v`")))?;
    let parsed = Version::parse(version_text)?;
    let canonical = format!("v{parsed}");
    if tag != canonical {
        return Err(failure(format!(
            "release tag `{tag}` is not canonical; expected `{canonical}`"
        )));
    }
    if parsed.to_string() != package_version {
        return Err(failure(format!(
            "release tag `{tag}` does not match Cargo version `{package_version}`"
        )));
    }
    if tag_commit != expected_commit {
        return Err(failure(format!(
            "release tag `{tag}` resolves to `{tag_commit}`, expected `{expected_commit}`"
        )));
    }
    Ok(())
}

fn verify_tracked_worktree_clean(root: &Path) -> XtaskResult<()> {
    let status = capture_program(
        root,
        "git",
        &["status", "--porcelain", "--untracked-files=no"],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(failure(format!(
            "release checkout has tracked modifications:\n{status}"
        )))
    }
}

fn verify_required_lockfile(root: &Path) -> XtaskResult<PathBuf> {
    let lockfile = root.join("Cargo.lock");
    if lockfile.is_file() {
        Ok(lockfile)
    } else {
        Err(failure(format!(
            "required lockfile is missing: {}",
            lockfile.display()
        )))
    }
}

fn build_release_manifest(
    metadata: &ReleaseMetadata,
    artifacts_dir: &Path,
) -> XtaskResult<ReleaseManifest> {
    let files = collect_release_artifacts(artifacts_dir)?;
    let expected = expected_release_artifact_names(&metadata.version);
    let actual = files.keys().cloned().collect::<BTreeSet<_>>();
    if actual != expected {
        let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
        let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
        return Err(failure(format!(
            "release artifact set mismatch; missing=[{}], unexpected=[{}]",
            missing.join(", "),
            unexpected.join(", ")
        )));
    }

    let artifacts = files
        .into_iter()
        .map(|(name, path)| {
            Ok(ReleaseArtifact {
                name,
                size: fs::metadata(&path)?.len(),
                sha256: sha256_file(&path)?,
            })
        })
        .collect::<XtaskResult<Vec<_>>>()?;

    Ok(ReleaseManifest {
        schema_version: RELEASE_MANIFEST_SCHEMA_VERSION,
        package: metadata.package.clone(),
        version: metadata.version.clone(),
        tag: metadata.tag.clone(),
        channel: metadata.channel.clone(),
        commit: metadata.commit.clone(),
        artifacts,
    })
}

fn expected_release_artifact_names(version: &str) -> BTreeSet<String> {
    [
        "rustle-linux-x86_64.AppImage".to_owned(),
        "rustle-macos-arm64.dmg".to_owned(),
        "rustle-macos-x86_64.dmg".to_owned(),
        format!("rustle-{version}-windows-x86_64.msi"),
        "rustle-windows-x86_64.exe".to_owned(),
    ]
    .into_iter()
    .collect()
}

fn collect_release_artifacts(directory: &Path) -> XtaskResult<BTreeMap<String, PathBuf>> {
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(failure(format!(
            "release artifacts directory must be a real directory: {}",
            directory.display()
        )));
    }

    let mut files = BTreeMap::new();
    collect_release_artifacts_recursive(directory, &mut files)?;
    Ok(files)
}

fn collect_release_artifacts_recursive(
    directory: &Path,
    files: &mut BTreeMap<String, PathBuf>,
) -> XtaskResult<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(failure(format!(
                "release artifact tree contains a symlink: {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect_release_artifacts_recursive(&path, files)?;
        } else if file_type.is_file() {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| failure("release artifact basename is not valid UTF-8"))?;
            if let Some(previous) = files.insert(name.clone(), path.clone()) {
                return Err(failure(format!(
                    "duplicate release artifact basename `{name}`: {} and {}",
                    previous.display(),
                    path.display()
                )));
            }
        } else {
            return Err(failure(format!(
                "release artifact tree contains an unsupported file type: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn write_release_checksums(
    manifest: &ReleaseManifest,
    artifacts_dir: &Path,
    metadata_path: &Path,
    manifest_path: &Path,
    output: &Path,
) -> XtaskResult<()> {
    let files = collect_release_artifacts(artifacts_dir)?;
    let mut checksums = BTreeMap::new();
    for artifact in &manifest.artifacts {
        let path = files.get(&artifact.name).ok_or_else(|| {
            failure(format!(
                "release artifact disappeared before checksum publication: {}",
                artifact.name
            ))
        })?;
        let actual = sha256_file(path)?;
        if actual != artifact.sha256 {
            return Err(failure(format!(
                "release artifact changed before checksum publication: {}",
                artifact.name
            )));
        }
        checksums.insert(artifact.name.clone(), actual);
    }
    insert_checksum_file(&mut checksums, metadata_path)?;
    insert_checksum_file(&mut checksums, manifest_path)?;

    let contents = checksums
        .into_iter()
        .map(|(name, hash)| format!("{hash}  {name}\n"))
        .collect::<String>();
    write_atomic(output, contents.as_bytes())
}

fn insert_checksum_file(checksums: &mut BTreeMap<String, String>, path: &Path) -> XtaskResult<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            failure(format!(
                "checksum path has no UTF-8 basename: {}",
                path.display()
            ))
        })?
        .to_owned();
    if checksums.insert(name.clone(), sha256_file(path)?).is_some() {
        return Err(failure(format!("duplicate checksum basename `{name}`")));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> XtaskResult<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> XtaskResult<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, contents: &[u8]) -> XtaskResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| failure(format!("output path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent)?;
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            failure(format!(
                "output path has no UTF-8 basename: {}",
                path.display()
            ))
        })?;
    let temporary = parent.join(format!(
        ".{file_name}.tmp-{}-{sequence}",
        std::process::id()
    ));

    let result = (|| -> XtaskResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        replace_complete_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_complete_file(temporary: &Path, destination: &Path) -> XtaskResult<()> {
    fs::rename(temporary, destination)?;
    Ok(())
}

#[cfg(windows)]
fn replace_complete_file(temporary: &Path, destination: &Path) -> XtaskResult<()> {
    if !destination.exists() {
        fs::rename(temporary, destination)?;
        return Ok(());
    }

    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| failure("release output has no UTF-8 basename"))?;
    let backup = destination.with_file_name(format!(
        ".{file_name}.previous-{}-{sequence}",
        std::process::id()
    ));
    fs::rename(destination, &backup)?;
    if let Err(error) = fs::rename(temporary, destination) {
        let _ = fs::rename(&backup, destination);
        return Err(error.into());
    }
    fs::remove_file(backup)?;
    Ok(())
}

fn verify_toolchain(root: &Path) -> XtaskResult<String> {
    let toolchain_file = root.join("rust-toolchain.toml");
    let contents = fs::read_to_string(&toolchain_file)?;
    let pinned = quoted_setting(&contents, "channel")
        .ok_or_else(|| failure("rust-toolchain.toml has no quoted `channel`"))?;

    let rustc = capture_program(root, "rustc", &["--version"])?;
    let actual = rustc
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| failure(format!("unexpected rustc version output: {rustc}")))?;
    if actual != pinned {
        return Err(failure(format!(
            "rustc version `{actual}` does not match pinned toolchain `{pinned}`"
        )));
    }

    let _rustfmt = capture_program(root, "rustfmt", &["--version"])?;
    let _clippy = capture_cargo(root, &["clippy", "--version"])?;
    Ok(pinned.to_owned())
}

fn load_metadata(root: &Path) -> XtaskResult<Value> {
    let output = capture_cargo(root, &["metadata", "--locked", "--format-version", "1"])?;
    serde_json::from_str(&output).map_err(Into::into)
}

fn package_version<'a>(metadata: &'a Value, package_name: &str) -> XtaskResult<&'a str> {
    metadata_array(metadata, "packages")?
        .iter()
        .find(|package| package.get("name").and_then(Value::as_str) == Some(package_name))
        .ok_or_else(|| failure(format!("package `{package_name}` is absent from metadata")))
        .and_then(|package| value_string(package, "version"))
}

fn target_description(target: &Value) -> XtaskResult<String> {
    let name = value_string(target, "name")?;
    let kinds = metadata_array(target, "kind")?
        .iter()
        .map(|kind| {
            kind.as_str()
                .map(str::to_owned)
                .ok_or_else(|| failure("target kind is not a string"))
        })
        .collect::<XtaskResult<Vec<_>>>()?
        .join("+");
    Ok(format!("{name} [{kinds}]"))
}

fn metadata_array<'a>(value: &'a Value, key: &str) -> XtaskResult<&'a Vec<Value>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| failure(format!("metadata field `{key}` is missing or not an array")))
}

fn metadata_string<'a>(value: &'a Value, key: &str) -> XtaskResult<&'a str> {
    value_string(value, key)
}

fn value_string<'a>(value: &'a Value, key: &str) -> XtaskResult<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("field `{key}` is missing or not a string")))
}

fn run_cargo(root: &Path, args: &[&str]) -> XtaskResult<()> {
    print_command("cargo", args);
    let status = cargo(root).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "`cargo {}` failed with {status}",
            args.join(" ")
        )))
    }
}

fn run_program(root: &Path, program: &str, args: &[&str]) -> XtaskResult<()> {
    print_command(program, args);
    let status = Command::new(program)
        .current_dir(root)
        .args(args)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "`{program} {}` failed with {status}",
            args.join(" ")
        )))
    }
}

fn capture_cargo(root: &Path, args: &[&str]) -> XtaskResult<String> {
    print_command("cargo", args);
    output_text(cargo(root).args(args).output()?, "cargo", args)
}

fn capture_program(root: &Path, program: &str, args: &[&str]) -> XtaskResult<String> {
    print_command(program, args);
    let output = Command::new(program)
        .current_dir(root)
        .args(args)
        .output()?;
    output_text(output, program, args)
}

fn output_text(output: Output, program: &str, args: &[&str]) -> XtaskResult<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(failure(format!(
            "`{program} {}` failed with {}: {stderr}",
            args.join(" "),
            output.status
        )));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(Into::into)
}

fn cargo(root: &Path) -> Command {
    let executable = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(executable);
    command.current_dir(root);
    command
}

fn print_command(program: &str, args: &[&str]) {
    eprintln!("+ {program} {}", args.join(" "));
}

fn reject_extra_args(mut args: impl Iterator<Item = String>) -> XtaskResult<()> {
    match args.next() {
        Some(arg) => Err(failure(format!("unexpected argument `{arg}`"))),
        None => Ok(()),
    }
}

fn quoted_setting<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        if candidate.trim() != key {
            return None;
        }
        let value = value.trim();
        value.strip_prefix('"')?.strip_suffix('"')
    })
}

fn inline_quoted_setting<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split([',', '{', '}']).find_map(|field| {
        let (candidate, value) = field.split_once('=')?;
        if candidate.trim() != key {
            return None;
        }
        let value = value.trim();
        value.strip_prefix('"')?.strip_suffix('"')
    })
}

fn tool_version<'a>(output: &'a str, tool: &str) -> Option<&'a str> {
    let mut fields = output.split_whitespace();
    if fields.next()? != tool {
        return None;
    }
    fields.next()
}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    io::Error::other(message.into()).into()
}

fn print_usage() {
    println!("{}", usage());
}

fn usage() -> &'static str {
    "Rustle engineering tasks:\n\
     \n  cargo xtask check\
     \n  cargo xtask check-native\
     \n  cargo xtask check-production\
     \n  cargo xtask supply-chain\
     \n  cargo xtask metadata\
     \n  cargo xtask release-preflight --commit <40-lowercase-hex> [--tag vX.Y.Z] [--output PATH]\
     \n  cargo xtask release-manifest --metadata PATH --artifacts-dir DIR [--output PATH] [--checksums-output PATH]"
}

#[cfg(test)]
mod tests {
    use super::{
        RELEASE_CHECKSUMS_FILE, RELEASE_MANIFEST_FILE, RELEASE_METADATA_FILE,
        RELEASE_METADATA_SCHEMA_VERSION, RELEASE_PACKAGE, ReleaseMetadata,
        architecture_dependency_violations, architecture_source_contract_violations,
        build_release_manifest, ffi_panic_boundary_violations, forbidden_packages_in_tree,
        inline_quoted_setting, observability_source_contract_violations, quoted_setting,
        release_panic_contract_violations, source_contract_violations, tool_version,
        validate_commit_sha, validate_release_tag_identity, write_json_atomic,
        write_release_checksums,
    };
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rustle-xtask-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_release_metadata(version: &str) -> ReleaseMetadata {
        ReleaseMetadata {
            schema_version: RELEASE_METADATA_SCHEMA_VERSION,
            package: RELEASE_PACKAGE.to_owned(),
            version: version.to_owned(),
            tag: Some(format!("v{version}")),
            channel: "stable".to_owned(),
            commit: "0123456789012345678901234567890123456789".to_owned(),
            commit_timestamp: "2026-09-08T00:00:00+00:00".to_owned(),
            toolchain: "1.98.0".to_owned(),
            cargo_lock_sha256: "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
                .to_owned(),
        }
    }

    fn write_expected_release_artifacts(directory: &Path, version: &str) {
        let names = [
            "rustle-linux-x86_64.AppImage".to_owned(),
            "rustle-macos-arm64.dmg".to_owned(),
            "rustle-macos-x86_64.dmg".to_owned(),
            format!("rustle-{version}-windows-x86_64.msi"),
            "rustle-windows-x86_64.exe".to_owned(),
        ];
        for (index, name) in names.into_iter().enumerate() {
            let nested = directory.join(format!("job-{index}"));
            fs::create_dir(&nested).unwrap();
            fs::write(nested.join(name), format!("artifact-{index}")).unwrap();
        }
    }

    #[test]
    fn reads_quoted_toolchain_setting() {
        let manifest = "[toolchain]\nchannel = \"1.98.0\"\nprofile = \"minimal\"\n";
        assert_eq!(quoted_setting(manifest, "channel"), Some("1.98.0"));
        assert_eq!(quoted_setting(manifest, "missing"), None);
    }

    #[test]
    fn release_identity_requires_canonical_tag_commit_and_version() {
        let commit = "0123456789012345678901234567890123456789";
        assert!(validate_commit_sha(commit).is_ok());
        assert!(validate_commit_sha("ABC").is_err());
        assert!(validate_release_tag_identity("v0.5.2", "0.5.2", commit, commit).is_ok());
        assert!(validate_release_tag_identity("0.5.2", "0.5.2", commit, commit).is_err());
        assert!(validate_release_tag_identity("v0.5.02", "0.5.2", commit, commit).is_err());
        assert!(validate_release_tag_identity("v0.5.3", "0.5.2", commit, commit).is_err());
        assert!(
            validate_release_tag_identity(
                "v0.5.2",
                "0.5.2",
                "1111111111111111111111111111111111111111",
                commit
            )
            .is_err()
        );
    }

    #[test]
    fn release_artifact_manifest_is_exact_sorted_and_deterministic() {
        let directory = TestDirectory::new("manifest");
        let products = directory.path().join("products");
        fs::create_dir(&products).unwrap();
        let metadata = sample_release_metadata("0.5.2");
        write_expected_release_artifacts(&products, &metadata.version);

        let first = build_release_manifest(&metadata, &products).unwrap();
        let second = build_release_manifest(&metadata, &products).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.artifacts.len(), 5);
        assert!(
            first
                .artifacts
                .windows(2)
                .all(|pair| pair[0].name < pair[1].name)
        );
        assert_eq!(
            serde_json::to_vec_pretty(&first).unwrap(),
            serde_json::to_vec_pretty(&second).unwrap()
        );

        let metadata_path = directory.path().join(RELEASE_METADATA_FILE);
        let manifest_path = directory.path().join(RELEASE_MANIFEST_FILE);
        let checksums_path = directory.path().join(RELEASE_CHECKSUMS_FILE);
        write_json_atomic(&metadata_path, &metadata).unwrap();
        write_json_atomic(&manifest_path, &first).unwrap();
        write_release_checksums(
            &first,
            &products,
            &metadata_path,
            &manifest_path,
            &checksums_path,
        )
        .unwrap();
        let first_checksums = fs::read_to_string(&checksums_path).unwrap();
        write_release_checksums(
            &first,
            &products,
            &metadata_path,
            &manifest_path,
            &checksums_path,
        )
        .unwrap();
        let second_checksums = fs::read_to_string(&checksums_path).unwrap();
        assert_eq!(first_checksums, second_checksums);
        let names = first_checksums
            .lines()
            .map(|line| line.split_once("  ").unwrap().1)
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 7);
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn release_artifact_manifest_rejects_missing_unexpected_and_duplicate_names() {
        let metadata = sample_release_metadata("0.5.2");

        let missing = TestDirectory::new("missing");
        write_expected_release_artifacts(missing.path(), &metadata.version);
        fs::remove_file(
            missing
                .path()
                .join("job-0")
                .join("rustle-linux-x86_64.AppImage"),
        )
        .unwrap();
        assert!(
            build_release_manifest(&metadata, missing.path())
                .unwrap_err()
                .to_string()
                .contains("missing=[rustle-linux-x86_64.AppImage]")
        );

        let unexpected = TestDirectory::new("unexpected");
        write_expected_release_artifacts(unexpected.path(), &metadata.version);
        fs::write(unexpected.path().join("notes.txt"), "unexpected").unwrap();
        assert!(
            build_release_manifest(&metadata, unexpected.path())
                .unwrap_err()
                .to_string()
                .contains("unexpected=[notes.txt]")
        );

        let duplicate = TestDirectory::new("duplicate");
        write_expected_release_artifacts(duplicate.path(), &metadata.version);
        let duplicate_dir = duplicate.path().join("duplicate-job");
        fs::create_dir(&duplicate_dir).unwrap();
        fs::write(duplicate_dir.join("rustle-windows-x86_64.exe"), "duplicate").unwrap();
        assert!(
            build_release_manifest(&metadata, duplicate.path())
                .unwrap_err()
                .to_string()
                .contains("duplicate release artifact basename")
        );
    }

    #[test]
    fn parses_exact_tool_version_output() {
        assert_eq!(
            tool_version("cargo-deny 0.20.2", "cargo-deny"),
            Some("0.20.2")
        );
        assert_eq!(tool_version("other 0.20.2", "cargo-deny"), None);
        assert_eq!(tool_version("cargo-deny", "cargo-deny"), None);
    }

    #[test]
    fn parses_inline_git_dependency_settings() {
        let dependency = r#"iced = { git = "https://example.invalid/iced", rev = "0123456789012345678901234567890123456789" }"#;
        assert_eq!(
            inline_quoted_setting(dependency, "git"),
            Some("https://example.invalid/iced")
        );
        assert_eq!(
            inline_quoted_setting(dependency, "rev"),
            Some("0123456789012345678901234567890123456789")
        );
        assert_eq!(inline_quoted_setting(dependency, "branch"), None);
    }

    #[test]
    fn production_tree_rejects_debug_only_packages() {
        let tree = "rustle v0.5.2\n└── iced v0.15.0-dev\n    ├── iced_beacon v0.15.0-dev\n    └── iced_devtools v0.15.0-dev";
        assert_eq!(
            forbidden_packages_in_tree(tree),
            vec!["iced_beacon", "iced_devtools"]
        );
        assert!(forbidden_packages_in_tree("rustle v0.5.2\n└── iced v0.15.0-dev").is_empty());
    }

    #[test]
    fn typed_error_source_contracts_reject_string_regressions() {
        assert!(
            !source_contract_violations(
                "crates/rustle-audio/src/player.rs",
                "fn classify_playback_error(message: &str) {}"
            )
            .is_empty()
        );
        assert!(
            !source_contract_violations(
                "apps/rustle-desktop/src/app/message.rs",
                "SongResolveFailed(PlaybackContext, String),"
            )
            .is_empty()
        );
        assert!(
            source_contract_violations(
                "apps/rustle-desktop/src/app/message.rs",
                "SongResolveFailed(PlaybackContext, AppError),"
            )
            .is_empty()
        );
    }

    #[test]
    fn architecture_contracts_reject_reverse_dependencies() {
        assert!(
            architecture_source_contract_violations(
                "src/features/lyrics/parser.rs",
                "use crate::ui::pages::LyricLine;",
            )
            .iter()
            .any(|violation| violation.contains("crate::ui"))
        );
        assert!(
            architecture_source_contract_violations(
                "crates/rustle-audio/src/streaming.rs",
                "crate::cache::publish_or_reuse(); crate::api::NcmQualityLevel;",
            )
            .len()
                == 2
        );
        assert!(
            architecture_source_contract_violations(
                "crates/rustle-platform/src/tray/windows.rs",
                "crate::i18n::t(); crate::app::Message::Noop;",
            )
            .len()
                == 2
        );
        assert!(
            architecture_source_contract_violations(
                "crates/rustle-domain/src/playback.rs",
                "use iced::Theme; use rustle_application::Service;",
            )
            .len()
                == 2
        );
        assert!(
            architecture_source_contract_violations(
                "crates/rustle-application/src/tray.rs",
                "use rustle_domain::playback::PlayMode;",
            )
            .is_empty()
        );
        assert!(
            architecture_source_contract_violations(
                "crates/rustle-storage/src/database.rs",
                "use rustle_application::error::AppError; use sqlx::SqlitePool;",
            )
            .is_empty()
        );
        assert_eq!(
            architecture_source_contract_violations(
                "crates/rustle-storage/src/cache.rs",
                "use crate::api::PlaylistDetail; use rustle_audio::Player;",
            )
            .len(),
            2
        );
        assert_eq!(
            architecture_source_contract_violations(
                "crates/rustle-ncm/src/client.rs",
                "use crate::error::AppError; use rustle_storage::paths::cache_dir;",
            )
            .len(),
            2
        );
        assert_eq!(
            architecture_source_contract_violations(
                "crates/rustle-media/src/scan.rs",
                "use crate::database::Database; use rustle_ncm::NcmClient;",
            )
            .len(),
            2
        );
        assert_eq!(
            architecture_source_contract_violations(
                "crates/rustle-audio/src/thread.rs",
                "use crate::runtime::spawn_guarded; use rustle_storage::AudioManifest;",
            )
            .len(),
            2
        );
        assert_eq!(
            architecture_source_contract_violations(
                "crates/rustle-platform/src/protocol/ipc.rs",
                "fn send() -> Result<(), String> { anyhow::bail!(\"failed\") }",
            )
            .len(),
            2
        );
        assert_eq!(
            architecture_source_contract_violations(
                "crates/rustle-observability/src/lib.rs",
                "use rustle_storage::database; use iced::widget;",
            )
            .len(),
            2
        );
        assert_eq!(
            architecture_source_contract_violations(
                "crates/rustle-ui/src/lib.rs",
                "use crate::app::Message; use rustle_platform::theme;",
            )
            .len(),
            2
        );
    }

    #[test]
    fn architecture_graph_requires_physical_members_and_directed_dependencies() {
        let metadata = json!({
            "workspace_members": ["domain-id", "application-id", "audio-id", "cleanup-id", "media-id", "observability-id", "platform-id", "storage-id", "ncm-id", "ui-id", "root-id"],
            "packages": [
                {
                    "name": "rustle-domain",
                    "id": "domain-id",
                    "dependencies": [
                        {"name": "regex"},
                        {"name": "serde"},
                        {"name": "serde_json"}
                    ]
                },
                {
                    "name": "rustle-application",
                    "id": "application-id",
                    "dependencies": [{"name": "rustle-domain"}]
                },
                {
                    "name": "rustle-audio",
                    "id": "audio-id",
                    "dependencies": [
                        {"name": "parking_lot"},
                        {"name": "reqwest"},
                        {"name": "rodio"},
                        {"name": "rustle-application"},
                        {"name": "rustle-domain"},
                        {"name": "serde"},
                        {"name": "serde_json"},
                        {"name": "spectrum-analyzer"},
                        {"name": "tokio"},
                        {"name": "tracing"}
                    ]
                },
                {
                    "name": "rustle-cleanup",
                    "id": "cleanup-id",
                    "dependencies": []
                },
                {
                    "name": "rustle-media",
                    "id": "media-id",
                    "dependencies": [
                        {"name": "encoding_rs"},
                        {"name": "image"},
                        {"name": "lofty"},
                        {"name": "notify"},
                        {"name": "quick-xml"},
                        {"name": "rayon"},
                        {"name": "rodio"},
                        {"name": "rustle-application"},
                        {"name": "rustle-domain"},
                        {"name": "thiserror"},
                        {"name": "tokio"},
                        {"name": "tracing"},
                        {"name": "walkdir"},
                        {"name": "xxhash-rust"}
                    ]
                },
                {
                    "name": "rustle-observability",
                    "id": "observability-id",
                    "dependencies": [
                        {"name": "directories"},
                        {"name": "serde"},
                        {"name": "serde_json"},
                        {"name": "thiserror"},
                        {"name": "tracing"},
                        {"name": "tracing-appender"},
                        {"name": "tracing-subscriber"},
                        {"name": "zip"}
                    ]
                },
                {
                    "name": "rustle-platform",
                    "id": "platform-id",
                    "dependencies": [
                        {"name": "cosmic-text"},
                        {"name": "discord-rich-presence"},
                        {"name": "global-hotkey"},
                        {"name": "iced"},
                        {"name": "image"},
                        {"name": "interprocess"},
                        {"name": "ksni"},
                        {"name": "mpris-server"},
                        {"name": "objc2"},
                        {"name": "objc2-foundation"},
                        {"name": "rustle-application"},
                        {"name": "rustle-domain"},
                        {"name": "souvlaki"},
                        {"name": "thiserror"},
                        {"name": "tokio"},
                        {"name": "tracing"},
                        {"name": "tray-icon"},
                        {"name": "windows-sys"},
                        {"name": "x11rb"}
                    ]
                },
                {
                    "name": "rustle-storage",
                    "id": "storage-id",
                    "dependencies": [
                        {"name": "anyhow"},
                        {"name": "directories"},
                        {"name": "fs4"},
                        {"name": "futures-util"},
                        {"name": "rustle-application"},
                        {"name": "rustle-domain"},
                        {"name": "serde"},
                        {"name": "serde_json"},
                        {"name": "sqlx"},
                        {"name": "thiserror"},
                        {"name": "tokio"},
                        {"name": "tracing"},
                        {"name": "windows-sys"},
                        {"name": "xxhash-rust"}
                    ]
                },
                {
                    "name": "rustle-ncm",
                    "id": "ncm-id",
                    "dependencies": [
                        {"name": "directories"},
                        {"name": "futures-util"},
                        {"name": "ncm-api-rs"},
                        {"name": "parking_lot"},
                        {"name": "qrcode-generator"},
                        {"name": "reqwest"},
                        {"name": "rustle-application"},
                        {"name": "rustle-domain"},
                        {"name": "serde"},
                        {"name": "serde_json"},
                        {"name": "thiserror"},
                        {"name": "tracing"}
                    ]
                },
                {
                    "name": "rustle-ui",
                    "id": "ui-id",
                    "dependencies": [
                        {"name": "bytemuck"},
                        {"name": "iced"},
                        {"name": "iced_runtime"},
                        {"name": "image"},
                        {"name": "rand"},
                        {"name": "tracing"}
                    ]
                },
                {
                    "name": "rustle",
                    "id": "root-id",
                    "dependencies": [
                        {"name": "rustle-domain"},
                        {"name": "rustle-application"},
                        {"name": "rustle-audio"},
                        {"name": "rustle-media"},
                        {"name": "rustle-observability"},
                        {"name": "rustle-platform"},
                        {"name": "rustle-storage"},
                        {"name": "rustle-ncm"},
                        {"name": "rustle-ui"}
                    ]
                }
            ]
        });
        assert!(
            architecture_dependency_violations(&metadata)
                .unwrap()
                .is_empty()
        );

        let mut invalid = metadata;
        invalid["packages"][0]["dependencies"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "iced"}));
        assert!(
            architecture_dependency_violations(&invalid)
                .unwrap()
                .iter()
                .any(|violation| violation.contains("forbidden direct dependency `iced`"))
        );
    }

    #[test]
    fn observability_contract_rejects_secondary_subscriber_owners() {
        assert!(
            observability_source_contract_violations(
                "src/lib.rs",
                "tracing_subscriber::fmt::init();"
            )
            .len()
                == 1
        );
        assert!(
            observability_source_contract_violations(
                "crates/rustle-observability/src/lib.rs",
                "tracing_subscriber::registry();"
            )
            .is_empty()
        );
        assert!(
            observability_source_contract_violations("src/app.rs", "tracing::info!(\"ok\");")
                .is_empty()
        );
    }

    #[test]
    fn panic_contracts_require_no_unwind_ffi_and_release_unwind() {
        assert_eq!(
            ffi_panic_boundary_violations(
                "src/platform.rs",
                "unsafe extern \"system\" fn callback() {}"
            )
            .len(),
            1
        );
        assert!(
            ffi_panic_boundary_violations(
                "src/platform.rs",
                "unsafe extern \"system\" fn callback() { crate::runtime::catch_ffi_unwind(\"fixture\", || (), || ()); }"
            )
            .is_empty()
        );
        assert_eq!(
            ffi_panic_boundary_violations(
                "src/platform.rs",
                concat!(
                    "unsafe extern \"system\" fn guarded() { ",
                    "crate::runtime::catch_ffi_unwind(\"fixture\", || (), || ()); }\n",
                    "unsafe extern \"C\" fn unguarded() {}"
                )
            )
            .len(),
            1
        );
        assert!(
            !release_panic_contract_violations("[profile.release]\npanic = \"abort\"").is_empty()
        );
        assert!(
            release_panic_contract_violations("[profile.release]\npanic = \"unwind\"").is_empty()
        );
        assert!(
            !release_panic_contract_violations("panic = \"unwind\"\n[profile.release]\nlto = true")
                .is_empty()
        );
    }
}

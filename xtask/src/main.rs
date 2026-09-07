use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

type XtaskResult<T> = Result<T, Box<dyn Error>>;

const SUPPLY_CHAIN_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
];
const FORBIDDEN_PRODUCTION_PACKAGES: &[&str] = &["iced_beacon", "iced_devtools"];

struct SourceContractRule {
    path: &'static str,
    forbidden: &'static str,
    rationale: &'static str,
}

const ERROR_SOURCE_CONTRACTS: &[SourceContractRule] = &[
    SourceContractRule {
        path: "src/audio/player.rs",
        forbidden: "classify_playback_error",
        rationale: "playback failures must be classified by the producer, never by parsing text",
    },
    SourceContractRule {
        path: "src/app/update/song_resolver.rs",
        forbidden: "Result<ResolvedAudioSource, String>",
        rationale: "audio-source resolution is a stable typed application boundary",
    },
    SourceContractRule {
        path: "src/app/update/song_resolver.rs",
        forbidden: "Result<ResolvedSong, String>",
        rationale: "song resolution must preserve stable codes and source chains",
    },
    SourceContractRule {
        path: "src/platform/global_hotkeys.rs",
        forbidden: "Result<(), String>",
        rationale: "native registration and rollback failures require typed semantics",
    },
    SourceContractRule {
        path: "src/platform/global_hotkeys.rs",
        forbidden: "GlobalHotkeyError::new",
        rationale: "platform errors must select an explicit typed kind",
    },
    SourceContractRule {
        path: "src/app/message.rs",
        forbidden: "DatabaseError(String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "src/app/message.rs",
        forbidden: "SongResolveFailed(PlaybackContext, String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "src/app/message.rs",
        forbidden: "DownloadError(i64, String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "src/app/message.rs",
        forbidden: "LyricsLoadFailed(i64, String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "src/app/message.rs",
        forbidden: "NcmPlaylistLoadFailed(u64, i64, String)",
        rationale: "application error messages must carry AppError",
    },
    SourceContractRule {
        path: "src/audio/events.rs",
        forbidden: "error: String",
        rationale: "audio events must transport PlaybackError without a parallel text classifier",
    },
    SourceContractRule {
        path: "src/audio/streaming.rs",
        forbidden: "Error(String)",
        rationale: "streaming terminal events must retain PlaybackError",
    },
    SourceContractRule {
        path: "src/audio/streaming.rs",
        forbidden: "    Failed(String),",
        rationale: "streaming health must retain PlaybackError",
    },
    SourceContractRule {
        path: "src/audio/streaming.rs",
        forbidden: "Fatal(String)",
        rationale: "range failures must be classified at production",
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
    let source_root = root.join("src");
    let mut source_paths = Vec::new();
    collect_rust_source_paths(&source_root, &mut source_paths)?;
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
    if path == "src/observability.rs" {
        return Vec::new();
    }

    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("tracing_subscriber"))
        .map(|(line_index, _)| {
            format!(
                "{path}:{} references `tracing_subscriber`; subscriber configuration belongs to src/observability.rs",
                line_index + 1
            )
        })
        .collect()
}

fn verify_panic_boundary_contracts(root: &Path) -> XtaskResult<()> {
    let source_root = root.join("src");
    let mut source_paths = Vec::new();
    collect_rust_source_paths(&source_root, &mut source_paths)?;
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
    for source_root in [root.join("src"), root.join("crates")] {
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
        "rustle-media",
        "rustle-storage",
        "rustle-ncm",
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
        if matches!(package_name, "rustle-media" | "rustle-storage") {
            for required_dependency in ["rustle-domain", "rustle-application"] {
                if !metadata_array(package, "dependencies")?
                    .iter()
                    .any(|dependency| {
                        dependency.get("name").and_then(Value::as_str) == Some(required_dependency)
                    })
                {
                    violations.push(format!(
                        "package `rustle-storage` must depend on `{required_dependency}`"
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
        "rustle-media",
        "rustle-storage",
        "rustle-ncm",
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
    if path.starts_with("src/platform/tray/") || path == "src/platform/tray.rs" {
        forbidden.push(("crate::app::", "native tray code must emit pure commands"));
        forbidden.push((
            "crate::i18n",
            "native tray code must consume localized presentation",
        ));
        forbidden.push((
            "crate::features",
            "native tray code must use domain/application contracts",
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

fn release_preflight(root: &Path, mut args: impl Iterator<Item = String>) -> XtaskResult<()> {
    let tag = match args.next() {
        None => None,
        Some(flag) if flag == "--tag" => Some(
            args.next()
                .ok_or_else(|| failure("`--tag` requires a value"))?,
        ),
        Some(other) => {
            return Err(failure(format!(
                "unexpected release-preflight argument `{other}`"
            )));
        }
    };
    reject_extra_args(args)?;

    let toolchain = verify_toolchain(root)?;
    verify_production_dependency_graph(root)?;
    let lockfile = root.join("Cargo.lock");
    if !lockfile.is_file() {
        return Err(failure(format!(
            "required lockfile is missing: {}",
            lockfile.display()
        )));
    }

    let metadata = load_metadata(root)?;
    let version = package_version(&metadata, "rustle")?;
    if let Some(tag) = tag {
        let tag_version = normalize_release_tag(&tag);
        if tag_version != version {
            return Err(failure(format!(
                "release tag `{tag}` does not match Cargo version `{version}`"
            )));
        }
        println!("release tag: {tag}");
    } else {
        println!("release tag: not supplied (version-only preflight)");
    }

    println!("toolchain: {toolchain}");
    println!("package version: {version}");
    println!("lockfile: {}", lockfile.display());
    println!("release preflight: ok");
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

fn normalize_release_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
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
     \n  cargo xtask release-preflight [--tag vX.Y.Z]"
}

#[cfg(test)]
mod tests {
    use super::{
        architecture_dependency_violations, architecture_source_contract_violations,
        ffi_panic_boundary_violations, forbidden_packages_in_tree, inline_quoted_setting,
        normalize_release_tag, observability_source_contract_violations, quoted_setting,
        release_panic_contract_violations, source_contract_violations, tool_version,
    };
    use serde_json::json;

    #[test]
    fn reads_quoted_toolchain_setting() {
        let manifest = "[toolchain]\nchannel = \"1.98.0\"\nprofile = \"minimal\"\n";
        assert_eq!(quoted_setting(manifest, "channel"), Some("1.98.0"));
        assert_eq!(quoted_setting(manifest, "missing"), None);
    }

    #[test]
    fn normalizes_release_tag_prefix() {
        assert_eq!(normalize_release_tag("v0.5.2"), "0.5.2");
        assert_eq!(normalize_release_tag("0.5.2"), "0.5.2");
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
                "src/audio/player.rs",
                "fn classify_playback_error(message: &str) {}"
            )
            .is_empty()
        );
        assert!(
            !source_contract_violations(
                "src/app/message.rs",
                "SongResolveFailed(PlaybackContext, String),"
            )
            .is_empty()
        );
        assert!(
            source_contract_violations(
                "src/app/message.rs",
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
                "src/audio/streaming.rs",
                "crate::cache::publish_or_reuse(); crate::api::NcmQualityLevel;",
            )
            .len()
                == 2
        );
        assert!(
            architecture_source_contract_violations(
                "src/platform/tray/windows.rs",
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
    }

    #[test]
    fn architecture_graph_requires_physical_members_and_directed_dependencies() {
        let metadata = json!({
            "workspace_members": ["domain-id", "application-id", "media-id", "storage-id", "ncm-id", "root-id"],
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
                    "name": "rustle",
                    "id": "root-id",
                    "dependencies": [
                        {"name": "rustle-domain"},
                        {"name": "rustle-application"},
                        {"name": "rustle-media"},
                        {"name": "rustle-storage"},
                        {"name": "rustle-ncm"}
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
                "src/observability.rs",
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

use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

type XtaskResult<T> = Result<T, Box<dyn Error>>;

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
    run_cargo(root, &["fmt", "--all", "--check"])?;
    run_cargo(root, &["check", "--locked", "--workspace", "--all-targets"])?;
    run_cargo(root, &["test", "--locked", "--workspace", "--all-targets"])
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
     \n  cargo xtask metadata\
     \n  cargo xtask release-preflight [--tag vX.Y.Z]"
}

#[cfg(test)]
mod tests {
    use super::{normalize_release_tag, quoted_setting};

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
}

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

const VERSION_KEY: &str = "version = \"";
const WORKSPACE_PACKAGE_HEADER: &str = "[workspace.package]";

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<String, String> {
    let command = args
        .next()
        .ok_or_else(|| "expected `next-version` or `set-version <version>`".to_owned())?;
    let manifest_path = workspace_root().join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;

    match command.as_str() {
        "next-version" => {
            ensure_no_extra_args(args)?;
            let current = workspace_version(&manifest)?;
            bump_patch(current)
        }
        "set-version" => {
            let version = args
                .next()
                .ok_or_else(|| "`set-version` requires a version".to_owned())?;
            ensure_no_extra_args(args)?;
            validate_version(&version)?;
            write_workspace_version(&manifest_path, &manifest, &version)?;
            Ok(version)
        }
        _ => Err(format!("unknown command `{command}`")),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .to_path_buf()
}

fn ensure_no_extra_args(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    args.next().map_or(Ok(()), |argument| {
        Err(format!("unexpected argument `{argument}`"))
    })
}

fn workspace_version(manifest: &str) -> Result<&str, String> {
    let workspace_package = manifest
        .split_once(WORKSPACE_PACKAGE_HEADER)
        .map(|(_, rest)| rest)
        .ok_or_else(|| format!("missing {WORKSPACE_PACKAGE_HEADER}"))?;
    let section = workspace_package
        .split_once("\n[")
        .map_or(workspace_package, |(section, _)| section);

    section
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix(VERSION_KEY)
                .and_then(|value| value.strip_suffix('"'))
        })
        .ok_or_else(|| "missing workspace package version".to_owned())
}

fn validate_version(version: &str) -> Result<[u64; 3], String> {
    let mut components = version.split('.');
    let parse_component = |value: Option<&str>, name: &str| {
        value
            .ok_or_else(|| format!("missing {name} version component"))?
            .parse::<u64>()
            .map_err(|_| format!("invalid {name} version component"))
    };
    let parsed = [
        parse_component(components.next(), "major")?,
        parse_component(components.next(), "minor")?,
        parse_component(components.next(), "patch")?,
    ];
    if components.next().is_some() {
        return Err("versions must use major.minor.patch format".to_owned());
    }
    Ok(parsed)
}

fn bump_patch(version: &str) -> Result<String, String> {
    let [major, minor, patch] = validate_version(version)?;
    let patch = patch
        .checked_add(1)
        .ok_or_else(|| "patch version overflowed".to_owned())?;
    Ok(format!("{major}.{minor}.{patch}"))
}

fn write_workspace_version(path: &Path, manifest: &str, version: &str) -> Result<(), String> {
    let updated = updated_workspace_manifest(manifest, version)?;
    fs::write(path, updated).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn updated_workspace_manifest(manifest: &str, version: &str) -> Result<String, String> {
    let current = workspace_version(manifest)?;
    let old = format!("{VERSION_KEY}{current}\"");
    let new = format!("{VERSION_KEY}{version}\"");
    let workspace_header = manifest
        .find(WORKSPACE_PACKAGE_HEADER)
        .ok_or_else(|| format!("missing {WORKSPACE_PACKAGE_HEADER}"))?;
    let version_offset = manifest[workspace_header..]
        .find(&old)
        .map(|offset| workspace_header + offset)
        .ok_or_else(|| "cannot locate workspace version assignment".to_owned())?;

    let mut updated = manifest.to_owned();
    updated.replace_range(version_offset..version_offset + old.len(), &new);
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::{bump_patch, updated_workspace_manifest, validate_version, workspace_version};

    #[test]
    fn reads_only_the_workspace_package_version() {
        let manifest = r#"
[workspace]

[workspace.package]
version = "3.7.9"

[workspace.dependencies]
version = "99.0.0"
"#;
        assert_eq!(workspace_version(manifest), Ok("3.7.9"));
    }

    #[test]
    fn increments_only_the_patch_component() {
        assert_eq!(bump_patch("3.7.9"), Ok("3.7.10".to_owned()));
    }

    #[test]
    fn rejects_non_release_versions() {
        assert!(validate_version("1.2").is_err());
        assert!(validate_version("1.2.3-beta.1").is_err());
        assert!(validate_version("1.2.3.4").is_err());
    }

    #[test]
    fn replaces_only_the_workspace_version() {
        let manifest = r#"[workspace.package]
version = "1.2.3"

[workspace.dependencies]
some-package = { version = "1.2.3" }
"#;
        let expected = r#"[workspace.package]
version = "1.2.4"

[workspace.dependencies]
some-package = { version = "1.2.3" }
"#;
        assert_eq!(
            updated_workspace_manifest(manifest, "1.2.4"),
            Ok(expected.to_owned())
        );
    }
}

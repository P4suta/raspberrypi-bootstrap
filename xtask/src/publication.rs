use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Component, Path, PathBuf},
    process::Command,
};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;

struct SourcePath(PathBuf);

impl SourcePath {
    fn new(path: &Path) -> Result<Self> {
        ensure!(
            !path.as_os_str().is_empty()
                && path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "publication paths must be relative source files"
        );
        for component in path.components() {
            let name = component
                .as_os_str()
                .to_str()
                .context("publication paths must be UTF-8")?
                .to_ascii_lowercase();
            ensure!(
                ![
                    ".git",
                    ".local",
                    "target",
                    "runtime",
                    "backups",
                    "node_modules"
                ]
                .contains(&name.as_str()),
                "private and generated directories cannot be published"
            );
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("publication file needs a name")?
            .to_ascii_lowercase();
        ensure!(
            ![
                "secrets.json",
                "client.json",
                "credentials.txt",
                "iphone.txt",
                "nas-install.txt",
                "authorized_keys",
                "id_ed25519",
                "id_rsa"
            ]
            .contains(&name.as_str()),
            "private runtime inputs cannot be published"
        );
        ensure!(
            !(name.trim_start_matches('.').starts_with("mise.") && name.ends_with(".local.toml")),
            "local tool configuration cannot be published"
        );
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        ensure!(
            ![
                "env", "pem", "key", "pub", "p12", "sqlite", "db", "img", "dmg", "iso", "spk",
                "tar", "zip", "gz", "zst"
            ]
            .contains(&extension.as_str()),
            "private keys, databases, and deployment artifacts cannot be published"
        );
        Ok(Self(path.to_path_buf()))
    }
}

fn checked(command: &mut Command, operation: &str) -> Result<()> {
    let status = command.status().with_context(|| operation.to_owned())?;
    ensure!(status.success(), "{operation} failed with {status}");
    Ok(())
}

fn workspace<T>(operation: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let mut random = [0_u8; 12];
    getrandom::fill(&mut random)
        .map_err(|error| anyhow::anyhow!("temporary workspace randomness failed: {error}"))?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = std::env::temp_dir().join(format!("raspberrypi-check-{suffix}"));
    fs::DirBuilder::new().mode(0o700).create(&path)?;
    let result = operation(&path);
    let cleanup = fs::remove_dir_all(&path).context("remove task-owned check workspace");
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(primary), Err(cleanup)) => {
            Err(primary.context(format!("cleanup also failed: {cleanup:#}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn scanner(root: &Path, source: &Path) -> Command {
    let mut command = Command::new("gitleaks");
    command
        .args([
            "dir",
            "--redact=100",
            "--no-banner",
            "--ignore-gitleaks-allow",
            "--config",
        ])
        .arg(root.join(".gitleaks.toml"))
        .arg(source);
    command
}

#[derive(Deserialize)]
struct Leak {
    #[serde(rename = "RuleID")]
    rule_id: String,
}

pub(super) fn check(root: &Path) -> Result<()> {
    let root = fs::canonicalize(root)?;
    let output = Command::new("git")
        .current_dir(&root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()?;
    ensure!(
        output.status.success(),
        "cannot enumerate publication candidates"
    );
    let candidates = String::from_utf8(output.stdout)?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect::<BTreeSet<_>>();
    ensure!(
        !candidates.is_empty() && candidates.len() <= 4096,
        "publication source set must be nonempty and bounded"
    );
    let paths = candidates
        .iter()
        .map(|path| SourcePath::new(path))
        .collect::<Result<Vec<_>>>()?;
    workspace(|temporary| {
        let source = temporary.join("source");
        fs::create_dir(&source)?;
        let mut total = 0_u64;
        for SourcePath(path) in &paths {
            let original = root.join(path);
            let metadata = fs::symlink_metadata(&original)?;
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "publication candidate must be a regular source file: {}",
                path.display()
            );
            total = total
                .checked_add(metadata.len())
                .context("publication size overflow")?;
            ensure!(
                metadata.len() <= MAX_FILE_BYTES && total <= MAX_SOURCE_BYTES,
                "publication candidates exceed the source size limit"
            );
            let contents = fs::read(&original)?;
            ensure!(
                std::str::from_utf8(&contents).is_ok() && !contents.contains(&0),
                "publication candidate must be UTF-8 text: {}",
                path.display()
            );
            let destination = source.join(path);
            fs::create_dir_all(destination.parent().context("source file needs a parent")?)?;
            fs::write(destination, contents)?;
        }
        checked(&mut scanner(&root, &source), "scan publication candidates")?;
        let rejected = temporary.join("rejected");
        fs::create_dir(&rejected)?;
        let mut synthetic = String::from("gh");
        synthetic.push_str("p_");
        synthetic.push_str("Fq7R9mN2xV5kL8pS1a");
        synthetic.push_str("T4dG6hJ0zB3cD9wY2u");
        fs::write(
            rejected.join("fixture.env"),
            format!("GITHUB_TOKEN={synthetic}\n"),
        )?;
        let output = scanner(&root, &rejected)
            .args(["--report-format", "json", "--report-path", "-"])
            .output()?;
        let findings: Vec<Leak> =
            serde_json::from_slice(&output.stdout).context("decode synthetic secret rejection")?;
        ensure!(
            output.status.code() == Some(1)
                && findings
                    .iter()
                    .any(|finding| finding.rule_id == "github-pat"),
            "publication scanner must reject synthetic credentials (status {}, rules {:?})",
            output.status,
            findings
                .iter()
                .map(|finding| finding.rule_id.as_str())
                .collect::<Vec<_>>()
        );
        println!(
            "{} publication files passed private-path and secret checks; a synthetic credential leak was rejected",
            paths.len()
        );
        Ok(())
    })
}

pub(super) fn dependencies(root: &Path) -> Result<()> {
    workspace(|temporary| {
        let database = temporary.join("advisories");
        let command = |arguments: &[&str]| {
            let mut command = Command::new("cargo");
            command
                .current_dir(root)
                .env("RASPBERRYPI_ADVISORY_DB", &database)
                .args(arguments);
            command
        };
        checked(
            &mut command(&["fetch", "--locked"]),
            "fetch locked dependencies",
        )?;
        checked(
            &mut command(&["deny", "--locked", "fetch", "all"]),
            "fetch current dependency security data",
        )?;
        let mut revisions = BTreeSet::new();
        for entry in fs::read_dir(&database)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let output = Command::new("git")
                    .arg("-C")
                    .arg(entry.path())
                    .args(["rev-parse", "HEAD"])
                    .output()?;
                ensure!(
                    output.status.success(),
                    "advisory snapshot must identify its Git revision"
                );
                let revision = String::from_utf8(output.stdout)?.trim().to_owned();
                ensure!(
                    revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "invalid advisory snapshot revision"
                );
                revisions.insert(revision);
            }
        }
        ensure!(
            revisions.len() == 1,
            "expected one immutable advisory snapshot"
        );
        for revision in revisions {
            println!("RustSec advisory snapshot: {revision}");
        }
        checked(
            &mut command(&["deny", "--offline", "--locked", "check"]),
            "check locked dependencies using the fetched snapshot",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::SourcePath;
    use std::path::Path;

    #[test]
    fn refuses_private_inputs_and_deployment_artifacts() {
        for path in [
            ".local/monitoring/secrets.json",
            "target/debug/tool",
            "runtime/ntfy.env",
            "boot/recovery.pem",
            "card.img.zst",
            "settings/credentials.txt",
            "mise.local.toml",
            "../credentials.txt",
            "/tmp/secret",
        ] {
            assert!(SourcePath::new(Path::new(path)).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn accepts_reusable_configuration_and_source() {
        for path in [
            ".github/workflows/check.yml",
            "mise.checks.toml",
            "apps/monitoring/ntfy.json",
            "xtask/src/main.rs",
        ] {
            assert!(SourcePath::new(Path::new(path)).is_ok());
        }
    }
}

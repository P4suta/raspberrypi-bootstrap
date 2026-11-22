use crate::publication::SourcePath;
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MANIFEST: &str = "source.sha256";
const IDENTITY: &str = "source.id";

fn digest(bytes: &[u8]) -> Result<String> {
    let (program, arguments): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("/usr/bin/shasum", &["-a", "256"])
    } else {
        ("/usr/bin/sha256sum", &[])
    };
    let mut process = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    process
        .stdin
        .take()
        .context("checksum input is unavailable")?
        .write_all(bytes)?;
    let output = process.wait_with_output()?;
    ensure!(output.status.success(), "source checksum command failed");
    let output = String::from_utf8(output.stdout)?;
    let digest = output
        .split_whitespace()
        .next()
        .context("source checksum is missing")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid source checksum"
    );
    Ok(digest.to_owned())
}

fn source_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    let path = SourcePath::new(relative)?.0;
    ensure!(
        !path.to_string_lossy().chars().any(char::is_control),
        "source paths cannot contain control characters"
    );
    let mut full = root.to_path_buf();
    for part in path.components() {
        full.push(part);
        ensure!(
            !fs::symlink_metadata(&full)?.file_type().is_symlink(),
            "source links are forbidden"
        );
    }
    ensure!(
        fs::metadata(&full)?.is_file(),
        "source entry must be a regular file"
    );
    Ok(full)
}

fn decode(manifest: &str) -> Result<BTreeMap<PathBuf, String>> {
    let mut files = BTreeMap::new();
    for line in manifest.lines() {
        let (checksum, path) = line
            .split_once("  ")
            .context("invalid source manifest line")?;
        ensure!(
            checksum.len() == 64 && checksum.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid source checksum"
        );
        let path = SourcePath::new(Path::new(path))?.0;
        ensure!(
            files.insert(path, checksum.to_owned()).is_none(),
            "duplicate source manifest entry"
        );
    }
    ensure!(!files.is_empty(), "source manifest cannot be empty");
    Ok(files)
}

pub(crate) fn stage(root: &Path, destination: &Path) -> Result<()> {
    crate::publication::check(root)?;
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()?;
    ensure!(output.status.success(), "cannot enumerate source files");
    let names = String::from_utf8(output.stdout)?;
    let mut files = BTreeMap::new();
    for name in names.split('\0').filter(|name| !name.is_empty()) {
        let relative = PathBuf::from(name);
        let path = source_path(root, &relative)?;
        files.insert(relative, fs::read(path)?);
    }
    ensure!(!files.is_empty(), "source snapshot cannot be empty");
    fs::DirBuilder::new().mode(0o700).create(destination)?;
    let mut manifest = String::new();
    for (relative, content) in files {
        let target = destination.join(&relative);
        fs::create_dir_all(target.parent().context("source target has no parent")?)?;
        fs::write(&target, &content)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644))?;
        manifest.push_str(&format!("{}  {}\n", digest(&content)?, relative.display()));
    }
    fs::write(destination.join(MANIFEST), &manifest)?;
    fs::write(
        destination.join(IDENTITY),
        format!("{}\n", digest(manifest.as_bytes())?),
    )?;
    crate::publication::scan_snapshot(root, destination)?;
    verify(destination)
}

fn contents(root: &Path, directory: &Path, paths: &mut BTreeSet<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "source links are forbidden"
        );
        if metadata.is_dir() {
            contents(root, &path, paths)?;
        } else {
            ensure!(metadata.is_file(), "source entries must be regular files");
            let relative = path.strip_prefix(root)?;
            if relative != Path::new(MANIFEST) && relative != Path::new(IDENTITY) {
                paths.insert(SourcePath::new(relative)?.0);
            }
        }
    }
    Ok(())
}

pub(crate) fn verify(root: &Path) -> Result<()> {
    let mut actual_paths = BTreeSet::new();
    contents(root, root, &mut actual_paths)?;
    let manifest = fs::read_to_string(root.join(MANIFEST))?;
    let identity = fs::read_to_string(root.join(IDENTITY))?;
    ensure!(
        digest(manifest.as_bytes())? == identity.trim(),
        "source manifest identity differs"
    );
    let files = decode(&manifest)?;
    ensure!(
        actual_paths == files.keys().cloned().collect(),
        "source files differ from the declared file set"
    );
    for (relative, expected) in &files {
        let actual = digest(&fs::read(source_path(root, relative)?)?)?;
        ensure!(
            &actual == expected,
            "deployed source differs from its manifest"
        );
    }
    println!("{} source files match their applied identity", files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decode, digest, verify};
    use anyhow::Result;
    use std::fs;

    #[test]
    fn refuses_empty_ambiguous_and_private_source_manifests() {
        for manifest in [
            "".to_owned(),
            format!("{}  ../private\n", "a".repeat(64)),
            format!("{}  .local/client.json\n", "a".repeat(64)),
            format!("{0}  mise.toml\n{0}  mise.toml\n", "a".repeat(64)),
        ] {
            assert!(decode(&manifest).is_err());
        }
    }

    #[test]
    fn detects_source_corruption_and_manifest_replacement() -> Result<()> {
        let mut nonce = [0_u8; 12];
        getrandom::fill(&mut nonce)
            .map_err(|error| anyhow::anyhow!("temporary source identity failed: {error}"))?;
        let root = std::env::temp_dir().join(format!("source-check-{:x?}", nonce));
        fs::create_dir(&root)?;
        let result = || -> Result<()> {
            fs::write(root.join("mise.toml"), b"[tools]\n")?;
            let manifest = format!("{}  mise.toml\n", digest(b"[tools]\n")?);
            fs::write(root.join("source.sha256"), &manifest)?;
            fs::write(root.join("source.id"), digest(manifest.as_bytes())?)?;
            verify(&root)?;
            let identity = fs::read(root.join("source.id"))?;
            fs::remove_file(root.join("source.id"))?;
            std::os::unix::fs::symlink(root.join("absent"), root.join("source.id"))?;
            assert!(verify(&root).unwrap_err().to_string().contains("links"));
            fs::remove_file(root.join("source.id"))?;
            fs::write(root.join("source.id"), identity)?;
            fs::write(root.join("unexpected.toml"), b"[settings]\n")?;
            assert!(verify(&root).is_err());
            fs::remove_file(root.join("unexpected.toml"))?;
            fs::write(root.join("mise.toml"), b"[settings]\n")?;
            assert!(verify(&root).is_err());
            fs::write(root.join("source.sha256"), b"replaced\n")?;
            assert!(verify(&root).is_err());
            Ok(())
        }();
        fs::remove_dir_all(&root)?;
        result
    }
}

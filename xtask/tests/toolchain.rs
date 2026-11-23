use std::{fs, path::Path};

fn source(path: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(path),
    )
    .unwrap()
}

fn rust_entry(manifest: &str) -> String {
    let mut tools = false;
    let mut lines = manifest.lines();
    while let Some(line) = lines.next() {
        if line.starts_with('[') {
            tools = line == "[tools]";
        } else if tools && line.starts_with("rust = ") {
            let mut entry = line.to_owned();
            while !entry.ends_with('}') {
                let next = lines.next().expect("the Rust entry ends with a brace");
                entry.push(' ');
                entry.push_str(next.trim());
            }
            return entry.replace("[ ", "[").replace(", ]", "]");
        }
    }
    panic!("the manifest has no Rust tool entry")
}

fn recipe<'a>(justfile: &'a str, name: &str) -> Vec<&'a str> {
    justfile
        .lines()
        .skip_while(|line| *line != format!("{name}:"))
        .skip(1)
        .take_while(|line| line.starts_with("    "))
        .map(str::trim)
        .collect()
}

#[test]
fn rust_installs_the_minimal_profile_with_only_the_declared_components() {
    for path in ["mise.toml", "verification/preflight/mise.toml"] {
        let manifest = source(path);
        let entry = rust_entry(&manifest);
        assert!(entry.contains("profile = \"minimal\""), "{path}: {entry}");
        assert!(
            entry.contains("components = [\"clippy\", \"rustfmt\"]"),
            "{path}: {entry}"
        );
    }
}

#[test]
fn apply_prunes_tool_versions_after_installing_the_pinned_tools() {
    let justfile = source("justfile");
    let apply = recipe(&justfile, "apply");
    let install = apply
        .iter()
        .position(|line| line.starts_with("mise bootstrap ") && line.contains("tools"))
        .expect("apply installs the pinned tools");
    let prune = apply
        .iter()
        .position(|line| *line == "mise prune --yes --tools")
        .expect("apply prunes unused tool versions");
    assert!(install < prune);
}

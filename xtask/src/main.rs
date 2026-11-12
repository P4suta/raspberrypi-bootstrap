use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{env, fs, path::Path};

mod operations;
mod publication;

const USAGE: &str = "raspberrypi-maintenance check | check-publication | check-dependencies | preflight | preflight-init MISE AGENT | cached CONFIG ID | prepare|retire-nas|reader-password|reader-password-check|configure|status|notifications|verify CONFIG";

fn values<'a>(document: &'a str, key: &str) -> Result<Vec<&'a str>> {
    let mut section = "";
    let mut found = Vec::new();
    for line in document.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        ensure!(
            !line.ends_with('\\'),
            "continued unit values need native review"
        );
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        let (name, value) = line.split_once('=').context("invalid unit assignment")?;
        if section == "Container" && name.trim() == key {
            found.push(value.trim());
        }
    }
    Ok(found)
}

fn validate_container(document: &str) -> Result<()> {
    let images = values(document, "Image")?;
    ensure!(images.len() == 1, "one pinned image is required");
    let (_, digest) = images[0]
        .split_once("@sha256:")
        .context("image digest is required")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid image digest"
    );
    let ports = values(document, "PublishPort")?;
    ensure!(!ports.is_empty(), "a loopback listener is required");
    for port in ports {
        let parts: Vec<_> = port.split(':').collect();
        ensure!(
            parts.len() == 3 && parts[0] == "127.0.0.1",
            "listener must bind only to loopback"
        );
        for part in &parts[1..] {
            ensure!(part.parse::<u16>()? > 0, "port zero is not allowed");
        }
    }
    for (name, required) in [
        ("User", "10001:10001"),
        ("ReadOnly", "true"),
        ("DropCapability", "all"),
        ("NoNewPrivileges", "true"),
    ] {
        ensure!(
            values(document, name)? == [required],
            "invalid container boundary: {name}"
        );
    }
    Ok(())
}

fn validate_documents(hub: &str, ntfy: &str, server: &Value) -> Result<()> {
    validate_container(hub).context("hub")?;
    validate_container(ntfy).context("ntfy")?;
    ensure!(
        server["auth-default-access"] == "deny-all",
        "anonymous access must be denied"
    );
    ensure!(
        server["enable-signup"] == false,
        "public sign-up must be disabled"
    );
    Ok(())
}

fn main() -> Result<()> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    match arguments.as_slice() {
        [command] if command == "check" => check(Path::new(".")),
        [command] if command == "check-publication" => publication::check(Path::new(".")),
        [command] if command == "check-dependencies" => publication::dependencies(Path::new(".")),
        [command] if command == "preflight" => operations::local_preflight(),
        [command] if command == "--help" || command == "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        [command, mise, agent] if command == "preflight-init" => {
            operations::initialize_preflight(Path::new(mise), Path::new(agent))
        }
        [command, config, id] if command == "cached" => operations::cached(Path::new(config), id),
        [command, config] if command == "prepare" => operations::prepare(Path::new(config)),
        [command, config] if command == "retire-nas" => operations::retire_nas(Path::new(config)),
        [command, config] if command == "reader-password" => {
            operations::reader_password(Path::new(config))
        }
        [command, config] if command == "reader-password-check" => {
            operations::reader_password_check(Path::new(config))
        }
        [command, config] if command == "configure" => operations::configure(Path::new(config)),
        [command, config] if command == "status" => operations::status(Path::new(config)),
        [command, config] if command == "verify" => operations::verify(Path::new(config)),
        [command, action] if command == "fixture" => operations::fixture(action),
        [command, config] if command == "notifications" => {
            operations::notifications(Path::new(config))
        }
        _ => bail!("usage: {USAGE}"),
    }
}

fn check(root: &Path) -> Result<()> {
    let path = root.join("apps/monitoring");
    validate_documents(
        &fs::read_to_string(path.join("beszel.container"))?,
        &fs::read_to_string(path.join("ntfy.container"))?,
        &serde_json::from_slice(&fs::read(path.join("ntfy.json"))?)?,
    )?;
    println!(
        "Private listener, image identity, process permissions, and authentication checks passed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_documents;
    use anyhow::Result;
    use serde_json::json;

    fn container(image: &str, port: &str) -> String {
        format!(
            "[Container]\nImage={image}\nPublishPort={port}\nUser=10001:10001\nReadOnly=true\nDropCapability=all\nNoNewPrivileges=true\n"
        )
    }

    fn pinned() -> String {
        format!("example.invalid/service@sha256:{}", "a".repeat(64))
    }

    #[test]
    fn accepts_private_pinned_services() -> Result<()> {
        validate_documents(
            &container(&pinned(), "127.0.0.1:8090:8090"),
            &container(&pinned(), "127.0.0.1:8081:8080"),
            &json!({"auth-default-access":"deny-all", "enable-signup":false}),
        )
    }

    #[test]
    fn refuses_anonymous_read_or_write() {
        for access in ["read-write", "read-only", "write-only"] {
            assert!(
                validate_documents(
                    &container(&pinned(), "127.0.0.1:8090:8090"),
                    &container(&pinned(), "127.0.0.1:8081:8080"),
                    &json!({"auth-default-access":access,"enable-signup":false}),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn refuses_unpinned_or_exposed_services() {
        for (image, port) in [
            (
                "example.invalid/service:latest".to_owned(),
                "127.0.0.1:8090:8090",
            ),
            (pinned(), "8090:8090"),
            (pinned(), "0.0.0.0:8090:8090"),
            (pinned(), "[::]:8090:8090"),
        ] {
            assert!(
                validate_documents(
                    &container(&image, port),
                    &container(&pinned(), "127.0.0.1:8081:8080"),
                    &json!({"auth-default-access":"deny-all","enable-signup":false}),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn refuses_a_second_exposed_listener() {
        let ntfy = format!(
            "{}PublishPort=0.0.0.0:80:8080\n",
            container(&pinned(), "127.0.0.1:8081:8080")
        );
        assert!(
            validate_documents(
                &container(&pinned(), "127.0.0.1:8090:8090"),
                &ntfy,
                &json!({"auth-default-access":"deny-all","enable-signup":false}),
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_missing_authentication_policy() {
        assert!(
            validate_documents(
                &container(&pinned(), "127.0.0.1:8090:8090"),
                &container(&pinned(), "127.0.0.1:8081:8080"),
                &json!({}),
            )
            .is_err()
        );
    }
}

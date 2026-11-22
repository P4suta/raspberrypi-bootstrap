use super::{
    RotationAccess, credential_lock, password_hash, private_read, private_write, random_text,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt::Write, path::Path};

const IDENTITY: &str = "humidity-publisher";
const SECRET: &str = "humidity-publisher.json";
const TEMPLATE: &str = include_str!("../../../apps/humidity/monitor.toml.in");

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Publisher {
    password_hash: String,
    token: String,
}

impl Publisher {
    fn environment(&self, base: &str) -> Result<String> {
        let fields = [
            (
                "NTFY_AUTH_USERS",
                format!("{IDENTITY}:{}:user", self.password_hash),
            ),
            ("NTFY_AUTH_ACCESS", format!("{IDENTITY}:home-status:wo")),
            (
                "NTFY_AUTH_TOKENS",
                format!("{IDENTITY}:{}:Humidity", self.token),
            ),
        ];
        let mut result = String::new();
        let mut seen = [false; 3];
        for line in base.lines() {
            let Some((index, (_, entry))) = fields
                .iter()
                .enumerate()
                .find(|(_, (name, _))| line.split_once('=').is_some_and(|(key, _)| key == *name))
            else {
                writeln!(result, "{line}")?;
                continue;
            };
            ensure!(!seen[index], "duplicate ntfy credential declaration");
            seen[index] = true;
            let (_, value) = line
                .split_once('=')
                .context("invalid ntfy credential declaration")?;
            let mut existing = value
                .split(',')
                .filter(|entry| entry.split(':').next() == Some(IDENTITY));
            if let Some(item) = existing.next() {
                ensure!(
                    item == entry && existing.next().is_none(),
                    "conflicting humidity publisher declaration"
                );
                writeln!(result, "{line}")?;
            } else {
                let separator = if value.is_empty() { "" } else { "," };
                writeln!(result, "{line}{separator}{entry}")?;
            }
        }
        ensure!(
            seen.into_iter().all(|value| value),
            "missing ntfy credential declaration"
        );
        Ok(result)
    }
}

pub(super) fn preserve_publisher(dir: &Path, base: String) -> Result<String> {
    let path = dir.join(SECRET);
    if path.try_exists()? {
        let publisher: Publisher = serde_json::from_slice(&private_read(&path)?)?;
        publisher.environment(&base)
    } else {
        Ok(base)
    }
}

pub(crate) fn register(dir: &Path, address: &str) -> Result<()> {
    ensure!(
        address.len() == 17
            && address
                .split(':')
                .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit())),
        "invalid Bluetooth address"
    );
    let config = dir.join("monitoring.json");
    let _lock = credential_lock(&config, RotationAccess::Stable)?;
    let runtime = dir.join("runtime");
    let env_path = runtime.join("ntfy.env");
    let base = String::from_utf8(private_read(&env_path)?)?;
    let path = dir.join(SECRET);
    let publisher: Publisher = if path.try_exists()? {
        serde_json::from_slice(&private_read(&path)?)?
    } else {
        let new = Publisher {
            password_hash: password_hash(&random_text(32)?, 12)?,
            token: format!("tk_{}", random_text(29)?),
        };
        new.environment(&base)?;
        private_write(&path, &serde_json::to_vec(&new)?)?;
        new
    };
    let environment = publisher.environment(&base)?;
    private_write(&env_path, environment.as_bytes())?;
    private_write(
        &runtime.join("humidity-ntfy-token"),
        publisher.token.as_bytes(),
    )?;
    private_write(
        &runtime.join("monitor.toml"),
        TEMPLATE
            .replace("@METER_ADDRESS@", &address.to_ascii_uppercase())
            .as_bytes(),
    )?;
    Ok(())
}

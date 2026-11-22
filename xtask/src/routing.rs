#[cfg(test)]
mod tests {
    use super::Config;
    use anyhow::Result;

    #[test]
    fn private_routes_include_the_same_sites_via_addresses() -> Result<()> {
        let config = Config::parse(r#"{"site_id":7,"networks":["10.1.1.0/24"]}"#)?;
        assert_eq!(
            config.environment()?,
            "SUBNET_ROUTES=10.1.1.0/24,fd7a:115c:a1e0:b1a:0:7:a01:100/120\n"
        );
        Ok(())
    }

    #[test]
    fn reject_public_default_duplicate_and_noncanonical_routes() {
        for input in [
            r#"{"site_id":7,"networks":[]}"#,
            r#"{"site_id":7,"networks":["0.0.0.0/0"]}"#,
            r#"{"site_id":7,"networks":["8.8.8.0/24"]}"#,
            r#"{"site_id":7,"networks":["10.1.1.2/24"]}"#,
            r#"{"site_id":7,"networks":["192.168.0.0/24","192.168.0.0/24"]}"#,
            r#"{"site_id":65536,"networks":["10.0.0.0/8"]}"#,
            r#"{"site_id":7,"networks":["10.1.1.0/24\nHOST=other"]}"#,
        ] {
            assert!(
                Config::parse(input)
                    .and_then(|value| value.environment())
                    .is_err()
            );
        }
    }
}
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    net::{Ipv4Addr, Ipv6Addr},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    site_id: u16,
    networks: Vec<String>,
}

impl Config {
    #[cfg(test)]
    pub(crate) fn parse(input: &str) -> Result<Self> {
        Ok(serde_json::from_str(input)?)
    }

    pub(crate) fn environment(&self) -> Result<String> {
        ensure!(
            !self.networks.is_empty(),
            "at least one private network is required"
        );
        let mut seen = BTreeSet::new();
        let mut routes = Vec::new();
        for network in &self.networks {
            let (address, length) = network
                .split_once('/')
                .context("network prefix is required")?;
            let address: Ipv4Addr = address.parse()?;
            let length: u8 = length.parse()?;
            ensure!(
                (8..=32).contains(&length),
                "only private network prefixes are accepted"
            );
            let mask = u32::MAX << (32 - length);
            let bits = u32::from(address);
            ensure!(bits & mask == bits, "network contains host bits");
            ensure!(
                address.is_private() && Ipv4Addr::from(bits | !mask).is_private(),
                "network must be private"
            );
            ensure!(seen.insert((bits, length)), "duplicate network prefix");
            let octets = address.octets();
            let via = Ipv6Addr::new(
                0xfd7a,
                0x115c,
                0xa1e0,
                0x0b1a,
                0,
                self.site_id,
                u16::from_be_bytes([octets[0], octets[1]]),
                u16::from_be_bytes([octets[2], octets[3]]),
            );
            routes.push(format!("{address}/{length}"));
            routes.push(format!("{via}/{}", 96 + length));
        }
        Ok(format!("SUBNET_ROUTES={}\n", routes.join(",")))
    }
}

pub(crate) fn write(site: &str, networks: &str, destination: &Path) -> Result<()> {
    let config = Config {
        site_id: site.parse()?,
        networks: networks.split(',').map(str::to_owned).collect(),
    };
    let contents = config.environment()?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let nonce = getrandom::u64().map_err(|error| anyhow::anyhow!("{error}"))?;
    let staged = parent.join(format!(".routing-{nonce:016x}"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)?;
    let publish = (|| -> Result<()> {
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::hard_link(&staged, destination)?;
        Ok(())
    })();
    let cleanup = fs::remove_file(&staged);
    publish?;
    cleanup?;
    fs::File::open(parent)?.sync_all()?;
    println!("Validated native and 4via6 route configuration written");
    Ok(())
}

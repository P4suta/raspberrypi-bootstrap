use anyhow::{Context, Result, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, net::Ipv4Addr};

mod native;
mod source;
pub(crate) use native::{agent_migrate, agent_restore};
pub(crate) use native::{apply, bootline, check, plan, recover, restore};
pub(crate) use native::{apps_check, apps_restart, monitoring_check};
pub(crate) use source::{stage, verify};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppService {
    Metrics,
    Notifications,
    Agent,
    Telephone,
}

impl AppService {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "metrics" => Ok(Self::Metrics),
            "notifications" => Ok(Self::Notifications),
            "agent" => Ok(Self::Agent),
            "telephone" => Ok(Self::Telephone),
            _ => anyhow::bail!("unknown app service"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppOperation {
    Restart(AppService),
    StopTelephone,
    BootTelephone,
}

pub(crate) fn restart_app(
    service: AppService,
    mut execute: impl FnMut(AppOperation) -> Result<()>,
) -> Result<()> {
    match service {
        AppService::Telephone => {
            execute(AppOperation::StopTelephone)?;
            execute(AppOperation::BootTelephone)
        }
        _ => execute(AppOperation::Restart(service)),
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub machine_id: String,
    pub tailnet_name: String,
    pub network: Network,
    pub routes: Vec<Route>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Network {
    pub interface: String,
    pub address: String,
    pub peer: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Route {
    pub port: u16,
    pub target: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

pub(crate) struct MemoryBootLine(pub String);

impl MemoryBootLine {
    pub(crate) fn parse(existing: &str) -> Result<Self> {
        let line = existing.trim_end_matches(['\r', '\n']);
        ensure!(
            !line.is_empty() && !line.chars().any(|c| c.is_control() && c != '\t'),
            "kernel arguments must occupy one nonempty line"
        );
        let mut arguments = Vec::new();
        let mut remaining = line.trim_start_matches([' ', '\t']);
        while !remaining.is_empty() {
            let mut quoted = false;
            let end = remaining
                .find(|c: char| {
                    if c == '"' {
                        quoted = !quoted;
                    }
                    !quoted && matches!(c, ' ' | '\t')
                })
                .unwrap_or(remaining.len());
            let (argument, rest) = remaining.split_at(end);
            if argument == "--" {
                break;
            }
            remaining = rest.trim_start_matches([' ', '\t']);
            if let Some(controllers) = argument.strip_prefix("cgroup_disable=") {
                let controllers = controllers
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(controllers);
                let retained: Vec<_> = controllers.split(',').filter(|c| *c != "memory").collect();
                if !retained.is_empty() {
                    arguments.push(format!("cgroup_disable={}", retained.join(",")));
                }
            } else if argument != "cgroup_enable=memory" && !argument.starts_with("cgroup_memory=")
            {
                arguments.push(argument.to_owned());
            }
        }
        arguments.push("cgroup_enable=memory".to_owned());
        if !remaining.is_empty() {
            arguments.push(remaining.to_owned());
        }
        Ok(Self(format!("{}\n", arguments.join(" "))))
    }
}

impl Config {
    pub(crate) fn restore_routes(
        &self,
        before: &Value,
        applied: &Value,
        current: &Value,
    ) -> Result<Value> {
        self.validate()?;
        let mut restored = current.clone();
        for route in &self.routes {
            let port = route.port.to_string();
            let origin = format!("{}:{port}", self.tailnet_name);
            let old = &before["Web"][&origin];
            let installed = &applied["Web"][&origin];
            if old["Handlers"].get("/") != installed["Handlers"].get("/")
                && restored["Web"][&origin]["Handlers"].get("/") != old["Handlers"].get("/")
            {
                let web = restored["Web"]
                    .as_object_mut()
                    .context("missing Serve origins")?;
                let entry = web
                    .get_mut(&origin)
                    .context("managed origin changed after the update")?;
                restore_entry(
                    &old["Handlers"],
                    &installed["Handlers"],
                    &mut entry["Handlers"],
                    "/",
                )?;
                if old.is_null() && entry == &json!({"Handlers":{}}) {
                    web.remove(&origin);
                }
            }
            let shares_listener = restored["Web"].as_object().is_some_and(|web| {
                web.iter().any(|(name, site)| {
                    name.rsplit_once(':')
                        .is_some_and(|(_, number)| number == port)
                        && site["Handlers"]
                            .as_object()
                            .is_some_and(|mounts| !mounts.is_empty())
                })
            });
            if before["TCP"].get(&port).is_some() || !shares_listener {
                restore_entry(&before["TCP"], &applied["TCP"], &mut restored["TCP"], &port)?;
            }
        }
        Ok(restored)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.machine_id.len() == 32 && self.machine_id.bytes().all(|b| b.is_ascii_hexdigit()),
            "a machine identity is required"
        );
        ensure!(
            self.tailnet_name.ends_with(".ts.net")
                && self.tailnet_name.len() <= 253
                && self.tailnet_name.split('.').all(|label| !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')),
            "a canonical tailnet DNS name is required"
        );
        ensure!(
            self.network.interface == "eth0",
            "the managed network drop-in requires eth0"
        );
        let (address, prefix) = self
            .network
            .address
            .split_once('/')
            .context("management prefix is missing")?;
        let address: Ipv4Addr = address.parse().context("invalid management address")?;
        let peer: Ipv4Addr = self
            .network
            .peer
            .parse()
            .context("invalid management peer")?;
        ensure!(
            prefix == "30"
                && address.is_private()
                && peer.is_private()
                && u32::from(address) >> 2 == u32::from(peer) >> 2
                && address != peer
                && [1, 2].contains(&(u32::from(address) & 3))
                && [1, 2].contains(&(u32::from(peer) & 3)),
            "management endpoints must share a private /30 link"
        );
        ensure!(
            !self.routes.is_empty(),
            "at least one private route is required"
        );
        let mut ports = BTreeSet::new();
        for route in &self.routes {
            ensure!(
                route.port > 0 && ports.insert(route.port),
                "duplicate or invalid route port"
            );
            for target in std::iter::once(&route.target).chain(&route.alternatives) {
                let url = Url::parse(target).context("invalid private proxy target")?;
                ensure!(
                    url.scheme() == "http"
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.path() == "/"
                        && url.query().is_none()
                        && url.fragment().is_none()
                        && url.port_or_known_default().is_some_and(|port| port > 0)
                        && (url.host_str() == Some("127.0.0.1")
                            || (url.host_str() == Some(self.network.peer.as_str())
                                && url.port_or_known_default() == Some(80))),
                    "proxy targets must be local or the management peer"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn serve_config(&self, current: &Value) -> Result<Value> {
        self.validate()?;
        let mut updated = if current.is_null() {
            json!({})
        } else {
            current.clone()
        };
        ensure!(updated.is_object(), "Serve configuration must be an object");
        if let Some(funnel) = updated.get("AllowFunnel") {
            ensure!(
                funnel
                    .as_object()
                    .is_some_and(|entries| entries.values().all(|value| value == false)),
                "public Funnel exposure is outside the private host policy"
            );
        }
        for field in ["TCP", "Web"] {
            if updated.get(field).is_none_or(Value::is_null) {
                updated[field] = json!({});
            }
            ensure!(updated[field].is_object(), "invalid Serve endpoint map");
        }
        for route in &self.routes {
            let port = route.port.to_string();
            let origin = format!("{}:{}", self.tailnet_name, route.port);
            if !route.alternatives.is_empty()
                && let Some(existing) = updated["Web"][&origin]["Handlers"].get("/")
            {
                let proxy = existing["Proxy"]
                    .as_str()
                    .context("fallback route has no proxy")?;
                ensure!(
                    proxy == route.target
                        || route.alternatives.iter().any(|target| target == proxy),
                    "fallback route belongs to an unrecognized proxy"
                );
                ensure!(
                    updated["TCP"][&port]["HTTPS"] == true,
                    "fallback route must use HTTPS"
                );
                continue;
            }
            updated["TCP"][&port] = json!({"HTTPS":true});
            let site = updated["Web"]
                .as_object_mut()
                .context("invalid Serve origins")?
                .entry(origin)
                .or_insert_with(|| json!({}));
            let handlers = site
                .as_object_mut()
                .context("invalid Serve origin")?
                .entry("Handlers")
                .or_insert_with(|| json!({}));
            handlers
                .as_object_mut()
                .context("invalid Serve handlers")?
                .insert("/".to_owned(), json!({"Proxy":route.target}));
        }
        Ok(updated)
    }
}

fn restore_entry(before: &Value, applied: &Value, current: &mut Value, key: &str) -> Result<()> {
    if before.get(key) == applied.get(key) || current.get(key) == before.get(key) {
        return Ok(());
    }
    ensure!(
        current.get(key) == applied.get(key),
        "route changed after the managed update"
    );
    let entries = current
        .as_object_mut()
        .context("invalid Serve endpoint map")?;
    match before.get(key) {
        Some(value) => {
            entries.insert(key.to_owned(), value.clone());
        }
        None => {
            entries.remove(key);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppOperation, AppService, Config, MemoryBootLine, restart_app};
    use anyhow::Result;
    use serde_json::json;

    fn config() -> Result<Config> {
        Ok(serde_json::from_value(json!({
            "machine_id":"0123456789abcdef0123456789abcdef",
            "tailnet_name":"device.example.ts.net",
            "network":{"interface":"eth0","address":"172.30.254.249/30","peer":"172.30.254.250"},
            "routes":[
                {"port":8443,"target":"http://127.0.0.1:8090"},
                {"port":11443,"target":"http://172.30.254.250:80","alternatives":["http://127.0.0.1:8790"]}
            ]
        }))?)
    }

    #[test]
    fn refuses_os_service_targets_and_preserves_an_active_telephone_call() -> Result<()> {
        for name in ["ssh", "tailscaled", "../telephone", "all"] {
            assert!(AppService::parse(name).is_err());
        }
        let mut operations = Vec::new();
        let busy = restart_app(AppService::parse("telephone")?, |operation| {
            operations.push(operation);
            if operation == AppOperation::StopTelephone {
                anyhow::bail!("active call");
            }
            Ok(())
        });
        assert!(busy.is_err());
        assert_eq!(operations, [AppOperation::StopTelephone]);
        Ok(())
    }

    #[test]
    fn enables_memory_without_changing_root_or_other_controller_policy() -> Result<()> {
        let original = "console=tty1 root=PARTUUID=example cgroup_disable=memory,io rootwait\n";
        let updated = MemoryBootLine::parse(original)?.0;
        assert_eq!(
            updated,
            "console=tty1 root=PARTUUID=example cgroup_disable=io rootwait cgroup_enable=memory\n"
        );
        assert_eq!(MemoryBootLine::parse(&updated)?.0, updated);
        assert!(MemoryBootLine::parse("root=a\nroot=b\n").is_err());
        Ok(())
    }

    #[test]
    fn leaves_init_arguments_and_quoted_values_intact() -> Result<()> {
        let source = "rootwait note=\"one  two -- three\" cgroup_disable=memory,io -- single  cgroup_disable=memory\n";
        let expected = "rootwait note=\"one  two -- three\" cgroup_disable=io cgroup_enable=memory -- single  cgroup_disable=memory\n";
        assert_eq!(MemoryBootLine::parse(source)?.0, expected);
        assert_eq!(MemoryBootLine::parse(expected)?.0, expected);
        Ok(())
    }

    #[test]
    fn preserves_other_mounts_on_the_same_origin_during_apply_and_restore() -> Result<()> {
        let config = config()?;
        let origin = "device.example.ts.net:8443";
        let before = json!({"TCP":{"8443":{"HTTPS":true}},"Web":{origin:{
            "Handlers":{"/health":{"Text":"ready"}},"OpaqueField":true
        }}});
        let applied = config.serve_config(&before)?;
        assert_eq!(
            applied["Web"][origin]["Handlers"]["/health"],
            before["Web"][origin]["Handlers"]["/health"]
        );
        assert_eq!(applied["Web"][origin]["OpaqueField"], true);
        let mut current = applied.clone();
        current["Web"][origin]["Handlers"]["/health"] = json!({"Text":"updated"});
        let restored = config.restore_routes(&before, &applied, &current)?;
        assert_eq!(
            restored["Web"][origin]["Handlers"],
            json!({"/health":{"Text":"updated"}})
        );
        assert_eq!(restored["Web"][origin]["OpaqueField"], true);
        let mut added = config.serve_config(&json!({}))?;
        added["Web"][origin]["Handlers"]["/other"] = json!({"Text":"new"});
        let original_apply = config.serve_config(&json!({}))?;
        let restored = config.restore_routes(&json!({}), &original_apply, &added)?;
        assert_eq!(
            restored["Web"][origin]["Handlers"],
            json!({"/other":{"Text":"new"}})
        );
        assert_eq!(restored["TCP"]["8443"], json!({"HTTPS":true}));
        Ok(())
    }

    #[test]
    fn rejects_management_addresses_that_cannot_form_the_private_link() -> Result<()> {
        config()?.validate()?;
        for address in [
            "8.8.8.1/30",
            "172.30.254.248/30",
            "172.30.254.251/30",
            "172.30.254.249/24",
        ] {
            let mut invalid = config()?;
            invalid.network.address = address.to_owned();
            assert!(invalid.validate().is_err(), "accepted {address}");
        }
        let mut invalid = config()?;
        invalid.network.peer = "172.30.253.250".to_owned();
        assert!(invalid.validate().is_err());
        invalid = config()?;
        invalid.network.interface = "eth0\n[Route]".to_owned();
        assert!(invalid.validate().is_err());
        invalid.network.interface = "eth1".to_owned();
        assert!(invalid.validate().is_err());
        Ok(())
    }

    #[test]
    fn refuses_public_proxy_targets_and_duplicate_route_ownership() -> Result<()> {
        for target in [
            "https://example.com",
            "http://192.0.2.1:80",
            "http://user:pass@127.0.0.1:8090",
            "http://127.0.0.1:8090/path",
        ] {
            let mut invalid = config()?;
            invalid.routes[0].target = target.to_owned();
            assert!(invalid.validate().is_err(), "accepted {target}");
        }
        let mut invalid = config()?;
        invalid.routes.push(invalid.routes[0].clone());
        assert!(invalid.validate().is_err());
        Ok(())
    }

    #[test]
    fn preserves_unrelated_routes_and_a_validated_console_handoff() -> Result<()> {
        let current = json!({
            "TCP":{"12443":{"HTTPS":true},"11443":{"HTTPS":true}},
            "Web":{
                "device.example.ts.net:12443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787"}}},
                "device.example.ts.net:11443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8790"}}}
            }
        });
        let updated = config()?.serve_config(&current)?;
        assert_eq!(updated["TCP"]["12443"], current["TCP"]["12443"]);
        assert_eq!(
            updated["Web"]["device.example.ts.net:12443"],
            current["Web"]["device.example.ts.net:12443"]
        );
        assert_eq!(
            updated["Web"]["device.example.ts.net:11443"],
            current["Web"]["device.example.ts.net:11443"]
        );
        assert_eq!(
            updated["Web"]["device.example.ts.net:8443"]["Handlers"]["/"]["Proxy"],
            "http://127.0.0.1:8090"
        );
        assert_eq!(config()?.serve_config(&updated)?, updated);
        Ok(())
    }

    #[test]
    fn rejects_public_exposure_and_an_unknown_owner_on_a_fallback_port() -> Result<()> {
        let public = json!({"AllowFunnel":{"device.example.ts.net:8443":true}});
        assert!(config()?.serve_config(&public).is_err());
        let conflict = json!({"Web":{"device.example.ts.net:11443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:9999"}}}}});
        assert!(config()?.serve_config(&conflict).is_err());
        Ok(())
    }

    #[test]
    fn restores_owned_routes_without_overwriting_a_later_deployment() -> Result<()> {
        let config = config()?;
        let before = json!({"TCP":{"8443":{"HTTPS":true}},"Web":{
            "device.example.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8099"}}}
        }});
        let applied = config.serve_config(&before)?;
        let mut current = applied.clone();
        current["Web"]["device.example.ts.net:12443"] =
            json!({"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787"}}});
        let restored = config.restore_routes(&before, &applied, &current)?;
        assert_eq!(
            restored["Web"]["device.example.ts.net:8443"],
            before["Web"]["device.example.ts.net:8443"]
        );
        assert_eq!(
            restored["Web"]["device.example.ts.net:12443"],
            current["Web"]["device.example.ts.net:12443"]
        );
        assert!(restored["Web"].get("device.example.ts.net:11443").is_none());
        current["Web"]["device.example.ts.net:8443"]["Handlers"]["/"]["Proxy"] =
            json!("http://127.0.0.1:9999");
        assert!(config.restore_routes(&before, &applied, &current).is_err());
        Ok(())
    }
}

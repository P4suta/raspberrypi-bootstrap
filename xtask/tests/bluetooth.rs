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

#[test]
fn image_and_live_bootstrap_allow_bluetooth_and_keep_wifi_off() {
    let policy = source("config/radio-policy.txt");
    assert!(policy.lines().any(|s| s == "dtoverlay=disable-wifi"));
    assert!(!policy.lines().any(|s| s == "dtoverlay=disable-bt"));
    let manifest = source("mise.toml");
    assert!(manifest.contains("\"apt:bluez\""));
    assert!(manifest.contains("\"apt:bluez-firmware\""));
    assert!(manifest.contains(
        "[bootstrap.services.bluetooth]\nstate = \"running\"\nenabled = true\nmasked = false"
    ));
    assert!(
        manifest.contains(
            "[bootstrap.services.iwd]\nstate = \"stopped\"\nenabled = false\nmasked = true"
        )
    );
    let layer = source("image/layer/tailscale-development.yaml");
    assert!(
        layer
            .lines()
            .any(|line| line.contains("mask iwd.service") && !line.contains("bluetooth.service"))
    );
    assert!(layer.contains("enable bluetooth.service"));
    assert!(layer.lines().any(|line| line.trim() == "- bluez-firmware"));
    assert!(
        !layer
            .lines()
            .any(|line| line.contains("mask ") && line.contains("hciuart"))
    );
    assert!(
        source(
            "image/layer/tailscale-development.d/for/rpi-boot-firmware/config.txt.d/90-radio-policy"
        )
        .contains("include radio-policy.txt")
    );
}

#[test]
fn image_and_project_checks_include_the_humidity_profile() {
    assert!(source("image/layer/tailscale-development.yaml").contains("/../mise.humidity.toml"));
    let commands = source("justfile");
    for line in commands
        .lines()
        .filter(|line| line.contains("mise.monitoring.toml"))
    {
        assert!(line.contains("mise.humidity.toml"));
    }
    assert!(commands.contains("humidity-plan:") && commands.contains("humidity-apply:"));
}

#[test]
fn rootless_unit_uses_the_host_uid_and_exposes_only_a_loopback_listener() {
    let unit = source("apps/humidity/humidity-monitor.container");
    for line in [
        "UserNS=keep-id",
        "User=%U:%G",
        "Volume=/run/dbus/system_bus_socket:/run/dbus/system_bus_socket:ro",
        "PublishPort=127.0.0.1:8092:8080",
        "Network=monitoring.network",
        "NoNewPrivileges=true",
        "WantedBy=default.target",
    ] {
        assert!(unit.lines().any(|s| s == line), "missing {line}");
    }
    let settings = source("apps/humidity/monitor.toml.in");
    for setting in [
        "reference = [40, 50]",
        "dry_below = 35",
        "high_above = 55",
        "high_seconds = 1800",
        "loss_seconds = 600",
        "recovery_seconds = 300",
        "/home-status",
    ] {
        assert!(settings.contains(setting), "missing {setting}");
    }
}

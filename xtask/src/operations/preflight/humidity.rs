use super::{
    MACHINE, MISE_SHA256, app, app_command, command, copy, destination_directory, orb,
    validate_workspace, verify_tool,
};
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};

const SOURCE: &str = "/workspace/humidity-source";
const EXERCISE: &str = "/var/tmp/humidity-target/debug/examples/humidity_rehearsal";
const MAINTENANCE: &str = "/var/tmp/humidity-target/debug/raspberrypi-maintenance";

pub(crate) fn local(image: &Path, workspace: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(image).context("read the receiver image archive")?;
    ensure!(
        metadata.is_file() && metadata.len() > 0,
        "expected a receiver image archive"
    );
    validate_workspace(workspace)?;
    verify_tool(&workspace.join("tools/mise"), MISE_SHA256)?;
    ensure!(
        workspace.join("fixture/checkpoint.json").is_file(),
        "run the monitoring rehearsal first"
    );
    let source = std::env::current_dir()?;
    let staged = workspace.join("humidity-source");
    destination_directory(&staged)?;
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "xtask",
        "apps",
        "mise.toml",
        "mise.humidity.toml",
        "verification",
    ] {
        copy(&source.join(name), &staged.join(name))?;
    }
    copy(image, &staged.join("receiver.tar"))?;
    command("orb", &["start", MACHINE])?;
    orb(&[
        "env",
        "--chdir",
        SOURCE,
        "/workspace/tools/mise",
        "trust",
        "mise.toml",
    ])?;
    orb(&[
        "env",
        "--chdir",
        SOURCE,
        "CARGO_TARGET_DIR=/var/tmp/humidity-target",
        "CARGO_INCREMENTAL=0",
        "/workspace/tools/mise",
        "x",
        "--",
        "cargo",
        "build",
        "--locked",
        "--jobs",
        "2",
        "--bin",
        "raspberrypi-maintenance",
        "--examples",
    ])?;
    orb(&[MAINTENANCE, "fixture", "humidity"])?;
    orb(&[EXERCISE, "exercise"])?;
    command("orb", &["restart", MACHINE])?;
    orb(&[EXERCISE, "resumed"])
}

pub(super) fn install() -> Result<()> {
    ensure!(
        fs::read_to_string("/etc/hostname")?.trim() == MACHINE
            && Path::new("/workspace/fixture/checkpoint.json").is_file(),
        "requires the prepared ARM64 fixture"
    );
    command(
        "apt-get",
        &[
            "install",
            "-y",
            "--no-install-recommends",
            "bluez",
            "sqlite3",
        ],
    )?;
    let root = Path::new(SOURCE);
    let profile = Path::new("/var/tmp/humidity-profile");
    destination_directory(profile)?;
    copy(&root.join("mise.humidity.toml"), &profile.join("mise.toml"))?;
    copy(&root.join("apps"), &profile.join("apps"))?;
    command(
        "env",
        &[
            "--chdir",
            "/var/tmp/humidity-profile",
            "/workspace/tools/mise",
            "trust",
            "mise.toml",
        ],
    )?;
    command(
        "env",
        &[
            "--chdir",
            "/var/tmp/humidity-profile",
            "/workspace/tools/mise",
            "bootstrap",
            "--yes",
            "--only",
            "files",
        ],
    )?;
    command("systemctl", &["reload", "dbus"])?;
    command(
        "install",
        &[
            "-m",
            "0644",
            "/workspace/humidity-source/verification/preflight/humidity-radio.service",
            "/etc/systemd/system/humidity-radio.service",
        ],
    )?;
    command("systemctl", &["daemon-reload"])?;
    command("systemctl", &["enable", "--now", "humidity-radio.service"])?;
    command("systemctl", &["restart", "humidity-radio.service"])?;
    command(
        "install",
        &[
            "-m",
            "0644",
            "/workspace/humidity-source/receiver.tar",
            "/var/tmp/humidity-image.tar",
        ],
    )?;
    app(&["podman", "load", "-i", "/var/tmp/humidity-image.tar"])?;
    command(
        "install",
        &[
            "-d",
            "-m",
            "0700",
            "/var/tmp/humidity-credentials",
            "/var/tmp/humidity-credentials/runtime",
        ],
    )?;
    command(
        "install",
        &[
            "-m",
            "0600",
            "/workspace/fixture/ntfy.env",
            "/var/tmp/humidity-credentials/runtime/ntfy.env",
        ],
    )?;
    crate::operations::humidity::register(
        Path::new("/var/tmp/humidity-credentials"),
        "AA:BB:CC:DD:EE:FF",
    )?;
    for (input, output) in [
        (
            "/var/tmp/humidity-credentials/runtime/ntfy.env",
            "/var/lib/apps/.config/monitoring/ntfy.env",
        ),
        (
            "/var/tmp/humidity-credentials/runtime/monitor.toml",
            "/var/lib/apps/.config/humidity-monitor/monitor.toml",
        ),
    ] {
        command(
            "install",
            &["-o", "apps", "-g", "apps", "-m", "0600", input, output],
        )?;
    }
    let mut token = fs::File::open("/var/tmp/humidity-credentials/runtime/humidity-ntfy-token")?;
    let mut create = app_command(&[
        "podman",
        "secret",
        "create",
        "--replace",
        "humidity-ntfy-token",
        "-",
    ])?
    .stdin(std::process::Stdio::piped())
    .spawn()?;
    {
        let mut pipe = create.stdin.take().context("open Podman secret input")?;
        std::io::copy(&mut token, &mut pipe)?;
    }
    ensure!(
        create.wait()?.success(),
        "install the rehearsal publisher token"
    );
    command(EXERCISE, &["shorten-timers"])?;
    fs::write("/run/humidity-rehearsal-mode", "normal")?;
    app(&["systemctl", "--user", "daemon-reload"])?;
    app(&["systemctl", "--user", "restart", "monitoring-ntfy.service"])?;
    app(&["systemctl", "--user", "restart", "humidity-monitor.service"])?;
    app(&[
        "systemd-analyze",
        "--user",
        "verify",
        "--man=no",
        "--generators=true",
        "humidity-monitor.service",
    ])
}

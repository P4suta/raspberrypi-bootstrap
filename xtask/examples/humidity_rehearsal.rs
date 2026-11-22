use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs,
    os::unix::fs::MetadataExt,
    process::Command,
    thread,
    time::{Duration, Instant},
};

const DASHBOARD: &str = "http://127.0.0.1:8092/";
const TOPIC: &str = "http://127.0.0.1:8081/home-status/json?poll=1&since=all";
const HISTORY: &str = "/var/lib/apps/humidity-monitor/readings.sqlite";
const MODE: &str = "/run/humidity-rehearsal-mode";
const RESTART: &str = "/var/tmp/humidity-rehearsal-checkpoint";
const CONFIG: &str = "/var/lib/apps/.config/humidity-monitor/monitor.toml";

#[derive(Deserialize)]
struct Credentials {
    secrets: Reader,
}
#[derive(Deserialize)]
struct Reader {
    reader_password: String,
}
#[derive(Deserialize)]
struct Notification {
    id: String,
    message: String,
}

fn until(description: &str, mut probe: impl FnMut() -> Result<bool>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(55);
    while !probe()? {
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        thread::sleep(Duration::from_millis(400));
    }
    Ok(())
}

fn scalar(query: &str) -> Result<i64> {
    let data = Command::new("sqlite3")
        .args(["-readonly", "-cmd", ".timeout 4000", HISTORY, query])
        .output()?;
    ensure!(data.status.success(), "cannot read rehearsal history");
    Ok(std::str::from_utf8(&data.stdout)?.trim().parse()?)
}

fn live(client: &Client) -> Result<bool> {
    let Ok(response) = client.get(DASHBOARD).send() else {
        return Ok(false);
    };
    Ok(response.error_for_status()?.text()?.contains("45％"))
}

fn messages(client: &Client, reader: &Reader) -> Result<Vec<Notification>> {
    client
        .get(TOPIC)
        .basic_auth("iphone", Some(&reader.reader_password))
        .send()?
        .error_for_status()?
        .text()?
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn check_access(client: &Client) -> Result<()> {
    let pid = Command::new("runuser")
        .args([
            "-u",
            "apps",
            "--",
            "env",
            "--chdir=/var/lib/apps",
            "XDG_RUNTIME_DIR=/run/user/1000",
            "podman",
            "inspect",
            "--format",
            "{{.State.Pid}}",
            "humidity-monitor",
        ])
        .output()?;
    ensure!(pid.status.success(), "inspect rootless container");
    let pid: u32 = std::str::from_utf8(&pid.stdout)?.trim().parse()?;
    let process = fs::read_to_string(format!("/proc/{pid}/status"))?;
    let uids = process
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .context("missing process identity")?;
    ensure!(
        fs::metadata(HISTORY)?.uid() == 1000 && uids.split_whitespace().eq(["1000"; 4]),
        "monitor must run as apps and own its database"
    );
    let token = fs::read_to_string("/var/tmp/humidity-credentials/runtime/humidity-ntfy-token")?;
    for request in [
        client.get(TOPIC),
        client.get(TOPIC).bearer_auth(token.trim()),
        client
            .post("http://127.0.0.1:8081/separate-topic")
            .bearer_auth(token.trim())
            .body("fixture"),
    ] {
        ensure!(
            matches!(request.send()?.status().as_u16(), 401 | 403),
            "unexpected notification access"
        );
    }
    Ok(())
}

fn exercise(client: &Client, reader: &Reader) -> Result<()> {
    fs::write(MODE, "normal")?;
    until("a live humidity reading", || live(client))?;
    check_access(client)?;
    for (mode, expected) in [
        ("high", "高湿度"),
        ("normal", "高湿度から復旧"),
        ("silent", "受信途絶"),
        ("normal", "受信再開"),
    ] {
        let seen: HashSet<_> = messages(client, reader)?
            .into_iter()
            .map(|m| m.id)
            .collect();
        let title = format!("カメラ収納　{expected}");
        fs::write(MODE, mode)?;
        until(expected, || {
            Ok(messages(client, reader)?
                .iter()
                .any(|m| !seen.contains(&m.id) && m.message.lines().next() == Some(title.as_str())))
        })?;
        println!("Received {mode} transition: {expected}");
    }
    let received = scalar("SELECT received FROM monitor")?;
    let sample = scalar("SELECT min(received) FROM samples")?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    fs::write(RESTART, format!("{} {received} {sample}", boot.trim()))?;
    Ok(())
}

fn resumed(client: &Client) -> Result<()> {
    let saved = fs::read_to_string(RESTART)?;
    let mut values = saved.split_whitespace();
    let boot = values.next().context("missing saved boot ID")?;
    let previous: i64 = values.next().context("missing reception time")?.parse()?;
    let sample: i64 = values
        .next()
        .context("missing first minute sample")?
        .parse()?;
    ensure!(
        boot != fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim(),
        "guest has not restarted"
    );
    fs::write(MODE, "normal")?;
    until("a fresh reading after reboot", || {
        Ok(live(client)? && scalar("SELECT received FROM monitor")? > previous)
    })?;
    ensure!(
        scalar("SELECT min(received) FROM samples")? == sample,
        "reboot lost the first recorded sample"
    );
    println!("Fresh reception resumed after reboot and minute history survived");
    Ok(())
}

fn shorten_timers() -> Result<()> {
    let mut settings = fs::read_to_string(CONFIG)?;
    for (production, fixture) in [
        ("high_seconds = 1800", "high_seconds = 3"),
        ("recovery_seconds = 300", "recovery_seconds = 3"),
        ("loss_seconds = 600", "loss_seconds = 8"),
    ] {
        ensure!(
            settings.lines().any(|line| line == production),
            "unexpected deployment threshold"
        );
        settings = settings.replace(production, fixture);
    }
    fs::write(CONFIG, settings)?;
    Ok(())
}

fn main() -> Result<()> {
    ensure!(
        std::env::consts::ARCH == "aarch64"
            && fs::read_to_string("/etc/hostname")?.trim() == "pi-operations-preflight",
        "requires the isolated ARM64 rehearsal"
    );
    let credentials: Credentials =
        serde_json::from_slice(&fs::read("/workspace/fixture/state.json")?)?;
    let client = Client::builder().timeout(Duration::from_secs(4)).build()?;
    match std::env::args().nth(1).as_deref() {
        Some("shorten-timers") => shorten_timers(),
        Some("exercise") => exercise(&client, &credentials.secrets),
        Some("resumed") => resumed(&client),
        _ => bail!("expected shorten-timers, exercise or resumed"),
    }
}

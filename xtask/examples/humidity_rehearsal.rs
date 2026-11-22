use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const DASHBOARD: &str = "http://127.0.0.1:8092/";
const TOPIC: &str = "http://127.0.0.1:8081/home-status/json";
const DATA: &str = "/var/lib/apps/humidity-monitor";
const MODE: &str = "/run/humidity-rehearsal-mode";
const RESTART: &str = "/var/tmp/humidity-rehearsal-checkpoint";
const CONFIG: &str = "/var/lib/apps/.config/humidity-monitor/monitor.toml";

#[derive(Clone, Copy)]
enum RadioMode {
    Normal,
    High,
    Silent,
}

impl RadioMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::High => "high",
            Self::Silent => "silent",
        }
    }

    fn publish(self, path: &Path) -> Result<()> {
        let next = path.with_extension(format!("next-{}", std::process::id()));
        fs::write(&next, self.as_str())?;
        Ok(fs::rename(next, path)?)
    }
}

#[derive(Deserialize)]
struct Credentials {
    secrets: Reader,
}
#[derive(Deserialize)]
struct Reader {
    reader_password: String,
}
#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Notification {
    Open,
    Keepalive,
    Message { message: String },
}

fn until(description: &str, mut probe: impl FnMut() -> Result<bool>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(55);
    while !probe()? {
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn configured_history(settings: &str) -> Result<PathBuf> {
    let name = settings
        .lines()
        .find_map(|line| line.strip_prefix("database = \"/data/")?.strip_suffix('"'))
        .context("missing recorder database path")?;
    ensure!(
        Path::new(name).file_name().is_some_and(|file| file == name) && name.ends_with(".redb"),
        "invalid recorder database path"
    );
    Ok(Path::new(DATA).join(name))
}

fn timestamp(fragment: &str) -> Result<&str> {
    let (_, rest) = fragment
        .split_once("<time datetime=\"")
        .context("missing observation time")?;
    let (value, _) = rest
        .split_once('"')
        .context("incomplete observation time")?;
    ensure!(
        value.len() == 20 && value.ends_with('Z'),
        "expected a UTC timestamp in seconds"
    );
    Ok(value)
}

fn page(client: &Client) -> Result<String> {
    Ok(client.get(DASHBOARD).send()?.error_for_status()?.text()?)
}

fn live(body: &str) -> bool {
    body.contains("45<small>％</small>") && !body.contains("<p class=\"status\">受信途絶")
}

fn imported_time(body: &str) -> Result<&str> {
    let row = body
        .split("<tr>")
        .find(|row| {
            row.split("</tr>")
                .next()
                .is_some_and(|cells| cells.contains("<td>43％</td>"))
        })
        .context("missing imported humidity sample")?;
    timestamp(row)
}

fn seed_history(path: &Path) -> Result<()> {
    ensure!(!path.exists(), "expected a fresh rehearsal history");
    let sql = "CREATE TABLE monitor(id INTEGER PRIMARY KEY,address TEXT,started INTEGER,received INTEGER,temperature INTEGER,humidity INTEGER,battery INTEGER,pending_since INTEGER,high INTEGER,lost INTEGER);
        CREATE TABLE samples(minute INTEGER PRIMARY KEY,received INTEGER,temperature INTEGER,humidity INTEGER,battery INTEGER);
        CREATE TABLE outbox(id INTEGER PRIMARY KEY AUTOINCREMENT,kind TEXT,at INTEGER);
        INSERT INTO monitor VALUES(1,'AA:BB:CC:DD:EE:FF',unixepoch()-600,unixepoch()-240,225,43,99,NULL,0,0);
        INSERT INTO samples SELECT received/60,received,temperature,humidity,battery FROM monitor;";
    let status = Command::new("runuser")
        .args(["-u", "apps", "--", "sqlite3", "-bail"])
        .arg(path)
        .arg(sql)
        .status()?;
    ensure!(status.success(), "seed the previous receiver format");
    Ok(())
}

fn check_access(client: &Client) -> Result<()> {
    let account = Command::new("id").args(["-u", "apps"]).output()?;
    ensure!(account.status.success(), "read the app account UID");
    let uid = std::str::from_utf8(&account.stdout)?.trim();
    let app_uid: u32 = uid.parse()?;
    ensure!(app_uid > 0, "the app account must be unprivileged");
    let runtime = format!("XDG_RUNTIME_DIR=/run/user/{uid}");
    let pid = Command::new("runuser")
        .args([
            "-u",
            "apps",
            "--",
            "env",
            "--chdir=/var/lib/apps",
            &runtime,
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
        fs::metadata(configured_history(&fs::read_to_string(CONFIG)?)?)?.uid() == app_uid
            && uids.split_whitespace().eq([uid; 4]),
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
    RadioMode::Normal.publish(Path::new(MODE))?;
    until("a live humidity reading", || {
        Ok(page(client).is_ok_and(|body| live(&body)))
    })?;
    check_access(client)?;
    let subscription = client
        .get(TOPIC)
        .basic_auth("iphone", Some(&reader.reader_password))
        .timeout(Duration::from_secs(100))
        .send()?
        .error_for_status()?;
    let mut lines = BufReader::new(subscription).lines();
    let mut next = || -> Result<Notification> {
        Ok(serde_json::from_str(
            &lines.next().context("subscription closed")??,
        )?)
    };
    ensure!(
        matches!(next()?, Notification::Open),
        "subscriber did not open"
    );
    for (mode, expected) in [
        (RadioMode::High, "高湿度"),
        (RadioMode::Normal, "高湿度から復旧"),
        (RadioMode::Silent, "受信途絶"),
        (RadioMode::Normal, "受信再開"),
    ] {
        let title = format!("カメラ収納　{expected}");
        mode.publish(Path::new(MODE))?;
        loop {
            if let Notification::Message { message } = next()?
                && message.lines().next() == Some(title.as_str())
            {
                break;
            }
        }
        println!("Received {} transition: {expected}", mode.as_str());
    }
    let body = page(client)?;
    let received = timestamp(&body)?;
    let sample = imported_time(&body)?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    fs::write(RESTART, format!("{} {received} {sample}", boot.trim()))?;
    Ok(())
}

fn resumed(client: &Client) -> Result<()> {
    let saved = fs::read_to_string(RESTART)?;
    let mut values = saved.split_whitespace();
    let boot = values.next().context("missing saved boot ID")?;
    let previous = values.next().context("missing reception time")?;
    let sample = values.next().context("missing first minute sample")?;
    ensure!(
        boot != fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim(),
        "guest has not restarted"
    );
    RadioMode::Normal.publish(Path::new(MODE))?;
    until("a fresh reading after reboot", || {
        let Ok(body) = page(client) else {
            return Ok(false);
        };
        Ok(live(&body) && timestamp(&body)? > previous)
    })?;
    ensure!(
        imported_time(&page(client)?)? == sample,
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
    let old_path = configured_history(&settings)?;
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let file = format!("rehearsal-{run}.redb");
    settings = settings.replace(
        "database = \"/data/readings.redb\"",
        &format!("database = \"/data/{file}\""),
    );
    let path = configured_history(&settings)?;
    ensure!(path != old_path, "expected the production redb path");
    seed_history(&path.with_extension("sqlite"))?;
    fs::write(CONFIG, settings)?;
    RadioMode::Normal.publish(Path::new(MODE))?;
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
    let client = Client::builder().timeout(Duration::from_secs(12)).build()?;
    match std::env::args().nth(1).as_deref() {
        Some("shorten-timers") => shorten_timers(),
        Some("exercise") => exercise(&client, &credentials.secrets),
        Some("resumed") => resumed(&client),
        _ => bail!("expected shorten-timers, exercise or resumed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn the_saved_sample_uses_its_own_row_and_rejects_an_unrelated_humidity() -> Result<()> {
        let html = r#"<time datetime="2026-01-01T00:03:00Z"></time><tr><td><time datetime="2026-01-01T00:02:00Z"></time></td><td>45％</td></tr><tr><td><time datetime="2026-01-01T00:01:00Z"></time></td><td>43％</td></tr>"#;
        assert_eq!(timestamp(html)?, "2026-01-01T00:03:00Z");
        assert_eq!(imported_time(html)?, "2026-01-01T00:01:00Z");
        assert!(imported_time(&html.replace("43％", "41％")).is_err());
        assert!(configured_history("database = \"/data/../wrong.redb\"").is_err());
        Ok(())
    }

    #[test]
    fn readers_keep_a_complete_mode_during_a_transition() -> Result<()> {
        let file = std::env::temp_dir().join(format!("humidity-mode-{}", std::process::id()));
        RadioMode::Normal.publish(&file)?;
        let mut previous = fs::File::open(&file)?;
        RadioMode::High.publish(&file)?;
        let mut observed = String::new();
        previous.read_to_string(&mut observed)?;
        let next = fs::read_to_string(&file)?;
        fs::remove_file(file)?;
        assert_eq!((observed.as_str(), next.as_str()), ("normal", "high"));
        Ok(())
    }
}

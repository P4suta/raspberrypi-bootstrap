use super::{
    Secrets, Session, client, configure_delivery, configure_rules, notification_checks,
    private_read, private_write, random_text, string,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

const ROOT: &str = "/workspace/fixture";
const EMAIL: &str = "preflight@example.invalid";
const SYSTEM: &str = "Preflight Linux";
const HUB: &str = "http://127.0.0.1:8090/";
const NTFY: &str = "http://127.0.0.1:8081/";
const MACHINE: &str = "monitoring-preflight";
const MISE_SHA256: &str = "ae1151a87d70146119f68f7378001a12f09d2a38853087ed6139b7527a55f148";
const AGENT_SHA256: &str = "bd6eed7a98493e0900c3398177552a53809546733ea69a8ad200e2cf4c7b677c";

fn validate_machine(source: &Path) -> Result<()> {
    let output = Command::new("orb")
        .args(["info", MACHINE, "--format", "json"])
        .output()?;
    ensure!(
        output.status.success(),
        "initialize the isolated rehearsal machine first"
    );
    let info: Value = serde_json::from_slice(&output.stdout)?;
    let record = &info["record"];
    let config = &record["config"];
    let mount = fs::canonicalize(source.join(".local/preflight"))?;
    ensure!(
        record["image"]["distro"] == "debian"
            && record["image"]["version"] == "trixie"
            && record["image"]["arch"] == "arm64",
        "rehearsal requires ARM64 Debian 13"
    );
    ensure!(
        config["isolated"] == true
            && config["isolate_network"] == true
            && config["forward_ssh_agent"] == false,
        "rehearsal must isolate the home, private network, and SSH agent"
    );
    ensure!(
        config["mounts"] == json!([{"source":mount,"destination":"/workspace"}]),
        "rehearsal must expose only its disposable workspace"
    );
    ensure!(
        config["cpu_limit"] == 2
            && config["memory_limit_mib"] == 4096
            && config["disk_limit_bytes"] == 12_884_901_888_u64,
        "rehearsal resource limits changed"
    );
    Ok(())
}

pub(super) fn initialize(mise: &Path, agent: &Path) -> Result<()> {
    ensure!(
        std::env::consts::OS == "macos",
        "the rehearsal initializer requires OrbStack on macOS"
    );
    let source = std::env::current_dir()?;
    let workspace = source.join(".local/preflight");
    let tools = workspace.join("tools");
    fs::create_dir_all(&tools)?;
    for (binary, name, expected) in [
        (mise, "mise", MISE_SHA256),
        (agent, "beszel-agent", AGENT_SHA256),
    ] {
        let output = Command::new("shasum")
            .args(["-a", "256"])
            .arg(binary)
            .output()?;
        ensure!(output.status.success(), "rehearsal binary checksum failed");
        ensure!(
            String::from_utf8(output.stdout)?.split_whitespace().next() == Some(expected),
            "rehearsal binary does not match its pinned release"
        );
        fs::copy(binary, tools.join(name))?;
        fs::set_permissions(tools.join(name), fs::Permissions::from_mode(0o755))?;
    }
    let exists = Command::new("orb")
        .args(["info", MACHINE, "--format", "json"])
        .output()?;
    if !exists.status.success() {
        let mount = format!("{}:/workspace", fs::canonicalize(&workspace)?.display());
        command(
            "orb",
            &[
                "create",
                "--isolated",
                "--isolate-network",
                "--arch",
                "arm64",
                "--cpus",
                "2",
                "--memory",
                "4G",
                "--disk",
                "12G",
                "--mount",
                &mount,
                "debian:13",
                MACHINE,
            ],
        )?;
    }
    validate_machine(&source)?;
    println!("Isolated ARM64 Debian rehearsal machine and pinned tools are ready");
    Ok(())
}

fn copy(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "rehearsal sources must not contain symlinks"
    );
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn orb(arguments: &[&str]) -> Result<()> {
    println!("Local rehearsal: {}", arguments.join(" "));
    command(
        "orb",
        &[
            &[
                "-m",
                "monitoring-preflight",
                "-u",
                "root",
                "-w",
                "/workspace/source",
            ],
            arguments,
        ]
        .concat(),
    )
}

pub(super) fn local() -> Result<()> {
    let source = std::env::current_dir()?;
    crate::check(&source)?;
    validate_machine(&source)?;
    let staged = source.join(".local/preflight/source");
    fs::create_dir_all(&staged)?;
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "xtask",
        "justfile",
        "mise.apps.toml",
        "mise.monitoring.toml",
        "mise.armhf.toml",
        "mise.arm64.toml",
        "mise.sites.toml",
        "repository.toml",
        "apps",
        "config",
        "verification",
        "boot",
        "image",
    ] {
        copy(&source.join(name), &staged.join(name))?;
    }
    fs::copy(
        source.join("verification/preflight/mise.toml"),
        staged.join("mise.toml"),
    )?;
    orb(&[
        "env",
        "MISE_ENV=apps,monitoring",
        "/workspace/tools/mise",
        "bootstrap",
        "--yes",
        "--only",
        "accounts,packages,files,services",
    ])?;
    orb(&["/workspace/tools/mise", "install"])?;
    orb(&[
        "/workspace/tools/mise",
        "x",
        "--",
        "cargo",
        "build",
        "--locked",
    ])?;
    orb(&["apt-get", "remove", "-y", "aardvark-dns", "nftables"])?;
    let rejected = Command::new("orb")
        .args([
            "-m",
            "monitoring-preflight",
            "-u",
            "root",
            "-w",
            "/workspace/source",
            "target/debug/raspberrypi-maintenance",
            "fixture",
            "tools",
        ])
        .status()?;
    ensure!(
        !rejected.success(),
        "missing network dependencies must fail before a deployment"
    );
    orb(&[
        "env",
        "MISE_ENV=apps,monitoring",
        "/workspace/tools/mise",
        "bootstrap",
        "--yes",
        "--only",
        "packages",
    ])?;
    orb(&["target/debug/raspberrypi-maintenance", "fixture", "tools"])?;
    for action in [
        "prepare",
        "seed",
        "pagination",
        "check",
        "reader-password",
        "alerts",
        "restore",
        "checkpoint",
    ] {
        orb(&["target/debug/raspberrypi-maintenance", "fixture", action])?;
    }
    command("orb", &["restart", "monitoring-preflight"])?;
    orb(&["target/debug/raspberrypi-maintenance", "fixture", "resumed"])?;
    println!(
        "Isolated ARM64 deployment rehearsal passed, including missing dependencies, alert delivery, backup restoration, and reboot persistence"
    );
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Fixture {
    account_password: String,
    secrets: Secrets,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    public_key: String,
    system_id: String,
    statistics: Vec<String>,
    boot_id: String,
}

fn command(program: &str, arguments: &[&str]) -> Result<()> {
    let status = Command::new(program).args(arguments).status()?;
    ensure!(
        status.success(),
        "fixture command {program} failed with {status}"
    );
    Ok(())
}

fn app_command(arguments: &[&str]) -> Result<Command> {
    let uid = Command::new("id").args(["-u", "apps"]).output()?;
    ensure!(uid.status.success(), "fixture app account is missing");
    let runtime = format!(
        "XDG_RUNTIME_DIR=/run/user/{}",
        String::from_utf8(uid.stdout)?.trim()
    );
    let bus = format!(
        "DBUS_SESSION_BUS_ADDRESS=unix:path={}/bus",
        runtime.trim_start_matches("XDG_RUNTIME_DIR=")
    );
    let mut command = Command::new("runuser");
    command
        .args([
            "-u",
            "apps",
            "--",
            "env",
            "--chdir=/var/lib/apps",
            &runtime,
            &bus,
        ])
        .args(arguments);
    Ok(command)
}

fn app(arguments: &[&str]) -> Result<()> {
    let status = app_command(arguments)?.status()?;
    ensure!(status.success(), "fixture app command failed with {status}");
    Ok(())
}

fn app_output(arguments: &[&str]) -> Result<String> {
    let output = app_command(arguments)?.output()?;
    ensure!(output.status.success(), "fixture inspection failed");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn fixture() -> Result<Fixture> {
    Ok(serde_json::from_slice(&private_read(
        &Path::new(ROOT).join("state.json"),
    )?)?)
}

fn prepare() -> Result<()> {
    fs::create_dir_all(ROOT)?;
    fs::set_permissions(ROOT, fs::Permissions::from_mode(0o700))?;
    let path = Path::new(ROOT).join("state.json");
    if !path.exists() {
        let fixture = Fixture {
            account_password: random_text(32)?,
            secrets: Secrets {
                publisher_password: random_text(32)?,
                reader_password: random_text(32)?,
                publisher_token: format!("tk_{}", random_text(29)?),
                pi_token: random_text(48)?,
                nas_token: None,
            },
        };
        private_write(&path, &serde_json::to_vec_pretty(&fixture)?)?;
    }
    let fixture = fixture()?;
    private_write(&Path::new(ROOT).join("hub.env"), b"APP_URL=http://127.0.0.1:8090\nDISABLE_SSH=true\nCHECK_UPDATES=false\nCONTAINER_DETAILS=false\n")?;
    let publisher = bcrypt::hash(&fixture.secrets.publisher_password, 12)?;
    let reader = bcrypt::hash(&fixture.secrets.reader_password, 12)?;
    let ntfy = format!(
        "NTFY_BASE_URL={NTFY}\nNTFY_UPSTREAM_BASE_URL=\nNTFY_AUTH_USERS=beszel:{publisher}:user,iphone:{reader}:user\nNTFY_AUTH_ACCESS=beszel:home-status:wo,iphone:home-status:ro\nNTFY_AUTH_TOKENS=beszel:{}:Beszel\n",
        fixture.secrets.publisher_token
    );
    private_write(&Path::new(ROOT).join("ntfy.env"), ntfy.as_bytes())?;
    command(
        "install",
        &[
            "-o",
            "apps",
            "-g",
            "apps",
            "-m",
            "600",
            "/workspace/fixture/hub.env",
            "/workspace/fixture/ntfy.env",
            "/var/lib/apps/.config/monitoring/",
        ],
    )?;
    command(
        "install",
        &[
            "-o",
            "apps",
            "-g",
            "apps",
            "-m",
            "755",
            "/workspace/tools/beszel-agent",
            "/var/lib/apps/monitoring/bin/beszel-agent",
        ],
    )?;
    command("loginctl", &["enable-linger", "apps"])?;
    let uid = Command::new("id").args(["-u", "apps"]).output()?;
    ensure!(uid.status.success(), "fixture app account is missing");
    let manager = format!("user@{}.service", String::from_utf8(uid.stdout)?.trim());
    command("systemctl", &["start", &manager])?;
    app(&["systemctl", "--user", "daemon-reload"])?;
    app(&[
        "systemctl",
        "--user",
        "start",
        "monitoring-beszel.service",
        "monitoring-ntfy.service",
    ])?;
    println!(
        "Isolated fixture services started with synthetic credentials and no notification upstream"
    );
    Ok(())
}

fn session() -> Result<Session> {
    let fixture = fixture()?;
    let client = client()?;
    let base = Url::parse(HUB)?;
    wait_http(&base.join("api/health")?)?;
    let first = client
        .get(base.join("api/beszel/first-run")?)
        .send()?
        .error_for_status()?
        .json::<Value>()?;
    if first["firstRun"] == true {
        client
            .post(base.join("api/beszel/create-user")?)
            .json(&json!({"email":EMAIL,"password":fixture.account_password}))
            .send()?
            .error_for_status()?;
    }
    let auth = client
        .post(base.join("api/collections/users/auth-with-password")?)
        .json(&json!({"identity":EMAIL,"password":fixture.account_password}))
        .send()?
        .error_for_status()?
        .json::<Value>()?;
    Ok(Session {
        client,
        base,
        token: string(&auth, "token")?.to_owned(),
        user_id: string(&auth["record"], "id")?.to_owned(),
    })
}

fn wait_http(url: &Url) -> Result<()> {
    let client = client()?;
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        if client
            .get(url.clone())
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    bail!("fixture HTTP service did not become ready before the deadline")
}

fn key(session: &Session) -> Result<String> {
    let info = session.request(Method::GET, "api/beszel/info", None)?;
    Ok(string(&info, "key")?.to_owned())
}

fn seed() -> Result<()> {
    let fixture = fixture()?;
    let session = session()?;
    if session.records("user_settings")?.is_empty() {
        session.create("user_settings", &json!({"user":session.user_id,"settings":{"chartTime":"1h","emails":[],"webhooks":[]}}))?;
    }
    session.system(SYSTEM, "127.0.0.1", &fixture.secrets.pi_token)?;
    let public_key = key(&session)?;
    ensure!(
        public_key.starts_with("ssh-ed25519 "),
        "fixture hub key is missing"
    );
    let environment = format!(
        "HUB_URL={HUB}\nKEY={public_key}\nTOKEN={}\nDISABLE_SSH=true\nSKIP_GPU=true\nSKIP_SYSTEMD=true\nSKIP_SYSTEMD_LOGS=true\nSMART_DEVICES=\nDOCKER_HOST=\n",
        fixture.secrets.pi_token
    );
    private_write(
        &Path::new(ROOT).join("pi-agent.env"),
        environment.as_bytes(),
    )?;
    command(
        "install",
        &[
            "-o",
            "apps",
            "-g",
            "apps",
            "-m",
            "600",
            "/workspace/fixture/pi-agent.env",
            "/var/lib/apps/.config/monitoring/pi-agent.env",
        ],
    )?;
    command("systemctl", &["daemon-reload"])?;
    command(
        "systemctl",
        &["enable", "--now", "monitoring-agent.service"],
    )?;
    wait_up(&session)?;
    println!("Native ARM64 agent connected to the isolated hub");
    Ok(())
}

fn wait_up(session: &Session) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if session
            .records("systems")?
            .iter()
            .any(|system| system["name"] == SYSTEM && system["status"] == "up")
            && !session.records("system_stats")?.is_empty()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    bail!("fixture agent did not publish native statistics before the deadline")
}

fn snapshot(session: &Session) -> Result<Snapshot> {
    let systems = session.records("systems")?;
    let system = systems
        .iter()
        .find(|system| system["name"] == SYSTEM)
        .context("fixture system is missing")?;
    let statistics = session
        .records("system_stats")?
        .iter()
        .map(|record| string(record, "id").map(str::to_owned))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        !statistics.is_empty(),
        "backup test needs recorded statistics"
    );
    Ok(Snapshot {
        public_key: key(session)?,
        system_id: string(system, "id")?.to_owned(),
        statistics,
        boot_id: fs::read_to_string("/proc/sys/kernel/random/boot_id")?,
    })
}

fn check() -> Result<()> {
    let fixture = fixture()?;
    let session = session()?;
    wait_up(&session)?;
    ensure!(
        app_output(&["podman", "info", "--format", "{{.Host.Security.Rootless}}"])? == "true",
        "fixture containers must be rootless"
    );
    for container in ["monitoring-beszel", "monitoring-ntfy"] {
        ensure!(
            app_output(&[
                "podman",
                "inspect",
                "--format",
                "{{.Config.User}}",
                container
            ])? == "10001:10001",
            "fixture container identity must remain unprivileged"
        );
    }
    let user = Command::new("systemctl")
        .args([
            "show",
            "--property=User",
            "--value",
            "monitoring-agent.service",
        ])
        .output()?;
    ensure!(
        user.status.success() && String::from_utf8(user.stdout)?.trim() == "apps",
        "native agent must run as apps"
    );
    notification_checks(Url::parse(NTFY)?, &fixture.secrets)?;
    super::hub_notification(&session, &fixture.secrets)?;
    let received = client()?
        .get(format!("{NTFY}home-status/json?poll=1&since=all"))
        .basic_auth("iphone", Some(&fixture.secrets.reader_password))
        .send()?
        .error_for_status()?
        .text()?;
    ensure!(
        received.contains("notification from Beszel"),
        "hub notification did not reach the private subscriber"
    );
    println!(
        "Native collection, internal DNS, hub delivery, anonymous denial, and separate publisher/subscriber permissions passed"
    );
    Ok(())
}

fn messages() -> Result<Vec<Value>> {
    let fixture = fixture()?;
    let text = client()?
        .get(format!("{NTFY}home-status/json?poll=1&since=all"))
        .basic_auth("iphone", Some(&fixture.secrets.reader_password))
        .send()?
        .error_for_status()?
        .text()?;
    text.lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn wait_notification(previous: &BTreeSet<String>, message: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(150);
    while Instant::now() < deadline {
        if messages()?.iter().any(|item| {
            item["id"].as_str().is_some_and(|id| !previous.contains(id))
                && ["message", "title"].iter().any(|field| {
                    item[field]
                        .as_str()
                        .is_some_and(|text| text.contains(message))
                })
        }) {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(5));
    }
    bail!("expected fixture alert did not arrive before the deadline: {message}")
}

fn alerts() -> Result<()> {
    let fixture = fixture()?;
    let session = session()?;
    wait_up(&session)?;
    configure_delivery(&session, &fixture.secrets)?;
    let system_id = snapshot(&session)?.system_id;
    let rule = json!({"name":"Status","value":0,"min":1,"system":system_id,"user":session.user_id});
    let existing = session.records("alerts")?;
    if let Some(alert) = existing
        .iter()
        .find(|item| item["system"] == system_id && item["name"] == "Status")
    {
        session.update("alerts", string(alert, "id")?, &rule)?;
    } else {
        session.create("alerts", &rule)?;
    }
    let previous = messages()?
        .iter()
        .map(|item| string(item, "id").map(str::to_owned))
        .collect::<Result<BTreeSet<_>>>()?;
    println!(
        "Local alert test: stopping the disposable agent and waiting for the one-minute threshold"
    );
    command("systemctl", &["stop", "monitoring-agent.service"])?;
    let down = wait_notification(&previous, "Connection to Preflight Linux is down");
    let restart = command("systemctl", &["start", "monitoring-agent.service"]);
    down?;
    restart?;
    wait_up(&session)?;
    wait_notification(&previous, "Connection to Preflight Linux is up")?;
    println!("Actual disconnect and recovery alerts reached the private subscriber");
    let disk_rule = json!({"name":"Disk","value":1,"min":1,"triggered":false,"system":system_id,"user":session.user_id});
    let disk = match existing
        .iter()
        .find(|item| item["system"] == system_id && item["name"] == "Disk")
    {
        Some(alert) => session.update("alerts", string(alert, "id")?, &disk_rule)?,
        None => session.create("alerts", &disk_rule)?,
    };
    println!("Local alert test: applying a low disk threshold to the disposable filesystem");
    wait_notification(&previous, "Preflight Linux disk usage above threshold")?;
    session.update("alerts", string(&disk, "id")?, &json!({"value":100}))?;
    wait_notification(&previous, "Preflight Linux disk usage below threshold")?;
    println!("Actual disk threshold and recovery alerts reached the private subscriber");
    retirement()
}

fn assert_snapshot(previous: &Snapshot, actual: &Snapshot) -> Result<()> {
    ensure!(
        actual.public_key == previous.public_key && actual.system_id == previous.system_id,
        "restored fixture identity changed"
    );
    ensure!(
        previous
            .statistics
            .iter()
            .all(|id| actual.statistics.contains(id)),
        "restored fixture lost recorded statistics"
    );
    Ok(())
}

fn retirement() -> Result<()> {
    let session = session()?;
    let targets = [SYSTEM, "Preflight Mac", "Preflight Windows", "Preflight Pi"].map(str::to_owned);
    let retired = "Preflight NAS";
    let existing = session.records("systems")?;
    for name in targets.iter().map(String::as_str).chain([retired]) {
        if !existing.iter().any(|item| item["name"] == name) {
            session.system(name, "127.0.0.1", &random_text(48)?)?;
        }
    }
    let mut previous_targets = targets.to_vec();
    previous_targets.push(retired.to_owned());
    configure_rules(
        &session,
        &previous_targets,
        &[SYSTEM.to_owned(), retired.to_owned()],
    )?;
    let systems = session.records("systems")?;
    let retired_id = string(
        systems
            .iter()
            .find(|item| item["name"] == retired)
            .context("retirement fixture is missing")?,
        "id",
    )?;
    let retained_ids = |records: Vec<Value>, field: &str| -> Result<BTreeSet<String>> {
        records
            .iter()
            .filter(|item| item[field] != retired_id)
            .map(|item| string(item, "id").map(str::to_owned))
            .collect()
    };
    let system_ids = retained_ids(systems.clone(), "id")?;
    let alert_ids = retained_ids(session.records("alerts")?, "system")?;
    let fingerprint_ids = retained_ids(session.records("fingerprints")?, "system")?;
    session.retire_system(retired)?;
    session.retire_system(retired)?;
    ensure!(
        !session
            .records("systems")?
            .iter()
            .any(|item| item["id"] == retired_id),
        "retired NAS registration remains"
    );
    ensure!(
        retained_ids(session.records("systems")?, "id")? == system_ids,
        "retirement changed another system"
    );
    ensure!(
        retained_ids(session.records("alerts")?, "system")? == alert_ids,
        "retirement changed another alert"
    );
    ensure!(
        retained_ids(session.records("fingerprints")?, "system")? == fingerprint_ids,
        "retirement changed another registration identity"
    );
    for collection in ["alerts", "fingerprints"] {
        ensure!(
            !session
                .records(collection)?
                .iter()
                .any(|item| item["system"] == retired_id),
            "retired NAS still has {collection}"
        );
    }
    configure_rules(&session, &targets, &targets[..1])?;
    super::verify_rules(&session, &targets, &targets[..1])?;
    ensure!(
        session.records("systems")?.len() == 4,
        "retirement must leave four systems"
    );
    ensure!(
        session.records("alerts")?.len() == 5,
        "retirement must leave four disk alerts and one disconnect alert"
    );
    println!(
        "NAS retirement preserved four systems and removed its alerts and registration identity"
    );
    Ok(())
}

fn pagination() -> Result<()> {
    let session = session()?;
    let initial = session
        .records("systems")?
        .iter()
        .map(|item| string(item, "id").map(str::to_owned))
        .collect::<Result<BTreeSet<_>>>()?;
    let mut created = Vec::new();
    let result = (|| -> Result<()> {
        for index in 0..101 {
            let record = session.create("systems", &json!({"name":format!("Preflight pagination {index}"), "host":"127.0.0.1", "port":"45876", "status":"paused", "users":[session.user_id]}))?;
            created.push(string(&record, "id")?.to_owned());
        }
        let records = session.records("systems")?;
        let actual = records
            .iter()
            .map(|item| string(item, "id").map(str::to_owned))
            .collect::<Result<BTreeSet<_>>>()?;
        ensure!(
            records.len() == initial.len() + created.len()
                && actual.len() == records.len()
                && initial
                    .iter()
                    .chain(created.iter())
                    .all(|id| actual.contains(id)),
            "paginated reads lost or duplicated records"
        );
        Ok(())
    })();
    let mut cleanup_error = None;
    for id in &created {
        if let Err(error) = session.delete("systems", id) {
            cleanup_error.get_or_insert(error);
        }
    }
    if let Err(primary) = result {
        return Err(match cleanup_error {
            Some(cleanup) => primary.context(format!(
                "pagination fixture cleanup also failed: {cleanup:#}"
            )),
            None => primary,
        });
    }
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    let remaining = session
        .records("systems")?
        .iter()
        .map(|item| string(item, "id").map(str::to_owned))
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        initial == remaining,
        "pagination fixture changed an existing registration"
    );
    println!(
        "Paginated reads preserved every record across the 100-record page boundary; disposable registrations were removed"
    );
    Ok(())
}

fn reader_password() -> Result<()> {
    let fixture = fixture()?;
    let root = Path::new(ROOT).join("reader-password");
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    fs::create_dir(root.join("runtime"))?;
    fs::set_permissions(root.join("runtime"), fs::Permissions::from_mode(0o700))?;
    let config = root.join("client.json");
    private_write(
        &config,
        &serde_json::to_vec(&json!({
            "credentials_file":"unused", "public_key_file":"unused",
            "source_url":"https://fixture.example.ts.net", "hub_url":"https://fixture.example.ts.net",
            "ntfy_url":"https://fixture.example.ts.net", "pi_host":"fixture.example.ts.net",
            "required_systems":[SYSTEM,"Preflight Mac","Preflight Windows","Preflight Pi"], "offline_systems":[SYSTEM]
        }))?,
    )?;
    private_write(
        &root.join("secrets.json"),
        &serde_json::to_vec(&fixture.secrets)?,
    )?;
    private_write(
        &root.join("runtime/ntfy.env"),
        &private_read(&Path::new(ROOT).join("ntfy.env"))?,
    )?;
    private_write(
        &root.join("runtime/iphone.txt"),
        b"Synthetic phone configuration\n",
    )?;
    let mut password = "Aa0!$`'\"\\,:&|~".repeat(5);
    password.truncate(64);
    super::rotate_reader_password(&config, &password)?;
    let active = super::secrets(&config)?;
    let result = (|| -> Result<()> {
        command(
            "install",
            &[
                "-o",
                "apps",
                "-g",
                "apps",
                "-m",
                "600",
                root.join("runtime/ntfy.env")
                    .to_str()
                    .context("fixture path must be UTF-8")?,
                "/var/lib/apps/.config/monitoring/ntfy.env",
            ],
        )?;
        for _ in 0..2 {
            app(&["systemctl", "--user", "restart", "monitoring-ntfy.service"])?;
            wait_http(&Url::parse(NTFY)?.join("v1/health")?)?;
            super::verify_reader_rotation(Url::parse(NTFY)?, &fixture.secrets, &active)?;
        }
        super::hub_notification(&session()?, &active)?;
        Ok(())
    })();
    let restored = (|| -> Result<()> {
        command(
            "install",
            &[
                "-o",
                "apps",
                "-g",
                "apps",
                "-m",
                "600",
                "/workspace/fixture/ntfy.env",
                "/var/lib/apps/.config/monitoring/ntfy.env",
            ],
        )?;
        app(&["systemctl", "--user", "restart", "monitoring-ntfy.service"])?;
        wait_http(&Url::parse(NTFY)?.join("v1/health")?)?;
        notification_checks(Url::parse(NTFY)?, &fixture.secrets)
    })();
    if let Err(primary) = result {
        return match restored {
            Ok(()) => Err(primary),
            Err(cleanup) => {
                Err(primary.context(format!("fixture recovery also failed: {cleanup:#}")))
            }
        };
    }
    restored?;
    println!(
        "A 64-character reader password with punctuation survived service restarts; the old password was denied and publisher access was preserved"
    );
    Ok(())
}

fn checkpoint() -> Result<()> {
    let session = session()?;
    wait_up(&session)?;
    private_write(
        &Path::new(ROOT).join("checkpoint.json"),
        &serde_json::to_vec(&snapshot(&session)?)?,
    )?;
    Ok(())
}

fn resumed() -> Result<()> {
    let previous: Snapshot =
        serde_json::from_slice(&private_read(&Path::new(ROOT).join("checkpoint.json"))?)?;
    let session = session()?;
    wait_up(&session)?;
    let actual = snapshot(&session)?;
    assert_snapshot(&previous, &actual)?;
    ensure!(
        previous.boot_id != actual.boot_id,
        "reboot test needs a changed kernel boot identity"
    );
    ensure!(
        messages()?.iter().any(|item| item["message"]
            .as_str()
            .is_some_and(|text| text.contains("notification from Beszel"))),
        "ntfy cache did not survive the reboot"
    );
    check()?;
    println!(
        "Hub identity, accounts, statistics, agent startup, and private notification cache survived the local machine reboot"
    );
    Ok(())
}

fn restore() -> Result<()> {
    let initial = session()?;
    wait_up(&initial)?;
    let previous = snapshot(&initial)?;
    command("systemctl", &["stop", "monitoring-agent.service"])?;
    app(&["systemctl", "--user", "stop", "monitoring-beszel.service"])?;
    let mount = app_output(&[
        "podman",
        "volume",
        "inspect",
        "--format",
        "{{.Mountpoint}}",
        "monitoring-beszel",
    ])?;
    ensure!(
        mount == "/var/lib/apps/.local/share/containers/storage/volumes/monitoring-beszel/_data",
        "unexpected fixture volume location"
    );
    app(&["podman", "unshare", "chown", "-R", "501:20", &mount])?;
    app(&[
        "podman",
        "volume",
        "export",
        "--output",
        "/var/lib/apps/backups/preflight-beszel.tar",
        "monitoring-beszel",
    ])?;
    app(&["podman", "volume", "rm", "monitoring-beszel"])?;
    app(&["podman", "volume", "create", "monitoring-beszel"])?;
    app(&[
        "podman",
        "volume",
        "import",
        "monitoring-beszel",
        "/var/lib/apps/backups/preflight-beszel.tar",
    ])?;
    app(&["systemctl", "--user", "start", "monitoring-beszel.service"])?;
    command("systemctl", &["start", "monitoring-agent.service"])?;
    let restored = session()?;
    wait_up(&restored)?;
    let actual = snapshot(&restored)?;
    assert_snapshot(&previous, &actual)?;
    println!(
        "Stopped-volume export and import preserved the hub key, account, system identity, and recorded statistics"
    );
    Ok(())
}

pub(super) fn run(action: &str) -> Result<()> {
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .context("fixtures require the isolated Linux rehearsal machine")?;
    ensure!(
        kernel.to_ascii_lowercase().contains("orbstack"),
        "fixtures refuse physical-device kernels"
    );
    match action {
        "tools" => {
            command("/usr/sbin/nft", &["--version"])?;
            command("/usr/lib/podman/aardvark-dns", &["--version"])
        }
        "prepare" => prepare(),
        "seed" => seed(),
        "check" => check(),
        "alerts" => alerts(),
        "retirement" => retirement(),
        "pagination" => pagination(),
        "reader-password" => reader_password(),
        "restore" => restore(),
        "checkpoint" => checkpoint(),
        "resumed" => resumed(),
        _ => bail!("unknown fixture action"),
    }
}

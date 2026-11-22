use super::{AppOperation, AppService, Config, MemoryBootLine, restart_app};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const SOCKET: &str = "/var/run/tailscale/tailscaled.sock";
const API: &str = "http://local-tailscaled.sock/localapi/v0/serve-config";
const NETWORK: &str = "/etc/systemd/network/01-eth0.network.d/50-ht813-management.conf";
const BOOT: &str = "/boot/firmware/cmdline.txt";
const CONFIG: &str = "/etc/raspberrypi-maintenance/host.json";
const PENDING: &str = "/var/lib/raspberrypi-maintenance/pending-host-backup";

fn host_lock() -> Result<crate::OperationLock> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open("/run/raspberrypi-maintenance.lock")?;
    crate::OperationLock::acquire(file).context("another host maintenance operation is active")
}

struct FileChange {
    name: &'static str,
    path: PathBuf,
    content: Vec<u8>,
    mode: u32,
}

fn output(command: &mut Command) -> Result<Vec<u8>> {
    let result = command.output()?;
    ensure!(
        result.status.success(),
        "native command failed: {}",
        result.status
    );
    Ok(result.stdout)
}

fn identity(config: &Config) -> Result<()> {
    config.validate()?;
    ensure!(
        fs::read_to_string("/etc/machine-id")?.trim() == config.machine_id,
        "host configuration belongs to another machine"
    );
    let state: Value = serde_json::from_slice(&output(
        Command::new("/usr/bin/tailscale").args(["status", "--json"]),
    )?)?;
    ensure!(
        state["Self"]["DNSName"]
            .as_str()
            .is_some_and(|name| name.trim_end_matches('.') == config.tailnet_name),
        "tailnet identity differs from the host configuration"
    );
    Ok(())
}

fn config(path: &Path) -> Result<Config> {
    let config: Config = serde_json::from_slice(&fs::read(path)?)?;
    identity(&config)?;
    Ok(config)
}

fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn write_new(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(mode)
        .open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}

fn replace(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "managed file is not a regular file"
        );
    }
    let parent = path.parent().context("managed file has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".maintenance-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(mode)
        .open(&temporary)?;
    let mut operation = || -> Result<()> {
        file.write_all(content)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    };
    let result = operation();
    if result.is_err() && temporary.exists() {
        fs::remove_file(&temporary).context(
            "atomic file replacement failed and its temporary file could not be removed",
        )?;
    }
    result
}

fn files(config: &Config) -> Result<Vec<FileChange>> {
    Ok(vec![
        FileChange {
            name: "management-network",
            path: NETWORK.into(),
            content: format!(
                "[Address]\nAddress={}\nDuplicateAddressDetection=ipv4\n",
                config.network.address
            )
            .into_bytes(),
            mode: 0o644,
        },
        FileChange {
            name: "kernel-command-line",
            path: BOOT.into(),
            content: MemoryBootLine::parse(&fs::read_to_string(BOOT)?)?
                .0
                .into_bytes(),
            mode: 0o755,
        },
        FileChange {
            name: "host-config",
            path: CONFIG.into(),
            content: serde_json::to_vec_pretty(config)?,
            mode: 0o600,
        },
    ])
}

fn read_serve() -> Result<(Value, String)> {
    read_serve_at(SOCKET)
}

fn read_serve_at(socket: &str) -> Result<(Value, String)> {
    let response = output(Command::new("/usr/bin/curl").args([
        "--fail",
        "--silent",
        "--show-error",
        "--max-time",
        "10",
        "--noproxy",
        "*",
        "--unix-socket",
        socket,
        "--dump-header",
        "-",
        API,
    ]))?;
    let response = String::from_utf8(response)?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .context("invalid local API response")?;
    let tag = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("etag")
                .then(|| value.trim().to_owned())
        })
        .context("Serve API omitted its concurrency tag")?;
    ensure!(
        !tag.is_empty() && !tag.chars().any(char::is_control),
        "invalid concurrency tag"
    );
    Ok((serde_json::from_str(body)?, tag))
}

fn write_serve(value: &Value, tag: &str) -> Result<()> {
    write_serve_at(SOCKET, value, tag)
}

fn write_serve_at(socket: &str, value: &Value, tag: &str) -> Result<()> {
    let mut child = Command::new("/usr/bin/curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "10",
            "--noproxy",
            "*",
            "--unix-socket",
            socket,
            "--request",
            "POST",
            "--header",
            &format!("If-Match: {tag}"),
            "--header",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
            API,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .context("local API input is unavailable")?
        .write_all(&serde_json::to_vec(value)?)?;
    ensure!(
        child.wait()?.success(),
        "Serve update failed or its concurrency tag changed"
    );
    Ok(())
}

fn root() -> Result<()> {
    ensure!(
        output(Command::new("/usr/bin/id").arg("-u"))? == b"0\n",
        "host changes require maintenance access"
    );
    Ok(())
}

pub(crate) fn plan(path: &Path) -> Result<()> {
    let config = config(path)?;
    let changed = files(&config)?
        .into_iter()
        .filter(|file| fs::read(&file.path).ok().as_deref() != Some(file.content.as_slice()))
        .count();
    let (before, _) = read_serve()?;
    let after = config.serve_config(&before)?;
    println!(
        "Host plan: {changed} file changes; HTTPS routes need update: {}",
        before != after
    );
    Ok(())
}

pub(crate) fn apply(path: &Path, backup: &Path) -> Result<()> {
    root()?;
    let _lock = host_lock()?;
    ensure!(
        !Path::new(PENDING).exists(),
        "recover the pending host update before applying another configuration"
    );
    let config = config(path)?;
    let files = files(&config)?;
    let (before, tag) = read_serve()?;
    let after = config.serve_config(&before)?;
    private_directory(backup)?;
    write_new(
        &backup.join("desired-host.json"),
        &serde_json::to_vec_pretty(&config)?,
        0o600,
    )?;
    write_new(
        &backup.join("before-serve.json"),
        &serde_json::to_vec_pretty(&before)?,
        0o600,
    )?;
    write_new(
        &backup.join("applied-serve.json"),
        &serde_json::to_vec_pretty(&after)?,
        0o600,
    )?;
    for file in &files {
        match fs::symlink_metadata(&file.path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "cannot back up a nonregular managed file"
                );
                write_new(&backup.join(file.name), &fs::read(&file.path)?, 0o600)?;
                write_new(
                    &backup.join(format!("{}.mode", file.name)),
                    format!("{}", metadata.permissions().mode() & 0o777).as_bytes(),
                    0o600,
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                write_new(&backup.join(format!("{}.absent", file.name)), b"", 0o600)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    write_new(&backup.join("pending"), b"", 0o600)?;
    fs::create_dir_all(
        Path::new(PENDING)
            .parent()
            .context("pending update has no parent")?,
    )?;
    write_new(
        Path::new(PENDING),
        fs::canonicalize(backup)?
            .to_str()
            .context("backup path must be UTF-8")?
            .as_bytes(),
        0o600,
    )?;
    let change = || -> Result<()> {
        for file in &files {
            replace(&file.path, &file.content, file.mode)?;
        }
        output(Command::new("/usr/bin/networkctl").arg("reload"))?;
        if before != after {
            write_serve(&after, &tag)?;
        }
        let (actual, _) = read_serve()?;
        ensure!(
            config.serve_config(&actual)? == actual,
            "HTTPS route readback failed"
        );
        fs::rename(backup.join("pending"), backup.join("complete"))?;
        fs::remove_file(PENDING)?;
        Ok(())
    };
    if let Err(error) = change() {
        return match restore_locked(backup) {
            Ok(()) => Err(error.context("host change failed; previous configuration restored")),
            Err(recovery) => Err(error.context(format!(
                "host change failed; recovery also failed: {recovery}"
            ))),
        };
    }
    println!(
        "Managed host files and private HTTPS routes match the declarations; verify memory accounting after reboot"
    );
    Ok(())
}

pub(crate) fn restore(backup: &Path) -> Result<()> {
    root()?;
    let _lock = host_lock()?;
    restore_locked(backup)
}

fn restore_locked(backup: &Path) -> Result<()> {
    let config: Config = serde_json::from_slice(&fs::read(backup.join("desired-host.json"))?)?;
    config.validate()?;
    ensure!(
        fs::read_to_string("/etc/machine-id")?.trim() == config.machine_id,
        "host backup belongs to another machine"
    );
    if Path::new(PENDING).exists() {
        ensure!(
            fs::canonicalize(backup)? == Path::new(fs::read_to_string(PENDING)?.trim()),
            "a different host update is pending"
        );
    }
    let before: Value = serde_json::from_slice(&fs::read(backup.join("before-serve.json"))?)?;
    let applied: Value = serde_json::from_slice(&fs::read(backup.join("applied-serve.json"))?)?;
    for (name, path) in [
        ("management-network", NETWORK),
        ("kernel-command-line", BOOT),
        ("host-config", CONFIG),
    ] {
        let path = Path::new(path);
        if backup.join(format!("{name}.absent")).exists() {
            if path.exists() {
                fs::remove_file(path)?;
            }
        } else {
            let mode: u32 = fs::read_to_string(backup.join(format!("{name}.mode")))?.parse()?;
            replace(path, &fs::read(backup.join(name))?, mode)?;
        }
    }
    output(Command::new("/usr/bin/networkctl").arg("reload"))?;
    let (current, tag) = read_serve().context(
        "host files restored; HTTPS recovery remains pending until Tailscale is available",
    )?;
    let restored = config.restore_routes(&before, &applied, &current)?;
    if current != restored {
        write_serve(&restored, &tag)?;
    }
    if backup.join("pending").exists() {
        fs::rename(backup.join("pending"), backup.join("restored"))?;
    }
    if Path::new(PENDING).exists() {
        fs::remove_file(PENDING)?;
    }
    println!("Previous host configuration restored without changing unrelated HTTPS routes");
    Ok(())
}

pub(crate) fn recover() -> Result<()> {
    let backup = fs::read_to_string(PENDING).context("no pending host update was found")?;
    restore(Path::new(backup.trim()))
}

pub(crate) fn check(path: &Path) -> Result<()> {
    ensure!(
        !Path::new(PENDING).exists(),
        "a host update is pending recovery"
    );
    let config = config(path)?;
    for file in files(&config)? {
        ensure!(
            fs::read(file.path)? == file.content,
            "managed host file differs from its declaration"
        );
    }
    let (actual, _) = read_serve()?;
    ensure!(
        config.serve_config(&actual)? == actual,
        "private HTTPS routes differ"
    );
    let controllers = fs::read_to_string("/sys/fs/cgroup/cgroup.controllers")?;
    ensure!(
        controllers.split_whitespace().any(|name| name == "memory"),
        "memory accounting requires a verified reboot"
    );
    let peer: std::net::Ipv4Addr = config.network.peer.parse()?;
    let endpoint = std::net::SocketAddr::from((peer, 80));
    let connection =
        std::net::TcpStream::connect_timeout(&endpoint, std::time::Duration::from_secs(3))?;
    let source: std::net::IpAddr = config
        .network
        .address
        .split_once('/')
        .context("missing management prefix")?
        .0
        .parse()?;
    ensure!(
        connection.local_addr()?.ip() == source,
        "management traffic uses a dynamic source address"
    );
    println!("Host identity, managed files, private routes, and memory controller checks passed");
    Ok(())
}

pub(crate) fn bootline(path: &Path) -> Result<()> {
    let current = fs::read_to_string(path)?;
    let updated = MemoryBootLine::parse(&current)?.0;
    if current != updated {
        let mode = fs::metadata(path)?.permissions().mode() & 0o777;
        replace(path, updated.as_bytes(), mode)?;
    }
    Ok(())
}

fn app(arguments: &[&str]) -> Result<Command> {
    let uid = String::from_utf8(output(Command::new("/usr/bin/id").args(["-u", "apps"]))?)?;
    let uid: u32 = uid.trim().parse()?;
    ensure!(uid > 0, "app account must remain unprivileged");
    let mut command = Command::new("/usr/sbin/runuser");
    command
        .args([
            "-u",
            "apps",
            "--",
            "/usr/bin/env",
            "--chdir=/var/lib/apps",
            &format!("XDG_RUNTIME_DIR=/run/user/{uid}"),
            &format!("DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus"),
        ])
        .args(arguments);
    Ok(command)
}

pub(crate) fn agent_migrate(backup: &Path) -> Result<()> {
    root()?;
    let legacy = Path::new("/etc/systemd/system/monitoring-agent.service");
    if !legacy.exists() {
        return Ok(());
    }
    output(&mut app(&[
        "/usr/bin/systemd-analyze",
        "--user",
        "verify",
        "--man=no",
        "monitoring-agent.service",
    ])?)?;
    private_directory(backup)?;
    write_new(
        &backup.join("machine-id"),
        fs::read_to_string("/etc/machine-id")?.as_bytes(),
        0o600,
    )?;
    write_new(&backup.join("agent.service"), &fs::read(legacy)?, 0o600)?;
    for (marker, operation) in [("enabled", "is-enabled"), ("active", "is-active")] {
        if Command::new("/usr/bin/systemctl")
            .args([operation, "--quiet", "monitoring-agent.service"])
            .status()?
            .success()
        {
            write_new(&backup.join(marker), b"", 0o600)?;
        }
    }
    write_new(&backup.join("pending"), b"", 0o600)?;
    let result = || -> Result<()> {
        output(Command::new("/usr/bin/systemctl").args([
            "disable",
            "--now",
            "monitoring-agent.service",
        ]))?;
        output(&mut app(&[
            "/usr/bin/systemctl",
            "--user",
            "daemon-reload",
        ])?)?;
        output(&mut app(&[
            "/usr/bin/systemctl",
            "--user",
            "enable",
            "--now",
            "monitoring-agent.service",
        ])?)?;
        output(&mut app(&[
            "/usr/bin/systemctl",
            "--user",
            "is-active",
            "--quiet",
            "monitoring-agent.service",
        ])?)?;
        fs::remove_file(legacy)?;
        output(Command::new("/usr/bin/systemctl").arg("daemon-reload"))?;
        fs::rename(backup.join("pending"), backup.join("complete"))?;
        Ok(())
    }();
    if let Err(error) = result {
        return match agent_restore(backup) {
            Ok(()) => Err(error.context("agent migration failed; system service restored")),
            Err(restore) => {
                Err(error.context(format!("agent migration and restoration failed: {restore}")))
            }
        };
    }
    println!("Metrics agent moved to the unprivileged user service manager");
    Ok(())
}

pub(crate) fn agent_restore(backup: &Path) -> Result<()> {
    root()?;
    ensure!(
        fs::read(backup.join("machine-id"))? == fs::read("/etc/machine-id")?,
        "agent backup belongs to another host"
    );
    output(&mut app(&[
        "/usr/bin/systemctl",
        "--user",
        "disable",
        "--now",
        "monitoring-agent.service",
    ])?)?;
    replace(
        Path::new("/etc/systemd/system/monitoring-agent.service"),
        &fs::read(backup.join("agent.service"))?,
        0o644,
    )?;
    output(Command::new("/usr/bin/systemctl").arg("daemon-reload"))?;
    if backup.join("enabled").exists() {
        output(Command::new("/usr/bin/systemctl").args(["enable", "monitoring-agent.service"]))?;
    }
    if backup.join("active").exists() {
        output(Command::new("/usr/bin/systemctl").args(["start", "monitoring-agent.service"]))?;
    }
    if backup.join("pending").exists() {
        fs::rename(backup.join("pending"), backup.join("restored"))?;
    }
    println!("Previous metrics agent system service restored");
    Ok(())
}

fn operator_command(program: &str, arguments: &[&str]) -> Result<Command> {
    ensure!(
        output(Command::new("/usr/bin/id").arg("-un"))? == b"apps\n",
        "daily operations must run as the unprivileged apps account"
    );
    let uid = String::from_utf8(output(Command::new("/usr/bin/id").arg("-u"))?)?;
    let uid: u32 = uid.trim().parse()?;
    ensure!(uid > 0, "app operations cannot run as root");
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/usr/sbin:/bin:/sbin")
        .env("HOME", "/var/lib/apps")
        .env("XDG_RUNTIME_DIR", format!("/run/user/{uid}"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path=/run/user/{uid}/bus"),
        )
        .env("HOMEPHONE_CONTAINER_ENGINE", "podman")
        .current_dir("/var/lib/apps")
        .args(arguments);
    Ok(command)
}

pub(crate) fn apps_check() -> Result<()> {
    output(&mut operator_command(
        "/usr/bin/podman",
        &["unshare", "/usr/bin/true"],
    )?)?;
    let denied = operator_command("/usr/bin/sudo", &["-n", "true"])?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    ensure!(
        !denied.success(),
        "app operations unexpectedly have administrator access"
    );
    let phone = "/var/lib/apps/homephone/bin/homephone";
    if Path::new(phone).is_file() {
        output(&mut operator_command(
            phone,
            &["doctor", "--directory", "/var/lib/apps/homephone/telephone"],
        )?)?;
    }
    println!("Unprivileged app runtime is ready without sudo");
    Ok(())
}

pub(crate) fn monitoring_check() -> Result<()> {
    apps_check()?;
    for unit in [
        "monitoring-beszel.service",
        "monitoring-ntfy.service",
        "monitoring-agent.service",
    ] {
        let state = output(&mut operator_command(
            "/usr/bin/systemctl",
            &["--user", "is-active", unit],
        )?)?;
        ensure!(state == b"active\n", "app service is not active");
    }
    println!("All monitoring user services are active");
    Ok(())
}

pub(crate) fn apps_restart(service: &str) -> Result<()> {
    restart_app(AppService::parse(service)?, |operation| {
        let mut command = match operation {
            AppOperation::Restart(service) => {
                let unit = match service {
                    AppService::Metrics => "monitoring-beszel.service",
                    AppService::Notifications => "monitoring-ntfy.service",
                    AppService::Agent => "monitoring-agent.service",
                    AppService::Telephone => {
                        anyhow::bail!("telephone restart requires its call guard")
                    }
                };
                operator_command("/usr/bin/systemctl", &["--user", "restart", unit])?
            }
            AppOperation::StopTelephone => operator_command(
                "/var/lib/apps/homephone/bin/homephone",
                &["stop", "--directory", "/var/lib/apps/homephone/telephone"],
            )?,
            AppOperation::BootTelephone => operator_command(
                "/var/lib/apps/homephone/bin/homephone",
                &["boot", "--directory", "/var/lib/apps/homephone/telephone"],
            )?,
        };
        output(&mut command)?;
        Ok(())
    })?;
    println!("Requested app restart completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{read_serve_at, write_serve_at};
    use anyhow::{Context, Result, ensure};
    use serde_json::json;
    use std::{
        fs,
        io::{Read, Write},
        os::unix::net::UnixListener,
        thread,
        time::{Duration, Instant},
    };

    fn exchange(response: &str, client: impl FnOnce(&str) -> Result<()>) -> Result<String> {
        let mut nonce = [0_u8; 8];
        getrandom::fill(&mut nonce).map_err(|error| anyhow::anyhow!("socket identity: {error}"))?;
        let socket = std::env::temp_dir().join(format!("pi-api-{:x?}", nonce));
        let listener = UnixListener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        let response = response.to_owned();
        let server = thread::spawn(move || -> Result<String> {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut connection = loop {
                match listener.accept() {
                    Ok((connection, _)) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        ensure!(
                            Instant::now() < deadline,
                            "local API client did not connect"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            connection.set_nonblocking(false)?;
            connection.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut bytes = Vec::new();
            let mut byte = [0_u8; 1];
            while !bytes.ends_with(b"\r\n\r\n") {
                connection.read_exact(&mut byte)?;
                bytes.push(byte[0]);
                ensure!(
                    bytes.len() < 8192,
                    "local API headers exceed the fixture bound"
                );
            }
            let headers = String::from_utf8(bytes)?;
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>())
                })
                .transpose()?
                .unwrap_or(0);
            ensure!(length < 8192, "local API body exceeds the fixture bound");
            let mut body = vec![0_u8; length];
            connection.read_exact(&mut body)?;
            connection.write_all(response.as_bytes())?;
            Ok(format!("{headers}{}", String::from_utf8(body)?))
        });
        let result = client(socket.to_str().context("socket path is not UTF-8")?);
        let request = server
            .join()
            .map_err(|_| anyhow::anyhow!("local API fixture panicked"))?;
        fs::remove_file(socket)?;
        let request = request.context("local API fixture exchange failed")?;
        result?;
        Ok(request)
    }

    #[test]
    fn uses_native_serve_etag_and_refuses_a_conflicting_write() -> Result<()> {
        let request = exchange(
            "HTTP/1.1 200 OK\r\nEtag: expected-tag\r\nContent-Length: 2\r\n\r\n{}",
            |socket| {
                let (config, tag) = read_serve_at(socket)?;
                assert_eq!(config, json!({}));
                assert_eq!(tag, "expected-tag");
                Ok(())
            },
        )?;
        assert!(request.starts_with("GET /localapi/v0/serve-config HTTP/1.1\r\n"));
        let request = exchange(
            "HTTP/1.1 412 Precondition Failed\r\nContent-Length: 0\r\n\r\n",
            |socket| {
                assert!(write_serve_at(socket, &json!({"TCP":{}}), "stale-tag").is_err());
                Ok(())
            },
        )?;
        assert!(request.contains("If-Match: stale-tag\r\n"));
        assert!(request.ends_with("{\"TCP\":{}}"));
        exchange("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}", |socket| {
            assert!(read_serve_at(socket).is_err());
            Ok(())
        })?;
        Ok(())
    }
}

use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn registration_is_repeatable_and_grants_a_separate_write_only_topic() {
    let dir = std::env::temp_dir().join(format!("pi-humidity-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let runtime = dir.join("runtime");
    fs::create_dir(&runtime).unwrap();
    let env_file = runtime.join("ntfy.env");
    fs::write(&env_file,"NTFY_AUTH_USERS=reader:hash:user\nNTFY_AUTH_ACCESS=reader:home-status:ro\nNTFY_AUTH_TOKENS=reader:prior:reader\n").unwrap();
    fs::set_permissions(&env_file, fs::Permissions::from_mode(0o600)).unwrap();
    let run = |address: &str| {
        Command::new(env!("CARGO_BIN_EXE_raspberrypi-maintenance"))
            .arg("humidity-prepare")
            .arg(&dir)
            .arg(address)
            .output()
            .unwrap()
    };
    assert!(!run("invalid").status.success());
    assert!(!runtime.join("monitor.toml").exists());
    let output = run("aa:bb:cc:dd:ee:ff");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prepared = fs::read_to_string(&env_file).unwrap();
    assert!(prepared.contains("reader:home-status:ro,humidity-publisher:home-status:wo"));
    let token = fs::read_to_string(runtime.join("humidity-ntfy-token")).unwrap();
    assert!(
        token.starts_with("tk_")
            && prepared.contains(&format!("humidity-publisher:{token}:Humidity"))
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&token));
    assert_eq!(
        fs::metadata(runtime.join("humidity-ntfy-token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        fs::read_to_string(runtime.join("monitor.toml"))
            .unwrap()
            .contains("AA:BB:CC:DD:EE:FF")
    );
    assert!(run("AA:BB:CC:DD:EE:FF").status.success());
    assert_eq!(fs::read_to_string(&env_file).unwrap(), prepared);
    fs::write(
        &env_file,
        prepared.replace(
            "humidity-publisher:home-status:wo",
            "humidity-publisher:*:rw",
        ),
    )
    .unwrap();
    assert!(!run("AA:BB:CC:DD:EE:FF").status.success());
    fs::remove_dir_all(dir).unwrap();
}

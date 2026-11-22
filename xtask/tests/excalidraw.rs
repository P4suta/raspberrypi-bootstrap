use std::{fs, process::Command};

#[test]
fn rejects_an_insecure_origin_before_rendering_or_transferring() {
    let directory = std::env::temp_dir().join(format!("diagram-config-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let config = directory.join("config.json");
    fs::write(
        &config,
        r#"{"ssh_host":"demo","origin":"http://example.invalid"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_publish-excalidraw"))
        .env("EXCALIDRAW_REPLY_CONFIG", &config)
        .arg("diagram.mmd")
        .output()
        .unwrap();
    fs::remove_dir_all(directory).unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("HTTPS origin"));
    assert!(output.stdout.is_empty());
}

use anyhow::Result;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn render(config: &Value, directory: &Path) -> Result<std::process::Output> {
    fs::write(directory.join("host.json"), serde_json::to_vec(config)?)?;
    Ok(Command::new(env!("CARGO_BIN_EXE_raspberrypi-maintenance"))
        .current_dir(directory)
        .args(["portal-render", "host.json", "portal.html"])
        .output()?)
}

#[test]
fn renders_named_https_routes_and_replaces_the_previous_page() -> Result<()> {
    let directory = std::env::temp_dir().join(format!("pi-portal-test-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let result = (|| -> Result<()> {
        let mut config: Value =
            serde_json::from_str(include_str!("../../verification/examples/host.json"))?;
        config["routes"] = json!([
            {"port":8443, "target":"http://127.0.0.1:8090", "label":"Room <A> & B"},
            {"port":10443, "target":"http://127.0.0.1:8081"}
        ]);
        let output = render(&config, &directory)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let path = directory.join("portal.html");
        let page = fs::read_to_string(&path)?;
        let input = directory.join("host.json");
        let original = fs::read(&input)?;
        let same_file = Command::new(env!("CARGO_BIN_EXE_raspberrypi-maintenance"))
            .arg("portal-render")
            .arg(&input)
            .arg(&input)
            .output()?;
        assert!(!same_file.status.success());
        assert_eq!(fs::read(&input)?, original);
        assert!(page.contains("href=\"https://device.example.ts.net:8443/\""));
        assert!(page.contains("Room &lt;A&gt; &amp; B"));
        for private in [
            "127.0.0.1",
            "10443",
            "172.30.254.",
            config["machine_id"].as_str().unwrap(),
        ] {
            assert!(!page.contains(private));
        }
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);

        config["routes"][0]["port"] = json!(12443);
        assert!(render(&config, &directory)?.status.success());
        let moved = fs::read_to_string(&path)?;
        assert!(moved.contains("https://device.example.ts.net:12443/"));
        assert!(!moved.contains(":8443/"));

        for invalid in ["  ", "first\nsecond", "first\u{2028}second"] {
            config["routes"][0]["label"] = json!(invalid);
            assert!(!render(&config, &directory)?.status.success());
            assert_eq!(fs::read_to_string(&path)?, moved);
        }

        config["routes"].as_array_mut().unwrap().remove(0);
        assert!(render(&config, &directory)?.status.success());
        let empty = fs::read_to_string(path)?;
        assert!(!empty.contains("<a "));
        assert!(empty.contains("Add a display name"));
        Ok(())
    })();
    fs::remove_dir_all(directory)?;
    result
}

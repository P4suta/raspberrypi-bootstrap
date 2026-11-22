use anyhow::Result;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::Command,
};

fn generate(site: &str, networks: &str, path: &Path) -> Result<bool> {
    Ok(Command::new(env!("CARGO_BIN_EXE_raspberrypi-maintenance"))
        .args(["routing-config", site, networks])
        .arg(path)
        .output()?
        .status
        .success())
}

#[test]
fn preserves_existing_files_and_writes_each_private_prefix_with_its_site() -> Result<()> {
    let root = std::env::temp_dir().join(format!("pi-routing-cli-{}", std::process::id()));
    fs::create_dir(&root)?;
    let file = root.join("routes.env");
    let prefixes = "10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,192.168.255.255/32";
    assert!(generate("65535", prefixes, &file)?);
    let expected = concat!(
        "SUBNET_ROUTES=10.0.0.0/8,fd7a:115c:a1e0:b1a:0:ffff:a00:0/104,",
        "172.16.0.0/12,fd7a:115c:a1e0:b1a:0:ffff:ac10:0/108,",
        "192.168.0.0/16,fd7a:115c:a1e0:b1a:0:ffff:c0a8:0/112,",
        "192.168.255.255/32,fd7a:115c:a1e0:b1a:0:ffff:c0a8:ffff/128\n",
    );
    assert_eq!(fs::read_to_string(&file)?, expected);
    assert_eq!(fs::metadata(&file)?.permissions().mode() & 0o777, 0o600);
    assert!(!generate("1", "10.1.2.3/32", &file)?);
    let alias = root.join("alias");
    symlink(&file, &alias)?;
    assert!(!generate("1", "10.1.2.3/32", &alias)?);
    assert_eq!(fs::read_to_string(&file)?, expected);
    let rejected = root.join("rejected");
    for networks in [
        "",
        "172.16.0.0/11",
        "192.168.0.0/15",
        "10.0.0.0/7",
        "10.1.2.3/33",
        "127.0.0.0/8",
        "10.0.0.0/8,8.8.8.8/32",
    ] {
        assert!(!generate("0", networks, &rejected)?, "accepted {networks}");
        assert!(!rejected.exists());
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

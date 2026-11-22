use anyhow::{Result, ensure};
use std::{
    collections::HashMap,
    fs,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use zbus::{OwnedVariant, Variant, connection::Builder};

struct Adapter(AtomicBool);

#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    fn set_discovery_filter(&self, _filter: HashMap<String, OwnedVariant>) {}
    fn start_discovery(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    #[zbus(property)]
    fn discovering(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    ensure!(
        std::env::consts::ARCH == "aarch64"
            && fs::read_to_string("/etc/hostname")?.trim() == "pi-operations-preflight"
            && std::path::Path::new("/workspace/fixture/checkpoint.json").is_file(),
        "requires the isolated Pi rehearsal"
    );
    let bus = Builder::system()
        .name("org.bluez")
        .serve_at("/org/bluez/hci0", Adapter(AtomicBool::new(false)))
        .build()
        .await?;
    loop {
        if let Ok(mode) = fs::read_to_string("/run/humidity-rehearsal-mode") {
            let humidity = match mode.trim() {
                "normal" => Some(45u8),
                "high" => Some(56),
                "silent" => None,
                _ => anyhow::bail!("unknown fixture mode"),
            };
            if let Some(humidity) = humidity {
                let service = HashMap::from([(
                    "0000fd3d-0000-1000-8000-00805f9b34fb",
                    Variant::from(vec![b'w', 0, 100]),
                )]);
                let manufacturer = HashMap::from([(
                    2409u16,
                    Variant::from(vec![
                        0xaa_u8, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x36, 3, 2, 0x96, humidity, 0,
                    ]),
                )]);
                let properties = HashMap::from([
                    ("ManufacturerData", Variant::from(manufacturer)),
                    ("ServiceData", Variant::from(service)),
                ]);
                bus.emit_signal(
                    None::<&str>,
                    "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF",
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &("org.bluez.Device1", properties, Vec::<String>::new()),
                )
                .await?;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

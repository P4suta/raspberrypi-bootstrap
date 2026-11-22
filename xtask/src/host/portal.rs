use super::Config;
use anyhow::{Context, Result, ensure};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

pub(super) const FILE: &str = "/var/lib/raspberrypi-maintenance/service-portal.html";
pub(super) const BACKUP_NAME: &str = "service-portal";

pub(super) fn html(config: &Config) -> String {
    let mut links = String::new();
    for route in &config.routes {
        if let Some(label) = &route.label {
            let name = label
                .trim()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            links.push_str(&format!(
                "<a href=\"https://{}:{}/\"><span aria-hidden=\"true\">↗</span><span>{name}</span></a>\n",
                config.tailnet_name, route.port
            ));
        }
    }
    let content = if links.is_empty() {
        "<p>Add a display name to a host route to list a service here.</p>".to_owned()
    } else {
        format!("<nav aria-label=\"Services\">\n{links}</nav>")
    };
    include_str!("portal.html").replace("<!-- service links -->", &content)
}

pub(crate) fn render_file(input: &Path, output: &Path) -> Result<()> {
    let config: Config = serde_json::from_slice(&fs::read(input)?)?;
    config.validate()?;
    if output.try_exists()? {
        let source = fs::metadata(input)?;
        let destination = fs::metadata(output)?;
        ensure!(
            (source.dev(), source.ino()) != (destination.dev(), destination.ino()),
            "the portal output must differ from the host configuration"
        );
    }
    super::native::replace(output, html(&config).as_bytes(), 0o600)
        .context("could not write the portal preview")?;
    println!("{}", output.display());
    Ok(())
}

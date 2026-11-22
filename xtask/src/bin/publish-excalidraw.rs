use anyhow::{Context, Result, bail, ensure};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{Read, Write},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const CLI: &str = "@excalidraw/cli@1.1.2";
const SCENE: &str = "scene.excalidraw";
const PREVIEW: &str = "preview.png";
const REMOTE: &str = ".local/share/excalidraw-replies";
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const READBACK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    ssh_host: String,
    origin: String,
}

impl Config {
    fn origin(&self) -> Result<String> {
        ensure!(
            self.ssh_host
                .starts_with(|c: char| c.is_ascii_alphanumeric())
                && self
                    .ssh_host
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
            "expected an SSH host alias"
        );
        let url = Url::parse(&self.origin)?;
        ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "expected an HTTPS origin without a path or credentials"
        );
        Ok(url.origin().ascii_serialization())
    }
}

fn directory(variable: &str, fallback: &str) -> Result<PathBuf> {
    if let Some(value) = env::var_os(variable).filter(|p| Path::new(p).is_absolute()) {
        return Ok(PathBuf::from(value));
    }
    Ok(PathBuf::from(env::var_os("HOME").context("missing home directory")?).join(fallback))
}

fn cli(operation: &str, source: &Path, destination: &Path) -> Result<()> {
    let status = Command::new("npx")
        .args(["--yes", CLI, operation])
        .arg(source)
        .arg("--out")
        .arg(destination)
        .stdout(Stdio::null())
        .status()
        .context("start the Excalidraw CLI with npx")?;
    ensure!(status.success(), "Excalidraw {operation} failed: {status}");
    Ok(())
}

fn render(input: &Path, archive: &Path) -> Result<()> {
    let scene = archive.join(SCENE);
    match input.extension().and_then(|v| v.to_str()) {
        Some("mmd") => {
            cli("mermaid", input, &scene)?;
            let mut document: Value = serde_json::from_slice(&fs::read(&scene)?)?;
            document["appState"]["viewBackgroundColor"] = json!("#ffffff");
            let elements = document["elements"]
                .as_array_mut()
                .context("missing scene elements")?;
            ensure!(!elements.is_empty(), "empty diagram");
            for element in elements {
                element["strokeColor"] = json!("#000000");
                if element["backgroundColor"]
                    .as_str()
                    .is_some_and(|color| color != "transparent")
                {
                    element["backgroundColor"] = json!("#ffffff");
                }
            }
            fs::write(&scene, serde_json::to_vec(&document)?)?;
        }
        Some("excalidraw") => {
            fs::copy(input, &scene)?;
        }
        _ => bail!("expected a Mermaid (.mmd) or Excalidraw (.excalidraw) file"),
    }
    cli("render", &scene, &archive.join(PREVIEW))
}

fn transfer(transport: &mut Command, archive: &Path, batch: &str) -> Result<()> {
    let mut child = transport
        .current_dir(archive)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("start SFTP")?;
    let written = child
        .stdin
        .take()
        .context("open SFTP batch input")?
        .write_all(batch.as_bytes());
    let status = child.wait()?;
    written?;
    ensure!(status.success(), "SFTP failed: {status}");
    Ok(())
}

fn upload(transport: &mut Command, archive: &Path, id: &str) -> Result<()> {
    let staging = format!("{REMOTE}/incoming/{id}");
    transfer(transport, archive, &format!("mkdir {staging}\n"))?;
    let batch = format!(
        "put {SCENE} {staging}/{SCENE}\nput {PREVIEW} {staging}/{PREVIEW}\nchmod 644 {staging}/{SCENE}\nchmod 644 {staging}/{PREVIEW}\nchmod 755 {staging}\nrename {staging} {REMOTE}/diagrams/{id}\n"
    );
    if let Err(error) = transfer(transport, archive, &batch) {
        let cleanup = format!("-rm {staging}/{SCENE}\n-rm {staging}/{PREVIEW}\n-rmdir {staging}\n");
        if let Err(failure) = transfer(transport, archive, &cleanup) {
            eprintln!("Could not clear unpublished transfer {id}: {failure}");
        }
        return Err(error);
    }
    Ok(())
}

fn verify(client: &Client, url: &str, path: &Path) -> Result<()> {
    let expected = fs::read(path)?;
    let response = client.get(url).send()?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "readback returned {}",
        response.status()
    );
    let mut bytes = Vec::new();
    response
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes == expected,
        "published file differs from {}",
        path.display()
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("Usage: publish-excalidraw DRAWING.mmd|DRAWING.excalidraw");
        return Ok(());
    }
    ensure!(args.len() == 1, "expected one .mmd or .excalidraw input");
    let config_path = match env::var_os("EXCALIDRAW_REPLY_CONFIG") {
        Some(path) => PathBuf::from(path),
        None => directory("XDG_CONFIG_HOME", ".config")?.join("excalidraw-replies/config.json"),
    };
    let config: Config = serde_json::from_slice(
        &fs::read(&config_path).with_context(|| format!("read {}", config_path.display()))?,
    )?;
    let origin = config.origin()?;
    let input = fs::canonicalize(&args[0]).context("open diagram input")?;
    let root = directory("XDG_DATA_HOME", ".local/share")?.join("excalidraw-replies");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIRECTORY_MODE)
        .create(&root)?;
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|e| anyhow::anyhow!("allocate diagram ID: {e}"))?;
    let id = u128::from_be_bytes(random);
    let id = format!("{id:032x}");
    let archive = root.join(&id);
    fs::DirBuilder::new()
        .mode(PRIVATE_DIRECTORY_MODE)
        .create(&archive)?;
    if let Err(error) = render(&input, &archive) {
        if let Err(cleanup) = fs::remove_dir_all(&archive) {
            eprintln!("Could not remove incomplete local drawing: {cleanup}");
        }
        return Err(error);
    }
    let prefix = format!("/diagrams/{id}");
    let scene_url = format!("{origin}{prefix}/{SCENE}");
    let png_url = format!("{origin}{prefix}/{PREVIEW}");
    let client = Client::builder()
        .timeout(READBACK_TIMEOUT)
        .redirect(Policy::none())
        .build()?;
    (|| -> Result<()> {
        let mut sftp = Command::new("sftp");
        sftp.args([
            "-q",
            "-oBatchMode=yes",
            "-oConnectTimeout=10",
            "-b",
            "-",
            &config.ssh_host,
        ]);
        upload(&mut sftp, &archive, &id)?;
        verify(&client, &scene_url, &archive.join(SCENE))?;
        verify(&client, &png_url, &archive.join(PREVIEW))
    })()
    .with_context(|| {
        format!(
            "Could not verify publication; local drawing: {}",
            archive.display()
        )
    })?;
    println!(
        "{}",
        json!({
            "preview_path": archive.join(PREVIEW),
            "scene_path": archive.join(SCENE),
            "png_url": png_url,
            "editor_url": format!("{origin}/#url={prefix}/{SCENE}"),
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_accepts_only_an_https_origin_and_a_single_host_alias() {
        let mut config = Config {
            ssh_host: "fixture-host".into(),
            origin: "https://example.invalid:1443".into(),
        };
        assert_eq!(config.origin().unwrap(), config.origin);
        for origin in [
            "http://example.invalid",
            "https://reader@example.invalid",
            "https://example.invalid/drawing",
            "https://example.invalid/?view=1",
            "https://example.invalid/#preview",
        ] {
            config.origin = origin.into();
            assert!(config.origin().is_err());
        }
        config.origin = "https://example.invalid".into();
        for alias in ["", "-host", "two hosts", "host/name"] {
            config.ssh_host = alias.into();
            assert!(config.origin().is_err());
        }
    }

    #[test]
    fn publication_requires_a_successful_readback_with_identical_bytes() -> Result<()> {
        use std::net::TcpListener;
        let drawing = env::temp_dir().join(format!("diagram-readback-{}", std::process::id()));
        fs::write(&drawing, "scene")?;
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(5))
            .build()?;
        for (status, contents, success) in [
            (200, "scene", true),
            (200, "scen", false),
            (200, "scene!", false),
            (302, "scene", false),
            (404, "scene", false),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0))?;
            let origin = format!("http://{}", listener.local_addr()?);
            let server = std::thread::spawn(move || -> std::io::Result<()> {
                use std::io::BufRead;
                let (mut stream, _) = listener.accept()?;
                for line in std::io::BufReader::new(&stream).lines() {
                    if line?.is_empty() {
                        break;
                    }
                }
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{contents}",
                    contents.len()
                )
            });
            let verified = verify(&client, &origin, &drawing);
            server.join().unwrap()?;
            assert_eq!(verified.is_ok(), success, "HTTP {status}, {contents}");
        }
        fs::remove_file(drawing)?;
        Ok(())
    }

    #[test]
    fn sftp_only_exposes_complete_drawings_and_preserves_existing_replies() -> Result<()> {
        let server = ["/usr/libexec/sftp-server", "/usr/lib/openssh/sftp-server"]
            .into_iter()
            .find(|p| Path::new(p).is_file())
            .context("install the OpenSSH SFTP server for the transfer test")?;
        let mut nonce = [0_u8; 8];
        getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("{e}"))?;
        let root = env::temp_dir().join(format!("excalidraw-sftp-{}", u64::from_be_bytes(nonce)));
        fs::DirBuilder::new()
            .mode(PRIVATE_DIRECTORY_MODE)
            .create(&root)?;
        let archive = root.join(REMOTE);
        let public = archive.join("diagrams");
        fs::create_dir_all(archive.join("incoming"))?;
        fs::create_dir(&public)?;
        fs::write(root.join(SCENE), "first")?;
        let mut sftp = Command::new("sftp");
        sftp.args(["-q", "-b", "-", "-D", server, "local"]);
        assert!(upload(&mut sftp, &root, "one").is_err());
        assert!(!public.join("one").exists());
        assert!(!archive.join("incoming/one").exists());
        fs::write(root.join(PREVIEW), "preview")?;
        let occupied = archive.join("incoming/one");
        fs::create_dir(&occupied)?;
        fs::write(occupied.join(SCENE), "in progress")?;
        assert!(upload(&mut sftp, &root, "one").is_err());
        assert_eq!(fs::read(occupied.join(SCENE))?, b"in progress");
        fs::remove_dir_all(&occupied)?;
        upload(&mut sftp, &root, "one")?;
        fs::write(root.join(SCENE), "second")?;
        upload(&mut sftp, &root, "two")?;
        assert!(upload(&mut sftp, &root, "one").is_err());
        assert_eq!(fs::read(public.join("one").join(SCENE))?, b"first");
        assert_eq!(fs::read(public.join("two").join(SCENE))?, b"second");
        assert_eq!(fs::read(public.join("two").join(PREVIEW))?, b"preview");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}

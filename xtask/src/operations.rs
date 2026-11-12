use anyhow::{Context, Result, ensure};
use reqwest::{Method, Url, blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

mod preflight;

pub(super) fn fixture(action: &str) -> Result<()> {
    preflight::run(action)
}

pub(super) fn local_preflight() -> Result<()> {
    preflight::local()
}

pub(super) fn initialize_preflight(mise: &Path, agent: &Path) -> Result<()> {
    preflight::initialize(mise, agent)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inputs {
    credentials_file: PathBuf,
    public_key_file: PathBuf,
    source_url: String,
    hub_url: String,
    ntfy_url: String,
    pi_host: String,
    required_systems: Vec<String>,
    offline_systems: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Secrets {
    publisher_password: String,
    reader_password: String,
    publisher_token: String,
    pi_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nas_token: Option<String>,
}

struct Session {
    client: Client,
    base: Url,
    token: String,
    user_id: String,
}

fn validate_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)?;
    let host = url.host_str().context("endpoint needs a host")?;
    ensure!(
        url.scheme() == "https" && host.ends_with(".ts.net"),
        "endpoint must use HTTPS on the private tailnet"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "credentials cannot appear in endpoint URLs"
    );
    ensure!(
        url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
        "endpoint must be an origin"
    );
    Ok(url)
}

fn inputs(path: &Path) -> Result<Inputs> {
    let value: Inputs = serde_json::from_slice(&private_read(path)?)?;
    for endpoint in [&value.source_url, &value.hub_url, &value.ntfy_url] {
        validate_url(endpoint)?;
    }
    ensure!(
        value.required_systems.len() == 4
            && value.required_systems.iter().collect::<BTreeSet<_>>().len() == 4,
        "four distinct target systems are required"
    );
    ensure!(
        value.offline_systems.iter().collect::<BTreeSet<_>>().len() == value.offline_systems.len()
            && value
                .offline_systems
                .iter()
                .all(|name| value.required_systems.contains(name)),
        "offline targets must belong to the target set"
    );
    Ok(value)
}

fn private_read(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).context("read private file metadata")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "private input must be a regular file"
    );
    ensure!(
        metadata.permissions().mode() & 0o077 == 0,
        "private input must have owner-only permissions"
    );
    fs::read(path).context("read private input")
}

fn private_write(path: &Path, contents: &[u8]) -> Result<()> {
    private_replace(path, |file| Ok(file.write_all(contents)?))
}

fn parent_directory(path: &Path) -> Result<&Path> {
    let parent = path.parent().context("file needs a parent directory")?;
    Ok(if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    })
}

fn private_replace(path: &Path, write: impl FnOnce(&mut fs::File) -> Result<()>) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            private_read(path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = parent_directory(path)?;
    let staged = parent.join(format!(".private-write-{}", random_text(24)?));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&staged)?;
    let result = (|| -> Result<()> {
        write(&mut file)?;
        file.sync_all()?;
        fs::rename(&staged, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let cleanup = match fs::remove_file(&staged) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    };
    finish_cleanup(result, cleanup)
}

fn finish_cleanup(result: Result<()>, cleanup: Result<()>) -> Result<()> {
    match (result, cleanup) {
        (Err(primary), Err(cleanup)) => {
            Err(primary.context(format!("cleanup also failed: {cleanup:#}")))
        }
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), cleanup) => cleanup,
    }
}

enum RotationAccess {
    Stable,
    Pending,
}

struct CredentialLock {
    _file: fs::File,
    config: PathBuf,
}

fn credential_lock(config: &Path, access: RotationAccess) -> Result<CredentialLock> {
    let root = parent_directory(config)?;
    let path = root.join("credentials.lock");
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            private_read(&path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&path)?;
    lock.try_lock()
        .context("another credential operation is active")?;
    let pending = match fs::symlink_metadata(root.join("reader-password-backup")) {
        Ok(metadata) => {
            ensure!(
                metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.permissions().mode() & 0o077 == 0,
                "rotation backup must be a private directory"
            );
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    match access {
        RotationAccess::Stable => ensure!(
            !pending,
            "finish the pending rotation before preparing or changing credentials"
        ),
        RotationAccess::Pending => ensure!(pending, "no pending rotation exists"),
    }
    Ok(CredentialLock {
        _file: lock,
        config: config.to_owned(),
    })
}

fn random_text(length: usize) -> Result<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    let mut bytes = vec![0_u8; length];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("operating system randomness failed: {error}"))?;
    Ok(bytes
        .into_iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 63)]))
        .collect())
}

fn password_hash(password: &str, cost: u32) -> Result<String> {
    ensure!(
        (1..=72).contains(&password.len()) && password.bytes().all(|byte| byte.is_ascii_graphic()),
        "notification password must contain 1 to 72 printable ASCII characters without spaces"
    );
    bcrypt::hash(password, cost).context("hash notification password")
}

fn replace_reader_hash(environment: &str, hash: &str) -> Result<String> {
    let lines = environment
        .lines()
        .filter_map(|line| line.strip_prefix("NTFY_AUTH_USERS="))
        .collect::<Vec<_>>();
    ensure!(
        lines.len() == 1,
        "expected one provisioned user declaration"
    );
    let users = lines[0].split(',').collect::<Vec<_>>();
    let matching = users
        .iter()
        .filter(|entry| entry.starts_with("iphone:"))
        .collect::<Vec<_>>();
    ensure!(matching.len() == 1, "expected one iPhone reader identity");
    let fields = matching[0].split(':').collect::<Vec<_>>();
    ensure!(
        fields.len() == 3 && fields[2] == "user",
        "reader identity must remain a regular user"
    );
    let replacement = format!("iphone:{hash}:user");
    let updated = users
        .iter()
        .map(|entry| {
            if *entry == *matching[0] {
                replacement.as_str()
            } else {
                entry
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    Ok(environment.replacen(
        &format!("NTFY_AUTH_USERS={}", lines[0]),
        &format!("NTFY_AUTH_USERS={updated}"),
        1,
    ))
}

fn reader_backup(path: &Path) -> Result<PathBuf> {
    Ok(parent_directory(path)?.join("reader-password-backup"))
}

fn rotate_reader_password(path: &Path, password: &str) -> Result<()> {
    let lock = credential_lock(path, RotationAccess::Stable)?;
    let inputs = inputs(path)?;
    let hash = password_hash(password, 12)?;
    let mut secret = secrets(path)?;
    ensure!(
        password != secret.reader_password,
        "reader password has not changed"
    );
    let runtime = parent_directory(path)?.join("runtime");
    let old_environment = private_read(&runtime.join("ntfy.env"))?;
    let environment = replace_reader_hash(std::str::from_utf8(&old_environment)?, &hash)?;
    let old_phone = private_read(&runtime.join("iphone.txt"))?;
    let old_state = private_read(&state_path(path)?)?;
    secret.reader_password = password.to_owned();
    let phone = format!(
        "Server: {}\nTopic: home-status\nUsername: iphone\nPassword: {password}\n",
        inputs.ntfy_url
    );
    stage_reader_rotation(
        &lock,
        &[
            ("client.json", &private_read(path)?),
            ("ntfy.env", &old_environment),
            ("iphone.txt", &old_phone),
            ("secrets.json", &old_state),
            ("replacement-ntfy.env", environment.as_bytes()),
            ("replacement-iphone.txt", phone.as_bytes()),
            (
                "replacement-secrets.json",
                &serde_json::to_vec_pretty(&secret)?,
            ),
        ],
    )?;
    apply_reader_rotation(&lock)?;
    println!("Updated private reader credentials; deploy ntfy.env before checking the rotation");
    Ok(())
}

pub(super) fn reader_password(path: &Path) -> Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin().lock().take(74).read_to_end(&mut bytes)?;
    let password = String::from_utf8(bytes).context("password input must be UTF-8")?;
    ensure!(
        password.len() == 64,
        "reader password rotation expects exactly 64 ASCII characters"
    );
    rotate_reader_password(path, &password)
}

fn stage_reader_rotation(lock: &CredentialLock, files: &[(&str, &[u8])]) -> Result<()> {
    let root = parent_directory(&lock.config)?;
    let staged = root.join(format!(".reader-password-stage-{}", random_text(24)?));
    fs::DirBuilder::new().mode(0o700).create(&staged)?;
    let result = (|| -> Result<()> {
        for (name, bytes) in files {
            private_write(&staged.join(name), bytes)?;
        }
        fs::File::open(&staged)?.sync_all()?;
        fs::rename(&staged, reader_backup(&lock.config)?)?;
        fs::File::open(root)?.sync_all()?;
        Ok(())
    })();
    let cleanup = match fs::remove_dir_all(&staged) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    };
    finish_cleanup(result, cleanup)
}

fn reader_replacements(lock: &CredentialLock) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let path = &lock.config;
    let backup = reader_backup(path)?;
    ensure!(
        private_read(path)? == private_read(&backup.join("client.json"))?,
        "pending rotation configuration changed"
    );
    let previous: Secrets = serde_json::from_slice(&private_read(&backup.join("secrets.json"))?)?;
    let replacement: Secrets =
        serde_json::from_slice(&private_read(&backup.join("replacement-secrets.json"))?)?;
    validate_reader_transition(&previous, &replacement)?;
    let root = parent_directory(path)?;
    let mut replacements = Vec::new();
    for name in ["ntfy.env", "iphone.txt", "secrets.json"] {
        let destination = if name == "secrets.json" {
            root.join(name)
        } else {
            root.join("runtime").join(name)
        };
        let previous = private_read(&backup.join(name))?;
        let replacement = private_read(&backup.join(format!("replacement-{name}")))?;
        let active = private_read(&destination)?;
        ensure!(
            active == previous || active == replacement,
            "pending rotation conflicts with a changed credential file"
        );
        replacements.push((destination, replacement));
    }
    Ok(replacements)
}

fn apply_reader_rotation(lock: &CredentialLock) -> Result<()> {
    for (path, replacement) in reader_replacements(lock)? {
        private_write(&path, &replacement)?;
    }
    Ok(())
}

pub(super) fn reader_password_resume(path: &Path) -> Result<()> {
    inputs(path)?;
    let lock = credential_lock(path, RotationAccess::Pending)?;
    apply_reader_rotation(&lock)?;
    println!(
        "Restored the pending replacement files; deploy ntfy.env before checking the rotation"
    );
    Ok(())
}

fn validate_reader_transition(previous: &Secrets, active: &Secrets) -> Result<()> {
    ensure!(
        previous.reader_password != active.reader_password,
        "reader password has not changed"
    );
    let mut expected = previous.clone();
    expected.reader_password = active.reader_password.clone();
    ensure!(expected == *active, "rotation changed another credential");
    Ok(())
}

fn verify_reader_rotation(base: Url, previous: &Secrets, active: &Secrets) -> Result<()> {
    validate_reader_transition(previous, active)?;
    let status = client()?
        .get(base.join("home-status/json?poll=1")?)
        .basic_auth("iphone", Some(&previous.reader_password))
        .send()?
        .status();
    ensure!(
        matches!(status.as_u16(), 401 | 403),
        "previous reader password still authenticates"
    );
    notification_checks(base, active)?;
    println!("Previous reader password denied; replacement authenticated with read-only access");
    Ok(())
}

pub(super) fn reader_password_check(path: &Path) -> Result<()> {
    let lock = credential_lock(path, RotationAccess::Pending)?;
    for (destination, replacement) in reader_replacements(&lock)? {
        ensure!(
            private_read(&destination)? == replacement,
            "resume the pending rotation before deployment verification"
        );
    }
    let inputs = inputs(path)?;
    let previous: Secrets =
        serde_json::from_slice(&private_read(&reader_backup(path)?.join("secrets.json"))?)?;
    let active = secrets(path)?;
    verify_reader_rotation(validate_url(&inputs.ntfy_url)?, &previous, &active)?;
    hub_notification(&Session::login(&inputs, false)?, &active)?;
    finish_reader_rotation(&lock)?;
    println!("Reader password rotation passed; temporary previous credentials removed");
    Ok(())
}

fn finish_reader_rotation(lock: &CredentialLock) -> Result<()> {
    let path = &lock.config;
    let backup = reader_backup(path)?;
    let root = parent_directory(path)?;
    let retired = root.join(format!(".verified-reader-password-{}", random_text(24)?));
    fs::rename(backup, &retired)?;
    fs::File::open(root)?.sync_all()?;
    fs::remove_dir_all(&retired).context("remove verified previous credentials")?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

fn state_path(config: &Path) -> Result<PathBuf> {
    Ok(parent_directory(config)?.join("secrets.json"))
}

fn secrets(config: &Path) -> Result<Secrets> {
    serde_json::from_slice(&private_read(&state_path(config)?)?).context("read saved private state")
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(Policy::none())
        .build()?)
}

fn credentials(path: &Path) -> Result<(String, String)> {
    let text = String::from_utf8(private_read(path)?)?;
    let field = |name: &str| -> Result<String> {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .with_context(|| format!("missing credentials field {name}"))
    };
    Ok((field("Email:")?, field("Password:")?))
}

impl Session {
    fn login(inputs: &Inputs, source: bool) -> Result<Self> {
        let base = validate_url(if source {
            &inputs.source_url
        } else {
            &inputs.hub_url
        })?;
        let client = client()?;
        let (identity, password) = credentials(&inputs.credentials_file)?;
        let response = client
            .post(base.join("api/collections/users/auth-with-password")?)
            .json(&json!({"identity":identity,"password":password}))
            .send()?
            .error_for_status()?
            .json::<Value>()?;
        Ok(Self {
            client,
            base,
            token: string(&response, "token")?.to_owned(),
            user_id: string(&response["record"], "id")?.to_owned(),
        })
    }

    fn request(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let mut request = self
            .client
            .request(method, self.base.join(path)?)
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().with_context(|| format!("request {path}"))?;
        ensure!(
            response.status().is_success(),
            "request {path} failed with HTTP {}",
            response.status()
        );
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(Value::Null)
        } else {
            response.json().with_context(|| format!("decode {path}"))
        }
    }

    fn records(&self, collection: &str) -> Result<Vec<Value>> {
        let mut records = Vec::new();
        let mut identities = BTreeSet::new();
        let sort = if collection == "system_stats" {
            "created,id"
        } else {
            "id"
        };
        for page in 1_u64..=1000 {
            let response = self.request(
                Method::GET,
                &format!(
                    "api/collections/{collection}/records?perPage=100&page={page}&sort={sort}"
                ),
                None,
            )?;
            let pages = response["totalPages"]
                .as_u64()
                .context("record list needs a page count")?;
            ensure!(
                pages <= 1000 && response["page"] == page,
                "record list exceeds or ignores the pagination limit"
            );
            let items = response["items"]
                .as_array()
                .context("record list has no items")?;
            ensure!(
                items.len() <= 100,
                "record list exceeds the requested page size"
            );
            for item in items {
                ensure!(
                    identities.insert(string(item, "id")?.to_owned()),
                    "record list repeated an identity across pages"
                );
                records.push(item.clone());
            }
            if page >= pages {
                return Ok(records);
            }
        }
        anyhow::bail!("record pagination exceeded its bound")
    }

    fn create(&self, collection: &str, value: &Value) -> Result<Value> {
        self.request(
            Method::POST,
            &format!("api/collections/{collection}/records"),
            Some(value),
        )
    }

    fn update(&self, collection: &str, id: &str, value: &Value) -> Result<Value> {
        ensure!(
            id.len() == 15 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "invalid record identifier"
        );
        self.request(
            Method::PATCH,
            &format!("api/collections/{collection}/records/{id}"),
            Some(value),
        )
    }

    fn delete(&self, collection: &str, id: &str) -> Result<()> {
        let minimum = if collection == "fingerprints" { 9 } else { 15 };
        ensure!(
            (minimum..=15).contains(&id.len())
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()),
            "invalid {collection} record identifier (length {})",
            id.len()
        );
        self.request(
            Method::DELETE,
            &format!("api/collections/{collection}/records/{id}"),
            None,
        )?;
        Ok(())
    }

    fn retire_system(&self, name: &str) -> Result<()> {
        let systems = self.records("systems")?;
        let matching = systems
            .iter()
            .filter(|item| item["name"] == name)
            .collect::<Vec<_>>();
        ensure!(
            matching.len() <= 1,
            "retirement requires a unique system name"
        );
        let Some(system) = matching.first() else {
            return Ok(());
        };
        let id = string(system, "id")?;
        for collection in ["alerts", "fingerprints"] {
            for record in self
                .records(collection)?
                .iter()
                .filter(|item| item["system"] == id)
            {
                self.delete(collection, string(record, "id")?)?;
            }
        }
        self.delete("systems", id)?;
        ensure!(
            !self.records("systems")?.iter().any(|item| item["id"] == id),
            "retired system remains registered"
        );
        for collection in ["alerts", "fingerprints"] {
            ensure!(
                !self
                    .records(collection)?
                    .iter()
                    .any(|item| item["system"] == id),
                "retired system still has {collection}"
            );
        }
        Ok(())
    }

    fn system(&self, name: &str, host: &str, token: &str) -> Result<()> {
        let systems = self.records("systems")?;
        let system = match systems.iter().find(|item| item["name"] == name) {
            Some(system) => system.clone(),
            None => self.create("systems", &json!({"name":name,"host":host,"port":"45876","status":"pending","users":[self.user_id]}))?,
        };
        let id = string(&system, "id")?;
        let fingerprints = self.records("fingerprints")?;
        if let Some(existing) = fingerprints.iter().find(|item| item["system"] == id) {
            ensure!(
                existing["token"] == token,
                "target system already has a different registration identity"
            );
        } else {
            self.create("fingerprints", &json!({"system":id,"token":token}))?;
        }
        Ok(())
    }
}

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .with_context(|| format!("missing response field {field}"))
}

pub(super) fn prepare(path: &Path) -> Result<()> {
    let _lock = credential_lock(path, RotationAccess::Stable)?;
    let inputs = inputs(path)?;
    let state = state_path(path)?;
    let secret = if state.exists() {
        secrets(path)?
    } else {
        let secret = Secrets {
            publisher_password: random_text(32)?,
            reader_password: random_text(32)?,
            publisher_token: format!("tk_{}", random_text(29)?),
            pi_token: random_text(48)?,
            nas_token: None,
        };
        private_write(&state, &serde_json::to_vec_pretty(&secret)?)?;
        secret
    };
    let session = Session::login(&inputs, true)?;
    session.system("Raspberry Pi", &inputs.pi_host, &secret.pi_token)?;
    let runtime = parent_directory(path)?.join("runtime");
    fs::create_dir_all(&runtime)?;
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
    let hub = format!(
        "APP_URL={}\nDISABLE_SSH=true\nCHECK_UPDATES=false\nCONTAINER_DETAILS=false\n",
        inputs.hub_url
    );
    private_write(&runtime.join("hub.env"), hub.as_bytes())?;
    let publisher_hash = password_hash(&secret.publisher_password, 12)?;
    let reader_hash = password_hash(&secret.reader_password, 12)?;
    let ntfy = format!(
        "NTFY_BASE_URL={}\nNTFY_AUTH_USERS=beszel:{publisher_hash}:user,iphone:{reader_hash}:user\nNTFY_AUTH_ACCESS=beszel:home-status:wo,iphone:home-status:ro\nNTFY_AUTH_TOKENS=beszel:{}:Beszel\n",
        inputs.ntfy_url, secret.publisher_token
    );
    private_write(&runtime.join("ntfy.env"), ntfy.as_bytes())?;
    let public_key = fs::read_to_string(&inputs.public_key_file)?
        .trim()
        .to_owned();
    ensure!(
        public_key.starts_with("ssh-ed25519 ") && !public_key.contains('\n'),
        "invalid hub public key"
    );
    private_write(&runtime.join("hub.pub"), public_key.as_bytes())?;
    let environment = format!(
        "HUB_URL={}\nKEY={public_key}\nTOKEN={token}\nDISABLE_SSH=true\nSKIP_GPU=true\nSKIP_SYSTEMD=true\nSKIP_SYSTEMD_LOGS=true\nSMART_DEVICES=\nDOCKER_HOST=\n",
        inputs.hub_url,
        token = secret.pi_token
    );
    private_write(&runtime.join("pi-agent.env"), environment.as_bytes())?;
    let reader = format!(
        "Server: {}\nTopic: home-status\nUsername: iphone\nPassword: {}\n",
        inputs.ntfy_url, secret.reader_password
    );
    private_write(&runtime.join("iphone.txt"), reader.as_bytes())?;
    println!("Prepared private runtime files and the Pi agent registration");
    Ok(())
}

pub(super) fn retire_nas(path: &Path) -> Result<()> {
    let _lock = credential_lock(path, RotationAccess::Stable)?;
    let inputs = inputs(path)?;
    ensure!(
        !inputs
            .required_systems
            .iter()
            .any(|name| name == "Synology NAS"),
        "remove the NAS from the target configuration before retirement"
    );
    let mut secret = secrets(path)?;
    Session::login(&inputs, false)?.retire_system("Synology NAS")?;
    let runtime = parent_directory(path)?.join("runtime");
    for name in ["nas-agent.env", "nas-install.txt"] {
        let file = runtime.join(name);
        if file.exists() {
            private_read(&file)?;
            fs::remove_file(file)?;
        }
    }
    secret.nas_token = None;
    private_write(&state_path(path)?, &serde_json::to_vec_pretty(&secret)?)?;
    println!(
        "Removed the NAS registration, alerts, and saved agent credentials without contacting the NAS"
    );
    Ok(())
}

pub(super) fn configure(path: &Path) -> Result<()> {
    let inputs = inputs(path)?;
    let secrets = secrets(path)?;
    let session = Session::login(&inputs, false)?;
    configure_delivery(&session, &secrets)?;
    configure_rules(&session, &inputs.required_systems, &inputs.offline_systems)?;
    println!("Configured ntfy delivery, disk thresholds, and server connection notifications");
    Ok(())
}

fn configure_rules(session: &Session, required: &[String], offline: &[String]) -> Result<()> {
    let systems = session.records("systems")?;
    let alerts = session.records("alerts")?;
    for name in required {
        let system = systems
            .iter()
            .find(|item| item["name"] == *name)
            .with_context(|| format!("missing system {name}"))?;
        let id = string(system, "id")?;
        if !offline.contains(name) {
            for obsolete in alerts.iter().filter(|item| {
                item["system"] == id && item["name"] == "Status" && item["user"] == session.user_id
            }) {
                session.delete("alerts", string(obsolete, "id")?)?;
            }
        }
        let mut rules = vec![("Disk", 90, 10)];
        if offline.contains(name) {
            rules.push(("Status", 0, 5));
        }
        for (name, value, minutes) in rules {
            let rule =
                json!({"name":name,"value":value,"min":minutes,"system":id,"user":session.user_id});
            if let Some(existing) = alerts.iter().find(|item| {
                item["system"] == id && item["name"] == name && item["user"] == session.user_id
            }) {
                session.update("alerts", string(existing, "id")?, &rule)?;
            } else {
                session.create("alerts", &rule)?;
            }
        }
    }
    Ok(())
}

fn verify_rules(session: &Session, required: &[String], offline: &[String]) -> Result<()> {
    let systems = session.records("systems")?;
    let alerts = session.records("alerts")?;
    for name in required {
        let system = systems
            .iter()
            .find(|item| item["name"] == *name)
            .context("required system is missing")?;
        let id = string(system, "id")?;
        let rules = alerts
            .iter()
            .filter(|item| {
                item["system"] == id
                    && item["user"] == session.user_id
                    && matches!(item["name"].as_str(), Some("Disk" | "Status"))
            })
            .collect::<Vec<_>>();
        ensure!(
            rules.len() == 1 + usize::from(offline.contains(name)),
            "unexpected rules for {name}"
        );
        ensure!(
            rules
                .iter()
                .any(|item| item["name"] == "Disk" && item["value"] == 90 && item["min"] == 10),
            "disk policy changed for {name}"
        );
        if offline.contains(name) {
            ensure!(
                rules
                    .iter()
                    .any(|item| item["name"] == "Status" && item["value"] == 0 && item["min"] == 5),
                "disconnect policy changed for {name}"
            );
        }
    }
    Ok(())
}

fn configure_delivery(session: &Session, secrets: &Secrets) -> Result<()> {
    let settings = session.records("user_settings")?;
    let setting = settings
        .iter()
        .find(|item| item["user"] == session.user_id)
        .context("user settings are missing")?;
    let mut contents = setting["settings"].clone();
    contents["emails"] = json!([]);
    contents["webhooks"] = json!([format!(
        "ntfy://:{}@ntfy:8080/home-status?scheme=http",
        secrets.publisher_token
    )]);
    session.update(
        "user_settings",
        string(setting, "id")?,
        &json!({"settings":contents}),
    )?;
    Ok(())
}

pub(super) fn status(path: &Path) -> Result<()> {
    let inputs = inputs(path)?;
    let session = Session::login(&inputs, false)?;
    for system in session.records("systems")? {
        println!(
            "{}: {}",
            string(&system, "name")?,
            string(&system, "status")?
        );
    }
    Ok(())
}

pub(super) fn verify(path: &Path) -> Result<()> {
    let inputs = inputs(path)?;
    let session = Session::login(&inputs, false)?;
    let systems = session.records("systems")?;
    ensure!(
        systems.len() == inputs.required_systems.len(),
        "unexpected monitored systems"
    );
    for name in &inputs.required_systems {
        ensure!(
            systems
                .iter()
                .any(|item| item["name"] == *name && item["status"] == "up"),
            "system is not connected: {name}"
        );
    }
    verify_rules(&session, &inputs.required_systems, &inputs.offline_systems)?;
    notifications(path)?;
    println!("Four agents connected with the selected disk and disconnect rules");
    Ok(())
}

pub(super) fn notifications(path: &Path) -> Result<()> {
    let inputs = inputs(path)?;
    let secret = secrets(path)?;
    let base = validate_url(&inputs.ntfy_url)?;
    notification_checks(base, &secret)?;
    hub_notification(&Session::login(&inputs, false)?, &secret)
}

fn hub_notification(session: &Session, secret: &Secrets) -> Result<()> {
    let url = format!(
        "ntfy://:{}@ntfy:8080/home-status?scheme=http",
        secret.publisher_token
    );
    let response = session.request(
        Method::POST,
        "api/beszel/test-notification",
        Some(&json!({"url":url})),
    )?;
    ensure!(
        response["err"] == false,
        "hub-to-ntfy notification delivery failed"
    );
    println!("Hub-to-ntfy delivery passed");
    Ok(())
}

fn has_published_message(cached: &str, id: &str, message: &str) -> bool {
    cached.lines().any(|line| {
        serde_json::from_str::<Value>(line)
            .is_ok_and(|record| record["id"] == id && record["message"] == message)
    })
}

fn cached_messages(base: &Url, secret: &Secrets) -> Result<String> {
    Ok(client()?
        .get(base.join("home-status/json?poll=1&since=all")?)
        .basic_auth("iphone", Some(&secret.reader_password))
        .send()?
        .error_for_status()?
        .text()?)
}

pub(super) fn cached(path: &Path, id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= 32 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "invalid notification identity"
    );
    let inputs = inputs(path)?;
    let received = cached_messages(&validate_url(&inputs.ntfy_url)?, &secrets(path)?)?;
    ensure!(
        received
            .lines()
            .any(|line| serde_json::from_str::<Value>(line).is_ok_and(|record| record["id"] == id)),
        "notification cache no longer contains the selected message"
    );
    println!("Retained notification {id}");
    Ok(())
}

fn notification_checks(base: Url, secret: &Secrets) -> Result<()> {
    let client = client()?;
    let topic = base.join("home-status")?;
    for request in [
        client.get(base.join("home-status/json?poll=1")?),
        client
            .post(topic.clone())
            .body("Anonymous access verification"),
    ] {
        let status = request.send()?.status();
        ensure!(
            matches!(status.as_u16(), 401 | 403),
            "anonymous access was not denied"
        );
    }
    let status = client
        .post(topic.clone())
        .basic_auth("iphone", Some(&secret.reader_password))
        .body("Read-only identity verification")
        .send()?
        .status();
    ensure!(
        matches!(status.as_u16(), 401 | 403),
        "subscriber must not publish"
    );
    let status = client
        .get(base.join("home-status/json?poll=1")?)
        .bearer_auth(&secret.publisher_token)
        .send()?
        .status();
    ensure!(
        matches!(status.as_u16(), 401 | 403),
        "publisher must not subscribe"
    );
    let message = format!("Home monitoring delivery verification {}", random_text(12)?);
    let published = client
        .post(topic)
        .bearer_auth(&secret.publisher_token)
        .body(message.clone())
        .send()?
        .error_for_status()?
        .json::<Value>()?;
    let id = string(&published, "id")?;
    let received = cached_messages(&base, secret)?;
    ensure!(
        has_published_message(&received, id, &message),
        "subscriber did not retrieve the verification message"
    );
    println!("Verified notification {id}");
    println!(
        "Anonymous access and cross-role operations denied; notification publication and retrieval passed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        RotationAccess, Secrets, credential_lock, has_published_message, inputs, password_hash,
        prepare, private_replace, private_write, random_text, reader_password_resume,
        replace_reader_hash, rotate_reader_password, validate_url,
    };
    use serde_json::json;
    use std::{fs, io::Write, os::unix::fs::PermissionsExt, path::PathBuf};

    pub(super) struct Directory(pub(super) PathBuf);

    impl Directory {
        pub(super) fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("monitoring-test-{}", random_text(20).unwrap()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn config(&self, names: serde_json::Value) -> PathBuf {
            let path = self.0.join("client.json");
            private_write(&path, &serde_json::to_vec(&json!({
                "credentials_file": self.0.join("missing-credentials"),
                "public_key_file": self.0.join("missing-key"),
                "source_url":"https://fixture.example.ts.net", "hub_url":"https://fixture.example.ts.net",
                "ntfy_url":"https://fixture.example.ts.net", "pi_host":"fixture.example.ts.net",
                "required_systems":names, "offline_systems":[]
            })).unwrap()).unwrap();
            path
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn refuses_repeated_target_names() {
        let directory = Directory::new();
        let path = directory.config(json!(["Mac", "Mac", "Linux", "Pi"]));
        assert!(inputs(&path).is_err());
    }

    #[test]
    fn refuses_cleartext_tailnet_credentials() {
        assert!(validate_url("http://100.64.0.1:8090").is_err());
    }

    #[test]
    fn preserves_saved_credentials_when_a_replacement_write_fails() {
        let directory = Directory::new();
        let path = directory.0.join("state.json");
        private_write(&path, b"original").unwrap();
        let result = private_replace(&path, |file| {
            file.write_all(b"partial")?;
            anyhow::bail!("injected write failure")
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn writes_private_files_with_a_relative_leaf_name() {
        let name = format!(".credential-test-{}", random_text(24).unwrap());
        let path = std::path::Path::new(&name);
        let result = private_write(path, b"synthetic");
        let bytes = fs::read(path);
        let cleanup = fs::remove_file(path);
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(bytes.unwrap(), b"synthetic");
        cleanup.unwrap();
    }

    #[test]
    fn blocks_preparation_during_a_pending_reader_rotation() {
        let directory = Directory::new();
        let path = directory.config(json!(["Mac", "Windows", "Linux", "Pi"]));
        fs::create_dir(directory.0.join("reader-password-backup")).unwrap();
        fs::set_permissions(
            directory.0.join("reader-password-backup"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let error = prepare(&path).unwrap_err();
        assert!(format!("{error:#}").contains("pending rotation"));
        assert!(!directory.0.join("secrets.json").exists());
    }

    #[test]
    fn resumes_an_interrupted_reader_rotation_without_changing_publisher() {
        let directory = Directory::new();
        let path = directory.config(json!(["Mac", "Windows", "Linux", "Pi"]));
        let state = Secrets {
            publisher_password: "synthetic-publisher".into(),
            reader_password: "synthetic-reader".into(),
            publisher_token: "synthetic-token".into(),
            pi_token: "synthetic-agent".into(),
            nas_token: None,
        };
        fs::create_dir(directory.0.join("runtime")).unwrap();
        private_write(
            &directory.0.join("secrets.json"),
            &serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let environment = b"NTFY_AUTH_USERS=beszel:publisher-hash:user,iphone:reader-hash:user\nNTFY_AUTH_ACCESS=beszel:home-status:wo,iphone:home-status:ro\n";
        private_write(&directory.0.join("runtime/ntfy.env"), environment).unwrap();
        private_write(
            &directory.0.join("runtime/iphone.txt"),
            b"Previous synthetic phone configuration",
        )
        .unwrap();
        rotate_reader_password(&path, &"x".repeat(64)).unwrap();
        private_write(&directory.0.join("runtime/ntfy.env"), environment).unwrap();
        private_write(
            &directory.0.join("secrets.json"),
            &serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert!(prepare(&path).is_err());
        reader_password_resume(&path).unwrap();
        reader_password_resume(&path).unwrap();
        let saved: Secrets =
            serde_json::from_slice(&fs::read(directory.0.join("secrets.json")).unwrap()).unwrap();
        assert_eq!(saved.reader_password, "x".repeat(64));
        assert_eq!(saved.publisher_password, state.publisher_password);
        let updated = fs::read_to_string(directory.0.join("runtime/ntfy.env")).unwrap();
        assert!(updated.contains("beszel:publisher-hash:user"));
        assert!(!updated.contains("iphone:reader-hash:user"));
        assert!(
            directory
                .0
                .join("reader-password-backup/secrets.json")
                .exists()
        );
        private_write(
            &directory.0.join("runtime/ntfy.env"),
            b"Unrelated operator change",
        )
        .unwrap();
        assert!(reader_password_resume(&path).is_err());
        assert_eq!(
            fs::read(directory.0.join("runtime/ntfy.env")).unwrap(),
            b"Unrelated operator change"
        );
    }

    #[test]
    fn serializes_credential_operations_and_releases_the_lock() {
        let directory = Directory::new();
        let path = directory.config(json!(["Mac", "Windows", "Linux", "Pi"]));
        let lock = credential_lock(&path, RotationAccess::Stable).unwrap();
        assert!(credential_lock(&path, RotationAccess::Stable).is_err());
        drop(lock);
        assert!(credential_lock(&path, RotationAccess::Stable).is_ok());
    }

    #[test]
    fn refuses_passwords_that_bcrypt_would_truncate() {
        assert!(password_hash(&"x".repeat(100), 4).is_err());
        assert!(password_hash("line\nbreak", 4).is_err());
        assert!(password_hash("", 4).is_err());
    }

    #[test]
    fn hashes_the_full_printable_password_including_its_last_character() {
        let mut password = "Aa0!$`'\"\\,:&|~".repeat(5);
        password.truncate(64);
        let hash = password_hash(&password, 4).unwrap();
        assert!(bcrypt::verify(&password, &hash).unwrap());
        password.pop();
        password.push('Z');
        assert!(!bcrypt::verify(&password, &hash).unwrap());
        assert!(password_hash(&"x".repeat(72), 4).is_ok());
    }

    #[test]
    fn preserves_publisher_configuration_during_reader_rotation() {
        let environment = "NTFY_BASE_URL=https://fixture.example.ts.net\nNTFY_AUTH_USERS=beszel:publisher-hash:user,iphone:reader-hash:user\nNTFY_AUTH_ACCESS=beszel:home-status:wo,iphone:home-status:ro\nNTFY_AUTH_TOKENS=beszel:unchanged:Beszel\n";
        assert_eq!(
            replace_reader_hash(environment, "replacement").unwrap(),
            environment.replace("iphone:reader-hash:user", "iphone:replacement:user")
        );
        assert!(
            replace_reader_hash(
                "NTFY_AUTH_USERS=iphone:first:user,iphone:second:user\n",
                "replacement"
            )
            .is_err()
        );
        assert!(replace_reader_hash("NTFY_AUTH_USERS=iphone:hash:admin\n", "replacement").is_err());
    }

    #[test]
    fn refuses_a_cached_message_from_an_earlier_probe() {
        let cached = r#"{"id":"previous","message":"Delivery verification"}"#;
        assert!(!has_published_message(
            cached,
            "current",
            "Delivery verification"
        ));
        assert!(has_published_message(
            cached,
            "previous",
            "Delivery verification"
        ));
        assert!(!has_published_message(
            cached,
            "previous",
            "A different message"
        ));
    }

    #[test]
    fn refuses_credentials_or_external_hosts() {
        for value in [
            "https://example.com",
            "http://example.ts.net",
            "https://secret@example.ts.net",
            "https://example.ts.net/?token=secret",
        ] {
            assert!(validate_url(value).is_err());
        }
    }

    #[test]
    fn accepts_encrypted_tailnet_origins() {
        assert!(validate_url("https://device.example.ts.net:8443").is_ok());
    }
}

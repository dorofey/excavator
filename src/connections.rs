//! Non-secret connection metadata and explicit SSH trust. All I/O is blocking.
use crate::credentials;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Protocol {
    Sftp,
    Ftps,
    S3,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionRecord {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub root: String,
    pub bucket: String,
    pub region: String,
    pub endpoint: String,
    #[serde(default)]
    pub ca_bundle: String,
    /// User-defined non-secret group name; empty means Ungrouped.
    #[serde(default)]
    pub group: String,
    /// Optional path to an OpenSSH-compatible private key; key bytes remain on disk.
    #[serde(default)]
    pub ssh_key_path: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionSecrets {
    pub password: String,
    pub access_key: String,
    pub secret_key: String,
    pub session_token: String,
    #[serde(default)]
    pub ssh_key_passphrase: String,
}
impl std::fmt::Debug for ConnectionSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionSecrets { [REDACTED] }")
    }
}

impl ConnectionRecord {
    pub fn validate(&self) -> Result<(), String> {
        if !self.ca_bundle.is_empty()
            && (!std::path::Path::new(&self.ca_bundle).is_absolute()
                || self.ca_bundle.chars().any(char::is_control))
        {
            return Err("Custom CA bundle must be an absolute local file path".into());
        }
        if !self.ssh_key_path.is_empty()
            && (!std::path::Path::new(&self.ssh_key_path).is_absolute()
                || self.ssh_key_path.chars().any(char::is_control))
        {
            return Err("SSH private key must be an absolute local file path".into());
        }
        if self.id.is_empty()
            || self.id.len() > 128
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("Connection ID must contain only letters, numbers, - or _".into());
        }
        if self.name.trim().is_empty() || self.name.chars().any(char::is_control) {
            return Err("A connection name without control characters is required".into());
        }
        if self.group.len() > 128
            || self.group.chars().any(char::is_control)
            || (!self.group.is_empty() && self.group.trim() != self.group)
        {
            return Err("Connection group must be at most 128 characters without leading/trailing whitespace or control characters".into());
        }
        if self.protocol != Protocol::Sftp && !self.ssh_key_path.is_empty() {
            return Err("SSH private keys are available only for SFTP connections".into());
        }
        for field in [
            &self.host,
            &self.username,
            &self.root,
            &self.bucket,
            &self.region,
            &self.endpoint,
        ] {
            if field.chars().any(char::is_control) {
                return Err("Connection fields cannot contain control characters".into());
            }
        }
        match self.protocol {
            Protocol::Sftp | Protocol::Ftps => {
                if self.host.is_empty()
                    || self.host.trim() != self.host
                    || self.host.contains(['/', '@', '?', '#', ' '])
                    || self.port == 0
                    || self.username.trim().is_empty()
                {
                    return Err("A hostname, nonzero port and username are required".into());
                }
                let host = self.host.trim_start_matches('[').trim_end_matches(']');
                url::Host::parse(host).map_err(|_| "Invalid connection hostname".to_string())?;
                if !self.root.starts_with('/') {
                    return Err("Remote root must be an absolute path".into());
                }
                if !self.endpoint.is_empty() || !self.bucket.is_empty() || !self.region.is_empty() {
                    return Err("SFTP and FTPS cannot contain S3 settings".into());
                }
            }
            Protocol::S3 => {
                if self.bucket.is_empty()
                    || self.bucket.contains('/')
                    || self.region.trim().is_empty()
                {
                    return Err("An S3 bucket and region are required".into());
                }
                if !self.host.is_empty() || !self.username.is_empty() {
                    return Err("Use the S3 endpoint field; host and username must be empty".into());
                }
                if !self.endpoint.is_empty() {
                    let endpoint = url::Url::parse(&self.endpoint)
                        .map_err(|_| "Invalid S3 endpoint".to_string())?;
                    if !endpoint.username().is_empty()
                        || endpoint.password().is_some()
                        || endpoint.query().is_some()
                        || endpoint.fragment().is_some()
                        || endpoint.host_str().is_none()
                        || (endpoint.path() != "/" && !endpoint.path().is_empty())
                    {
                        return Err("S3 endpoint must be an origin without credentials, query, fragment or path".into());
                    }
                    let loopback = match endpoint.host() {
                        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
                        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                        None => false,
                    };
                    if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
                        return Err(
                            "S3 requires HTTPS; HTTP is allowed only for a loopback test server"
                                .into(),
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedHost {
    host: String,
    port: u16,
    fingerprint: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    version: u32,
    records: Vec<ConnectionRecord>,
    known_hosts: BTreeMap<String, TrustedHost>,
    #[serde(default)]
    groups: Vec<String>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            version: 1,
            records: vec![],
            known_hosts: BTreeMap::new(),
            groups: vec![],
        }
    }
}
static STORE_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Lock ordering is always process mutex, then the per-user file lock.
/// Closing this descriptor releases flock, including when a process exits.
fn metadata_lock() -> Result<fs::File, String> {
    let metadata_path = path()?;
    let parent = metadata_path
        .parent()
        .ok_or("Invalid connection metadata path")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot create connection metadata directory")?;
    let directory =
        fs::symlink_metadata(parent).map_err(|_| "Cannot inspect connection metadata directory")?;
    if !directory.is_dir()
        || directory.uid() != unsafe { libc::geteuid() }
        || directory.mode() & 0o022 != 0
    {
        return Err(
            "Connection metadata directory must be owned, without a symlink or group/public write permission".into(),
        );
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent.join(".connections.lock"))
        .map_err(|_| "Cannot open connection metadata lock")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect connection metadata lock")?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err("Connection metadata lock must be a private owned regular file".into());
    }
    let started = std::time::Instant::now();
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::WouldBlock && error.kind() != io::ErrorKind::Interrupted {
            return Err("Cannot lock connection metadata".into());
        }
        if started.elapsed() >= std::time::Duration::from_secs(15) {
            return Err(
                "Another process is updating connection metadata; retry when it finishes".into(),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
pub fn new_id() -> String {
    format!(
        "connection-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
fn path() -> Result<PathBuf, String> {
    if let Some(directory) = std::env::var_os("EXCAVATOR_CONFIG_DIR") {
        let directory = PathBuf::from(directory);
        if !directory.is_absolute() {
            return Err("EXCAVATOR_CONFIG_DIR must be an absolute directory".into());
        }
        return Ok(directory.join("connections.json"));
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").ok_or("Home directory is unavailable")?)
            .join("Library/Application Support/Excavator/connections.json"),
    )
}
fn read_store() -> Result<Store, String> {
    let bytes = match fs::read(path()?) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(error) => return Err(format!("Cannot read connection metadata: {error}")),
    };
    // Do not include JSON decoder text: a corrupt file could contain secret material.
    let mut store: Store = serde_json::from_slice(&bytes)
        .map_err(|_| "Connection metadata is corrupt; original file preserved".to_string())?;
    if store.version != 1 {
        return Err("Unsupported connection metadata version; original file preserved".into());
    }
    let mut ids = BTreeSet::new();
    for record in &store.records {
        record.validate()?;
        if !ids.insert(&record.id) {
            return Err("Connection metadata contains duplicate IDs".into());
        }
    }
    // Older metadata and imports may assign groups without a separate catalog.
    for record in &store.records {
        if !record.group.is_empty() && !store.groups.contains(&record.group) {
            store.groups.push(record.group.clone());
        }
    }
    store.groups.sort_by_key(|group| group.to_lowercase());
    store.groups.dedup();
    Ok(store)
}
fn write_store(store: &Store) -> Result<(), String> {
    let path = path()?;
    let parent = path.parent().ok_or("Invalid connection metadata path")?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Cannot create connection metadata directory: {e}"))?;
    let stage = parent.join(format!(".connections-{}.tmp", new_id()));
    let result = (|| -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage)
            .map_err(|e| format!("Cannot stage connection metadata: {e}"))?;
        file.write_all(
            &serde_json::to_vec_pretty(store).map_err(|_| "Cannot encode connection metadata")?,
        )
        .map_err(|e| format!("Cannot write connection metadata: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Cannot synchronize connection metadata: {e}"))?;
        fs::rename(&stage, &path)
            .map_err(|e| format!("Cannot install connection metadata: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(stage);
    }
    result
}
pub fn load() -> Result<Vec<ConnectionRecord>, String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    Ok(read_store()?.records)
}
pub fn load_groups() -> Result<Vec<String>, String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    Ok(read_store()?.groups)
}
fn validate_group_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.trim() != name
        || name.len() > 128
        || name.chars().any(char::is_control)
    {
        return Err("Group name must contain 1–128 characters without leading/trailing whitespace or control characters".into());
    }
    Ok(())
}
pub fn save_group(name: &str) -> Result<Vec<String>, String> {
    validate_group_name(name)?;
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    if store
        .groups
        .iter()
        .any(|group| group != name && group.eq_ignore_ascii_case(name))
    {
        return Err("A group with that name already exists".into());
    }
    if !store.groups.iter().any(|group| group == name) {
        store.groups.push(name.to_string());
        store.groups.sort_by_key(|group| group.to_lowercase());
        write_store(&store)?;
    }
    Ok(store.groups)
}
pub fn rename_group(old: &str, new: &str) -> Result<(Vec<String>, Vec<ConnectionRecord>), String> {
    validate_group_name(new)?;
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    rename_group_in_store(&mut store, old, new)?;
    write_store(&store)?;
    Ok((store.groups, store.records))
}
fn rename_group_in_store(store: &mut Store, old: &str, new: &str) -> Result<(), String> {
    validate_group_name(new)?;
    if old != new
        && store
            .groups
            .iter()
            .any(|group| *group != old && group.eq_ignore_ascii_case(new))
    {
        return Err("A group with that name already exists".into());
    }
    let Some(index) = store.groups.iter().position(|group| group == old) else {
        return Err("The connection group no longer exists".into());
    };
    store.groups[index] = new.to_string();
    for record in &mut store.records {
        if record.group == old {
            record.group = new.to_string();
        }
    }
    store.groups.sort_by_key(|group| group.to_lowercase());
    Ok(())
}
pub fn delete_group(name: &str) -> Result<(Vec<String>, Vec<ConnectionRecord>), String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    delete_group_in_store(&mut store, name)?;
    write_store(&store)?;
    Ok((store.groups, store.records))
}
fn delete_group_in_store(store: &mut Store, name: &str) -> Result<(), String> {
    let Some(index) = store.groups.iter().position(|group| group == name) else {
        return Err("The connection group no longer exists".into());
    };
    store.groups.remove(index);
    for record in &mut store.records {
        if record.group == name {
            record.group.clear();
        }
    }
    Ok(())
}
pub struct ImportOutcome {
    pub added: usize,
    pub duplicates: usize,
    pub records: Vec<ConnectionRecord>,
}

/// Endpoint identity intentionally preserves usernames and provider path case.
pub fn metadata_duplicate(a: &ConnectionRecord, b: &ConnectionRecord) -> bool {
    a.protocol == b.protocol
        && a.host
            .trim_end_matches('.')
            .eq_ignore_ascii_case(b.host.trim_end_matches('.'))
        && a.port == b.port
        && a.username == b.username
        && a.root == b.root
        && a.bucket == b.bucket
        && a.region == b.region
        && a.endpoint == b.endpoint
}

/// One atomic metadata-only batch; existing credentials and trust remain untouched.
pub fn import_metadata(records: &[ConnectionRecord]) -> Result<ImportOutcome, String> {
    for record in records {
        record.validate()?;
    }
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    let mut added = 0;
    let mut duplicates = 0;
    for candidate in records {
        if store
            .records
            .iter()
            .any(|old| metadata_duplicate(old, candidate))
        {
            duplicates += 1;
            continue;
        }
        let mut record = candidate.clone();
        record.id = new_id();
        store.records.push(record);
        added += 1;
    }
    if added > 0 {
        write_store(&store)?;
    }
    Ok(ImportOutcome {
        added,
        duplicates,
        records: store.records,
    })
}
pub fn save(record: &ConnectionRecord, secret: &Option<ConnectionSecrets>) -> Result<(), String> {
    record.validate()?;
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    save_locked(&mut store, record, secret)
}
/// Credential fields are password, access key, secret key, session token, and
/// SSH passphrase. None preserves each field; Some("") explicitly clears it.
pub fn save_with_patch(
    record: &ConnectionRecord,
    expected: Option<&ConnectionRecord>,
    patch: [Option<String>; 5],
) -> Result<(), String> {
    record.validate()?;
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    match expected {
        Some(expected) => {
            if expected.id != record.id {
                return Err("Connection ID changed during review".into());
            }
            require_matching_record(&store, expected)?;
        }
        None if store
            .records
            .iter()
            .any(|existing| existing.id == record.id) =>
        {
            return Err("Connection ID already exists; reopen the connection".into());
        }
        None => {}
    }
    let secret = if patch.iter().any(Option::is_some) {
        let mut secret = match credentials::get(&record.id)? {
            Some(bytes) => serde_json::from_slice::<ConnectionSecrets>(&bytes).map_err(
                |_| "Stored connection credentials are invalid; original credentials preserved",
            )?,
            None => ConnectionSecrets::default(),
        };
        let fields = [
            &mut secret.password,
            &mut secret.access_key,
            &mut secret.secret_key,
            &mut secret.session_token,
            &mut secret.ssh_key_passphrase,
        ];
        for (field, replacement) in fields.into_iter().zip(patch) {
            if let Some(replacement) = replacement {
                *field = replacement;
            }
        }
        Some(secret)
    } else {
        None
    };
    save_locked(&mut store, record, &secret)
}

fn require_matching_record(store: &Store, expected: &ConnectionRecord) -> Result<(), String> {
    let current = store
        .records
        .iter()
        .find(|record| record.id == expected.id)
        .ok_or("Connection was removed during review")?;
    if serde_json::to_vec(current).map_err(|_| "Cannot compare connection metadata")?
        != serde_json::to_vec(expected).map_err(|_| "Cannot compare connection metadata")?
    {
        return Err("Connection settings changed during review; reopen the connection".into());
    }
    Ok(())
}

fn save_locked(
    store: &mut Store,
    record: &ConnectionRecord,
    secret: &Option<ConnectionSecrets>,
) -> Result<(), String> {
    let previous_secret = if secret.is_some() {
        Some(credentials::get(&record.id)?)
    } else {
        None
    };
    if let Some(secret) = secret {
        let bytes =
            serde_json::to_vec(secret).map_err(|_| "Cannot encode connection credentials")?;
        credentials::set(&record.id, &bytes)?;
    }
    if !record.group.is_empty() && !store.groups.contains(&record.group) {
        store.groups.push(record.group.clone());
        store.groups.sort_by_key(|group| group.to_lowercase());
    }
    if let Some(old) = store.records.iter_mut().find(|old| old.id == record.id) {
        *old = record.clone();
    } else {
        store.records.push(record.clone());
    }
    if let Err(error) = write_store(store) {
        if let Some(previous) = previous_secret {
            let restored = match previous {
                Some(bytes) => credentials::set(&record.id, &bytes),
                None => credentials::remove(&record.id),
            };
            return Err(match restored {
                Ok(()) => format!("{error}; previous credentials restored"),
                Err(restore) => format!(
                    "{error}; credential recovery also failed: {restore}. Review connection credentials before reconnecting"
                ),
            });
        }
        return Err(error);
    }
    Ok(())
}
pub fn remove(id: &str) -> Result<(), String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    remove_locked(&mut store, id)
}

pub fn remove_if_matches(expected: &ConnectionRecord) -> Result<(), String> {
    expected.validate()?;
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    require_matching_record(&store, expected)?;
    remove_locked(&mut store, &expected.id)
}

fn remove_locked(store: &mut Store, id: &str) -> Result<(), String> {
    // If Keychain is locked retain the metadata so removal can be retried.
    let previous_secret = credentials::get(id)?;
    credentials::remove(id)?;
    store.records.retain(|record| record.id != id);
    store.known_hosts.remove(id);
    if let Err(error) = write_store(store) {
        if let Some(bytes) = previous_secret {
            return Err(match credentials::set(id, &bytes) {
                Ok(()) => format!("{error}; removed credentials restored"),
                Err(restore) => format!(
                    "{error}; credential recovery also failed: {restore}. Connection metadata remains but credentials may be missing"
                ),
            });
        }
        return Err(error);
    }
    Ok(())
}
pub fn secrets(id: &str) -> Result<ConnectionSecrets, String> {
    let bytes = credentials::get(id)?
        .ok_or("Connection credentials are missing; edit the connection to provide them")?;
    serde_json::from_slice(&bytes).map_err(|_| "Stored connection credentials are invalid".into())
}
pub fn secrets_or_empty_for_key(id: &str) -> Result<ConnectionSecrets, String> {
    let Some(bytes) = credentials::get(id)? else {
        return Ok(ConnectionSecrets::default());
    };
    serde_json::from_slice(&bytes).map_err(|_| "Stored connection credentials are invalid".into())
}
pub fn known_host(record: &ConnectionRecord) -> Result<Option<String>, String> {
    record.validate()?;
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    Ok(read_store()?
        .known_hosts
        .get(&record.id)
        .filter(|known| known.host.eq_ignore_ascii_case(&record.host) && known.port == record.port)
        .map(|known| known.fingerprint.clone()))
}
pub fn trust_host(record: &ConnectionRecord, fingerprint: &str) -> Result<(), String> {
    record.validate()?;
    if record.protocol != Protocol::Sftp {
        return Err("Host key trust applies only to SFTP".into());
    }
    let digest = fingerprint
        .strip_prefix("SHA256:")
        .ok_or("An SHA256 SSH fingerprint is required")?;
    if digest.len() != 43
        || !digest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return Err("Invalid SHA256 SSH fingerprint".into());
    }
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    if let Some(current) = store.records.iter().find(|current| current.id == record.id)
        && (current.protocol != record.protocol
            || current.host != record.host
            || current.port != record.port)
    {
        return Err(
            "Connection endpoint changed during host key review; inspect the host again".into(),
        );
    }
    if let Some(old) = store.known_hosts.get(&record.id) {
        if old.host.eq_ignore_ascii_case(&record.host)
            && old.port == record.port
            && old.fingerprint != fingerprint
        {
            return Err(
                "Host key changed; explicitly forget the trusted key before trusting a replacement"
                    .into(),
            );
        }
    }
    store.known_hosts.insert(
        record.id.clone(),
        TrustedHost {
            host: record.host.clone(),
            port: record.port,
            fingerprint: fingerprint.into(),
        },
    );
    write_store(&store)
}
pub fn forget_host(record: &ConnectionRecord) -> Result<(), String> {
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    store.known_hosts.remove(&record.id);
    write_store(&store)
}

/// Atomically reset only the endpoint and fingerprint explicitly reviewed.
pub fn forget_host_if_matches(
    record: &ConnectionRecord,
    expected_fingerprint: &str,
) -> Result<(), String> {
    record.validate()?;
    if record.protocol != Protocol::Sftp {
        return Err("Host key trust applies only to SFTP".into());
    }
    let _guard = STORE_LOCK
        .lock()
        .map_err(|_| "Connection metadata lock failed")?;
    let _file_guard = metadata_lock()?;
    let mut store = read_store()?;
    let current = store
        .records
        .iter()
        .find(|current| current.id == record.id)
        .ok_or("Connection was removed during host key review")?;
    if current.protocol != record.protocol
        || current.host != record.host
        || current.port != record.port
    {
        return Err(
            "Connection endpoint changed during host key review; inspect the host again".into(),
        );
    }
    let trusted = store
        .known_hosts
        .get(&record.id)
        .ok_or("Trusted key changed during review; inspect the host again")?;
    if !trusted.host.eq_ignore_ascii_case(&record.host)
        || trusted.port != record.port
        || trusted.fingerprint != expected_fingerprint
    {
        return Err("Trusted key changed during review; inspect the host again".into());
    }
    store.known_hosts.remove(&record.id);
    write_store(&store)
}

#[cfg(test)]
mod group_management_tests {
    use super::*;
    #[test]
    fn names_reject_blank_padding_and_controls() {
        for name in ["", " padded", "padded ", "line\nbreak"] {
            assert!(validate_group_name(name).is_err());
        }
        assert!(validate_group_name("Production").is_ok());
    }
    #[test]
    fn rename_and_remove_keep_records_and_ungroup_members() {
        let mut store = Store::default();
        store.groups = vec!["Production".into(), "Staging".into()];
        let mut record: ConnectionRecord = serde_json::from_value(serde_json::json!({"id":"fixture", "name":"Server", "protocol":"Sftp", "host":"example.test", "port":22, "username":"developer", "root":"/", "bucket":"", "region":"", "endpoint":"", "group":"Production"})).unwrap();
        record.group = "Production".into();
        store.records.push(record.clone());
        assert!(rename_group_in_store(&mut store, "Production", "staging").is_err());
        rename_group_in_store(&mut store, "Production", "PRODUCTION").unwrap();
        assert_eq!(store.records[0].group, "PRODUCTION");
        delete_group_in_store(&mut store, "PRODUCTION").unwrap();
        record.group.clear();
        assert_eq!(
            serde_json::to_value(&store.records[0]).unwrap(),
            serde_json::to_value(&record).unwrap()
        );
        assert_eq!(store.groups, vec!["Staging"]);
        assert!(delete_group_in_store(&mut store, "PRODUCTION").is_err());
    }
}

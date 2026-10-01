//! Protocol adapters own all third-party types. All methods and stream I/O are
//! blocking and must run on a background worker, including stream destruction.
use super::{CancellationToken, FileSystem, ListOptions, ProviderFuture, local::LocalFileSystem};
use crate::{
    connections::{self, ConnectionRecord, ConnectionSecrets, Protocol},
    domain::{Capabilities, Entry, EntryId, EntryKind, FsError, FsErrorKind, Location, ProviderId},
};
use std::os::unix::fs::MetadataExt;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};
const TIMEOUT: Duration = Duration::from_secs(15);
const PART_SIZE: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 250_000;
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    overrides:
        std::sync::Arc<std::collections::HashMap<String, (ConnectionRecord, ConnectionSecrets)>>,
    trust: std::sync::Arc<std::collections::HashMap<String, Option<String>>>,
}

fn error(location: &Location, kind: FsErrorKind, message: impl Into<String>) -> FsError {
    FsError {
        kind,
        location: location.clone(),
        message: message.into(),
    }
}
fn check(cancel: &CancellationToken, location: &Location) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(FsError::cancelled(location.clone()))
    } else {
        Ok(())
    }
}
fn unsupported(location: &Location, message: &str) -> FsError {
    error(location, FsErrorKind::Unsupported, message)
}
fn loc(record: &ConnectionRecord) -> Location {
    match record.protocol {
        Protocol::Sftp => Location::Sftp {
            connection: record.id.clone(),
            path: record.root.clone(),
        },
        Protocol::Ftps => Location::Ftps {
            connection: record.id.clone(),
            path: record.root.clone(),
        },
        Protocol::S3 => Location::S3 {
            connection: record.id.clone(),
            bucket: record.bucket.clone(),
            key: record.root.clone(),
            prefix: true,
        },
    }
}
fn config(location: &Location) -> Result<(ConnectionRecord, ConnectionSecrets), FsError> {
    let id = location
        .connection_id()
        .ok_or_else(|| unsupported(location, "Expected a remote connection"))?;
    let record = connections::load()
        .map_err(|_| {
            error(
                location,
                FsErrorKind::Io,
                "Unable to load connection metadata",
            )
        })?
        .into_iter()
        .find(|record| record.id == id)
        .ok_or_else(|| error(location, FsErrorKind::NotFound, "Connection was removed"))?;
    let expected = match location {
        Location::Sftp { .. } => Protocol::Sftp,
        Location::Ftps { .. } => Protocol::Ftps,
        Location::S3 { .. } => Protocol::S3,
        _ => unreachable!(),
    };
    if record.protocol != expected {
        return Err(error(
            location,
            FsErrorKind::InvalidOperation,
            "Location protocol does not match connection",
        ));
    }
    record.validate().map_err(|_| {
        error(
            location,
            FsErrorKind::InvalidOperation,
            "Connection settings are invalid",
        )
    })?;
    let secrets = connections::secrets(id).map_err(|_| {
        error(
            location,
            FsErrorKind::Authentication,
            "Connection credentials are missing, locked, or invalid; edit the connection",
        )
    })?;
    Ok((record, secrets))
}
fn tcp(record: &ConnectionRecord) -> Result<TcpStream, String> {
    let host = record.host.trim_start_matches('[').trim_end_matches(']');
    let addresses = (host, record.port)
        .to_socket_addrs()
        .map_err(|_| "Unable to resolve remote host".to_string())?;
    for addr in addresses.take(8) {
        if let Ok(socket) = TcpStream::connect_timeout(&addr, TIMEOUT) {
            socket
                .set_read_timeout(Some(TIMEOUT))
                .map_err(|_| "Unable to set network timeout")?;
            socket
                .set_write_timeout(Some(TIMEOUT))
                .map_err(|_| "Unable to set network timeout")?;
            return Ok(socket);
        }
    }
    Err("Unable to connect to remote host within network timeout".into())
}
fn ssh_handshake(record: &ConnectionRecord) -> Result<(ssh2::Session, String), String> {
    let socket = tcp(record)?;
    let mut session = ssh2::Session::new().map_err(|_| "Unable to initialize SSH")?;
    session.set_timeout(TIMEOUT.as_millis() as u32);
    session.set_tcp_stream(socket);
    session.handshake().map_err(|_| "SSH handshake failed")?;
    let hash = session
        .host_key_hash(ssh2::HashType::Sha256)
        .ok_or("SSH server did not supply a SHA256 host key")?;
    let fingerprint = format!("SHA256:{}", base64_unpadded(hash));
    Ok((session, fingerprint))
}
fn base64_unpadded(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut value = 0u32;
    let mut bits = 0u8;
    let mut out = String::new();
    for byte in bytes {
        value = (value << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(TABLE[((value >> bits) & 63) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(TABLE[((value << (6 - bits)) & 63) as usize] as char);
    }
    out
}
pub fn probe_host(record: &ConnectionRecord) -> Result<String, String> {
    record.validate()?;
    if record.protocol != Protocol::Sftp {
        return Err("SSH host key probe applies only to SFTP".into());
    }
    ssh_handshake(record).map(|(_, fingerprint)| fingerprint)
}
fn sftp(
    record: &ConnectionRecord,
    secrets: &ConnectionSecrets,
    location: &Location,
    cancel: &CancellationToken,
    trust_override: Option<Option<&str>>,
) -> Result<ssh2::Sftp, FsError> {
    check(cancel, location)?;
    let (session, fingerprint) =
        ssh_handshake(record).map_err(|message| error(location, FsErrorKind::Io, message))?;
    let trusted = match trust_override {
        Some(value) => value.map(str::to_string),
        None => connections::known_host(record).map_err(|_| {
            error(
                location,
                FsErrorKind::Io,
                "Unable to read SSH trust metadata",
            )
        })?,
    };
    match trusted {
        None => {
            return Err(error(
                location,
                FsErrorKind::HostKeyUnknown,
                format!(
                    "Untrusted SSH host key {fingerprint}; verify and explicitly trust this fingerprint"
                ),
            ));
        }
        Some(trusted) if trusted != fingerprint => {
            return Err(error(
                location,
                FsErrorKind::HostKeyChanged,
                format!("SSH host key changed to {fingerprint}; connection rejected"),
            ));
        }
        _ => {}
    }
    check(cancel, location)?;
    session
        .userauth_password(&record.username, &secrets.password)
        .map_err(|_| {
            error(
                location,
                FsErrorKind::Authentication,
                "SSH authentication failed",
            )
        })?;
    session.sftp().map_err(|_| {
        error(
            location,
            FsErrorKind::Io,
            "Unable to initialize SFTP subsystem",
        )
    })
}
fn ftp(
    record: &ConnectionRecord,
    secrets: &ConnectionSecrets,
    location: &Location,
    cancel: &CancellationToken,
) -> Result<suppaftp::NativeTlsFtpStream, FsError> {
    check(cancel, location)?;
    let socket = tcp(record).map_err(|message| error(location, FsErrorKind::Io, message))?;
    let control_peer = socket
        .peer_addr()
        .map_err(|_| error(location, FsErrorKind::Io, "Cannot determine FTPS peer"))?
        .ip();
    let stream = suppaftp::NativeTlsFtpStream::connect_with_stream(socket)
        .map_err(|_| error(location, FsErrorKind::Io, "FTP greeting failed"))?;
    let mut builder = suppaftp::native_tls::TlsConnector::builder();
    if !record.ca_bundle.is_empty() {
        let pem = fs::read(&record.ca_bundle).map_err(|_| {
            error(
                location,
                FsErrorKind::Tls,
                "Cannot read configured TLS CA certificate",
            )
        })?;
        let certificate = suppaftp::native_tls::Certificate::from_pem(&pem).map_err(|_| {
            error(
                location,
                FsErrorKind::Tls,
                "Configured TLS CA certificate is invalid",
            )
        })?;
        builder.add_root_certificate(certificate);
    }
    let tls = builder.build().map_err(|_| {
        error(
            location,
            FsErrorKind::Tls,
            "Cannot initialize TLS verification",
        )
    })?;
    let mut stream = stream
        .into_secure(suppaftp::NativeTlsConnector::from(tls), &record.host)
        .map_err(|_| {
            error(
                location,
                FsErrorKind::Tls,
                "FTPS TLS certificate or hostname verification failed",
            )
        })?;
    stream = stream.passive_stream_builder(move |addr| {
        let addr = std::net::SocketAddr::new(control_peer, addr.port());
        let socket = TcpStream::connect_timeout(&addr, TIMEOUT)
            .map_err(suppaftp::FtpError::ConnectionError)?;
        socket
            .set_read_timeout(Some(TIMEOUT))
            .map_err(suppaftp::FtpError::ConnectionError)?;
        socket
            .set_write_timeout(Some(TIMEOUT))
            .map_err(suppaftp::FtpError::ConnectionError)?;
        Ok(socket)
    });
    stream
        .login(&record.username, &secrets.password)
        .map_err(|_| {
            error(
                location,
                FsErrorKind::Authentication,
                "FTPS authentication failed",
            )
        })?;
    stream
        .transfer_type(suppaftp::types::FileType::Binary)
        .map_err(|_| {
            error(
                location,
                FsErrorKind::Io,
                "FTPS binary transfer mode failed",
            )
        })?;
    check(cancel, location)?;
    Ok(stream)
}
fn safe_s3_key(location: &Location) -> Result<(), FsError> {
    if let Location::S3 { key, .. } = location {
        if key.starts_with('/') || key.split('/').any(|part| part == "." || part == "..") {
            return Err(unsupported(
                location,
                "This S3 client cannot safely address leading-slash or dot-segment keys; item preserved",
            ));
        }
    }
    Ok(())
}
fn retry_s3<T>(
    mut action: impl FnMut() -> Result<T, s3::error::S3Error>,
) -> Result<T, s3::error::S3Error> {
    for attempt in 0..3 {
        match action() {
            Ok(value) => return Ok(value),
            Err(error) if attempt == 2 => return Err(error),
            Err(s3::error::S3Error::HttpFailWithBody(status, body))
                if matches!(status, 400 | 401 | 403 | 404 | 409 | 412) =>
            {
                return Err(s3::error::S3Error::HttpFailWithBody(status, body));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    unreachable!()
}
fn bucket(
    record: &ConnectionRecord,
    secrets: &ConnectionSecrets,
    location: &Location,
) -> Result<Box<s3::Bucket>, FsError> {
    safe_s3_key(location)?;
    record.validate().map_err(|_| {
        error(
            location,
            FsErrorKind::InvalidOperation,
            "Invalid S3 connection settings",
        )
    })?;
    if let Location::S3 { bucket, .. } = location {
        if bucket != &record.bucket {
            return Err(error(
                location,
                FsErrorKind::InvalidOperation,
                "Location bucket does not match connection",
            ));
        }
    }
    let region = if record.endpoint.is_empty() {
        record.region.parse().map_err(|_| {
            error(
                location,
                FsErrorKind::InvalidOperation,
                "S3 region is invalid",
            )
        })?
    } else {
        s3::Region::Custom {
            region: record.region.clone(),
            endpoint: record.endpoint.clone(),
        }
    };
    let credentials = s3::creds::Credentials::new(
        Some(&secrets.access_key),
        Some(&secrets.secret_key),
        None,
        if secrets.session_token.is_empty() {
            None
        } else {
            Some(&secrets.session_token)
        },
        None,
    )
    .map_err(|_| {
        error(
            location,
            FsErrorKind::Authentication,
            "S3 credentials are invalid",
        )
    })?;
    let mut bucket = s3::Bucket::new(&record.bucket, region, credentials).map_err(|_| {
        error(
            location,
            FsErrorKind::InvalidOperation,
            "Unable to initialize S3 bucket",
        )
    })?;
    if !record.endpoint.is_empty() {
        bucket = bucket.with_path_style();
    }
    bucket.set_request_timeout(Some(TIMEOUT));
    Ok(bucket)
}
fn ssh_error(location: &Location, e: ssh2::Error) -> FsError {
    let kind = match e.code() {
        ssh2::ErrorCode::SFTP(2) => FsErrorKind::NotFound,
        ssh2::ErrorCode::SFTP(3) => FsErrorKind::PermissionDenied,
        ssh2::ErrorCode::SFTP(11) => FsErrorKind::Conflict,
        _ => FsErrorKind::Io,
    };
    error(
        location,
        kind,
        "SFTP operation failed; check access, path, connection, and timeout",
    )
}
fn ftp_error(location: &Location, e: suppaftp::FtpError) -> FsError {
    let kind = match &e {
        suppaftp::FtpError::UnexpectedResponse(response) if response.status.code() == 550 => {
            FsErrorKind::NotFound
        }
        _ => FsErrorKind::Io,
    };
    error(
        location,
        kind,
        "FTPS operation failed; check access, path, connection, and timeout",
    )
}
fn s3_error(location: &Location, e: s3::error::S3Error) -> FsError {
    let kind = match e {
        s3::error::S3Error::HttpFailWithBody(404, _) => FsErrorKind::NotFound,
        s3::error::S3Error::HttpFailWithBody(403, _) => FsErrorKind::PermissionDenied,
        s3::error::S3Error::HttpFailWithBody(409 | 412, _) => FsErrorKind::Conflict,
        _ => FsErrorKind::Io,
    };
    error(
        location,
        kind,
        "S3 request failed; check access, object, endpoint, TLS, and timeout",
    )
}
fn entry(
    location: Location,
    kind: EntryKind,
    size: Option<u64>,
    modified: Option<std::time::SystemTime>,
) -> Entry {
    Entry {
        id: EntryId(location.clone()),
        name: OsString::from(location.label()),
        location,
        kind,
        size,
        modified,
    }
}
fn ssh_entry(location: Location, stat: ssh2::FileStat) -> Entry {
    let kind = match stat.file_type() {
        ssh2::FileType::Directory => EntryKind::Directory,
        ssh2::FileType::RegularFile => EntryKind::File,
        ssh2::FileType::Symlink => EntryKind::Symlink,
        _ => EntryKind::Other,
    };
    entry(
        location,
        kind,
        if kind == EntryKind::File {
            stat.size
        } else {
            None
        },
        stat.mtime
            .and_then(|t| UNIX_EPOCH.checked_add(Duration::from_secs(t))),
    )
}
fn ftp_entry(location: Location, file: suppaftp::list::File) -> Entry {
    let kind = if file.is_symlink() {
        EntryKind::Symlink
    } else if file.is_directory() {
        EntryKind::Directory
    } else if file.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    entry(
        location,
        kind,
        if kind == EntryKind::File {
            Some(file.size() as u64)
        } else {
            None
        },
        Some(file.modified()),
    )
}
fn remote_path(location: &Location) -> Result<&str, FsError> {
    match location {
        Location::Sftp { path, .. } | Location::Ftps { path, .. } => {
            if !path.starts_with('/') || path.contains('\0') || path.contains(['\r', '\n']) {
                return Err(error(
                    location,
                    FsErrorKind::InvalidOperation,
                    "Remote path must be absolute and contain no protocol control characters",
                ));
            }
            Ok(path)
        }
        _ => Err(unsupported(location, "Expected a remote filesystem path")),
    }
}

impl FileSystem for ProviderRegistry {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Registry
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            list: true,
            metadata: true,
            ..Default::default()
        }
    }
    fn list(
        &self,
        location: Location,
        options: ListOptions,
        cancel: CancellationToken,
    ) -> ProviderFuture<Vec<Entry>> {
        let registry = self.clone();
        Box::pin(async move { registry.list_sync(&location, options, &cancel) })
    }
    fn metadata(&self, location: Location, cancel: CancellationToken) -> ProviderFuture<Entry> {
        let registry = self.clone();
        Box::pin(async move { registry.metadata_sync(&location, &cancel) })
    }
}
impl ProviderRegistry {
    /// Explicit dependency injection for disposable fixtures; production default
    /// resolves secrets exclusively from the operating-system credential store.
    pub fn capabilities_for(&self, location: &Location) -> Capabilities {
        if location.is_local() {
            return LocalFileSystem.capabilities();
        }
        Capabilities {
            list: true,
            metadata: true,
            rename: matches!(location, Location::Sftp { .. }),
            seek: matches!(location, Location::S3 { .. }),
            ..Capabilities::default()
        }
    }
    pub fn with_connections(records: Vec<(ConnectionRecord, ConnectionSecrets)>) -> Self {
        let trust = std::sync::Arc::new(
            records
                .iter()
                .map(|pair| (pair.0.id.clone(), None))
                .collect(),
        );
        Self {
            trust,
            overrides: std::sync::Arc::new(
                records
                    .into_iter()
                    .map(|pair| (pair.0.id.clone(), pair))
                    .collect(),
            ),
        }
    }
    pub fn with_trusted_host(mut self, connection: &str, fingerprint: String) -> Self {
        std::sync::Arc::make_mut(&mut self.trust).insert(connection.to_string(), Some(fingerprint));
        self
    }
    fn sftp(
        &self,
        record: &ConnectionRecord,
        secrets: &ConnectionSecrets,
        location: &Location,
        cancel: &CancellationToken,
    ) -> Result<ssh2::Sftp, FsError> {
        sftp(
            record,
            secrets,
            location,
            cancel,
            self.trust.get(&record.id).map(|value| value.as_deref()),
        )
    }
    fn config(
        &self,
        location: &Location,
    ) -> Result<(ConnectionRecord, ConnectionSecrets), FsError> {
        if let Some(pair) = location
            .connection_id()
            .and_then(|id| self.overrides.get(id))
        {
            pair.0.validate().map_err(|_| {
                error(
                    location,
                    FsErrorKind::InvalidOperation,
                    "Injected connection settings are invalid",
                )
            })?;
            let protocol = match location {
                Location::Sftp { .. } => Protocol::Sftp,
                Location::Ftps { .. } => Protocol::Ftps,
                Location::S3 { .. } => Protocol::S3,
                _ => return Err(unsupported(location, "Expected a remote location")),
            };
            if pair.0.protocol != protocol {
                return Err(error(
                    location,
                    FsErrorKind::InvalidOperation,
                    "Location protocol mismatch",
                ));
            }
            return Ok(pair.clone());
        }
        config(location)
    }
    pub fn list_sync(
        &self,
        location: &Location,
        options: ListOptions,
        cancel: &CancellationToken,
    ) -> Result<Vec<Entry>, FsError> {
        check(cancel, location)?;
        if location.is_local() {
            return ready_local(LocalFileSystem.list(location.clone(), options, cancel.clone()));
        }
        let (record, secrets) = self.config(location)?;
        let mut entries = Vec::new();
        match location {
            Location::Sftp { .. } => {
                let sftp = self.sftp(&record, &secrets, location, cancel)?;
                let mut directory = sftp
                    .opendir(Path::new(remote_path(location)?))
                    .map_err(|e| ssh_error(location, e))?;
                loop {
                    check(cancel, location)?;
                    let (name, stat) = match directory.readdir() {
                        Ok(item) => item,
                        Err(e) if e.code() == ssh2::ErrorCode::Session(-16) => break,
                        Err(e) => return Err(ssh_error(location, e)),
                    };
                    let name = name.to_str().ok_or_else(|| {
                        unsupported(
                            location,
                            "Remote filename is not UTF-8; cannot address it safely",
                        )
                    })?;
                    if name == "." || name == ".." {
                        continue;
                    }
                    let child = location.join(std::ffi::OsStr::new(name))?;
                    entries.push(ssh_entry(child, stat));
                    if entries.len() > MAX_ENTRIES {
                        return Err(unsupported(
                            location,
                            "Directory exceeds the safe listing limit",
                        ));
                    }
                }
            }
            Location::Ftps { .. } => {
                let mut ftp = ftp(&record, &secrets, location, cancel)?;
                for line in ftp
                    .mlsd(Some(remote_path(location)?))
                    .map_err(|e| ftp_error(location, e))?
                {
                    check(cancel, location)?;
                    let file = suppaftp::list::ListParser::parse_mlsd(&line).map_err(|_| {
                        error(
                            location,
                            FsErrorKind::Io,
                            "FTPS MLSD listing contains an unsupported entry",
                        )
                    })?;
                    if file.name() == "." || file.name() == ".." {
                        continue;
                    }
                    let child = location.join(std::ffi::OsStr::new(file.name()))?;
                    entries.push(ftp_entry(child, file));
                    if entries.len() > MAX_ENTRIES {
                        return Err(unsupported(
                            location,
                            "Directory exceeds the safe listing limit",
                        ));
                    }
                }
            }
            Location::S3 {
                connection,
                bucket: bucket_name,
                key,
                prefix,
            } => {
                if !prefix {
                    return Err(error(
                        location,
                        FsErrorKind::NotDirectory,
                        "An S3 object is not a prefix listing",
                    ));
                }
                let bucket = bucket(&record, &secrets, location)?;
                let mut token = None;
                let mut seen = std::collections::HashSet::new();
                loop {
                    check(cancel, location)?;
                    let (page, _) = retry_s3(|| {
                        check(cancel, location).map_err(|_| s3::error::S3Error::HttpFail)?;
                        bucket.list_page(
                            key.clone(),
                            Some("/".into()),
                            token.clone(),
                            None,
                            Some(1000),
                        )
                    })
                    .map_err(|e| s3_error(location, e))?;
                    for common in page.common_prefixes.unwrap_or_default() {
                        let child = Location::S3 {
                            connection: connection.clone(),
                            bucket: bucket_name.clone(),
                            key: common.prefix,
                            prefix: true,
                        };
                        entries.push(entry(child, EntryKind::Directory, None, None));
                    }
                    for object in page.contents {
                        let child = Location::S3 {
                            connection: connection.clone(),
                            bucket: bucket_name.clone(),
                            key: object.key,
                            prefix: false,
                        };
                        entries.push(entry(
                            child,
                            EntryKind::File,
                            Some(object.size),
                            parse_time(&object.last_modified),
                        ));
                    }
                    if entries.len() > MAX_ENTRIES {
                        return Err(unsupported(
                            location,
                            "Prefix exceeds the safe listing limit",
                        ));
                    }
                    if !page.is_truncated {
                        break;
                    }
                    let next = page.next_continuation_token.ok_or_else(|| {
                        error(
                            location,
                            FsErrorKind::Io,
                            "S3 truncated listing omitted its continuation token",
                        )
                    })?;
                    if !seen.insert(next.clone()) {
                        return Err(error(
                            location,
                            FsErrorKind::Io,
                            "S3 repeated its continuation token",
                        ));
                    }
                    token = Some(next);
                }
            }
            Location::Local(_) => unreachable!(),
        }
        entries
            .retain(|entry| options.show_hidden || !entry.name.to_string_lossy().starts_with('.'));
        entries.sort_by(|a, b| {
            (a.kind != EntryKind::Directory)
                .cmp(&(b.kind != EntryKind::Directory))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.location.display().cmp(&b.location.display()))
        });
        check(cancel, location)?;
        Ok(entries)
    }
    pub fn metadata_sync(
        &self,
        location: &Location,
        cancel: &CancellationToken,
    ) -> Result<Entry, FsError> {
        check(cancel, location)?;
        if location.is_local() {
            return ready_local(LocalFileSystem.metadata(location.clone(), cancel.clone()));
        }
        let (record, secrets) = self.config(location)?;
        match location {
            Location::Sftp { .. } => {
                let client = self.sftp(&record, &secrets, location, cancel)?;
                Ok(ssh_entry(
                    location.clone(),
                    client
                        .lstat(Path::new(remote_path(location)?))
                        .map_err(|e| ssh_error(location, e))?,
                ))
            }
            Location::Ftps { .. } => {
                let mut client = ftp(&record, &secrets, location, cancel)?;
                let line = client
                    .mlst(Some(remote_path(location)?))
                    .map_err(|e| ftp_error(location, e))?;
                let parsed = suppaftp::list::ListParser::parse_mlst(&line)
                    .map_err(|_| error(location, FsErrorKind::Io, "Cannot parse FTPS metadata"))?;
                Ok(ftp_entry(location.clone(), parsed))
            }
            Location::S3 { key, prefix, .. } => {
                if *prefix {
                    self.list_sync(location, ListOptions { show_hidden: true }, cancel)?;
                    return Ok(entry(location.clone(), EntryKind::Directory, None, None));
                }
                let bucket = bucket(&record, &secrets, location)?;
                let (meta, _) = bucket.head_object(key).map_err(|e| s3_error(location, e))?;
                Ok(entry(
                    location.clone(),
                    EntryKind::File,
                    meta.content_length.and_then(|n| u64::try_from(n).ok()),
                    meta.last_modified.as_deref().and_then(parse_time),
                ))
            }
            _ => unreachable!(),
        }
    }
    pub fn open_read(
        &self,
        location: &Location,
        cancel: &CancellationToken,
    ) -> Result<Box<dyn Read + Send>, FsError> {
        check(cancel, location)?;
        let metadata = self.metadata_sync(location, cancel)?;
        if metadata.kind != EntryKind::File {
            return Err(unsupported(
                location,
                "Streaming copy supports regular files only; symlinks are never followed",
            ));
        }
        if let Location::Local(path) = location {
            let before =
                fs::symlink_metadata(path).map_err(|e| FsError::from_io(location.clone(), e))?;
            let input = File::open(path).map_err(|e| FsError::from_io(location.clone(), e))?;
            let opened = input
                .metadata()
                .map_err(|e| FsError::from_io(location.clone(), e))?;
            if !before.file_type().is_file()
                || before.dev() != opened.dev()
                || before.ino() != opened.ino()
            {
                return Err(error(
                    location,
                    FsErrorKind::InvalidOperation,
                    "Local source changed while opening",
                ));
            }
            return Ok(Box::new(input));
        }
        let (record, secrets) = self.config(location)?;
        match location {
            Location::Sftp { .. } => {
                let sftp = self.sftp(&record, &secrets, location, cancel)?;
                let mut file = sftp
                    .open(Path::new(remote_path(location)?))
                    .map_err(|e| ssh_error(location, e))?;
                let opened = file.stat().map_err(|e| ssh_error(location, e))?;
                if opened.file_type() != ssh2::FileType::RegularFile || opened.size != metadata.size
                {
                    return Err(error(
                        location,
                        FsErrorKind::InvalidOperation,
                        "SFTP source changed while opening",
                    ));
                }
                Ok(Box::new(file))
            }
            Location::Ftps { .. } => {
                let mut client = ftp(&record, &secrets, location, cancel)?;
                let stream = client
                    .retr_as_stream(remote_path(location)?)
                    .map_err(|e| ftp_error(location, e))?;
                Ok(Box::new(FtpReader {
                    stream: Some(stream),
                    _client: client,
                }))
            }
            Location::S3 { key, .. } => {
                let mut bucket = bucket(&record, &secrets, location)?;
                let (head, _) = bucket.head_object(key).map_err(|e| s3_error(location, e))?;
                let etag = head.e_tag.ok_or_else(|| {
                    unsupported(
                        location,
                        "S3 source has no ETag for consistent streaming reads",
                    )
                })?;
                bucket.add_header("If-Match", &etag);
                let length = head
                    .content_length
                    .and_then(|n| u64::try_from(n).ok())
                    .ok_or_else(|| unsupported(location, "S3 source length is unavailable"))?;
                Ok(Box::new(S3Reader {
                    bucket,
                    key: key.clone(),
                    offset: 0,
                    length,
                    cancel: cancel.clone(),
                }))
            }
            _ => unreachable!(),
        }
    }
    pub fn create_write(
        &self,
        location: &Location,
        cancel: &CancellationToken,
    ) -> Result<Box<dyn TransferWriter>, FsError> {
        check(cancel, location)?;
        if let Location::Local(target) = location {
            let parent = target.parent().ok_or_else(|| {
                error(
                    location,
                    FsErrorKind::InvalidOperation,
                    "Destination needs a parent directory",
                )
            })?;
            let stage = parent.join(format!(".excavator-remote-stage-{}", connections::new_id()));
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)
                .map_err(|e| FsError::from_io(location.clone(), e))?;
            return Ok(Box::new(LocalWriter {
                location: location.clone(),
                stage,
                file,
                done: false,
                committed: false,
            }));
        }
        let (record, secrets) = self.config(location)?;
        match location {
            Location::Sftp { .. } => {
                let client = self.sftp(&record, &secrets, location, cancel)?;
                let parent = location.parent().ok_or_else(|| {
                    error(
                        location,
                        FsErrorKind::InvalidOperation,
                        "Destination needs a parent",
                    )
                })?;
                let stage = parent.join(std::ffi::OsStr::new(&format!(
                    ".excavator-stage-{}",
                    connections::new_id()
                )))?;
                let file = client
                    .open_mode(
                        Path::new(remote_path(&stage)?),
                        ssh2::OpenFlags::WRITE
                            | ssh2::OpenFlags::CREATE
                            | ssh2::OpenFlags::EXCLUSIVE,
                        0o600,
                        ssh2::OpenType::File,
                    )
                    .map_err(|e| ssh_error(&stage, e))?;
                Ok(Box::new(SftpWriter {
                    client,
                    file: Some(file),
                    location: location.clone(),
                    stage,
                    done: false,
                    committed: false,
                }))
            }
            Location::Ftps { .. } => Err(unsupported(
                location,
                "FTPS STOR/RNTO cannot guarantee exclusive destination creation; uploads and replacement are disabled",
            )),
            Location::S3 { key, prefix, .. } => {
                if *prefix || key.is_empty() {
                    return Err(unsupported(
                        location,
                        "Upload requires an S3 object key, not a prefix",
                    ));
                }
                let bucket = bucket(&record, &secrets, location)?;
                Ok(Box::new(S3Writer {
                    bucket,
                    location: location.clone(),
                    key: key.clone(),
                    buffer: Vec::with_capacity(PART_SIZE),
                    upload: None,
                    parts: Vec::new(),
                    done: false,
                    total: 0,
                    committed: false,
                }))
            }
            _ => unreachable!(),
        }
    }
    pub fn validate_copy(
        &self,
        source: &Location,
        parent: &Location,
        cancel: &CancellationToken,
    ) -> Result<(), FsError> {
        if matches!(parent, Location::Ftps { .. }) {
            return Err(unsupported(
                parent,
                "FTPS uploads cannot guarantee exclusive destination creation; copy to local, SFTP, or S3 instead",
            ));
        }
        if source.connection_id() != parent.connection_id()
            && matches!(
                (source, parent),
                (Location::Sftp { .. }, Location::Sftp { .. })
                    | (Location::S3 { .. }, Location::S3 { .. })
            )
        {
            if self.metadata_sync(source, cancel)?.kind == EntryKind::Directory {
                return Err(unsupported(
                    source,
                    "Directory copies across saved connections of the same protocol are disabled until distinct backend identity can be established",
                ));
            }
        }
        if source.is_local() && parent.is_local() {
            return Err(unsupported(
                source,
                "Mixed-provider batches cannot route local-to-local operations through the remote engine",
            ));
        }
        if let (Location::Sftp { connection: a, .. }, Location::Sftp { connection: b, .. }) =
            (source, parent)
        {
            if a == b {
                let (record, secrets) = self.config(source)?;
                let client = self.sftp(&record, &secrets, source, cancel)?;
                let real_source = client
                    .realpath(Path::new(remote_path(source)?))
                    .map_err(|e| ssh_error(source, e))?;
                let real_parent = client
                    .realpath(Path::new(remote_path(parent)?))
                    .map_err(|e| ssh_error(parent, e))?;
                let metadata = client
                    .lstat(Path::new(remote_path(source)?))
                    .map_err(|e| ssh_error(source, e))?;
                if metadata.file_type() == ssh2::FileType::Directory
                    && real_parent.starts_with(&real_source)
                {
                    return Err(error(
                        source,
                        FsErrorKind::InvalidOperation,
                        "Remote destination resolves inside the selected source tree",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn create_dir(
        &self,
        location: &Location,
        cancel: &CancellationToken,
    ) -> Result<(), FsError> {
        check(cancel, location)?;
        if let Location::Local(path) = location {
            return fs::create_dir(path).map_err(|e| FsError::from_io(location.clone(), e));
        }
        let (record, secrets) = self.config(location)?;
        match location {
            Location::Sftp { .. } => self
                .sftp(&record, &secrets, location, cancel)?
                .mkdir(Path::new(remote_path(location)?), 0o755)
                .map_err(|e| ssh_error(location, e)),
            Location::Ftps { .. } => ftp(&record, &secrets, location, cancel)?
                .mkdir(remote_path(location)?)
                .map_err(|e| ftp_error(location, e)),
            Location::S3 { .. } => Err(unsupported(
                location,
                "S3 prefixes are created by uploading objects; empty folders are not filesystem directories",
            )),
            _ => unreachable!(),
        }
    }
    pub fn rename(
        &self,
        source: &Location,
        target: &Location,
        cancel: &CancellationToken,
    ) -> Result<(), FsError> {
        check(cancel, source)?;
        if source.connection_id() != target.connection_id() {
            return Err(unsupported(
                source,
                "Remote rename must stay within one connection",
            ));
        }
        let (record, secrets) = self.config(source)?;
        match (source, target) {
            (Location::Sftp { .. }, Location::Sftp { .. }) => {
                let client = self.sftp(&record, &secrets, source, cancel)?;
                client
                    .rename(
                        Path::new(remote_path(source)?),
                        Path::new(remote_path(target)?),
                        Some(ssh2::RenameFlags::empty()),
                    )
                    .map_err(|e| ssh_error(target, e))
            }
            (Location::Ftps { .. }, _) => Err(unsupported(
                source,
                "FTPS RNTO may overwrite an existing destination; safe rename is disabled",
            )),
            _ => Err(unsupported(
                source,
                "This provider has no safe native rename",
            )),
        }
    }
    /// Permanent deletion only; caller must present an explicit confirmation.
    /// Never recursive: files and empty real directories only.
    pub fn delete(&self, location: &Location, cancel: &CancellationToken) -> Result<(), FsError> {
        check(cancel, location)?;
        let metadata = self.metadata_sync(location, cancel)?;
        let (record, secrets) = self.config(location)?;
        match location {
            Location::Sftp { .. } => {
                let client = self.sftp(&record, &secrets, location, cancel)?;
                if metadata.kind == EntryKind::Directory {
                    client
                        .rmdir(Path::new(remote_path(location)?))
                        .map_err(|e| ssh_error(location, e))
                } else if metadata.kind == EntryKind::File || metadata.kind == EntryKind::Symlink {
                    client
                        .unlink(Path::new(remote_path(location)?))
                        .map_err(|e| ssh_error(location, e))
                } else {
                    Err(unsupported(
                        location,
                        "Special remote files cannot be deleted",
                    ))
                }
            }
            Location::Ftps { .. } => {
                let mut client = ftp(&record, &secrets, location, cancel)?;
                if metadata.kind == EntryKind::Directory {
                    client
                        .rmdir(remote_path(location)?)
                        .map_err(|e| ftp_error(location, e))
                } else if metadata.kind == EntryKind::File {
                    client
                        .rm(remote_path(location)?)
                        .map_err(|e| ftp_error(location, e))
                } else {
                    Err(unsupported(
                        location,
                        "FTPS symlinks and special files cannot be deleted safely",
                    ))
                }
            }
            Location::S3 { key, prefix, .. } => {
                if *prefix {
                    return Err(unsupported(
                        location,
                        "S3 prefix deletion is disabled; select individual objects",
                    ));
                }
                let mut bucket = bucket(&record, &secrets, location)?;
                let (meta, _) = bucket.head_object(key).map_err(|e| s3_error(location, e))?;
                let etag = meta.e_tag.ok_or_else(|| {
                    unsupported(location, "S3 object lacks a deletion precondition ETag")
                })?;
                bucket.add_header("If-Match", &etag);
                bucket
                    .delete_object(key)
                    .map_err(|e| s3_error(location, e))?;
                Ok(())
            }
            _ => Err(unsupported(
                location,
                "Local deletion belongs to the local safety engine",
            )),
        }
    }
}
fn ready_local<T>(mut future: ProviderFuture<T>) -> Result<T, FsError> {
    use std::task::{Context, Poll, Waker};
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(result) => result,
        Poll::Pending => unreachable!("local provider futures do not suspend"),
    }
}
pub fn test_connection(
    record: &ConnectionRecord,
    secrets: &ConnectionSecrets,
    cancel: CancellationToken,
) -> Result<String, FsError> {
    let location = loc(record);
    record.validate().map_err(|_| {
        error(
            &location,
            FsErrorKind::InvalidOperation,
            "Connection settings are invalid",
        )
    })?;
    check(&cancel, &location)?;
    match record.protocol {
        Protocol::Sftp => {
            let client = sftp(record, secrets, &location, &cancel, None)?;
            client
                .lstat(Path::new(&record.root))
                .map_err(|e| ssh_error(&location, e))?;
        }
        Protocol::Ftps => {
            let mut client = ftp(record, secrets, &location, &cancel)?;
            client
                .mlst(Some(&record.root))
                .map_err(|e| ftp_error(&location, e))?;
        }
        Protocol::S3 => {
            let client = bucket(record, secrets, &location)?;
            client
                .list_page(record.root.clone(), Some("/".into()), None, None, Some(1))
                .map_err(|e| s3_error(&location, e))?;
        }
    }
    check(&cancel, &location)?;
    Ok("Connection verified".into())
}

struct FtpReader<T: suppaftp::TlsStream> {
    stream: Option<suppaftp::TransferStream<T>>,
    _client: suppaftp::ImplFtpStream<T>,
}
impl<T: suppaftp::TlsStream> Read for FtpReader<T> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(0);
        };
        let count = stream.read(buffer)?;
        if count == 0 {
            self.stream
                .take()
                .unwrap()
                .finish()
                .map_err(|_| io::Error::other("FTPS transfer completion failed"))?;
        }
        Ok(count)
    }
}
struct RangeBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for RangeBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other(
                "S3 range response exceeded bounded buffer",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct S3Reader {
    bucket: Box<s3::Bucket>,
    key: String,
    offset: u64,
    length: u64,
    cancel: CancellationToken,
}
impl Read for S3Reader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Transfer cancelled",
            ));
        }
        if output.is_empty() || self.offset == self.length {
            return Ok(0);
        }
        let count = output
            .len()
            .min(256 * 1024)
            .min((self.length - self.offset) as usize);
        let mut writer = RangeBuffer {
            bytes: Vec::with_capacity(count),
            limit: count,
        };
        let status = retry_s3(|| {
            if self.cancel.is_cancelled() {
                return Err(s3::error::S3Error::HttpFail);
            }
            writer.bytes.clear();
            self.bucket.get_object_range_to_writer(
                &self.key,
                self.offset,
                Some(self.offset + count as u64 - 1),
                &mut writer,
            )
        })
        .map_err(|_| io::Error::other("S3 ranged read failed; source may have changed"))?;
        if !(200..300).contains(&status) || writer.bytes.len() != count {
            return Err(io::Error::other(
                "S3 ranged read returned an unexpected length",
            ));
        }
        output[..count].copy_from_slice(&writer.bytes);
        self.offset += count as u64;
        Ok(count)
    }
}
pub trait TransferWriter: Write + Send {
    fn committed(&self) -> bool;
    fn finish(&mut self, cancel: &CancellationToken) -> Result<(), FsError>;
    fn abort(&mut self) -> Result<(), FsError>;
}
struct LocalWriter {
    location: Location,
    stage: PathBuf,
    file: File,
    done: bool,
    committed: bool,
}
impl Write for LocalWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl TransferWriter for LocalWriter {
    fn committed(&self) -> bool {
        self.committed
    }
    fn finish(&mut self, cancel: &CancellationToken) -> Result<(), FsError> {
        check(cancel, &self.location)?;
        self.file
            .sync_all()
            .map_err(|e| FsError::from_io(self.location.clone(), e))?;
        fs::hard_link(&self.stage, self.location.local_path()).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                error(
                    &self.location,
                    FsErrorKind::Conflict,
                    "Destination appeared while copying; existing item retained",
                )
            } else {
                FsError::from_io(self.location.clone(), e)
            }
        })?;
        self.done = true;
        self.committed = true;
        fs::remove_file(&self.stage).map_err(|_| {
            error(
                &Location::Local(self.stage.clone()),
                FsErrorKind::Io,
                format!(
                    "Destination committed; staging cleanup failed at {}",
                    self.stage.display()
                ),
            )
        })?;
        Ok(())
    }
    fn abort(&mut self) -> Result<(), FsError> {
        if !self.done {
            fs::remove_file(&self.stage).map_err(|e| FsError::from_io(self.location.clone(), e))?;
            self.done = true;
        }
        Ok(())
    }
}
struct SftpWriter {
    client: ssh2::Sftp,
    file: Option<ssh2::File>,
    location: Location,
    stage: Location,
    done: bool,
    committed: bool,
}
impl Write for SftpWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("Upload is closed"))?
            .write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("Upload is closed"))?
            .flush()
    }
}
impl TransferWriter for SftpWriter {
    fn committed(&self) -> bool {
        self.committed
    }
    fn finish(&mut self, cancel: &CancellationToken) -> Result<(), FsError> {
        check(cancel, &self.location)?;
        if let Some(mut file) = self.file.take() {
            file.close().map_err(|e| ssh_error(&self.stage, e))?;
        }
        self.client
            .rename(
                Path::new(remote_path(&self.stage)?),
                Path::new(remote_path(&self.location)?),
                Some(ssh2::RenameFlags::empty()),
            )
            .map_err(|e| ssh_error(&self.location, e))?;
        self.done = true;
        self.committed = true;
        Ok(())
    }
    fn abort(&mut self) -> Result<(), FsError> {
        self.file.take();
        if !self.done {
            self.client
                .unlink(Path::new(remote_path(&self.stage)?))
                .map_err(|e| ssh_error(&self.stage, e))?;
            self.done = true;
        }
        Ok(())
    }
}
struct S3Writer {
    bucket: Box<s3::Bucket>,
    location: Location,
    key: String,
    buffer: Vec<u8>,
    upload: Option<String>,
    parts: Vec<s3::serde_types::Part>,
    done: bool,
    committed: bool,
    total: u64,
}
impl S3Writer {
    fn upload_part(&mut self) -> Result<(), FsError> {
        if self.parts.len() >= 10_000 {
            return Err(unsupported(
                &self.location,
                "Upload exceeds 10,000 bounded multipart parts",
            ));
        }
        if self.upload.is_none() {
            self.upload = Some(
                self.bucket
                    .initiate_multipart_upload(&self.key, "application/octet-stream")
                    .map_err(|e| s3_error(&self.location, e))?
                    .upload_id,
            );
        }
        let part = retry_s3(|| {
            self.bucket.put_multipart_chunk(
                &self.buffer,
                &self.key,
                self.parts.len() as u32 + 1,
                self.upload.as_ref().unwrap(),
                "application/octet-stream",
            )
        })
        .map_err(|e| s3_error(&self.location, e))?;
        self.parts.push(part);
        self.buffer.clear();
        Ok(())
    }
}
impl Write for S3Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.done {
            return Err(io::Error::other("Upload is closed"));
        }
        let count = bytes.len().min(PART_SIZE - self.buffer.len());
        self.buffer.extend_from_slice(&bytes[..count]);
        self.total += count as u64;
        if self.buffer.len() == PART_SIZE {
            self.upload_part()
                .map_err(|_| io::Error::other("S3 multipart upload failed"))?;
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl TransferWriter for S3Writer {
    fn committed(&self) -> bool {
        self.committed
    }
    fn finish(&mut self, cancel: &CancellationToken) -> Result<(), FsError> {
        check(cancel, &self.location)?;
        let mut conditional = self.bucket.clone();
        conditional.add_header("If-None-Match", "*");
        if let Some(upload) = self.upload.clone() {
            if !self.buffer.is_empty() {
                self.upload_part()?;
            }
            check(cancel, &self.location)?;
            let response = conditional
                .complete_multipart_upload(&self.key, &upload, std::mem::take(&mut self.parts))
                .map_err(|e| s3_error(&self.location, e))?;
            if !(200..300).contains(&response.status_code()) {
                return Err(error(
                    &self.location,
                    FsErrorKind::Io,
                    "S3 multipart completion failed",
                ));
            }
            if !completion_success(response.as_slice()) {
                return Err(error(
                    &self.location,
                    FsErrorKind::Io,
                    "S3 multipart completion returned an error document",
                ));
            }
        } else {
            conditional
                .put_object(&self.key, &self.buffer)
                .map_err(|e| s3_error(&self.location, e))?;
        }
        self.committed = true;
        self.upload = None;
        let (head, _) = retry_s3(|| self.bucket.head_object(&self.key))
            .map_err(|e| s3_error(&self.location, e))?;
        if head.content_length.and_then(|n| u64::try_from(n).ok()) != Some(self.total) {
            return Err(error(
                &self.location,
                FsErrorKind::Io,
                "S3 committed object length differs; source retained, destination needs review",
            ));
        }
        self.done = true;
        self.upload = None;
        self.buffer.clear();
        Ok(())
    }
    fn abort(&mut self) -> Result<(), FsError> {
        if let Some(upload) = self.upload.as_ref() {
            self.bucket.abort_upload(&self.key, upload).map_err(|_| {
                error(
                    &self.location,
                    FsErrorKind::Io,
                    format!("S3 upload cleanup failed; upload ID {upload} retained for recovery"),
                )
            })?;
            self.upload = None;
        }
        self.buffer.clear();
        self.done = true;
        Ok(())
    }
}

fn completion_success(bytes: &[u8]) -> bool {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut found = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                if !found {
                    if element.local_name().as_ref() != b"CompleteMultipartUploadResult" {
                        return false;
                    }
                    found = true;
                } else if element.local_name().as_ref() == b"Error" {
                    return false;
                }
            }
            Ok(Event::Empty(element)) => {
                if !found || element.local_name().as_ref() == b"Error" {
                    return false;
                }
            }
            Ok(Event::Eof) => return found,
            Err(_) => return false,
            _ => {}
        }
    }
}

fn parse_time(value: &str) -> Option<std::time::SystemTime> {
    chrono::DateTime::parse_from_rfc3339(value)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(value))
        .ok()
        .map(Into::into)
}

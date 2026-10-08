//! Same-user, bounded cross-instance discovery. All filesystem/socket/CLI methods
//! except `publish` and `take_invalidation` must run off the terminal event loop.
use crate::domain::Location;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const VERSION: u16 = 1;
const MAX_FRAME: usize = 64 * 1024;
const MAX_INSTANCES: usize = 128;
const DEADLINE: Duration = Duration::from_millis(1200);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub terminal_id: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Peer {
    /// Shell targets are local destinations only; runtime restricts them to copy.
    pub is_shell: bool,
    pub instance_id: String,
    pub pid: u32,
    pub location: Location,
    pub generation: u64,
    pub identity: Option<Identity>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "provider", deny_unknown_fields)]
enum WireLocation {
    Local {
        bytes: Vec<u8>,
    },
    Sftp {
        connection: String,
        path: String,
    },
    Ftps {
        connection: String,
        path: String,
    },
    S3 {
        connection: String,
        bucket: String,
        key: String,
        prefix: bool,
    },
}
impl From<&Location> for WireLocation {
    fn from(location: &Location) -> Self {
        match location {
            Location::Local(path) => Self::Local {
                bytes: path.as_os_str().as_bytes().to_vec(),
            },
            Location::Sftp { connection, path } => Self::Sftp {
                connection: connection.clone(),
                path: path.clone(),
            },
            Location::Ftps { connection, path } => Self::Ftps {
                connection: connection.clone(),
                path: path.clone(),
            },
            Location::S3 {
                connection,
                bucket,
                key,
                prefix,
            } => Self::S3 {
                connection: connection.clone(),
                bucket: bucket.clone(),
                key: key.clone(),
                prefix: *prefix,
            },
        }
    }
}
impl WireLocation {
    fn location(self) -> Result<Location, String> {
        let location = match self {
            Self::Local { bytes } => {
                if bytes.contains(&0) {
                    return Err("Peer sent a local path containing NUL.".into());
                }
                let path = PathBuf::from(OsString::from_vec(bytes));
                if !path.is_absolute() {
                    return Err("Peer sent a relative local path.".into());
                }
                Location::Local(path)
            }
            Self::Sftp { connection, path } => Location::Sftp { connection, path },
            Self::Ftps { connection, path } => Location::Ftps { connection, path },
            Self::S3 {
                connection,
                bucket,
                key,
                prefix,
            } => Location::S3 {
                connection,
                bucket,
                key,
                prefix,
            },
        };
        Ok(location)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u16,
    instance_id: String,
    pid: u32,
    startup_identity: Option<Identity>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "request", deny_unknown_fields)]
enum Request {
    Current {
        version: u16,
    },
    Invalidate {
        version: u16,
        locations: Vec<WireLocation>,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "response", deny_unknown_fields)]
enum Response {
    Current {
        version: u16,
        instance_id: String,
        pid: u32,
        location: WireLocation,
        generation: u64,
        identity: Option<Identity>,
    },
    Acknowledged {
        version: u16,
    },
    Error {
        version: u16,
        message: String,
    },
}
struct Shared {
    location: Location,
    generation: u64,
}
pub struct Coordination {
    directory: PathBuf,
    instance_id: String,
    shared: Arc<Mutex<Shared>>,
    invalidated: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    socket_inode: u64,
    record_inode: u64,
}
impl Coordination {
    /// Starts one bounded listener worker. Call from a startup/background worker.
    pub fn start(location: Location) -> Result<Self, String> {
        let directory = runtime_directory()?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let instance_id = format!("{}-{nonce:x}", std::process::id());
        let socket = directory.join(format!("{instance_id}.sock"));
        if socket.as_os_str().as_bytes().len() >= 104 {
            return Err("Coordination socket path exceeds macOS limit.".into());
        }
        let listener = UnixListener::bind(&socket)
            .map_err(|e| format!("Cannot create coordination socket: {e}"))?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let socket_inode = fs::symlink_metadata(&socket)
            .map_err(|e| e.to_string())?
            .ino();
        let record = Record {
            version: VERSION,
            instance_id: instance_id.clone(),
            pid: std::process::id(),
            startup_identity: live_identity().ok(),
        };
        let record_path = directory.join(format!("{instance_id}.json"));
        let temporary = directory.join(format!("{instance_id}.tmp"));
        let result = (|| {
            let bytes = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temporary, &record_path).map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_file(&socket);
            return Err(error);
        }
        let record_inode = fs::symlink_metadata(&record_path)
            .map_err(|e| e.to_string())?
            .ino();
        let shared = Arc::new(Mutex::new(Shared {
            location,
            generation: 0,
        }));
        let invalidated = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_shared = shared.clone();
        let worker_invalidation = invalidated.clone();
        let worker_stop = stop.clone();
        let worker_id = instance_id.clone();
        thread::Builder::new()
            .name("tui-coordination".into())
            .spawn(move || {
                // One request at a time bounds concurrency and memory; clients have deadlines.
                while !worker_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if verify_peer(&stream).is_err() {
                                continue;
                            }
                            let _ = stream.set_read_timeout(Some(DEADLINE));
                            let _ = stream.set_write_timeout(Some(DEADLINE));
                            let response = match read_frame::<Request>(&mut stream) {
                                Ok(Request::Current { version }) if version == VERSION => {
                                    let identity = live_identity().ok();
                                    let state =
                                        worker_shared.lock().unwrap_or_else(|p| p.into_inner());
                                    Response::Current {
                                        version: VERSION,
                                        instance_id: worker_id.clone(),
                                        pid: std::process::id(),
                                        location: (&state.location).into(),
                                        generation: state.generation,
                                        identity,
                                    }
                                }
                                Ok(Request::Invalidate { version, locations })
                                    if version == VERSION && locations.len() <= MAX_INSTANCES =>
                                {
                                    let state =
                                        worker_shared.lock().unwrap_or_else(|p| p.into_inner());
                                    if locations
                                        .into_iter()
                                        .filter_map(|location| location.location().ok())
                                        .any(|location| location == state.location)
                                    {
                                        worker_invalidation.store(true, Ordering::Release);
                                    }
                                    Response::Acknowledged { version: VERSION }
                                }
                                Ok(_) => Response::Error {
                                    version: VERSION,
                                    message:
                                        "Incompatible coordination protocol or oversized request."
                                            .into(),
                                },
                                Err(error) => Response::Error {
                                    version: VERSION,
                                    message: error,
                                },
                            };
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            let _ = write_frame(&mut stream, &response);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(30))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| {
                let _ = fs::remove_file(&socket);
                let _ = fs::remove_file(&record_path);
                e.to_string()
            })?;
        Ok(Self {
            directory,
            instance_id,
            shared,
            invalidated,
            stop,
            socket_inode,
            record_inode,
        })
    }
    /// Exit-only cleanup: stops new requests and unlinks only this instance's
    /// original owned inode artifacts. Safe to call repeatedly.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        for (suffix, inode) in [("sock", self.socket_inode), ("json", self.record_inode)] {
            let path = self
                .directory
                .join(format!("{}.{suffix}", self.instance_id));
            if fs::symlink_metadata(&path)
                .is_ok_and(|m| m.uid() == uid() && m.ino() == inode && !m.file_type().is_symlink())
            {
                let _ = fs::remove_file(path);
            }
        }
    }
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
    /// Pure in-memory publication; generation never decreases.
    pub fn publish(&self, location: Location, generation: u64) {
        let mut state = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if generation >= state.generation {
            state.location = location;
            state.generation = generation;
        }
    }
    pub fn take_invalidation(&self) -> bool {
        self.invalidated.swap(false, Ordering::AcqRel)
    }
    /// Explicit discovery also works outside herdr. Directional discovery fails closed.
    pub fn discover(&self, direction: Option<Direction>) -> Result<Vec<Peer>, String> {
        verify_directory(&self.directory)?;
        let mut peers = Vec::new();
        let mut incompatible = 0;
        let deadline = Instant::now() + Duration::from_secs(8);
        for record in records(&self.directory)? {
            if Instant::now() >= deadline {
                return Err("Destination discovery deadline expired. Retry after unresponsive instances exit.".into());
            }
            if record.version != VERSION {
                incompatible += 1;
                continue;
            }
            if record.instance_id == self.instance_id {
                continue;
            }
            match self.current(&record.instance_id) {
                Ok(peer) if peer.pid == record.pid => peers.push(peer),
                Err(error) if error.contains("protocol") => incompatible += 1,
                _ => (), // Dead/stale records are not destinations; never delete another instance's files.
            }
        }
        // Shell working directories are discovered only in the caller's visible
        // tab. Failure outside Herdr must not disable ordinary browser discovery.
        let shell_scope = (|| {
            let identity = live_identity()?;
            verify_visible_scope(&identity)?;
            let layout = live_layout(&identity)?;
            Ok::<_, String>((identity, layout))
        })();
        if let Ok((identity, layout)) = &shell_scope {
            for (pane, _) in layout {
                if Instant::now() >= deadline {
                    return Err("Destination discovery deadline expired. Retry.".into());
                }
                if pane != &identity.pane_id
                    && !peers.iter().any(|peer| {
                        peer.identity
                            .as_ref()
                            .is_some_and(|other| other.pane_id == *pane)
                    })
                    && let Ok(peer) = shell_peer(pane, identity)
                {
                    peers.push(peer);
                }
            }
        }
        if let Some(direction) = direction {
            let (identity, layout) = shell_scope?;
            let origin = layout
                .iter()
                .find(|(pane, _)| pane == &identity.pane_id)
                .ok_or("Current pane is absent from live herdr layout.")?
                .1;
            peers.retain(|peer| {
                peer.identity.as_ref().is_some_and(|other| {
                    other.workspace_id == identity.workspace_id
                        && other.tab_id == identity.tab_id
                        && other.pane_id != identity.pane_id
                })
            });
            let mut ranked = Vec::new();
            for peer in peers {
                let target = layout
                    .iter()
                    .find(|(pane, _)| {
                        Some(pane.as_str()) == peer.identity.as_ref().map(|i| i.pane_id.as_str())
                    })
                    .map(|(_, rectangle)| *rectangle);
                if let Some(rectangle) = target
                    && let Some(rank) = directional_rank(origin, rectangle, direction)
                {
                    ranked.push((rank, peer));
                }
            }
            ranked.sort_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then_with(|| a.1.instance_id.cmp(&b.1.instance_id))
            });
            peers = ranked.into_iter().map(|(_, peer)| peer).collect();
        } else {
            peers.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        }
        if incompatible > 0 {
            return Err("An incompatible coordination protocol instance was found. Stop or update that instance, then retry.".into());
        }
        Ok(peers)
    }
    pub fn validate(&self, peer: &Peer) -> Result<Peer, String> {
        let current = if peer.is_shell {
            let identity = live_identity()?;
            verify_visible_scope(&identity)?;
            let layout = live_layout(&identity)?;
            let target = peer
                .identity
                .as_ref()
                .ok_or("Shell target identity is absent.")?;
            if target.workspace_id != identity.workspace_id
                || target.tab_id != identity.tab_id
                || target.pane_id == identity.pane_id
                || !layout.iter().any(|(pane, _)| pane == &target.pane_id)
            {
                return Err(
                    "Shell destination moved or is no longer visible. Review again.".into(),
                );
            }
            shell_peer(&target.pane_id, &identity)?
        } else {
            self.current(&peer.instance_id)?
        };
        if current.is_shell != peer.is_shell
            || current.instance_id != peer.instance_id
            || current.pid != peer.pid
            || current.generation != peer.generation
            || current.location != peer.location
            || current.identity != peer.identity
        {
            return Err("Destination changed. Choose and review it again.".into());
        }
        Ok(current)
    }
    /// Best-effort completion refresh. Notification failure never fails a transfer.
    pub fn notify_locations(&self, locations: &[Location]) {
        if locations.iter().any(|location| {
            self.shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .location
                == *location
        }) {
            self.invalidated.store(true, Ordering::Release);
        }
        if let Ok(records) = records(&self.directory) {
            let locations: Vec<_> = locations
                .iter()
                .take(MAX_INSTANCES)
                .map(WireLocation::from)
                .collect();
            for record in records {
                if record.instance_id != self.instance_id {
                    let _ = request(
                        &self.directory,
                        &record.instance_id,
                        &Request::Invalidate {
                            version: VERSION,
                            locations: locations.clone(),
                        },
                    );
                }
            }
        }
    }
    fn current(&self, id: &str) -> Result<Peer, String> {
        match request(&self.directory, id, &Request::Current { version: VERSION })? {
            Response::Current {
                version,
                instance_id,
                pid,
                location,
                generation,
                identity,
            } if version == VERSION && instance_id == id => Ok(Peer {
                is_shell: false,
                instance_id,
                pid,
                location: location.location()?,
                generation,
                identity,
            }),
            Response::Error { message, .. } => {
                Err(format!("Coordination protocol rejected: {message}"))
            }
            _ => Err("Incompatible coordination protocol response.".into()),
        }
    }
}
impl Drop for Coordination {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
fn runtime_directory() -> Result<PathBuf, String> {
    let path = PathBuf::from(format!("/private/tmp/excavator-tui-{}", uid()));
    match fs::create_dir(&path) {
        Ok(()) => fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(format!("Cannot create private runtime directory: {error}")),
    }
    verify_directory(&path)?;
    Ok(path)
}
fn verify_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != uid()
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(
            "Runtime directory must be owner-only, owned by this user, and not a symlink.".into(),
        );
    }
    Ok(())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}
fn records(directory: &Path) -> Result<Vec<Record>, String> {
    verify_directory(directory)?;
    let mut records = Vec::new();
    let mut count = 0;
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        count += 1;
        if count > MAX_INSTANCES {
            return Err("Coordination registry exceeds the 128-instance limit. Remove stale owned records after stopping their processes.".into());
        }
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(entry.path())
        {
            Ok(file) => file,
            Err(_) => continue,
        };
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.uid() != uid()
            || metadata.mode() & 0o777 != 0o600
            || metadata.len() > MAX_FRAME as u64
        {
            continue;
        }
        let record: Record = match serde_json::from_reader(file.take(MAX_FRAME as u64)) {
            Ok(record) => record,
            Err(_) => continue,
        };
        if !valid_id(&record.instance_id)
            || entry.path().file_stem().and_then(|s| s.to_str()) != Some(&record.instance_id)
        {
            continue;
        }
        // Retain versions so handshake incompatibility is reported instead of silently hidden.
        records.push(record);
    }
    Ok(records)
}
fn request(directory: &Path, id: &str, request: &Request) -> Result<Response, String> {
    verify_directory(directory)?;
    if !valid_id(id) {
        return Err("Invalid coordination instance identifier.".into());
    }
    let socket = directory.join(format!("{id}.sock"));
    let metadata = fs::symlink_metadata(&socket)
        .map_err(|_| "Destination exited or is unavailable.".to_string())?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != uid()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err("Destination socket ownership or permissions are unsafe.".into());
    }
    let mut stream = connect_deadline(&socket)?;
    verify_peer(&stream)?;
    stream
        .set_read_timeout(Some(DEADLINE))
        .map_err(|e| format!("socket read timeout: {e}"))?;
    stream
        .set_write_timeout(Some(DEADLINE))
        .map_err(|e| format!("socket write timeout: {e}"))?;
    write_frame(&mut stream, request)?;
    read_frame(&mut stream)
}
fn connect_deadline(path: &Path) -> Result<UnixStream, String> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err("Cannot create coordination connection.".into());
    }
    // OwnedFd closes every error path. Only the socket created here is modified.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    #[cfg(target_os = "macos")]
    {
        let enabled: libc::c_int = 1;
        if unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_NOSIGPIPE,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as libc::socklen_t,
            )
        } != 0
        {
            return Err("Cannot protect coordination socket writes.".into());
        }
    }
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err("Cannot protect coordination descriptor inheritance.".into());
    }
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err("Cannot set coordination connection deadline.".into());
    }
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() {
        return Err("Coordination socket path is too long.".into());
    }
    for (destination, byte) in address.sun_path.iter_mut().zip(bytes) {
        *destination = *byte as libc::c_char;
    }
    let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = length as u8;
    }
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length,
        )
    };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if !matches!(
            error.raw_os_error(),
            Some(libc::EINPROGRESS) | Some(libc::EAGAIN)
        ) {
            return Err("Destination exited or is unavailable.".into());
        }
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, DEADLINE.as_millis() as i32) } <= 0 {
            return Err("Coordination connect deadline expired.".into());
        }
        let mut socket_error: libc::c_int = 0;
        let mut error_length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut socket_error as *mut libc::c_int).cast(),
                &mut error_length,
            )
        } < 0
            || socket_error != 0
        {
            return Err("Destination exited or is unavailable.".into());
        }
    }
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags) } < 0 {
        return Err("Cannot restore coordination socket mode.".into());
    }
    Ok(UnixStream::from(fd))
}
fn verify_peer(stream: &UnixStream) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "macos")]
    {
        let mut peer_uid = 0;
        let mut peer_gid = 0;
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut peer_uid, &mut peer_gid) } != 0
            || peer_uid != uid()
        {
            return Err("Coordination peer is not the current user.".into());
        }
    }
    #[cfg(target_os = "linux")]
    {
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut length,
            )
        } != 0
            || credentials.uid != uid()
        {
            return Err("Coordination peer is not the current user.".into());
        }
    }
    Ok(())
}
fn wait_socket(
    stream: &UnixStream,
    events: libc::c_short,
    deadline: Instant,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or("Coordination frame deadline expired.")?;
    let mut poll = libc::pollfd {
        fd: stream.as_raw_fd(),
        events,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut poll, 1, remaining.as_millis().max(1) as i32) };
    if result <= 0 {
        return Err("Coordination frame deadline expired.".into());
    }
    if poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err("Coordination socket failed.".into());
    }
    Ok(())
}
fn write_frame(stream: &mut UnixStream, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FRAME {
        return Err("Coordination message exceeds 64 KiB.".into());
    }
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let mut frame = Vec::with_capacity(bytes.len() + 4);
    frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(&bytes);
    let deadline = Instant::now() + DEADLINE;
    let mut remaining = frame.as_slice();
    while !remaining.is_empty() {
        wait_socket(stream, libc::POLLOUT, deadline)?;
        match stream.write(remaining) {
            Ok(0) => return Err("Coordination connection closed during write.".into()),
            Ok(count) => remaining = &remaining[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(format!("Coordination write failed: {error}")),
        }
    }
    Ok(())
}
fn read_bytes(stream: &mut UnixStream, bytes: &mut [u8], deadline: Instant) -> Result<(), String> {
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let mut offset = 0;
    while offset < bytes.len() {
        wait_socket(stream, libc::POLLIN, deadline)?;
        match stream.read(&mut bytes[offset..]) {
            Ok(0) => return Err("Coordination connection closed during read.".into()),
            Ok(count) => offset += count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(format!("Coordination read failed: {error}")),
        }
    }
    Ok(())
}
fn read_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> Result<T, String> {
    let deadline = Instant::now() + DEADLINE;
    let mut header = [0; 4];
    read_bytes(stream, &mut header, deadline)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err("Coordination frame exceeds 64 KiB or is empty.".into());
    }
    let mut bytes = vec![0; length];
    read_bytes(stream, &mut bytes, deadline)?;
    serde_json::from_slice(&bytes).map_err(|_| "Malformed coordination protocol message.".into())
}
fn herdr_json(arguments: &[&str]) -> Result<serde_json::Value, String> {
    if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
        return Err("Directional discovery requires a live herdr caller pane. Choose a destination manually.".into());
    }
    // Capture to owner-only temporary files, not pipes that can deadlock on a full buffer.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let path = runtime_directory()?.join(format!("cli-{}-{nonce:x}.tmp", std::process::id()));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let output = file.try_clone().map_err(|e| e.to_string())?;
    let result = (|| {
        let mut child = Command::new("herdr")
            .args(arguments)
            .stdout(Stdio::from(output))
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Cannot query herdr: {e}"))?;
        let start = Instant::now();
        loop {
            if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FRAME as u64 {
                let _ = child.kill();
                let _ = child.wait();
                return Err("herdr layout response exceeds 64 KiB.".into());
            }
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) if status.success() => break,
                Some(_) => {
                    return Err(
                        "herdr caller pane is unavailable. Directional targeting is disabled."
                            .into(),
                    );
                }
                None if start.elapsed() < Duration::from_millis(800) => {
                    thread::sleep(Duration::from_millis(10))
                }
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("herdr discovery timed out.".into());
                }
            }
        }
        if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FRAME as u64 {
            return Err("herdr layout response exceeds 64 KiB.".into());
        }
        // stdout's shared file offset is at EOF; read a separate no-follow descriptor.
        let input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_reader(input.take(MAX_FRAME as u64))
            .map_err(|_| "Malformed herdr response.".to_string())?;
        if value.get("error").is_some() {
            return Err("herdr rejected caller identity.".into());
        }
        Ok(value)
    })();
    let _ = fs::remove_file(path);
    result
}
fn string(value: &serde_json::Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(str::to_owned)
        .ok_or_else(|| format!("herdr omitted {key}."))
}
fn live_identity() -> Result<Identity, String> {
    let value = herdr_json(&["pane", "current", "--current"])?;
    let pane = value
        .pointer("/result/pane")
        .ok_or("herdr omitted current pane.")?;
    Ok(Identity {
        workspace_id: string(pane, "workspace_id")?,
        tab_id: string(pane, "tab_id")?,
        pane_id: string(pane, "pane_id")?,
        terminal_id: string(pane, "terminal_id")?,
    })
}
/// Query the shell itself, never the pane's launch directory or prompt text.
/// All callers run on the coordination worker, so filesystem validation is off UI.
fn shell_peer(pane_id: &str, caller: &Identity) -> Result<Peer, String> {
    let value = herdr_json(&["pane", "get", pane_id])?;
    let pane = value
        .pointer("/result/pane")
        .ok_or("herdr omitted shell pane.")?;
    let identity = Identity {
        workspace_id: string(pane, "workspace_id")?,
        tab_id: string(pane, "tab_id")?,
        pane_id: string(pane, "pane_id")?,
        terminal_id: string(pane, "terminal_id")?,
    };
    if identity.workspace_id != caller.workspace_id
        || identity.tab_id != caller.tab_id
        || identity.pane_id != pane_id
        || identity.pane_id == caller.pane_id
    {
        return Err("Shell pane changed scope during discovery.".into());
    }
    let value = herdr_json(&["pane", "process-info", "--pane", pane_id])?;
    let info = value
        .pointer("/result/process_info")
        .ok_or("herdr omitted process info.")?;
    let pid = info
        .get("shell_pid")
        .and_then(|v| v.as_u64())
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 0)
        .ok_or("Shell process is unavailable.")?;
    let processes = info
        .get("foreground_processes")
        .and_then(|v| v.as_array())
        .ok_or("Foreground shell process is unavailable.")?;
    if info.get("pane_id").and_then(|v| v.as_str()) != Some(pane_id)
        || info
            .get("foreground_process_group_id")
            .and_then(|v| v.as_u64())
            != Some(u64::from(pid))
        || processes.len() != 1
        || processes[0].get("pid").and_then(|v| v.as_u64()) != Some(u64::from(pid))
    {
        return Err("Destination pane is running a foreground command, not an idle shell.".into());
    }
    let process = &processes[0];
    let name = process.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(
        name,
        "zsh" | "bash" | "sh" | "fish" | "dash" | "ksh" | "tcsh" | "csh" | "nu"
    ) {
        return Err("Destination foreground process is not a supported local shell.".into());
    }
    let cwd = process
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|cwd| !cwd.is_empty() && cwd.len() <= 8192 && !cwd.contains('\0'))
        .ok_or("Shell working directory is unavailable.")?;
    let path = PathBuf::from(cwd);
    if !path.is_absolute() || !fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir()) {
        return Err("Shell working directory is not an existing absolute local directory.".into());
    }
    Ok(Peer {
        is_shell: true,
        instance_id: format!("shell:{}:{pid}", identity.terminal_id),
        pid,
        location: Location::Local(path),
        generation: 0,
        identity: Some(identity),
    })
}
#[derive(Clone, Copy)]
struct Rectangle {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
fn verify_visible_scope(identity: &Identity) -> Result<(), String> {
    let value = herdr_json(&["workspace", "list"])?;
    let workspaces = value
        .pointer("/result/workspaces")
        .and_then(|v| v.as_array())
        .ok_or("herdr omitted workspace visibility.")?;
    if workspaces.len() > MAX_INSTANCES {
        return Err("herdr workspace response exceeds the discovery limit.".into());
    }
    // The caller identity is authoritative. Focus only validates its visibility;
    // it never substitutes a different workspace or tab for the caller.
    let workspace = workspaces
        .iter()
        .find(|workspace| {
            workspace.get("workspace_id").and_then(|v| v.as_str())
                == Some(identity.workspace_id.as_str())
        })
        .ok_or("Current caller workspace is absent from live herdr state.")?;
    if workspace.get("focused").and_then(|v| v.as_bool()) != Some(true)
        || workspace.get("active_tab_id").and_then(|v| v.as_str()) != Some(identity.tab_id.as_str())
    {
        return Err("Directional targeting requires this browser's visible herdr workspace and tab. Focus it, or choose a destination explicitly.".into());
    }
    Ok(())
}
fn live_layout(identity: &Identity) -> Result<Vec<(String, Rectangle)>, String> {
    let value = herdr_json(&["pane", "layout", "--pane", &identity.pane_id])?;
    let layout = value
        .pointer("/result/layout")
        .ok_or("herdr omitted pane layout.")?;
    if string(layout, "workspace_id")? != identity.workspace_id
        || string(layout, "tab_id")? != identity.tab_id
    {
        return Err("herdr scope changed during discovery. Retry destination selection.".into());
    }
    if layout.get("zoomed").and_then(|v| v.as_bool()) == Some(true) {
        return Err("Unzoom the herdr pane before directional targeting.".into());
    }
    let panes = layout
        .get("panes")
        .and_then(|v| v.as_array())
        .ok_or("herdr omitted pane geometry.")?;
    if panes.len() > MAX_INSTANCES {
        return Err("herdr layout exceeds pane limit.".into());
    }
    panes
        .iter()
        .map(|pane| {
            let rectangle = pane.get("rect").ok_or("herdr omitted pane rectangle.")?;
            let number = |key| {
                rectangle
                    .get(key)
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 1_000_000.0)
                    .ok_or("Invalid herdr rectangle.")
            };
            let rectangle = Rectangle {
                x: number("x")?,
                y: number("y")?,
                width: number("width")?,
                height: number("height")?,
            };
            if rectangle.width <= 0.0 || rectangle.height <= 0.0 {
                return Err("Empty herdr pane geometry.".into());
            }
            Ok((string(pane, "pane_id")?, rectangle))
        })
        .collect()
}
fn directional_rank(source: Rectangle, target: Rectangle, direction: Direction) -> Option<f64> {
    let (gap, overlap) = match direction {
        Direction::Left if target.x + target.width <= source.x => (
            source.x - target.x - target.width,
            (source.y + source.height).min(target.y + target.height) - source.y.max(target.y),
        ),
        Direction::Right if target.x >= source.x + source.width => (
            target.x - source.x - source.width,
            (source.y + source.height).min(target.y + target.height) - source.y.max(target.y),
        ),
        Direction::Up if target.y + target.height <= source.y => (
            source.y - target.y - target.height,
            (source.x + source.width).min(target.x + target.width) - source.x.max(target.x),
        ),
        Direction::Down if target.y >= source.y + source.height => (
            target.y - source.y - source.height,
            (source.x + source.width).min(target.x + target.width) - source.x.max(target.x),
        ),
        _ => return None,
    };
    // Prefer orthogonal overlap, then nearest edge. Keep all eligible candidates
    // so more than one browser is reviewed in a picker rather than silently guessed.
    Some(if overlap > 0.0 {
        gap / (1.0 + overlap)
    } else {
        1_000_000.0 + gap + overlap.abs()
    })
}

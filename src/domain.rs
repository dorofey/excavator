use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum Location {
    Local(PathBuf),
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

impl Location {
    pub fn local_path(&self) -> &Path {
        match self {
            Self::Local(path) => path,
            _ => panic!("local_path called for remote location"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct EntryId(pub Location);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: EntryId,
    pub location: Location,
    pub name: OsString,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ProviderId {
    Registry,
    Local,
    Sftp,
    Ftps,
    S3,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Capabilities {
    pub list: bool,
    pub metadata: bool,
    pub rename: bool,
    pub server_side_copy: bool,
    pub trash: bool,
    pub permissions: bool,
    pub symlinks: bool,
    pub seek: bool,
    pub resumable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsErrorKind {
    NotFound,
    PermissionDenied,
    NotDirectory,
    Cancelled,
    Io,
    Conflict,
    InvalidOperation,
    Unsupported,
    Authentication,
    Tls,
    HostKeyUnknown,
    HostKeyChanged,
}

#[derive(Clone, Debug)]
pub struct FsError {
    pub kind: FsErrorKind,
    pub location: Location,
    pub message: String,
}

impl FsError {
    pub fn cancelled(location: Location) -> Self {
        Self {
            kind: FsErrorKind::Cancelled,
            location,
            message: "Directory request cancelled".into(),
        }
    }

    pub fn from_io(location: Location, error: std::io::Error) -> Self {
        let kind = match error.kind() {
            std::io::ErrorKind::NotFound => FsErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
            std::io::ErrorKind::NotADirectory => FsErrorKind::NotDirectory,
            _ => FsErrorKind::Io,
        };
        Self {
            kind,
            location,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location.display(), self.message)
    }
}

impl std::error::Error for FsError {}

impl Location {
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local(_))
    }
    pub fn connection_id(&self) -> Option<&str> {
        match self {
            Self::Local(_) => None,
            Self::Sftp { connection, .. }
            | Self::Ftps { connection, .. }
            | Self::S3 { connection, .. } => Some(connection),
        }
    }
    pub fn display(&self) -> String {
        match self {
            Self::Local(p) => p.display().to_string(),
            Self::Sftp { connection, path } => format!("sftp://{connection}{path}"),
            Self::Ftps { connection, path } => format!("ftps://{connection}{path}"),
            Self::S3 {
                connection,
                bucket,
                key,
                ..
            } => format!("s3://{connection}/{bucket}/{key}"),
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Local(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.display()),
            Self::Sftp { path, .. } | Self::Ftps { path, .. } => path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("/")
                .into(),
            Self::S3 { bucket, key, .. } => key
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(bucket)
                .into(),
        }
    }
    pub fn parent(&self) -> Option<Self> {
        match self {
            Self::Local(p) => p.parent().map(|p| Self::Local(p.into())),
            Self::Sftp { connection, path } | Self::Ftps { connection, path } => {
                if path == "/" {
                    return None;
                }
                let p = path
                    .trim_end_matches('/')
                    .rsplit_once('/')
                    .map(|(p, _)| if p.is_empty() { "/" } else { p })
                    .unwrap_or("/")
                    .to_string();
                Some(if matches!(self, Self::Sftp { .. }) {
                    Self::Sftp {
                        connection: connection.clone(),
                        path: p,
                    }
                } else {
                    Self::Ftps {
                        connection: connection.clone(),
                        path: p,
                    }
                })
            }
            Self::S3 {
                connection,
                bucket,
                key,
                ..
            } => {
                if key.is_empty() {
                    return None;
                }
                let key = key
                    .trim_end_matches('/')
                    .rsplit_once('/')
                    .map(|(p, _)| format!("{p}/"))
                    .unwrap_or_default();
                Some(Self::S3 {
                    connection: connection.clone(),
                    bucket: bucket.clone(),
                    key,
                    prefix: true,
                })
            }
        }
    }
    pub fn join(&self, name: &std::ffi::OsStr) -> Result<Self, FsError> {
        let invalid = || FsError {
            kind: FsErrorKind::InvalidOperation,
            location: self.clone(),
            message: "A single non-empty name without separators is required".into(),
        };
        if name.is_empty() || name == "." || name == ".." {
            return Err(invalid());
        }
        if let Self::Local(p) = self {
            use std::os::unix::ffi::OsStrExt;
            if name.as_bytes().contains(&b'/') || name.as_bytes().contains(&0) {
                return Err(invalid());
            }
            return Ok(Self::Local(p.join(name)));
        }
        let name = name.to_str().ok_or_else(invalid)?;
        if name.contains('/') || name.contains('\0') {
            return Err(invalid());
        }
        Ok(match self {
            Self::Sftp { connection, path } => Self::Sftp {
                connection: connection.clone(),
                path: format!("{}/{name}", path.trim_end_matches('/')),
            },
            Self::Ftps { connection, path } => Self::Ftps {
                connection: connection.clone(),
                path: format!("{}/{name}", path.trim_end_matches('/')),
            },
            Self::S3 {
                connection,
                bucket,
                key,
                ..
            } => Self::S3 {
                connection: connection.clone(),
                bucket: bucket.clone(),
                key: format!(
                    "{}{name}",
                    if key.is_empty() {
                        String::new()
                    } else {
                        if key.ends_with('/') {
                            key.clone()
                        } else {
                            format!("{key}/")
                        }
                    }
                ),
                prefix: false,
            },
            Self::Local(_) => unreachable!(),
        })
    }
    pub fn parse_path(&self, input: &str) -> Result<Self, FsError> {
        let err = || FsError {
            kind: FsErrorKind::InvalidOperation,
            location: self.clone(),
            message: "Enter an absolute provider path; endpoints belong in Connections".into(),
        };
        if input == self.display() {
            return Ok(self.clone());
        }
        if input.contains('\0') {
            return Err(err());
        }
        match self {
            Self::Local(_) => {
                let p = if input == "~" || input.starts_with("~/") {
                    let home = std::env::var_os("HOME").ok_or_else(err)?;
                    PathBuf::from(home).join(input.trim_start_matches('~').trim_start_matches('/'))
                } else {
                    PathBuf::from(input)
                };
                if !p.is_absolute() {
                    return Err(err());
                }
                Ok(Self::Local(p))
            }
            Self::Sftp { connection, .. } | Self::Ftps { connection, .. } => {
                if !input.starts_with('/') {
                    return Err(err());
                }
                Ok(if matches!(self, Self::Sftp { .. }) {
                    Self::Sftp {
                        connection: connection.clone(),
                        path: input.into(),
                    }
                } else {
                    Self::Ftps {
                        connection: connection.clone(),
                        path: input.into(),
                    }
                })
            }
            Self::S3 {
                connection, bucket, ..
            } => Ok(Self::S3 {
                connection: connection.clone(),
                bucket: bucket.clone(),
                key: input.to_string(),
                prefix: true,
            }),
        }
    }
}

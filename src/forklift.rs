//! Read-only, bounded ForkLift Core Data favorites import. Credentials are never imported.
use crate::connections::{ConnectionRecord, Protocol, new_id};
use plist::Value;
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub struct ImportPlan {
    pub candidates: Vec<ConnectionRecord>,
    pub selected: Vec<bool>,
    pub duplicates: usize,
    pub ignored: usize,
    pub skipped: Vec<String>,
}
pub fn default_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME") else {
        return vec![];
    };
    let home = PathBuf::from(home);
    vec![
        home.join("Library/Group Containers/J3CP9BBBN6.com.binarynights.ForkLift/Favorites.sqlite"),
    ]
}
pub fn read(path: &Path, existing: &[ConnectionRecord]) -> Result<ImportPlan, String> {
    let mut db=Connection::open_with_flags(path,OpenFlags::SQLITE_OPEN_READ_ONLY|OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|_|"Cannot open ForkLift favorites. Choose its Favorites.sqlite file, including its adjacent WAL file when present.".to_string())?;
    db.busy_timeout(Duration::from_secs(2))
        .map_err(|_| "Cannot initialize ForkLift reader")?;
    let transaction = db
        .transaction()
        .map_err(|_| "ForkLift favorites are busy; retry after closing ForkLift")?;
    let mut query = transaction
        .prepare("SELECT length(ZDATA), CASE WHEN length(ZDATA) <= 1048576 THEN ZDATA ELSE NULL END FROM ZFAVORITE WHERE ZDATA IS NOT NULL LIMIT 10001")
        .map_err(|_| "This file is not a supported ForkLift favorites database")?;
    let mut rows = query
        .query([])
        .map_err(|_| "Cannot read ForkLift favorites")?;
    let mut plan = ImportPlan {
        candidates: vec![],
        selected: vec![],
        duplicates: 0,
        ignored: 0,
        skipped: vec![],
    };
    let mut total = 0usize;
    let mut count = 0;
    while let Some(row) = rows.next().map_err(|_| "Cannot read ForkLift favorites")? {
        count += 1;
        if count > 10000 {
            return Err("ForkLift import exceeds 10000 favorites".into());
        }
        let length: i64 = row.get(0).map_err(|_| "Invalid ForkLift favorite data")?;
        let len = usize::try_from(length).map_err(|_| "Invalid ForkLift favorite data length")?;
        total = total
            .checked_add(len)
            .ok_or("ForkLift import is too large")?;
        if len > 1024 * 1024 || total > 32 * 1024 * 1024 {
            return Err("ForkLift import exceeds its data limit".into());
        }
        let blob: Vec<u8> = row.get(1).map_err(|_| "Invalid ForkLift favorite data")?;
        let fields = match decode(&blob) {
            Ok(fields) => fields,
            Err(_) => {
                plan.skipped.push("Malformed archived favorite".into());
                continue;
            }
        };
        let kind = fields.get("type").map(String::as_str).unwrap_or("");
        if kind != "sftp" {
            if matches!(kind, "group" | "tagGroup" | "tags" | "local") {
                plan.ignored += 1;
            } else {
                plan.skipped.push(
                    if kind == "s3" {
                        "S3 favorite requires explicit bucket and region settings; add it manually"
                    } else {
                        "Unsupported favorite protocol; plain FTP is not upgraded to FTPS"
                    }
                    .into(),
                );
            }
            continue;
        }
        match sftp(&fields) {
            Ok(record) => {
                if existing
                    .iter()
                    .chain(plan.candidates.iter())
                    .any(|old| crate::connections::metadata_duplicate(old, &record))
                {
                    plan.duplicates += 1;
                } else {
                    plan.candidates.push(record);
                    plan.selected.push(true);
                }
            }
            Err(reason) => plan.skipped.push(reason),
        }
    }
    Ok(plan)
}
fn decode(blob: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let value = Value::from_reader(std::io::Cursor::new(blob)).map_err(|_| "Invalid archive")?;
    let archive = value.as_dictionary().ok_or("Invalid archive")?;
    if archive.get("$archiver").and_then(Value::as_string) != Some("NSKeyedArchiver") {
        return Err("Unknown archive".into());
    }
    let objects = archive
        .get("$objects")
        .and_then(Value::as_array)
        .ok_or("Missing objects")?;
    if objects.len() > 4096 {
        return Err("Archive object limit".into());
    }
    let top = archive
        .get("$top")
        .and_then(Value::as_dictionary)
        .and_then(|d| d.get("root"))
        .ok_or("Missing root")?;
    let root = resolve(top, objects, 0, &mut BTreeSet::new())?
        .as_dictionary()
        .ok_or("Invalid root")?;
    let keys = root
        .get("NS.keys")
        .and_then(Value::as_array)
        .ok_or("Missing keys")?;
    let values = root
        .get("NS.objects")
        .and_then(Value::as_array)
        .ok_or("Missing values")?;
    if keys.len() != values.len() || keys.len() > 256 {
        return Err("Invalid dictionary".into());
    }
    let mut fields = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (key, value) in keys.iter().zip(values) {
        let key = resolve(key, objects, 0, &mut BTreeSet::new())?
            .as_string()
            .ok_or("Invalid key")?;
        if !seen.insert(key) {
            return Err("Duplicate key".into());
        }
        // Only read non-secret, documented-by-observation connection metadata.
        if !matches!(key, "type" | "name" | "username" | "host" | "path") {
            continue;
        }
        let value = resolve(value, objects, 0, &mut BTreeSet::new())?;
        if let Some(value) = value.as_string() {
            if value.len() > 16384 {
                return Err("Field limit".into());
            }
            fields.insert(key.into(), value.into());
        }
    }
    Ok(fields)
}
fn resolve<'a>(
    value: &'a Value,
    objects: &'a [Value],
    depth: usize,
    visited: &mut BTreeSet<u64>,
) -> Result<&'a Value, String> {
    if depth > 16 {
        return Err("Archive depth limit".into());
    }
    if let Value::Uid(uid) = value {
        let n = uid.get();
        if !visited.insert(n) {
            return Err("Archive cycle".into());
        }
        let value = objects.get(n as usize).ok_or("Invalid object reference")?;
        resolve(value, objects, depth + 1, visited)
    } else {
        Ok(value)
    }
}
fn sftp(fields: &BTreeMap<String, String>) -> Result<ConnectionRecord, String> {
    let host = fields.get("host").cloned().unwrap_or_default();
    let username = fields.get("username").cloned().unwrap_or_default();
    let root = fields
        .get("path")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "/".into());
    let mut record = ConnectionRecord {
        id: new_id(),
        name: fields
            .get("name")
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "Imported SFTP connection".into()),
        protocol: Protocol::Sftp,
        host,
        port: 22,
        username,
        root,
        bucket: String::new(),
        region: String::new(),
        endpoint: String::new(),
        ca_bundle: String::new(),
    };
    // A URL host is accepted only when it contains no credentials or extra path.
    if record.host.contains("://") {
        let url = url::Url::parse(&record.host).map_err(|_| "SFTP favorite has an invalid host")?;
        if url.scheme() != "sftp"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err("SFTP host URL contains unsupported connection fields".into());
        }
        record.port = url.port().unwrap_or(22);
        record.host = url
            .host_str()
            .ok_or("SFTP favorite is missing its host")?
            .into();
    }
    record.validate().map_err(|_| {
        "SFTP favorite requires a valid host, username and absolute path".to_string()
    })?;
    Ok(record)
}

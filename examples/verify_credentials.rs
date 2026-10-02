//! Disposable metadata and Keychain fixture; uses generated connection IDs only.
#[path = "../src/connections.rs"]
mod connections;
#[path = "../src/credentials.rs"]
mod credentials;
use connections::{ConnectionRecord, ConnectionSecrets, Protocol};
use std::{fs, path::PathBuf};

fn record() -> ConnectionRecord {
    ConnectionRecord {
        id: connections::new_id(),
        name: "Disposable fixture".into(),
        protocol: Protocol::Sftp,
        host: "localhost".into(),
        port: 22222,
        username: "fixture".into(),
        root: "/".into(),
        bucket: String::new(),
        region: String::new(),
        endpoint: String::new(),
        ca_bundle: String::new(),
        group: String::new(),
        ssh_key_path: String::new(),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("EXCAVATOR_CONNECTION_FIXTURE_CHILD").is_none() {
        let temp = std::env::temp_dir().join(connections::new_id());
        fs::create_dir(&temp)?;
        let status = std::process::Command::new(std::env::current_exe()?)
            .env("HOME", &temp)
            .env("EXCAVATOR_CONNECTION_FIXTURE_CHILD", "1")
            .status()?;
        fs::remove_dir_all(&temp)?;
        if !status.success() {
            return Err("Connection fixture failed".into());
        }
        return Ok(());
    }
    assert!(connections::load()?.is_empty());
    let mut record = record();
    connections::save(&record, &None)?;
    assert_eq!(connections::load()?[0].id, record.id);
    assert!(
        connections::secrets(&record.id)
            .unwrap_err()
            .contains("missing")
    );
    let fingerprint = format!("SHA256:{}", "A".repeat(43));
    connections::trust_host(&record, &fingerprint)?;
    assert_eq!(connections::known_host(&record)?, Some(fingerprint.clone()));
    assert!(connections::trust_host(&record, &format!("SHA256:{}", "B".repeat(43))).is_err());
    record.port += 1;
    assert!(connections::known_host(&record)?.is_none());
    record.port -= 1;
    connections::forget_host(&record)?;
    assert!(connections::known_host(&record)?.is_none());
    connections::remove(&record.id)?;
    assert!(connections::load()?.is_empty());
    connections::save(&record, &None)?;
    let secret = ConnectionSecrets {
        password: "disposable-fixture-password".into(),
        ..Default::default()
    };
    assert!(!format!("{secret:?}").contains(&secret.password));
    // Set this flag only when physical Keychain roundtrip is desired.
    if std::env::var_os("EXCAVATOR_VERIFY_KEYCHAIN").is_some() {
        connections::save(&record, &Some(secret.clone()))?;
        let result = connections::secrets(&record.id);
        let cleanup = credentials::remove(&record.id);
        assert_eq!(result?.password, secret.password);
        cleanup?;
        assert!(credentials::get(&record.id)?.is_none());
    }
    let path = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Library/Application Support/Excavator/connections.json");
    let valid = fs::read(&path)?;
    assert!(!String::from_utf8_lossy(&valid).contains(&secret.password));
    fs::write(&path, b"invalid-json")?;
    assert!(connections::save(&record, &None).is_err());
    assert_eq!(fs::read(&path)?, b"invalid-json");
    let mut future: serde_json::Value = serde_json::from_slice(&valid)?;
    future["version"] = 99.into();
    let future = serde_json::to_vec(&future)?;
    fs::write(&path, &future)?;
    assert!(connections::save(&record, &None).is_err());
    assert_eq!(fs::read(&path)?, future);
    let mut s3 = record.clone();
    s3.protocol = Protocol::S3;
    s3.host.clear();
    s3.username.clear();
    s3.bucket = "fixture-bucket".into();
    s3.region = "us-east-1".into();
    for endpoint in [
        "http://example.com",
        "https://name:secret@example.com",
        "https://example.com/?token=secret",
        "https://example.com/path",
    ] {
        s3.endpoint = endpoint.into();
        assert!(s3.validate().is_err());
    }
    for endpoint in [
        "https://example.com",
        "http://localhost:5000",
        "http://127.0.0.1:5000",
        "http://[::1]:5000",
    ] {
        s3.endpoint = endpoint.into();
        s3.validate()?;
    }
    println!(
        "Connection metadata, missing credentials, redaction, explicit host trust, endpoint validation and corrupt/future preservation passed"
    );
    Ok(())
}

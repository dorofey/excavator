//! Safe loopback provider checks. Requires scripts/remote-fixtures.py running.
//! Credentials are public fixture values injected only into this registry instance.
#![allow(dead_code)]
#[path = "../src/connections.rs"]
mod connections;
#[path = "../src/credentials.rs"]
mod credentials;
#[path = "../src/domain.rs"]
mod domain;
#[path = "../src/providers/mod.rs"]
mod providers;
#[path = "../src/transfers/mod.rs"]
mod transfers;
use connections::{ConnectionRecord, ConnectionSecrets, Protocol};
use domain::{FsErrorKind, Location};
use providers::{CancellationToken, ListOptions, ProviderRegistry};
use std::{
    ffi::OsStr,
    io::{Read, Write},
};
use transfers::{ConflictPolicy, JobState, Operation, OperationPlan, TransferManager};
fn record(id: &str, protocol: Protocol, port: u16) -> ConnectionRecord {
    ConnectionRecord {
        id: id.into(),
        name: format!("Disposable {id}"),
        protocol,
        host: if protocol == Protocol::S3 {
            String::new()
        } else {
            "localhost".into()
        },
        port,
        username: if protocol == Protocol::S3 {
            String::new()
        } else {
            "fixture".into()
        },
        root: if protocol == Protocol::S3 {
            String::new()
        } else {
            "/".into()
        },
        bucket: if protocol == Protocol::S3 {
            "excavator-fixture".into()
        } else {
            String::new()
        },
        region: if protocol == Protocol::S3 {
            "us-east-1".into()
        } else {
            String::new()
        },
        endpoint: if protocol == Protocol::S3 {
            format!("http://127.0.0.1:{port}")
        } else {
            String::new()
        },
        ca_bundle: if protocol == Protocol::Ftps {
            std::env::current_dir()
                .unwrap()
                .join(".remote-fixture-data/ca.pem")
                .display()
                .to_string()
        } else {
            String::new()
        },
        group: String::new(),
        ssh_key_path: String::new(),
    }
}
fn location(record: &ConnectionRecord) -> Location {
    match record.protocol {
        Protocol::Sftp => Location::Sftp {
            connection: record.id.clone(),
            path: "/".into(),
        },
        Protocol::Ftps => Location::Ftps {
            connection: record.id.clone(),
            path: "/".into(),
        },
        Protocol::S3 => Location::S3 {
            connection: record.id.clone(),
            bucket: record.bucket.clone(),
            key: String::new(),
            prefix: true,
        },
    }
}
fn read(registry: &ProviderRegistry, location: &Location) -> Vec<u8> {
    let mut input = registry
        .open_read(location, &CancellationToken::new())
        .unwrap();
    let mut result = Vec::new();
    input.read_to_end(&mut result).unwrap();
    result
}
fn write(registry: &ProviderRegistry, location: &Location, bytes: &[u8]) {
    let cancel = CancellationToken::new();
    let mut output = registry.create_write(location, &cancel).unwrap();
    output.write_all(bytes).unwrap();
    output.finish(&cancel).unwrap();
}
fn main() {
    let sftp = record("fixture-sftp", Protocol::Sftp, 22220);
    let ftps = record("fixture-ftps", Protocol::Ftps, 22221);
    let s3 = record("fixture-s3", Protocol::S3, 22222);
    let proxy = record("fixture-proxy", Protocol::S3, 22223);
    let secret = ConnectionSecrets {
        password: "fixture-password".into(),
        access_key: "fixture".into(),
        secret_key: "fixture-secret".into(),
        session_token: String::new(),
        ssh_key_passphrase: String::new(),
    };
    let registry = ProviderRegistry::with_connections(vec![
        (sftp.clone(), secret.clone()),
        (ftps.clone(), secret.clone()),
        (s3.clone(), secret.clone()),
        (proxy.clone(), secret.clone()),
    ]);
    let cancel = CancellationToken::new();
    let options = ListOptions { show_hidden: true };
    assert_eq!(
        registry
            .list_sync(&location(&sftp), options, &cancel)
            .unwrap_err()
            .kind,
        FsErrorKind::HostKeyUnknown
    );
    let fingerprint = providers::probe_host(&sftp).unwrap();
    assert!(fingerprint.starts_with("SHA256:"));
    assert_eq!(fingerprint.len(), 50);
    let changed = registry
        .clone()
        .with_trusted_host(&sftp.id, format!("SHA256:{}", "A".repeat(43)));
    assert_eq!(
        changed
            .list_sync(&location(&sftp), options, &cancel)
            .unwrap_err()
            .kind,
        FsErrorKind::HostKeyChanged
    );
    let registry = registry.with_trusted_host(&sftp.id, fingerprint);
    for record in [&sftp, &ftps, &s3] {
        let root = location(record);
        let entries = registry.list_sync(&root, options, &cancel).unwrap();
        assert!(entries.iter().any(|entry| entry.name == "hello.txt"));
        assert_eq!(
            read(&registry, &root.join(OsStr::new("hello.txt")).unwrap()),
            b"remote fixture bytes\n"
        );
        println!("PASS: {:?} list/stat/bounded download", record.protocol);
    }
    let pages = Location::S3 {
        connection: s3.id.clone(),
        bucket: s3.bucket.clone(),
        key: "pages/".into(),
        prefix: true,
    };
    assert_eq!(
        registry.list_sync(&pages, options, &cancel).unwrap().len(),
        1005
    );
    println!("PASS: S3 continuation pagination 1005 objects");
    let unique = format!(
        "fixture-check-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    for record in [&sftp, &s3] {
        let root = location(record);
        let target = root.join(OsStr::new(&format!("{unique}.bin"))).unwrap();
        let payload = vec![0x5a; 9 * 1024 * 1024 + 31];
        write(&registry, &target, &payload);
        assert_eq!(read(&registry, &target), payload);
        let mut conflict = registry.create_write(&target, &cancel).unwrap();
        if record.protocol == Protocol::S3 {
            conflict.write_all(&vec![0x66; 16 * 1024 * 1024]).unwrap();
        } else {
            conflict.write_all(b"cannot replace").unwrap();
        }
        assert!(conflict.finish(&cancel).is_err());
        conflict.abort().unwrap();
        assert_eq!(read(&registry, &target), payload);
        registry.delete(&target, &cancel).unwrap();
        println!(
            "PASS: {:?} exclusive upload/roundtrip/conflict/nonrecursive delete",
            record.protocol
        );
    }
    let ssh_root = location(&sftp);
    let created = ssh_root.join(OsStr::new(&format!("{unique}-dir"))).unwrap();
    registry.create_dir(&created, &cancel).unwrap();
    registry.delete(&created, &cancel).unwrap();
    let first = ssh_root
        .join(OsStr::new(&format!("{unique}-rename-first")))
        .unwrap();
    let second = ssh_root
        .join(OsStr::new(&format!("{unique}-rename-second")))
        .unwrap();
    write(&registry, &first, b"rename fixture");
    registry.rename(&first, &second, &cancel).unwrap();
    assert_eq!(read(&registry, &second), b"rename fixture");
    assert!(
        registry
            .rename(
                &second,
                &ssh_root.join(OsStr::new("hello.txt")).unwrap(),
                &cancel
            )
            .is_err()
    );
    registry.delete(&second, &cancel).unwrap();
    println!("PASS: SFTP create/delete empty directory and no-clobber rename");
    let ftp_root = location(&ftps);
    let created = ftp_root
        .join(OsStr::new(&format!("{unique}-ftp-dir")))
        .unwrap();
    registry.create_dir(&created, &cancel).unwrap();
    registry.delete(&created, &cancel).unwrap();
    let ftp_file = std::env::current_dir()
        .unwrap()
        .join(".remote-fixture-data/ftps")
        .join(format!("{unique}-ftp-delete.txt"));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&ftp_file)
        .unwrap();
    std::fs::write(&ftp_file, b"new disposable FTPS delete fixture").unwrap();
    registry
        .delete(
            &ftp_root.join(ftp_file.file_name().unwrap()).unwrap(),
            &cancel,
        )
        .unwrap();
    assert!(!ftp_file.exists());
    assert!(
        registry
            .create_write(
                &ftp_root.join(OsStr::new("unsupported-upload")).unwrap(),
                &cancel
            )
            .is_err()
    );
    println!(
        "PASS: FTPS create/delete empty dir, explicitly delete own fixture file, unsafe upload refusal"
    );
    let cancel_target = location(&s3)
        .join(OsStr::new(&format!("{unique}-cancel-multipart")))
        .unwrap();
    let upload_cancel = CancellationToken::new();
    let mut cancelled_writer = registry
        .create_write(&cancel_target, &upload_cancel)
        .unwrap();
    cancelled_writer
        .write_all(&vec![0x77; 9 * 1024 * 1024])
        .unwrap();
    upload_cancel.cancel();
    assert_eq!(
        cancelled_writer.finish(&upload_cancel).unwrap_err().kind,
        FsErrorKind::Cancelled
    );
    cancelled_writer.abort().unwrap();
    assert_eq!(
        registry
            .metadata_sync(&cancel_target, &cancel)
            .unwrap_err()
            .kind,
        FsErrorKind::NotFound
    );
    println!("PASS: active S3 multipart cancellation aborts without object install");
    let mut bad_ftps = ftps.clone();
    bad_ftps.id = "fixture-bad-tls".into();
    bad_ftps.ca_bundle.clear();
    let bad_registry = ProviderRegistry::with_connections(vec![(bad_ftps.clone(), secret.clone())]);
    assert_eq!(
        bad_registry
            .list_sync(&location(&bad_ftps), options, &cancel)
            .unwrap_err()
            .kind,
        FsErrorKind::Tls
    );
    println!("PASS: FTPS certificate rejection");
    let mut wrong_sftp = secret.clone();
    wrong_sftp.password = "incorrect-fixture-password".into();
    let bad_registry = ProviderRegistry::with_connections(vec![(sftp.clone(), wrong_sftp)])
        .with_trusted_host(&sftp.id, providers::probe_host(&sftp).unwrap());
    assert_eq!(
        bad_registry
            .list_sync(&location(&sftp), options, &cancel)
            .unwrap_err()
            .kind,
        FsErrorKind::Authentication
    );
    println!("PASS: SFTP invalid credential rejection");
    for key in ["/hello.txt", "a/../hello.txt", "./hello.txt"] {
        let bad = Location::S3 {
            connection: s3.id.clone(),
            bucket: s3.bucket.clone(),
            key: key.into(),
            prefix: false,
        };
        assert_eq!(
            registry.metadata_sync(&bad, &cancel).unwrap_err().kind,
            FsErrorKind::Unsupported
        );
        assert_eq!(
            registry.delete(&bad, &cancel).unwrap_err().kind,
            FsErrorKind::Unsupported
        );
    }
    assert_eq!(
        read(
            &registry,
            &location(&s3).join(OsStr::new("hello.txt")).unwrap()
        ),
        b"remote fixture bytes\n"
    );
    println!("PASS: unsupported exact keys cannot target normalized neighbors");
    let retry = Location::S3 {
        connection: proxy.id.clone(),
        bucket: proxy.bucket.clone(),
        key: "retry-prefix/".into(),
        prefix: true,
    };
    assert!(
        !registry
            .list_sync(&retry, options, &cancel)
            .unwrap()
            .is_empty()
    );
    println!("PASS: S3 safe listing recovers from injected transient failures");
    let partial = Location::S3 {
        connection: proxy.id.clone(),
        bucket: proxy.bucket.clone(),
        key: "partial.txt".into(),
        prefix: false,
    };
    let mut reader = registry.open_read(&partial, &cancel).unwrap();
    let mut bytes = Vec::new();
    assert!(reader.read_to_end(&mut bytes).is_err());
    println!("PASS: partial S3 response fails visibly");
    let failed = Location::S3 {
        connection: proxy.id.clone(),
        bucket: proxy.bucket.clone(),
        key: "fail-upload.bin".into(),
        prefix: false,
    };
    let mut writer = registry.create_write(&failed, &cancel).unwrap();
    assert!(writer.write_all(&vec![1; 9 * 1024 * 1024]).is_err());
    writer.abort().unwrap();
    assert_eq!(
        registry.metadata_sync(&failed, &cancel).unwrap_err().kind,
        FsErrorKind::NotFound
    );
    println!("PASS: failed multipart upload aborts without installing object");
    let local = std::env::temp_dir().join(&unique);
    std::fs::create_dir(&local).unwrap();
    let manager = TransferManager::with_registry(registry.clone());
    let source = location(&sftp).join(OsStr::new("hello.txt")).unwrap();
    let id = manager.enqueue(OperationPlan {
        operation: Operation::Copy,
        sources: vec![source.clone()],
        destination: Some(Location::Local(local.clone())),
        new_name: None,
        conflict_policy: ConflictPolicy::Ask,
    });
    manager.run_pending();
    let snapshot = manager
        .snapshots()
        .into_iter()
        .find(|job| job.id == id)
        .unwrap();
    assert_eq!(snapshot.state, JobState::Completed, "{:?}", snapshot.error);
    assert_eq!(
        std::fs::read(local.join("hello.txt")).unwrap(),
        b"remote fixture bytes\n"
    );
    assert!(!snapshot.journal.is_empty());
    let id = manager.enqueue(OperationPlan {
        operation: Operation::Copy,
        sources: vec![partial.clone()],
        destination: Some(Location::Local(local.clone())),
        new_name: None,
        conflict_policy: ConflictPolicy::Ask,
    });
    manager.run_pending();
    let failed = manager
        .snapshots()
        .into_iter()
        .find(|j| j.id == id)
        .unwrap();
    assert_eq!(failed.state, JobState::Failed);
    assert!(failed.error.is_some());
    assert!(!local.join("partial.txt").exists());
    println!("PASS: partial response fails queue without installing local destination");
    let large = local.join("queued-cancel-large.bin");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    let id = manager.enqueue(OperationPlan {
        operation: Operation::Copy,
        sources: vec![Location::Local(large.clone())],
        destination: Some(location(&s3)),
        new_name: None,
        conflict_policy: ConflictPolicy::Ask,
    });
    let worker = manager.clone();
    let handle = std::thread::spawn(move || worker.run_pending());
    loop {
        let snapshot = manager
            .snapshots()
            .into_iter()
            .find(|j| j.id == id)
            .unwrap();
        if snapshot.bytes_copied > 0 {
            manager.cancel(id);
            break;
        }
        assert!(
            matches!(snapshot.state, JobState::Queued | JobState::Running),
            "{:?}",
            snapshot.error
        );
        std::thread::yield_now();
    }
    handle.join().unwrap();
    assert_eq!(
        manager
            .snapshots()
            .into_iter()
            .find(|j| j.id == id)
            .unwrap()
            .state,
        JobState::Cancelled
    );
    assert!(large.exists());
    println!("PASS: active provider-neutral queue cancellation retains source");
    // A recursive remote copy reports its partial destination and refuses to
    // traverse a symlink, even when an ordinary sibling was already committed.
    let tree_name = format!("{unique}-symlink-tree");
    let tree = std::env::current_dir()
        .unwrap()
        .join(".remote-fixture-data/sftp")
        .join(&tree_name);
    std::fs::create_dir(&tree).unwrap();
    std::fs::write(tree.join("a-ordinary"), b"safe ordinary sibling").unwrap();
    std::os::unix::fs::symlink("a-ordinary", tree.join("z-link")).unwrap();
    let tree_source = location(&sftp).join(OsStr::new(&tree_name)).unwrap();
    assert_eq!(
        registry
            .metadata_sync(&tree_source.join(OsStr::new("z-link")).unwrap(), &cancel)
            .unwrap()
            .kind,
        domain::EntryKind::Symlink
    );
    assert!(
        registry
            .open_read(&tree_source.join(OsStr::new("z-link")).unwrap(), &cancel)
            .is_err()
    );
    let id = manager.enqueue(OperationPlan {
        operation: Operation::Copy,
        sources: vec![tree_source.clone()],
        destination: Some(Location::Local(local.clone())),
        new_name: None,
        conflict_policy: ConflictPolicy::Ask,
    });
    manager.run_pending();
    let partial_tree = manager
        .snapshots()
        .into_iter()
        .find(|j| j.id == id)
        .unwrap();
    assert_eq!(partial_tree.state, JobState::Failed);
    assert_eq!(partial_tree.error.unwrap().kind, FsErrorKind::Unsupported);
    assert!(
        partial_tree
            .journal
            .iter()
            .any(|j| j.description.contains("partial"))
    );
    assert_eq!(
        std::fs::read(local.join(&tree_name).join("a-ordinary")).unwrap(),
        b"safe ordinary sibling"
    );
    assert!(!local.join(&tree_name).join("z-link").exists());
    assert!(
        tree.join("z-link")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let mut duplicate = sftp.clone();
    duplicate.id = "fixture-sftp-alias".into();
    let duplicated = ProviderRegistry::with_connections(vec![
        (sftp.clone(), secret.clone()),
        (duplicate.clone(), secret.clone()),
    ])
    .with_trusted_host(&sftp.id, providers::probe_host(&sftp).unwrap())
    .with_trusted_host(&duplicate.id, providers::probe_host(&duplicate).unwrap());
    assert_eq!(
        duplicated
            .validate_copy(&tree_source, &location(&duplicate), &cancel)
            .unwrap_err()
            .kind,
        FsErrorKind::Unsupported
    );
    let mut duplicate = s3.clone();
    duplicate.id = "fixture-s3-alias".into();
    let duplicated = ProviderRegistry::with_connections(vec![
        (s3.clone(), secret.clone()),
        (duplicate.clone(), secret.clone()),
    ]);
    let prefix = Location::S3 {
        connection: s3.id.clone(),
        bucket: s3.bucket.clone(),
        key: "prefix/".into(),
        prefix: true,
    };
    assert_eq!(
        duplicated
            .validate_copy(&prefix, &location(&duplicate), &cancel)
            .unwrap_err()
            .kind,
        FsErrorKind::Unsupported
    );
    std::fs::remove_dir_all(tree).unwrap();
    println!(
        "PASS: recursive symlink refusal reports partial destination; connection aliases cannot recurse into source"
    );
    std::fs::remove_dir_all(local).unwrap();
    println!("PASS: provider-neutral transfer queue download + progress/journal");
    println!(
        "PASS: remote fixtures; secrets injected in memory; Keychain and persisted trust UI remain separate OS acceptance checks"
    );
}

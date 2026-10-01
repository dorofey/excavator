//! Synthetic archive checks; optional actual database count-only preview/apply.
#![allow(dead_code)]
#[path = "../src/connections.rs"]
mod connections;
#[path = "../src/credentials.rs"]
mod credentials;
#[path = "../src/forklift.rs"]
mod forklift;
use plist::{Dictionary, Uid, Value};
use std::path::PathBuf;
fn archive(fields: &[(&str, &str)]) -> Vec<u8> {
    let mut objects = vec![
        Value::String("$null".into()),
        Value::Dictionary(Dictionary::new()),
    ];
    let mut keys = vec![];
    let mut values = vec![];
    for (key, value) in fields {
        keys.push(Value::Uid(Uid::new(objects.len() as u64)));
        objects.push(Value::String((*key).into()));
        values.push(Value::Uid(Uid::new(objects.len() as u64)));
        objects.push(Value::String((*value).into()));
    }
    let mut root = Dictionary::new();
    root.insert("NS.keys".into(), Value::Array(keys));
    root.insert("NS.objects".into(), Value::Array(values));
    objects[1] = Value::Dictionary(root);
    let mut top = Dictionary::new();
    top.insert("root".into(), Value::Uid(Uid::new(1)));
    let mut archive = Dictionary::new();
    archive.insert("$archiver".into(), Value::String("NSKeyedArchiver".into()));
    archive.insert("$top".into(), Value::Dictionary(top));
    archive.insert("$objects".into(), Value::Array(objects));
    let mut output = vec![];
    Value::Dictionary(archive)
        .to_writer_binary(&mut output)
        .unwrap();
    output
}
fn main() {
    let dir =
        std::env::temp_dir().join(format!("excavator-forklift-verify-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("Favorites.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("CREATE TABLE ZFAVORITE(ZDATA BLOB)", [])
        .unwrap();
    for fields in [
        vec![
            ("type", "sftp"),
            ("name", "Fixture"),
            ("host", "example.invalid"),
            ("username", "fixture"),
            ("path", "/it's safe"),
            ("password", "DO_NOT_IMPORT"),
        ],
        vec![
            ("type", "sftp"),
            ("name", "Duplicate"),
            ("host", "EXAMPLE.invalid"),
            ("username", "fixture"),
            ("path", "/it's safe"),
        ],
        vec![("type", "local")],
        vec![("type", "s3")],
        vec![("type", "ftp")],
        vec![
            ("type", "sftp"),
            ("host", "sftp://example.invalid:2222"),
            ("username", "fixture"),
        ],
        vec![
            ("type", "sftp"),
            ("host", "bad host"),
            ("username", "fixture"),
        ],
        vec![("type", "sftp"), ("type", "local")],
    ] {
        db.execute("INSERT INTO ZFAVORITE VALUES (?1)", [archive(&fields)])
            .unwrap();
    }
    db.execute("INSERT INTO ZFAVORITE VALUES (?1)", [vec![1u8, 2, 3]])
        .unwrap();
    drop(db);
    let plan = forklift::read(&path, &[]).unwrap();
    assert_eq!(plan.candidates.len(), 2);
    assert_eq!(plan.duplicates, 1);
    assert_eq!(plan.ignored, 1);
    assert_eq!(plan.skipped.len(), 5);
    assert_eq!(plan.candidates[1].port, 2222);
    assert_eq!(plan.candidates[1].root, "/");
    assert_eq!(plan.candidates[0].root, "/it's safe");
    let encoded = serde_json::to_string(&plan.candidates).unwrap();
    assert!(!encoded.contains("DO_NOT_IMPORT"));
    assert!(!encoded.contains("password"));
    let second = forklift::read(&path, &plan.candidates).unwrap();
    assert_eq!(second.candidates.len(), 0);
    assert_eq!(second.duplicates, 3);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(dir).unwrap();
    println!(
        "Synthetic ForkLift archive checks passed: defaults, quoting, protocol skips, duplicate filtering, malformed archives and secret exclusion."
    );
    let mut args = std::env::args_os().skip(1);
    if let Some(path) = args.next() {
        let path = PathBuf::from(path);
        let existing = connections::load().unwrap();
        let plan = forklift::read(&path, &existing).unwrap();
        println!(
            "ForkLift preview: {} supported, {} duplicates, {} non-connections, {} skipped",
            plan.candidates.len(),
            plan.duplicates,
            plan.ignored,
            plan.skipped.len()
        );
        if args.next().is_some_and(|arg| arg == "--apply") {
            let outcome = connections::import_metadata(&plan.candidates).unwrap();
            let readback = connections::load().unwrap();
            assert_eq!(readback.len(), outcome.records.len());
            assert!(plan.candidates.iter().all(|record| {
                readback
                    .iter()
                    .any(|old| connections::metadata_duplicate(old, record))
            }));
            println!(
                "Applied {} metadata records; {} duplicates skipped; {} records verified by readback",
                outcome.added,
                outcome.duplicates,
                readback.len()
            );
        }
    }
}

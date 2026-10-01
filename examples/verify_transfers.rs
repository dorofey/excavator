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
use domain::Location;
use std::{ffi::OsString, fs, os::unix::fs::symlink, path::Path};
use transfers::*;
fn plan(op: Operation, source: &Path, destination: &Path, policy: ConflictPolicy) -> OperationPlan {
    OperationPlan {
        operation: op,
        sources: vec![Location::Local(source.into())],
        destination: Some(Location::Local(destination.into())),
        new_name: None,
        conflict_policy: policy,
    }
}
fn state(manager: &TransferManager, id: JobId) -> JobState {
    manager
        .snapshots()
        .into_iter()
        .find(|s| s.id == id)
        .unwrap()
        .state
}
fn main() {
    let root =
        std::env::temp_dir().join(format!("excavator-transfer-fixture-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let file = source.join("a.txt");
    fs::write(&file, b"hello fixture").unwrap();
    let manager = TransferManager::default();
    let id = manager.enqueue(plan(
        Operation::Copy,
        &file,
        &destination,
        ConflictPolicy::Ask,
    ));
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Completed);
    assert_eq!(
        fs::read(destination.join("a.txt")).unwrap(),
        b"hello fixture"
    );
    let id = manager.enqueue(plan(
        Operation::Copy,
        &file,
        &destination,
        ConflictPolicy::Ask,
    ));
    manager.run_pending();
    assert!(matches!(
        state(&manager, id),
        JobState::AwaitingConflict { .. }
    ));
    manager.resolve_conflict(id, ConflictPolicy::KeepBoth);
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Completed);
    assert!(destination.join("a.txt (copy 1)").exists());
    fs::write(&file, b"changed").unwrap();
    let id = manager.enqueue(plan(
        Operation::Copy,
        &file,
        &destination,
        ConflictPolicy::Replace,
    ));
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Completed);
    assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"changed");
    assert!(
        manager
            .snapshots()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap()
            .journal
            .iter()
            .any(|j| j.description.contains("backup"))
    );
    let replaced = manager
        .snapshots()
        .into_iter()
        .find(|s| s.id == id)
        .unwrap();
    let backup = replaced
        .journal
        .iter()
        .find(|j| j.description.contains("backup"))
        .unwrap()
        .destination
        .as_ref()
        .unwrap();
    assert_eq!(fs::read(backup.local_path()).unwrap(), b"hello fixture");
    let tree = source.join("tree");
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("child"), b"tree bytes").unwrap();
    symlink("child", tree.join("link")).unwrap();
    symlink("missing", tree.join("dangling")).unwrap();
    let id = manager.enqueue(plan(
        Operation::Move,
        &tree,
        &destination,
        ConflictPolicy::Ask,
    ));
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Completed);
    assert!(!tree.exists());
    assert_eq!(
        fs::read(destination.join("tree/child")).unwrap(),
        b"tree bytes"
    );
    assert_eq!(
        fs::read_link(destination.join("tree/dangling")).unwrap(),
        Path::new("missing")
    );
    let id = manager.enqueue(plan(
        Operation::Copy,
        &file,
        &destination,
        ConflictPolicy::Skip,
    ));
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Completed);
    let id = manager.enqueue(plan(
        Operation::Move,
        &file,
        &destination,
        ConflictPolicy::Skip,
    ));
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Failed);
    assert!(file.exists());
    let id = manager.enqueue(plan(Operation::Copy, &source, &source, ConflictPolicy::Ask));
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Failed);
    let id = manager.enqueue(plan(
        Operation::Copy,
        &file,
        &destination,
        ConflictPolicy::Ask,
    ));
    manager.cancel(id);
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Cancelled);
    let id = manager.enqueue(OperationPlan {
        operation: Operation::CreateDirectory,
        sources: vec![],
        destination: Some(Location::Local(destination.clone())),
        new_name: Some(OsString::from("created")),
        conflict_policy: ConflictPolicy::Ask,
    });
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Completed);
    let id = manager.enqueue(OperationPlan {
        operation: Operation::Rename,
        sources: vec![Location::Local(file.clone())],
        destination: None,
        new_name: Some(OsString::from("renamed")),
        conflict_policy: ConflictPolicy::Ask,
    });
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Completed);
    assert!(!file.exists());
    assert!(source.join("renamed").exists());
    // Source and destination aliases must be rejected before replacement.
    let renamed = source.join("renamed");
    let id = manager.enqueue(plan(
        Operation::Copy,
        &renamed,
        &source,
        ConflictPolicy::Replace,
    ));
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Failed);
    assert_eq!(fs::read(&renamed).unwrap(), b"changed");
    let id = manager.enqueue(OperationPlan {
        operation: Operation::CreateDirectory,
        sources: vec![],
        destination: Some(Location::Local(destination.clone())),
        new_name: Some(OsString::from("../escape")),
        conflict_policy: ConflictPolicy::Ask,
    });
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Failed);
    assert!(!root.join("escape").exists());
    // Simultaneous worker requests must not strand queued jobs or run duplicates.
    let mut handles = Vec::new();
    let mut queued = Vec::new();
    for n in 0..32 {
        let p = source.join(format!("concurrent-{n}"));
        fs::write(&p, b"bounded").unwrap();
        let id = manager.enqueue(plan(Operation::Copy, &p, &destination, ConflictPolicy::Ask));
        queued.push(id);
        let m = manager.clone();
        handles.push(std::thread::spawn(move || m.run_pending()));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    manager.run_pending();
    for id in queued {
        assert_eq!(state(&manager, id), JobState::Completed);
        assert_eq!(
            manager
                .snapshots()
                .into_iter()
                .find(|j| j.id == id)
                .unwrap()
                .completed_items,
            1
        );
    }
    for job in manager.snapshots() {
        if job.state == JobState::Completed {
            assert!(!job.journal.is_empty());
        }
    }
    // Cooperative cancellation after at least one copied chunk leaves the source
    // intact and removes the owned staging file before destination installation.
    let large = source.join("cancel-large");
    fs::File::create(&large)
        .unwrap()
        .set_len(512 * 1024 * 1024)
        .unwrap();
    let id = manager.enqueue(plan(
        Operation::Move,
        &large,
        &destination,
        ConflictPolicy::Ask,
    ));
    let worker = manager.clone();
    let handle = std::thread::spawn(move || worker.run_pending());
    loop {
        let job = manager
            .snapshots()
            .into_iter()
            .find(|j| j.id == id)
            .unwrap();
        if job.bytes_copied > 0 {
            manager.cancel(id);
            break;
        }
        assert!(matches!(job.state, JobState::Queued | JobState::Running));
        std::thread::yield_now();
    }
    handle.join().unwrap();
    assert_eq!(state(&manager, id), JobState::Cancelled);
    assert!(large.exists());
    assert!(!destination.join("cancel-large").exists());
    // Unsupported children produce a visible partial-copy journal while a move
    // retains the complete source tree.
    let partial = source.join("partial");
    fs::create_dir(&partial).unwrap();
    fs::write(partial.join("ordinary"), b"partial fixture").unwrap();
    let socket = std::os::unix::net::UnixListener::bind(partial.join("socket")).unwrap();
    let id = manager.enqueue(plan(
        Operation::Move,
        &partial,
        &destination,
        ConflictPolicy::Ask,
    ));
    manager.run_pending();
    assert_eq!(state(&manager, id), JobState::Failed);
    let failed = manager
        .snapshots()
        .into_iter()
        .find(|j| j.id == id)
        .unwrap();
    assert!(!failed.journal.is_empty());
    assert!(failed.error.is_some());
    assert!(partial.join("ordinary").exists());
    assert!(partial.join("socket").exists());
    drop(socket);
    // Opt into a real second filesystem. Never reuse or overwrite its existing
    // contents; on any failure, retain both fixture paths for inspection.
    if let Some(other_volume) = std::env::var_os("EXCAVATOR_OTHER_VOLUME") {
        use std::os::unix::fs::MetadataExt;
        let mount = std::path::PathBuf::from(other_volume);
        assert!(
            mount.is_dir(),
            "EXCAVATOR_OTHER_VOLUME must be a mounted directory"
        );
        assert_ne!(
            fs::metadata(&root).unwrap().dev(),
            fs::metadata(&mount).unwrap().dev(),
            "Cross-volume fixture requires distinct device IDs"
        );
        let unique = format!(
            "excavator-cross-volume-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let other = mount.join(unique);
        fs::create_dir(&other).expect("Create new cross-volume fixture child");
        println!(
            "Cross-volume fixture paths (retained on failure): {} and {}",
            root.display(),
            other.display()
        );
        let cross = source.join("cross-volume-tree");
        fs::create_dir(&cross).unwrap();
        fs::create_dir(cross.join("nested")).unwrap();
        fs::write(cross.join("nested/data"), b"real cross-volume payload").unwrap();
        symlink("nested/data", cross.join("link")).unwrap();
        symlink("missing", cross.join("dangling")).unwrap();
        let id = manager.enqueue(plan(Operation::Copy, &cross, &other, ConflictPolicy::Ask));
        manager.run_pending();
        let copied = manager
            .snapshots()
            .into_iter()
            .find(|j| j.id == id)
            .unwrap();
        assert_eq!(
            copied.state,
            JobState::Completed,
            "Cross-volume copy failed: {:?}",
            copied.error
        );
        assert!(cross.exists());
        let copied_target = other.join("cross-volume-tree");
        assert_eq!(
            fs::read(copied_target.join("nested/data")).unwrap(),
            b"real cross-volume payload"
        );
        assert_eq!(
            fs::read_link(copied_target.join("link")).unwrap(),
            Path::new("nested/data")
        );
        assert_eq!(
            fs::read_link(copied_target.join("dangling")).unwrap(),
            Path::new("missing")
        );
        assert_ne!(
            fs::metadata(&cross).unwrap().dev(),
            fs::metadata(&copied_target).unwrap().dev()
        );
        let move_parent = other.join("move-destination");
        fs::create_dir(&move_parent).unwrap();
        let id = manager.enqueue(plan(
            Operation::Move,
            &cross,
            &move_parent,
            ConflictPolicy::Ask,
        ));
        manager.run_pending();
        let moved = manager
            .snapshots()
            .into_iter()
            .find(|j| j.id == id)
            .unwrap();
        assert_eq!(
            moved.state,
            JobState::Completed,
            "Cross-volume move failed: {:?}",
            moved.error
        );
        assert!(!cross.exists());
        let moved_target = move_parent.join("cross-volume-tree");
        assert_eq!(
            fs::read(moved_target.join("nested/data")).unwrap(),
            b"real cross-volume payload"
        );
        assert_eq!(
            fs::read_link(moved_target.join("dangling")).unwrap(),
            Path::new("missing")
        );
        assert!(moved.journal.iter().any(|entry| {
            entry
                .description
                .contains("Verified copy and removed source")
        }));
        fs::remove_dir_all(&other).expect("Cleanup only owned cross-volume fixture child");
        println!("PASS: real distinct-device recursive copy and verified move");
    }
    // Explicit opt-in only: exercise the OS Trash using our own new fixture file.
    if std::env::var_os("EXCAVATOR_CHECK_TRASH").is_some_and(|value| value == "1") {
        let name = format!(
            "excavator-trash-fixture-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let trash_fixture = source.join(&name);
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&trash_fixture)
            .unwrap();
        fs::write(
            &trash_fixture,
            b"Disposable Excavator OS Trash verification fixture",
        )
        .unwrap();
        println!(
            "OS Trash fixture recovery name: {name}; original path: {}",
            trash_fixture.display()
        );
        let id = manager.enqueue(OperationPlan {
            operation: Operation::Trash,
            sources: vec![Location::Local(trash_fixture.clone())],
            destination: None,
            new_name: None,
            conflict_policy: ConflictPolicy::Ask,
        });
        manager.run_pending();
        let trashed = manager
            .snapshots()
            .into_iter()
            .find(|job| job.id == id)
            .unwrap();
        assert_eq!(
            trashed.state,
            JobState::Completed,
            "OS Trash failed: {:?}",
            trashed.error
        );
        assert!(!trash_fixture.exists());
        let canonical_original = fs::canonicalize(trash_fixture.parent().unwrap())
            .unwrap()
            .join(trash_fixture.file_name().unwrap());
        assert!(
            trashed
                .journal
                .iter()
                .any(|entry| entry.description == "Moved to system Trash"
                    && entry.source.as_ref() == Some(&Location::Local(canonical_original.clone())))
        );
        println!("PASS: OS Trash moved only the disposable fixture; recover by its printed name");
    }
    fs::remove_dir_all(root).unwrap();
    println!(
        "PASS: copy, conflicts, keep-both, backed-up replace, verified tree move with dangling link, skip, unsafe move refusal, descendant refusal, precancel, create, rename"
    );
}

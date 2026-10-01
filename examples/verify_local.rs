#[path = "../src/connections.rs"]
mod connections;
#[path = "../src/credentials.rs"]
mod credentials;
// Safe fixture verification. All writes are confined to a new temporary folder.
#[path = "../src/domain.rs"]
mod domain;
#[path = "../src/persistence.rs"]
mod persistence;
#[path = "../src/providers/mod.rs"]
mod providers;

use domain::{EntryKind, FsErrorKind, Location};
use providers::local::LocalFileSystem;
use providers::{CancellationToken, FileSystem, ListOptions};
use std::{
    fs,
    future::Future,
    path::PathBuf,
    task::{Context, Poll, Waker},
};

fn run<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut Context::from_waker(waker)) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("excavator-check-{}", std::process::id()));
    fs::create_dir(&root)?;
    // This fixture executable starts no threads and isolates settings from real HOME.
    unsafe { std::env::set_var("HOME", &root) };
    let folder = root.join("listing");
    fs::create_dir(&folder)?;
    fs::create_dir(folder.join("empty"))?;
    fs::write(folder.join("雪.txt"), b"unicode")?;
    fs::write(folder.join(".hidden"), b"hidden")?;
    let large = fs::File::create(folder.join("large.bin"))?;
    large.set_len(5 * 1024 * 1024 * 1024)?;
    std::os::unix::fs::symlink("missing", folder.join("dangling"))?;
    let provider = LocalFileSystem;
    let list = run(provider.list(
        Location::Local(folder.clone()),
        ListOptions { show_hidden: false },
        CancellationToken::default(),
    ))?;
    assert_eq!(list.len(), 4);
    assert_eq!(list[0].kind, EntryKind::Directory);
    assert!(list.iter().any(|entry| entry.kind == EntryKind::Symlink));
    assert_eq!(
        list.iter()
            .find(|entry| entry.name == "large.bin")
            .unwrap()
            .size,
        Some(5 * 1024 * 1024 * 1024)
    );
    assert!(list.iter().all(|entry| entry.modified.is_some()));
    let link = run(provider.metadata(
        Location::Local(folder.join("dangling")),
        CancellationToken::default(),
    ))?;
    assert_eq!(link.kind, EntryKind::Symlink);
    assert!(provider.capabilities().list);
    assert_eq!(provider.provider_id(), domain::ProviderId::Local);
    let denied = folder.join("permission-denied");
    fs::create_dir(&denied)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o000))?;
    let denied_result = run(provider.list(
        Location::Local(denied.clone()),
        ListOptions::default(),
        CancellationToken::default(),
    ));
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o700))?;
    fs::remove_dir(&denied)?;
    assert_eq!(
        denied_result.unwrap_err().kind,
        FsErrorKind::PermissionDenied
    );
    let all = run(provider.list(
        Location::Local(folder.clone()),
        ListOptions { show_hidden: true },
        CancellationToken::default(),
    ))?;
    assert_eq!(all.len(), 5);
    let cancel = CancellationToken::default();
    cancel.cancel();
    assert_eq!(
        run(provider.list(Location::Local(folder), ListOptions::default(), cancel))
            .unwrap_err()
            .kind,
        FsErrorKind::Cancelled
    );
    assert_eq!(
        run(provider.list(
            Location::Local(root.join("missing")),
            ListOptions::default(),
            CancellationToken::default()
        ))
        .unwrap_err()
        .kind,
        FsErrorKind::NotFound
    );

    let (mut preferences, warning) = persistence::load();
    assert!(warning.is_none());
    use std::os::unix::ffi::OsStringExt;
    let raw_path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/name-\xff".to_vec()));
    preferences.left = raw_path.clone();
    preferences.favorites.push(raw_path);
    persistence::save(&preferences)?;
    let (readback, warning) = persistence::load();
    assert!(warning.is_none());
    assert_eq!(readback.left, preferences.left);
    assert_eq!(readback.favorites, preferences.favorites);
    let settings = root.join("Library/Application Support/Excavator/preferences.json");
    for invalid in [b"invalid".as_slice(), b"{\"version\":99}".as_slice()] {
        fs::write(&settings, invalid)?;
        assert!(persistence::load().1.is_some());
        assert!(persistence::save(&preferences).is_err());
        assert_eq!(fs::read(&settings)?, invalid);
    }
    println!(
        "Local listing, metadata, cancellation, native-path preferences, and corrupt/future settings preservation passed."
    );
    fs::remove_dir_all(&root)?;
    Ok(())
}

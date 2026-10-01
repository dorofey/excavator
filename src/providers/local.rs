use super::{CancellationToken, FileSystem, ListOptions, ProviderFuture};
use crate::domain::{Capabilities, Entry, EntryId, EntryKind, FsError, Location, ProviderId};
use std::{fs, path::PathBuf};

#[derive(Default)]
pub struct LocalFileSystem;

fn check_cancel(cancel: &CancellationToken, location: &Location) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(FsError::cancelled(location.clone()))
    } else {
        Ok(())
    }
}

fn read_entry(path: PathBuf) -> Result<Entry, FsError> {
    let location = Location::Local(path.clone());
    // Inspect the link itself, including dangling links; do not traverse its target.
    let metadata =
        fs::symlink_metadata(&path).map_err(|error| FsError::from_io(location.clone(), error))?;
    let file_type = metadata.file_type();
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Ok(Entry {
        name: path.file_name().unwrap_or(path.as_os_str()).to_os_string(),
        id: EntryId(location.clone()),
        location,
        kind,
        size: (kind == EntryKind::File).then_some(metadata.len()),
        modified: metadata.modified().ok(),
    })
}

impl FileSystem for LocalFileSystem {
    fn provider_id(&self) -> ProviderId {
        ProviderId::Local
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            list: true,
            metadata: true,
            symlinks: true,
            ..Capabilities::default()
        }
    }

    fn list(
        &self,
        location: Location,
        options: ListOptions,
        cancel: CancellationToken,
    ) -> ProviderFuture<Vec<Entry>> {
        Box::pin(async move {
            check_cancel(&cancel, &location)?;
            let directory = fs::read_dir(location.local_path())
                .map_err(|error| FsError::from_io(location.clone(), error))?;
            let mut entries = Vec::new();
            for result in directory {
                check_cancel(&cancel, &location)?;
                let item = result.map_err(|error| FsError::from_io(location.clone(), error))?;
                // as_encoded_bytes preserves non-UTF8 names; dot is ASCII on supported OSes.
                if !options.show_hidden
                    && item.file_name().as_encoded_bytes().first() == Some(&b'.')
                {
                    continue;
                }
                entries.push(read_entry(item.path())?);
            }
            check_cancel(&cancel, &location)?;
            entries.sort_by(|a, b| {
                (a.kind != EntryKind::Directory)
                    .cmp(&(b.kind != EntryKind::Directory))
                    .then_with(|| a.name.cmp(&b.name))
                    .then_with(|| a.location.display().cmp(&b.location.display()))
            });
            check_cancel(&cancel, &location)?;
            Ok(entries)
        })
    }

    fn metadata(&self, location: Location, cancel: CancellationToken) -> ProviderFuture<Entry> {
        Box::pin(async move {
            check_cancel(&cancel, &location)?;
            let entry = read_entry(location.local_path().to_path_buf())?;
            check_cancel(&cancel, &location)?;
            Ok(entry)
        })
    }
}

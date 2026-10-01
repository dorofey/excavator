//! Provider-neutral streaming operations. Called only by the bounded background worker.
use super::{
    CompletedOperation, ConflictPolicy, JobId, JobState, Operation, OperationPlan, TransferManager,
};
use crate::{
    domain::{EntryKind, FsError, FsErrorKind, Location},
    providers::{CancellationToken, ListOptions, ProviderRegistry},
};
use std::{
    ffi::OsStr,
    io::{Read, Write},
};
fn error(location: &Location, kind: FsErrorKind, message: &str) -> FsError {
    FsError {
        location: location.clone(),
        kind,
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
fn journal(
    manager: &TransferManager,
    id: JobId,
    source: Option<&Location>,
    target: Option<&Location>,
    description: &str,
) {
    manager.update(id, |job| {
        job.journal.push(CompletedOperation {
            source: source.cloned(),
            destination: target.cloned(),
            description: description.into(),
        })
    });
}
fn existing(
    registry: &ProviderRegistry,
    target: &Location,
    cancel: &CancellationToken,
) -> Result<bool, FsError> {
    match registry.metadata_sync(target, cancel) {
        Ok(_) => Ok(true),
        Err(e) if e.kind == FsErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
fn destination(
    registry: &ProviderRegistry,
    manager: &TransferManager,
    id: JobId,
    source: &Location,
    mut target: Location,
    policy: ConflictPolicy,
    cancel: &CancellationToken,
) -> Result<Option<Location>, FsError> {
    if source == &target {
        return Err(error(
            source,
            FsErrorKind::InvalidOperation,
            "Source and destination are the same item",
        ));
    }
    if !existing(registry, &target, cancel)? {
        return Ok(Some(target));
    }
    match policy {
        ConflictPolicy::Ask => {
            manager.update(id, |job| {
                job.state = JobState::AwaitingConflict {
                    source: source.clone(),
                    destination: target.clone(),
                }
            });
            Err(error(
                &target,
                FsErrorKind::Conflict,
                "Destination exists; keep both, skip, or cancel. Remote replacement is disabled",
            ))
        }
        ConflictPolicy::Skip => {
            journal(
                manager,
                id,
                Some(source),
                Some(&target),
                "Skipped existing remote destination",
            );
            Ok(None)
        }
        ConflictPolicy::KeepBoth => {
            let parent = target.parent().ok_or_else(|| {
                error(
                    &target,
                    FsErrorKind::InvalidOperation,
                    "Destination needs a parent",
                )
            })?;
            let original = target.label();
            for n in 1..10000 {
                check(cancel, &target)?;
                target = parent.join(OsStr::new(&format!("{original} (copy {n})")))?;
                if !existing(registry, &target, cancel)? {
                    return Ok(Some(target));
                }
            }
            Err(error(
                &target,
                FsErrorKind::Conflict,
                "Cannot allocate an unused destination name",
            ))
        }
        ConflictPolicy::Replace => Err(error(
            &target,
            FsErrorKind::Unsupported,
            "Safe remote replacement is disabled; use Keep both",
        )),
        ConflictPolicy::Cancel => Err(FsError::cancelled(source.clone())),
    }
}
pub(super) fn execute_remote(
    manager: &TransferManager,
    id: JobId,
    plan: &OperationPlan,
    cancel: &CancellationToken,
) -> Result<(), FsError> {
    let registry = &manager.0.registry;
    let context = plan
        .destination
        .as_ref()
        .or(plan.sources.first())
        .ok_or_else(|| {
            error(
                &Location::Local(".".into()),
                FsErrorKind::InvalidOperation,
                "Operation requires a source or destination",
            )
        })?;
    check(cancel, context)?;
    if matches!(plan.operation, Operation::Move | Operation::Trash) {
        return Err(error(
            context,
            FsErrorKind::Unsupported,
            "Remote Move and Trash are disabled; copy and verify first, then explicitly delete individual remote items",
        ));
    }
    if plan.operation == Operation::CreateDirectory {
        let parent = plan.destination.as_ref().ok_or_else(|| {
            error(
                context,
                FsErrorKind::InvalidOperation,
                "Choose a destination directory",
            )
        })?;
        let name = plan
            .new_name
            .as_ref()
            .ok_or_else(|| error(context, FsErrorKind::InvalidOperation, "A name is required"))?;
        let target = parent.join(name)?;
        registry.create_dir(&target, cancel)?;
        journal(
            manager,
            id,
            None,
            Some(&target),
            "Created provider directory",
        );
        manager.update(id, |job| job.completed_items = 1);
        return Ok(());
    }
    if plan.sources.is_empty() {
        return Err(error(
            context,
            FsErrorKind::InvalidOperation,
            "Select at least one source",
        ));
    }
    if plan.operation == Operation::Delete {
        for source in &plan.sources {
            check(cancel, source)?;
            if source.is_local() {
                return Err(error(
                    source,
                    FsErrorKind::Unsupported,
                    "Permanent local deletion is disabled; use system Trash",
                ));
            }
            manager.update(id, |job| job.current = Some(source.clone()));
            registry.delete(source, cancel)?;
            journal(
                manager,
                id,
                Some(source),
                None,
                "Permanently deleted explicitly confirmed remote item (nonrecursive)",
            );
            manager.update(id, |job| job.completed_items += 1);
        }
        return Ok(());
    }
    if plan.operation == Operation::Rename {
        if plan.sources.len() != 1 {
            return Err(error(
                context,
                FsErrorKind::InvalidOperation,
                "Rename requires one source",
            ));
        }
        let source = &plan.sources[0];
        let parent = source.parent().ok_or_else(|| {
            error(
                source,
                FsErrorKind::InvalidOperation,
                "Cannot rename a provider root",
            )
        })?;
        let name = plan
            .new_name
            .as_ref()
            .ok_or_else(|| error(source, FsErrorKind::InvalidOperation, "A name is required"))?;
        let target = parent.join(name)?;
        let Some(target) = destination(
            registry,
            manager,
            id,
            source,
            target,
            plan.conflict_policy,
            cancel,
        )?
        else {
            return Ok(());
        };
        registry.rename(source, &target, cancel)?;
        journal(
            manager,
            id,
            Some(source),
            Some(&target),
            "Renamed remote item without replacing existing destination",
        );
        manager.update(id, |job| job.completed_items = 1);
        return Ok(());
    }
    let parent = plan.destination.as_ref().ok_or_else(|| {
        error(
            context,
            FsErrorKind::InvalidOperation,
            "Choose a destination directory",
        )
    })?;
    let mut mappings = Vec::new();
    // Preflight all top-level conflicts before creating any destination tree.
    for source in &plan.sources {
        registry.validate_copy(source, parent, cancel)?;
        let entry = registry.metadata_sync(source, cancel)?;
        let target = parent.join(&entry.name)?;
        guard_descendant(source, &target)?;
        if let Some(target) = destination(
            registry,
            manager,
            id,
            source,
            target,
            plan.conflict_policy,
            cancel,
        )? {
            if mappings.iter().any(|(_, prior)| prior == &target) {
                return Err(error(
                    &target,
                    FsErrorKind::InvalidOperation,
                    "Multiple sources map to one destination",
                ));
            }
            mappings.push((source.clone(), target));
        }
    }
    for (source, target) in mappings {
        check(cancel, &source)?;
        manager.update(id, |job| job.current = Some(source.clone()));
        copy_tree(registry, manager, id, &source, &target, cancel, 0)?;
        manager.update(id, |job| job.completed_items += 1);
    }
    Ok(())
}
fn guard_descendant(source: &Location, target: &Location) -> Result<(), FsError> {
    let overlaps = match (source, target) {
        (
            Location::Sftp {
                connection: a,
                path: x,
            },
            Location::Sftp {
                connection: b,
                path: y,
            },
        )
        | (
            Location::Ftps {
                connection: a,
                path: x,
            },
            Location::Ftps {
                connection: b,
                path: y,
            },
        ) => a == b && (x == y || y.starts_with(&format!("{}/", x.trim_end_matches('/')))),
        (
            Location::S3 {
                connection: a,
                bucket: ab,
                key: x,
                prefix: true,
            },
            Location::S3 {
                connection: b,
                bucket: bb,
                key: y,
                ..
            },
        ) => a == b && ab == bb && (x == y || y.starts_with(x)),
        _ => source == target,
    };
    if overlaps {
        Err(error(
            source,
            FsErrorKind::InvalidOperation,
            "Destination is inside the selected source",
        ))
    } else {
        Ok(())
    }
}
fn copy_tree(
    registry: &ProviderRegistry,
    manager: &TransferManager,
    id: JobId,
    source: &Location,
    target: &Location,
    cancel: &CancellationToken,
    depth: usize,
) -> Result<(), FsError> {
    check(cancel, source)?;
    if depth > 128 {
        return Err(error(
            source,
            FsErrorKind::Unsupported,
            "Directory nesting exceeds the safe transfer limit",
        ));
    }
    let before = registry.metadata_sync(source, cancel)?;
    if before.kind == EntryKind::Directory {
        // S3 prefixes are implicit; child objects create them without marker writes.
        let directory_target = match target {
            Location::S3 {
                connection,
                bucket,
                key,
                ..
            } => Location::S3 {
                connection: connection.clone(),
                bucket: bucket.clone(),
                key: format!("{}/", key.trim_end_matches('/')),
                prefix: true,
            },
            _ => {
                registry.create_dir(target, cancel)?;
                target.clone()
            }
        };
        journal(
            manager,
            id,
            Some(source),
            Some(&directory_target),
            "Created destination tree; may be partial on cancellation or failure",
        );
        for child in registry.list_sync(source, ListOptions { show_hidden: true }, cancel)? {
            let target = directory_target.join(&child.name)?;
            copy_tree(
                registry,
                manager,
                id,
                &child.location,
                &target,
                cancel,
                depth + 1,
            )?;
        }
        return Ok(());
    }
    if before.kind != EntryKind::File {
        return Err(error(
            source,
            FsErrorKind::Unsupported,
            "Remote transfer does not follow or recreate symlinks and special files",
        ));
    }
    let mut input = registry.open_read(source, cancel)?;
    let mut output = registry.create_write(target, cancel)?;
    let copied = (|| -> Result<(), FsError> {
        let mut buffer = vec![0; 256 * 1024];
        let mut bytes = 0u64;
        loop {
            check(cancel, source)?;
            let count = input.read(&mut buffer).map_err(|_| {
                if cancel.is_cancelled() {
                    return FsError::cancelled(source.clone());
                }
                error(
                    source,
                    FsErrorKind::Io,
                    "Streaming source read failed; destination is not committed",
                )
            })?;
            if count == 0 {
                break;
            }
            check(cancel, target)?;
            output.write_all(&buffer[..count]).map_err(|_| {
                error(
                    target,
                    FsErrorKind::Io,
                    "Streaming destination write failed; destination is not committed",
                )
            })?;
            bytes += count as u64;
            manager.update(id, |job| job.bytes_copied += count as u64);
        }
        if before.size.is_some_and(|size| size != bytes) {
            return Err(error(
                source,
                FsErrorKind::Io,
                "Source byte count changed during streaming",
            ));
        }
        let after = registry.metadata_sync(source, cancel)?;
        if before.kind != after.kind
            || before.size != after.size
            || before.modified != after.modified
        {
            return Err(error(
                source,
                FsErrorKind::InvalidOperation,
                "Source metadata changed during streaming; destination is not committed",
            ));
        }
        check(cancel, target)?;
        output.finish(cancel)?;
        Ok(())
    })();
    if let Err(mut failure) = copied {
        if output.committed() {
            journal(
                manager,
                id,
                Some(source),
                Some(target),
                &format!(
                    "Destination committed but subsequent verification/cleanup failed: {}",
                    failure.message
                ),
            );
        }
        if let Err(cleanup) = output.abort() {
            journal(
                manager,
                id,
                Some(source),
                Some(&cleanup.location),
                &format!("Remote staging cleanup failed: {}", cleanup.message),
            );
            failure.message.push_str(&format!(
                "; staging cleanup also failed: {}",
                cleanup.message
            ));
        }
        return Err(failure);
    }
    journal(
        manager,
        id,
        Some(source),
        Some(target),
        "Committed provider file copy; source retained",
    );
    Ok(())
}

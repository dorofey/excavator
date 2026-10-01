//! Local operation queue. `run_pending` must run on a background executor.
//! One worker executes at a time; UI access only takes brief snapshot locks.
mod remote;
use crate::domain::{FsError, FsErrorKind, Location};
use crate::providers::CancellationToken;
use std::os::unix::{
    ffi::OsStrExt,
    fs::{MetadataExt, symlink},
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

pub type JobId = u64;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    CreateDirectory,
    Rename,
    Copy,
    Move,
    Trash,
    Delete,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictPolicy {
    Ask,
    KeepBoth,
    Skip,
    Replace,
    Cancel,
}
#[derive(Clone, Debug)]
pub struct OperationPlan {
    pub operation: Operation,
    pub sources: Vec<Location>,
    pub destination: Option<Location>,
    pub new_name: Option<OsString>,
    pub conflict_policy: ConflictPolicy,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobState {
    Queued,
    Running,
    AwaitingConflict {
        source: Location,
        destination: Location,
    },
    Completed,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug)]
pub struct CompletedOperation {
    pub source: Option<Location>,
    pub destination: Option<Location>,
    pub description: String,
}
#[derive(Clone, Debug)]
pub struct JobSnapshot {
    pub id: JobId,
    pub plan: OperationPlan,
    pub state: JobState,
    pub completed_items: usize,
    pub total_items: usize,
    pub bytes_copied: u64,
    pub current: Option<Location>,
    pub journal: Vec<CompletedOperation>,
    pub error: Option<FsError>,
}
struct Job {
    snapshot: JobSnapshot,
    cancel: CancellationToken,
}
#[derive(Default)]
struct Shared {
    jobs: Mutex<BTreeMap<JobId, Job>>,
    next: AtomicU64,
    running: AtomicBool,
    registry: crate::providers::ProviderRegistry,
}
#[derive(Clone, Default)]
pub struct TransferManager(Arc<Shared>);
struct WorkerGuard(Arc<Shared>, bool);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.1 {
            self.0.running.store(false, Ordering::Release);
        }
    }
}

impl TransferManager {
    pub fn with_registry(registry: crate::providers::ProviderRegistry) -> Self {
        Self(Arc::new(Shared {
            registry,
            ..Shared::default()
        }))
    }
    pub fn enqueue(&self, plan: OperationPlan) -> JobId {
        let id = self.0.next.fetch_add(1, Ordering::Relaxed) + 1;
        let total_items = if plan.operation == Operation::CreateDirectory {
            1
        } else {
            plan.sources.len()
        };
        self.0.jobs.lock().unwrap().insert(
            id,
            Job {
                snapshot: JobSnapshot {
                    id,
                    plan,
                    state: JobState::Queued,
                    completed_items: 0,
                    total_items,
                    bytes_copied: 0,
                    current: None,
                    journal: Vec::new(),
                    error: None,
                },
                cancel: CancellationToken::new(),
            },
        );
        id
    }
    pub fn snapshots(&self) -> Vec<JobSnapshot> {
        self.0
            .jobs
            .lock()
            .unwrap()
            .values()
            .map(|job| job.snapshot.clone())
            .collect()
    }
    pub fn cancel(&self, id: JobId) {
        if let Some(job) = self.0.jobs.lock().unwrap().get_mut(&id) {
            job.cancel.cancel();
            if matches!(
                job.snapshot.state,
                JobState::Queued | JobState::AwaitingConflict { .. }
            ) {
                job.snapshot.state = JobState::Cancelled;
            }
        }
    }
    pub fn resolve_conflict(&self, id: JobId, policy: ConflictPolicy) {
        if let Some(job) = self.0.jobs.lock().unwrap().get_mut(&id) {
            if matches!(job.snapshot.state, JobState::AwaitingConflict { .. }) {
                job.snapshot.plan.conflict_policy = policy;
                job.snapshot.error = None;
                job.snapshot.state = if policy == ConflictPolicy::Cancel {
                    JobState::Cancelled
                } else {
                    JobState::Queued
                };
            }
        }
    }
    fn update(&self, id: JobId, update: impl FnOnce(&mut JobSnapshot)) {
        if let Some(job) = self.0.jobs.lock().unwrap().get_mut(&id) {
            update(&mut job.snapshot);
        }
    }
    fn record(
        &self,
        id: JobId,
        source: Option<&Path>,
        destination: Option<&Path>,
        description: &str,
    ) {
        self.update(id, |job| {
            job.journal.push(CompletedOperation {
                source: source.map(|p| Location::Local(p.into())),
                destination: destination.map(|p| Location::Local(p.into())),
                description: description.into(),
            })
        });
    }
    /// Blocking worker entry point. Concurrent calls cannot execute concurrent jobs.
    pub fn run_pending(&self) {
        if self
            .0
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let mut guard = WorkerGuard(self.0.clone(), true);
        loop {
            let next = {
                let mut jobs = self.0.jobs.lock().unwrap();
                let next = jobs
                    .values_mut()
                    .find(|job| job.snapshot.state == JobState::Queued)
                    .map(|job| {
                        job.snapshot.state = JobState::Running;
                        (
                            job.snapshot.id,
                            job.snapshot.plan.clone(),
                            job.cancel.clone(),
                        )
                    });
                if next.is_none() {
                    // Release worker ownership under the same lock as enqueue,
                    // avoiding a queued job losing its background-worker wakeup.
                    self.0.running.store(false, Ordering::Release);
                    guard.1 = false;
                }
                next
            };
            let Some((id, plan, cancel)) = next else {
                break;
            };
            match self.execute(id, &plan, &cancel) {
                Ok(()) => self.update(id, |job| {
                    job.state = JobState::Completed;
                    job.current = None;
                }),
                Err(error) => self.update(id, |job| {
                    if !matches!(job.state, JobState::AwaitingConflict { .. }) {
                        job.state = if error.kind == FsErrorKind::Cancelled {
                            JobState::Cancelled
                        } else {
                            JobState::Failed
                        };
                    }
                    job.error = Some(error);
                    job.current = None;
                }),
            }
        }
    }

    fn execute(
        &self,
        id: JobId,
        plan: &OperationPlan,
        cancel: &CancellationToken,
    ) -> Result<(), FsError> {
        if plan
            .sources
            .iter()
            .chain(plan.destination.iter())
            .any(|location| !location.is_local())
        {
            return remote::execute_remote(self, id, plan, cancel);
        }
        if plan.operation == Operation::Delete {
            return Err(FsError {
                kind: FsErrorKind::Unsupported,
                location: plan
                    .sources
                    .first()
                    .cloned()
                    .unwrap_or(Location::Local(PathBuf::from("."))),
                message: "Local deletion uses Trash; permanent local deletion is unavailable"
                    .into(),
            });
        }
        let fallback = plan
            .destination
            .as_ref()
            .or(plan.sources.first())
            .map(|l| l.local_path())
            .unwrap_or(Path::new("."));
        check(cancel, fallback)?;
        if plan.conflict_policy == ConflictPolicy::Cancel {
            return Err(FsError::cancelled(Location::Local(fallback.into())));
        }
        if plan.operation == Operation::CreateDirectory {
            let parent = plan
                .destination
                .as_ref()
                .ok_or_else(|| invalid(fallback, "A destination directory is required"))?
                .local_path();
            let name = valid_name(plan.new_name.as_ref(), parent)?;
            let target = parent.join(name);
            fs::create_dir(&target).map_err(|e| io(&target, e))?;
            self.record(id, None, Some(&target), "Created directory");
            self.update(id, |job| job.completed_items += 1);
            return Ok(());
        }
        if plan.sources.is_empty() {
            return Err(invalid(fallback, "Select at least one source"));
        }
        let sources: Vec<PathBuf> = plan
            .sources
            .iter()
            .map(|l| absolute_leaf(l.local_path()))
            .collect::<Result<_, _>>()?;
        for (index, source) in sources.iter().enumerate() {
            for other in &sources[index + 1..] {
                let first = fs::symlink_metadata(source).map_err(|e| io(source, e))?;
                let second = fs::symlink_metadata(other).map_err(|e| io(other, e))?;
                let same_item = first.dev() == second.dev() && first.ino() == second.ino();
                let aliases_overlap = if first.is_dir() || second.is_dir() {
                    let a = if first.file_type().is_symlink() {
                        source.clone()
                    } else {
                        fs::canonicalize(source).map_err(|e| io(source, e))?
                    };
                    let b = if second.file_type().is_symlink() {
                        other.clone()
                    } else {
                        fs::canonicalize(other).map_err(|e| io(other, e))?
                    };
                    a.starts_with(&b) || b.starts_with(&a)
                } else {
                    false
                };
                if same_item
                    || aliases_overlap
                    || source.starts_with(other)
                    || other.starts_with(source)
                {
                    return Err(invalid(
                        source,
                        "Sources overlap or alias the same item; select each tree only once",
                    ));
                }
            }
        }
        if plan.operation == Operation::Trash {
            for source in &sources {
                check(cancel, source)?;
                self.update(id, |job| {
                    job.current = Some(Location::Local(source.clone()))
                });
                trash::delete(source).map_err(|e| FsError {
                    kind: FsErrorKind::Io,
                    location: Location::Local(source.clone()),
                    message: format!("Trash failed: {e}"),
                })?;
                self.record(id, Some(source), None, "Moved to system Trash");
                self.update(id, |job| job.completed_items += 1);
            }
            return Ok(());
        }
        if plan.operation == Operation::Rename && sources.len() != 1 {
            return Err(invalid(fallback, "Rename requires exactly one source"));
        }
        let moving = matches!(plan.operation, Operation::Move | Operation::Rename);
        let mut targets = Vec::new();
        // Resolve all conflicts before writing any source. Ask leaves a resumable job.
        for source in &sources {
            let metadata = fs::symlink_metadata(source).map_err(|e| io(source, e))?;
            let parent = if plan.operation == Operation::Rename {
                source.parent().unwrap().to_path_buf()
            } else {
                fs::canonicalize(
                    plan.destination
                        .as_ref()
                        .ok_or_else(|| invalid(fallback, "A destination directory is required"))?
                        .local_path(),
                )
                .map_err(|e| io(fallback, e))?
            };
            if !fs::metadata(&parent).map_err(|e| io(&parent, e))?.is_dir() {
                return Err(invalid(&parent, "Destination is not a directory"));
            }
            let name = if plan.operation == Operation::Rename {
                valid_name(plan.new_name.as_ref(), source)?
            } else {
                source
                    .file_name()
                    .ok_or_else(|| invalid(source, "Cannot operate on a filesystem root"))?
            };
            let mut target = parent.join(name);
            if source == &target || (metadata.is_dir() && target.starts_with(source)) {
                return Err(invalid(
                    source,
                    "Destination is the source or inside its directory tree",
                ));
            }
            let existing = existing_metadata(&target)?;
            if existing.as_ref().is_some_and(|destination| {
                destination.dev() == metadata.dev() && destination.ino() == metadata.ino()
            }) {
                return Err(invalid(
                    source,
                    "Destination refers to the same filesystem item as source",
                ));
            }
            if metadata.is_dir() {
                let real_source = fs::canonicalize(source).map_err(|e| io(source, e))?;
                if parent.starts_with(&real_source) {
                    return Err(invalid(
                        source,
                        "Destination parent is inside the source tree",
                    ));
                }
            }
            let replace = match existing {
                None => false,
                Some(ref destination_metadata) => match plan.conflict_policy {
                    ConflictPolicy::Ask => {
                        self.update(id, |job| {
                            job.state = JobState::AwaitingConflict {
                                source: Location::Local(source.clone()),
                                destination: Location::Local(target.clone()),
                            }
                        });
                        return Err(FsError {
                            kind: FsErrorKind::Conflict,
                            location: Location::Local(target),
                            message:
                                "Destination exists; choose replace, keep both, skip, or cancel"
                                    .into(),
                        });
                    }
                    ConflictPolicy::KeepBoth => {
                        target = keep_both(&target)?;
                        false
                    }
                    ConflictPolicy::Skip => {
                        if moving {
                            return Err(invalid(
                                source,
                                "Move cannot skip conflicts; source was left unchanged",
                            ));
                        }
                        self.record(
                            id,
                            Some(source),
                            Some(&target),
                            "Skipped existing destination",
                        );
                        continue;
                    }
                    ConflictPolicy::Replace => {
                        if !metadata.file_type().is_file()
                            || !destination_metadata.file_type().is_file()
                        {
                            return Err(invalid(
                                &target,
                                "Replace supports regular files only; directories and symlinks are protected",
                            ));
                        }
                        true
                    }
                    ConflictPolicy::Cancel => unreachable!(),
                },
            };
            if targets.iter().any(|(_, previous, _)| previous == &target) {
                return Err(invalid(
                    &target,
                    "Multiple sources map to the same destination",
                ));
            }
            targets.push((source.clone(), target, replace));
        }
        for (source, target, replace) in targets {
            check(cancel, &source)?;
            self.update(id, |job| {
                job.current = Some(Location::Local(source.clone()))
            });
            let original = collect_tree(&source, cancel)?;
            self.copy_tree(id, &source, &target, replace, cancel)?;
            if moving {
                check(cancel, &source)?;
                verify_tree(&source, &target, &original, cancel)?;
                check(cancel, &source)?;
                // Never traverse symlinks during deletion. Verify the tree again immediately
                // before deletion; concurrent filesystem mutation can still race OS calls.
                if collect_tree(&source, cancel)? != original {
                    return Err(invalid(
                        &source,
                        "Source changed after copying; source retained",
                    ));
                }
                remove_tree_verified(&source, &original, cancel, self, id)?;
                self.record(
                    id,
                    Some(&source),
                    Some(&target),
                    "Verified copy and removed source",
                );
            } else {
                self.record(id, Some(&source), Some(&target), "Copied source");
            }
            self.update(id, |job| job.completed_items += 1);
        }
        Ok(())
    }

    fn copy_tree(
        &self,
        id: JobId,
        source: &Path,
        target: &Path,
        replace: bool,
        cancel: &CancellationToken,
    ) -> Result<(), FsError> {
        check(cancel, source)?;
        let metadata = fs::symlink_metadata(source).map_err(|e| io(source, e))?;
        if metadata.file_type().is_symlink() {
            let link = fs::read_link(source).map_err(|e| io(source, e))?;
            symlink(link, target).map_err(|e| io(target, e))?;
            self.record(
                id,
                Some(source),
                Some(target),
                "Created symbolic link without following target",
            );
        } else if metadata.is_dir() {
            fs::create_dir(target).map_err(|e| io(target, e))?;
            self.record(
                id,
                Some(source),
                Some(target),
                "Created destination directory (may be partial)",
            );
            for child in fs::read_dir(source).map_err(|e| io(source, e))? {
                check(cancel, source)?;
                let child = child.map_err(|e| io(source, e))?;
                self.copy_tree(
                    id,
                    &child.path(),
                    &target.join(child.file_name()),
                    false,
                    cancel,
                )?;
            }
            fs::set_permissions(target, metadata.permissions()).map_err(|e| io(target, e))?;
        } else if metadata.is_file() {
            self.copy_file(id, source, target, replace, cancel)?;
        } else {
            return Err(FsError {
                kind: FsErrorKind::Unsupported,
                location: Location::Local(source.into()),
                message: "Special files cannot be copied".into(),
            });
        }
        Ok(())
    }

    fn copy_file(
        &self,
        id: JobId,
        source: &Path,
        target: &Path,
        replace: bool,
        cancel: &CancellationToken,
    ) -> Result<(), FsError> {
        let (stage, mut output) = create_stage(target, id)?;
        let mut cleanup = StageGuard(Some(stage.clone()), self.clone(), id);
        let mut input = File::open(source).map_err(|e| io(source, e))?;
        let before = fingerprint(&input.metadata().map_err(|e| io(source, e))?);
        let source_meta = fs::symlink_metadata(source).map_err(|e| io(source, e))?;
        if !source_meta.file_type().is_file() || fingerprint(&source_meta) != before {
            return Err(invalid(
                source,
                "Source changed while opening; copy stopped",
            ));
        }
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            check(cancel, source)?;
            let count = input.read(&mut buffer).map_err(|e| io(source, e))?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| io(&stage, e))?;
            self.update(id, |job| job.bytes_copied += count as u64);
        }
        output
            .set_permissions(source_meta.permissions())
            .map_err(|e| io(&stage, e))?;
        output.sync_all().map_err(|e| io(&stage, e))?;
        if fingerprint(&input.metadata().map_err(|e| io(source, e))?) != before
            || fingerprint(&fs::symlink_metadata(source).map_err(|e| io(source, e))?) != before
        {
            return Err(invalid(
                source,
                "Source changed during copying; destination not installed",
            ));
        }
        check(cancel, source)?;
        if replace {
            let old = fs::symlink_metadata(target).map_err(|e| io(target, e))?;
            if !old.file_type().is_file() {
                return Err(invalid(
                    target,
                    "Replacement target is no longer a regular file",
                ));
            }
            // macOS atomically exchanges the complete staged file and destination.
            // The previous destination remains at the owned staging path: no
            // check-then-unlink window can destroy a concurrently changed target.
            atomic_exchange(&stage, target)?;
            cleanup.0 = None;
            self.record(
                id,
                Some(target),
                Some(&stage),
                "Retained replaced file backup at staging path",
            );
            let saved = fs::symlink_metadata(&stage).map_err(|e| io(&stage, e))?;
            if fingerprint(&saved) != fingerprint(&old) {
                return Err(invalid(
                    &stage,
                    "Destination changed during replacement; exchanged original retained here; review both paths",
                ));
            }
        } else {
            // Same-directory hard link installs atomically without overwriting a race winner.
            fs::hard_link(&stage, target).map_err(|e| io(target, e))?;
        }
        self.record(id, Some(source), Some(target), "Installed complete file");
        Ok(())
    }
}

fn io(path: &Path, error: std::io::Error) -> FsError {
    FsError::from_io(Location::Local(path.into()), error)
}
fn invalid(path: &Path, message: &str) -> FsError {
    FsError {
        kind: FsErrorKind::InvalidOperation,
        location: Location::Local(path.into()),
        message: message.into(),
    }
}
fn check(cancel: &CancellationToken, path: &Path) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(FsError::cancelled(Location::Local(path.into())))
    } else {
        Ok(())
    }
}
fn valid_name<'a>(
    name: Option<&'a OsString>,
    context: &Path,
) -> Result<&'a std::ffi::OsStr, FsError> {
    let name = name.ok_or_else(|| invalid(context, "A name is required"))?;
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || name.as_bytes().contains(&0)
        || name.as_bytes().contains(&b'/')
    {
        return Err(invalid(
            context,
            "Name must be one filename, without path separators",
        ));
    }
    Ok(name)
}
fn absolute_leaf(path: &Path) -> Result<PathBuf, FsError> {
    let name = path
        .file_name()
        .ok_or_else(|| invalid(path, "Cannot operate on a filesystem root"))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|e| io(path, e))?;
    Ok(parent.join(name))
}
fn existing_metadata(path: &Path) -> Result<Option<Metadata>, FsError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(path, e)),
    }
}
fn keep_both(target: &Path) -> Result<PathBuf, FsError> {
    for number in 1..100_000 {
        let mut name = target.file_name().unwrap().to_os_string();
        name.push(format!(" (copy {number})"));
        let candidate = target.with_file_name(name);
        if existing_metadata(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(invalid(target, "Could not find an unused destination name"))
}
fn unused_sibling(target: &Path, id: JobId, role: &str) -> Result<PathBuf, FsError> {
    for number in 0..100_000 {
        let path = target.with_file_name(format!(
            ".excavator-{role}-{}-{id}-{number}",
            std::process::id()
        ));
        if existing_metadata(&path)?.is_none() {
            return Ok(path);
        }
    }
    Err(invalid(target, "Could not allocate operation staging name"))
}
fn create_stage(target: &Path, id: JobId) -> Result<(PathBuf, File), FsError> {
    for _ in 0..1000 {
        let path = unused_sibling(target, id, "stage")?;
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io(&path, e)),
        }
    }
    Err(invalid(target, "Staging file allocation raced repeatedly"))
}
struct StageGuard(Option<PathBuf>, TransferManager, JobId);
impl Drop for StageGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            if let Err(error) = fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    self.1.record(
                        self.2,
                        None,
                        Some(path),
                        &format!("Staging cleanup failed; retained path: {error}"),
                    );
                }
            }
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Fingerprint {
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    modified: i64,
    modified_nsec: i64,
}
fn fingerprint(meta: &Metadata) -> Fingerprint {
    Fingerprint {
        device: meta.dev(),
        inode: meta.ino(),
        mode: meta.mode(),
        size: meta.len(),
        modified: meta.mtime(),
        modified_nsec: meta.mtime_nsec(),
    }
}
fn collect_tree(
    root: &Path,
    cancel: &CancellationToken,
) -> Result<BTreeMap<PathBuf, Fingerprint>, FsError> {
    fn visit(
        root: &Path,
        path: &Path,
        cancel: &CancellationToken,
        entries: &mut BTreeMap<PathBuf, Fingerprint>,
    ) -> Result<(), FsError> {
        check(cancel, path)?;
        let metadata = fs::symlink_metadata(path).map_err(|e| io(path, e))?;
        entries.insert(
            path.strip_prefix(root).unwrap().into(),
            fingerprint(&metadata),
        );
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            for child in fs::read_dir(path).map_err(|e| io(path, e))? {
                visit(
                    root,
                    &child.map_err(|e| io(path, e))?.path(),
                    cancel,
                    entries,
                )?;
            }
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    visit(root, root, cancel, &mut entries)?;
    Ok(entries)
}
fn verify_tree(
    source: &Path,
    target: &Path,
    original: &BTreeMap<PathBuf, Fingerprint>,
    cancel: &CancellationToken,
) -> Result<(), FsError> {
    if &collect_tree(source, cancel)? != original {
        return Err(invalid(
            source,
            "Source changed during copying; source retained",
        ));
    }
    let copied_snapshot = collect_tree(target, cancel)?;
    for relative in original.keys() {
        let from = tree_path(source, relative);
        let to = tree_path(target, relative);
        check(cancel, &from)?;
        let a = fs::symlink_metadata(&from).map_err(|e| io(&from, e))?;
        let b = fs::symlink_metadata(&to).map_err(|e| io(&to, e))?;
        if a.file_type() != b.file_type() {
            return Err(invalid(&from, "Copied type differs; source retained"));
        }
        if a.file_type().is_symlink() {
            if fs::read_link(&from).map_err(|e| io(&from, e))?
                != fs::read_link(&to).map_err(|e| io(&to, e))?
            {
                return Err(invalid(&from, "Copied link differs; source retained"));
            }
        } else if a.is_file() {
            let mut left = File::open(&from).map_err(|e| io(&from, e))?;
            let mut right = File::open(&to).map_err(|e| io(&to, e))?;
            let mut x = vec![0u8; 256 * 1024];
            let mut y = vec![0u8; 256 * 1024];
            loop {
                check(cancel, &from)?;
                let count = left.read(&mut x).map_err(|e| io(&from, e))?;
                right.read_exact(&mut y[..count]).map_err(|e| io(&to, e))?;
                if x[..count] != y[..count] {
                    return Err(invalid(&from, "Copied bytes differ; source retained"));
                }
                if count == 0 {
                    let mut tail = [0];
                    if right.read(&mut tail).map_err(|e| io(&to, e))? != 0 {
                        return Err(invalid(&to, "Copied file has extra bytes; source retained"));
                    }
                    break;
                }
            }
        }
    }
    if &collect_tree(source, cancel)? != original {
        return Err(invalid(
            source,
            "Source changed during verification; source retained",
        ));
    }
    if collect_tree(target, cancel)? != copied_snapshot {
        return Err(invalid(
            target,
            "Destination changed during verification; source retained",
        ));
    }
    Ok(())
}
fn remove_tree_verified(
    root: &Path,
    original: &BTreeMap<PathBuf, Fingerprint>,
    cancel: &CancellationToken,
    manager: &TransferManager,
    id: JobId,
) -> Result<(), FsError> {
    let mut paths: Vec<_> = original.keys().collect();
    paths.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for relative in paths {
        let path = tree_path(root, relative);
        check(cancel, &path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| io(&path, e))?;
        let saved = &original[relative];
        // Directory mtimes change as children are removed; inode/type still must match.
        let current = fingerprint(&metadata);
        if current.device != saved.device
            || current.inode != saved.inode
            || current.mode != saved.mode
            || (!metadata.is_dir() && current != *saved)
        {
            return Err(invalid(
                &path,
                "Source changed before removal; remaining source retained",
            ));
        }
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir(&path).map_err(|e| io(&path, e))?;
        } else {
            fs::remove_file(&path).map_err(|e| io(&path, e))?;
        }
        manager.record(
            id,
            Some(&path),
            None,
            "Removed verified source item; destination copy retained",
        );
    }
    Ok(())
}

fn tree_path(root: &Path, relative: &Path) -> PathBuf {
    if relative.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    }
}

#[cfg(target_os = "macos")]
fn atomic_exchange(stage: &Path, target: &Path) -> Result<(), FsError> {
    use std::ffi::CString;
    unsafe extern "C" {
        fn renamex_np(
            from: *const std::ffi::c_char,
            to: *const std::ffi::c_char,
            flags: u32,
        ) -> std::ffi::c_int;
    }
    let from = CString::new(stage.as_os_str().as_bytes())
        .map_err(|_| invalid(stage, "Path contains NUL"))?;
    let to = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| invalid(target, "Path contains NUL"))?;
    // SDK sys/stdio.h declares renamex_np and RENAME_SWAP = 0x00000002.
    if unsafe { renamex_np(from.as_ptr(), to.as_ptr(), 0x00000002) } != 0 {
        return Err(io(target, std::io::Error::last_os_error()));
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn atomic_exchange(_stage: &Path, target: &Path) -> Result<(), FsError> {
    Err(FsError {
        kind: FsErrorKind::Unsupported,
        location: Location::Local(target.into()),
        message: "Safe replacement requires the macOS atomic file exchange adapter".into(),
    })
}

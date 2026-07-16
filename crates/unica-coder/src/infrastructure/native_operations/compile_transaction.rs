//! Failure-atomic publication for the metadata compile writers.
//!
//! The compile families build their files in memory, add them to a single
//! [`CompileTransaction`], and publish the transaction once.  Object files are
//! create-only.  Existing XML registration targets are edited textually through
//! the canonical `cf.edit` registrar and are never re-serialized.
//!
//! "Failure-atomic" here covers reported I/O and validation errors. It does not
//! claim process-crash or power-loss atomicity; those require a persistent journal
//! and directory-entry synchronization that this transaction does not provide.

use fs2::FileExt;
use roxmltree::Document;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError, Weak};

#[cfg(test)]
use std::cell::{Cell, RefCell};

#[cfg(test)]
use std::sync::{mpsc::Sender, Barrier};

use super::cf::cf_edit_add_child_object_text;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static REGISTRATION_PROCESS_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
    OnceLock::new();

/// Result of asking the canonical registrar to add one child object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegistrationStatus {
    Added,
    AlreadyPresent,
    MissingTarget,
}

/// Exact replacement required to turn the original bytes into planned bytes.
///
/// Applying `after` to `byte_range` of the original file reproduces the planned
/// registration file byte-for-byte.  `before` is included so callers can render
/// or verify a dry-run preview without reading the target again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegistrationDiff {
    pub(crate) path: PathBuf,
    pub(crate) byte_range: Range<usize>,
    pub(crate) before: Vec<u8>,
    pub(crate) after: Vec<u8>,
}

/// Files actually published by a successful transaction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CommitReport {
    pub(crate) created: Vec<PathBuf>,
    pub(crate) updated: Vec<PathBuf>,
    /// Cleanup failures do not invalidate already-validated published bytes.
    /// They are surfaced so a caller can report an orphaned backup explicitly.
    pub(crate) cleanup_warnings: Vec<String>,
}

#[derive(Debug)]
struct PlannedCreate {
    path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct PlannedRegistration {
    path: PathBuf,
    lock_path: PathBuf,
    original: Vec<u8>,
    updated: Vec<u8>,
    original_permissions: fs::Permissions,
}

impl PlannedRegistration {
    fn changed(&self) -> bool {
        self.original != self.updated
    }

    fn diff(&self) -> Option<RegistrationDiff> {
        self.changed()
            .then(|| byte_diff(&self.path, &self.original, &self.updated))
    }
}

/// In-memory plan for one compile invocation, including a `meta.compile` batch.
#[derive(Debug, Default)]
pub(crate) struct CompileTransaction {
    creates: Vec<PlannedCreate>,
    create_paths: HashSet<PathBuf>,
    registrations: BTreeMap<PathBuf, PlannedRegistration>,
}

impl CompileTransaction {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Plan a create-only file with exact bytes.
    pub(crate) fn create_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        let path = path.into();
        self.reject_duplicate_plan_path(&path)?;
        reject_existing_or_symlink_create_target(&path)?;
        self.create_paths.insert(path.clone());
        self.creates.push(PlannedCreate {
            path,
            bytes: bytes.into(),
        });
        Ok(())
    }

    /// Plan a create-only UTF-8 file without adding a BOM.
    #[allow(dead_code)]
    pub(crate) fn create_text(
        &mut self,
        path: impl Into<PathBuf>,
        text: impl AsRef<str>,
    ) -> Result<(), String> {
        self.create_bytes(path, text.as_ref().as_bytes().to_vec())
    }

    /// Plan a create-only UTF-8 file with exactly one leading BOM.
    pub(crate) fn create_utf8_bom_text(
        &mut self,
        path: impl Into<PathBuf>,
        text: impl AsRef<str>,
    ) -> Result<(), String> {
        let text = text.as_ref().trim_start_matches('\u{feff}');
        let mut bytes = Vec::with_capacity(UTF8_BOM.len() + text.len());
        bytes.extend_from_slice(UTF8_BOM);
        bytes.extend_from_slice(text.as_bytes());
        self.create_bytes(path, bytes)
    }

    /// Add one child object to an existing XML target using the canonical
    /// registrar. Multiple calls for the same target accumulate in memory and
    /// result in one replacement during commit.
    pub(crate) fn register_canonical_child(
        &mut self,
        target: impl Into<PathBuf>,
        type_name: &str,
        object_name: &str,
    ) -> Result<RegistrationStatus, String> {
        let target = target.into();
        if self.create_paths.contains(&target) {
            return Err(format!(
                "compile transaction path is both create-only and a registration target: {}",
                target.display()
            ));
        }

        if !self.registrations.contains_key(&target) {
            let metadata = match fs::symlink_metadata(&target) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    return Ok(RegistrationStatus::MissingTarget);
                }
                Err(error) => {
                    return Err(format!(
                        "failed to inspect registration target {}: {error}",
                        target.display()
                    ));
                }
            };
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "registration target must not be a symbolic link: {}",
                    target.display()
                ));
            }
            if !metadata.is_file() {
                return Err(format!(
                    "registration target is not a regular file: {}",
                    target.display()
                ));
            }
            let original = fs::read(&target).map_err(|error| {
                format!(
                    "failed to read registration target {}: {error}",
                    target.display()
                )
            })?;
            validate_xml_bytes(&target, &original)?;
            let lock_path = registration_lock_path(&target)?;
            self.registrations.insert(
                target.clone(),
                PlannedRegistration {
                    path: target.clone(),
                    lock_path,
                    updated: original.clone(),
                    original,
                    original_permissions: metadata.permissions(),
                },
            );
        }

        let registration = self
            .registrations
            .get_mut(&target)
            .ok_or_else(|| format!("registration target was not cached: {}", target.display()))?;
        let (bom, payload) = split_utf8_bom_prefix(&registration.updated);
        let source = std::str::from_utf8(payload).map_err(|error| {
            format!(
                "registration target is not valid UTF-8 {}: {error}",
                target.display()
            )
        })?;
        let source = source.to_string();
        let mut updated = source.clone();
        let changed = cf_edit_add_child_object_text(&mut updated, type_name, object_name).map_err(
            |error| {
                format!(
                    "failed to plan registration in {}: {error}",
                    target.display()
                )
            },
        )?;
        if !changed {
            return Ok(RegistrationStatus::AlreadyPresent);
        }

        updated = preserve_inserted_line_endings(&source, &updated);
        let mut updated_bytes = Vec::with_capacity(bom.len() + updated.len());
        updated_bytes.extend_from_slice(bom);
        updated_bytes.extend_from_slice(updated.as_bytes());
        validate_xml_bytes(&target, &updated_bytes)?;
        registration.updated = updated_bytes;
        Ok(RegistrationStatus::Added)
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.creates.is_empty()
            && self
                .registrations
                .values()
                .all(|registration| !registration.changed())
    }

    pub(crate) fn planned_created_paths(&self) -> Vec<PathBuf> {
        self.creates.iter().map(|file| file.path.clone()).collect()
    }

    pub(crate) fn planned_updated_paths(&self) -> Vec<PathBuf> {
        self.registrations
            .values()
            .filter(|registration| registration.changed())
            .map(|registration| registration.path.clone())
            .collect()
    }

    pub(crate) fn registration_diffs(&self) -> Vec<RegistrationDiff> {
        self.registrations
            .values()
            .filter_map(PlannedRegistration::diff)
            .collect()
    }

    /// Stable, compact `changes` entries suitable for a dry-run result.
    pub(crate) fn dry_run_changes(&self) -> Vec<String> {
        let mut changes = self
            .creates
            .iter()
            .map(|file| {
                format!(
                    "would create {} ({} bytes)",
                    file.path.display(),
                    file.bytes.len()
                )
            })
            .collect::<Vec<_>>();
        changes.extend(self.registration_diffs().into_iter().map(|diff| {
            format!(
                "would update {} bytes {}..{} ({} replacement bytes)",
                diff.path.display(),
                diff.byte_range.start,
                diff.byte_range.end,
                diff.after.len()
            )
        }));
        changes
    }

    /// Human-readable preview with hexadecimal before/after fragments. Hex is
    /// used deliberately so CR/LF, BOM, and non-ASCII bytes remain exact.
    pub(crate) fn dry_run_stdout(&self) -> String {
        let mut lines = self
            .creates
            .iter()
            .map(|file| {
                format!(
                    "[DRY-RUN] would create {} ({} bytes)",
                    file.path.display(),
                    file.bytes.len()
                )
            })
            .collect::<Vec<_>>();
        for diff in self.registration_diffs() {
            lines.push(format!("[DRY-RUN] would update {}", diff.path.display()));
            lines.push(format!(
                "@@ bytes {}..{} @@",
                diff.byte_range.start, diff.byte_range.end
            ));
            lines.push(format!(
                "  before-utf8: {:?}",
                String::from_utf8_lossy(&diff.before)
            ));
            lines.push(format!(
                "  after-utf8:  {:?}",
                String::from_utf8_lossy(&diff.after)
            ));
            lines.push(format!("  before-hex: {}", bytes_hex(&diff.before)));
            lines.push(format!("  after-hex:  {}", bytes_hex(&diff.after)));
        }
        if lines.is_empty() {
            "[DRY-RUN] no file changes\n".to_string()
        } else {
            format!("{}\n", lines.join("\n"))
        }
    }

    /// Stage, publish, validate, and finalize every planned change as one
    /// failure-atomic transaction for reported errors. This is not a
    /// process-crash or power-loss atomicity guarantee.
    pub(crate) fn commit(self) -> Result<CommitReport, String> {
        let lock_paths = self.registration_lock_paths();
        let process_locks = registration_process_locks(&lock_paths);
        let process_guards = process_locks
            .iter()
            .map(|lock| lock_registration_process_mutex(lock))
            .collect::<Vec<_>>();
        let file_locks = acquire_registration_file_locks(&lock_paths)?;
        pause_after_registration_locks();

        let mut state = PublishState::default();
        let result = match self.commit_inner(&mut state) {
            Ok(mut report) => {
                report.cleanup_warnings = finalize_success(&mut state);
                Ok(report)
            }
            Err(error) => {
                let rollback_errors = rollback(&mut state);
                if rollback_errors.is_empty() {
                    Err(error)
                } else {
                    Err(format!(
                        "{error}; rollback encountered: {}",
                        rollback_errors.join("; ")
                    ))
                }
            }
        };

        // Keep both lock layers alive through success cleanup or rollback. File
        // closure releases the advisory locks; persistent lock files must not be
        // removed because a waiter may already hold their inode open.
        drop(file_locks);
        drop(process_guards);
        result
    }

    fn commit_inner(&self, state: &mut PublishState) -> Result<CommitReport, String> {
        self.preflight()?;

        for create in &self.creates {
            ensure_parent_directories(&create.path, &mut state.created_dirs)?;
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
            ensure_parent_directories(&registration.path, &mut state.created_dirs)?;
        }

        for create in &self.creates {
            let staged = stage_bytes(&create.path, &create.bytes, None)?;
            state.staged_paths.push(staged.clone());
            state.create_stages.push(StagedCreate {
                target: create.path.clone(),
                staged,
            });
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
            let staged = stage_bytes(
                &registration.path,
                &registration.updated,
                Some(registration.original_permissions.clone()),
            )?;
            state.staged_paths.push(staged.clone());
            state.registration_stages.push(StagedRegistration {
                target: registration.path.clone(),
                staged,
                original: registration.original.clone(),
            });
        }

        for staged in &state.create_stages {
            reject_existing_or_symlink_create_target(&staged.target)?;
            fs::hard_link(&staged.staged, &staged.target).map_err(|error| {
                format!(
                    "failed to publish create-only file {}: {error}",
                    staged.target.display()
                )
            })?;
            state.created_paths.push(staged.target.clone());
            remove_if_exists(&staged.staged).map_err(|error| {
                format!(
                    "failed to remove staged link {}: {error}",
                    staged.staged.display()
                )
            })?;
        }

        failpoint_after_object_files()?;

        for staged in &state.registration_stages {
            reject_registration_target_change(&staged.target, &staged.original)?;
            let backup = reserve_backup(&staged.target)?;
            if let Err(error) = fs::rename(&staged.target, &backup.path) {
                let cleanup_error = fs::remove_dir(&backup.directory).err();
                let cleanup_note = cleanup_error.map_or_else(String::new, |cleanup_error| {
                    format!(
                        "; failed to remove empty backup reservation {}: {cleanup_error}",
                        backup.directory.display()
                    )
                });
                return Err(format!(
                    "failed to move registration target {} to backup {}: {error}{cleanup_note}",
                    staged.target.display(),
                    backup.path.display()
                ));
            }
            state.published_registrations.push(PublishedRegistration {
                target: staged.target.clone(),
                backup: backup.path,
                backup_directory: backup.directory,
                original: staged.original.clone(),
            });
            failpoint_after_registration_backup()?;
            fs::rename(&staged.staged, &staged.target).map_err(|error| {
                format!(
                    "failed to publish registration target {}: {error}",
                    staged.target.display()
                )
            })?;
        }

        self.post_validate()?;
        failpoint_post_write_validation()?;

        Ok(CommitReport {
            created: self.planned_created_paths(),
            updated: self.planned_updated_paths(),
            cleanup_warnings: Vec::new(),
        })
    }

    fn reject_duplicate_plan_path(&self, path: &Path) -> Result<(), String> {
        if self.create_paths.contains(path) || self.registrations.contains_key(path) {
            Err(format!(
                "compile transaction contains duplicate path: {}",
                path.display()
            ))
        } else {
            Ok(())
        }
    }

    fn registration_lock_paths(&self) -> Vec<PathBuf> {
        let mut paths = self
            .registrations
            .values()
            .filter(|registration| registration.changed())
            .map(|registration| registration.lock_path.clone())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }

    fn preflight(&self) -> Result<(), String> {
        for create in &self.creates {
            reject_existing_or_symlink_create_target(&create.path)?;
            validate_xml_when_applicable(&create.path, &create.bytes)?;
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
            reject_registration_target_change(&registration.path, &registration.original)?;
            validate_xml_bytes(&registration.path, &registration.updated)?;
        }
        Ok(())
    }

    fn post_validate(&self) -> Result<(), String> {
        for create in &self.creates {
            validate_published_file(&create.path, &create.bytes)?;
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
            validate_published_file(&registration.path, &registration.updated)?;
            validate_xml_bytes(&registration.path, &registration.updated)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct StagedCreate {
    target: PathBuf,
    staged: PathBuf,
}

#[derive(Debug)]
struct StagedRegistration {
    target: PathBuf,
    staged: PathBuf,
    original: Vec<u8>,
}

#[derive(Debug)]
struct PublishedRegistration {
    target: PathBuf,
    backup: PathBuf,
    backup_directory: PathBuf,
    original: Vec<u8>,
}

#[derive(Debug)]
struct BackupReservation {
    directory: PathBuf,
    path: PathBuf,
}

#[derive(Debug, Default)]
struct PublishState {
    create_stages: Vec<StagedCreate>,
    registration_stages: Vec<StagedRegistration>,
    staged_paths: Vec<PathBuf>,
    created_paths: Vec<PathBuf>,
    published_registrations: Vec<PublishedRegistration>,
    created_dirs: Vec<PathBuf>,
}

fn registration_lock_path(target: &Path) -> Result<PathBuf, String> {
    let canonical_target = fs::canonicalize(target).map_err(|error| {
        format!(
            "failed to canonicalize registration target {} for locking: {error}",
            target.display()
        )
    })?;
    let canonical_text = canonical_target.to_string_lossy();
    let mut hasher = Sha256::new();
    hasher.update(b"unica-compile-registration-lock-v1\0");
    #[cfg(any(windows, target_os = "macos"))]
    hasher.update(canonical_text.to_lowercase().as_bytes());
    #[cfg(not(any(windows, target_os = "macos")))]
    hasher.update(canonical_text.as_bytes());
    let lock_name = format!("{:x}.lock", hasher.finalize());
    Ok(std::env::temp_dir()
        .join("unica-compile-registration-locks-v1")
        .join(lock_name))
}

fn registration_process_locks(lock_paths: &[PathBuf]) -> Vec<Arc<Mutex<()>>> {
    let registry = REGISTRATION_PROCESS_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.retain(|_, lock| lock.strong_count() > 0);

    lock_paths
        .iter()
        .map(|path| {
            if let Some(lock) = registry.get(path).and_then(Weak::upgrade) {
                return lock;
            }
            let lock = Arc::new(Mutex::new(()));
            registry.insert(path.clone(), Arc::downgrade(&lock));
            lock
        })
        .collect()
}

fn lock_registration_process_mutex(lock: &Mutex<()>) -> MutexGuard<'_, ()> {
    match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => {
            signal_registration_lock_contention();
            lock.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
    }
}

fn acquire_registration_file_locks(lock_paths: &[PathBuf]) -> Result<Vec<File>, String> {
    lock_paths
        .iter()
        .map(|path| {
            let file = open_registration_lock_file(path)?;
            FileExt::lock_exclusive(&file).map_err(|error| {
                format!(
                    "failed to lock registration target via {}: {error}",
                    path.display()
                )
            })?;
            Ok(file)
        })
        .collect()
}

fn open_registration_lock_file(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create registration lock directory {}: {error}",
                parent.display()
            )
        })?;
    }
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).map_err(|inspect_error| {
                format!(
                    "failed to inspect existing registration lock {} after {error}: {inspect_error}",
                    path.display()
                )
            })?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "registration lock must not be a symbolic link: {}",
                    path.display()
                ));
            }
            if !metadata.is_file() {
                return Err(format!(
                    "registration lock is not a regular file: {}",
                    path.display()
                ));
            }
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|open_error| {
                    format!(
                        "failed to open existing registration lock {}: {open_error}",
                        path.display()
                    )
                })
        }
        Err(error) => Err(format!(
            "failed to create registration lock {}: {error}",
            path.display()
        )),
    }
}

fn reject_existing_or_symlink_create_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let kind = if metadata.file_type().is_symlink() {
                "symbolic link"
            } else if metadata.is_dir() {
                "directory"
            } else {
                "existing file"
            };
            Err(format!(
                "create-only compile target is already a {kind}: {}",
                path.display()
            ))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect create-only target {}: {error}",
            path.display()
        )),
    }
}

fn reject_registration_target_change(path: &Path, original: &[u8]) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "registration target disappeared before commit {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "registration target became a symbolic link before commit: {}",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "registration target is no longer a regular file: {}",
            path.display()
        ));
    }
    let current =
        fs::read(path).map_err(|error| format!("failed to re-read {}: {error}", path.display()))?;
    if current != original {
        return Err(format!(
            "registration target changed after planning: {}",
            path.display()
        ));
    }
    Ok(())
}

fn validate_published_file(path: &Path, expected: &[u8]) -> Result<(), String> {
    let actual = fs::read(path)
        .map_err(|error| format!("failed to validate published {}: {error}", path.display()))?;
    if actual != expected {
        return Err(format!(
            "post-write byte validation failed for {}",
            path.display()
        ));
    }
    validate_xml_when_applicable(path, &actual)
}

fn validate_xml_when_applicable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let is_xml = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"));
    if is_xml {
        validate_xml_bytes(path, bytes)
    } else {
        Ok(())
    }
}

fn validate_xml_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let (_, payload) = split_utf8_bom_prefix(bytes);
    let text = std::str::from_utf8(payload)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
    Document::parse(text.trim_start_matches('\u{feff}'))
        .map(|_| ())
        .map_err(|error| format!("XML parse error in {}: {error}", path.display()))
}

fn stage_bytes(
    target: &Path,
    bytes: &[u8],
    permissions: Option<fs::Permissions>,
) -> Result<PathBuf, String> {
    let mut attempts = 0usize;
    loop {
        attempts += 1;
        let staged = unique_sibling_path(target, "stage");
        let open = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staged);
        let mut file = match open {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempts < 16 => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create staged file {}: {error}",
                    staged.display()
                ));
            }
        };
        let write_result = write_and_sync(&mut file, bytes);
        drop(file);
        if let Err(error) = write_result {
            let _ = fs::remove_file(&staged);
            return Err(format!(
                "failed to write staged file {}: {error}",
                staged.display()
            ));
        }
        if let Some(permissions) = permissions {
            if let Err(error) = fs::set_permissions(&staged, permissions) {
                let _ = fs::remove_file(&staged);
                return Err(format!(
                    "failed to preserve permissions on staged file {}: {error}",
                    staged.display()
                ));
            }
        }
        return Ok(staged);
    }
}

fn reserve_backup(target: &Path) -> Result<BackupReservation, String> {
    reserve_backup_with(target, || unique_sibling_path(target, "backup"))
}

fn reserve_backup_with(
    target: &Path,
    mut next_directory: impl FnMut() -> PathBuf,
) -> Result<BackupReservation, String> {
    for attempt in 1..=16 {
        let directory = next_directory();
        match fs::create_dir(&directory) {
            Ok(()) => {
                return Ok(BackupReservation {
                    path: directory.join("original"),
                    directory,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt < 16 => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve no-clobber backup for {} at {}: {error}",
                    target.display(),
                    directory.display()
                ));
            }
        }
    }
    Err(format!(
        "failed to reserve no-clobber backup for {}",
        target.display()
    ))
}

fn write_and_sync(file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}

fn unique_sibling_path(target: &Path, label: &str) -> PathBuf {
    let parent = usable_parent(target);
    let mut name = OsString::from(".");
    name.push(
        target
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("compile")),
    );
    name.push(format!(
        ".unica-{label}-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    parent.join(name)
}

fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn ensure_parent_directories(path: &Path, created_dirs: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut current = usable_parent(path).to_path_buf();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(format!(
                        "compile target parent is not a directory: {}",
                        current.display()
                    ));
                }
                break;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                missing.push(current.clone());
                let Some(parent) = current.parent() else {
                    return Err(format!(
                        "cannot find an existing ancestor for {}",
                        path.display()
                    ));
                };
                current = if parent.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    parent.to_path_buf()
                };
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect parent {}: {error}",
                    current.display()
                ));
            }
        }
    }

    for directory in missing.into_iter().rev() {
        match fs::create_dir(&directory) {
            Ok(()) => created_dirs.push(directory),
            Err(error) if error.kind() == ErrorKind::AlreadyExists && directory.is_dir() => {}
            Err(error) => {
                return Err(format!(
                    "failed to create directory {}: {error}",
                    directory.display()
                ));
            }
        }
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    prepare_file_for_removal(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(windows))]
fn prepare_file_for_removal(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "on Windows this only clears the FILE_ATTRIBUTE_READONLY flag"
)]
fn prepare_file_for_removal(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn rollback(state: &mut PublishState) -> Vec<String> {
    let mut errors = Vec::new();

    for published in state.published_registrations.iter().rev() {
        match fs::symlink_metadata(&published.target) {
            Ok(_) => {
                if let Err(error) = remove_if_exists(&published.target) {
                    errors.push(format!(
                        "failed to remove published registration {}: {error}; original remains at {}",
                        published.target.display(),
                        published.backup.display()
                    ));
                    continue;
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                errors.push(format!(
                    "failed to inspect published registration {}: {error}; original remains at {}",
                    published.target.display(),
                    published.backup.display()
                ));
                continue;
            }
        }
        if let Err(error) = fs::rename(&published.backup, &published.target) {
            errors.push(format!(
                "failed to restore registration {} from {}: {error}",
                published.target.display(),
                published.backup.display()
            ));
            continue;
        }
        match fs::read(&published.target) {
            Ok(bytes) if bytes == published.original => {}
            Ok(_) => errors.push(format!(
                "restored registration bytes differ from original: {}",
                published.target.display()
            )),
            Err(error) => errors.push(format!(
                "failed to verify restored registration {}: {error}",
                published.target.display()
            )),
        }
        if let Err(error) = fs::remove_dir(&published.backup_directory) {
            errors.push(format!(
                "failed to remove restored registration backup directory {}: {error}",
                published.backup_directory.display()
            ));
        }
    }

    for path in state.created_paths.iter().rev() {
        if let Err(error) = remove_if_exists(path) {
            errors.push(format!(
                "failed to remove published create-only file {}: {error}",
                path.display()
            ));
        }
    }
    for path in &state.staged_paths {
        if let Err(error) = remove_if_exists(path) {
            errors.push(format!(
                "failed to remove staged file {}: {error}",
                path.display()
            ));
        }
    }
    for directory in state.created_dirs.iter().rev() {
        match fs::remove_dir(directory) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => errors.push(format!(
                "failed to remove transaction-created directory {}: {error}",
                directory.display()
            )),
        }
    }
    errors
}

fn finalize_success(state: &mut PublishState) -> Vec<String> {
    let mut warnings = Vec::new();
    for path in &state.staged_paths {
        if let Err(error) = remove_if_exists(path) {
            warnings.push(format!(
                "failed to remove staged file {}: {error}",
                path.display()
            ));
        }
    }
    for published in &state.published_registrations {
        if let Err(error) = remove_if_exists(&published.backup) {
            warnings.push(format!(
                "failed to remove registration backup {}: {error}",
                published.backup.display()
            ));
            continue;
        }
        if let Err(error) = fs::remove_dir(&published.backup_directory) {
            warnings.push(format!(
                "failed to remove registration backup directory {}: {error}",
                published.backup_directory.display()
            ));
        }
    }
    warnings
}

fn split_utf8_bom_prefix(bytes: &[u8]) -> (&[u8], &[u8]) {
    let mut offset = 0usize;
    while bytes[offset..].starts_with(UTF8_BOM) {
        offset += UTF8_BOM.len();
    }
    bytes.split_at(offset)
}

fn preserve_inserted_line_endings(source: &str, updated: &str) -> String {
    let line_ending = source_line_ending(source);
    if line_ending == "\n" {
        return updated.to_string();
    }

    let (prefix, _source_end, updated_end) = string_diff_bounds(source, updated);
    let changed = &updated[prefix..updated_end];
    let changed = replace_bare_lf(changed, line_ending);
    format!(
        "{}{}{}",
        &updated[..prefix],
        changed,
        &updated[updated_end..]
    )
}

fn source_line_ending(text: &str) -> &'static str {
    let bytes = text.as_bytes();
    if let Some(index) = bytes.iter().position(|byte| *byte == b'\n') {
        if index > 0 && bytes[index - 1] == b'\r' {
            "\r\n"
        } else {
            "\n"
        }
    } else if bytes.contains(&b'\r') {
        "\r"
    } else {
        "\n"
    }
}

fn replace_bare_lf(text: &str, line_ending: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut previous_was_cr = false;
    for character in text.chars() {
        if character == '\n' && !previous_was_cr {
            output.push_str(line_ending);
        } else {
            output.push(character);
        }
        previous_was_cr = character == '\r';
    }
    output
}

fn string_diff_bounds(before: &str, after: &str) -> (usize, usize, usize) {
    let mut prefix = common_prefix_len(before.as_bytes(), after.as_bytes());
    while prefix > 0 && (!before.is_char_boundary(prefix) || !after.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let max_suffix = before.len().min(after.len()).saturating_sub(prefix);
    let mut suffix = common_suffix_len(&before.as_bytes()[prefix..], &after.as_bytes()[prefix..])
        .min(max_suffix);
    while suffix > 0
        && (!before.is_char_boundary(before.len() - suffix)
            || !after.is_char_boundary(after.len() - suffix))
    {
        suffix -= 1;
    }
    (prefix, before.len() - suffix, after.len() - suffix)
}

fn byte_diff(path: &Path, before: &[u8], after: &[u8]) -> RegistrationDiff {
    let prefix = common_prefix_len(before, after);
    let suffix = common_suffix_len(&before[prefix..], &after[prefix..]);
    let before_end = before.len() - suffix;
    let after_end = after.len() - suffix;
    RegistrationDiff {
        path: path.to_path_buf(),
        byte_range: prefix..before_end,
        before: before[prefix..before_end].to_vec(),
        after: after[prefix..after_end].to_vec(),
    }
}

fn common_prefix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

fn common_suffix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(left, right)| left == right)
        .count()
}

fn bytes_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitFailpoint {
    AfterObjectFiles,
    AfterRegistrationBackup,
    PostWriteValidation,
}

#[cfg(test)]
#[derive(Clone)]
struct RegistrationLockPause {
    acquired: Arc<Barrier>,
    release: Arc<Barrier>,
}

#[cfg(test)]
thread_local! {
    static TEST_FAILPOINT: Cell<Option<CommitFailpoint>> = const { Cell::new(None) };
    static TEST_REGISTRATION_LOCK_PAUSE: RefCell<Option<RegistrationLockPause>> = const { RefCell::new(None) };
    static TEST_REGISTRATION_LOCK_CONTENDED: RefCell<Option<Sender<()>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_commit_failpoint<T>(
    failpoint: CommitFailpoint,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<CommitFailpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_FAILPOINT.with(|slot| slot.set(self.0));
        }
    }

    let previous = TEST_FAILPOINT.with(|slot| slot.replace(Some(failpoint)));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_registration_lock_pause<T>(
    acquired: Arc<Barrier>,
    release: Arc<Barrier>,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RegistrationLockPause>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_REGISTRATION_LOCK_PAUSE.with(|slot| slot.replace(self.0.take()));
        }
    }

    let pause = RegistrationLockPause { acquired, release };
    let previous = TEST_REGISTRATION_LOCK_PAUSE.with(|slot| slot.replace(Some(pause)));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_registration_lock_contention_signal<T>(
    sender: Sender<()>,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Sender<()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_REGISTRATION_LOCK_CONTENDED.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous = TEST_REGISTRATION_LOCK_CONTENDED.with(|slot| slot.replace(Some(sender)));
    let _reset = Reset(previous);
    action()
}

fn pause_after_registration_locks() {
    #[cfg(test)]
    TEST_REGISTRATION_LOCK_PAUSE.with(|slot| {
        if let Some(pause) = slot.borrow().clone() {
            pause.acquired.wait();
            pause.release.wait();
        }
    });
}

fn signal_registration_lock_contention() {
    #[cfg(test)]
    TEST_REGISTRATION_LOCK_CONTENDED.with(|slot| {
        if let Some(sender) = slot.borrow().as_ref() {
            let _ = sender.send(());
        }
    });
}

fn failpoint_after_object_files() -> Result<(), String> {
    #[cfg(test)]
    if TEST_FAILPOINT.with(|slot| slot.get()) == Some(CommitFailpoint::AfterObjectFiles) {
        return Err("injected compile transaction failure after object files".to_string());
    }
    Ok(())
}

fn failpoint_after_registration_backup() -> Result<(), String> {
    #[cfg(test)]
    if TEST_FAILPOINT.with(|slot| slot.get()) == Some(CommitFailpoint::AfterRegistrationBackup) {
        return Err("injected compile transaction failure after registration backup".to_string());
    }
    Ok(())
}

fn failpoint_post_write_validation() -> Result<(), String> {
    #[cfg(test)]
    if TEST_FAILPOINT.with(|slot| slot.get()) == Some(CommitFailpoint::PostWriteValidation) {
        return Err("injected compile transaction post-write validation failure".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-compile-transaction-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temporary root must be created");
        root
    }

    fn configuration_bytes() -> Vec<u8> {
        let text = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "xmlns:cfg=\"urn:kept-only-as-qname\" xsi:type=\"cfg:MetaDataObject\">\r\n",
            "\t<Configuration>\r\n",
            "\t\t<ChildObjects>\r\n",
            "\t\t\t<Catalog>Items</Catalog>\r\n",
            "\t\t</ChildObjects>\r\n",
            "\t</Configuration>\r\n",
            "</MetaDataObject><!--tail stays exact-->"
        );
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    fn assert_no_bare_lf(bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                assert!(index > 0 && bytes[index - 1] == b'\r', "bare LF at {index}");
            }
        }
    }

    fn transaction_debris(root: &Path) -> Vec<PathBuf> {
        fn visit(path: &Path, result: &mut Vec<PathBuf>) {
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.contains(".unica-stage-") || name.contains(".unica-backup-")
                    })
                {
                    result.push(path.clone());
                }
                if path.is_dir() {
                    visit(&path, result);
                }
            }
        }

        let mut result = Vec::new();
        visit(root, &mut result);
        result
    }

    #[test]
    fn canonical_registrations_accumulate_and_preserve_source_bytes() {
        let root = temp_root("canonical");
        let config = root.join("Configuration.xml");
        let original = configuration_bytes();
        fs::write(&config, &original).expect("fixture must be written");
        let mut transaction = CompileTransaction::new();

        assert_eq!(
            transaction
                .register_canonical_child(&config, "Role", "Reader")
                .expect("role registration must plan"),
            RegistrationStatus::Added
        );
        assert_eq!(
            transaction
                .register_canonical_child(&config, "Subsystem", "Core")
                .expect("subsystem registration must plan"),
            RegistrationStatus::Added
        );
        assert_eq!(
            transaction
                .register_canonical_child(&config, "Role", "Reader")
                .expect("duplicate must be detected"),
            RegistrationStatus::AlreadyPresent
        );

        let diffs = transaction.registration_diffs();
        assert_eq!(diffs.len(), 1);
        let diff = &diffs[0];
        let mut reconstructed = original.clone();
        reconstructed.splice(diff.byte_range.clone(), diff.after.clone());
        assert_eq!(transaction.planned_updated_paths(), vec![config.clone()]);
        assert!(transaction.dry_run_changes()[0].starts_with("would update "));
        let preview = transaction.dry_run_stdout();
        assert!(preview.contains("@@ bytes"), "{preview}");
        assert!(preview.contains("<Role>Reader</Role>\\r\\n"), "{preview}");
        assert!(preview.contains("after-hex"), "{preview}");

        let report = transaction.commit().expect("transaction must commit");
        assert!(report.created.is_empty());
        assert_eq!(report.updated, vec![config.clone()]);
        assert!(report.cleanup_warnings.is_empty());
        let actual = fs::read(&config).expect("configuration must remain readable");
        assert_eq!(actual, reconstructed);
        assert!(actual.starts_with(UTF8_BOM));
        assert_no_bare_lf(&actual);
        let text = String::from_utf8(actual).expect("configuration must be UTF-8");
        assert!(text.contains("xmlns:cfg=\"urn:kept-only-as-qname\""));
        assert!(text.contains("xsi:type=\"cfg:MetaDataObject\""));
        assert!(text.ends_with("</MetaDataObject><!--tail stays exact-->"));
        let subsystem = text.find("<Subsystem>Core</Subsystem>").unwrap();
        let role = text.find("<Role>Reader</Role>").unwrap();
        let catalog = text.find("<Catalog>Items</Catalog>").unwrap();
        assert!(subsystem < role && role < catalog);
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn missing_registration_target_is_explicit_and_does_not_add_an_update() {
        let root = temp_root("missing-target");
        let mut transaction = CompileTransaction::new();
        assert_eq!(
            transaction
                .register_canonical_child(root.join("Configuration.xml"), "Role", "Reader")
                .expect("missing target is an allowed status"),
            RegistrationStatus::MissingTarget
        );
        assert!(transaction.is_empty());
        assert!(transaction.registration_diffs().is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn self_closing_registration_keeps_a_bom_free_lf_source_bom_free() {
        let root = temp_root("bom-free-self-closing");
        let config = root.join("Configuration.xml");
        let original = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>"
        );
        fs::write(&config, original).expect("fixture must be written");
        let mut transaction = CompileTransaction::new();

        assert_eq!(
            transaction
                .register_canonical_child(&config, "Role", "Reader")
                .expect("registration must plan"),
            RegistrationStatus::Added
        );
        transaction.commit().expect("transaction must commit");

        let actual = fs::read(&config).expect("configuration must remain readable");
        assert!(!actual.starts_with(UTF8_BOM));
        assert!(String::from_utf8(actual)
            .unwrap()
            .contains("<ChildObjects>\n\t<Role>Reader</Role>\n</ChildObjects>"));
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn appended_registration_preserves_cr_only_line_boundaries() {
        let root = temp_root("cr-only-append");
        let config = root.join("Configuration.xml");
        fs::write(
            &config,
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r",
                "<MetaDataObject>\r",
                "\t<Configuration>\r",
                "\t\t<ChildObjects>\r",
                "\t\t\t<Catalog>Items</Catalog>\r",
                "\t\t</ChildObjects>\r",
                "\t</Configuration>\r",
                "</MetaDataObject>"
            ),
        )
        .expect("fixture must be written");
        let mut transaction = CompileTransaction::new();

        assert_eq!(
            transaction
                .register_canonical_child(&config, "Document", "Orders")
                .expect("registration must plan"),
            RegistrationStatus::Added
        );
        transaction.commit().expect("transaction must commit");

        let actual = String::from_utf8(fs::read(&config).expect("configuration must be readable"))
            .expect("configuration must remain UTF-8");
        assert!(
            actual.contains(concat!(
                "\t\t\t<Catalog>Items</Catalog>\r",
                "\t\t\t<Document>Orders</Document>\r",
                "\t\t</ChildObjects>"
            )),
            "{actual:?}"
        );
        assert!(!actual.contains("\r\r\t\t</ChildObjects>"), "{actual:?}");
        assert!(!actual.contains('\n'));
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn already_present_registration_commits_as_a_byte_for_byte_noop() {
        let root = temp_root("registration-noop");
        let config = root.join("Configuration.xml");
        let original = concat!(
            "<?xml version=\"1.0\"?>\r\n",
            "<MetaDataObject><Configuration><ChildObjects>\r\n",
            "\t<Role>Reader</Role>\r\n",
            "</ChildObjects></Configuration></MetaDataObject>"
        )
        .as_bytes()
        .to_vec();
        fs::write(&config, &original).expect("fixture must be written");
        let mut transaction = CompileTransaction::new();

        assert_eq!(
            transaction
                .register_canonical_child(&config, "Role", "Reader")
                .expect("duplicate registration must plan"),
            RegistrationStatus::AlreadyPresent
        );
        assert!(transaction.is_empty());
        let report = transaction.commit().expect("no-op transaction must commit");

        assert!(report.created.is_empty());
        assert!(report.updated.is_empty());
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn existing_target_without_child_objects_is_rejected_without_mutation() {
        let root = temp_root("missing-child-objects");
        let config = root.join("Configuration.xml");
        let original = b"<?xml version=\"1.0\"?><MetaDataObject><Configuration/></MetaDataObject>";
        fs::write(&config, original).expect("fixture must be written");
        let mut transaction = CompileTransaction::new();

        let error = transaction
            .register_canonical_child(&config, "Role", "Reader")
            .expect_err("missing ChildObjects must fail");

        assert!(error.contains("No <ChildObjects>"), "{error}");
        assert_eq!(fs::read(&config).unwrap(), original);
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn commit_creates_bom_text_and_updates_registration_once() {
        let root = temp_root("success");
        let config = root.join("Configuration.xml");
        fs::write(&config, configuration_bytes()).expect("fixture must be written");
        let object = root.join("Roles/Reader.xml");
        let rights = root.join("Roles/Reader/Ext/Rights.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(
                &object,
                "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
            )
            .unwrap();
        transaction
            .create_utf8_bom_text(
                &rights,
                "<?xml version=\"1.0\"?><Rights xmlns=\"http://v8.1c.ru/8.2/roles\"/>",
            )
            .unwrap();
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .unwrap();

        let report = transaction.commit().expect("transaction must commit");

        assert_eq!(report.created, vec![object.clone(), rights.clone()]);
        assert_eq!(report.updated, vec![config.clone()]);
        assert!(fs::read(&object).unwrap().starts_with(UTF8_BOM));
        assert!(fs::read(&rights).unwrap().starts_with(UTF8_BOM));
        assert!(fs::read_to_string(&config)
            .unwrap()
            .contains("<Role>Reader</Role>"));
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn after_object_files_failure_removes_files_and_created_directories() {
        let root = temp_root("rollback-objects");
        let config = root.join("Configuration.xml");
        let original = configuration_bytes();
        fs::write(&config, &original).expect("fixture must be written");
        let object = root.join("Deep/Roles/Reader.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(
                &object,
                "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
            )
            .unwrap();
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .unwrap();

        let error =
            with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || transaction.commit())
                .expect_err("failpoint must abort commit");

        assert!(error.contains("after object files"), "{error}");
        assert!(!object.exists());
        assert!(!root.join("Deep").exists());
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn post_write_validation_failure_restores_exact_registration_bytes() {
        let root = temp_root("rollback-validation");
        let config = root.join("Configuration.xml");
        let original = configuration_bytes();
        fs::write(&config, &original).expect("fixture must be written");
        let object = root.join("Roles/Reader.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(
                &object,
                "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
            )
            .unwrap();
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .unwrap();

        let error = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            transaction.commit()
        })
        .expect_err("failpoint must abort commit");

        assert!(error.contains("post-write validation"), "{error}");
        assert!(!object.exists());
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn after_registration_backup_failure_restores_exact_bytes_and_removes_debris() {
        let root = temp_root("rollback-registration-backup");
        let config = root.join("Configuration.xml");
        let original = configuration_bytes();
        fs::write(&config, &original).expect("fixture must be written");
        let object = root.join("Roles/Reader.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(
                &object,
                "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
            )
            .unwrap();
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .unwrap();

        let error = with_commit_failpoint(CommitFailpoint::AfterRegistrationBackup, || {
            transaction.commit()
        })
        .expect_err("failpoint must abort between registration renames");

        assert!(error.contains("after registration backup"), "{error}");
        assert!(!object.exists());
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(transaction_debris(&root).is_empty());
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("unica-compile.lock")
        }));
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn backup_reservation_retries_without_clobbering_an_occupied_candidate() {
        let root = temp_root("backup-reservation-collision");
        let target = root.join("Configuration.xml");
        let occupied = root.join("occupied-backup");
        let available = root.join("available-backup");
        fs::write(&occupied, b"must remain exact").expect("collision fixture must be written");
        let mut candidates = vec![occupied.clone(), available.clone()].into_iter();

        let reservation = reserve_backup_with(&target, || {
            candidates
                .next()
                .expect("reservation should need only one retry")
        })
        .expect("second candidate must reserve successfully");

        assert_eq!(fs::read(&occupied).unwrap(), b"must remain exact");
        assert_eq!(reservation.directory, available);
        assert!(reservation.directory.is_dir());
        assert!(!reservation.path.exists());
        fs::remove_dir(&reservation.directory).expect("reservation must be removable");
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn concurrent_registration_commits_serialize_before_preflight() {
        let root = temp_root("concurrent-registration");
        let config = root.join("Configuration.xml");
        fs::write(&config, configuration_bytes()).expect("fixture must be written");
        let object_a = root.join("Roles/ReaderA.xml");
        let object_b = root.join("Roles/ReaderB.xml");

        let mut transaction_a = CompileTransaction::new();
        transaction_a
            .create_utf8_bom_text(
                &object_a,
                "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
            )
            .unwrap();
        transaction_a
            .register_canonical_child(&config, "Role", "ReaderA")
            .unwrap();

        let mut transaction_b = CompileTransaction::new();
        transaction_b
            .create_utf8_bom_text(
                &object_b,
                "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
            )
            .unwrap();
        transaction_b
            .register_canonical_child(&config, "Role", "ReaderB")
            .unwrap();

        let acquired = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let acquired_by_a = acquired.clone();
        let release_a = release.clone();
        let thread_a = thread::spawn(move || {
            with_registration_lock_pause(acquired_by_a, release_a, || transaction_a.commit())
        });
        acquired.wait();

        let (contended_sender, contended_receiver) = mpsc::channel();
        let thread_b = thread::spawn(move || {
            with_registration_lock_contention_signal(contended_sender, || transaction_b.commit())
        });
        let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
        release.wait();

        let result_a = thread_a.join().expect("first commit thread must not panic");
        let result_b = thread_b
            .join()
            .expect("second commit thread must not panic");
        contention.expect("second thread must contend on the in-process registration lock");
        let report_a = result_a.expect("first transaction must commit");
        let error_b = result_b.expect_err("stale second plan must fail after acquiring the lock");

        assert_eq!(report_a.created, vec![object_a.clone()]);
        assert!(error_b.contains("changed after planning"), "{error_b}");
        assert!(object_a.is_file());
        assert!(!object_b.exists());
        let actual = fs::read_to_string(&config).unwrap();
        assert!(actual.contains("<Role>ReaderA</Role>"));
        assert!(!actual.contains("<Role>ReaderB</Role>"));
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn create_only_collision_is_rejected_before_publication() {
        let root = temp_root("collision");
        let target = root.join("existing.txt");
        fs::write(&target, b"original").expect("fixture must be written");
        let mut transaction = CompileTransaction::new();

        let error = transaction
            .create_text(&target, "replacement")
            .expect_err("existing target must be rejected");

        assert!(error.contains("create-only"), "{error}");
        assert_eq!(fs::read(&target).unwrap(), b"original");
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_targets_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = temp_root("symlink");
        let real = root.join("real.xml");
        let link = root.join("Configuration.xml");
        fs::write(&real, configuration_bytes()).expect("fixture must be written");
        symlink(&real, &link).expect("symlink must be created");
        let mut transaction = CompileTransaction::new();

        let error = transaction
            .register_canonical_child(&link, "Role", "Reader")
            .expect_err("symlink registration target must be rejected");

        assert!(error.contains("symbolic link"), "{error}");
        assert_eq!(fs::read(&real).unwrap(), configuration_bytes());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }
}

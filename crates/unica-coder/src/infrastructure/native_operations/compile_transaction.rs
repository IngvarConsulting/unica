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

use roxmltree::Document;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::infrastructure::platform::filesystem::{
    prepare_file_for_removal, replace_file_atomically, PortablePermissions,
};

#[cfg(test)]
use std::cell::{Cell, RefCell};

#[cfg(test)]
use std::sync::{Arc, Barrier};

use super::cf::cf_edit_add_child_object_text;
use super::single_file_publisher::{
    cleanup_publication_artifact, prepare, with_publication_locks, write_exact_new_file,
    CleanupWarning, PreparedCreate, PreparedPublication, PreparedReplace, PublicationLockToken,
    PublishError, PublishErrorKind, PublishMode, PublishRequest,
};

#[cfg(test)]
use super::single_file_publisher::{
    with_publication_lock_contention_signal, with_publication_lock_pause,
};

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
static RECOVERY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

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
    /// They are surfaced so a caller can report an orphaned recovery copy explicitly.
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
    original: Vec<u8>,
    updated: Vec<u8>,
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
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let kind = if metadata.file_type().is_symlink() {
                    "symbolic link"
                } else if metadata.is_dir() {
                    "directory"
                } else {
                    "existing file"
                };
                return Err(format!(
                    "create-only compile target is already a {kind}: {}",
                    path.display()
                ));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect create-only target {}: {error}",
                    path.display()
                ));
            }
        }
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
            self.registrations.insert(
                target.clone(),
                PlannedRegistration {
                    path: target.clone(),
                    updated: original.clone(),
                    original,
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
        let mut state = PublishState::default();
        self.semantic_preflight()?;

        for create in &self.creates {
            if let Err(error) = ensure_parent_directories(&create.path, &mut state.created_dirs) {
                let cleanup_errors = cleanup_created_directories(&mut state.created_dirs);
                return Err(with_cleanup_diagnostics(error, cleanup_errors));
            }
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
            if let Err(error) =
                ensure_parent_directories(&registration.path, &mut state.created_dirs)
            {
                let cleanup_errors = cleanup_created_directories(&mut state.created_dirs);
                return Err(with_cleanup_diagnostics(error, cleanup_errors));
            }
        }

        let mut targets = self
            .creates
            .iter()
            .map(|create| create.path.clone())
            .collect::<Vec<_>>();
        targets.extend(
            self.registrations
                .values()
                .filter(|registration| registration.changed())
                .map(|registration| registration.path.clone()),
        );

        match with_publication_locks(&targets, |lock| self.commit_locked(lock, &mut state)) {
            Ok(result) => result,
            Err(error) => {
                let primary = adapt_publish_error(&error, PublicationRole::Transaction);
                record_publish_error_cleanup(&mut state, &error);
                let mut cleanup_errors = retry_warned_artifacts(&mut state);
                cleanup_errors.extend(cleanup_created_directories(&mut state.created_dirs));
                cleanup_errors.extend(std::mem::take(&mut state.cleanup_warnings));
                Err(with_cleanup_diagnostics(primary, cleanup_errors))
            }
        }
    }

    fn commit_locked<'request, 'lock, 'scope>(
        &'request self,
        lock: &'lock PublicationLockToken<'scope>,
        state: &mut PublishState,
    ) -> Result<CommitReport, String> {
        let mut prepared_creates: VecDeque<(
            &'request PlannedCreate,
            PreparedCreate<'request, 'lock, 'scope>,
        )> = VecDeque::new();
        let mut prepared_registrations: VecDeque<(
            &'request PlannedRegistration,
            PreparedReplace<'request, 'lock, 'scope>,
        )> = VecDeque::new();

        let operation = (|| -> Result<CommitReport, String> {
            for create in &self.creates {
                let publication = prepare(
                    lock,
                    PublishRequest {
                        target: &create.path,
                        replacement: &create.bytes,
                        mode: PublishMode::CreateOnly,
                    },
                )
                .map_err(|error| {
                    let message = adapt_publish_error(&error, PublicationRole::Create);
                    record_publish_error_cleanup(state, &error);
                    message
                })?;
                match publication {
                    PreparedPublication::Create(prepared) => {
                        prepared_creates.push_back((create, prepared));
                    }
                    PreparedPublication::Replace(prepared) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(format!(
                            "create-only publication prepared an invalid state for {}",
                            create.path.display()
                        ));
                    }
                    PreparedPublication::Unchanged => {
                        return Err(format!(
                            "create-only publication prepared an invalid state for {}",
                            create.path.display()
                        ));
                    }
                }
            }

            for registration in self.registrations.values().filter(|item| item.changed()) {
                let publication = prepare(
                    lock,
                    PublishRequest {
                        target: &registration.path,
                        replacement: &registration.updated,
                        mode: PublishMode::ReplaceExisting {
                            expected_preimage: &registration.original,
                        },
                    },
                )
                .map_err(|error| {
                    let message = adapt_publish_error(&error, PublicationRole::Registration);
                    record_publish_error_cleanup(state, &error);
                    message
                })?;
                match publication {
                    PreparedPublication::Replace(prepared) => {
                        prepared_registrations.push_back((registration, prepared));
                    }
                    PreparedPublication::Create(prepared) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(format!(
                            "changed registration prepared an invalid state for {}",
                            registration.path.display()
                        ));
                    }
                    PreparedPublication::Unchanged => {
                        return Err(format!(
                            "changed registration prepared an invalid state for {}",
                            registration.path.display()
                        ));
                    }
                }
            }

            while let Some((create, prepared)) = prepared_creates.pop_front() {
                let report = prepared.commit().map_err(|error| {
                    let message = adapt_publish_error(&error, PublicationRole::Create);
                    record_publish_error_cleanup(state, &error);
                    message
                })?;
                record_cleanup_warnings(state, report.cleanup_warnings);
                state.created_paths.push(create.path.clone());
            }

            failpoint_after_object_files()?;

            while let Some((registration, prepared)) = prepared_registrations.pop_front() {
                let permissions = prepared.portable_permissions().clone();
                let mut recovery = match reserve_recovery(&registration.path) {
                    Ok(recovery) => recovery,
                    Err(error) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(error);
                    }
                };
                if let Err(error) =
                    write_exact_new_file(&recovery.path, &registration.original, &permissions)
                {
                    let message = adapt_publish_error(&error, PublicationRole::Recovery);
                    record_publish_error_cleanup(state, &error);
                    record_cleanup_warnings(state, prepared.discard());
                    record_cleanup_strings(state, recovery.cleanup());
                    return Err(message);
                }

                pause_after_registration_recovery();
                if let Err(error) = failpoint_after_registration_backup() {
                    record_cleanup_warnings(state, prepared.discard());
                    record_cleanup_strings(state, recovery.cleanup());
                    return Err(error);
                }

                let report = match prepared.commit() {
                    Ok(report) => report,
                    Err(error) => {
                        let message = adapt_publish_error(&error, PublicationRole::Registration);
                        record_publish_error_cleanup(state, &error);
                        record_cleanup_strings(state, recovery.cleanup());
                        return Err(message);
                    }
                };
                record_cleanup_warnings(state, report.cleanup_warnings);
                state.published_registrations.push(recovery.into_published(
                    registration.path.clone(),
                    registration.original.clone(),
                    permissions,
                ));
            }

            self.post_validate()?;
            failpoint_post_write_validation()?;

            Ok(CommitReport {
                created: self.planned_created_paths(),
                updated: self.planned_updated_paths(),
                cleanup_warnings: Vec::new(),
            })
        })();

        match operation {
            Ok(mut report) => {
                debug_assert!(prepared_creates.is_empty());
                debug_assert!(prepared_registrations.is_empty());
                finalize_success(state);
                report.cleanup_warnings = std::mem::take(&mut state.cleanup_warnings);
                Ok(report)
            }
            Err(primary) => {
                discard_prepared(state, &mut prepared_creates, &mut prepared_registrations);
                let mut rollback_errors = rollback(state);
                rollback_errors.extend(std::mem::take(&mut state.cleanup_warnings));
                Err(with_rollback_diagnostics(primary, rollback_errors))
            }
        }
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

    fn semantic_preflight(&self) -> Result<(), String> {
        for create in &self.creates {
            validate_xml_when_applicable(&create.path, &create.bytes)?;
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
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
struct PublishedRegistration {
    target: PathBuf,
    recovery: PathBuf,
    recovery_directory: PathBuf,
    original: Vec<u8>,
    original_permissions: PortablePermissions,
}

#[derive(Debug)]
struct PendingRecovery {
    directory: PathBuf,
    path: PathBuf,
    armed: bool,
}

impl PendingRecovery {
    fn cleanup(&mut self) -> Vec<String> {
        if !self.armed {
            return Vec::new();
        }

        if let Err(warning) = cleanup_publication_artifact(&self.path) {
            return vec![format!(
                "failed to remove pending registration recovery {warning}"
            )];
        }
        match fs::remove_dir(&self.directory) {
            Ok(()) => {
                self.armed = false;
                Vec::new()
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.armed = false;
                Vec::new()
            }
            Err(error) => vec![format!(
                "failed to remove pending registration recovery directory {}: {error}",
                self.directory.display()
            )],
        }
    }

    fn into_published(
        mut self,
        target: PathBuf,
        original: Vec<u8>,
        original_permissions: PortablePermissions,
    ) -> PublishedRegistration {
        self.armed = false;
        PublishedRegistration {
            target,
            recovery: self.path.clone(),
            recovery_directory: self.directory.clone(),
            original,
            original_permissions,
        }
    }
}

impl Drop for PendingRecovery {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cleanup();
        }
    }
}

#[derive(Debug, Default)]
struct PublishState {
    created_paths: Vec<PathBuf>,
    published_registrations: Vec<PublishedRegistration>,
    created_dirs: Vec<PathBuf>,
    warned_artifacts: Vec<PathBuf>,
    cleanup_warnings: Vec<String>,
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

fn reserve_recovery(target: &Path) -> Result<PendingRecovery, String> {
    reserve_recovery_with(target, || unique_recovery_directory(target))
}

fn reserve_recovery_with(
    target: &Path,
    mut next_directory: impl FnMut() -> PathBuf,
) -> Result<PendingRecovery, String> {
    for attempt in 1..=16 {
        let directory = next_directory();
        match fs::create_dir(&directory) {
            Ok(()) => {
                return Ok(PendingRecovery {
                    path: directory.join("original"),
                    directory,
                    armed: true,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt < 16 => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve no-clobber recovery for {} at {}: {error}",
                    target.display(),
                    directory.display()
                ));
            }
        }
    }
    Err(format!(
        "failed to reserve no-clobber recovery for {}",
        target.display()
    ))
}

fn unique_recovery_directory(target: &Path) -> PathBuf {
    let parent = usable_parent(target);
    let mut name = OsString::from(".");
    name.push(
        target
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("compile")),
    );
    name.push(format!(
        ".unica-recovery-{}-{}",
        std::process::id(),
        RECOVERY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
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

#[derive(Debug, Clone, Copy)]
enum PublicationRole {
    Create,
    Registration,
    Recovery,
    Transaction,
}

fn adapt_publish_error(error: &PublishError, role: PublicationRole) -> String {
    match error.kind() {
        PublishErrorKind::StalePreimage { target } => match role {
            PublicationRole::Registration => format!(
                "registration target changed after planning: {}",
                target.display()
            ),
            PublicationRole::Create | PublicationRole::Recovery | PublicationRole::Transaction => {
                error.to_string()
            }
        },
        PublishErrorKind::MetadataChanged { target } => match role {
            PublicationRole::Registration => format!(
                "registration target metadata changed after planning: {}",
                target.display()
            ),
            PublicationRole::Create | PublicationRole::Recovery | PublicationRole::Transaction => {
                error.to_string()
            }
        },
        PublishErrorKind::MissingTarget { target } => match role {
            PublicationRole::Registration => format!(
                "registration target disappeared before commit: {}",
                target.display()
            ),
            PublicationRole::Create | PublicationRole::Recovery | PublicationRole::Transaction => {
                error.to_string()
            }
        },
        PublishErrorKind::InvalidTarget { .. }
        | PublishErrorKind::AlreadyExists { .. }
        | PublishErrorKind::LinkOrReparsePoint { .. }
        | PublishErrorKind::NonRegular { .. }
        | PublishErrorKind::ReadOnly { .. }
        | PublishErrorKind::MultipleHardLinks { .. }
        | PublishErrorKind::StageCollisionsExhausted { .. }
        | PublishErrorKind::Io { .. } => error.to_string(),
    }
}

fn record_publish_error_cleanup(state: &mut PublishState, error: &PublishError) {
    record_cleanup_warnings(state, error.cleanup_warnings().iter().cloned());
}

fn record_cleanup_warnings(
    state: &mut PublishState,
    warnings: impl IntoIterator<Item = CleanupWarning>,
) {
    for warning in warnings {
        if !state.warned_artifacts.contains(&warning.path) {
            state.warned_artifacts.push(warning.path.clone());
        }
        state.cleanup_warnings.push(warning.to_string());
    }
}

fn record_cleanup_strings(state: &mut PublishState, warnings: impl IntoIterator<Item = String>) {
    state.cleanup_warnings.extend(warnings);
}

fn discard_prepared<'request, 'lock, 'scope>(
    state: &mut PublishState,
    creates: &mut VecDeque<(
        &'request PlannedCreate,
        PreparedCreate<'request, 'lock, 'scope>,
    )>,
    registrations: &mut VecDeque<(
        &'request PlannedRegistration,
        PreparedReplace<'request, 'lock, 'scope>,
    )>,
) {
    while let Some((_create_plan, prepared)) = creates.pop_front() {
        record_cleanup_warnings(state, prepared.discard());
    }
    while let Some((_registration_plan, prepared)) = registrations.pop_front() {
        record_cleanup_warnings(state, prepared.discard());
    }
}

fn retry_warned_artifacts(state: &mut PublishState) -> Vec<String> {
    std::mem::take(&mut state.warned_artifacts)
        .into_iter()
        .filter_map(|path| {
            cleanup_publication_artifact(&path)
                .err()
                .map(|warning| format!("failed to retry publication cleanup {warning}"))
        })
        .collect()
}

fn cleanup_created_directories(created_dirs: &mut Vec<PathBuf>) -> Vec<String> {
    let mut errors = Vec::new();
    for directory in created_dirs.iter().rev() {
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
    created_dirs.clear();
    errors
}

fn with_cleanup_diagnostics(primary: String, diagnostics: Vec<String>) -> String {
    if diagnostics.is_empty() {
        primary
    } else {
        format!("{primary}; cleanup encountered: {}", diagnostics.join("; "))
    }
}

fn with_rollback_diagnostics(primary: String, diagnostics: Vec<String>) -> String {
    if diagnostics.is_empty() {
        primary
    } else {
        format!(
            "{primary}; rollback encountered: {}",
            diagnostics.join("; ")
        )
    }
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    prepare_file_for_removal(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rollback(state: &mut PublishState) -> Vec<String> {
    let mut errors = Vec::new();

    for published in state.published_registrations.iter().rev() {
        if let Err(error) = replace_file_atomically(&published.recovery, &published.target) {
            errors.push(format!(
                "failed to atomically restore registration {} from {}; recovery is preserved at {}: {error}",
                published.target.display(),
                published.recovery.display(),
                published.recovery.display()
            ));
            continue;
        }

        let bytes_restored = match fs::read(&published.target) {
            Ok(bytes) if bytes == published.original => true,
            Ok(_) => {
                errors.push(format!(
                    "restored registration bytes differ from original: {}",
                    published.target.display()
                ));
                false
            }
            Err(error) => {
                errors.push(format!(
                    "failed to verify restored registration {}: {error}",
                    published.target.display()
                ));
                false
            }
        };
        let permissions_restored = match fs::metadata(&published.target) {
            Ok(metadata) if published.original_permissions.matches(&metadata) => true,
            Ok(_) => {
                errors.push(format!(
                    "restored registration permissions differ from original: {}",
                    published.target.display()
                ));
                false
            }
            Err(error) => {
                errors.push(format!(
                    "failed to verify restored registration permissions {}: {error}",
                    published.target.display()
                ));
                false
            }
        };
        if !bytes_restored || !permissions_restored {
            preserve_recovery_copy(published, &mut errors);
            continue;
        }
        if let Err(error) = fs::remove_dir(&published.recovery_directory) {
            errors.push(format!(
                "failed to remove restored registration recovery directory {}: {error}",
                published.recovery_directory.display()
            ));
            preserve_recovery_copy(published, &mut errors);
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
    errors.extend(retry_warned_artifacts(state));
    errors.extend(cleanup_created_directories(&mut state.created_dirs));
    errors
}

fn preserve_recovery_copy(published: &PublishedRegistration, diagnostics: &mut Vec<String>) {
    if published.recovery.exists() {
        diagnostics.push(format!(
            "registration recovery is preserved at {}",
            published.recovery.display()
        ));
        return;
    }
    match write_exact_new_file(
        &published.recovery,
        &published.original,
        &published.original_permissions,
    ) {
        Ok(()) => diagnostics.push(format!(
            "registration recovery is preserved at {}",
            published.recovery.display()
        )),
        Err(error) => diagnostics.push(format!(
            "failed to preserve recovery copy {} after rollback cleanup failure: {error}",
            published.recovery.display()
        )),
    }
}

fn finalize_success(state: &mut PublishState) {
    for published in &state.published_registrations {
        if let Err(warning) = cleanup_publication_artifact(&published.recovery) {
            state.cleanup_warnings.push(format!(
                "failed to remove registration recovery {warning}; recovery is preserved at {}",
                published.recovery.display()
            ));
            continue;
        }
        if let Err(error) = fs::remove_dir(&published.recovery_directory) {
            state.cleanup_warnings.push(format!(
                "failed to remove registration recovery directory {}: {error}",
                published.recovery_directory.display()
            ));
            let mut preservation_errors = Vec::new();
            preserve_recovery_copy(published, &mut preservation_errors);
            state.cleanup_warnings.extend(preservation_errors);
        }
    }
    let retry_warnings = retry_warned_artifacts(state);
    state.cleanup_warnings.extend(retry_warnings);
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
struct RegistrationRecoveryPause {
    ready: Arc<Barrier>,
    release: Arc<Barrier>,
}

#[cfg(test)]
thread_local! {
    static TEST_FAILPOINT: Cell<Option<CommitFailpoint>> = const { Cell::new(None) };
    static TEST_REGISTRATION_RECOVERY_PAUSE: RefCell<Option<RegistrationRecoveryPause>> = const { RefCell::new(None) };
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
fn with_registration_recovery_pause<T>(
    ready: Arc<Barrier>,
    release: Arc<Barrier>,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RegistrationRecoveryPause>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_REGISTRATION_RECOVERY_PAUSE.with(|slot| slot.replace(self.0.take()));
        }
    }

    let pause = RegistrationRecoveryPause { ready, release };
    let previous = TEST_REGISTRATION_RECOVERY_PAUSE.with(|slot| slot.replace(Some(pause)));
    let _reset = Reset(previous);
    action()
}

fn pause_after_registration_recovery() {
    #[cfg(test)]
    let pause = TEST_REGISTRATION_RECOVERY_PAUSE.with(|slot| slot.borrow_mut().take());
    #[cfg(test)]
    if let Some(pause) = pause {
        pause.ready.wait();
        pause.release.wait();
    }
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
    use crate::application::UnicaApplication;
    use crate::infrastructure::platform::testing;
    use serde_json::{Map, Value};
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

    fn public_compile_workspace(name: &str) -> PathBuf {
        let root = temp_root(name);
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.17">
  <Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
    <Properties>
      <Name>Demo</Name>
      <Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Demo</v8:content></v8:item></Synonym>
      <Version>1.0</Version>
      <Vendor>Vendor</Vendor>
      <CompatibilityMode>Version8_3_24</CompatibilityMode>
      <DefaultRunMode>ManagedApplication</DefaultRunMode>
      <ScriptVariant>Russian</ScriptVariant>
      <DefaultLanguage>Russian</DefaultLanguage>
      <DataLockControlMode>Managed</DataLockControlMode>
      <ModalityUseMode>DontUse</ModalityUseMode>
      <InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
    </Properties>
    <ChildObjects><Catalog>Items</Catalog></ChildObjects>
  </Configuration>
</MetaDataObject>"#,
        )
        .unwrap();
        root
    }

    fn call_meta_compile(
        workspace: &Path,
        json_path: &Path,
    ) -> crate::application::OperationResult {
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));
        UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
            .unwrap()
    }

    #[test]
    fn public_meta_compile_batch_rolls_back_after_object_files_failure() {
        let root = public_compile_workspace("public-meta-batch-rollback");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let config_path = src.join("Configuration.xml");
        let config_before = fs::read(&config_path).unwrap();
        let json_path = workspace.join("batch.json");
        fs::write(
            &json_path,
            r#"[
  {"type":"CommonModule","name":"RollbackService"},
  {"type":"Catalog","name":"RollbackCatalog"}
]"#,
        )
        .unwrap();

        let result = with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            call_meta_compile(&workspace, &json_path)
        });

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("after object files"));
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert!(!src.join("CommonModules").exists());
        assert!(!src.join("Catalogs").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_role_compile_rolls_back_after_object_files_failure() {
        let root = public_compile_workspace("public-role-rollback");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let config_path = src.join("Configuration.xml");
        let config_before = fs::read(&config_path).unwrap();
        let role_json = workspace.join("rollback-user.json");
        fs::write(
            &role_json,
            r#"{
  "name": "RollbackUser",
  "synonym": "Rollback user",
  "objects": ["Catalog.Items: @view"]
}"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "JsonPath".to_string(),
            Value::String(role_json.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));

        let result = with_commit_failpoint(CommitFailpoint::AfterObjectFiles, || {
            UnicaApplication::new()
                .call_tool("unica.role.compile", &args)
                .unwrap()
        });

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("after object files"));
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert!(!src.join("Roles").exists());

        let _ = fs::remove_dir_all(root);
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
                        name.contains(".unica-stage-")
                            || name.contains(".unica-backup-")
                            || name.contains(".unica-recovery-")
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
    fn post_validation_rollback_restores_bytes_and_unix_mode_0600() {
        let root = temp_root("rollback-mode");
        let config = root.join("Configuration.xml");
        let original = configuration_bytes();
        fs::write(&config, &original).expect("fixture must be written");
        if !testing::set_unix_mode_for_test(&config, 0o600)
            .expect("mode fixture must be configurable")
        {
            fs::remove_dir_all(root).expect("temporary root must be removed");
            return;
        }
        let mut transaction = CompileTransaction::new();
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .expect("registration must plan");

        let error = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            transaction.commit()
        })
        .expect_err("post-validation failpoint must roll the transaction back");

        assert!(error.contains("post-write validation"), "{error}");
        assert_eq!(fs::read(&config).unwrap(), original);
        assert_eq!(
            testing::unix_mode_for_test(&config).expect("mode must remain readable"),
            Some(0o600)
        );
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn registration_target_remains_present_after_backup_preparation() {
        let root = temp_root("recovery-keeps-target-present");
        let config = root.join("Configuration.xml");
        let original = configuration_bytes();
        fs::write(&config, &original).expect("fixture must be written");
        let mut transaction = CompileTransaction::new();
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .expect("registration must plan");

        let recovery_ready = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let recovery_ready_in_commit = Arc::clone(&recovery_ready);
        let release_in_commit = Arc::clone(&release);
        let commit_thread = thread::spawn(move || {
            with_registration_recovery_pause(recovery_ready_in_commit, release_in_commit, || {
                transaction.commit()
            })
        });

        recovery_ready.wait();
        let target_present = fs::symlink_metadata(&config).is_ok();
        let bytes_during_recovery = fs::read(&config);
        release.wait();

        let commit_result = commit_thread.join().expect("commit thread must not panic");
        assert!(
            target_present,
            "the target entry must remain present while recovery is ready"
        );
        assert_eq!(bytes_during_recovery.unwrap(), original);
        let report = commit_result.expect("transaction must commit after the pause");
        assert_eq!(report.updated, vec![config.clone()]);
        assert!(fs::read_to_string(&config)
            .unwrap()
            .contains("<Role>Reader</Role>"));
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn compile_transaction_rejects_readonly_registration_without_partial_creates() {
        let root = temp_root("readonly-preflight");
        let config = root.join("Configuration.xml");
        let original = configuration_bytes();
        fs::write(&config, &original).expect("fixture must be written");
        if !testing::set_unix_mode_for_test(&config, 0o400)
            .expect("mode fixture must be configurable")
        {
            let mut permissions = fs::metadata(&config).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&config, permissions).unwrap();
        }
        let original_mode = testing::unix_mode_for_test(&config).unwrap();
        let object = root.join("Deep/Roles/Reader.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_text(&object, "<Object/>")
            .expect("create must plan");
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .expect("registration must plan");

        let error = transaction
            .commit()
            .expect_err("read-only registration must reject the complete transaction");

        assert!(error.contains("read-only"), "{error}");
        assert_eq!(fs::read(&config).unwrap(), original);
        assert_eq!(testing::unix_mode_for_test(&config).unwrap(), original_mode);
        assert!(!object.exists());
        assert!(!root.join("Deep").exists());
        assert!(transaction_debris(&root).is_empty());
        prepare_file_for_removal(&config).expect("fixture must be removable");
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn compile_transaction_rejects_hard_linked_registration_without_mutation() {
        let root = temp_root("hard-link-preflight");
        let config = root.join("Configuration.xml");
        let alias = root.join("Configuration.alias.xml");
        let original = configuration_bytes();
        fs::write(&config, &original).expect("fixture must be written");
        fs::hard_link(&config, &alias).expect("hard-link fixture must be created");
        let object = root.join("Deep/Roles/Reader.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_text(&object, "<Object/>")
            .expect("create must plan");
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .expect("registration must plan");

        let error = transaction
            .commit()
            .expect_err("hard-linked registration must reject the complete transaction");

        assert!(error.contains("hard links"), "{error}");
        assert_eq!(fs::read(&config).unwrap(), original);
        assert_eq!(fs::read(&alias).unwrap(), original);
        assert_eq!(
            crate::infrastructure::platform::filesystem::hard_link_count(
                &fs::File::open(&config).unwrap()
            )
            .unwrap(),
            2
        );
        assert!(!object.exists());
        assert!(!root.join("Deep").exists());
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn post_validation_failure_rolls_back_two_registrations_and_one_create() {
        let root = temp_root("rollback-two-registrations");
        let config_a = root.join("Configuration.xml");
        let config_b = root.join("Subsystems/Core.xml");
        fs::create_dir_all(config_b.parent().unwrap()).expect("fixture parent must be created");
        let original_a = configuration_bytes();
        let original_b = configuration_bytes();
        fs::write(&config_a, &original_a).expect("first fixture must be written");
        fs::write(&config_b, &original_b).expect("second fixture must be written");
        let modes_supported = testing::set_unix_mode_for_test(&config_a, 0o600)
            .and_then(|supported| {
                testing::set_unix_mode_for_test(&config_b, 0o640)
                    .map(|second_supported| supported && second_supported)
            })
            .expect("mode fixtures must be configurable");
        let object = root.join("Deep/Roles/Reader.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_text(&object, "<Object/>")
            .expect("create must plan");
        transaction
            .register_canonical_child(&config_a, "Role", "Reader")
            .expect("first registration must plan");
        transaction
            .register_canonical_child(&config_b, "Catalog", "Orders")
            .expect("second registration must plan");

        let error = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            transaction.commit()
        })
        .expect_err("post-validation failure must roll every publication back");

        assert!(error.contains("post-write validation"), "{error}");
        assert_eq!(fs::read(&config_a).unwrap(), original_a);
        assert_eq!(fs::read(&config_b).unwrap(), original_b);
        if modes_supported {
            assert_eq!(testing::unix_mode_for_test(&config_a).unwrap(), Some(0o600));
            assert_eq!(testing::unix_mode_for_test(&config_b).unwrap(), Some(0o640));
        }
        assert!(!object.exists());
        assert!(!root.join("Deep").exists());
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
    fn recovery_reservation_retries_without_clobbering_an_occupied_candidate() {
        let root = temp_root("recovery-reservation-collision");
        let target = root.join("Configuration.xml");
        let occupied = root.join("occupied-recovery");
        let available = root.join("available-recovery");
        fs::write(&occupied, b"must remain exact").expect("collision fixture must be written");
        let mut candidates = vec![occupied.clone(), available.clone()].into_iter();

        let mut reservation = reserve_recovery_with(&target, || {
            candidates
                .next()
                .expect("reservation should need only one retry")
        })
        .expect("second candidate must reserve successfully");

        assert_eq!(fs::read(&occupied).unwrap(), b"must remain exact");
        assert_eq!(reservation.directory, available);
        assert!(reservation.directory.is_dir());
        assert!(!reservation.path.exists());
        assert!(reservation.cleanup().is_empty());
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
            with_publication_lock_pause(acquired_by_a, release_a, || transaction_a.commit())
        });
        acquired.wait();

        let (contended_sender, contended_receiver) = mpsc::channel();
        let thread_b = thread::spawn(move || {
            with_publication_lock_contention_signal(contended_sender, || transaction_b.commit())
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
    fn two_create_only_transactions_contend_on_the_shared_target_lock() {
        let root = temp_root("concurrent-create-only");
        let target = root.join("shared.bin");
        let mut transaction_a = CompileTransaction::new();
        transaction_a
            .create_bytes(&target, b"from transaction A".to_vec())
            .expect("first create must plan");
        let mut transaction_b = CompileTransaction::new();
        transaction_b
            .create_bytes(&target, b"from transaction B".to_vec())
            .expect("second create must plan from the same absent preimage");

        let acquired = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let acquired_by_a = Arc::clone(&acquired);
        let release_a = Arc::clone(&release);
        let thread_a = thread::spawn(move || {
            with_publication_lock_pause(acquired_by_a, release_a, || transaction_a.commit())
        });
        acquired.wait();

        let (contended_sender, contended_receiver) = mpsc::channel();
        let thread_b = thread::spawn(move || {
            with_publication_lock_contention_signal(contended_sender, || transaction_b.commit())
        });
        let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
        release.wait();

        let report_a = thread_a
            .join()
            .expect("first commit thread must not panic")
            .expect("first create transaction must commit");
        let error_b = thread_b
            .join()
            .expect("second commit thread must not panic")
            .expect_err("second create transaction must observe the committed target");
        contention.expect("second create transaction must contend on the publisher lock");
        assert_eq!(report_a.created, vec![target.clone()]);
        assert!(error_b.contains("already exists"), "{error_b}");
        assert_eq!(fs::read(&target).unwrap(), b"from transaction A");
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

    #[test]
    fn symlink_targets_are_rejected() {
        let root = temp_root("symlink");
        let real = root.join("real.xml");
        let link = root.join("Configuration.xml");
        fs::write(&real, configuration_bytes()).expect("fixture must be written");
        let Some(symlink) = testing::create_file_symlink_for_test(&real, &link) else {
            fs::remove_dir_all(root).expect("temporary root must be removed");
            return;
        };
        symlink.expect("symlink must be created");
        let mut transaction = CompileTransaction::new();

        let error = transaction
            .register_canonical_child(&link, "Role", "Reader")
            .expect_err("symlink registration target must be rejected");

        assert!(error.contains("symbolic link"), "{error}");
        assert_eq!(fs::read(&real).unwrap(), configuration_bytes());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }
}

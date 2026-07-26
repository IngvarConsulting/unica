//! Failure-atomic publication for the metadata compile writers.
//!
//! The compile families build their files in memory, add them to a single
//! [`CompileTransaction`], and publish the transaction once. New files remain
//! create-only; existing regular files require an exact preimage before atomic
//! replacement. Removal trees are streamed into size/SHA-256 snapshots, reject
//! links, and are moved to same-filesystem recovery locations until the whole
//! transaction has passed post-write validation. Canonical XML registrations
//! are still edited textually through the `cf.edit` registrar.
//!
//! "Failure-atomic" here covers reported I/O and validation errors. It does not
//! claim process-crash or power-loss atomicity; those require a persistent journal
//! and directory-entry synchronization that this transaction does not provide.

use roxmltree::Document;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{ErrorKind, Read};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::infrastructure::platform::filesystem::{
    file_identity, hard_link_count, metadata_is_link_or_reparse_point, prepare_file_for_removal,
    rename_no_replace, FileIdentity, PortablePermissions,
};
use crate::infrastructure::source_roots::normalize_path_identity;

#[cfg(test)]
use std::cell::{Cell, RefCell};

#[cfg(test)]
use crate::infrastructure::platform::filesystem::replace_file_atomically;

#[cfg(test)]
use std::sync::{Arc, Barrier};

use super::cf::cf_edit_add_child_object_text;
use super::single_file_publisher::{
    cleanup_publication_artifact, prepare, with_publication_locks_mode_and_guard_targets,
    write_exact_new_file, CleanupWarning, PreparedCreate, PreparedPublication, PreparedReplace,
    PublicationLockToken, PublicationTreeLockMode, PublishError, PublishErrorKind, PublishMode,
    PublishRequest,
};

#[cfg(test)]
use super::single_file_publisher::{
    with_before_commit_hook, with_publication_lock_contention_signal, with_publication_lock_pause,
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

#[derive(Debug)]
struct PlannedReadGuard {
    path: PathBuf,
    expected_preimage: Vec<u8>,
}

#[derive(Debug)]
struct PlannedRemoval {
    path: PathBuf,
    snapshot: RemovalSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovalSnapshot {
    entries: Vec<RemovalEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovalEntry {
    relative_path: PathBuf,
    kind: RemovalEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemovalEntryKind {
    File { size: u64, sha256: [u8; 32] },
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectoryMembershipSelector {
    /// Direct regular files whose extension is exactly lowercase `xml`.
    XmlFiles,
    /// Direct regular files whose extension is ASCII-case-insensitively `cf`.
    CfFilesAsciiCaseInsensitive,
    /// Every direct regular file or directory. This binds recursive scanner
    /// topology one directory at a time without following links.
    AllDirectEntries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DirectoryTopologyEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DirectoryTopologyEntry {
    pub(crate) name: OsString,
    pub(crate) kind: DirectoryTopologyEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectoryMembershipGuard {
    directory: PathBuf,
    selector: DirectoryMembershipSelector,
    expected_entries: Vec<DirectoryTopologyEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedPathKind {
    Create,
    Registration,
    ReadGuard,
    AbsenceGuard,
    Removal,
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
    registrations: BTreeMap<PathBuf, PlannedRegistration>,
    read_guards: BTreeMap<PathBuf, PlannedReadGuard>,
    absence_guards: BTreeSet<PathBuf>,
    directory_membership_guards: BTreeMap<PathBuf, DirectoryMembershipGuard>,
    removals: Vec<PlannedRemoval>,
    planned_path_identities: BTreeMap<PathBuf, PlannedPathKind>,
}

impl CompileTransaction {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether an existing exact plan already protects this path with the
    /// transaction lock/preimage machinery. Callers use this to avoid adding
    /// a read guard for an owner that the same transaction replaces/removes.
    pub(crate) fn protects_path(&self, path: &Path) -> Result<bool, String> {
        let identity = normalize_transaction_path_identity(path)?;
        Ok(find_overlapping_planned_identity(&self.planned_path_identities, &identity).is_some())
    }

    /// Plan a create-only file with exact bytes.
    pub(crate) fn create_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        let path = path.into();
        let identity = self.reject_duplicate_plan_path(&path)?;
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
        self.record_planned_path(identity, PlannedPathKind::Create);
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

    /// Require an existing owner to retain one exact byte preimage while this
    /// transaction publishes other paths. The owner joins the transaction's
    /// publication lock set but is never staged, replaced, or rolled back.
    pub(crate) fn guard_exact_preimage(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
    ) -> Result<(), String> {
        let path = path.into();
        let identity = self.reject_duplicate_plan_path(&path)?;
        let expected_preimage = expected_preimage.as_ref().to_vec();
        validate_exact_read_guard(&path, &expected_preimage, "while planning")?;
        self.read_guards.insert(
            identity.clone(),
            PlannedReadGuard {
                path,
                expected_preimage,
            },
        );
        self.record_planned_path(identity, PlannedPathKind::ReadGuard);
        Ok(())
    }

    /// Require one exact normalized path to remain absent for the complete
    /// transaction. This binds plans that intentionally tolerate a missing
    /// semantic owner so a concurrently appearing owner cannot be left
    /// unregistered.
    pub(crate) fn guard_path_absent(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let requested_path = path.into();
        let identity = normalize_transaction_path_identity(&requested_path)?;
        if self.absence_guards.contains(&identity) {
            return validate_absence_guard(&identity, "while planning");
        }
        self.reject_duplicate_normalized_plan_path(&identity, &requested_path)?;
        validate_absence_guard(&identity, "while planning")?;
        self.absence_guards.insert(identity.clone());
        self.record_planned_path(identity, PlannedPathKind::AbsenceGuard);
        Ok(())
    }

    /// Bind the exact relevant direct-child listing of a directory to this
    /// transaction. Membership guards conservatively take the global
    /// publication-tree lock exclusively at commit, so another cooperating
    /// Unica writer cannot add or remove a matching entry between checks.
    pub(crate) fn guard_or_verify_directory_membership(
        &mut self,
        directory: impl Into<PathBuf>,
        selector: DirectoryMembershipSelector,
        expected_names: Vec<OsString>,
    ) -> Result<(), String> {
        let expected_entries = expected_names
            .into_iter()
            .map(|name| DirectoryTopologyEntry {
                name,
                kind: DirectoryTopologyEntryKind::File,
            })
            .collect();
        self.guard_or_verify_directory_entries(directory.into(), selector, expected_entries)
    }

    /// Bind the exact direct-child names and filesystem kinds observed by a
    /// recursive scanner. File-to-directory replacement must be detected even
    /// when the entry name itself is unchanged, because the replacement can
    /// introduce an unscanned subtree.
    pub(crate) fn guard_or_verify_directory_topology(
        &mut self,
        directory: impl Into<PathBuf>,
        expected_entries: Vec<DirectoryTopologyEntry>,
    ) -> Result<(), String> {
        self.guard_or_verify_directory_entries(
            directory.into(),
            DirectoryMembershipSelector::AllDirectEntries,
            expected_entries,
        )
    }

    fn guard_or_verify_directory_entries(
        &mut self,
        requested_directory: PathBuf,
        selector: DirectoryMembershipSelector,
        expected_entries: Vec<DirectoryTopologyEntry>,
    ) -> Result<(), String> {
        let expected_entries = normalize_expected_directory_entries(selector, expected_entries)?;
        validate_directory_membership_guard(
            &requested_directory,
            selector,
            &expected_entries,
            "while planning",
        )?;
        let directory = normalize_transaction_path_identity(&requested_directory)?;
        if let Some(existing) = self.directory_membership_guards.get(&directory) {
            if existing.selector == selector && existing.expected_entries == expected_entries {
                return Ok(());
            }
            return Err(format!(
                "directory membership changed while planning: {}",
                requested_directory.display()
            ));
        }
        for (removal, kind) in &self.planned_path_identities {
            if *kind == PlannedPathKind::Removal && directory.starts_with(removal) {
                return Err(format!(
                    "compile transaction cannot guard membership of a directory scheduled for removal: {}",
                    requested_directory.display()
                ));
            }
        }
        self.directory_membership_guards.insert(
            directory.clone(),
            DirectoryMembershipGuard {
                directory,
                selector,
                expected_entries,
            },
        );
        Ok(())
    }

    /// Bind bytes that were read before another plan entry for the same file
    /// was created. A registration already carries an exact preimage, so it
    /// satisfies this guard only when it was derived from those same bytes.
    pub(crate) fn guard_or_verify_exact_preimage(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
    ) -> Result<(), String> {
        let path = path.into();
        let expected_preimage = expected_preimage.as_ref();
        let identity = normalize_transaction_path_identity(&path)?;
        if self.planned_path_identities.get(&identity) == Some(&PlannedPathKind::Registration) {
            let registration = self
                .registrations
                .get(&identity)
                .expect("registered path identity must retain its registration");
            if registration.original == expected_preimage {
                return Ok(());
            }
            return Err(format!(
                "protected path changed while planning: {}",
                path.display()
            ));
        }
        if self.planned_path_identities.get(&identity) == Some(&PlannedPathKind::ReadGuard) {
            let guarded = self
                .read_guards
                .get(&identity)
                .expect("guarded path identity must retain its read guard");
            if guarded.expected_preimage == expected_preimage {
                return Ok(());
            }
            return Err(format!(
                "protected path changed while planning: {}",
                path.display()
            ));
        }
        self.reject_duplicate_normalized_plan_path(&identity, &path)?;
        validate_exact_read_guard(&path, expected_preimage, "while planning")?;
        self.read_guards.insert(
            identity.clone(),
            PlannedReadGuard {
                path,
                expected_preimage: expected_preimage.to_vec(),
            },
        );
        self.record_planned_path(identity, PlannedPathKind::ReadGuard);
        Ok(())
    }

    /// Plan creation or replacement according to the target state observed
    /// now. Existing bytes become the exact preimage required at commit.
    pub(crate) fn create_or_replace_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        replacement: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        let path = path.into();
        let replacement = replacement.into();
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.create_bytes(path, replacement)
            }
            Err(error) => Err(format!(
                "failed to inspect publication target {}: {error}",
                path.display()
            )),
            Ok(metadata) => {
                if metadata_is_link_or_reparse_point(&metadata) {
                    return Err(format!(
                        "publication target must not be a symbolic link or reparse point: {}",
                        path.display()
                    ));
                }
                if !metadata.is_file() {
                    return Err(format!(
                        "publication target is not a regular file: {}",
                        path.display()
                    ));
                }
                let original = fs::read(&path).map_err(|error| {
                    format!(
                        "failed to read publication target {}: {error}",
                        path.display()
                    )
                })?;
                self.replace_bytes(path, &original, replacement)
            }
        }
    }

    /// Plan an exact replacement of an existing regular file. The caller's
    /// preimage is rechecked both while planning and immediately before the
    /// atomic publication, so a concurrent edit is never overwritten.
    pub(crate) fn replace_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
        replacement: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        let path = path.into();
        let identity = self.reject_duplicate_plan_path(&path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "failed to inspect replacement target {}: {error}",
                path.display()
            )
        })?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "replacement target must not be a symbolic link or reparse point: {}",
                path.display()
            ));
        }
        if !metadata.is_file() {
            return Err(format!(
                "replacement target is not a regular file: {}",
                path.display()
            ));
        }
        let original = fs::read(&path).map_err(|error| {
            format!(
                "failed to read replacement target {}: {error}",
                path.display()
            )
        })?;
        if original != expected_preimage.as_ref() {
            return Err(format!(
                "replacement target changed while planning: {}",
                path.display()
            ));
        }
        let updated = replacement.into();
        validate_xml_when_applicable(&path, &updated)?;
        self.registrations.insert(
            identity.clone(),
            PlannedRegistration {
                path,
                original,
                updated,
            },
        );
        self.record_planned_path(identity, PlannedPathKind::Registration);
        Ok(())
    }

    /// Plan removal of one regular file or directory tree. The complete tree
    /// is snapshotted before commit and moved to a same-filesystem recovery
    /// location during publication, allowing byte-exact rollback.
    pub(crate) fn remove_path(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let path = path.into();
        let removal_identity = self.reject_duplicate_plan_path(&path)?;
        if self
            .directory_membership_guards
            .keys()
            .any(|directory| directory.starts_with(&removal_identity))
        {
            return Err(format!(
                "compile transaction cannot remove a directory whose membership is guarded: {}",
                path.display()
            ));
        }
        let snapshot = snapshot_removal_path(&path)?;
        self.record_planned_path(removal_identity, PlannedPathKind::Removal);
        self.removals.push(PlannedRemoval { path, snapshot });
        Ok(())
    }

    /// Plan removal of a collection directory only when its exact direct-child
    /// inventory is the caller's target set. The decision is bound to the same
    /// full removal snapshot that commit rechecks, so a concurrent new sibling
    /// cannot be captured and deleted accidentally.
    pub(crate) fn remove_directory_if_only_direct_entries(
        &mut self,
        path: impl Into<PathBuf>,
        expected_names: Vec<OsString>,
    ) -> Result<bool, String> {
        let path = path.into();
        let expected_names = normalize_direct_entry_names(expected_names)?;
        if snapshot_direct_entry_names(&path)? != expected_names {
            return Ok(false);
        }

        let removal_identity = self.reject_duplicate_plan_path(&path)?;
        if self
            .directory_membership_guards
            .keys()
            .any(|directory| directory.starts_with(&removal_identity))
        {
            return Err(format!(
                "compile transaction cannot remove a directory whose membership is guarded: {}",
                path.display()
            ));
        }
        let snapshot = snapshot_removal_path(&path)?;
        if removal_snapshot_direct_entry_names(&snapshot)? != expected_names {
            return Ok(false);
        }
        self.record_planned_path(removal_identity, PlannedPathKind::Removal);
        self.removals.push(PlannedRemoval { path, snapshot });
        Ok(true)
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
        let normalized_target = normalize_transaction_path_identity(&target)?;
        if self.absence_guards.contains(&normalized_target) {
            validate_absence_guard(&normalized_target, "while planning")?;
            return Ok(RegistrationStatus::MissingTarget);
        }
        let existing_registration = self.planned_path_identities.get(&normalized_target)
            == Some(&PlannedPathKind::Registration);
        if !existing_registration {
            if let Some((_, kind)) =
                find_overlapping_planned_identity(&self.planned_path_identities, &normalized_target)
            {
                let conflict = match kind {
                    PlannedPathKind::Create => "create-only",
                    PlannedPathKind::Registration => "another registration",
                    PlannedPathKind::ReadGuard => "an exact read guard",
                    PlannedPathKind::AbsenceGuard => "an absence guard",
                    PlannedPathKind::Removal => "a removal",
                };
                return Err(format!(
                    "compile transaction path is both {conflict} and a registration target: {}",
                    target.display()
                ));
            }
        }

        if !existing_registration {
            let metadata = match fs::symlink_metadata(&target) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    validate_absence_guard(&normalized_target, "while planning")?;
                    self.absence_guards.insert(normalized_target.clone());
                    self.record_planned_path(normalized_target, PlannedPathKind::AbsenceGuard);
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
                normalized_target.clone(),
                PlannedRegistration {
                    path: target.clone(),
                    updated: original.clone(),
                    original,
                },
            );
            self.record_planned_path(normalized_target.clone(), PlannedPathKind::Registration);
        }

        let registration = self
            .registrations
            .get_mut(&normalized_target)
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
            && self.read_guards.is_empty()
            && self.absence_guards.is_empty()
            && self.directory_membership_guards.is_empty()
            && self.removals.is_empty()
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
        changes.extend(
            self.removals
                .iter()
                .map(|removal| format!("would remove {}", removal.path.display())),
        );
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
        lines.extend(
            self.removals
                .iter()
                .map(|removal| format!("[DRY-RUN] would remove {}", removal.path.display())),
        );
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
        self.commit_with_post_validation(|| Ok(()))
    }

    /// Commit the transaction and run one caller-provided validation while
    /// every publication lock and rollback recovery is still held. A
    /// validation error rolls the complete transaction back before success is
    /// finalized.
    pub(crate) fn commit_with_post_validation<F>(
        self,
        post_validation: F,
    ) -> Result<CommitReport, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
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
                .map(|registration| registration.path.clone()),
        );
        targets.extend(self.read_guards.values().map(|guard| guard.path.clone()));
        targets.extend(self.removals.iter().map(|removal| removal.path.clone()));
        let mut guard_targets = self.absence_guards.iter().cloned().collect::<Vec<_>>();
        guard_targets.extend(self.directory_membership_guards.keys().cloned());

        let mut post_validation = Some(post_validation);
        let tree_lock_mode =
            if self.removals.is_empty() && self.directory_membership_guards.is_empty() {
                PublicationTreeLockMode::Shared
            } else {
                PublicationTreeLockMode::Exclusive
            };
        match with_publication_locks_mode_and_guard_targets(
            &targets,
            &guard_targets,
            tree_lock_mode,
            |lock| self.commit_locked(lock, &mut state, &mut post_validation),
        ) {
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

    fn commit_locked<'request, 'lock, 'scope, F>(
        &'request self,
        lock: &'lock PublicationLockToken<'scope>,
        state: &mut PublishState,
        post_validation: &mut Option<F>,
    ) -> Result<CommitReport, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let mut prepared_creates: VecDeque<(
            &'request PlannedCreate,
            PreparedCreate<'request, 'lock, 'scope>,
        )> = VecDeque::new();
        let mut prepared_registrations: VecDeque<(
            &'request PlannedRegistration,
            PreparedReplace<'request, 'lock, 'scope>,
        )> = VecDeque::new();
        let mut prepared_removals: VecDeque<(&'request PlannedRemoval, PendingRemovalRecovery)> =
            VecDeque::new();

        let operation = (|| -> Result<CommitReport, String> {
            self.recheck_exact_read_guards("before publication")?;
            self.recheck_absence_guards("before publication")?;
            self.recheck_directory_membership_guards(false, &[], "before publication")?;

            for removal in &self.removals {
                recheck_removal(removal)?;
            }

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

            for registration in self.registrations.values() {
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
                    PreparedPublication::Replace(prepared) if registration.changed() => {
                        prepared_registrations.push_back((registration, prepared));
                    }
                    PreparedPublication::Replace(prepared) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(format!(
                            "unchanged registration prepared a replacement for {}",
                            registration.path.display()
                        ));
                    }
                    PreparedPublication::Create(prepared) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(format!(
                            "changed registration prepared an invalid state for {}",
                            registration.path.display()
                        ));
                    }
                    PreparedPublication::Unchanged if !registration.changed() => {}
                    PreparedPublication::Unchanged => {
                        return Err(format!(
                            "changed registration prepared an invalid state for {}",
                            registration.path.display()
                        ));
                    }
                }
            }

            for removal in &self.removals {
                prepared_removals.push_back((removal, reserve_removal_recovery(&removal.path)?));
            }

            while let Some((create, prepared)) = prepared_creates.pop_front() {
                let published_identity = match prepared.staged_file_identity() {
                    Ok(identity) => identity,
                    Err(error) => {
                        let message = adapt_publish_error(&error, PublicationRole::Create);
                        record_publish_error_cleanup(state, &error);
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(message);
                    }
                };
                let report = prepared.commit().map_err(|error| {
                    let message = adapt_publish_error(&error, PublicationRole::Create);
                    record_publish_error_cleanup(state, &error);
                    message
                })?;
                record_cleanup_warnings(state, report.cleanup_warnings);
                state.published_creates.push(PublishedCreate {
                    target: create.path.clone(),
                    published: PublishedFileExpectation::new(
                        published_identity,
                        create.bytes.clone(),
                    ),
                });
            }

            failpoint_after_object_files()?;

            while let Some((registration, prepared)) = prepared_registrations.pop_front() {
                let published_identity = match prepared.staged_file_identity() {
                    Ok(identity) => identity,
                    Err(error) => {
                        let message = adapt_publish_error(&error, PublicationRole::Registration);
                        record_publish_error_cleanup(state, &error);
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(message);
                    }
                };
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
                    PublishedFileExpectation::new(published_identity, registration.updated.clone()),
                    permissions,
                ));
            }

            while let Some((removal, recovery)) = prepared_removals.pop_front() {
                recheck_removal(removal)?;
                rename_no_replace(&removal.path, &recovery.path).map_err(|error| {
                    format!(
                        "failed to move removal target {} to no-clobber recovery {}: {error}",
                        removal.path.display(),
                        recovery.path.display()
                    )
                })?;
                state
                    .published_removals
                    .push(recovery.into_published(removal.path.clone(), removal.snapshot.clone()));
                let published = state
                    .published_removals
                    .last()
                    .expect("published removal was just recorded");
                let moved_snapshot = snapshot_removal_path(&published.recovery)?;
                if moved_snapshot != removal.snapshot {
                    return Err(format!(
                        "removal target changed while moving to recovery: {}",
                        removal.path.display()
                    ));
                }
            }

            self.post_validate()?;
            let validate = post_validation.take().ok_or_else(|| {
                "compile transaction post-validation was already consumed".to_string()
            })?;
            validate()?;
            failpoint_post_write_validation()?;
            self.recheck_exact_read_guards("before successful completion")?;
            self.recheck_absence_guards("before successful completion")?;
            let recovery_directories = state
                .published_registrations
                .iter()
                .map(|published| published.recovery_directory.clone())
                .chain(
                    state
                        .published_removals
                        .iter()
                        .map(|published| published.recovery_directory.clone()),
                )
                .collect::<Vec<_>>();
            self.recheck_directory_membership_guards(
                true,
                &recovery_directories,
                "before successful completion",
            )?;

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
                debug_assert!(prepared_removals.is_empty());
                finalize_success(state);
                report.cleanup_warnings = std::mem::take(&mut state.cleanup_warnings);
                Ok(report)
            }
            Err(primary) => {
                discard_prepared(state, &mut prepared_creates, &mut prepared_registrations);
                discard_prepared_removals(state, &mut prepared_removals);
                let mut rollback_errors = rollback(state);
                rollback_errors.extend(std::mem::take(&mut state.cleanup_warnings));
                Err(with_rollback_diagnostics(primary, rollback_errors))
            }
        }
    }

    fn reject_duplicate_plan_path(&self, path: &Path) -> Result<PathBuf, String> {
        let identity = normalize_transaction_path_identity(path)?;
        self.reject_duplicate_normalized_plan_path(&identity, path)?;
        Ok(identity)
    }

    fn reject_duplicate_normalized_plan_path(
        &self,
        identity: &Path,
        requested_path: &Path,
    ) -> Result<(), String> {
        if find_overlapping_planned_identity(&self.planned_path_identities, identity).is_some() {
            Err(format!(
                "compile transaction contains duplicate or overlapping path: {}",
                requested_path.display()
            ))
        } else {
            Ok(())
        }
    }

    fn record_planned_path(&mut self, identity: PathBuf, kind: PlannedPathKind) {
        let previous = self.planned_path_identities.insert(identity, kind);
        debug_assert!(
            previous.is_none(),
            "validated planned path identity must be unique"
        );
    }

    fn semantic_preflight(&self) -> Result<(), String> {
        for create in &self.creates {
            validate_xml_when_applicable(&create.path, &create.bytes)?;
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
            validate_xml_when_applicable(&registration.path, &registration.updated)?;
        }
        Ok(())
    }

    fn recheck_exact_read_guards(&self, phase: &str) -> Result<(), String> {
        for guard in self.read_guards.values() {
            validate_exact_read_guard(&guard.path, &guard.expected_preimage, phase)?;
        }
        Ok(())
    }

    fn recheck_absence_guards(&self, phase: &str) -> Result<(), String> {
        for path in &self.absence_guards {
            validate_absence_guard(path, phase)?;
        }
        Ok(())
    }

    fn recheck_directory_membership_guards(
        &self,
        include_planned_deltas: bool,
        transient_additions: &[PathBuf],
        phase: &str,
    ) -> Result<(), String> {
        for guard in self.directory_membership_guards.values() {
            let expected_entries = if include_planned_deltas {
                self.directory_membership_after_planned_deltas(guard, transient_additions)?
            } else {
                guard.expected_entries.clone()
            };
            validate_directory_membership_guard(
                &guard.directory,
                guard.selector,
                &expected_entries,
                phase,
            )?;
        }
        Ok(())
    }

    fn directory_membership_after_planned_deltas(
        &self,
        guard: &DirectoryMembershipGuard,
        transient_additions: &[PathBuf],
    ) -> Result<Vec<DirectoryTopologyEntry>, String> {
        let mut expected = guard
            .expected_entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.kind))
            .collect::<BTreeMap<_, _>>();
        for create in &self.creates {
            apply_direct_membership_delta(
                &guard.directory,
                guard.selector,
                &create.path,
                true,
                Some(DirectoryTopologyEntryKind::File),
                &mut expected,
            )?;
        }
        for removal in &self.removals {
            apply_direct_membership_delta(
                &guard.directory,
                guard.selector,
                &removal.path,
                false,
                None,
                &mut expected,
            )?;
        }
        for path in transient_additions {
            apply_direct_membership_delta(
                &guard.directory,
                guard.selector,
                path,
                true,
                Some(DirectoryTopologyEntryKind::Directory),
                &mut expected,
            )?;
        }
        Ok(expected
            .into_iter()
            .map(|(name, kind)| DirectoryTopologyEntry { name, kind })
            .collect())
    }

    fn post_validate(&self) -> Result<(), String> {
        for create in &self.creates {
            validate_published_file(&create.path, &create.bytes)?;
        }
        for registration in self.registrations.values() {
            validate_published_file(&registration.path, &registration.updated)?;
            validate_xml_when_applicable(&registration.path, &registration.updated)?;
        }
        for removal in &self.removals {
            match fs::symlink_metadata(&removal.path) {
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(format!(
                        "post-write removal validation failed for {}",
                        removal.path.display()
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "failed to validate removed {}: {error}",
                        removal.path.display()
                    ));
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn snapshot_directory_membership(
    directory: &Path,
    selector: DirectoryMembershipSelector,
) -> Result<Vec<OsString>, String> {
    Ok(snapshot_directory_membership_entries(directory, selector)?
        .into_iter()
        .map(|entry| entry.name)
        .collect())
}

fn snapshot_directory_membership_entries(
    directory: &Path,
    selector: DirectoryMembershipSelector,
) -> Result<Vec<DirectoryTopologyEntry>, String> {
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "failed to inspect directory membership guard {}: {error}",
                directory.display()
            ));
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "directory membership guard must not be a symbolic link or reparse point: {}",
            directory.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "directory membership guard target is not a directory: {}",
            directory.display()
        ));
    }

    let mut entries = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "failed to read directory membership guard {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to read directory membership guard entry in {}: {error}",
                directory.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut names = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        if !directory_membership_name_matches(selector, &name) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "failed to inspect directory membership entry {}: {error}",
                path.display()
            )
        })?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "directory membership entry must not be a symbolic link or reparse point: {}",
                path.display()
            ));
        }
        let valid_entry_kind = match selector {
            DirectoryMembershipSelector::AllDirectEntries => {
                metadata.is_file() || metadata.is_dir()
            }
            DirectoryMembershipSelector::XmlFiles
            | DirectoryMembershipSelector::CfFilesAsciiCaseInsensitive => metadata.is_file(),
        };
        if !valid_entry_kind {
            return Err(format!(
                "directory membership entry has an unsupported filesystem type: {}",
                path.display()
            ));
        }
        let kind = if metadata.is_dir() {
            DirectoryTopologyEntryKind::Directory
        } else {
            DirectoryTopologyEntryKind::File
        };
        names.push(DirectoryTopologyEntry { name, kind });
    }
    Ok(names)
}

fn snapshot_direct_entry_names(directory: &Path) -> Result<Vec<OsString>, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "failed to inspect conditional removal directory {}: {error}",
            directory.display()
        )
    })?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "conditional removal directory must not be a symbolic link or reparse point: {}",
            directory.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "conditional removal target is not a directory: {}",
            directory.display()
        ));
    }
    let mut names = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "failed to enumerate conditional removal directory {}: {error}",
                directory.display()
            )
        })?
        .map(|entry| {
            entry.map(|entry| entry.file_name()).map_err(|error| {
                format!(
                    "failed to enumerate conditional removal directory {}: {error}",
                    directory.display()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

fn normalize_direct_entry_names(expected_names: Vec<OsString>) -> Result<Vec<OsString>, String> {
    let mut normalized = BTreeSet::new();
    for name in expected_names {
        let path = Path::new(&name);
        let mut components = path.components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(format!(
                "invalid conditional removal entry name: {}",
                path.display()
            ));
        }
        if !normalized.insert(name.clone()) {
            return Err(format!(
                "duplicate conditional removal entry name: {}",
                path.display()
            ));
        }
    }
    Ok(normalized.into_iter().collect())
}

fn normalize_expected_directory_entries(
    selector: DirectoryMembershipSelector,
    expected_entries: Vec<DirectoryTopologyEntry>,
) -> Result<Vec<DirectoryTopologyEntry>, String> {
    let mut normalized = BTreeMap::new();
    for entry in expected_entries {
        let path = Path::new(&entry.name);
        let mut components = path.components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
            || !directory_membership_name_matches(selector, &entry.name)
            || (selector != DirectoryMembershipSelector::AllDirectEntries
                && entry.kind != DirectoryTopologyEntryKind::File)
        {
            return Err(format!(
                "invalid directory membership entry name: {}",
                path.display()
            ));
        }
        if normalized.insert(entry.name.clone(), entry.kind).is_some() {
            return Err(format!(
                "duplicate directory membership entry name: {}",
                path.display()
            ));
        }
    }
    Ok(normalized
        .into_iter()
        .map(|(name, kind)| DirectoryTopologyEntry { name, kind })
        .collect())
}

fn directory_membership_name_matches(selector: DirectoryMembershipSelector, name: &OsStr) -> bool {
    match selector {
        DirectoryMembershipSelector::XmlFiles => {
            Path::new(name).extension() == Some(OsStr::new("xml"))
        }
        DirectoryMembershipSelector::CfFilesAsciiCaseInsensitive => Path::new(name)
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cf")),
        DirectoryMembershipSelector::AllDirectEntries => true,
    }
}

fn validate_directory_membership_guard(
    directory: &Path,
    selector: DirectoryMembershipSelector,
    expected_entries: &[DirectoryTopologyEntry],
    phase: &str,
) -> Result<(), String> {
    let actual_entries = snapshot_directory_membership_entries(directory, selector)?;
    if actual_entries != expected_entries {
        let render = |entries: &[DirectoryTopologyEntry]| {
            entries
                .iter()
                .map(|entry| {
                    let suffix = match entry.kind {
                        DirectoryTopologyEntryKind::File => "",
                        DirectoryTopologyEntryKind::Directory => "/",
                    };
                    format!("{}{suffix}", entry.name.to_string_lossy())
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(format!(
            "compile transaction directory membership guard changed after planning {phase}: {} (expected [{}], actual [{}])",
            directory.display(),
            render(expected_entries),
            render(&actual_entries)
        ));
    }
    Ok(())
}

fn apply_direct_membership_delta(
    directory: &Path,
    selector: DirectoryMembershipSelector,
    path: &Path,
    add: bool,
    added_kind: Option<DirectoryTopologyEntryKind>,
    expected: &mut BTreeMap<OsString, DirectoryTopologyEntryKind>,
) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if normalize_transaction_path_identity(parent)?
        != normalize_transaction_path_identity(directory)?
    {
        return Ok(());
    }
    let Some(name) = path.file_name() else {
        return Ok(());
    };
    if !directory_membership_name_matches(selector, name) {
        return Ok(());
    }
    if add {
        let kind = added_kind.ok_or_else(|| {
            format!(
                "missing directory membership kind for planned addition: {}",
                path.display()
            )
        })?;
        if selector != DirectoryMembershipSelector::AllDirectEntries
            && kind != DirectoryTopologyEntryKind::File
        {
            return Err(format!(
                "planned directory membership entry is not a regular file: {}",
                path.display()
            ));
        }
        expected.insert(name.to_os_string(), kind);
    } else {
        expected.remove(name);
    }
    Ok(())
}

fn validate_exact_read_guard(
    path: &Path,
    expected_preimage: &[u8],
    phase: &str,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect compile transaction read guard {} {phase}: {error}",
            path.display()
        )
    })?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "compile transaction read guard must not be a symbolic link or reparse point: {}",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "compile transaction read guard is not a regular file: {}",
            path.display()
        ));
    }
    let actual = fs::read(path).map_err(|error| {
        format!(
            "failed to read compile transaction read guard {} {phase}: {error}",
            path.display()
        )
    })?;
    if actual != expected_preimage {
        return Err(format!(
            "compile transaction read guard changed after planning {phase}: {}",
            path.display()
        ));
    }
    Ok(())
}

fn validate_absence_guard(path: &Path, phase: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "compile transaction absence guard was violated {phase}: {}",
            path.display()
        )),
        Err(error) => Err(format!(
            "failed to inspect compile transaction absence guard {} {phase}: {error}",
            path.display()
        )),
    }
}

#[derive(Debug)]
struct PublishedCreate {
    target: PathBuf,
    published: PublishedFileExpectation,
}

#[derive(Debug)]
struct PublishedRegistration {
    target: PathBuf,
    recovery: PathBuf,
    recovery_directory: PathBuf,
    original: Vec<u8>,
    published: PublishedFileExpectation,
    original_permissions: PortablePermissions,
}

#[derive(Debug)]
struct PublishedFileExpectation {
    identity: FileIdentity,
    bytes: Vec<u8>,
    sha256: [u8; 32],
}

impl PublishedFileExpectation {
    fn new(identity: FileIdentity, bytes: Vec<u8>) -> Self {
        let sha256 = Sha256::digest(&bytes).into();
        Self {
            identity,
            bytes,
            sha256,
        }
    }
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
        published: PublishedFileExpectation,
        original_permissions: PortablePermissions,
    ) -> PublishedRegistration {
        self.armed = false;
        PublishedRegistration {
            target,
            recovery: self.path.clone(),
            recovery_directory: self.directory.clone(),
            original,
            published,
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

#[derive(Debug)]
struct PublishedRemoval {
    target: PathBuf,
    recovery: PathBuf,
    recovery_directory: PathBuf,
    snapshot: RemovalSnapshot,
}

#[derive(Debug)]
struct PendingRemovalRecovery {
    directory: PathBuf,
    path: PathBuf,
    armed: bool,
}

impl PendingRemovalRecovery {
    fn cleanup(&mut self) -> Vec<String> {
        if !self.armed {
            return Vec::new();
        }
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {
                return vec![format!(
                    "pending removal recovery is preserved at {}",
                    self.path.display()
                )];
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return vec![format!(
                    "failed to inspect pending removal recovery {}: {error}",
                    self.path.display()
                )];
            }
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
                "failed to remove pending removal recovery directory {}: {error}",
                self.directory.display()
            )],
        }
    }

    fn into_published(mut self, target: PathBuf, snapshot: RemovalSnapshot) -> PublishedRemoval {
        self.armed = false;
        PublishedRemoval {
            target,
            recovery: self.path.clone(),
            recovery_directory: self.directory.clone(),
            snapshot,
        }
    }
}

impl Drop for PendingRemovalRecovery {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cleanup();
        }
    }
}

#[derive(Debug, Default)]
struct PublishState {
    published_creates: Vec<PublishedCreate>,
    published_registrations: Vec<PublishedRegistration>,
    published_removals: Vec<PublishedRemoval>,
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

fn snapshot_removal_path(path: &Path) -> Result<RemovalSnapshot, String> {
    fn visit(
        current: &Path,
        relative_path: PathBuf,
        entries: &mut Vec<RemovalEntry>,
    ) -> Result<(), String> {
        let metadata = fs::symlink_metadata(current).map_err(|error| {
            format!(
                "failed to inspect removal target {}: {error}",
                current.display()
            )
        })?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "removal target tree must not contain a symbolic link or reparse point: {}",
                current.display()
            ));
        }
        if metadata.is_file() {
            let mut file = fs::File::open(current).map_err(|error| {
                format!(
                    "failed to open removal target {}: {error}",
                    current.display()
                )
            })?;
            let link_count = hard_link_count(&file).map_err(|error| {
                format!(
                    "failed to inspect hard links for removal target {}: {error}",
                    current.display()
                )
            })?;
            if link_count != 1 {
                return Err(format!(
                    "removal target has multiple hard links ({link_count}): {}",
                    current.display()
                ));
            }
            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 64 * 1024];
            let mut size = 0u64;
            loop {
                let count = file.read(&mut buffer).map_err(|error| {
                    format!(
                        "failed to read removal target {}: {error}",
                        current.display()
                    )
                })?;
                if count == 0 {
                    break;
                }
                size = size
                    .checked_add(count as u64)
                    .ok_or_else(|| format!("removal target is too large: {}", current.display()))?;
                hasher.update(&buffer[..count]);
            }
            let final_link_count = hard_link_count(&file).map_err(|error| {
                format!(
                    "failed to recheck hard links for removal target {}: {error}",
                    current.display()
                )
            })?;
            if final_link_count != 1 {
                return Err(format!(
                    "removal target has multiple hard links ({final_link_count}): {}",
                    current.display()
                ));
            }
            let final_path_metadata = fs::symlink_metadata(current).map_err(|error| {
                format!(
                    "failed to recheck removal target {}: {error}",
                    current.display()
                )
            })?;
            if metadata_is_link_or_reparse_point(&final_path_metadata)
                || !final_path_metadata.is_file()
            {
                return Err(format!(
                    "removal target type changed while snapshotting: {}",
                    current.display()
                ));
            }
            if final_path_metadata.len() != size {
                return Err(format!(
                    "removal target size changed while snapshotting: {}",
                    current.display()
                ));
            }
            let sha256 = hasher.finalize().into();
            entries.push(RemovalEntry {
                relative_path,
                kind: RemovalEntryKind::File { size, sha256 },
            });
            return Ok(());
        }
        if !metadata.is_dir() {
            return Err(format!(
                "removal target is neither a regular file nor a directory: {}",
                current.display()
            ));
        }

        entries.push(RemovalEntry {
            relative_path: relative_path.clone(),
            kind: RemovalEntryKind::Directory,
        });
        let mut children = fs::read_dir(current)
            .map_err(|error| {
                format!(
                    "failed to read removal directory {}: {error}",
                    current.display()
                )
            })?
            .map(|entry| {
                entry
                    .map(|entry| (entry.file_name(), entry.path()))
                    .map_err(|error| {
                        format!(
                            "failed to read removal directory entry in {}: {error}",
                            current.display()
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        children.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, child) in children {
            visit(&child, relative_path.join(name), entries)?;
        }
        Ok(())
    }

    let mut entries = Vec::new();
    visit(path, PathBuf::new(), &mut entries)?;
    Ok(RemovalSnapshot { entries })
}

fn removal_snapshot_direct_entry_names(
    snapshot: &RemovalSnapshot,
) -> Result<Vec<OsString>, String> {
    let root_is_directory = snapshot.entries.iter().any(|entry| {
        entry.relative_path.as_os_str().is_empty()
            && matches!(entry.kind, RemovalEntryKind::Directory)
    });
    if !root_is_directory {
        return Err("conditional removal target snapshot is not a directory".to_string());
    }

    let mut names = snapshot
        .entries
        .iter()
        .filter_map(|entry| {
            let mut components = entry.relative_path.components();
            let first = components.next()?;
            if components.next().is_some() {
                return None;
            }
            match first {
                std::path::Component::Normal(name) => Some(name.to_os_string()),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn recheck_removal(removal: &PlannedRemoval) -> Result<(), String> {
    let current = snapshot_removal_path(&removal.path)?;
    if current == removal.snapshot {
        Ok(())
    } else {
        Err(format!(
            "removal target changed after planning: {}",
            removal.path.display()
        ))
    }
}

fn reserve_removal_recovery(target: &Path) -> Result<PendingRemovalRecovery, String> {
    for attempt in 1..=16 {
        let directory = unique_recovery_directory(target);
        match fs::create_dir(&directory) {
            Ok(()) => {
                return Ok(PendingRemovalRecovery {
                    path: directory.join("original"),
                    directory,
                    armed: true,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt < 16 => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve removal recovery for {} at {}: {error}",
                    target.display(),
                    directory.display()
                ));
            }
        }
    }
    Err(format!(
        "failed to reserve removal recovery for {}",
        target.display()
    ))
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

fn normalize_transaction_path_identity(path: &Path) -> Result<PathBuf, String> {
    #[cfg(test)]
    TEST_PATH_IDENTITY_NORMALIZATIONS.with(|count| count.set(count.get() + 1));
    normalize_path_identity(path).map_err(|error| {
        format!(
            "failed to normalize compile transaction path identity {}: {error}",
            path.display()
        )
    })
}

fn find_overlapping_planned_identity<'a>(
    identities: &'a BTreeMap<PathBuf, PlannedPathKind>,
    target: &Path,
) -> Option<(&'a PathBuf, &'a PlannedPathKind)> {
    for ancestor in target.ancestors() {
        if let Some(existing) = identities.get_key_value(ancestor) {
            return Some(existing);
        }
    }
    identities
        .range(target.to_path_buf()..)
        .next()
        .filter(|(identity, _)| identity.starts_with(target))
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

fn discard_prepared_removals(
    state: &mut PublishState,
    removals: &mut VecDeque<(&PlannedRemoval, PendingRemovalRecovery)>,
) {
    while let Some((_removal, mut recovery)) = removals.pop_front() {
        record_cleanup_strings(state, recovery.cleanup());
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

fn remove_recovery_path(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        remove_if_exists(path)
    }
}

enum PublishedTargetState {
    Matches,
    Missing,
    Conflict(String),
}

fn inspect_published_target(
    path: &Path,
    expected: &PublishedFileExpectation,
) -> PublishedTargetState {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return PublishedTargetState::Missing;
        }
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to inspect the published path: {error}"
            ));
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) {
        return PublishedTargetState::Conflict(
            "the published path became a symbolic link or reparse point".to_string(),
        );
    }
    if !metadata.is_file() {
        return PublishedTargetState::Conflict(
            "the published path is no longer a regular file".to_string(),
        );
    }

    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to open the published path: {error}"
            ));
        }
    };
    let identity = match file_identity(&file) {
        Ok(identity) => identity,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to identify the published file: {error}"
            ));
        }
    };
    if identity != expected.identity {
        return PublishedTargetState::Conflict(
            "the path now names a different file identity".to_string(),
        );
    }
    let links = match hard_link_count(&file) {
        Ok(links) => links,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to inspect published hard links: {error}"
            ));
        }
    };
    if links != 1 {
        return PublishedTargetState::Conflict(format!(
            "the published file now has {links} hard links"
        ));
    }

    let mut bytes = Vec::new();
    if let Err(error) = file.read_to_end(&mut bytes) {
        return PublishedTargetState::Conflict(format!(
            "failed to read the published file: {error}"
        ));
    }
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    if sha256 != expected.sha256 || bytes != expected.bytes {
        return PublishedTargetState::Conflict(
            "the published file bytes changed after publication".to_string(),
        );
    }

    let final_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to recheck the published path: {error}"
            ));
        }
    };
    if metadata_is_link_or_reparse_point(&final_metadata) || !final_metadata.is_file() {
        return PublishedTargetState::Conflict(
            "the published path type changed during rollback inspection".to_string(),
        );
    }
    let final_file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to reopen the published path: {error}"
            ));
        }
    };
    match file_identity(&final_file) {
        Ok(final_identity) if final_identity == expected.identity => PublishedTargetState::Matches,
        Ok(_) => PublishedTargetState::Conflict(
            "the published path identity changed during rollback inspection".to_string(),
        ),
        Err(error) => PublishedTargetState::Conflict(format!(
            "failed to re-identify the published path: {error}"
        )),
    }
}

fn reserve_rollback_quarantine(target: &Path) -> Result<(PathBuf, PathBuf), String> {
    for attempt in 1..=16 {
        let directory = unique_recovery_directory(target);
        match fs::create_dir(&directory) {
            Ok(()) => {
                let path = directory.join("published");
                return Ok((directory, path));
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt < 16 => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve rollback quarantine for {} at {}: {error}",
                    target.display(),
                    directory.display()
                ));
            }
        }
    }
    Err(format!(
        "failed to reserve rollback quarantine for {}",
        target.display()
    ))
}

fn restore_quarantined_file_no_clobber(quarantine: &Path, target: &Path) -> Result<(), String> {
    fs::hard_link(quarantine, target).map_err(|error| {
        format!(
            "failed to restore quarantined concurrent file {} to {} without clobbering: {error}",
            quarantine.display(),
            target.display()
        )
    })?;
    remove_if_exists(quarantine).map_err(|error| {
        format!(
            "restored quarantined concurrent file to {}, but failed to remove recovery link {}: {error}",
            target.display(),
            quarantine.display()
        )
    })
}

fn rollback_registration(published: &PublishedRegistration, errors: &mut Vec<String>) {
    match inspect_published_target(&published.target, &published.published) {
        PublishedTargetState::Matches => {}
        PublishedTargetState::Missing => {
            errors.push(format!(
                "rollback conflict for registration {}: the published target was removed concurrently; recovery is preserved at {}",
                published.target.display(),
                published.recovery.display()
            ));
            return;
        }
        PublishedTargetState::Conflict(reason) => {
            errors.push(format!(
                "rollback conflict for registration {}: {reason}; concurrent target is preserved; recovery is preserved at {}",
                published.target.display(),
                published.recovery.display()
            ));
            return;
        }
    }
    run_before_rollback_mutation(&published.target);

    let quarantined = published.recovery_directory.join("published");
    if let Err(error) = rename_no_replace(&published.target, &quarantined) {
        let message = if fs::symlink_metadata(&quarantined).is_ok() {
            format!(
                "rollback conflict for registration {}: rollback quarantine appeared concurrently at {}; published target and recovery are preserved at {}: {error}",
                published.target.display(),
                quarantined.display(),
                published.recovery.display()
            )
        } else {
            format!(
                "failed to quarantine published registration {} without clobbering before rollback; recovery is preserved at {}: {error}",
                published.target.display(),
                published.recovery.display()
            )
        };
        errors.push(message);
        return;
    }
    match inspect_published_target(&quarantined, &published.published) {
        PublishedTargetState::Matches => {}
        PublishedTargetState::Missing => {
            errors.push(format!(
                "rollback conflict for registration {}: quarantined publication disappeared; original recovery is preserved at {}",
                published.target.display(),
                published.recovery.display()
            ));
            return;
        }
        PublishedTargetState::Conflict(reason) => {
            let restore = restore_quarantined_file_no_clobber(&quarantined, &published.target);
            let preservation = match restore {
                Ok(()) => "concurrent target was restored without clobbering".to_string(),
                Err(error) => format!(
                    "{error}; concurrent file is preserved at {}",
                    quarantined.display()
                ),
            };
            errors.push(format!(
                "rollback conflict for registration {}: {reason}; {preservation}; original recovery is preserved at {}",
                published.target.display(),
                published.recovery.display()
            ));
            return;
        }
    }

    if let Err(error) = fs::hard_link(&published.recovery, &published.target) {
        errors.push(format!(
            "failed to restore registration {} without clobbering; original recovery is preserved at {} and published bytes at {}: {error}",
            published.target.display(),
            published.recovery.display(),
            quarantined.display()
        ));
        return;
    }
    if let Err(error) = remove_if_exists(&published.recovery) {
        errors.push(format!(
            "restored registration {}, but failed to remove original recovery link {}; recovery is preserved: {error}",
            published.target.display(),
            published.recovery.display()
        ));
    }
    if let Err(error) = remove_if_exists(&quarantined) {
        errors.push(format!(
            "restored registration {}, but failed to remove quarantined published bytes {}; recovery is preserved: {error}",
            published.target.display(),
            quarantined.display()
        ));
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
        preserve_recovery_copy(published, errors);
        return;
    }
    if let Err(error) = fs::remove_dir(&published.recovery_directory) {
        errors.push(format!(
            "failed to remove restored registration recovery directory {}: {error}",
            published.recovery_directory.display()
        ));
        preserve_recovery_copy(published, errors);
    }
}

fn rollback_create(published: &PublishedCreate, errors: &mut Vec<String>) {
    match inspect_published_target(&published.target, &published.published) {
        PublishedTargetState::Matches => {}
        PublishedTargetState::Missing => return,
        PublishedTargetState::Conflict(reason) => {
            errors.push(format!(
                "rollback conflict for create-only target {}: {reason}; concurrent target is preserved",
                published.target.display()
            ));
            return;
        }
    }
    run_before_rollback_mutation(&published.target);

    let (quarantine_directory, quarantined) = match reserve_rollback_quarantine(&published.target) {
        Ok(quarantine) => quarantine,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    if let Err(error) = rename_no_replace(&published.target, &quarantined) {
        let message = if fs::symlink_metadata(&quarantined).is_ok() {
            format!(
                "rollback conflict for create-only target {}: rollback quarantine appeared concurrently at {}; published target is preserved: {error}",
                published.target.display(),
                quarantined.display()
            )
        } else {
            format!(
                "failed to quarantine create-only target {} without clobbering before rollback: {error}",
                published.target.display()
            )
        };
        errors.push(message);
        let _ = fs::remove_dir(&quarantine_directory);
        return;
    }

    match inspect_published_target(&quarantined, &published.published) {
        PublishedTargetState::Matches => {
            let concurrent_target_exists = match fs::symlink_metadata(&published.target) {
                Err(error) if error.kind() == ErrorKind::NotFound => false,
                Ok(_) => true,
                Err(error) => {
                    errors.push(format!(
                        "failed to inspect create-only target {} after quarantine: {error}",
                        published.target.display()
                    ));
                    true
                }
            };
            if let Err(error) = remove_if_exists(&quarantined) {
                errors.push(format!(
                    "failed to remove quarantined create-only publication {}; recovery is preserved: {error}",
                    quarantined.display()
                ));
                return;
            }
            if concurrent_target_exists {
                errors.push(format!(
                    "rollback conflict for create-only target {}: a concurrent target appeared during rollback and is preserved",
                    published.target.display()
                ));
            }
        }
        PublishedTargetState::Missing => {
            errors.push(format!(
                "rollback conflict for create-only target {}: quarantined publication disappeared",
                published.target.display()
            ));
        }
        PublishedTargetState::Conflict(reason) => {
            let restore = restore_quarantined_file_no_clobber(&quarantined, &published.target);
            let preservation = match restore {
                Ok(()) => "concurrent target was restored without clobbering".to_string(),
                Err(error) => format!(
                    "{error}; concurrent file is preserved at {}",
                    quarantined.display()
                ),
            };
            errors.push(format!(
                "rollback conflict for create-only target {}: {reason}; {preservation}",
                published.target.display()
            ));
        }
    }
    if let Err(error) = fs::remove_dir(&quarantine_directory) {
        if error.kind() != ErrorKind::DirectoryNotEmpty {
            errors.push(format!(
                "failed to remove create-only rollback quarantine directory {}: {error}",
                quarantine_directory.display()
            ));
        }
    }
}

fn rollback(state: &mut PublishState) -> Vec<String> {
    let mut errors = Vec::new();

    for published in state.published_removals.iter().rev() {
        match fs::symlink_metadata(&published.target) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Ok(_) => {
                errors.push(format!(
                    "rollback conflict for removed target {}: a concurrent target already exists and is preserved; recovery is preserved at {}",
                    published.target.display(),
                    published.recovery.display()
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!(
                    "failed to inspect removed target before rollback {}: {error}; recovery is preserved at {}",
                    published.target.display(),
                    published.recovery.display()
                ));
                continue;
            }
        }
        run_before_removal_rollback_restore(&published.recovery, &published.target);
        if let Err(error) = rename_no_replace(&published.recovery, &published.target) {
            let message = if fs::symlink_metadata(&published.target).is_ok() {
                format!(
                    "rollback conflict for removed target {}: a concurrent target appeared before atomic restoration and is preserved; recovery is preserved at {}: {error}",
                    published.target.display(),
                    published.recovery.display()
                )
            } else {
                format!(
                    "failed to restore removed target {} from {} without clobbering; recovery is preserved: {error}",
                    published.target.display(),
                    published.recovery.display()
                )
            };
            errors.push(message);
            continue;
        }
        match snapshot_removal_path(&published.target) {
            Ok(snapshot) if snapshot == published.snapshot => {}
            Ok(_) => errors.push(format!(
                "restored removal target differs from its planned snapshot: {}",
                published.target.display()
            )),
            Err(error) => errors.push(format!(
                "failed to validate restored removal target {}: {error}",
                published.target.display()
            )),
        }
        if let Err(error) = fs::remove_dir(&published.recovery_directory) {
            errors.push(format!(
                "failed to remove restored removal recovery directory {}: {error}",
                published.recovery_directory.display()
            ));
        }
    }

    for published in state.published_registrations.iter().rev() {
        rollback_registration(published, &mut errors);
    }

    for published in state.published_creates.iter().rev() {
        rollback_create(published, &mut errors);
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
    for published in &state.published_removals {
        if let Err(error) = remove_recovery_path(&published.recovery) {
            state.cleanup_warnings.push(format!(
                "failed to remove recovery for deleted target {}; recovery is preserved at {}: {error}",
                published.target.display(),
                published.recovery.display()
            ));
            continue;
        }
        if let Err(error) = fs::remove_dir(&published.recovery_directory) {
            state.cleanup_warnings.push(format!(
                "failed to remove deleted-target recovery directory {}: {error}",
                published.recovery_directory.display()
            ));
        }
    }
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
type BeforeRollbackMutationHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
type BeforeRemovalRollbackRestoreHook = Box<dyn FnOnce(&Path, &Path)>;

#[cfg(test)]
thread_local! {
    static TEST_FAILPOINT: Cell<Option<CommitFailpoint>> = const { Cell::new(None) };
    static TEST_REGISTRATION_RECOVERY_PAUSE: RefCell<Option<RegistrationRecoveryPause>> = const { RefCell::new(None) };
    static TEST_PATH_IDENTITY_NORMALIZATIONS: Cell<usize> = const { Cell::new(0) };
    static TEST_BEFORE_ROLLBACK_MUTATION_HOOK: RefCell<Option<BeforeRollbackMutationHook>> = const { RefCell::new(None) };
    static TEST_BEFORE_REMOVAL_ROLLBACK_RESTORE_HOOK: RefCell<Option<BeforeRemovalRollbackRestoreHook>> = const { RefCell::new(None) };
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

#[cfg(test)]
fn with_before_rollback_mutation_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeRollbackMutationHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_ROLLBACK_MUTATION_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        TEST_BEFORE_ROLLBACK_MUTATION_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_before_removal_rollback_restore_hook<T>(
    hook: impl FnOnce(&Path, &Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeRemovalRollbackRestoreHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_REMOVAL_ROLLBACK_RESTORE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        TEST_BEFORE_REMOVAL_ROLLBACK_RESTORE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

fn run_before_rollback_mutation(_path: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_BEFORE_ROLLBACK_MUTATION_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_before_removal_rollback_restore(_recovery: &Path, _target: &Path) {
    #[cfg(test)]
    if let Some(hook) =
        TEST_BEFORE_REMOVAL_ROLLBACK_RESTORE_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(_recovery, _target);
    }
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
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
    <InternalInfo>
      <xr:ContainedObject><xr:ClassId>9cd510cd-abfc-11d4-9434-004095e12fc7</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000002</xr:ObjectId></xr:ContainedObject>
      <xr:ContainedObject><xr:ClassId>9fcd25a0-4822-11d4-9414-008048da11f9</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000003</xr:ObjectId></xr:ContainedObject>
      <xr:ContainedObject><xr:ClassId>e3687481-0a87-462c-a166-9f34594f9bba</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000004</xr:ObjectId></xr:ContainedObject>
      <xr:ContainedObject><xr:ClassId>9de14907-ec23-4a07-96f0-85521cb6b53b</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000005</xr:ObjectId></xr:ContainedObject>
      <xr:ContainedObject><xr:ClassId>51f2d5d8-ea4d-4064-8892-82951750031e</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000006</xr:ObjectId></xr:ContainedObject>
      <xr:ContainedObject><xr:ClassId>e68182ea-4237-4383-967f-90c1e3370bc7</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000007</xr:ObjectId></xr:ContainedObject>
      <xr:ContainedObject><xr:ClassId>fb282519-d103-4dd3-bc12-cb271d631dfc</xr:ClassId><xr:ObjectId>00000000-0000-0000-0000-000000000008</xr:ObjectId></xr:ContainedObject>
    </InternalInfo>
    <Properties>
      <Name>Demo</Name>
      <ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode>
      <DefaultLanguage>Language.English</DefaultLanguage>
    </Properties>
    <ChildObjects><Language>English</Language><Catalog>Items</Catalog></ChildObjects>
  </Configuration>
</MetaDataObject>"#,
        )
        .unwrap();
        fs::create_dir_all(src.join("Languages")).unwrap();
        fs::write(src.join("Languages/English.xml"), b"language marker").unwrap();
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
    fn canonical_registrations_accumulate_across_path_aliases() {
        let root = temp_root("canonical-path-alias");
        let detour = root.join("detour");
        fs::create_dir(&detour).expect("detour directory must be created");
        let config = root.join("Configuration.xml");
        let config_alias = detour.join("..").join("Configuration.xml");
        fs::write(&config, configuration_bytes()).expect("fixture must be written");
        let mut transaction = CompileTransaction::new();

        assert_eq!(
            transaction
                .register_canonical_child(&config_alias, "Role", "Reader")
                .expect("aliased role registration must plan"),
            RegistrationStatus::Added
        );
        assert_eq!(
            transaction
                .register_canonical_child(&config, "Subsystem", "Core")
                .expect("canonical subsystem registration must join the same plan"),
            RegistrationStatus::Added
        );
        assert_eq!(transaction.registration_diffs().len(), 1);

        transaction
            .commit()
            .expect("aliased transaction must commit");
        let published = fs::read_to_string(&config).expect("published config must be readable");
        assert!(published.contains("<Role>Reader</Role>"), "{published}");
        assert!(
            published.contains("<Subsystem>Core</Subsystem>"),
            "{published}"
        );
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn binary_replacement_uses_the_same_transaction_without_xml_validation() {
        let root = temp_root("binary-replacement");
        let target = root.join("Ext/ParentConfigurations.bin");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = vec![0, 0xff, 0x01, 0x80];
        let replacement = vec![0x7f, 0, 0xfe, 0x02, 0x03];
        fs::write(&target, &original).unwrap();
        let mut transaction = CompileTransaction::new();
        transaction
            .replace_bytes(&target, &original, replacement.clone())
            .unwrap();

        let report = transaction.commit().expect("binary update must commit");

        assert_eq!(report.updated, vec![target.clone()]);
        assert_eq!(fs::read(&target).unwrap(), replacement);
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_registration_target_is_explicit_and_guards_normalized_absence() {
        let root = temp_root("missing-target");
        let detour = root.join("detour");
        fs::create_dir(&detour).expect("detour directory must be created");
        let target = root.join("Configuration.xml");
        let target_alias = detour.join("..").join("Configuration.xml");
        let mut transaction = CompileTransaction::new();
        assert_eq!(
            transaction
                .register_canonical_child(&target_alias, "Role", "Reader")
                .expect("missing target is an allowed status"),
            RegistrationStatus::MissingTarget
        );
        assert!(!transaction.is_empty());
        assert!(transaction
            .protects_path(&target)
            .expect("absence identity must normalize"));
        assert!(transaction.registration_diffs().is_empty());
        let report = transaction
            .commit()
            .expect("unchanged absent target must satisfy the guard");
        assert!(report.created.is_empty());
        assert!(report.updated.is_empty());
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn nested_absence_guard_with_missing_parent_acquires_a_lock_only_identity() {
        let root = temp_root("nested-absence-missing-parent");
        let marker = root.join("Configuration/Configuration.mdo");
        let output = root.join("Generated.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .guard_path_absent(&marker)
            .expect("nested marker absence must plan");
        transaction
            .create_bytes(&output, b"<Generated/>\n".to_vec())
            .expect("output creation must plan");

        let report = transaction
            .commit()
            .expect("missing marker parent must not prevent guard locking");

        assert_eq!(report.created, vec![output.clone()]);
        assert!(!marker.exists());
        assert_eq!(fs::read(&output).unwrap(), b"<Generated/>\n");
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn directory_membership_guard_accepts_its_own_top_level_xml_create() {
        let root = temp_root("membership-own-create");
        let expected =
            snapshot_directory_membership(&root, DirectoryMembershipSelector::XmlFiles).unwrap();
        let created = root.join("Created.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .guard_or_verify_directory_membership(
                &root,
                DirectoryMembershipSelector::XmlFiles,
                expected,
            )
            .unwrap();
        transaction
            .create_bytes(&created, b"<Created/>\n".to_vec())
            .unwrap();

        let report = transaction
            .commit()
            .expect("planned membership delta must be accepted");

        assert_eq!(report.created, vec![created.clone()]);
        assert_eq!(
            snapshot_directory_membership(&root, DirectoryMembershipSelector::XmlFiles).unwrap(),
            vec![OsString::from("Created.xml")]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_membership_guard_accepts_own_create_in_an_absent_root() {
        let root = temp_root("membership-absent-root");
        let external_root = root.join("external");
        let expected =
            snapshot_directory_membership(&external_root, DirectoryMembershipSelector::XmlFiles)
                .unwrap();
        let created = external_root.join("Created.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .guard_or_verify_directory_membership(
                &external_root,
                DirectoryMembershipSelector::XmlFiles,
                expected,
            )
            .unwrap();
        transaction
            .create_bytes(&created, b"<Created/>\n".to_vec())
            .unwrap();

        transaction
            .commit()
            .expect("absent guarded root must allow its planned descriptor");

        assert_eq!(fs::read(&created).unwrap(), b"<Created/>\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_membership_guard_rolls_back_on_an_unplanned_top_level_xml_entry() {
        let root = temp_root("membership-concurrent-create");
        let existing = root.join("Existing.xml");
        let planned = root.join("Planned.xml");
        let concurrent = root.join("Concurrent.xml");
        fs::write(&existing, b"<Existing/>\n").unwrap();
        let expected =
            snapshot_directory_membership(&root, DirectoryMembershipSelector::XmlFiles).unwrap();
        let mut transaction = CompileTransaction::new();
        transaction
            .guard_or_verify_directory_membership(
                &root,
                DirectoryMembershipSelector::XmlFiles,
                expected,
            )
            .unwrap();
        transaction
            .create_bytes(&planned, b"<Planned/>\n".to_vec())
            .unwrap();
        let concurrent_for_hook = concurrent.clone();

        let error = with_before_commit_hook(
            move |_| fs::write(&concurrent_for_hook, b"<Concurrent/>\n").unwrap(),
            || transaction.commit(),
        )
        .expect_err("unplanned direct XML entry must abort publication");

        assert!(error.contains("directory membership guard"), "{error}");
        assert!(!planned.exists());
        assert_eq!(fs::read(&existing).unwrap(), b"<Existing/>\n");
        assert_eq!(fs::read(&concurrent).unwrap(), b"<Concurrent/>\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_membership_guard_exclusively_serializes_a_cooperating_create() {
        let root = temp_root("membership-exclusive-tree-lock");
        let expected =
            snapshot_directory_membership(&root, DirectoryMembershipSelector::XmlFiles).unwrap();
        let mut guarded = CompileTransaction::new();
        guarded
            .guard_or_verify_directory_membership(
                &root,
                DirectoryMembershipSelector::XmlFiles,
                expected,
            )
            .unwrap();
        let created = root.join("Concurrent.xml");
        let mut creator = CompileTransaction::new();
        creator
            .create_bytes(&created, b"<Concurrent/>\n".to_vec())
            .unwrap();

        let acquired = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let acquired_by_guard = Arc::clone(&acquired);
        let release_guard = Arc::clone(&release);
        let guarded_thread = thread::spawn(move || {
            with_publication_lock_pause(acquired_by_guard, release_guard, || guarded.commit())
        });
        acquired.wait();

        let (contended_sender, contended_receiver) = mpsc::channel();
        let creator_thread = thread::spawn(move || {
            with_publication_lock_contention_signal(contended_sender, || creator.commit())
        });
        let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
        release.wait();

        guarded_thread
            .join()
            .expect("guarded transaction thread must not panic")
            .expect("stable membership guard must commit");
        creator_thread
            .join()
            .expect("creator transaction thread must not panic")
            .expect("creator must commit after the membership guard releases");

        contention.expect("cooperating create must wait on the exclusive tree gate");
        assert_eq!(fs::read(&created).unwrap(), b"<Concurrent/>\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_registration_target_appearance_rolls_back_created_outputs() {
        let root = temp_root("missing-target-appears");
        let config = root.join("Configuration.xml");
        let output = root.join("Roles/Reader.xml");
        let mut transaction = CompileTransaction::new();
        transaction
            .create_utf8_bom_text(
                &output,
                "<?xml version=\"1.0\"?><MetaDataObject><Role/></MetaDataObject>",
            )
            .expect("role output must plan");
        assert_eq!(
            transaction
                .register_canonical_child(&config, "Role", "Reader")
                .expect("missing target is an allowed status"),
            RegistrationStatus::MissingTarget
        );
        let config_for_hook = config.clone();
        let supported_config = configuration_bytes();

        let error = with_before_commit_hook(
            move |_| fs::write(&config_for_hook, &supported_config).unwrap(),
            || transaction.commit(),
        )
        .expect_err("late owner appearance must abort the transaction");

        assert!(error.contains("absence guard"), "{error}");
        assert!(config.is_file());
        assert!(!output.exists());
        assert!(transaction_debris(&root).is_empty());
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
    fn removal_plan_rejects_hard_linked_file_in_tree() {
        let root = temp_root("hard-linked-removal-plan");
        let tree = root.join("Template");
        let payload = tree.join("Ext/Template.bin");
        let alias = root.join("Template.alias.bin");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::write(&payload, b"payload-before").unwrap();
        fs::hard_link(&payload, &alias).unwrap();
        let mut transaction = CompileTransaction::new();

        let error = transaction
            .remove_path(&tree)
            .expect_err("hard-linked removal payload must be rejected");

        assert!(error.contains("multiple hard links"), "{error}");
        assert_eq!(fs::read(&payload).unwrap(), b"payload-before");
        assert_eq!(fs::read(&alias).unwrap(), b"payload-before");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conditional_collection_removal_preserves_an_unrelated_direct_child() {
        let root = temp_root("conditional-collection-preserves-sibling");
        let collection = root.join("Forms");
        fs::create_dir_all(&collection).unwrap();
        let target = collection.join("Main.xml");
        let sibling = collection.join("Other.xml");
        fs::write(&target, b"target").unwrap();
        fs::write(&sibling, b"sibling").unwrap();
        let mut transaction = CompileTransaction::new();

        assert!(!transaction
            .remove_directory_if_only_direct_entries(&collection, vec![OsString::from("Main.xml")],)
            .unwrap());
        transaction.remove_path(&target).unwrap();
        transaction.commit().unwrap();

        assert!(collection.is_dir());
        assert!(!target.exists());
        assert_eq!(fs::read(&sibling).unwrap(), b"sibling");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conditional_collection_removal_rejects_a_late_direct_child() {
        let root = temp_root("conditional-collection-late-child");
        let collection = root.join("Forms");
        fs::create_dir_all(&collection).unwrap();
        let target = collection.join("Main.xml");
        let late = collection.join("Late.xml");
        let owner = root.join("Owner.xml");
        let owner_before = b"<Owner>before</Owner>".to_vec();
        let owner_after = b"<Owner>after</Owner>".to_vec();
        fs::write(&target, b"target").unwrap();
        fs::write(&owner, &owner_before).unwrap();
        let mut transaction = CompileTransaction::new();
        transaction
            .replace_bytes(&owner, &owner_before, owner_after)
            .unwrap();

        assert!(transaction
            .remove_directory_if_only_direct_entries(&collection, vec![OsString::from("Main.xml")],)
            .unwrap());
        let late_for_hook = late.clone();
        let error = with_before_commit_hook(
            move |_| fs::write(&late_for_hook, b"late").unwrap(),
            || transaction.commit(),
        )
        .expect_err("late collection member must invalidate the removal snapshot");

        assert!(error.contains("removal target changed"), "{error}");
        assert_eq!(fs::read(&owner).unwrap(), owner_before);
        assert_eq!(fs::read(&target).unwrap(), b"target");
        assert_eq!(fs::read(&late).unwrap(), b"late");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removal_recheck_rejects_late_hard_link_and_rolls_back_replacement() {
        let root = temp_root("late-hard-linked-removal");
        let owner = root.join("Owner.xml");
        let owner_before = b"<Owner><State>before</State></Owner>".to_vec();
        let owner_after = b"<Owner><State>after</State></Owner>".to_vec();
        fs::write(&owner, &owner_before).unwrap();
        let tree = root.join("Template");
        let payload = tree.join("Ext/Template.bin");
        let payload_before = b"payload-before".to_vec();
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::write(&payload, &payload_before).unwrap();
        let alias = root.join("late.alias.bin");
        let payload_for_hook = payload.clone();
        let alias_for_hook = alias.clone();
        let mut transaction = CompileTransaction::new();
        transaction
            .replace_bytes(&owner, &owner_before, owner_after)
            .unwrap();
        transaction.remove_path(&tree).unwrap();

        let result = with_before_commit_hook(
            move |_| fs::hard_link(&payload_for_hook, &alias_for_hook).unwrap(),
            || transaction.commit(),
        );
        let error = result.expect_err("late hard link must abort the whole transaction");

        assert!(error.contains("multiple hard links"), "{error}");
        assert_eq!(fs::read(&owner).unwrap(), owner_before);
        assert_eq!(fs::read(&payload).unwrap(), payload_before);
        assert_eq!(fs::read(&alias).unwrap(), payload_before);
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
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
    fn rollback_preserves_concurrent_registration_replacement_and_original_recovery() {
        let root = temp_root("rollback-registration-concurrent-replacement");
        let config = root.join("Configuration.xml");
        let original = configuration_bytes();
        let guard = root.join("Decision.bsl");
        fs::write(&config, &original).unwrap();
        fs::write(&guard, b"stable").unwrap();
        let mut transaction = CompileTransaction::new();
        transaction
            .register_canonical_child(&config, "Role", "Reader")
            .unwrap();
        let diff = transaction.registration_diffs().pop().unwrap();
        let mut concurrent = original.clone();
        concurrent.splice(diff.byte_range, diff.after);
        transaction.guard_exact_preimage(&guard, b"stable").unwrap();
        let config_for_hook = config.clone();
        let root_for_hook = root.clone();
        let concurrent_for_hook = concurrent.clone();

        let error = with_before_rollback_mutation_hook(
            move |path| {
                assert_eq!(path, config_for_hook);
                let external = root_for_hook.join("external-registration.xml");
                fs::write(&external, &concurrent_for_hook).unwrap();
                replace_file_atomically(&external, &config_for_hook).unwrap();
            },
            || {
                transaction.commit_with_post_validation(|| {
                    fs::write(&guard, b"changed").unwrap();
                    Ok(())
                })
            },
        )
        .expect_err("late guard failure must not overwrite a concurrent replacement");

        assert!(error.contains("read guard"), "{error}");
        assert!(error.contains("rollback conflict"), "{error}");
        assert!(error.contains("recovery is preserved"), "{error}");
        assert_eq!(fs::read(&config).unwrap(), concurrent);
        let recovery_directory = transaction_debris(&root)
            .into_iter()
            .find(|path| path.is_dir())
            .expect("unsafe registration rollback must preserve its recovery directory");
        assert_eq!(
            fs::read(recovery_directory.join("original")).unwrap(),
            original
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_preserves_concurrent_edit_of_create_only_target() {
        let root = temp_root("rollback-create-concurrent-replacement");
        let target = root.join("Generated.xml");
        let concurrent = b"<ConcurrentCreate/>\n".to_vec();
        let guard = root.join("Decision.bsl");
        fs::write(&guard, b"stable").unwrap();
        let mut transaction = CompileTransaction::new();
        transaction
            .create_bytes(&target, b"<Generated/>\n".to_vec())
            .unwrap();
        transaction.guard_exact_preimage(&guard, b"stable").unwrap();
        let target_for_hook = target.clone();
        let concurrent_for_hook = concurrent.clone();

        let error = with_before_rollback_mutation_hook(
            move |path| {
                assert_eq!(path, target_for_hook);
                fs::write(&target_for_hook, &concurrent_for_hook).unwrap();
            },
            || {
                transaction.commit_with_post_validation(|| {
                    fs::write(&guard, b"changed").unwrap();
                    Ok(())
                })
            },
        )
        .expect_err("late guard failure must not delete a concurrent replacement");

        assert!(error.contains("read guard"), "{error}");
        assert!(error.contains("rollback conflict"), "{error}");
        assert_eq!(fs::read(&target).unwrap(), concurrent);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removal_rollback_preserves_concurrent_file_and_recovery_artifact() {
        let root = temp_root("rollback-removal-concurrent-file");
        let target = root.join("Removed.bin");
        let original = b"removed-before".to_vec();
        let concurrent = b"concurrent-file".to_vec();
        fs::write(&target, &original).unwrap();
        let mut transaction = CompileTransaction::new();
        transaction.remove_path(&target).unwrap();
        let target_for_hook = target.clone();
        let concurrent_for_hook = concurrent.clone();

        let error = with_before_removal_rollback_restore_hook(
            move |recovery, restore_target| {
                assert!(recovery.is_file(), "recovery file must be ready");
                assert_eq!(restore_target, target_for_hook);
                assert!(
                    !restore_target.exists(),
                    "restore target must still be absent at the race point"
                );
                fs::write(restore_target, &concurrent_for_hook).unwrap();
            },
            || {
                with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                    transaction.commit()
                })
            },
        )
        .expect_err("late validation failure must preserve a concurrent restore target");

        assert!(error.contains("rollback conflict"), "{error}");
        assert!(error.contains("recovery is preserved"), "{error}");
        assert_eq!(fs::read(&target).unwrap(), concurrent);
        let recovery_directory = transaction_debris(&root)
            .into_iter()
            .find(|path| path.is_dir())
            .expect("removed file recovery directory must be preserved");
        let recovered_original = recovery_directory.join("original");
        assert_eq!(fs::read(&recovered_original).unwrap(), original);
        assert_ne!(recovered_original, target);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removal_rollback_preserves_concurrent_empty_directory_and_recovery_tree() {
        let root = temp_root("rollback-removal-concurrent-directory");
        let target = root.join("RemovedTree");
        let original_payload = target.join("Ext/original.bin");
        fs::create_dir_all(original_payload.parent().unwrap()).unwrap();
        fs::write(&original_payload, b"removed-tree-before").unwrap();
        let mut transaction = CompileTransaction::new();
        transaction.remove_path(&target).unwrap();
        let target_for_hook = target.clone();

        let error = with_before_removal_rollback_restore_hook(
            move |recovery, restore_target| {
                assert!(recovery.is_dir(), "recovery tree must be ready");
                assert_eq!(restore_target, target_for_hook);
                assert!(
                    !restore_target.exists(),
                    "restore target must still be absent at the race point"
                );
                fs::create_dir(restore_target).unwrap();
            },
            || {
                with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
                    transaction.commit()
                })
            },
        )
        .expect_err("late validation failure must preserve a concurrent restore directory");

        assert!(error.contains("rollback conflict"), "{error}");
        assert!(error.contains("recovery is preserved"), "{error}");
        assert!(
            fs::read_dir(&target).unwrap().next().is_none(),
            "concurrent empty directory must not be replaced by the removed tree"
        );
        let recovery_directory = transaction_debris(&root)
            .into_iter()
            .find(|path| path.is_dir())
            .expect("removed tree recovery directory must be preserved");
        let recovered_original = recovery_directory.join("original/Ext/original.bin");
        assert_eq!(
            fs::read(recovered_original).unwrap(),
            b"removed-tree-before"
        );
        fs::remove_dir_all(root).unwrap();
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
    fn path_identity_alias_is_protected_as_same_transaction_target() {
        let root = temp_root("path-identity-alias");
        let detour = root.join("detour");
        fs::create_dir(&detour).expect("detour directory must be created");
        let owner = root.join("Owner.xml");
        let owner_alias = detour.join("..").join("Owner.xml");
        let before = b"<Owner><State>before</State></Owner>\n";
        fs::write(&owner, before).expect("owner fixture must be written");

        let mut transaction = CompileTransaction::new();
        transaction
            .replace_bytes(
                &owner_alias,
                before,
                b"<Owner><State>after</State></Owner>\n".to_vec(),
            )
            .expect("owner alias replacement must plan");

        assert!(
            transaction
                .protects_path(&owner)
                .expect("path identities must normalize"),
            "lexical and filesystem aliases must resolve to one protected target"
        );
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn exact_input_guard_reuses_replacement_preimage_through_path_alias() {
        let root = temp_root("exact-input-path-alias");
        let detour = root.join("detour");
        fs::create_dir(&detour).expect("detour directory must be created");
        let owner = root.join("Owner.xml");
        let owner_alias = detour.join("..").join("Owner.xml");
        let before = b"<Owner><State>before</State></Owner>\n";
        let after = b"<Owner><State>after</State></Owner>\n";
        fs::write(&owner, before).expect("owner fixture must be written");

        let mut transaction = CompileTransaction::new();
        transaction
            .replace_bytes(&owner_alias, before, after.to_vec())
            .expect("replacement through alias must plan");
        transaction
            .guard_or_verify_exact_preimage(&owner, before)
            .expect("same normalized preimage must reuse the replacement guard");
        transaction.commit().expect("transaction must commit");

        assert_eq!(fs::read(&owner).unwrap(), after);
        fs::remove_dir_all(root).expect("temporary root must be removed");
    }

    #[test]
    fn binding_many_exact_read_guards_normalizes_each_path_a_constant_number_of_times() {
        let root = temp_root("exact-read-guard-normalization-complexity");
        let file_count = 64usize;
        let paths = (0..file_count)
            .map(|index| {
                let path = root.join(format!("Input{index:03}.bsl"));
                fs::write(&path, b"stable").unwrap();
                path
            })
            .collect::<Vec<_>>();
        TEST_PATH_IDENTITY_NORMALIZATIONS.with(|count| count.set(0));
        let mut transaction = CompileTransaction::new();

        for path in &paths {
            transaction
                .guard_or_verify_exact_preimage(path, b"stable")
                .unwrap();
        }

        let normalization_count = TEST_PATH_IDENTITY_NORMALIZATIONS.with(Cell::get);
        assert!(
            normalization_count <= file_count * 4,
            "binding {file_count} guards performed {normalization_count} path normalizations"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exact_read_guard_serializes_with_owner_writer_and_rejects_stale_plan() {
        let root = temp_root("read-guard-concurrent-owner-writer");
        let owner = root.join("Owner.xml");
        let owner_before = b"<Owner><State>before</State></Owner>\n".to_vec();
        let owner_concurrent = b"<Owner><State>concurrent</State></Owner>\n".to_vec();
        let target = root.join("Generated.xml");
        fs::write(&owner, &owner_before).unwrap();

        let mut guarded_transaction = CompileTransaction::new();
        guarded_transaction
            .guard_exact_preimage(&owner, &owner_before)
            .expect("owner read guard must plan");
        guarded_transaction
            .create_bytes(&target, b"<Generated/>\n".to_vec())
            .expect("target create must plan");

        let mut owner_writer = CompileTransaction::new();
        owner_writer
            .replace_bytes(&owner, &owner_before, owner_concurrent.clone())
            .expect("owner replacement must plan");

        let acquired = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let acquired_by_writer = Arc::clone(&acquired);
        let release_writer = Arc::clone(&release);
        let writer_thread = thread::spawn(move || {
            with_publication_lock_pause(acquired_by_writer, release_writer, || {
                owner_writer.commit()
            })
        });
        acquired.wait();

        let (contended_sender, contended_receiver) = mpsc::channel();
        let guarded_thread = thread::spawn(move || {
            with_publication_lock_contention_signal(contended_sender, || {
                guarded_transaction.commit()
            })
        });
        let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
        release.wait();

        writer_thread
            .join()
            .expect("owner writer thread must not panic")
            .expect("owner writer must commit");
        let error = guarded_thread
            .join()
            .expect("guarded transaction thread must not panic")
            .expect_err("stale guarded owner must reject the transaction");

        contention.expect("read guard must use the same publication lock as the owner writer");
        assert!(error.contains("read guard"), "{error}");
        assert!(error.contains("changed after planning"), "{error}");
        assert_eq!(fs::read(&owner).unwrap(), owner_concurrent);
        assert!(!target.exists());
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exact_read_guard_rechecks_after_writes_and_rolls_back_created_target() {
        let root = temp_root("read-guard-late-owner-change");
        let owner = root.join("Owner.xml");
        let owner_before = b"<Owner><State>before</State></Owner>\n".to_vec();
        let owner_concurrent = b"<Owner><State>concurrent</State></Owner>\n".to_vec();
        let target = root.join("Generated.xml");
        fs::write(&owner, &owner_before).unwrap();

        let mut transaction = CompileTransaction::new();
        transaction
            .guard_exact_preimage(&owner, &owner_before)
            .expect("owner read guard must plan");
        transaction
            .create_bytes(&target, b"<Generated/>\n".to_vec())
            .expect("target create must plan");
        let owner_for_hook = owner.clone();
        let target_for_hook = target.clone();
        let concurrent_for_hook = owner_concurrent.clone();

        let result = with_before_commit_hook(
            move |path| {
                assert_eq!(path, target_for_hook);
                fs::write(&owner_for_hook, &concurrent_for_hook).unwrap();
            },
            || transaction.commit(),
        );
        let error = result.expect_err("late guarded owner change must roll the transaction back");

        assert!(error.contains("read guard"), "{error}");
        assert!(error.contains("changed after planning"), "{error}");
        assert_eq!(fs::read(&owner).unwrap(), owner_concurrent);
        assert!(!target.exists());
        assert!(transaction_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
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
    fn subtree_removal_excludes_a_stale_unchanged_descendant_transaction() {
        let root = temp_root("subtree-removal-excludes-descendant");
        let tree = root.join("tree");
        let child = tree.join("child.xml");
        let outside = root.join("outside.xml");
        fs::create_dir_all(&tree).unwrap();
        let original = b"<Child><State>original</State></Child>\n";
        fs::write(&child, original).unwrap();

        let mut descendant_transaction = CompileTransaction::new();
        descendant_transaction
            .replace_bytes(&child, original, original.to_vec())
            .unwrap();
        descendant_transaction
            .create_bytes(&outside, b"<Outside/>\n".to_vec())
            .unwrap();

        let mut removal_transaction = CompileTransaction::new();
        removal_transaction.remove_path(&tree).unwrap();

        let acquired = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let acquired_by_removal = Arc::clone(&acquired);
        let release_removal = Arc::clone(&release);
        let removal_thread = thread::spawn(move || {
            with_publication_lock_pause(acquired_by_removal, release_removal, || {
                removal_transaction.commit()
            })
        });
        acquired.wait();

        let (contended_sender, contended_receiver) = mpsc::channel();
        let descendant_thread = thread::spawn(move || {
            with_publication_lock_contention_signal(contended_sender, || {
                descendant_transaction.commit()
            })
        });
        let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
        release.wait();

        let removal_result = removal_thread
            .join()
            .expect("removal transaction thread must not panic");
        let descendant_result = descendant_thread
            .join()
            .expect("descendant transaction thread must not panic");

        contention.expect("descendant publication must contend on the subtree-removal gate");
        removal_result.expect("subtree removal must commit");
        let error = descendant_result
            .expect_err("stale descendant preimage must fail after subtree removal");
        assert!(
            error.contains("invalid publication target")
                || error.contains("failed to inspect")
                || error.contains("No such file"),
            "{error}"
        );
        assert!(!tree.exists());
        assert!(!outside.exists());
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

use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_sources::{ProjectSourceMap, SourceFormat, SourceSetKind};
use crate::infrastructure::platform::filesystem::{
    is_link_loop_error, FileIdentity, RetainedChildCapability, RetainedDirectoryCapability,
    RetainedRegularFileCapability,
};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const SELECTION_READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_ACTOR_EXACT_RETAINED_BYTES: usize = 32 * 1024 * 1024;
const MAX_ACTOR_EVIDENCE_RECORDS: usize = 65_536;
const MAX_ACTOR_ENUMERATED_MEMBERS: usize = 16_384;
const MAX_ACTOR_UNIQUE_RETAINED_DIRECTORIES: usize = 128;
const MAX_ACTOR_ROUTE_AND_NAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
struct SelectionEvidenceBudgets {
    exact_bytes: usize,
    exact_work_bytes: usize,
    evidence_records: usize,
    enumerated_members: usize,
    unique_directories: usize,
    route_and_name_bytes: usize,
}

impl SelectionEvidenceBudgets {
    fn actor_admission() -> Self {
        let production = Self {
            exact_bytes: MAX_ACTOR_EXACT_RETAINED_BYTES,
            exact_work_bytes: MAX_ACTOR_EXACT_RETAINED_BYTES,
            evidence_records: MAX_ACTOR_EVIDENCE_RECORDS,
            enumerated_members: MAX_ACTOR_ENUMERATED_MEMBERS,
            unique_directories: MAX_ACTOR_UNIQUE_RETAINED_DIRECTORIES,
            route_and_name_bytes: MAX_ACTOR_ROUTE_AND_NAME_BYTES,
        };
        #[cfg(test)]
        if let Some(test) = actor_selection_test_budgets() {
            return Self {
                exact_bytes: test.exact_bytes,
                exact_work_bytes: test.exact_work_bytes,
                evidence_records: test.evidence_records,
                enumerated_members: test.enumerated_members,
                unique_directories: test.unique_directories,
                route_and_name_bytes: test.route_and_name_bytes,
            };
        }
        production
    }
}

struct SelectionEvidenceUsage {
    budgets: SelectionEvidenceBudgets,
    exact_retained_bytes: usize,
    exact_work_bytes: usize,
    evidence_records: usize,
    enumerated_members: usize,
    unique_directories: usize,
    route_and_name_bytes: usize,
}

impl SelectionEvidenceUsage {
    fn actor_admission() -> Self {
        Self {
            budgets: SelectionEvidenceBudgets::actor_admission(),
            exact_retained_bytes: 0,
            exact_work_bytes: 0,
            evidence_records: 0,
            enumerated_members: 0,
            unique_directories: 0,
            route_and_name_bytes: 0,
        }
    }

    fn ensure_exact_bytes(&self, bytes: usize) -> Result<(), String> {
        ensure_budget(
            self.exact_retained_bytes,
            bytes,
            self.budgets.exact_bytes,
            "project source-map actor exact-byte budget",
            "bytes",
        )
    }

    fn commit_exact_bytes(&mut self, bytes: usize) {
        self.exact_retained_bytes += bytes;
    }

    fn charge_exact_work(&mut self, bytes: usize) -> Result<(), String> {
        ensure_budget(
            self.exact_work_bytes,
            bytes,
            self.budgets.exact_work_bytes,
            "project source-map actor exact-work budget",
            "bytes",
        )?;
        self.exact_work_bytes += bytes;
        Ok(())
    }

    fn ensure_records(&self, records: usize) -> Result<(), String> {
        ensure_budget(
            self.evidence_records,
            records,
            self.budgets.evidence_records,
            "project source-map actor evidence-record budget",
            "entries",
        )
    }

    fn commit_records(&mut self, records: usize) {
        self.evidence_records += records;
    }

    fn ensure_members(&self, members: usize) -> Result<(), String> {
        ensure_budget(
            self.enumerated_members,
            members,
            self.budgets.enumerated_members,
            "project source-map actor enumerated-member budget",
            "entries",
        )
    }

    fn commit_members(&mut self, members: usize) {
        self.enumerated_members += members;
    }

    fn remaining_members(&self) -> usize {
        self.budgets
            .enumerated_members
            .saturating_sub(self.enumerated_members)
    }

    fn remaining_records(&self) -> usize {
        self.budgets
            .evidence_records
            .saturating_sub(self.evidence_records)
    }

    fn remaining_directories(&self) -> usize {
        self.budgets
            .unique_directories
            .saturating_sub(self.unique_directories)
    }

    fn remaining_route_and_name_bytes(&self) -> usize {
        self.budgets
            .route_and_name_bytes
            .saturating_sub(self.route_and_name_bytes)
    }

    fn ensure_directory(&self, route_bytes: usize) -> Result<(), String> {
        ensure_budget(
            self.unique_directories,
            1,
            self.budgets.unique_directories,
            "project source-map actor unique-directory budget",
            "entries",
        )?;
        self.ensure_route_bytes(route_bytes)
    }

    fn commit_directory(&mut self, route_bytes: usize) {
        self.unique_directories += 1;
        self.evidence_records += 1;
        self.route_and_name_bytes += route_bytes;
    }

    fn ensure_route_bytes(&self, bytes: usize) -> Result<(), String> {
        ensure_budget(
            self.route_and_name_bytes,
            bytes,
            self.budgets.route_and_name_bytes,
            "project source-map actor route/name budget",
            "bytes",
        )
    }

    fn commit_route_bytes(&mut self, bytes: usize) {
        self.route_and_name_bytes += bytes;
    }
}

fn ensure_budget(
    current: usize,
    additional: usize,
    limit: usize,
    label: &str,
    unit: &str,
) -> Result<(), String> {
    if current
        .checked_add(additional)
        .is_none_or(|total| total > limit)
    {
        Err(format!("{label} exceeds {limit} {unit}"))
    } else {
        Ok(())
    }
}

fn retained_route_bytes(path: &Path) -> usize {
    path.as_os_str().as_encoded_bytes().len()
}

fn retained_name_bytes(name: &std::ffi::OsStr) -> usize {
    name.as_encoded_bytes().len()
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct ActorSelectionTestBudgets {
    exact_bytes: usize,
    exact_work_bytes: usize,
    evidence_records: usize,
    enumerated_members: usize,
    unique_directories: usize,
    route_and_name_bytes: usize,
}

#[cfg(test)]
impl ActorSelectionTestBudgets {
    const fn generous() -> Self {
        Self {
            exact_bytes: usize::MAX,
            exact_work_bytes: usize::MAX,
            evidence_records: usize::MAX,
            enumerated_members: usize::MAX,
            unique_directories: usize::MAX,
            route_and_name_bytes: usize::MAX,
        }
    }
}

#[cfg(test)]
thread_local! {
    static ACTOR_SELECTION_TEST_BUDGETS: std::cell::Cell<Option<ActorSelectionTestBudgets>> = const {
        std::cell::Cell::new(None)
    };
    static PASS_COMPARISON_TEST_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    static REGULAR_PATH_CHILD_OPEN_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static REGULAR_EXACT_READ_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static REGULAR_EXACT_READ_BYTES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static REGULAR_EXACT_MAX_BUFFER_BYTES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static REGULAR_EXACT_AFTER_READ_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    static MEMBERSHIP_ENUMERATION_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static MEMBERSHIP_CHILD_OPEN_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
struct ActorSelectionTestBudgetGuard;

#[cfg(test)]
impl Drop for ActorSelectionTestBudgetGuard {
    fn drop(&mut self) {
        ACTOR_SELECTION_TEST_BUDGETS.with(|slot| slot.set(None));
    }
}

#[cfg(test)]
fn install_actor_selection_test_budgets(
    budgets: ActorSelectionTestBudgets,
) -> ActorSelectionTestBudgetGuard {
    ACTOR_SELECTION_TEST_BUDGETS.with(|slot| {
        assert!(slot.replace(Some(budgets)).is_none());
    });
    ActorSelectionTestBudgetGuard
}

#[cfg(test)]
#[allow(dead_code)]
fn actor_selection_test_budgets() -> Option<ActorSelectionTestBudgets> {
    ACTOR_SELECTION_TEST_BUDGETS.with(std::cell::Cell::get)
}

#[cfg(test)]
struct PassComparisonTestHookGuard;

#[cfg(test)]
impl Drop for PassComparisonTestHookGuard {
    fn drop(&mut self) {
        PASS_COMPARISON_TEST_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
fn install_pass_comparison_test_hook(hook: impl FnOnce() + 'static) -> PassComparisonTestHookGuard {
    PASS_COMPARISON_TEST_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
    PassComparisonTestHookGuard
}

#[cfg(test)]
#[allow(dead_code)]
fn run_pass_comparison_test_hook() {
    PASS_COMPARISON_TEST_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn record_regular_path_child_open_attempt() {
    REGULAR_PATH_CHILD_OPEN_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
}

#[cfg(test)]
fn reset_regular_path_child_open_attempts() {
    REGULAR_PATH_CHILD_OPEN_ATTEMPTS.with(|attempts| attempts.set(0));
}

#[cfg(test)]
fn regular_path_child_open_attempts() -> usize {
    REGULAR_PATH_CHILD_OPEN_ATTEMPTS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn reset_regular_exact_read_metrics() {
    REGULAR_EXACT_READ_ATTEMPTS.with(|attempts| attempts.set(0));
    REGULAR_EXACT_READ_BYTES.with(|bytes| bytes.set(0));
    REGULAR_EXACT_MAX_BUFFER_BYTES.with(|bytes| bytes.set(0));
}

#[cfg(test)]
fn record_regular_exact_read_attempt() {
    REGULAR_EXACT_READ_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
}

#[cfg(test)]
fn record_regular_exact_read_bytes(bytes: usize) {
    REGULAR_EXACT_READ_BYTES.with(|observed| observed.set(observed.get() + bytes));
}

#[cfg(test)]
fn record_regular_exact_buffer_bytes(bytes: usize) {
    REGULAR_EXACT_MAX_BUFFER_BYTES.with(|observed| observed.set(observed.get().max(bytes)));
}

#[cfg(test)]
fn regular_exact_read_metrics() -> (usize, usize, usize) {
    (
        REGULAR_EXACT_READ_ATTEMPTS.with(std::cell::Cell::get),
        REGULAR_EXACT_READ_BYTES.with(std::cell::Cell::get),
        REGULAR_EXACT_MAX_BUFFER_BYTES.with(std::cell::Cell::get),
    )
}

#[cfg(test)]
struct RegularExactAfterReadHookGuard;

#[cfg(test)]
impl Drop for RegularExactAfterReadHookGuard {
    fn drop(&mut self) {
        REGULAR_EXACT_AFTER_READ_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
fn install_regular_exact_after_read_hook(
    hook: impl FnOnce() + 'static,
) -> RegularExactAfterReadHookGuard {
    REGULAR_EXACT_AFTER_READ_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace(Box::new(hook)).is_none());
    });
    RegularExactAfterReadHookGuard
}

#[cfg(test)]
fn run_regular_exact_after_read_hook() {
    REGULAR_EXACT_AFTER_READ_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn reset_membership_test_metrics() {
    MEMBERSHIP_ENUMERATION_ATTEMPTS.with(|attempts| attempts.set(0));
    MEMBERSHIP_CHILD_OPEN_ATTEMPTS.with(|attempts| attempts.set(0));
}

#[cfg(test)]
fn record_membership_enumeration_attempt() {
    MEMBERSHIP_ENUMERATION_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
}

#[cfg(test)]
fn record_membership_child_open_attempt() {
    MEMBERSHIP_CHILD_OPEN_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
}

#[cfg(test)]
fn membership_test_metrics() -> (usize, usize) {
    (
        MEMBERSHIP_ENUMERATION_ATTEMPTS.with(std::cell::Cell::get),
        MEMBERSHIP_CHILD_OPEN_ATTEMPTS.with(std::cell::Cell::get),
    )
}

pub(in crate::infrastructure) struct ResolvedProjectSourceAdmission {
    map: ProjectSourceMap,
    evidence: RetainedSourceSelectionEvidence,
}

pub(in crate::infrastructure) struct RetainedSourceSelectionEvidence {
    workspace: RetainedDirectoryCapability,
    directories: BTreeMap<PathBuf, RetainedDirectoryCapability>,
    paths: BTreeMap<PathBuf, RetainedPathObservation>,
    memberships: BTreeMap<PathBuf, Vec<SelectionMember>>,
    admitted_semantic: CompleteProjectSourceSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompleteProjectSourceSelection {
    workspace_root: PathBuf,
    config_present: bool,
    configured_format_raw: Option<String>,
    source_sets: Vec<CompleteSourceSetSelection>,
    effective_source_set: Option<String>,
    effective_source_root: Option<String>,
    source_selection_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CompleteSourceSetSelection {
    name: String,
    kind: SourceSetKind,
    path: String,
    source_format: SourceFormat,
    format_evidence: Vec<String>,
    format_probe_error: Option<String>,
}

enum RetainedPathObservation {
    RegularFile {
        parent: PathBuf,
        name: OsString,
        identity: FileIdentity,
        bytes: Option<Arc<[u8]>>,
    },
    OversizedRegularFile {
        parent: PathBuf,
        name: OsString,
        identity: FileIdentity,
        maximum: usize,
    },
    UnresolvedRoute {
        ancestor: PathBuf,
        route: Vec<OsString>,
        classification: SelectionPathClassification,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionPathClassification {
    Absent,
    Directory,
    RegularFile,
    ReparseOrUnsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectionMember {
    name: OsString,
    kind: SelectionMemberKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionMemberKind {
    Directory,
    RegularFile,
    ReparseOrUnsupported,
}

struct SelectionPassSnapshot {
    workspace: FileIdentity,
    directories: BTreeMap<PathBuf, FileIdentity>,
    paths: BTreeMap<PathBuf, SnapshotPathObservation>,
    memberships: BTreeMap<PathBuf, Vec<SelectionMember>>,
}

enum SnapshotPathObservation {
    RegularFile {
        parent: PathBuf,
        name: OsString,
        identity: FileIdentity,
        bytes: Option<Arc<[u8]>>,
    },
    OversizedRegularFile {
        parent: PathBuf,
        name: OsString,
        identity: FileIdentity,
        maximum: usize,
    },
    UnresolvedRoute {
        ancestor: PathBuf,
        route: Vec<OsString>,
        classification: SelectionPathClassification,
    },
}

pub(in crate::infrastructure) enum RetainedRegularObservation {
    Exact(Arc<[u8]>),
    Present,
    Oversized,
    Absent,
    WrongKind,
}

pub(in crate::infrastructure) enum RetainedDirectoryObservation {
    Present(RetainedDirectoryCapability),
    AbsentOrWrongKind,
}

pub(in crate::infrastructure) struct RetainedMembershipEntry {
    pub(in crate::infrastructure) name: OsString,
    pub(in crate::infrastructure) directory: Option<RetainedDirectoryCapability>,
}

pub(in crate::infrastructure) struct RetainedSelectionPass {
    workspace: RetainedDirectoryCapability,
    directories: BTreeMap<PathBuf, RetainedDirectoryCapability>,
    paths: BTreeMap<PathBuf, RetainedPathObservation>,
    memberships: BTreeMap<PathBuf, Vec<SelectionMember>>,
    usage: SelectionEvidenceUsage,
}

impl std::fmt::Debug for ResolvedProjectSourceAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedProjectSourceAdmission")
            .field("source_sets", &self.map.source_sets.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RetainedSourceSelectionEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetainedSourceSelectionEvidence")
            .field(
                "input_count",
                &(self.paths.len() + self.memberships.len() + self.directories.len()),
            )
            .finish_non_exhaustive()
    }
}

impl ResolvedProjectSourceAdmission {
    pub(in crate::infrastructure) fn map(&self) -> &ProjectSourceMap {
        &self.map
    }

    pub(in crate::infrastructure) fn source_root_identity(
        &self,
        relative: &Path,
    ) -> Option<FileIdentity> {
        self.evidence.source_root_identity(relative)
    }

    pub(in crate::infrastructure) fn into_evidence(self) -> RetainedSourceSelectionEvidence {
        self.evidence
    }
}

pub(in crate::infrastructure) fn discover_project_source_admission(
    workspace_root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ResolvedProjectSourceAdmission, String> {
    checkpoint()?;
    let canonical_workspace =
        crate::infrastructure::source_roots::normalize_path_identity(workspace_root)?;
    let workspace = RetainedDirectoryCapability::open(&canonical_workspace).map_err(|error| {
        format!("project source-map workspace cannot be retained without following links: {error}")
    })?;
    workspace
        .validate_named_identity()
        .map_err(|error| format!("project source-map workspace identity changed: {error}"))?;

    let (first_map, first_pass) =
        crate::infrastructure::project_sources::discover_project_source_map_actor_pass(
            workspace.clone(),
            checkpoint,
        )?;
    let first_semantic = CompleteProjectSourceSelection::from_map(&first_map, workspace.path());
    drop(first_map);
    let first_snapshot = first_pass.into_snapshot();
    checkpoint()?;
    let (second_map, second_pass) =
        crate::infrastructure::project_sources::discover_project_source_map_actor_pass(
            workspace.clone(),
            checkpoint,
        )?;
    workspace
        .validate_named_identity()
        .map_err(|error| format!("project source-map workspace identity changed: {error}"))?;

    let second_semantic = CompleteProjectSourceSelection::from_map(&second_map, workspace.path());
    #[cfg(test)]
    run_pass_comparison_test_hook();
    checkpoint()?;
    if !first_semantic.matches_checkpointed(&second_semantic, checkpoint)?
        || !first_snapshot.matches_pass_checkpointed(&second_pass, checkpoint)?
    {
        return Err("project source-map changed during retained actor admission".to_string());
    }
    checkpoint()?;
    Ok(ResolvedProjectSourceAdmission {
        map: second_map,
        evidence: RetainedSourceSelectionEvidence {
            workspace,
            directories: second_pass.directories,
            paths: second_pass.paths,
            memberships: second_pass.memberships,
            admitted_semantic: second_semantic,
        },
    })
}

impl CompleteProjectSourceSelection {
    fn from_map(map: &ProjectSourceMap, workspace_root: &Path) -> Self {
        let mut source_sets = map
            .source_sets
            .iter()
            .map(|source| CompleteSourceSetSelection {
                name: source.name.clone(),
                kind: source.kind,
                path: source.path.clone(),
                source_format: source.source_format,
                format_evidence: source.format_evidence.clone(),
                format_probe_error: source.format_probe_error.clone(),
            })
            .collect::<Vec<_>>();
        source_sets.sort();
        Self {
            workspace_root: workspace_root.to_path_buf(),
            config_present: map.config_path.is_some(),
            configured_format_raw: map.configured_format_raw.clone(),
            source_sets,
            effective_source_set: map.effective_source_set.clone(),
            effective_source_root: map.effective_source_root.clone(),
            source_selection_error: map.source_selection_error.clone(),
        }
    }
}

impl RetainedSelectionPass {
    pub(in crate::infrastructure) fn new(
        workspace: RetainedDirectoryCapability,
    ) -> Result<Self, String> {
        let mut pass = Self {
            workspace: workspace.clone(),
            directories: BTreeMap::new(),
            paths: BTreeMap::new(),
            memberships: BTreeMap::new(),
            usage: SelectionEvidenceUsage::actor_admission(),
        };
        pass.record_directory(PathBuf::new(), workspace)?;
        Ok(pass)
    }

    pub(in crate::infrastructure) fn workspace_path(&self) -> &Path {
        self.workspace.path()
    }

    pub(in crate::infrastructure) fn observe_regular(
        &mut self,
        relative: &Path,
        max_bytes: usize,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<RetainedRegularObservation, String> {
        self.observe_regular_internal(relative, max_bytes, true, checkpoint)
    }

    pub(in crate::infrastructure) fn observe_regular_presence(
        &mut self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<RetainedRegularObservation, String> {
        self.observe_regular_internal(relative, 0, false, checkpoint)
    }

    fn observe_regular_internal(
        &mut self,
        relative: &Path,
        max_bytes: usize,
        capture_bytes: bool,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<RetainedRegularObservation, String> {
        checkpoint()?;
        let components = normal_components(relative)?;
        let Some((name, parents)) = components.split_last() else {
            return Ok(RetainedRegularObservation::WrongKind);
        };
        let mut ancestor = self.workspace.clone();
        for (index, component) in parents.iter().enumerate() {
            checkpoint()?;
            let prefix = relative_prefix(&components, index + 1);
            match self.retain_directory_component(&ancestor, &prefix, component)? {
                Ok(RetainedChildCapability::Directory(directory)) => {
                    ancestor = directory;
                }
                Ok(child) => {
                    self.record_unresolved(
                        relative_prefix(&components, index),
                        relative,
                        &components[index..],
                        classification_of_child(&child),
                    )?;
                    return Ok(RetainedRegularObservation::WrongKind);
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    self.record_unresolved(
                        relative_prefix(&components, index),
                        relative,
                        &components[index..],
                        SelectionPathClassification::Absent,
                    )?;
                    return Ok(RetainedRegularObservation::Absent);
                }
                Err(error) if open_error_is_wrong_kind(&error) => {
                    self.record_unresolved(
                        relative_prefix(&components, index),
                        relative,
                        &components[index..],
                        SelectionPathClassification::ReparseOrUnsupported,
                    )?;
                    return Ok(RetainedRegularObservation::WrongKind);
                }
                Err(error) => {
                    return Err(format!(
                        "project source-map route {} cannot be inspected: {error}",
                        relative.display()
                    ));
                }
            }
        }
        checkpoint()?;
        if !self.paths.contains_key(relative) {
            let parent = relative_prefix(&components, parents.len());
            let route_bytes = retained_route_bytes(relative)
                .saturating_add(retained_route_bytes(&parent))
                .saturating_add(retained_name_bytes(name));
            self.usage.ensure_records(1)?;
            self.usage.ensure_route_bytes(route_bytes)?;
        }
        #[cfg(test)]
        record_regular_path_child_open_attempt();
        match ancestor.retain_immediate_child_nofollow(name) {
            Ok(RetainedChildCapability::RegularFile(file)) => {
                file.validate_named_identity().map_err(|error| {
                    format!(
                        "project source-map input {} identity changed before read: {error}",
                        relative.display()
                    )
                })?;
                let parent = relative_prefix(&components, parents.len());
                let identity = file.identity();
                let (existing_bytes, existing_oversized) = match self.paths.get(relative) {
                    Some(RetainedPathObservation::RegularFile {
                        parent: expected_parent,
                        name: expected_name,
                        identity: expected_identity,
                        bytes,
                    }) if *expected_parent == parent
                        && *expected_name == *name
                        && *expected_identity == identity =>
                    {
                        (Some(bytes.clone()), false)
                    }
                    Some(RetainedPathObservation::OversizedRegularFile {
                        parent: expected_parent,
                        name: expected_name,
                        identity: expected_identity,
                        maximum,
                    }) if *expected_parent == parent
                        && *expected_name == *name
                        && *expected_identity == identity
                        && *maximum == max_bytes =>
                    {
                        (None, true)
                    }
                    Some(_) => return Err(observation_changed(relative)),
                    None => (None, false),
                };
                let bytes = if capture_bytes {
                    let file_bytes = usize::try_from(
                        file.try_clone_file()
                            .and_then(|file| file.metadata())
                            .map_err(|error| {
                                format!(
                                    "project source-map input {} length cannot be inspected: {error}",
                                    relative.display()
                                )
                            })?
                            .len(),
                    )
                    .map_err(|_| {
                        format!(
                            "project source-map input {} length is not representable",
                            relative.display()
                        )
                    })?;
                    self.usage.charge_exact_work(file_bytes)?;
                    if file_bytes > max_bytes {
                        file.validate_named_identity().map_err(|error| {
                            format!(
                                "project source-map input {} identity changed after length probe: {error}",
                                relative.display()
                            )
                        })?;
                        drop(file);
                        self.record_oversized_regular(
                            relative,
                            parent,
                            name.clone(),
                            identity,
                            max_bytes,
                        )?;
                        return Ok(RetainedRegularObservation::Oversized);
                    }
                    if existing_oversized {
                        return Err(observation_changed(relative));
                    }
                    if let Some(Some(canonical)) = existing_bytes {
                        if !stream_regular_exact_matches(
                            &file,
                            canonical.as_ref(),
                            max_bytes,
                            checkpoint,
                        )? {
                            return Err(observation_changed(relative));
                        }
                        Some(canonical)
                    } else {
                        self.usage.ensure_exact_bytes(file_bytes)?;
                        Some(Arc::<[u8]>::from(read_regular_exact(
                            &file, max_bytes, checkpoint,
                        )?))
                    }
                } else {
                    None
                };
                file.validate_named_identity().map_err(|error| {
                    format!(
                        "project source-map input {} identity changed after read: {error}",
                        relative.display()
                    )
                })?;
                drop(file);
                self.record_regular(
                    relative,
                    parent,
                    name.clone(),
                    identity,
                    bytes.clone(),
                    checkpoint,
                )?;
                Ok(match bytes {
                    Some(bytes) => RetainedRegularObservation::Exact(bytes),
                    None => RetainedRegularObservation::Present,
                })
            }
            Ok(child) => {
                self.record_unresolved(
                    relative_prefix(&components, parents.len()),
                    relative,
                    std::slice::from_ref(name),
                    classification_of_child(&child),
                )?;
                Ok(RetainedRegularObservation::WrongKind)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.record_unresolved(
                    relative_prefix(&components, parents.len()),
                    relative,
                    std::slice::from_ref(name),
                    SelectionPathClassification::Absent,
                )?;
                Ok(RetainedRegularObservation::Absent)
            }
            Err(error) if open_error_is_wrong_kind(&error) => {
                self.record_unresolved(
                    relative_prefix(&components, parents.len()),
                    relative,
                    std::slice::from_ref(name),
                    SelectionPathClassification::ReparseOrUnsupported,
                )?;
                Ok(RetainedRegularObservation::WrongKind)
            }
            Err(error) => Err(format!(
                "project source-map input {} cannot be inspected: {error}",
                relative.display()
            )),
        }
    }

    pub(in crate::infrastructure) fn observe_directory(
        &mut self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<RetainedDirectoryObservation, String> {
        checkpoint()?;
        let components = normal_components(relative)?;
        let mut ancestor = self.workspace.clone();
        for (index, component) in components.iter().enumerate() {
            checkpoint()?;
            let prefix = relative_prefix(&components, index + 1);
            match self.retain_directory_component(&ancestor, &prefix, component)? {
                Ok(RetainedChildCapability::Directory(directory)) => {
                    ancestor = directory;
                }
                Ok(child) => {
                    self.record_unresolved(
                        relative_prefix(&components, index),
                        relative,
                        &components[index..],
                        classification_of_child(&child),
                    )?;
                    return Ok(RetainedDirectoryObservation::AbsentOrWrongKind);
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    self.record_unresolved(
                        relative_prefix(&components, index),
                        relative,
                        &components[index..],
                        SelectionPathClassification::Absent,
                    )?;
                    return Ok(RetainedDirectoryObservation::AbsentOrWrongKind);
                }
                Err(error) if open_error_is_wrong_kind(&error) => {
                    self.record_unresolved(
                        relative_prefix(&components, index),
                        relative,
                        &components[index..],
                        SelectionPathClassification::ReparseOrUnsupported,
                    )?;
                    return Ok(RetainedDirectoryObservation::AbsentOrWrongKind);
                }
                Err(error) => {
                    return Err(format!(
                        "project source-map directory {} cannot be inspected: {error}",
                        relative.display()
                    ));
                }
            }
        }
        ancestor.validate_named_identity().map_err(|error| {
            format!(
                "project source-map directory {} identity changed: {error}",
                relative.display()
            )
        })?;
        Ok(RetainedDirectoryObservation::Present(ancestor))
    }

    pub(in crate::infrastructure) fn observe_membership(
        &mut self,
        relative: &Path,
        directory: &RetainedDirectoryCapability,
        limit: usize,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<Vec<RetainedMembershipEntry>, String> {
        directory.validate_named_identity().map_err(|error| {
            format!(
                "project source-map directory {} identity changed before enumeration: {error}",
                relative.display()
            )
        })?;
        let new_membership = !self.memberships.contains_key(relative);
        if new_membership {
            let requested_records = 1_usize.saturating_add(limit);
            if requested_records > self.usage.remaining_records() {
                self.usage.ensure_records(requested_records)?;
            }
            let minimum_requested_route_bytes =
                retained_route_bytes(relative).saturating_add(limit);
            if minimum_requested_route_bytes > self.usage.remaining_route_and_name_bytes() {
                self.usage
                    .ensure_route_bytes(minimum_requested_route_bytes)?;
            }
        }
        let remaining_members = self.usage.remaining_members();
        if limit > remaining_members {
            self.usage.ensure_members(limit)?;
        }
        let enumeration_limit = limit.saturating_add(1);
        let mut checkpoint_error = None;
        #[cfg(test)]
        record_membership_enumeration_attempt();
        let names =
            directory.read_immediate_names_bounded(enumeration_limit, || match checkpoint() {
                Ok(()) => Ok(()),
                Err(reason) => {
                    checkpoint_error = Some(reason);
                    Err(std::io::Error::new(
                        ErrorKind::Interrupted,
                        "project source-map checkpoint stopped enumeration",
                    ))
                }
            });
        if let Some(reason) = checkpoint_error {
            return Err(reason);
        }
        let names = names.map_err(|error| {
            if limit == remaining_members {
                format!(
                    "project source-map actor enumerated-member budget exceeds {} entries",
                    self.usage.budgets.enumerated_members
                )
            } else {
                format!(
                    "project source-map directory {} cannot be enumerated: {error}",
                    relative.display()
                )
            }
        })?;
        self.usage.ensure_members(names.len())?;
        let membership_records = 1_usize.saturating_add(names.len());
        let membership_route_bytes = retained_route_bytes(relative).saturating_add(
            names
                .iter()
                .map(|name| retained_name_bytes(name))
                .sum::<usize>(),
        );
        if new_membership {
            self.usage.ensure_records(membership_records)?;
            self.usage.ensure_route_bytes(membership_route_bytes)?;
        }
        let mut members = Vec::with_capacity(names.len());
        let mut result = Vec::with_capacity(names.len());
        for name in names {
            checkpoint()?;
            let child_route = relative.join(&name);
            if !self.directories.contains_key(&child_route)
                && self.usage.remaining_directories() == 0
            {
                self.usage
                    .ensure_directory(retained_route_bytes(&child_route))?;
            }
            #[cfg(test)]
            record_membership_child_open_attempt();
            let (kind, child_directory) = match directory.retain_immediate_child_nofollow(&name) {
                Ok(RetainedChildCapability::Directory(child)) => {
                    self.record_directory(child_route, child.clone())?;
                    (SelectionMemberKind::Directory, Some(child))
                }
                Ok(RetainedChildCapability::RegularFile(_)) => {
                    (SelectionMemberKind::RegularFile, None)
                }
                Ok(
                    RetainedChildCapability::ReparsePoint | RetainedChildCapability::Unsupported,
                ) => (SelectionMemberKind::ReparseOrUnsupported, None),
                Err(error) if open_error_is_wrong_kind(&error) => {
                    (SelectionMemberKind::ReparseOrUnsupported, None)
                }
                Err(error) => {
                    return Err(format!(
                        "project source-map directory member {} changed during enumeration: {error}",
                        Path::new(relative).join(&name).display()
                    ));
                }
            };
            members.push(SelectionMember {
                name: name.clone(),
                kind,
            });
            result.push(RetainedMembershipEntry {
                name,
                directory: child_directory,
            });
        }
        directory.validate_named_identity().map_err(|error| {
            format!(
                "project source-map directory {} identity changed after enumeration: {error}",
                relative.display()
            )
        })?;
        match self.memberships.get(relative) {
            Some(existing) if existing == &members => {}
            Some(_) => return Err(observation_changed(relative)),
            None => {
                self.usage.ensure_records(membership_records)?;
                self.usage.ensure_route_bytes(membership_route_bytes)?;
                self.memberships.insert(relative.to_path_buf(), members);
                self.usage.commit_records(membership_records);
                self.usage.commit_route_bytes(membership_route_bytes);
            }
        }
        self.usage.commit_members(result.len());
        Ok(result)
    }

    fn retain_directory_component(
        &mut self,
        ancestor: &RetainedDirectoryCapability,
        relative: &Path,
        name: &std::ffi::OsStr,
    ) -> Result<Result<RetainedChildCapability, std::io::Error>, String> {
        if let Some(existing) = self.directories.get(relative) {
            existing
                .validate_named_identity()
                .map_err(|_| observation_changed(relative))?;
            return Ok(Ok(RetainedChildCapability::Directory(existing.clone())));
        }
        self.usage.ensure_records(1)?;
        self.usage
            .ensure_directory(retained_route_bytes(relative))?;
        let child = ancestor.retain_immediate_child_nofollow(name);
        if let Ok(RetainedChildCapability::Directory(directory)) = &child {
            self.record_directory(relative.to_path_buf(), directory.clone())?;
        }
        Ok(child)
    }

    fn record_directory(
        &mut self,
        relative: PathBuf,
        directory: RetainedDirectoryCapability,
    ) -> Result<(), String> {
        match self.directories.get(&relative) {
            Some(existing) if existing.identity() == directory.identity() => Ok(()),
            Some(_) => Err(observation_changed(&relative)),
            None => {
                let route_bytes = retained_route_bytes(&relative);
                self.usage.ensure_records(1)?;
                self.usage.ensure_directory(route_bytes)?;
                self.directories.insert(relative, directory);
                self.usage.commit_directory(route_bytes);
                Ok(())
            }
        }
    }

    fn record_unresolved(
        &mut self,
        ancestor: PathBuf,
        relative: &Path,
        route: &[OsString],
        classification: SelectionPathClassification,
    ) -> Result<(), String> {
        let existing_matches = self.paths.get(relative).is_some_and(|existing| {
            matches!(
                existing,
                RetainedPathObservation::UnresolvedRoute {
                    ancestor: expected_ancestor,
                    route: expected_route,
                    classification: expected_classification,
                } if expected_ancestor == &ancestor
                    && expected_route == route
                    && *expected_classification == classification
            )
        });
        match self.paths.get(relative) {
            Some(_) if existing_matches => Ok(()),
            Some(_) => Err(observation_changed(relative)),
            None => {
                let route_bytes = retained_route_bytes(relative)
                    .saturating_add(retained_route_bytes(&ancestor))
                    .saturating_add(
                        route
                            .iter()
                            .map(|name| retained_name_bytes(name))
                            .sum::<usize>(),
                    );
                self.usage.ensure_records(1)?;
                self.usage.ensure_route_bytes(route_bytes)?;
                let observed = RetainedPathObservation::UnresolvedRoute {
                    ancestor: ancestor.clone(),
                    route: route.to_vec(),
                    classification,
                };
                self.paths.insert(relative.to_path_buf(), observed);
                self.usage.commit_records(1);
                self.usage.commit_route_bytes(route_bytes);
                Ok(())
            }
        }
    }

    fn record_regular(
        &mut self,
        relative: &Path,
        parent: PathBuf,
        name: OsString,
        identity: FileIdentity,
        bytes: Option<Arc<[u8]>>,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        let observed = RetainedPathObservation::RegularFile {
            parent: parent.clone(),
            name: name.clone(),
            identity,
            bytes: bytes.clone(),
        };
        if let Some(existing) = self.paths.get_mut(relative) {
            match (existing, observed) {
                (
                    RetainedPathObservation::RegularFile {
                        parent: expected_parent,
                        name: expected_name,
                        identity: expected_identity,
                        bytes: expected_bytes,
                    },
                    RetainedPathObservation::RegularFile {
                        parent,
                        name,
                        identity,
                        bytes,
                    },
                ) if *expected_parent == parent
                    && *expected_name == name
                    && *expected_identity == identity =>
                {
                    match (expected_bytes.as_ref(), bytes) {
                        (Some(expected), Some(current)) => {
                            if !checkpointed_bytes_equal(expected, &current, checkpoint)? {
                                return Err(observation_changed(relative));
                            }
                        }
                        (None, Some(current)) => {
                            self.usage.ensure_exact_bytes(current.len())?;
                            self.usage.commit_exact_bytes(current.len());
                            *expected_bytes = Some(current);
                        }
                        (Some(_), None) | (None, None) => {}
                    }
                    Ok(())
                }
                _ => Err(observation_changed(relative)),
            }
        } else {
            let route_bytes = retained_route_bytes(relative)
                .saturating_add(retained_route_bytes(&parent))
                .saturating_add(retained_name_bytes(&name));
            self.usage.ensure_records(1)?;
            self.usage.ensure_route_bytes(route_bytes)?;
            if let Some(bytes) = &bytes {
                self.usage.ensure_exact_bytes(bytes.len())?;
            }
            self.paths.insert(relative.to_path_buf(), observed);
            self.usage.commit_records(1);
            self.usage.commit_route_bytes(route_bytes);
            if let Some(bytes) = bytes {
                self.usage.commit_exact_bytes(bytes.len());
            }
            Ok(())
        }
    }

    fn record_oversized_regular(
        &mut self,
        relative: &Path,
        parent: PathBuf,
        name: OsString,
        identity: FileIdentity,
        maximum: usize,
    ) -> Result<(), String> {
        let existing_matches = matches!(
            self.paths.get(relative),
            Some(RetainedPathObservation::OversizedRegularFile {
                parent: expected_parent,
                name: expected_name,
                identity: expected_identity,
                maximum: expected_maximum,
            }) if *expected_parent == parent
                && *expected_name == name
                && *expected_identity == identity
                && *expected_maximum == maximum
        );
        match self.paths.get(relative) {
            Some(_) if existing_matches => Ok(()),
            Some(_) => Err(observation_changed(relative)),
            None => {
                let route_bytes = retained_route_bytes(relative)
                    .saturating_add(retained_route_bytes(&parent))
                    .saturating_add(retained_name_bytes(&name));
                self.usage.ensure_records(1)?;
                self.usage.ensure_route_bytes(route_bytes)?;
                self.paths.insert(
                    relative.to_path_buf(),
                    RetainedPathObservation::OversizedRegularFile {
                        parent,
                        name,
                        identity,
                        maximum,
                    },
                );
                self.usage.commit_records(1);
                self.usage.commit_route_bytes(route_bytes);
                Ok(())
            }
        }
    }

    fn into_snapshot(self) -> SelectionPassSnapshot {
        SelectionPassSnapshot {
            workspace: self.workspace.identity(),
            directories: self
                .directories
                .into_iter()
                .map(|(relative, directory)| (relative, directory.identity()))
                .collect(),
            paths: self
                .paths
                .into_iter()
                .map(|(relative, observation)| {
                    let snapshot = match observation {
                        RetainedPathObservation::RegularFile {
                            parent,
                            name,
                            identity,
                            bytes,
                        } => SnapshotPathObservation::RegularFile {
                            parent,
                            name,
                            identity,
                            bytes,
                        },
                        RetainedPathObservation::OversizedRegularFile {
                            parent,
                            name,
                            identity,
                            maximum,
                        } => SnapshotPathObservation::OversizedRegularFile {
                            parent,
                            name,
                            identity,
                            maximum,
                        },
                        RetainedPathObservation::UnresolvedRoute {
                            ancestor,
                            route,
                            classification,
                        } => SnapshotPathObservation::UnresolvedRoute {
                            ancestor,
                            route,
                            classification,
                        },
                    };
                    (relative, snapshot)
                })
                .collect(),
            memberships: self.memberships,
        }
    }

    #[cfg(test)]
    fn observation_counts_for_test(&self) -> (usize, usize, usize, usize) {
        let (regular, unresolved) = self
            .paths
            .values()
            .fold((0, 0), |counts, input| match input {
                RetainedPathObservation::RegularFile { .. }
                | RetainedPathObservation::OversizedRegularFile { .. } => (counts.0 + 1, counts.1),
                RetainedPathObservation::UnresolvedRoute { .. } => (counts.0, counts.1 + 1),
            });
        (
            self.directories.len(),
            regular,
            unresolved,
            self.memberships.len(),
        )
    }
}

fn observation_changed(relative: &Path) -> String {
    format!(
        "project source-map actor observation changed within one pass: {}",
        relative.display()
    )
}

impl CompleteProjectSourceSelection {
    fn matches_checkpointed(
        &self,
        other: &Self,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<bool, String> {
        checkpoint()?;
        if !checkpointed_path_equal(&self.workspace_root, &other.workspace_root, checkpoint)?
            || self.config_present != other.config_present
            || !checkpointed_optional_string_equal(
                self.configured_format_raw.as_deref(),
                other.configured_format_raw.as_deref(),
                checkpoint,
            )?
            || self.source_sets.len() != other.source_sets.len()
            || !checkpointed_optional_string_equal(
                self.effective_source_set.as_deref(),
                other.effective_source_set.as_deref(),
                checkpoint,
            )?
            || !checkpointed_optional_string_equal(
                self.effective_source_root.as_deref(),
                other.effective_source_root.as_deref(),
                checkpoint,
            )?
            || !checkpointed_optional_string_equal(
                self.source_selection_error.as_deref(),
                other.source_selection_error.as_deref(),
                checkpoint,
            )?
        {
            return Ok(false);
        }
        for (left, right) in self.source_sets.iter().zip(&other.source_sets) {
            checkpoint()?;
            if left.kind != right.kind
                || left.source_format != right.source_format
                || !checkpointed_bytes_equal(
                    left.name.as_bytes(),
                    right.name.as_bytes(),
                    checkpoint,
                )?
                || !checkpointed_bytes_equal(
                    left.path.as_bytes(),
                    right.path.as_bytes(),
                    checkpoint,
                )?
                || left.format_evidence.len() != right.format_evidence.len()
                || !checkpointed_optional_string_equal(
                    left.format_probe_error.as_deref(),
                    right.format_probe_error.as_deref(),
                    checkpoint,
                )?
            {
                return Ok(false);
            }
            for (left_evidence, right_evidence) in
                left.format_evidence.iter().zip(&right.format_evidence)
            {
                if !checkpointed_bytes_equal(
                    left_evidence.as_bytes(),
                    right_evidence.as_bytes(),
                    checkpoint,
                )? {
                    return Ok(false);
                }
            }
        }
        checkpoint()?;
        Ok(true)
    }
}

impl SelectionPassSnapshot {
    fn matches_pass_checkpointed(
        &self,
        pass: &RetainedSelectionPass,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<bool, String> {
        checkpoint()?;
        if self.workspace != pass.workspace.identity()
            || self.directories.len() != pass.directories.len()
            || self.paths.len() != pass.paths.len()
            || self.memberships.len() != pass.memberships.len()
        {
            return Ok(false);
        }
        for ((left_route, left_identity), (right_route, right_directory)) in
            self.directories.iter().zip(&pass.directories)
        {
            checkpoint()?;
            if !checkpointed_path_equal(left_route, right_route, checkpoint)?
                || *left_identity != right_directory.identity()
            {
                return Ok(false);
            }
        }
        for ((left_route, left), (right_route, right)) in self.paths.iter().zip(&pass.paths) {
            checkpoint()?;
            if !checkpointed_path_equal(left_route, right_route, checkpoint)?
                || !snapshot_path_matches(left, right, checkpoint)?
            {
                return Ok(false);
            }
        }
        for ((left_route, left), (right_route, right)) in
            self.memberships.iter().zip(&pass.memberships)
        {
            checkpoint()?;
            if !checkpointed_path_equal(left_route, right_route, checkpoint)?
                || left.len() != right.len()
            {
                return Ok(false);
            }
            for (left_member, right_member) in left.iter().zip(right) {
                checkpoint()?;
                if left_member.kind != right_member.kind
                    || !checkpointed_bytes_equal(
                        left_member.name.as_encoded_bytes(),
                        right_member.name.as_encoded_bytes(),
                        checkpoint,
                    )?
                {
                    return Ok(false);
                }
            }
        }
        checkpoint()?;
        Ok(true)
    }
}

fn snapshot_path_matches(
    snapshot: &SnapshotPathObservation,
    retained: &RetainedPathObservation,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    match (snapshot, retained) {
        (
            SnapshotPathObservation::RegularFile {
                parent: left_parent,
                name: left_name,
                identity: left_identity,
                bytes: left_bytes,
            },
            RetainedPathObservation::RegularFile {
                parent: right_parent,
                name: right_name,
                identity: right_identity,
                bytes: right_bytes,
            },
        ) => {
            if *left_identity != *right_identity
                || !checkpointed_path_equal(left_parent, right_parent, checkpoint)?
                || !checkpointed_bytes_equal(
                    left_name.as_encoded_bytes(),
                    right_name.as_encoded_bytes(),
                    checkpoint,
                )?
            {
                return Ok(false);
            }
            match (left_bytes, right_bytes) {
                (Some(left), Some(right)) => checkpointed_bytes_equal(left, right, checkpoint),
                (None, None) => Ok(true),
                _ => Ok(false),
            }
        }
        (
            SnapshotPathObservation::OversizedRegularFile {
                parent: left_parent,
                name: left_name,
                identity: left_identity,
                maximum: left_maximum,
            },
            RetainedPathObservation::OversizedRegularFile {
                parent: right_parent,
                name: right_name,
                identity: right_identity,
                maximum: right_maximum,
            },
        ) => Ok(*left_identity == *right_identity
            && *left_maximum == *right_maximum
            && checkpointed_path_equal(left_parent, right_parent, checkpoint)?
            && checkpointed_bytes_equal(
                left_name.as_encoded_bytes(),
                right_name.as_encoded_bytes(),
                checkpoint,
            )?),
        (
            SnapshotPathObservation::UnresolvedRoute {
                ancestor: left_ancestor,
                route: left_route,
                classification: left_classification,
            },
            RetainedPathObservation::UnresolvedRoute {
                ancestor: right_ancestor,
                route: right_route,
                classification: right_classification,
            },
        ) => {
            if left_classification != right_classification
                || left_route.len() != right_route.len()
                || !checkpointed_path_equal(left_ancestor, right_ancestor, checkpoint)?
            {
                return Ok(false);
            }
            for (left, right) in left_route.iter().zip(right_route) {
                if !checkpointed_bytes_equal(
                    left.as_encoded_bytes(),
                    right.as_encoded_bytes(),
                    checkpoint,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn checkpointed_optional_string_equal(
    left: Option<&str>,
    right: Option<&str>,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    match (left, right) {
        (Some(left), Some(right)) => {
            checkpointed_bytes_equal(left.as_bytes(), right.as_bytes(), checkpoint)
        }
        (None, None) => Ok(true),
        _ => Ok(false),
    }
}

fn checkpointed_path_equal(
    left: &Path,
    right: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    checkpointed_bytes_equal(
        left.as_os_str().as_encoded_bytes(),
        right.as_os_str().as_encoded_bytes(),
        checkpoint,
    )
}

fn checkpointed_bytes_equal(
    left: &[u8],
    right: &[u8],
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left
        .chunks(SELECTION_READ_CHUNK_BYTES)
        .zip(right.chunks(SELECTION_READ_CHUNK_BYTES))
    {
        checkpoint()?;
        if left != right {
            return Ok(false);
        }
        checkpoint()?;
    }
    Ok(true)
}

impl RetainedSourceSelectionEvidence {
    pub(in crate::infrastructure) fn validate(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceSelectionEvidenceError> {
        self.validate_complete_pass(deadline, cancellation)?;
        self.validate_complete_pass(deadline, cancellation)
    }

    fn validate_complete_pass(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceSelectionEvidenceError> {
        selection_checkpoint(deadline, cancellation)?;
        self.workspace
            .validate_named_identity()
            .map_err(changed_identity)?;
        if self.admitted_semantic.workspace_root != self.workspace.path() {
            return Err(SourceSelectionEvidenceError::provider(
                "project source-map semantic workspace does not match retained authority",
            ));
        }
        for directory in self.directories.values() {
            selection_checkpoint(deadline, cancellation)?;
            directory
                .validate_named_identity()
                .map_err(changed_identity)?;
            selection_checkpoint(deadline, cancellation)?;
        }
        for observation in self.paths.values() {
            selection_checkpoint(deadline, cancellation)?;
            match observation {
                RetainedPathObservation::RegularFile {
                    parent,
                    name,
                    identity,
                    bytes,
                } => {
                    let parent = self.directories.get(parent).ok_or_else(|| {
                        SourceSelectionEvidenceError::provider(
                            "project source-map retained file parent authority is unavailable",
                        )
                    })?;
                    parent.validate_named_identity().map_err(changed_identity)?;
                    let file = match parent.retain_immediate_child_nofollow(name) {
                        Ok(RetainedChildCapability::RegularFile(file)) => file,
                        Ok(_) => {
                            return Err(SourceSelectionEvidenceError::changed(
                                "project source-map retained file kind changed",
                            ));
                        }
                        Err(error)
                            if error.kind() == ErrorKind::NotFound
                                || open_error_is_wrong_kind(&error) =>
                        {
                            return Err(SourceSelectionEvidenceError::changed(
                                "project source-map retained file changed",
                            ));
                        }
                        Err(error) => {
                            return Err(SourceSelectionEvidenceError::provider(format!(
                                "project source-map retained file cannot be reopened: {error}"
                            )));
                        }
                    };
                    if file.identity() != *identity {
                        return Err(SourceSelectionEvidenceError::changed(
                            "project source-map retained file identity changed",
                        ));
                    }
                    if let Some(bytes) = bytes {
                        validate_regular_bytes(&file, bytes, deadline, cancellation)?;
                    }
                    file.validate_named_identity_relative()
                        .map_err(changed_identity)?;
                    drop(file);
                    parent.validate_named_identity().map_err(changed_identity)?;
                }
                RetainedPathObservation::OversizedRegularFile {
                    parent,
                    name,
                    identity,
                    maximum,
                } => {
                    let parent = self.directories.get(parent).ok_or_else(|| {
                        SourceSelectionEvidenceError::provider(
                            "project source-map retained oversized-file parent authority is unavailable",
                        )
                    })?;
                    parent.validate_named_identity().map_err(changed_identity)?;
                    let file = match parent.retain_immediate_child_nofollow(name) {
                        Ok(RetainedChildCapability::RegularFile(file)) => file,
                        Ok(_) => {
                            return Err(SourceSelectionEvidenceError::changed(
                                "project source-map oversized terminal file kind changed",
                            ));
                        }
                        Err(error)
                            if error.kind() == ErrorKind::NotFound
                                || open_error_is_wrong_kind(&error) =>
                        {
                            return Err(SourceSelectionEvidenceError::changed(
                                "project source-map oversized terminal file changed",
                            ));
                        }
                        Err(error) => {
                            return Err(SourceSelectionEvidenceError::provider(format!(
                                "project source-map oversized terminal file cannot be reopened: {error}"
                            )));
                        }
                    };
                    if file.identity() != *identity {
                        return Err(SourceSelectionEvidenceError::changed(
                            "project source-map oversized terminal file identity changed",
                        ));
                    }
                    let current_length = file
                        .try_clone_file()
                        .and_then(|file| file.metadata())
                        .map_err(changed_identity)?
                        .len();
                    if current_length <= *maximum as u64 {
                        return Err(SourceSelectionEvidenceError::changed(
                            "project source-map oversized terminal classification changed",
                        ));
                    }
                    file.validate_named_identity_relative()
                        .map_err(changed_identity)?;
                    drop(file);
                    parent.validate_named_identity().map_err(changed_identity)?;
                }
                RetainedPathObservation::UnresolvedRoute {
                    ancestor,
                    route,
                    classification,
                } => {
                    let ancestor = self.directories.get(ancestor).ok_or_else(|| {
                        SourceSelectionEvidenceError::provider(
                            "project source-map unresolved ancestor authority is unavailable",
                        )
                    })?;
                    validate_unresolved_route(ancestor, route, *classification)?;
                }
            }
            selection_checkpoint(deadline, cancellation)?;
        }
        for (relative, members) in &self.memberships {
            selection_checkpoint(deadline, cancellation)?;
            let directory = self.directories.get(relative).ok_or_else(|| {
                SourceSelectionEvidenceError::provider(
                    "project source-map membership directory authority is unavailable",
                )
            })?;
            validate_membership(directory, members, deadline, cancellation)?;
            selection_checkpoint(deadline, cancellation)?;
        }
        self.workspace
            .validate_named_identity()
            .map_err(changed_identity)?;
        selection_checkpoint(deadline, cancellation)
    }

    fn source_root_identity(&self, relative: &Path) -> Option<FileIdentity> {
        if relative.as_os_str().is_empty() || relative == Path::new(".") {
            return Some(self.workspace.identity());
        }
        self.directories
            .get(relative)
            .map(|directory| directory.identity())
    }

    pub(in crate::infrastructure) fn validate_dry_result(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceSelectionEvidenceError> {
        self.validate(deadline, cancellation)
    }

    pub(in crate::infrastructure) fn validate_final(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), SourceSelectionEvidenceError> {
        self.validate(deadline, cancellation)
    }
}

fn validate_membership(
    directory: &RetainedDirectoryCapability,
    expected: &[SelectionMember],
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), SourceSelectionEvidenceError> {
    directory
        .validate_named_identity()
        .map_err(changed_identity)?;
    let mut checkpoint_error = None;
    let names = directory.read_immediate_names_bounded(expected.len().saturating_add(1), || {
        match selection_checkpoint(deadline, cancellation) {
            Ok(()) => Ok(()),
            Err(error) => {
                checkpoint_error = Some(error);
                Err(std::io::Error::new(
                    ErrorKind::Interrupted,
                    "project source-map membership validation stopped",
                ))
            }
        }
    });
    if let Some(error) = checkpoint_error {
        return Err(error);
    }
    let names = names.map_err(|error| {
        SourceSelectionEvidenceError::changed(format!(
            "project source-map retained directory membership changed: {error}"
        ))
    })?;
    let mut observed = Vec::with_capacity(names.len());
    for name in names {
        selection_checkpoint(deadline, cancellation)?;
        let kind = match directory.retain_immediate_child_nofollow(&name) {
            Ok(RetainedChildCapability::Directory(_)) => SelectionMemberKind::Directory,
            Ok(RetainedChildCapability::RegularFile(_)) => SelectionMemberKind::RegularFile,
            Ok(RetainedChildCapability::ReparsePoint | RetainedChildCapability::Unsupported) => {
                SelectionMemberKind::ReparseOrUnsupported
            }
            Err(error) if open_error_is_wrong_kind(&error) => {
                SelectionMemberKind::ReparseOrUnsupported
            }
            Err(error) => {
                return Err(SourceSelectionEvidenceError::changed(format!(
                    "project source-map retained directory member changed: {error}"
                )));
            }
        };
        observed.push(SelectionMember { name, kind });
    }
    if observed == expected {
        directory
            .validate_named_identity()
            .map_err(changed_identity)
    } else {
        Err(SourceSelectionEvidenceError::changed(
            "project source-map retained directory membership changed",
        ))
    }
}

fn validate_unresolved_route(
    ancestor: &RetainedDirectoryCapability,
    route: &[OsString],
    expected: SelectionPathClassification,
) -> Result<(), SourceSelectionEvidenceError> {
    ancestor
        .validate_named_identity()
        .map_err(changed_identity)?;
    let Some(name) = route.first() else {
        return Err(SourceSelectionEvidenceError::provider(
            "project source-map unresolved route is empty",
        ));
    };
    let observed = match ancestor.retain_immediate_child_nofollow(name) {
        Ok(child) => classification_of_child(&child),
        Err(error) if error.kind() == ErrorKind::NotFound => SelectionPathClassification::Absent,
        Err(error) if open_error_is_wrong_kind(&error) => {
            SelectionPathClassification::ReparseOrUnsupported
        }
        Err(error) => {
            return Err(SourceSelectionEvidenceError::provider(format!(
                "project source-map unresolved route cannot be inspected: {error}"
            )));
        }
    };
    ancestor
        .validate_named_identity()
        .map_err(changed_identity)?;
    if observed == expected {
        Ok(())
    } else {
        Err(SourceSelectionEvidenceError::changed(
            "project source-map unresolved route changed",
        ))
    }
}

fn normal_components(relative: &Path) -> Result<Vec<OsString>, String> {
    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => components.push(name.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "project source-map route is not a closed workspace-relative path: {}",
                    relative.display()
                ));
            }
        }
    }
    Ok(components)
}

fn relative_prefix(components: &[OsString], count: usize) -> PathBuf {
    components.iter().take(count).collect()
}

fn classification_of_child(child: &RetainedChildCapability) -> SelectionPathClassification {
    match child {
        RetainedChildCapability::Directory(_) => SelectionPathClassification::Directory,
        RetainedChildCapability::RegularFile(_) => SelectionPathClassification::RegularFile,
        RetainedChildCapability::ReparsePoint | RetainedChildCapability::Unsupported => {
            SelectionPathClassification::ReparseOrUnsupported
        }
    }
}

fn open_error_is_wrong_kind(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::InvalidInput | ErrorKind::InvalidData | ErrorKind::NotADirectory
    ) || is_link_loop_error(error)
}

fn stream_regular_exact_matches(
    file: &RetainedRegularFileCapability,
    expected: &[u8],
    max_bytes: usize,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    let mut file = file
        .try_clone_file()
        .map_err(|error| format!("project source-map input cannot be cloned: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("project source-map input cannot be rewound: {error}"))?;
    let mut chunk = [0_u8; SELECTION_READ_CHUNK_BYTES];
    let mut offset = 0_usize;
    loop {
        checkpoint()?;
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("project source-map input cannot be read: {error}"))?;
        if read == 0 {
            return Ok(offset == expected.len());
        }
        let Some(end) = offset.checked_add(read) else {
            return Ok(false);
        };
        if end > max_bytes || end > expected.len() || chunk[..read] != expected[offset..end] {
            return Ok(false);
        }
        offset = end;
        checkpoint()?;
    }
}

fn read_regular_exact(
    file: &RetainedRegularFileCapability,
    max_bytes: usize,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    #[cfg(test)]
    record_regular_exact_read_attempt();
    let mut file = file
        .try_clone_file()
        .map_err(|error| format!("project source-map input cannot be cloned: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("project source-map input cannot be rewound: {error}"))?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; SELECTION_READ_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("project source-map input cannot be read: {error}"))?;
        if read == 0 {
            break;
        }
        #[cfg(test)]
        {
            record_regular_exact_read_bytes(read);
            run_regular_exact_after_read_hook();
        }
        let next_len = bytes
            .len()
            .checked_add(read)
            .ok_or_else(|| format!("project source-map input exceeds {max_bytes} bytes"))?;
        if next_len > max_bytes {
            return Err(format!(
                "project source-map input exceeds {max_bytes} bytes"
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
        #[cfg(test)]
        record_regular_exact_buffer_bytes(bytes.len());
        checkpoint()?;
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::infrastructure) enum SourceSelectionEvidenceErrorKind {
    Cancelled,
    Deadline,
    Changed,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::infrastructure) struct SourceSelectionEvidenceError {
    kind: SourceSelectionEvidenceErrorKind,
    message: String,
}

impl SourceSelectionEvidenceError {
    pub(in crate::infrastructure) const fn kind(&self) -> SourceSelectionEvidenceErrorKind {
        self.kind
    }

    fn changed(message: impl Into<String>) -> Self {
        Self {
            kind: SourceSelectionEvidenceErrorKind::Changed,
            message: message.into(),
        }
    }

    fn provider(message: impl Into<String>) -> Self {
        Self {
            kind: SourceSelectionEvidenceErrorKind::Provider,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SourceSelectionEvidenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for SourceSelectionEvidenceError {}

fn selection_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), SourceSelectionEvidenceError> {
    if cancellation.is_cancelled() {
        Err(SourceSelectionEvidenceError {
            kind: SourceSelectionEvidenceErrorKind::Cancelled,
            message: "project source-map validation cancelled".to_string(),
        })
    } else if deadline.remaining().is_zero() {
        Err(SourceSelectionEvidenceError {
            kind: SourceSelectionEvidenceErrorKind::Deadline,
            message: "project source-map validation deadline exceeded".to_string(),
        })
    } else {
        Ok(())
    }
}

fn changed_identity(error: std::io::Error) -> SourceSelectionEvidenceError {
    SourceSelectionEvidenceError::changed(format!(
        "project source-map retained identity changed: {error}"
    ))
}

fn validate_regular_bytes(
    file: &RetainedRegularFileCapability,
    expected: &[u8],
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), SourceSelectionEvidenceError> {
    selection_checkpoint(deadline, cancellation)?;
    let mut file = file.try_clone_file().map_err(changed_identity)?;
    file.seek(SeekFrom::Start(0)).map_err(changed_identity)?;
    let mut chunk = [0_u8; SELECTION_READ_CHUNK_BYTES];
    let mut offset = 0_usize;
    loop {
        selection_checkpoint(deadline, cancellation)?;
        let read = file.read(&mut chunk).map_err(changed_identity)?;
        if read == 0 {
            break;
        }
        let end = offset.saturating_add(read);
        if end > expected.len() || chunk[..read] != expected[offset..end] {
            return Err(SourceSelectionEvidenceError::changed(
                "project source-map retained file bytes changed",
            ));
        }
        offset = end;
        selection_checkpoint(deadline, cancellation)?;
    }
    if offset == expected.len() {
        Ok(())
    } else {
        Err(SourceSelectionEvidenceError::changed(
            "project source-map retained file length changed",
        ))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn write(path: &Path, bytes: impl AsRef<[u8]>) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn configured_workspace(label: &str) -> tempfile::TempDir {
        let root = tempfile::Builder::new().prefix(label).tempdir().unwrap();
        write(
            &root.path().join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        );
        write(
            &root.path().join("src/Configuration.xml"),
            b"<MetaDataObject><Configuration/></MetaDataObject>",
        );
        root
    }

    #[test]
    pub(crate) fn actor_admission_rejects_aggregate_exact_byte_budget() {
        let root = configured_workspace("unica-source-selection-byte-budget");
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            exact_bytes: 32,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut checkpoint = || Ok(());

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("actor admission ignored its aggregate exact-byte budget");

        assert_eq!(
            error,
            "project source-map actor exact-byte budget exceeds 32 bytes"
        );
    }

    #[test]
    pub(crate) fn actor_admission_charges_repeated_exact_work_before_second_read() {
        let root = tempfile::tempdir().unwrap();
        let config = concat!(
            "format: EDT\n",
            "source-set:\n",
            "  - name: first\n    type: EXTERNAL_DATA_PROCESSORS\n    path: shared\n",
            "  - name: second\n    type: EXTERNAL_DATA_PROCESSORS\n    path: shared\n",
        );
        write(&root.path().join("v8project.yaml"), config);
        write(&root.path().join("shared/ConfigDumpInfo.xml"), b"12345678");
        let exact_work_limit = config.len() + 8;
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            exact_work_bytes: exact_work_limit,
            ..ActorSelectionTestBudgets::generous()
        });
        reset_regular_exact_read_metrics();
        let mut checkpoint = || Ok(());

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("second source-set exact observation escaped the pass-global work budget");

        assert_eq!(
            error,
            format!("project source-map actor exact-work budget exceeds {exact_work_limit} bytes")
        );
        let (attempts, bytes, _) = regular_exact_read_metrics();
        assert_eq!(
            attempts, 2,
            "second exact observation started a content read"
        );
        assert_eq!(
            bytes, exact_work_limit,
            "second exact observation read content before work-budget rejection"
        );
    }

    #[test]
    pub(crate) fn actor_admission_bounds_unique_retained_directories_without_ulimit() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: first\n    type: CONFIGURATION\n    path: first\n",
                "  - name: second\n    type: EXTENSION\n    path: second\n",
            ),
        );
        std::fs::create_dir(root.path().join("first")).unwrap();
        std::fs::create_dir(root.path().join("second")).unwrap();
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            unique_directories: 2,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut checkpoint = || Ok(());

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("actor admission retained more unique directory handles than its budget");

        assert_eq!(
            error,
            "project source-map actor unique-directory budget exceeds 2 entries"
        );

        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let mut pass = RetainedSelectionPass::new(workspace).unwrap();
        assert!(matches!(
            pass.observe_directory(Path::new("first"), &mut checkpoint)
                .unwrap(),
            RetainedDirectoryObservation::Present(_)
        ));
        let error = match pass.observe_directory(Path::new("second"), &mut checkpoint) {
            Ok(_) => panic!("a third unique retained directory escaped the budget"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            "project source-map actor unique-directory budget exceeds 2 entries"
        );
        assert_eq!(
            pass.observation_counts_for_test().0,
            2,
            "retained directory evidence exceeded its deterministic handle budget"
        );
    }

    #[test]
    pub(crate) fn actor_admission_bounds_global_membership_across_external_source_sets() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
                "  - name: reports\n",
                "    type: EXTERNAL_REPORTS\n",
                "    path: erf\n",
            ),
        );
        for relative in [
            "epf/first.xml",
            "epf/second.xml",
            "erf/third.xml",
            "erf/fourth.xml",
        ] {
            std::fs::create_dir_all(root.path().join(relative)).unwrap();
        }
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            enumerated_members: 3,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut checkpoint = || Ok(());

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("per-source-set enumeration reset the actor's global membership budget");

        assert_eq!(
            error,
            "project source-map actor enumerated-member budget exceeds 3 entries"
        );
    }

    #[test]
    pub(crate) fn actor_admission_counts_repeated_membership_enumeration_globally() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: first\n    type: EXTERNAL_REPORTS\n    path: shared\n",
                "  - name: second\n    type: EXTERNAL_REPORTS\n    path: shared\n",
                "  - name: third\n    type: EXTERNAL_REPORTS\n    path: shared\n",
            ),
        );
        for relative in ["shared/first.xml", "shared/second.xml"] {
            std::fs::create_dir_all(root.path().join(relative)).unwrap();
        }
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            enumerated_members: 4,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut checkpoint = || Ok(());

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("repeated membership enumeration escaped the actor-global work budget");

        assert_eq!(
            error,
            "project source-map actor enumerated-member budget exceeds 4 entries"
        );
    }

    #[test]
    pub(crate) fn retained_selection_pass_checks_membership_budget_before_enumeration() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("member.xml"), b"member");
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            evidence_records: 1,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut pass = RetainedSelectionPass::new(workspace.clone()).unwrap();
        let mut checkpoints = 0_usize;
        let mut checkpoint = || {
            checkpoints += 1;
            Ok(())
        };

        let error = pass
            .observe_membership(Path::new(""), &workspace, 8, &mut checkpoint)
            .err()
            .expect("membership enumeration happened despite an exhausted record budget");

        assert_eq!(
            error,
            "project source-map actor evidence-record budget exceeds 1 entries"
        );
        assert_eq!(checkpoints, 0, "membership was enumerated before rejection");
    }

    #[test]
    pub(crate) fn retained_selection_pass_checks_remaining_record_capacity_before_enumeration() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("a"), b"a");
        write(&root.path().join("b"), b"b");
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            evidence_records: 2,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut pass = RetainedSelectionPass::new(workspace.clone()).unwrap();
        reset_membership_test_metrics();
        let mut checkpoint = || Ok(());

        let error = pass
            .observe_membership(Path::new(""), &workspace, 2, &mut checkpoint)
            .err()
            .expect("membership enumerated with only its record header slot remaining");

        assert_eq!(
            error,
            "project source-map actor evidence-record budget exceeds 2 entries"
        );
        assert_eq!(
            membership_test_metrics().0,
            0,
            "membership was enumerated before its full requested record cost was admitted"
        );
    }

    #[test]
    pub(crate) fn retained_selection_pass_checks_remaining_name_capacity_before_enumeration() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("a"), b"a");
        write(&root.path().join("b"), b"b");
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            route_and_name_bytes: 1,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut pass = RetainedSelectionPass::new(workspace.clone()).unwrap();
        reset_membership_test_metrics();
        let mut checkpoint = || Ok(());

        let error = pass
            .observe_membership(Path::new(""), &workspace, 2, &mut checkpoint)
            .err()
            .expect("membership enumerated beyond its requested route/name capacity");

        assert_eq!(
            error,
            "project source-map actor route/name budget exceeds 1 bytes"
        );
        assert_eq!(
            membership_test_metrics().0,
            0,
            "membership was enumerated before its requested name capacity was admitted"
        );
    }

    #[test]
    pub(crate) fn retained_selection_pass_rejects_before_unseen_member_child_open() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("child")).unwrap();
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            unique_directories: 1,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut pass = RetainedSelectionPass::new(workspace.clone()).unwrap();
        reset_membership_test_metrics();
        let mut checkpoint = || Ok(());

        let error = pass
            .observe_membership(Path::new(""), &workspace, 1, &mut checkpoint)
            .err()
            .expect("membership opened a child after unique-directory capacity was exhausted");

        assert_eq!(
            error,
            "project source-map actor unique-directory budget exceeds 1 entries"
        );
        assert_eq!(
            membership_test_metrics().1,
            0,
            "an unseen member child capability was opened before directory admission"
        );
    }

    #[test]
    pub(crate) fn retained_exact_read_never_appends_a_growth_chunk_past_the_limit() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("marker.xml");
        write(&marker, b"1234");
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let mut pass = RetainedSelectionPass::new(workspace).unwrap();
        reset_regular_exact_read_metrics();
        let grow_marker = marker.clone();
        let _hook = install_regular_exact_after_read_hook(move || {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(grow_marker)
                .unwrap();
            std::io::Write::write_all(&mut file, b"5678").unwrap();
        });
        let mut checkpoint = || Ok(());

        let error = pass
            .observe_regular(Path::new("marker.xml"), 4, &mut checkpoint)
            .err()
            .expect("concurrent file growth escaped the exact-read maximum");

        assert_eq!(error, "project source-map input exceeds 4 bytes");
        assert_eq!(
            regular_exact_read_metrics().2,
            4,
            "a concurrent growth chunk was appended before the exact-read limit check"
        );
    }

    #[test]
    pub(crate) fn retained_selection_pass_checks_record_budget_before_regular_open() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("marker.xml"), b"marker");
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            evidence_records: 1,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut pass = RetainedSelectionPass::new(workspace).unwrap();
        reset_regular_path_child_open_attempts();
        let mut checkpoint = || Ok(());

        let error = pass
            .observe_regular_presence(Path::new("marker.xml"), &mut checkpoint)
            .err()
            .expect("regular input was opened despite an exhausted record budget");

        assert_eq!(
            error,
            "project source-map actor evidence-record budget exceeds 1 entries"
        );
        assert_eq!(
            regular_path_child_open_attempts(),
            0,
            "regular input was opened before rejection"
        );
    }

    #[test]
    pub(crate) fn actor_admission_rejects_total_evidence_record_budget() {
        let root = configured_workspace("unica-source-selection-record-budget");
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            evidence_records: 3,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut checkpoint = || Ok(());

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("actor admission ignored its aggregate evidence-record budget");

        assert_eq!(
            error,
            "project source-map actor evidence-record budget exceeds 3 entries"
        );
    }

    #[test]
    pub(crate) fn actor_admission_rejects_route_and_name_byte_budget() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: retained-route-is-long\n",
            ),
        );
        std::fs::create_dir(root.path().join("retained-route-is-long")).unwrap();
        let _budgets = install_actor_selection_test_budgets(ActorSelectionTestBudgets {
            route_and_name_bytes: 8,
            ..ActorSelectionTestBudgets::generous()
        });
        let mut checkpoint = || Ok(());

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("actor admission ignored retained route/name bytes");

        assert_eq!(
            error,
            "project source-map actor route/name budget exceeds 8 bytes"
        );
    }

    #[test]
    pub(crate) fn actor_admission_comparison_honors_cancellation() {
        let root = configured_workspace("unica-source-selection-compare-cancel");
        let cancellation = CancellationToken::new();
        let cancel_during_compare = cancellation.clone();
        let _hook = install_pass_comparison_test_hook(move || {
            cancel_during_compare.cancel();
        });
        let mut checkpoint = || {
            if cancellation.is_cancelled() {
                Err("project source-map comparison cancelled".to_string())
            } else {
                Ok(())
            }
        };

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("pass comparison did not consult cancellation authority");

        assert_eq!(error, "project source-map comparison cancelled");
    }

    #[test]
    pub(crate) fn actor_admission_comparison_honors_deadline() {
        let root = configured_workspace("unica-source-selection-compare-deadline");
        let expired = Arc::new(AtomicBool::new(false));
        let expire_during_compare = Arc::clone(&expired);
        let _hook = install_pass_comparison_test_hook(move || {
            expire_during_compare.store(true, Ordering::SeqCst);
        });
        let mut checkpoint = || {
            if expired.load(Ordering::SeqCst) {
                Err("project source-map comparison deadline exceeded".to_string())
            } else {
                Ok(())
            }
        };

        let error = discover_project_source_admission(root.path(), &mut checkpoint)
            .expect_err("pass comparison did not consult deadline authority");

        assert_eq!(error, "project source-map comparison deadline exceeded");
    }

    #[test]
    pub(crate) fn retained_selection_pass_deduplicates_repeated_observations() {
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("source/marker.xml"), b"marker");
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let mut pass = RetainedSelectionPass::new(workspace).unwrap();
        let mut checkpoint = || Ok(());

        let directory = match pass
            .observe_directory(Path::new("source"), &mut checkpoint)
            .unwrap()
        {
            RetainedDirectoryObservation::Present(directory) => directory,
            RetainedDirectoryObservation::AbsentOrWrongKind => panic!("source directory absent"),
        };
        pass.observe_regular(Path::new("source/marker.xml"), 32, &mut checkpoint)
            .unwrap();
        pass.observe_membership(Path::new("source"), &directory, 8, &mut checkpoint)
            .unwrap();
        let once = pass.observation_counts_for_test();

        let repeated_directory = match pass
            .observe_directory(Path::new("source"), &mut checkpoint)
            .unwrap()
        {
            RetainedDirectoryObservation::Present(directory) => directory,
            RetainedDirectoryObservation::AbsentOrWrongKind => panic!("source directory absent"),
        };
        pass.observe_regular(Path::new("source/marker.xml"), 32, &mut checkpoint)
            .unwrap();
        pass.observe_membership(Path::new("source"), &repeated_directory, 8, &mut checkpoint)
            .unwrap();

        assert_eq!(
            pass.observation_counts_for_test(),
            once,
            "repeated identical observations consumed new evidence/handle records"
        );
    }

    #[test]
    pub(crate) fn retained_selection_pass_rejects_inconsistent_regular_repeat() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("marker.xml");
        write(&marker, b"first");
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let mut pass = RetainedSelectionPass::new(workspace).unwrap();
        let mut checkpoint = || Ok(());
        pass.observe_regular(Path::new("marker.xml"), 32, &mut checkpoint)
            .unwrap();
        write(&marker, b"second");

        let error = pass
            .observe_regular(Path::new("marker.xml"), 32, &mut checkpoint)
            .err()
            .expect("inconsistent repeated exact bytes were accepted");

        assert_eq!(
            error,
            "project source-map actor observation changed within one pass: marker.xml"
        );
    }

    #[test]
    pub(crate) fn retained_selection_pass_rejects_inconsistent_directory_repeat() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("source");
        std::fs::create_dir(&directory).unwrap();
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let mut pass = RetainedSelectionPass::new(workspace).unwrap();
        let mut checkpoint = || Ok(());
        pass.observe_directory(Path::new("source"), &mut checkpoint)
            .unwrap();
        std::fs::rename(&directory, root.path().join("displaced")).unwrap();
        std::fs::create_dir(&directory).unwrap();

        let error = pass
            .observe_directory(Path::new("source"), &mut checkpoint)
            .err()
            .expect("inconsistent repeated directory identity was accepted");

        assert_eq!(
            error,
            "project source-map actor observation changed within one pass: source"
        );
    }

    #[test]
    pub(crate) fn retained_selection_pass_rejects_inconsistent_membership_repeat() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("source");
        std::fs::create_dir(&directory).unwrap();
        let workspace =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(root.path()).unwrap())
                .unwrap();
        let mut pass = RetainedSelectionPass::new(workspace).unwrap();
        let mut checkpoint = || Ok(());
        let retained = match pass
            .observe_directory(Path::new("source"), &mut checkpoint)
            .unwrap()
        {
            RetainedDirectoryObservation::Present(directory) => directory,
            RetainedDirectoryObservation::AbsentOrWrongKind => panic!("source directory absent"),
        };
        pass.observe_membership(Path::new("source"), &retained, 8, &mut checkpoint)
            .unwrap();
        write(&directory.join("appeared.xml"), b"new");

        let error = pass
            .observe_membership(Path::new("source"), &retained, 8, &mut checkpoint)
            .err()
            .expect("inconsistent repeated membership was accepted");

        assert_eq!(
            error,
            "project source-map actor observation changed within one pass: source"
        );
    }
}

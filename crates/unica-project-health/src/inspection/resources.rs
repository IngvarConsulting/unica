use crate::inspection::git::{read_git_index, GitIndexDigest, GitIndexEntry, IndexReadError};
use crate::inspection::layout::InspectedSourceRoot;
use crate::inspection::{
    SourceRootOwnerIndex, SourceRootOwnerIndexError, SourceRootOwnerLookupError,
};
use crate::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthInspectionError,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;
#[cfg(test)]
use unica_process::runner::system_process_runner;
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
use unica_source_contract::SourceFormat;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::paths::host_path_text;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::read::ReadFile as File;
use unica_workspace::read::{
    open_absolute_read_directory_nofollow, open_any_read_child_nofollow,
    open_read_child_for_secure_tree_use, OpenedChildKind,
};

pub(crate) const LFS_SINGLE_FILE_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const LFS_AGGREGATE_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;
const MAX_STAGED_POLICY_FILES: usize = 1024;
const MAX_STAGED_POLICY_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_INDEX_EOL_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_INDEX_EOL_BATCH_FILES: usize = 64;
const MAX_INDEX_EOL_BATCH_BLOB_BYTES: usize = 32 * 1024 * 1024;
const MAX_CLASSIFIED_RESOURCES: usize = 65_536;
const MAX_RESOURCE_OWNER_EXPANSION_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESOURCE_OWNERSHIP_REASON_BYTES: usize = 512;
const MAX_CONTINUATION_WORKSPACES: usize = 4;
const MAX_CONTINUATION_EVIDENCE_BYTES: usize = 32 * 1024 * 1024;
const CONTINUATION_TTL: Duration = Duration::from_secs(15 * 60);

#[cfg(any(test, feature = "test-support"))]
type WorkingEolBlockHook = Box<dyn FnOnce() -> Result<(), WorkingEolInspectionError>>;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static BEFORE_WORKING_REVALIDATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static STAGED_EOL_STOP_AFTER: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    static AFTER_WORKING_EOL_BLOCK: std::cell::RefCell<Option<WorkingEolBlockHook>> =
        std::cell::RefCell::new(None);
}

#[cfg(any(test, feature = "test-support"))]
pub fn stop_working_eol_after_block_for_test() {
    AFTER_WORKING_EOL_BLOCK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(|| Err(WorkingEolInspectionError::TimedOut)));
    });
}

#[cfg(any(test, feature = "test-support"))]
fn run_working_revalidation_test_hook() {
    BEFORE_WORKING_REVALIDATION.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(any(test, feature = "test-support")))]
fn run_working_revalidation_test_hook() {}

#[cfg(any(test, feature = "test-support"))]
pub fn stop_staged_eol_after_for_test(completed: usize) {
    STAGED_EOL_STOP_AFTER.with(|slot| slot.set(Some(completed)));
}

#[cfg(any(test, feature = "test-support"))]
fn staged_eol_test_timeout() -> bool {
    STAGED_EOL_STOP_AFTER.with(|slot| match slot.get() {
        Some(0) => {
            slot.set(None);
            true
        }
        Some(remaining) => {
            slot.set(Some(remaining - 1));
            false
        }
        None => false,
    })
}

#[cfg(not(any(test, feature = "test-support")))]
fn staged_eol_test_timeout() -> bool {
    false
}

#[derive(Default)]
pub struct RootCheckContinuationStore {
    entries: Mutex<BTreeMap<PathBuf, ContinuationEntry>>,
}

struct ContinuationEntry {
    state: Arc<Mutex<ResourceContinuation>>,
    last_used: Instant,
}

impl RootCheckContinuationStore {
    pub fn for_workspace(&self, root: &Path) -> Option<Arc<Mutex<ResourceContinuation>>> {
        let mut entries = self.entries.lock().ok()?;
        let now = Instant::now();
        entries.retain(|_, entry| {
            now.duration_since(entry.last_used) < CONTINUATION_TTL
                || Arc::strong_count(&entry.state) > 1
        });
        if let Some(entry) = entries.get_mut(root) {
            entry.last_used = now;
            return Some(Arc::clone(&entry.state));
        }
        if entries.len() >= MAX_CONTINUATION_WORKSPACES {
            let oldest_idle = entries
                .iter()
                .filter(|(_, entry)| Arc::strong_count(&entry.state) == 1)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(root, _)| root.clone())?;
            entries.remove(&oldest_idle);
        }
        let state = Arc::new(Mutex::new(ResourceContinuation::default()));
        entries.insert(
            root.to_path_buf(),
            ContinuationEntry {
                state: Arc::clone(&state),
                last_used: now,
            },
        );
        Some(state)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ResourceInspectionProgress {
    pub text_resources: usize,
    pub staged_eol_retained: usize,
    pub working_eol_retained: usize,
    pub capacity_limited: bool,
}

#[derive(Default)]
pub struct ResourceContinuation {
    index_eol: BTreeMap<EvidenceKey, IndexEol>,
    working_eol: BTreeMap<EvidenceKey, WorkingEolEvidence>,
    evidence_bytes: usize,
    index_eol_batch_files: Option<usize>,
    pub(crate) progress: ResourceInspectionProgress,
    #[cfg(any(test, feature = "test-support"))]
    limit_override: Option<usize>,
}

type EvidenceKey = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexEol {
    Lf,
    CrLf,
    Mixed,
    None,
    Binary,
}

impl IndexEol {
    fn from_git(value: &str) -> Option<Self> {
        match value {
            "lf" => Some(Self::Lf),
            "crlf" => Some(Self::CrLf),
            "mixed" => Some(Self::Mixed),
            "none" => Some(Self::None),
            "-text" => Some(Self::Binary),
            _ => None,
        }
    }

    fn as_git(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::CrLf => "crlf",
            Self::Mixed => "mixed",
            Self::None => "none",
            Self::Binary => "-text",
        }
    }
}

#[derive(Clone)]
struct WorkingEolEvidence {
    fingerprint: WorkingFileFingerprint,
    eol: WorkingEolProgress,
}

#[derive(Clone)]
enum WorkingEolProgress {
    // Partial bytes are not evidence of readiness. Their original fingerprint
    // must still match when reopening the file before the next read.
    Partial(WorkingEolScanner),
    Complete(WorkingEol),
}

#[derive(Debug, Clone, Copy, Default)]
struct WorkingEolScanner {
    offset: u64,
    previous_cr: bool,
    lf: bool,
    crlf: bool,
    bare_cr: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkingFileFingerprint {
    identity: unica_workspace::read::FileIdentity,
    size: u64,
    modified: SystemTime,
    change_time: (i64, i64),
}

impl ResourceContinuation {
    pub fn progress(&self) -> ResourceInspectionProgress {
        self.progress
    }

    pub fn reset_progress(&mut self) {
        self.progress = ResourceInspectionProgress::default();
    }

    // BTreeMap nodes reserve up to 11 slots but may hold only five entries
    // after splits. Twice the fixed-size pair plus 64 bytes per entry covers
    // that occupancy, child pointers, headers, and allocator metadata.
    const INDEX_CHARGE: usize = 2 * std::mem::size_of::<(EvidenceKey, IndexEol)>() + 64;
    const WORKING_CHARGE: usize = 2 * std::mem::size_of::<(EvidenceKey, WorkingEolEvidence)>() + 64;

    #[cfg(any(test, feature = "test-support"))]
    pub fn working_eol_offset_for_test(&self, path: &str) -> Option<u64> {
        match self.working_eol.get(&working_evidence_key(path))?.eol {
            WorkingEolProgress::Partial(scanner) => Some(scanner.offset),
            WorkingEolProgress::Complete(_) => None,
        }
    }

    fn index_eol_batch_files(&self) -> usize {
        self.index_eol_batch_files
            .unwrap_or(MAX_INDEX_EOL_BATCH_FILES)
    }

    fn reduce_index_eol_batch(&mut self, failed_files: usize) {
        self.index_eol_batch_files = Some((failed_files / 2).max(1));
    }

    fn evidence_limit(&self) -> usize {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(limit) = self.limit_override {
            return limit;
        }
        MAX_CONTINUATION_EVIDENCE_BYTES
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_test_evidence_limit(&mut self, limit: usize) {
        self.limit_override = Some(limit);
    }

    fn retain_current(&mut self, index: &BTreeSet<EvidenceKey>, working: &BTreeSet<EvidenceKey>) {
        self.index_eol.retain(|key, _| index.contains(key));
        self.working_eol.retain(|key, _| working.contains(key));
        self.evidence_bytes = self
            .index_eol
            .len()
            .saturating_mul(Self::INDEX_CHARGE)
            .saturating_add(self.working_eol.len().saturating_mul(Self::WORKING_CHARGE));
    }

    fn remember_index(&mut self, key: EvidenceKey, eol: IndexEol) {
        let new_charge = if self.index_eol.contains_key(&key) {
            0
        } else {
            Self::INDEX_CHARGE
        };
        let next_bytes = self.evidence_bytes.saturating_add(new_charge);
        if next_bytes > self.evidence_limit() {
            self.progress.capacity_limited = true;
        } else {
            self.index_eol.insert(key, eol);
            self.evidence_bytes = next_bytes;
        }
    }

    fn remember_working(&mut self, key: EvidenceKey, evidence: WorkingEolEvidence) {
        let new_charge = if self.working_eol.contains_key(&key) {
            0
        } else {
            Self::WORKING_CHARGE
        };
        let next_bytes = self.evidence_bytes.saturating_add(new_charge);
        if next_bytes > self.evidence_limit() {
            self.progress.capacity_limited = true;
        } else {
            self.working_eol.insert(key, evidence);
            self.evidence_bytes = next_bytes;
        }
    }

    fn forget_working(&mut self, key: &EvidenceKey) {
        if self.working_eol.remove(key).is_some() {
            self.evidence_bytes = self.evidence_bytes.saturating_sub(Self::WORKING_CHARGE);
        }
    }
}

fn hash_evidence_field(hasher: &mut Sha256, field: &str) {
    hasher.update((field.len() as u64).to_be_bytes());
    hasher.update(field.as_bytes());
}

fn index_evidence_key(
    path: &str,
    oid: &str,
    staged: &AttributeValues,
    effective: &AttributeValues,
) -> EvidenceKey {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-index-eol-v1\0");
    for field in [
        path,
        oid,
        &staged.text,
        &staged.eol,
        &staged.filter,
        &effective.text,
        &effective.eol,
        &effective.filter,
    ] {
        hash_evidence_field(&mut hasher, field);
    }
    hasher.finalize().into()
}

fn working_evidence_key(path: &str) -> EvidenceKey {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-working-eol-v1\0");
    hash_evidence_field(&mut hasher, path);
    hasher.finalize().into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryResourceKind {
    Text,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryResource {
    pub(crate) source_set: String,
    pub(crate) repo_path: String,
    pub(crate) worktree_path: PathBuf,
    pub(crate) kind: RepositoryResourceKind,
    pub(crate) blob_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceOwnershipError {
    pub(crate) repo_path: String,
    pub(crate) source_sets: Vec<String>,
}

enum ResourceClassificationError {
    Cancelled,
    TimedOut,
    Incomplete(String),
    Ownership(ResourceOwnershipError),
}

struct ResourceClassification {
    resources: Vec<RepositoryResource>,
    ownership_errors: Vec<ResourceOwnershipError>,
}

fn reserve_resource_owner_expansion(
    retained_count: usize,
    retained: usize,
    additional_count: usize,
    additional: usize,
) -> Result<(usize, usize), ResourceClassificationError> {
    let next_count = retained_count.saturating_add(additional_count);
    if next_count > MAX_CLASSIFIED_RESOURCES {
        return Err(ResourceClassificationError::Incomplete(format!(
            "resource classification count exceeds {MAX_CLASSIFIED_RESOURCES} entries"
        )));
    }
    let next = retained.saturating_add(additional);
    if next > MAX_RESOURCE_OWNER_EXPANSION_BYTES {
        return Err(ResourceClassificationError::Incomplete(format!(
            "resource owner expansion budget exceeds {MAX_RESOURCE_OWNER_EXPANSION_BYTES} bytes"
        )));
    }
    Ok((next_count, next))
}

fn bounded_resource_ownership_reason(error: &ResourceOwnershipError) -> String {
    const PREFIX: &str =
        "resource does not have one regular stage-0 blob or has ambiguous source-set owners";
    let detailed_bytes = PREFIX
        .len()
        .saturating_add(2)
        .saturating_add(error.repo_path.len())
        .saturating_add(2)
        .saturating_add(error.source_sets.iter().fold(0_usize, |total, source_set| {
            total.saturating_add(source_set.len()).saturating_add(2)
        }));
    if detailed_bytes <= MAX_RESOURCE_OWNERSHIP_REASON_BYTES {
        return format!(
            "{PREFIX}: {}; {}",
            error.repo_path,
            error.source_sets.join(", ")
        );
    }
    format!(
        "resource ownership is ambiguous or non-regular; path bytes={}, owners={} (details omitted by inspection budget)",
        error.repo_path.len(),
        error.source_sets.len()
    )
}

pub(crate) struct RepositoryPolicyInspection {
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

pub(crate) struct SourceResourcePolicyInspector<'a> {
    runner: &'a dyn ProcessRunner,
    response_handoff_window: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributeValues {
    text: String,
    eol: String,
    filter: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EolValues {
    index: String,
    worktree: String,
}

struct AttributePolicyEvaluation {
    facts: Vec<ProjectHealthFact>,
    text_marked_binary: BTreeSet<String>,
}

#[derive(Debug)]
struct IsolatedGitIndex {
    _root: TempDir,
    git_dir: PathBuf,
    worktree: PathBuf,
    index: PathBuf,
    object_directory: PathBuf,
}

#[cfg(test)]
impl SourceResourcePolicyInspector<'static> {
    pub(crate) fn new() -> Self {
        Self {
            runner: system_process_runner(),
            response_handoff_window: super::TEST_RESPONSE_HANDOFF_WINDOW,
        }
    }
}

impl<'a> SourceResourcePolicyInspector<'a> {
    pub(crate) fn with_process_runner(
        runner: &'a dyn ProcessRunner,
        response_handoff_window: Duration,
    ) -> Self {
        Self {
            runner,
            response_handoff_window,
        }
    }

    #[cfg(test)]
    pub(crate) fn classify(
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
    ) -> Result<Vec<RepositoryResource>, ResourceOwnershipError> {
        let classification = Self::classify_with_checkpoint(
            repository_root,
            roots,
            entries,
            excluded_runtime_sidecars,
            &mut || Ok(()),
        )
        .map_err(|error| match error {
            ResourceClassificationError::Cancelled | ResourceClassificationError::TimedOut => {
                unreachable!("uncontrolled resource classification cannot stop")
            }
            ResourceClassificationError::Incomplete(reason) => ResourceOwnershipError {
                repo_path: reason,
                source_sets: Vec::new(),
            },
            ResourceClassificationError::Ownership(error) => error,
        })?;
        if let Some(error) = classification.ownership_errors.into_iter().next() {
            Err(error)
        } else {
            Ok(classification.resources)
        }
    }

    fn classify_with_checkpoint(
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
        checkpoint: &mut dyn FnMut() -> Result<(), ResourceClassificationError>,
    ) -> Result<ResourceClassification, ResourceClassificationError> {
        let repository_root = normalize_path_identity(repository_root).map_err(|_| {
            ResourceClassificationError::Ownership(ResourceOwnershipError {
                repo_path: repository_root.display().to_string(),
                source_sets: Vec::new(),
            })
        })?;
        let owners =
            SourceRootOwnerIndex::new_with_checkpoint(&repository_root, roots.iter(), checkpoint)
                .map_err(|error| match error {
                SourceRootOwnerIndexError::Checkpoint(error) => error,
                SourceRootOwnerIndexError::Path(reason) => {
                    ResourceClassificationError::Ownership(ResourceOwnershipError {
                        repo_path: reason,
                        source_sets: Vec::new(),
                    })
                }
            })?;
        let mut resources = Vec::new();
        let mut ownership_errors = Vec::new();
        let mut retained_owner_count = 0_usize;
        let mut retained_owner_bytes = 0_usize;
        checkpoint()?;
        for (entry_index, entry) in entries.iter().enumerate() {
            if entry_index % 256 == 0 {
                checkpoint()?;
            }
            if excluded_runtime_sidecars.contains(&entry.repo_path) {
                continue;
            }
            let Some((owners, prefix_depth)) = owners
                .deepest_owners_with_checkpoint(&entry.repo_path, checkpoint)
                .map_err(|error| match error {
                    SourceRootOwnerLookupError::Checkpoint(error) => error,
                    SourceRootOwnerLookupError::Path(reason) => {
                        ResourceClassificationError::Ownership(ResourceOwnershipError {
                            repo_path: reason,
                            source_sets: Vec::new(),
                        })
                    }
                })?
            else {
                continue;
            };
            if owners.len() > 1 {
                (retained_owner_count, retained_owner_bytes) = reserve_resource_owner_expansion(
                    retained_owner_count,
                    retained_owner_bytes,
                    owners.len(),
                    owners.iter().fold(0_usize, |total, root| {
                        total.saturating_add(root.source_set.name.len())
                    }),
                )?;
                let mut source_sets = owners
                    .iter()
                    .map(|root| root.source_set.name.clone())
                    .collect::<Vec<_>>();
                source_sets.sort();
                source_sets.dedup();
                ownership_errors.push(ResourceOwnershipError {
                    repo_path: entry.repo_path.clone(),
                    source_sets,
                });
                continue;
            }
            let root = owners[0];
            if root.source_set.source_format != SourceFormat::PlatformXml {
                continue;
            }
            let mut relative = String::new();
            for (component_index, component) in Path::new(&entry.repo_path)
                .components()
                .skip(prefix_depth)
                .enumerate()
            {
                if component_index % 256 == 0 {
                    checkpoint()?;
                }
                if !relative.is_empty() {
                    relative.push('/');
                }
                relative.push_str(&component.as_os_str().to_string_lossy());
            }
            if let Some(kind) = classify_platform_xml_relative_path(&relative) {
                if entry.blob_oid.is_none()
                    || !entry
                        .mode
                        .as_deref()
                        .is_some_and(|mode| matches!(mode, "100644" | "100755"))
                {
                    (retained_owner_count, retained_owner_bytes) =
                        reserve_resource_owner_expansion(
                            retained_owner_count,
                            retained_owner_bytes,
                            1,
                            root.source_set.name.len(),
                        )?;
                    ownership_errors.push(ResourceOwnershipError {
                        repo_path: entry.repo_path.clone(),
                        source_sets: vec![root.source_set.name.clone()],
                    });
                    continue;
                }
                (retained_owner_count, retained_owner_bytes) = reserve_resource_owner_expansion(
                    retained_owner_count,
                    retained_owner_bytes,
                    1,
                    root.source_set.name.len(),
                )?;
                resources.push(RepositoryResource {
                    source_set: root.source_set.name.clone(),
                    repo_path: entry.repo_path.clone(),
                    worktree_path: repository_root.join(&entry.repo_path),
                    kind,
                    blob_oid: entry
                        .blob_oid
                        .clone()
                        .expect("regular stage-0 resource has a blob object id"),
                });
            }
        }
        resources.sort_by(|left, right| left.repo_path.cmp(&right.repo_path));
        checkpoint()?;
        Ok(ResourceClassification {
            resources,
            ownership_errors,
        })
    }

    #[cfg(test)]
    pub(crate) fn inspect(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        self.inspect_excluding(
            repository_root,
            roots,
            entries,
            &BTreeSet::new(),
            cancellation,
            deadline,
        )
    }

    #[cfg(test)]
    pub(crate) fn inspect_excluding(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        self.inspect_excluding_continued(
            repository_root,
            roots,
            entries,
            excluded_runtime_sidecars,
            cancellation,
            deadline,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn inspect_excluding_continued(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
        mut continuation: Option<&mut ResourceContinuation>,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        let mut observations = resource_observations(
            roots.iter().map(|root| &root.source_set),
            "repository resource check was not reached",
            true,
        );
        let classification = match Self::classify_with_checkpoint(
            repository_root,
            roots,
            entries,
            excluded_runtime_sidecars,
            &mut || {
                if cancellation.is_cancelled() {
                    Err(ResourceClassificationError::Cancelled)
                } else if deadline.is_elapsed() {
                    Err(ResourceClassificationError::TimedOut)
                } else {
                    Ok(())
                }
            },
        ) {
            Ok(classification) => classification,
            Err(ResourceClassificationError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(ResourceClassificationError::TimedOut) => {
                let reason = "resource ownership classification exceeded the inspection deadline";
                for check in resource_check_ids() {
                    mark_check_not_run(&mut observations, check, reason);
                }
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts: vec![ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryAttributes,
                        source_set: None,
                    }],
                });
            }
            Err(ResourceClassificationError::Incomplete(reason)) => {
                for check in resource_check_ids() {
                    mark_check_not_run(&mut observations, check, &reason);
                }
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts: vec![ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryAttributes,
                        source_set: None,
                        reason,
                    }],
                });
            }
            Err(ResourceClassificationError::Ownership(error)) => {
                let reason = format!(
                    "resource ownership could not be proven from repository root {}",
                    error.repo_path
                );
                for check in resource_check_ids() {
                    mark_check_not_run(&mut observations, check, &reason);
                }
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts: vec![ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryAttributes,
                        source_set: None,
                        reason,
                    }],
                });
            }
        };
        let mut incomplete_reasons = BTreeMap::<String, (usize, String)>::new();
        let mut global_incomplete_reason = None::<(usize, String)>;
        for (error_index, error) in classification.ownership_errors.into_iter().enumerate() {
            if error_index.is_multiple_of(256) {
                if cancellation.is_cancelled() {
                    return Err(ProjectHealthInspectionError::Cancelled);
                }
                if deadline.is_elapsed() {
                    return Ok(timeout_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                    ));
                }
            }
            let reason = bounded_resource_ownership_reason(&error);
            for (source_set_index, source_set) in error.source_sets.iter().enumerate() {
                if source_set_index.is_multiple_of(256) {
                    if cancellation.is_cancelled() {
                        return Err(ProjectHealthInspectionError::Cancelled);
                    }
                    if deadline.is_elapsed() {
                        return Ok(timeout_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                        ));
                    }
                }
                let summary = incomplete_reasons
                    .entry(source_set.clone())
                    .or_insert_with(|| (0, reason.clone()));
                summary.0 = summary.0.saturating_add(1);
            }
            if error.source_sets.is_empty() {
                let summary = global_incomplete_reason.get_or_insert((0, reason));
                summary.0 = summary.0.saturating_add(1);
            }
        }
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if deadline.is_elapsed() {
            return Ok(timeout_policy(
                observations,
                ProjectCheckId::RepositoryAttributes,
            ));
        }
        let incomplete_source_sets = incomplete_reasons.keys().cloned().collect::<BTreeSet<_>>();
        let aggregate_reason = global_incomplete_reason
            .as_ref()
            .map(|(_, reason)| reason.clone())
            .unwrap_or_else(|| {
                format!(
                    "resource ownership is incomplete for {} source sets",
                    incomplete_source_sets.len()
                )
            });
        for observation in &mut observations {
            if !resource_check_ids().contains(&observation.id)
                || matches!(
                    observation.outcome,
                    ProjectCheckOutcome::NotApplicable { .. }
                )
            {
                continue;
            }
            let reason = match observation.source_set.as_deref() {
                Some(source_set) => incomplete_reasons.get(source_set).map(|(_, reason)| reason),
                None if !incomplete_source_sets.is_empty()
                    || global_incomplete_reason.is_some() =>
                {
                    Some(&aggregate_reason)
                }
                None => None,
            };
            if let Some(reason) = reason {
                observation.outcome = ProjectCheckOutcome::NotRun {
                    reason: reason.clone(),
                };
            }
        }
        let mut facts = Vec::with_capacity(
            incomplete_reasons
                .len()
                .saturating_add(usize::from(global_incomplete_reason.is_some())),
        );
        for (source_set_index, (source_set, (count, reason))) in
            incomplete_reasons.iter().enumerate()
        {
            if source_set_index.is_multiple_of(256) {
                if cancellation.is_cancelled() {
                    return Err(ProjectHealthInspectionError::Cancelled);
                }
                if deadline.is_elapsed() {
                    return Ok(timeout_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                    ));
                }
            }
            facts.push(ProjectHealthFact::GitInspectionIncompleteCounted {
                check: ProjectCheckId::RepositoryAttributes,
                source_set: Some(source_set.clone()),
                reason: reason.clone(),
                count: *count,
            });
        }
        if let Some((count, reason)) = global_incomplete_reason {
            facts.push(ProjectHealthFact::GitInspectionIncompleteCounted {
                check: ProjectCheckId::RepositoryAttributes,
                source_set: None,
                reason,
                count,
            });
        }
        let resources = classification
            .resources
            .into_iter()
            .filter(|resource| !incomplete_source_sets.contains(&resource.source_set))
            .collect::<Vec<_>>();
        if resources.is_empty() {
            if observations.iter().any(|observation| {
                matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }) {
                for check in resource_check_ids() {
                    mark_check_completed(&mut observations, check, &incomplete_source_sets);
                }
            }
            return Ok(RepositoryPolicyInspection {
                observations,
                facts,
            });
        }
        let paths = resources
            .iter()
            .map(|resource| resource.repo_path.clone())
            .collect::<Vec<_>>();
        let input = nul_input(&paths);
        let effective = match self.run_with_input(
            repository_root,
            [
                "check-attr",
                "-z",
                "--cached",
                "text",
                "eol",
                "filter",
                "--stdin",
            ],
            &input,
            cancellation,
            deadline,
        ) {
            Ok(output) if semantic_success(&output) => {
                match parse_attribute_records_controlled(
                    &output.stdout,
                    &paths,
                    cancellation,
                    deadline,
                ) {
                    Ok(records) => records,
                    Err(ResourceProtocolParseError::Cancelled) => {
                        return Err(ProjectHealthInspectionError::Cancelled)
                    }
                    Err(ResourceProtocolParseError::TimedOut) => {
                        return Ok(timeout_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                        ));
                    }
                    Err(ResourceProtocolParseError::Malformed(reason)) => {
                        return Ok(incomplete_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                            reason,
                        ));
                    }
                }
            }
            Ok(output) if output.cancelled => return Err(ProjectHealthInspectionError::Cancelled),
            Ok(output) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    output_reason(&output),
                ));
            }
            Err(_reason) if cancellation.is_cancelled() => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(reason) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    reason,
                ));
            }
        };

        let staged_attribute_entries = entries
            .iter()
            .filter(|entry| {
                Path::new(&entry.repo_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(".gitattributes")
            })
            .collect::<Vec<_>>();
        if staged_attribute_entries.len() > MAX_STAGED_POLICY_FILES {
            return Ok(incomplete_policy(
                observations,
                ProjectCheckId::RepositoryAttributes,
                format!(
                    "staged attribute policy contains {} .gitattributes files; inspection supports at most {MAX_STAGED_POLICY_FILES}",
                    staged_attribute_entries.len()
                ),
            ));
        }
        if let Err(reason) = self.validate_blob_sizes(
            repository_root,
            &staged_attribute_entries,
            MAX_STAGED_POLICY_TOTAL_BYTES,
            Some(MAX_STAGED_POLICY_TOTAL_BYTES),
            "staged attribute policy",
            cancellation,
            deadline,
        ) {
            return if cancellation.is_cancelled() {
                Err(ProjectHealthInspectionError::Cancelled)
            } else {
                Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    reason,
                ))
            };
        }
        let staged_index = match self.create_isolated_index(
            repository_root,
            &staged_attribute_entries,
            cancellation,
            deadline,
        ) {
            Ok(index) => index,
            Err(_) if cancellation.is_cancelled() => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(reason) => {
                return if cancellation.is_cancelled() {
                    Err(ProjectHealthInspectionError::Cancelled)
                } else {
                    Ok(incomplete_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                        reason,
                    ))
                };
            }
        };
        let staged_command = isolated_git_command(
            repository_root,
            &staged_index,
            vec![
                "-c".into(),
                format!(
                    "core.attributesFile={}",
                    host_path_text(
                        staged_index
                            ._root
                            .path()
                            .join("empty-attributes")
                            .display()
                            .to_string()
                    )
                ),
                "check-attr".into(),
                "-z".into(),
                "--cached".into(),
                "text".into(),
                "eol".into(),
                "filter".into(),
                "--stdin".into(),
            ],
            cancellation,
            deadline,
        );
        let staged = match sticky_process_result(
            self.runner.run_with_input(&staged_command, &input),
            cancellation,
        ) {
            Ok(output) if semantic_success(&output) => {
                match parse_attribute_records_controlled(
                    &output.stdout,
                    &paths,
                    cancellation,
                    deadline,
                ) {
                    Ok(records) => records,
                    Err(ResourceProtocolParseError::Cancelled) => {
                        return Err(ProjectHealthInspectionError::Cancelled)
                    }
                    Err(ResourceProtocolParseError::TimedOut) => {
                        return Ok(timeout_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                        ));
                    }
                    Err(ResourceProtocolParseError::Malformed(reason)) => {
                        return Ok(incomplete_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                            reason,
                        ));
                    }
                }
            }
            Ok(output) if output.cancelled => return Err(ProjectHealthInspectionError::Cancelled),
            Ok(output) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    output_reason(&output),
                ));
            }
            Err(reason) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    reason,
                ));
            }
        };
        let attribute_policy = match evaluate_attribute_policy_with_checkpoint(
            &resources,
            &staged,
            &effective,
            &mut || resource_protocol_checkpoint(cancellation, deadline),
        ) {
            Ok(policy) => policy,
            Err(ResourceProtocolParseError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(ResourceProtocolParseError::TimedOut) => {
                return Ok(timeout_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                ));
            }
            Err(ResourceProtocolParseError::Malformed(reason)) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    reason,
                ));
            }
        };
        let AttributePolicyEvaluation {
            facts: attribute_facts,
            text_marked_binary,
        } = attribute_policy;
        facts.extend(attribute_facts);
        mark_check_completed(
            &mut observations,
            ProjectCheckId::RepositoryAttributes,
            &incomplete_source_sets,
        );

        let expected_text_resources = resources
            .iter()
            .filter(|resource| resource.kind == RepositoryResourceKind::Text)
            .collect::<Vec<_>>();
        if let Some(state) = continuation.as_deref_mut() {
            state.progress.text_resources = expected_text_resources.len();
        }
        let blob_sizes = match self.validate_resource_blob_sizes(
            repository_root,
            &expected_text_resources,
            cancellation,
            deadline,
        ) {
            Ok(sizes) => sizes,
            Err(reason) => {
                if cancellation.is_cancelled() {
                    return Err(ProjectHealthInspectionError::Cancelled);
                }
                mark_check_not_run(
                    &mut observations,
                    ProjectCheckId::RepositoryIndexEol,
                    &reason,
                );
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIndexEol,
                    source_set: None,
                    reason,
                });
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts,
                });
            }
        };
        let eol = if expected_text_resources.is_empty() {
            BTreeMap::new()
        } else if let Some(state) = continuation.as_deref_mut() {
            match self.incremental_index_eol(
                repository_root,
                &expected_text_resources,
                &blob_sizes,
                &staged,
                &effective,
                state,
                cancellation,
                deadline,
            ) {
                Ok(eol) => eol,
                Err(ResourceProtocolParseError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(ResourceProtocolParseError::TimedOut) => {
                    return Ok(retain_prior_facts(
                        timeout_policy(observations, ProjectCheckId::RepositoryIndexEol),
                        facts,
                    ));
                }
                Err(ResourceProtocolParseError::Malformed(reason)) => {
                    return Ok(retain_prior_facts(
                        incomplete_policy(observations, ProjectCheckId::RepositoryIndexEol, reason),
                        facts,
                    ));
                }
            }
        } else {
            let eol_entries = expected_text_resources
                .iter()
                .map(|resource| GitIndexEntry {
                    repo_path: resource.repo_path.clone(),
                    blob_oid: Some(resource.blob_oid.clone()),
                    mode: Some("100644".into()),
                })
                .collect::<Vec<_>>();
            let eol_entry_refs = eol_entries.iter().collect::<Vec<_>>();
            let eol_index = match self.create_isolated_index(
                repository_root,
                &eol_entry_refs,
                cancellation,
                deadline,
            ) {
                Ok(index) => index,
                Err(_) if cancellation.is_cancelled() => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(reason) => {
                    mark_check_not_run(
                        &mut observations,
                        ProjectCheckId::RepositoryIndexEol,
                        &reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIndexEol,
                        source_set: None,
                        reason,
                    });
                    return Ok(RepositoryPolicyInspection {
                        observations,
                        facts,
                    });
                }
            };
            let eol_command = isolated_git_command(
                repository_root,
                &eol_index,
                vec!["ls-files".into(), "--eol".into(), "-z".into()],
                cancellation,
                deadline,
            );
            let eol_output =
                match sticky_process_result(self.runner.run(&eol_command), cancellation) {
                    Ok(output) if semantic_success(&output) => output,
                    Ok(output) if output.cancelled => {
                        return Err(ProjectHealthInspectionError::Cancelled)
                    }
                    Ok(output) => {
                        let reason = output_reason(&output);
                        mark_check_not_run(
                            &mut observations,
                            ProjectCheckId::RepositoryIndexEol,
                            &reason,
                        );
                        facts.push(ProjectHealthFact::GitInspectionIncomplete {
                            check: ProjectCheckId::RepositoryIndexEol,
                            source_set: None,
                            reason,
                        });
                        return Ok(RepositoryPolicyInspection {
                            observations,
                            facts,
                        });
                    }
                    Err(reason) => {
                        mark_check_not_run(
                            &mut observations,
                            ProjectCheckId::RepositoryIndexEol,
                            &reason,
                        );
                        facts.push(ProjectHealthFact::GitInspectionIncomplete {
                            check: ProjectCheckId::RepositoryIndexEol,
                            source_set: None,
                            reason,
                        });
                        return Ok(RepositoryPolicyInspection {
                            observations,
                            facts,
                        });
                    }
                };
            let expected_text_eol_paths = expected_text_resources
                .iter()
                .map(|resource| resource.repo_path.clone())
                .collect::<Vec<_>>();
            match parse_eol_records_controlled(
                &eol_output.stdout,
                &expected_text_eol_paths,
                cancellation,
                deadline,
            ) {
                Ok(records) => records,
                Err(ResourceProtocolParseError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(ResourceProtocolParseError::TimedOut) => {
                    let reason = "Git EOL protocol parsing exceeded the inspection deadline";
                    mark_check_not_run(
                        &mut observations,
                        ProjectCheckId::RepositoryIndexEol,
                        reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryIndexEol,
                        source_set: None,
                    });
                    return Ok(RepositoryPolicyInspection {
                        observations,
                        facts,
                    });
                }
                Err(ResourceProtocolParseError::Malformed(reason)) => {
                    mark_check_not_run(
                        &mut observations,
                        ProjectCheckId::RepositoryIndexEol,
                        &reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIndexEol,
                        source_set: None,
                        reason,
                    });
                    return Ok(RepositoryPolicyInspection {
                        observations,
                        facts,
                    });
                }
            }
        };
        let index_eol_facts = match index_eol_facts_with_checkpoint(
            &resources,
            &eol,
            &text_marked_binary,
            &mut || resource_protocol_checkpoint(cancellation, deadline),
        ) {
            Ok(index_eol_facts) => index_eol_facts,
            Err(ResourceProtocolParseError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(ResourceProtocolParseError::TimedOut) => {
                let reason = "Git index EOL evaluation exceeded the inspection deadline";
                mark_check_not_run(
                    &mut observations,
                    ProjectCheckId::RepositoryIndexEol,
                    reason,
                );
                facts.push(ProjectHealthFact::GitInspectionTimeout {
                    check: ProjectCheckId::RepositoryIndexEol,
                    source_set: None,
                });
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts,
                });
            }
            Err(ResourceProtocolParseError::Malformed(reason)) => {
                return Ok(retain_prior_facts(
                    incomplete_policy(observations, ProjectCheckId::RepositoryIndexEol, reason),
                    facts,
                ));
            }
        };
        facts.extend(index_eol_facts);
        mark_check_completed(
            &mut observations,
            ProjectCheckId::RepositoryIndexEol,
            &incomplete_source_sets,
        );
        let mut working_incomplete_source_sets = BTreeSet::new();
        let mut working_timed_out = false;
        let mut working_facts = Vec::new();
        let mut working_witnesses = Vec::new();
        let mut lfs_incomplete_source_sets = BTreeSet::new();
        let mut lfs_timed_out = false;
        let mut binary_by_source = BTreeMap::<String, Vec<(String, u64)>>::new();
        for resource in &resources {
            if cancellation.is_cancelled() {
                return Err(ProjectHealthInspectionError::Cancelled);
            }
            if deadline.is_elapsed() {
                match resource.kind {
                    RepositoryResourceKind::Text => working_timed_out = true,
                    RepositoryResourceKind::Binary => lfs_timed_out = true,
                }
                continue;
            }
            let attributes = &staged[&resource.repo_path];
            match resource.kind {
                RepositoryResourceKind::Text => {
                    // `-text` is the primary policy defect for a known platform XML
                    // resource. Git deliberately stops treating that path as text, so
                    // any EOL interpretation derived from its working bytes would be a
                    // misleading secondary diagnostic.
                    if text_marked_binary.contains(&resource.repo_path) {
                        continue;
                    }
                    let inspected = if let Some(state) = continuation.as_deref_mut() {
                        inspect_working_eol_continued(
                            repository_root,
                            Path::new(&resource.repo_path),
                            state,
                            cancellation,
                            deadline,
                        )
                    } else {
                        inspect_working_eol(
                            repository_root,
                            Path::new(&resource.repo_path),
                            cancellation,
                            deadline,
                        )
                        .map(|eol| (eol, None))
                    };
                    if let Ok((_, fingerprint)) = &inspected {
                        working_witnesses.push((resource.repo_path.clone(), fingerprint.clone()));
                    }
                    match inspected {
                        Ok((Some(WorkingEol::Mixed), _)) => {
                            working_facts.push(ProjectHealthFact::MixedEol {
                                source_set: resource.source_set.clone(),
                                path: resource.repo_path.clone(),
                            });
                        }
                        Ok((Some(WorkingEol::BareCr), _)) => {
                            working_facts.push(ProjectHealthFact::WorkingEolUnsupported {
                                source_set: resource.source_set.clone(),
                                path: resource.repo_path.clone(),
                                observed: "cr".into(),
                            });
                        }
                        Ok((Some(WorkingEol::Supported) | None, _)) => {}
                        Err(WorkingEolInspectionError::Cancelled) => {
                            return Err(ProjectHealthInspectionError::Cancelled);
                        }
                        Err(WorkingEolInspectionError::TimedOut) => {
                            working_timed_out = true;
                        }
                        Err(WorkingEolInspectionError::Incomplete(reason)) => {
                            let reason = format!("{}: {reason}", resource.repo_path);
                            working_incomplete_source_sets.insert(resource.source_set.clone());
                            mark_source_check_not_run(
                                &mut observations,
                                ProjectCheckId::RepositoryWorkingEol,
                                &resource.source_set,
                                &reason,
                            );
                            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                check: ProjectCheckId::RepositoryWorkingEol,
                                source_set: Some(resource.source_set.clone()),
                                reason,
                            });
                        }
                    }
                }
                RepositoryResourceKind::Binary => {
                    if attributes.filter == "lfs" {
                        continue;
                    }
                    match repository_regular_file_size(
                        repository_root,
                        Path::new(&resource.repo_path),
                    ) {
                        Ok(Some(size)) => binary_by_source
                            .entry(resource.source_set.clone())
                            .or_default()
                            .push((resource.repo_path.clone(), size)),
                        Ok(None) => {}
                        Err(reason) => {
                            let reason = format!("{}: {reason}", resource.repo_path);
                            lfs_incomplete_source_sets.insert(resource.source_set.clone());
                            mark_source_check_not_run(
                                &mut observations,
                                ProjectCheckId::RepositoryLfs,
                                &resource.source_set,
                                &reason,
                            );
                            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                check: ProjectCheckId::RepositoryLfs,
                                source_set: Some(resource.source_set.clone()),
                                reason,
                            });
                        }
                    }
                }
            }
        }
        if continuation.is_some() && !working_timed_out {
            run_working_revalidation_test_hook();
        }
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        let working_revalidation_error = if continuation.is_some() && !working_timed_out {
            working_witnesses.iter().find_map(|(path, expected)| {
                if cancellation.is_cancelled() || deadline.is_elapsed() {
                    return None;
                }
                let current =
                    open_repository_regular_file_nofollow(repository_root, Path::new(path))
                        .and_then(|file| working_file_fingerprint(&file));
                match (expected, current) {
                    (Some(expected), Ok(current)) if current == *expected => None,
                    (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
                    (_, Ok(_)) | (None, Err(_)) => {
                        Some(format!("working path {path} changed during EOL inspection"))
                    }
                    (Some(_), Err(error)) => Some(format!(
                        "working path {path} could not be revalidated: {error}"
                    )),
                }
            })
        } else {
            None
        };
        if let Some(reason) = working_revalidation_error {
            mark_check_not_run(
                &mut observations,
                ProjectCheckId::RepositoryWorkingEol,
                &reason,
            );
            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryWorkingEol,
                source_set: None,
                reason,
            });
        } else if working_timed_out || (continuation.is_some() && deadline.is_elapsed()) {
            let reason = "deadline expired during working EOL inspection";
            mark_check_not_run(
                &mut observations,
                ProjectCheckId::RepositoryWorkingEol,
                reason,
            );
            facts.push(ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryWorkingEol,
                source_set: None,
            });
        } else {
            working_facts.retain(|fact| match fact {
                ProjectHealthFact::MixedEol { source_set, .. }
                | ProjectHealthFact::WorkingEolUnsupported { source_set, .. } => {
                    !working_incomplete_source_sets.contains(source_set)
                }
                _ => true,
            });
            let working_incomplete_source_sets = incomplete_source_sets
                .union(&working_incomplete_source_sets)
                .cloned()
                .collect();
            mark_check_completed(
                &mut observations,
                ProjectCheckId::RepositoryWorkingEol,
                &working_incomplete_source_sets,
            );
            facts.extend(working_facts);
        }
        if lfs_timed_out {
            mark_check_not_run(
                &mut observations,
                ProjectCheckId::RepositoryLfs,
                "deadline expired during LFS size inspection",
            );
            facts.push(ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryLfs,
                source_set: None,
            });
        } else {
            for source_set in &lfs_incomplete_source_sets {
                binary_by_source.remove(source_set);
            }
            match lfs_facts_with_checkpoint(binary_by_source, &mut || {
                resource_protocol_checkpoint(cancellation, deadline)
            }) {
                Ok(lfs_facts) => {
                    facts.extend(lfs_facts);
                    let lfs_incomplete_source_sets = incomplete_source_sets
                        .union(&lfs_incomplete_source_sets)
                        .cloned()
                        .collect();
                    mark_check_completed(
                        &mut observations,
                        ProjectCheckId::RepositoryLfs,
                        &lfs_incomplete_source_sets,
                    );
                }
                Err(ResourceProtocolParseError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(ResourceProtocolParseError::TimedOut) => {
                    mark_check_not_run(
                        &mut observations,
                        ProjectCheckId::RepositoryLfs,
                        "deadline expired during LFS aggregation",
                    );
                    facts.push(ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryLfs,
                        source_set: None,
                    });
                }
                Err(ResourceProtocolParseError::Malformed(reason)) => {
                    mark_check_not_run(&mut observations, ProjectCheckId::RepositoryLfs, &reason);
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryLfs,
                        source_set: None,
                        reason,
                    });
                }
            }
        }
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if continuation.is_some()
            && !observations.iter().any(|observation| {
                resource_check_ids().contains(&observation.id)
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            })
        {
            // A failed final fence cannot substantiate any earlier resource
            // finding. The helpers restart NotRun at RepositoryAttributes and
            // intentionally discard the facts from the old index snapshot.
            if deadline.is_elapsed() {
                return Ok(timeout_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                ));
            }
            let mut expected_digest = GitIndexDigest::default();
            for entry in entries {
                expected_digest.update(entry);
            }
            let mut current_digest = GitIndexDigest::default();
            let current_index = read_git_index(
                self.runner,
                repository_root,
                cancellation,
                deadline,
                &mut |entry| current_digest.update(&entry),
            );
            match current_index {
                Ok(()) => {}
                Err(IndexReadError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(IndexReadError::ProcessTimedOut | IndexReadError::ParseTimedOut) => {
                    return Ok(timeout_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                    ));
                }
                Err(
                    IndexReadError::Failed(reason)
                    | IndexReadError::Incomplete(reason)
                    | IndexReadError::Malformed(reason),
                ) => {
                    return Ok(incomplete_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                        reason,
                    ));
                }
            }
            if current_digest.finish() != expected_digest.finish() {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    "Git index changed during resource inspection".into(),
                ));
            }
            let current_effective = match self.run_with_input(
                repository_root,
                [
                    "check-attr",
                    "-z",
                    "--cached",
                    "text",
                    "eol",
                    "filter",
                    "--stdin",
                ],
                &input,
                cancellation,
                deadline,
            ) {
                Ok(output) if semantic_success(&output) => output,
                Ok(output) if output.cancelled => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Ok(output) if output.timed_out => {
                    return Ok(timeout_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                    ));
                }
                Ok(output) => {
                    return Ok(incomplete_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                        output_reason(&output),
                    ));
                }
                Err(_reason) if cancellation.is_cancelled() => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(reason) => {
                    return Ok(incomplete_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                        reason,
                    ));
                }
            };
            let current_effective = match parse_attribute_records_controlled(
                &current_effective.stdout,
                &paths,
                cancellation,
                deadline,
            ) {
                Ok(attributes) => attributes,
                Err(ResourceProtocolParseError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(ResourceProtocolParseError::TimedOut) => {
                    return Ok(timeout_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                    ));
                }
                Err(ResourceProtocolParseError::Malformed(reason)) => {
                    return Ok(incomplete_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                        reason,
                    ));
                }
            };
            if current_effective != effective {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    "effective Git attributes changed during resource inspection".into(),
                ));
            }
        }
        Ok(RepositoryPolicyInspection {
            observations,
            facts,
        })
    }

    fn run<const N: usize>(
        &self,
        cwd: &Path,
        args: [&str; N],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProcessOutput, String> {
        let result = self.runner.run(&process_command(
            cwd,
            args.into_iter().map(str::to_owned).collect(),
            cancellation,
            deadline,
        ));
        sticky_process_result(result, cancellation)
    }

    fn run_with_input<const N: usize>(
        &self,
        cwd: &Path,
        args: [&str; N],
        input: &[u8],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProcessOutput, String> {
        self.run_with_input_vec(
            cwd,
            args.into_iter().map(str::to_owned).collect(),
            input,
            cancellation,
            deadline,
        )
    }

    fn run_with_input_vec(
        &self,
        cwd: &Path,
        args: Vec<String>,
        input: &[u8],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProcessOutput, String> {
        let result = self
            .runner
            .run_with_input(&process_command(cwd, args, cancellation, deadline), input);
        sticky_process_result(result, cancellation)
    }

    #[allow(clippy::too_many_arguments)]
    fn incremental_index_eol(
        &self,
        repository_root: &Path,
        resources: &[&RepositoryResource],
        blob_sizes: &BTreeMap<String, usize>,
        staged: &BTreeMap<String, AttributeValues>,
        effective: &BTreeMap<String, AttributeValues>,
        continuation: &mut ResourceContinuation,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<BTreeMap<String, EolValues>, ResourceProtocolParseError> {
        let mut current_index = BTreeSet::new();
        let mut current_working = BTreeSet::new();
        for (position, resource) in resources.iter().enumerate() {
            if position.is_multiple_of(256) {
                resource_protocol_checkpoint(cancellation, deadline)?;
            }
            let path = &resource.repo_path;
            let staged_attributes = staged.get(path).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "staged Git attribute policy omitted {path}"
                ))
            })?;
            let effective_attributes = effective.get(path).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "effective Git attribute policy omitted {path}"
                ))
            })?;
            current_index.insert(index_evidence_key(
                path,
                &resource.blob_oid,
                staged_attributes,
                effective_attributes,
            ));
            current_working.insert(working_evidence_key(path));
        }
        continuation.retain_current(&current_index, &current_working);
        let mut result = BTreeMap::new();
        let mut pending = Vec::<(&RepositoryResource, EvidenceKey)>::new();
        let mut pending_bytes = 0_usize;
        let mut completed_batches = 0_usize;
        for resource in resources {
            resource_protocol_checkpoint(cancellation, deadline)?;
            let path = &resource.repo_path;
            let staged_attributes = staged.get(path).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "staged Git attribute policy omitted {path}"
                ))
            })?;
            let effective_attributes = effective.get(path).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "effective Git attribute policy omitted {path}"
                ))
            })?;
            let key = index_evidence_key(
                path,
                &resource.blob_oid,
                staged_attributes,
                effective_attributes,
            );
            if let Some(cached) = continuation.index_eol.get(&key) {
                result.insert(
                    path.clone(),
                    EolValues {
                        index: cached.as_git().into(),
                        worktree: String::new(),
                    },
                );
                continuation.progress.staged_eol_retained += 1;
                continue;
            }
            if staged_eol_test_timeout() {
                if !pending.is_empty() {
                    self.commit_index_eol_batch(
                        repository_root,
                        &pending,
                        continuation,
                        &mut result,
                        cancellation,
                        deadline,
                        completed_batches == 0,
                    )?;
                }
                return Err(ResourceProtocolParseError::TimedOut);
            }
            let size = *blob_sizes.get(&resource.blob_oid).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "Git omitted staged text blob metadata for {path}"
                ))
            })?;
            if !pending.is_empty()
                && (pending.len() >= continuation.index_eol_batch_files()
                    || pending_bytes.saturating_add(size) > MAX_INDEX_EOL_BATCH_BLOB_BYTES)
            {
                self.commit_index_eol_batch(
                    repository_root,
                    &pending,
                    continuation,
                    &mut result,
                    cancellation,
                    deadline,
                    completed_batches == 0,
                )?;
                completed_batches += 1;
                pending.clear();
                pending_bytes = 0;
            }
            pending.push((resource, key));
            pending_bytes = pending_bytes.saturating_add(size);
        }
        if !pending.is_empty() {
            self.commit_index_eol_batch(
                repository_root,
                &pending,
                continuation,
                &mut result,
                cancellation,
                deadline,
                completed_batches == 0,
            )?;
        }
        // A completed index pass proves the retained paths for this snapshot;
        // start at full throughput when a later revision adds new work.
        continuation.index_eol_batch_files = None;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_index_eol_batch(
        &self,
        repository_root: &Path,
        batch: &[(&RepositoryResource, EvidenceKey)],
        continuation: &mut ResourceContinuation,
        result: &mut BTreeMap<String, EolValues>,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
        first_batch: bool,
    ) -> Result<(), ResourceProtocolParseError> {
        let budget_at_start = deadline.remaining();
        let inspected =
            self.inspect_index_eol_batch(repository_root, batch, cancellation, deadline);
        let inspected = match inspected {
            Err(ResourceProtocolParseError::TimedOut) => {
                // A later batch normally inherits only the tail of this call's
                // deadline. Only a first batch given at least half of the
                // seven-second handoff window can indicate an oversized group.
                if first_batch
                    && budget_at_start
                        .is_some_and(|budget| budget >= self.response_handoff_window / 2)
                {
                    continuation.reduce_index_eol_batch(batch.len());
                }
                return Err(ResourceProtocolParseError::TimedOut);
            }
            other => other?,
        };
        // A timed-out, cancelled, truncated or malformed response must never
        // publish evidence for only a prefix of this batch.
        for ((resource, key), (index_eol, eol)) in batch.iter().zip(inspected) {
            continuation.remember_index(*key, index_eol);
            if continuation.index_eol.contains_key(key) {
                continuation.progress.staged_eol_retained += 1;
            }
            result.insert(resource.repo_path.clone(), eol);
        }
        Ok(())
    }

    fn inspect_index_eol_batch(
        &self,
        repository_root: &Path,
        batch: &[(&RepositoryResource, EvidenceKey)],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<Vec<(IndexEol, EolValues)>, ResourceProtocolParseError> {
        let entries = batch
            .iter()
            .map(|(resource, _)| GitIndexEntry {
                repo_path: resource.repo_path.clone(),
                blob_oid: Some(resource.blob_oid.clone()),
                mode: Some("100644".into()),
            })
            .collect::<Vec<_>>();
        let index = self
            .create_isolated_index(
                repository_root,
                &entries.iter().collect::<Vec<_>>(),
                cancellation,
                deadline,
            )
            .map_err(|reason| resource_process_error(reason, cancellation, deadline))?;
        let command = isolated_git_command(
            repository_root,
            &index,
            vec!["ls-files".into(), "--eol".into(), "-z".into()],
            cancellation,
            deadline,
        );
        let output = sticky_process_result(self.runner.run(&command), cancellation)
            .map_err(|reason| resource_process_error(reason, cancellation, deadline))?;
        if output.cancelled || cancellation.is_cancelled() {
            return Err(ResourceProtocolParseError::Cancelled);
        }
        if output.timed_out || deadline.is_elapsed() {
            return Err(ResourceProtocolParseError::TimedOut);
        }
        if !semantic_success(&output) {
            return Err(ResourceProtocolParseError::Malformed(output_reason(
                &output,
            )));
        }
        let paths = entries
            .iter()
            .map(|entry| entry.repo_path.clone())
            .collect::<Vec<_>>();
        let records = parse_eol_records_controlled(&output.stdout, &paths, cancellation, deadline)?;
        if records.len() != paths.len() {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol returned an unexpected path".into(),
            ));
        }
        let mut inspected = Vec::with_capacity(batch.len());
        for path in &paths {
            let eol = records
                .get(path)
                .expect("parser checks every expected path")
                .clone();
            let index_eol = IndexEol::from_git(&eol.index).ok_or_else(|| {
                ResourceProtocolParseError::Malformed(format!(
                    "git ls-files --eol returned unsupported index EOL metadata for {path}"
                ))
            })?;
            inspected.push((index_eol, eol));
        }
        resource_protocol_checkpoint(cancellation, deadline)?;
        Ok(inspected)
    }

    fn validate_resource_blob_sizes(
        &self,
        repository_root: &Path,
        resources: &[&RepositoryResource],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<BTreeMap<String, usize>, String> {
        let entries = resources
            .iter()
            .map(|resource| GitIndexEntry {
                repo_path: resource.repo_path.clone(),
                blob_oid: Some(resource.blob_oid.clone()),
                mode: Some("100644".into()),
            })
            .collect::<Vec<_>>();
        self.validate_blob_sizes(
            repository_root,
            &entries.iter().collect::<Vec<_>>(),
            MAX_INDEX_EOL_FILE_BYTES,
            None,
            "staged text resource policy",
            cancellation,
            deadline,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_blob_sizes(
        &self,
        repository_root: &Path,
        entries: &[&GitIndexEntry],
        max_file_bytes: usize,
        max_total_bytes: Option<usize>,
        label: &str,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<BTreeMap<String, usize>, String> {
        let mut unique_oids = BTreeSet::new();
        for entry in entries {
            if cancellation.is_cancelled() {
                return Err(format!("{label} inspection was cancelled"));
            }
            if deadline.is_elapsed() {
                return Err(format!("{label} inspection timed out"));
            }
            let oid = entry.blob_oid.as_deref().ok_or_else(|| {
                format!(
                    "{label} path does not have one regular stage-0 blob: {}",
                    entry.repo_path
                )
            })?;
            if !entry
                .mode
                .as_deref()
                .is_some_and(|mode| matches!(mode, "100644" | "100755"))
            {
                return Err(format!(
                    "{label} path does not have one regular stage-0 blob: {}",
                    entry.repo_path
                ));
            }
            unique_oids.insert(oid.to_owned());
        }
        let mut input = Vec::new();
        for oid in &unique_oids {
            input.extend_from_slice(oid.as_bytes());
            input.push(b'\n');
        }
        let mut by_oid = BTreeMap::<String, usize>::new();
        if !unique_oids.is_empty() {
            let output = self.run_with_input_vec(
                repository_root,
                vec![
                    "--no-replace-objects".into(),
                    "cat-file".into(),
                    "--batch-check=%(objectname) %(objecttype) %(objectsize)".into(),
                ],
                &input,
                cancellation,
                deadline,
            )?;
            if !semantic_success(&output) {
                return Err(output_reason(&output));
            }
            for line in output.stdout.lines() {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                if fields.len() != 3 || fields[1] != "blob" {
                    return Err(format!(
                        "Git returned invalid {label} blob metadata: {line}"
                    ));
                }
                let size = fields[2]
                    .parse::<usize>()
                    .map_err(|_| format!("Git returned an invalid blob size: {line}"))?;
                if by_oid.insert(fields[0].to_owned(), size).is_some() {
                    return Err(format!("Git returned duplicate {label} blob metadata"));
                }
            }
            if by_oid.len() != unique_oids.len()
                || unique_oids.iter().any(|oid| !by_oid.contains_key(oid))
            {
                return Err(format!("Git omitted {label} blob metadata"));
            }
        }
        let mut total = 0_usize;
        for entry in entries {
            let oid = entry
                .blob_oid
                .as_deref()
                .expect("validated staged blob has an object id");
            let size = by_oid[oid];
            if size > max_file_bytes {
                return Err(format!(
                    "{label} path {} is {size} bytes; inspection supports at most {max_file_bytes} bytes per file",
                    entry.repo_path
                ));
            }
            total = total.saturating_add(size);
            if let Some(max_total_bytes) = max_total_bytes.filter(|bound| total > *bound) {
                return Err(format!(
                    "{label} is {total} bytes; inspection supports at most {max_total_bytes} bytes in total"
                ));
            }
        }
        Ok(by_oid)
    }

    fn create_isolated_index(
        &self,
        repository_root: &Path,
        entries: &[&GitIndexEntry],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<IsolatedGitIndex, String> {
        let root = TempDir::new()
            .map_err(|error| format!("create isolated Git inspection directory: {error}"))?;
        let git_dir = root.path().join("git");
        let worktree = root.path().join("worktree");
        let index = root.path().join("index");
        fs::create_dir_all(git_dir.join("refs/heads"))
            .map_err(|error| format!("create isolated Git refs: {error}"))?;
        fs::create_dir_all(git_dir.join("objects"))
            .map_err(|error| format!("create isolated Git objects directory: {error}"))?;
        fs::create_dir_all(&worktree)
            .map_err(|error| format!("create isolated Git worktree: {error}"))?;
        fs::write(git_dir.join("HEAD"), b"ref: refs/heads/unborn\n")
            .map_err(|error| format!("write isolated Git HEAD: {error}"))?;
        let object_output = self.run(
            repository_root,
            [
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
            cancellation,
            deadline,
        )?;
        if !semantic_success(&object_output) {
            return Err(output_reason(&object_output));
        }
        let object_directory_text = object_output.stdout.strip_suffix('\n').ok_or_else(|| {
            "Git object-directory output is missing its terminal newline".to_string()
        })?;
        let object_directory = PathBuf::from(object_directory_text);
        if !object_directory.is_absolute() {
            return Err("Git returned a non-absolute object directory".into());
        }
        let object_format_output = self.run(
            repository_root,
            ["rev-parse", "--show-object-format"],
            cancellation,
            deadline,
        )?;
        if !semantic_success(&object_format_output) {
            return Err(output_reason(&object_format_output));
        }
        let object_format = object_format_output
            .stdout
            .strip_suffix('\n')
            .ok_or_else(|| {
                "Git object-format output is missing its terminal newline".to_string()
            })?;
        let isolated_config = match object_format {
            "sha1" => "[core]\nrepositoryformatversion = 0\nbare = false\n".to_string(),
            "sha256" => "[core]\nrepositoryformatversion = 1\nbare = false\n[extensions]\nobjectFormat = sha256\n".to_string(),
            other => return Err(format!("unsupported Git object format: {other}")),
        };
        fs::write(git_dir.join("config"), isolated_config)
            .map_err(|error| format!("write isolated Git config: {error}"))?;
        let isolated = IsolatedGitIndex {
            _root: root,
            git_dir,
            worktree,
            index,
            object_directory,
        };
        let initialize = isolated_git_command(
            repository_root,
            &isolated,
            vec!["read-tree".into(), "--empty".into()],
            cancellation,
            deadline,
        );
        let output = sticky_process_result(self.runner.run(&initialize), cancellation)?;
        if !semantic_success(&output) {
            return Err(output_reason(&output));
        }
        if !entries.is_empty() {
            let mut index_info = Vec::new();
            for entry in entries {
                let path = Path::new(&entry.repo_path);
                if path.is_absolute()
                    || path
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                {
                    return Err(format!(
                        "staged policy path is not repository-relative: {}",
                        entry.repo_path
                    ));
                }
                let mode = entry.mode.as_deref().ok_or_else(|| {
                    format!("staged policy path has no mode: {}", entry.repo_path)
                })?;
                let oid = entry.blob_oid.as_deref().ok_or_else(|| {
                    format!("staged policy path has no blob: {}", entry.repo_path)
                })?;
                index_info
                    .extend_from_slice(format!("{mode} {oid}\t{}\0", entry.repo_path).as_bytes());
            }
            let update = isolated_git_command(
                repository_root,
                &isolated,
                vec!["update-index".into(), "-z".into(), "--index-info".into()],
                cancellation,
                deadline,
            );
            let output = sticky_process_result(
                self.runner.run_with_input(&update, &index_info),
                cancellation,
            )?;
            if !semantic_success(&output) {
                return Err(output_reason(&output));
            }
        }
        Ok(isolated)
    }
}

pub(crate) fn classify_platform_xml_relative_path(
    normalized_relative_path: &str,
) -> Option<RepositoryResourceKind> {
    let segments = normalized_relative_path.split('/').collect::<Vec<_>>();
    if segments.len() == 4
        && segments[0] == "XDTOPackages"
        && !segments[1].is_empty()
        && segments[2] == "Ext"
        && segments[3] == "Package.bin"
    {
        return Some(RepositoryResourceKind::Text);
    }
    let extension = Path::new(normalized_relative_path)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "xml" | "bsl") {
        Some(RepositoryResourceKind::Text)
    } else if matches!(
        extension.as_str(),
        "bin"
            | "axdt"
            | "addin"
            | "cf"
            | "cfe"
            | "epf"
            | "erf"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "bmp"
            | "ico"
            | "zip"
            | "7z"
            | "gz"
    ) {
        Some(RepositoryResourceKind::Binary)
    } else {
        None
    }
}

#[cfg(test)]
fn parse_attribute_records(
    stdout: &str,
    expected_paths: &[String],
) -> Result<BTreeMap<String, AttributeValues>, String> {
    parse_attribute_records_with_checkpoint(stdout, expected_paths, &mut || Ok(()))
        .map_err(ResourceProtocolParseError::into_reason)
}

fn parse_attribute_records_controlled(
    stdout: &str,
    expected_paths: &[String],
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<BTreeMap<String, AttributeValues>, ResourceProtocolParseError> {
    parse_attribute_records_with_checkpoint(stdout, expected_paths, &mut || {
        resource_protocol_checkpoint(cancellation, deadline)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResourceProtocolParseError {
    Malformed(String),
    Cancelled,
    TimedOut,
}

impl ResourceProtocolParseError {
    #[cfg(test)]
    fn into_reason(self) -> String {
        match self {
            Self::Malformed(reason) => reason,
            Self::Cancelled => "Git resource protocol parsing was cancelled".into(),
            Self::TimedOut => "Git resource protocol parsing timed out".into(),
        }
    }
}

fn parse_attribute_records_with_checkpoint(
    stdout: &str,
    expected_paths: &[String],
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<BTreeMap<String, AttributeValues>, ResourceProtocolParseError> {
    checkpoint()?;
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(ResourceProtocolParseError::Malformed(
            "Git check-attr output is missing its terminal NUL".into(),
        ));
    }
    let mut expected = BTreeSet::new();
    for (path_index, path) in expected_paths.iter().enumerate() {
        if path_index.is_multiple_of(256) {
            checkpoint()?;
        }
        expected.insert(path.as_str());
    }
    let mut raw = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut fields = stdout.split_terminator('\0');
    let mut record_index = 0_usize;
    loop {
        if record_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let Some(path) = fields.next() else { break };
        let attribute = fields.next().ok_or_else(|| {
            ResourceProtocolParseError::Malformed(
                "Git check-attr output is not a sequence of triples".into(),
            )
        })?;
        let value = fields.next().ok_or_else(|| {
            ResourceProtocolParseError::Malformed(
                "Git check-attr output is not a sequence of triples".into(),
            )
        })?;
        if !expected.contains(path) {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "Git check-attr returned unexpected path {path}"
            )));
        }
        let attributes = raw.entry(path.to_owned()).or_default();
        if attributes
            .insert(attribute.to_owned(), value.to_owned())
            .is_some()
        {
            return Err(ResourceProtocolParseError::Malformed(
                "Git check-attr returned a duplicate attribute".into(),
            ));
        }
        record_index += 1;
    }
    let mut records = BTreeMap::new();
    for (path_index, path) in expected_paths.iter().enumerate() {
        if path_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let attributes = raw.remove(path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!("Git check-attr omitted {path}"))
        })?;
        if attributes.len() != 3 {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "Git check-attr returned incomplete attributes for {path}"
            )));
        }
        records.insert(
            path.clone(),
            AttributeValues {
                text: required_attribute(&attributes, "text", path)
                    .map_err(ResourceProtocolParseError::Malformed)?,
                eol: required_attribute(&attributes, "eol", path)
                    .map_err(ResourceProtocolParseError::Malformed)?,
                filter: required_attribute(&attributes, "filter", path)
                    .map_err(ResourceProtocolParseError::Malformed)?,
            },
        );
    }
    Ok(records)
}

fn required_attribute(
    attributes: &BTreeMap<String, String>,
    name: &str,
    path: &str,
) -> Result<String, String> {
    attributes
        .get(name)
        .cloned()
        .ok_or_else(|| format!("Git check-attr omitted {name} for {path}"))
}

#[cfg(test)]
fn parse_eol_records(
    stdout: &str,
    expected_text_paths: &[String],
) -> Result<BTreeMap<String, EolValues>, String> {
    parse_eol_records_with_checkpoint(stdout, expected_text_paths, &mut || Ok(()))
        .map_err(ResourceProtocolParseError::into_reason)
}

fn parse_eol_records_controlled(
    stdout: &str,
    expected_text_paths: &[String],
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<BTreeMap<String, EolValues>, ResourceProtocolParseError> {
    parse_eol_records_with_checkpoint(stdout, expected_text_paths, &mut || {
        resource_protocol_checkpoint(cancellation, deadline)
    })
}

fn resource_protocol_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), ResourceProtocolParseError> {
    if cancellation.is_cancelled() {
        Err(ResourceProtocolParseError::Cancelled)
    } else if deadline.is_elapsed() {
        Err(ResourceProtocolParseError::TimedOut)
    } else {
        Ok(())
    }
}

fn resource_process_error(
    reason: String,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ResourceProtocolParseError {
    if cancellation.is_cancelled() {
        ResourceProtocolParseError::Cancelled
    } else if deadline.is_elapsed() {
        ResourceProtocolParseError::TimedOut
    } else {
        ResourceProtocolParseError::Malformed(reason)
    }
}

fn evaluate_attribute_policy_with_checkpoint(
    resources: &[RepositoryResource],
    staged: &BTreeMap<String, AttributeValues>,
    effective: &BTreeMap<String, AttributeValues>,
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<AttributePolicyEvaluation, ResourceProtocolParseError> {
    let mut evaluation = AttributePolicyEvaluation {
        facts: Vec::new(),
        text_marked_binary: BTreeSet::new(),
    };
    for (resource_index, resource) in resources.iter().enumerate() {
        if resource_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let attributes = staged.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "staged Git attribute policy omitted {}",
                resource.repo_path
            ))
        })?;
        let effective_attributes = effective.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "effective Git attribute policy omitted {}",
                resource.repo_path
            ))
        })?;
        let staged_policy_sufficient = policy_satisfied(resource.kind, attributes);
        if !staged_policy_sufficient && policy_satisfied(resource.kind, effective_attributes) {
            evaluation
                .facts
                .push(ProjectHealthFact::AttributesLocalOnly {
                    source_set: resource.source_set.clone(),
                    path: resource.repo_path.clone(),
                    evidence: vec![format!(
                        "local-only text={} eol={} filter={}",
                        effective_attributes.text,
                        effective_attributes.eol,
                        effective_attributes.filter
                    )],
                });
            continue;
        }
        match resource.kind {
            RepositoryResourceKind::Text if attributes.text == "unset" => {
                evaluation
                    .text_marked_binary
                    .insert(resource.repo_path.clone());
                evaluation
                    .facts
                    .push(ProjectHealthFact::TextResourceMarkedBinary {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
            }
            RepositoryResourceKind::Text if !text_policy_satisfied(attributes) => {
                evaluation.facts.push(ProjectHealthFact::TextPolicyMissing {
                    source_set: resource.source_set.clone(),
                    path: resource.repo_path.clone(),
                });
            }
            RepositoryResourceKind::Binary if attributes.text != "unset" => {
                evaluation
                    .facts
                    .push(ProjectHealthFact::BinaryPolicyMissing {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
            }
            RepositoryResourceKind::Text | RepositoryResourceKind::Binary => {}
        }
    }
    checkpoint()?;
    Ok(evaluation)
}

fn parse_eol_records_with_checkpoint(
    stdout: &str,
    expected_text_paths: &[String],
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<BTreeMap<String, EolValues>, ResourceProtocolParseError> {
    checkpoint()?;
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(ResourceProtocolParseError::Malformed(
            "git ls-files --eol output is missing its terminal NUL".into(),
        ));
    }
    let mut result = BTreeMap::new();
    for (record_index, record) in stdout.split_terminator('\0').enumerate() {
        if record_index % 256 == 0 {
            checkpoint()?;
        }
        let (metadata, path) = record.split_once('\t').ok_or_else(|| {
            ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has no tab separator".into(),
            )
        })?;
        if path.is_empty() {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has an empty path".into(),
            ));
        }
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 2 || !fields[0].starts_with("i/") || !fields[1].starts_with("w/") {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has invalid metadata".into(),
            ));
        }
        let index = &fields[0][2..];
        let worktree = &fields[1][2..];
        if !matches!(index, "" | "lf" | "crlf" | "mixed" | "none" | "-text") {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol returned unsupported index EOL metadata {index}"
            )));
        }
        if !matches!(worktree, "" | "lf" | "crlf" | "mixed" | "none" | "-text") {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol returned unsupported worktree EOL metadata {worktree}"
            )));
        }
        if result
            .insert(
                path.into(),
                EolValues {
                    index: fields[0][2..].into(),
                    worktree: fields[1][2..].into(),
                },
            )
            .is_some()
        {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol returned a duplicate path".into(),
            ));
        }
    }
    for (path_index, path) in expected_text_paths.iter().enumerate() {
        if path_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let observed = result.get(path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!("git ls-files --eol omitted {path}"))
        })?;
        if observed.index.is_empty() {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol returned empty index EOL metadata for tracked text path {path}"
            )));
        }
    }
    Ok(result)
}

fn index_eol_facts_with_checkpoint(
    resources: &[RepositoryResource],
    eol: &BTreeMap<String, EolValues>,
    text_marked_binary: &BTreeSet<String>,
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<Vec<ProjectHealthFact>, ResourceProtocolParseError> {
    let mut facts = Vec::new();
    for (resource_index, resource) in resources.iter().enumerate() {
        if resource_index.is_multiple_of(256) {
            checkpoint()?;
        }
        if resource.kind != RepositoryResourceKind::Text
            || text_marked_binary.contains(&resource.repo_path)
        {
            continue;
        }
        let observed = eol.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol omitted {}",
                resource.repo_path
            ))
        })?;
        if !matches!(observed.index.as_str(), "lf" | "none") {
            facts.push(ProjectHealthFact::IndexEolNotLf {
                source_set: resource.source_set.clone(),
                path: resource.repo_path.clone(),
                observed: observed.index.clone(),
            });
        }
    }
    checkpoint()?;
    Ok(facts)
}

fn lfs_facts_with_checkpoint(
    binary_by_source: BTreeMap<String, Vec<(String, u64)>>,
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<Vec<ProjectHealthFact>, ResourceProtocolParseError> {
    let mut facts = Vec::new();
    for (source_index, (source_set, files)) in binary_by_source.into_iter().enumerate() {
        if source_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let mut total_bytes = 0_u64;
        let mut largest = None::<&(String, u64)>;
        let mut sample_paths = BTreeSet::new();
        for (file_index, file) in files.iter().enumerate() {
            if file_index.is_multiple_of(256) {
                checkpoint()?;
            }
            total_bytes = total_bytes.saturating_add(file.1);
            if largest.is_none_or(|current| file.1 > current.1) {
                largest = Some(file);
            }
            sample_paths.insert(file.0.clone());
            if sample_paths.len() > crate::MAX_PROJECT_DIAGNOSTIC_PATHS {
                sample_paths.pop_last();
            }
        }
        if let Some((largest_path, largest_bytes)) = largest.filter(|(_, largest)| {
            *largest >= LFS_SINGLE_FILE_THRESHOLD_BYTES
                || total_bytes >= LFS_AGGREGATE_THRESHOLD_BYTES
        }) {
            facts.push(ProjectHealthFact::LfsConsider {
                source_set,
                count: files.len(),
                total_bytes,
                largest_path: largest_path.clone(),
                largest_bytes: *largest_bytes,
                single_threshold_bytes: LFS_SINGLE_FILE_THRESHOLD_BYTES,
                aggregate_threshold_bytes: LFS_AGGREGATE_THRESHOLD_BYTES,
                paths: sample_paths.into_iter().collect(),
            });
        }
    }
    checkpoint()?;
    Ok(facts)
}

fn process_command(
    cwd: &Path,
    args: Vec<String>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ProcessCommand {
    ProcessCommand {
        program: PathBuf::from("git"),
        args,
        cwd: cwd.to_path_buf(),
        env: vec![
            (OsString::from("LC_ALL"), OsString::from("C")),
            (OsString::from("LANG"), OsString::from("C")),
            (OsString::from("GIT_NO_LAZY_FETCH"), OsString::from("1")),
            (
                OsString::from("GIT_NO_REPLACE_OBJECTS"),
                OsString::from("1"),
            ),
            (OsString::from("GIT_OPTIONAL_LOCKS"), OsString::from("0")),
            (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
            (OsString::from("GIT_CONFIG_COUNT"), OsString::from("1")),
            (
                OsString::from("GIT_CONFIG_KEY_0"),
                OsString::from("core.fsmonitor"),
            ),
            (
                OsString::from("GIT_CONFIG_VALUE_0"),
                OsString::from("false"),
            ),
            (OsString::from("GIT_ATTR_NOSYSTEM"), OsString::from("1")),
        ],
        env_remove: super::git::git_environment_removals(),
        capture_limits: Some((
            super::PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT,
            unica_process::STDERR_CAPTURE_LIMIT,
        )),
        timeout: deadline.remaining(),
        cancellation: cancellation.clone(),
    }
}

fn isolated_git_command(
    cwd: &Path,
    index: &IsolatedGitIndex,
    args: Vec<String>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ProcessCommand {
    let mut command = process_command(cwd, args, cancellation, deadline);
    command.args.splice(
        0..0,
        [
            format!(
                "--git-dir={}",
                host_path_text(index.git_dir.display().to_string())
            ),
            format!(
                "--work-tree={}",
                host_path_text(index.worktree.display().to_string())
            ),
        ],
    );
    command.env.extend([
        (
            OsString::from("GIT_INDEX_FILE"),
            index.index.as_os_str().to_os_string(),
        ),
        (
            OsString::from("GIT_OBJECT_DIRECTORY"),
            index.object_directory.as_os_str().to_os_string(),
        ),
    ]);
    command
}

fn semantic_success(output: &ProcessOutput) -> bool {
    output.status_success
        && !output.timed_out
        && !output.cancelled
        && !output.stdout_truncated
        && !output.stderr_truncated
        && !output.stdout_had_invalid_utf8
        && !output.stderr_had_invalid_utf8
}

fn sticky_process_result(
    result: Result<ProcessOutput, String>,
    cancellation: &CancellationToken,
) -> Result<ProcessOutput, String> {
    if cancellation.is_cancelled() {
        return Ok(ProcessOutput {
            cancelled: true,
            status_success: false,
            status: "cancelled".into(),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        });
    }
    result
}

fn output_reason(output: &ProcessOutput) -> String {
    if output.timed_out {
        "Git resource inspection timed out".into()
    } else if output.stdout_truncated || output.stderr_truncated {
        "Git resource inspection output was truncated".into()
    } else if output.stdout_had_invalid_utf8 || output.stderr_had_invalid_utf8 {
        "Git resource inspection output contained invalid UTF-8".into()
    } else if output.stderr.trim().is_empty() {
        format!("Git resource inspection exited with {}", output.status)
    } else {
        format!(
            "Git resource inspection exited with {}: {}",
            output.status,
            output.stderr.trim()
        )
    }
}

fn incomplete_policy(
    mut observations: Vec<ProjectCheckObservation>,
    check: ProjectCheckId,
    reason: String,
) -> RepositoryPolicyInspection {
    let started = resource_check_ids()
        .into_iter()
        .position(|candidate| candidate == check)
        .unwrap_or(0);
    for dependent in resource_check_ids().into_iter().skip(started) {
        mark_check_not_run(&mut observations, dependent, &reason);
    }
    RepositoryPolicyInspection {
        observations,
        facts: vec![ProjectHealthFact::GitInspectionIncomplete {
            check,
            source_set: None,
            reason,
        }],
    }
}

fn retain_prior_facts(
    mut policy: RepositoryPolicyInspection,
    mut prior_facts: Vec<ProjectHealthFact>,
) -> RepositoryPolicyInspection {
    prior_facts.append(&mut policy.facts);
    policy.facts = prior_facts;
    policy
}

fn timeout_policy(
    mut observations: Vec<ProjectCheckObservation>,
    check: ProjectCheckId,
) -> RepositoryPolicyInspection {
    let reason = "Git resource protocol parsing exceeded the inspection deadline";
    let started = resource_check_ids()
        .into_iter()
        .position(|candidate| candidate == check)
        .unwrap_or(0);
    for dependent in resource_check_ids().into_iter().skip(started) {
        mark_check_not_run(&mut observations, dependent, reason);
    }
    RepositoryPolicyInspection {
        observations,
        facts: vec![ProjectHealthFact::GitInspectionTimeout {
            check,
            source_set: None,
        }],
    }
}

fn mark_check_completed(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    incomplete_source_sets: &BTreeSet<String>,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && match observation.source_set.as_ref() {
                Some(source_set) => !incomplete_source_sets.contains(source_set),
                None => incomplete_source_sets.is_empty(),
            }
    }) {
        if matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. }) {
            observation.outcome = ProjectCheckOutcome::Completed;
        }
    }
}

fn mark_check_not_run(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    reason: &str,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && !matches!(
                observation.outcome,
                ProjectCheckOutcome::NotApplicable { .. }
            )
    }) {
        observation.outcome = ProjectCheckOutcome::NotRun {
            reason: reason.to_owned(),
        };
    }
}

fn mark_source_check_not_run(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    source_set: &str,
    reason: &str,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && observation.source_set.as_deref() == Some(source_set)
            && !matches!(
                observation.outcome,
                ProjectCheckOutcome::NotApplicable { .. }
            )
    }) {
        observation.outcome = ProjectCheckOutcome::NotRun {
            reason: reason.to_owned(),
        };
    }
}

fn resource_check_ids() -> [ProjectCheckId; 4] {
    [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ]
}

pub(super) fn resource_observations<'a>(
    source_sets: impl IntoIterator<Item = &'a unica_source_contract::ProjectSourceSet>,
    not_run_reason: &str,
    source_profiles_complete: bool,
) -> Vec<ProjectCheckObservation> {
    let mut source_sets = source_sets.into_iter().collect::<Vec<_>>();
    source_sets.sort_by(|left, right| left.name.cmp(&right.name));
    source_sets.dedup_by(|left, right| left.name == right.name);
    let has_platform_xml = source_sets
        .iter()
        .any(|source_set| source_set.source_format == SourceFormat::PlatformXml);
    let mut observations = Vec::new();
    for id in resource_check_ids() {
        observations.push(ProjectCheckObservation {
            id,
            scope: id.scope(),
            source_set: None,
            outcome: if source_profiles_complete && !has_platform_xml {
                ProjectCheckOutcome::NotApplicable {
                    reason: "no Platform XML source roots were proven".into(),
                }
            } else {
                ProjectCheckOutcome::NotRun {
                    reason: not_run_reason.into(),
                }
            },
        });
        observations.extend(
            source_sets
                .iter()
                .map(|source_set| ProjectCheckObservation {
                    id,
                    scope: id.scope(),
                    source_set: Some(source_set.name.clone()),
                    outcome: if source_set.source_format == SourceFormat::Edt {
                        ProjectCheckOutcome::NotApplicable {
                            reason: "EDT has no format-specific repository resource policy".into(),
                        }
                    } else {
                        ProjectCheckOutcome::NotRun {
                            reason: not_run_reason.into(),
                        }
                    },
                }),
        );
    }
    observations
}

fn text_policy_satisfied(attributes: &AttributeValues) -> bool {
    attributes.text != "unset"
        && (matches!(attributes.text.as_str(), "set" | "auto")
            || matches!(attributes.eol.as_str(), "lf" | "crlf"))
}

fn policy_satisfied(kind: RepositoryResourceKind, attributes: &AttributeValues) -> bool {
    match kind {
        RepositoryResourceKind::Text => text_policy_satisfied(attributes),
        RepositoryResourceKind::Binary => attributes.text == "unset",
    }
}

fn nul_input(paths: &[String]) -> Vec<u8> {
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    input
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkingEol {
    Supported,
    Mixed,
    BareCr,
}

#[derive(Debug, PartialEq, Eq)]
enum WorkingEolInspectionError {
    Cancelled,
    TimedOut,
    Incomplete(String),
}

impl std::fmt::Display for WorkingEolInspectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("working EOL inspection was cancelled"),
            Self::TimedOut => formatter.write_str("deadline expired during working EOL inspection"),
            Self::Incomplete(reason) => formatter.write_str(reason),
        }
    }
}

fn inspect_working_eol(
    repository_root: &Path,
    repo_path: &Path,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Option<WorkingEol>, WorkingEolInspectionError> {
    let file = match open_repository_regular_file_nofollow(repository_root, repo_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkingEolInspectionError::Incomplete(error.to_string())),
    };
    inspect_working_eol_reader(file, cancellation, deadline)
}

fn inspect_working_eol_continued(
    repository_root: &Path,
    repo_path: &Path,
    continuation: &mut ResourceContinuation,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(Option<WorkingEol>, Option<WorkingFileFingerprint>), WorkingEolInspectionError> {
    let path = repo_path.to_string_lossy();
    let key = working_evidence_key(&path);
    let mut file = match open_repository_regular_file_nofollow(repository_root, repo_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            continuation.forget_working(&key);
            return Ok((None, None));
        }
        Err(error) => return Err(WorkingEolInspectionError::Incomplete(error.to_string())),
    };
    let before = working_file_fingerprint(&file)
        .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?;
    let mut scanner = match continuation.working_eol.get(&key) {
        Some(cached) if cached.fingerprint == before => match cached.eol {
            WorkingEolProgress::Complete(eol) => {
                continuation.progress.working_eol_retained += 1;
                return Ok((Some(eol), Some(before)));
            }
            WorkingEolProgress::Partial(scanner) => scanner,
        },
        _ => WorkingEolScanner::default(),
    };
    continuation.forget_working(&key);
    if scanner.offset > before.size {
        return Err(WorkingEolInspectionError::Incomplete(
            "working EOL checkpoint exceeds file size".into(),
        ));
    }
    if scanner.offset != 0 {
        file.seek(SeekFrom::Start(scanner.offset))
            .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?;
    }
    let eol = match inspect_working_eol_reader_from_state(
        &mut file,
        &mut scanner,
        cancellation,
        deadline,
    ) {
        Ok(eol) => eol,
        Err(WorkingEolInspectionError::TimedOut) => {
            // Keep the original fingerprint, never a newer one attached to an
            // old prefix. Reopening on the next request validates this state;
            // saving it performs no filesystem work after the deadline.
            continuation.remember_working(
                key,
                WorkingEolEvidence {
                    fingerprint: before,
                    eol: WorkingEolProgress::Partial(scanner),
                },
            );
            return Err(WorkingEolInspectionError::TimedOut);
        }
        Err(error) => return Err(error),
    };
    let after = working_file_fingerprint(&file)
        .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?;
    if before != after {
        return Err(WorkingEolInspectionError::Incomplete(
            "working file changed during EOL inspection".into(),
        ));
    }
    let named = open_repository_regular_file_nofollow(repository_root, repo_path)
        .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?;
    if working_file_fingerprint(&named)
        .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?
        != before
    {
        return Err(WorkingEolInspectionError::Incomplete(
            "working path changed during EOL inspection".into(),
        ));
    }
    if let Some(eol) = eol {
        continuation.remember_working(
            key,
            WorkingEolEvidence {
                fingerprint: before.clone(),
                eol: WorkingEolProgress::Complete(eol),
            },
        );
        if continuation.working_eol.contains_key(&key) {
            continuation.progress.working_eol_retained += 1;
        }
    }
    Ok((eol, Some(before)))
}

fn working_file_fingerprint(file: &File) -> std::io::Result<WorkingFileFingerprint> {
    let metadata = file.metadata()?;
    Ok(WorkingFileFingerprint {
        identity: (file).identity()?,
        size: metadata.len(),
        modified: metadata.modified()?,
        change_time: file.change_time(&metadata)?,
    })
}

fn inspect_working_eol_reader(
    file: impl Read,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Option<WorkingEol>, WorkingEolInspectionError> {
    inspect_working_eol_reader_from_state(
        file,
        &mut WorkingEolScanner::default(),
        cancellation,
        deadline,
    )
}

fn working_eol_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), WorkingEolInspectionError> {
    if cancellation.is_cancelled() {
        return Err(WorkingEolInspectionError::Cancelled);
    }
    if deadline.is_elapsed() {
        return Err(WorkingEolInspectionError::TimedOut);
    }
    Ok(())
}

fn inspect_working_eol_reader_from_state(
    mut file: impl Read,
    scanner: &mut WorkingEolScanner,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Option<WorkingEol>, WorkingEolInspectionError> {
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        working_eol_checkpoint(cancellation, deadline)?;
        let count = file
            .read(&mut buffer)
            .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?;
        if count == 0 {
            working_eol_checkpoint(cancellation, deadline)?;
            break;
        }
        let next_offset = scanner.offset.checked_add(count as u64).ok_or_else(|| {
            WorkingEolInspectionError::Incomplete("working EOL offset overflow".into())
        })?;
        for byte in &buffer[..count] {
            if scanner.previous_cr {
                if *byte == b'\n' {
                    scanner.crlf = true;
                    scanner.previous_cr = false;
                    continue;
                }
                scanner.bare_cr = true;
                scanner.previous_cr = false;
            }
            match *byte {
                b'\r' => scanner.previous_cr = true,
                b'\n' => scanner.lf = true,
                _ => {}
            }
        }
        scanner.offset = next_offset;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = AFTER_WORKING_EOL_BLOCK.with(|slot| slot.borrow_mut().take()) {
            hook()?;
        }
    }
    let bare_cr = scanner.bare_cr || scanner.previous_cr;
    let styles = usize::from(scanner.lf) + usize::from(scanner.crlf) + usize::from(bare_cr);
    Ok(Some(if styles > 1 {
        WorkingEol::Mixed
    } else if bare_cr {
        WorkingEol::BareCr
    } else {
        WorkingEol::Supported
    }))
}

fn open_repository_regular_file_nofollow(
    repository_root: &Path,
    repo_path: &Path,
) -> std::io::Result<File> {
    use std::path::Component;

    if !repository_root.is_absolute() || repo_path.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "repository root must be absolute and resource path relative",
        ));
    }
    let mut components = repo_path.components().peekable();
    let mut parent = open_absolute_read_directory_nofollow(repository_root)?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "resource path contains a non-normal component",
            ));
        };
        let (classification_anchor, kind) = open_any_read_child_nofollow(&parent, name)?;
        if components.peek().is_some() {
            if kind != OpenedChildKind::Directory {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "resource path contains a linked or non-directory parent",
                ));
            }
            parent = open_read_child_for_secure_tree_use(
                &parent,
                name,
                classification_anchor,
                OpenedChildKind::Directory,
            )?;
        } else if kind == OpenedChildKind::RegularFile {
            return open_read_child_for_secure_tree_use(
                &parent,
                name,
                classification_anchor,
                OpenedChildKind::RegularFile,
            );
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "resource path is linked or is not a regular file",
            ));
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "resource path is empty",
    ))
}

fn repository_regular_file_size(
    repository_root: &Path,
    repo_path: &Path,
) -> Result<Option<u64>, String> {
    let file = match open_repository_regular_file_nofollow(repository_root, repo_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("binary working path is not a regular file".into());
    }
    Ok(Some(metadata.len()))
}

#[cfg(test)]
#[path = "../../tests/platform/project_health_resources.rs"]
mod tests;

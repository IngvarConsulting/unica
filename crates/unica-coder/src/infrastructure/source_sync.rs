//! Durable source-sync repository and platform-XML target resolution.

use crate::domain::project_sources::{
    discover_project_source_map, ProjectSourceSet, SourceFormat, SourceSetKind,
};
use crate::domain::source_sync::{
    FileFingerprint, RelativeSourcePath, SourceManifest, SourceSetName, SourceSyncState,
    SourceTarget, SourceTargetKind, SourceTargetRecord, SourceTargetScope, SynchronizedManifest,
    TargetId, SOURCE_SYNC_SCHEMA_VERSION,
};
use crate::domain::workspace::WorkspaceContext;
use fs2::FileExt;
use roxmltree::Document;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const STATE_FILE_NAME: &str = "state.json";
const LOCK_FILE_NAME: &str = "lifecycle.lock";
const PUBLICATIONS_DIR_NAME: &str = "publications";
const PUBLICATION_JOURNAL_NAME: &str = "journal.json";
const PUBLICATION_SCHEMA_VERSION: u32 = 2;

#[derive(Debug)]
pub struct LifecycleLockGuard {
    file: File,
    workspace_id: String,
}

/// A duplicate of the lifecycle lock deliberately kept inheritable while an
/// external runtime process is being spawned. On Unix `flock` is associated
/// with the open file description, so a v8-runner/Designer descendant retains
/// the workspace lease if the Unica parent is terminated unexpectedly.
#[derive(Debug)]
pub struct LifecycleChildLease {
    _file: File,
}

impl LifecycleLockGuard {
    pub fn child_lease(&self) -> Result<LifecycleChildLease, String> {
        let file = self
            .file
            .try_clone()
            .map_err(|error| format!("failed to duplicate lifecycle lock for child: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;

            let descriptor = file.as_raw_fd();
            // SAFETY: `descriptor` belongs to `file` for this entire block.
            // fcntl only reads/updates its close-on-exec descriptor flags.
            let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
            if flags == -1 {
                return Err(format!(
                    "failed to inspect lifecycle child lease flags: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: as above; clearing FD_CLOEXEC makes this duplicate
            // survive only the immediately spawned child/descendant chain.
            let result =
                unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) };
            if result == -1 {
                return Err(format!(
                    "failed to make lifecycle child lease inheritable: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(LifecycleChildLease { _file: file })
        }
        #[cfg(not(unix))]
        {
            drop(file);
            Err(
                "source-sync runtime operations are blocked on this platform until an inheritable lifecycle lease is implemented"
                    .to_string(),
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirtyTargetSnapshot {
    pub generation: u64,
    pub targets: Vec<SourceTargetRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineReceipt {
    pub target_id: TargetId,
    pub previous_generation: u64,
    pub generation: u64,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationRecordResult {
    pub generation: u64,
    pub target: SourceTargetRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynchronizationConflict {
    pub target_id: TargetId,
    pub reason: String,
    pub expected: Option<SourceManifest>,
    pub current: Option<SourceManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynchronizationCasResult {
    pub generation: u64,
    pub processed: Vec<TargetId>,
    pub conflicted: Vec<SynchronizationConflict>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublicationRecoveryReport {
    pub rolled_back: Vec<String>,
    pub cleaned_committed: Vec<String>,
    pub cleaned_unprepared: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformCdfiPreimage {
    source_set: SourceSetName,
    source_root: RelativeSourcePath,
    path: RelativeSourcePath,
    original: FileFingerprint,
    original_bytes: Option<Vec<u8>>,
    configuration_path: RelativeSourcePath,
    configuration_original: FileFingerprint,
    configuration_bytes: Vec<u8>,
    guarded_owners: Vec<PlatformGuardedOwner>,
}

impl PlatformCdfiPreimage {
    pub fn source_root(&self) -> &RelativeSourcePath {
        &self.source_root
    }

    pub fn configuration_bytes(&self) -> &[u8] {
        &self.configuration_bytes
    }

    pub fn config_dump_info_bytes(&self) -> Option<&[u8]> {
        self.original_bytes.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformGuardedOwner {
    target: SourceTarget,
    excluded_module_paths: BTreeSet<RelativeSourcePath>,
    original: SourceManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationOutcome {
    pub published_paths: Vec<RelativeSourcePath>,
    pub cleanup_warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationError {
    pub reason: &'static str,
    pub message: String,
    pub affected_paths: Vec<RelativeSourcePath>,
    pub source_may_have_changed: bool,
    pub recovery_required: bool,
}

impl PublicationError {
    fn before_write(message: String) -> Self {
        Self {
            reason: "publicationPreflightFailed",
            message,
            affected_paths: Vec::new(),
            source_may_have_changed: false,
            recovery_required: false,
        }
    }

    fn rolled_back(message: String, affected_paths: Vec<RelativeSourcePath>) -> Self {
        Self {
            reason: "publicationRolledBack",
            message,
            affected_paths,
            source_may_have_changed: false,
            recovery_required: false,
        }
    }

    fn recovery_required(message: String, affected_paths: Vec<RelativeSourcePath>) -> Self {
        Self {
            reason: "publicationRecoveryRequired",
            message,
            affected_paths,
            source_may_have_changed: true,
            recovery_required: true,
        }
    }
}

impl fmt::Display for PublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PublicationError {}

impl From<String> for PublicationError {
    fn from(message: String) -> Self {
        Self::before_write(message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationFailureHandling {
    Rollback,
    #[cfg(test)]
    LeavePrepared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum PublicationPhase {
    Prepared,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicationJournal {
    schema_version: u32,
    workspace_id: String,
    workspace_root: String,
    transaction_id: String,
    phase: PublicationPhase,
    files: Vec<PublicationJournalFile>,
    created_directories: Vec<RelativeSourcePath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublicationJournalFile {
    role: PublicationFileRole,
    path: RelativeSourcePath,
    original: FileFingerprint,
    desired: FileFingerprint,
    backup_file: Option<String>,
    stage_path: Option<RelativeSourcePath>,
    original_mode: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum PublicationFileRole {
    TargetOwned,
    PlatformConfigDumpInfo,
}

#[derive(Debug)]
struct PublicationFilePlan {
    journal: PublicationJournalFile,
    desired_bytes: Option<Vec<u8>>,
}

#[derive(Debug)]
struct PublicationPlan {
    transaction_dir: PathBuf,
    journal: PublicationJournal,
    files: Vec<PublicationFilePlan>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct PublicationJournalWriteOutcome {
    commit_warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationScope {
    source_set: SourceSetName,
    source_root: RelativeSourcePath,
    alternate_source_root: PathBuf,
}

struct PublicationRequest<'a> {
    guard: &'a LifecycleLockGuard,
    requested: &'a [SourceTargetRecord],
    desired: &'a BTreeMap<TargetId, SourceManifest>,
    alternate_source_root: &'a Path,
    cdfi_preimage: &'a PlatformCdfiPreimage,
}

#[derive(Debug, Clone)]
pub struct SourceSyncRepository {
    workspace_root: PathBuf,
    workspace_root_text: String,
    workspace_id: String,
    authority_root: PathBuf,
    transaction_root: PathBuf,
    state_path: PathBuf,
    lock_path: PathBuf,
}

impl SourceSyncRepository {
    pub fn new(context: &WorkspaceContext) -> Result<Self, String> {
        let workspace_root = context.workspace_root.canonicalize().map_err(|error| {
            format!(
                "failed to canonicalize source-sync workspace {}: {error}",
                context.workspace_root.display()
            )
        })?;
        if !workspace_root.is_dir() {
            return Err(format!(
                "source-sync workspace is not a directory: {}",
                workspace_root.display()
            ));
        }
        let workspace_root_text = path_for_identity(&workspace_root);
        let workspace_id = format!("{:x}", Sha256::digest(workspace_root_text.as_bytes()));
        let authority_root = workspace_root.join(".build").join("unica");
        let transaction_root = authority_root.join("source-sync").join(&workspace_id);
        Ok(Self {
            workspace_root,
            workspace_root_text,
            workspace_id,
            authority_root,
            state_path: transaction_root.join(STATE_FILE_NAME),
            lock_path: transaction_root.join(LOCK_FILE_NAME),
            transaction_root,
        })
    }

    #[cfg(test)]
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub fn transaction_root(&self) -> &Path {
        &self.transaction_root
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn acquire_lifecycle_lock(&self) -> Result<LifecycleLockGuard, String> {
        self.prepare_storage()?;
        reject_symlink(&self.lock_path, MissingPath::Allowed)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&self.lock_path)
            .map_err(|error| format!("failed to open lifecycle lock: {error}"))?;
        file.lock_exclusive()
            .map_err(|error| format!("failed to acquire lifecycle lock: {error}"))?;
        Ok(LifecycleLockGuard {
            file,
            workspace_id: self.workspace_id.clone(),
        })
    }

    pub fn load_state(&self) -> Result<SourceSyncState, String> {
        self.validate_existing_storage()?;
        match fs::symlink_metadata(&self.state_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "source-sync refuses symlink {}",
                    self.state_path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(self.empty_state());
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect source-sync state {}: {error}",
                    self.state_path.display()
                ));
            }
        }
        let bytes = fs::read(&self.state_path).map_err(|error| {
            format!(
                "failed to read source-sync state {}: {error}",
                self.state_path.display()
            )
        })?;
        let value = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
            format!(
                "source-sync state {} is malformed; refusing to treat it as empty: {error}",
                self.state_path.display()
            )
        })?;
        let schema = value
            .get("schemaVersion")
            .and_then(Value::as_u64)
            .ok_or_else(|| "source-sync state has no valid schemaVersion".to_string())?;
        if schema > u64::from(SOURCE_SYNC_SCHEMA_VERSION) {
            return Err(format!(
                "source-sync state uses newer schemaVersion {schema}; supported {}",
                SOURCE_SYNC_SCHEMA_VERSION
            ));
        }
        if schema != u64::from(SOURCE_SYNC_SCHEMA_VERSION) {
            return Err(format!(
                "unsupported source-sync schemaVersion {schema}; expected {}",
                SOURCE_SYNC_SCHEMA_VERSION
            ));
        }
        let state = serde_json::from_value::<SourceSyncState>(value)
            .map_err(|error| format!("source-sync state violates schema: {error}"))?;
        state.validate()?;
        if state.workspace_id != self.workspace_id
            || state.workspace_root != self.workspace_root_text
        {
            return Err(format!(
                "source-sync state belongs to foreign workspace `{}` at `{}`",
                state.workspace_id, state.workspace_root
            ));
        }
        Ok(state)
    }

    #[cfg(test)]
    pub fn has_active_state(&self) -> Result<bool, String> {
        Ok(self
            .load_state()?
            .targets
            .values()
            .any(SourceTargetRecord::is_dirty))
    }

    #[cfg(test)]
    pub fn target(&self, id: &TargetId) -> Result<Option<SourceTargetRecord>, String> {
        Ok(self.load_state()?.targets.get(id).cloned())
    }

    #[cfg(test)]
    pub fn dirty_targets(&self, source_set: Option<&str>) -> Result<DirtyTargetSnapshot, String> {
        let state = self.load_state()?;
        let targets = state
            .targets
            .values()
            .filter(|record| record.is_dirty())
            .filter(|record| {
                source_set.is_none_or(|expected| {
                    record
                        .target
                        .source_set
                        .as_ref()
                        .is_some_and(|actual| actual.as_str() == expected)
                })
            })
            .cloned()
            .collect();
        Ok(DirtyTargetSnapshot {
            generation: state.generation,
            targets,
        })
    }

    pub fn capture_manifest(&self, target: &SourceTarget) -> Result<SourceManifest, String> {
        target.validate()?;
        capture_manifest_at(&self.workspace_root, target)
    }

    /// Prove that a persisted target still refers to the same configured
    /// platform-XML configuration source set and canonical working root.
    pub fn validate_target_topology(&self, record: &SourceTargetRecord) -> Result<(), String> {
        validate_target_topology(self, record)
    }

    /// Capture a target from a shadow source root while retaining working-tree
    /// manifest keys. Previously known paths missing from the shadow are
    /// represented as explicit deletions.
    pub fn capture_manifest_from_source_root(
        &self,
        target: &SourceTarget,
        alternate_source_root: &Path,
    ) -> Result<SourceManifest, String> {
        target.validate()?;
        let captured = capture_manifest_at_alternate_root(target, alternate_source_root)?;
        let state = self.load_state()?;
        let existing = state.targets.get(&target.id);
        Ok(normalize_manifest(
            captured,
            existing.map(|record| &record.current),
            existing.map(|record| &record.synchronized),
        ))
    }

    /// Prove that a Designer shadow still represents the exact persisted
    /// metadata owner before any bytes can be classified or published.
    pub fn validate_shadow_target(
        &self,
        record: &SourceTargetRecord,
        alternate_source_root: &Path,
    ) -> Result<(), String> {
        let working_source_root = workspace_path(&self.workspace_root, &record.target.source_root)?;
        validate_target_at_source_root(&record.target, alternate_source_root, &working_source_root)
    }

    /// Capture the platform-owned ConfigDumpInfo.xml preimage while the caller
    /// holds this workspace's lifecycle lock. The file remains deliberately
    /// outside target manifests and dirty-state derivation.
    pub fn capture_platform_cdfi_preimage(
        &self,
        guard: &LifecycleLockGuard,
        requested: &[SourceTargetRecord],
    ) -> Result<PlatformCdfiPreimage, String> {
        self.validate_lifecycle_guard(guard)?;
        let scope = validate_requested_scope(self, requested, None)?;
        validate_requested_records(self, requested)?;
        let path = platform_cdfi_path(&scope.source_root)?;
        let original_bytes = read_optional_working_path(self, &path)?;
        let original = original_bytes
            .as_deref()
            .map_or(FileFingerprint::Deleted, FileFingerprint::present);
        let configuration_path =
            join_source_relative_path(&scope.source_root, Path::new("Configuration.xml"))?;
        let configuration_bytes = read_optional_working_path(self, &configuration_path)?
            .ok_or_else(|| "platform Configuration.xml seed is missing".to_string())?;
        let configuration_original = FileFingerprint::present(&configuration_bytes);
        let guarded_owners = capture_platform_guarded_owners(self, requested)?;
        Ok(PlatformCdfiPreimage {
            source_set: scope.source_set,
            source_root: scope.source_root,
            path,
            original,
            original_bytes,
            configuration_path,
            configuration_original,
            configuration_bytes,
            guarded_owners,
        })
    }

    pub fn validate_shadow_platform_guards(
        &self,
        preimage: &PlatformCdfiPreimage,
        alternate_source_root: &Path,
    ) -> Result<(), String> {
        validate_shadow_platform_guards(preimage, alternate_source_root)
    }

    /// Failure-atomic publication of selected target manifests from a shadow
    /// platform source root, including the platform-generated CDFI auxiliary
    /// file. CDFI is verified and journaled but never enters target manifests.
    pub fn publish_from_source_root(
        &self,
        guard: &LifecycleLockGuard,
        requested: &[SourceTargetRecord],
        desired: &BTreeMap<TargetId, SourceManifest>,
        alternate_source_root: &Path,
        cdfi_preimage: &PlatformCdfiPreimage,
    ) -> Result<PublicationOutcome, PublicationError> {
        self.publish_from_source_root_transaction(
            PublicationRequest {
                guard,
                requested,
                desired,
                alternate_source_root,
                cdfi_preimage,
            },
            PublicationFailureHandling::Rollback,
            |_| Ok(()),
        )
    }

    /// Recover prepared publication journals and remove committed or
    /// pre-journal crash orphans.
    /// Callers hold the workspace lifecycle lock while invoking this method.
    pub fn recover_pending_publications(&self) -> Result<PublicationRecoveryReport, String> {
        recover_publications(self)
    }

    /// A dry-run never performs recovery. Refuse a preview when apply would
    /// first need to mutate publication journals or working source through
    /// recovery, rather than reporting a misleading executable plan.
    pub fn audit_pending_publication_recovery(&self) -> Result<(), String> {
        self.validate_existing_storage()?;
        let root = self.transaction_root.join(PUBLICATIONS_DIR_NAME);
        let metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "failed to inspect publication recovery root {}: {error}",
                    root.display()
                ))
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "publication recovery root {} must be a non-symlink directory",
                root.display()
            ));
        }
        let mut entries = fs::read_dir(&root)
            .map_err(|error| format!("failed to scan publication recovery root: {error}"))?;
        if entries
            .next()
            .transpose()
            .map_err(|error| {
                format!(
                    "failed to inspect publication recovery root {}: {error}",
                    root.display()
                )
            })?
            .is_some()
        {
            return Err(
                "source-sync preview cannot prove apply behavior while publication recovery is pending"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn publish_from_source_root_transaction<AfterPublish>(
        &self,
        request: PublicationRequest<'_>,
        failure_handling: PublicationFailureHandling,
        after_publish: AfterPublish,
    ) -> Result<PublicationOutcome, PublicationError>
    where
        AfterPublish: FnMut(usize) -> Result<(), String>,
    {
        self.publish_from_source_root_transaction_with_journal_sync(
            request,
            failure_handling,
            after_publish,
            |_phase: PublicationPhase, path: &Path| sync_directory(path),
        )
    }

    fn publish_from_source_root_transaction_with_journal_sync<AfterPublish, JournalSync>(
        &self,
        request: PublicationRequest<'_>,
        failure_handling: PublicationFailureHandling,
        mut after_publish: AfterPublish,
        mut sync_journal_directory: JournalSync,
    ) -> Result<PublicationOutcome, PublicationError>
    where
        AfterPublish: FnMut(usize) -> Result<(), String>,
        JournalSync: FnMut(PublicationPhase, &Path) -> Result<(), String>,
    {
        let PublicationRequest {
            guard,
            requested,
            desired,
            alternate_source_root,
            cdfi_preimage,
        } = request;
        self.validate_lifecycle_guard(guard)?;
        self.recover_pending_publications()?;
        let mut plan = prepare_publication_plan(
            self,
            requested,
            desired,
            alternate_source_root,
            cdfi_preimage,
        )?;
        let prepared_phase = plan.journal.phase;
        if let Err(error) =
            write_publication_journal_with_sync(&plan.transaction_dir, &plan.journal, |path| {
                sync_journal_directory(prepared_phase, path)
            })
        {
            let _ = remove_generated_tree(&plan.transaction_dir);
            return Err(PublicationError::before_write(error));
        }

        if let Err(error) =
            verify_publication_cas(self, requested, desired, cdfi_preimage, &plan.files)
        {
            cleanup_prepared_publication(self, &plan)?;
            return Err(PublicationError::before_write(error));
        }

        let execution = execute_publication(self, &mut plan, cdfi_preimage, &mut after_publish);
        if let Err(error) = execution {
            let affected_paths = publication_paths(&plan);
            return match failure_handling {
                PublicationFailureHandling::Rollback => {
                    match rollback_publication(self, &plan.transaction_dir, &plan.journal) {
                        Ok(()) => Err(PublicationError::rolled_back(error, affected_paths)),
                        Err(rollback_error) => Err(PublicationError::recovery_required(
                            format!(
                                "{error}; publication rollback also failed: {rollback_error}; recovery journal retained at {}",
                                plan.transaction_dir.display()
                            ),
                            affected_paths,
                        )),
                    }
                }
                #[cfg(test)]
                PublicationFailureHandling::LeavePrepared => {
                    Err(PublicationError::recovery_required(
                        format!(
                            "{error}; prepared recovery journal retained at {}",
                            plan.transaction_dir.display()
                        ),
                        affected_paths,
                    ))
                }
            };
        }

        plan.journal.phase = PublicationPhase::Committed;
        let committed_phase = plan.journal.phase;
        let journal_outcome = match write_publication_journal_with_sync(
            &plan.transaction_dir,
            &plan.journal,
            |path| sync_journal_directory(committed_phase, path),
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                let affected_paths = publication_paths(&plan);
                let rollback = rollback_publication(self, &plan.transaction_dir, &plan.journal);
                return match rollback {
                    Ok(()) => Err(PublicationError::rolled_back(error, affected_paths)),
                    Err(rollback_error) => Err(PublicationError::recovery_required(
                        format!("{error}; publication rollback also failed: {rollback_error}"),
                        affected_paths,
                    )),
                };
            }
        };
        let published_paths = publication_paths(&plan);
        // A directory-synced Committed journal is the normal commit point. If
        // that directory sync failed, successful cleanup plus parent sync can
        // still establish a durable outcome. If both durability routes fail,
        // the source has changed but synchronization state must remain dirty:
        // after power loss the last durable journal may still be Prepared.
        let commit_warning = journal_outcome.commit_warning;
        let cleanup = cleanup_committed_publication(self, &plan.transaction_dir, &plan.journal);
        if let (Some(commit_error), Err(cleanup_error)) = (&commit_warning, &cleanup) {
            return Err(PublicationError::recovery_required(
                format!(
                    "{commit_error}; publication cleanup also failed: {cleanup_error}; synchronization state was not advanced"
                ),
                published_paths,
            ));
        }
        let cleanup_warning = cleanup.err().map(|error| {
            format!("publication committed but cleanup was deferred to recovery: {error}")
        });
        Ok(PublicationOutcome {
            published_paths,
            cleanup_warning: combine_publication_warnings(commit_warning, cleanup_warning),
        })
    }

    fn validate_lifecycle_guard(&self, guard: &LifecycleLockGuard) -> Result<(), String> {
        if guard.workspace_id == self.workspace_id {
            Ok(())
        } else {
            Err("lifecycle lock belongs to a different workspace".to_string())
        }
    }

    /// Persist a clean preimage before invoking a source writer.
    pub fn ensure_baseline(
        &self,
        target: &SourceTarget,
        pre: &SourceManifest,
    ) -> Result<BaselineReceipt, String> {
        target.validate()?;
        let mut state = self.load_state()?;
        let previous_generation = state.generation;
        let created = !state.targets.contains_key(&target.id);
        let mut changed = created;
        if let Some(record) = state.targets.get_mut(&target.id) {
            ensure_same_target(&record.target, target)?;
            let current = normalize_manifest(
                pre.clone(),
                Some(&record.current),
                Some(&record.synchronized),
            );
            if current != record.current {
                record.current = current;
                changed = true;
            }
        } else {
            let baseline = normalize_manifest(pre.clone(), None, None);
            state.targets.insert(
                target.id.clone(),
                SourceTargetRecord {
                    target: target.clone(),
                    current: baseline.clone(),
                    synchronized: SynchronizedManifest::known(&baseline),
                },
            );
        }
        if changed {
            state.generation = next_generation(previous_generation)?;
            self.persist_state_if_generation(&state, previous_generation)?;
        }
        Ok(BaselineReceipt {
            target_id: target.id.clone(),
            previous_generation,
            generation: state.generation,
            created,
        })
    }

    /// Remove a new clean baseline after a proven no-op. Any CAS mismatch keeps it.
    pub fn discard_clean_baseline(
        &self,
        receipt: &BaselineReceipt,
        pre: &SourceManifest,
    ) -> Result<bool, String> {
        if !receipt.created {
            return Ok(false);
        }
        let mut state = self.load_state()?;
        if state.generation != receipt.generation {
            return Ok(false);
        }
        let Some(record) = state.targets.get(&receipt.target_id).cloned() else {
            return Ok(false);
        };
        let expected = normalize_manifest(
            pre.clone(),
            Some(&record.current),
            Some(&record.synchronized),
        );
        let actual = normalize_manifest(
            self.capture_manifest(&record.target)?,
            Some(&record.current),
            Some(&record.synchronized),
        );
        if record.current != expected
            || actual != expected
            || !record.synchronized.matches_current(&expected)
        {
            return Ok(false);
        }
        state.targets.remove(&receipt.target_id);
        let expected_generation = state.generation;
        state.generation = receipt.previous_generation;
        self.persist_state_if_generation(&state, expected_generation)?;
        Ok(true)
    }

    pub fn record_mutation(
        &self,
        target: &SourceTarget,
        pre: &SourceManifest,
        post: &SourceManifest,
    ) -> Result<MutationRecordResult, String> {
        target.validate()?;
        if pre == post {
            return Err(format!("refusing to record no-op `{}`", target.id.as_str()));
        }
        let mut state = self.load_state()?;
        let record = if let Some(record) = state.targets.get_mut(&target.id) {
            ensure_same_target(&record.target, target)?;
            record.current = normalize_manifest(
                post.clone(),
                Some(&record.current),
                Some(&record.synchronized),
            );
            record.clone()
        } else {
            let pre = normalize_manifest(pre.clone(), None, None);
            let post = normalize_manifest(post.clone(), Some(&pre), None);
            let record = SourceTargetRecord {
                target: target.clone(),
                current: post,
                synchronized: SynchronizedManifest::known(&pre),
            };
            state.targets.insert(target.id.clone(), record.clone());
            record
        };
        let expected = state.generation;
        state.generation = next_generation(expected)?;
        self.persist_state_if_generation(&state, expected)?;
        Ok(MutationRecordResult {
            generation: state.generation,
            target: record,
        })
    }

    pub fn current_manifest(&self, id: &TargetId) -> Result<SourceManifest, String> {
        let state = self.load_state()?;
        let record = state
            .targets
            .get(id)
            .ok_or_else(|| format!("source-sync target `{}` is not registered", id.as_str()))?;
        Ok(normalize_manifest(
            self.capture_manifest(&record.target)?,
            Some(&record.current),
            Some(&record.synchronized),
        ))
    }

    #[cfg(test)]
    pub fn reconcile_current(&self, id: &TargetId) -> Result<SourceTargetRecord, String> {
        let mut state = self.load_state()?;
        let old = state
            .targets
            .get(id)
            .cloned()
            .ok_or_else(|| format!("source-sync target `{}` is not registered", id.as_str()))?;
        let current = normalize_manifest(
            self.capture_manifest(&old.target)?,
            Some(&old.current),
            Some(&old.synchronized),
        );
        if current != old.current {
            let record = state.targets.get_mut(id).ok_or_else(|| {
                format!(
                    "source-sync target `{}` disappeared during reconciliation",
                    id.as_str()
                )
            })?;
            record.current = current;
            let expected = state.generation;
            state.generation = next_generation(expected)?;
            self.persist_state_if_generation(&state, expected)?;
        }
        state
            .targets
            .get(id)
            .cloned()
            .ok_or_else(|| format!("source-sync target `{}` disappeared", id.as_str()))
    }

    pub fn reconcile_all(&self) -> Result<DirtyTargetSnapshot, String> {
        let mut state = self.load_state()?;
        let mut updates = Vec::new();
        for (id, record) in &state.targets {
            let current = normalize_manifest(
                self.capture_manifest(&record.target)?,
                Some(&record.current),
                Some(&record.synchronized),
            );
            if current != record.current {
                updates.push((id.clone(), current));
            }
        }
        if !updates.is_empty() {
            for (id, current) in updates {
                let record = state.targets.get_mut(&id).ok_or_else(|| {
                    format!(
                        "source-sync target `{}` disappeared during batch reconciliation",
                        id.as_str()
                    )
                })?;
                record.current = current;
            }
            let expected = state.generation;
            state.generation = next_generation(expected)?;
            self.persist_state_if_generation(&state, expected)?;
        }
        Ok(DirtyTargetSnapshot {
            generation: state.generation,
            targets: state
                .targets
                .values()
                .filter(|record| record.is_dirty())
                .cloned()
                .collect(),
        })
    }

    pub fn mark_synchronized(
        &self,
        expected_generation: u64,
        expected_manifests: &BTreeMap<TargetId, SourceManifest>,
    ) -> Result<SynchronizationCasResult, String> {
        let mut state = self.load_state()?;
        if state.generation != expected_generation {
            return Ok(SynchronizationCasResult {
                generation: state.generation,
                processed: Vec::new(),
                conflicted: expected_manifests
                    .iter()
                    .map(|(id, expected)| SynchronizationConflict {
                        target_id: id.clone(),
                        reason: format!(
                            "staleGeneration: expected {expected_generation}, current {}",
                            state.generation
                        ),
                        expected: Some(expected.clone()),
                        current: state.targets.get(id).map(|record| record.current.clone()),
                    })
                    .collect(),
            });
        }
        let mut processed = Vec::new();
        let mut conflicted = Vec::new();
        let mut observed = Vec::new();
        for (id, expected) in expected_manifests {
            let Some(record) = state.targets.get(id) else {
                conflicted.push(SynchronizationConflict {
                    target_id: id.clone(),
                    reason: "missingTarget".to_string(),
                    expected: Some(expected.clone()),
                    current: None,
                });
                continue;
            };
            let actual = normalize_manifest(
                self.capture_manifest(&record.target)?,
                Some(&record.current),
                Some(&record.synchronized),
            );
            let expected = normalize_manifest(
                expected.clone(),
                Some(&record.current),
                Some(&record.synchronized),
            );
            if actual != expected || record.current != expected {
                if actual != record.current {
                    observed.push((id.clone(), actual.clone()));
                }
                conflicted.push(SynchronizationConflict {
                    target_id: id.clone(),
                    reason: "manifestChanged".to_string(),
                    expected: Some(expected),
                    current: Some(actual),
                });
            } else {
                processed.push(id.clone());
            }
        }
        let state_changed = !processed.is_empty() || !observed.is_empty();
        for (id, current) in observed {
            let record = state.targets.get_mut(&id).ok_or_else(|| {
                format!(
                    "source-sync target `{}` disappeared while recording observed CAS state",
                    id.as_str()
                )
            })?;
            record.current = current;
        }
        for id in &processed {
            let record = state.targets.get_mut(id).ok_or_else(|| {
                format!(
                    "source-sync target `{}` disappeared while committing synchronization",
                    id.as_str()
                )
            })?;
            record.synchronized = SynchronizedManifest::known(&record.current);
        }
        if state_changed {
            state.generation = next_generation(state.generation)?;
            self.persist_state_if_generation(&state, expected_generation)?;
        }
        Ok(SynchronizationCasResult {
            generation: state.generation,
            processed,
            conflicted,
        })
    }

    #[cfg(test)]
    pub fn mark_synchronized_target(
        &self,
        generation: u64,
        id: TargetId,
        manifest: SourceManifest,
    ) -> Result<SynchronizationCasResult, String> {
        self.mark_synchronized(generation, &BTreeMap::from([(id, manifest)]))
    }

    fn empty_state(&self) -> SourceSyncState {
        SourceSyncState::empty(&self.workspace_id, &self.workspace_root_text)
    }

    fn persist_state_if_generation(
        &self,
        state: &SourceSyncState,
        expected: u64,
    ) -> Result<(), String> {
        let current = self.load_state()?;
        if current.generation != expected {
            return Err(format!(
                "source-sync generation changed: expected {expected}, current {}",
                current.generation
            ));
        }
        self.write_state_atomically(state)
    }

    fn write_state_atomically(&self, state: &SourceSyncState) -> Result<(), String> {
        self.write_state_atomically_with(state, |_| Ok(()))
    }

    fn write_state_atomically_with<F>(
        &self,
        state: &SourceSyncState,
        before_rename: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&Path) -> Result<(), String>,
    {
        state.validate()?;
        if state.workspace_id != self.workspace_id
            || state.workspace_root != self.workspace_root_text
        {
            return Err("refusing to persist state for a foreign workspace".to_string());
        }
        self.prepare_storage()?;
        let mut bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("failed to serialize source-sync state: {error}"))?;
        bytes.push(b'\n');
        let temp_path = self.transaction_root.join(format!(
            ".state.{}.{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let result = (|| -> Result<(), String> {
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut temp = options
                .open(&temp_path)
                .map_err(|error| format!("failed to create staging state: {error}"))?;
            temp.write_all(&bytes)
                .map_err(|error| format!("failed to write staging state: {error}"))?;
            temp.flush()
                .map_err(|error| format!("failed to flush staging state: {error}"))?;
            temp.sync_all()
                .map_err(|error| format!("failed to sync staging state: {error}"))?;
            before_rename(&temp_path)?;
            reject_symlink(&self.state_path, MissingPath::Allowed)?;
            replace_path_atomically(&temp_path, &self.state_path)?;
            sync_directory(&self.transaction_root)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp_path);
        }
        result
    }

    fn prepare_storage(&self) -> Result<(), String> {
        let build_root = self.workspace_root.join(".build");
        let source_sync_root = self.authority_root.join("source-sync");
        for (path, private) in [
            (&build_root, false),
            (&self.authority_root, false),
            (&source_sync_root, true),
            (&self.transaction_root, true),
        ] {
            ensure_storage_directory(path, private)?;
        }
        let authority = self
            .authority_root
            .canonicalize()
            .map_err(|error| format!("failed to canonicalize source-sync authority: {error}"))?;
        if !authority.starts_with(&self.workspace_root) {
            return Err(
                "source-sync authority escapes its workspace through a symlink".to_string(),
            );
        }
        let root = self
            .transaction_root
            .canonicalize()
            .map_err(|error| format!("failed to canonicalize source-sync storage: {error}"))?;
        if !root.starts_with(&authority) {
            return Err(
                "source-sync storage escapes its authority root through a symlink".to_string(),
            );
        }
        Ok(())
    }

    fn validate_existing_storage(&self) -> Result<(), String> {
        for path in [
            self.workspace_root.join(".build"),
            self.authority_root.clone(),
            self.authority_root.join("source-sync"),
            self.transaction_root.clone(),
        ] {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(format!(
                        "source-sync storage component {} must be a non-symlink directory",
                        path.display()
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "failed to inspect source-sync storage component {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        Ok(())
    }
}

fn ensure_storage_directory(path: &Path, private: bool) -> Result<(), String> {
    let mut created = false;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "source-sync storage component {} must be a non-symlink directory",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            if private {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            let result = builder.create(path);
            match result {
                Ok(()) => created = true,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!(
                        "failed to create source-sync storage component {}: {error}",
                        path.display()
                    ));
                }
            }
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "failed to verify source-sync storage component {}: {error}",
                    path.display()
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "source-sync storage component {} must be a non-symlink directory",
                    path.display()
                ));
            }
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect source-sync storage component {}: {error}",
                path.display()
            ));
        }
    }
    if private {
        set_directory_private(path)?;
    }
    if created {
        sync_parent_directory(path)?;
    }
    Ok(())
}

fn prepare_publication_plan(
    repository: &SourceSyncRepository,
    requested: &[SourceTargetRecord],
    desired: &BTreeMap<TargetId, SourceManifest>,
    alternate_source_root: &Path,
    cdfi_preimage: &PlatformCdfiPreimage,
) -> Result<PublicationPlan, String> {
    prepare_publication_plan_with_backup_sync(
        repository,
        requested,
        desired,
        alternate_source_root,
        cdfi_preimage,
        sync_directory,
    )
}

fn prepare_publication_plan_with_backup_sync<BackupSync>(
    repository: &SourceSyncRepository,
    requested: &[SourceTargetRecord],
    desired: &BTreeMap<TargetId, SourceManifest>,
    alternate_source_root: &Path,
    cdfi_preimage: &PlatformCdfiPreimage,
    mut sync_backup_directory: BackupSync,
) -> Result<PublicationPlan, String>
where
    BackupSync: FnMut(&Path) -> Result<(), String>,
{
    repository.prepare_storage()?;
    let scope = validate_requested_scope(repository, requested, Some(alternate_source_root))?;
    let requested_by_id = requested_records_by_id(requested)?;
    for target_id in desired.keys() {
        if !requested_by_id.contains_key(target_id) {
            return Err(format!(
                "publication requested unknown target `{}`",
                target_id.as_str()
            ));
        }
    }
    validate_requested_records(repository, requested)?;
    validate_cdfi_preimage(&scope, cdfi_preimage)?;
    verify_platform_guard_cas(
        repository,
        cdfi_preimage,
        &cdfi_preimage.original,
        &BTreeSet::new(),
    )?;
    let current_cdfi = fingerprint_working_path(repository, &cdfi_preimage.path)?;
    if current_cdfi != cdfi_preimage.original {
        return Err("publication ConfigDumpInfo.xml CAS changed before staging".to_string());
    }
    let mut merged = BTreeMap::<RelativeSourcePath, MergedPublicationFile>::new();

    for (target_id, desired_manifest) in desired {
        let record = requested_by_id[target_id];
        validate_publication_manifest(&record.target, desired_manifest)?;
        let paths = record
            .current
            .files
            .keys()
            .chain(desired_manifest.files.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for path in paths {
            if !target_owns_path(&record.target, &path) {
                return Err(format!(
                    "publication target `{}` does not own `{}`",
                    target_id.as_str(),
                    path.as_str()
                ));
            }
            let original = record
                .current
                .files
                .get(&path)
                .cloned()
                .unwrap_or(FileFingerprint::Deleted);
            let desired_fingerprint = desired_manifest
                .files
                .get(&path)
                .cloned()
                .unwrap_or(FileFingerprint::Deleted);
            if original == desired_fingerprint {
                continue;
            }
            let desired_bytes = read_verified_shadow_bytes(
                &record.target,
                &path,
                &desired_fingerprint,
                &scope.alternate_source_root,
            )?;
            let candidate = MergedPublicationFile {
                role: PublicationFileRole::TargetOwned,
                original,
                desired: desired_fingerprint,
                desired_bytes,
            };
            if let Some(existing) = merged.get(&path) {
                if existing != &candidate {
                    return Err(format!(
                        "overlapping publication targets disagree about `{}`",
                        path.as_str()
                    ));
                }
            } else {
                merged.insert(path, candidate);
            }
        }
    }

    let cdfi_shadow = scope.alternate_source_root.join("ConfigDumpInfo.xml");
    reject_symlink(&cdfi_shadow, MissingPath::Rejected).map_err(|error| {
        format!("shadow ConfigDumpInfo.xml is required for forced publication: {error}")
    })?;
    let cdfi_bytes = read_stable_regular_file(&cdfi_shadow, &scope.alternate_source_root)
        .map_err(|error| format!("failed to read shadow ConfigDumpInfo.xml: {error}"))?;
    let cdfi_desired = FileFingerprint::present(&cdfi_bytes);
    if cdfi_preimage.original != cdfi_desired {
        merged.insert(
            cdfi_preimage.path.clone(),
            MergedPublicationFile {
                role: PublicationFileRole::PlatformConfigDumpInfo,
                original: cdfi_preimage.original.clone(),
                desired: cdfi_desired,
                desired_bytes: Some(cdfi_bytes),
            },
        );
    }

    for record in requested {
        verify_record_manifest_cas(repository, record, desired.get(&record.target.id))?;
    }
    // Verify the complete batch, including unchanged requested targets and the
    // global CDFI preimage, before creating any publication artifacts.
    for (path, file) in &merged {
        let actual = fingerprint_working_path(repository, path)?;
        if actual != file.original {
            return Err(format!(
                "publication final file CAS failed for `{}` before staging",
                path.as_str()
            ));
        }
    }

    let publications_root = repository.transaction_root.join(PUBLICATIONS_DIR_NAME);
    create_private_directory_all(&publications_root)?;
    validate_directory_below(&publications_root, &repository.transaction_root)?;
    sync_directory(&repository.transaction_root)?;
    let transaction_id = uuid::Uuid::new_v4().to_string();
    let transaction_dir = publications_root.join(format!("publication-{transaction_id}"));
    create_private_directory(&transaction_dir)?;
    sync_directory(&publications_root)?;
    let backup_dir = transaction_dir.join("backups");
    create_private_directory(&backup_dir)?;
    // Persist the backup-directory entry itself. Syncing `backup_dir` below
    // makes its files durable, while syncing the parent makes recovery able to
    // reach that directory after a crash.
    sync_directory(&transaction_dir)?;

    let prepared = (|| -> Result<PublicationPlan, String> {
        let mut files = Vec::with_capacity(merged.len());
        let mut created_directories = BTreeSet::new();
        for (index, (path, merged_file)) in merged.into_iter().enumerate() {
            let destination = destination_path(repository, &path)?;
            validate_destination_components(repository, &destination)?;
            let actual = fingerprint_working_path(repository, &path)?;
            if actual != merged_file.original {
                return Err(format!(
                    "publication CAS failed for `{}` before staging",
                    path.as_str()
                ));
            }
            let backup_file = match &merged_file.original {
                FileFingerprint::Present { .. } => {
                    let bytes = read_stable_regular_file(&destination, &repository.workspace_root)?;
                    if FileFingerprint::present(&bytes) != merged_file.original {
                        return Err(format!(
                            "publication backup hash changed for `{}`",
                            path.as_str()
                        ));
                    }
                    let name = format!("{index:08}.bin");
                    write_private_new_file(&backup_dir.join(&name), &bytes)?;
                    Some(name)
                }
                FileFingerprint::Deleted => None,
            };
            if matches!(merged_file.desired, FileFingerprint::Present { .. }) {
                collect_missing_parent_directories(
                    repository,
                    &destination,
                    &mut created_directories,
                )?;
            }
            let stage_path = matches!(merged_file.desired, FileFingerprint::Present { .. })
                .then(|| {
                    let parent = destination
                        .parent()
                        .ok_or_else(|| "publication destination has no parent".to_string())?;
                    let stage =
                        parent.join(format!(".unica-publish-{transaction_id}-{index:08}.tmp"));
                    RelativeSourcePath::new(relative_to_workspace(
                        &repository.workspace_root,
                        &stage,
                    )?)
                })
                .transpose()?;
            let original_mode = file_mode(&destination)?;
            files.push(PublicationFilePlan {
                journal: PublicationJournalFile {
                    role: merged_file.role,
                    path,
                    original: merged_file.original,
                    desired: merged_file.desired,
                    backup_file,
                    stage_path,
                    original_mode,
                },
                desired_bytes: merged_file.desired_bytes,
            });
        }
        // Every backup file is individually synced by write_private_new_file.
        // Persist the backup directory entries before a Prepared journal can
        // make source writes recoverable from those backups.
        sync_backup_directory(&backup_dir)?;
        let journal = PublicationJournal {
            schema_version: PUBLICATION_SCHEMA_VERSION,
            workspace_id: repository.workspace_id.clone(),
            workspace_root: repository.workspace_root_text.clone(),
            transaction_id,
            phase: PublicationPhase::Prepared,
            files: files.iter().map(|file| file.journal.clone()).collect(),
            created_directories: created_directories.into_iter().collect(),
        };
        Ok(PublicationPlan {
            transaction_dir: transaction_dir.clone(),
            journal,
            files,
        })
    })();
    if prepared.is_err() {
        let _ = remove_generated_tree(&transaction_dir);
    }
    prepared
}

#[derive(Debug, PartialEq, Eq)]
struct MergedPublicationFile {
    role: PublicationFileRole,
    original: FileFingerprint,
    desired: FileFingerprint,
    desired_bytes: Option<Vec<u8>>,
}

fn requested_records_by_id(
    requested: &[SourceTargetRecord],
) -> Result<BTreeMap<TargetId, &SourceTargetRecord>, String> {
    let mut records = BTreeMap::new();
    for record in requested {
        if let Some(existing) = records.insert(record.target.id.clone(), record) {
            if existing != record {
                return Err(format!(
                    "duplicate publication target `{}` has inconsistent records",
                    record.target.id.as_str()
                ));
            }
        }
    }
    Ok(records)
}

fn validate_requested_scope(
    repository: &SourceSyncRepository,
    requested: &[SourceTargetRecord],
    alternate_source_root: Option<&Path>,
) -> Result<PublicationScope, String> {
    let first = requested
        .first()
        .ok_or_else(|| "forced publication requires at least one requested target".to_string())?;
    first.target.validate()?;
    let source_set = first
        .target
        .source_set
        .clone()
        .ok_or_else(|| "forced publication requires a configured sourceSet".to_string())?;
    let source_root = first.target.source_root.clone();
    for record in requested {
        record.target.validate()?;
        if record.target.source_set.as_ref() != Some(&source_set)
            || record.target.source_root != source_root
        {
            return Err(format!(
                "forced publication batch mixes sourceSet/sourceRoot at target `{}`",
                record.target.id.as_str()
            ));
        }
        validate_target_topology(repository, record)?;
    }

    let working_source_root = workspace_path(&repository.workspace_root, &source_root)?;
    reject_symlink(&working_source_root, MissingPath::Rejected)?;
    let working_source_root = working_source_root.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize working source root {}: {error}",
            working_source_root.display()
        )
    })?;
    let alternate_source_root = match alternate_source_root {
        Some(alternate) => {
            reject_symlink(alternate, MissingPath::Rejected)?;
            let alternate = alternate.canonicalize().map_err(|error| {
                format!(
                    "failed to canonicalize publication shadow root {}: {error}",
                    alternate.display()
                )
            })?;
            if !alternate.is_dir() {
                return Err(format!(
                    "publication shadow root is not a directory: {}",
                    alternate.display()
                ));
            }
            if alternate == working_source_root
                || alternate.starts_with(&working_source_root)
                || working_source_root.starts_with(&alternate)
            {
                return Err(
                    "publication shadow root must be isolated from working source root".to_string(),
                );
            }
            alternate
        }
        None => working_source_root.clone(),
    };

    Ok(PublicationScope {
        source_set,
        source_root,
        alternate_source_root,
    })
}

fn validate_requested_records(
    repository: &SourceSyncRepository,
    requested: &[SourceTargetRecord],
) -> Result<(), String> {
    let records = requested_records_by_id(requested)?;
    let persisted = repository.load_state()?;
    for (target_id, record) in records {
        match persisted.targets.get(&target_id) {
            Some(current) if current == record => {}
            Some(_) => {
                return Err(format!(
                    "publication target `{}` is stale relative to durable source-sync state",
                    target_id.as_str()
                ))
            }
            None => {
                return Err(format!(
                    "publication target `{}` is absent from durable source-sync state",
                    target_id.as_str()
                ))
            }
        }
    }
    Ok(())
}

fn validate_cdfi_preimage(
    scope: &PublicationScope,
    preimage: &PlatformCdfiPreimage,
) -> Result<(), String> {
    let expected_path = platform_cdfi_path(&scope.source_root)?;
    let expected_configuration =
        join_source_relative_path(&scope.source_root, Path::new("Configuration.xml"))?;
    if preimage.source_set != scope.source_set
        || preimage.source_root != scope.source_root
        || preimage.path != expected_path
        || preimage.configuration_path != expected_configuration
    {
        return Err(
            "ConfigDumpInfo.xml preimage does not match publication sourceSet/sourceRoot"
                .to_string(),
        );
    }
    Ok(())
}

fn platform_cdfi_path(source_root: &RelativeSourcePath) -> Result<RelativeSourcePath, String> {
    join_source_relative_path(source_root, Path::new("ConfigDumpInfo.xml"))
}

fn capture_platform_guarded_owners(
    repository: &SourceSyncRepository,
    requested: &[SourceTargetRecord],
) -> Result<Vec<PlatformGuardedOwner>, String> {
    let metadata_owners = requested
        .iter()
        .filter(|record| record.target.target_kind == SourceTargetKind::MetadataOwner)
        .map(|record| record.target.owner_selector.as_str())
        .collect::<BTreeSet<_>>();
    let mut module_paths = BTreeMap::<String, BTreeSet<RelativeSourcePath>>::new();
    for record in requested {
        let SourceTargetScope::Module { path } = &record.target.scope else {
            continue;
        };
        if metadata_owners.contains(record.target.owner_selector.as_str()) {
            continue;
        }
        module_paths
            .entry(record.target.owner_selector.clone())
            .or_default()
            .insert(path.clone());
    }

    let first = requested
        .first()
        .ok_or_else(|| "platform guards require at least one requested target".to_string())?;
    let source_set = first
        .target
        .source_set
        .clone()
        .ok_or_else(|| "platform guards require a configured sourceSet".to_string())?;
    let mut guarded = Vec::new();
    for (owner_selector, excluded_module_paths) in module_paths {
        let (object_type, object_name) = owner_selector
            .split_once(':')
            .ok_or_else(|| format!("module owner selector `{owner_selector}` is not canonical"))?;
        let collection = super::native_operations::cf::cf_validate_child_type_dir(object_type)
            .ok_or_else(|| format!("unsupported module owner type `{object_type}`"))?;
        let descriptor_path = join_source_relative_path(
            &first.target.source_root,
            &Path::new(collection).join(format!("{object_name}.xml")),
        )?;
        let owner_directory = join_source_relative_path(
            &first.target.source_root,
            &Path::new(collection).join(object_name),
        )?;
        let target = SourceTarget {
            id: TargetId::new(format!("guard:{}:{owner_selector}", source_set.as_str()))?,
            target_kind: SourceTargetKind::MetadataOwner,
            source_set: Some(source_set.clone()),
            source_root: first.target.source_root.clone(),
            owner_selector,
            scope: SourceTargetScope::MetadataOwner {
                descriptor_path,
                owner_directory,
            },
        };
        let mut original = repository.capture_manifest(&target)?;
        original
            .files
            .retain(|path, _| !excluded_module_paths.contains(path));
        guarded.push(PlatformGuardedOwner {
            target,
            excluded_module_paths,
            original,
        });
    }
    Ok(guarded)
}

fn validate_shadow_platform_guards(
    preimage: &PlatformCdfiPreimage,
    alternate_source_root: &Path,
) -> Result<(), String> {
    let configuration = fingerprint_alternate_source_path(
        &preimage.source_root,
        &preimage.configuration_path,
        alternate_source_root,
    )?;
    if configuration != preimage.configuration_original {
        return Err(
            "shadow root Configuration.xml differs from the locked working preimage".to_string(),
        );
    }
    for owner in &preimage.guarded_owners {
        let mut shadow = capture_manifest_at_alternate_root(&owner.target, alternate_source_root)?;
        shadow
            .files
            .retain(|path, _| !owner.excluded_module_paths.contains(path));
        if shadow != owner.original {
            return Err(format!(
                "shadow owner bundle `{}` changed outside the requested module paths",
                owner.target.owner_selector
            ));
        }
    }
    Ok(())
}

fn verify_platform_guard_cas(
    repository: &SourceSyncRepository,
    preimage: &PlatformCdfiPreimage,
    expected_cdfi: &FileFingerprint,
    ignored_staging_paths: &BTreeSet<RelativeSourcePath>,
) -> Result<(), String> {
    let cdfi = fingerprint_working_path(repository, &preimage.path)?;
    if cdfi != *expected_cdfi {
        return Err("working ConfigDumpInfo.xml CAS changed during forced publication".to_string());
    }
    let configuration = fingerprint_working_path(repository, &preimage.configuration_path)?;
    if configuration != preimage.configuration_original {
        return Err("working Configuration.xml changed during forced publication".to_string());
    }
    for owner in &preimage.guarded_owners {
        let mut current = repository.capture_manifest(&owner.target)?;
        current.files.retain(|path, _| {
            !owner.excluded_module_paths.contains(path) && !ignored_staging_paths.contains(path)
        });
        if current != owner.original {
            return Err(format!(
                "working owner bundle `{}` changed during forced publication",
                owner.target.owner_selector
            ));
        }
    }
    Ok(())
}

fn fingerprint_alternate_source_path(
    source_root: &RelativeSourcePath,
    path: &RelativeSourcePath,
    alternate_source_root: &Path,
) -> Result<FileFingerprint, String> {
    let relative = relative_target_scope_path(source_root, path)?;
    let alternate = alternate_source_root.join(relative);
    match fs::symlink_metadata(&alternate) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "alternate guarded path is not a regular file: {}",
            alternate.display()
        )),
        Ok(_) => Ok(FileFingerprint::present(&read_stable_regular_file(
            &alternate,
            alternate_source_root,
        )?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileFingerprint::Deleted),
        Err(error) => Err(format!(
            "failed to inspect alternate guarded path {}: {error}",
            alternate.display()
        )),
    }
}

fn publication_paths(plan: &PublicationPlan) -> Vec<RelativeSourcePath> {
    plan.files
        .iter()
        .map(|file| file.journal.path.clone())
        .collect()
}

fn validate_target_topology(
    repository: &SourceSyncRepository,
    record: &SourceTargetRecord,
) -> Result<(), String> {
    record.target.validate()?;
    let source_set = record.target.source_set.as_ref().ok_or_else(|| {
        format!(
            "target `{}` has no configured sourceSet",
            record.target.id.as_str()
        )
    })?;
    let source_map = discover_project_source_map(&repository.workspace_root)?;
    if !source_map.source_sets_from_config {
        return Err(
            "source topology is no longer backed by an explicit non-empty `source-set` in v8project.yaml"
                .to_string(),
        );
    }
    let mut matches = source_map
        .source_sets
        .iter()
        .filter(|candidate| candidate.name == source_set.as_str());
    let selected = matches.next().ok_or_else(|| {
        format!(
            "target `{}` sourceSet `{}` is no longer configured",
            record.target.id.as_str(),
            source_set.as_str()
        )
    })?;
    if matches.next().is_some() {
        return Err(format!(
            "target `{}` sourceSet `{}` is now ambiguous",
            record.target.id.as_str(),
            source_set.as_str()
        ));
    }
    if selected.kind != SourceSetKind::Configuration
        || selected.source_format != SourceFormat::PlatformXml
    {
        return Err(format!(
            "target `{}` sourceSet `{}` is not a platform-XML configuration",
            record.target.id.as_str(),
            source_set.as_str()
        ));
    }
    let configured = normalize_lexically(&repository.workspace_root.join(&selected.path));
    reject_symlink(&configured, MissingPath::Rejected)?;
    let configured = configured.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize configured sourceSet `{}` root {}: {error}",
            source_set.as_str(),
            configured.display()
        )
    })?;
    if !configured.starts_with(&repository.workspace_root) {
        return Err(format!(
            "configured sourceSet `{}` escapes the workspace",
            source_set.as_str()
        ));
    }
    ensure_unique_source_set_root(
        &source_map.source_sets,
        &repository.workspace_root,
        &configured,
    )?;
    let persisted = workspace_path(&repository.workspace_root, &record.target.source_root)?;
    reject_symlink(&persisted, MissingPath::Rejected)?;
    let persisted = persisted.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize persisted target root {}: {error}",
            persisted.display()
        )
    })?;
    if configured != persisted {
        return Err(format!(
            "target `{}` source topology changed: persisted `{}`, configured `{}`",
            record.target.id.as_str(),
            persisted.display(),
            configured.display()
        ));
    }
    validate_persisted_target_identity(repository, record, &configured, source_set)?;
    Ok(())
}

fn validate_persisted_target_identity(
    repository: &SourceSyncRepository,
    record: &SourceTargetRecord,
    source_root: &Path,
    source_set: &SourceSetName,
) -> Result<(), String> {
    match &record.target.scope {
        SourceTargetScope::Module { path } => {
            let relative = relative_target_scope_path(&record.target.source_root, path)?;
            let observed_selector = owner_selector_from_relative_path(source_root, &relative)?;
            if record.target.owner_selector != observed_selector {
                return Err(format!(
                    "target `{}` owner selector `{}` does not match canonical `{observed_selector}`",
                    record.target.id.as_str(),
                    record.target.owner_selector
                ));
            }
            let expected_id = format!(
                "module:{}:{}",
                source_set.as_str(),
                path_for_json(&relative)
            );
            if record.target.id.as_str() != expected_id {
                return Err(format!(
                    "module target id `{}` does not match deterministic `{expected_id}`",
                    record.target.id.as_str()
                ));
            }
        }
        SourceTargetScope::MetadataOwner {
            descriptor_path,
            owner_directory,
        } => {
            let (object_type, object_name) = record
                .target
                .owner_selector
                .split_once(':')
                .filter(|(object_type, object_name)| {
                    !object_type.is_empty() && !object_name.is_empty() && !object_name.contains(':')
                })
                .ok_or_else(|| {
                    format!(
                        "metadata target `{}` has a non-canonical owner selector",
                        record.target.id.as_str()
                    )
                })?;
            let collection = super::native_operations::cf::cf_validate_child_type_dir(object_type)
                .ok_or_else(|| {
                    format!(
                        "metadata target `{}` has unsupported owner type `{object_type}`",
                        record.target.id.as_str()
                    )
                })?;
            let expected_descriptor = join_source_relative_path(
                &record.target.source_root,
                &Path::new(collection).join(format!("{object_name}.xml")),
            )?;
            let expected_owner = join_source_relative_path(
                &record.target.source_root,
                &Path::new(collection).join(object_name),
            )?;
            if descriptor_path != &expected_descriptor || owner_directory != &expected_owner {
                return Err(format!(
                    "metadata target `{}` scope does not match canonical {collection}/{object_name}",
                    record.target.id.as_str()
                ));
            }
            let expected_id = format!(
                "metadata:{}:{}",
                source_set.as_str(),
                record.target.owner_selector
            );
            if record.target.id.as_str() != expected_id {
                return Err(format!(
                    "metadata target id `{}` does not match deterministic `{expected_id}`",
                    record.target.id.as_str()
                ));
            }
            let descriptor = workspace_path(&repository.workspace_root, descriptor_path)?;
            reject_symlink(&descriptor, MissingPath::Allowed)?;
            match fs::symlink_metadata(&descriptor) {
                Ok(metadata) if metadata.is_file() => {
                    let (observed_type, observed_name) = metadata_identity(&descriptor)?;
                    if observed_type != object_type || observed_name != object_name {
                        return Err(format!(
                            "metadata target descriptor identity `{observed_type}:{observed_name}` does not match `{}`",
                            record.target.owner_selector
                        ));
                    }
                    validate_root_object_registration(source_root, object_type, object_name)?;
                }
                Ok(_) => {
                    return Err(format!(
                        "metadata target descriptor is not a regular file: {}",
                        descriptor.display()
                    ))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(format!(
                        "metadata target descriptor is missing: {}",
                        descriptor.display()
                    ))
                }
                Err(error) => {
                    return Err(format!(
                        "failed to inspect metadata target descriptor {}: {error}",
                        descriptor.display()
                    ))
                }
            }
        }
    }
    Ok(())
}

fn relative_target_scope_path(
    source_root: &RelativeSourcePath,
    path: &RelativeSourcePath,
) -> Result<PathBuf, String> {
    if source_root.as_str() == "." {
        return Ok(PathBuf::from(path.as_str()));
    }
    Path::new(path.as_str())
        .strip_prefix(source_root.as_str())
        .map(Path::to_path_buf)
        .map_err(|_| {
            format!(
                "target path `{}` is outside source root `{}`",
                path.as_str(),
                source_root.as_str()
            )
        })
}

fn validate_publication_manifest(
    target: &SourceTarget,
    manifest: &SourceManifest,
) -> Result<(), String> {
    for path in manifest.files.keys() {
        if is_cdfi_name(path.as_str()) {
            return Err(format!(
                "forced publication must never write ConfigDumpInfo.xml: {}",
                path.as_str()
            ));
        }
        if !target_owns_path(target, path) {
            return Err(format!(
                "target `{}` does not own publication path `{}`",
                target.id.as_str(),
                path.as_str()
            ));
        }
    }
    Ok(())
}

fn target_owns_path(target: &SourceTarget, path: &RelativeSourcePath) -> bool {
    if is_cdfi_name(path.as_str()) || strip_source_root(target, path).is_err() {
        return false;
    }
    match &target.scope {
        SourceTargetScope::Module { path: module } => module == path,
        SourceTargetScope::MetadataOwner {
            descriptor_path,
            owner_directory,
        } => {
            descriptor_path == path
                || Path::new(path.as_str()).starts_with(owner_directory.as_str())
                    && path != owner_directory
        }
    }
}

fn read_verified_shadow_bytes(
    target: &SourceTarget,
    path: &RelativeSourcePath,
    desired: &FileFingerprint,
    alternate_source_root: &Path,
) -> Result<Option<Vec<u8>>, String> {
    let relative = strip_source_root(target, path)?;
    let shadow_path = alternate_source_root.join(relative);
    reject_symlink(&shadow_path, MissingPath::Allowed)?;
    match desired {
        FileFingerprint::Present { .. } => {
            let bytes = read_stable_regular_file(&shadow_path, alternate_source_root)?;
            if FileFingerprint::present(&bytes) != *desired {
                return Err(format!(
                    "shadow bytes do not match desired manifest for `{}`",
                    path.as_str()
                ));
            }
            Ok(Some(bytes))
        }
        FileFingerprint::Deleted => {
            if shadow_path.exists() {
                return Err(format!(
                    "shadow path `{}` exists but desired manifest marks it deleted",
                    shadow_path.display()
                ));
            }
            Ok(None)
        }
    }
}

fn verify_record_manifest_cas(
    repository: &SourceSyncRepository,
    record: &SourceTargetRecord,
    desired: Option<&SourceManifest>,
) -> Result<(), String> {
    let mut actual = normalize_manifest(
        repository.capture_manifest(&record.target)?,
        Some(&record.current),
        Some(&record.synchronized),
    );
    let mut expected = normalize_manifest(
        record.current.clone(),
        Some(&record.current),
        Some(&record.synchronized),
    );
    if let Some(desired) = desired {
        for path in desired.files.keys() {
            actual
                .files
                .entry(path.clone())
                .or_insert(FileFingerprint::Deleted);
            expected
                .files
                .entry(path.clone())
                .or_insert(FileFingerprint::Deleted);
        }
    }
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "publication manifest CAS failed for target `{}`",
            record.target.id.as_str()
        ))
    }
}

fn verify_publication_cas(
    repository: &SourceSyncRepository,
    requested: &[SourceTargetRecord],
    desired: &BTreeMap<TargetId, SourceManifest>,
    cdfi_preimage: &PlatformCdfiPreimage,
    files: &[PublicationFilePlan],
) -> Result<(), String> {
    requested_records_by_id(requested)?;
    let persisted = repository.load_state()?;
    for record in requested {
        let target_id = &record.target.id;
        if persisted.targets.get(target_id) != Some(record) {
            return Err(format!(
                "publication target `{}` changed in durable state",
                target_id.as_str()
            ));
        }
        verify_record_manifest_cas(repository, record, desired.get(target_id))?;
    }
    let current_cdfi = fingerprint_working_path(repository, &cdfi_preimage.path)?;
    if current_cdfi != cdfi_preimage.original {
        return Err("publication final ConfigDumpInfo.xml CAS failed".to_string());
    }
    verify_platform_guard_cas(
        repository,
        cdfi_preimage,
        &cdfi_preimage.original,
        &BTreeSet::new(),
    )?;
    for file in files {
        let actual = fingerprint_working_path(repository, &file.journal.path)?;
        if actual != file.journal.original {
            return Err(format!(
                "publication final file CAS failed for `{}`",
                file.journal.path.as_str()
            ));
        }
    }
    Ok(())
}

fn execute_publication<AfterPublish>(
    repository: &SourceSyncRepository,
    plan: &mut PublicationPlan,
    cdfi_preimage: &PlatformCdfiPreimage,
    after_publish: &mut AfterPublish,
) -> Result<(), String>
where
    AfterPublish: FnMut(usize) -> Result<(), String>,
{
    let mut expected_cdfi = cdfi_preimage.original.clone();
    let ignored_staging_paths = plan
        .files
        .iter()
        .filter_map(|file| file.journal.stage_path.clone())
        .collect::<BTreeSet<_>>();
    for directory in &plan.journal.created_directories {
        let path = destination_path(repository, directory)?;
        if path.exists() {
            validate_destination_components(repository, &path)?;
        } else {
            create_source_directory(&path)?;
        }
    }

    for file in &plan.files {
        if let Some(stage_path) = &file.journal.stage_path {
            let bytes = file.desired_bytes.as_deref().ok_or_else(|| {
                format!(
                    "publication stage for `{}` has no verified shadow bytes",
                    file.journal.path.as_str()
                )
            })?;
            let stage = destination_path(repository, stage_path)?;
            write_source_stage_file(&stage, bytes, file.journal.original_mode.unwrap_or(0o644))?;
        }
    }

    for (index, file) in plan.files.iter().enumerate() {
        verify_platform_guard_cas(
            repository,
            cdfi_preimage,
            &expected_cdfi,
            &ignored_staging_paths,
        )?;
        let destination = destination_path(repository, &file.journal.path)?;
        let current = fingerprint_working_path(repository, &file.journal.path)?;
        if current != file.journal.original {
            return Err(format!(
                "publication file CAS changed immediately before `{}`",
                file.journal.path.as_str()
            ));
        }
        match &file.journal.desired {
            FileFingerprint::Present { .. } => {
                let stage_path = file.journal.stage_path.as_ref().ok_or_else(|| {
                    format!(
                        "publication stage missing for `{}`",
                        file.journal.path.as_str()
                    )
                })?;
                let stage = destination_path(repository, stage_path)?;
                reject_symlink(&destination, MissingPath::Allowed)?;
                replace_path_atomically(&stage, &destination).map_err(|error| {
                    format!(
                        "failed to atomically publish `{}`: {error}",
                        file.journal.path.as_str()
                    )
                })?;
            }
            FileFingerprint::Deleted => {
                reject_symlink(&destination, MissingPath::Rejected)?;
                fs::remove_file(&destination).map_err(|error| {
                    format!(
                        "failed to publish deletion `{}`: {error}",
                        file.journal.path.as_str()
                    )
                })?;
            }
        }
        sync_parent_directory(&destination)?;
        let published = fingerprint_working_path(repository, &file.journal.path)?;
        if published != file.journal.desired {
            return Err(format!(
                "published bytes failed verification for `{}`",
                file.journal.path.as_str()
            ));
        }
        if file.journal.role == PublicationFileRole::PlatformConfigDumpInfo {
            expected_cdfi = file.journal.desired.clone();
        }
        after_publish(index)?;
        verify_platform_guard_cas(
            repository,
            cdfi_preimage,
            &expected_cdfi,
            &ignored_staging_paths,
        )?;
    }

    verify_platform_guard_cas(
        repository,
        cdfi_preimage,
        &expected_cdfi,
        &ignored_staging_paths,
    )?;
    Ok(())
}

fn write_publication_journal_with_sync<SyncDirectory>(
    transaction_dir: &Path,
    journal: &PublicationJournal,
    mut sync_journal_directory: SyncDirectory,
) -> Result<PublicationJournalWriteOutcome, String>
where
    SyncDirectory: FnMut(&Path) -> Result<(), String>,
{
    validate_publication_journal(journal)?;
    let mut bytes = serde_json::to_vec_pretty(journal)
        .map_err(|error| format!("failed to serialize publication journal: {error}"))?;
    bytes.push(b'\n');
    let journal_path = transaction_dir.join(PUBLICATION_JOURNAL_NAME);
    let temp_path = transaction_dir.join(format!(".journal.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        write_private_new_file(&temp_path, &bytes)?;
        reject_symlink(&journal_path, MissingPath::Allowed)?;
        replace_path_atomically(&temp_path, &journal_path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(temp_path);
        return Err(error);
    }

    match sync_journal_directory(transaction_dir) {
        Ok(()) => Ok(PublicationJournalWriteOutcome::default()),
        Err(error) => match journal.phase {
            PublicationPhase::Prepared => Err(error),
            PublicationPhase::Committed => Ok(PublicationJournalWriteOutcome {
                // Atomic replacement is the publication commit point. Once a
                // Committed journal is visible, no error path may attempt a
                // rollback; restart recovery may only finish cleanup.
                commit_warning: Some(format!(
                    "publication committed but journal directory sync was deferred: {error}"
                )),
            }),
        },
    }
}

fn combine_publication_warnings(
    journal_warning: Option<String>,
    cleanup_warning: Option<String>,
) -> Option<String> {
    match (journal_warning, cleanup_warning) {
        (Some(journal), Some(cleanup)) => Some(format!("{journal}; {cleanup}")),
        (Some(journal), None) => Some(journal),
        (None, Some(cleanup)) => Some(cleanup),
        (None, None) => None,
    }
}

fn read_publication_journal(transaction_dir: &Path) -> Result<PublicationJournal, String> {
    let path = transaction_dir.join(PUBLICATION_JOURNAL_NAME);
    reject_symlink(&path, MissingPath::Rejected)?;
    let bytes =
        fs::read(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let journal = serde_json::from_slice::<PublicationJournal>(&bytes).map_err(|error| {
        format!(
            "publication journal {} is malformed; recovery is blocked: {error}",
            path.display()
        )
    })?;
    validate_publication_journal(&journal)?;
    Ok(journal)
}

fn validate_publication_journal(journal: &PublicationJournal) -> Result<(), String> {
    if journal.schema_version != PUBLICATION_SCHEMA_VERSION {
        return Err(format!(
            "unsupported publication journal schemaVersion {}",
            journal.schema_version
        ));
    }
    validate_publication_transaction_id(&journal.transaction_id)?;
    let mut paths = BTreeSet::new();
    let mut allowed_created_directories = BTreeSet::new();
    for (index, file) in journal.files.iter().enumerate() {
        match (file.role, is_cdfi_name(file.path.as_str())) {
            (PublicationFileRole::TargetOwned, false)
            | (PublicationFileRole::PlatformConfigDumpInfo, true) => {}
            (PublicationFileRole::TargetOwned, true) => {
                return Err(
                    "publication journal classifies ConfigDumpInfo.xml as target-owned".to_string(),
                )
            }
            (PublicationFileRole::PlatformConfigDumpInfo, false) => {
                return Err(format!(
                    "publication journal classifies non-CDFI path `{}` as platform CDFI",
                    file.path.as_str()
                ))
            }
        }
        if file.role == PublicationFileRole::PlatformConfigDumpInfo
            && matches!(file.desired, FileFingerprint::Deleted)
        {
            return Err(
                "publication journal must never delete platform ConfigDumpInfo.xml".to_string(),
            );
        }
        if !paths.insert(file.path.clone()) {
            return Err(format!(
                "publication journal contains duplicate `{}`",
                file.path.as_str()
            ));
        }
        match (&file.desired, &file.stage_path) {
            (FileFingerprint::Present { .. }, Some(stage_path)) => {
                let expected =
                    expected_publication_stage_path(&file.path, &journal.transaction_id, index)?;
                if stage_path != &expected {
                    return Err(format!(
                        "publication journal has unexpected stage `{}` for `{}`; expected `{}`",
                        stage_path.as_str(),
                        file.path.as_str(),
                        expected.as_str()
                    ));
                }
                allowed_created_directories.extend(destination_parent_paths(&file.path)?);
            }
            (FileFingerprint::Deleted, None) => {}
            (FileFingerprint::Present { .. }, None) | (FileFingerprint::Deleted, Some(_)) => {
                return Err(format!(
                    "publication journal has inconsistent stage for `{}`",
                    file.path.as_str()
                ))
            }
        }
        match (&file.original, &file.backup_file) {
            (FileFingerprint::Present { .. }, Some(name)) if is_safe_backup_name(name) => {}
            (FileFingerprint::Deleted, None) => {}
            (FileFingerprint::Present { .. }, Some(_))
            | (FileFingerprint::Present { .. }, None)
            | (FileFingerprint::Deleted, Some(_)) => {
                return Err(format!(
                    "publication journal has invalid backup for `{}`",
                    file.path.as_str()
                ))
            }
        }
    }
    let mut created_directories = BTreeSet::new();
    for directory in &journal.created_directories {
        if !created_directories.insert(directory.clone()) {
            return Err(format!(
                "publication journal contains duplicate created directory `{}`",
                directory.as_str()
            ));
        }
        if !allowed_created_directories.contains(directory) {
            return Err(format!(
                "publication journal created directory `{}` is not an ancestor of a published destination",
                directory.as_str()
            ));
        }
    }
    Ok(())
}

fn validate_publication_transaction_id(transaction_id: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(transaction_id).map_err(|error| {
        format!("publication journal transactionId `{transaction_id}` is not a UUID: {error}")
    })?;
    if parsed.to_string() == transaction_id {
        Ok(())
    } else {
        Err(format!(
            "publication journal transactionId `{transaction_id}` is not canonical"
        ))
    }
}

fn publication_transaction_id_from_directory_name(name: &str) -> Result<String, String> {
    let transaction_id = name
        .strip_prefix("publication-")
        .ok_or_else(|| format!("unexpected entry in publication recovery root: {name}"))?;
    validate_publication_transaction_id(transaction_id)
        .map_err(|error| format!("invalid publication recovery directory `{name}`: {error}"))?;
    Ok(transaction_id.to_string())
}

fn expected_publication_stage_path(
    destination: &RelativeSourcePath,
    transaction_id: &str,
    index: usize,
) -> Result<RelativeSourcePath, String> {
    if destination.as_str() == "." {
        return Err("publication journal destination cannot be the workspace root".to_string());
    }
    let stage_name = format!(".unica-publish-{transaction_id}-{index:08}.tmp");
    let stage = destination
        .as_str()
        .rsplit_once('/')
        .map_or(stage_name.clone(), |(parent, _file_name)| {
            format!("{parent}/{stage_name}")
        });
    RelativeSourcePath::new(stage)
}

fn destination_parent_paths(
    destination: &RelativeSourcePath,
) -> Result<Vec<RelativeSourcePath>, String> {
    if destination.as_str() == "." {
        return Err("publication journal destination cannot be the workspace root".to_string());
    }
    let components = destination.as_str().split('/').collect::<Vec<_>>();
    (1..components.len())
        .map(|end| RelativeSourcePath::new(components[..end].join("/")))
        .collect()
}

fn recover_publications(
    repository: &SourceSyncRepository,
) -> Result<PublicationRecoveryReport, String> {
    let publications_root = repository.transaction_root.join(PUBLICATIONS_DIR_NAME);
    if !publications_root.exists() {
        return Ok(PublicationRecoveryReport::default());
    }
    validate_directory_below(&publications_root, &repository.transaction_root)?;
    let mut entries = fs::read_dir(&publications_root)
        .map_err(|error| format!("failed to scan publication recovery root: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate publication recovery root: {error}"))?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut report = PublicationRecoveryReport::default();
    for entry in entries {
        let name = entry.file_name().into_string().map_err(|name| {
            format!(
                "publication recovery entry is not valid UTF-8: {}",
                name.to_string_lossy()
            )
        })?;
        let directory_transaction_id = publication_transaction_id_from_directory_name(&name)?;
        let transaction_dir = entry.path();
        reject_symlink(&transaction_dir, MissingPath::Rejected)?;
        if !transaction_dir.is_dir() {
            return Err(format!(
                "publication recovery entry is not a directory: {}",
                transaction_dir.display()
            ));
        }
        let journal_path = transaction_dir.join(PUBLICATION_JOURNAL_NAME);
        match fs::symlink_metadata(&journal_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Source files are never touched before the first Prepared
                // journal has been atomically installed. A canonical
                // transaction directory without that journal is therefore a
                // safely removable pre-write crash orphan.
                remove_generated_tree(&transaction_dir)?;
                report.cleaned_unprepared.push(directory_transaction_id);
                continue;
            }
            Ok(_metadata) => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect publication journal {}: {error}",
                    journal_path.display()
                ));
            }
        }
        let journal = read_publication_journal(&transaction_dir)?;
        if journal.transaction_id != directory_transaction_id {
            return Err(format!(
                "publication journal transactionId `{}` does not match directory `{name}`",
                journal.transaction_id
            ));
        }
        if journal.workspace_id != repository.workspace_id
            || journal.workspace_root != repository.workspace_root_text
        {
            return Err(format!(
                "publication journal {} belongs to a foreign workspace",
                transaction_dir.display()
            ));
        }
        match journal.phase {
            PublicationPhase::Prepared => {
                rollback_publication(repository, &transaction_dir, &journal)?;
                report.rolled_back.push(journal.transaction_id);
            }
            PublicationPhase::Committed => {
                cleanup_committed_publication(repository, &transaction_dir, &journal)?;
                report.cleaned_committed.push(journal.transaction_id);
            }
        }
    }
    Ok(report)
}

fn rollback_publication(
    repository: &SourceSyncRepository,
    transaction_dir: &Path,
    journal: &PublicationJournal,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for file in journal.files.iter().rev() {
        if let Some(stage) = &file.stage_path {
            if let Ok(stage_path) = destination_path(repository, stage) {
                if let Err(error) = remove_regular_file_if_present(&stage_path) {
                    errors.push(error);
                }
            }
        }
        if let Err(error) = rollback_publication_file(repository, transaction_dir, file) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        remove_created_directories(repository, &journal.created_directories)?;
        remove_generated_tree(transaction_dir)
    } else {
        Err(errors.join("; "))
    }
}

fn rollback_publication_file(
    repository: &SourceSyncRepository,
    transaction_dir: &Path,
    file: &PublicationJournalFile,
) -> Result<(), String> {
    let destination = destination_path(repository, &file.path)?;
    let current = fingerprint_working_path(repository, &file.path)?;
    if current == file.original {
        return Ok(());
    }
    if current != file.desired {
        return Err(format!(
            "refusing to overwrite concurrent edit while recovering `{}`",
            file.path.as_str()
        ));
    }
    match &file.original {
        FileFingerprint::Present { .. } => {
            let name = file
                .backup_file
                .as_deref()
                .ok_or_else(|| format!("recovery backup missing for `{}`", file.path.as_str()))?;
            if !is_safe_backup_name(name) {
                return Err(format!("unsafe recovery backup name `{name}`"));
            }
            let backup = transaction_dir.join("backups").join(name);
            let bytes = read_stable_regular_file(&backup, transaction_dir)?;
            if FileFingerprint::present(&bytes) != file.original {
                return Err(format!(
                    "recovery backup hash mismatch for `{}`",
                    file.path.as_str()
                ));
            }
            if let Some(parent) = destination.parent() {
                create_source_directory_all(parent, &repository.workspace_root)?;
            }
            atomic_restore_bytes(&destination, &bytes, file.original_mode.unwrap_or(0o644))?;
        }
        FileFingerprint::Deleted => {
            remove_regular_file_if_present(&destination)?;
            sync_parent_directory(&destination)?;
        }
    }
    let restored = fingerprint_working_path(repository, &file.path)?;
    if restored != file.original {
        return Err(format!(
            "rollback verification failed for `{}`",
            file.path.as_str()
        ));
    }
    Ok(())
}

fn cleanup_prepared_publication(
    _repository: &SourceSyncRepository,
    plan: &PublicationPlan,
) -> Result<(), String> {
    remove_generated_tree(&plan.transaction_dir)
}

fn cleanup_committed_publication(
    repository: &SourceSyncRepository,
    transaction_dir: &Path,
    journal: &PublicationJournal,
) -> Result<(), String> {
    for file in &journal.files {
        if let Some(stage) = &file.stage_path {
            remove_regular_file_if_present(&destination_path(repository, stage)?)?;
        }
    }
    remove_generated_tree(transaction_dir)
}

fn destination_path(
    repository: &SourceSyncRepository,
    path: &RelativeSourcePath,
) -> Result<PathBuf, String> {
    workspace_path(&repository.workspace_root, path)
}

fn fingerprint_working_path(
    repository: &SourceSyncRepository,
    path: &RelativeSourcePath,
) -> Result<FileFingerprint, String> {
    Ok(read_optional_working_path(repository, path)?
        .as_deref()
        .map_or(FileFingerprint::Deleted, FileFingerprint::present))
}

fn read_optional_working_path(
    repository: &SourceSyncRepository,
    path: &RelativeSourcePath,
) -> Result<Option<Vec<u8>>, String> {
    let destination = destination_path(repository, path)?;
    validate_destination_components(repository, &destination)?;
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "platform preimage path must be a regular non-symlink file: {}",
            destination.display()
        )),
        Ok(_) => Ok(Some(read_stable_regular_file(
            &destination,
            &repository.workspace_root,
        )?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to inspect platform preimage {}: {error}",
            destination.display()
        )),
    }
}

fn validate_destination_components(
    repository: &SourceSyncRepository,
    destination: &Path,
) -> Result<(), String> {
    let relative = destination
        .strip_prefix(&repository.workspace_root)
        .map_err(|_| format!("destination escapes workspace: {}", destination.display()))?;
    let mut current = repository.workspace_root.clone();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(format!(
                "unsafe destination path: {}",
                destination.display()
            ));
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "publication refuses symlink component {}",
                    current.display()
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!(
                    "failed to inspect destination {}: {error}",
                    current.display()
                ))
            }
        }
    }
    Ok(())
}

fn collect_missing_parent_directories(
    repository: &SourceSyncRepository,
    destination: &Path,
    output: &mut BTreeSet<RelativeSourcePath>,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "publication destination has no parent".to_string())?;
    validate_destination_components(repository, parent)?;
    let mut missing = Vec::new();
    let mut current = parent;
    while !current.exists() {
        if current == repository.workspace_root {
            break;
        }
        missing.push(current.to_path_buf());
        current = current
            .parent()
            .ok_or_else(|| "publication parent escaped workspace".to_string())?;
    }
    for directory in missing.into_iter().rev() {
        output.insert(RelativeSourcePath::new(relative_to_workspace(
            &repository.workspace_root,
            &directory,
        )?)?);
    }
    Ok(())
}

fn remove_created_directories(
    repository: &SourceSyncRepository,
    directories: &[RelativeSourcePath],
) -> Result<(), String> {
    for directory in directories.iter().rev() {
        let path = destination_path(repository, directory)?;
        reject_symlink(&path, MissingPath::Allowed)?;
        match fs::remove_dir(&path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    sync_directory(parent)?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => {
                return Err(format!(
                    "failed to remove publication-created directory {}: {error}",
                    path.display()
                ))
            }
        }
    }
    Ok(())
}

fn create_private_directory_all(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| {
        format!(
            "failed to create private directory {}: {error}",
            path.display()
        )
    })?;
    reject_symlink(path, MissingPath::Rejected)?;
    set_directory_private(path)
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path).map_err(|error| {
            format!(
                "failed to create private directory {}: {error}",
                path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path).map_err(|error| {
            format!(
                "failed to create private directory {}: {error}",
                path.display()
            )
        })?;
    }
    reject_symlink(path, MissingPath::Rejected)?;
    set_directory_private(path)
}

fn set_directory_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("failed to secure directory {}: {error}", path.display()))?;
    }
    Ok(())
}

fn write_private_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("failed to create private file {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    file.flush()
        .map_err(|error| format!("failed to flush {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", path.display()))
}

fn create_source_directory(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|error| {
        format!(
            "failed to create source directory {}: {error}",
            path.display()
        )
    })?;
    reject_symlink(path, MissingPath::Rejected)?;
    sync_parent_directory(path)
}

fn create_source_directory_all(path: &Path, workspace_root: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(workspace_root)
        .map_err(|_| format!("source directory escapes workspace: {}", path.display()))?;
    let mut current = workspace_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(format!("unsafe source directory: {}", path.display()));
        };
        current.push(part);
        reject_symlink(&current, MissingPath::Allowed)?;
        if !current.exists() {
            create_source_directory(&current)?;
        }
    }
    Ok(())
}

fn write_source_stage_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode & 0o777);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("failed to create source stage {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write source stage {}: {error}", path.display()))?;
    file.flush()
        .map_err(|error| format!("failed to flush source stage {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync source stage {}: {error}", path.display()))
}

#[cfg(not(windows))]
fn replace_path_atomically(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target)
        .map_err(|error| format!("failed to atomically replace {}: {error}", target.display()))
}

#[cfg(windows)]
fn replace_path_atomically(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers for the
    // duration of the call, and the flags request one atomic replacement.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(format!(
            "failed to atomically replace {}: {}",
            target.display(),
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

fn atomic_restore_bytes(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "restore destination has no parent".to_string())?;
    let stage = parent.join(format!(".unica-restore-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        write_source_stage_file(&stage, bytes, mode)?;
        reject_symlink(path, MissingPath::Allowed)?;
        replace_path_atomically(&stage, path)
            .map_err(|error| format!("failed to restore {}: {error}", path.display()))?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(stage);
    }
    result
}

fn file_mode(path: &Path) -> Result<Option<u32>, String> {
    if !path.exists() {
        return Ok(None);
    }
    reject_symlink(path, MissingPath::Rejected)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| Some(metadata.permissions().mode()))
            .map_err(|error| format!("failed to read permissions for {}: {error}", path.display()))
    }
    #[cfg(not(unix))]
    {
        Ok(None)
    }
}

fn sync_parent_directory(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    sync_directory(parent)
}

fn remove_regular_file_if_present(path: &Path) -> Result<(), String> {
    reject_symlink(path, MissingPath::Allowed)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove {}: {error}", path.display())),
    }
}

fn remove_generated_tree(path: &Path) -> Result<(), String> {
    reject_symlink(path, MissingPath::Allowed)?;
    match fs::remove_dir_all(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove {}: {error}", path.display())),
    }
}

fn validate_directory_below(path: &Path, root: &Path) -> Result<(), String> {
    reject_symlink(path, MissingPath::Rejected)?;
    let path = path
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))?;
    let root = root
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize {}: {error}", root.display()))?;
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(format!(
            "directory escapes transaction root: {}",
            path.display()
        ))
    }
}

fn is_safe_backup_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_digit() || byte == b'.' || byte == b'b' || byte == b'i' || byte == b'n'
        })
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
}

pub fn resolve_mutation_target(
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<SourceTarget, String> {
    match tool_name {
        "unica.code.patch" => resolve_code_patch_target(args, context),
        "unica.meta.edit" => resolve_meta_edit_target(args, context),
        other => Err(format!(
            "source-sync resolution is not defined for `{other}`"
        )),
    }
}

fn resolve_code_patch_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<SourceTarget, String> {
    let module = super::native_operations::code::resolve_module_target(args, context)?;
    let source_map = discover_project_source_map(&context.workspace_root)?;
    require_authoritative_source_map(&source_map)?;
    let (source_root, source_set) =
        resolve_requested_source_root(args, context, &source_map.source_sets)?;
    let relative_module = module.strip_prefix(&source_root).map_err(|_| {
        format!(
            "resolved module {} is outside source root {}",
            module.display(),
            source_root.display()
        )
    })?;
    let workspace_module =
        RelativeSourcePath::new(relative_to_workspace(&context.workspace_root, &module)?)?;
    let source_root_path = RelativeSourcePath::new(relative_to_workspace(
        &context.workspace_root,
        &source_root,
    )?)?;
    let owner_selector = owner_selector_from_relative_path(&source_root, relative_module)?;
    let identity_root = source_set
        .as_ref()
        .map(SourceSetName::as_str)
        .unwrap_or(source_root_path.as_str());
    Ok(SourceTarget {
        id: TargetId::new(format!(
            "module:{identity_root}:{}",
            path_for_json(relative_module)
        ))?,
        target_kind: SourceTargetKind::Module,
        source_set,
        source_root: source_root_path,
        owner_selector,
        scope: SourceTargetScope::Module {
            path: workspace_module,
        },
    })
}

fn resolve_meta_edit_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<SourceTarget, String> {
    let raw = first_string(args, &["objectPath", "ObjectPath", "path", "Path"])
        .ok_or_else(|| "`ObjectPath` must be a non-empty string".to_string())?;
    let candidate = super::native_operations::meta::resolve_meta_edit_object_path(
        Path::new(raw),
        &context.cwd,
    )?;
    reject_symlink(&candidate, MissingPath::Rejected)?;
    let descriptor = candidate.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize metadata object {}: {error}",
            candidate.display()
        )
    })?;
    let workspace_root = context
        .workspace_root
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize workspace root: {error}"))?;
    if !descriptor.starts_with(&workspace_root) || !descriptor.is_file() {
        return Err(format!(
            "metadata object must be a regular file inside the workspace: {}",
            candidate.display()
        ));
    }
    if is_cdfi_path(&descriptor) {
        return Err("ConfigDumpInfo.xml cannot be a source-sync target".to_string());
    }
    let source_map = discover_project_source_map(&workspace_root)?;
    require_authoritative_source_map(&source_map)?;
    let selected =
        select_containing_source_set(&descriptor, &workspace_root, &source_map.source_sets)?;
    let (source_root, selected) = selected.ok_or_else(|| {
        format!(
            "metadata object {} must belong to an explicit platform_xml configuration source-set in v8project.yaml",
            descriptor.display()
        )
    })?;
    ensure_unique_source_set_name(&source_map.source_sets, &selected.name)?;
    let source_set = Some(SourceSetName::new(selected.name.clone())?);
    let (object_type, object_name) = metadata_identity(&descriptor)?;
    validate_metadata_descriptor_layout(&source_root, &descriptor, &object_type, &object_name)?;
    validate_root_object_registration(&source_root, &object_type, &object_name)?;
    let stem = descriptor
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("metadata path has no UTF-8 stem: {}", descriptor.display()))?;
    if stem != object_name {
        return Err(format!(
            "metadata descriptor name `{stem}` does not match XML object name `{object_name}`"
        ));
    }
    let owner_selector = format!("{object_type}:{object_name}");
    let owner_dir = descriptor
        .parent()
        .ok_or_else(|| "metadata descriptor has no parent directory".to_string())?
        .join(stem);
    reject_symlink(&owner_dir, MissingPath::Allowed)?;
    let source_root_path =
        RelativeSourcePath::new(relative_to_workspace(&workspace_root, &source_root)?)?;
    let identity_root = source_set
        .as_ref()
        .map(SourceSetName::as_str)
        .unwrap_or(source_root_path.as_str());
    Ok(SourceTarget {
        id: TargetId::new(format!("metadata:{identity_root}:{owner_selector}"))?,
        target_kind: SourceTargetKind::MetadataOwner,
        source_set,
        source_root: source_root_path,
        owner_selector,
        scope: SourceTargetScope::MetadataOwner {
            descriptor_path: RelativeSourcePath::new(relative_to_workspace(
                &workspace_root,
                &descriptor,
            )?)?,
            owner_directory: RelativeSourcePath::new(relative_to_workspace(
                &workspace_root,
                &owner_dir,
            )?)?,
        },
    })
}

fn validate_metadata_descriptor_layout(
    source_root: &Path,
    descriptor: &Path,
    object_type: &str,
    object_name: &str,
) -> Result<(), String> {
    let collection = super::native_operations::cf::cf_validate_child_type_dir(object_type)
        .ok_or_else(|| {
            format!("metadata type `{object_type}` is not a Designer-addressable child object")
        })?;
    let relative = descriptor.strip_prefix(source_root).map_err(|_| {
        format!(
            "metadata descriptor {} is outside source root {}",
            descriptor.display(),
            source_root.display()
        )
    })?;
    let expected = Path::new(collection).join(format!("{object_name}.xml"));
    if relative != expected {
        return Err(format!(
            "metadata descriptor `{}` does not match canonical `{}` layout for {object_type}:{object_name}",
            relative.display(),
            expected.display()
        ));
    }
    Ok(())
}

fn resolve_requested_source_root(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    source_sets: &[ProjectSourceSet],
) -> Result<(PathBuf, Option<SourceSetName>), String> {
    match (
        first_identity_string(args, &["sourceSet"]),
        first_string(args, &["sourceDir"]),
    ) {
        (Some(name), None) => {
            let matches = source_sets
                .iter()
                .filter(|candidate| candidate.name == name)
                .collect::<Vec<_>>();
            let selected = match matches.as_slice() {
                [] => return Err(format!("source-set `{name}` was not found")),
                [selected] => *selected,
                _ => return Err(format!("source-set `{name}` is ambiguous")),
            };
            validate_platform_source_set(selected)?;
            let root = context
                .workspace_root
                .join(&selected.path)
                .canonicalize()
                .map_err(|error| format!("failed to canonicalize source-set `{name}`: {error}"))?;
            ensure_contained(&root, &context.workspace_root)?;
            ensure_unique_source_set_root(source_sets, &context.workspace_root, &root)?;
            Ok((root, Some(SourceSetName::new(name)?)))
        }
        (None, Some(raw)) => {
            let candidate = if Path::new(raw).is_absolute() {
                PathBuf::from(raw)
            } else {
                context.cwd.join(raw)
            };
            reject_symlink(&candidate, MissingPath::Rejected)?;
            let root = candidate.canonicalize().map_err(|error| {
                format!(
                    "failed to canonicalize sourceDir {}: {error}",
                    candidate.display()
                )
            })?;
            ensure_contained(&root, &context.workspace_root)?;
            let matches = source_sets
                .iter()
                .filter(|source_set| {
                    context
                        .workspace_root
                        .join(&source_set.path)
                        .canonicalize()
                        .is_ok_and(|candidate| candidate == root)
                })
                .collect::<Vec<_>>();
            let source_set = match matches.as_slice() {
                [] => None,
                [selected] => {
                    ensure_unique_source_set_name(source_sets, &selected.name)?;
                    validate_platform_source_set(selected)?;
                    Some(SourceSetName::new(selected.name.clone())?)
                }
                _ => {
                    return Err(format!(
                        "sourceDir {} maps to multiple source-sets",
                        root.display()
                    ))
                }
            };
            Ok((root, source_set))
        }
        (Some(_), Some(_)) | (None, None) => {
            Err("exactly one of `sourceSet` or `sourceDir` must be provided".to_string())
        }
    }
}

fn require_authoritative_source_map(
    source_map: &crate::domain::project_sources::ProjectSourceMap,
) -> Result<(), String> {
    if source_map.source_sets_from_config {
        Ok(())
    } else {
        Err(
            "source-sync mutations require an explicit non-empty `source-set` in v8project.yaml"
                .to_string(),
        )
    }
}

fn ensure_unique_source_set_name(
    source_sets: &[ProjectSourceSet],
    name: &str,
) -> Result<(), String> {
    if source_sets
        .iter()
        .filter(|candidate| candidate.name == name)
        .count()
        == 1
    {
        Ok(())
    } else {
        Err(format!(
            "source-set name `{name}` is ambiguous in v8project.yaml"
        ))
    }
}

fn ensure_unique_source_set_root(
    source_sets: &[ProjectSourceSet],
    workspace_root: &Path,
    selected_root: &Path,
) -> Result<(), String> {
    let owners = source_sets
        .iter()
        .filter(|candidate| {
            workspace_root
                .join(&candidate.path)
                .canonicalize()
                .is_ok_and(|root| root == selected_root)
        })
        .count();
    if owners == 1 {
        Ok(())
    } else {
        Err(format!(
            "source root {} is assigned to {owners} source-set entries in v8project.yaml",
            selected_root.display()
        ))
    }
}

fn select_containing_source_set<'a>(
    target: &Path,
    workspace_root: &Path,
    source_sets: &'a [ProjectSourceSet],
) -> Result<Option<(PathBuf, &'a ProjectSourceSet)>, String> {
    let mut matches = Vec::new();
    for source_set in source_sets {
        if source_set.kind != SourceSetKind::Configuration
            || source_set.source_format != SourceFormat::PlatformXml
        {
            continue;
        }
        let candidate = workspace_root.join(&source_set.path);
        reject_symlink(&candidate, MissingPath::Rejected)?;
        let root = candidate.canonicalize().map_err(|error| {
            format!(
                "failed to canonicalize source-set `{}`: {error}",
                source_set.name
            )
        })?;
        ensure_contained(&root, workspace_root)?;
        if target.starts_with(&root) {
            matches.push((root, source_set));
        }
    }
    matches.sort_by_key(|(root, set)| (root.components().count(), &set.name));
    let Some(deepest) = matches.last() else {
        return Ok(None);
    };
    let same_depth = matches
        .iter()
        .filter(|(root, _)| root.components().count() == deepest.0.components().count())
        .collect::<Vec<_>>();
    match same_depth.as_slice() {
        [only] => Ok(Some((only.0.clone(), only.1))),
        _ => Err(format!(
            "metadata object {} maps to multiple source-sets",
            target.display()
        )),
    }
}

fn validate_platform_source_set(source_set: &ProjectSourceSet) -> Result<(), String> {
    if source_set.kind == SourceSetKind::Configuration
        && source_set.source_format == SourceFormat::PlatformXml
    {
        Ok(())
    } else {
        Err(format!(
            "source-set `{}` must be a platform_xml configuration",
            source_set.name
        ))
    }
}

fn metadata_identity(path: &Path) -> Result<(String, String), String> {
    let bytes = read_stable_regular_file(path, path.parent().unwrap_or(path))?;
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
        .map_err(|error| format!("metadata {} is not UTF-8: {error}", path.display()))?;
    let document = Document::parse(text)
        .map_err(|error| format!("metadata {} is invalid XML: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "metadata {} root must be MetaDataObject",
            path.display()
        ));
    }
    let object = exactly_one_element_child(root, None).map_err(|error| {
        format!(
            "metadata {} must contain exactly one object element: {error}",
            path.display()
        )
    })?;
    let properties = exactly_one_element_child(object, Some("Properties")).map_err(|error| {
        format!(
            "metadata {} must contain exactly one direct Properties element: {error}",
            path.display()
        )
    })?;
    let name_node = exactly_one_element_child(properties, Some("Name")).map_err(|error| {
        format!(
            "metadata {} must contain exactly one direct Name element: {error}",
            path.display()
        )
    })?;
    let name = name_node
        .text()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("metadata {} has an empty Name", path.display()))?;
    Ok((object.tag_name().name().to_string(), name.to_string()))
}

fn exactly_one_element_child<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    expected_name: Option<&str>,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let mut children = parent.children().filter(|node| {
        node.is_element() && expected_name.is_none_or(|expected| node.tag_name().name() == expected)
    });
    let child = children
        .next()
        .ok_or_else(|| "matching element is missing".to_string())?;
    if children.next().is_some() {
        return Err("multiple matching elements were found".to_string());
    }
    Ok(child)
}

fn validate_root_object_registration(
    source_root: &Path,
    object_type: &str,
    object_name: &str,
) -> Result<(), String> {
    let expected_collection = super::native_operations::cf::cf_validate_child_type_dir(object_type)
        .ok_or_else(|| {
            format!("metadata type `{object_type}` is not a supported Configuration child")
        })?;
    if metadata_type_for_collection(expected_collection) != Some(object_type) {
        return Err(format!(
            "metadata type `{object_type}` has an inconsistent collection mapping"
        ));
    }
    let path = source_root.join("Configuration.xml");
    let bytes = read_stable_regular_file(&path, source_root)?;
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
        .map_err(|error| format!("root metadata {} is not UTF-8: {error}", path.display()))?;
    let document = Document::parse(text)
        .map_err(|error| format!("root metadata {} is invalid XML: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "root metadata {} must be MetaDataObject",
            path.display()
        ));
    }
    let configuration =
        exactly_one_element_child(root, Some("Configuration")).map_err(|error| {
            format!(
                "root metadata {} must contain exactly one direct Configuration: {error}",
                path.display()
            )
        })?;
    let child_objects =
        exactly_one_element_child(configuration, Some("ChildObjects")).map_err(|error| {
            format!(
                "root metadata {} must contain exactly one direct ChildObjects: {error}",
                path.display()
            )
        })?;
    let count = child_objects
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == object_type
                && node.text().is_some_and(|text| text.trim() == object_name)
        })
        .count();
    match count {
        1 => Ok(()),
        0 => Err(format!(
            "metadata object `{object_type}:{object_name}` is not registered in root Configuration.xml"
        )),
        _ => Err(format!(
            "metadata object `{object_type}:{object_name}` is registered more than once in root Configuration.xml"
        )),
    }
}

fn owner_selector_from_relative_path(source_root: &Path, path: &Path) -> Result<String, String> {
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(part) => part
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "module path must be valid UTF-8".to_string()),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => Err(format!(
                "module path contains an unsafe component: {}",
                path.display()
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if matches!(components.first().map(String::as_str), Some("Ext")) {
        return Err(format!(
            "root configuration module `{}` is not proven addressable by an object-scoped Designer selector",
            path.display()
        ));
    }
    if components.len() < 4 {
        return Err(format!(
            "module `{}` has no Designer-addressable metadata owner",
            path.display()
        ));
    }
    let collection = &components[0];
    let name = &components[1];
    let object_type = metadata_type_for_collection(collection).ok_or_else(|| {
        format!(
            "module `{}` belongs to unsupported metadata collection `{collection}`",
            path.display()
        )
    })?;
    let descriptor = source_root.join(collection).join(format!("{name}.xml"));
    reject_symlink(&descriptor, MissingPath::Rejected)?;
    if !descriptor.is_file() {
        return Err(format!(
            "module owner descriptor is missing: {}",
            descriptor.display()
        ));
    }
    let (observed_type, observed_name) = metadata_identity(&descriptor)?;
    if observed_type != object_type || observed_name != *name {
        return Err(format!(
            "module owner descriptor identity `{observed_type}:{observed_name}` does not match `{object_type}:{name}`"
        ));
    }
    validate_addressable_module_role(object_type, &components[2..]).map_err(|error| {
        format!(
            "module `{}` is not safely addressable: {error}",
            path.display()
        )
    })?;
    validate_root_object_registration(source_root, object_type, name)?;
    Ok(format!("{object_type}:{name}"))
}

fn validate_target_at_source_root(
    target: &SourceTarget,
    alternate_source_root: &Path,
    working_source_root: &Path,
) -> Result<(), String> {
    target.validate()?;
    reject_symlink(alternate_source_root, MissingPath::Rejected)?;
    let alternate_source_root = alternate_source_root.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize alternate source root {}: {error}",
            alternate_source_root.display()
        )
    })?;
    match &target.scope {
        SourceTargetScope::Module { path } => {
            let relative = relative_target_scope_path(&target.source_root, path)?;
            let observed = owner_selector_from_relative_path(&alternate_source_root, &relative)?;
            if observed != target.owner_selector {
                return Err(format!(
                    "shadow module owner `{observed}` does not match persisted `{}`",
                    target.owner_selector
                ));
            }
            let (object_type, object_name) = target
                .owner_selector
                .split_once(':')
                .ok_or_else(|| "persisted module owner selector is not canonical".to_string())?;
            let collection = super::native_operations::cf::cf_validate_child_type_dir(object_type)
                .ok_or_else(|| format!("unsupported module owner type `{object_type}`"))?;
            let relative_descriptor = Path::new(collection).join(format!("{object_name}.xml"));
            let shadow_descriptor = alternate_source_root.join(&relative_descriptor);
            let working_descriptor = working_source_root.join(&relative_descriptor);
            let shadow_bytes =
                read_stable_regular_file(&shadow_descriptor, &alternate_source_root)?;
            let working_bytes = read_stable_regular_file(&working_descriptor, working_source_root)?;
            if shadow_bytes != working_bytes {
                return Err(format!(
                    "shadow module owner descriptor {} differs from working source; module-only force cannot publish a matching ConfigDumpInfo.xml safely",
                    relative_descriptor.display()
                ));
            }
        }
        SourceTargetScope::MetadataOwner {
            descriptor_path, ..
        } => {
            let relative = relative_target_scope_path(&target.source_root, descriptor_path)?;
            let descriptor = alternate_source_root.join(&relative);
            reject_symlink(&descriptor, MissingPath::Rejected)?;
            let metadata = fs::symlink_metadata(&descriptor).map_err(|error| {
                format!(
                    "shadow metadata descriptor {} is required and must not be deleted: {error}",
                    descriptor.display()
                )
            })?;
            if !metadata.is_file() {
                return Err(format!(
                    "shadow metadata descriptor is not a regular file: {}",
                    descriptor.display()
                ));
            }
            let (object_type, object_name) = metadata_identity(&descriptor)?;
            let observed = format!("{object_type}:{object_name}");
            if observed != target.owner_selector {
                return Err(format!(
                    "shadow metadata descriptor identity `{observed}` does not match persisted `{}`",
                    target.owner_selector
                ));
            }
            validate_metadata_descriptor_layout(
                &alternate_source_root,
                &descriptor,
                &object_type,
                &object_name,
            )?;
            validate_root_object_registration(&alternate_source_root, &object_type, &object_name)?;
        }
    }
    Ok(())
}

fn validate_addressable_module_role(object_type: &str, suffix: &[String]) -> Result<(), String> {
    let suffix = suffix.iter().map(String::as_str).collect::<Vec<_>>();
    let allowed = match suffix.as_slice() {
        ["Ext", "Module.bsl"] => matches!(
            object_type,
            "CommonModule" | "HTTPService" | "WebService" | "IntegrationService"
        ),
        ["Ext", "CommandModule.bsl"] => object_type == "CommonCommand",
        ["Ext", "Form", "Module.bsl"] => object_type == "CommonForm",
        ["Forms", form_name, "Ext", "Form", "Module.bsl"] => {
            !form_name.is_empty() && supports_nested_form_or_command_module(object_type)
        }
        ["Commands", command_name, "Ext", "CommandModule.bsl"] => {
            !command_name.is_empty() && supports_nested_form_or_command_module(object_type)
        }
        ["Ext", "ObjectModule.bsl"] => matches!(
            object_type,
            "Catalog"
                | "Document"
                | "ExchangePlan"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "Report"
                | "DataProcessor"
        ),
        ["Ext", "ManagerModule.bsl"] => matches!(
            object_type,
            "Catalog"
                | "Document"
                | "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "ExchangePlan"
                | "Enum"
                | "Report"
                | "DataProcessor"
                | "Constant"
                | "DocumentJournal"
                | "FilterCriterion"
                | "SettingsStorage"
        ),
        ["Ext", "RecordSetModule.bsl"] => matches!(
            object_type,
            "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
        ),
        ["Ext", "ValueManagerModule.bsl"] => object_type == "Constant",
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "module role `{}` is not whitelisted for metadata type `{object_type}`",
            suffix.join("/")
        ))
    }
}

/// These metadata owners are dumped safely through their canonical
/// `TYPE:NAME` selector. The exact nested suffix prevents arbitrary `.bsl`
/// files under an owner from becoming patchable while retaining the platform's
/// standard managed-form and command module layouts.
fn supports_nested_form_or_command_module(object_type: &str) -> bool {
    matches!(
        object_type,
        "Catalog"
            | "Document"
            | "ExchangePlan"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "BusinessProcess"
            | "Task"
            | "Report"
            | "DataProcessor"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "DocumentJournal"
            | "Enum"
            | "Constant"
            | "Sequence"
            | "DocumentNumerator"
    )
}

fn metadata_type_for_collection(collection: &str) -> Option<&'static str> {
    super::native_operations::cf::cf_validate_child_object_types()
        .iter()
        .copied()
        .find(|object_type| {
            super::native_operations::cf::cf_validate_child_type_dir(object_type)
                == Some(collection)
        })
}

fn capture_manifest_at(
    workspace_root: &Path,
    target: &SourceTarget,
) -> Result<SourceManifest, String> {
    let source_root = workspace_path(workspace_root, &target.source_root)?;
    reject_symlink(&source_root, MissingPath::Rejected)?;
    let source_root = source_root.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize source root {}: {error}",
            source_root.display()
        )
    })?;
    if !source_root.starts_with(workspace_root) {
        return Err("target source root escapes workspace".to_string());
    }
    let mut manifest = SourceManifest::default();
    match &target.scope {
        SourceTargetScope::Module { path } => {
            capture_declared_file(workspace_root, &source_root, path, &mut manifest)?;
        }
        SourceTargetScope::MetadataOwner {
            descriptor_path,
            owner_directory,
        } => {
            capture_declared_file(workspace_root, &source_root, descriptor_path, &mut manifest)?;
            let directory = workspace_path(workspace_root, owner_directory)?;
            reject_symlink(&directory, MissingPath::Allowed)?;
            if directory.exists() {
                collect_directory_files(workspace_root, &source_root, &directory, &mut manifest)?;
            }
        }
    }
    Ok(manifest)
}

fn capture_manifest_at_alternate_root(
    target: &SourceTarget,
    alternate_source_root: &Path,
) -> Result<SourceManifest, String> {
    reject_symlink(alternate_source_root, MissingPath::Rejected)?;
    let alternate_source_root = alternate_source_root.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize alternate source root {}: {error}",
            alternate_source_root.display()
        )
    })?;
    if !alternate_source_root.is_dir() {
        return Err(format!(
            "alternate source root is not a directory: {}",
            alternate_source_root.display()
        ));
    }
    let mut manifest = SourceManifest::default();
    match &target.scope {
        SourceTargetScope::Module { path } => {
            capture_alternate_declared_file(target, &alternate_source_root, path, &mut manifest)?;
        }
        SourceTargetScope::MetadataOwner {
            descriptor_path,
            owner_directory,
        } => {
            capture_alternate_declared_file(
                target,
                &alternate_source_root,
                descriptor_path,
                &mut manifest,
            )?;
            let relative_owner = strip_source_root(target, owner_directory)?;
            let alternate_owner = alternate_source_root.join(&relative_owner);
            reject_symlink(&alternate_owner, MissingPath::Allowed)?;
            if alternate_owner.exists() {
                collect_alternate_directory_files(
                    target,
                    &alternate_source_root,
                    &alternate_owner,
                    &mut manifest,
                )?;
            }
        }
    }
    Ok(manifest)
}

fn capture_alternate_declared_file(
    target: &SourceTarget,
    alternate_source_root: &Path,
    original_path: &RelativeSourcePath,
    manifest: &mut SourceManifest,
) -> Result<(), String> {
    let relative = strip_source_root(target, original_path)?;
    let alternate = alternate_source_root.join(relative);
    reject_symlink(&alternate, MissingPath::Allowed)?;
    let fingerprint = if alternate.exists() {
        FileFingerprint::present(&read_stable_regular_file(
            &alternate,
            alternate_source_root,
        )?)
    } else {
        FileFingerprint::Deleted
    };
    manifest.files.insert(original_path.clone(), fingerprint);
    Ok(())
}

fn collect_alternate_directory_files(
    target: &SourceTarget,
    alternate_source_root: &Path,
    directory: &Path,
    manifest: &mut SourceManifest,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "source-sync rejects symlink inside shadow target: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            collect_alternate_directory_files(target, alternate_source_root, &path, manifest)?;
        } else if metadata.is_file() {
            if is_cdfi_path(&path) {
                continue;
            }
            let relative = path.strip_prefix(alternate_source_root).map_err(|_| {
                format!(
                    "shadow target escapes alternate source root: {}",
                    path.display()
                )
            })?;
            let original = join_source_relative_path(&target.source_root, relative)?;
            manifest.files.insert(
                original,
                FileFingerprint::present(&read_stable_regular_file(&path, alternate_source_root)?),
            );
        } else {
            return Err(format!(
                "shadow source target contains non-regular entry {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn strip_source_root(
    target: &SourceTarget,
    workspace_path: &RelativeSourcePath,
) -> Result<PathBuf, String> {
    let full = Path::new(workspace_path.as_str());
    if target.source_root.as_str() == "." {
        return Ok(full.to_path_buf());
    }
    full.strip_prefix(target.source_root.as_str())
        .map(Path::to_path_buf)
        .map_err(|_| {
            format!(
                "target path `{}` is outside declared source root `{}`",
                workspace_path.as_str(),
                target.source_root.as_str()
            )
        })
}

fn join_source_relative_path(
    source_root: &RelativeSourcePath,
    relative: &Path,
) -> Result<RelativeSourcePath, String> {
    let joined = if source_root.as_str() == "." {
        relative.to_path_buf()
    } else {
        Path::new(source_root.as_str()).join(relative)
    };
    RelativeSourcePath::new(path_for_json(&joined))
}

fn capture_declared_file(
    workspace_root: &Path,
    source_root: &Path,
    relative: &RelativeSourcePath,
    manifest: &mut SourceManifest,
) -> Result<(), String> {
    if is_cdfi_name(relative.as_str()) {
        return Err(format!(
            "ConfigDumpInfo.xml is excluded from source-sync: {}",
            relative.as_str()
        ));
    }
    let path = workspace_path(workspace_root, relative)?;
    reject_symlink(&path, MissingPath::Allowed)?;
    let fingerprint = if path.exists() {
        FileFingerprint::present(&read_stable_regular_file(&path, source_root)?)
    } else {
        FileFingerprint::Deleted
    };
    manifest.files.insert(relative.clone(), fingerprint);
    Ok(())
}

fn collect_directory_files(
    workspace_root: &Path,
    source_root: &Path,
    directory: &Path,
    manifest: &mut SourceManifest,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "source-sync rejects symlink inside target: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            collect_directory_files(workspace_root, source_root, &path, manifest)?;
        } else if metadata.is_file() {
            if is_cdfi_path(&path) {
                continue;
            }
            let relative = RelativeSourcePath::new(relative_to_workspace(workspace_root, &path)?)?;
            manifest.files.insert(
                relative,
                FileFingerprint::present(&read_stable_regular_file(&path, source_root)?),
            );
        } else {
            return Err(format!(
                "source target contains non-regular entry {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn read_stable_regular_file(path: &Path, containment_root: &Path) -> Result<Vec<u8>, String> {
    reject_symlink(path, MissingPath::Rejected)?;
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))?;
    let root = containment_root.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize {}: {error}",
            containment_root.display()
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(format!(
            "source file escapes target root: {}",
            path.display()
        ));
    }
    let mut first = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut first))
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    reject_symlink(path, MissingPath::Rejected)?;
    let second =
        fs::read(path).map_err(|error| format!("failed to verify {}: {error}", path.display()))?;
    if first != second {
        return Err(format!(
            "source file changed while capturing manifest: {}",
            path.display()
        ));
    }
    Ok(first)
}

fn normalize_manifest(
    mut manifest: SourceManifest,
    previous: Option<&SourceManifest>,
    synchronized: Option<&SynchronizedManifest>,
) -> SourceManifest {
    if let Some(previous) = previous {
        for path in previous.files.keys() {
            manifest
                .files
                .entry(path.clone())
                .or_insert(FileFingerprint::Deleted);
        }
    }
    if let Some(synchronized) = synchronized {
        for path in synchronized.files.keys() {
            manifest
                .files
                .entry(path.clone())
                .or_insert(FileFingerprint::Deleted);
        }
    }
    manifest
}

fn ensure_same_target(existing: &SourceTarget, requested: &SourceTarget) -> Result<(), String> {
    if existing == requested {
        Ok(())
    } else {
        Err(format!(
            "target id `{}` resolved to a different source target",
            requested.id.as_str()
        ))
    }
}

fn next_generation(generation: u64) -> Result<u64, String> {
    generation
        .checked_add(1)
        .ok_or_else(|| "source-sync generation overflow".to_string())
}

fn first_string<'a>(args: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn first_identity_string<'a>(args: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
}

fn workspace_path(root: &Path, relative: &RelativeSourcePath) -> Result<PathBuf, String> {
    let path = if relative.as_str() == "." {
        root.to_path_buf()
    } else {
        root.join(relative.as_str())
    };
    let path = normalize_lexically(&path);
    if path.starts_with(root) {
        Ok(path)
    } else {
        Err(format!(
            "source path escapes workspace: {}",
            relative.as_str()
        ))
    }
}

fn relative_to_workspace(root: &Path, path: &Path) -> Result<String, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize workspace: {error}"))?;
    let normalized = normalize_lexically(path);
    let relative = normalized
        .strip_prefix(&canonical_root)
        .or_else(|_| normalized.strip_prefix(root))
        .map_err(|_| format!("path is outside workspace: {}", path.display()))?;
    let value = path_for_json(relative);
    Ok(if value.is_empty() {
        ".".to_string()
    } else {
        value
    })
}

fn ensure_contained(path: &Path, root: &Path) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize workspace: {error}"))?;
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(format!("source root escapes workspace: {}", path.display()))
    }
}

fn path_for_json(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_for_identity(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.display().to_string().replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.display().to_string()
    }
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[derive(Debug, Clone, Copy)]
enum MissingPath {
    Allowed,
    Rejected,
}

fn reject_symlink(path: &Path, missing: MissingPath) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("source-sync refuses symlink {}", path.display()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match missing {
            MissingPath::Allowed => Ok(()),
            MissingPath::Rejected => {
                Err(format!("source-sync path is missing: {}", path.display()))
            }
        },
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

fn is_cdfi_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
}

fn is_cdfi_name(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("failed to sync directory {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    // `std::fs::File::open` cannot open directories on Windows without
    // FILE_FLAG_BACKUP_SEMANTICS. Atomic replacements there use
    // MoveFileExW(MOVEFILE_WRITE_THROUGH), so directory fsync is a Unix-only
    // durability primitive at this layer.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn metadata_owner_manifest_covers_raw_bundle_and_excludes_cdfi() {
        let fixture = Fixture::new("metadata-bundle");
        let descriptor = fixture.source.join("Catalogs/Goods.xml");
        write(
            &descriptor,
            br#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><Name>Goods</Name></Properties></Catalog></MetaDataObject>"#,
        );
        let module = b"\xef\xbb\xbfProcedure Test()\r\nEndProcedure\r\n";
        write(
            &fixture.source.join("Catalogs/Goods/Ext/ObjectModule.bsl"),
            module,
        );
        write(
            &fixture.source.join("Catalogs/Goods.xml"),
            b"<MetaDataObject><Catalog><Properties><Name>Goods</Name></Properties></Catalog></MetaDataObject>",
        );
        write(
            &fixture
                .source
                .join("Catalogs/Goods/Forms/Item/Ext/Form.xml"),
            b"<Form/>\n",
        );
        write(&fixture.source.join("ConfigDumpInfo.xml"), b"platform");
        fixture.register_object("Catalog", "Goods");
        let args = json!({"ObjectPath": descriptor.display().to_string()})
            .as_object()
            .unwrap()
            .clone();

        let target = resolve_mutation_target("unica.meta.edit", &args, &fixture.context).unwrap();
        let manifest = fixture.repository.capture_manifest(&target).unwrap();

        assert_eq!(target.id.as_str(), "metadata:main:Catalog:Goods");
        assert_eq!(target.owner_selector, "Catalog:Goods");
        assert_eq!(manifest.files.len(), 3);
        assert!(!manifest
            .files
            .keys()
            .any(|path| path.as_str().ends_with("ConfigDumpInfo.xml")));
        let fingerprint = manifest
            .files
            .iter()
            .find(|(path, _)| path.as_str().ends_with("ObjectModule.bsl"))
            .map(|(_, fingerprint)| fingerprint)
            .unwrap();
        assert_eq!(fingerprint, &FileFingerprint::present(module));
    }

    #[test]
    fn module_resolution_is_source_set_scoped_and_exact() {
        let fixture = Fixture::new("module");
        let (target, _) = fixture.module_target("Jobs", b"Procedure Run()\nEndProcedure\n");
        let manifest = fixture.repository.capture_manifest(&target).unwrap();

        assert_eq!(
            target.id.as_str(),
            "module:main:CommonModules/Jobs/Ext/Module.bsl"
        );
        assert_eq!(target.owner_selector, "CommonModule:Jobs");
        assert_eq!(target.source_set.as_ref().unwrap().as_str(), "main");
        assert_eq!(manifest.files.len(), 1);
    }

    #[test]
    fn module_resolution_preserves_raw_source_set_identity() {
        let fixture = Fixture::new("module-source-set-identity");
        write(
            &fixture.context.workspace_root.join("v8project.yaml"),
            b"format: DESIGNER\nsource-set:\n  - name: \" main \"\n    type: CONFIGURATION\n    path: src\n",
        );
        let module = fixture.source.join("CommonModules/Jobs/Ext/Module.bsl");
        write(&module, b"Procedure Run()\nEndProcedure\n");
        write(
            &fixture.source.join("CommonModules/Jobs.xml"),
            b"<MetaDataObject><CommonModule><Properties><Name>Jobs</Name></Properties></CommonModule></MetaDataObject>",
        );
        fixture.register_object("CommonModule", "Jobs");
        let args = json!({
            "sourceSet": " main ",
            "modulePath": "CommonModules/Jobs/Ext/Module.bsl"
        })
        .as_object()
        .unwrap()
        .clone();

        let target = resolve_mutation_target("unica.code.patch", &args, &fixture.context).unwrap();

        assert_eq!(target.source_set.as_ref().unwrap().as_str(), " main ");
        assert_eq!(
            target.id.as_str(),
            "module: main :CommonModules/Jobs/Ext/Module.bsl"
        );
    }

    #[test]
    fn module_resolution_requires_a_designer_addressable_owner() {
        let fixture = Fixture::new("module-owner");
        write(
            &fixture.source.join("CommonCommands/Run.xml"),
            b"<MetaDataObject><CommonCommand><Properties><Name>Run</Name></Properties></CommonCommand></MetaDataObject>",
        );
        write(
            &fixture
                .source
                .join("CommonCommands/Run/Ext/CommandModule.bsl"),
            b"Procedure Execute()\nEndProcedure\n",
        );
        fixture.register_object("CommonCommand", "Run");
        let common_command = json!({
            "sourceSet": "main",
            "modulePath": "CommonCommands/Run/Ext/CommandModule.bsl"
        })
        .as_object()
        .unwrap()
        .clone();
        let common_command =
            resolve_mutation_target("unica.code.patch", &common_command, &fixture.context).unwrap();
        assert_eq!(common_command.owner_selector, "CommonCommand:Run");

        write(
            &fixture.source.join("Ext/ManagedApplicationModule.bsl"),
            b"Procedure Start()\nEndProcedure\n",
        );
        let configuration = json!({
            "sourceSet": "main",
            "modulePath": "Ext/ManagedApplicationModule.bsl"
        })
        .as_object()
        .unwrap()
        .clone();
        let error = resolve_mutation_target("unica.code.patch", &configuration, &fixture.context)
            .unwrap_err();
        assert!(error.contains("root configuration module"), "{error}");

        write(
            &fixture.source.join("Catalogs/Goods.xml"),
            b"<MetaDataObject><Catalog><Properties><Name>Goods</Name></Properties></Catalog></MetaDataObject>",
        );
        fixture.register_object("Catalog", "Goods");
        write(
            &fixture.source.join("Catalogs/Goods/Trash/Loose.bsl"),
            b"Procedure Loose()\nEndProcedure\n",
        );
        let arbitrary_nested = json!({
            "sourceSet": "main",
            "modulePath": "Catalogs/Goods/Trash/Loose.bsl"
        })
        .as_object()
        .unwrap()
        .clone();
        let error =
            resolve_mutation_target("unica.code.patch", &arbitrary_nested, &fixture.context)
                .unwrap_err();
        assert!(error.contains("not whitelisted"), "{error}");

        write(
            &fixture.source.join("Notes/Loose/Ext/Module.bsl"),
            b"Procedure Loose()\nEndProcedure\n",
        );
        let unsupported = json!({
            "sourceSet": "main",
            "modulePath": "Notes/Loose/Ext/Module.bsl"
        })
        .as_object()
        .unwrap()
        .clone();
        let error = resolve_mutation_target("unica.code.patch", &unsupported, &fixture.context)
            .unwrap_err();
        assert!(error.contains("unsupported metadata collection"), "{error}");
    }

    #[test]
    fn module_resolution_allows_standard_nested_and_manager_modules() {
        let fixture = Fixture::new("standard-module-roles");
        let cases = [
            (
                "Catalog",
                "Catalogs",
                "Goods",
                "Catalogs/Goods/Forms/Item/Ext/Form/Module.bsl",
            ),
            (
                "Document",
                "Documents",
                "Order",
                "Documents/Order/Commands/Print/Ext/CommandModule.bsl",
            ),
            (
                "DocumentJournal",
                "DocumentJournals",
                "Sales",
                "DocumentJournals/Sales/Ext/ManagerModule.bsl",
            ),
            (
                "FilterCriterion",
                "FilterCriteria",
                "ByPartner",
                "FilterCriteria/ByPartner/Ext/ManagerModule.bsl",
            ),
            (
                "SettingsStorage",
                "SettingsStorages",
                "Ui",
                "SettingsStorages/Ui/Ext/ManagerModule.bsl",
            ),
        ];
        for (object_type, collection, name, module_path) in cases {
            write(
                &fixture.source.join(format!("{collection}/{name}.xml")),
                format!(
                    "<MetaDataObject><{object_type}><Properties><Name>{name}</Name></Properties></{object_type}></MetaDataObject>"
                )
                .as_bytes(),
            );
            write(
                &fixture.source.join(module_path),
                b"Procedure Test()\nEndProcedure\n",
            );
            fixture.register_object(object_type, name);
            let args = json!({"sourceSet": "main", "modulePath": module_path})
                .as_object()
                .unwrap()
                .clone();
            let target = resolve_mutation_target("unica.code.patch", &args, &fixture.context)
                .unwrap_or_else(|error| panic!("{module_path}: {error}"));
            assert_eq!(target.owner_selector, format!("{object_type}:{name}"));
        }
    }

    #[test]
    fn metadata_resolution_rejects_noncanonical_collection_or_type() {
        let fixture = Fixture::new("metadata-layout");
        let misplaced = fixture.source.join("Notes/Goods.xml");
        write(
            &misplaced,
            b"<MetaDataObject><Catalog><Properties><Name>Goods</Name></Properties></Catalog></MetaDataObject>",
        );
        let args = json!({"ObjectPath": misplaced.display().to_string()})
            .as_object()
            .unwrap()
            .clone();
        let error =
            resolve_mutation_target("unica.meta.edit", &args, &fixture.context).unwrap_err();
        assert!(error.contains("canonical `Catalogs/Goods.xml`"), "{error}");

        let mismatched = fixture.source.join("Catalogs/Order.xml");
        write(
            &mismatched,
            b"<MetaDataObject><Document><Properties><Name>Order</Name></Properties></Document></MetaDataObject>",
        );
        let args = json!({"ObjectPath": mismatched.display().to_string()})
            .as_object()
            .unwrap()
            .clone();
        let error =
            resolve_mutation_target("unica.meta.edit", &args, &fixture.context).unwrap_err();
        assert!(error.contains("canonical `Documents/Order.xml`"), "{error}");
    }

    #[test]
    fn baseline_survives_restart_and_detects_unrecorded_postimage() {
        let fixture = Fixture::new("restart");
        let (target, module) = fixture.module_target("Jobs", b"Before\n");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let pre = fixture.repository.capture_manifest(&target).unwrap();
        fixture.repository.ensure_baseline(&target, &pre).unwrap();
        fs::write(module, b"After\r\n").unwrap();

        let restarted = SourceSyncRepository::new(&fixture.context).unwrap();
        let record = restarted.reconcile_current(&target.id).unwrap();

        assert!(record.is_dirty());
        assert_ne!(record.current, pre);
        assert!(restarted.has_active_state().unwrap());
    }

    #[test]
    fn no_op_baseline_restores_original_generation_and_leaves_no_target() {
        let fixture = Fixture::new("no-op");
        let (target, _) = fixture.module_target("Jobs", b"Before\n");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let pre = fixture.repository.capture_manifest(&target).unwrap();
        let receipt = fixture.repository.ensure_baseline(&target, &pre).unwrap();
        assert_eq!(receipt.previous_generation, 0);
        assert!(fixture
            .repository
            .discard_clean_baseline(&receipt, &pre)
            .unwrap());
        let state = fixture.repository.load_state().unwrap();
        assert_eq!(state.generation, 0);
        assert!(state.targets.is_empty());
    }

    #[test]
    fn repeated_mutation_preserves_first_baseline_and_explicit_deletion() {
        let fixture = Fixture::new("deletion");
        let (target, module) = fixture.module_target("Jobs", b"One\n");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let first = fixture.repository.capture_manifest(&target).unwrap();
        fixture.repository.ensure_baseline(&target, &first).unwrap();
        fs::write(&module, b"Two\n").unwrap();
        let second = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .record_mutation(&target, &first, &second)
            .unwrap();
        fs::remove_file(module).unwrap();
        let deleted = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .record_mutation(&target, &second, &deleted)
            .unwrap();

        let record = fixture.repository.target(&target.id).unwrap().unwrap();
        assert!(record.is_dirty());
        assert!(matches!(
            record.current.files.values().next(),
            Some(FileFingerprint::Deleted)
        ));
        assert!(record.synchronized.matches_current(&first));
    }

    #[test]
    fn shared_cache_keeps_canonical_workspaces_separate() {
        let root = temp_root("shared-cache");
        let cache = root.join("cache");
        let first = Fixture::at(root.join("one"), cache.clone());
        let second = Fixture::at(root.join("two"), cache);

        assert_ne!(
            first.repository.workspace_id(),
            second.repository.workspace_id()
        );
        assert_ne!(
            first.repository.transaction_root(),
            second.repository.transaction_root()
        );
        let (target, _) = first.module_target("Jobs", b"One\n");
        let _guard = first.repository.acquire_lifecycle_lock().unwrap();
        let manifest = first.repository.capture_manifest(&target).unwrap();
        first
            .repository
            .ensure_baseline(&target, &manifest)
            .unwrap();
        assert!(second.repository.load_state().unwrap().targets.is_empty());
    }

    #[test]
    fn same_workspace_uses_one_authority_across_cache_roots() {
        let fixture = Fixture::new("cache-independent-authority");
        let first_cache = fixture.context.cache_root.clone();
        let second_cache = fixture.root.join("other-cache");
        let mut second_context = fixture.context.clone();
        second_context.cache_root = second_cache.clone();
        let second_repository = SourceSyncRepository::new(&second_context).unwrap();
        let expected_root = fixture
            .context
            .workspace_root
            .canonicalize()
            .unwrap()
            .join(".build")
            .join("unica")
            .join("source-sync")
            .join(fixture.repository.workspace_id());

        assert_eq!(fixture.repository.transaction_root(), expected_root);
        assert_eq!(
            fixture.repository.transaction_root(),
            second_repository.transaction_root()
        );
        assert_eq!(fixture.repository.state_path, second_repository.state_path);
        assert_eq!(fixture.repository.lock_path, second_repository.lock_path);

        let (target, _) = fixture.module_target("Jobs", b"Before\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let manifest = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &manifest)
            .unwrap();
        drop(guard);

        assert!(second_repository
            .load_state()
            .unwrap()
            .targets
            .contains_key(&target.id));
        assert!(!first_cache.exists());
        assert!(!second_cache.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_workspace_build_root_is_rejected_before_external_write() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("symlinked-build-authority");
        let outside = fixture.root.join("outside-authority");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, fixture.context.workspace_root.join(".build")).unwrap();

        let read_error = fixture.repository.load_state().unwrap_err();
        let error = fixture.repository.acquire_lifecycle_lock().unwrap_err();

        assert!(read_error.contains("non-symlink directory"), "{read_error}");
        assert!(error.contains("non-symlink directory"), "{error}");
        assert!(fs::read_dir(outside).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn source_sync_authority_and_state_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new("private-authority");
        let (target, _) = fixture.module_target("Jobs", b"Before\n");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let manifest = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &manifest)
            .unwrap();

        for directory in [
            fixture.repository.transaction_root.parent().unwrap(),
            fixture.repository.transaction_root(),
        ] {
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700,
                "{} must be private",
                directory.display()
            );
        }
        for file in [
            fixture.repository.lock_path.as_path(),
            fixture.repository.state_path.as_path(),
        ] {
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600,
                "{} must be private",
                file.display()
            );
        }
    }

    #[test]
    fn malformed_newer_and_foreign_state_fail_closed() {
        for (name, content, expected) in [
            ("malformed", "{", "malformed"),
            (
                "newer",
                r#"{"schemaVersion":999,"generation":0,"workspaceId":"x","workspaceRoot":"x","targets":{}}"#,
                "newer schemaVersion",
            ),
        ] {
            let fixture = Fixture::new(name);
            fixture.repository.prepare_storage().unwrap();
            fs::write(&fixture.repository.state_path, content).unwrap();
            let error = fixture.repository.load_state().unwrap_err();
            assert!(error.contains(expected), "{error}");
        }

        let fixture = Fixture::new("foreign");
        fixture.repository.prepare_storage().unwrap();
        let mut state = fixture.repository.empty_state();
        state.workspace_id = "foreign".to_string();
        fs::write(
            &fixture.repository.state_path,
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert!(fixture
            .repository
            .load_state()
            .unwrap_err()
            .contains("foreign workspace"));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_state_symlink_fails_closed() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("dangling-state-symlink");
        fixture.repository.prepare_storage().unwrap();
        let missing_target = fixture.root.join("missing-state-target.json");
        symlink(&missing_target, &fixture.repository.state_path).unwrap();

        assert!(!fixture.repository.state_path.exists());
        let error = fixture.repository.load_state().unwrap_err();
        assert!(error.contains("refuses symlink"), "{error}");
    }

    #[test]
    fn atomic_write_failure_preserves_previous_state_and_cleans_staging() {
        let fixture = Fixture::new("atomic");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let state = fixture.repository.empty_state();
        fixture.repository.write_state_atomically(&state).unwrap();
        let mut next = state.clone();
        next.generation = 1;

        let error = fixture
            .repository
            .write_state_atomically_with(&next, |_| Err("injected failure".to_string()))
            .unwrap_err();

        assert_eq!(error, "injected failure");
        assert_eq!(fixture.repository.load_state().unwrap(), state);
        let names = fs::read_dir(fixture.repository.transaction_root())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!names.iter().any(|name| name.ends_with(".tmp")));
    }

    #[test]
    fn generation_and_manifest_cas_never_clear_concurrent_source_change() {
        let fixture = Fixture::new("cas");
        let (target, module) = fixture.module_target("Jobs", b"Before\n");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let pre = fixture.repository.capture_manifest(&target).unwrap();
        fixture.repository.ensure_baseline(&target, &pre).unwrap();
        fs::write(&module, b"After\n").unwrap();
        let post = fixture.repository.capture_manifest(&target).unwrap();
        let recorded = fixture
            .repository
            .record_mutation(&target, &pre, &post)
            .unwrap();

        let stale = fixture
            .repository
            .mark_synchronized_target(recorded.generation - 1, target.id.clone(), post.clone())
            .unwrap();
        assert!(stale.processed.is_empty());
        assert!(stale.conflicted[0].reason.starts_with("staleGeneration"));

        fs::write(module, b"Concurrent\n").unwrap();
        let changed = fixture
            .repository
            .mark_synchronized_target(recorded.generation, target.id.clone(), post)
            .unwrap();
        assert!(changed.processed.is_empty());
        assert_eq!(changed.conflicted[0].reason, "manifestChanged");
        assert!(fixture
            .repository
            .target(&target.id)
            .unwrap()
            .unwrap()
            .is_dirty());
    }

    #[test]
    fn shadow_capture_remaps_paths_and_fills_known_deletions() {
        let fixture = Fixture::new("shadow");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        fs::write(module, b"Dirty\n").unwrap();
        let dirty = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .record_mutation(&target, &working, &dirty)
            .unwrap();
        let shadow = fixture.root.join("shadow");
        fs::create_dir_all(&shadow).unwrap();

        let absent = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        assert!(matches!(
            absent.files.values().next(),
            Some(FileFingerprint::Deleted)
        ));

        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Infobase\r\n",
        );
        let captured = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        assert_eq!(captured.files.len(), 1);
        assert_eq!(
            captured.files.values().next().unwrap(),
            &FileFingerprint::present(b"Infobase\r\n")
        );
    }

    #[test]
    fn multi_file_publication_rolls_back_every_byte_after_injected_failure() {
        let fixture = Fixture::new("publish-rollback");
        let (target, descriptor, module, form) = fixture.catalog_target();
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let record = fixture.repository.target(&target.id).unwrap().unwrap();
        let requested = [record];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow");
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        write(
            &shadow.join("Catalogs/Goods.xml"),
            b"<MetaDataObject><Catalog><Properties><Name>Goods</Name><Comment>IB</Comment></Properties></Catalog></MetaDataObject>",
        );
        write(
            &shadow.join("Catalogs/Goods/Ext/ObjectModule.bsl"),
            b"InfobaseModule\r\n",
        );
        write(
            &shadow.join("Catalogs/Goods/Forms/Item/Ext/Form.xml"),
            b"<Form>Infobase</Form>\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        let before = [
            fs::read(&descriptor).unwrap(),
            fs::read(&module).unwrap(),
            fs::read(&form).unwrap(),
        ];

        let error = fixture
            .repository
            .publish_from_source_root_transaction(
                PublicationRequest {
                    guard: &guard,
                    requested: &requested,
                    desired: &BTreeMap::from([(target.id.clone(), desired)]),
                    alternate_source_root: &shadow,
                    cdfi_preimage: &cdfi_preimage,
                },
                PublicationFailureHandling::Rollback,
                |index| {
                    if index == 0 {
                        Err("injected publication failure".to_string())
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();

        assert!(error.to_string().contains("injected publication failure"));
        assert_eq!(fs::read(descriptor).unwrap(), before[0]);
        assert_eq!(fs::read(module).unwrap(), before[1]);
        assert_eq!(fs::read(form).unwrap(), before[2]);
        let publications = fixture
            .repository
            .transaction_root()
            .join(PUBLICATIONS_DIR_NAME);
        assert_eq!(fs::read_dir(publications).unwrap().count(), 0);
    }

    #[test]
    fn final_manifest_cas_blocks_publication_before_any_working_write() {
        let fixture = Fixture::new("publish-cas");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let record = fixture.repository.target(&target.id).unwrap().unwrap();
        let requested = [record];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow");
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Infobase\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        fs::write(&module, b"Concurrent editor\n").unwrap();

        let error = fixture
            .repository
            .publish_from_source_root(
                &guard,
                &requested,
                &BTreeMap::from([(target.id, desired)]),
                &shadow,
                &cdfi_preimage,
            )
            .unwrap_err();

        assert!(error.to_string().contains("CAS"), "{error}");
        assert_eq!(fs::read(module).unwrap(), b"Concurrent editor\n");
    }

    #[test]
    fn forced_publication_creates_and_deletes_only_owned_files() {
        let fixture = Fixture::new("publish-create-delete");
        let (target, descriptor, module, form) = fixture.catalog_target();
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let record = fixture.repository.target(&target.id).unwrap().unwrap();
        let requested = [record];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow");
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        write(
            &shadow.join("Catalogs/Goods.xml"),
            &fs::read(&descriptor).unwrap(),
        );
        write(
            &shadow.join("Catalogs/Goods/Ext/ObjectModule.bsl"),
            b"Replaced\n",
        );
        let new_file = shadow.join("Catalogs/Goods/Forms/New/Ext/Form.xml");
        write(&new_file, b"<Form>New</Form>\n");
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();

        fixture
            .repository
            .publish_from_source_root(
                &guard,
                &requested,
                &BTreeMap::from([(target.id, desired)]),
                &shadow,
                &cdfi_preimage,
            )
            .unwrap();

        assert_eq!(fs::read(module).unwrap(), b"Replaced\n");
        assert!(!form.exists());
        assert_eq!(
            fs::read(fixture.source.join("Catalogs/Goods/Forms/New/Ext/Form.xml")).unwrap(),
            b"<Form>New</Form>\n"
        );
        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            b"<ConfigDumpInfo version=\"working\"/>\r\n"
        );
    }

    #[test]
    fn overlapping_metadata_and_module_targets_publish_one_consistent_file() {
        let fixture = Fixture::new("publish-overlap");
        let (metadata_target, descriptor, module, form) = fixture.catalog_target();
        let module_args = json!({
            "sourceSet": "main",
            "modulePath": "Catalogs/Goods/Ext/ObjectModule.bsl"
        })
        .as_object()
        .unwrap()
        .clone();
        let module_target =
            resolve_mutation_target("unica.code.patch", &module_args, &fixture.context).unwrap();
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        for target in [&metadata_target, &module_target] {
            let manifest = fixture.repository.capture_manifest(target).unwrap();
            fixture
                .repository
                .ensure_baseline(target, &manifest)
                .unwrap();
        }
        let requested = [
            fixture
                .repository
                .target(&metadata_target.id)
                .unwrap()
                .unwrap(),
            fixture
                .repository
                .target(&module_target.id)
                .unwrap()
                .unwrap(),
        ];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow");
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        write(
            &shadow.join("Catalogs/Goods.xml"),
            &fs::read(descriptor).unwrap(),
        );
        write(
            &shadow.join("Catalogs/Goods/Ext/ObjectModule.bsl"),
            b"Shared desired bytes\n",
        );
        write(
            &shadow.join("Catalogs/Goods/Forms/Item/Ext/Form.xml"),
            &fs::read(form).unwrap(),
        );
        let desired = requested
            .iter()
            .map(|record| {
                fixture
                    .repository
                    .capture_manifest_from_source_root(&record.target, &shadow)
                    .map(|manifest| (record.target.id.clone(), manifest))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .unwrap();

        fixture
            .repository
            .publish_from_source_root(&guard, &requested, &desired, &shadow, &cdfi_preimage)
            .unwrap();

        assert_eq!(fs::read(module).unwrap(), b"Shared desired bytes\n");
    }

    #[test]
    fn prepared_orphan_is_rolled_back_on_restart_recovery() {
        let fixture = Fixture::new("publish-recovery");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let record = fixture.repository.target(&target.id).unwrap().unwrap();
        let requested = [record];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow");
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Published before crash\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();

        let error = fixture
            .repository
            .publish_from_source_root_transaction(
                PublicationRequest {
                    guard: &guard,
                    requested: &requested,
                    desired: &BTreeMap::from([(target.id, desired)]),
                    alternate_source_root: &shadow,
                    cdfi_preimage: &cdfi_preimage,
                },
                PublicationFailureHandling::LeavePrepared,
                |_| Err("simulated process crash".to_string()),
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("prepared recovery journal retained"));
        assert_eq!(fs::read(&module).unwrap(), b"Published before crash\n");

        let restarted = SourceSyncRepository::new(&fixture.context).unwrap();
        let report = restarted.recover_pending_publications().unwrap();

        assert_eq!(report.rolled_back.len(), 1);
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
    }

    #[test]
    fn backup_directory_sync_is_required_before_prepared_journal() {
        let fixture = Fixture::new("publish-backup-dir-sync");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow-backup-dir-sync");
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Infobase\n",
        );
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        let backup_sync_called = Cell::new(false);

        let error = prepare_publication_plan_with_backup_sync(
            &fixture.repository,
            &requested,
            &BTreeMap::from([(target.id, desired)]),
            &shadow,
            &cdfi_preimage,
            |backup_dir| {
                backup_sync_called.set(true);
                let backups = fs::read_dir(backup_dir)
                    .map_err(|error| format!("failed to inspect injected backup dir: {error}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| format!("failed to enumerate injected backups: {error}"))?;
                if backups.is_empty() {
                    return Err("backup directory was synced before backup creation".to_string());
                }
                if backup_dir
                    .parent()
                    .is_some_and(|transaction| transaction.join(PUBLICATION_JOURNAL_NAME).exists())
                {
                    return Err("Prepared journal existed before backup directory sync".to_string());
                }
                Err("injected backup directory sync failure".to_string())
            },
        )
        .unwrap_err();

        assert!(backup_sync_called.get());
        assert!(error.contains("injected backup directory sync failure"));
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
        let publications = fixture
            .repository
            .transaction_root()
            .join(PUBLICATIONS_DIR_NAME);
        assert_eq!(fs::read_dir(publications).unwrap().count(), 0);
    }

    #[test]
    fn committed_journal_sync_and_cleanup_failures_never_trigger_rollback() {
        let fixture = Fixture::new("publish-committed-sync-failure");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow-committed-sync-failure");
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Committed infobase bytes\n",
        );
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        let cleanup_blocker = RefCell::new(None::<PathBuf>);

        let error = fixture
            .repository
            .publish_from_source_root_transaction_with_journal_sync(
                PublicationRequest {
                    guard: &guard,
                    requested: &requested,
                    desired: &BTreeMap::from([(target.id, desired)]),
                    alternate_source_root: &shadow,
                    cdfi_preimage: &cdfi_preimage,
                },
                PublicationFailureHandling::Rollback,
                |_| Ok(()),
                |phase, transaction_dir| match phase {
                    PublicationPhase::Prepared => sync_directory(transaction_dir),
                    PublicationPhase::Committed => {
                        let journal = read_publication_journal(transaction_dir)?;
                        let stage = journal
                            .files
                            .iter()
                            .find_map(|file| file.stage_path.as_ref())
                            .ok_or_else(|| "committed journal has no stage path".to_string())?;
                        let blocker = destination_path(&fixture.repository, stage)?;
                        fs::create_dir(&blocker).map_err(|error| {
                            format!("failed to inject committed cleanup blocker: {error}")
                        })?;
                        cleanup_blocker.replace(Some(blocker));
                        Err("injected committed journal directory sync failure".to_string())
                    }
                },
            )
            .unwrap_err();

        assert_eq!(fs::read(&module).unwrap(), b"Committed infobase bytes\n");
        assert!(error.recovery_required);
        assert!(error.source_may_have_changed);
        assert!(error
            .message
            .contains("injected committed journal directory sync failure"));
        assert!(error.message.contains("publication cleanup also failed"));

        let publications = fixture
            .repository
            .transaction_root()
            .join(PUBLICATIONS_DIR_NAME);
        let transaction_dir = fs::read_dir(&publications)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut committed = read_publication_journal(&transaction_dir).unwrap();
        assert_eq!(committed.phase, PublicationPhase::Committed);
        let transaction_id = committed.transaction_id.clone();

        let blocker = cleanup_blocker.into_inner().unwrap();
        fs::remove_dir(blocker).unwrap();
        // Model power loss retaining the last directory-durable Prepared
        // journal even though the unsynced Committed replacement was visible
        // before the crash.
        committed.phase = PublicationPhase::Prepared;
        write_publication_journal_with_sync(&transaction_dir, &committed, sync_directory).unwrap();
        let restarted = SourceSyncRepository::new(&fixture.context).unwrap();
        let report = restarted.recover_pending_publications().unwrap();

        assert!(report.cleaned_committed.is_empty());
        assert_eq!(report.rolled_back, vec![transaction_id]);
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
        assert_eq!(fs::read_dir(publications).unwrap().count(), 0);
    }

    #[test]
    fn pre_journal_crash_orphan_is_removed_without_touching_sources() {
        let fixture = Fixture::new("publish-pre-journal-orphan");
        let (_target, module) = fixture.module_target("Jobs", b"Working\n");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let transaction_dir = fixture
            .repository
            .transaction_root()
            .join(PUBLICATIONS_DIR_NAME)
            .join(format!("publication-{transaction_id}"));
        write(&transaction_dir.join("backups/00000000.bin"), b"backup");
        write(
            &transaction_dir.join(format!(".journal.{}.tmp", uuid::Uuid::new_v4())),
            b"partial journal",
        );

        let report = fixture.repository.recover_pending_publications().unwrap();

        assert_eq!(report.cleaned_unprepared, vec![transaction_id]);
        assert!(!transaction_dir.exists());
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
    }

    #[test]
    fn recovery_rejects_noncanonical_directory_and_mismatched_transaction_id() {
        let fixture = Fixture::new("publish-transaction-identity");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let publications = fixture
            .repository
            .transaction_root()
            .join(PUBLICATIONS_DIR_NAME);
        let invalid = publications.join("publication-not-a-uuid");
        fs::create_dir_all(&invalid).unwrap();

        let invalid_error = fixture
            .repository
            .recover_pending_publications()
            .unwrap_err();
        assert!(invalid_error.contains("invalid publication recovery directory"));
        assert!(invalid.exists());
        fs::remove_dir_all(invalid).unwrap();

        let directory_id = uuid::Uuid::new_v4().to_string();
        let transaction_dir = publications.join(format!("publication-{directory_id}"));
        fs::create_dir_all(&transaction_dir).unwrap();
        let journal = valid_test_publication_journal(uuid::Uuid::new_v4().to_string());
        write(
            &transaction_dir.join(PUBLICATION_JOURNAL_NAME),
            &serde_json::to_vec_pretty(&journal).unwrap(),
        );

        let mismatch = fixture
            .repository
            .recover_pending_publications()
            .unwrap_err();
        assert!(mismatch.contains("does not match directory"));
        assert!(transaction_dir.exists());
    }

    #[test]
    fn journal_validation_pins_stage_names_and_created_destination_ancestors() {
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let journal = valid_test_publication_journal(transaction_id);
        validate_publication_journal(&journal).unwrap();

        let noncanonical_id = uuid::Uuid::new_v4().to_string().to_uppercase();
        let noncanonical = valid_test_publication_journal(noncanonical_id);
        let id_error = validate_publication_journal(&noncanonical).unwrap_err();
        assert!(id_error.contains("transactionId"));
        assert!(id_error.contains("not canonical"));

        let mut unexpected_stage = journal.clone();
        unexpected_stage.files[0].stage_path = Some(
            RelativeSourcePath::new("src/CommonModules/Jobs/Ext/.unica-publish-other.tmp").unwrap(),
        );
        let stage_error = validate_publication_journal(&unexpected_stage).unwrap_err();
        assert!(stage_error.contains("unexpected stage"));

        let mut unrelated_directory = journal.clone();
        unrelated_directory
            .created_directories
            .push(RelativeSourcePath::new("unrelated/empty").unwrap());
        let directory_error = validate_publication_journal(&unrelated_directory).unwrap_err();
        assert!(directory_error.contains("is not an ancestor"));

        let mut duplicate_directory = journal;
        let duplicate = duplicate_directory.created_directories[0].clone();
        duplicate_directory.created_directories.push(duplicate);
        let duplicate_error = validate_publication_journal(&duplicate_directory).unwrap_err();
        assert!(duplicate_error.contains("duplicate created directory"));
    }

    #[test]
    fn forced_publication_publishes_exact_platform_cdfi_bytes_outside_target_state() {
        let fixture = Fixture::new("publish-cdfi-exact");
        let (target, _) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow-cdfi-exact");
        let platform_bytes =
            b"\xef\xbb\xbf<ConfigDumpInfo version=\"platform\">\r\n</ConfigDumpInfo>\r\n";
        write(&shadow.join("ConfigDumpInfo.xml"), platform_bytes);

        let publication = fixture
            .repository
            .publish_from_source_root(
                &guard,
                &requested,
                &BTreeMap::new(),
                &shadow,
                &cdfi_preimage,
            )
            .unwrap();

        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            platform_bytes
        );
        assert_eq!(publication.published_paths.len(), 1);
        assert!(is_cdfi_name(publication.published_paths[0].as_str()));
        assert!(fixture
            .repository
            .target(&target.id)
            .unwrap()
            .unwrap()
            .current
            .files
            .keys()
            .all(|path| !is_cdfi_name(path.as_str())));
    }

    #[test]
    fn missing_shadow_cdfi_fails_before_any_working_write() {
        let fixture = Fixture::new("publish-cdfi-missing");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let cdfi_before = fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap();
        let shadow = fixture.root.join("shadow-cdfi-missing");
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Infobase\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();

        let error = fixture
            .repository
            .publish_from_source_root(
                &guard,
                &requested,
                &BTreeMap::from([(target.id, desired)]),
                &shadow,
                &cdfi_preimage,
            )
            .unwrap_err();

        assert_eq!(error.reason, "publicationPreflightFailed");
        assert!(!error.source_may_have_changed);
        assert!(error.message.contains("ConfigDumpInfo.xml is required"));
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            cdfi_before
        );
    }

    #[test]
    fn cdfi_final_cas_blocks_owner_and_cdfi_publication() {
        let fixture = Fixture::new("publish-cdfi-cas");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow-cdfi-cas");
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Infobase\n",
        );
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"infobase\"/>\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        write(
            &fixture.source.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"concurrent\"/>\n",
        );

        let error = fixture
            .repository
            .publish_from_source_root(
                &guard,
                &requested,
                &BTreeMap::from([(target.id, desired)]),
                &shadow,
                &cdfi_preimage,
            )
            .unwrap_err();

        assert!(error.message.contains("CAS"), "{error}");
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            b"<ConfigDumpInfo version=\"concurrent\"/>\n"
        );
    }

    #[test]
    fn unchanged_shadow_cdfi_still_participates_in_final_cas() {
        let fixture = Fixture::new("publish-unchanged-cdfi-cas");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let shadow = fixture.root.join("shadow-unchanged-cdfi-cas");
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Infobase\n",
        );
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        write(
            &fixture.source.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"concurrent\"/>\n",
        );

        let error = fixture
            .repository
            .publish_from_source_root(
                &guard,
                &requested,
                &BTreeMap::from([(target.id, desired)]),
                &shadow,
                &cdfi_preimage,
            )
            .unwrap_err();

        assert!(error.message.contains("ConfigDumpInfo.xml CAS"), "{error}");
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            b"<ConfigDumpInfo version=\"concurrent\"/>\n"
        );
    }

    #[test]
    fn unchanged_shadow_cdfi_change_during_owner_publication_rolls_back() {
        let fixture = Fixture::new("publish-unchanged-cdfi-midflight");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        assert_eq!(
            cdfi_preimage.original,
            FileFingerprint::present(b"<ConfigDumpInfo version=\"working\"/>\r\n")
        );
        let shadow = fixture.root.join("shadow-unchanged-cdfi-midflight");
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Infobase\n",
        );
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"working\"/>\r\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();
        let concurrent_cdfi = fixture.source.join("ConfigDumpInfo.xml");

        let error = fixture
            .repository
            .publish_from_source_root_transaction(
                PublicationRequest {
                    guard: &guard,
                    requested: &requested,
                    desired: &BTreeMap::from([(target.id, desired)]),
                    alternate_source_root: &shadow,
                    cdfi_preimage: &cdfi_preimage,
                },
                PublicationFailureHandling::Rollback,
                |index| {
                    if index == 0 {
                        assert_eq!(fs::read(&module).unwrap(), b"Infobase\n");
                        write(
                            &concurrent_cdfi,
                            b"<ConfigDumpInfo version=\"concurrent\"/>\n",
                        );
                    }
                    Ok(())
                },
            )
            .unwrap_err();

        assert_eq!(error.reason, "publicationRolledBack");
        assert!(!error.source_may_have_changed);
        assert!(
            error.message.contains("ConfigDumpInfo.xml CAS changed"),
            "{error}"
        );
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
        assert_eq!(
            fs::read(concurrent_cdfi).unwrap(),
            b"<ConfigDumpInfo version=\"concurrent\"/>\n"
        );
    }

    #[test]
    fn injected_owner_and_cdfi_failure_rolls_back_both() {
        let fixture = Fixture::new("publish-cdfi-rollback");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let cdfi_before = fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap();
        let shadow = fixture.root.join("shadow-cdfi-rollback");
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Published owner\n",
        );
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"published\"/>\n",
        );
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();

        let error = fixture
            .repository
            .publish_from_source_root_transaction(
                PublicationRequest {
                    guard: &guard,
                    requested: &requested,
                    desired: &BTreeMap::from([(target.id, desired)]),
                    alternate_source_root: &shadow,
                    cdfi_preimage: &cdfi_preimage,
                },
                PublicationFailureHandling::Rollback,
                |index| {
                    (index != 1)
                        .then_some(())
                        .ok_or_else(|| "injected after CDFI".to_string())
                },
            )
            .unwrap_err();

        assert_eq!(error.reason, "publicationRolledBack");
        assert!(!error.source_may_have_changed);
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            cdfi_before
        );
    }

    #[test]
    fn restart_recovery_restores_owner_and_cdfi_from_prepared_orphan() {
        let fixture = Fixture::new("publish-cdfi-restart");
        let (target, module) = fixture.module_target("Jobs", b"Working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let requested = [fixture.repository.target(&target.id).unwrap().unwrap()];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let cdfi_before = fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap();
        let shadow = fixture.root.join("shadow-cdfi-restart");
        let cdfi_after = b"<ConfigDumpInfo version=\"published\"/>\n";
        write(
            &shadow.join("CommonModules/Jobs/Ext/Module.bsl"),
            b"Published owner\n",
        );
        write(&shadow.join("ConfigDumpInfo.xml"), cdfi_after);
        let desired = fixture
            .repository
            .capture_manifest_from_source_root(&target, &shadow)
            .unwrap();

        let error = fixture
            .repository
            .publish_from_source_root_transaction(
                PublicationRequest {
                    guard: &guard,
                    requested: &requested,
                    desired: &BTreeMap::from([(target.id, desired)]),
                    alternate_source_root: &shadow,
                    cdfi_preimage: &cdfi_preimage,
                },
                PublicationFailureHandling::LeavePrepared,
                |index| {
                    (index != 1)
                        .then_some(())
                        .ok_or_else(|| "simulated crash".to_string())
                },
            )
            .unwrap_err();
        assert!(error.source_may_have_changed);
        assert!(error.recovery_required);
        assert_eq!(fs::read(&module).unwrap(), b"Published owner\n");
        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            cdfi_after
        );

        let restarted = SourceSyncRepository::new(&fixture.context).unwrap();
        let report = restarted.recover_pending_publications().unwrap();

        assert_eq!(report.rolled_back.len(), 1);
        assert_eq!(fs::read(module).unwrap(), b"Working\n");
        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            cdfi_before
        );
    }

    #[test]
    fn unchanged_requested_target_cas_blocks_entire_forced_batch() {
        let fixture = Fixture::new("publish-full-batch-cas");
        let (changed_target, changed_module) = fixture.module_target("Changed", b"A working\n");
        let (stable_target, stable_module) = fixture.module_target("Stable", b"B working\n");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        for target in [&changed_target, &stable_target] {
            let manifest = fixture.repository.capture_manifest(target).unwrap();
            fixture
                .repository
                .ensure_baseline(target, &manifest)
                .unwrap();
        }
        let requested = [
            fixture
                .repository
                .target(&changed_target.id)
                .unwrap()
                .unwrap(),
            fixture
                .repository
                .target(&stable_target.id)
                .unwrap()
                .unwrap(),
        ];
        let cdfi_preimage = fixture.cdfi_preimage(&guard, &requested);
        let cdfi_before = fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap();
        let shadow = fixture.root.join("shadow-full-batch-cas");
        write(
            &shadow.join("CommonModules/Changed/Ext/Module.bsl"),
            b"A infobase\n",
        );
        write(
            &shadow.join("ConfigDumpInfo.xml"),
            b"<ConfigDumpInfo version=\"infobase\"/>\n",
        );
        let changed_desired = fixture
            .repository
            .capture_manifest_from_source_root(&changed_target, &shadow)
            .unwrap();
        write(&stable_module, b"B concurrent editor\n");

        let error = fixture
            .repository
            .publish_from_source_root(
                &guard,
                &requested,
                &BTreeMap::from([(changed_target.id, changed_desired)]),
                &shadow,
                &cdfi_preimage,
            )
            .unwrap_err();

        assert!(error.message.contains("Stable") || error.message.contains("manifest CAS"));
        assert_eq!(fs::read(changed_module).unwrap(), b"A working\n");
        assert_eq!(fs::read(stable_module).unwrap(), b"B concurrent editor\n");
        assert_eq!(
            fs::read(fixture.source.join("ConfigDumpInfo.xml")).unwrap(),
            cdfi_before
        );
    }

    #[test]
    fn topology_validation_rejects_configured_source_root_switch() {
        let fixture = Fixture::new("topology-switch");
        let (target, _) = fixture.module_target("Jobs", b"Working\n");
        let _guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let working = fixture.repository.capture_manifest(&target).unwrap();
        fixture
            .repository
            .ensure_baseline(&target, &working)
            .unwrap();
        let record = fixture.repository.target(&target.id).unwrap().unwrap();
        let replacement = fixture.context.workspace_root.join("srcB");
        write(
            &replacement.join("Configuration.xml"),
            b"<MetaDataObject><Configuration><Properties><Name>Other</Name></Properties></Configuration></MetaDataObject>",
        );
        write(
            &fixture.context.workspace_root.join("v8project.yaml"),
            b"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: srcB\n",
        );

        let error = fixture
            .repository
            .validate_target_topology(&record)
            .unwrap_err();

        assert!(error.contains("source topology changed"), "{error}");
    }

    #[test]
    fn atomic_replace_contract_replaces_existing_destination() {
        let root = temp_root("replace-existing");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("stage.tmp");
        let destination = root.join("state.json");
        write(&source, b"new");
        write(&destination, b"old");

        replace_path_atomically(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lifecycle_lock_prevents_lost_updates_between_writers() {
        let fixture = Fixture::new("concurrent");
        for name in ["One", "Two"] {
            write(
                &fixture
                    .source
                    .join(format!("CommonModules/{name}.xml")),
                format!(
                    "<MetaDataObject><CommonModule><Properties><Name>{name}</Name></Properties></CommonModule></MetaDataObject>"
                )
                .as_bytes(),
            );
            fixture.register_object("CommonModule", name);
        }
        let repository = Arc::new(fixture.repository.clone());
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for name in ["One", "Two"] {
            let repository = Arc::clone(&repository);
            let barrier = Arc::clone(&barrier);
            let context = fixture.context.clone();
            let source = fixture.source.clone();
            handles.push(thread::spawn(move || {
                let module = source.join(format!("CommonModules/{name}/Ext/Module.bsl"));
                write(&module, format!("{name}\n").as_bytes());
                let args = json!({
                    "sourceSet": "main",
                    "modulePath": format!("CommonModules/{name}/Ext/Module.bsl")
                })
                .as_object()
                .unwrap()
                .clone();
                let target = resolve_mutation_target("unica.code.patch", &args, &context).unwrap();
                barrier.wait();
                let _guard = repository.acquire_lifecycle_lock().unwrap();
                let manifest = repository.capture_manifest(&target).unwrap();
                repository.ensure_baseline(&target, &manifest).unwrap();
            }));
        }
        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(repository.load_state().unwrap().targets.len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn inherited_child_lease_keeps_lifecycle_lock_after_parent_guard_drops() {
        use std::process::Command;
        use std::sync::mpsc;
        use std::time::Duration;

        let fixture = Fixture::new("inherited-lifecycle-lease");
        let guard = fixture.repository.acquire_lifecycle_lock().unwrap();
        let child_lease = guard.child_lease().unwrap();
        // `exec` prevents a shell intermediate from retaining the descriptor
        // after the process we explicitly terminate below has exited.
        let mut child = Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .spawn()
            .expect("test helper must start sleep");
        drop(child_lease);
        drop(guard);

        let repository = fixture.repository.clone();
        let (sender, receiver) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let result = repository.acquire_lifecycle_lock();
            sender.send(result.map(|_guard| ())).unwrap();
        });
        let early_result = receiver.recv_timeout(Duration::from_millis(150));
        child.kill().expect("test helper must stop sleep");
        child.wait().expect("test helper must reap sleep");
        match early_result {
            Err(mpsc::RecvTimeoutError::Timeout) => receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("lifecycle lock must become available after child exit")
                .unwrap(),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("lock waiter stopped before child exit")
            }
            Ok(result) => {
                result.unwrap();
                panic!(
                    "a second process-equivalent operation acquired the lock while the inherited child lease was live"
                );
            }
        }
        waiter.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_module_is_rejected_before_hashing() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("symlink");
        let outside = fixture.root.join("outside.bsl");
        write(&outside, b"Outside\n");
        let module = fixture.source.join("CommonModules/Jobs/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        symlink(outside, &module).unwrap();
        let args = json!({
            "sourceSet": "main",
            "modulePath": "CommonModules/Jobs/Ext/Module.bsl"
        })
        .as_object()
        .unwrap()
        .clone();

        let error =
            resolve_mutation_target("unica.code.patch", &args, &fixture.context).unwrap_err();
        assert!(error.contains("symlink"), "{error}");
    }

    fn valid_test_publication_journal(transaction_id: String) -> PublicationJournal {
        let path = RelativeSourcePath::new("src/CommonModules/Jobs/Ext/Module.bsl").unwrap();
        let stage_path = expected_publication_stage_path(&path, &transaction_id, 0).unwrap();
        PublicationJournal {
            schema_version: PUBLICATION_SCHEMA_VERSION,
            workspace_id: "test-workspace".to_string(),
            workspace_root: "/test/workspace".to_string(),
            transaction_id,
            phase: PublicationPhase::Prepared,
            files: vec![PublicationJournalFile {
                role: PublicationFileRole::TargetOwned,
                path,
                original: FileFingerprint::Deleted,
                desired: FileFingerprint::present(b"desired"),
                backup_file: None,
                stage_path: Some(stage_path),
                original_mode: None,
            }],
            created_directories: vec![
                RelativeSourcePath::new("src").unwrap(),
                RelativeSourcePath::new("src/CommonModules").unwrap(),
                RelativeSourcePath::new("src/CommonModules/Jobs").unwrap(),
                RelativeSourcePath::new("src/CommonModules/Jobs/Ext").unwrap(),
            ],
        }
    }

    struct Fixture {
        root: PathBuf,
        source: PathBuf,
        context: WorkspaceContext,
        repository: SourceSyncRepository,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = temp_root(name);
            Self::at(root.join("workspace"), root.join("cache"))
        }

        fn at(workspace: PathBuf, cache_root: PathBuf) -> Self {
            let root = workspace.parent().unwrap().to_path_buf();
            let source = workspace.join("src");
            fs::create_dir_all(&source).unwrap();
            write(
                &workspace.join("v8project.yaml"),
                b"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            );
            write(
                &source.join("Configuration.xml"),
                br#"<MetaDataObject><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects></ChildObjects></Configuration></MetaDataObject>"#,
            );
            write(
                &source.join("ConfigDumpInfo.xml"),
                b"<ConfigDumpInfo version=\"working\"/>\r\n",
            );
            let context = WorkspaceContext {
                cwd: workspace.clone(),
                workspace_root: workspace,
                cache_root,
                workspace_epoch: 1,
            };
            let repository = SourceSyncRepository::new(&context).unwrap();
            Self {
                root,
                source,
                context,
                repository,
            }
        }

        fn module_target(&self, name: &str, bytes: &[u8]) -> (SourceTarget, PathBuf) {
            let relative = format!("CommonModules/{name}/Ext/Module.bsl");
            let module = self.source.join(&relative);
            write(&module, bytes);
            write(
                &self.source.join(format!("CommonModules/{name}.xml")),
                format!(
                    "<MetaDataObject><CommonModule><Properties><Name>{name}</Name></Properties></CommonModule></MetaDataObject>"
                )
                .as_bytes(),
            );
            self.register_object("CommonModule", name);
            let args = json!({"sourceSet": "main", "modulePath": relative})
                .as_object()
                .unwrap()
                .clone();
            (
                resolve_mutation_target("unica.code.patch", &args, &self.context).unwrap(),
                module,
            )
        }

        fn catalog_target(&self) -> (SourceTarget, PathBuf, PathBuf, PathBuf) {
            let descriptor = self.source.join("Catalogs/Goods.xml");
            let module = self.source.join("Catalogs/Goods/Ext/ObjectModule.bsl");
            let form = self.source.join("Catalogs/Goods/Forms/Item/Ext/Form.xml");
            write(
                &descriptor,
                b"<MetaDataObject><Catalog><Properties><Name>Goods</Name></Properties></Catalog></MetaDataObject>",
            );
            write(&module, b"Working module\n");
            write(&form, b"<Form>Working</Form>\n");
            self.register_object("Catalog", "Goods");
            let args = json!({"ObjectPath": descriptor.display().to_string()})
                .as_object()
                .unwrap()
                .clone();
            (
                resolve_mutation_target("unica.meta.edit", &args, &self.context).unwrap(),
                descriptor,
                module,
                form,
            )
        }

        fn register_object(&self, object_type: &str, name: &str) {
            let path = self.source.join("Configuration.xml");
            let text = fs::read_to_string(&path).unwrap();
            let entry = format!("<{object_type}>{name}</{object_type}>");
            if !text.contains(&entry) {
                fs::write(
                    path,
                    text.replacen("</ChildObjects>", &format!("{entry}</ChildObjects>"), 1),
                )
                .unwrap();
            }
        }

        fn cdfi_preimage(
            &self,
            guard: &LifecycleLockGuard,
            requested: &[SourceTargetRecord],
        ) -> PlatformCdfiPreimage {
            self.repository
                .capture_platform_cdfi_preimage(guard, requested)
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-source-sync-{name}-{}-{nanos}",
            std::process::id()
        ))
    }
}

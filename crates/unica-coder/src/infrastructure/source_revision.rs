use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::source_revision::{
    SourceRevision, SourceRevisionMachine, SourceRevisionState, SourceRevisionTrustLoss,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::source_revision_fence::{
    platform_fence, FenceCapability, FenceOutcome, SourceRevisionFence,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const GENERATED_DIR_NAME: &str = ".build";
const MAX_SOURCE_DEPTH: usize = 64;
const REVISION_RECORD_SCHEMA_VERSION: u32 = 2;

#[derive(Clone)]
struct SourceEntryDigest {
    kind: u8,
    digest: [u8; 32],
}

#[cfg(test)]
#[derive(Default)]
struct ReconcileEverySnapshotFence {
    at_reconcile_boundary: AtomicBool,
}

#[cfg(test)]
impl SourceRevisionFence for ReconcileEverySnapshotFence {
    fn capability(&self) -> FenceCapability {
        FenceCapability::ProvenFast
    }

    fn flush(
        &self,
        _deadline: ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> Result<FenceOutcome, String> {
        if self.at_reconcile_boundary.fetch_xor(true, Ordering::AcqRel) {
            Ok(FenceOutcome::Proven {
                changed_paths: Vec::new(),
            })
        } else {
            Ok(FenceOutcome::TrustLost(SourceRevisionTrustLoss::WatcherGap))
        }
    }
}

type SourceManifest = BTreeMap<PathBuf, SourceEntryDigest>;
type SourceManifestScanner =
    dyn Fn(&Path, &(dyn Fn() -> bool + Sync)) -> Result<SourceManifest, String> + Send + Sync;
type SourceFileReader = dyn Fn(&Path) -> Result<Vec<u8>, String> + Send + Sync;

pub(crate) struct SourceRevisionService {
    workspace_root: PathBuf,
    source_root: PathBuf,
    record_path: PathBuf,
    machine: Mutex<SourceRevisionMachine>,
    manifest: Mutex<Option<SourceManifest>>,
    operation: Mutex<()>,
    fence: Arc<dyn SourceRevisionFence>,
    scanner: Arc<SourceManifestScanner>,
    file_reader: Arc<SourceFileReader>,
}

impl fmt::Debug for SourceRevisionService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceRevisionService")
            .field("workspace_root", &self.workspace_root)
            .field("source_root", &self.source_root)
            .field("record_path", &self.record_path)
            .field("fence", &self.fence.capability())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceRevisionRecord {
    schema_version: u32,
    workspace_root: String,
    source_root: String,
    revision: SourceRevision,
}

impl SourceRevisionService {
    pub(crate) fn new(context: &WorkspaceContext, source_root: &Path) -> Result<Self, String> {
        let canonical_source_root = fs::canonicalize(source_root)
            .map_err(|error| format!("source revision root cannot be normalized: {error}"))?;
        let fence = platform_fence(&canonical_source_root, &context.cache_root)?;
        Self::with_fence(context, &canonical_source_root, fence)
    }

    #[cfg(test)]
    pub(crate) fn new_reconciling_for_test(
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<Self, String> {
        Self::with_fence(
            context,
            source_root,
            Arc::new(ReconcileEverySnapshotFence::default()),
        )
    }

    fn with_fence(
        context: &WorkspaceContext,
        source_root: &Path,
        fence: Arc<dyn SourceRevisionFence>,
    ) -> Result<Self, String> {
        let workspace_root = fs::canonicalize(&context.workspace_root)
            .map_err(|error| format!("workspace revision root cannot be normalized: {error}"))?;
        let source_root = fs::canonicalize(source_root)
            .map_err(|error| format!("source revision root cannot be normalized: {error}"))?;
        let mut identity = Sha256::new();
        update_identity_path(&mut identity, &workspace_root);
        identity.update([0]);
        update_identity_path(&mut identity, &source_root);
        let identity = format!("{:x}", identity.finalize());
        let record_path = context
            .cache_root
            .join("source-revisions")
            .join(format!("{identity}.json"));
        let machine = load_revision_record(&record_path, &workspace_root, &source_root)
            .and_then(|revision| SourceRevisionMachine::from_revision(revision).ok())
            .unwrap_or_default();
        Ok(Self {
            workspace_root,
            source_root,
            record_path,
            machine: Mutex::new(machine),
            manifest: Mutex::new(None),
            operation: Mutex::new(()),
            fence,
            scanner: Arc::new(scan_source_manifest),
            file_reader: Arc::new(read_source_file),
        })
    }

    pub(crate) fn snapshot(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<SourceRevision, String> {
        let _operation = self
            .operation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.fence.capability() == FenceCapability::Unsupported {
            self.machine
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .lose_trust(SourceRevisionTrustLoss::UnsupportedFence);
            return Err(
                "source revision fence is unsupported; freshness cannot be proven".to_string(),
            );
        }
        let fence_outcome = self.fence.flush(deadline, cancellation).inspect_err(|_| {
            self.machine
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
        })?;
        let needs_reconcile = match fence_outcome {
            FenceOutcome::Proven { changed_paths } => {
                let (trusted, trust_loss_epoch) = {
                    let machine = self
                        .machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    (
                        matches!(machine.state(), SourceRevisionState::Trusted(_)),
                        machine.trust_loss_epoch(),
                    )
                };
                if changed_paths.is_empty() {
                    !trusted
                } else {
                    !(trusted
                        && self.apply_incremental(
                            changed_paths,
                            trust_loss_epoch,
                            deadline,
                            cancellation,
                        )?)
                }
            }
            FenceOutcome::TrustLost(reason) => {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(reason);
                true
            }
        };
        if needs_reconcile {
            self.reconcile(deadline, cancellation)?;
        }
        self.trusted_snapshot()
    }

    fn trusted_snapshot(&self) -> Result<SourceRevision, String> {
        let machine = self
            .machine
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match machine.state() {
            SourceRevisionState::Trusted(revision) => Ok(revision.clone()),
            _ => Err("source revision remains untrusted after reconcile".to_string()),
        }
    }

    pub(crate) fn mark_dirty(&self) {
        self.machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lose_trust(SourceRevisionTrustLoss::WatcherGap);
    }

    fn apply_incremental(
        &self,
        mut changed_paths: Vec<PathBuf>,
        expected_trust_loss_epoch: u64,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<bool, String> {
        let Some(mut manifest) = self
            .manifest
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        else {
            return Ok(false);
        };
        for _ in 0..3 {
            {
                let mut machine = self
                    .machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if machine.trust_loss_epoch() != expected_trust_loss_epoch {
                    return Ok(false);
                }
                machine.begin_reconcile();
            }
            for relative_path in changed_paths {
                if cancellation.is_cancelled() || deadline.remaining().is_zero() {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
                    return Err("source revision reconcile cancelled".to_string());
                }
                if !self.apply_changed_path(&mut manifest, &relative_path)? {
                    return Ok(false);
                }
            }
            match self.fence.flush(deadline, cancellation).inspect_err(|_| {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
            })? {
                FenceOutcome::Proven {
                    changed_paths: additional,
                } if !additional.is_empty() => {
                    changed_paths = additional;
                }
                FenceOutcome::Proven { .. } => {
                    let digest = digest_source_manifest(&manifest)?;
                    return self.publish_revision(&manifest, digest, expected_trust_loss_epoch);
                }
                FenceOutcome::TrustLost(reason) => {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(reason);
                    return Ok(false);
                }
            }
        }
        self.machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
        Ok(false)
    }

    fn apply_changed_path(
        &self,
        manifest: &mut SourceManifest,
        relative_path: &Path,
    ) -> Result<bool, String> {
        if relative_path.is_absolute()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            self.lose_incremental_trust();
            return Err("source revision event escaped its root".to_string());
        }
        let path = self.source_root.join(relative_path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                manifest.remove(relative_path);
                return Ok(true);
            }
            Err(error) => {
                self.lose_incremental_trust();
                return Err(format!("source revision file cannot be inspected: {error}"));
            }
        };
        if metadata.file_type().is_symlink() {
            if is_source_file(&path) {
                self.lose_incremental_trust();
                return Err(format!(
                    "source revision corpus contains an indexed symbolic link: {}",
                    relative_path.display()
                ));
            }
            manifest.remove(relative_path);
            return Ok(true);
        }
        if metadata.is_dir() {
            return Ok(false);
        }
        if !metadata.is_file() || !is_source_file(&path) {
            manifest.remove(relative_path);
            return Ok(true);
        }
        let canonical = fs::canonicalize(&path)
            .map_err(|error| format!("source revision file cannot be normalized: {error}"))?;
        if canonical != path {
            return Ok(false);
        }
        let bytes = (self.file_reader)(&path).inspect_err(|_| self.lose_incremental_trust())?;
        manifest.insert(
            relative_path.to_path_buf(),
            SourceEntryDigest {
                kind: 2,
                digest: Sha256::digest(bytes).into(),
            },
        );
        Ok(true)
    }

    fn lose_incremental_trust(&self) {
        self.machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
    }

    fn reconcile(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        for _ in 0..3 {
            let trust_loss_epoch = {
                let mut machine = self
                    .machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let trust_loss_epoch = machine.trust_loss_epoch();
                machine.begin_reconcile();
                trust_loss_epoch
            };
            let manifest = (self.scanner)(&self.source_root, &|| {
                cancellation.is_cancelled() || deadline.remaining().is_zero()
            })
            .inspect_err(|_| {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
            })?;
            let fence_outcome = self.fence.flush(deadline, cancellation).inspect_err(|_| {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
            })?;
            match fence_outcome {
                FenceOutcome::Proven { changed_paths } if !changed_paths.is_empty() => continue,
                FenceOutcome::TrustLost(reason) => {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(reason);
                    continue;
                }
                FenceOutcome::Proven { .. } => {}
            }
            let digest = digest_source_manifest(&manifest)?;
            if self.publish_revision(&manifest, digest, trust_loss_epoch)? {
                return Ok(());
            }
        }
        Err("source revision did not stabilize during reconcile".to_string())
    }

    fn publish_revision(
        &self,
        manifest: &SourceManifest,
        digest: String,
        trust_loss_epoch: u64,
    ) -> Result<bool, String> {
        let Some(revision) = self
            .machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .finish_reconcile_if_trust_unchanged(digest, trust_loss_epoch)?
        else {
            return Ok(false);
        };
        persist_revision_record(
            &self.record_path,
            &self.workspace_root,
            &self.source_root,
            &revision,
        )
        .inspect_err(|_| {
            self.machine
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
        })?;
        *self
            .manifest
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(manifest.clone());
        Ok(true)
    }
}

fn update_identity_path(identity: &mut Sha256, path: &Path) {
    let bytes = path.as_os_str().as_encoded_bytes();
    identity.update((bytes.len() as u64).to_le_bytes());
    identity.update(bytes);
}

fn load_revision_record(
    path: &Path,
    workspace_root: &Path,
    source_root: &Path,
) -> Option<SourceRevision> {
    let record: SourceRevisionRecord = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (record.schema_version == REVISION_RECORD_SCHEMA_VERSION
        && Path::new(&record.workspace_root) == workspace_root
        && Path::new(&record.source_root) == source_root)
        .then_some(record.revision)
}

fn persist_revision_record(
    path: &Path,
    workspace_root: &Path,
    source_root: &Path,
    revision: &SourceRevision,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "source revision record has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("source revision record directory cannot be created: {error}"))?;
    let record = SourceRevisionRecord {
        schema_version: REVISION_RECORD_SCHEMA_VERSION,
        workspace_root: workspace_root.to_string_lossy().into_owned(),
        source_root: source_root.to_string_lossy().into_owned(),
        revision: revision.clone(),
    };
    let bytes = serde_json::to_vec(&record)
        .map_err(|error| format!("source revision record cannot be serialized: {error}"))?;
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    fs::write(&temporary, bytes)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| format!("source revision record cannot be published: {error}"))
}

#[cfg(test)]
fn scan_source_digest(
    source_root: &Path,
    should_stop: &(dyn Fn() -> bool + Sync),
) -> Result<String, String> {
    let manifest = scan_source_manifest(source_root, should_stop)?;
    digest_source_manifest(&manifest)
}

fn scan_source_manifest(
    source_root: &Path,
    should_stop: &(dyn Fn() -> bool + Sync),
) -> Result<SourceManifest, String> {
    if should_stop() {
        return Err("source revision reconcile cancelled".to_string());
    }
    let mut manifest = BTreeMap::new();
    scan_directory(source_root, source_root, 0, should_stop, &mut manifest)?;
    Ok(manifest)
}

fn digest_source_manifest(manifest: &SourceManifest) -> Result<String, String> {
    let mut corpus = Sha256::new();
    corpus.update(b"unica-source-sha256-v1\0");
    for (relative, entry) in manifest {
        let path = relative.as_os_str().as_encoded_bytes();
        corpus.update([entry.kind]);
        corpus.update((path.len() as u64).to_le_bytes());
        corpus.update(path);
        corpus.update(entry.digest);
    }
    Ok(format!("{:x}", corpus.finalize()))
}

fn read_source_file(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("source revision file cannot be read: {error}"))
}

fn scan_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    should_stop: &(dyn Fn() -> bool + Sync),
    manifest: &mut SourceManifest,
) -> Result<(), String> {
    if should_stop() {
        return Err("source revision reconcile cancelled".to_string());
    }
    if depth > MAX_SOURCE_DEPTH {
        return Err("source revision corpus exceeds maximum depth".to_string());
    }
    let mut children = fs::read_dir(directory)
        .map_err(|error| format!("source revision directory cannot be read: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("source revision entry cannot be read: {error}"))?;
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        if should_stop() {
            return Err("source revision reconcile cancelled".to_string());
        }
        let file_type = child
            .file_type()
            .map_err(|error| format!("source revision entry type cannot be read: {error}"))?;
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "source revision entry escaped its root".to_string())?
            .to_path_buf();
        if file_type.is_symlink() {
            if is_source_file(&path) {
                return Err(format!(
                    "source revision corpus contains an indexed symbolic link: {}",
                    relative.display()
                ));
            }
            continue;
        }
        if file_type.is_dir() {
            if child.file_name() == OsStr::new(GENERATED_DIR_NAME) {
                continue;
            }
            manifest.insert(
                relative,
                SourceEntryDigest {
                    kind: 1,
                    digest: [0; 32],
                },
            );
            scan_directory(root, &path, depth + 1, should_stop, manifest)?;
        } else if file_type.is_file() && is_source_file(&path) {
            let bytes = read_source_file(&path)?;
            let digest: [u8; 32] = Sha256::digest(bytes).into();
            manifest.insert(relative, SourceEntryDigest { kind: 2, digest });
        }
    }
    Ok(())
}

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "bsl" | "xml" | "mdo" | "form" | "rights" | "xdto" | "command" | "yaml" | "yml"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::platform::testing::{
        create_dir_symlink_for_test, create_file_symlink_for_test,
    };
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use tempfile::tempdir;

    struct UnsupportedFence;

    impl SourceRevisionFence for UnsupportedFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::Unsupported
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            Ok(FenceOutcome::TrustLost(
                SourceRevisionTrustLoss::UnsupportedFence,
            ))
        }
    }

    struct ProvenCleanFence;

    impl SourceRevisionFence for ProvenCleanFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::ProvenFast
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            Ok(FenceOutcome::Proven {
                changed_paths: Vec::new(),
            })
        }
    }

    struct FailOnceFence {
        calls: AtomicUsize,
    }

    impl SourceRevisionFence for FailOnceFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::ProvenFast
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
                Err("synthetic fence failure".to_string())
            } else {
                Ok(FenceOutcome::Proven {
                    changed_paths: Vec::new(),
                })
            }
        }
    }

    struct ScriptedFence {
        outcomes: Mutex<VecDeque<FenceOutcome>>,
    }

    impl SourceRevisionFence for ScriptedFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::ProvenFast
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            Ok(self
                .outcomes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front()
                .unwrap_or(FenceOutcome::Proven {
                    changed_paths: Vec::new(),
                }))
        }
    }

    #[test]
    fn unsupported_fence_never_promotes_repeated_scans_to_trusted() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let full_scans = Arc::new(AtomicUsize::new(0));
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            record_path: workspace.path().join("revision.json"),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            operation: Mutex::new(()),
            fence: Arc::new(UnsupportedFence),
            scanner: Arc::new({
                let full_scans = Arc::clone(&full_scans);
                move |root, should_stop| {
                    full_scans.fetch_add(1, Ordering::AcqRel);
                    scan_source_manifest(root, should_stop)
                }
            }),
            file_reader: Arc::new(read_source_file),
        };

        let error = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .expect_err("an unsupported fence cannot prove a trusted revision");

        assert!(error.contains("unsupported"), "{error}");
        assert_eq!(full_scans.load(Ordering::Acquire), 0);
    }

    #[test]
    fn revision_record_identity_includes_workspace_and_source_roots() {
        let sandbox = tempdir().unwrap();
        let workspace_a = sandbox.path().join("workspace-a");
        let workspace_b = sandbox.path().join("workspace-b");
        let source_root = sandbox.path().join("shared-source");
        let cache_root = sandbox.path().join("shared-cache");
        for path in [&workspace_a, &workspace_b, &source_root, &cache_root] {
            fs::create_dir_all(path).unwrap();
        }
        let context = |workspace_root: &Path| WorkspaceContext {
            cwd: workspace_root.to_path_buf(),
            workspace_root: workspace_root.to_path_buf(),
            cache_root: cache_root.clone(),
            workspace_epoch: 0,
        };

        let first = SourceRevisionService::new(&context(&workspace_a), &source_root).unwrap();
        let second = SourceRevisionService::new(&context(&workspace_b), &source_root).unwrap();

        assert_ne!(
            first.record_path, second.record_path,
            "a shared cache must not alias revision records from different workspaces"
        );
    }

    #[test]
    fn failed_revision_record_publication_does_not_leave_a_trusted_snapshot() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let record_parent = workspace.path().join("record-parent");
        fs::write(&record_parent, "not a directory").unwrap();
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            record_path: record_parent.join("revision.json"),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            operation: Mutex::new(()),
            fence: Arc::new(ProvenCleanFence),
            scanner: Arc::new(scan_source_manifest),
            file_reader: Arc::new(read_source_file),
        };
        let snapshot = || {
            service.snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
        };

        let first_error = snapshot().expect_err("publishing the revision must fail");
        assert!(first_error.contains("cannot be created"), "{first_error}");
        let second_error =
            snapshot().expect_err("a revision that was not published must remain untrusted");
        assert!(second_error.contains("cannot be created"), "{second_error}");
    }

    #[test]
    fn failed_fence_does_not_leave_the_previous_revision_trusted() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        let module = source_root.join("Module.bsl");
        fs::write(&module, "Процедура A()\n").unwrap();
        let initial_digest = scan_source_digest(&source_root, &|| false).unwrap();
        let mut machine = SourceRevisionMachine::default();
        let initial_revision = machine.finish_reconcile(initial_digest).unwrap();
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            record_path: workspace.path().join("revision.json"),
            machine: Mutex::new(machine),
            manifest: Mutex::new(None),
            operation: Mutex::new(()),
            fence: Arc::new(FailOnceFence {
                calls: AtomicUsize::new(0),
            }),
            scanner: Arc::new(scan_source_manifest),
            file_reader: Arc::new(read_source_file),
        };

        let first_error = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .expect_err("the failed fence must reject the snapshot");
        assert_eq!(first_error, "synthetic fence failure");
        fs::write(&module, "Процедура B()\n").unwrap();

        let recovered = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(recovered.generation > initial_revision.generation);
        assert_ne!(recovered.digest, initial_revision.digest);
    }

    #[test]
    fn external_file_edit_does_not_repeat_the_full_corpus_scan() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        fs::write(source_root.join("Unchanged.bsl"), "Процедура B()\n").unwrap();
        let full_scans = Arc::new(AtomicUsize::new(0));
        let incremental_reads = Arc::new(AtomicUsize::new(0));
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            record_path: workspace.path().join("revision.json"),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            operation: Mutex::new(()),
            fence: Arc::new(ScriptedFence {
                outcomes: Mutex::new(VecDeque::from([
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: vec![PathBuf::from("Module.bsl")],
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: vec![PathBuf::from("Module.bsl")],
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                ])),
            }),
            scanner: Arc::new({
                let full_scans = Arc::clone(&full_scans);
                move |root, should_stop| {
                    full_scans.fetch_add(1, Ordering::AcqRel);
                    scan_source_manifest(root, should_stop)
                }
            }),
            file_reader: Arc::new({
                let incremental_reads = Arc::clone(&incremental_reads);
                move |path| {
                    incremental_reads.fetch_add(1, Ordering::AcqRel);
                    read_source_file(path)
                }
            }),
        };
        let cancellation = CancellationToken::new();
        let snapshot = || {
            service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &cancellation,
                )
                .unwrap()
        };

        let scans_before = full_scans.load(Ordering::Acquire);
        let first = snapshot();
        let scans_after_cold = full_scans.load(Ordering::Acquire);
        assert!(scans_after_cold > scans_before);
        let warm = snapshot();
        assert_eq!(warm, first);
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            scans_after_cold,
            "a warm trusted snapshot must not walk the source tree"
        );

        fs::write(source_root.join("Module.bsl"), "Процедура B()\n").unwrap();
        let changed = snapshot();
        assert!(changed.generation > first.generation);
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            scans_after_cold,
            "a precise external file event must not reread the whole corpus"
        );
        assert_eq!(
            incremental_reads.load(Ordering::Acquire),
            1,
            "only the changed file must be read"
        );

        fs::remove_file(source_root.join("Module.bsl")).unwrap();
        let removed = snapshot();
        assert!(removed.generation > changed.generation);
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            scans_after_cold,
            "a precise file removal must update the manifest without a full scan"
        );
        assert_eq!(
            incremental_reads.load(Ordering::Acquire),
            1,
            "a removed file must not be read"
        );
    }

    #[test]
    fn concurrent_mark_dirty_rejects_incremental_publication_and_reconciles_again() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        let module = source_root.join("Module.bsl");
        fs::write(&module, "Процедура A()\n").unwrap();
        let full_scans = Arc::new(AtomicUsize::new(0));
        let (read_started_tx, read_started_rx) = mpsc::channel();
        let (read_release_tx, read_release_rx) = mpsc::channel();
        let read_release_rx = Mutex::new(read_release_rx);
        let service = Arc::new(SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            record_path: workspace.path().join("revision.json"),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            operation: Mutex::new(()),
            fence: Arc::new(ScriptedFence {
                outcomes: Mutex::new(VecDeque::from([
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: vec![PathBuf::from("Module.bsl")],
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                ])),
            }),
            scanner: Arc::new({
                let full_scans = Arc::clone(&full_scans);
                move |root, should_stop| {
                    full_scans.fetch_add(1, Ordering::AcqRel);
                    scan_source_manifest(root, should_stop)
                }
            }),
            file_reader: Arc::new(move |path| {
                read_started_tx.send(()).unwrap();
                read_release_rx.lock().unwrap().recv().unwrap();
                read_source_file(path)
            }),
        });
        let initial = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(full_scans.load(Ordering::Acquire), 1);
        fs::write(&module, "Процедура B()\n").unwrap();

        let worker_service = Arc::clone(&service);
        let worker = thread::spawn(move || {
            worker_service.snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
        });
        read_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        service.mark_dirty();
        read_release_tx.send(()).unwrap();
        let changed = worker.join().unwrap().unwrap();

        assert!(changed.generation > initial.generation);
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            2,
            "a watcher gap during incremental update must force a full reconcile"
        );
    }

    #[test]
    fn corpus_digest_tracks_content_and_path_but_ignores_generated_cache() {
        let first = tempdir().unwrap();
        let second = tempdir().unwrap();
        for root in [first.path(), second.path()] {
            fs::create_dir_all(root.join("CommonModules/Sales/Ext")).unwrap();
            fs::write(
                root.join("CommonModules/Sales/Ext/Module.bsl"),
                "Процедура A()\n",
            )
            .unwrap();
        }
        let baseline = scan_source_digest(first.path(), &|| false).unwrap();
        assert_eq!(
            baseline,
            scan_source_digest(second.path(), &|| false).unwrap()
        );

        fs::create_dir_all(first.path().join(".build")).unwrap();
        fs::write(first.path().join(".build/cache.db"), "private").unwrap();
        assert_eq!(
            baseline,
            scan_source_digest(first.path(), &|| false).unwrap()
        );

        fs::write(
            first.path().join("CommonModules/Sales/Ext/Module.bsl"),
            "Процедура B()\n",
        )
        .unwrap();
        assert_ne!(
            baseline,
            scan_source_digest(first.path(), &|| false).unwrap()
        );
    }

    #[test]
    fn corpus_digest_accepts_a_non_utf8_relative_path() {
        let Some(path) =
            crate::infrastructure::platform::testing::non_utf8_relative_path_for_test()
        else {
            return;
        };
        let mut manifest = SourceManifest::new();
        manifest.insert(
            path,
            SourceEntryDigest {
                kind: 2,
                digest: [7; 32],
            },
        );

        assert!(
            digest_source_manifest(&manifest).is_ok(),
            "a valid filesystem name must not disable source revisions"
        );
    }

    #[test]
    fn corpus_digest_ignores_a_symlink_outside_the_rlm_corpus() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let baseline = scan_source_digest(&source_root, &|| false).unwrap();
        let outside = workspace.path().join("README.txt");
        fs::write(&outside, "not indexed by RLM").unwrap();
        let Some(link) = create_file_symlink_for_test(&outside, source_root.join("README.txt"))
        else {
            return;
        };
        link.unwrap();

        assert_eq!(
            baseline,
            scan_source_digest(&source_root, &|| false).unwrap()
        );
    }

    #[test]
    fn corpus_digest_does_not_follow_a_symlinked_directory() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        let outside = workspace.path().join("outside");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("External.bsl"), "Процедура A()\n").unwrap();
        let baseline = scan_source_digest(&source_root, &|| false).unwrap();
        let Some(link) = create_dir_symlink_for_test(&outside, source_root.join("vendor")) else {
            return;
        };
        link.unwrap();

        assert_eq!(
            baseline,
            scan_source_digest(&source_root, &|| false).unwrap()
        );
    }

    #[test]
    fn corpus_digest_rejects_an_indexed_symlink_with_its_relative_path() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        let outside = workspace.path().join("Module.bsl");
        fs::write(&outside, "Процедура A()\n").unwrap();
        let Some(link) = create_file_symlink_for_test(&outside, source_root.join("Linked.bsl"))
        else {
            return;
        };
        link.unwrap();

        let error = scan_source_digest(&source_root, &|| false)
            .expect_err("an RLM-indexed symlink cannot produce a trusted revision");

        assert!(error.contains("Linked.bsl"), "{error}");
    }

    #[test]
    fn corpus_digest_tracks_edt_metadata() {
        let source_root = tempdir().unwrap();
        let metadata = source_root.path().join("Catalog.mdo");
        fs::write(&metadata, "<mdclass:Catalog uuid=\"first\"/>").unwrap();
        let baseline = scan_source_digest(source_root.path(), &|| false).unwrap();

        fs::write(&metadata, "<mdclass:Catalog uuid=\"second\"/>").unwrap();

        assert_ne!(
            baseline,
            scan_source_digest(source_root.path(), &|| false).unwrap()
        );
    }
}

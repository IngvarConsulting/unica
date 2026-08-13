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
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const GENERATED_DIR_NAME: &str = ".build";
const MAX_SOURCE_DEPTH: usize = 64;
const REVISION_RECORD_SCHEMA_VERSION: u32 = 1;

type SourceDigestScanner =
    dyn Fn(&Path, &(dyn Fn() -> bool + Sync)) -> Result<String, String> + Send + Sync;

pub(crate) struct SourceRevisionService {
    source_root: PathBuf,
    record_path: PathBuf,
    machine: Mutex<SourceRevisionMachine>,
    fence: Arc<dyn SourceRevisionFence>,
    scanner: Arc<SourceDigestScanner>,
}

impl fmt::Debug for SourceRevisionService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceRevisionService")
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
    source_root: String,
    revision: SourceRevision,
}

impl SourceRevisionService {
    pub(crate) fn new(context: &WorkspaceContext, source_root: &Path) -> Result<Self, String> {
        let source_root = fs::canonicalize(source_root)
            .map_err(|error| format!("source revision root cannot be normalized: {error}"))?;
        let mut identity = Sha256::new();
        identity.update(source_root.as_os_str().as_encoded_bytes());
        let identity = format!("{:x}", identity.finalize());
        let record_path = context
            .cache_root
            .join("source-revisions")
            .join(format!("{identity}.json"));
        let machine = load_revision_record(&record_path, &source_root)
            .and_then(|revision| SourceRevisionMachine::from_revision(revision).ok())
            .unwrap_or_default();
        let fence = platform_fence(&source_root, &context.cache_root)?;
        Ok(Self {
            source_root,
            record_path,
            machine: Mutex::new(machine),
            fence,
            scanner: Arc::new(scan_source_digest),
        })
    }

    pub(crate) fn snapshot(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<SourceRevision, String> {
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
            FenceOutcome::Proven { dirty } => {
                dirty
                    || !matches!(
                        self.machine
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .state(),
                        SourceRevisionState::Trusted(_)
                    )
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

    fn reconcile(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        for _ in 0..3 {
            {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .begin_reconcile();
            }
            let digest = (self.scanner)(&self.source_root, &|| {
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
                FenceOutcome::Proven { dirty: true } => continue,
                FenceOutcome::TrustLost(reason) => {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(reason);
                    continue;
                }
                FenceOutcome::Proven { dirty: false } => {}
            }
            let revision = self
                .machine
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .finish_reconcile(digest)?;
            persist_revision_record(&self.record_path, &self.source_root, &revision).inspect_err(
                |_| {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
                },
            )?;
            return Ok(());
        }
        Err("source revision did not stabilize during reconcile".to_string())
    }
}

fn load_revision_record(path: &Path, source_root: &Path) -> Option<SourceRevision> {
    let record: SourceRevisionRecord = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (record.schema_version == REVISION_RECORD_SCHEMA_VERSION
        && Path::new(&record.source_root) == source_root)
        .then_some(record.revision)
}

fn persist_revision_record(
    path: &Path,
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

pub(crate) fn scan_source_digest(
    source_root: &Path,
    should_stop: &(dyn Fn() -> bool + Sync),
) -> Result<String, String> {
    if should_stop() {
        return Err("source revision reconcile cancelled".to_string());
    }
    let mut entries = Vec::new();
    scan_directory(source_root, source_root, 0, should_stop, &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut corpus = Sha256::new();
    corpus.update(b"unica-source-sha256-v1\0");
    for (relative, kind, digest) in entries {
        let path = relative
            .to_str()
            .ok_or_else(|| "source revision contains a non-UTF-8 relative path".to_string())?;
        corpus.update([kind]);
        corpus.update((path.len() as u64).to_le_bytes());
        corpus.update(path.as_bytes());
        corpus.update(digest);
    }
    Ok(format!("{:x}", corpus.finalize()))
}

fn scan_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    should_stop: &(dyn Fn() -> bool + Sync),
    entries: &mut Vec<(PathBuf, u8, [u8; 32])>,
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
        if file_type.is_symlink() {
            return Err("source revision corpus contains a symbolic link".to_string());
        }
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "source revision entry escaped its root".to_string())?
            .to_path_buf();
        if file_type.is_dir() {
            if child.file_name() == OsStr::new(GENERATED_DIR_NAME) {
                continue;
            }
            entries.push((relative, 1, [0; 32]));
            scan_directory(root, &path, depth + 1, should_stop, entries)?;
        } else if file_type.is_file() && is_source_file(&path) {
            let bytes = fs::read(&path)
                .map_err(|error| format!("source revision file cannot be read: {error}"))?;
            let digest: [u8; 32] = Sha256::digest(bytes).into();
            entries.push((relative, 2, digest));
        }
    }
    Ok(())
}

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| matches!(extension, "bsl" | "xml" | "yaml" | "yml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
            Ok(FenceOutcome::Proven { dirty: false })
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
                Ok(FenceOutcome::Proven { dirty: false })
            }
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
            source_root: fs::canonicalize(&source_root).unwrap(),
            record_path: workspace.path().join("revision.json"),
            machine: Mutex::new(SourceRevisionMachine::default()),
            fence: Arc::new(UnsupportedFence),
            scanner: Arc::new({
                let full_scans = Arc::clone(&full_scans);
                move |root, should_stop| {
                    full_scans.fetch_add(1, Ordering::AcqRel);
                    scan_source_digest(root, should_stop)
                }
            }),
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
    fn failed_revision_record_publication_does_not_leave_a_trusted_snapshot() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let record_parent = workspace.path().join("record-parent");
        fs::write(&record_parent, "not a directory").unwrap();
        let service = SourceRevisionService {
            source_root: fs::canonicalize(&source_root).unwrap(),
            record_path: record_parent.join("revision.json"),
            machine: Mutex::new(SourceRevisionMachine::default()),
            fence: Arc::new(ProvenCleanFence),
            scanner: Arc::new(scan_source_digest),
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
            source_root: fs::canonicalize(&source_root).unwrap(),
            record_path: workspace.path().join("revision.json"),
            machine: Mutex::new(machine),
            fence: Arc::new(FailOnceFence {
                calls: AtomicUsize::new(0),
            }),
            scanner: Arc::new(scan_source_digest),
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
    fn warm_snapshot_uses_fences_and_external_change_reconciles_once() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        let cache_root = workspace.path().join(".cache");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let context = WorkspaceContext {
            cwd: workspace.path().to_path_buf(),
            workspace_root: workspace.path().to_path_buf(),
            cache_root,
            workspace_epoch: 0,
        };
        let mut service = SourceRevisionService::new(&context, &source_root).unwrap();
        if service.fence.capability() != FenceCapability::ProvenFast {
            return;
        }
        let full_scans = Arc::new(AtomicUsize::new(0));
        service.scanner = Arc::new({
            let full_scans = Arc::clone(&full_scans);
            move |root, should_stop| {
                full_scans.fetch_add(1, Ordering::AcqRel);
                scan_source_digest(root, should_stop)
            }
        });
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
        assert!(full_scans.load(Ordering::Acquire) > scans_after_cold);
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
}

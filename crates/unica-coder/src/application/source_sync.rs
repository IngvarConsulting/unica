use super::{ToolHandler, ToolSpec};
use crate::domain::source_sync::{
    BuildStepMode, FileFingerprint, SourceManifest, SourceTargetKind, SourceTargetRecord,
    SourceTargetScope, TargetId,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::internal_adapters::parse_runtime_build_report;
use crate::infrastructure::internal_adapters::RuntimeBuildReport;
use crate::infrastructure::shadow_dump::{
    audit_stale_shadow_dumps, recover_stale_shadow_dumps, validate_pinned_shadow_config,
    ShadowDumpPreparation, ShadowPlatformSeeds,
};
use crate::infrastructure::source_sync::{
    BaselineReceipt, DirtyTargetSnapshot, LifecycleChildLease, LifecycleLockGuard,
    PlatformCdfiPreimage, PublicationError, PublicationOutcome, SourceSyncRepository,
    SynchronizationConflict,
};
use crate::infrastructure::AdapterOutcome;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const BUILD_SNAPSHOT_PREFIX: &str = "build-snapshot-";
const BUILD_SNAPSHOT_CREATING_PREFIX: &str = "build-snapshot-creating-";
const BUILD_SNAPSHOT_OWNERSHIP_FILE: &str = "ownership.json";
const BUILD_SNAPSHOT_OWNERSHIP_TEMP_FILE: &str = ".ownership.json.tmp";
const BUILD_SNAPSHOT_OWNERSHIP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildSnapshotOwnership {
    schema_version: u32,
    transaction_id: String,
    primary_sha256: String,
    local_sha256: Option<String>,
}

pub(crate) struct BuildReconciliation {
    pub synchronized: Vec<(TargetId, SourceManifest)>,
    pub details: Value,
}

pub(crate) struct DumpReconciliation {
    pub publish: Vec<(TargetId, SourceManifest)>,
    pub synchronized: Vec<(TargetId, SourceManifest)>,
    pub details: Value,
    pub blocked: bool,
}

pub(crate) struct MutationSession {
    repository: SourceSyncRepository,
    _lock: LifecycleLockGuard,
    target: crate::domain::source_sync::SourceTarget,
    handler_args: Map<String, Value>,
    preimage: SourceManifest,
    baseline: BaselineReceipt,
}

pub(crate) enum MutationPreparation {
    None,
    Ready(Box<MutationSession>),
    Blocked { outcome: Box<AdapterOutcome> },
}

pub(crate) struct BuildSession {
    repository: SourceSyncRepository,
    _lock: LifecycleLockGuard,
    generation: u64,
    requested: Vec<SourceTargetRecord>,
    config_snapshot: RuntimeConfigSnapshot,
    pinned_config: PinnedBuildConfig,
    handler_args: Map<String, Value>,
}

#[derive(Clone, PartialEq, Eq)]
struct RuntimeConfigSnapshot {
    files: BTreeMap<PathBuf, FileFingerprint>,
}

struct PinnedBuildConfig {
    directory: PathBuf,
    primary_path: PathBuf,
    original_config_dir: PathBuf,
    snapshot: RuntimeConfigSnapshot,
    cleaned: Cell<bool>,
}

struct PreparedPinnedBuildBytes {
    primary: Vec<u8>,
    local: Option<Vec<u8>>,
    original_config_dir: PathBuf,
}

pub(crate) enum BuildPreparation {
    None,
    Ready(Box<BuildSession>),
    Blocked {
        outcome: Box<AdapterOutcome>,
        details: Value,
    },
}

pub(crate) enum RuntimePreviewPreparation {
    None,
    Ready(Value),
    Blocked {
        outcome: Box<AdapterOutcome>,
        details: Value,
    },
}

pub(crate) enum DumpPreparation {
    None,
    Passthrough {
        _lock: LifecycleLockGuard,
    },
    Ready(Box<DumpSession>),
    Blocked {
        outcome: Box<AdapterOutcome>,
        details: Value,
    },
}

pub(crate) struct DumpSession {
    repository: SourceSyncRepository,
    lock: LifecycleLockGuard,
    generation: u64,
    requested: Vec<SourceTargetRecord>,
    force: bool,
    config_snapshot: RuntimeConfigSnapshot,
    shadow_config_snapshot: RuntimeConfigSnapshot,
    cdfi_preimage: Box<PlatformCdfiPreimage>,
    shadow: ShadowDumpPreparation,
}

pub(crate) enum LegacyPreparation {
    None,
    Allowed(LifecycleLockGuard),
    Blocked {
        outcome: Box<AdapterOutcome>,
        details: Value,
    },
}

/// Keep one inheritable duplicate of the active lifecycle lock open solely
/// across the external runtime invocation. The child then retains the same
/// lease after an unexpected parent death, preventing another Unica process
/// from recovering or starting a competing source-sync transaction too early.
pub(crate) fn runtime_child_lease(
    build: &BuildPreparation,
    dump: &DumpPreparation,
    legacy: &LegacyPreparation,
) -> Result<Option<LifecycleChildLease>, String> {
    match build {
        BuildPreparation::Ready(session) => session._lock.child_lease().map(Some),
        BuildPreparation::None | BuildPreparation::Blocked { .. } => match dump {
            DumpPreparation::Ready(session) => session.lock.child_lease().map(Some),
            DumpPreparation::Passthrough { _lock } => _lock.child_lease().map(Some),
            DumpPreparation::None | DumpPreparation::Blocked { .. } => match legacy {
                LegacyPreparation::Allowed(lock) => lock.child_lease().map(Some),
                LegacyPreparation::None | LegacyPreparation::Blocked { .. } => Ok(None),
            },
        },
    }
}

pub(crate) fn prepare_mutation(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> Result<MutationPreparation, String> {
    if !is_tracked_mutation(spec) {
        return Ok(MutationPreparation::None);
    }
    // Dry-run stays delegated to the native adapter. In particular, the
    // documented MCP examples intentionally use placeholder paths, so
    // source-sync must neither create durable authority state nor require a
    // resolvable runtime source-set before that adapter produces its preview.
    if dry_run {
        return Ok(MutationPreparation::None);
    }

    // Apply resolves and pins the target only while holding the lifecycle
    // lock. Waiting for a prior operation can therefore never leave us with a
    // stale path/identity paired with a newer baseline.
    // Reject an invalid target before creating the durable authority or lock.
    // It is resolved again while locked below, which retains the TOCTOU
    // protection for every accepted mutation.
    if let Some(outcome) = native_mutation_precheck_failure(spec, args, context) {
        return Ok(MutationPreparation::Blocked {
            outcome: Box::new(outcome),
        });
    }
    let preliminary_target = match resolve_validated_mutation_target(spec, args, context) {
        Ok(target) => target,
        Err(error) if is_missing_platform_root(&error) => {
            // Existing source-only workspaces may declare a source-set before
            // their first Configuration.xml exists. Keep the native editor's
            // established behavior in that untracked state. Once a target was
            // recorded, however, the same missing root is a safety failure and
            // must remain fail-closed.
            let repository = SourceSyncRepository::new(context)?;
            if repository.load_state()?.targets.is_empty() {
                return Ok(MutationPreparation::None);
            }
            return Err(format!(
                "source-sync has tracked targets and cannot bypass the missing platform root: {error}"
            ));
        }
        Err(error) => return Err(error),
    };
    let repository = SourceSyncRepository::new(context)?;
    let lock = repository.acquire_lifecycle_lock()?;
    repository.recover_pending_publications()?;
    recover_stale_build_snapshots(repository.transaction_root())?;
    let bound_context = bind_context_to_repository(context, &repository)?;
    if let Some(outcome) = native_mutation_precheck_failure(spec, args, &bound_context) {
        return Ok(MutationPreparation::Blocked {
            outcome: Box::new(outcome),
        });
    }
    let target = resolve_validated_mutation_target(spec, args, &bound_context)?;
    if target != preliminary_target {
        return Err(
            "source-sync mutation target changed while acquiring the lifecycle lock".to_string(),
        );
    }
    let handler_args = pinned_mutation_handler_args(spec, args, &bound_context, &target)?;
    let preimage = repository.capture_manifest(&target)?;
    let baseline = repository.ensure_baseline(&target, &preimage)?;
    Ok(MutationPreparation::Ready(Box::new(MutationSession {
        repository,
        _lock: lock,
        target,
        handler_args,
        preimage,
        baseline,
    })))
}

fn is_missing_platform_root(error: &str) -> bool {
    error
        .strip_prefix("source-sync path is missing: ")
        .is_some_and(|path| path.ends_with("Configuration.xml"))
}

fn resolve_validated_mutation_target(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<crate::domain::source_sync::SourceTarget, String> {
    let target =
        crate::infrastructure::source_sync::resolve_mutation_target(spec.name, args, context)?;
    if target.source_set.is_none() {
        return Err(format!(
            "{} target must belong to exactly one configured platform-XML configuration source-set; refusing to create an unresolvable source-sync record",
            spec.name
        ));
    }
    Ok(target)
}

fn native_mutation_precheck_failure(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    let error = match spec.name {
        "unica.code.patch" => {
            crate::infrastructure::native_operations::code::resolve_module_target(args, context)
                .err()
        }
        "unica.meta.edit" => {
            let raw = ["objectPath", "ObjectPath", "path", "Path"]
                .iter()
                .find_map(|key| args.get(*key).and_then(Value::as_str))?;
            crate::infrastructure::native_operations::meta::resolve_meta_edit_object_path(
                std::path::Path::new(raw),
                &context.cwd,
            )
            .err()
        }
        _ => None,
    }?;
    let mut outcome = AdapterOutcome::ok(match spec.name {
        "unica.code.patch" => "code patch rejected without changing files",
        "unica.meta.edit" => "unica.meta.edit failed in native metadata editor",
        _ => "native mutation target rejected without changing files",
    });
    outcome.ok = false;
    outcome.errors.push(error.clone());
    if spec.name == "unica.meta.edit" {
        outcome.stderr = Some(format!("{error}\n"));
    }
    Some(outcome)
}

impl MutationSession {
    pub(crate) fn handler_args(&self) -> &Map<String, Value> {
        &self.handler_args
    }

    pub(crate) fn finish(&self) -> Result<Option<Value>, String> {
        let postimage = self.repository.capture_manifest(&self.target)?;
        if postimage == self.preimage {
            self.repository
                .discard_clean_baseline(&self.baseline, &self.preimage)?;
            return Ok(None);
        }
        let recorded = self
            .repository
            .record_mutation(&self.target, &self.preimage, &postimage)?;
        Ok(Some(json!({
            "affectedTargets": [affected_target_details(&recorded.target, &self.preimage)]
        })))
    }
}

fn pinned_mutation_handler_args(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    target: &crate::domain::source_sync::SourceTarget,
) -> Result<Map<String, Value>, String> {
    let mut pinned = args.clone();
    match spec.name {
        "unica.code.patch" => {
            let source_root = context
                .workspace_root
                .join(target.source_root.as_str())
                .canonicalize()
                .map_err(|error| {
                    format!(
                        "failed to pin code.patch source root {}: {error}",
                        target.source_root.as_str()
                    )
                })?;
            pinned.remove("sourceSet");
            pinned.remove("sourceDir");
            pinned.insert(
                "sourceDir".to_string(),
                json!(source_root.display().to_string()),
            );
        }
        "unica.meta.edit" => {
            let SourceTargetScope::MetadataOwner {
                descriptor_path, ..
            } = &target.scope
            else {
                return Err("meta.edit source-sync target is not a metadata owner".to_string());
            };
            let descriptor = context
                .workspace_root
                .join(descriptor_path.as_str())
                .canonicalize()
                .map_err(|error| {
                    format!(
                        "failed to pin meta.edit object {}: {error}",
                        descriptor_path.as_str()
                    )
                })?;
            for key in ["objectPath", "ObjectPath", "path", "Path"] {
                if pinned.contains_key(key) {
                    pinned.insert(key.to_string(), json!(descriptor.display().to_string()));
                }
            }
        }
        _ => return Err(format!("{} is not a tracked mutation", spec.name)),
    }
    Ok(pinned)
}

pub(crate) fn prepare_build(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> Result<BuildPreparation, String> {
    if dry_run || !is_typed_runtime_operation(spec, args, "build") {
        return Ok(BuildPreparation::None);
    }
    let prepared = (|| -> Result<BuildPreparation, String> {
        let repository = SourceSyncRepository::new(context)?;
        let lock = repository.acquire_lifecycle_lock()?;
        repository.recover_pending_publications()?;
        recover_stale_build_snapshots(repository.transaction_root())?;
        let bound_context = bind_context_to_repository(context, &repository)?;
        let requested_source_set = args.get("sourceSet").and_then(Value::as_str);
        let DirtyTargetSnapshot {
            generation,
            mut targets,
        } = repository.reconcile_all()?;
        if let Some(source_set) = requested_source_set {
            targets.retain(|record| {
                record
                    .target
                    .source_set
                    .as_ref()
                    .is_some_and(|name| name.as_str() == source_set)
            });
        }
        // Clean durable records must not capture an otherwise unrelated
        // custom build. Pinning is required only while this invocation can
        // advance dirty source-sync state.
        if targets.is_empty() {
            return Ok(BuildPreparation::None);
        }

        // Snapshot before topology discovery. The later pin step rereads only
        // these expected bytes, so a config swap cannot redirect the runner
        // between topology validation and private-config creation.
        let config_snapshot = match RuntimeConfigSnapshot::capture(&bound_context) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let conflicted = targets
                    .iter()
                    .map(|record| {
                        terminal_details(record, "runtimeConfigObservationFailed", Some(&error))
                    })
                    .collect::<Vec<_>>();
                let mut outcome = AdapterOutcome::ok(
                    "build blocked before runner execution by runtime config observation",
                );
                outcome.ok = false;
                outcome.errors.push(error);
                return Ok(BuildPreparation::Blocked {
                    outcome: Box::new(outcome),
                    details: json!({
                        "requested": targets.iter().map(target_details).collect::<Vec<_>>(),
                        "processed": [],
                        "skipped": [],
                        "conflicted": conflicted,
                    }),
                });
            }
        };
        let config_error = validate_build_config_identity(args, &bound_context).err();
        let mut conflicts = Vec::new();
        for record in &targets {
            let topology_error = config_error
                .as_deref()
                .map(str::to_string)
                .or_else(|| repository.validate_target_topology(record).err());
            if let Some(error) = topology_error {
                conflicts.push(terminal_details(
                    record,
                    "sourceTopologyChanged",
                    Some(&error),
                ));
            }
        }
        if !conflicts.is_empty() {
            let conflict_ids = conflicts
                .iter()
                .filter_map(|entry| entry.get("targetId").and_then(Value::as_str))
                .collect::<std::collections::BTreeSet<_>>();
            let skipped = targets
                .iter()
                .filter(|record| !conflict_ids.contains(record.target.id.as_str()))
                .map(|record| terminal_details(record, "batchConflict", None))
                .collect::<Vec<_>>();
            let mut outcome = AdapterOutcome::ok(
                "build blocked before runner execution by source-sync topology preflight",
            );
            outcome.ok = false;
            outcome.errors.push(
                "build source topology no longer matches durable source-sync state; runner was not invoked"
                    .to_string(),
            );
            return Ok(BuildPreparation::Blocked {
                outcome: Box::new(outcome),
                details: json!({
                    "requested": targets.iter().map(target_details).collect::<Vec<_>>(),
                    "processed": [],
                    "skipped": skipped,
                    "conflicted": conflicts,
                }),
            });
        }
        let pinned_config = PinnedBuildConfig::prepare(
            &bound_context,
            repository.transaction_root(),
            &config_snapshot,
        )?;
        let mut handler_args = args.clone();
        if let Some(workdir) = handler_args.get_mut("workdir") {
            absolutize_json_path(workdir, &pinned_config.original_config_dir, "workdir")?;
        }
        handler_args.insert(
            "config".to_string(),
            json!(pinned_config.primary_path.display().to_string()),
        );
        Ok(BuildPreparation::Ready(Box::new(BuildSession {
            repository,
            _lock: lock,
            generation,
            requested: targets,
            config_snapshot,
            pinned_config,
            handler_args,
        })))
    })();
    Ok(match prepared {
        Ok(preparation) => preparation,
        Err(error) => build_internal_error_preparation(args, context, error),
    })
}

impl RuntimeConfigSnapshot {
    fn capture(context: &WorkspaceContext) -> Result<Self, String> {
        Self::capture_from_paths([
            context.workspace_root.join("v8project.yaml"),
            context.workspace_root.join("v8project.local.yaml"),
        ])
    }

    fn capture_from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self, String> {
        let files = paths
            .into_iter()
            .map(|path| capture_runtime_config_file(&path).map(|fingerprint| (path, fingerprint)))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(Self { files })
    }
}

fn bind_context_to_repository(
    context: &WorkspaceContext,
    repository: &SourceSyncRepository,
) -> Result<WorkspaceContext, String> {
    let relative_cwd = context
        .cwd
        .strip_prefix(&context.workspace_root)
        .map_err(|_| "workspace cwd is outside the discovered workspace root".to_string())?;
    let relative_cache = context
        .cache_root
        .strip_prefix(&context.workspace_root)
        .ok();
    Ok(WorkspaceContext {
        cwd: repository.workspace_root().join(relative_cwd),
        workspace_root: repository.workspace_root().to_path_buf(),
        cache_root: relative_cache.map_or_else(
            || context.cache_root.clone(),
            |relative| repository.workspace_root().join(relative),
        ),
        workspace_epoch: context.workspace_epoch,
    })
}

impl PinnedBuildConfig {
    fn prepare(
        context: &WorkspaceContext,
        transaction_root: &Path,
        expected: &RuntimeConfigSnapshot,
    ) -> Result<Self, String> {
        let prepared_bytes = prepare_pinned_build_bytes(context, expected)?;
        let PreparedPinnedBuildBytes {
            primary: primary_bytes,
            local: local_bytes,
            original_config_dir,
        } = prepared_bytes;
        let ownership = BuildSnapshotOwnership {
            schema_version: BUILD_SNAPSHOT_OWNERSHIP_SCHEMA_VERSION,
            transaction_id: String::new(),
            primary_sha256: sha256_hex(&primary_bytes),
            local_sha256: local_bytes.as_deref().map(sha256_hex),
        };
        let (transaction_id, creating_directory) =
            create_unique_build_snapshot_directory(transaction_root)?;
        let mut ownership = ownership;
        ownership.transaction_id = transaction_id.hyphenated().to_string();
        let directory = transaction_root.join(format!(
            "{BUILD_SNAPSHOT_PREFIX}{}",
            transaction_id.hyphenated()
        ));
        let primary_path = directory.join("v8project.yaml");
        let local_path = directory.join("v8project.local.yaml");
        let prepared = (|| -> Result<Self, String> {
            write_build_snapshot_ownership(&creating_directory, &ownership)?;
            write_private_snapshot_config(&creating_directory, "v8project.yaml", &primary_bytes)?;
            if let Some(local_bytes) = local_bytes.as_deref() {
                write_private_snapshot_config(
                    &creating_directory,
                    "v8project.local.yaml",
                    local_bytes,
                )?;
            }
            sync_build_snapshot_directory(&creating_directory)?;
            fs::rename(&creating_directory, &directory).map_err(|error| {
                format!(
                    "failed to commit pinned build snapshot {}: {error}",
                    directory.display()
                )
            })?;
            sync_build_snapshot_directory(transaction_root)?;
            let snapshot =
                RuntimeConfigSnapshot::capture_from_paths([primary_path.clone(), local_path])?;
            Ok(Self {
                directory: directory.clone(),
                primary_path,
                original_config_dir,
                snapshot,
                cleaned: Cell::new(false),
            })
        })();
        match prepared {
            Ok(config) => Ok(config),
            Err(error) => {
                let cleanup_path = if directory.exists() {
                    &directory
                } else {
                    &creating_directory
                };
                let cleanup = remove_owned_build_snapshot(cleanup_path).err();
                Err(match cleanup {
                    Some(cleanup) => format!(
                        "{error}; incomplete pinned build snapshot was retained because cleanup could not be proven safe: {cleanup}"
                    ),
                    None => error,
                })
            }
        }
    }

    fn verify_unchanged(&self) -> Result<(), String> {
        let observed =
            RuntimeConfigSnapshot::capture_from_paths(self.snapshot.files.keys().cloned())?;
        if observed == self.snapshot {
            Ok(())
        } else {
            Err("private pinned build config changed while runner was executing".to_string())
        }
    }

    fn cleanup(&self) -> Result<(), String> {
        if self.cleaned.get() {
            return Ok(());
        }
        remove_owned_build_snapshot(&self.directory)?;
        self.cleaned.set(true);
        Ok(())
    }
}

fn prepare_pinned_build_bytes(
    context: &WorkspaceContext,
    expected: &RuntimeConfigSnapshot,
) -> Result<PreparedPinnedBuildBytes, String> {
    let original_primary = context.workspace_root.join("v8project.yaml");
    let original_local = context.workspace_root.join("v8project.local.yaml");
    let original_config_dir = original_primary
        .parent()
        .ok_or_else(|| "v8project.yaml has no parent directory".to_string())?
        .to_path_buf();
    let primary_bytes = read_expected_runtime_config(&original_primary, expected)?
        .ok_or_else(|| "tracked build requires v8project.yaml".to_string())?;
    let mut primary = serde_yaml::from_slice::<YamlValue>(&primary_bytes).map_err(|error| {
        format!(
            "failed to parse runtime config {} for pinned build: {error}",
            original_primary.display()
        )
    })?;
    normalize_pinned_build_yaml(&mut primary, &original_config_dir, false)?;
    let primary_bytes = serialize_pinned_yaml(&primary)?;

    let local_bytes = match read_expected_runtime_config(&original_local, expected)? {
        Some(local_bytes) => {
            let mut local = serde_yaml::from_slice::<YamlValue>(&local_bytes).map_err(|error| {
                format!(
                    "failed to parse runtime overlay {} for pinned build: {error}",
                    original_local.display()
                )
            })?;
            normalize_pinned_build_yaml(&mut local, &original_config_dir, true)?;
            Some(serialize_pinned_yaml(&local)?)
        }
        None => None,
    };

    Ok(PreparedPinnedBuildBytes {
        primary: primary_bytes,
        local: local_bytes,
        original_config_dir,
    })
}

impl Drop for PinnedBuildConfig {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn read_expected_runtime_config(
    path: &Path,
    expected: &RuntimeConfigSnapshot,
) -> Result<Option<Vec<u8>>, String> {
    let expected_fingerprint = expected.files.get(path).ok_or_else(|| {
        format!(
            "runtime config snapshot does not contain {}",
            path.display()
        )
    })?;
    match expected_fingerprint {
        FileFingerprint::Deleted => match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Ok(_) => Err(format!(
                "runtime config {} appeared after its snapshot",
                path.display()
            )),
            Err(error) => Err(format!(
                "failed to verify absent runtime config {}: {error}",
                path.display()
            )),
        },
        FileFingerprint::Present { .. } => {
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                format!(
                    "failed to inspect runtime config {}: {error}",
                    path.display()
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "runtime config {} must remain a regular non-symlink file",
                    path.display()
                ));
            }
            let bytes = fs::read(path).map_err(|error| {
                format!("failed to read runtime config {}: {error}", path.display())
            })?;
            if &FileFingerprint::present(&bytes) != expected_fingerprint {
                return Err(format!(
                    "runtime config {} changed before it could be pinned",
                    path.display()
                ));
            }
            Ok(Some(bytes))
        }
    }
}

fn normalize_pinned_build_yaml(
    value: &mut YamlValue,
    original_config_dir: &Path,
    local: bool,
) -> Result<(), String> {
    let mapping = value
        .as_mapping_mut()
        .ok_or_else(|| "runtime config root must be a YAML mapping".to_string())?;
    if local {
        const ALLOWED_LOCAL_KEYS: &[&str] = &["workPath", "infobase", "tools", "tests", "mcp"];
        for key in mapping.keys() {
            let key = key.as_str().ok_or_else(|| {
                "v8project.local.yaml keys must be strings for a pinned build".to_string()
            })?;
            if !ALLOWED_LOCAL_KEYS.contains(&key) {
                return Err(format!(
                    "v8project.local.yaml key `{key}` is not allowed in the pinned build overlay"
                ));
            }
        }
    }

    absolutize_yaml_path(mapping, "workPath", original_config_dir)?;
    if !local {
        let base_path_key = YamlValue::String("basePath".to_string());
        let base_path = mapping
            .remove(&base_path_key)
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| "pinned build basePath must be a string".to_string())
                    .map(PathBuf::from)
            })
            .transpose()?
            .unwrap_or_default();
        let absolute_base_path = if base_path.as_os_str().is_empty() {
            original_config_dir.to_path_buf()
        } else if base_path.is_absolute() {
            base_path
        } else {
            original_config_dir.join(base_path)
        };
        absolutize_source_set_paths(mapping, &absolute_base_path)?;
    }

    for path in [
        &["tools", "va", "epf_path"][..],
        &["tools", "client_mcp", "extension", "source", "path"][..],
        &["tools", "client_mcp", "extension", "artifact", "path"][..],
        &["tests", "va", "params_path"][..],
    ] {
        absolutize_nested_yaml_path(value, path, original_config_dir)?;
    }
    absolutize_va_profile_paths(value, original_config_dir)?;
    absolutize_infobase_connection(value, original_config_dir)?;
    Ok(())
}

fn absolutize_source_set_paths(
    mapping: &mut serde_yaml::Mapping,
    resolution_base: &Path,
) -> Result<(), String> {
    let source_sets_key = YamlValue::String("source-set".to_string());
    let source_sets = mapping
        .get_mut(&source_sets_key)
        .and_then(YamlValue::as_sequence_mut)
        .ok_or_else(|| "pinned build config requires a source-set sequence".to_string())?;
    for (index, source_set) in source_sets.iter_mut().enumerate() {
        let source_set = source_set
            .as_mapping_mut()
            .ok_or_else(|| format!("pinned build source-set[{index}] must be a YAML mapping"))?;
        let path = source_set
            .get_mut(YamlValue::String("path".to_string()))
            .ok_or_else(|| format!("pinned build source-set[{index}].path is required"))?;
        let raw = path.as_str().ok_or_else(|| {
            format!("pinned build source-set[{index}].path must be a non-empty string")
        })?;
        if raw.is_empty() {
            return Err(format!(
                "pinned build source-set[{index}].path must be a non-empty string"
            ));
        }
        let raw = PathBuf::from(raw);
        let absolute = if raw.is_absolute() {
            raw
        } else {
            resolution_base.join(raw)
        };
        *path = YamlValue::String(absolute.display().to_string());
    }
    Ok(())
}

fn absolutize_yaml_path(
    mapping: &mut serde_yaml::Mapping,
    key: &str,
    workspace_root: &Path,
) -> Result<(), String> {
    let key = YamlValue::String(key.to_string());
    let Some(value) = mapping.get_mut(&key) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let raw = value.as_str().ok_or_else(|| {
        format!(
            "pinned build path `{}` must be a string",
            key.as_str().unwrap()
        )
    })?;
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    };
    *value = YamlValue::String(path.display().to_string());
    Ok(())
}

fn absolutize_nested_yaml_path(
    value: &mut YamlValue,
    path: &[&str],
    original_config_dir: &Path,
) -> Result<(), String> {
    let Some((head, tail)) = path.split_first() else {
        return Ok(());
    };
    let mapping = value.as_mapping_mut().ok_or_else(|| {
        format!(
            "pinned build `{}` parent must be a YAML mapping",
            path.join(".")
        )
    })?;
    let key = YamlValue::String((*head).to_string());
    let Some(child) = mapping.get_mut(&key) else {
        return Ok(());
    };
    if child.is_null() {
        return Ok(());
    }
    if tail.is_empty() {
        let raw = child
            .as_str()
            .ok_or_else(|| format!("pinned build path `{}` must be a string", path.join(".")))?;
        let raw = PathBuf::from(raw);
        let absolute = if raw.is_absolute() {
            raw
        } else {
            original_config_dir.join(raw)
        };
        *child = YamlValue::String(absolute.display().to_string());
        Ok(())
    } else {
        absolutize_nested_yaml_path(child, tail, original_config_dir)
    }
}

fn absolutize_va_profile_paths(
    value: &mut YamlValue,
    original_config_dir: &Path,
) -> Result<(), String> {
    let Some(profiles) = yaml_mapping_at_mut(value, &["tests", "va", "profiles"])? else {
        return Ok(());
    };
    for (name, profile) in profiles {
        let name = name
            .as_str()
            .ok_or_else(|| "tests.va.profiles keys must be strings".to_string())?;
        let profile = profile
            .as_mapping_mut()
            .ok_or_else(|| format!("tests.va.profiles.{name} must be a YAML mapping"))?;
        absolutize_yaml_path(profile, "feature_path", original_config_dir)?;
    }
    Ok(())
}

fn yaml_mapping_at_mut<'a>(
    value: &'a mut YamlValue,
    path: &[&str],
) -> Result<Option<&'a mut serde_yaml::Mapping>, String> {
    let mut current = value;
    for (index, segment) in path.iter().enumerate() {
        if current.is_null() {
            return Ok(None);
        }
        let mapping = current.as_mapping_mut().ok_or_else(|| {
            format!(
                "pinned build `{}` must be a YAML mapping",
                path[..index].join(".")
            )
        })?;
        let key = YamlValue::String((*segment).to_string());
        let Some(next) = mapping.get_mut(&key) else {
            return Ok(None);
        };
        current = next;
    }
    if current.is_null() {
        return Ok(None);
    }
    current
        .as_mapping_mut()
        .map(Some)
        .ok_or_else(|| format!("pinned build `{}` must be a YAML mapping", path.join(".")))
}

fn absolutize_infobase_connection(
    value: &mut YamlValue,
    original_config_dir: &Path,
) -> Result<(), String> {
    let Some(infobase) = yaml_mapping_at_mut(value, &["infobase"])? else {
        return Ok(());
    };
    let connection_key = YamlValue::String("connection".to_string());
    let Some(connection) = infobase.get_mut(&connection_key) else {
        return Ok(());
    };
    let normalized = connection
        .as_str()
        .ok_or_else(|| "pinned build infobase.connection must be a string".to_string())
        .map(|raw| absolutize_file_connection(raw, original_config_dir))?;
    *connection = YamlValue::String(normalized);
    Ok(())
}

fn absolutize_file_connection(connection: &str, workspace_root: &Path) -> String {
    let trimmed = connection.trim();
    if trimmed.starts_with('/') || trimmed.starts_with('-') {
        return absolutize_raw_file_connection(trimmed, workspace_root);
    }
    let mut changed = false;
    let segments = connection
        .split(';')
        .map(|segment| {
            let segment = segment.trim();
            let lower = segment.to_ascii_lowercase();
            if lower.starts_with("file=") {
                let normalized = normalize_connection_file_path(&segment[5..], workspace_root);
                changed |= normalized != segment[5..];
                format!("{}{}", &segment[..5], normalized)
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>();
    if changed {
        segments.join(";")
    } else {
        connection.to_string()
    }
}

fn absolutize_raw_file_connection(connection: &str, original_config_dir: &Path) -> String {
    let mut args = split_connection_args(connection);
    let mut changed = false;
    let mut index = 0;
    while index + 1 < args.len() {
        if args[index].eq_ignore_ascii_case("/f") || args[index].eq_ignore_ascii_case("-f") {
            let normalized = normalize_connection_file_path(&args[index + 1], original_config_dir);
            changed |= normalized != args[index + 1];
            args[index + 1] = normalized;
            index += 2;
        } else {
            index += 1;
        }
    }
    if changed {
        join_connection_args(&args)
    } else {
        connection.to_string()
    }
}

fn normalize_connection_file_path(path: &str, original_config_dir: &Path) -> String {
    let path = path.trim();
    let path = strip_matching_connection_quotes(path).unwrap_or(path);
    let path = Path::new(path);
    if path.is_absolute() {
        path.display().to_string()
    } else {
        original_config_dir.join(path).display().to_string()
    }
}

fn strip_matching_connection_quotes(value: &str) -> Option<&str> {
    if value.len() < 2 {
        return None;
    }
    let quote = value.as_bytes()[0];
    let last = *value.as_bytes().last()?;
    ((quote == b'\'' || quote == b'"') && quote == last).then_some(&value[1..value.len() - 1])
}

fn split_connection_args(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for character in raw.chars() {
        match character {
            '"' => in_quotes = !in_quotes,
            character if character.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

fn join_connection_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() || arg.chars().any(char::is_whitespace) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn absolutize_json_path(value: &mut Value, base: &Path, key: &str) -> Result<(), String> {
    let raw = value
        .as_str()
        .ok_or_else(|| format!("runtime argument `{key}` must be a string"))?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        *value = json!(base.join(path).display().to_string());
    }
    Ok(())
}

fn serialize_pinned_yaml(value: &YamlValue) -> Result<Vec<u8>, String> {
    let mut bytes = serde_yaml::to_string(value)
        .map_err(|error| format!("failed to serialize pinned build config: {error}"))?
        .into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn create_unique_build_snapshot_directory(root: &Path) -> Result<(uuid::Uuid, PathBuf), String> {
    for _ in 0..32 {
        let transaction_id = uuid::Uuid::new_v4();
        let path = root.join(format!(
            "{BUILD_SNAPSHOT_CREATING_PREFIX}{}",
            transaction_id.hyphenated()
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => {
                if let Err(error) = sync_build_snapshot_directory(root) {
                    let _ = fs::remove_dir(&path);
                    return Err(error);
                }
                return Ok((transaction_id, path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create pinned build directory {}: {error}",
                    path.display()
                ))
            }
        }
    }
    Err("failed to allocate a unique pinned build directory".to_string())
}

fn write_build_snapshot_ownership(
    directory: &Path,
    ownership: &BuildSnapshotOwnership,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(ownership)
        .map_err(|error| format!("failed to serialize pinned build ownership: {error}"))?;
    bytes.push(b'\n');
    let temporary = directory.join(BUILD_SNAPSHOT_OWNERSHIP_TEMP_FILE);
    write_private_bytes(&temporary, &bytes)?;
    let committed = directory.join(BUILD_SNAPSHOT_OWNERSHIP_FILE);
    fs::rename(&temporary, &committed).map_err(|error| {
        format!(
            "failed to commit pinned build ownership {}: {error}",
            committed.display()
        )
    })?;
    sync_build_snapshot_directory(directory)
}

fn write_private_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("failed to create pinned config {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write pinned config {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync pinned config {}: {error}", path.display()))
}

fn write_private_snapshot_config(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let temporary = directory.join(format!(".{name}.tmp"));
    write_private_bytes(&temporary, bytes)?;
    let committed = directory.join(name);
    fs::rename(&temporary, &committed).map_err(|error| {
        format!(
            "failed to commit pinned config {}: {error}",
            committed.display()
        )
    })?;
    sync_build_snapshot_directory(directory)
}

fn recover_stale_build_snapshots(root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|error| {
        format!(
            "failed to scan source-sync root {}: {error}",
            root.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read source-sync entry in {}: {error}",
                root.display()
            )
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let phase = parse_build_snapshot_name(&name);
        if phase.is_none() {
            continue;
        }
        let marker = entry.path().join(BUILD_SNAPSHOT_OWNERSHIP_FILE);
        if matches!(phase, Some(BuildSnapshotPhase::Creating)) && !marker.exists() {
            cleanup_uncommitted_build_snapshot(&entry.path())?;
            continue;
        }
        remove_owned_build_snapshot(&entry.path())?;
    }
    Ok(())
}

fn audit_stale_build_snapshots(root: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to inspect source-sync root {}: {error}",
                root.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "source-sync root {} must be a non-symlink directory",
            root.display()
        ));
    }
    for entry in fs::read_dir(root).map_err(|error| {
        format!(
            "failed to scan source-sync root {}: {error}",
            root.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read source-sync entry in {}: {error}",
                root.display()
            )
        })?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| parse_build_snapshot_name(name).is_some())
        {
            return Err(
                "source-sync preview cannot prove apply behavior while pinned build recovery is pending"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn cleanup_uncommitted_build_snapshot(directory: &Path) -> Result<(), String> {
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "creating build snapshot name must be UTF-8".to_string())?;
    if parse_build_snapshot_name(name) != Some(BuildSnapshotPhase::Creating) {
        return Err("refusing to clean a non-creating build snapshot".to_string());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "failed to inspect creating build snapshot {}: {error}",
            directory.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "creating build snapshot {} must be a non-symlink directory",
            directory.display()
        ));
    }
    let entries = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "failed to scan creating build snapshot {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect creating build snapshot: {error}"))?;
    for entry in &entries {
        if entry.file_name().to_str() != Some(BUILD_SNAPSHOT_OWNERSHIP_TEMP_FILE) {
            // No config bytes are written by this protocol until the marker
            // is atomically committed. Preserve any foreign shape.
            return Ok(());
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            format!(
                "failed to inspect temporary build ownership {}: {error}",
                entry.path().display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "temporary build ownership {} must be a regular non-symlink file",
                entry.path().display()
            ));
        }
    }
    for entry in entries {
        fs::remove_file(entry.path()).map_err(|error| {
            format!(
                "failed to remove temporary build ownership {}: {error}",
                entry.path().display()
            )
        })?;
    }
    fs::remove_dir(directory).map_err(|error| {
        format!(
            "failed to remove uncommitted build snapshot {}: {error}",
            directory.display()
        )
    })?;
    if let Some(parent) = directory.parent() {
        sync_build_snapshot_directory(parent)?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildSnapshotPhase {
    Creating,
    Committed,
}

fn parse_build_snapshot_name(name: &str) -> Option<BuildSnapshotPhase> {
    let (phase, uuid_raw) = if let Some(uuid) = name.strip_prefix(BUILD_SNAPSHOT_CREATING_PREFIX) {
        (BuildSnapshotPhase::Creating, uuid)
    } else {
        (
            BuildSnapshotPhase::Committed,
            name.strip_prefix(BUILD_SNAPSHOT_PREFIX)?,
        )
    };
    let uuid = uuid::Uuid::parse_str(uuid_raw).ok()?;
    if uuid.hyphenated().to_string() != uuid_raw {
        return None;
    }
    Some(phase)
}

fn remove_owned_build_snapshot(directory: &Path) -> Result<(), String> {
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "pinned build snapshot name must be UTF-8".to_string())?;
    parse_build_snapshot_name(name)
        .ok_or_else(|| format!("pinned build snapshot name is not canonical: {name}"))?;
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "failed to inspect pinned build snapshot {}: {error}",
            directory.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "pinned build snapshot {} must be a non-symlink directory",
            directory.display()
        ));
    }
    let entries = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "failed to scan pinned build snapshot {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to inspect pinned build entry in {}: {error}",
                directory.display()
            )
        })?;
    if entries.is_empty() {
        fs::remove_dir(directory).map_err(|error| {
            format!(
                "failed to remove empty pinned build snapshot {}: {error}",
                directory.display()
            )
        })?;
        if let Some(parent) = directory.parent() {
            sync_build_snapshot_directory(parent)?;
        }
        return Ok(());
    }

    let marker_path = directory.join(BUILD_SNAPSHOT_OWNERSHIP_FILE);
    let marker_bytes = read_stable_build_snapshot_file(&marker_path).map_err(|error| {
        format!(
            "pinned build snapshot {} has no valid ownership marker: {error}",
            directory.display()
        )
    })?;
    let ownership: BuildSnapshotOwnership =
        serde_json::from_slice(&marker_bytes).map_err(|error| {
            format!(
                "pinned build ownership {} is invalid: {error}",
                marker_path.display()
            )
        })?;
    validate_build_snapshot_ownership(name, &ownership)?;

    let mut primary_present = false;
    let mut local_present = false;
    let mut partial_configs = Vec::new();
    for entry in entries {
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| "pinned build snapshot contains a non-UTF-8 entry".to_string())?;
        match name.as_str() {
            BUILD_SNAPSHOT_OWNERSHIP_FILE => {}
            "v8project.yaml" if !primary_present => {
                let bytes = read_stable_build_snapshot_file(&entry.path())?;
                if sha256_hex(&bytes) != ownership.primary_sha256 {
                    return Err(format!(
                        "pinned primary config {} ownership fingerprint changed",
                        entry.path().display()
                    ));
                }
                primary_present = true;
            }
            "v8project.local.yaml" if !local_present => {
                let expected = ownership.local_sha256.as_deref().ok_or_else(|| {
                    format!(
                        "pinned build snapshot {} contains an unowned local overlay",
                        directory.display()
                    )
                })?;
                let bytes = read_stable_build_snapshot_file(&entry.path())?;
                if sha256_hex(&bytes) != expected {
                    return Err(format!(
                        "pinned local config {} ownership fingerprint changed",
                        entry.path().display()
                    ));
                }
                local_present = true;
            }
            ".v8project.yaml.tmp" | ".v8project.local.yaml.tmp" => {
                let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                    format!(
                        "failed to inspect partial pinned config {}: {error}",
                        entry.path().display()
                    )
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(format!(
                        "partial pinned config {} must be a regular non-symlink file",
                        entry.path().display()
                    ));
                }
                partial_configs.push(entry.path());
            }
            _ => {
                return Err(format!(
                    "pinned build snapshot {} contains unowned entry `{name}`",
                    directory.display()
                ))
            }
        }
    }
    if primary_present {
        fs::remove_file(directory.join("v8project.yaml")).map_err(|error| {
            format!(
                "failed to remove pinned primary config in {}: {error}",
                directory.display()
            )
        })?;
    }
    if local_present {
        fs::remove_file(directory.join("v8project.local.yaml")).map_err(|error| {
            format!(
                "failed to remove pinned local config in {}: {error}",
                directory.display()
            )
        })?;
    }
    for partial in partial_configs {
        fs::remove_file(&partial).map_err(|error| {
            format!(
                "failed to remove partial pinned config {}: {error}",
                partial.display()
            )
        })?;
    }
    // Make deletion of all credential-bearing files durable while the marker
    // still exists, so every crash point remains safely recoverable.
    sync_build_snapshot_directory(directory)?;
    fs::remove_file(&marker_path).map_err(|error| {
        format!(
            "failed to remove pinned build ownership {}: {error}",
            marker_path.display()
        )
    })?;
    sync_build_snapshot_directory(directory)?;
    fs::remove_dir(directory).map_err(|error| {
        format!(
            "failed to remove pinned build snapshot {}: {error}",
            directory.display()
        )
    })?;
    if let Some(parent) = directory.parent() {
        sync_build_snapshot_directory(parent)?;
    }
    Ok(())
}

fn validate_build_snapshot_ownership(
    directory_name: &str,
    ownership: &BuildSnapshotOwnership,
) -> Result<(), String> {
    if ownership.schema_version != BUILD_SNAPSHOT_OWNERSHIP_SCHEMA_VERSION {
        return Err(format!(
            "unsupported pinned build ownership schema {}",
            ownership.schema_version
        ));
    }
    let transaction = uuid::Uuid::parse_str(&ownership.transaction_id)
        .map_err(|_| "pinned build ownership transaction id is invalid".to_string())?;
    if transaction.hyphenated().to_string() != ownership.transaction_id
        || !directory_name.ends_with(&ownership.transaction_id)
    {
        return Err("pinned build ownership does not match its directory".to_string());
    }
    for hash in
        std::iter::once(ownership.primary_sha256.as_str()).chain(ownership.local_sha256.as_deref())
    {
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("pinned build ownership contains an invalid fingerprint".to_string());
        }
    }
    Ok(())
}

fn read_stable_build_snapshot_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect pinned config {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "pinned config {} must be a regular non-symlink file",
            path.display()
        ));
    }
    let first = fs::read(path)
        .map_err(|error| format!("failed to read pinned config {}: {error}", path.display()))?;
    let second = fs::read(path)
        .map_err(|error| format!("failed to verify pinned config {}: {error}", path.display()))?;
    if first != second {
        return Err(format!(
            "pinned config {} changed while it was inspected",
            path.display()
        ));
    }
    Ok(first)
}

#[cfg(unix)]
fn sync_build_snapshot_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("failed to sync build snapshot {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_build_snapshot_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn capture_runtime_config_file(path: &Path) -> Result<FileFingerprint, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileFingerprint::Deleted)
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect runtime config {}: {error}",
                path.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "runtime config {} must be a regular non-symlink file",
            path.display()
        ));
    }
    let first = fs::read(path)
        .map_err(|error| format!("failed to read runtime config {}: {error}", path.display()))?;
    let second = fs::read(path).map_err(|error| {
        format!(
            "failed to verify runtime config {}: {error}",
            path.display()
        )
    })?;
    if first != second {
        return Err(format!(
            "runtime config {} changed while it was observed",
            path.display()
        ));
    }
    Ok(FileFingerprint::present(&first))
}

fn validate_build_config_identity(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let Some(raw_config) = args.get("config").and_then(Value::as_str) else {
        return Ok(());
    };
    let requested = PathBuf::from(raw_config);
    let requested = if requested.is_absolute() {
        requested
    } else {
        context.cwd.join(requested)
    };
    let requested = requested.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize requested build config {}: {error}",
            requested.display()
        )
    })?;
    let expected = context
        .workspace_root
        .join("v8project.yaml")
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize durable build config: {error}"))?;
    if requested != expected {
        return Err(format!(
            "requested build config `{}` does not match durable source topology config `{}`",
            requested.display(),
            expected.display()
        ));
    }
    capture_runtime_config_file(&expected)?;
    Ok(())
}

pub(crate) fn preview_runtime_sync(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> Result<RuntimePreviewPreparation, String> {
    if !dry_run || !matches!(spec.handler, ToolHandler::RuntimeAdapter) {
        return Ok(RuntimePreviewPreparation::None);
    }
    let operation = args.get("operation").and_then(Value::as_str);
    if !matches!(operation, Some("build" | "dump")) {
        return Ok(RuntimePreviewPreparation::None);
    }
    let preview = (|| -> Result<Value, String> {
        let repository = SourceSyncRepository::new(context)?;
        let bound_context = bind_context_to_repository(context, &repository)?;
        repository.audit_pending_publication_recovery()?;
        audit_stale_build_snapshots(repository.transaction_root())?;
        if operation == Some("dump") {
            audit_stale_shadow_dumps(&bound_context)?;
        }
        let state = repository.load_state()?;
        match operation {
            Some("build") => preview_build_sync(&repository, state, args, &bound_context),
            Some("dump") => preview_dump_sync(&repository, state, args, &bound_context),
            _ => unreachable!("runtime sync preview operation was checked"),
        }
    })();
    Ok(match preview {
        Ok(details) => RuntimePreviewPreparation::Ready(details),
        Err(error) => {
            let mut details = runtime_internal_failure_details(
                args,
                context,
                operation.unwrap_or("runtime"),
                "previewFailed",
                &error,
            );
            details["dryRun"] = json!(true);
            details["wouldBlock"] = json!(true);
            let mut outcome = AdapterOutcome::ok("source-sync dry-run preview failed closed");
            outcome.ok = false;
            outcome.errors.push(error);
            RuntimePreviewPreparation::Blocked {
                outcome: Box::new(outcome),
                details,
            }
        }
    })
}

fn preview_build_sync(
    repository: &SourceSyncRepository,
    state: crate::domain::source_sync::SourceSyncState,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Value, String> {
    let requested_source_set = args.get("sourceSet").and_then(Value::as_str);
    let mut requested = state.targets.into_values().collect::<Vec<_>>();
    // Mirror reconcile_all without persisting: observe every durable record
    // before request filtering. An unreadable unrelated target blocks apply's
    // reconciliation and therefore must also block preview.
    for record in &mut requested {
        record.current = repository.current_manifest(&record.target.id)?;
    }
    requested.retain(SourceTargetRecord::is_dirty);
    requested.retain(|record| {
        requested_source_set.is_none_or(|expected| {
            record
                .target
                .source_set
                .as_ref()
                .is_some_and(|actual| actual.as_str() == expected)
        })
    });
    if requested.is_empty() {
        return Ok(json!({
            "dryRun": true,
            "wouldBlock": false,
            "requested": [],
            "processed": [],
            "skipped": [],
            "conflicted": [],
        }));
    }
    let config_snapshot = RuntimeConfigSnapshot::capture(context)?;
    prepare_pinned_build_bytes(context, &config_snapshot)?;
    let config_error = validate_build_config_identity(args, context).err();
    let mut conflicts = Vec::new();
    for record in &requested {
        let error = config_error
            .as_deref()
            .map(str::to_string)
            .or_else(|| repository.validate_target_topology(record).err());
        if let Some(error) = error {
            conflicts.push(terminal_details(
                record,
                "sourceTopologyChanged",
                Some(&error),
            ));
        }
    }
    let conflict_ids = conflicts
        .iter()
        .filter_map(|entry| entry.get("targetId").and_then(Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    let skipped = requested
        .iter()
        .filter(|record| !conflict_ids.contains(record.target.id.as_str()))
        .map(|record| {
            terminal_details(
                record,
                if conflicts.is_empty() {
                    "dryRun"
                } else {
                    "batchConflict"
                },
                None,
            )
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "dryRun": true,
        "wouldBlock": !conflicts.is_empty(),
        "requested": requested.iter().map(target_details).collect::<Vec<_>>(),
        "processed": [],
        "skipped": skipped,
        "conflicted": conflicts,
    }))
}

fn preview_dump_sync(
    repository: &SourceSyncRepository,
    state: crate::domain::source_sync::SourceSyncState,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Value, String> {
    let config_snapshot = if state.targets.is_empty() {
        None
    } else {
        Some(RuntimeConfigSnapshot::capture(context)?)
    };
    if args.get("mode").and_then(Value::as_str) != Some("partial") {
        let requested = state.targets.into_values().collect::<Vec<_>>();
        let conflicts = requested
            .iter()
            .map(|record| terminal_details(record, "unsafeDumpMode", None))
            .collect::<Vec<_>>();
        return Ok(json!({
            "dryRun": true,
            "wouldBlock": !conflicts.is_empty(),
            "requested": requested.iter().map(target_details).collect::<Vec<_>>(),
            "processed": [],
            "skipped": [],
            "conflicted": conflicts,
        }));
    }

    let selectors = super::tool_contracts::normalized_runtime_dump_selectors(args)?;
    let requested_source_set = args.get("sourceSet").and_then(Value::as_str);
    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
    let mut requested = Vec::new();
    let mut unknown = Vec::new();
    for selector in &selectors {
        let mut matches = state
            .targets
            .values()
            .filter(|record| record.target.owner_selector == *selector)
            .filter(|record| {
                requested_source_set.is_none_or(|expected| {
                    record
                        .target
                        .source_set
                        .as_ref()
                        .is_some_and(|actual| actual.as_str() == expected)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| left.target.id.cmp(&right.target.id));
        if matches.is_empty() {
            unknown.push(selector.clone());
        } else {
            for record in matches {
                if !requested
                    .iter()
                    .any(|existing: &SourceTargetRecord| existing.target.id == record.target.id)
                {
                    requested.push(record);
                }
            }
        }
    }

    let matched_source_sets = requested
        .iter()
        .filter_map(|record| record.target.source_set.as_ref().map(|name| name.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    let all_source_sets = state
        .targets
        .values()
        .filter_map(|record| record.target.source_set.as_ref().map(|name| name.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    let inferred_source_sets = if matched_source_sets.is_empty() {
        &all_source_sets
    } else {
        &matched_source_sets
    };
    let source_set = match requested_source_set {
        Some(source_set) => Some(source_set.to_string()),
        None if inferred_source_sets.len() == 1 => {
            inferred_source_sets.first().map(|name| (*name).to_string())
        }
        None => None,
    };

    let mut conflicts = unknown
        .iter()
        .map(|selector| {
            json!({
                "ownerSelector": selector,
                "sourceSet": source_set.as_deref(),
                "targetId": Value::Null,
                "reason": "baselineMissing",
            })
        })
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for record in &mut requested {
        if source_set.is_none() && record.target.source_set.is_some() {
            conflicts.push(terminal_details(record, "sourceSetAmbiguous", None));
            continue;
        }
        if record.target.source_set.is_none() {
            conflicts.push(terminal_details(record, "sourceSetUnknown", None));
            continue;
        }
        if let Err(error) = repository.validate_target_topology(record) {
            conflicts.push(terminal_details(
                record,
                "sourceTopologyChanged",
                Some(&error),
            ));
            continue;
        }
        match repository.capture_manifest(&record.target) {
            Ok(current) => record.current = current,
            Err(error) => {
                conflicts.push(terminal_details(
                    record,
                    "sourceObservationFailed",
                    Some(&error),
                ));
                continue;
            }
        }
        if record.is_dirty() && !force {
            conflicts.push(terminal_details(record, "localSourceDiverged", None));
        } else {
            candidates.push(record.target.id.clone());
        }
    }

    if conflicts.is_empty() {
        let source_set = source_set.as_deref().ok_or_else(|| {
            "partial dump sourceSet could not be resolved after source-sync preview".to_string()
        })?;
        validate_build_config_identity(args, context)?;
        let prepared = prepare_pinned_build_bytes(
            context,
            config_snapshot.as_ref().ok_or_else(|| {
                "partial dump source-sync state disappeared during preview".to_string()
            })?,
        )?;
        validate_pinned_shadow_config(
            source_set,
            &selectors,
            args,
            &prepared.primary,
            prepared.local.as_deref(),
        )?;
    }

    let skipped_reason = if conflicts.is_empty() {
        "dryRunRequiresShadow"
    } else {
        "batchConflict"
    };
    let skipped = requested
        .iter()
        .filter(|record| candidates.iter().any(|id| id == &record.target.id))
        .map(|record| terminal_details(record, skipped_reason, None))
        .collect::<Vec<_>>();
    let mut requested_details = requested.iter().map(target_details).collect::<Vec<_>>();
    requested_details.extend(unknown.iter().map(|selector| {
        json!({
            "ownerSelector": selector,
            "sourceSet": source_set.as_deref(),
            "targetId": Value::Null,
        })
    }));
    Ok(json!({
        "dryRun": true,
        "wouldBlock": !conflicts.is_empty(),
        "requested": requested_details,
        "processed": [],
        "skipped": skipped,
        "conflicted": conflicts,
    }))
}

impl BuildSession {
    pub(crate) fn handler_args(&self) -> &Map<String, Value> {
        &self.handler_args
    }

    pub(crate) fn failure_details(&self, reason: &str, message: &str) -> Value {
        terminal_failure_details(&self.requested, reason, message)
    }

    pub(crate) fn cleanup_with_warning(&self, outcome: &mut AdapterOutcome) {
        if let Err(error) = self.pinned_config.cleanup() {
            outcome.warnings.push(format!(
                "pinned build config cleanup was deferred to the next locked operation: {error}"
            ));
        }
    }

    pub(crate) fn finish(&self, outcome: &mut AdapterOutcome) -> Result<Value, String> {
        let current_config =
            RuntimeConfigSnapshot::capture_from_paths(self.config_snapshot.files.keys().cloned());
        let config_error = match current_config {
            Ok(snapshot) if snapshot == self.config_snapshot => None,
            Ok(_) => Some(
                "runtime config changed while build was running; refusing to clear source-sync targets"
                    .to_string(),
            ),
            Err(error) => Some(error),
        }
        .or_else(|| self.pinned_config.verify_unchanged().err());
        let mut post_run_conflicts = Vec::new();
        let stable_requested = self
            .requested
            .iter()
            .filter(|record| {
                let error = config_error
                    .as_deref()
                    .map(str::to_string)
                    .or_else(|| self.repository.validate_target_topology(record).err());
                if let Some(error) = error {
                    post_run_conflicts.push(terminal_details(
                        record,
                        if config_error.is_some() {
                            "runtimeConfigChanged"
                        } else {
                            "sourceTopologyChangedDuringBuild"
                        },
                        Some(&error),
                    ));
                    false
                } else {
                    true
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        let after_build = stable_requested
            .iter()
            .map(|record| {
                self.repository
                    .current_manifest(&record.target.id)
                    .map(|manifest| (record.target.id.clone(), manifest))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let parsed = outcome
            .stdout
            .as_deref()
            .ok_or_else(|| "v8-runner build returned no JSON stdout".to_string())
            .and_then(parse_runtime_build_report);
        let mut reconciliation = classify_build_result(
            &stable_requested,
            &after_build,
            parsed.as_ref().map_err(String::as_str),
        );
        reconciliation.details["requested"] = Value::Array(
            self.requested
                .iter()
                .map(target_details)
                .collect::<Vec<_>>(),
        );
        reconciliation.details["conflicted"]
            .as_array_mut()
            .expect("build details always contain conflicted array")
            .extend(post_run_conflicts);

        if !reconciliation.synchronized.is_empty() {
            let expected = reconciliation
                .synchronized
                .iter()
                .cloned()
                .collect::<BTreeMap<_, _>>();
            let cas = self
                .repository
                .mark_synchronized(self.generation, &expected)?;
            if !cas.conflicted.is_empty() {
                apply_synchronization_conflicts(
                    &mut reconciliation.details,
                    &stable_requested,
                    &cas.conflicted,
                );
                reconciliation
                    .synchronized
                    .retain(|(id, _)| cas.processed.iter().any(|processed| processed == id));
            }
        }

        if !reconciliation.synchronized.is_empty() && outcome.changes.is_empty() {
            outcome.changes.push(format!(
                "{} source-sync target(s) reached the infobase and durable synchronization state was advanced",
                reconciliation.synchronized.len()
            ));
        }

        let incomplete = !reconciliation.details["skipped"]
            .as_array()
            .is_none_or(Vec::is_empty)
            || !reconciliation.details["conflicted"]
                .as_array()
                .is_none_or(Vec::is_empty);
        if incomplete {
            outcome.ok = false;
            outcome.errors.push(
                "source-sync build reconciliation was incomplete; affected targets remain dirty"
                    .to_string(),
            );
            outcome.summary =
                "unica.runtime.execute could not prove all requested source targets reached the infobase"
                    .to_string();
        }
        Ok(reconciliation.details)
    }
}

pub(crate) fn prepare_dump(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> Result<DumpPreparation, String> {
    if dry_run || !is_typed_runtime_operation(spec, args, "dump") {
        return Ok(DumpPreparation::None);
    }

    let prepared = (|| -> Result<DumpPreparation, String> {
        let repository = SourceSyncRepository::new(context)?;
        let lock = repository.acquire_lifecycle_lock()?;
        repository.recover_pending_publications()?;
        recover_stale_build_snapshots(repository.transaction_root())?;
        let bound_context = bind_context_to_repository(context, &repository)?;
        recover_stale_shadow_dumps(&bound_context)?;
        let state_before_reconciliation = repository.load_state()?;
        let initial_config_snapshot = if state_before_reconciliation.targets.is_empty() {
            None
        } else {
            Some(RuntimeConfigSnapshot::capture(&bound_context)?)
        };
        repository.reconcile_all()?;
        let state = repository.load_state()?;

        if args.get("mode").and_then(Value::as_str) != Some("partial") {
            if state.targets.is_empty() {
                return Ok(DumpPreparation::Passthrough { _lock: lock });
            }
            let requested = state.targets.values().cloned().collect::<Vec<_>>();
            let conflicted = requested
                .iter()
                .map(|record| terminal_details(record, "unsafeDumpMode", None))
                .collect::<Vec<_>>();
            let mut outcome = AdapterOutcome::ok(
                "dump blocked before runner execution by persistent source-sync state",
            );
            outcome.ok = false;
            outcome.errors.push(
            "full and incremental dump modes cannot overwrite working sources while source-sync state is active; use a targeted partial dump"
                .to_string(),
        );
            return Ok(DumpPreparation::Blocked {
                outcome: Box::new(outcome),
                details: json!({
                    "requested": requested.iter().map(target_details).collect::<Vec<_>>(),
                    "processed": [],
                    "skipped": [],
                    "conflicted": conflicted,
                }),
            });
        }

        let generation = state.generation;
        let selectors = super::tool_contracts::normalized_runtime_dump_selectors(args)?;
        let requested_source_set = args.get("sourceSet").and_then(Value::as_str);
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);

        let mut requested = Vec::new();
        let mut unknown = Vec::new();
        for selector in &selectors {
            let mut matches = state
                .targets
                .values()
                .filter(|record| record.target.owner_selector == *selector)
                .filter(|record| {
                    requested_source_set.is_none_or(|expected| {
                        record
                            .target
                            .source_set
                            .as_ref()
                            .is_some_and(|actual| actual.as_str() == expected)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            matches.sort_by(|left, right| left.target.id.cmp(&right.target.id));
            if matches.is_empty() {
                unknown.push(selector.clone());
            } else {
                for record in matches {
                    if !requested
                        .iter()
                        .any(|existing: &SourceTargetRecord| existing.target.id == record.target.id)
                    {
                        requested.push(record);
                    }
                }
            }
        }

        let matched_source_sets = requested
            .iter()
            .filter_map(|record| record.target.source_set.as_ref().map(|name| name.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        let source_sets = if matched_source_sets.is_empty() {
            state
                .targets
                .values()
                .filter_map(|record| record.target.source_set.as_ref().map(|name| name.as_str()))
                .collect::<std::collections::BTreeSet<_>>()
        } else {
            matched_source_sets
        };
        let source_set = match requested_source_set {
            Some(source_set) => Some(source_set.to_string()),
            None if source_sets.len() == 1 => source_sets.first().map(|name| (*name).to_string()),
            None => None,
        };

        let mut conflicts = Vec::new();
        for selector in &unknown {
            conflicts.push(json!({
                "ownerSelector": selector,
                "sourceSet": source_set.as_deref(),
                "targetId": Value::Null,
                "reason": "baselineMissing",
            }));
        }
        for record in &requested {
            if source_set.is_none() && record.target.source_set.is_some() {
                conflicts.push(terminal_details(record, "sourceSetAmbiguous", None));
            } else if record.target.source_set.is_none() {
                conflicts.push(terminal_details(record, "sourceSetUnknown", None));
            } else if let Err(error) = repository.validate_target_topology(record) {
                conflicts.push(terminal_details(
                    record,
                    "sourceTopologyChanged",
                    Some(&error),
                ));
            } else if record.is_dirty() && !force {
                conflicts.push(terminal_details(record, "localSourceDiverged", None));
            }
        }
        if !conflicts.is_empty() {
            let conflict_ids = conflicts
                .iter()
                .filter_map(|entry| entry.get("targetId").and_then(Value::as_str))
                .collect::<std::collections::BTreeSet<_>>();
            let skipped = requested
                .iter()
                .filter(|record| !conflict_ids.contains(record.target.id.as_str()))
                .map(|record| terminal_details(record, "batchConflict", None))
                .collect::<Vec<_>>();
            let mut outcome = AdapterOutcome::ok(
                "partial dump blocked before runner execution by source-sync preflight",
            );
            outcome.ok = false;
            outcome.errors.push(
                "partial dump has unresolved source-sync conflicts; working source was not changed"
                    .to_string(),
            );
            let mut requested_details = requested.iter().map(target_details).collect::<Vec<_>>();
            requested_details.extend(unknown.iter().map(|selector| {
                json!({
                    "ownerSelector": selector,
                    "sourceSet": source_set.as_deref(),
                    "targetId": Value::Null,
                })
            }));
            return Ok(DumpPreparation::Blocked {
                outcome: Box::new(outcome),
                details: json!({
                    "requested": requested_details,
                    "processed": [],
                    "skipped": skipped,
                    "conflicted": conflicts,
                }),
            });
        }

        let source_set = source_set.ok_or_else(|| {
            "partial dump sourceSet could not be resolved after source-sync preflight".to_string()
        })?;
        let cdfi_preimage = Box::new(repository.capture_platform_cdfi_preimage(&lock, &requested)?);
        let initial_config_snapshot = initial_config_snapshot.ok_or_else(|| {
            "partial dump source-sync state disappeared during locked preparation".to_string()
        })?;
        validate_build_config_identity(args, &bound_context)?;
        let prepared_config = prepare_pinned_build_bytes(&bound_context, &initial_config_snapshot)?;
        let original_source_root = bound_context
            .workspace_root
            .join(cdfi_preimage.source_root().as_str());
        let shadow = ShadowDumpPreparation::prepare_pinned(
            &bound_context,
            &source_set,
            &selectors,
            args,
            &original_source_root,
            &prepared_config.primary,
            prepared_config.local.as_deref(),
            ShadowPlatformSeeds {
                configuration: cdfi_preimage.configuration_bytes(),
                config_dump_info: cdfi_preimage.config_dump_info_bytes(),
            },
        )?;
        let config_snapshot = RuntimeConfigSnapshot::capture(&bound_context)?;
        if config_snapshot != initial_config_snapshot {
            return Err(
                "runtime config changed while the shadow dump session was prepared".to_string(),
            );
        }
        let shadow_config_snapshot =
            RuntimeConfigSnapshot::capture_from_paths(shadow.runtime_config_paths())?;
        Ok(DumpPreparation::Ready(Box::new(DumpSession {
            repository,
            lock,
            generation,
            requested,
            force,
            config_snapshot,
            shadow_config_snapshot,
            cdfi_preimage,
            shadow,
        })))
    })();
    Ok(match prepared {
        Ok(preparation) => preparation,
        Err(error) => dump_internal_error_preparation(args, context, error),
    })
}

impl DumpSession {
    pub(crate) fn runtime_args(&self) -> &Map<String, Value> {
        self.shadow.runtime_args()
    }

    pub(crate) fn invocation_failure_details(
        &mut self,
        outcome: &mut AdapterOutcome,
        message: &str,
    ) -> Value {
        let details = terminal_skipped_details(&self.requested, "runnerInvocationFailed", message);
        cleanup_shadow_with_warning(&mut self.shadow, outcome);
        details
    }

    pub(crate) fn finalization_failure_details(
        &mut self,
        outcome: &mut AdapterOutcome,
        message: &str,
    ) -> Value {
        let details =
            terminal_failure_details(&self.requested, "sourceSyncFinalizationFailed", message);
        cleanup_shadow_with_warning(&mut self.shadow, outcome);
        details
    }

    pub(crate) fn finish(&mut self, outcome: &mut AdapterOutcome) -> Result<Value, String> {
        if !outcome.ok {
            let details = json!({
                "requested": self.requested.iter().map(target_details).collect::<Vec<_>>(),
                "processed": [],
                "skipped": self.requested.iter().map(|record| {
                    terminal_details(record, "runnerFailed", outcome.errors.first().map(String::as_str))
                }).collect::<Vec<_>>(),
                "conflicted": [],
            });
            cleanup_shadow_with_warning(&mut self.shadow, outcome);
            return Ok(details);
        }

        let current_config =
            RuntimeConfigSnapshot::capture_from_paths(self.config_snapshot.files.keys().cloned());
        let config_error = match current_config {
            Ok(snapshot) if snapshot == self.config_snapshot => None,
            Ok(_) => {
                Some("runtime config changed while the shadow partial dump was running".to_string())
            }
            Err(error) => Some(error),
        };
        let shadow_config_error = match RuntimeConfigSnapshot::capture_from_paths(
            self.shadow_config_snapshot.files.keys().cloned(),
        ) {
            Ok(snapshot) if snapshot == self.shadow_config_snapshot => None,
            Ok(_) => Some(
                "generated shadow runtime config changed while the partial dump was running"
                    .to_string(),
            ),
            Err(error) => Some(error),
        };
        let mut post_run_conflicts = Vec::new();
        for record in &self.requested {
            let (reason, error) = if let Some(error) = config_error.as_deref() {
                ("runtimeConfigChanged", Some(error.to_string()))
            } else if let Some(error) = shadow_config_error.as_deref() {
                ("shadowConfigChanged", Some(error.to_string()))
            } else {
                match self.repository.validate_target_topology(record) {
                    Ok(()) => continue,
                    Err(error) => ("sourceTopologyChangedDuringDump", Some(error)),
                }
            };
            post_run_conflicts.push(terminal_details(record, reason, error.as_deref()));
        }
        if !post_run_conflicts.is_empty() {
            outcome.ok = false;
            outcome.errors.push(
                "runtime config or source topology changed during the partial dump; shadow output was not published"
                    .to_string(),
            );
            outcome.summary =
                "shadow partial dump failed post-run topology verification".to_string();
            let details = json!({
                "requested": self.requested.iter().map(target_details).collect::<Vec<_>>(),
                "processed": [],
                "skipped": [],
                "conflicted": post_run_conflicts,
            });
            cleanup_shadow_with_warning(&mut self.shadow, outcome);
            return Ok(details);
        }

        let mut shadow_manifests = BTreeMap::new();
        let mut shadow_conflicts = Vec::new();
        if let Err(error) = self
            .repository
            .validate_shadow_platform_guards(&self.cdfi_preimage, self.shadow.shadow_source_dir())
        {
            shadow_conflicts.extend(self.requested.iter().map(|record| {
                terminal_details(record, "shadowPlatformGuardChanged", Some(&error))
            }));
        }
        for record in &self.requested {
            match self
                .repository
                .validate_shadow_target(record, self.shadow.shadow_source_dir(), &self.requested)
                .and_then(|()| {
                    self.repository.capture_manifest_from_source_root(
                        &record.target,
                        self.shadow.shadow_source_dir(),
                    )
                }) {
                Ok(manifest) => {
                    shadow_manifests.insert(record.target.id.clone(), manifest);
                }
                Err(error) => shadow_conflicts.push(terminal_details(
                    record,
                    "shadowTopologyInvalid",
                    Some(&error),
                )),
            }
        }
        if !shadow_conflicts.is_empty() {
            outcome.ok = false;
            outcome.changes.clear();
            outcome.errors.push(
                "shadow partial dump does not represent the persisted source owners; working source was not changed"
                    .to_string(),
            );
            outcome.summary = "shadow partial dump failed semantic verification".to_string();
            let conflict_ids = shadow_conflicts
                .iter()
                .filter_map(|entry| entry.get("targetId").and_then(Value::as_str))
                .collect::<std::collections::BTreeSet<_>>();
            let skipped = self
                .requested
                .iter()
                .filter(|record| !conflict_ids.contains(record.target.id.as_str()))
                .map(|record| terminal_details(record, "batchConflict", None))
                .collect::<Vec<_>>();
            let details = json!({
                "requested": self.requested.iter().map(target_details).collect::<Vec<_>>(),
                "processed": [],
                "skipped": skipped,
                "conflicted": shadow_conflicts,
            });
            cleanup_shadow_with_warning(&mut self.shadow, outcome);
            return Ok(details);
        }
        let mut reconciliation =
            classify_shadow_dump(&self.requested, &shadow_manifests, self.force);
        if reconciliation.blocked {
            outcome.ok = false;
            outcome.changes.clear();
            outcome.errors.push(
                "partial dump diverges from the synchronized baseline; working source was not changed"
                    .to_string(),
            );
            outcome.summary = "partial dump blocked by source-sync conflict".to_string();
            cleanup_shadow_with_warning(&mut self.shadow, outcome);
            return Ok(reconciliation.details);
        }

        // Only explicit force may publish platform output. It also publishes
        // the verified CDFI auxiliary preimage/postimage even when requested
        // owner bytes are identical.
        let publication = if self.force {
            match self.repository.publish_from_source_root(
                &self.lock,
                &self.requested,
                &reconciliation
                    .publish
                    .iter()
                    .cloned()
                    .collect::<BTreeMap<_, _>>(),
                self.shadow.shadow_source_dir(),
                &self.cdfi_preimage,
            ) {
                Ok(publication) => Some(publication),
                Err(error) => {
                    apply_publication_failure(
                        outcome,
                        &mut reconciliation.details,
                        &self.requested,
                        &error,
                    );
                    cleanup_shadow_with_warning(&mut self.shadow, outcome);
                    return Ok(reconciliation.details);
                }
            }
        } else {
            None
        };

        if let Some(publication) = &publication {
            outcome.changes = publication_change_messages(publication);
            if let Some(warning) = &publication.cleanup_warning {
                outcome.warnings.push(warning.clone());
            }
        } else {
            outcome.changes.clear();
        }

        if publication.is_some() {
            match self.repository.reconcile_all() {
                Ok(reconciled) => self.generation = reconciled.generation,
                Err(error) => {
                    apply_dump_finalization_failure(
                        outcome,
                        &mut reconciliation.details,
                        &self.requested,
                        "postPublicationReconcileFailed",
                        &error,
                    );
                    cleanup_shadow_with_warning(&mut self.shadow, outcome);
                    return Ok(reconciliation.details);
                }
            }
        }

        let expected = reconciliation
            .synchronized
            .iter()
            .cloned()
            .collect::<BTreeMap<_, _>>();
        let cas = match self
            .repository
            .mark_synchronized(self.generation, &expected)
        {
            Ok(cas) => cas,
            Err(error) => {
                apply_dump_finalization_failure(
                    outcome,
                    &mut reconciliation.details,
                    &self.requested,
                    "synchronizationStateFailed",
                    &error,
                );
                cleanup_shadow_with_warning(&mut self.shadow, outcome);
                return Ok(reconciliation.details);
            }
        };
        if !cas.conflicted.is_empty() {
            apply_synchronization_conflicts(
                &mut reconciliation.details,
                &self.requested,
                &cas.conflicted,
            );
            outcome.ok = false;
            outcome.errors.push(
                "source changed during partial dump finalization; targets remain dirty".to_string(),
            );
            outcome.summary = if outcome.changes.is_empty() {
                "partial dump synchronization CAS failed closed".to_string()
            } else {
                "source changed but sync finalization failed".to_string()
            };
        }
        cleanup_shadow_with_warning(&mut self.shadow, outcome);
        Ok(reconciliation.details)
    }
}

fn publication_change_messages(publication: &PublicationOutcome) -> Vec<String> {
    publication
        .published_paths
        .iter()
        .map(|path| format!("published {} from shadow dump", path.as_str()))
        .collect()
}

fn apply_publication_failure(
    outcome: &mut AdapterOutcome,
    details: &mut Value,
    requested: &[SourceTargetRecord],
    error: &PublicationError,
) {
    outcome.ok = false;
    outcome.errors.push(error.message.clone());
    if error.source_may_have_changed {
        outcome.changes = error
            .affected_paths
            .iter()
            .map(|path| {
                format!(
                    "publication may have changed {}; recovery is required",
                    path.as_str()
                )
            })
            .collect();
        outcome.summary =
            "source may have changed and publication recovery is required".to_string();
    } else {
        outcome.changes.clear();
        outcome.summary = "forced partial dump publication failed closed".to_string();
    }
    if error.recovery_required {
        outcome.warnings.push(
            "a prepared publication journal was retained; the next locked operation will recover it"
                .to_string(),
        );
    }
    replace_dump_terminals_with_conflict(details, requested, error.reason, &error.message);
}

fn apply_dump_finalization_failure(
    outcome: &mut AdapterOutcome,
    details: &mut Value,
    requested: &[SourceTargetRecord],
    reason: &str,
    message: &str,
) {
    outcome.ok = false;
    outcome.errors.push(message.to_string());
    outcome.summary = if outcome.changes.is_empty() {
        "partial dump synchronization failed closed".to_string()
    } else {
        "source changed but sync finalization failed".to_string()
    };
    replace_dump_terminals_with_conflict(details, requested, reason, message);
}

fn replace_dump_terminals_with_conflict(
    details: &mut Value,
    requested: &[SourceTargetRecord],
    reason: &str,
    message: &str,
) {
    details["processed"] = json!([]);
    details["skipped"] = json!([]);
    details["conflicted"] = Value::Array(
        requested
            .iter()
            .map(|record| terminal_details(record, reason, Some(message)))
            .collect(),
    );
}

fn cleanup_shadow_with_warning(shadow: &mut ShadowDumpPreparation, outcome: &mut AdapterOutcome) {
    if let Err(error) = shadow.cleanup() {
        outcome.warnings.push(format!(
            "shadow dump cleanup was deferred to the next recovery pass: {error}"
        ));
    }
}

pub(crate) fn prepare_legacy_guard(
    spec: ToolSpec,
    context: &WorkspaceContext,
    dry_run: bool,
) -> Result<LegacyPreparation, String> {
    let ToolHandler::BuildRuntime { command, .. } = spec.handler else {
        return Ok(LegacyPreparation::None);
    };
    if dry_run || !matches!(command.first().copied(), Some("build" | "dump")) {
        return Ok(LegacyPreparation::None);
    }
    let repository = SourceSyncRepository::new(context)?;
    let lock = repository.acquire_lifecycle_lock()?;
    repository.recover_pending_publications()?;
    recover_stale_build_snapshots(repository.transaction_root())?;
    let state = repository.load_state()?;
    if state.targets.is_empty() {
        return Ok(LegacyPreparation::Allowed(lock));
    }

    let records = state.targets.values().cloned().collect::<Vec<_>>();
    let conflicted = records
        .iter()
        .map(|record| terminal_details(record, "legacyRuntimeBypass", None))
        .collect::<Vec<_>>();
    let mut outcome = AdapterOutcome::ok("legacy runtime command blocked by source-sync state");
    outcome.ok = false;
    outcome.errors.push(format!(
        "{} cannot bypass persistent source-sync state; use typed `unica.runtime.execute`",
        spec.name
    ));
    Ok(LegacyPreparation::Blocked {
        outcome: Box::new(outcome),
        details: json!({
            "requested": records.iter().map(target_details).collect::<Vec<_>>(),
            "processed": [],
            "skipped": [],
            "conflicted": conflicted,
        }),
    })
}

fn is_tracked_mutation(spec: ToolSpec) -> bool {
    matches!(spec.name, "unica.meta.edit" | "unica.code.patch")
}

fn is_typed_runtime_operation(spec: ToolSpec, args: &Map<String, Value>, operation: &str) -> bool {
    matches!(spec.handler, ToolHandler::RuntimeAdapter)
        && args.get("operation").and_then(Value::as_str) == Some(operation)
}

fn build_internal_error_preparation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    error: String,
) -> BuildPreparation {
    let details =
        runtime_internal_failure_details(args, context, "build", "buildPreparationFailed", &error);
    let mut outcome = AdapterOutcome::ok("source-sync build preparation failed closed");
    outcome.ok = false;
    outcome.errors.push(error);
    BuildPreparation::Blocked {
        outcome: Box::new(outcome),
        details,
    }
}

fn dump_internal_error_preparation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    error: String,
) -> DumpPreparation {
    let details =
        runtime_internal_failure_details(args, context, "dump", "dumpPreparationFailed", &error);
    let mut outcome = AdapterOutcome::ok("source-sync dump preparation failed closed");
    outcome.ok = false;
    outcome.errors.push(error);
    DumpPreparation::Blocked {
        outcome: Box::new(outcome),
        details,
    }
}

fn runtime_internal_failure_details(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    operation: &str,
    reason: &str,
    message: &str,
) -> Value {
    let records = best_effort_runtime_records(args, context, operation);
    if !records.is_empty() {
        return terminal_failure_details(&records, reason, message);
    }
    let requested = synthetic_runtime_requests(args, operation);
    let conflicted = requested
        .iter()
        .cloned()
        .map(|mut entry| {
            if let Some(object) = entry.as_object_mut() {
                object.insert("reason".to_string(), json!(reason));
                object.insert("message".to_string(), json!(message));
            }
            entry
        })
        .collect::<Vec<_>>();
    json!({
        "requested": requested,
        "processed": [],
        "skipped": [],
        "conflicted": conflicted,
    })
}

fn best_effort_runtime_records(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    operation: &str,
) -> Vec<SourceTargetRecord> {
    let Ok(repository) = SourceSyncRepository::new(context) else {
        return Vec::new();
    };
    let Ok(state) = repository.load_state() else {
        return Vec::new();
    };
    let requested_source_set = args.get("sourceSet").and_then(Value::as_str);
    let selectors = if operation == "dump" {
        super::tool_contracts::normalized_runtime_dump_selectors(args).ok()
    } else {
        None
    };
    state
        .targets
        .into_values()
        .filter(|record| {
            requested_source_set.is_none_or(|expected| {
                record
                    .target
                    .source_set
                    .as_ref()
                    .is_some_and(|actual| actual.as_str() == expected)
            })
        })
        .filter(|record| {
            selectors
                .as_ref()
                .is_none_or(|selectors| selectors.contains(&record.target.owner_selector))
        })
        .collect()
}

fn synthetic_runtime_requests(args: &Map<String, Value>, operation: &str) -> Vec<Value> {
    let source_set = args.get("sourceSet").and_then(Value::as_str);
    if operation == "dump" {
        let mut selectors = Vec::new();
        if let Some(selector) = args.get("object").and_then(Value::as_str) {
            selectors.push(selector.to_string());
        }
        if let Some(values) = args.get("objects").and_then(Value::as_array) {
            selectors.extend(values.iter().filter_map(Value::as_str).map(str::to_string));
        }
        selectors.sort();
        selectors.dedup();
        if !selectors.is_empty() {
            return selectors
                .into_iter()
                .map(|selector| {
                    json!({
                        "targetId": Value::Null,
                        "sourceSet": source_set,
                        "ownerSelector": selector,
                        "operation": operation,
                    })
                })
                .collect();
        }
    }
    vec![json!({
        "targetId": Value::Null,
        "sourceSet": source_set,
        "ownerSelector": Value::Null,
        "operation": operation,
    })]
}

fn terminal_failure_details(
    requested: &[SourceTargetRecord],
    reason: &str,
    message: &str,
) -> Value {
    json!({
        "requested": requested.iter().map(target_details).collect::<Vec<_>>(),
        "processed": [],
        "skipped": [],
        "conflicted": requested.iter().map(|record| {
            terminal_details(record, reason, Some(message))
        }).collect::<Vec<_>>(),
    })
}

fn terminal_skipped_details(
    requested: &[SourceTargetRecord],
    reason: &str,
    message: &str,
) -> Value {
    json!({
        "requested": requested.iter().map(target_details).collect::<Vec<_>>(),
        "processed": [],
        "skipped": requested.iter().map(|record| {
            terminal_details(record, reason, Some(message))
        }).collect::<Vec<_>>(),
        "conflicted": [],
    })
}

fn apply_synchronization_conflicts(
    details: &mut Value,
    requested: &[SourceTargetRecord],
    conflicts: &[SynchronizationConflict],
) {
    let conflict_ids = conflicts
        .iter()
        .map(|conflict| conflict.target_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(processed) = details["processed"].as_array_mut() {
        processed.retain(|entry| {
            entry
                .get("targetId")
                .and_then(Value::as_str)
                .is_none_or(|id| !conflict_ids.contains(id))
        });
    }
    let terminal = details["conflicted"]
        .as_array_mut()
        .expect("build details always contain conflicted array");
    for conflict in conflicts {
        let Some(record) = requested
            .iter()
            .find(|record| record.target.id == conflict.target_id)
        else {
            continue;
        };
        let mut entry = terminal_details(record, "synchronizationCasConflict", None);
        let object = entry
            .as_object_mut()
            .expect("terminal details are always an object");
        object.insert("message".to_string(), json!(conflict.reason));
        object.insert("expected".to_string(), json!(conflict.expected));
        object.insert("observedCurrent".to_string(), json!(conflict.current));
        terminal.push(entry);
    }
}

pub(crate) fn classify_build_result(
    requested: &[SourceTargetRecord],
    after_build: &BTreeMap<TargetId, SourceManifest>,
    report: Result<&RuntimeBuildReport, &str>,
) -> BuildReconciliation {
    let requested_details = requested.iter().map(target_details).collect::<Vec<_>>();
    let mut processed = Vec::new();
    let mut skipped = Vec::new();
    let mut conflicted = Vec::new();
    let mut synchronized = Vec::new();

    for record in requested {
        let terminal = match &report {
            Err(error) => {
                conflicted.push(terminal_details(record, "runnerJsonInvalid", Some(error)));
                continue;
            }
            Ok(report) => report,
        };
        let Some(source_set) = record.target.source_set.as_ref() else {
            conflicted.push(terminal_details(record, "sourceSetUnknown", None));
            continue;
        };
        let Some(step) = terminal
            .steps
            .iter()
            .find(|step| &step.source_set == source_set)
        else {
            skipped.push(terminal_details(record, "buildStepMissing", None));
            continue;
        };
        if matches!(step.mode, BuildStepMode::Skipped) {
            skipped.push(terminal_details(
                record,
                "buildStepSkipped",
                step.message.as_deref(),
            ));
            continue;
        }
        if !step.ok {
            skipped.push(terminal_details(
                record,
                "buildStepFailed",
                step.message.as_deref(),
            ));
            continue;
        }
        let Some(after) = after_build.get(&record.target.id) else {
            conflicted.push(terminal_details(record, "postBuildManifestMissing", None));
            continue;
        };
        if after != &record.current {
            conflicted.push(terminal_details_with_current(
                record,
                "sourceChangedDuringBuild",
                after,
            ));
            continue;
        }

        processed.push(terminal_details(record, "buildStepSucceeded", None));
        synchronized.push((record.target.id.clone(), after.clone()));
    }

    BuildReconciliation {
        synchronized,
        details: json!({
            "requested": requested_details,
            "processed": processed,
            "skipped": skipped,
            "conflicted": conflicted,
        }),
    }
}

pub(crate) fn merge_operation_details(base: Option<Value>, supplemental: Value) -> Value {
    let mut object = match base {
        Some(Value::Object(object)) => object,
        Some(value) => serde_json::Map::from_iter([("operationResult".to_string(), value)]),
        None => serde_json::Map::new(),
    };
    if let Value::Object(supplemental) = supplemental {
        object.extend(supplemental);
    }
    Value::Object(object)
}

pub(crate) fn classify_shadow_dump(
    requested: &[SourceTargetRecord],
    shadow: &BTreeMap<TargetId, SourceManifest>,
    force: bool,
) -> DumpReconciliation {
    let requested_details = requested.iter().map(target_details).collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut conflicted = Vec::new();

    for record in requested {
        let Some(shadow_manifest) = shadow.get(&record.target.id) else {
            conflicted.push(terminal_details(record, "shadowManifestMissing", None));
            continue;
        };
        let local_diverged = record.is_dirty();
        let infobase_diverged = !record.synchronized.matches_current(shadow_manifest);
        if !force && (local_diverged || infobase_diverged) {
            let reason = match (local_diverged, infobase_diverged) {
                (true, true) => "localAndInfobaseDiverged",
                (true, false) => "localSourceDiverged",
                (false, true) => "infobaseDiverged",
                (false, false) => unreachable!("divergence branch requires a divergence"),
            };
            conflicted.push(terminal_details_with_current(
                record,
                reason,
                shadow_manifest,
            ));
            continue;
        }
        candidates.push((record, shadow_manifest.clone()));
    }

    let blocked = !conflicted.is_empty();
    let mut processed = Vec::new();
    let mut skipped = Vec::new();
    let mut publish = Vec::new();
    let mut synchronized = Vec::new();
    if blocked {
        skipped.extend(
            candidates
                .into_iter()
                .map(|(record, _)| terminal_details(record, "batchConflict", None)),
        );
    } else {
        for (record, shadow_manifest) in candidates {
            let changed = shadow_manifest != record.current;
            let mut terminal = terminal_details(
                record,
                if force && changed {
                    "forcedInfobasePublication"
                } else {
                    "shadowMatchesSource"
                },
                None,
            );
            terminal
                .as_object_mut()
                .expect("terminal details are always an object")
                .insert("forced".to_string(), json!(force && changed));
            let object = terminal
                .as_object_mut()
                .expect("terminal details are always an object");
            object.insert(
                "observedShadow".to_string(),
                serde_json::to_value(&shadow_manifest).expect("source manifest is serializable"),
            );
            if force && changed {
                object.insert(
                    "publishedManifest".to_string(),
                    serde_json::to_value(&shadow_manifest)
                        .expect("source manifest is serializable"),
                );
            }
            processed.push(terminal);
            if changed {
                publish.push((record.target.id.clone(), shadow_manifest.clone()));
            }
            synchronized.push((record.target.id.clone(), shadow_manifest));
        }
    }

    DumpReconciliation {
        publish,
        synchronized,
        details: json!({
            "requested": requested_details,
            "processed": processed,
            "skipped": skipped,
            "conflicted": conflicted,
        }),
        blocked,
    }
}

pub(crate) fn affected_target_details(
    record: &SourceTargetRecord,
    preimage: &SourceManifest,
) -> Value {
    let mut details = target_details(record);
    let object = details
        .as_object_mut()
        .expect("target details are always an object");
    object.insert(
        "preManifest".to_string(),
        serde_json::to_value(preimage).expect("source manifest is serializable"),
    );
    object.insert(
        "postManifest".to_string(),
        serde_json::to_value(&record.current).expect("source manifest is serializable"),
    );
    if preimage.files.len() == 1 && record.current.files.len() == 1 {
        let pre_hash = preimage.files.values().next().and_then(fingerprint_hash);
        let post_hash = record
            .current
            .files
            .values()
            .next()
            .and_then(fingerprint_hash);
        object.insert("preHash".to_string(), json!(pre_hash));
        object.insert("postHash".to_string(), json!(post_hash));
    }
    details
}

fn target_details(record: &SourceTargetRecord) -> Value {
    json!({
        "targetId": record.target.id.as_str(),
        "sourceSet": record.target.source_set.as_ref().map(|name| name.as_str()),
        "sourceRoot": record.target.source_root.as_str(),
        "ownerSelector": record.target.owner_selector,
        "kind": match record.target.target_kind {
            SourceTargetKind::Module => "module",
            SourceTargetKind::MetadataOwner => "metadataOwner",
        },
        "current": record.current,
        "synchronized": record.synchronized,
    })
}

fn terminal_details(record: &SourceTargetRecord, reason: &str, message: Option<&str>) -> Value {
    let mut details = target_details(record);
    let object = details
        .as_object_mut()
        .expect("target details are always an object");
    object.insert("reason".to_string(), json!(reason));
    if let Some(message) = message {
        object.insert("message".to_string(), json!(message));
    }
    details
}

fn terminal_details_with_current(
    record: &SourceTargetRecord,
    reason: &str,
    current: &SourceManifest,
) -> Value {
    let mut details = terminal_details(record, reason, None);
    details
        .as_object_mut()
        .expect("target details are always an object")
        .insert(
            "observedCurrent".to_string(),
            serde_json::to_value(current).expect("source manifest is serializable"),
        );
    details
}

fn fingerprint_hash(fingerprint: &FileFingerprint) -> Option<&str> {
    fingerprint.sha256().map(|hash| hash.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source_sync::{
        BuildTerminalEntry, RelativeSourcePath, SourceSetName, SourceTarget, SourceTargetScope,
        SynchronizedManifest,
    };

    #[test]
    fn partial_build_success_advances_only_stable_successful_source_sets() {
        let main = record("main", "Demo", b"changed");
        let ext = record("ext", "Extension", b"changed-ext");
        let requested = vec![main.clone(), ext.clone()];
        let after = BTreeMap::from([
            (main.target.id.clone(), main.current.clone()),
            (ext.target.id.clone(), manifest(b"concurrent")),
        ]);
        let report = RuntimeBuildReport {
            envelope_ok: false,
            steps: vec![
                BuildTerminalEntry {
                    source_set: SourceSetName::new("main").unwrap(),
                    mode: BuildStepMode::Partial { file_count: 1 },
                    ok: true,
                    message: None,
                },
                BuildTerminalEntry {
                    source_set: SourceSetName::new("ext").unwrap(),
                    mode: BuildStepMode::Full,
                    ok: true,
                    message: None,
                },
            ],
        };

        let classified = classify_build_result(&requested, &after, Ok(&report));

        assert_eq!(classified.synchronized.len(), 1);
        assert_eq!(classified.synchronized[0].0, main.target.id);
        assert_eq!(classified.details["processed"].as_array().unwrap().len(), 1);
        assert_eq!(
            classified.details["conflicted"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            classified.details["conflicted"][0]["reason"],
            "sourceChangedDuringBuild"
        );
    }

    #[test]
    fn build_reconciliation_matches_source_set_identity_without_trimming() {
        let record = record(" main ", "Demo", b"changed");
        let after = BTreeMap::from([(record.target.id.clone(), record.current.clone())]);
        let report = RuntimeBuildReport {
            envelope_ok: true,
            steps: vec![BuildTerminalEntry {
                source_set: SourceSetName::new(" main ").unwrap(),
                mode: BuildStepMode::Full,
                ok: true,
                message: None,
            }],
        };

        let classified = classify_build_result(std::slice::from_ref(&record), &after, Ok(&report));

        assert_eq!(classified.synchronized.len(), 1);
        assert_eq!(classified.synchronized[0].0, record.target.id);
        assert!(classified.details["skipped"].as_array().unwrap().is_empty());
    }

    #[test]
    fn invalid_json_skipped_and_failed_steps_never_advance_targets() {
        let record = record("main", "Demo", b"changed");
        let after = BTreeMap::from([(record.target.id.clone(), record.current.clone())]);

        let invalid = classify_build_result(std::slice::from_ref(&record), &after, Err("bad JSON"));
        assert!(invalid.synchronized.is_empty());
        assert_eq!(
            invalid.details["conflicted"][0]["reason"],
            "runnerJsonInvalid"
        );

        for (mode, ok, reason) in [
            (BuildStepMode::Skipped, false, "buildStepSkipped"),
            (BuildStepMode::Full, false, "buildStepFailed"),
        ] {
            let report = RuntimeBuildReport {
                envelope_ok: false,
                steps: vec![BuildTerminalEntry {
                    source_set: SourceSetName::new("main").unwrap(),
                    mode,
                    ok,
                    message: Some("runner terminal message".to_string()),
                }],
            };
            let classified =
                classify_build_result(std::slice::from_ref(&record), &after, Ok(&report));
            assert!(classified.synchronized.is_empty());
            assert_eq!(classified.details["skipped"][0]["reason"], reason);
        }
    }

    #[test]
    fn affected_target_keeps_raw_single_file_hashes_and_merges_patch_details() {
        let record = record("main", "Demo", b"post\r\n");
        let pre = manifest(b"\xef\xbb\xbfpre\n");
        let affected = affected_target_details(&record, &pre);
        assert_ne!(affected["preHash"], affected["postHash"]);

        let merged = merge_operation_details(
            Some(json!({"selector": {"kind": "anchor"}, "applied": true})),
            json!({"affectedTargets": [affected]}),
        );
        assert_eq!(merged["selector"]["kind"], "anchor");
        assert_eq!(merged["affectedTargets"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn shadow_dump_blocks_entire_batch_before_publication_on_any_divergence() {
        let clean = record("main", "Clean", b"same");
        let divergent = record("main", "Divergent", b"local");
        let requested = vec![clean.clone(), divergent.clone()];
        let shadow = BTreeMap::from([
            (clean.target.id.clone(), clean.current.clone()),
            (divergent.target.id.clone(), manifest_for(&divergent, b"ib")),
        ]);

        let classified = classify_shadow_dump(&requested, &shadow, false);

        assert!(classified.blocked);
        assert!(classified.publish.is_empty());
        assert!(classified.synchronized.is_empty());
        assert_eq!(
            classified.details["conflicted"].as_array().unwrap().len(),
            1
        );
        assert_eq!(classified.details["skipped"][0]["reason"], "batchConflict");
    }

    #[test]
    fn explicit_force_selects_only_changed_shadow_manifests_for_publication() {
        let record = record("main", "Demo", b"local");
        let shadow_manifest = manifest_for(&record, b"ib");
        let shadow = BTreeMap::from([(record.target.id.clone(), shadow_manifest.clone())]);

        let classified = classify_shadow_dump(std::slice::from_ref(&record), &shadow, true);

        assert!(!classified.blocked);
        assert_eq!(classified.publish.len(), 1);
        assert_eq!(classified.synchronized.len(), 1);
        assert_eq!(classified.publish[0].1, shadow_manifest);
        assert_eq!(classified.details["processed"][0]["forced"], true);
    }

    #[test]
    fn publication_recovery_error_keeps_change_signal_and_structured_terminals() {
        let record = record("main", "Demo", b"local");
        let mut outcome = AdapterOutcome::ok("runner succeeded");
        let mut details = json!({
            "requested": [target_details(&record)],
            "processed": [terminal_details(&record, "forcedInfobasePublication", None)],
            "skipped": [],
            "conflicted": [],
        });
        let path = record.current.files.keys().next().unwrap().clone();
        let error = PublicationError {
            reason: "publicationRecoveryRequired",
            message: "injected rollback failure".to_string(),
            affected_paths: vec![path],
            source_may_have_changed: true,
            recovery_required: true,
        };

        apply_publication_failure(
            &mut outcome,
            &mut details,
            std::slice::from_ref(&record),
            &error,
        );

        assert!(!outcome.ok);
        assert!(!outcome.changes.is_empty());
        assert_eq!(details["requested"].as_array().unwrap().len(), 1);
        assert_eq!(details["processed"], json!([]));
        assert_eq!(details["skipped"], json!([]));
        assert_eq!(
            details["conflicted"][0]["reason"],
            "publicationRecoveryRequired"
        );
    }

    #[test]
    fn postpublication_finalization_failure_preserves_committed_change_signal() {
        let record = record("main", "Demo", b"local");
        let mut outcome = AdapterOutcome::ok("runner succeeded");
        outcome
            .changes
            .push("published src/ConfigDumpInfo.xml from shadow dump".to_string());
        let mut details = json!({
            "requested": [target_details(&record)],
            "processed": [terminal_details(&record, "forcedInfobasePublication", None)],
            "skipped": [],
            "conflicted": [],
        });

        apply_dump_finalization_failure(
            &mut outcome,
            &mut details,
            std::slice::from_ref(&record),
            "synchronizationStateFailed",
            "injected state failure",
        );

        assert!(!outcome.ok);
        assert_eq!(outcome.changes.len(), 1);
        assert_eq!(
            outcome.summary,
            "source changed but sync finalization failed"
        );
        assert_eq!(details["processed"], json!([]));
        assert_eq!(
            details["conflicted"][0]["reason"],
            "synchronizationStateFailed"
        );
    }

    #[test]
    fn pinned_primary_config_removes_base_path_and_absolutizes_source_set_paths() {
        let original_config_dir = Path::new("/workspace/project");
        let mut config: YamlValue = serde_yaml::from_str(
            r#"
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: src
  - name: external
    type: EXTERNAL_PROCESSOR
    path: /shared/external
"#,
        )
        .unwrap();

        normalize_pinned_build_yaml(&mut config, original_config_dir, false).unwrap();

        let mapping = config.as_mapping().unwrap();
        assert!(!mapping.contains_key(YamlValue::String("basePath".to_string())));
        let source_sets = mapping
            .get(YamlValue::String("source-set".to_string()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(
            source_sets[0]
                .as_mapping()
                .unwrap()
                .get(YamlValue::String("path".to_string()))
                .unwrap()
                .as_str(),
            Some("/workspace/project/src")
        );
        assert_eq!(
            source_sets[1]
                .as_mapping()
                .unwrap()
                .get(YamlValue::String("path".to_string()))
                .unwrap()
                .as_str(),
            Some("/shared/external")
        );
    }

    #[test]
    fn pinned_primary_config_uses_legacy_base_path_without_serializing_it() {
        let mut config: YamlValue = serde_yaml::from_str(
            r#"
format: DESIGNER
basePath: legacy
source-set:
  - name: main
    type: CONFIGURATION
    path: link/../src
"#,
        )
        .unwrap();

        normalize_pinned_build_yaml(&mut config, Path::new("/workspace/project"), false).unwrap();

        let mapping = config.as_mapping().unwrap();
        assert!(!mapping.contains_key(YamlValue::String("basePath".to_string())));
        let source_set = mapping
            .get(YamlValue::String("source-set".to_string()))
            .unwrap()
            .as_sequence()
            .unwrap()
            .first()
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(
            source_set
                .get(YamlValue::String("path".to_string()))
                .unwrap()
                .as_str(),
            Some("/workspace/project/legacy/link/../src")
        );
    }

    #[test]
    fn pinned_primary_config_rejects_malformed_base_path_and_source_set_entries() {
        let mut malformed_base_path: YamlValue = serde_yaml::from_str(
            r#"
basePath: 42
source-set: []
"#,
        )
        .unwrap();
        let error = normalize_pinned_build_yaml(
            &mut malformed_base_path,
            Path::new("/workspace/project"),
            false,
        )
        .unwrap_err();
        assert!(error.contains("basePath must be a string"), "{error}");

        let mut malformed_source_set: YamlValue = serde_yaml::from_str(
            r#"
source-set:
  - name: main
"#,
        )
        .unwrap();
        let error = normalize_pinned_build_yaml(
            &mut malformed_source_set,
            Path::new("/workspace/project"),
            false,
        )
        .unwrap_err();
        assert!(error.contains("source-set[0].path"), "{error}");
    }

    fn record(source_set: &str, name: &str, bytes: &[u8]) -> SourceTargetRecord {
        let path =
            RelativeSourcePath::new(format!("src/CommonModules/{name}/Ext/Module.bsl")).unwrap();
        let current = SourceManifest {
            files: BTreeMap::from([(path.clone(), FileFingerprint::present(bytes))]),
        };
        SourceTargetRecord {
            target: SourceTarget {
                id: TargetId::new(format!("module:{source_set}:{name}")).unwrap(),
                target_kind: SourceTargetKind::Module,
                source_set: Some(SourceSetName::new(source_set).unwrap()),
                source_root: RelativeSourcePath::new("src").unwrap(),
                owner_selector: format!("CommonModule:{name}"),
                scope: SourceTargetScope::Module { path },
            },
            synchronized: SynchronizedManifest::known(&current),
            current,
        }
    }

    fn manifest(bytes: &[u8]) -> SourceManifest {
        SourceManifest {
            files: BTreeMap::from([(
                RelativeSourcePath::new("src/CommonModules/Demo/Ext/Module.bsl").unwrap(),
                FileFingerprint::present(bytes),
            )]),
        }
    }

    fn manifest_for(record: &SourceTargetRecord, bytes: &[u8]) -> SourceManifest {
        SourceManifest {
            files: record
                .current
                .files
                .keys()
                .cloned()
                .map(|path| (path, FileFingerprint::present(bytes)))
                .collect(),
        }
    }
}

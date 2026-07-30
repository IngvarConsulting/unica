use crate::domain::cache::{path_for_report, CacheAccess, CacheImpact, CacheReport};
use crate::domain::events::DomainEvent;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::workspace_index::bsl_index_is_ready;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;

pub(crate) const CACHE_STATE_CONFLICT_RETRIES: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceState {
    workspace_root: String,
    workspace_epoch: u64,
    caches: BTreeMap<String, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    status: CacheStatus,
    epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum CacheStatus {
    Fresh,
    Stale,
}

pub struct WorkspaceStateRepository {
    state_path: PathBuf,
}

struct PlannedWorkspaceStateFile {
    path: PathBuf,
    preimage: Option<Vec<u8>>,
    postimage: Vec<u8>,
}

struct PlannedWorkspaceReport {
    report: CacheReport,
    state: Option<PlannedWorkspaceStateFile>,
    metadata: Vec<PlannedWorkspaceStateFile>,
}

pub(crate) struct StagedCacheEffects {
    report: CacheReport,
    publication_paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct CacheEffectsStagingError {
    message: String,
    publication_paths: Vec<PathBuf>,
}

impl StagedCacheEffects {
    pub(crate) fn publication_paths(&self) -> &[PathBuf] {
        &self.publication_paths
    }

    pub(crate) fn into_report(self) -> CacheReport {
        self.report
    }
}

impl CacheEffectsStagingError {
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn publication_paths(&self) -> &[PathBuf] {
        &self.publication_paths
    }
}

impl PlannedWorkspaceReport {
    fn publication_paths(&self) -> Vec<PathBuf> {
        self.metadata
            .iter()
            .chain(self.state.iter())
            .map(|file| file.path.clone())
            .collect()
    }

    fn stage(self, transaction: &mut CompileTransaction) -> Result<CacheReport, String> {
        for file in self.metadata.into_iter().chain(self.state) {
            let PlannedWorkspaceStateFile {
                path,
                preimage,
                postimage,
            } = file;
            let expected_existing = preimage.is_some();
            let staged = match preimage {
                Some(preimage) => transaction.replace_bytes(&path, preimage, postimage),
                None => transaction.create_bytes(&path, postimage),
            };
            if let Err(error) = staged {
                if expected_existing
                    && fs::symlink_metadata(&path)
                        .is_err_and(|source| source.kind() == ErrorKind::NotFound)
                {
                    return Err(format!(
                        "cache publication target disappeared while staging: {}",
                        path.display()
                    ));
                }
                return Err(error);
            }
        }
        Ok(self.report)
    }
}

impl WorkspaceStateRepository {
    pub fn new(context: &WorkspaceContext) -> Self {
        Self {
            state_path: context.cache_root.join("state.json"),
        }
    }

    pub fn report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, String> {
        for attempt in 0..=CACHE_STATE_CONFLICT_RETRIES {
            let plan = self.plan_report(context, events, dry_run, cache_access)?;
            if plan.state.is_none() && plan.metadata.is_empty() {
                return Ok(plan.report);
            }
            let publication_paths = plan.publication_paths();
            let result = (|| -> Result<CacheReport, String> {
                let mut transaction = CompileTransaction::new();
                let mut report = plan.stage(&mut transaction)?;
                let commit = transaction.commit()?;
                report.publication_warnings = cache_cleanup_warnings(commit.cleanup_warnings);
                Ok(report)
            })();
            match result {
                Ok(report) => return Ok(report),
                Err(error)
                    if attempt < CACHE_STATE_CONFLICT_RETRIES
                        && is_retryable_cache_state_conflict(&error, &publication_paths) =>
                {
                    std::thread::yield_now();
                }
                Err(error) => {
                    return Err(format!(
                        "failed to atomically publish Unica cache state: {error}"
                    ));
                }
            }
        }
        unreachable!("bounded cache state retry loop always returns")
    }

    pub(crate) fn stage_event_effects(
        &self,
        transaction: &mut CompileTransaction,
        context: &WorkspaceContext,
        events: &[DomainEvent],
    ) -> Result<StagedCacheEffects, CacheEffectsStagingError> {
        if events.is_empty() {
            return Err(CacheEffectsStagingError {
                message: "cannot stage cache effects without a domain event".to_string(),
                publication_paths: Vec::new(),
            });
        }
        let plan = self
            .plan_report(context, events, false, CacheAccess::default())
            .map_err(|message| CacheEffectsStagingError {
                message,
                publication_paths: Vec::new(),
            })?;
        let publication_paths = plan.publication_paths();
        let report = plan
            .stage(transaction)
            .map_err(|message| CacheEffectsStagingError {
                message,
                publication_paths: publication_paths.clone(),
            })?;
        Ok(StagedCacheEffects {
            report,
            publication_paths,
        })
    }

    fn plan_report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<PlannedWorkspaceReport, String> {
        let impact = CacheImpact::from_events(events);
        let (mut state, state_preimage) = self.load_snapshot(context)?;
        let mut invalidated = sorted(impact.invalidated);
        let mut refreshed = sorted(impact.eager_refresh);
        let mut lazy_rebuilt = Vec::new();
        let mut metadata = Vec::new();
        let mut write_state = false;

        if !events.is_empty() && !dry_run {
            for name in &invalidated {
                state.caches.insert(
                    name.clone(),
                    CacheEntry {
                        status: CacheStatus::Stale,
                        epoch: context.workspace_epoch,
                    },
                );
            }
            for name in &refreshed {
                state.caches.insert(
                    name.clone(),
                    CacheEntry {
                        status: CacheStatus::Fresh,
                        epoch: context.workspace_epoch,
                    },
                );
                metadata.push(self.plan_cache_metadata(context, name, "eager")?);
            }
            state.workspace_epoch = context.workspace_epoch;
            write_state = true;
        }

        if dry_run {
            refreshed.clear();
        }
        if events.is_empty() {
            invalidated.clear();
            refreshed.clear();
        }

        if events.is_empty() && !dry_run {
            for name in cache_access.reads {
                if *name == "bsl_index" {
                    state.caches.insert(
                        (*name).to_string(),
                        CacheEntry {
                            status: if bsl_index_is_ready(context) {
                                CacheStatus::Fresh
                            } else {
                                CacheStatus::Stale
                            },
                            epoch: context.workspace_epoch,
                        },
                    );
                    continue;
                }
                let is_stale = state
                    .caches
                    .get(*name)
                    .map(|entry| entry.status == CacheStatus::Stale)
                    .unwrap_or_else(|| is_lazy_cache(name));
                if is_stale && is_lazy_cache(name) {
                    state.caches.insert(
                        (*name).to_string(),
                        CacheEntry {
                            status: CacheStatus::Fresh,
                            epoch: context.workspace_epoch,
                        },
                    );
                    metadata.push(self.plan_cache_metadata(context, name, "lazy")?);
                    lazy_rebuilt.push((*name).to_string());
                }
            }
            if !lazy_rebuilt.is_empty() {
                state.workspace_epoch = context.workspace_epoch;
                write_state = true;
            }
        }

        let mut stale = Vec::new();
        let mut fresh = Vec::new();
        for (name, entry) in &state.caches {
            match entry.status {
                CacheStatus::Fresh => fresh.push(name.clone()),
                CacheStatus::Stale => stale.push(name.clone()),
            }
        }

        let report = CacheReport {
            mode: if events.is_empty() {
                "read".to_string()
            } else if dry_run {
                "dry-run".to_string()
            } else {
                "applied".to_string()
            },
            root: path_for_report(&context.cache_root),
            workspace_epoch: context.workspace_epoch,
            events: events
                .iter()
                .map(|event| event.name().to_string())
                .collect(),
            invalidated,
            refreshed,
            lazy_rebuilt,
            stale,
            fresh,
            publication_warnings: Vec::new(),
        };
        let state = if write_state {
            let mut postimage =
                serde_json::to_vec_pretty(&state).map_err(|error| error.to_string())?;
            postimage.push(b'\n');
            Some(PlannedWorkspaceStateFile {
                path: self.state_path.clone(),
                preimage: state_preimage,
                postimage,
            })
        } else {
            None
        };
        Ok(PlannedWorkspaceReport {
            report,
            state,
            metadata,
        })
    }

    fn load_snapshot(
        &self,
        context: &WorkspaceContext,
    ) -> Result<(WorkspaceState, Option<Vec<u8>>), String> {
        match fs::read(&self.state_path) {
            Ok(bytes) => {
                let state =
                    serde_json::from_slice(&bytes).unwrap_or_else(|_| default_state(context));
                Ok((state, Some(bytes)))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok((default_state(context), None)),
            Err(error) => Err(format!(
                "failed to read Unica cache state {}: {error}",
                self.state_path.display()
            )),
        }
    }

    fn plan_cache_metadata(
        &self,
        context: &WorkspaceContext,
        name: &str,
        mode: &str,
    ) -> Result<PlannedWorkspaceStateFile, String> {
        let path = context
            .cache_root
            .join("caches")
            .join(format!("{name}.json"));
        let text = serde_json::json!({
            "name": name,
            "mode": mode,
            "workspaceEpoch": context.workspace_epoch,
        });
        let mut postimage = serde_json::to_vec_pretty(&text).map_err(|error| error.to_string())?;
        postimage.push(b'\n');
        let preimage = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "failed to read Unica cache metadata {}: {error}",
                    path.display()
                ));
            }
        };
        Ok(PlannedWorkspaceStateFile {
            path,
            preimage,
            postimage,
        })
    }
}

fn default_state(context: &WorkspaceContext) -> WorkspaceState {
    WorkspaceState {
        workspace_root: context.workspace_root.display().to_string(),
        workspace_epoch: context.workspace_epoch,
        caches: BTreeMap::new(),
    }
}

fn sorted(values: std::collections::BTreeSet<String>) -> Vec<String> {
    values.into_iter().collect()
}

fn is_lazy_cache(name: &str) -> bool {
    matches!(name, "bsl_diagnostics")
}

fn cache_cleanup_warnings(cleanup_warnings: Vec<String>) -> Vec<String> {
    if cleanup_warnings.is_empty() {
        return Vec::new();
    }
    let mut stderr = std::io::stderr().lock();
    for warning in &cleanup_warnings {
        let _ = writeln!(
            stderr,
            "Unica cache state committed with cleanup warning: {warning}"
        );
    }
    vec![format!(
        "cache state committed, but {} recovery or staging artifact(s) require cleanup; inspect the Unica server log",
        cleanup_warnings.len()
    )]
}

pub(crate) fn is_retryable_cache_state_conflict(error: &str, paths: &[PathBuf]) -> bool {
    let error = error.trim_end();
    let Some(conflict_prefix) = paths.iter().find_map(|path| {
        let suffix = format!(": {}", path.to_string_lossy());
        error.strip_suffix(&suffix)
    }) else {
        return false;
    };
    if [
        "; rollback encountered:",
        "; cleanup encountered:",
        "; cleanup warnings:",
    ]
    .iter()
    .any(|marker| conflict_prefix.contains(marker))
    {
        return false;
    }
    [
        "create-only compile target is already",
        "create-only publication target already exists",
        "publication target differs from the expected preimage",
        "publication target metadata changed before commit",
        "replacement target changed while planning",
        "registration target changed after planning",
        "registration target metadata changed after planning",
        "registration target disappeared before commit",
        "cache publication target disappeared while staging",
    ]
    .iter()
    .any(|marker| conflict_prefix.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::events::{DomainEvent, DomainEventKind};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn bsl_index_read_reflects_real_index_status_instead_of_lazy_rebuild() {
        let root = temp_root("unica-cache-lazy");
        fs::create_dir_all(&root).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".cache"),
            workspace_epoch: 1,
        };
        let repo = WorkspaceStateRepository::new(&context);

        let invalidation = repo
            .report(
                &context,
                &[DomainEvent::new(
                    DomainEventKind::ModuleChanged,
                    "Module.bsl",
                )],
                false,
                CacheAccess::default(),
            )
            .unwrap();
        assert!(invalidation.stale.contains(&"bsl_index".to_string()));

        let reported = repo
            .report(
                &context,
                &[],
                false,
                CacheAccess {
                    reads: &["bsl_index"],
                    writes: &[],
                },
            )
            .unwrap();
        assert!(reported.lazy_rebuilt.is_empty());
        assert!(reported.stale.contains(&"bsl_index".to_string()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_cache_plan_cannot_overwrite_a_concurrent_event_projection() {
        let root = temp_root("unica-cache-concurrent-plan");
        fs::create_dir_all(&root).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".cache"),
            workspace_epoch: 1,
        };
        let repo = WorkspaceStateRepository::new(&context);
        let module_event = DomainEvent::new(DomainEventKind::ModuleChanged, "Module.bsl");
        let form_event = DomainEvent::new(DomainEventKind::FormChanged, "Form.xml");
        let mut module_transaction = CompileTransaction::new();
        repo.stage_event_effects(
            &mut module_transaction,
            &context,
            std::slice::from_ref(&module_event),
        )
        .unwrap();
        let mut stale_form_transaction = CompileTransaction::new();
        repo.stage_event_effects(
            &mut stale_form_transaction,
            &context,
            std::slice::from_ref(&form_event),
        )
        .unwrap();

        module_transaction.commit().unwrap();
        let conflict = stale_form_transaction
            .commit()
            .expect_err("a stale cache plan must not overwrite the first event");
        assert!(
            conflict.contains("exists") || conflict.contains("changed"),
            "{conflict}"
        );

        let mut retried_form_transaction = CompileTransaction::new();
        repo.stage_event_effects(
            &mut retried_form_transaction,
            &context,
            std::slice::from_ref(&form_event),
        )
        .unwrap();
        retried_form_transaction.commit().unwrap();

        let (state, _) = repo.load_snapshot(&context).unwrap();
        assert_eq!(
            state.caches.get("bsl_index").map(|entry| entry.status),
            Some(CacheStatus::Stale)
        );
        assert_eq!(
            state.caches.get("form_graph").map(|entry| entry.status),
            Some(CacheStatus::Stale)
        );
        assert_eq!(
            state.caches.get("metadata_graph").map(|entry| entry.status),
            Some(CacheStatus::Fresh)
        );
        assert_eq!(
            state
                .caches
                .get("bsl_diagnostics")
                .map(|entry| entry.status),
            Some(CacheStatus::Stale)
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cache_conflict_classifier_requires_an_exact_planned_cache_path() {
        let root = PathBuf::from("/workspace");
        let state = root.join("state.json");
        let source = root.join("src/CommonModules/Shared/Ext/Module.bsl");

        assert!(is_retryable_cache_state_conflict(
            &format!(
                "create-only compile target is already an existing file: {}",
                state.display()
            ),
            std::slice::from_ref(&state),
        ));
        let recovery_named_state = PathBuf::from("/workspace/recovery-project/state.json");
        assert!(is_retryable_cache_state_conflict(
            &format!(
                "create-only publication target already exists: {}",
                recovery_named_state.display()
            ),
            std::slice::from_ref(&recovery_named_state),
        ));
        assert!(!is_retryable_cache_state_conflict(
            &format!(
                "publication target differs from the expected preimage: {}",
                source.display()
            ),
            std::slice::from_ref(&state),
        ));
        assert!(!is_retryable_cache_state_conflict(
            &format!(
                "publication target differs from the expected preimage: {}.backup",
                state.display()
            ),
            std::slice::from_ref(&state),
        ));
        assert!(!is_retryable_cache_state_conflict(
            &format!(
                "publication target differs from the expected preimage: {}; rollback encountered: recovery failed",
                state.display()
            ),
            std::slice::from_ref(&state),
        ));
        let warnings =
            cache_cleanup_warnings(vec![format!("{} could not be removed", state.display())]);
        assert_eq!(warnings.len(), 1);
        assert!(!warnings[0].contains(state.to_string_lossy().as_ref()));
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}

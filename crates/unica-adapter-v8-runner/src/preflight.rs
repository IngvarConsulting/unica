use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use unica_runner::attempt::RuntimeWorkspaceObservation;
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::paths::path_lock_identity;
use unica_workspace::read::read_root_relative_regular_file;

const RUNTIME_CONFIG_MAX_BYTES: usize = 64 * 1024 * 1024;
const LOCAL_RUNTIME_CONFIG_FILE_NAME: &str = "v8project.local.yaml";

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static BEFORE_REAUTHORIZATION_HOOK: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(any(test, feature = "test-support"))]
pub struct BeforeReauthorizationHookGuard;

#[cfg(any(test, feature = "test-support"))]
impl Drop for BeforeReauthorizationHookGuard {
    fn drop(&mut self) {
        BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn set_before_reauthorization_hook_for_test(
    hook: impl FnMut() + 'static,
) -> BeforeReauthorizationHookGuard {
    BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none(), "runtime build reauthorization hook leaked");
        *slot = Some(Box::new(hook));
    });
    BeforeReauthorizationHookGuard
}

fn run_before_reauthorization_hook() {
    #[cfg(any(test, feature = "test-support"))]
    BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook();
        }
    });
}

pub struct RuntimeInvocationPlan {
    pub args: Map<String, Value>,
    pub warnings: Vec<String>,
    pub build_preflight: Option<RuntimeBuildPreflight>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeBuildPreflight {
    cwd: PathBuf,
    workspace_root: PathBuf,
    workspace_epoch: u64,
    config: PathBuf,
    config_sha256: [u8; 32],
    local_config_sha256: Option<[u8; 32]>,
}

impl RuntimeBuildPreflight {
    pub fn capture(
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        observer: &dyn RuntimeWorkspaceObservation,
    ) -> Result<Self, String> {
        let current = observer.discover(&context.cwd).map_err(|error| {
            format!("runtime build could not rediscover the workspace: {error}")
        })?;
        if path_lock_identity(&current.workspace_root)
            != path_lock_identity(&context.workspace_root)
            || current.workspace_epoch != context.workspace_epoch
        {
            return Err(workspace_identity_changed_error());
        }
        let config = resolved_runtime_config(args, &current)?;
        let config_sha256 = runtime_config_digest(&config)?;
        let local_config_sha256 = runtime_local_config_digest(&config)?;
        Ok(Self {
            cwd: context.cwd.clone(),
            workspace_root: current.workspace_root,
            workspace_epoch: current.workspace_epoch,
            config,
            config_sha256,
            local_config_sha256,
        })
    }

    pub(crate) fn scope_evidence(&self) -> Vec<u8> {
        // The existing strict serialized DTO is the immutable admitted evidence;
        // this is serialization only, never a second parser/authorization path.
        serde_json::to_vec(self).expect("runtime preflight DTO serialization is infallible")
    }

    pub fn reauthorize_current_workspace(
        &self,
        observer: &dyn RuntimeWorkspaceObservation,
    ) -> Result<(), String> {
        run_before_reauthorization_hook();
        let current = observer.discover(&self.cwd).map_err(|error| {
            format!("runtime build could not rediscover the workspace before launch: {error}")
        })?;
        if path_lock_identity(&self.workspace_root) != path_lock_identity(&current.workspace_root)
            || self.workspace_epoch != current.workspace_epoch
        {
            return Err(workspace_identity_changed_error());
        }
        let current_config = normalize_path_identity(&self.config).map_err(|error| {
            format!(
                "runtime build could not resolve config `{}` before launch: {error}",
                self.config.display()
            )
        })?;
        if current_config != self.config
            || runtime_config_digest(&current_config)? != self.config_sha256
        {
            return Err("runtime build project config changed before v8-runner launch".to_string());
        }
        if runtime_local_config_digest(&current_config)? != self.local_config_sha256 {
            return Err(
                "runtime build local project config changed before v8-runner launch".to_string(),
            );
        }
        Ok(())
    }
}

/// Normalizes build identity without inspecting configuration support. A
/// default build remains a default v8-runner build; Unica decides about one
/// full fallback only from the structured result of an actually failed partial
/// step (#404).
pub fn plan_runtime_invocation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    observer: &dyn RuntimeWorkspaceObservation,
) -> Result<RuntimeInvocationPlan, String> {
    let mut planned_args = args.clone();
    if args.get("operation").and_then(Value::as_str) != Some("build") {
        return Ok(RuntimeInvocationPlan {
            args: planned_args,
            warnings: Vec::new(),
            build_preflight: None,
        });
    }
    if args.contains_key("fullRebuild")
        && args.get("fullRebuild").and_then(Value::as_bool).is_none()
    {
        return Err("operation `build` argument `fullRebuild` must be boolean".to_string());
    }

    let config = resolved_runtime_config(args, context)?;
    planned_args.insert(
        "config".to_string(),
        Value::String(
            config
                .to_str()
                .ok_or_else(|| {
                    "runtime build config path is not valid UTF-8 for v8-runner".to_string()
                })?
                .to_string(),
        ),
    );
    if let Some(source_set) = args.get("sourceSet") {
        let source_set = source_set
            .as_str()
            .ok_or_else(|| "operation `build` argument `sourceSet` must be string".to_string())?
            .trim();
        if source_set.is_empty() {
            return Err("operation `build` argument `sourceSet` must not be empty".to_string());
        }
        planned_args.insert(
            "sourceSet".to_string(),
            Value::String(source_set.to_string()),
        );
    }

    let explicit_full = args.get("fullRebuild").and_then(Value::as_bool) == Some(true);
    let build_preflight = (!explicit_full)
        .then(|| RuntimeBuildPreflight::capture(&planned_args, context, observer))
        .transpose()?;
    Ok(RuntimeInvocationPlan {
        args: planned_args,
        warnings: if explicit_full {
            Vec::new()
        } else {
            // `--json-message` replaces the runner's streamed text with one
            // envelope printed at exit, which is what the classifier needs and
            // what leaves the job logs empty until the build ends. A caller
            // polling `unica.runtime.job.logs` has to be told that up front.
            vec![
                "v8-runner runs its normal build strategy first and reports one structured result when the process exits, so a durable build streams no progress into its logs; if that result proves a failed partial platform step, `unica.runtime.job.start` will retry once with a full rebuild"
                    .to_string(),
            ]
        },
        build_preflight,
    })
}

fn resolved_runtime_config(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let configured = match args.get("config") {
        Some(Value::String(path)) => PathBuf::from(path),
        Some(_) => return Err("operation `build` argument `config` must be string".to_string()),
        None => context.workspace_root.join("v8project.yaml"),
    };
    let effective = if configured.is_absolute() {
        configured
    } else {
        context.cwd.join(configured)
    };
    let config = normalize_path_identity(&effective).map_err(|error| {
        format!(
            "runtime build could not resolve config `{}`: {error}",
            effective.display()
        )
    })?;
    let workspace_root = normalize_path_identity(&context.workspace_root).map_err(|error| {
        format!(
            "runtime build could not resolve workspace root `{}`: {error}",
            context.workspace_root.display()
        )
    })?;
    if !config.starts_with(&workspace_root) {
        return Err(format!(
            "runtime build config `{}` is outside the workspace root `{}`",
            config.display(),
            workspace_root.display()
        ));
    }
    Ok(config)
}

fn runtime_config_digest(path: &Path) -> Result<[u8; 32], String> {
    let root = path.parent().ok_or_else(|| {
        format!(
            "runtime build config `{}` has no parent directory",
            path.display()
        )
    })?;
    let read = read_root_relative_regular_file(root, path, RUNTIME_CONFIG_MAX_BYTES, |_| {})
        .map_err(|error| {
            format!(
                "runtime build config `{}` must be a bounded regular file: {error}",
                path.display()
            )
        })?;
    Ok(Sha256::digest(&read.bytes).into())
}

fn runtime_local_config_digest(config: &Path) -> Result<Option<[u8; 32]>, String> {
    let root = config.parent().ok_or_else(|| {
        format!(
            "runtime build config `{}` has no parent directory",
            config.display()
        )
    })?;
    let local_config = root.join(LOCAL_RUNTIME_CONFIG_FILE_NAME);
    match read_root_relative_regular_file(root, &local_config, RUNTIME_CONFIG_MAX_BYTES, |_| {}) {
        Ok(read) => Ok(Some(Sha256::digest(&read.bytes).into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "runtime build local project config `{}` must be a bounded regular file: {error}",
            local_config.display()
        )),
    }
}

fn workspace_identity_changed_error() -> String {
    "runtime build workspace identity changed before v8-runner launch".to_string()
}

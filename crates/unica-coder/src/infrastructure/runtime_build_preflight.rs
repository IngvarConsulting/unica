use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::path_lock_identity;
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace::discover_workspace;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const RUNTIME_CONFIG_MAX_BYTES: usize = 64 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    static BEFORE_REAUTHORIZATION_HOOK: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        std::cell::RefCell::new(None);
    static AFTER_REAUTHORIZATION_HOOK: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) struct BeforeReauthorizationHookGuard;

#[cfg(test)]
impl Drop for BeforeReauthorizationHookGuard {
    fn drop(&mut self) {
        BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
#[must_use]
pub(crate) fn set_before_reauthorization_hook_for_test(
    hook: impl FnMut() + 'static,
) -> BeforeReauthorizationHookGuard {
    BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none(), "runtime build reauthorization hook leaked");
        *slot = Some(Box::new(hook));
    });
    BeforeReauthorizationHookGuard
}

#[cfg(test)]
pub(crate) struct AfterReauthorizationHookGuard;

#[cfg(test)]
impl Drop for AfterReauthorizationHookGuard {
    fn drop(&mut self) {
        AFTER_REAUTHORIZATION_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
#[must_use]
pub(crate) fn set_after_reauthorization_hook_for_test(
    hook: impl FnMut() + 'static,
) -> AfterReauthorizationHookGuard {
    AFTER_REAUTHORIZATION_HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none(), "runtime build reauthorization hook leaked");
        *slot = Some(Box::new(hook));
    });
    AfterReauthorizationHookGuard
}

fn run_after_reauthorization_hook() {
    #[cfg(test)]
    AFTER_REAUTHORIZATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook();
        }
    });
}

fn run_before_reauthorization_hook() {
    #[cfg(test)]
    BEFORE_REAUTHORIZATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook();
        }
    });
}

pub(crate) struct RuntimeInvocationPlan {
    pub(crate) args: Map<String, Value>,
    pub(crate) warnings: Vec<String>,
    pub(crate) build_preflight: Option<RuntimeBuildPreflight>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeBuildPreflight {
    cwd: PathBuf,
    workspace_root: PathBuf,
    workspace_epoch: u64,
    config: PathBuf,
    config_sha256: [u8; 32],
}

impl RuntimeBuildPreflight {
    pub(crate) fn capture(
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<Self, String> {
        let current = discover_workspace(Some(context.cwd.clone())).map_err(|error| {
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
        Ok(Self {
            cwd: context.cwd.clone(),
            workspace_root: current.workspace_root,
            workspace_epoch: current.workspace_epoch,
            config,
            config_sha256,
        })
    }

    pub(crate) fn reauthorize_in_context(&self, context: &WorkspaceContext) -> Result<(), String> {
        if path_lock_identity(&self.workspace_root) != path_lock_identity(&context.workspace_root) {
            return Err(workspace_identity_changed_error());
        }
        self.reauthorize_current_workspace()
    }

    pub(crate) fn reauthorize_current_workspace(&self) -> Result<(), String> {
        run_before_reauthorization_hook();
        let current = discover_workspace(Some(self.cwd.clone())).map_err(|error| {
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
        run_after_reauthorization_hook();
        Ok(())
    }
}

/// Normalizes build identity without inspecting configuration support. A
/// default build remains a default v8-runner build; Unica decides about one
/// full fallback only from the structured result of an actually failed partial
/// step (#404, ADR-0059).
pub(crate) fn plan_runtime_invocation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
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
        .then(|| RuntimeBuildPreflight::capture(&planned_args, context))
        .transpose()?;
    Ok(RuntimeInvocationPlan {
        args: planned_args,
        warnings: if explicit_full {
            Vec::new()
        } else {
            vec![
                "v8-runner will try its normal build strategy first; if it reports a failed partial platform step, Unica will retry once with a full rebuild"
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

fn workspace_identity_changed_error() -> String {
    "runtime build workspace identity changed before v8-runner launch".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn workspace() -> (TempDir, WorkspaceContext) {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set: []\n",
        )
        .unwrap();
        let context = discover_workspace(Some(directory.path().to_path_buf())).unwrap();
        (directory, context)
    }

    #[test]
    fn default_build_is_not_forced_full_and_does_not_read_support() {
        let (_directory, context) = workspace();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);

        let plan = plan_runtime_invocation(&args, &context).unwrap();

        assert_eq!(plan.args.get("fullRebuild"), None);
        assert!(plan.build_preflight.is_some());
        assert!(plan.warnings[0].contains("retry once"));
    }

    #[test]
    fn explicit_full_build_has_no_fallback_preflight() {
        let (_directory, context) = workspace();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("fullRebuild".to_string(), json!(true)),
        ]);

        let plan = plan_runtime_invocation(&args, &context).unwrap();

        assert_eq!(plan.args.get("fullRebuild"), Some(&json!(true)));
        assert!(plan.build_preflight.is_none());
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn build_preflight_rejects_a_changed_config_without_reading_support() {
        let (_directory, context) = workspace();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let preflight = RuntimeBuildPreflight::capture(&args, &context).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "format: EDT\nsource-set: []\n",
        )
        .unwrap();

        let error = preflight
            .reauthorize_in_context(&context)
            .expect_err("changed config must fail closed");

        assert!(error.contains("identity changed") || error.contains("config changed"));
    }

    #[test]
    fn build_preflight_rejects_a_different_workspace_root() {
        let (_directory, context) = workspace();
        let (_other_directory, other_context) = workspace();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let preflight = RuntimeBuildPreflight::capture(&args, &context).unwrap();

        let error = preflight
            .reauthorize_in_context(&other_context)
            .expect_err("authorization must stay bound to one workspace root");

        assert!(error.contains("workspace identity changed"), "{error}");
    }

    #[test]
    fn build_preflight_rejects_a_changed_workspace_epoch() {
        let (directory, _) = workspace();
        fs::create_dir_all(directory.path().join(".git")).unwrap();
        let head = directory.path().join(".git/HEAD");
        fs::write(&head, "ref: refs/heads/feature-a\n").unwrap();
        let context = discover_workspace(Some(directory.path().to_path_buf())).unwrap();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let preflight = RuntimeBuildPreflight::capture(&args, &context).unwrap();
        fs::write(&head, "ref: refs/heads/feature-b\n").unwrap();

        let error = preflight
            .reauthorize_current_workspace()
            .expect_err("a changed workspace epoch must invalidate fallback authorization");

        assert!(error.contains("workspace identity changed"), "{error}");
    }

    #[test]
    fn build_preflight_rejects_a_config_outside_the_workspace() {
        let (_directory, context) = workspace();
        let outside = tempfile::tempdir().unwrap();
        let outside_config = outside.path().join("v8project.yaml");
        fs::write(&outside_config, "format: DESIGNER\nsource-set: []\n").unwrap();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            (
                "config".to_string(),
                json!(outside_config.to_string_lossy().into_owned()),
            ),
        ]);

        let error = match plan_runtime_invocation(&args, &context) {
            Ok(_) => panic!("an external project config must not escape the workspace lock"),
            Err(error) => error,
        };

        assert!(error.contains("outside the workspace"), "{error}");
    }

    #[test]
    fn build_preflight_accepts_an_alternate_config_inside_the_workspace() {
        let (_directory, context) = workspace();
        let alternate = context.workspace_root.join("configs/alternate.yaml");
        fs::create_dir_all(alternate.parent().unwrap()).unwrap();
        fs::write(&alternate, "format: DESIGNER\nsource-set: []\n").unwrap();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("config".to_string(), json!("configs/alternate.yaml")),
        ]);

        let plan = plan_runtime_invocation(&args, &context).unwrap();

        assert_eq!(
            plan.args.get("config"),
            Some(&json!(normalize_path_identity(&alternate)
                .unwrap()
                .to_string_lossy()
                .into_owned()))
        );
        assert!(plan.build_preflight.is_some());
    }

    #[test]
    fn build_preflight_rejects_a_non_regular_config() {
        let (_directory, context) = workspace();
        let directory_path = context.workspace_root.join("runtime-config-directory");
        fs::create_dir(&directory_path).unwrap();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            (
                "config".to_string(),
                json!(directory_path.to_string_lossy().into_owned()),
            ),
        ]);

        let error = match plan_runtime_invocation(&args, &context) {
            Ok(_) => panic!("a directory must not become a runtime project config"),
            Err(error) => error,
        };

        assert!(error.contains("regular file"), "{error}");
    }
}

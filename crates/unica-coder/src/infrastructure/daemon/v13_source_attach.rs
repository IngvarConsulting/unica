use super::protocol::InvocationRequest;
use super::v13_workspace_bootstrap::project_config_recipe;
use crate::application::invocation::InvocationResponseDeadline;
use crate::application::invocation_store::ToolIdentity;
use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::tool_catalog::{catalog_for, RunIntent};
use crate::domain::invocation::DomainResult;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::project_sources::{
    discover_project_source_map_controlled, discover_project_source_map_with_provenance,
};
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub(super) fn execute_source_attach(
    request: &InvocationRequest,
    deadline: &InvocationResponseDeadline,
) -> Option<DomainResult> {
    if request.tool() != ToolIdentity::Run {
        return None;
    }
    match request.arguments().get("op") {
        None if request.arguments().is_empty() => Some(run_dictionary_result()),
        None => Some(reject(
            "bad_value",
            "run without op lists the operation dictionary and accepts no other arguments",
        )),
        Some(Value::String(op)) if op == "source.attach" => Some(execute(request, deadline)),
        Some(_) => None,
    }
}

pub(super) fn run_dictionary_result() -> DomainResult {
    let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
    let operations = catalog
        .run_dictionary
        .iter()
        .map(|operation| {
            let preview_required = matches!(
                operation.intent,
                RunIntent::SourceCreate
                    | RunIntent::SourceAttach
                    | RunIntent::InfobaseCreate
                    | RunIntent::InfobaseBuild
                    | RunIntent::SourceDump
                    | RunIntent::SourceConvert
                    | RunIntent::ArtifactMake
                    | RunIntent::ArtifactLoad
                    | RunIntent::ExtensionSync
            );
            json!({
                "op": operation.name(),
                "description": operation.description(),
                "execution": operation.execution(),
                "effects": operation.effects(),
                "implemented": operation.implemented,
                "terminal": operation.terminal,
                "rejectsSessions": operation.rejects_sessions,
                "previewRequired": preview_required,
                "ifRevRequiredOnApply": preview_required,
            })
        })
        .collect::<Vec<_>>();
    let mut result = DomainResult::success("canonical run operation dictionary returned");
    result.data = Some(json!({"operations": operations}));
    result
}

fn execute(request: &InvocationRequest, deadline: &InvocationResponseDeadline) -> DomainResult {
    let arguments = request.arguments();
    let args = match arguments.get("args") {
        None => Map::new(),
        Some(Value::Object(args)) => args.clone(),
        Some(_) => return reject("bad_value", "source.attach args must be an object"),
    };
    if let Some(argument) = args.keys().next() {
        return reject(
            "bad_value",
            format!("source.attach does not accept argument `{argument}`; it attaches the autodetected source sets"),
        );
    }
    let dry_run = match arguments.get("dryRun") {
        Some(Value::Bool(value)) => *value,
        Some(_) => return reject("bad_value", "source.attach dryRun must be a boolean"),
        None => return reject(
            "bad_value",
            "source.attach requires dryRun: true to preview or dryRun: false with ifRev to apply",
        ),
    };
    let if_rev = match arguments.get("ifRev") {
        None => None,
        Some(Value::String(value)) if !value.is_empty() => Some(value.as_str()),
        Some(_) => return reject("bad_value", "source.attach ifRev must be non-empty text"),
    };
    if dry_run && if_rev.is_some() {
        return reject(
            "bad_value",
            "source.attach preview does not accept ifRev; use the returned rev when applying",
        );
    }
    if !dry_run && if_rev.is_none() {
        return reject(
            "bad_value",
            "source.attach apply requires ifRev from a prior dryRun preview",
        );
    }

    let context = match discover_workspace(Some(PathBuf::from(request.workspace_hint()))) {
        Ok(context) => context,
        Err(error) => {
            return reject(
                "provider_unavailable",
                format!("workspace discovery failed: {error}"),
            )
        }
    };
    let (source_map, provenance) = if dry_run {
        let mut checkpoint = || deadline.checkpoint_handoff().map_err(str::to_string);
        match discover_project_source_map_controlled(&context.workspace_root, &mut checkpoint) {
            Ok(source_map) => (source_map, None),
            Err(error) => {
                return reject(
                    "provider_unavailable",
                    format!("workspace source discovery failed: {error}"),
                )
            }
        }
    } else {
        if let Err(error) = deadline.checkpoint_handoff() {
            return reject("deadline_exceeded", error);
        }
        match discover_project_source_map_with_provenance(&context.workspace_root) {
            Ok((source_map, provenance)) => (source_map, Some(provenance)),
            Err(error) => {
                return reject(
                    "provider_unavailable",
                    format!("workspace source discovery failed: {error}"),
                )
            }
        }
    };
    if source_map.config_path.is_some() {
        return reject(
            "invalid_state",
            "source.attach creates only a missing v8project.yaml and never overwrites an existing project config",
        );
    }
    if source_map.source_sets.is_empty() {
        return reject(
            "not_found",
            "source.attach found no 1C source roots; create or import sources first, then call unica.view {} again",
        );
    }
    let Some(recipe) = project_config_recipe(&source_map) else {
        return reject(
            "ambiguous_source_format",
            "source.attach cannot choose one v8project.yaml format because the discovered source sets are mixed, unknown, or invalid",
        );
    };
    let revision = attachment_revision(&source_map, &recipe);
    let source_sets = serde_json::to_value(&source_map.source_sets)
        .expect("project source sets always serialize");
    let mut result = DomainResult::success(if dry_run {
        "source attachment planned; no files were changed"
    } else {
        "autodetected source sets attached"
    });
    result.data = Some(json!({
        "op": "source.attach",
        "dryRun": dry_run,
        "target": "v8project.yaml",
        "sourceSets": source_sets,
        "content": recipe,
        "requiresPlatform": false,
    }));
    result.rev = Some(revision.clone());

    if dry_run {
        result.next.push(json!({
            "tool": "unica.run",
            "args": {
                "op": "source.attach",
                "args": {},
                "dryRun": false,
                "ifRev": revision,
            },
            "reason": "apply exactly this source attachment plan",
        }));
        return result;
    }
    if if_rev != Some(revision.as_str()) {
        return reject(
            "revision_mismatch",
            "source.attach discovery changed after preview; call dryRun: true again",
        );
    }
    if let Err(error) = deadline.checkpoint_handoff() {
        return reject("deadline_exceeded", error);
    }
    let target = context.workspace_root.join("v8project.yaml");
    let mut transaction = CompileTransaction::new();
    if let Err(error) = transaction.create_text(&target, &recipe) {
        return reject("provider_unavailable", error);
    }
    if let Err(error) = provenance
        .expect("applied source attachment captures discovery provenance")
        .bind_to(&mut transaction)
    {
        return reject("concurrent_change", error);
    }
    match transaction.commit() {
        Ok(report) => {
            result.changed.push(json!({
                "path": "v8project.yaml",
                "kind": "created",
            }));
            for warning in report.cleanup_warnings {
                result.warnings.push(json!({
                    "code": "cleanup_incomplete",
                    "message": warning,
                }));
            }
            result.next.push(json!({
                "tool": "unica.view",
                "args": {},
                "reason": "rediscover the now-configured workspace",
            }));
            result
        }
        Err(error) => reject(
            "concurrent_change",
            format!("source.attach did not publish v8project.yaml: {error}"),
        ),
    }
}

fn attachment_revision(
    source_map: &crate::domain::project_sources::ProjectSourceMap,
    recipe: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-source-attach-v1\0");
    hasher.update(
        serde_json::to_vec(source_map).expect("project source map always serializes for revision"),
    );
    hasher.update([0]);
    hasher.update(recipe.as_bytes());
    format!("unica-source-attach-sha256-v1:{:x}", hasher.finalize())
}

fn reject(code: &'static str, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(Some("source.attach".to_string()), code, message)
}

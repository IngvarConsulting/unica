use super::protocol::InvocationRequest;
use crate::application::invocation_store::ToolIdentity;
use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::tool_catalog::{catalog_for, RunIntent};
use crate::domain::invocation::DomainResult;
use crate::domain::refusal::RefusalCode;
use serde_json::{json, Value};

/// Словарь операций `run` и отказ по неизвестному имени.
///
/// Исполнителя здесь больше нет: `v8project.yaml` заводит человек или модель
/// своими файловыми средствами, а инструмента записи проектного файла в
/// продукте не остаётся вовсе.
pub(super) fn execute_run_dictionary(request: &InvocationRequest) -> Option<DomainResult> {
    if request.tool() != ToolIdentity::Run {
        return None;
    }
    match request.arguments().get("op") {
        None if request.arguments().is_empty() => Some(run_dictionary_result()),
        None => Some(DomainResult::canonical_rejection(
            None,
            RefusalCode::BadValue,
            "run without op lists the operation dictionary and accepts no other arguments",
        )),
        Some(Value::String(op)) => {
            let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
            if catalog
                .run_dictionary
                .iter()
                .any(|operation| operation.name() == op)
            {
                None
            } else {
                Some(reject_run_operation(
                    op,
                    format!("unknown canonical run operation `{op}`"),
                ))
            }
        }
        Some(_) => None,
    }
}

pub(super) fn reject_unavailable_run_before_admission(
    request: &InvocationRequest,
) -> Option<DomainResult> {
    if request.tool() != ToolIdentity::Run {
        return None;
    }
    let op = match request.arguments().get("op") {
        Some(Value::String(op)) => op,
        Some(_) => {
            return Some(DomainResult::canonical_rejection(
                None,
                RefusalCode::BadValue,
                "run op must be a string",
            ))
        }
        None => return None,
    };
    let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
    match catalog
        .run_dictionary
        .iter()
        .find(|operation| operation.name() == op)
    {
        Some(operation) if operation.implemented => None,
        Some(_) => Some(reject_run_operation(
            op,
            format!("canonical run operation `{op}` is not implemented yet"),
        )),
        None => Some(reject_run_operation(
            op,
            format!("unknown canonical run operation `{op}`"),
        )),
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
                RunIntent::InfobaseCreate
                    | RunIntent::InfobaseBuild
                    | RunIntent::SourceDump
                    | RunIntent::SourceConvert
                    | RunIntent::ArtifactBuild
                    | RunIntent::InfobaseConfigurationExport
                    | RunIntent::InfobaseConfigurationLoad
                    | RunIntent::InfobaseDump
                    | RunIntent::InfobaseRestore
            );
            json!({
                "op": operation.name(),
                "description": operation.description(),
                "argsSchema": operation.args_schema(),
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

fn reject_run_operation(op: &str, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(
        Some(op.to_string()),
        RefusalCode::UnsupportedOperation,
        message,
    )
}

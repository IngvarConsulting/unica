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
            if is_test_seam_operation(op) {
                return None;
            }
            let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
            if catalog
                .run_dictionary
                .iter()
                .any(|operation| operation.name() == op)
            {
                reject_unavailable_run_before_admission(request)
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
    if request
        .arguments()
        .get("infobase")
        .is_some_and(|v| v.as_str() != Some("origin"))
    {
        return Some(reject_run_operation(op, "runner 0.11 adapter supports only the named infobase origin; no fallback to another target"));
    }
    if is_test_seam_operation(op) {
        return None;
    }
    let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
    match catalog
        .run_dictionary
        .iter()
        .find(|operation| operation.name() == op)
    {
        Some(operation) if operation.implemented => None,
        Some(operation) => Some(reject_run_operation(
            op,
            operation
                .support_reason()
                .unwrap_or("operation is unavailable")
                .to_string(),
        )),
        None => Some(reject_run_operation(
            op,
            format!("unknown canonical run operation `{op}`"),
        )),
    }
}

/// Тестовый шов: пока словарь не был реализован целиком, нереализованная
/// операция доходила до актора рабочего пространства, и тесты долгой работы
/// демона ходили через неё. Теперь каждая операция готовится до admission, и
/// тесты зовут актора именем `test.*`, которого в продукте не существует:
/// вне тестов такое имя — неизвестная операция.
#[cfg(test)]
fn is_test_seam_operation(op: &str) -> bool {
    op.starts_with("test.")
}

#[cfg(not(test))]
const fn is_test_seam_operation(_op: &str) -> bool {
    false
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
                    | RunIntent::SourceImport
                    | RunIntent::SourceExport
                    | RunIntent::ArtifactBuild
                    | RunIntent::CfExport
                    | RunIntent::CfImport
                    | RunIntent::InfobaseExport
                    | RunIntent::InfobaseImport
                    | RunIntent::ExtensionList
                    | RunIntent::ConfigurationApply
                    | RunIntent::ConfigurationReset
                    | RunIntent::ExtensionActivate
            );
            json!({
                "op": operation.name(),
                "description": operation.description(),
                "argsSchema": operation.args_schema(),
                "execution": operation.execution(),
                "effects": operation.effects(),
                "implemented": operation.implemented,
                "support": {"adapter":"v8-runner/0.11.1", "state": if !operation.implemented {"unavailable"} else if operation.support_reason().is_some() {"limited"} else {"supported"}, "reason":operation.support_reason(), "supportedInfobases":["origin"], "supportedArgs": operation.args_schema()},
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

#[cfg(test)]
mod runner_one_tests {
    use super::*;
    #[test]
    fn development_cycle_admits_the_explicit_compatibility_subset() {
        for (op, args) in [
            ("push", json!({"force":true,"full":true})),
            ("pull", json!({"force":true})),
            ("infobase.create", json!({})),
            ("upload", json!({"input":"dist/main.cf"})),
            ("apply", json!({})),
            ("reset", json!({"force":true})),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":op,"args":args,"dryRun":true}),
                "/not-a-workspace",
                7000,
            )
            .unwrap();
            assert!(
                execute_run_dictionary(&request).is_none(),
                "{op} must reach its typed compatibility handler"
            );
        }
    }

    #[test]
    fn runner_one_refuses_unsupported_semantics_and_old_names_before_admission() {
        for (op, args) in [
            ("source.import", json!({})),
            ("source.export", json!({})),
            ("cf.import", json!({"input":"x.cf"})),
            ("extension.create", json!({"name":"X"})),
            ("extension.info", json!({"name":"X"})),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":op,"args":args,"dryRun":true}),
                "/workspace-does-not-exist",
                7000,
            )
            .unwrap();
            let result =
                execute_run_dictionary(&request).expect("refused before workspace or process");
            assert!(!result.ok, "{op}");
            assert_eq!(result.diagnostics[0]["code"], "unsupported_operation");
            assert!(result.rev.is_none());
        }
    }
    #[test]
    fn runner_one_never_redirects_an_unsupported_infobase_to_origin() {
        let request = InvocationRequest::new(ToolIdentity::Run,json!({"op":"download","infobase":"production","args":{"state":"working","output":"x.cf"},"dryRun":true}),"/workspace-does-not-exist",7000).unwrap();
        let result = execute_run_dictionary(&request).unwrap();
        assert!(!result.ok);
        assert!(result.summary.contains("no fallback"));
    }
    #[test]
    fn limited_delete_and_known_operations_reach_their_typed_parsers() {
        for (op, args) in [
            ("push", json!({"delete":"Patch"})),
            ("extensions.set", json!({"name":"Patch","active":false})),
            ("download", json!({"state":"working","output":"x.cf"})),
        ] {
            let request = InvocationRequest::new(
                ToolIdentity::Run,
                json!({"op":op,"infobase":"origin","args":args,"dryRun":true}),
                "/workspace",
                7000,
            )
            .unwrap();
            assert!(execute_run_dictionary(&request).is_none());
        }
        let dictionary = run_dictionary_result();
        let operations = dictionary.data.as_ref().unwrap()["operations"]
            .as_array()
            .unwrap();
        let push = operations.iter().find(|v| v["op"] == "push").unwrap();
        assert_eq!(push["implemented"], true);
        assert_eq!(push["support"]["state"], "limited");
        assert!(push["support"]["supportedArgs"]["properties"]
            .get("delete")
            .is_some());
    }
}

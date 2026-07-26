use super::{code, form, meta, registry, NativeOperationAdapter};
use crate::{application::AdapterOutcome, domain::workspace::WorkspaceContext};
use serde::Serialize;
use serde_json::{json, Map, Value};

pub(crate) struct NativeOperationResult {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) data: Option<Value>,
}

impl NativeOperationAdapter {
    pub(crate) fn invoke_with_data(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<NativeOperationResult, String> {
        if !mutating && operation == "meta-info" {
            let navigation = meta::inspect_meta_navigation(args, context)?;
            return Ok(NativeOperationResult {
                adapter: AdapterOutcome::ok("semantic metadata navigation inspected"),
                data: Some(json!({ "navigation": navigation })),
            });
        }

        if mutating {
            match registry::typed_mutation_handler(operation) {
                Some(registry::TypedMutationHandler::CodePatch) => {
                    let execution = if dry_run {
                        code::preview_with_data(args, context)
                    } else {
                        code::apply_with_data(args, context)
                    };
                    return typed_mutation_result(execution.outcome, execution.data, "code patch");
                }
                Some(registry::TypedMutationHandler::FormEdit) if form::has_edit_payload(args) => {
                    let execution = if dry_run {
                        form::preview_with_data(args, context)
                    } else {
                        form::apply_with_data(args, context)
                    };
                    return typed_mutation_result(execution.outcome, execution.data, "form edit");
                }
                Some(registry::TypedMutationHandler::FormEdit) => {}
                None => {}
            }
        }

        Self::invoke(operation, tool_name, args, context, dry_run, mutating).map(|adapter| {
            NativeOperationResult {
                adapter,
                data: None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_GATEWAY_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn meta_info_typed_gateway_returns_direct_navigation_for_normal_dry_run_and_unavailable() {
        let (context, path) = fixture("2.20");
        let args = json!({"ObjectPath": path}).as_object().unwrap().clone();
        for dry_run in [false, true] {
            let result = NativeOperationAdapter::invoke_with_data(
                "meta-info",
                "unica.meta.info",
                &args,
                &context,
                dry_run,
                false,
            )
            .unwrap();
            assert!(result.adapter.ok);
            assert!(result.adapter.stdout.is_none());
            let data = result.data.unwrap();
            let navigation = &data["navigation"];
            let keys = navigation
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                keys,
                std::collections::BTreeSet::from([
                    "diagnostics".to_string(),
                    "nodes".to_string(),
                    "relations".to_string(),
                    "root".to_string(),
                    "schemaVersion".to_string(),
                    "snapshot".to_string(),
                    "status".to_string()
                ])
            );
            assert_eq!(navigation["status"], "ready");
            assert!(navigation.get("graph").is_none());
            assert!(data.get("text").is_none());
        }
        std::fs::remove_dir_all(context.workspace_root).unwrap();

        let (unsupported_context, unsupported_path) = fixture("2.19");
        let unavailable = NativeOperationAdapter::invoke_with_data(
            "meta-info",
            "unica.meta.info",
            &json!({"ObjectPath": unsupported_path})
                .as_object()
                .unwrap()
                .clone(),
            &unsupported_context,
            false,
            false,
        )
        .unwrap();
        assert!(unavailable.adapter.ok);
        assert!(unavailable.adapter.stdout.is_none());
        let navigation = &unavailable.data.unwrap()["navigation"];
        assert_eq!(navigation["status"], "unavailable");
        assert!(navigation["snapshot"].is_null());
        assert!(navigation["root"].is_null());
        assert_eq!(navigation["nodes"], json!([]));
        assert_eq!(navigation["relations"], json!([]));
        std::fs::remove_dir_all(unsupported_context.workspace_root).unwrap();
    }

    fn fixture(version: &str) -> (WorkspaceContext, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-typed-gateway-{}-{}",
            std::process::id(),
            NEXT_GATEWAY_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        let src = root.join("src");
        let path = src.join("Catalogs/Items.xml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(root.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n").unwrap();
        std::fs::write(src.join("Configuration.xml"), format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Configuration</Name></Properties></Configuration></MetaDataObject>"#)).unwrap();
        std::fs::write(&path, format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Catalog uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#)).unwrap();
        (
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            path,
        )
    }
}

fn typed_mutation_result<T: Serialize>(
    adapter: AdapterOutcome,
    data: Option<T>,
    operation: &str,
) -> Result<NativeOperationResult, String> {
    let data = data
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| format!("serialize typed {operation} result: {error}"))?;
    Ok(NativeOperationResult { adapter, data })
}

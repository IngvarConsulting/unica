//! Native XML/DSL operation facade.
//!
//! Family modules under `native_operations/` own operation-specific XML/DSL
//! behavior; this facade keeps the public adapter surface thin.

pub(crate) mod cf;
pub(crate) mod cfe;
pub(crate) mod common;
pub(crate) mod compile_transaction;
pub(crate) mod form;
pub(crate) mod help;
pub(crate) mod interface;
pub(crate) mod meta;
pub(crate) mod mxl;
pub(crate) mod registry;
pub(crate) mod role;
pub(crate) mod skd;
pub(crate) mod subsystem;
pub(crate) mod support;
pub(crate) mod template;

use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::AdapterOutcome;
use serde_json::{Map, Value};
use std::fs;

pub struct NativeOperationAdapter;

impl NativeOperationAdapter {
    pub fn invoke(
        operation: &str,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<AdapterOutcome, String> {
        if dry_run {
            let mut fallback = AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} would execute native XML/DSL operation"),
                changes: if mutating {
                    vec!["no files changed because dryRun is true".to_string()]
                } else {
                    Vec::new()
                },
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            };
            if let Some(preview) = registry::invoke_preview(operation, args, context) {
                return match preview {
                    registry::PreviewInvocation::Unavailable(error) => {
                        fallback.warnings.push(format!(
                            "detailed compile preview is unavailable; using safe placeholder: {error}"
                        ));
                        Ok(fallback)
                    }
                    registry::PreviewInvocation::Planned(Ok(outcome)) => Ok(outcome),
                    registry::PreviewInvocation::Planned(Err(error)) => Ok(AdapterOutcome {
                        ok: false,
                        summary: format!("dry run: {tool_name} compile planning failed"),
                        changes: Vec::new(),
                        warnings: Vec::new(),
                        errors: vec![error.clone()],
                        artifacts: Vec::new(),
                        stdout: None,
                        stderr: Some(format!("{error}\n")),
                        command: None,
                    }),
                };
            }
            return Ok(fallback);
        }

        if mutating {
            return registry::invoke_mutation(operation, tool_name, args, context).ok_or_else(|| {
                format!(
                    "native mutation handler is not registered for {tool_name} operation `{operation}`"
                )
            });
        }

        if let Some(outcome) = registry::invoke_read(operation, tool_name, args, context) {
            return outcome;
        }

        let target = common::resolve_target(operation, args, context)?;
        let text = fs::read_to_string(&target)
            .map_err(|err| format!("failed to read {}: {err}", target.display()))?;
        Ok(common::analyze_xml(operation, tool_name, &target, &text))
    }
}

#[cfg(test)]
mod tests {
    use super::NativeOperationAdapter;
    use crate::domain::workspace::WorkspaceContext;
    use serde_json::Map;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn missing_native_mutation_handler_is_contract_error() {
        let root = temp_root("missing-mutation-handler");
        fs::create_dir_all(root.join("src")).unwrap();
        let context = WorkspaceContext::discover(root.clone()).unwrap();

        let result = NativeOperationAdapter::invoke(
            "definitely-missing-operation",
            "unica.definitely.missing",
            &Map::new(),
            &context,
            false,
            true,
        );

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("native mutation handler is not registered"));
    }

    #[test]
    fn compile_preview_without_payload_uses_the_safe_dry_run_placeholder() {
        let root = temp_root("compile-preview-fallback");
        let context = WorkspaceContext::discover(root.clone()).unwrap();

        let result = NativeOperationAdapter::invoke(
            "meta-compile",
            "unica.meta.compile",
            &Map::new(),
            &context,
            true,
            true,
        )
        .expect("a missing preview payload must preserve the legacy dry-run contract");

        assert!(result.ok);
        assert!(result.summary.contains("dry run"));
        assert_eq!(
            result.changes,
            vec!["no files changed because dryRun is true".to_string()]
        );
        assert!(result.artifacts.is_empty());
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("detailed compile preview is unavailable")));
        assert!(fs::read_dir(&root).unwrap().next().is_none());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn subsystem_preview_with_unavailable_parent_uses_the_legacy_placeholder() {
        let root = temp_root("subsystem-preview-parent-fallback");
        let context = WorkspaceContext::discover(root.clone()).unwrap();
        let args = serde_json::from_value(serde_json::json!({
            "OutputDir": root.display().to_string(),
            "Value": r#"{"name":"Child"}"#,
            "Parent": "Subsystems/Missing.xml"
        }))
        .unwrap();

        let result = NativeOperationAdapter::invoke(
            "subsystem-compile",
            "unica.subsystem.compile",
            &args,
            &context,
            true,
            true,
        )
        .unwrap();

        assert!(result.ok);
        assert!(result.summary.contains("dry run"));
        assert!(result.warnings[0].contains("parent subsystem is unavailable"));
        fs::remove_dir_all(root).unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-native-ops-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}

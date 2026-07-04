//! Native XML/DSL operation facade.
//!
//! Family modules under `native_operations/` own operation-specific XML/DSL
//! behavior; this facade keeps the public adapter surface thin.

pub(crate) mod cf;
pub(crate) mod cfe;
pub(crate) mod common;
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
            return Ok(AdapterOutcome {
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
            });
        }

        if mutating {
            return Ok(
                registry::invoke_mutation(operation, tool_name, args, context)
                    .unwrap_or_else(|| unimplemented_mutation(tool_name)),
            );
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

fn unimplemented_mutation(tool_name: &str) -> AdapterOutcome {
    AdapterOutcome {
        ok: false,
        summary: format!(
            "{tool_name} is native, but this mutating operation needs a concrete JSON/XML payload before execution"
        ),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![format!(
            "native handler for {tool_name} refuses to mutate without an implemented operation-specific writer"
        )],
        artifacts: Vec::new(),
        stdout: None,
        stderr: None,
        command: None,
    }
}

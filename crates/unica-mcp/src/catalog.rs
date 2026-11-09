//! Каталог инструментов двери MCP из реестра описаний приложения.

use std::sync::Arc;

use rmcp::model::Tool;
use serde_json::Value;

use crate::application::ToolSpec;
use crate::{input_schema_for_tool, strip_schema_descriptions, structured_output_schema};

/// Data-driven MCP tool definitions from the application descriptor registry.
pub fn tool_definitions(specs: &[ToolSpec]) -> Vec<Tool> {
    specs
        .iter()
        .map(|spec| {
            // #479 §1 schema-only baseline (owner decision, 2026-08-17): the
            // wire surface carries no prose while descriptions are reauthored;
            // the v0.12 history keeps the previous texts.
            let mut input_schema = input_schema_for_tool(spec);
            strip_schema_descriptions(&mut input_schema);
            let schema = match input_schema {
                Value::Object(schema) => schema,
                other => {
                    unreachable!("tool {} produced a non-object schema: {other}", spec.name)
                }
            };
            let mut tool = Tool::new(spec.name, spec.description, schema);
            tool.description = None;
            if let Some(mut schema) = structured_output_schema(spec) {
                strip_schema_descriptions(&mut schema);
                let output_schema = match schema {
                    Value::Object(schema) => schema,
                    other => unreachable!("OperationResult produced a non-object schema: {other}"),
                };
                tool.with_raw_output_schema(Arc::new(output_schema))
            } else {
                tool
            }
        })
        .collect()
}

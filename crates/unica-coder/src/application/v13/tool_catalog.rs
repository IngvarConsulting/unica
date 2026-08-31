use crate::application::tool_contracts::SurfaceRelease;
use serde_json::{json, Value};

#[derive(Debug)]
pub(crate) struct V13ToolContract {
    pub(crate) name: &'static str,
    pub(crate) input_schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindProjection {
    AddressCandidates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchProjection {
    ContentOrSymbolMatches,
}

/// Semantic limits that cannot be expressed by a shallow input schema, kept
/// alongside it so later handlers must preserve the approved division of work.
#[derive(Debug)]
pub(crate) struct CatalogSemantics {
    pub(crate) find_projection: FindProjection,
    pub(crate) search_projection: SearchProjection,
    pub(crate) check_reads_persisted_state: bool,
    pub(crate) apply_dry_run_uses_validator_registry: bool,
    pub(crate) diff_is_read_only: bool,
    pub(crate) diff_cursor_carries_both_source_revisions: bool,
    pub(crate) diff_rejects_incomparable_node_kinds: bool,
    pub(crate) search_scope_is_logical_subtree_address: bool,
    pub(crate) docs_filters_source_kinds_not_provider_identities: bool,
    pub(crate) apply_operations_come_from_node_can_data: bool,
    pub(crate) run_dictionary_is_data_not_command_lines: bool,
    pub(crate) empty_optional_result_slots_are_omitted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunIntent {
    SourceCreate,
    SourceAttach,
    InfobaseCreate,
    InfobaseBuild,
    SourceDump,
    SourceConvert,
    ArtifactMake,
    ArtifactLoad,
    SyntaxCheck,
    TestRun,
    ClientRun,
    ExtensionSync,
}

#[derive(Debug)]
pub(crate) struct RunOperation {
    pub(crate) intent: RunIntent,
    pub(crate) terminal: bool,
    pub(crate) rejects_sessions: bool,
    pub(crate) implemented: bool,
}

impl RunOperation {
    pub(crate) const fn name(&self) -> &'static str {
        match self.intent {
            RunIntent::SourceCreate => "source.create",
            RunIntent::SourceAttach => "source.attach",
            RunIntent::InfobaseCreate => "infobase.create",
            RunIntent::InfobaseBuild => "infobase.build",
            RunIntent::SourceDump => "source.dump",
            RunIntent::SourceConvert => "source.convert",
            RunIntent::ArtifactMake => "artifact.make",
            RunIntent::ArtifactLoad => "artifact.load",
            RunIntent::SyntaxCheck => "syntax.check",
            RunIntent::TestRun => "test.run",
            RunIntent::ClientRun => "client.run",
            RunIntent::ExtensionSync => "extension.sync",
        }
    }
}

#[derive(Debug)]
pub(crate) struct V13Catalog {
    pub(crate) tools: Vec<V13ToolContract>,
    pub(crate) semantics: CatalogSemantics,
    pub(crate) run_dictionary: Vec<RunOperation>,
    pub(crate) result_envelope_schema: Value,
}

/// Returns the canonical v0.13 catalog selected by package routing. V12 is
/// retained only as an explicit compatibility/test seam and has no v0.13
/// catalog.
pub(crate) fn catalog_for(release: SurfaceRelease) -> Option<V13Catalog> {
    match release {
        SurfaceRelease::V12 => None,
        SurfaceRelease::V13 => Some(V13Catalog {
            tools: vec![
                V13ToolContract {
                    name: "view",
                    input_schema: schema(
                        json!({
                            "at": logical_address(),
                            "filter": data_object(),
                            "limit": limit(),
                            "cursor": cursor(),
                        }),
                        json!(["at"]),
                    ),
                },
                V13ToolContract {
                    name: "apply",
                    input_schema: schema(
                        json!({
                            "at": logical_address(),
                            "ops": {
                                "type": "array",
                                "minItems": 1,
                                "items": {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": {"type": "string"},
                                        "args": data_object(),
                                    },
                                    "required": ["op"],
                                },
                            },
                            "dryRun": {"type": "boolean", "default": false},
                            "ifRev": {"type": "string"},
                        }),
                        json!(["at", "ops"]),
                    ),
                },
                V13ToolContract {
                    name: "find",
                    input_schema: schema(
                        json!({
                            "query": {"type": "string"},
                            "kind": {"type": "string"},
                            "limit": limit(),
                        }),
                        json!(["query"]),
                    ),
                },
                V13ToolContract {
                    name: "search",
                    input_schema: schema(
                        json!({
                            "query": {"type": "string"},
                            "scope": logical_subtree_address(),
                            "regex": {"type": "boolean", "default": false},
                            "limit": limit(),
                        }),
                        json!(["query"]),
                    ),
                },
                V13ToolContract {
                    name: "check",
                    input_schema: schema(
                        json!({
                            "at": logical_address(),
                            "filter": data_object(),
                        }),
                        json!([]),
                    ),
                },
                V13ToolContract {
                    name: "diff",
                    input_schema: schema(
                        json!({
                            "left": logical_address(),
                            "right": logical_address(),
                            "filter": data_object(),
                            "limit": limit(),
                            "cursor": cursor(),
                        }),
                        json!(["left", "right"]),
                    ),
                },
                V13ToolContract {
                    name: "run",
                    input_schema: schema(
                        json!({
                            "op": {"type": "string"},
                            "args": data_object(),
                        }),
                        json!([]),
                    ),
                },
                V13ToolContract {
                    name: "docs",
                    input_schema: schema(
                        json!({
                            "query": {"type": "string"},
                            "source": {"type": "string"},
                        }),
                        json!(["query"]),
                    ),
                },
            ],
            semantics: CatalogSemantics {
                find_projection: FindProjection::AddressCandidates,
                search_projection: SearchProjection::ContentOrSymbolMatches,
                check_reads_persisted_state: true,
                apply_dry_run_uses_validator_registry: true,
                diff_is_read_only: true,
                diff_cursor_carries_both_source_revisions: true,
                diff_rejects_incomparable_node_kinds: true,
                search_scope_is_logical_subtree_address: true,
                docs_filters_source_kinds_not_provider_identities: true,
                apply_operations_come_from_node_can_data: true,
                run_dictionary_is_data_not_command_lines: true,
                empty_optional_result_slots_are_omitted: true,
            },
            run_dictionary: run_dictionary(),
            result_envelope_schema: result_envelope_schema(),
        }),
    }
}

fn schema(properties: Value, required: Value) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

fn logical_address() -> Value {
    json!({"type": "string", "description": "qualified logical address"})
}

fn logical_subtree_address() -> Value {
    json!({"type": "string", "description": "logical subtree address"})
}

fn data_object() -> Value {
    json!({"type": "object"})
}

fn limit() -> Value {
    json!({"type": "integer", "minimum": 1})
}

fn cursor() -> Value {
    json!({"type": "string"})
}

fn run_dictionary() -> Vec<RunOperation> {
    [
        RunIntent::SourceCreate,
        RunIntent::SourceAttach,
        RunIntent::InfobaseCreate,
        RunIntent::InfobaseBuild,
        RunIntent::SourceDump,
        RunIntent::SourceConvert,
        RunIntent::ArtifactMake,
        RunIntent::ArtifactLoad,
        RunIntent::SyntaxCheck,
        RunIntent::TestRun,
        RunIntent::ClientRun,
        RunIntent::ExtensionSync,
    ]
    .into_iter()
    .map(|intent| RunOperation {
        terminal: intent == RunIntent::ClientRun,
        rejects_sessions: intent == RunIntent::ClientRun,
        implemented: intent == RunIntent::SyntaxCheck,
        intent,
    })
    .collect()
}

fn result_envelope_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "ok": {"type": "boolean"},
            "at": logical_address(),
            "summary": {"type": "string"},
            "data": {},
            "changed": {"type": "array", "minItems": 1, "items": {}},
            "warnings": {"type": "array", "minItems": 1, "items": {}},
            "diagnostics": {"type": "array", "minItems": 1, "items": {}},
            "artifacts": {"type": "array", "minItems": 1, "items": {}},
            "next": {"type": "array", "minItems": 1, "items": {}},
            "rev": {"type": "string"},
            "cursor": cursor(),
        },
        "required": ["ok", "summary"],
    })
}

#[cfg(test)]
mod tests {
    use super::{catalog_for, FindProjection, RunIntent, SearchProjection};
    use crate::application::tool_contracts::SurfaceRelease;
    use serde_json::{json, Value};

    fn contract<'a>(
        catalog: &'a [super::V13ToolContract],
        name: &str,
    ) -> &'a super::V13ToolContract {
        catalog
            .iter()
            .find(|contract| contract.name == name)
            .unwrap_or_else(|| panic!("missing v0.13 contract unica.{name}"))
    }

    fn assert_schema(
        catalog: &[super::V13ToolContract],
        name: &str,
        required: Value,
        properties: &[&str],
    ) {
        let schema = &contract(catalog, name).input_schema;
        assert_eq!(
            schema["type"], "object",
            "unica.{name} must accept an object"
        );
        assert_eq!(
            schema["additionalProperties"], false,
            "unica.{name} must reject unknown top-level arguments"
        );
        assert_eq!(
            schema["required"], required,
            "unica.{name} required arguments drifted"
        );
        assert_eq!(
            schema["properties"]
                .as_object()
                .expect("input properties must be an object")
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            properties,
            "unica.{name} argument set drifted"
        );
        for forbidden in ["jobId", "path", "provider", "providerId"] {
            assert!(
                schema["properties"].get(forbidden).is_none(),
                "unica.{name} must not expose `{forbidden}`"
            );
        }
    }

    fn input_field<'a>(
        catalog: &'a [super::V13ToolContract],
        tool: &str,
        field: &str,
    ) -> &'a Value {
        &contract(catalog, tool).input_schema["properties"][field]
    }

    fn assert_field_type(
        catalog: &[super::V13ToolContract],
        tool: &str,
        field: &str,
        expected: &str,
    ) {
        assert_eq!(
            input_field(catalog, tool, field)["type"],
            expected,
            "unica.{tool}.{field} type drifted"
        );
    }

    fn assert_data_object(value: &Value, location: &str) {
        assert_eq!(
            value,
            &json!({"type": "object"}),
            "{location} must remain an unconstrained shallow data object"
        );
    }

    #[test]
    fn v13_catalog_locks_the_eight_domain_contracts_without_publishing_them() {
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");

        assert_eq!(
            catalog
                .tools
                .iter()
                .map(|contract| contract.name)
                .collect::<Vec<_>>(),
            ["view", "apply", "find", "search", "check", "diff", "run", "docs"]
        );
        assert_eq!(
            SurfaceRelease::from_package_version(),
            SurfaceRelease::V13,
            "the package-selected release must expose the canonical v0.13 surface"
        );
        assert!(catalog_for(SurfaceRelease::V12).is_none());

        assert_schema(
            &catalog.tools,
            "view",
            json!(["at"]),
            &["at", "filter", "limit", "cursor"],
        );
        assert_schema(
            &catalog.tools,
            "apply",
            json!(["at", "ops"]),
            &["at", "ops", "dryRun", "ifRev"],
        );
        assert_schema(
            &catalog.tools,
            "find",
            json!(["query"]),
            &["query", "kind", "limit"],
        );
        assert_schema(
            &catalog.tools,
            "search",
            json!(["query"]),
            &["query", "scope", "regex", "limit"],
        );
        assert_schema(&catalog.tools, "check", json!([]), &["at", "filter"]);
        assert_schema(
            &catalog.tools,
            "diff",
            json!(["left", "right"]),
            &["left", "right", "filter", "limit", "cursor"],
        );
        assert_schema(&catalog.tools, "run", json!([]), &["op", "args"]);
        assert_schema(
            &catalog.tools,
            "docs",
            json!(["query"]),
            &["query", "source"],
        );

        for (tool, field) in [
            ("view", "at"),
            ("apply", "at"),
            ("apply", "ifRev"),
            ("find", "query"),
            ("find", "kind"),
            ("search", "query"),
            ("search", "scope"),
            ("check", "at"),
            ("diff", "left"),
            ("diff", "right"),
            ("diff", "cursor"),
            ("run", "op"),
            ("docs", "query"),
            ("docs", "source"),
        ] {
            assert_field_type(&catalog.tools, tool, field, "string");
        }
        for (tool, field) in [
            ("view", "filter"),
            ("apply", "ops"),
            ("check", "filter"),
            ("diff", "filter"),
            ("run", "args"),
        ] {
            assert_field_type(
                &catalog.tools,
                tool,
                field,
                if field == "ops" { "array" } else { "object" },
            );
        }
        for (tool, field) in [
            ("view", "limit"),
            ("find", "limit"),
            ("search", "limit"),
            ("diff", "limit"),
        ] {
            let limit = input_field(&catalog.tools, tool, field);
            assert_eq!(limit["type"], "integer");
            assert_eq!(limit["minimum"], 1);
        }
        assert_field_type(&catalog.tools, "view", "cursor", "string");
        assert_data_object(
            input_field(&catalog.tools, "view", "filter"),
            "unica.view.filter",
        );
        assert_data_object(
            input_field(&catalog.tools, "check", "filter"),
            "unica.check.filter",
        );
        assert_data_object(
            input_field(&catalog.tools, "diff", "filter"),
            "unica.diff.filter",
        );
        assert_data_object(input_field(&catalog.tools, "run", "args"), "unica.run.args");
        assert_eq!(
            input_field(&catalog.tools, "apply", "dryRun")["type"],
            "boolean"
        );
        assert_eq!(
            input_field(&catalog.tools, "apply", "dryRun")["default"],
            false
        );
        assert_eq!(
            input_field(&catalog.tools, "search", "regex")["type"],
            "boolean"
        );
        assert_eq!(
            input_field(&catalog.tools, "search", "regex")["default"],
            false
        );

        let apply = contract(&catalog.tools, "apply");
        assert_eq!(apply.input_schema["properties"]["ops"]["type"], "array");
        assert_eq!(apply.input_schema["properties"]["ops"]["minItems"], 1);
        assert_eq!(apply.input_schema["properties"]["dryRun"]["default"], false);
        assert_eq!(
            apply.input_schema["properties"]["ops"]["items"]["type"],
            "object"
        );
        assert_eq!(
            apply.input_schema["properties"]["ops"]["items"]["additionalProperties"],
            false
        );
        assert_eq!(
            apply.input_schema["properties"]["ops"]["items"]["required"],
            json!(["op"])
        );
        assert_eq!(
            apply.input_schema["properties"]["ops"]["items"]["properties"]["op"]["type"],
            "string"
        );
        assert_data_object(
            &apply.input_schema["properties"]["ops"]["items"]["properties"]["args"],
            "unica.apply.ops[].args",
        );
        for keyword in ["enum", "oneOf", "anyOf", "allOf"] {
            assert!(
                apply.input_schema["properties"]["ops"]["items"]["properties"]["op"]
                    .get(keyword)
                    .is_none(),
                "unica.apply.ops[].op must not publish a deep operation union"
            );
            assert!(
                apply.input_schema["properties"]["ops"]["items"]
                    .get(keyword)
                    .is_none(),
                "unica.apply.ops[] must not publish a deep operation union"
            );
        }
        assert_eq!(
            contract(&catalog.tools, "run").input_schema["properties"]["args"]["type"],
            "object"
        );
        assert!(
            contract(&catalog.tools, "run").input_schema["properties"]["op"]
                .get("enum")
                .is_none()
        );

        assert_eq!(
            catalog.semantics.find_projection,
            FindProjection::AddressCandidates
        );
        assert_eq!(
            catalog.semantics.search_projection,
            SearchProjection::ContentOrSymbolMatches
        );
        assert!(catalog.semantics.check_reads_persisted_state);
        assert!(catalog.semantics.apply_dry_run_uses_validator_registry);
        assert!(catalog.semantics.diff_is_read_only);
        assert!(catalog.semantics.diff_cursor_carries_both_source_revisions);
        assert!(catalog.semantics.diff_rejects_incomparable_node_kinds);
        assert!(
            catalog
                .semantics
                .docs_filters_source_kinds_not_provider_identities
        );
        assert!(catalog.semantics.apply_operations_come_from_node_can_data);
        assert!(catalog.semantics.run_dictionary_is_data_not_command_lines);
        assert!(catalog.semantics.empty_optional_result_slots_are_omitted);
        assert!(catalog.semantics.search_scope_is_logical_subtree_address);
        assert_eq!(
            input_field(&catalog.tools, "search", "scope")["description"],
            "logical subtree address"
        );

        assert_eq!(
            catalog
                .run_dictionary
                .iter()
                .map(|operation| operation.intent)
                .collect::<Vec<_>>(),
            [
                RunIntent::SourceCreate,
                RunIntent::SourceAttach,
                RunIntent::InfobaseCreate,
                RunIntent::InfobaseBuild,
                RunIntent::SourceDump,
                RunIntent::SourceConvert,
                RunIntent::ArtifactMake,
                RunIntent::ArtifactLoad,
                RunIntent::SyntaxCheck,
                RunIntent::TestRun,
                RunIntent::ClientRun,
                RunIntent::ExtensionSync,
            ]
        );
        assert!(catalog
            .run_dictionary
            .iter()
            .all(|operation| operation.name() != "tools-download"));
        let client_run = catalog
            .run_dictionary
            .iter()
            .find(|operation| operation.intent == RunIntent::ClientRun)
            .expect("client.run belongs to the v0.13 dictionary");
        assert!(client_run.terminal);
        assert!(client_run.rejects_sessions);
        assert_eq!(
            catalog
                .run_dictionary
                .iter()
                .filter(|operation| operation.implemented)
                .map(|operation| operation.name())
                .collect::<Vec<_>>(),
            ["syntax.check"]
        );

        let output = &catalog.result_envelope_schema;
        assert_eq!(output["type"], "object");
        assert_eq!(output["additionalProperties"], false);
        assert_eq!(output["required"], json!(["ok", "summary"]));
        assert_eq!(
            output["properties"]
                .as_object()
                .expect("result properties must be an object")
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "ok",
                "at",
                "summary",
                "data",
                "changed",
                "warnings",
                "diagnostics",
                "artifacts",
                "next",
                "rev",
                "cursor"
            ]
        );
        for forbidden in ["set", "sourceState", "fileExists", "job", "work"] {
            assert!(output["properties"].get(forbidden).is_none());
        }
        for slot in ["changed", "warnings", "diagnostics", "artifacts", "next"] {
            assert_eq!(output["properties"][slot]["type"], "array");
            assert_eq!(
                output["properties"][slot]["minItems"], 1,
                "empty `{slot}` must be omitted rather than serialized"
            );
        }
    }

    #[test]
    fn v13_run_dictionary_has_twelve_operations_without_query_execution() {
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");
        let names = catalog
            .run_dictionary
            .iter()
            .map(|operation| operation.name())
            .collect::<Vec<_>>();

        assert_eq!(names.len(), 12, "v0.13 Run dictionary drifted: {names:?}");
        assert!(
            !names.contains(&"query.execute"),
            "v0.13 must not publish query execution: {names:?}"
        );
    }
}

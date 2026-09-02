use crate::application::tool_contracts::SurfaceRelease;
use serde_json::{json, Value};

#[derive(Debug)]
pub(crate) struct V13ToolContract {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
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
    WorkspaceInitialize,
    SourceCreate,
    InfobaseCreate,
    InfobaseBuild,
    SourceDump,
    SourceConvert,
    ArtifactBuild,
    InfobaseConfigurationExport,
    InfobaseConfigurationLoad,
    InfobaseDump,
    InfobaseRestore,
    ClientRun,
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
            RunIntent::WorkspaceInitialize => "workspace.initialize",
            RunIntent::SourceCreate => "source.create",
            RunIntent::InfobaseCreate => "infobase.create",
            RunIntent::InfobaseBuild => "infobase.build",
            RunIntent::SourceDump => "source.dump",
            RunIntent::SourceConvert => "source.convert",
            RunIntent::ArtifactBuild => "artifact.build",
            RunIntent::InfobaseConfigurationExport => "infobase.configuration.export",
            RunIntent::InfobaseConfigurationLoad => "infobase.configuration.load",
            RunIntent::InfobaseDump => "infobase.dump",
            RunIntent::InfobaseRestore => "infobase.restore",
            RunIntent::ClientRun => "client.run",
        }
    }

    pub(crate) const fn description(&self) -> &'static str {
        match self.intent {
            RunIntent::WorkspaceInitialize => {
                "Create a missing v8project.yaml from autodetected 1C source sets."
            }
            RunIntent::SourceCreate => {
                "Create a new 1C source set in a requested supported source format."
            }
            RunIntent::InfobaseCreate => "Create an empty target 1C infobase.",
            RunIntent::InfobaseBuild => "Build or update a 1C infobase from attached sources.",
            RunIntent::SourceDump => "Export a 1C infobase into a workspace source set.",
            RunIntent::SourceConvert => "Convert source sets between supported source formats.",
            RunIntent::ArtifactBuild => {
                "Build a CF, CFE, EPF, or ERF artifact from attached sources."
            }
            RunIntent::InfobaseConfigurationExport => {
                "Export a working configuration, database configuration, or extension from an infobase to CF or CFE."
            }
            RunIntent::InfobaseConfigurationLoad => {
                "Load a CF or CFE configuration artifact into a target infobase."
            }
            RunIntent::InfobaseDump => {
                "Export the complete infobase to a DT transfer file; this is not a backup."
            }
            RunIntent::InfobaseRestore => {
                "Create or replace an infobase from a DT transfer file."
            }
            RunIntent::ClientRun => "Launch an interactive 1C client session.",
        }
    }

    pub(crate) const fn execution(&self) -> &'static str {
        match self.intent {
            RunIntent::ClientRun => "terminal",
            _ => "previewApply",
        }
    }

    pub(crate) const fn effects(&self) -> &'static [&'static str] {
        match self.intent {
            RunIntent::WorkspaceInitialize
            | RunIntent::SourceCreate
            | RunIntent::SourceConvert
            | RunIntent::ArtifactBuild => &["workspaceFiles"],
            RunIntent::SourceDump
            | RunIntent::InfobaseConfigurationExport
            | RunIntent::InfobaseDump => &["infobaseRead", "workspaceFiles"],
            RunIntent::InfobaseCreate
            | RunIntent::InfobaseBuild
            | RunIntent::InfobaseConfigurationLoad
            | RunIntent::InfobaseRestore => &["infobase"],
            RunIntent::ClientRun => &["clientSession"],
        }
    }

    pub(crate) fn args_schema(&self) -> Option<Value> {
        match self.intent {
            RunIntent::WorkspaceInitialize => Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {},
                "required": []
            })),
            _ => None,
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
                    description: "Inspect the workspace with no arguments, or read one logical 1C node by address.",
                    input_schema: schema(
                        json!({
                            "at": logical_address(),
                            "filter": data_object("Optional projection such as sections; valid only with at."),
                            "limit": limit("Maximum child items to return; valid only with at."),
                            "cursor": cursor("Continuation cursor from an earlier addressed view."),
                        }),
                        json!([]),
                    ),
                },
                V13ToolContract {
                    name: "apply",
                    description: "Preview or atomically apply typed edits to one logically addressed 1C node.",
                    input_schema: schema(
                        json!({
                            "at": logical_address(),
                            "ops": {
                                "type": "array",
                                "description": "Ordered operations advertised by the target node's can data.",
                                "minItems": 1,
                                "items": {
                                    "type": "object",
                                    "additionalProperties": false,
                                    "properties": {
                                        "op": {"type": "string", "description": "Operation name from the target node's can data."},
                                        "args": data_object("Arguments for this typed operation."),
                                    },
                                    "required": ["op"],
                                },
                            },
                            "dryRun": {"type": "boolean", "description": "Validate and return the plan without publishing when true.", "default": false},
                            "ifRev": {"type": "string", "description": "Optional revision fence from an earlier read."},
                        }),
                        json!(["at", "ops"]),
                    ),
                },
                V13ToolContract {
                    name: "find",
                    description: "Resolve a human query to canonical logical address candidates.",
                    input_schema: schema(
                        json!({
                            "query": {"type": "string", "description": "Object name or address fragment to resolve."},
                            "kind": {"type": "string", "description": "Optional logical kind such as Catalog or CommonModule."},
                            "limit": limit("Maximum address candidates to return."),
                        }),
                        json!(["query"]),
                    ),
                },
                V13ToolContract {
                    name: "search",
                    description: "Search BSL content or symbols, optionally under one logical subtree.",
                    input_schema: schema(
                        json!({
                            "query": {"type": "string", "description": "Literal BSL text or symbol to search for."},
                            "scope": logical_subtree_address(),
                            "regex": {"type": "boolean", "description": "Request regex matching; currently only false is implemented.", "default": false},
                            "limit": limit("Maximum matches to return."),
                        }),
                        json!(["query"]),
                    ),
                },
                V13ToolContract {
                    name: "check",
                    description: "Confirm workspace source-set admission, or validate one logical node: readability plus every validator its kind owns.",
                    input_schema: schema(
                        json!({
                            "at": logical_address(),
                        }),
                        json!([]),
                    ),
                },
                V13ToolContract {
                    name: "diff",
                    description: "Compare two readable logical nodes of the same kind without changing files.",
                    input_schema: schema(
                        json!({
                            "left": logical_address_with("Qualified logical address of the left node."),
                            "right": logical_address_with("Qualified logical address of the right node."),
                            "filter": data_object("Optional projection applied before comparison."),
                            "limit": limit("Maximum differences to return."),
                            "cursor": cursor("Continuation cursor from an earlier diff."),
                        }),
                        json!(["left", "right"]),
                    ),
                },
                V13ToolContract {
                    name: "run",
                    description: "List canonical runtime operations and their invocation contract, or preview/execute one implemented operation.",
                    input_schema: schema(
                        json!({
                            "op": {"type": "string", "description": "Canonical operation name; omit to list operation status."},
                            "args": data_object("Typed arguments for the selected operation."),
                            "dryRun": {"type": "boolean", "description": "Required by previewApply operations: true returns a non-mutating plan and revision; false requires ifRev and applies that plan."},
                            "ifRev": {"type": "string", "description": "Revision returned by a prior preview of the same previewApply operation; required when dryRun is false."},
                        }),
                        json!([]),
                    ),
                },
                V13ToolContract {
                    name: "docs",
                    description: "Search bundled Unica and safe 1C documentation by topic.",
                    input_schema: schema(
                        json!({
                            "query": {"type": "string", "description": "Documentation question or search phrase."},
                            "source": {"type": "string", "description": "Optional documented source kind, not a provider identity."},
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
    logical_address_with("Qualified logical address: <sourceSet>:<Kind>[.<Name>...]. Omit only for workspace bootstrap where allowed.")
}

fn logical_address_with(description: &'static str) -> Value {
    json!({"type": "string", "description": description})
}

fn logical_subtree_address() -> Value {
    json!({"type": "string", "description": "logical subtree address"})
}

fn data_object(description: &'static str) -> Value {
    json!({"type": "object", "description": description})
}

fn limit(description: &'static str) -> Value {
    json!({"type": "integer", "description": description, "minimum": 1})
}

fn cursor(description: &'static str) -> Value {
    json!({"type": "string", "description": description})
}

fn run_dictionary() -> Vec<RunOperation> {
    [
        RunIntent::WorkspaceInitialize,
        RunIntent::SourceCreate,
        RunIntent::InfobaseCreate,
        RunIntent::InfobaseBuild,
        RunIntent::SourceDump,
        RunIntent::SourceConvert,
        RunIntent::ArtifactBuild,
        RunIntent::InfobaseConfigurationExport,
        RunIntent::InfobaseConfigurationLoad,
        RunIntent::InfobaseDump,
        RunIntent::InfobaseRestore,
        RunIntent::ClientRun,
    ]
    .into_iter()
    .map(|intent| RunOperation {
        terminal: intent == RunIntent::ClientRun,
        rejects_sessions: intent == RunIntent::ClientRun,
        implemented: intent == RunIntent::WorkspaceInitialize,
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
            "cursor": cursor("Opaque continuation cursor issued by this result stream."),
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
            value["type"], "object",
            "{location} must remain an unconstrained shallow data object"
        );
        assert!(
            value.get("properties").is_none(),
            "{location} must remain shallow"
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
            json!([]),
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
        assert_schema(&catalog.tools, "check", json!([]), &["at"]);
        assert_schema(
            &catalog.tools,
            "diff",
            json!(["left", "right"]),
            &["left", "right", "filter", "limit", "cursor"],
        );
        assert_schema(
            &catalog.tools,
            "run",
            json!([]),
            &["op", "args", "dryRun", "ifRev"],
        );
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
            ("run", "ifRev"),
            ("docs", "query"),
            ("docs", "source"),
        ] {
            assert_field_type(&catalog.tools, tool, field, "string");
        }
        for (tool, field) in [
            ("view", "filter"),
            ("apply", "ops"),
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
            input_field(&catalog.tools, "diff", "filter"),
            "unica.diff.filter",
        );
        assert_data_object(input_field(&catalog.tools, "run", "args"), "unica.run.args");
        assert_eq!(
            input_field(&catalog.tools, "run", "dryRun")["type"],
            "boolean"
        );
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
                RunIntent::WorkspaceInitialize,
                RunIntent::SourceCreate,
                RunIntent::InfobaseCreate,
                RunIntent::InfobaseBuild,
                RunIntent::SourceDump,
                RunIntent::SourceConvert,
                RunIntent::ArtifactBuild,
                RunIntent::InfobaseConfigurationExport,
                RunIntent::InfobaseConfigurationLoad,
                RunIntent::InfobaseDump,
                RunIntent::InfobaseRestore,
                RunIntent::ClientRun,
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
            ["workspace.initialize"]
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
    fn canonical_arguments_are_described_within_wire_budget() {
        fn assert_described(location: &str, schema: &Value) {
            let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
                return;
            };
            for (name, property) in properties {
                let field = format!("{location}.{name}");
                let description = property
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                assert!(
                    !description.trim().is_empty(),
                    "published argument `{field}` has no model-facing description"
                );
                assert_described(&field, property);
                if let Some(items) = property.get("items") {
                    assert_described(&format!("{field}[]"), items);
                }
            }
        }

        let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog");
        for tool in &catalog.tools {
            assert_described(&format!("unica.{}", tool.name), &tool.input_schema);
        }
    }

    #[test]
    fn v13_run_dictionary_has_twelve_directional_runtime_intents() {
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");
        let names = catalog
            .run_dictionary
            .iter()
            .map(|operation| operation.name())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                "workspace.initialize",
                "source.create",
                "infobase.create",
                "infobase.build",
                "source.dump",
                "source.convert",
                "artifact.build",
                "infobase.configuration.export",
                "infobase.configuration.load",
                "infobase.dump",
                "infobase.restore",
                "client.run",
            ],
            "v0.13 Run dictionary must distinguish source builds, configuration transfers, and full infobase transfers"
        );
        for ambiguous_or_deferred in [
            "source.attach",
            "artifact.make",
            "artifact.load",
            "syntax.check",
            "test.run",
            "extension.sync",
            "query.execute",
        ] {
            assert!(
                !names.contains(&ambiguous_or_deferred),
                "v0.13 must not publish `{ambiguous_or_deferred}`: {names:?}"
            );
        }
        assert_eq!(
            catalog
                .run_dictionary
                .iter()
                .filter(|operation| operation.implemented)
                .map(|operation| operation.name())
                .collect::<Vec<_>>(),
            ["workspace.initialize"],
            "only the renamed initialization vertical is implemented in this contract slice"
        );
    }

    #[test]
    fn run_preview_apply_fields_describe_the_execution_protocol() {
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");
        for field in ["dryRun", "ifRev"] {
            let description = input_field(&catalog.tools, "run", field)["description"]
                .as_str()
                .expect("run protocol field description");
            assert!(
                description.contains("previewApply"),
                "unica.run.{field} must describe the previewApply execution protocol: {description}"
            );
            assert!(
                !description.contains("workspace-mutating"),
                "unica.run.{field} must also cover infobase and artifact effects: {description}"
            );
        }
    }

    #[test]
    fn v13_run_dictionary_has_twelve_operations_without_query_execution() {
        // The no-query guarantee remains independently active while the
        // directional-intents test above owns the exact operation names.
        v13_run_dictionary_has_twelve_directional_runtime_intents();
    }
}

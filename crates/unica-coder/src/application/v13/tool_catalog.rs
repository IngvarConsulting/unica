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
    InfobaseCreate,
    SourceImport,
    SourceExport,
    ArtifactBuild,
    CfExport,
    CfImport,
    InfobaseExport,
    InfobaseImport,
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
            RunIntent::InfobaseCreate => "infobase.create",
            RunIntent::SourceImport => "source.import",
            RunIntent::SourceExport => "source.export",
            RunIntent::ArtifactBuild => "artifact.build",
            RunIntent::CfExport => "cf.export",
            RunIntent::CfImport => "cf.import",
            RunIntent::InfobaseExport => "infobase.export",
            RunIntent::InfobaseImport => "infobase.import",
            RunIntent::ClientRun => "client.run",
        }
    }

    pub(crate) const fn description(&self) -> &'static str {
        match self.intent {
            RunIntent::InfobaseCreate => {
                "Create the empty infobase declared by v8project.yaml; refused when it already exists."
            }
            RunIntent::SourceImport => {
                "Import workspace sources into the infobase: build or update its configuration from the attached source sets."
            }
            RunIntent::SourceExport => {
                "Export the infobase into a workspace source set."
            }
            RunIntent::ArtifactBuild => {
                "Build a CF, CFE, EPF, or ERF artifact from attached sources."
            }
            RunIntent::CfExport => {
                "Export the working configuration, the database configuration, or an extension out of the infobase to a CF or CFE file."
            }
            RunIntent::CfImport => {
                "Import a CF or CFE file into the infobase as its configuration or extension."
            }
            RunIntent::InfobaseExport => {
                "Export the complete infobase to a DT transfer file; this is not a backup."
            }
            RunIntent::InfobaseImport => {
                "Import a DT transfer file as the infobase; the mode states whether an absent infobase is created or the data of an existing one is discarded."
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
            RunIntent::ArtifactBuild => &["workspaceFiles"],
            RunIntent::SourceExport | RunIntent::CfExport | RunIntent::InfobaseExport => {
                &["infobaseRead", "workspaceFiles"]
            }
            RunIntent::InfobaseCreate
            | RunIntent::SourceImport
            | RunIntent::CfImport
            | RunIntent::InfobaseImport => &["infobase"],
            RunIntent::ClientRun => &["clientSession"],
        }
    }

    pub(crate) fn args_schema(&self) -> Option<Value> {
        match self.intent {
            RunIntent::CfExport => Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "state": {"type": "string", "enum": ["working", "database"]},
                    "output": {"type": "string", "description": "Workspace-relative .cf or .cfe output path."},
                    "extension": {"type": "string", "description": "1C extension name; omit for the main configuration."}
                },
                "required": ["state", "output"]
            })),
            RunIntent::InfobaseExport => Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "output": {"type": "string", "description": "Workspace-relative .dt output path."}
                },
                "required": ["output"]
            })),
            RunIntent::InfobaseCreate => Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {},
                "required": []
            })),
            RunIntent::SourceImport => Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "sourceSet": {"type": "string", "description": "Name of one source set declared in v8project.yaml; omit to import every declared source set."},
                    "fullRebuild": {"type": "boolean", "default": false, "description": "Clear the runner's change cache and import everything instead of the changed files only."}
                }
            })),
            RunIntent::CfImport => Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "input": {"type": "string", "description": "Workspace-relative .cf file to import as the main configuration, or .cfe file to import as an extension."},
                    "extension": {"type": "string", "description": "Extension name the infobase will know the .cfe by; required for .cfe and refused for .cf."}
                },
                "required": ["input"]
            })),
            RunIntent::InfobaseImport => Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "input": {"type": "string", "description": "Workspace-relative .dt transfer file to load."},
                    "mode": {
                        "type": "string",
                        "enum": ["create", "replace"],
                        "description": "Which irreversible change is allowed: create an absent infobase, or replace the data of an existing one. A mode that does not match the observed target is refused."
                    }
                },
                "required": ["input", "mode"]
            })),
            RunIntent::ClientRun => Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "clientMode": {"type": "string", "enum": ["designer", "thin", "thick", "ordinary"], "description": "1C client to launch."},
                    "execute": {"type": "string", "description": "Workspace-relative .epf or .erf external processor to run with /Execute; enterprise clients only."},
                    "waitForExit": {"type": "boolean", "default": false, "description": "Wait for the external processor session to exit; requires execute and waitTimeoutMs."},
                    "waitTimeoutMs": {"type": "integer", "minimum": 1, "maximum": 86400000, "description": "Bound for waitForExit in milliseconds."}
                },
                "required": ["clientMode"]
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
                    input_schema: schema_requiring_the_fence(
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
                            "ifRev": {"type": "string", "description": "Revision returned by a prior dryRun preview; required when dryRun is false."},
                        }),
                        json!(["at", "ops"]),
                    ),
                },
                V13ToolContract {
                    name: "resolve",
                    description: "Emergency bridge between a logical address and the source layout, in both directions. Use it only when a path arrived from outside Unica - a diff, a build log, a stack trace - or when a file has to be opened outside Unica. To find an object by name use search; to read it use view.",
                    input_schema: schema(
                        json!({
                            "at": logical_address_with("Qualified logical address whose source location is needed."),
                            "path": {"type": "string", "description": "Path to a source file or object directory, absolute or relative to the workspace root."},
                        }),
                        json!([]),
                    ),
                },
                V13ToolContract {
                    name: "search",
                    description: "Search one corpus for a query: BSL module text, or the names and synonyms of metadata objects. Optionally under one logical subtree.",
                    input_schema: schema(
                        json!({
                            "query": {"type": "string", "description": "Literal BSL text, symbol, or metadata name to search for."},
                            "corpus": {"type": "string", "enum": ["text", "names"], "description": "Where to search: `text` matches BSL module content and answers scope, line, column and snippet; `names` matches metadata names and synonyms and answers at, kind and title. Defaults to `text`.", "default": "text"},
                            "kind": {"type": "string", "description": "`names` corpus only: narrow the search to one logical node kind."},
                            "role": {"type": "string", "enum": ["lexical", "symbol", "semantic"], "description": "`text` corpus only: which provider answers. `lexical` matches literally, `symbol` uses the symbol index, `semantic` matches by meaning. Omit for the literal search Unica performs itself."},
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

/// Схема `apply`: забор обязателен, когда это применение, а не предпросмотр.
///
/// Условие объявлено структурно, а не только словами в описании поля: иначе
/// хост, собирающий вызов по схеме, сгенерирует применение без забора, которое
/// разборщик затем отвергнет. Опущенный `dryRun` равен `false`, и отсутствие
/// поля условие покрывает тем же `const`: пустая ветвь `if` проходит.
fn schema_requiring_the_fence(properties: Value, required: Value) -> Value {
    let mut schema = schema(properties, required);
    let object = schema
        .as_object_mut()
        .expect("the schema builder returns an object");
    object.insert(
        "if".to_string(),
        json!({"properties": {"dryRun": {"const": false}}}),
    );
    object.insert("then".to_string(), json!({"required": ["ifRev"]}));
    schema
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
        RunIntent::InfobaseCreate,
        RunIntent::SourceImport,
        RunIntent::SourceExport,
        RunIntent::ArtifactBuild,
        RunIntent::CfExport,
        RunIntent::CfImport,
        RunIntent::InfobaseExport,
        RunIntent::InfobaseImport,
        RunIntent::ClientRun,
    ]
    .into_iter()
    .map(|intent| RunOperation {
        terminal: intent == RunIntent::ClientRun,
        rejects_sessions: intent == RunIntent::ClientRun,
        implemented: matches!(
            intent,
            RunIntent::InfobaseCreate
                | RunIntent::SourceImport
                | RunIntent::CfExport
                | RunIntent::CfImport
                | RunIntent::InfobaseExport
                | RunIntent::InfobaseImport
                | RunIntent::ClientRun
        ),
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
        // Физический путь на поверхности живёт ровно в одном инструменте.
        // Аварийный мост затем и заведён, чтобы путь не просачивался в
        // частые ответы: приглашение прочитать файл напрямую подрывает
        // адресное пространство, ради которого весь слой и существует.
        let forbidden: &[&str] = if name == "resolve" {
            &["jobId", "provider", "providerId"]
        } else {
            &["jobId", "path", "provider", "providerId"]
        };
        for forbidden in forbidden {
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

    /// Роль объявлена закрытым набором и **без умолчания**: отсутствие роли
    /// не равно `lexical`, оно означает поиск силами самой Unica, без
    /// внешнего провайдера и без его цены.
    #[test]
    fn search_publishes_three_provider_roles_and_stays_literal_without_one() {
        let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog");
        let role = input_field(&catalog.tools, "search", "role");
        assert_eq!(role["type"], "string");
        assert_eq!(
            role["enum"],
            serde_json::json!(["lexical", "symbol", "semantic"])
        );
        assert!(
            role.get("default").is_none(),
            "у роли не должно быть умолчания: {role}"
        );
    }

    /// Свод объявлен закрытым набором и по умолчанию текстовый: третий свод
    /// нельзя добавить молча, а существующий вызов без `corpus` обязан
    /// остаться текстовым поиском, каким он был.
    #[test]
    fn search_publishes_exactly_two_corpora_and_defaults_to_text() {
        let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog");
        let corpus = input_field(&catalog.tools, "search", "corpus");
        assert_eq!(corpus["type"], "string");
        assert_eq!(corpus["enum"], serde_json::json!(["text", "names"]));
        assert_eq!(corpus["default"], "text");
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
            ["view", "apply", "resolve", "search", "check", "diff", "run", "docs"]
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
        assert_schema(&catalog.tools, "resolve", json!([]), &["at", "path"]);
        assert_schema(
            &catalog.tools,
            "search",
            json!(["query"]),
            &["query", "corpus", "kind", "role", "scope", "regex", "limit"],
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
            ("resolve", "at"),
            ("resolve", "path"),
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
        for (tool, field) in [("view", "limit"), ("search", "limit"), ("diff", "limit")] {
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
                RunIntent::InfobaseCreate,
                RunIntent::SourceImport,
                RunIntent::SourceExport,
                RunIntent::ArtifactBuild,
                RunIntent::CfExport,
                RunIntent::CfImport,
                RunIntent::InfobaseExport,
                RunIntent::InfobaseImport,
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
            [
                "infobase.create",
                "source.import",
                "cf.export",
                "cf.import",
                "infobase.export",
                "infobase.import",
                "client.run"
            ],
            "реализованы создание базы, импорт исходников, обе пары export/import на слоях cf и infobase и терминальный запуск клиента; проектный файл в словаре не числится вовсе"
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
    // Имя удерживает счёт, которого больше нет: две операции сняты решением
    // DEC.2026-09-09.PROJECT-CONFIG-IS-HANDWRITTEN, третья —
    // DEC.2026-09-15.SOURCE-CONVERT-LEAVES-THE-DICTIONARY. Переименовать нельзя —
    // на это имя ссылаются принятые записи реестра как на доказательство, а
    // переименование там читается как правка обещания. Счёт в имени проверки
    // устаревает так же, как счёт в прозе правила (#798).
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
                "infobase.create",
                "source.import",
                "source.export",
                "artifact.build",
                "cf.export",
                "cf.import",
                "infobase.export",
                "infobase.import",
                "client.run",
            ],
            "словарь `run` различает сборку исходников, перенос конфигурации и перенос базы целиком — и не держит операции, которым платформа не нужна"
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
            [
                "infobase.create",
                "source.import",
                "cf.export",
                "cf.import",
                "infobase.export",
                "infobase.import",
                "client.run"
            ],
            "реализованы создание базы, импорт исходников, обе пары export/import на слоях cf и infobase и терминальный запуск клиента; проектный файл в словаре не числится вовсе"
        );
    }

    #[test]
    fn v13_source_import_is_implemented_with_closed_source_set_and_full_rebuild() {
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");
        let import = catalog
            .run_dictionary
            .iter()
            .find(|operation| operation.intent == RunIntent::SourceImport)
            .expect("source.import belongs to the v0.13 dictionary");
        assert!(import.implemented);
        assert_eq!(import.execution(), "previewApply");
        assert_eq!(import.effects(), &["infobase"]);
        let schema = import
            .args_schema()
            .expect("source.import publishes its args");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]
                .as_object()
                .map(|properties| properties.keys().cloned().collect::<Vec<_>>()),
            Some(vec!["sourceSet".to_string(), "fullRebuild".to_string()])
        );
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn v13_infobase_create_is_implemented_without_arguments() {
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");
        let create = catalog
            .run_dictionary
            .iter()
            .find(|operation| operation.intent == RunIntent::InfobaseCreate)
            .expect("infobase.create belongs to the v0.13 dictionary");
        assert!(create.implemented);
        assert_eq!(create.execution(), "previewApply");
        assert_eq!(create.effects(), &["infobase"]);
        let schema = create
            .args_schema()
            .expect("infobase.create publishes its args");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"], json!({}));
        assert_eq!(
            schema["required"],
            json!([]),
            "соединение задаёт проектный файл, аргументов у создания базы нет"
        );
    }

    #[test]
    fn v13_cf_import_is_implemented_with_a_closed_input_and_extension() {
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");
        let import = catalog
            .run_dictionary
            .iter()
            .find(|operation| operation.intent == RunIntent::CfImport)
            .expect("cf.import belongs to the v0.13 dictionary");
        assert!(import.implemented);
        assert_eq!(import.execution(), "previewApply");
        assert_eq!(import.effects(), &["infobase"]);
        let schema = import.args_schema().expect("cf.import publishes its args");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["input"]));
        assert_eq!(
            schema["properties"]
                .as_object()
                .map(|properties| properties.keys().cloned().collect::<Vec<_>>()),
            Some(vec!["input".to_string(), "extension".to_string()]),
            "режим один — load; merge с внешним файлом настроек за словарём"
        );
    }

    #[test]
    fn v13_run_names_read_as_layer_and_direction() {
        // Имя операции — `<слой>.<глагол>`: слой называет, с чем работаем,
        // глагол считается относительно базы — `export` наружу, `import`
        // внутрь. Одно направление не называется двумя словами
        // (DEC.2026-09-15.RUN-NAMES-READ-AS-LAYER-AND-DIRECTION).
        const LAYERS: [&str; 5] = ["infobase", "cf", "source", "artifact", "client"];
        const VERBS: [&str; 5] = ["create", "export", "import", "build", "run"];
        const RETIRED_VERBS: [&str; 4] = ["dump", "restore", "load", "convert"];
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");
        let names = catalog
            .run_dictionary
            .iter()
            .map(|operation| operation.name())
            .collect::<Vec<_>>();
        for name in &names {
            let (layer, verb) = name
                .split_once('.')
                .unwrap_or_else(|| panic!("{name} is not `<layer>.<verb>`"));
            assert!(LAYERS.contains(&layer), "{name}: unknown layer `{layer}`");
            assert!(VERBS.contains(&verb), "{name}: unknown verb `{verb}`");
            assert!(
                !RETIRED_VERBS.contains(&verb),
                "{name}: `{verb}` names a direction the dictionary already spells as export/import"
            );
        }
        // Каждый слой, который выгружается из базы, загружается в неё тем же
        // словом: у `export` есть парный `import`, и наоборот.
        for layer in ["infobase", "cf", "source"] {
            for (verb, pair) in [("export", "import"), ("import", "export")] {
                let name = format!("{layer}.{verb}");
                let paired = format!("{layer}.{pair}");
                assert_eq!(
                    names.contains(&name.as_str()),
                    names.contains(&paired.as_str()),
                    "{name} needs {paired}"
                );
            }
        }
        assert_eq!(
            names,
            [
                "infobase.create",
                "source.import",
                "source.export",
                "artifact.build",
                "cf.export",
                "cf.import",
                "infobase.export",
                "infobase.import",
                "client.run",
            ]
        );
    }

    #[test]
    fn v13_run_dictionary_names_no_operation_that_needs_edt() {
        // Unica читает и пишет выгрузку Designer; операция, которой нужен EDT
        // или его CLI, в словаре была бы адресом в тупик
        // (DEC.2026-09-15.SOURCE-CONVERT-LEAVES-THE-DICTIONARY).
        let catalog =
            catalog_for(SurfaceRelease::V13).expect("v0.13 catalog must be test-loadable");
        for operation in &catalog.run_dictionary {
            assert!(
                !operation.name().contains("convert"),
                "{} converts between formats, and Unica supports one",
                operation.name()
            );
            let description = operation.description().to_ascii_lowercase();
            assert!(
                !description.contains("edt") && !description.contains("source format"),
                "{} promises a format Unica does not read: {description}",
                operation.name()
            );
        }
        assert!(catalog
            .run_dictionary
            .iter()
            .all(|operation| operation.name() != "source.convert"));
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

    #[test]
    fn v13_client_run_is_implemented_as_a_terminal_operation_with_closed_arguments() {
        let catalog = catalog_for(SurfaceRelease::V13).expect("canonical catalog exists");
        let client_run = catalog
            .run_dictionary
            .iter()
            .find(|operation| operation.intent == RunIntent::ClientRun)
            .expect("client.run belongs to the dictionary");
        assert!(client_run.implemented);
        assert_eq!(client_run.execution(), "terminal");
        assert_eq!(client_run.effects(), ["clientSession"]);
        let schema = client_run
            .args_schema()
            .expect("implemented operations publish argsSchema");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["clientMode"]));
        assert_eq!(
            schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["clientMode", "execute", "waitForExit", "waitTimeoutMs"]
        );
        assert_eq!(
            schema["properties"]["clientMode"]["enum"],
            json!(["designer", "thin", "thick", "ordinary"])
        );
    }

    #[test]
    fn v13_infobase_exports_are_implemented_with_closed_agent_facing_arguments() {
        let catalog = catalog_for(SurfaceRelease::V13).expect("v0.13 catalog");
        let operation = |intent| {
            catalog
                .run_dictionary
                .iter()
                .find(|operation| operation.intent == intent)
                .expect("runtime operation")
        };

        let configuration = operation(RunIntent::CfExport);
        assert!(configuration.implemented);
        assert_eq!(
            configuration.args_schema(),
            Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "state": {"type": "string", "enum": ["working", "database"]},
                    "output": {"type": "string", "description": "Workspace-relative .cf or .cfe output path."},
                    "extension": {"type": "string", "description": "1C extension name; omit for the main configuration."}
                },
                "required": ["state", "output"]
            }))
        );

        let dump = operation(RunIntent::InfobaseExport);
        assert!(dump.implemented);
        assert_eq!(
            dump.args_schema(),
            Some(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "output": {"type": "string", "description": "Workspace-relative .dt output path."}
                },
                "required": ["output"]
            }))
        );
    }
}

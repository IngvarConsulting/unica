use super::operation_descriptors::{
    diagnostic_action_descriptor, native_operation_descriptor, native_path_alias_groups,
    DIAGNOSTIC_ACTION_DESCRIPTORS,
};
use super::source_navigation::SOURCE_NAVIGATION_LIMIT_MAX;
#[cfg(test)]
use super::ToolExecution;
use super::{
    CodeIntelligenceOperation, InvocationMode, RuntimeJobAction, SourceNavigationOperation,
    SourceResourceOperation, ToolHandler, ToolSpec,
};
use crate::domain::diagnostics::{DiagnosticAction, LIVE_DIAGNOSTIC_PROVIDERS};
use crate::domain::form_edit::{form_edit_definition_schema, validate_form_edit_definition};
use crate::domain::role::{
    all_role_right_names, parse_role_edit_request, ROLE_METADATA_PATH_PATTERN,
    ROLE_OBJECT_NAME_PATTERN,
};
use crate::domain::source_resources::{SOURCE_READ_LIMIT_MAX, SOURCE_RESOURCE_PAGE_LIMIT_MAX};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

const COMMON_ARGS: &[&str] = &["cwd", "confirm"];
const MUTATION_ARGS: &[&str] = &["dryRun"];
/// How much of a logical address a bridged reader can actually use. Publishing
/// `metadataPath` on a tool that never reads one would be a lie in the schema,
/// so the three cases are distinguished rather than collapsed into a flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogicalAddress {
    /// The target is the source root itself: `unica.cf.*` reads
    /// `Configuration.xml`, which has no address.
    Absent,
    /// The address narrows the read, and its absence selects the whole set:
    /// `unica.subsystem.info` answers with the entire registered tree.
    Optional,
    /// The tool reads exactly one addressed object.
    Required,
}

impl LogicalAddress {
    /// The arguments the logical branch requires.
    const fn required_args(self) -> &'static [&'static str] {
        match self {
            Self::Absent | Self::Optional => &["sourceSet"],
            Self::Required => &["sourceSet", "metadataPath"],
        }
    }

    const fn publishes_address(self) -> bool {
        !matches!(self, Self::Absent)
    }
}

/// ADR-0049 transitional bridge: tool name, its canonical legacy target
/// argument, and how much address the tool takes. Removing the legacy column is
/// the separate per-tool slice ADR-0021 §13 requires.
const BRIDGED_SELECTORS: &[(&str, &str, LogicalAddress)] = &[
    ("unica.cf.info", "ConfigPath", LogicalAddress::Absent),
    ("unica.cf.validate", "ConfigPath", LogicalAddress::Absent),
    (
        "unica.subsystem.info",
        "SubsystemPath",
        LogicalAddress::Optional,
    ),
    (
        "unica.subsystem.validate",
        "SubsystemPath",
        LogicalAddress::Required,
    ),
    ("unica.role.info", "RightsPath", LogicalAddress::Required),
    (
        "unica.role.validate",
        "RightsPath",
        LogicalAddress::Required,
    ),
    ("unica.form.info", "FormPath", LogicalAddress::Required),
    ("unica.form.validate", "FormPath", LogicalAddress::Required),
    ("unica.dcs.info", "TemplatePath", LogicalAddress::Required),
    (
        "unica.dcs.validate",
        "TemplatePath",
        LogicalAddress::Required,
    ),
    ("unica.mxl.info", "TemplatePath", LogicalAddress::Required),
    (
        "unica.mxl.validate",
        "TemplatePath",
        LogicalAddress::Required,
    ),
    (
        "unica.mxl.decompile",
        "TemplatePath",
        LogicalAddress::Required,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReaderMigrationMode {
    Bridge,
    DirectSwitch,
}

/// Production-owned migration inventory. Schema routing and parity evidence
/// consume this exact owner rather than maintaining independent reader lists.
const READER_MIGRATION_INVENTORY: &[(&str, ReaderMigrationMode)] = &[
    ("unica.cf.info", ReaderMigrationMode::Bridge),
    ("unica.cf.validate", ReaderMigrationMode::Bridge),
    ("unica.subsystem.info", ReaderMigrationMode::Bridge),
    ("unica.subsystem.validate", ReaderMigrationMode::Bridge),
    ("unica.role.info", ReaderMigrationMode::Bridge),
    ("unica.role.validate", ReaderMigrationMode::Bridge),
    ("unica.form.info", ReaderMigrationMode::Bridge),
    ("unica.form.validate", ReaderMigrationMode::Bridge),
    ("unica.dcs.info", ReaderMigrationMode::Bridge),
    ("unica.dcs.validate", ReaderMigrationMode::Bridge),
    ("unica.mxl.info", ReaderMigrationMode::Bridge),
    ("unica.mxl.validate", ReaderMigrationMode::Bridge),
    ("unica.mxl.decompile", ReaderMigrationMode::Bridge),
    ("unica.code.diagnostics", ReaderMigrationMode::DirectSwitch),
];

pub(crate) fn authoritative_reader_migration_inventory(
) -> impl Iterator<Item = (&'static str, ReaderMigrationMode)> {
    READER_MIGRATION_INVENTORY.iter().copied()
}

/// Arguments a bridged reader still accepts but must not advertise: no branch
/// of its schema can honour them, and publishing one would name a selector the
/// tool cannot use. Refusing them instead would break calls that worked before
/// the bridge, which it does not do.
///
/// `MetadataPath` is always here. The shared catch-all hands it to every native
/// tool and no handler has ever read it; once the bridge gave the lowercase
/// name a meaning, publishing both would put two arguments differing only in
/// case, with different semantics, in one schema.
fn unpublished_bridge_args(name: &str) -> &'static [&'static str] {
    match bridged_selector(name) {
        Some((_, address)) if !address.publishes_address() => &["metadataPath", "MetadataPath"],
        Some(_) => &["MetadataPath"],
        None => &[],
    }
}

fn bridged_selector(name: &str) -> Option<(&'static str, LogicalAddress)> {
    if !authoritative_reader_migration_inventory()
        .any(|(reader, mode)| reader == name && mode == ReaderMigrationMode::Bridge)
    {
        return None;
    }
    BRIDGED_SELECTORS
        .iter()
        .find(|(tool, _, _)| *tool == name)
        .map(|(_, legacy, address)| (*legacy, *address))
}
const CODE_PATCH_ARGS: &[&str] = &[
    "sourceSet",
    "metadataPath",
    "operation",
    "selector",
    "content",
    "position",
];
/// `cf.info` answers with typed data, so the levers that existed to shrink its
/// printed report -- `Mode`, `Section`, `Limit`, `Offset` -- select nothing any
/// more and are not published.
const CF_INFO_ARGS: &[&str] = &["ConfigPath", "configPath", "Path", "path"];
/// `role.info` answers with typed data: `ShowDenied` selected nothing once the
/// denied list is always present, and pagination cut printed lines.
const ROLE_INFO_ARGS: &[&str] = &["RightsPath", "rightsPath", "Path", "path"];
const ROLE_EDIT_ARGS: &[&str] = &["sourceSet", "metadataPath", "operations", "dryRun"];
/// `subsystem.info` answers with typed data: the path selects a complete tree
/// or a concrete subsystem with its focused ancestor-and-descendant context,
/// so `Mode` no longer selects a printable projection.
const SUBSYSTEM_INFO_ARGS: &[&str] = &["SubsystemPath", "subsystemPath", "Path", "path"];

/// A typed reader publishes only what it reads. `dcs.info` and `form.info`
/// answer with every section at once, so `Mode`, `Raw`, `Name`, `Limit` and
/// `Expand` no longer select or trim anything (ADR-0023).
const DCS_INFO_ARGS: &[&str] = &["TemplatePath", "templatePath", "Path", "path"];
const FORM_INFO_ARGS: &[&str] = &["FormPath", "formPath", "Path", "path"];
/// `mxl.info` answers with typed data. `WithText` stays: it selects cell
/// content, the way `includeMethods` selects methods in ADR-0020. `Format`,
/// `MaxParams`, `Limit` and `Offset` only shaped a printed report. `SrcDir`
/// addressed a template only together with `ProcessorName` and `TemplateName`,
/// and `TemplatePath` has always been required, so that composite address was
/// never reachable through the published schema.
const MXL_INFO_ARGS: &[&str] = &[
    "TemplatePath",
    "templatePath",
    "Path",
    "path",
    "WithText",
    "withText",
];
/// `cfe.diff` answers with typed data: `Mode` chose between two views of one
/// extension, and both are now reported together.
const CFE_DIFF_ARGS: &[&str] = &["ExtensionPath", "extensionPath", "ConfigPath", "configPath"];
const XDTO_INFO_ARGS: &[&str] = &["sourceSet", "metadataPath", "typeName", "limit", "cursor"];
const XDTO_EDIT_ARGS: &[&str] = &["sourceSet", "metadataPath", "operations"];
/// ADR-0071: the closed camelCase tags of the `operations` union, in the
/// order the schema publishes their variants.
const XDTO_EDIT_OPS: &[&str] = &[
    "addValueType",
    "addObjectType",
    "addProperty",
    "removeType",
    "removeProperty",
];
/// ADR-0071: the retired flat form of `unica.xdto.edit`. Any of these at the
/// top level fails closed with `legacy_arguments_removed` before dispatch.
const XDTO_EDIT_LEGACY_ARGS: &[&str] = &[
    "operation",
    "name",
    "base",
    "typeName",
    "propertyPath",
    "property",
];
const RUNTIME_JOB_STATUS_ARGS: &[&str] = &["jobId"];
const RUNTIME_JOB_WAIT_ARGS: &[&str] = &["jobId", "timeoutSeconds"];
const RUNTIME_JOB_LOGS_ARGS: &[&str] = &["jobId", "tailChars"];
const SOURCE_RESOLVE_ARGS: &[&str] = &[
    "sourceSet",
    "query",
    "mode",
    "targetKind",
    "limit",
    "cursor",
];
const SOURCE_CHILDREN_ARGS: &[&str] = &["sourceSet", "metadataPath", "limit", "cursor"];
const SOURCE_LOCATE_ARGS: &[&str] = &["sourceSet", "path"];
const SOURCE_RESOURCES_ARGS: &[&str] = &[
    "sourceSet",
    "metadataPath",
    "scope",
    "snapshotId",
    "cursor",
    "limit",
];
const SOURCE_READ_ARGS: &[&str] = &["snapshotId", "resourceId", "offset", "limit"];
pub(crate) const DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS: u64 = 30;
pub(crate) const DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS: u64 = 3600;

const CFE_PATCH_METHOD_CONTEXTS: &[&str] = &["НаСервере", "НаКлиенте", "НаСервереБезКонтекста"];
const CFE_PATCH_METHOD_INTERCEPTOR_TYPES: &[&str] = &["Before", "After"];
const CFE_PATCH_METHOD_IDENTIFIER_PATTERN: &str = r"^[A-Za-z_А-Яа-яЁё][A-Za-z0-9_А-Яа-яЁё]*$";

const NATIVE_XML_DSL_ARGS: &[&str] = &[
    "BodyLimit",
    "BorrowMainAttribute",
    "Capability",
    "CIPath",
    "CompatibilityMode",
    "ConfigDir",
    "ConfigPath",
    "Context",
    "CreateIfMissing",
    "DataSet",
    "DefinitionFile",
    "Detailed",
    "EmitDsl",
    "ExtensionPath",
    "Expand",
    "Force",
    "FromObject",
    "FormName",
    "FormPath",
    "Format",
    "InterceptorType",
    "JsonPath",
    "Kind",
    "Lang",
    "Language",
    "Limit",
    "IsFunction",
    "MaxErrors",
    "MethodName",
    "MetadataPath",
    "Mode",
    "ModulePath",
    "Name",
    "NamePrefix",
    "NoSelection",
    "NoRole",
    "NoValidate",
    "Object",
    "ObjectName",
    "ObjectPath",
    "Offset",
    "Operation",
    "OutputDir",
    "OutputPath",
    "Parent",
    "Path",
    "ProcessorName",
    "Purpose",
    "RightsPath",
    "Raw",
    "Section",
    "Set",
    "SetDefault",
    "SetMainSKD",
    "ShowDenied",
    "SrcDir",
    "SubsystemPath",
    "Synonym",
    "TemplateName",
    "TemplatePath",
    "TemplateType",
    "TargetPath",
    "Value",
    "Variant",
    "Vendor",
    "Version",
    "WithText",
    "bodyLimit",
    "borrowMainAttribute",
    "capability",
    "ciPath",
    "compatibilityMode",
    "configDir",
    "configPath",
    "context",
    "createIfMissing",
    "dataSet",
    "definitionFile",
    "detailed",
    "emitDsl",
    "extensionPath",
    "expand",
    "force",
    "fromObject",
    "formName",
    "formPath",
    "format",
    "interceptorType",
    "jsonPath",
    "kind",
    "lang",
    "language",
    "limit",
    "isFunction",
    "maxErrors",
    "methodName",
    "metadataPath",
    "mode",
    "modulePath",
    "name",
    "namePrefix",
    "noSelection",
    "noRole",
    "noValidate",
    "object",
    "objectName",
    "objectPath",
    "offset",
    "operation",
    "outputDir",
    "outputPath",
    "parent",
    "path",
    "processorName",
    "purpose",
    "rightsPath",
    "raw",
    "section",
    "set",
    "setDefault",
    "setMainSKD",
    "showDenied",
    "srcDir",
    "subsystemPath",
    "synonym",
    "templateName",
    "templatePath",
    "templateType",
    "targetPath",
    "value",
    "variant",
    "vendor",
    "version",
    "withText",
];

const EXTERNAL_INIT_ARGS: &[&str] = &["FormName", "Name", "OutputDir", "Synonym"];

const BUILD_ARGS: &[&str] = &[
    "config",
    "database",
    "dbPassword",
    "dbUser",
    "format",
    "infobase",
    "mode",
    "password",
    "path",
    "sourceDir",
    "sourceSet",
    "target",
    "user",
];

/// `unica.build.make` alone: the destination of the exported artifact and, for
/// a cfe export, the extension it is exported from.
const BUILD_MAKE_ARGS: &[&str] = &["extension", "output"];

const RUNTIME_ARGS: &[&str] = &[
    "allExtensions",
    "builder",
    "c",
    "checkUseModality",
    "checkUseSynchronousCalls",
    "clientMode",
    "config",
    "configLogIntegrity",
    "connection",
    "distributiveModules",
    "emptyHandlers",
    "execute",
    "stderrOutput",
    "extension",
    "externalConnection",
    "externalConnectionServer",
    "features",
    "filterTags",
    "format",
    "force",
    "fullOutput",
    "fullRebuild",
    "handlersExistence",
    "ignoreTags",
    "incorrectReferences",
    "mcpConfig",
    "mcpPort",
    "mobileAppClient",
    "mobileAppServer",
    "mobileClient",
    "mobileClientDigiSign",
    "mode",
    "module",
    "object",
    "objects",
    "operation",
    "output",
    "path",
    "projects",
    "rawKeys",
    "scenarioFilters",
    "server",
    "settings",
    "sourceSet",
    "sourceSets",
    "sources",
    "testRunner",
    "testScope",
    "thickClientManagedApplication",
    "thickClientOrdinaryApplication",
    "thickClientServerManagedApplication",
    "thickClientServerOrdinaryApplication",
    "thinClient",
    "tool",
    "unsupportedFunctional",
    "unreferenceProcedures",
    "usePrivilegedMode",
    "waitForExit",
    "waitTimeoutMs",
    "webClient",
    "workdir",
];

pub(super) const RUNTIME_OPERATIONS: &[&str] = &[
    "config-init",
    "init",
    "build",
    "dump",
    "convert",
    "make",
    "load",
    "syntax",
    "test",
    "launch",
    "extensions",
    "tools-download",
];

const RUNTIME_STRING_ARGS: &[&str] = &[
    "builder",
    "c",
    "clientMode",
    "config",
    "connection",
    "execute",
    "stderrOutput",
    "extension",
    "format",
    "mcpConfig",
    "mode",
    "module",
    "object",
    "operation",
    "output",
    "path",
    "settings",
    "sourceSet",
    "testRunner",
    "testScope",
    "tool",
    "workdir",
];

const RUNTIME_ARRAY_ARGS: &[&str] = &[
    "features",
    "filterTags",
    "ignoreTags",
    "objects",
    "projects",
    "rawKeys",
    "scenarioFilters",
    "sourceSets",
];

const RUNTIME_CLIENT_MODES: &[&str] = &["designer", "thin", "thick", "ordinary", "mcp", "mcp-va"];
const RUNTIME_TEST_RUNNERS: &[&str] = &["yaxunit", "va"];
const RUNTIME_TEST_SCOPES: &[&str] = &["all", "module"];
const RUNTIME_TOOLS: &[&str] = &["yaxunit", "vanessa", "client-mcp"];
const RUNTIME_DUMP_MODES: &[&str] = &["full", "incremental", "partial"];
const RUNTIME_LOAD_MODES: &[&str] = &["load", "merge"];
pub(super) const RUNTIME_SYNTAX_MODES: &[&str] = &["designer-config", "designer-modules", "edt"];

const RUNTIME_CONFIG_INIT_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "sourceSet",
    "connection",
    "format",
    "builder",
    "force",
];
const RUNTIME_INIT_ARGS: &[&str] = &["operation", "config", "workdir"];
const RUNTIME_BUILD_OPERATION_ARGS: &[&str] =
    &["operation", "config", "workdir", "sourceSet", "fullRebuild"];
const RUNTIME_DUMP_OPERATION_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "mode",
    "object",
    "objects",
    "sourceSet",
    "extension",
];
const RUNTIME_CONVERT_OPERATION_ARGS: &[&str] =
    &["operation", "config", "workdir", "sourceSet", "output"];
const RUNTIME_MAKE_OPERATION_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "output",
    "sourceSet",
    "extension",
];
const RUNTIME_LOAD_OPERATION_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "path",
    "mode",
    "settings",
    "extension",
];
const RUNTIME_SYNTAX_OPERATION_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "mode",
    "server",
    "thinClient",
    "webClient",
    "mobileClient",
    "externalConnection",
    "externalConnectionServer",
    "thickClientManagedApplication",
    "thickClientServerManagedApplication",
    "thickClientOrdinaryApplication",
    "thickClientServerOrdinaryApplication",
    "mobileAppClient",
    "mobileAppServer",
    "mobileClientDigiSign",
    "distributiveModules",
    "unreferenceProcedures",
    "handlersExistence",
    "emptyHandlers",
    "extendedModulesCheck",
    "checkUseSynchronousCalls",
    "checkUseModality",
    "unsupportedFunctional",
    "configLogIntegrity",
    "incorrectReferences",
    "extension",
    "allExtensions",
    "projects",
];
const RUNTIME_TEST_OPERATION_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "testRunner",
    "testScope",
    "module",
    "fullOutput",
    "features",
    "filterTags",
    "ignoreTags",
    "scenarioFilters",
];
const RUNTIME_LAUNCH_OPERATION_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "clientMode",
    "mode",
    "mcpConfig",
    "mcpPort",
    "c",
    "execute",
    "usePrivilegedMode",
    "output",
    "stderrOutput",
    "waitForExit",
    "waitTimeoutMs",
    "rawKeys",
];
const RUNTIME_EXTENSIONS_OPERATION_ARGS: &[&str] =
    &["operation", "config", "workdir", "sourceSet", "sourceSets"];
const RUNTIME_TOOLS_DOWNLOAD_OPERATION_ARGS: &[&str] =
    &["operation", "config", "workdir", "tool", "sources", "force"];

const CODE_ARGS: &[&str] = &[
    "config",
    "format",
    "limit",
    "mode",
    "path",
    "query",
    "sourceDir",
];

const CODE_DEFINITION_ARGS: &[&str] = &["limit", "moduleHint", "name", "sourceDir"];
const CODE_OUTLINE_ARGS: &[&str] = &["includeMethods", "path", "sourceDir"];
const CODE_SEARCH_ARGS: &[&str] = &["limit", "metadataPath", "query", "sourceDir", "sourceSet"];
const CODE_GRAPH_ARGS: &[&str] = &[
    "detail",
    "dir",
    "edgeKinds",
    "id",
    "ids",
    "limit",
    "maxOutputTokens",
    "mode",
    "provenance",
    "query",
    "sourceDir",
];
const CODE_GRAPH_MODES: &[&str] = &[
    "status",
    "overview",
    "resolve",
    "node",
    "source",
    "neighbors",
    "callers",
    "callees",
];
const CODE_GRAPH_DIRECTIONS: &[&str] = &["in", "out", "both"];
const CODE_GRAPH_DETAIL: &[&str] = &["names", "signatures", "bodies"];
const CODE_DIAGNOSTIC_SEVERITIES: &[&str] = &["error", "warning", "info", "hint"];
const STANDARDS_ARGS: &[&str] = &[
    "body_limit",
    "bodyLimit",
    "codes",
    "id",
    "idOrAliasOrUrl",
    "language",
    "limit",
    "mode",
    "query",
    "snippet",
    "types",
];
const DOCUMENTATION_SEARCH_ARGS: &[&str] = &[
    "language",
    "limit",
    "platformVersion",
    "query",
    "sourceKinds",
];
const DOCUMENTATION_GET_ARGS: &[&str] = &["documentId", "language", "platformVersion"];

/// Removes JSON Schema `description` annotations from a schema tree without
/// touching members that are property names (a property literally called
/// `description` survives; only its own annotation is dropped).
///
/// #479 §1 baseline experiment (owner decision, 2026-08-17): the served tool
/// surface is schema-only while descriptions are reauthored under the
/// "минимум токенов на решение" objective; the v0.12 history keeps the
/// previous texts.
pub fn strip_schema_descriptions(value: &mut Value) {
    fn walk(value: &mut Value, keys_are_member_names: bool) {
        match value {
            Value::Object(map) => {
                if keys_are_member_names {
                    for child in map.values_mut() {
                        walk(child, false);
                    }
                } else {
                    map.remove("description");
                    for (key, child) in map.iter_mut() {
                        // Instance-value keywords carry data, not schemas;
                        // an object constant keeps its `description` member.
                        if matches!(key.as_str(), "const" | "enum" | "default" | "examples") {
                            continue;
                        }
                        let named = matches!(
                            key.as_str(),
                            "properties" | "patternProperties" | "$defs" | "definitions"
                        );
                        walk(child, named);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, false);
                }
            }
            _ => {}
        }
    }
    walk(value, false);
}

pub fn input_schema_for_tool(tool: &ToolSpec) -> Value {
    if matches!(tool.handler, ToolHandler::Diagnostics) {
        return diagnostics_input_schema();
    }
    if let ToolHandler::Metadata { operation } = tool.handler {
        return super::metadata::metadata_input_schema(operation);
    }
    let mut property_names = allowed_args(tool);
    if let ToolHandler::NativeOperation { operation, .. } = tool.handler {
        // ADR-0019: aliases remain accepted by normalize_native_path_aliases,
        // while tools/list publishes one host-portable canonical path contract.
        for group in native_path_alias_groups(operation) {
            property_names.retain(|name| {
                *name == group.canonical || !group.aliases.iter().any(|alias| alias == name)
            });
        }
    }
    // ADR-0049: still accepted for compatibility, never advertised.
    let unpublished = unpublished_bridge_args(tool.name);
    property_names.retain(|name| !unpublished.contains(name));
    let mut properties = Map::new();
    for name in property_names {
        let mut property = property_schema_for_tool(tool, name);
        // Attached here rather than inside property_schema so that the
        // tool-specific overrides above, which return their own enums and
        // patterns, are described too.
        if let Some(description) = description_for_arg(name) {
            if let Some(object) = property.as_object_mut() {
                object
                    .entry("description".to_string())
                    .or_insert_with(|| json!(description));
            }
        }
        properties.insert(name.to_string(), property);
    }

    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required_args(tool),
    });
    if let Some((legacy, address)) = bridged_selector(tool.name) {
        // ADR-0049: the two selectors are alternatives, never a precedence
        // question, so the schema says so instead of leaving a client to guess
        // which one an answer came from.
        let logical_required = address.required_args();
        let mut forbidden_in_legacy = logical_required
            .iter()
            .map(|name| json!({"required": [name]}))
            .collect::<Vec<_>>();
        if address == LogicalAddress::Optional {
            forbidden_in_legacy.push(json!({"required": ["metadataPath"]}));
        }
        schema["oneOf"] = json!([
            {
                "required": logical_required,
                "not": {"required": [legacy]}
            },
            {
                "required": [legacy],
                "not": {"anyOf": forbidden_in_legacy}
            }
        ]);
    }
    if tool.name == "unica.form.edit" {
        schema["anyOf"] = json!([
            {"required": ["JsonPath"]},
            {"required": ["definition"]}
        ]);
    }
    if tool.name == "unica.code.patch" {
        schema["oneOf"] = json!([
            {
                "properties": {"operation": {"const": "insert"}},
                "required": ["selector", "position"]
            },
            {
                "properties": {"operation": {"const": "insert"}},
                "not": {"anyOf": [
                    {"required": ["selector"]},
                    {"required": ["position"]}
                ]}
            },
            {
                "properties": {"operation": {"const": "replace"}},
                "required": ["selector"],
                "not": {"required": ["position"]}
            }
        ]);
    }
    if tool.name == "unica.code.search" {
        schema["oneOf"] = json!([
            {
                "required": ["sourceSet"],
                "not": {"required": ["sourceDir"]}
            },
            {
                "required": ["sourceDir"],
                "not": {"anyOf": [
                    {"required": ["sourceSet"]},
                    {"required": ["metadataPath"]}
                ]}
            }
        ]);
    }
    if tool.name == "unica.source.resources" {
        schema["oneOf"] = json!([
            {
                "required": ["sourceSet"],
                "not": {"anyOf": [{"required": ["snapshotId"]}, {"required": ["cursor"]}]}
            },
            {
                "required": ["snapshotId", "cursor"],
                "not": {"anyOf": [
                    {"required": ["sourceSet"]},
                    {"required": ["metadataPath"]},
                    {"required": ["scope"]}
                ]}
            }
        ]);
    }
    if tool.name == "unica.xdto.info" {
        schema["not"] = json!({
            "anyOf": [
                {"required": ["typeName", "limit"]},
                {"required": ["typeName", "cursor"]}
            ]
        });
    }
    schema
}

fn diagnostics_input_schema() -> Value {
    let provider_ids = LIVE_DIAGNOSTIC_PROVIDERS
        .iter()
        .map(|provider| provider.as_str())
        .collect::<Vec<_>>();
    let code_filter = || {
        json!({
            "type": "array",
            "uniqueItems": true,
            "items": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "provider": {"type": "string", "enum": provider_ids},
                    "code": {"type": "string", "minLength": 1, "pattern": r"\S"}
                },
                "required": ["provider", "code"]
            }
        })
    };
    let filter = |action: DiagnosticAction| {
        let mut properties = Map::new();
        properties.insert("codes".to_string(), code_filter());
        if !matches!(action, DiagnosticAction::Catalog) {
            properties.insert(
                "minSeverity".to_string(),
                json!({"type": "string", "enum": CODE_DIAGNOSTIC_SEVERITIES, "default": "warning"}),
            );
        }
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": properties
        })
    };
    let range = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "startLine": {"type": "integer", "minimum": 0},
            "startColumn": {"type": "integer", "minimum": 0},
            "endLine": {"type": "integer", "minimum": 0},
            "endColumn": {"type": "integer", "minimum": 0}
        },
        "required": ["startLine", "startColumn", "endLine", "endColumn"]
    });
    let branches = DIAGNOSTIC_ACTION_DESCRIPTORS
        .iter()
        .map(|descriptor| {
            let mut properties = Map::new();
            for name in descriptor.allowed_args {
                let schema = match *name {
                    "action" => json!({
                        "type": "string",
                        "const": descriptor.action.as_str(),
                        "description": "Closed diagnostics action selecting analyze, findings, status, or catalog behavior."
                    }),
                    "sourceSet" => json!({
                        "type": "string",
                        "minLength": 1,
                        "pattern": r"^\S(?:.*\S)?$",
                        "description": "Exact source-set name from the workspace project map; no implicit fallback is used."
                    }),
                    "metadataPath" => json!({
                        "type": "string",
                        "minLength": 1,
                        "pattern": r"^\S(?:.*\S)?$",
                        "description": "Exact logical 1C target address inside sourceSet; required only by findings."
                    }),
                    "cwd" => json!({
                        "type": "string",
                        "description": "Absolute path selecting the workspace context; it never identifies a diagnostic target and is not echoed in result data."
                    }),
                    "filter" => {
                        let mut schema = filter(descriptor.action);
                        schema["description"] = json!("Strict diagnostic severity and provider-qualified code filter applied after normalization.");
                        schema
                    },
                    "range" => {
                        let mut schema = range.clone();
                        schema["description"] = json!("Zero-based, end-exclusive source range accepted only by findings for a module target.");
                        schema
                    },
                    "limit" => {
                        json!({
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 200,
                            "default": 200,
                            "description": "Maximum number of diagnostic entities returned after filtering and deterministic ordering."
                        })
                    },
                    "timeoutSeconds" => json!({
                        "type": "integer",
                        "minimum": DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
                        "maximum": DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS,
                        "description": "Total action=analyze budget. Overrides operational.code_diagnostics.analyze_timeout_seconds from workspace config."
                    }),
                    _ => unreachable!("diagnostic descriptor contains unknown field {name}"),
                };
                properties.insert((*name).to_string(), schema);
            }
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": properties,
                "required": descriptor.required_args
            })
        })
        .collect::<Vec<_>>();
    let mut properties = Map::new();
    for branch in &branches {
        for (name, schema) in branch["properties"]
            .as_object()
            .expect("diagnostics action branch properties are an object")
        {
            properties
                .entry(name.clone())
                .or_insert_with(|| schema.clone());
        }
    }
    properties.insert(
        "action".to_string(),
        json!({
            "type": "string",
            "enum": DIAGNOSTIC_ACTION_DESCRIPTORS
                .iter()
                .map(|descriptor| descriptor.action.as_str())
                .collect::<Vec<_>>(),
            "description": "Closed diagnostics action selecting analyze, findings, status, or catalog behavior."
        }),
    );
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": [],
        "oneOf": branches
    })
}

/// ADR-0071: the closed tagged union of `unica.xdto.edit` operations, placed
/// directly in `properties.operations.items` per ADR-0025 §4 — no
/// `allOf`/`if`/`then`/`$ref`, every variant closed and naming its own
/// required fields. Field semantics are exactly what the package writer read
/// before the array form; only the shape moved.
fn xdto_edit_operations_schema() -> Value {
    let ncname = || json!({"type": "string", "minLength": 1, "pattern": xml_ncname_pattern()});
    let qname = || json!({"type": "string", "minLength": 1, "pattern": xml_qname_pattern()});
    let property_path =
        || json!({"type": "string", "minLength": 1, "pattern": xml_property_path_pattern()});
    let property = || {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": ncname(),
                "type": qname(),
                "minOccurs": {"type": "integer", "minimum": 0, "maximum": 1}
            },
            "required": ["name", "type"]
        })
    };
    json!({
        "type": "array",
        "minItems": 1,
        "items": {
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "op": {"enum": ["addValueType"]},
                        "name": ncname(),
                        "base": qname()
                    },
                    "required": ["op", "name", "base"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "op": {"enum": ["addObjectType"]},
                        "name": ncname()
                    },
                    "required": ["op", "name"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "op": {"enum": ["addProperty"]},
                        "typeName": ncname(),
                        "property": property(),
                        "propertyPath": property_path()
                    },
                    "required": ["op", "typeName", "property"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "op": {"enum": ["removeType"]},
                        "name": ncname()
                    },
                    "required": ["op", "name"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "op": {"enum": ["removeProperty"]},
                        "typeName": ncname(),
                        "name": ncname(),
                        "propertyPath": property_path()
                    },
                    "required": ["op", "typeName", "name"]
                }
            ]
        },
        "description": "Ordered closed XDTO package operations applied in one transaction: later operations see earlier results, a failed operation leaves no partial write, and each effect is reported by operationIndex."
    })
}

pub(crate) fn normalize_native_path_aliases(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let mut normalized = args.clone();
    let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
        return Ok(normalized);
    };

    for group in native_path_alias_groups(operation) {
        let present = group
            .aliases
            .iter()
            .filter_map(|alias| args.get(*alias).map(|value| (*alias, value)))
            .collect::<Vec<_>>();
        if present.is_empty() {
            continue;
        }

        let non_empty = present
            .iter()
            .copied()
            .filter(|(_, value)| !is_empty_path_alias_value(value))
            .collect::<Vec<_>>();
        if let Some((_, expected)) = non_empty.first().copied() {
            if non_empty.iter().any(|(_, value)| *value != expected) {
                return Err(format!(
                    "{} received conflicting path aliases with different non-empty values: {}",
                    tool.name,
                    non_empty
                        .iter()
                        .map(|(alias, _)| *alias)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }

        let selected = non_empty
            .first()
            .or_else(|| present.first())
            .map(|(_, value)| (*value).clone())
            .expect("present path aliases cannot be empty");
        for alias in group.aliases {
            normalized.remove(*alias);
        }
        normalized.insert(group.canonical.to_string(), selected);
    }

    Ok(normalized)
}

fn is_empty_path_alias_value(value: &Value) -> bool {
    value.as_str().is_some_and(|value| value.trim().is_empty())
}

pub fn validate_tool_argument_shape(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if let ToolHandler::Metadata { operation } = tool.handler {
        return super::metadata::validate_metadata_argument_shape(operation, args).map_err(
            |failure| {
                let detail = failure
                    .diagnostics
                    .first()
                    .map(|diagnostic| diagnostic.message.as_str())
                    .unwrap_or("metadata arguments are invalid");
                format!("{} invalid arguments: {detail}", tool.name)
            },
        );
    }
    validate_removed_target_arguments(tool, args)?;
    validate_removed_xdto_edit_arguments(tool, args)?;
    let allowed = allowed_args(&tool).into_iter().collect::<BTreeSet<_>>();
    for key in args.keys() {
        if !allowed.contains(key.as_str()) {
            let accepted = allowed.iter().copied().collect::<Vec<_>>();
            return Err(format!(
                "{} does not accept argument `{key}`;{} use typed MCP arguments only; accepted arguments: {}",
                tool.name,
                did_you_mean_clause(key, &accepted),
                accepted.join(", ")
            ));
        }
    }
    for (key, value) in args {
        validate_argument_type(tool.name, key, value)?;
    }
    Ok(())
}

pub fn validate_tool_argument_semantics(
    tool: ToolSpec,
    args: &Map<String, Value>,
    mode: InvocationMode,
) -> Result<(), String> {
    if let ToolHandler::Metadata { operation } = tool.handler {
        return super::metadata::parse_metadata_request_after_shape(operation, args)
            .map(|_| ())
            .map_err(|failure| {
                let detail = failure
                    .diagnostics
                    .first()
                    .map(|diagnostic| diagnostic.message.as_str())
                    .unwrap_or("metadata arguments are invalid");
                format!("{} invalid arguments: {detail}", tool.name)
            });
    }
    let dry_run = mode.is_preview();
    if matches!(tool.handler, ToolHandler::RuntimeAdapter) {
        validate_runtime_arguments(tool.name, args, dry_run)?;
    }
    if let ToolHandler::RuntimeJob { action } = tool.handler {
        validate_runtime_job_arguments(tool.name, action, args, dry_run)?;
    }
    validate_code_arguments(tool, args, dry_run)?;
    validate_source_navigation_arguments(tool, args)?;
    validate_source_resource_arguments(tool, args)?;
    validate_code_patch_arguments(tool, args)?;
    validate_form_add_arguments(tool, args)?;
    validate_form_edit_arguments(tool, args, dry_run)?;
    validate_template_add_arguments(tool, args)?;
    validate_support_arguments(tool, args, dry_run)?;
    validate_external_init_arguments(tool, args)?;
    validate_cfe_patch_method_arguments(tool, args)?;
    validate_xdto_arguments(tool, args)?;
    validate_role_edit_arguments(tool, args)?;
    validate_bridged_selector(tool, args)?;

    if !dry_run || is_external_init_tool(tool) {
        for required in required_args(&tool) {
            if !args.contains_key(required) {
                return Err(format!("{} requires `{required}` argument", tool.name));
            }
        }
    }

    Ok(())
}

fn validate_role_edit_arguments(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    if tool.name != "unica.role.edit" {
        return Ok(());
    }
    parse_role_edit_request(args)
        .map(|_| ())
        .map_err(|error| format!("{} invalid arguments: {error}", tool.name))
}

#[cfg(test)]
fn validate_tool_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
) -> Result<(), String> {
    validate_tool_argument_shape(tool, args)?;
    let mode = match (tool.execution, dry_run) {
        (ToolExecution::Read, _) => InvocationMode::Read,
        (ToolExecution::Mutation, true) => InvocationMode::Preview,
        (ToolExecution::Mutation, false) => InvocationMode::Apply,
    };
    validate_tool_argument_semantics(tool, args, mode)
}

fn validate_xdto_arguments(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    if !matches!(tool.name, "unica.xdto.info" | "unica.xdto.edit") {
        return Ok(());
    }
    let source_set = xdto_required_string(tool.name, args, "sourceSet")?;
    if source_set != source_set.trim() {
        return Err(format!(
            "{} argument `sourceSet` must not have surrounding whitespace",
            tool.name
        ));
    }
    let metadata_path = xdto_required_string(tool.name, args, "metadataPath")?;
    let package_name = metadata_path
        .strip_prefix("XDTOPackage.")
        .or_else(|| metadata_path.strip_prefix("ПакетXDTO."));
    if package_name.is_none_or(|name| !is_xml_ncname(name) || name.contains('.')) {
        return Err(format!(
            "{} argument `metadataPath` must be XDTOPackage.<NCName>",
            tool.name
        ));
    }

    if tool.name == "unica.xdto.info" {
        if let Some(type_name) = xdto_optional_string(tool.name, args, "typeName")? {
            if !is_xml_ncname(type_name) {
                return Err(format!(
                    "{} argument `typeName` must be an XML NCName",
                    tool.name
                ));
            }
            if args.contains_key("limit") || args.contains_key("cursor") {
                return Err(format!(
                    "{} `typeName` detail does not accept `limit` or `cursor`",
                    tool.name
                ));
            }
        }
        validate_integer_bound(
            tool.name,
            args,
            "limit",
            1,
            SOURCE_NAVIGATION_LIMIT_MAX as u64,
        )?;
        if args.get("cursor").is_some_and(|cursor| {
            cursor
                .as_str()
                .is_none_or(|value| value.is_empty() || value.chars().any(char::is_whitespace))
        }) {
            return Err(format!(
                "{} argument `cursor` must be a non-empty string without whitespace",
                tool.name
            ));
        }
        return Ok(());
    }

    let operations = args
        .get("operations")
        .ok_or_else(|| format!("{} requires `operations` argument", tool.name))?
        .as_array()
        .ok_or_else(|| format!("{} argument `operations` must be an array", tool.name))?;
    if operations.is_empty() {
        return Err(format!(
            "{} argument `operations` must not be empty",
            tool.name
        ));
    }
    for (index, item) in operations.iter().enumerate() {
        validate_xdto_operation_item(tool.name, index, item)?;
    }
    Ok(())
}

/// One element of the `unica.xdto.edit` operations union (ADR-0071). Every
/// message names the failing element as `operations[<index>]` so a rejected
/// batch points at the exact operation.
fn validate_xdto_operation_item(tool_name: &str, index: usize, item: &Value) -> Result<(), String> {
    let element = |message: &str| format!("{tool_name} operations[{index}]: {message}");
    let item = item
        .as_object()
        .ok_or_else(|| element("must be an object"))?;
    let op = item
        .get("op")
        .and_then(Value::as_str)
        .filter(|value| XDTO_EDIT_OPS.contains(value))
        .ok_or_else(|| {
            element(&format!(
                "`op` must be one of: {}",
                XDTO_EDIT_OPS.join(", ")
            ))
        })?;
    let (required, optional): (&[&str], &[&str]) = match op {
        "addValueType" => (&["name", "base"], &[]),
        "addObjectType" | "removeType" => (&["name"], &[]),
        "addProperty" => (&["typeName", "property"], &["propertyPath"]),
        "removeProperty" => (&["typeName", "name"], &["propertyPath"]),
        _ => unreachable!("op was checked against the closed set"),
    };
    for field in required {
        if !item.contains_key(*field) {
            return Err(element(&format!("op `{op}` requires `{field}`")));
        }
    }
    for key in item.keys() {
        if key != "op" && !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(element(&format!("op `{op}` does not accept `{key}`")));
        }
    }
    for field in ["name", "typeName"] {
        if let Some(value) = item.get(field) {
            value
                .as_str()
                .filter(|value| is_xml_ncname(value))
                .ok_or_else(|| element(&format!("`{field}` must be an XML NCName")))?;
        }
    }
    if let Some(base) = item.get("base") {
        base.as_str()
            .filter(|value| is_xml_prefixed_qname(value))
            .ok_or_else(|| {
                element("`base` must be a prefixed XML QName without surrounding whitespace")
            })?;
    }
    if let Some(path) = item.get("propertyPath") {
        path.as_str()
            .filter(|value| is_xdto_property_path(value))
            .ok_or_else(|| {
                element(
                    "`propertyPath` must contain dot-separated XML NCNames with literal dots escaped as `\\.`",
                )
            })?;
    }
    if let Some(property) = item.get("property") {
        let property = property
            .as_object()
            .ok_or_else(|| element("`property` must be an object"))?;
        if property
            .keys()
            .any(|key| !matches!(key.as_str(), "name" | "type" | "minOccurs"))
        {
            return Err(element("`property` accepts only name, type, minOccurs"));
        }
        property
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| is_xml_ncname(value))
            .ok_or_else(|| element("`property.name` must be an XML NCName"))?;
        property
            .get("type")
            .and_then(Value::as_str)
            .filter(|value| is_xml_prefixed_qname(value))
            .ok_or_else(|| {
                element(
                    "`property.type` must be a prefixed XML QName without surrounding whitespace",
                )
            })?;
        if property
            .get("minOccurs")
            .is_some_and(|value| !matches!(value.as_u64(), Some(0 | 1)))
        {
            return Err(element("`property.minOccurs` must be 0 or 1"));
        }
    }
    Ok(())
}

fn xdto_required_string<'a>(
    tool_name: &str,
    args: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, String> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{tool_name} requires non-empty `{field}` argument"))
}

fn xdto_optional_string<'a>(
    tool_name: &str,
    args: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, String> {
    args.get(field)
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("{tool_name} argument `{field}` must be a non-empty string"))
        })
        .transpose()
}

fn is_xdto_property_path(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let mut segment = String::new();
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                if characters.next() != Some('.') {
                    return false;
                }
                segment.push('.');
            }
            '.' => {
                if !is_xml_ncname(&segment) {
                    return false;
                }
                segment.clear();
            }
            _ => segment.push(character),
        }
    }
    is_xml_ncname(&segment)
}

fn is_xml_prefixed_qname(value: &str) -> bool {
    let mut parts = value.split(':');
    let prefix = parts.next().unwrap_or_default();
    let local = parts.next().unwrap_or_default();
    parts.next().is_none() && is_xml_ncname(prefix) && is_xml_ncname(local)
}

fn is_xml_ncname(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    is_xml_ncname_start(first) && characters.all(is_xml_ncname_char)
}

// XML 1.0 Fifth Edition NCName ranges. The BMP grammar is shared by runtime
// validation and the published JSON-Schema patterns. Astral ranges remain a
// runtime-only addition because an ECMAScript pattern without Unicode mode
// cannot portably represent them as single code points.
const XML_NCNAME_START_BMP_RANGES: &[(char, char)] = &[
    ('A', 'Z'),
    ('_', '_'),
    ('a', 'z'),
    ('\u{00c0}', '\u{00d6}'),
    ('\u{00d8}', '\u{00f6}'),
    ('\u{00f8}', '\u{02ff}'),
    ('\u{0370}', '\u{037d}'),
    ('\u{037f}', '\u{1fff}'),
    ('\u{200c}', '\u{200d}'),
    ('\u{2070}', '\u{218f}'),
    ('\u{2c00}', '\u{2fef}'),
    ('\u{3001}', '\u{d7ff}'),
    ('\u{f900}', '\u{fdcf}'),
    ('\u{fdf0}', '\u{fffd}'),
];
const XML_NCNAME_START_ASTRAL_RANGES: &[(char, char)] = &[('\u{10000}', '\u{effff}')];
const XML_NCNAME_CONTINUATION_RANGES: &[(char, char)] = &[
    ('-', '-'),
    ('.', '.'),
    ('0', '9'),
    ('\u{00b7}', '\u{00b7}'),
    ('\u{0300}', '\u{036f}'),
    ('\u{203f}', '\u{2040}'),
];

fn xml_character_is_in_ranges(character: char, ranges: &[(char, char)]) -> bool {
    ranges
        .iter()
        .any(|&(start, end)| start <= character && character <= end)
}

fn is_xml_ncname_start(character: char) -> bool {
    xml_character_is_in_ranges(character, XML_NCNAME_START_BMP_RANGES)
        || xml_character_is_in_ranges(character, XML_NCNAME_START_ASTRAL_RANGES)
}

fn is_xml_ncname_char(character: char) -> bool {
    is_xml_ncname_start(character)
        || xml_character_is_in_ranges(character, XML_NCNAME_CONTINUATION_RANGES)
}

fn validate_source_resource_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    let ToolHandler::SourceResources { operation } = tool.handler else {
        return Ok(());
    };
    match operation {
        SourceResourceOperation::Resources => {
            validate_integer_bound(
                tool.name,
                args,
                "limit",
                1,
                SOURCE_RESOURCE_PAGE_LIMIT_MAX as u64,
            )?;
            if let Some(value) = args.get("scope") {
                let scope = value
                    .as_str()
                    .ok_or_else(|| format!("{} argument `scope` must be a string", tool.name))?;
                if !matches!(scope, "self" | "aggregate" | "registrations") {
                    return Err(format!(
                        "{} argument `scope` must be `self`, `aggregate`, or `registrations`",
                        tool.name
                    ));
                }
            }
        }
        SourceResourceOperation::Read => {
            validate_integer_bound(tool.name, args, "limit", 1, SOURCE_READ_LIMIT_MAX as u64)?;
        }
    }
    Ok(())
}

fn validate_source_navigation_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    let ToolHandler::SourceNavigation { operation } = tool.handler else {
        return Ok(());
    };
    for required in match operation {
        SourceNavigationOperation::Resolve => &["sourceSet", "query"][..],
        SourceNavigationOperation::Children => &["sourceSet"][..],
        SourceNavigationOperation::Locate => &["sourceSet", "path"][..],
    } {
        let value = args
            .get(*required)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{} requires `{required}` argument", tool.name))?;
        debug_assert!(!value.is_empty());
    }
    if let Some(value) = args.get("mode") {
        let mode = value
            .as_str()
            .ok_or_else(|| format!("{} argument `mode` must be a string", tool.name))?;
        if !matches!(mode, "exact" | "prefix") {
            return Err(format!(
                "{} argument `mode` must be `exact` or `prefix`",
                tool.name
            ));
        }
    }
    if let Some(value) = args.get("targetKind") {
        let target_kind = value
            .as_str()
            .ok_or_else(|| format!("{} argument `targetKind` must be a string", tool.name))?;
        if !matches!(target_kind, "metadataObject" | "module") {
            return Err(format!(
                "{} argument `targetKind` must be `metadataObject` or `module`",
                tool.name
            ));
        }
    }
    if let Some(value) = args.get("limit") {
        let limit = value
            .as_u64()
            .ok_or_else(|| format!("{} argument `limit` must be a positive integer", tool.name))?;
        if !(1..=u64::try_from(SOURCE_NAVIGATION_LIMIT_MAX).expect("small constant"))
            .contains(&limit)
        {
            return Err(format!(
                "{} argument `limit` must be between 1 and {SOURCE_NAVIGATION_LIMIT_MAX}",
                tool.name
            ));
        }
    }
    for optional in ["metadataPath", "cursor"] {
        if let Some(value) = args.get(optional) {
            let non_empty = value
                .as_str()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            if !non_empty {
                return Err(format!(
                    "{} argument `{optional}` must be a non-empty string",
                    tool.name
                ));
            }
        }
    }
    Ok(())
}

fn validate_removed_target_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if tool.name == "unica.code.patch"
        && ["path", "sourceDir"]
            .iter()
            .any(|field| args.contains_key(*field))
    {
        return Err(
            "legacy_target_removed: unica.code.patch no longer accepts `path` or `sourceDir`; use `sourceSet + metadataPath`"
                .to_string(),
        );
    }
    if tool.name == "unica.code.diagnostics"
        && ["mode", "sourceDir", "path"]
            .iter()
            .any(|field| args.contains_key(*field))
    {
        return Err(
            "legacy_target_removed: unica.code.diagnostics no longer accepts `mode`, `sourceDir`, or `path`; use `action + sourceSet` and `metadataPath` for findings"
                .to_string(),
        );
    }
    Ok(())
}

/// ADR-0071: the flat single-operation form of `unica.xdto.edit` is retired.
/// The check precedes unknown-argument validation so the caller of the old
/// contract gets the stable migration code, not a spelling suggestion.
fn validate_removed_xdto_edit_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if tool.name != "unica.xdto.edit" {
        return Ok(());
    }
    if XDTO_EDIT_LEGACY_ARGS
        .iter()
        .any(|field| args.contains_key(*field))
    {
        return Err(
            "legacy_arguments_removed: unica.xdto.edit no longer accepts `operation`, `name`, `base`, `typeName`, `propertyPath`, or `property` at the top level; pass one element of the typed `operations` array instead"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_code_patch_arguments(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    if tool.name != "unica.code.patch" {
        return Ok(());
    }
    for key in ["sourceSet", "metadataPath", "operation", "content"] {
        let value = args
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} argument `{key}` must be a non-empty string", tool.name))?;
        if value.trim().is_empty() {
            return Err(format!(
                "{} argument `{key}` must be a non-empty string",
                tool.name
            ));
        }
    }
    let operation = args
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(operation, "insert" | "replace") {
        return Err(format!(
            "{} supports operation `insert` or `replace`",
            tool.name
        ));
    }
    // `position` places content relative to a selector. A replacement overwrites
    // the selected span and has nowhere to place anything; a selector-less
    // insertion goes to the end of the module, which is not a relative place
    // either. Both refuse `position` rather than ignoring it.
    let has_selector = args.contains_key("selector");
    if operation == "insert" && has_selector {
        if !matches!(
            args.get("position").and_then(Value::as_str),
            Some("before" | "after")
        ) {
            return Err(format!(
                "{} argument `position` must be `before` or `after` when `insert` names a selector",
                tool.name
            ));
        }
    } else if args.contains_key("position") {
        return Err(format!(
            "{} does not accept `position` here; only `insert` with a `selector` places content relative to it",
            tool.name
        ));
    }
    // A selector-less insertion has nothing left to validate: the end of the
    // module is not addressed, it is implied.
    if operation == "insert" && !has_selector {
        return Ok(());
    }
    let selector = args
        .get("selector")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{} argument `selector` must be an object", tool.name))?;
    if selector.len() != 1
        || !selector
            .keys()
            .all(|key| matches!(key.as_str(), "method" | "anchor"))
    {
        return Err(format!(
            "{} selector must contain exactly one of `method` or `anchor`",
            tool.name
        ));
    }
    let value = selector
        .values()
        .next()
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    if value.is_none() {
        return Err(format!(
            "{} selector value must be a non-empty string",
            tool.name
        ));
    }
    Ok(())
}

fn validate_cfe_patch_method_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if tool.name != "unica.cfe.patch_method" {
        return Ok(());
    }
    for aliases in [
        &["MethodName", "methodName"][..],
        &["Context", "context"][..],
        &["InterceptorType", "interceptorType"][..],
        &["IsFunction", "isFunction"][..],
    ] {
        validate_unique_alias_group(tool.name, args, aliases)?;
    }
    for name in ["MethodName", "methodName"] {
        let Some(value) = args.get(name) else {
            continue;
        };
        let value = value
            .as_str()
            .ok_or_else(|| format!("{} argument `{name}` must be string", tool.name))?;
        if !is_cfe_patch_method_identifier(value) {
            return Err(format!(
                "{} argument `MethodName` must be a valid 1C identifier",
                tool.name
            ));
        }
    }
    for name in ["Context", "context"] {
        let Some(value) = args.get(name) else {
            continue;
        };
        let value = value
            .as_str()
            .ok_or_else(|| format!("{} argument `{name}` must be string", tool.name))?;
        if !CFE_PATCH_METHOD_CONTEXTS.contains(&value) {
            return Err(format!(
                "{} argument `Context` must be one of: {}",
                tool.name,
                CFE_PATCH_METHOD_CONTEXTS.join(", ")
            ));
        }
    }
    for name in ["InterceptorType", "interceptorType"] {
        let Some(value) = args.get(name) else {
            continue;
        };
        let value = value
            .as_str()
            .ok_or_else(|| format!("{} argument `{name}` must be string", tool.name))?;
        if !CFE_PATCH_METHOD_INTERCEPTOR_TYPES.contains(&value) {
            return Err(format!(
                "{} argument `InterceptorType` must be one of: {}",
                tool.name,
                CFE_PATCH_METHOD_INTERCEPTOR_TYPES.join(", ")
            ));
        }
    }
    for name in ["IsFunction", "isFunction"] {
        let Some(value) = args.get(name) else {
            continue;
        };
        let value = value
            .as_bool()
            .ok_or_else(|| format!("{} argument `{name}` must be boolean", tool.name))?;
        if value {
            return Err(format!(
                "{} v1 requires a parameterless procedure; a base method signature resolver for functions and parameterized methods is not implemented",
                tool.name
            ));
        }
    }
    Ok(())
}

fn is_cfe_patch_method_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let valid_start = |ch: char| {
        ch == '_'
            || ch.is_ascii_alphabetic()
            || ('А'..='я').contains(&ch)
            || matches!(ch, 'Ё' | 'ё')
    };
    valid_start(first) && chars.all(|ch| valid_start(ch) || ch.is_ascii_digit())
}

fn validate_external_init_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if !is_external_init_tool(tool) {
        return Ok(());
    }
    for key in ["Name", "Synonym", "OutputDir", "FormName"] {
        let Some(value) = args.get(key) else {
            continue;
        };
        let Some(value) = value.as_str() else {
            return Err(format!("{} argument `{key}` must be string", tool.name));
        };
        if value.trim().is_empty() {
            return Err(format!(
                "{} argument `{key}` must be a non-empty string",
                tool.name
            ));
        }
    }
    Ok(())
}

fn validate_form_add_arguments(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    if tool.name != "unica.form.add" {
        return Ok(());
    }
    validate_unique_alias_group(tool.name, args, &["SetDefault", "setDefault"])
}

/// ADR-0049: a bridged reader accepts exactly one selector. Two at once is a
/// caller mistake, not a precedence question — resolving it silently would hide
/// which selector produced the answer. Zero is the pre-existing missing-argument
/// error, restated so it names both ways in.
fn validate_bridged_selector(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    let Some((legacy, address)) = bridged_selector(tool.name) else {
        return Ok(());
    };
    let logical = address.required_args();
    let has_logical = contains_any(args, logical) || args.contains_key("metadataPath");
    let has_legacy = args.contains_key(legacy);
    if has_logical && has_legacy {
        return Err(format!(
            "selector_conflict: {} accepts either `{}` or `{legacy}`, not both",
            tool.name,
            logical.join("` + `"),
        ));
    }
    if !has_logical && !has_legacy {
        return Err(format!(
            "{} requires either `{}` or `{legacy}` argument",
            tool.name,
            logical.join("` + `"),
        ));
    }
    if has_logical {
        for required in logical {
            if !args.contains_key(*required) {
                return Err(format!("{} requires `{required}` argument", tool.name));
            }
        }
    }
    Ok(())
}

fn validate_form_edit_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
) -> Result<(), String> {
    if tool.name != "unica.form.edit" {
        return Ok(());
    }

    validate_unique_alias_group(tool.name, args, &["FormPath", "formPath", "Path", "path"])?;
    validate_unique_alias_group(tool.name, args, &["JsonPath", "jsonPath", "definition"])?;

    let has_target = contains_any(args, &["FormPath", "formPath", "Path", "path"]);
    let has_payload = contains_any(args, &["JsonPath", "jsonPath", "definition"]);
    if !dry_run || has_target || has_payload {
        if !has_target {
            return Err(format!("{} requires `FormPath` argument", tool.name));
        }
        if !has_payload {
            return Err(format!(
                "{} requires exactly one of `JsonPath` or `definition`",
                tool.name
            ));
        }
    }

    if let Some(definition) = args.get("definition") {
        validate_form_edit_definition(definition)?;
    }

    Ok(())
}

fn validate_template_add_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if tool.name != "unica.template.add" {
        return Ok(());
    }
    validate_unique_alias_group(tool.name, args, &["SetMainSKD", "setMainSKD"])
}

fn validate_support_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
) -> Result<(), String> {
    if tool.name != "unica.support.edit" {
        return Ok(());
    }

    validate_unique_alias_group(tool.name, args, &["Capability", "capability"])?;
    validate_unique_alias_group(tool.name, args, &["Set", "set"])?;
    validate_unique_alias_group(
        tool.name,
        args,
        &["Path", "path", "TargetPath", "targetPath"],
    )?;
    validate_enum_alias_argument(
        tool.name,
        args,
        &["Capability", "capability"],
        &["on", "off"],
    )?;
    validate_enum_alias_argument(
        tool.name,
        args,
        &["Set", "set"],
        &["editable", "off-support", "locked"],
    )?;

    if dry_run {
        return Ok(());
    }

    if !contains_any(args, &["Path", "path", "TargetPath", "targetPath"]) {
        return Err(format!("{} requires `Path` argument", tool.name));
    }
    let has_capability = contains_any(args, &["Capability", "capability"]);
    let has_set = contains_any(args, &["Set", "set"]);
    if has_capability == has_set {
        return Err(format!(
            "{} requires exactly one of `Capability` or `Set`",
            tool.name
        ));
    }

    Ok(())
}

fn contains_any(args: &Map<String, Value>, names: &[&str]) -> bool {
    names.iter().any(|name| args.contains_key(*name))
}

fn validate_unique_alias_group(
    tool_name: &str,
    args: &Map<String, Value>,
    names: &[&str],
) -> Result<(), String> {
    let present = names
        .iter()
        .copied()
        .filter(|name| args.contains_key(*name))
        .collect::<Vec<_>>();
    if present.len() > 1 {
        return Err(format!(
            "{tool_name} received conflicting aliases: {}",
            present.join(", ")
        ));
    }
    Ok(())
}

fn validate_enum_alias_argument(
    tool_name: &'static str,
    args: &Map<String, Value>,
    names: &[&str],
    allowed: &[&str],
) -> Result<(), String> {
    for name in names {
        if let Some(value) = args.get(*name) {
            let Some(value) = value.as_str() else {
                return Err(format!("{tool_name} argument `{name}` must be string"));
            };
            if !allowed.contains(&value) {
                return Err(format!(
                    "{tool_name} argument `{name}` must be one of: {}",
                    allowed.join(", ")
                ));
            }
        }
    }
    Ok(())
}

fn validate_code_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
    _dry_run: bool,
) -> Result<(), String> {
    match tool.name {
        "unica.code.search" => {
            if args
                .get("query")
                .and_then(Value::as_str)
                .is_some_and(|query| query.trim().is_empty())
            {
                return Err(format!(
                    "{} argument `query` must be a non-empty string",
                    tool.name
                ));
            }
            validate_integer_bound(tool.name, args, "limit", 1, 50)?;
            let source_set = args.get("sourceSet").and_then(Value::as_str);
            let source_dir = args.get("sourceDir").and_then(Value::as_str);
            if source_set.is_some() == source_dir.is_some() {
                return Err(format!(
                    "{} requires exactly one of `sourceSet` or migration `sourceDir`",
                    tool.name
                ));
            }
            if args.contains_key("metadataPath") && source_set.is_none() {
                return Err(format!(
                    "{} argument `metadataPath` requires `sourceSet`",
                    tool.name
                ));
            }
            for name in ["sourceSet", "sourceDir", "metadataPath"] {
                if args
                    .get(name)
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.trim().is_empty())
                {
                    return Err(format!(
                        "{} argument `{name}` must be a non-empty string",
                        tool.name
                    ));
                }
            }
        }
        "unica.code.graph" => {
            validate_enum_argument(tool.name, args, "mode", CODE_GRAPH_MODES)?;
            validate_enum_argument(tool.name, args, "dir", CODE_GRAPH_DIRECTIONS)?;
            validate_enum_argument(tool.name, args, "detail", CODE_GRAPH_DETAIL)?;
        }
        "unica.code.diagnostics" => {
            validate_diagnostics_arguments(tool.name, args)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_diagnostics_arguments(
    tool_name: &str,
    args: &Map<String, Value>,
) -> Result<(), String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .and_then(DiagnosticAction::parse)
        .ok_or_else(|| {
            format!(
                "{tool_name} argument `action` must be one of: analyze, findings, status, catalog"
            )
        })?;
    for required in diagnostic_action_descriptor(action).required_args {
        let value = args.get(*required).ok_or_else(|| {
            format!(
                "{tool_name} action `{}` requires `{required}` argument",
                action.as_str()
            )
        })?;
        if matches!(*required, "action" | "sourceSet" | "metadataPath") {
            let value = value.as_str().ok_or_else(|| {
                format!("{tool_name} argument `{required}` must be a non-empty string")
            })?;
            if value.trim().is_empty() {
                return Err(format!(
                    "{tool_name} argument `{required}` must be a non-empty string"
                ));
            }
            if value != value.trim() {
                return Err(format!(
                    "{tool_name} argument `{required}` must not have surrounding whitespace"
                ));
            }
        }
    }
    let allowed = diagnostic_action_descriptor(action)
        .allowed_args
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for field in args.keys() {
        if !allowed.contains(field.as_str()) {
            return Err(format!(
                "{tool_name} action `{}` does not accept `{field}` argument",
                action.as_str()
            ));
        }
    }
    validate_integer_bound(tool_name, args, "limit", 1, 200)?;
    if args.contains_key("timeoutSeconds") {
        validate_integer_bound(
            tool_name,
            args,
            "timeoutSeconds",
            DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
            DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS,
        )?;
    }
    validate_diagnostic_filter(tool_name, action, args.get("filter"))?;
    if let Some(range) = args.get("range") {
        validate_diagnostic_range(tool_name, range)?;
    }
    Ok(())
}

fn validate_diagnostic_filter(
    tool_name: &str,
    action: DiagnosticAction,
    filter: Option<&Value>,
) -> Result<(), String> {
    let Some(filter) = filter else {
        return Ok(());
    };
    let filter = filter
        .as_object()
        .ok_or_else(|| format!("{tool_name} argument `filter` must be object"))?;
    for field in filter.keys() {
        if field != "codes" && !(field == "minSeverity" && action != DiagnosticAction::Catalog) {
            return Err(format!(
                "{tool_name} argument `filter` does not accept `{field}`"
            ));
        }
    }
    if let Some(severity) = filter.get("minSeverity") {
        let severity = severity
            .as_str()
            .ok_or_else(|| format!("{tool_name} argument `filter.minSeverity` must be string"))?;
        if !CODE_DIAGNOSTIC_SEVERITIES.contains(&severity) {
            return Err(format!(
                "{tool_name} argument `filter.minSeverity` must be one of: {}",
                CODE_DIAGNOSTIC_SEVERITIES.join(", ")
            ));
        }
    }
    let Some(codes) = filter.get("codes") else {
        return Ok(());
    };
    let codes = codes
        .as_array()
        .ok_or_else(|| format!("{tool_name} argument `filter.codes` must be array"))?;
    let mut seen = BTreeSet::new();
    for entry in codes {
        let entry = entry
            .as_object()
            .ok_or_else(|| format!("{tool_name} argument `filter.codes` must contain objects"))?;
        if entry.len() != 2 || !entry.contains_key("provider") || !entry.contains_key("code") {
            return Err(format!(
                "{tool_name} argument `filter.codes` entries require only `provider` and `code`"
            ));
        }
        let provider = entry
            .get("provider")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("{tool_name} argument `filter.codes.provider` must be string")
            })?;
        let code = entry
            .get("code")
            .and_then(Value::as_str)
            .filter(|code| !code.trim().is_empty())
            .ok_or_else(|| {
                format!("{tool_name} argument `filter.codes.code` must be a non-empty string")
            })?;
        if !LIVE_DIAGNOSTIC_PROVIDERS
            .iter()
            .any(|candidate| candidate.as_str() == provider)
        {
            return Err(format!(
                "{tool_name} unknown diagnostic provider `{provider}`"
            ));
        }
        if !seen.insert((provider, code)) {
            return Err(format!(
                "{tool_name} argument `filter.codes` must contain unique provider/code pairs"
            ));
        }
    }
    Ok(())
}

fn validate_diagnostic_range(tool_name: &str, range: &Value) -> Result<(), String> {
    let range = range
        .as_object()
        .ok_or_else(|| format!("{tool_name} argument `range` must be object"))?;
    const FIELDS: [&str; 4] = ["startLine", "startColumn", "endLine", "endColumn"];
    if range.len() != FIELDS.len() || FIELDS.iter().any(|field| !range.contains_key(*field)) {
        return Err(format!(
            "{tool_name} argument `range` requires only startLine, startColumn, endLine, endColumn"
        ));
    }
    let values = FIELDS
        .map(|field| {
            range.get(field).and_then(Value::as_u64).ok_or_else(|| {
                format!("{tool_name} argument `range.{field}` must be a non-negative integer")
            })
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let start = (values[0], values[1]);
    let end = (values[2], values[3]);
    if start >= end {
        return Err(format!(
            "{tool_name} argument `range` must be ordered and non-empty"
        ));
    }
    Ok(())
}

fn validate_enum_argument(
    tool_name: &str,
    args: &Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<(), String> {
    let Some(value) = args.get(key) else {
        return Ok(());
    };
    let Some(value) = value.as_str() else {
        return Err(format!("{tool_name} argument `{key}` must be string"));
    };
    if !allowed.contains(&value) {
        return Err(format!(
            "{tool_name} argument `{key}` must be one of: {}",
            allowed.join(", ")
        ));
    }
    Ok(())
}

fn validate_runtime_arguments(
    tool_name: &str,
    args: &Map<String, Value>,
    dry_run: bool,
) -> Result<(), String> {
    let operation = match args.get("operation") {
        Some(Value::String(operation)) => operation.as_str(),
        Some(_) => return Err(format!("{tool_name} argument `operation` must be string")),
        None => return Err(format!("{tool_name} requires `operation` argument")),
    };
    for key in RUNTIME_STRING_ARGS {
        if let Some(value) = args.get(*key) {
            if !value.is_string() {
                return Err(format!("{tool_name} argument `{key}` must be string"));
            }
        }
    }
    for key in RUNTIME_ARRAY_ARGS {
        validate_string_array_argument(tool_name, args, key)?;
    }
    if !RUNTIME_OPERATIONS.contains(&operation) {
        return Err(format!(
            "{tool_name} argument `operation` must be one of: {}",
            RUNTIME_OPERATIONS.join(", ")
        ));
    }
    validate_runtime_operation_payload(tool_name, operation, args)?;

    if dry_run {
        return Ok(());
    }

    let required = match operation {
        "load" => &["path"][..],
        "make" => &["output"][..],
        "syntax" => &["mode"][..],
        "test" => &["testRunner"][..],
        "launch" => &["clientMode"][..],
        "tools-download" => &["tool"][..],
        _ => &[][..],
    };
    for key in required {
        if !args.contains_key(*key) {
            return Err(format!(
                "{tool_name} operation `{operation}` requires `{key}` argument"
            ));
        }
    }

    Ok(())
}

fn validate_runtime_job_arguments(
    tool_name: &str,
    action: RuntimeJobAction,
    args: &Map<String, Value>,
    dry_run: bool,
) -> Result<(), String> {
    if action == RuntimeJobAction::Start {
        return validate_runtime_arguments(tool_name, args, dry_run);
    }
    if action == RuntimeJobAction::List {
        return Ok(());
    }
    let Some(job_id) = args.get("jobId") else {
        return Err(format!("{tool_name} requires `jobId` argument"));
    };
    let Some(job_id) = job_id.as_str() else {
        return Err(format!("{tool_name} argument `jobId` must be string"));
    };
    Uuid::parse_str(job_id).map_err(|_| format!("{tool_name} argument `jobId` must be a UUID"))?;

    if action == RuntimeJobAction::Wait {
        validate_integer_bound(tool_name, args, "timeoutSeconds", 1, 60)?;
    }
    if action == RuntimeJobAction::Logs {
        validate_integer_bound(tool_name, args, "tailChars", 1, 32_768)?;
    }
    Ok(())
}

fn validate_integer_bound(
    tool_name: &str,
    args: &Map<String, Value>,
    key: &str,
    minimum: u64,
    maximum: u64,
) -> Result<(), String> {
    let Some(value) = args.get(key) else {
        return Ok(());
    };
    let Some(value) = value.as_u64() else {
        return Err(format!("{tool_name} argument `{key}` must be integer"));
    };
    if !(minimum..=maximum).contains(&value) {
        return Err(format!(
            "{tool_name} argument `{key}` must be between {minimum} and {maximum}"
        ));
    }
    Ok(())
}

fn validate_string_array_argument(
    tool_name: &str,
    args: &Map<String, Value>,
    key: &str,
) -> Result<(), String> {
    let Some(value) = args.get(key) else {
        return Ok(());
    };
    let Some(items) = value.as_array() else {
        return Err(format!("{tool_name} argument `{key}` must be array"));
    };
    for item in items {
        if !item.is_string() {
            return Err(format!("{tool_name} argument `{key}` must contain strings"));
        }
    }
    Ok(())
}

fn validate_runtime_operation_payload(
    tool_name: &str,
    operation: &str,
    args: &Map<String, Value>,
) -> Result<(), String> {
    let allowed = runtime_operation_args(operation);
    for key in args.keys() {
        if COMMON_ARGS.contains(&key.as_str()) || MUTATION_ARGS.contains(&key.as_str()) {
            continue;
        }
        if !allowed.contains(&key.as_str()) {
            let mut accepted = allowed.to_vec();
            accepted.extend_from_slice(COMMON_ARGS);
            accepted.extend_from_slice(MUTATION_ARGS);
            accepted.sort_unstable();
            accepted.dedup();
            return Err(format!(
                "{tool_name} operation `{operation}` does not accept `{key}`;{} accepted arguments: {}",
                did_you_mean_clause(key, &accepted),
                accepted.join(", ")
            ));
        }
    }

    match operation {
        "dump" => {
            validate_enum_argument(tool_name, args, "mode", RUNTIME_DUMP_MODES)?;
            if args
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode == "partial")
                && !args.contains_key("object")
                && !has_non_empty_array_arg(args, "objects")
            {
                return Err(format!(
                    "{tool_name} operation `dump` with mode `partial` requires `object` or `objects`"
                ));
            }
        }
        "load" => {
            if args
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode == "update")
            {
                return Err(format!(
                    "{tool_name} load --mode update is not supported; use `load` or `merge`"
                ));
            }
            validate_enum_argument(tool_name, args, "mode", RUNTIME_LOAD_MODES)?;
            if args
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode == "merge")
                && !args.contains_key("settings")
            {
                return Err(format!(
                    "{tool_name} operation `load` with mode `merge` requires `settings`"
                ));
            }
            if args.contains_key("settings")
                && args.get("mode").and_then(Value::as_str) != Some("merge")
            {
                return Err(format!(
                    "{tool_name} operation `load` accepts `settings` only with mode `merge`"
                ));
            }
        }
        "syntax" => {
            validate_enum_argument(tool_name, args, "mode", RUNTIME_SYNTAX_MODES)?;
            let mode = args.get("mode").and_then(Value::as_str);
            if mode == Some("edt") && contains_any(args, &["extension", "allExtensions"]) {
                return Err(format!(
                    "{tool_name} operation `syntax` mode `edt` does not accept extension flags"
                ));
            }
            if matches!(mode, Some("designer-config" | "designer-modules"))
                && args.contains_key("projects")
            {
                return Err(format!(
                    "{tool_name} operation `syntax` accepts `projects` only with mode `edt`"
                ));
            }
        }
        "test" => {
            validate_enum_argument(tool_name, args, "testRunner", RUNTIME_TEST_RUNNERS)?;
            validate_enum_argument(tool_name, args, "testScope", RUNTIME_TEST_SCOPES)?;
            match args.get("testRunner").and_then(Value::as_str) {
                Some("yaxunit") => {
                    if !args.contains_key("testScope") {
                        return Err(format!(
                            "{tool_name} operation `test` with runner `yaxunit` requires `testScope`"
                        ));
                    }
                    if args
                        .get("testScope")
                        .and_then(Value::as_str)
                        .is_some_and(|scope| scope == "module")
                        && !args.contains_key("module")
                    {
                        return Err(format!(
                            "{tool_name} operation `test` with scope `module` requires `module`"
                        ));
                    }
                }
                Some("va") if contains_any(args, &["testScope", "module"]) => {
                    return Err(format!(
                        "{tool_name} operation `test` runner `va` does not accept `testScope` or `module`"
                    ));
                }
                _ => {}
            }
        }
        "launch" => {
            validate_enum_argument(tool_name, args, "clientMode", RUNTIME_CLIENT_MODES)?;
            let client_mode = args.get("clientMode").and_then(Value::as_str);
            let is_mcp_client = matches!(client_mode, Some("mcp" | "mcp-va"));
            if is_mcp_client
                && (contains_any(args, &["c", "execute", "usePrivilegedMode", "output"])
                    || has_non_empty_array_arg(args, "rawKeys"))
            {
                return Err(format!(
                    "{tool_name} operation `launch` clientMode `mcp` does not accept direct launch flags"
                ));
            }
            if client_mode.is_some()
                && !is_mcp_client
                && contains_any(args, &["mcpConfig", "mcpPort"])
            {
                return Err(format!(
                    "{tool_name} operation `launch` direct client modes do not accept MCP flags"
                ));
            }
        }
        "tools-download" => {
            validate_enum_argument(tool_name, args, "tool", RUNTIME_TOOLS)?;
            if args
                .get("sources")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && args
                    .get("tool")
                    .and_then(Value::as_str)
                    .is_some_and(|tool| tool == "vanessa")
            {
                return Err(format!(
                    "{tool_name} operation `tools-download` accepts `sources` only for `yaxunit` or `client-mcp`"
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn runtime_operation_args(operation: &str) -> &'static [&'static str] {
    match operation {
        "config-init" => RUNTIME_CONFIG_INIT_ARGS,
        "init" => RUNTIME_INIT_ARGS,
        "build" => RUNTIME_BUILD_OPERATION_ARGS,
        "dump" => RUNTIME_DUMP_OPERATION_ARGS,
        "convert" => RUNTIME_CONVERT_OPERATION_ARGS,
        "make" => RUNTIME_MAKE_OPERATION_ARGS,
        "load" => RUNTIME_LOAD_OPERATION_ARGS,
        "syntax" => RUNTIME_SYNTAX_OPERATION_ARGS,
        "test" => RUNTIME_TEST_OPERATION_ARGS,
        "launch" => RUNTIME_LAUNCH_OPERATION_ARGS,
        "extensions" => RUNTIME_EXTENSIONS_OPERATION_ARGS,
        "tools-download" => RUNTIME_TOOLS_DOWNLOAD_OPERATION_ARGS,
        _ => &[],
    }
}

fn has_non_empty_array_arg(args: &Map<String, Value>, key: &str) -> bool {
    args.get(key)
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
}

/// Most names an unknown-argument error offers as a correction. A rejected
/// argument rarely resembles more than a couple of accepted ones, and a longer
/// list stops reading as a suggestion.
const ARGUMENT_SUGGESTION_LIMIT: usize = 3;

/// Renders the ` did you mean \`x\` or \`y\`?` fragment, or an empty string when
/// nothing accepted is close enough to the rejected name. The fragment starts
/// with a space so callers can splice it straight after their own `;`.
fn did_you_mean_clause(key: &str, accepted: &[&str]) -> String {
    let suggestions = closest_argument_names(key, accepted);
    if suggestions.is_empty() {
        return String::new();
    }
    format!(
        " did you mean {}?",
        suggestions
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(" or ")
    )
}

/// Accepted names close enough to `key` to be the one the caller meant. A name
/// differing only in case wins outright: that is a spelling mistake, not a
/// guess, and mixing it with edit-distance matches would bury it.
fn closest_argument_names<'a>(key: &str, accepted: &[&'a str]) -> Vec<&'a str> {
    let needle = key.to_lowercase();
    let same_spelling = accepted
        .iter()
        .copied()
        .filter(|name| name.to_lowercase() == needle)
        .take(ARGUMENT_SUGGESTION_LIMIT)
        .collect::<Vec<_>>();
    if !same_spelling.is_empty() {
        return same_spelling;
    }

    let budget = (needle.chars().count() / 3).max(1);
    let mut scored = accepted
        .iter()
        .copied()
        .filter_map(|name| {
            let distance = argument_name_distance(&needle, &name.to_lowercase());
            (distance <= budget).then_some((distance, name))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    scored
        .into_iter()
        .take(ARGUMENT_SUGGESTION_LIMIT)
        .map(|(_, name)| name)
        .collect()
}

/// Optimal string alignment distance: Levenshtein that also counts a swap of
/// two adjacent characters as one edit, so `nmae` stays one step from `name`.
fn argument_name_distance(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut before_previous = vec![0usize; right.len() + 1];
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0usize; right.len() + 1];

    for i in 0..left.len() {
        current[0] = i + 1;
        for j in 0..right.len() {
            let substitution = usize::from(left[i] != right[j]);
            let mut distance = (previous[j] + substitution)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
            if i > 0 && j > 0 && left[i] == right[j - 1] && left[i - 1] == right[j] {
                distance = distance.min(before_previous[j - 1] + 1);
            }
            current[j + 1] = distance;
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right.len()]
}

fn allowed_args(tool: &ToolSpec) -> Vec<&'static str> {
    let mut names = COMMON_ARGS.to_vec();
    if tool.execution.is_mutating() {
        names.extend(MUTATION_ARGS);
    }
    match tool.handler {
        ToolHandler::Metadata { .. } => names.clear(),
        ToolHandler::NativeOperation { operation, .. } => {
            match operation {
                "code-patch" => names.extend(CODE_PATCH_ARGS),
                "xdto-info" => names.extend(XDTO_INFO_ARGS),
                "xdto-edit" => names.extend(XDTO_EDIT_ARGS),
                "cf-info" => names.extend(CF_INFO_ARGS),
                "role-info" => names.extend(ROLE_INFO_ARGS),
                "role-edit" => {
                    names.clear();
                    names.extend(ROLE_EDIT_ARGS);
                }
                "subsystem-info" => names.extend(SUBSYSTEM_INFO_ARGS),
                "mxl-info" => names.extend(MXL_INFO_ARGS),
                "cfe-diff" => names.extend(CFE_DIFF_ARGS),
                "dcs-info" => names.extend(DCS_INFO_ARGS),
                "form-info" => names.extend(FORM_INFO_ARGS),
                _ => names.extend(native_args_for(operation)),
            }
            if operation == "form-edit" {
                names.push("definition");
            }
            // ADR-0070: the deferred-capable readers publish the continuation
            // vocabulary alongside their own arguments.
            if super::deferred_delivery::supports_operation(operation) {
                names.extend(super::deferred_delivery::CONTINUATION_ARGS);
            }
        }
        ToolHandler::BuildRuntime { command, .. } => {
            names.extend(BUILD_ARGS);
            // `make` is the only build command whose CLI takes an artifact
            // destination, and the adapter already forwards both names. The
            // schema is closed, so leaving them unpublished made the tool
            // impossible to call correctly (#318).
            if command.first() == Some(&"make") {
                names.extend(BUILD_MAKE_ARGS);
            }
        }
        ToolHandler::RuntimeAdapter => names.extend(RUNTIME_ARGS),
        ToolHandler::RuntimeJob { action } => names.extend(runtime_job_args(action)),
        ToolHandler::CodeIntelligence { operation } => {
            names.extend(code_intelligence_args(operation))
        }
        ToolHandler::SourceNavigation { operation } => names.extend(match operation {
            SourceNavigationOperation::Resolve => SOURCE_RESOLVE_ARGS,
            SourceNavigationOperation::Children => SOURCE_CHILDREN_ARGS,
            SourceNavigationOperation::Locate => SOURCE_LOCATE_ARGS,
        }),
        ToolHandler::SourceResources { operation } => names.extend(match operation {
            SourceResourceOperation::Resources => SOURCE_RESOURCES_ARGS,
            SourceResourceOperation::Read => SOURCE_READ_ARGS,
        }),
        ToolHandler::Diagnostics => {
            names.clear();
            names.extend(
                DIAGNOSTIC_ACTION_DESCRIPTORS
                    .iter()
                    .flat_map(|descriptor| descriptor.allowed_args.iter().copied()),
            );
        }
        ToolHandler::CodeAdapter { .. } => names.extend(code_args_for(tool.name)),
        ToolHandler::StandardsAdapter { .. } => names.extend(STANDARDS_ARGS),
        ToolHandler::Documentation { operation: "get" } => names.extend(DOCUMENTATION_GET_ARGS),
        ToolHandler::Documentation { .. } => names.extend(DOCUMENTATION_SEARCH_ARGS),
        ToolHandler::ProjectStatus | ToolHandler::ProjectMap => {}
    }
    if tool.name == "unica.mxl.decompile" {
        names.retain(|name| *name != "OutputPath" && *name != "outputPath");
    }
    // ADR-0049: one place adds the logical selector to all thirteen bridged
    // readers, so the narrow reader lists and the shared validator list cannot
    // drift apart. Narrowing the validator lists themselves is a separate
    // contract question and stays out of this bridge.
    if let Some((_, address)) = bridged_selector(tool.name) {
        names.push("sourceSet");
        if address.publishes_address() {
            names.push("metadataPath");
        }
        // A tool whose branches cannot honour an address does not gain one
        // here, but it does not lose one either: the shared catch-all already
        // handed `metadataPath` to every native tool, and this bridge removes
        // nothing. It stays accepted and is filtered out of the published
        // schema by `unpublished_bridge_args`.
    }
    names.sort_unstable();
    names.dedup();
    names
}

fn native_args_for(operation: &str) -> &'static [&'static str] {
    match operation {
        "epf-init" | "erf-init" => EXTERNAL_INIT_ARGS,
        "xdto-info" => XDTO_INFO_ARGS,
        "xdto-edit" => XDTO_EDIT_ARGS,
        "role-edit" => ROLE_EDIT_ARGS,
        _ => NATIVE_XML_DSL_ARGS,
    }
}

fn is_external_init_tool(tool: ToolSpec) -> bool {
    matches!(tool.name, "unica.epf.init" | "unica.erf.init")
}

fn required_args(tool: &ToolSpec) -> Vec<&'static str> {
    match tool.handler {
        ToolHandler::NativeOperation { operation, .. } => native_operation_descriptor(operation)
            .map(|descriptor| descriptor.required_args.to_vec())
            .unwrap_or_default(),
        ToolHandler::StandardsAdapter {
            operation: "search",
            ..
        } => vec!["query"],
        ToolHandler::Documentation {
            operation: "search",
            ..
        } => vec!["query"],
        ToolHandler::Documentation {
            operation: "get", ..
        } => vec!["documentId"],
        ToolHandler::RuntimeAdapter => runtime_required_args(tool),
        ToolHandler::RuntimeJob { action } => runtime_job_required_args(action),
        ToolHandler::CodeIntelligence { operation } => match operation {
            CodeIntelligenceOperation::Search => vec!["query"],
            CodeIntelligenceOperation::Definition => vec!["name"],
            CodeIntelligenceOperation::Outline => vec!["path"],
        },
        ToolHandler::SourceNavigation { operation } => match operation {
            SourceNavigationOperation::Resolve => vec!["sourceSet", "query"],
            SourceNavigationOperation::Children => vec!["sourceSet"],
            SourceNavigationOperation::Locate => vec!["sourceSet", "path"],
        },
        ToolHandler::SourceResources { operation } => match operation {
            SourceResourceOperation::Resources => Vec::new(),
            SourceResourceOperation::Read => vec!["snapshotId", "resourceId"],
        },
        ToolHandler::Diagnostics => vec!["action", "sourceSet"],
        ToolHandler::CodeAdapter { .. } => match tool.name {
            "unica.code.graph" => vec!["mode"],
            _ => Vec::new(),
        },
        // `v8-runner make` refuses the call without `--output`, so the schema
        // says so instead of letting every call fail in the adapter.
        ToolHandler::BuildRuntime { command, .. } if command.first() == Some(&"make") => {
            vec!["output"]
        }
        _ => Vec::new(),
    }
}

fn code_args_for(tool_name: &str) -> &'static [&'static str] {
    match tool_name {
        "unica.code.search" => CODE_SEARCH_ARGS,
        "unica.code.definition" => CODE_DEFINITION_ARGS,
        "unica.code.outline" => CODE_OUTLINE_ARGS,
        "unica.code.graph" => CODE_GRAPH_ARGS,
        _ => CODE_ARGS,
    }
}

fn code_intelligence_args(operation: CodeIntelligenceOperation) -> &'static [&'static str] {
    match operation {
        CodeIntelligenceOperation::Search => CODE_SEARCH_ARGS,
        CodeIntelligenceOperation::Definition => CODE_DEFINITION_ARGS,
        CodeIntelligenceOperation::Outline => CODE_OUTLINE_ARGS,
    }
}

fn runtime_required_args(tool: &ToolSpec) -> Vec<&'static str> {
    debug_assert!(matches!(tool.handler, ToolHandler::RuntimeAdapter));
    vec!["operation"]
}

fn runtime_job_args(action: RuntimeJobAction) -> Vec<&'static str> {
    match action {
        RuntimeJobAction::Start => RUNTIME_ARGS
            .iter()
            .copied()
            .filter(|name| !matches!(*name, "waitForExit" | "waitTimeoutMs" | "stderrOutput"))
            .collect(),
        RuntimeJobAction::Status | RuntimeJobAction::Cancel => RUNTIME_JOB_STATUS_ARGS.to_vec(),
        RuntimeJobAction::Wait => RUNTIME_JOB_WAIT_ARGS.to_vec(),
        RuntimeJobAction::Logs => RUNTIME_JOB_LOGS_ARGS.to_vec(),
        RuntimeJobAction::List => Vec::new(),
    }
}

fn runtime_job_required_args(action: RuntimeJobAction) -> Vec<&'static str> {
    match action {
        RuntimeJobAction::Start => vec!["operation"],
        RuntimeJobAction::Status
        | RuntimeJobAction::Wait
        | RuntimeJobAction::Logs
        | RuntimeJobAction::Cancel => vec!["jobId"],
        RuntimeJobAction::List => Vec::new(),
    }
}

fn property_schema(name: &str) -> Value {
    if name == "dryRun" {
        return json!({
            "type": "boolean",
            "default": true
        });
    }
    if name == "waitTimeoutMs" {
        return json!({
            "type": "integer",
            "minimum": 1,
            "maximum": 86_400_000
        });
    }

    let value_type = if matches!(
        name,
        "confirm"
            | "Detailed"
            | "detailed"
            | "Force"
            | "force"
            | "FromObject"
            | "fromObject"
            | "NoValidate"
            | "noValidate"
            | "NoRole"
            | "noRole"
            | "SetDefault"
            | "setDefault"
            | "SetMainSKD"
            | "setMainSKD"
            | "Raw"
            | "raw"
            | "WithText"
            | "withText"
            | "CreateIfMissing"
            | "createIfMissing"
            | "IsFunction"
            | "isFunction"
            | "allExtensions"
            | "checkUseModality"
            | "checkUseSynchronousCalls"
            | "configLogIntegrity"
            | "distributiveModules"
            | "emptyHandlers"
            | "externalConnection"
            | "externalConnectionServer"
            | "fullOutput"
            | "fullRebuild"
            | "handlersExistence"
            | "incorrectReferences"
            | "mobileAppClient"
            | "mobileAppServer"
            | "mobileClient"
            | "mobileClientDigiSign"
            | "server"
            | "sources"
            | "thickClientManagedApplication"
            | "thickClientOrdinaryApplication"
            | "thickClientServerManagedApplication"
            | "thickClientServerOrdinaryApplication"
            | "thinClient"
            | "unsupportedFunctional"
            | "unreferenceProcedures"
            | "usePrivilegedMode"
            | "waitForExit"
            | "webClient"
            | "includeMethods"
    ) {
        "boolean"
    } else if matches!(name, "definition" | "property") {
        "object"
    } else if matches!(
        name,
        "limit"
            | "Offset"
            | "offset"
            | "MaxParams"
            | "maxParams"
            | "mcpPort"
            | "waitTimeoutMs"
            | "maxOutputTokens"
            | "maxFiles"
            | "rangeStart"
            | "rangeEnd"
            | "timeoutSeconds"
            | "tailChars"
            | "lowerBound"
            | "upperBound"
    ) {
        "integer"
    } else if matches!(
        name,
        "codes"
            | "types"
            | "Fields"
            | "fields"
            | "Children"
            | "children"
            | "ids"
            | "edgeKinds"
            | "provenance"
            | "sections"
            | "features"
            | "filterTags"
            | "ignoreTags"
            | "objects"
            | "projects"
            | "rawKeys"
            | "scenarioFilters"
            | "sourceKinds"
            | "sourceSets"
    ) {
        "array"
    } else {
        "string"
    };

    if value_type == "array" {
        json!({ "type": "array", "items": { "type": "string" } })
    } else {
        json!({ "type": value_type })
    }
}

/// Argument documentation, keyed by the camelCase spelling.
///
/// Every argument is accepted in both PascalCase and camelCase, so the lookup
/// folds the first character and one entry serves both spellings.
///
/// A model reaches these before it reaches the skills: under MCP tool search
/// the schema is what it inspects when deciding how to call. An undescribed
/// argument therefore has to be guessed, and the tools that share
/// `NATIVE_XML_DSL_ARGS` offer well over a hundred of them.
const ARG_DESCRIPTIONS: &[(&str, &str)] = &[
    (
        "allExtensions",
        "Boolean --all-extensions covering every extension in operation syntax; only for the designer-* modes, since mode edt rejects it together with extension",
    ),
    (
        "bodyLimit",
        "Max page-body size for `unica.standards.explain` when it fetches a standard by `id`/`idOrAliasOrUrl`; the XML/DSL tools accept the key but never read it",
    ),
    (
        "body_limit",
        "Maximum size of the standard page body returned by unica.standards.explain in page mode (snake_case alias of bodyLimit); honoured only alongside id/idOrAliasOrUrl, and ignored by standards.search.",
    ),
    (
        "typeName",
        "Name of the XDTO valueType or objectType whose full detail `unica.xdto.info` returns instead of the paged type list.",
    ),
    (
        "borrowMainAttribute",
        "`unica.cfe.borrow` only: `\"Form\"` (or `true`) borrows just the attributes already shown on the form, `\"All\"` borrows every object attribute; omit it to borrow the form without data bindings",
    ),
    (
        "builder",
        "Build backend recorded by operation config-init, DESIGNER or IBCMD; DESIGNER covers the full workflow set while IBCMD needs infobase.dbms settings for server bases",
    ),
    (
        "c",
        "String passed as the platform /C key on a direct-client operation launch, e.g. StartFeaturePlayer;VAParams=tools/VAParams.json; put the processing command here rather than in rawKeys",
    ),
    (
        "cIPath",
        "The `CIPath` spelling of the command-interface path: a subsystem's `Ext/CommandInterface.xml` or its directory, relative to `cwd`, for `unica.interface.edit`/`validate`",
    ),
    (
        "capability",
        "`unica.support.edit` only: `\"on\"` or `\"off\"`, toggling whether the vendor-supported configuration may be edited at all; pass exactly one of `capability` or `set`",
    ),
    (
        "checkUseModality",
        "Boolean Designer syntax-check option (--check-use-modality) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "checkUseSynchronousCalls",
        "Boolean Designer syntax-check option (--check-use-synchronous-calls) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "ciPath",
        "Path to a subsystem's `Ext/CommandInterface.xml` (the subsystem directory also resolves) for `unica.interface.edit` and `unica.interface.validate`, relative to `cwd`",
    ),
    (
        "clientMode",
        "Required client kind for operation launch: designer, thin, thick, ordinary, mcp or mcp-va; mcp and mcp-va take mcpConfig/mcpPort, the others take the direct launch flags",
    ),
    (
        "codes",
        "Array of diagnostic codes such as \"АПК:142\" or \"LineLength\"; on standards.explain it selects diagnostics mode and outranks snippet/id/query, while standards.search ignores it. Diagnostics uses provider-qualified entries inside filter.codes instead.",
    ),
    (
        "compatibilityMode",
        "Platform compatibility mode for the generated `Configuration.xml`, e.g. `Version8_3_27`; default `Version8_3_27` in `unica.cf.init` and `Version8_3_24` in `unica.cfe.init`, which infers it from `configPath`",
    ),
    (
        "config",
        "Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml.",
    ),
    (
        "configDir",
        "Root of a configuration dump (the directory holding `Configuration.xml`), relative to `cwd`",
    ),
    (
        "configLogIntegrity",
        "Boolean Designer syntax-check option (--config-log-integrity) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "configPath",
        "Path to `Configuration.xml` or the dump directory for `unica.cf.edit`, `unica.cf.info` and `unica.cf.validate`, and the path of the base configuration for `unica.cfe.init`/`borrow`/`diff`; relative to `cwd`. `unica.cf.init` ignores it and writes to `outputDir`.",
    ),
    (
        "confirm",
        "Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own.",
    ),
    (
        "connection",
        "Infobase connection string (for example File=build/ib) honoured only by operation config-init, which stores it as infobase.connection in the project config; unica.build.* does not accept it",
    ),
    (
        "content",
        "BSL text for unica.code.patch: inserted at the selector for operation insert, appended to the end of the module when insert names no selector, or written over the selected method or anchor for operation replace",
    ),
    (
        "context",
        "`unica.cfe.patch_method` only: BSL context directive, one of `НаСервере`, `НаКлиенте`, `НаСервереБезКонтекста`; omit for object, manager, record-set and value-manager modules, which take no directive",
    ),
    (
        "createIfMissing",
        "`unica.interface.edit` only: boolean, create `CommandInterface.xml` when it does not exist yet instead of failing",
    ),
    (
        "cursor",
        "Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot",
    ),
    (
        "cwd",
        "Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it",
    ),
    (
        "dataSet",
        "`unica.dcs.edit` only: name of the data set the operation applies to, defaulting to the first data set in the schema",
    ),
    (
        "database",
        "String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute",
    ),
    (
        "dbPassword",
        "String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments",
    ),
    (
        "dbUser",
        "String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name",
    ),
    (
        "definition",
        "`unica.form.edit` only: the edit DSL as an inline JSON object (`elements`, `attributes`, `commands`, `formEvents`, `removeElements`, …); supply either this or `jsonPath`, never both",
    ),
    (
        "definitionFile",
        "Path to a JSON file holding a batch of operations or a full definition, for `unica.cf.edit`, `interface.edit`, `subsystem.edit`/`compile` and `dcs.compile`; relative to `cwd`",
    ),
    (
        "detail",
        "How much detail to return from unica.code.graph: names, signatures or bodies",
    ),
    (
        "detailed",
        "Boolean for the `*.validate` tools: print every check, including the ones that passed, instead of only the failures",
    ),
    (
        "dir",
        "Edge direction to follow on unica.code.graph - in, out, or both; applies to the traversal modes such as neighbors, callers, and callees",
    ),
    (
        "documentId",
        "Stable locator of a unica.documentation.search hit, passed verbatim to unica.documentation.get to fetch the full document text: configuration-help:<source-set>:<path> for the workspace configuration's embedded help, platform-syntax-help:<corpus>:<path> for the installed platform's help, an absolute https://kb.1ci.com/... page address for the vendor knowledge base, and an https://v8std.ru/... address for a development standard; the provider that minted the locator is the only one that resolves it.",
    ),
    (
        "distributiveModules",
        "Boolean Designer syntax-check option (--distributive-modules) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "dryRun",
        "Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution.",
    ),
    (
        "edgeKinds",
        "Array of graph edge-kind names, forwarded to the analyzer as edge_kinds; unica.code.graph only, and the Unica contract does not enumerate the accepted values",
    ),
    ("emitDsl", "Declared string argument that no tool handler reads"),
    (
        "emptyHandlers",
        "Boolean Designer syntax-check option (--empty-handlers) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "execute",
        "Workspace-relative .epf to run via the platform /Execute key on a direct-client operation launch; required and must end in .epf when waitForExit is true",
    ),
    (
        "expand",
        "`unica.form.info` only: name or title of a collapsed section to expand, or `\"*\"` to expand all of them",
    ),
    (
        "extension",
        "Name of the 1C extension to act on for operation dump, make, load or syntax; build rejects it, so build an extension by selecting its configured sourceSet instead",
    ),
    (
        "extensionPath",
        "Path to the extension — its directory or its `Configuration.xml` — for every `unica.cfe.*` tool, relative to `cwd`; the base configuration goes in `configPath` instead",
    ),
    (
        "externalConnection",
        "Boolean Designer syntax-check context flag (--external-connection) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "externalConnectionServer",
        "Boolean Designer syntax-check context flag (--external-connection-server) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "features",
        "Array of Vanessa Automation feature paths narrowing operation test with testRunner va; each entry becomes one --feature",
    ),
    (
        "filterTags",
        "Array of Vanessa Automation tags to include for operation test with testRunner va; each entry becomes one --filter-tag",
    ),
    (
        "force",
        "Boolean --force: on unica.runtime.execute it overwrites an existing project config for config-init and re-downloads the payload for tools-download; native XML tools may expose their own operation-specific Force argument.",
    ),
    (
        "formName",
        "Name of the managed form as a 1C identifier: the form to create in `unica.form.add`, `epf.init` and `erf.init`, or the form to delete in `unica.form.remove`",
    ),
    (
        "formPath",
        "Path to an existing `Form.xml`, or the form directory that resolves to it, for `unica.form.info`, `unica.form.edit` and `unica.form.validate`, relative to `cwd`",
    ),
    (
        "format",
        "On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough.",
    ),
    (
        "fromObject",
        "`unica.form.compile` only: boolean selecting preset generation from the object's metadata; use it instead of `jsonPath`, and let `outputPath` supply the object when `objectPath` is omitted",
    ),
    (
        "fullOutput",
        "Boolean turning on the runner's --full output verbosity for operation test; it is not a build full rebuild, which is fullRebuild on operation build",
    ),
    (
        "fullRebuild",
        "Boolean for operation build that runs one complete rebuild instead of the runner's normal build strategy and disables the automatic full fallback; use it after branch switches, rebases, large object moves or suspect incremental state",
    ),
    (
        "handlersExistence",
        "Boolean Designer syntax-check option (--handlers-existence) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "id",
        "Standard id, alias or URL for standards.explain (lower-precedence alias of idOrAliasOrUrl), but a graph node id such as method:CommonModule.Sales.OnPost for code.graph; standards.search ignores it.",
    ),
    (
        "idOrAliasOrUrl",
        "Standard number, alias or full URL (e.g. \"644\") that puts standards.explain in page-fetch mode; prefer it over id, which it overrides when both are passed, and standards.search ignores it.",
    ),
    (
        "ids",
        "Array of code-graph node ids for unica.code.graph, forwarded as ids alongside the single-node id argument; use it when one request targets several nodes",
    ),
    (
        "ignoreTags",
        "Array of Vanessa Automation tags to exclude for operation test with testRunner va; each entry becomes one --ignore-tag",
    ),
    (
        "includeMethods",
        "Boolean for unica.code.outline controlling whether method entries appear in the outline; defaults to true",
    ),
    (
        "incorrectReferences",
        "Boolean Designer syntax-check option (--incorrect-references) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "infobase",
        "String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init",
    ),
    (
        "interceptorType",
        "`unica.cfe.patch_method` only: `\"Before\"` to generate a `&Перед` interceptor or `\"After\"` for `&После`",
    ),
    (
        "isFunction",
        "`unica.cfe.patch_method` only: reserved boolean constrained to `false`, because v1 patches parameterless procedures only",
    ),
    (
        "jobId",
        "UUID of a durable runtime job as returned by unica.runtime.job.start; required by the job status, wait, logs and cancel tools",
    ),
    (
        "jsonPath",
        "Path to the JSON DSL file, relative to `cwd`, for `unica.form.compile`, `unica.form.edit`, `unica.mxl.compile` and `unica.role.compile`",
    ),
    ("kind", "Declared string argument that no tool handler reads"),
    (
        "lang",
        "`unica.help.add` only: language code of the help page to create, default `\"ru\"`; `language` is accepted as an alias for it",
    ),
    (
        "language",
        "Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained; on `unica.documentation.search` and `unica.documentation.get` it picks the locale of the platform help containers to read and of the signature returned with each hit, defaulting to ru, and each corpus falls back on its own to the installed locale (the English `root` container first) when the installation ships no containers in the requested one, so every section and document reports the locale that actually answered",
    ),
    (
        "limit",
        "Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per role), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048).",
    ),
    (
        "maxErrors",
        "Stop a validate tool after this many errors: default 30 for `unica.cf.validate`, `cfe.validate`, `form.validate` and `interface.validate`, 20 for `unica.dcs.validate` and `unica.mxl.validate`; `unica.role.validate` and `unica.subsystem.validate` accept the key but ignore it.",
    ),
    (
        "maxOutputTokens",
        "Integer output budget for unica.code.graph, forwarded as max_output_tokens; use it to keep a large graph answer within context",
    ),
    (
        "mcpConfig",
        "Workspace-relative path to the client MCP config file; accepted only by operation launch with clientMode mcp or mcp-va",
    ),
    (
        "mcpPort",
        "Integer TCP port for the client MCP server; accepted only by operation launch with clientMode mcp or mcp-va",
    ),
    (
        "metadataPath",
        "Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics.",
    ),
    (
        "methodName",
        "`unica.cfe.patch_method` only: name of the existing parameterless procedure to intercept; must match a 1C identifier (Latin or Cyrillic letter or underscore, then letters, digits, underscores)",
    ),
    (
        "minSeverity",
        "Lowest diagnostic severity unica.code.diagnostics should report: error, warning, info, or hint",
    ),
    (
        "mobileAppClient",
        "Boolean Designer syntax-check context flag (--mobile-app-client) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "mobileAppServer",
        "Boolean Designer syntax-check context flag (--mobile-app-server) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "mobileClient",
        "Boolean including the mobile client context in operation syntax (--mobile-client); only for the designer-* modes",
    ),
    (
        "mobileClientDigiSign",
        "Boolean Designer syntax-check option (--mobile-client-digi-sign) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "mode",
        "Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full|incremental|partial for dump, load|merge for load, designer-config|designer-modules|edt for syntax, and the client kind for an mcp or mcp-va launch; every other tool defines its own enum.",
    ),
    (
        "module",
        "Full BSL module name such as CommonModule.МоиТесты; required for operation test with testRunner yaxunit and testScope module, and rejected for testRunner va",
    ),
    (
        "moduleHint",
        "Substring of a module path or object name that narrows unica.code.definition when the same method name exists in several modules; matched case-insensitively",
    ),
    (
        "modulePath",
        "`unica.cfe.patch_method` only: dotted module reference such as `Catalog.X.ObjectModule`, `CommonModule.X` or `Document.X.Form.Y` — a metadata path, not a filesystem path",
    ),
    (
        "name",
        "Subject name whose meaning is tool-scoped: the object being created by `cf.init`, `cfe.init`, `epf.init` and `erf.init`, and the required BSL method to locate on `unica.code.definition`. The eight narrowed native XML readers no longer take it: they answer with every section at once, so there is nothing left for it to drill into (ADR-0048).",
    ),
    (
        "namePrefix",
        "`unica.cfe.init` only: prefix for the extension's own objects, defaulting to the extension name plus `_`; `unica.cfe.patch_method` reuses the stored prefix to name generated procedures",
    ),
    ("noRole", "`unica.cfe.init` only: boolean, skip scaffolding the extension's main role"),
    (
        "noSelection",
        "`unica.dcs.edit` only: do not add the new field to the settings variant's selection; pass a real JSON boolean, a string is ignored",
    ),
    (
        "noValidate",
        "Boolean for `unica.cf.edit`, `interface.edit`, `subsystem.edit` and `dcs.edit`/`compile`: hide the verbose auto-validation report; the mandatory 8.3.27 check before commit still runs. `unica.subsystem.compile` accepts the argument but ignores it.",
    ),
    (
        "object",
        "On unica.runtime.execute this is one metadata object name for operation dump with mode partial, written in colon form such as Catalog:Номенклатура (use objects for several); on the native XML tools Object is instead the dotted metadata reference the tool acts on, such as Catalog.Контрагенты.Form.ФормаЭлемента for unica.cfe.borrow.",
    ),
    (
        "objectName",
        "Name of the owning object for `unica.form.remove` and `unica.template.add`/`remove`; for `unica.help.add` it is instead the object's path under `srcDir`, e.g. `Catalogs/МойСправочник`",
    ),
    (
        "objectPath",
        "Path to an object's metadata XML — a directory resolves to `<name>/<name>.xml` — for `unica.form.add`, relative to `cwd`",
    ),
    (
        "objects",
        "Array of metadata object names for operation dump with mode partial; supply this or object, and note partial dump is preview-only so pair it with dryRun true",
    ),
    (
        "offset",
        "Number of output lines to skip in the paginating read tools, default 0; combine it with `limit` to page through a long report",
    ),
    (
        "operation",
        "Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch — read the enum published in the tool's own schema.",
    ),
    (
        "output",
        "Workspace-relative destination: the artifact file for make (a publish directory for external source-sets), the conversion directory for convert, and the platform /Out log for a direct-client launch",
    ),
    (
        "outputDir",
        "Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `role.compile`/`subsystem.compile`",
    ),
    (
        "outputPath",
        "Path of the single file to generate: the `Form.xml` for `unica.form.compile` or the `Template.xml` for `unica.dcs.compile` and `unica.mxl.compile`",
    ),
    (
        "parent",
        "`unica.subsystem.compile` only: path to the parent subsystem's XML when creating a nested subsystem; omit it to register the new subsystem in `Configuration.xml`",
    ),
    (
        "password",
        "String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments",
    ),
    (
        "path",
        "Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for path-based unica.code.* tools, the canonical alias of the object/config path argument on native XML tools, and a plain --path passthrough on unica.build.*.",
    ),
    (
        "platformVersion",
        "Requested platform installation version for unica.documentation.search and unica.documentation.get, matched against an installation directory name exactly, for example 8.3.27.2074; when omitted the project's own tools.platform.version constrains the choice, and without that the numerically newest installation found under a configured platform root wins; a tools.platform.path pin names the installation directly instead of walking the roots, with the same version constraints applied to it.",
    ),
    (
        "position",
        "Where unica.code.patch places the content relative to the selector: before or after. Accepted only when insert names a selector",
    ),
    (
        "processorName",
        "Name of the owning object, used together with `templateName` and `srcDir` instead of a direct `templatePath` by `unica.mxl.validate`; `unica.help.add` also accepts it as an alias of `objectName`. `unica.mxl.info` addresses a template by `templatePath` only.",
    ),
    (
        "projects",
        "Array of EDT project names for operation syntax with mode edt; the designer-config and designer-modules modes reject it",
    ),
    (
        "provenance",
        "Array of provenance filter values forwarded to the analyzer as provenance; unica.code.graph only, and the Unica contract does not enumerate the accepted values",
    ),
    (
        "purpose",
        "Two different enums: form purpose, which differs per tool — `unica.form.add` takes `Object`, `List`, `Choice`, `Record` (default `Object`), while `unica.form.compile` takes `Item`, `Folder`, `List`, `Choice`, `Record` (default `Item`, inferred from the form name, and its from-object path currently supports only `List` and `Item`) — and extension purpose for `unica.cfe.init` (`Patch`, `Customization`, `AddOn`, default `Customization`).",
    ),
    (
        "query",
        "Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback",
    ),
    (
        "raw",
        "`unica.dcs.info` only: supported only with `Mode=query`; true returns the full query text without headers or pagination and ignores `limit`/`offset`.",
    ),
    (
        "rawKeys",
        "Array of extra non-reserved platform launch keys such as /TESTMANAGER for a direct-client operation launch; never repeat /C, /Execute or /Out here",
    ),
    (
        "rightsPath",
        "Path to a role's `Rights.xml`, or the role directory that resolves to it, for `unica.role.info` and `unica.role.validate`, relative to `cwd`",
    ),
    (
        "resourceId",
        "Opaque resource identifier returned inside one source.resources snapshot; valid only together with the snapshotId that issued it",
    ),
    (
        "scenarioFilters",
        "Array of Vanessa Automation scenario filters for operation test with testRunner va; each entry becomes one --scenario-filter",
    ),
    (
        "scope",
        "Bounded source.resources manifest scope: self, aggregate, or registrations",
    ),
    (
        "delivery",
        "Deferred continuation only: `\"full\"` asks for the whole stored snapshot; it expresses the caller's intent and proves no human confirmation (ADR-0070)",
    ),
    (
        "filter",
        "Deferred continuation only: case-insensitive substring over entity names inside the selected section of the stored snapshot",
    ),
    (
        "page",
        "Deferred continuation only: 1-based page of 50 entities inside the selected section of the stored snapshot",
    ),
    (
        "resultRef",
        "Continuation reference issued by a deferred manifest of the same tool: the call is served from the immutable stored snapshot without re-reading the source (ADR-0070)",
    ),
    (
        "section",
        "On `unica.cf.info`: drill-down section of `Configuration.xml`, currently just `\"home-page\"`; `name` is accepted as an alias. On deferred continuations: the top-level section of the stored snapshot to slice",
    ),
    (
        "selector",
        "Optional object naming the unica.code.patch edit point: exactly one of {\"method\": \"Name\"} for a whole procedure or function, or {\"anchor\": \"text\"} for a fragment that occurs once inside one method. Required by replace; when insert omits it the content goes to the end of the module",
    ),
    (
        "server",
        "Boolean including the server context in operation syntax (--server); only for the designer-* modes",
    ),
    (
        "set",
        "`unica.support.edit` only: the new support rule for the object at `path` — `\"editable\"`, `\"off-support\"` or `\"locked\"`; pass exactly one of `set` or `capability`",
    ),
    (
        "setDefault",
        "`unica.form.add` only: `true` assigns the new form to the object's `Default*Form` slot, `false` leaves that slot untouched, and omitting it fills only an empty slot",
    ),
    (
        "setMainSKD",
        "`unica.template.add` only: boolean overwriting an already-filled `MainDataCompositionSchema` with the new DCS template; an empty slot is filled automatically anyway",
    ),
    (
        "settings",
        "Workspace-relative path to the merge settings XML; required by operation load with mode merge and rejected with any other load mode",
    ),
    (
        "showDenied",
        "`unica.role.info` only: also list denied rights, which are hidden by default; pass a real JSON boolean, a string is ignored",
    ),
    (
        "snippet",
        "Literal BSL source text for standards.explain to explain against standards, sent with language and limit; codes outranks it when both are passed, and standards.search ignores it.",
    ),
    (
        "snapshotId",
        "Opaque application-instance and workspace-bound identifier returned by source.resources; expires after five minutes",
    ),
    (
        "sourceDir",
        "Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead.",
    ),
    (
        "sourceKinds",
        "Optional filter of unica.documentation.search by source kind, not by provider id: an array of configuration-documentation, platform-help and/or development-standard; providers without a matching corpus are not polled and their sections are not published, an empty or omitted array means every kind, and an unknown value is refused rather than silently ignored.",
    ),
    (
        "sourceSet",
        "Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set",
    ),
    (
        "sourceSets",
        "Array of source-set names for operation extensions when several extensions are synchronized at once; use the singular sourceSet for one",
    ),
    (
        "sources",
        "Boolean that on operation tools-download fetches sources instead of the prebuilt release artifact; omit it to get the ready artifact, such as build/tools/client_mcp.cfe. What the source route yields differs by tool: client-mcp gets an EDT tree that only 1cedtcli can build and no .cfe at all, while yaxunit gets the tests source-set. Supported only for tool yaxunit or client-mcp and rejected for vanessa",
    ),
    (
        "srcDir",
        "Directory holding `<objectName>.xml`, default `src`; for `unica.form.remove` and `unica.template.add`/`remove` point it at the type folder such as `src/Reports`, and `unica.help.add` uses it too",
    ),
    (
        "stderrOutput",
        "Workspace-relative file capturing stderr of the 1C client process in a bounded launch; requires waitForExit true, must differ from output, and unica.runtime.job.start rejects it",
    ),
    (
        "subsystemPath",
        "Path to a subsystem XML or `Subsystems` directory, relative to `cwd`; `unica.subsystem.info` returns the registered tree for a directory, the ancestor chain plus descendants for a registered XML, and local data without `tree` for an unregistered XML",
    ),
    (
        "synonym",
        "Human-readable synonym written into the generated XML; it defaults to the matching `name`, `formName` or `templateName` when omitted",
    ),
    (
        "tailChars",
        "Integer 1..32768 bounding how many trailing characters of stdout and stderr unica.runtime.job.logs returns, defaulting to 4096",
    ),
    (
        "target",
        "String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name",
    ),
    (
        "targetKind",
        "Optional `unica.source.resolve` filter: `metadataObject` or `module`; it narrows exact or prefix matches without changing their canonical metadataPath",
    ),
    (
        "targetPath",
        "Alias of `path` for `unica.support.edit`: the dump directory, object XML or form XML whose support state is being changed",
    ),
    (
        "templateName",
        "Name of the template to create with `unica.template.add`, delete with `unica.template.remove`, or address with `unica.mxl.validate` together with `processorName`",
    ),
    (
        "templatePath",
        "Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument.",
    ),
    (
        "templateType",
        "`unica.template.add` only: one of `HTML`, `Text`, `SpreadsheetDocument`, `BinaryData`, `DataCompositionSchema` — the input keyword, not the resulting metadata type name",
    ),
    (
        "testRunner",
        "Required test engine for operation test: yaxunit, which then also requires testScope, or va, which rejects testScope and module",
    ),
    (
        "testScope",
        "YaXUnit scope for operation test, all or module; required with testRunner yaxunit, needs module when set to module, and rejected with testRunner va",
    ),
    (
        "thickClientManagedApplication",
        "Boolean Designer syntax-check context flag (--thick-client-managed-application) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "thickClientOrdinaryApplication",
        "Boolean Designer syntax-check context flag (--thick-client-ordinary-application) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "thickClientServerManagedApplication",
        "Boolean Designer syntax-check context flag (--thick-client-server-managed-application) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "thickClientServerOrdinaryApplication",
        "Boolean Designer syntax-check context flag (--thick-client-server-ordinary-application) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "thinClient",
        "Boolean including the thin client context in operation syntax (--thin-client); only for the designer-* modes",
    ),
    (
        "timeoutSeconds",
        "Integer seconds bounding a blocking call: 1..60 (default 30) for unica.runtime.job.wait, and 30..3600 for unica.code.diagnostics action=analyze; diagnostics falls back to operational.code_diagnostics.analyze_timeout_seconds from workspace config, then to 120.",
    ),
    (
        "tool",
        "Runner tool payload to fetch with operation tools-download: yaxunit, vanessa or client-mcp",
    ),
    (
        "types",
        "Array of strings forwarded unchanged as the types parameter of the standards search; honoured only by standards.search and by standards.explain given query alone, with no allowed values declared.",
    ),
    (
        "unreferenceProcedures",
        "Boolean Designer syntax-check option (--unreference-procedures) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "unsupportedFunctional",
        "Boolean Designer syntax-check option (--unsupported-functional) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "usePrivilegedMode",
        "Boolean --use-privileged-mode for a direct-client operation launch; the mcp and mcp-va client modes reject it",
    ),
    (
        "user",
        "String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name",
    ),
    (
        "value",
        "Payload for `operation`: a shorthand string batched with `;;`, a JSON string, or the whole inline JSON definition for `unica.dcs.compile` and `unica.subsystem.compile`",
    ),
    (
        "variant",
        "`unica.dcs.edit` only: name of the settings variant the operation applies to, defaulting to the first variant in the schema",
    ),
    (
        "vendor",
        "Vendor string written into the generated `Configuration.xml` by `unica.cf.init` and `unica.cfe.init`",
    ),
    (
        "version",
        "Configuration or extension version string such as `1.0.0.1`, written into the generated `Configuration.xml` by `unica.cf.init` and `unica.cfe.init`",
    ),
    (
        "waitForExit",
        "Boolean opt-in to a bounded external EPF launch; true requires clientMode thin plus execute, output, stderrOutput and waitTimeoutMs, and unica.runtime.job.start does not accept it",
    ),
    (
        "waitTimeoutMs",
        "Integer 1..86400000 milliseconds bounding a waitForExit launch; it is not the runner's overall timeout, which is execution_timeout in v8project.yaml",
    ),
    (
        "webClient",
        "Boolean including the web client context in operation syntax (--web-client); only for the designer-* modes",
    ),
    (
        "withText",
        "`unica.mxl.info` only: boolean including static cell text and template strings with `[Parameter]` substitutions in the report",
    ),
    (
        "workdir",
        "Working directory string forwarded to the runner as --workdir; accepted by every runtime operation and left unset in all documented workflows",
    ),
];

fn description_for_arg(name: &str) -> Option<&'static str> {
    let mut canonical = String::with_capacity(name.len());
    let mut chars = name.chars();
    if let Some(first) = chars.next() {
        canonical.extend(first.to_lowercase());
        canonical.push_str(chars.as_str());
    }
    ARG_DESCRIPTIONS
        .iter()
        .find(|(candidate, _)| *candidate == canonical)
        .map(|(_, description)| *description)
}

fn property_schema_for_tool(tool: &ToolSpec, name: &str) -> Value {
    if let ToolHandler::NativeOperation { operation, .. } = tool.handler {
        if super::deferred_delivery::supports_operation(operation) {
            match name {
                "resultRef" => return json!({"type": "string", "minLength": 1}),
                "section" => return json!({"type": "string", "minLength": 1}),
                "filter" => return json!({"type": "string", "minLength": 1}),
                "page" => return json!({"type": "integer", "minimum": 1}),
                "delivery" => return json!({"type": "string", "const": "full"}),
                _ => {}
            }
        }
    }
    if tool.name == "unica.runtime.execute" && name == "dryRun" {
        return json!({
            "type": "boolean",
            "default": true,
            "description": "Preview typed v8-runner runtime arguments; omitted or true reports the planned command without mutation, while false runs a classified operation and returns its terminal result in this call with a named risk warning; an unclassified operation stays refused."
        });
    }
    if tool.name == "unica.role.edit" {
        return match name {
            "sourceSet" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": r"^\S(?:.*\S)?$",
                "description": "Exact configured source-set name; physical source paths are not accepted."
            }),
            "metadataPath" => json!({
                "type": "string",
                "minLength": 6,
                "pattern": ROLE_METADATA_PATH_PATTERN,
                "description": "Canonical logical role address in the form Role.<name>."
            }),
            "operations" => json!({
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "op": {"const": "setRight"},
                        "objectName": {
                            "type": "string",
                            "minLength": 3,
                            "pattern": ROLE_OBJECT_NAME_PATTERN
                        },
                        "right": {
                            "type": "string",
                            "enum": all_role_right_names().into_iter().collect::<Vec<_>>()
                        },
                        "value": {"type": "boolean"}
                    },
                    "required": ["op", "objectName", "right", "value"]
                },
                "description": "Ordered closed setRight operations; each effect is reported by operationIndex."
            }),
            "dryRun" => json!({
                "type": "boolean",
                "default": true,
                "description": "Preview the typed role edit without writing workspace files; when omitted it defaults to true. Send false only when the user explicitly requests application."
            }),
            _ => property_schema(name),
        };
    }
    if matches!(tool.name, "unica.xdto.info" | "unica.xdto.edit") {
        return match name {
            "sourceSet" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": r"^\S(?:.*\S)?$"
            }),
            "typeName" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": xml_ncname_pattern()
            }),
            "metadataPath" => json!({
                "type": "string",
                "pattern": format!(
                    r"^(?:XDTOPackage|ПакетXDTO)\.{}$",
                    xml_property_path_segment_pattern_body()
                ),
                "description": "Logical address of an XDTO package in the form `XDTOPackage.<name>`; the physical `Package.bin` path is rejected."
            }),
            "operations" => xdto_edit_operations_schema(),
            "dryRun" => json!({
                "type": "boolean",
                "default": true,
                "description": "Preview the typed XDTO operations without writing workspace files; when omitted it defaults to true. Send false only when the user explicitly requests application."
            }),
            "limit" => json!({
                "type": "integer",
                "minimum": 1,
                "maximum": SOURCE_NAVIGATION_LIMIT_MAX
            }),
            "cursor" => json!({"type": "string", "minLength": 1, "pattern": r"^\S+$"}),
            _ => property_schema(name),
        };
    }
    if tool.name == "unica.form.edit" && name == "definition" {
        return form_edit_definition_schema();
    }
    if tool.name == "unica.code.search" {
        return match name {
            "query" => json!({ "type": "string", "minLength": 1, "pattern": r"\S" }),
            "limit" => json!({ "type": "integer", "minimum": 1, "maximum": 50 }),
            "sourceSet" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": r"^\S(?:.*\S)?$",
                "description": "Canonical configured source-set name that scopes every search role."
            }),
            "metadataPath" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": r"\S",
                "description": "Optional canonical logical address inside sourceSet; every role is restricted to the resolved module or metadata-object subtree."
            }),
            "sourceDir" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": r"\S",
                "description": "Legacy workspace-relative migration selector used instead of sourceSet; it searches the entire resolved source root and cannot be combined with metadataPath. Prefer sourceSet for logical addressing."
            }),
            _ => property_schema(name),
        };
    }
    if tool.name == "unica.code.patch" {
        return match name {
            "sourceSet" | "content" => {
                json!({ "type": "string", "minLength": 1, "pattern": r"\S" })
            }
            "metadataPath" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": r"\S",
                "description": "Canonical logical module address inside sourceSet, for example CommonModule.Service.Module or Catalog.Items.ObjectModule."
            }),
            "operation" => json!({ "type": "string", "enum": ["insert", "replace"] }),
            "position" => json!({ "type": "string", "enum": ["before", "after"] }),
            "selector" => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "method": { "type": "string", "minLength": 1 },
                    "anchor": { "type": "string", "minLength": 1 }
                },
                "oneOf": [
                    { "required": ["method"] },
                    { "required": ["anchor"] }
                ]
            }),
            _ => property_schema(name),
        };
    }
    if matches!(tool.handler, ToolHandler::SourceNavigation { .. }) {
        return match name {
            "path" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": r"\S",
                "description": "Source file to look up, given either workspace-relative or relative to the named source set; the answer names the metadata address that owns it"
            }),
            "sourceSet" | "query" | "metadataPath" | "cursor" => {
                json!({ "type": "string", "minLength": 1, "pattern": r"\S" })
            }
            "mode" => json!({ "type": "string", "enum": ["exact", "prefix"] }),
            "targetKind" => json!({
                "type": "string",
                "enum": ["metadataObject", "module"]
            }),
            "limit" => json!({
                "type": "integer",
                "minimum": 1,
                "maximum": SOURCE_NAVIGATION_LIMIT_MAX
            }),
            _ => property_schema(name),
        };
    }
    if let ToolHandler::SourceResources { operation } = tool.handler {
        return match (operation, name) {
            (SourceResourceOperation::Resources, "scope") => json!({
                "type": "string",
                "enum": ["self", "aggregate", "registrations"]
            }),
            (SourceResourceOperation::Resources, "limit") => json!({
                "type": "integer",
                "minimum": 1,
                "maximum": SOURCE_RESOURCE_PAGE_LIMIT_MAX
            }),
            (SourceResourceOperation::Read, "limit") => json!({
                "type": "integer",
                "minimum": 1,
                "maximum": SOURCE_READ_LIMIT_MAX
            }),
            (SourceResourceOperation::Read, "offset") => json!({
                "type": "integer",
                "minimum": 0,
                "description": "Zero-based byte offset inside the immutable resource snapshot"
            }),
            (_, "sourceSet" | "metadataPath" | "snapshotId" | "resourceId" | "cursor") => {
                json!({ "type": "string", "minLength": 1, "pattern": r"\S" })
            }
            _ => property_schema(name),
        };
    }
    if tool.name == "unica.cfe.patch_method" {
        return match name {
            "Context" | "context" => {
                json!({ "type": "string", "enum": CFE_PATCH_METHOD_CONTEXTS })
            }
            "InterceptorType" | "interceptorType" => {
                json!({ "type": "string", "enum": CFE_PATCH_METHOD_INTERCEPTOR_TYPES })
            }
            "MethodName" | "methodName" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": CFE_PATCH_METHOD_IDENTIFIER_PATTERN
            }),
            "IsFunction" | "isFunction" => json!({
                "type": "boolean",
                "const": false,
                "description": "cfe.patch_method v1 supports parameterless procedures only; base method signature resolution is not implemented"
            }),
            _ => property_schema(name),
        };
    }
    if matches!(
        tool.handler,
        ToolHandler::RuntimeAdapter
            | ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Start
            }
    ) {
        match name {
            "operation" => return json!({ "type": "string", "enum": RUNTIME_OPERATIONS }),
            "clientMode" => {
                return json!({
                    "type": "string",
                    "enum": RUNTIME_CLIENT_MODES
                });
            }
            "testRunner" => return json!({ "type": "string", "enum": RUNTIME_TEST_RUNNERS }),
            "testScope" => return json!({ "type": "string", "enum": RUNTIME_TEST_SCOPES }),
            "tool" => return json!({ "type": "string", "enum": RUNTIME_TOOLS }),
            _ => {}
        }
    }
    match tool.name {
        "unica.support.edit" => match name {
            "Capability" | "capability" => {
                return json!({ "type": "string", "enum": ["on", "off"] });
            }
            "Set" | "set" => {
                return json!({ "type": "string", "enum": ["editable", "off-support", "locked"] });
            }
            _ => {}
        },
        "unica.code.graph" => match name {
            "mode" => return json!({ "type": "string", "enum": CODE_GRAPH_MODES }),
            "dir" => return json!({ "type": "string", "enum": CODE_GRAPH_DIRECTIONS }),
            "detail" => return json!({ "type": "string", "enum": CODE_GRAPH_DETAIL }),
            _ => {}
        },
        _ => {}
    }
    property_schema(name)
}

fn xml_ncname_pattern() -> String {
    format!("^{}$", xml_ncname_pattern_body())
}

fn xml_qname_pattern() -> String {
    let ncname = xml_ncname_pattern_body();
    format!("^{ncname}:{ncname}$")
}

fn xml_property_path_pattern() -> String {
    let ncname_without_dot = xml_property_path_segment_pattern_body();
    let continuation = xml_ncname_char_without_dot_pattern_body();
    let segment = format!(r"{ncname_without_dot}(?:\\\.{continuation}*)*");
    format!(r"^{segment}(?:\.{segment})*$")
}

fn xml_property_path_segment_pattern_body() -> String {
    format!(
        "{}{}*",
        xml_ncname_start_pattern_body(),
        xml_ncname_char_without_dot_pattern_body()
    )
}

fn xml_ncname_start_pattern_body() -> String {
    xml_character_class(XML_NCNAME_START_BMP_RANGES.iter())
}

fn xml_ncname_char_without_dot_pattern_body() -> String {
    xml_ncname_char_pattern_body(false)
}

fn xml_ncname_pattern_body() -> String {
    format!(
        "{}{}*",
        xml_ncname_start_pattern_body(),
        xml_ncname_char_pattern_body(true)
    )
}

fn xml_ncname_char_pattern_body(include_dot: bool) -> String {
    xml_character_class(
        XML_NCNAME_START_BMP_RANGES.iter().chain(
            XML_NCNAME_CONTINUATION_RANGES
                .iter()
                .filter(|&&(start, end)| include_dot || (start, end) != ('.', '.')),
        ),
    )
}

fn xml_character_class<'a>(ranges: impl IntoIterator<Item = &'a (char, char)>) -> String {
    let mut pattern = String::from("[");
    for &(start, end) in ranges {
        append_xml_pattern_character(&mut pattern, start);
        if start != end {
            pattern.push('-');
            append_xml_pattern_character(&mut pattern, end);
        }
    }
    pattern.push(']');
    pattern
}

fn append_xml_pattern_character(pattern: &mut String, character: char) {
    if matches!(character, '\\' | '[' | ']' | '^' | '-') {
        pattern.push('\\');
    }
    pattern.push(character);
}

fn validate_argument_type(tool_name: &str, key: &str, value: &Value) -> Result<(), String> {
    let expected = expected_scalar_type(key);
    match expected {
        Some("boolean") if !value.is_boolean() => {
            Err(format!("{tool_name} argument `{key}` must be boolean"))
        }
        Some("integer") if value.as_i64().is_none() => {
            Err(format!("{tool_name} argument `{key}` must be integer"))
        }
        Some("array") if !value.is_array() => {
            Err(format!("{tool_name} argument `{key}` must be array"))
        }
        Some("object") if !value.is_object() => {
            Err(format!("{tool_name} argument `{key}` must be object"))
        }
        Some("string") if !value.is_string() => {
            Err(format!("{tool_name} argument `{key}` must be string"))
        }
        _ => Ok(()),
    }
}

fn expected_scalar_type(key: &str) -> Option<&'static str> {
    if matches!(
        key,
        "dryRun"
            | "confirm"
            | "Detailed"
            | "detailed"
            | "Force"
            | "force"
            | "FromObject"
            | "fromObject"
            | "NoValidate"
            | "noValidate"
            | "NoRole"
            | "noRole"
            | "SetDefault"
            | "setDefault"
            | "SetMainSKD"
            | "setMainSKD"
            | "Raw"
            | "raw"
            | "WithText"
            | "withText"
            | "CreateIfMissing"
            | "createIfMissing"
            | "IsFunction"
            | "isFunction"
            | "allExtensions"
            | "checkUseModality"
            | "checkUseSynchronousCalls"
            | "configLogIntegrity"
            | "distributiveModules"
            | "emptyHandlers"
            | "externalConnection"
            | "externalConnectionServer"
            | "fullOutput"
            | "fullRebuild"
            | "handlersExistence"
            | "incorrectReferences"
            | "mobileAppClient"
            | "mobileAppServer"
            | "mobileClient"
            | "mobileClientDigiSign"
            | "server"
            | "sources"
            | "thickClientManagedApplication"
            | "thickClientOrdinaryApplication"
            | "thickClientServerManagedApplication"
            | "thickClientServerOrdinaryApplication"
            | "thinClient"
            | "unsupportedFunctional"
            | "unreferenceProcedures"
            | "usePrivilegedMode"
            | "waitForExit"
            | "webClient"
            | "includeMethods"
    ) {
        Some("boolean")
    } else if key == "query" {
        Some("string")
    } else if matches!(key, "definition" | "selector") {
        Some("object")
    } else if matches!(
        key,
        "limit"
            | "Offset"
            | "offset"
            | "MaxParams"
            | "maxParams"
            | "mcpPort"
            | "waitTimeoutMs"
            | "maxOutputTokens"
            | "maxFiles"
            | "rangeStart"
            | "rangeEnd"
            | "timeoutSeconds"
            | "tailChars"
    ) {
        Some("integer")
    } else if matches!(
        key,
        "codes"
            | "types"
            | "Fields"
            | "fields"
            | "Children"
            | "children"
            | "ids"
            | "edgeKinds"
            | "provenance"
            | "sections"
            | "features"
            | "filterTags"
            | "ignoreTags"
            | "objects"
            | "projects"
            | "rawKeys"
            | "scenarioFilters"
            | "sourceSets"
            | "operations"
    ) {
        Some("array")
    } else {
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::application::metadata::MetadataOperation;
    use crate::application::{tools, ResultContract, ToolExecution};

    #[test]
    fn strip_schema_descriptions_removes_annotations_only() {
        let mut schema = json!({
            "type": "object",
            "description": "tool prose",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "a property literally named description"
                },
                "state": {
                    "description": "annotation",
                    "const": { "description": "draft" },
                    "enum": [{ "description": "kept" }, "plain"],
                    "default": { "description": "kept" },
                    "examples": [{ "description": "kept" }]
                }
            },
            "$defs": {
                "description": { "type": "number", "description": "annotation" }
            }
        });
        strip_schema_descriptions(&mut schema);
        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string" },
                    "state": {
                        "const": { "description": "draft" },
                        "enum": [{ "description": "kept" }, "plain"],
                        "default": { "description": "kept" },
                        "examples": [{ "description": "kept" }]
                    }
                },
                "$defs": {
                    "description": { "type": "number" }
                }
            })
        );
    }

    fn metadata_tool(operation: MetadataOperation) -> ToolSpec {
        ToolSpec {
            name: match operation {
                MetadataOperation::Info => "unica.meta.info",
                MetadataOperation::Add => "unica.meta.add",
                MetadataOperation::Edit => "unica.meta.edit",
                MetadataOperation::Remove => "unica.meta.remove",
            },
            description: "direct metadata contract test",
            execution: if matches!(operation, MetadataOperation::Info) {
                ToolExecution::Read
            } else {
                ToolExecution::Mutation
            },
            result_contract: ResultContract::Typed,
            cache_access: crate::domain::cache::CacheAccess::default(),
            handler: ToolHandler::Metadata { operation },
        }
    }

    /// The published, host-visible operation union.
    ///
    /// ADR-0025 keeps the union kind-agnostic so it survives a host that renders
    /// `properties` alone; per-kind legality is the writer's, which answers
    /// `unsupported_kind` naming the exact field.
    fn metadata_operation_union(schema: &Value) -> &Value {
        &schema["properties"]["operations"]["items"]
    }

    fn sorted_property_names(value: &Value) -> Vec<&str> {
        let mut names = Vec::new();
        if let Some(properties) = value.get("properties").and_then(Value::as_object) {
            names.extend(properties.keys().map(String::as_str));
        } else if let Some(branches) = value.get("oneOf").and_then(Value::as_array) {
            for branch in branches {
                names.extend(sorted_property_names(branch));
            }
        } else {
            panic!("schema node must publish properties or full-object oneOf branches");
        }
        names.sort_unstable();
        names.dedup();
        names
    }

    #[test]
    fn readers_neither_publish_nor_accept_dry_run() {
        for tool in tools() {
            let schema = input_schema_for_tool(&tool);
            let publishes_dry_run = schema["properties"].get("dryRun").is_some();
            match tool.execution {
                crate::application::ToolExecution::Read => {
                    assert!(!publishes_dry_run, "{}", tool.name);
                    let args = Map::from_iter([("dryRun".to_string(), json!(true))]);
                    let error = validate_tool_arguments(tool, &args, false)
                        .expect_err("reader dryRun must fail before dispatch");
                    assert!(error.contains("dryRun"), "{}: {error}", tool.name);
                }
                crate::application::ToolExecution::Mutation => {
                    assert!(publishes_dry_run, "{}", tool.name);
                }
            }
        }
    }

    fn collect_schema_property_names<'a>(value: &'a Value, names: &mut Vec<&'a str>) {
        match value {
            Value::Object(object) => {
                if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                    names.extend(properties.keys().map(String::as_str));
                }
                for nested in object.values() {
                    collect_schema_property_names(nested, names);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect_schema_property_names(item, names);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn meta_contract_schema_snapshots_are_closed_and_registered() {
        let cases = [
            (
                MetadataOperation::Info,
                vec!["cwd", "limit", "metadataPath", "sections", "sourceSet"],
                json!(["sourceSet", "metadataPath"]),
            ),
            (
                MetadataOperation::Add,
                vec!["cwd", "dryRun", "kind", "name", "operations", "sourceSet"],
                json!(["sourceSet", "kind", "name"]),
            ),
            (
                MetadataOperation::Edit,
                vec!["cwd", "dryRun", "metadataPath", "operations", "sourceSet"],
                json!(["sourceSet", "metadataPath", "operations"]),
            ),
            (
                MetadataOperation::Remove,
                vec![
                    "confirm",
                    "cwd",
                    "dryRun",
                    "force",
                    "metadataPath",
                    "sourceSet",
                ],
                json!(["sourceSet", "metadataPath"]),
            ),
        ];

        for (operation, properties, required) in cases {
            let schema = input_schema_for_tool(&metadata_tool(operation));
            assert_eq!(schema["type"], "object");
            assert_eq!(schema["additionalProperties"], false);
            assert_eq!(sorted_property_names(&schema), properties);
            assert_eq!(schema["required"], required);
        }

        let info = input_schema_for_tool(&metadata_tool(MetadataOperation::Info));
        assert_eq!(info["properties"]["sections"]["default"], json!([]));
        assert_eq!(info["properties"]["limit"]["default"], 20);
        assert_eq!(info["properties"]["limit"]["maximum"], 50);
        assert_eq!(
            info["properties"]["sections"]["items"]["enum"],
            json!([
                "roles",
                "subscriptions",
                "functionalOptions",
                "predefinedItems"
            ])
        );

        for operation in [
            MetadataOperation::Add,
            MetadataOperation::Edit,
            MetadataOperation::Remove,
        ] {
            let schema = input_schema_for_tool(&metadata_tool(operation));
            assert_eq!(schema["properties"]["dryRun"]["default"], true);
        }

        let add = input_schema_for_tool(&metadata_tool(MetadataOperation::Add));
        assert_eq!(
            add["properties"]["kind"]["enum"],
            json!([
                "Catalog",
                "Document",
                "Enum",
                "Constant",
                "InformationRegister",
                "AccumulationRegister",
                "AccountingRegister",
                "CalculationRegister",
                "ChartOfAccounts",
                "ChartOfCharacteristicTypes",
                "ChartOfCalculationTypes",
                "BusinessProcess",
                "Task",
                "ExchangePlan",
                "DocumentJournal",
                "Report",
                "DataProcessor",
                "CommonModule",
                "ScheduledJob",
                "EventSubscription",
                "HTTPService",
                "WebService",
                "DefinedType"
            ])
        );

        let edit = input_schema_for_tool(&metadata_tool(MetadataOperation::Edit));
        assert_eq!(add["properties"]["operations"]["minItems"], 1);
        // No conditional branches: the union is the whole published contract,
        // and both mutations publish the same one.
        assert!(add.get("allOf").is_none());
        assert!(edit.get("allOf").is_none());
        assert_eq!(
            metadata_operation_union(&add),
            metadata_operation_union(&edit),
            "meta.add and meta.edit must publish one operation union"
        );
        let item = metadata_operation_union(&edit);
        let variants = item["oneOf"].as_array().expect("closed operation union");
        assert_eq!(variants.len(), 9);
        for (variant, tag, required, properties) in [
            (
                &variants[0],
                "setProperties",
                json!(["op", "values"]),
                vec!["op", "values"],
            ),
            (
                &variants[1],
                "add",
                json!(["op", "collection", "elements"]),
                vec!["collection", "elements", "op", "scope"],
            ),
            (
                &variants[2],
                "update",
                json!(["op", "collection", "elements"]),
                vec!["collection", "elements", "op", "scope"],
            ),
            (
                &variants[3],
                "remove",
                json!(["op", "collection", "names"]),
                vec!["collection", "names", "op", "scope"],
            ),
            (
                &variants[4],
                "add",
                json!(["op", "collection", "elements"]),
                vec!["collection", "elements", "op"],
            ),
            (
                &variants[5],
                "update",
                json!(["op", "collection", "elements"]),
                vec!["collection", "elements", "op"],
            ),
            (
                &variants[6],
                "remove",
                json!(["op", "collection", "ids"]),
                vec!["collection", "ids", "op"],
            ),
            // ADR-0072: embedded help is an object facet, so its create-only
            // operation lives in the same union as every other object change.
            (&variants[7], "addHelp", json!(["op"]), vec!["lang", "op"]),
            (
                &variants[8],
                "editRelations",
                json!(["op", "relation", "mode", "targets"]),
                vec!["mode", "op", "relation", "targets"],
            ),
        ] {
            assert!(
                variant.get("$ref").is_none(),
                "{tag}: a host that cannot resolve $ref must still see the variant"
            );
            assert_eq!(variant["type"], "object", "{tag}");
            assert_eq!(variant["additionalProperties"], false, "{tag}");
            assert_eq!(variant["required"], required, "{tag}");
            assert_eq!(sorted_property_names(variant), properties, "{tag}");
            assert_eq!(variant["properties"]["op"]["enum"], json!([tag]));
        }
        // The union publishes the closed domains themselves: every collection
        // and relation the writer knows, not the subset one kind allows.
        let mut add_collections = variants[1]["properties"]["collection"]["enum"]
            .as_array()
            .expect("the add branch publishes a closed collection domain")
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>();
        add_collections.sort_unstable();
        assert_eq!(
            add_collections,
            vec![
                "attributes",
                "columns",
                "commands",
                "dimensions",
                "enumValues",
                "forms",
                "resources",
                "tabularSections",
                "templates",
            ]
        );
        for variant in &variants[4..=6] {
            assert_eq!(
                variant["properties"]["collection"]["enum"],
                json!(["predefinedItems"])
            );
        }
        let mut relations = variants[8]["properties"]["relation"]["enum"]
            .as_array()
            .expect("the relation branch publishes a closed relation domain")
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>();
        relations.sort_unstable();
        assert_eq!(
            relations,
            vec![
                "basedOn",
                "inputByString",
                "owners",
                "registerRecords",
                "source",
            ]
        );
        assert_eq!(
            variants[8]["properties"]["mode"]["enum"],
            json!(["add", "remove", "replace"])
        );
        let targets = &variants[8]["properties"]["targets"];
        assert_eq!(targets["uniqueItems"], true);
        assert!(
            targets.get("minItems").is_none(),
            "relation-specific branches own target cardinality"
        );
        let relation_variants = variants[8]["oneOf"]
            .as_array()
            .expect("editRelations publishes a relation-correlated oneOf");
        assert_eq!(relation_variants.len(), 5);
        let relation = |name: &str| {
            relation_variants
                .iter()
                .find(|branch| branch["properties"]["relation"]["const"] == name)
                .unwrap_or_else(|| panic!("missing relation branch {name}"))
        };
        let source = relation("source");
        assert_eq!(source["properties"]["mode"]["enum"], json!(["replace"]));
        let source_targets = &source["properties"]["targets"];
        assert_eq!(source_targets["type"], "array");
        assert_eq!(source_targets["minItems"], 1);
        let target_variants = source_targets["items"]["oneOf"]
            .as_array()
            .expect("source targets publish a closed logical oneOf");
        assert_eq!(target_variants.len(), 6);
        for target in target_variants {
            assert_eq!(target["type"], "object");
            assert_eq!(target["additionalProperties"], false);
        }
        let event_kinds = target_variants
            .iter()
            .map(|target| {
                target["properties"]["kind"]["const"]
                    .as_str()
                    .expect("event target kind is a const")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            event_kinds,
            [
                "object",
                "manager",
                "manager",
                "recordSet",
                "definedType",
                "family",
            ]
        );
        assert!(target_variants.iter().any(|target| {
            target["properties"]["kind"]["const"] == "manager"
                && target["required"] == json!(["kind", "metadataPath", "sourceClass"])
                && target["properties"]["sourceClass"]["enum"]
                    == json!(["constantManager", "constantValueManager"])
        }));
        for name in ["owners", "registerRecords", "basedOn", "inputByString"] {
            let branch = relation(name);
            assert_eq!(
                branch["properties"]["mode"]["enum"],
                json!(["add", "remove", "replace"])
            );
            assert_eq!(branch["properties"]["targets"]["minItems"], 1);
        }
        assert_eq!(
            relation("owners")["properties"]["targets"]["items"]["required"],
            json!(["metadataPath"])
        );
        assert_eq!(
            relation("inputByString")["properties"]["targets"]["items"]["required"],
            json!(["fieldPath"])
        );
        assert!(variants[8]["description"]
            .as_str()
            .is_some_and(|description| description.contains("replace-only")));

        let schemas = [
            info,
            add,
            edit,
            input_schema_for_tool(&metadata_tool(MetadataOperation::Remove)),
        ];
        let mut published_property_names = Vec::new();
        for schema in &schemas {
            collect_schema_property_names(schema, &mut published_property_names);
        }
        for retired in [
            "JsonPath",
            "DefinitionFile",
            "Operation",
            "Value",
            "ObjectPath",
            "ConfigDir",
            "Object",
            "jsonPath",
            "definitionFile",
            "generatedType",
            "operation",
            "objectPath",
            "configDir",
            "object",
            "Path",
            "path",
        ] {
            assert!(!published_property_names.contains(&retired), "{retired}");
        }

        let published = tools()
            .into_iter()
            .filter(|tool| tool.name.starts_with("unica.meta."))
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(
            published,
            vec![
                "unica.meta.info",
                "unica.meta.add",
                "unica.meta.edit",
                "unica.meta.remove",
            ]
        );
    }

    #[test]
    fn every_published_argument_is_described() {
        let mut undescribed: Vec<String> = Vec::new();
        for tool in tools() {
            let schema = input_schema_for_tool(&tool);
            let property_maps =
                if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                    vec![properties]
                } else {
                    schema["oneOf"]
                        .as_array()
                        .expect("schema must publish properties or oneOf")
                        .iter()
                        .map(|branch| branch["properties"].as_object().unwrap())
                        .collect::<Vec<_>>()
                };
            for properties in property_maps {
                for (name, property) in properties {
                    let described = property
                        .get("description")
                        .and_then(Value::as_str)
                        .is_some_and(|text| text.trim().len() >= 15);
                    if !described {
                        undescribed.push(format!("{}:{name}", tool.name));
                    }
                }
            }
        }

        // A model inspects the schema before it reaches the skills, so an
        // argument without a description has to be guessed at the call site.
        assert!(
            undescribed.is_empty(),
            "arguments published without a description: {undescribed:?}"
        );
    }

    #[test]
    fn every_public_tool_lives_in_the_unica_namespace() {
        let offenders = tools()
            .into_iter()
            .map(|tool| tool.name)
            .filter(|name| !name.starts_with("unica."))
            .collect::<Vec<_>>();

        assert!(
            offenders.is_empty(),
            "foreign public tool names: {offenders:?}"
        );
    }

    #[test]
    fn argument_descriptions_cover_both_spellings_once() {
        let mut names: Vec<&str> = ARG_DESCRIPTIONS.iter().map(|(name, _)| *name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), count, "ARG_DESCRIPTIONS has duplicate keys");
        // Keys are the camelCase spelling; the lookup folds the first character
        // so one entry serves the PascalCase alias too.
        let pascal: Vec<&str> = ARG_DESCRIPTIONS
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| name.starts_with(char::is_uppercase))
            .collect();
        assert!(pascal.is_empty(), "keys must be camelCase: {pascal:?}");
    }

    #[test]
    fn output_path_description_excludes_read_only_mxl_decompile() {
        let (_, description) = ARG_DESCRIPTIONS
            .iter()
            .find(|(name, _)| *name == "outputPath")
            .expect("outputPath must have a shared description");

        assert!(description.contains("mxl.compile"));
        assert!(!description.contains("mxl.decompile"));
    }

    #[test]
    fn full_rebuild_description_distinguishes_explicit_full_from_normal_strategy() {
        let (_, description) = ARG_DESCRIPTIONS
            .iter()
            .find(|(name, _)| *name == "fullRebuild")
            .expect("fullRebuild must have a shared description");

        assert!(description.contains("runner's normal build strategy"));
        assert!(description.contains("disables the automatic full fallback"));
        assert!(!description.contains("instead of the incremental one"));
    }

    #[test]
    fn config_description_excludes_tools_that_stopped_accepting_it() {
        let (_, description) = ARG_DESCRIPTIONS
            .iter()
            .find(|(name, _)| *name == "config")
            .expect("config must have a shared description");

        assert!(!CODE_SEARCH_ARGS.contains(&"config"));
        assert!(!description.contains("unica.code.search"), "{description}");
        assert!(
            !description.contains("unica.code.diagnostics"),
            "{description}"
        );
    }

    /// #346. `sources` is exclusive, not additive. Pinned v8-runner 0.5.1
    /// reports `mode: artifacts` without it and writes the prebuilt
    /// `build/tools/client_mcp.cfe`; with it the runner reports `mode: sources`
    /// and writes an EDT tree under `build/tools/onec-client-mcp-devkit/exts/`
    /// and no `.cfe` at all. The description said the flag "also" downloads
    /// sources, which reads as artifact plus sources, so a caller who wanted
    /// the ready extension asked for the EDT path, took a `1cedtcli` dependency
    /// it never announced, and still lacked the artifact that
    /// `tools.client_mcp.extension.artifact.path` and the `build` preflight
    /// require.
    #[test]
    fn sources_description_says_it_replaces_the_prebuilt_artifact() {
        let (_, description) = ARG_DESCRIPTIONS
            .iter()
            .find(|(name, _)| *name == "sources")
            .expect("sources must have a shared description");

        assert!(
            !description.contains("also"),
            "`also` reads as artifact plus sources, but the flag replaces one with the other: {description}"
        );
        assert!(
            description.contains("instead of"),
            "the description has to say the source tree replaces the release artifact: {description}"
        );
        assert!(
            description.contains("1cedtcli"),
            "the source tree is EDT and still has to be built, so the description names that cost: {description}"
        );
        assert!(
            description.contains("omit"),
            "a caller who wants the prebuilt artifact needs to be told to leave the flag off: {description}"
        );
        // One description serves both tools, and their source routes differ:
        // yaxunit lays down the tests source-set, with no EDT tree and no
        // 1cedtcli anywhere in it. Naming that keeps the client-mcp cost from
        // reading as the price of the flag itself.
        assert!(
            description.contains("source-set"),
            "the yaxunit source route is not EDT, so the description says what it yields instead of letting the client-mcp cost stand for both: {description}"
        );
    }

    #[test]
    fn described_arguments_are_still_reachable() {
        let mut published = std::collections::BTreeSet::new();
        for tool in tools() {
            let schema = input_schema_for_tool(&tool);
            let mut names = Vec::new();
            collect_schema_property_names(&schema, &mut names);
            published.extend(names.into_iter().map(|name| {
                let mut chars = name.chars();
                match chars.next() {
                    Some(first) => first.to_lowercase().chain(chars).collect(),
                    None => String::new(),
                }
            }));
        }
        let stale: Vec<&str> = ARG_DESCRIPTIONS
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| !published.contains(*name))
            .collect();

        // Keeps the table from accumulating entries for arguments that were
        // removed from the tool surface.
        assert!(
            stale.is_empty(),
            "described arguments no longer exist: {stale:?}"
        );
    }

    /// #318. `v8-runner make` requires `--output`, and cfe export additionally
    /// requires `--extension`. The internal mapper already forwards both, but
    /// the published schema carried neither and is closed, so no MCP caller
    /// could ever pass them and the tool could not do its one job.
    #[test]
    fn build_make_publishes_the_arguments_its_command_requires() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.build.make")
            .unwrap();

        let schema = input_schema_for_tool(&tool);
        let properties = sorted_property_names(&schema);

        assert!(properties.contains(&"output"), "{properties:?}");
        assert!(properties.contains(&"extension"), "{properties:?}");
        assert_eq!(
            schema["required"],
            json!(["output"]),
            "the CLI refuses the call without it, so the schema says so"
        );

        let mut args = Map::new();
        args.insert("output".to_string(), json!("build/Extension.cfe"));
        args.insert("extension".to_string(), json!("МоёРасширение"));
        validate_tool_arguments(tool, &args, false).expect("the documented make call is accepted");
    }

    /// #319. `make` exports an artifact out of an infobase; it does not build
    /// one from sources. The description read as the latter, so an agent
    /// reached for `make` where `build` then `make` was needed.
    #[test]
    fn build_make_description_says_the_artifact_comes_out_of_the_infobase() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.build.make")
            .unwrap();

        let description = tool.description.to_lowercase();

        assert!(description.contains("infobase"), "{}", tool.description);
        assert!(
            description.contains("export"),
            "the verb names what the command does: {}",
            tool.description
        );

        // Review of #447: a description that sends the caller to a tool that
        // does not exist is worse than the vague one it replaced. Every
        // `unica.*` name it mentions must be in the registry.
        let published = tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<std::collections::BTreeSet<_>>();
        for word in tool.description.split_whitespace() {
            let referenced = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.');
            if referenced.starts_with("unica.") {
                assert!(
                    published.contains(referenced),
                    "description names a tool that does not exist: {referenced}"
                );
            }
        }
    }

    /// #315. A published argument that no handler reads is noise in every
    /// schema that carries the shared native list, and a caller who passes it
    /// gets silence instead of a rejection. The descriptions already say which
    /// ones those are, so the check reads them rather than repeating a list
    /// that would drift.
    #[test]
    fn no_published_argument_is_described_as_unread() {
        // Review of #451: the previous oracle was built from the descriptions
        // this change removes, so after the removal it was empty and the test
        // held nothing. The names are pinned here instead — the table is the
        // contract, and it does not vanish with the descriptions.
        const REMOVED: &[&str] = &[
            "baseForm",
            "batch",
            "child",
            "children",
            "columns",
            "command",
            "commandName",
            "dataPath",
            "field",
            "fields",
            "maxParams",
            "preset",
            "type",
        ];

        let published = tools()
            .into_iter()
            .flat_map(|tool| {
                let schema = input_schema_for_tool(&tool);
                sorted_property_names(&schema)
                    .into_iter()
                    .map(move |name| (tool.name, name.to_string()))
                    .collect::<Vec<_>>()
            })
            .filter(|(_, name)| {
                let lower = format!("{}{}", name[..1].to_lowercase(), &name[1..]);
                REMOVED.contains(&lower.as_str())
            })
            .collect::<Vec<_>>();

        assert!(published.is_empty(), "{published:?}");

        // And the descriptions must not describe an argument as unread while
        // still publishing it, so a future addition cannot reintroduce the
        // class the table pins by name.
        let unread = ARG_DESCRIPTIONS
            .iter()
            .filter(|(_, description)| description.contains("no handler reads"))
            .map(|(name, _)| *name)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            unread.is_empty(),
            "an argument described as unread must not stay published: {unread:?}"
        );
    }

    /// #290, требование 1: у каждого мутатора есть объявленная стратегия
    /// предпросмотра, и она выводится из реестра, а не из растущего набора
    /// исключений по имени операции. Читатель стратегии не имеет — предпросмотр
    /// ему не адресован (ADR-0044).
    /// ADR-0073 §5: переходный список замкнут и только сокращается. Паритет
    /// выживших закреплён их собственными тестами: meta.edit —
    /// `typed_edit_preview_bytes_equal_the_applied_post_image` и квитанции,
    /// form.edit — `form_edit_preview_apply_and_no_op_validate_the_projected_form`,
    /// code.patch — `code_patch_*_preview_apply_*`, cf.init —
    /// `cf_init_preview_shares_the_apply_data_shape_and_writes_nothing`.
    #[test]
    fn preview_gated_operations_stay_a_closed_shrinking_list() {
        use crate::application::PREVIEW_GATED_OPERATIONS;

        // Точный утверждённый состав: список только сокращается, поэтому
        // добавление операции обязано провалить ревью, а не пройти проверку
        // «список отсортирован и все имена настоящие».
        assert_eq!(
            PREVIEW_GATED_OPERATIONS,
            [
                "cf-edit",
                "cfe-borrow",
                "cfe-init",
                "cfe-patch-method",
                "dcs-edit",
                "epf-init",
                "erf-init",
                "form-add",
                "form-remove",
                "interface-edit",
                "subsystem-edit",
                "support-edit",
            ],
            "ADR-0073 §5: the transitional list is approved item by item"
        );
        let mut sorted = PREVIEW_GATED_OPERATIONS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted, PREVIEW_GATED_OPERATIONS,
            "the transitional list stays sorted and unique"
        );
        assert!(
            !PREVIEW_GATED_OPERATIONS.contains(&"cf-init"),
            "cf.init previews honestly (ADR-0073)"
        );
        let mutating_operations = tools()
            .into_iter()
            .filter(|tool| tool.execution.is_mutating())
            .filter_map(|tool| match tool.handler {
                ToolHandler::NativeOperation { operation, .. } => Some(operation),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        for operation in PREVIEW_GATED_OPERATIONS {
            assert!(
                mutating_operations.contains(operation),
                "{operation}: the transitional list must not hold ghosts"
            );
        }
    }

    #[test]
    fn every_mutating_tool_declares_a_preview_strategy() {
        use crate::application::{preview_strategy, PreviewStrategy};

        let mut planned = Vec::new();
        for tool in tools() {
            let strategy = preview_strategy(&tool);
            if !tool.execution.is_mutating() {
                assert_eq!(strategy, None, "{} is a reader", tool.name);
                continue;
            }
            let strategy = strategy
                .unwrap_or_else(|| panic!("{} has no declared preview strategy", tool.name));
            if strategy == PreviewStrategy::PlannedCommand {
                planned.push(tool.name);
            }
        }

        planned.sort_unstable();
        // Табличная часть: внешнюю команду показывают ровно те мутаторы,
        // которые её и запускают. Новый такой инструмент обязан появиться
        // здесь осознанно, а не попасть в класс молча.
        assert_eq!(
            planned,
            [
                "unica.build.dump",
                "unica.build.load",
                "unica.build.make",
                "unica.build.run",
                "unica.build.update",
                "unica.runtime.execute",
                "unica.runtime.job.cancel",
                "unica.runtime.job.start",
            ]
        );
    }

    #[test]
    fn native_contracts_reject_unknown_args() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.cf.info")
            .unwrap();
        let mut args = Map::new();
        args.insert("ConfigPath".to_string(), json!("Configuration.xml"));
        args.insert("unknown".to_string(), json!("value"));

        let error = validate_tool_arguments(tool, &args, false).unwrap_err();

        assert!(error.contains("does not accept argument `unknown`"));
    }

    fn reject_argument(tool_name: &str, argument: &str) -> String {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .unwrap();
        let args = Map::from_iter([(argument.to_string(), json!("value"))]);

        validate_tool_arguments(tool, &args, false).unwrap_err()
    }

    #[test]
    fn unknown_argument_error_lists_every_published_argument() {
        // The rejection has to answer "then what does it take?" on the spot;
        // otherwise the caller has to re-read `inputSchema` from `tools/list`.
        for tool in tools() {
            let error = reject_argument(tool.name, "definitelyNotAnArgument");
            let schema = input_schema_for_tool(&tool);
            for name in sorted_property_names(&schema) {
                assert!(
                    error.contains(name),
                    "{} rejection omits accepted argument `{name}`: {error}",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn unknown_argument_error_suggests_the_argument_spelled_differently() {
        let error = reject_argument("unica.code.definition", "SourceDir");

        assert!(
            error.contains("did you mean `sourceDir`?"),
            "missing case-only suggestion: {error}"
        );
    }

    #[test]
    fn unknown_argument_error_suggests_the_nearest_argument() {
        let error = reject_argument("unica.code.definition", "moduleHnit");

        assert!(
            error.contains("did you mean `moduleHint`?"),
            "missing near-miss suggestion: {error}"
        );
    }

    #[test]
    fn unknown_argument_error_omits_a_suggestion_without_a_near_match() {
        // `query` names a real argument of `unica.code.search`, so the caller is
        // wrong about the tool rather than about the spelling. Guessing here
        // would send them to an unrelated argument.
        let error = reject_argument("unica.code.definition", "query");

        assert!(
            !error.contains("did you mean"),
            "unexpected suggestion for an unrelated name: {error}"
        );
        assert!(
            error.contains("accepted arguments: confirm, cwd, limit, moduleHint, name, sourceDir"),
            "missing accepted arguments: {error}"
        );
    }

    #[test]
    fn unknown_runtime_operation_argument_error_lists_accepted_arguments() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.execute")
            .unwrap();
        // `sourceSets` belongs to the tool but not to `build`, so only the
        // operation-scoped check rejects it.
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("sourceSets".to_string(), json!(["main"])),
        ]);

        let error = validate_tool_arguments(tool, &args, false).unwrap_err();

        assert!(
            error.contains("operation `build` does not accept `sourceSets`"),
            "unexpected rejection: {error}"
        );
        assert!(
            error.contains("did you mean `sourceSet`?"),
            "missing near-miss suggestion: {error}"
        );
        for name in [
            "confirm",
            "config",
            "cwd",
            "dryRun",
            "fullRebuild",
            "operation",
            "sourceSet",
            "workdir",
        ] {
            assert!(
                error.contains(name),
                "rejection omits accepted argument `{name}`: {error}"
            );
        }
    }

    #[test]
    fn mxl_decompile_rejects_legacy_output_path_aliases() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.mxl.decompile")
            .unwrap();

        for argument in ["OutputPath", "outputPath"] {
            let args = Map::from_iter([(argument.to_string(), json!("result.json"))]);
            let error = validate_tool_arguments(tool, &args, false).unwrap_err();

            assert!(error.contains(&format!("does not accept argument `{argument}`")));
        }
    }

    #[test]
    fn read_only_native_tools_reject_out_file_arguments() {
        let required_path = |name: &str| match name {
            "unica.cf.info" | "unica.cf.validate" => ("ConfigPath", "src"),
            "unica.cfe.validate" => ("ExtensionPath", "src"),
            "unica.interface.validate" => ("CIPath", "src/CommandInterface.xml"),
            "unica.subsystem.info" | "unica.subsystem.validate" => {
                ("SubsystemPath", "src/Subsystems/Main.xml")
            }
            "unica.dcs.info" | "unica.dcs.validate" => ("TemplatePath", "src/Template.xml"),
            "unica.role.info" | "unica.role.validate" => ("RightsPath", "src/Rights.xml"),
            _ => unreachable!("unexpected read-only tool"),
        };

        for name in [
            "unica.cf.info",
            "unica.cf.validate",
            "unica.cfe.validate",
            "unica.interface.validate",
            "unica.subsystem.info",
            "unica.subsystem.validate",
            "unica.dcs.info",
            "unica.dcs.validate",
            "unica.role.info",
            "unica.role.validate",
        ] {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == name)
                .expect("read-only tool is registered");
            let (path_key, path) = required_path(name);
            for argument in ["OutFile", "outFile"] {
                let args = Map::from_iter([
                    (path_key.to_string(), json!(path)),
                    (argument.to_string(), json!("report.txt")),
                ]);

                let error = validate_tool_arguments(tool, &args, false).unwrap_err();
                assert!(
                    error.contains(&format!("does not accept argument `{argument}`")),
                    "{name}: {error}"
                );
            }
        }
    }

    #[test]
    fn code_patch_contract_is_narrow_and_requires_one_typed_selector() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.patch")
            .unwrap();
        let mut args = Map::new();
        args.insert("sourceSet".to_string(), json!("main"));
        args.insert("metadataPath".to_string(), json!("CommonModule.X.Module"));
        args.insert("operation".to_string(), json!("insert"));
        args.insert("selector".to_string(), json!({"method": "ПриСоздании"}));
        args.insert("content".to_string(), json!("Сообщить(\"ok\");"));
        args.insert("position".to_string(), json!("after"));
        validate_tool_arguments(tool, &args, false).unwrap();

        args.insert(
            "selector".to_string(),
            json!({"method": "A", "anchor": "B"}),
        );
        assert!(validate_tool_arguments(tool, &args, false).is_err());
        args.insert("rawArgs".to_string(), json!(["--unsafe"]));
        assert!(validate_tool_arguments(tool, &args, false).is_err());

        // `insert` without a selector is complete on its own: the end of the
        // module is implied, so `position` would have nothing to be relative to.
        let tail = Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("CommonModule.X.Module")),
            ("operation".to_string(), json!("insert")),
            (
                "content".to_string(),
                json!("Procedure Run()\nEndProcedure"),
            ),
        ]);
        validate_tool_arguments(tool, &tail, false).unwrap();
        let mut tail_with_position = tail.clone();
        tail_with_position.insert("position".to_string(), json!("after"));
        assert!(validate_tool_arguments(tool, &tail_with_position, false).is_err());
        let mut selector_without_position = tail;
        selector_without_position.insert("selector".to_string(), json!({"method": "Run"}));
        assert!(validate_tool_arguments(tool, &selector_without_position, false).is_err());
    }

    #[test]
    fn xdto_contract_publishes_and_enforces_typed_arguments() {
        let info = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.info")
            .unwrap();
        let edit = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.edit")
            .unwrap();
        let info_schema = input_schema_for_tool(&info);
        let edit_schema = input_schema_for_tool(&edit);
        let info_validator = jsonschema::validator_for(&info_schema).unwrap();
        let edit_validator = jsonschema::validator_for(&edit_schema).unwrap();

        assert!(info_validator.is_valid(&json!({
            "sourceSet": "configuration",
            "metadataPath": "XDTOPackage.EnterpriseData_1_17_3"
        })));
        assert!(!info_validator.is_valid(&json!({
            "sourceSet": "configuration",
            "metadataPath": "XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"
        })));
        assert!(!info_validator.is_valid(&json!({
            "sourceSet": "configuration",
            "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
            "typeName": "Document",
            "limit": 1
        })));
        assert_eq!(info_schema["properties"]["limit"]["maximum"], 50);

        let valid_calls = [
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [
                    {"op": "addValueType", "name": "Document", "base": "xs:string"}
                ]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addObjectType", "name": "Document"}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "propertyPath": "ObjectRef.Nested",
                    "property": {"name": "Document", "type": "tns:Document", "minOccurs": 0}
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "removeType", "name": "Document"}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "removeProperty",
                    "typeName": "AnyRef",
                    "propertyPath": "ObjectRef.Nested",
                    "name": "Document"
                }]
            }),
            // ADR-0071: one call carries an ordered batch in one transaction.
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [
                    {"op": "addObjectType", "name": "Order"},
                    {"op": "addProperty", "typeName": "Order",
                     "property": {"name": "Ref", "type": "tns:Document"}}
                ]
            }),
        ];
        // ADR-0025 §4: the union lives directly in properties.operations.items;
        // the tool publishes no top-level conditional composition.
        assert!(edit_schema.get("oneOf").is_none());
        assert!(edit_schema.get("allOf").is_none());
        assert!(edit_schema.get("not").is_none());
        assert_eq!(
            edit_schema["required"],
            json!(["sourceSet", "metadataPath", "operations"])
        );
        assert_eq!(edit_schema["properties"]["dryRun"]["default"], true);
        assert_eq!(edit_schema["properties"]["operations"]["minItems"], 1);
        let variants = edit_schema["properties"]["operations"]["items"]["oneOf"]
            .as_array()
            .expect("closed operation union");
        assert_eq!(variants.len(), XDTO_EDIT_OPS.len());
        for (variant, op) in variants.iter().zip(XDTO_EDIT_OPS) {
            assert_eq!(variant["properties"]["op"]["enum"], json!([op]));
            assert_eq!(variant["additionalProperties"], false);
            assert_eq!(
                variant["required"]
                    .as_array()
                    .and_then(|required| required.first()),
                Some(&json!("op")),
                "variant `{op}` must require its tag first"
            );
        }
        for call in &valid_calls {
            assert!(edit_validator.is_valid(call), "schema rejected {call}");
            for dry_run in [true, false] {
                validate_tool_arguments(edit, call.as_object().unwrap(), dry_run)
                    .unwrap_or_else(|error| panic!("dryRun={dry_run} rejected {call}: {error}"));
            }
        }
        let unicode_ncname = json!({
            "sourceSet": "configuration",
            "metadataPath": "ПакетXDTO.Обмен",
            "operations": [{"op": "addObjectType", "name": "A·B́"}]
        });
        assert!(edit_validator.is_valid(&unicode_ncname));
        validate_tool_arguments(edit, unicode_ncname.as_object().unwrap(), true).unwrap();

        // The retired flat form fails closed with a stable code naming the
        // typed replacement (ADR-0071).
        let legacy = json!({
            "sourceSet": "configuration",
            "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
            "operation": "add-object-type",
            "name": "Document"
        });
        assert!(
            !edit_validator.is_valid(&legacy),
            "schema accepted {legacy}"
        );
        for dry_run in [true, false] {
            let error = validate_tool_arguments(edit, legacy.as_object().unwrap(), dry_run)
                .expect_err("legacy flat form must fail before dispatch");
            assert!(
                error.contains("legacy_arguments_removed") && error.contains("operations"),
                "unexpected legacy rejection: {error}"
            );
        }

        // An element failure names the exact operations[<index>] element.
        let second_element_broken = json!({
            "sourceSet": "configuration",
            "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
            "operations": [
                {"op": "addObjectType", "name": "Document"},
                {"op": "addValueType", "name": "Document"}
            ]
        });
        assert!(!edit_validator.is_valid(&second_element_broken));
        let error = validate_tool_arguments(edit, second_element_broken.as_object().unwrap(), true)
            .expect_err("missing base must fail");
        assert!(error.contains("operations[1]"), "{error}");

        let element_required: &[(&str, &[&str], &str)] = &[
            ("addValueType", &["name", "base"], "typeName"),
            ("addObjectType", &["name"], "base"),
            ("addProperty", &["typeName", "property"], "name"),
            ("removeType", &["name"], "base"),
            ("removeProperty", &["typeName", "name"], "base"),
        ];
        for (call, (op, required, extra)) in valid_calls.iter().zip(element_required) {
            let element = call["operations"][0].as_object().unwrap().clone();
            assert_eq!(&element["op"], &json!(op));
            for field in *required {
                let mut missing_element = element.clone();
                missing_element.remove(*field);
                let mut missing = call.as_object().unwrap().clone();
                missing.insert("operations".to_string(), json!([missing_element]));
                let missing = Value::Object(missing);
                assert!(
                    !edit_validator.is_valid(&missing),
                    "schema accepted {missing}"
                );
                for dry_run in [true, false] {
                    assert!(
                        validate_tool_arguments(edit, missing.as_object().unwrap(), dry_run)
                            .is_err(),
                        "runtime accepted dryRun={dry_run}: {missing}"
                    );
                }
            }
            let mut extra_element = element.clone();
            extra_element.insert(
                (*extra).to_string(),
                if *extra == "base" {
                    json!("xs:string")
                } else {
                    json!("Value")
                },
            );
            let mut incompatible = call.as_object().unwrap().clone();
            incompatible.insert("operations".to_string(), json!([extra_element]));
            let incompatible = Value::Object(incompatible);
            assert!(
                !edit_validator.is_valid(&incompatible),
                "schema accepted {incompatible}"
            );
            for dry_run in [true, false] {
                assert!(
                    validate_tool_arguments(edit, incompatible.as_object().unwrap(), dry_run)
                        .is_err(),
                    "runtime accepted dryRun={dry_run}: {incompatible}"
                );
            }
        }
        for field in ["sourceSet", "metadataPath", "operations"] {
            let mut missing = valid_calls[0].as_object().unwrap().clone();
            missing.remove(field);
            for dry_run in [true, false] {
                assert!(
                    validate_tool_arguments(edit, &missing, dry_run).is_err(),
                    "runtime accepted missing {field} with dryRun={dry_run}"
                );
            }
        }

        let invalid_calls = [
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": []
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": {"op": "addObjectType", "name": "Document"}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addObjectType"}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addObjectType", "name": "Document", "base": "xs:string"}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "property": {"name": "Document"}
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "propertyPath": "ObjectRef..Nested",
                    "property": {"name": "Document", "type": "tns:Document"}
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "property": {"name": "Document", "type": "tns:Document", "minOccurs": 2}
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addValueType", "name": "bad:name", "base": "xs:string"}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addValueType", "name": "Document", "base": "xs::string"}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addObjectType", "name": 1}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addValueType", "name": "Document", "base": 1}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": 1,
                    "property": {"name": "Document", "type": "tns:Document"}
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "removeProperty",
                    "typeName": "AnyRef",
                    "name": "Document",
                    "propertyPath": 1
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "property": {"name": "Document", "type": "tns:Document", "minOccurs": -1}
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "property": {"name": "Document", "type": "tns:Document", "minOccurs": "0"}
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "property": {"name": "Document", "type": "tns:Document", "extra": true}
                }]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "renameType", "name": "Document"}]
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": " addObjectType", "name": "Document"}]
            }),
            json!({
                "sourceSet": " configuration ",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addObjectType", "name": "Document"}]
            }),
        ];
        for call in &invalid_calls {
            assert!(!edit_validator.is_valid(call), "schema accepted {call}");
            for dry_run in [true, false] {
                assert!(
                    validate_tool_arguments(edit, call.as_object().unwrap(), dry_run).is_err(),
                    "runtime accepted dryRun={dry_run}: {call}"
                );
            }
        }

        for dry_run in [true, false] {
            assert!(validate_tool_arguments(edit, &Map::new(), dry_run).is_err());
            assert!(validate_tool_arguments(info, &Map::new(), dry_run).is_err());
        }

        for invalid in [
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "typeName": "bad:name"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "typeName": 1
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "limit": 51
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "typeName": "Document",
                "cursor": "nav1-token"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.Enterprise.Data"
            }),
        ] {
            assert!(
                !info_validator.is_valid(&invalid),
                "schema accepted {invalid}"
            );
            assert!(
                validate_tool_arguments(info, invalid.as_object().unwrap(), false).is_err(),
                "runtime accepted {invalid}"
            );
        }

        let invalid_path = json!({
            "sourceSet": "configuration",
            "metadataPath": "Package.bin"
        });
        assert!(validate_tool_arguments(info, invalid_path.as_object().unwrap(), false).is_err());
        let invalid_property = json!({
            "sourceSet": "configuration",
            "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
            "operations": [{
                "op": "addProperty",
                "typeName": "AnyRef",
                "property": {"name": "Document", "type": "Document", "upperBound": 1}
            }]
        });
        assert!(
            validate_tool_arguments(edit, invalid_property.as_object().unwrap(), false).is_err()
        );
    }

    #[test]
    fn xdto_qname_schema_and_runtime_require_an_unpadded_prefix() {
        let edit = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.edit")
            .unwrap();
        let schema = input_schema_for_tool(&edit);
        let validator = jsonschema::validator_for(&schema).unwrap();
        let value_type = |base: &str| {
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{"op": "addValueType", "name": "Document", "base": base}]
            })
        };
        let property = |type_ref: &str| {
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "property": {"name": "Document", "type": type_ref}
                }]
            })
        };

        for call in [value_type("xs:string"), property("tns:Document")] {
            assert!(validator.is_valid(&call), "schema rejected {call}");
            validate_tool_arguments(edit, call.as_object().unwrap(), true)
                .unwrap_or_else(|error| panic!("runtime rejected {call}: {error}"));
        }

        for call in [
            value_type("string"),
            value_type(" xs:string"),
            value_type("xs:string "),
            property("Document"),
            property(" tns:Document"),
            property("tns:Document "),
        ] {
            assert!(!validator.is_valid(&call), "schema accepted {call}");
            assert!(
                validate_tool_arguments(edit, call.as_object().unwrap(), true).is_err(),
                "runtime accepted {call}"
            );
        }
    }

    #[test]
    fn xdto_published_patterns_do_not_embed_astral_code_points() {
        let info = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.info")
            .unwrap();
        let edit = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.edit")
            .unwrap();
        let info_schema = input_schema_for_tool(&info);
        let edit_schema = input_schema_for_tool(&edit);
        let union = &edit_schema["properties"]["operations"]["items"]["oneOf"];
        let patterns = [
            &info_schema["properties"]["metadataPath"]["pattern"],
            &info_schema["properties"]["typeName"]["pattern"],
            &edit_schema["properties"]["metadataPath"]["pattern"],
            &union[0]["properties"]["name"]["pattern"],
            &union[0]["properties"]["base"]["pattern"],
            &union[2]["properties"]["typeName"]["pattern"],
            &union[2]["properties"]["propertyPath"]["pattern"],
            &union[2]["properties"]["property"]["properties"]["name"]["pattern"],
            &union[2]["properties"]["property"]["properties"]["type"]["pattern"],
        ];

        for pattern in patterns {
            let pattern = pattern.as_str().expect("XDTO pattern must be a string");
            assert!(
                pattern.chars().all(|character| character <= '\u{ffff}'),
                "published ECMAScript pattern embeds an astral code point: {pattern}"
            );
        }
    }

    #[test]
    fn xdto_runtime_keeps_the_full_xml_ncname_range() {
        let edit = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.edit")
            .unwrap();
        let astral_name = "\u{10000}";
        let call = json!({
            "sourceSet": "configuration",
            "metadataPath": format!("XDTOPackage.{astral_name}"),
            "operations": [{
                "op": "addProperty",
                "typeName": astral_name,
                "propertyPath": format!("{astral_name}.{astral_name}"),
                "property": {
                    "name": astral_name,
                    "type": format!("{astral_name}:{astral_name}")
                }
            }]
        });

        validate_tool_arguments(edit, call.as_object().unwrap(), true)
            .unwrap_or_else(|error| panic!("runtime rejected XML astral NCNames: {error}"));
    }

    #[test]
    fn xdto_property_path_schema_and_runtime_share_the_escape_grammar() {
        let edit = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.edit")
            .unwrap();
        let schema = input_schema_for_tool(&edit);
        let validator = jsonschema::validator_for(&schema).unwrap();
        let call = |property_path: &str| {
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operations": [{
                    "op": "addProperty",
                    "typeName": "AnyRef",
                    "propertyPath": property_path,
                    "property": {"name": "Document", "type": "tns:Document"}
                }]
            })
        };

        for property_path in [r"A\.B", r"A\.B.Child", "A.Child"] {
            let call = call(property_path);
            assert!(validator.is_valid(&call), "schema rejected {call}");
            validate_tool_arguments(edit, call.as_object().unwrap(), true)
                .unwrap_or_else(|error| panic!("runtime rejected {call}: {error}"));
        }

        for property_path in [
            "", ".A", "A.", "A..B", r"\A", r"A\B", "A\\", r"A\\.B", r"A\.B\C",
        ] {
            let call = call(property_path);
            assert!(!validator.is_valid(&call), "schema accepted {call}");
            assert!(
                validate_tool_arguments(edit, call.as_object().unwrap(), true).is_err(),
                "runtime accepted {call}"
            );
        }
    }

    #[test]
    fn xdto_cursor_schema_and_runtime_reject_all_whitespace() {
        let info = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.info")
            .unwrap();
        let schema = input_schema_for_tool(&info);
        let validator = jsonschema::validator_for(&schema).unwrap();
        let call = |cursor: &str| {
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "cursor": cursor
            })
        };

        assert_eq!(schema["properties"]["cursor"]["pattern"], r"^\S+$");
        let valid = call("nav1-token");
        assert!(validator.is_valid(&valid));
        validate_tool_arguments(info, valid.as_object().unwrap(), false).unwrap();

        for cursor in [
            " nav1-token",
            "nav1-token ",
            "nav1 token",
            "nav1\ttoken",
            "nav1\u{00a0}token",
        ] {
            let call = call(cursor);
            assert!(!validator.is_valid(&call), "schema accepted {call}");
            assert!(
                validate_tool_arguments(info, call.as_object().unwrap(), false).is_err(),
                "runtime accepted {call}"
            );
        }
    }

    #[test]
    fn code_patch_legacy_target_fields_fail_with_a_stable_migration_error() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.patch")
            .unwrap();

        for legacy in [
            json!({"path": "src/CommonModules/X/Ext/Module.bsl"}),
            json!({"sourceDir": "src"}),
            json!({
                "path": "src/CommonModules/X/Ext/Module.bsl",
                "sourceDir": "src"
            }),
        ] {
            let error =
                validate_tool_arguments(tool, legacy.as_object().unwrap(), true).unwrap_err();
            assert!(
                error.starts_with("legacy_target_removed:"),
                "{legacy}: {error}"
            );
            assert!(error.contains("sourceSet + metadataPath"), "{error}");
        }
    }

    #[test]
    fn meta_mutation_schemas_fit_the_mcp_context_budget() {
        // These schemas are sent on every tools/list response and occupy model
        // context before the caller supplies any task data. Kind correlation
        // once expanded the response to 2.4 MB, so keep each standalone schema
        // below the reviewed compact-JSON budget while retaining closed shapes.
        for operation in [MetadataOperation::Add, MetadataOperation::Edit] {
            let schema = input_schema_for_tool(&metadata_tool(operation));
            let compact_bytes = serde_json::to_vec(&schema)
                .expect("metadata input schema must serialize as compact JSON")
                .len();
            assert!(
                compact_bytes < 70_000,
                "{operation:?} input schema consumes {compact_bytes} compact JSON bytes"
            );
        }
    }

    #[test]
    fn meta_info_publishes_only_the_logical_selector_and_what_it_reads() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.meta.info")
            .expect("unica.meta.info is registered");

        let mut args = Map::new();
        args.insert("sourceSet".to_string(), json!("main"));
        args.insert("metadataPath".to_string(), json!("Catalog.Items"));
        args.insert("limit".to_string(), json!(10));
        args.insert("sections".to_string(), json!(["roles", "subscriptions"]));
        validate_tool_arguments(tool, &args, false).unwrap();

        // Physical paths and report-formatting controls belong to the retired
        // surface; the typed tool publishes only logical selectors.
        for rejected in [
            "Mode", "Name", "offset", "Detailed", "detailed", "OutFile", "outFile", "SrcDir",
        ] {
            let mut with_rejected = args.clone();
            with_rejected.insert(rejected.to_string(), json!("value"));
            let error = validate_tool_arguments(tool, &with_rejected, false).unwrap_err();
            assert!(
                error.contains(&format!("does not accept argument `{rejected}`")),
                "{rejected}: {error}"
            );
        }
    }

    #[test]
    fn bridged_readers_publish_two_mutually_exclusive_selector_branches() {
        for (name, legacy, address) in BRIDGED_SELECTORS {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == *name)
                .unwrap_or_else(|| panic!("{name} is registered"));
            let schema = input_schema_for_tool(&tool);
            let properties = schema["properties"].as_object().expect("object schema");

            assert!(
                properties.contains_key("sourceSet"),
                "{name} must publish `sourceSet`"
            );
            assert!(
                properties.contains_key(*legacy),
                "{name} must keep `{legacy}` until its own removal slice"
            );
            assert_eq!(
                properties.contains_key("metadataPath"),
                address.publishes_address(),
                "{name} publishes `metadataPath` only when it can use one"
            );

            let mut forbidden_in_legacy = address
                .required_args()
                .iter()
                .map(|argument| json!({"required": [argument]}))
                .collect::<Vec<_>>();
            if *address == LogicalAddress::Optional {
                forbidden_in_legacy.push(json!({"required": ["metadataPath"]}));
            }
            assert_eq!(
                schema["oneOf"],
                json!([
                    {
                        "required": address.required_args(),
                        "not": {"required": [legacy]}
                    },
                    {
                        "required": [legacy],
                        "not": {"anyOf": forbidden_in_legacy}
                    }
                ]),
                "{name} must publish its two selector branches as mutually exclusive"
            );
            assert_eq!(
                schema["required"],
                json!([]),
                "{name} has no unconditionally required selector"
            );
        }
    }

    #[test]
    fn bridged_readers_that_have_no_address_do_not_publish_one() {
        // `unica.cf.*` reads `Configuration.xml` at the root of a source set,
        // so no branch of its schema can honour an address. Publishing one
        // would advertise a selector the tool cannot use.
        for name in ["unica.cf.info", "unica.cf.validate"] {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == name)
                .expect("tool is registered");
            let schema = input_schema_for_tool(&tool);
            assert!(
                schema["properties"].get("metadataPath").is_none(),
                "{name} publishes an address it cannot use: {schema}"
            );
        }
    }

    /// The bridged tools whose arguments come from `NATIVE_XML_DSL_ARGS`. The
    /// rest carry narrow published lists and never saw the catch-all names.
    const CATCH_ALL_BRIDGED_READERS: &[&str] = &[
        "unica.cf.validate",
        "unica.role.validate",
        "unica.form.validate",
        "unica.dcs.validate",
        "unica.mxl.validate",
        "unica.mxl.decompile",
        "unica.subsystem.validate",
    ];

    /// Two arguments differing only in case, with different meanings, is a
    /// contract no caller can read. The bridge is what made the lowercase name
    /// meaningful, so it is the bridge's job not to advertise the inherited
    /// uppercase one beside it — while still accepting it, because it was
    /// accepted before and no handler ever read it.
    #[test]
    fn a_bridged_reader_never_publishes_two_addresses_differing_only_in_case() {
        for (name, _, address) in BRIDGED_SELECTORS {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == *name)
                .expect("tool is registered");
            let schema = input_schema_for_tool(&tool);
            let properties = schema["properties"].as_object().expect("object schema");

            assert!(
                properties.get("MetadataPath").is_none(),
                "{name} publishes `MetadataPath` beside a selector it does not drive: {schema}"
            );
            assert_eq!(
                properties.contains_key("metadataPath"),
                address.publishes_address(),
                "{name}"
            );

            // Only the tools that drew from the shared catch-all ever accepted
            // it. The six `*.info` readers have carried narrow lists since
            // ADR-0023, so refusing it there is the contract they already had.
            if !CATCH_ALL_BRIDGED_READERS.contains(name) {
                continue;
            }
            let mut args = Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("MetadataPath".to_string(), json!("Role.Sales")),
            ]);
            if *address == LogicalAddress::Required {
                args.insert("metadataPath".to_string(), json!("Role.Sales"));
            }
            validate_tool_arguments(tool, &args, false).unwrap_or_else(|error| {
                panic!("{name} must keep tolerating the inherited name: {error}")
            });
        }
    }

    /// `unica.cf.validate` drew `metadataPath` from the shared catch-all before
    /// this bridge and ignored it. Publishing it would be a lie, but refusing
    /// it would break a call that used to work — and this bridge removes
    /// nothing. So it stays accepted and unpublished.
    ///
    /// `unica.cf.info` never took it: it has carried a narrow argument list
    /// since ADR-0023, so refusing it there is the contract it already had.
    #[test]
    fn cf_validate_still_tolerates_the_address_the_catch_all_used_to_hand_it() {
        let args = Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("Catalog.Items")),
        ]);

        let validate = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.cf.validate")
            .expect("tool is registered");
        validate_tool_arguments(validate, &args, false)
            .unwrap_or_else(|error| panic!("cf.validate must keep tolerating it: {error}"));

        let info = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.cf.info")
            .expect("tool is registered");
        let error = validate_tool_arguments(info, &args, false)
            .expect_err("cf.info never accepted an address");
        assert!(
            error.contains("does not accept argument `metadataPath`"),
            "{error}"
        );
    }

    #[test]
    fn bridged_readers_refuse_two_selectors_at_once() {
        for (name, legacy, address) in BRIDGED_SELECTORS {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == *name)
                .expect("tool is registered");
            let mut args = Map::from_iter([
                (legacy.to_string(), json!("src/whatever.xml")),
                ("sourceSet".to_string(), json!("main")),
            ]);
            if *address == LogicalAddress::Required {
                args.insert("metadataPath".to_string(), json!("Role.Sales"));
            }
            let error = validate_tool_arguments(tool, &args, false)
                .expect_err("two selectors must be refused before the handler");
            assert!(error.contains("selector_conflict"), "{name}: {error}");
        }
    }

    #[test]
    fn bridged_readers_still_refuse_a_call_with_no_selector() {
        for (name, _, _) in BRIDGED_SELECTORS {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == *name)
                .expect("tool is registered");
            assert!(
                validate_tool_arguments(tool, &Map::new(), false).is_err(),
                "{name} must still refuse a call carrying no selector at all"
            );
        }
    }

    #[test]
    fn bridged_readers_accept_either_selector_on_its_own() {
        for (name, legacy, address) in BRIDGED_SELECTORS {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == *name)
                .expect("tool is registered");

            let mut logical = Map::from_iter([("sourceSet".to_string(), json!("main"))]);
            if *address == LogicalAddress::Required {
                logical.insert("metadataPath".to_string(), json!("Role.Sales"));
            }
            validate_tool_arguments(tool, &logical, false)
                .unwrap_or_else(|error| panic!("{name} logical call: {error}"));

            let physical = Map::from_iter([(legacy.to_string(), json!("src/whatever.xml"))]);
            validate_tool_arguments(tool, &physical, false)
                .unwrap_or_else(|error| panic!("{name} physical call: {error}"));
        }
    }

    /// Registry-facing falsifier for the complete public selector bridge.
    /// The component tests stay independently runnable, while this name binds
    /// one architecture record to publication, exclusivity and acceptance for
    /// every bridged reader.
    #[test]
    fn bridged_reader_selector_schema_contract_is_complete() {
        bridged_readers_publish_two_mutually_exclusive_selector_branches();
        bridged_readers_that_have_no_address_do_not_publish_one();
        bridged_readers_refuse_two_selectors_at_once();
        bridged_readers_still_refuse_a_call_with_no_selector();
        bridged_readers_accept_either_selector_on_its_own();
    }

    #[test]
    fn subject_reader_migration_inventory_is_complete() {
        let inventory = authoritative_reader_migration_inventory().collect::<Vec<_>>();
        assert_eq!(
            inventory,
            vec![
                ("unica.cf.info", ReaderMigrationMode::Bridge),
                ("unica.cf.validate", ReaderMigrationMode::Bridge),
                ("unica.subsystem.info", ReaderMigrationMode::Bridge),
                ("unica.subsystem.validate", ReaderMigrationMode::Bridge),
                ("unica.role.info", ReaderMigrationMode::Bridge),
                ("unica.role.validate", ReaderMigrationMode::Bridge),
                ("unica.form.info", ReaderMigrationMode::Bridge),
                ("unica.form.validate", ReaderMigrationMode::Bridge),
                ("unica.dcs.info", ReaderMigrationMode::Bridge),
                ("unica.dcs.validate", ReaderMigrationMode::Bridge),
                ("unica.mxl.info", ReaderMigrationMode::Bridge),
                ("unica.mxl.validate", ReaderMigrationMode::Bridge),
                ("unica.mxl.decompile", ReaderMigrationMode::Bridge),
                ("unica.code.diagnostics", ReaderMigrationMode::DirectSwitch),
            ]
        );

        let bridge_names = inventory
            .iter()
            .filter_map(|(name, mode)| (*mode == ReaderMigrationMode::Bridge).then_some(*name))
            .collect::<Vec<_>>();
        assert_eq!(
            bridge_names,
            BRIDGED_SELECTORS
                .iter()
                .map(|(name, _, _)| *name)
                .collect::<Vec<_>>()
        );
        let direct_names = inventory
            .iter()
            .filter_map(|(name, mode)| {
                (*mode == ReaderMigrationMode::DirectSwitch).then_some(*name)
            })
            .collect::<Vec<_>>();
        assert_eq!(direct_names, ["unica.code.diagnostics"]);

        for (name, mode) in inventory {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("reader migration inventory lost {name}"));
            let schema = input_schema_for_tool(&tool);
            match mode {
                ReaderMigrationMode::Bridge => {
                    let (legacy, _) = bridged_selector(name).expect("bridge selector");
                    assert!(schema["properties"].get(legacy).is_some(), "{name}");
                    assert!(schema["properties"].get("sourceSet").is_some(), "{name}");
                }
                ReaderMigrationMode::DirectSwitch => {
                    assert!(matches!(tool.handler, ToolHandler::Diagnostics), "{name}");
                    for legacy in ["sourceDir", "path", "mode"] {
                        assert!(
                            schema["properties"].get(legacy).is_none(),
                            "{name}: {legacy}"
                        );
                    }
                }
            }
        }
    }

    fn native_mutation_schema_signature(schema: &Value) -> String {
        fn write_canonical(value: &Value, output: &mut Vec<u8>) {
            match value {
                Value::Array(items) => {
                    output.push(b'[');
                    for (index, item) in items.iter().enumerate() {
                        if index != 0 {
                            output.push(b',');
                        }
                        write_canonical(item, output);
                    }
                    output.push(b']');
                }
                Value::Object(object) => {
                    output.push(b'{');
                    let mut keys = object.keys().collect::<Vec<_>>();
                    keys.sort_unstable();
                    for (index, key) in keys.into_iter().enumerate() {
                        if index != 0 {
                            output.push(b',');
                        }
                        serde_json::to_writer(&mut *output, key).expect("schema key serializes");
                        output.push(b':');
                        write_canonical(&object[key], output);
                    }
                    output.push(b'}');
                }
                _ => serde_json::to_writer(output, value).expect("schema scalar serializes"),
            }
        }

        let mut canonical = Vec::new();
        write_canonical(schema, &mut canonical);
        let mut hash = 0xcbf29ce484222325u64;
        for byte in &canonical {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{}:{hash:016x}", canonical.len())
    }

    #[test]
    fn native_mutation_schema_fingerprint_detects_nested_contract_changes() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.cf.edit")
            .expect("cf.edit is registered");
        let schema = input_schema_for_tool(&tool);
        let mut changed = schema.clone();
        changed["properties"]["dryRun"]["type"] = json!("string");

        assert_ne!(
            native_mutation_schema_signature(&schema),
            native_mutation_schema_signature(&changed),
            "a nested schema type change must alter the complete fingerprint",
        );
    }

    #[test]
    pub(crate) fn native_mutation_surface_has_exact_operations_and_schemas() {
        use std::collections::BTreeMap;

        let actual = tools()
            .into_iter()
            .filter(|tool| tool.execution.is_mutating())
            .filter(|tool| {
                matches!(
                    tool.handler,
                    ToolHandler::NativeOperation { .. } | ToolHandler::Metadata { .. }
                )
            })
            .map(|tool| {
                let operation = match tool.handler {
                    ToolHandler::NativeOperation { operation, .. } => operation.to_string(),
                    ToolHandler::Metadata { operation } => format!("metadata:{operation:?}"),
                    _ => unreachable!(),
                };
                let schema = input_schema_for_tool(&tool);
                let signature = native_mutation_schema_signature(&schema);
                (tool.name, (operation, signature))
            })
            .collect::<BTreeMap<_, _>>();
        fn entry(operation: &str, signature: &str) -> (String, String) {
            (operation.to_string(), signature.to_string())
        }
        let expected = BTreeMap::from([
            ("unica.cf.edit", entry("cf-edit", "31140:0dfa39055a94ec4a")),
            ("unica.cf.init", entry("cf-init", "32217:893638c6206fff82")),
            (
                "unica.cfe.borrow",
                entry("cfe-borrow", "31181:e0138a6de5ab4446"),
            ),
            (
                "unica.cfe.init",
                entry("cfe-init", "30676:08fdc87570145611"),
            ),
            (
                "unica.cfe.patch_method",
                entry("cfe-patch-method", "32493:c72e4d6e943f0724"),
            ),
            (
                "unica.code.patch",
                entry("code-patch", "2893:4855cf0424173695"),
            ),
            (
                "unica.dcs.compile",
                entry("dcs-compile", "32043:6a7d31e2ba3e5813"),
            ),
            (
                "unica.dcs.edit",
                entry("dcs-edit", "31150:ab08c9da4f06de92"),
            ),
            ("unica.epf.init", entry("epf-init", "1729:02a6a6ebaf86d9f6")),
            ("unica.erf.init", entry("erf-init", "1729:02a6a6ebaf86d9f6")),
            (
                "unica.form.add",
                entry("form-add", "31503:ad0297af29ca9ed3"),
            ),
            (
                "unica.form.compile",
                entry("form-compile", "31140:8c354e756d3bb2f2"),
            ),
            (
                "unica.form.edit",
                entry("form-edit", "32016:c091f5e4c8fe6835"),
            ),
            (
                "unica.form.remove",
                entry("form-remove", "32225:41144b8a4d71de9c"),
            ),
            (
                "unica.interface.edit",
                entry("interface-edit", "31248:b6f1a20a2f4a1dde"),
            ),
            (
                "unica.meta.add",
                entry("metadata:Add", "27646:331032f4d1cfefb7"),
            ),
            (
                "unica.meta.edit",
                entry("metadata:Edit", "28145:c93c0b516cc77cf1"),
            ),
            (
                "unica.meta.remove",
                entry("metadata:Remove", "1025:b8843dc27d6b5b7e"),
            ),
            (
                "unica.mxl.compile",
                entry("mxl-compile", "32111:7da4e1b775eca3c1"),
            ),
            (
                "unica.role.compile",
                entry("role-compile", "32058:21d9a4e0c9bd3f92"),
            ),
            (
                "unica.role.edit",
                entry("role-edit", "2468:5f8ba12273760906"),
            ),
            (
                "unica.subsystem.compile",
                entry("subsystem-compile", "31797:0b2d56b36dee4581"),
            ),
            (
                "unica.subsystem.edit",
                entry("subsystem-edit", "31164:52fff7efa16e1c71"),
            ),
            (
                "unica.support.edit",
                entry("support-edit", "31857:0742d10d9c5080e6"),
            ),
            (
                "unica.xdto.edit",
                entry("xdto-edit", "6126:3957bd36139d3b35"),
            ),
        ]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn logical_only_tool_schemas_match_exact_property_allowlists() {
        use std::collections::{BTreeMap, BTreeSet};

        let mut actual = BTreeMap::new();
        for tool in tools() {
            let schema = input_schema_for_tool(&tool);
            let Some(properties) = schema["properties"].as_object() else {
                continue;
            };
            if !properties.contains_key("sourceSet") || !properties.contains_key("metadataPath") {
                continue;
            }
            if bridged_selector(tool.name).is_some()
                || tool.name == "unica.code.search"
                || matches!(
                    tool.handler,
                    ToolHandler::SourceNavigation {
                        operation: SourceNavigationOperation::Locate
                    }
                )
            {
                continue;
            }
            assert_eq!(schema["additionalProperties"], false, "{}", tool.name);
            actual.insert(
                tool.name,
                properties.keys().cloned().collect::<BTreeSet<_>>(),
            );
        }
        fn fields(names: &[&str]) -> BTreeSet<String> {
            names.iter().map(|name| (*name).to_string()).collect()
        }
        let expected = BTreeMap::from([
            (
                "unica.code.diagnostics",
                fields(&[
                    "action",
                    "cwd",
                    "filter",
                    "limit",
                    "metadataPath",
                    "range",
                    "sourceSet",
                    "timeoutSeconds",
                ]),
            ),
            (
                "unica.code.patch",
                fields(&[
                    "confirm",
                    "content",
                    "cwd",
                    "dryRun",
                    "metadataPath",
                    "operation",
                    "position",
                    "selector",
                    "sourceSet",
                ]),
            ),
            (
                "unica.meta.edit",
                fields(&["cwd", "dryRun", "metadataPath", "operations", "sourceSet"]),
            ),
            (
                "unica.meta.info",
                fields(&["cwd", "limit", "metadataPath", "sections", "sourceSet"]),
            ),
            (
                "unica.meta.remove",
                fields(&[
                    "confirm",
                    "cwd",
                    "dryRun",
                    "force",
                    "metadataPath",
                    "sourceSet",
                ]),
            ),
            (
                "unica.role.edit",
                fields(&["dryRun", "metadataPath", "operations", "sourceSet"]),
            ),
            (
                "unica.source.children",
                fields(&[
                    "confirm",
                    "cursor",
                    "cwd",
                    "limit",
                    "metadataPath",
                    "sourceSet",
                ]),
            ),
            (
                "unica.source.resources",
                fields(&[
                    "confirm",
                    "cursor",
                    "cwd",
                    "limit",
                    "metadataPath",
                    "scope",
                    "snapshotId",
                    "sourceSet",
                ]),
            ),
            (
                "unica.xdto.edit",
                fields(&[
                    "confirm",
                    "cwd",
                    "dryRun",
                    "metadataPath",
                    "operations",
                    "sourceSet",
                ]),
            ),
            (
                "unica.xdto.info",
                fields(&[
                    "confirm",
                    "cursor",
                    "cwd",
                    "limit",
                    "metadataPath",
                    "sourceSet",
                    "typeName",
                ]),
            ),
        ]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn meta_info_rejects_legacy_target_fields_as_unknown_arguments() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.meta.info")
            .expect("unica.meta.info is registered");

        for legacy in ["ObjectPath", "objectPath", "Path", "path"] {
            let args = Map::from_iter([(legacy.to_string(), json!("src/Catalogs/Items.xml"))]);
            let error = validate_tool_arguments(tool, &args, false).unwrap_err();
            assert!(
                error.contains(&format!("does not accept argument `{legacy}`")),
                "{legacy}: {error}"
            );
            assert!(error.contains("sourceSet"), "{error}");
            assert!(error.contains("metadataPath"), "{error}");
        }
    }

    #[test]
    fn code_patch_json_schema_accepts_each_documented_selector_variant() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.patch")
            .unwrap();
        let schema = input_schema_for_tool(&tool);
        let validator = jsonschema::validator_for(&schema).unwrap();
        let base = json!({
            "sourceSet": "main",
            "metadataPath": "CommonModule.X.Module",
            "operation": "insert",
            "content": "Сообщить(\"ok\");",
            "position": "after"
        });

        for selector in [
            json!({"method": "ПриСоздании"}),
            json!({"anchor": "Сообщить"}),
        ] {
            let mut instance = base.clone();
            instance["selector"] = selector;
            assert!(validator.is_valid(&instance), "{instance}");
        }

        let tail = json!({
            "sourceSet": "main",
            "metadataPath": "CommonModule.X.Module",
            "operation": "insert",
            "content": "Procedure Run()\nEndProcedure"
        });
        assert!(validator.is_valid(&tail), "{tail}");
        let mut tail_with_position = tail.clone();
        tail_with_position["position"] = json!("after");
        assert!(!validator.is_valid(&tail_with_position));
        let mut selector_without_position = tail;
        selector_without_position["selector"] = json!({"method": "Run"});
        assert!(!validator.is_valid(&selector_without_position));

        let mut invalid = base;
        invalid["selector"] = json!({"method": "A", "anchor": "B"});
        assert!(!validator.is_valid(&invalid));
    }

    #[test]
    fn code_patch_metadata_path_description_is_tool_specific() {
        let tools = tools();
        let code_patch = tools
            .iter()
            .find(|tool| tool.name == "unica.code.patch")
            .unwrap();
        let role_validate = tools
            .iter()
            .find(|tool| tool.name == "unica.role.validate")
            .unwrap();

        let code_patch_schema = input_schema_for_tool(code_patch);
        let role_validate_schema = input_schema_for_tool(role_validate);
        let code_patch_description = code_patch_schema["properties"]["metadataPath"]["description"]
            .as_str()
            .unwrap();
        let role_validate_description = role_validate_schema["properties"]["metadataPath"]
            ["description"]
            .as_str()
            .unwrap();

        assert!(code_patch_description.contains("logical module address"));
        assert!(code_patch_description.contains("sourceSet"));
        assert!(!role_validate_description.contains("unica.code.patch"));
        assert!(!role_validate_description.contains("module"));
    }

    #[test]
    fn cfe_patch_method_contract_exposes_closed_bsl_argument_domains() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.cfe.patch_method")
            .unwrap();
        let schema = input_schema_for_tool(&tool);
        assert_eq!(
            schema["properties"]["Context"]["enum"],
            json!(["НаСервере", "НаКлиенте", "НаСервереБезКонтекста"])
        );
        assert_eq!(
            schema["properties"]["InterceptorType"]["enum"],
            json!(["Before", "After"])
        );
        assert_eq!(
            schema["properties"]["IsFunction"]["const"],
            json!(false),
            "v1 exposes procedure-only interception"
        );
        assert!(schema["properties"]["MethodName"]["pattern"].is_string());

        let mut args = Map::from_iter([
            ("ExtensionPath".to_string(), json!("ext")),
            ("ModulePath".to_string(), json!("CommonModule.Server")),
            ("MethodName".to_string(), json!("Run")),
            ("InterceptorType".to_string(), json!("Before")),
            ("Context".to_string(), json!("AtServer")),
        ]);
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("Context"), "{error}");
        args.insert("Context".to_string(), json!("НаСервере"));
        args.insert("MethodName".to_string(), json!("Bad-Name"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("MethodName"), "{error}");
        args.insert("MethodName".to_string(), json!("Run"));
        args.insert(
            "InterceptorType".to_string(),
            json!("ModificationAndControl"),
        );
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("InterceptorType"), "{error}");
        args.insert("InterceptorType".to_string(), json!("Before"));
        args.insert("IsFunction".to_string(), json!(true));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("parameterless procedure"), "{error}");
        assert!(error.contains("not implemented"), "{error}");
    }

    #[test]
    fn mutating_dry_run_does_not_require_payload() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.form.edit")
            .unwrap();
        let args = Map::new();

        validate_tool_arguments(tool, &args, true).unwrap();
    }

    #[test]
    fn meta_remove_does_not_publish_keep_files() {
        let remove = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.meta.remove")
            .expect("unica.meta.remove must be registered");
        let schema = input_schema_for_tool(&remove);

        assert!(schema["properties"]["KeepFiles"].is_null());
        assert!(schema["properties"]["keepFiles"].is_null());
        for spelling in ["KeepFiles", "keepFiles"] {
            let error = validate_tool_arguments(
                remove,
                json!({
                    "sourceSet": "main",
                    "metadataPath": "Catalog.Legacy",
                    spelling: true,
                })
                .as_object()
                .expect("test arguments must be an object"),
                false,
            )
            .expect_err("meta.remove must reject the retired keep-files flag");
            assert!(error.contains(&format!("does not accept argument `{spelling}`")));
        }
    }

    #[test]
    fn native_required_paths_publish_canonical_json_schema_only() {
        let cases = [
            (
                "unica.cf.info",
                json!({"ConfigPath": "src"}),
                vec![
                    json!({"configPath": "src"}),
                    json!({"Path": "src"}),
                    json!({"path": "src"}),
                ],
            ),
            (
                "unica.form.edit",
                json!({"FormPath": "Ext/Form.xml", "definition": {}}),
                vec![
                    json!({"formPath": "Ext/Form.xml", "definition": {}}),
                    json!({"Path": "Ext/Form.xml", "definition": {}}),
                    json!({"path": "Ext/Form.xml", "definition": {}}),
                ],
            ),
            (
                "unica.interface.edit",
                json!({"CIPath": "Ext/CommandInterface.xml"}),
                vec![
                    json!({"ciPath": "Ext/CommandInterface.xml"}),
                    json!({"Path": "Ext/CommandInterface.xml"}),
                    json!({"path": "Ext/CommandInterface.xml"}),
                ],
            ),
            (
                "unica.subsystem.edit",
                json!({"SubsystemPath": "Subsystems/Sales.xml"}),
                vec![
                    json!({"subsystemPath": "Subsystems/Sales.xml"}),
                    json!({"Path": "Subsystems/Sales.xml"}),
                    json!({"path": "Subsystems/Sales.xml"}),
                ],
            ),
            (
                "unica.dcs.edit",
                json!({"TemplatePath": "Ext/Template.xml"}),
                vec![
                    json!({"templatePath": "Ext/Template.xml"}),
                    json!({"Path": "Ext/Template.xml"}),
                    json!({"path": "Ext/Template.xml"}),
                ],
            ),
            (
                "unica.form.compile",
                json!({"OutputPath": "Ext/Form.xml"}),
                vec![json!({"outputPath": "Ext/Form.xml"})],
            ),
        ];

        for (tool_name, canonical, aliases) in cases {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == tool_name)
                .unwrap();
            let schema = input_schema_for_tool(&tool);
            let validator = jsonschema::validator_for(&schema).unwrap();
            assert!(
                validator.is_valid(&canonical),
                "{tool_name} schema rejected canonical path: {canonical}; schema={schema}"
            );
            for instance in aliases {
                assert!(
                    !validator.is_valid(&instance),
                    "{tool_name} schema published runtime-only path alias: {instance}; schema={schema}"
                );
            }
        }
    }

    #[test]
    fn every_native_path_alias_group_normalizes_to_one_canonical_argument() {
        for tool in tools() {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let schema = input_schema_for_tool(&tool);
            let properties = schema["properties"].as_object().unwrap();
            let mut seen = BTreeSet::new();
            for group in native_path_alias_groups(operation) {
                assert_eq!(
                    group.aliases.first().copied(),
                    Some(group.canonical),
                    "{operation} canonical alias must be first"
                );
                for alias in group.aliases {
                    assert!(
                        seen.insert(*alias),
                        "{operation} assigns path alias {alias} to more than one group"
                    );
                    if *alias == group.canonical {
                        assert!(
                            properties.contains_key(*alias),
                            "{operation} canonical path {alias} is missing from its MCP schema"
                        );
                    } else {
                        assert!(
                            !properties.contains_key(*alias),
                            "{operation} runtime-only path alias {alias} is public in its MCP schema"
                        );
                    }
                    let raw =
                        Map::from_iter([(alias.to_string(), json!(format!("{operation}/value")))]);
                    let normalized = normalize_native_path_aliases(tool, &raw).unwrap();
                    assert_eq!(
                        normalized.get(group.canonical),
                        raw.get(*alias),
                        "{operation} failed to normalize {alias} to {}",
                        group.canonical
                    );
                    for removed in group.aliases {
                        if *removed != group.canonical {
                            assert!(
                                !normalized.contains_key(*removed),
                                "{operation} retained path alias {removed}"
                            );
                        }
                    }
                }

                if group.aliases.len() > 1 {
                    let raw = Map::from_iter([
                        (group.aliases[0].to_string(), json!("first")),
                        (group.aliases[1].to_string(), json!("second")),
                    ]);
                    let error = normalize_native_path_aliases(tool, &raw).unwrap_err();
                    assert!(
                        error.contains("conflicting path aliases"),
                        "{operation}: {error}"
                    );
                }
            }
        }
    }

    #[test]
    fn form_edit_contract_accepts_inline_definition_or_json_path() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.form.edit")
            .unwrap();
        let schema = input_schema_for_tool(&tool);
        assert_eq!(schema["properties"]["definition"]["type"], "object");
        assert_eq!(schema["required"], json!(["FormPath"]));
        assert!(schema.get("allOf").is_none());
        assert_eq!(
            schema["anyOf"],
            json!([
                {"required": ["JsonPath"]},
                {"required": ["definition"]}
            ])
        );

        let validate_tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.form.validate")
            .unwrap();
        let validate_schema = input_schema_for_tool(&validate_tool);
        assert!(validate_schema["properties"].get("definition").is_none());

        let mut inline = Map::new();
        inline.insert("FormPath".to_string(), json!("Form.xml"));
        inline.insert("definition".to_string(), json!({"formEvents": []}));
        validate_tool_arguments(tool, &inline, false).unwrap();

        let mut file = Map::new();
        file.insert("FormPath".to_string(), json!("Form.xml"));
        file.insert("JsonPath".to_string(), json!("edit.json"));
        validate_tool_arguments(tool, &file, false).unwrap();

        let mut both = inline.clone();
        both.insert("JsonPath".to_string(), json!("edit.json"));
        assert!(validate_tool_arguments(tool, &both, false)
            .unwrap_err()
            .contains("conflicting aliases"));

        let mut missing_payload = Map::new();
        missing_payload.insert("FormPath".to_string(), json!("Form.xml"));
        assert!(validate_tool_arguments(tool, &missing_payload, false)
            .unwrap_err()
            .contains("exactly one"));

        let mut wrong_type = Map::new();
        wrong_type.insert("FormPath".to_string(), json!("Form.xml"));
        wrong_type.insert("definition".to_string(), json!("not-an-object"));
        assert!(validate_tool_arguments(tool, &wrong_type, false)
            .unwrap_err()
            .contains("must be object"));
    }

    #[test]
    fn form_edit_contract_rejects_unknown_sections_and_malformed_removals() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.form.edit")
            .unwrap();
        let schema = input_schema_for_tool(&tool);
        let definition = &schema["properties"]["definition"];
        assert_eq!(definition["additionalProperties"], false);
        assert_eq!(definition["properties"]["removeElements"]["type"], "array");
        assert_eq!(
            definition["properties"]["removeElements"]["items"],
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {"name": {"type": "string", "minLength": 1, "pattern": r"\S"}},
                "required": ["name"]
            })
        );

        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(!validator.is_valid(&json!({
            "FormPath": "Form.xml",
            "definition": {"removeElements": [{"name": "   "}]}
        })));

        let cases = [
            (json!({"typoSection": []}), "FORM_EDIT_UNKNOWN_SECTION"),
            (
                json!({"removeElements": [{}]}),
                "FORM_EDIT_REMOVE_ELEMENT_MISSING_NAME",
            ),
            (
                json!({"removeElements": [{"name": 42}]}),
                "FORM_EDIT_REMOVE_ELEMENT_MISSING_NAME",
            ),
            (
                json!({"removeElements": [{"name": "Target", "after": "Other"}]}),
                "FORM_EDIT_REMOVE_ELEMENT_UNKNOWN_FIELD",
            ),
            (
                json!({"removeElements": [{"name": "   "}]}),
                "FORM_EDIT_REMOVE_ELEMENT_EMPTY_NAME",
            ),
            (
                json!({"removeElements": [{"name": "Target"}, {"name": "Target"}]}),
                "FORM_EDIT_REMOVE_ELEMENT_DUPLICATE",
            ),
        ];
        for (definition, code) in cases {
            let args = Map::from_iter([
                ("FormPath".to_string(), json!("Form.xml")),
                ("definition".to_string(), definition),
            ]);
            let error = validate_tool_arguments(tool, &args, false).unwrap_err();
            assert!(error.contains(code), "{error}");
        }
    }

    #[test]
    fn support_edit_contract_exposes_typed_enums_and_rejects_invalid_payloads() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.support.edit")
            .unwrap();

        let schema = input_schema_for_tool(&tool);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["Capability"]["enum"],
            json!(["on", "off"])
        );
        assert_eq!(
            schema["properties"]["Set"]["enum"],
            json!(["editable", "off-support", "locked"])
        );
        assert!(schema["properties"].get("args").is_none());

        let mut args = Map::new();
        args.insert("Path".to_string(), json!("src"));
        args.insert("Capability".to_string(), json!(true));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("Capability"));
        assert!(error.contains("string"));

        let mut args = Map::new();
        args.insert("Path".to_string(), json!("src"));
        args.insert("Capability".to_string(), json!("on"));
        args.insert("Set".to_string(), json!("editable"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("exactly one"));

        let mut args = Map::new();
        args.insert("Path".to_string(), json!("src"));
        args.insert("Capability".to_string(), json!("on"));
        args.insert("capability".to_string(), json!("off"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("conflicting aliases"));
        assert!(error.contains("Capability"));
        assert!(error.contains("capability"));

        let mut args = Map::new();
        args.insert("Path".to_string(), json!("src"));
        args.insert("Set".to_string(), json!("editable"));
        args.insert("set".to_string(), json!("locked"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("conflicting aliases"));
        assert!(error.contains("Set"));
        assert!(error.contains("set"));

        let mut args = Map::new();
        args.insert("Path".to_string(), json!("src"));
        args.insert("TargetPath".to_string(), json!("src/Catalogs/Items.xml"));
        args.insert("Capability".to_string(), json!("on"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("conflicting aliases"));
        assert!(error.contains("Path"));
        assert!(error.contains("TargetPath"));
    }

    #[test]
    fn meta_edit_contract_accepts_only_typed_ordered_operations() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.meta.edit")
            .unwrap();
        let schema = input_schema_for_tool(&tool);
        assert!(schema["properties"].get("Operation").is_none());
        assert!(schema["properties"].get("DefinitionFile").is_none());
        let mut operation_tags = metadata_operation_union(&schema)["oneOf"]
            .as_array()
            .expect("closed operation union")
            .iter()
            .map(|variant| variant["properties"]["op"]["enum"][0].clone())
            .collect::<Vec<_>>();
        operation_tags.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
        operation_tags.dedup();
        assert_eq!(
            Value::Array(operation_tags),
            json!([
                "add",
                "addHelp",
                "editRelations",
                "remove",
                "setProperties",
                "update"
            ])
        );

        let mut args = Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!("Catalog.Items")),
            (
                "operations".to_string(),
                json!([{"op": "setProperties", "values": {"Comment": "typed"}}]),
            ),
        ]);
        validate_tool_arguments(tool, &args, false).unwrap();

        args.insert("DefinitionFile".to_string(), json!("edit.json"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("does not accept argument `DefinitionFile`"));

        args.remove("DefinitionFile");
        args.insert("operations".to_string(), json!([{"op": "add-unknown"}]));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(
            error.contains("unsupported metadata edit operation"),
            "{error}"
        );
    }

    #[test]
    fn contracts_reject_wrong_scalar_type() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.cf.edit")
            .unwrap();
        let mut args = Map::new();
        args.insert("ConfigPath".to_string(), json!("Configuration.xml"));
        args.insert("dryRun".to_string(), json!("false"));

        let error = validate_tool_arguments(tool, &args, false).unwrap_err();

        assert!(error.contains("dryRun"));
        assert!(error.contains("boolean"));
    }

    #[test]
    fn form_boolean_flags_are_boolean_in_mcp_contract() {
        let form_add = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.form.add")
            .unwrap();
        let schema = input_schema_for_tool(&form_add);
        assert_eq!(schema["properties"]["SetDefault"]["type"], "boolean");
        assert_eq!(schema["properties"]["setDefault"]["type"], "boolean");

        let mut args = Map::new();
        args.insert("ObjectPath".to_string(), json!("src/Catalogs/Goods.xml"));
        args.insert("FormName".to_string(), json!("ListForm"));
        args.insert("SetDefault".to_string(), json!("false"));
        let error = validate_tool_arguments(form_add, &args, false).unwrap_err();
        assert!(error.contains("SetDefault"));
        assert!(error.contains("boolean"));

        let mut args = Map::new();
        args.insert("ObjectPath".to_string(), json!("src/Catalogs/Goods.xml"));
        args.insert("FormName".to_string(), json!("ListForm"));
        args.insert("SetDefault".to_string(), json!(false));
        args.insert("setDefault".to_string(), json!(true));
        let error = validate_tool_arguments(form_add, &args, false).unwrap_err();
        assert!(error.contains("conflicting aliases"));
    }

    #[test]
    fn runtime_contract_rejects_unknown_operation_and_raw_args() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.execute")
            .unwrap();
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("shell"));
        args.insert("args".to_string(), json!(["--unsafe"]));

        let error = validate_tool_arguments(tool, &args, false).unwrap_err();

        assert!(error.contains("does not accept argument `args`"));

        let mut args = Map::new();
        args.insert("operation".to_string(), json!("shell"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("must be one of"));
    }

    #[test]
    fn external_artifact_init_contracts_are_typed_and_require_destination() {
        for tool_name in ["unica.epf.init", "unica.erf.init"] {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == tool_name)
                .unwrap_or_else(|| panic!("missing tool {tool_name}"));
            let schema = input_schema_for_tool(&tool);

            assert_eq!(schema["additionalProperties"], false);
            assert!(schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("Name")));
            assert!(schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("OutputDir")));
            for argument in ["Name", "Synonym", "OutputDir", "FormName", "dryRun"] {
                assert!(
                    schema["properties"].get(argument).is_some(),
                    "{tool_name} must expose {argument}"
                );
            }
            assert!(schema["properties"].get("script").is_none());
            assert!(schema["properties"].get("args").is_none());
            let actual = schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                actual,
                BTreeSet::from([
                    "FormName",
                    "Name",
                    "OutputDir",
                    "Synonym",
                    "confirm",
                    "cwd",
                    "dryRun",
                ])
            );

            let invalid = json!({"Name": "Sample", "OutputDir": 42})
                .as_object()
                .unwrap()
                .clone();
            let error = validate_tool_arguments(tool, &invalid, false).unwrap_err();
            assert!(error.contains("OutputDir"), "{error}");
            assert!(error.contains("must be string"), "{error}");

            let missing_output = json!({"Name": "Sample"}).as_object().unwrap().clone();
            let error = validate_tool_arguments(tool, &missing_output, true).unwrap_err();
            assert!(error.contains("requires `OutputDir`"), "{error}");
        }
    }

    #[test]
    fn runtime_contract_requires_operation_specific_fields_for_real_execution() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.execute")
            .unwrap();
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("load"));

        validate_tool_arguments(tool, &args, true).unwrap();
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();

        assert!(error.contains("requires `path`"));
    }

    #[test]
    fn runtime_contract_rejects_operation_specific_unsupported_payloads() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.execute")
            .unwrap();
        let cases = vec![
            (
                json!({"operation": "build", "extension": "MyExtension"}),
                "operation `build` does not accept `extension`",
            ),
            (
                json!({"operation": "convert", "path": "src"}),
                "operation `convert` does not accept `path`",
            ),
            (
                json!({"operation": "test", "testRunner": "yaxunit", "fullRebuild": true}),
                "operation `test` does not accept `fullRebuild`",
            ),
            (
                json!({"operation": "load", "path": "build/config.cf", "mode": "update"}),
                "load --mode update is not supported",
            ),
            (
                json!({"operation": "load", "path": "build/config.cf", "mode": "merge"}),
                "operation `load` with mode `merge` requires `settings`",
            ),
            (
                json!({"operation": "load", "path": "build/config.cf", "settings": "merge-settings.xml"}),
                "operation `load` accepts `settings` only with mode `merge`",
            ),
            (
                json!({"operation": "dump", "mode": "partial"}),
                "operation `dump` with mode `partial` requires `object` or `objects`",
            ),
            (
                json!({"operation": "tools-download", "tool": "vanessa", "sources": true}),
                "operation `tools-download` accepts `sources` only for `yaxunit` or `client-mcp`",
            ),
        ];

        for (input, expected) in cases {
            let args = input.as_object().unwrap().clone();
            let error = validate_tool_arguments(tool, &args, false).unwrap_err();
            assert!(
                error.contains(expected),
                "expected error containing {expected:?}, got {error:?}"
            );
        }
    }

    #[test]
    fn runtime_schema_exposes_typed_arguments_without_additional_properties() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.execute")
            .unwrap();
        let schema = input_schema_for_tool(&tool);

        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("operation").is_some());
        assert!(schema["properties"].get("sourceSet").is_some());
        assert!(schema["properties"].get("args").is_none());
        assert!(schema["properties"].get("timeoutMs").is_none());
        assert_eq!(schema["properties"]["fullRebuild"]["type"], "boolean");
        assert_eq!(schema["properties"]["mcpPort"]["type"], "integer");
        assert_eq!(schema["properties"]["waitForExit"]["type"], "boolean");
        assert_eq!(schema["properties"]["waitTimeoutMs"]["type"], "integer");
        assert_eq!(schema["properties"]["waitTimeoutMs"]["minimum"], 1);
        assert_eq!(schema["properties"]["waitTimeoutMs"]["maximum"], 86_400_000);
        assert_eq!(schema["properties"]["stderrOutput"]["type"], "string");
        assert!(schema["properties"]["operation"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("build")));
        assert!(schema["properties"]["operation"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("tools-download")));
        assert!(schema["properties"]["clientMode"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("mcp-va")));
        assert!(schema["properties"]["tool"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("client-mcp")));
        assert_eq!(schema["properties"]["fullOutput"]["type"], "boolean");
        assert_eq!(schema["properties"]["objects"]["type"], "array");
        assert_eq!(schema["properties"]["sourceSets"]["type"], "array");
        assert_eq!(schema["properties"]["features"]["type"], "array");
        assert_eq!(schema["properties"]["filterTags"]["type"], "array");
        assert_eq!(schema["properties"]["ignoreTags"]["type"], "array");
        assert_eq!(schema["properties"]["scenarioFilters"]["type"], "array");
        assert_eq!(schema["properties"]["projects"]["type"], "array");
        assert_eq!(
            schema["properties"]["dryRun"]["description"],
            "Preview typed v8-runner runtime arguments; omitted or true reports the planned command without mutation, while false runs a classified operation and returns its terminal result in this call with a named risk warning; an unclassified operation stays refused."
        );
    }

    #[test]
    fn source_navigation_schemas_are_logical_exact_and_bounded() {
        let resolve = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.source.resolve")
            .expect("source.resolve is registered");
        let children = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.source.children")
            .expect("source.children is registered");

        let resolve_schema = input_schema_for_tool(&resolve);
        assert_eq!(resolve_schema["required"], json!(["sourceSet", "query"]));
        assert_eq!(
            resolve_schema["properties"]["mode"]["enum"],
            json!(["exact", "prefix"])
        );
        assert_eq!(
            resolve_schema["properties"]["targetKind"]["enum"],
            json!(["metadataObject", "module"])
        );
        assert_eq!(resolve_schema["properties"]["limit"]["minimum"], 1);
        assert_eq!(resolve_schema["properties"]["limit"]["maximum"], 50);
        for forbidden in ["path", "sourceDir", "provider", "handle"] {
            assert!(
                resolve_schema["properties"].get(forbidden).is_none(),
                "source.resolve must not publish {forbidden}"
            );
        }

        let children_schema = input_schema_for_tool(&children);
        assert_eq!(children_schema["required"], json!(["sourceSet"]));
        assert_eq!(
            children_schema["properties"]["metadataPath"]["type"],
            "string"
        );
        assert_eq!(children_schema["properties"]["cursor"]["type"], "string");
        assert_eq!(children_schema["properties"]["limit"]["minimum"], 1);
        assert_eq!(children_schema["properties"]["limit"]["maximum"], 50);
        for forbidden in ["path", "sourceDir", "provider", "handle", "collection"] {
            assert!(
                children_schema["properties"].get(forbidden).is_none(),
                "source.children must not publish {forbidden}"
            );
        }
    }

    #[test]
    fn source_resource_schemas_are_bounded_typed_and_path_free() {
        let resources = crate::application::tools()
            .into_iter()
            .find(|tool| tool.name == "unica.source.resources")
            .expect("source.resources is registered");
        let read = crate::application::tools()
            .into_iter()
            .find(|tool| tool.name == "unica.source.read")
            .expect("source.read is registered");
        // The bounded resource surface is read-only; BSL mutation lives in
        // `unica.code.patch`.
        assert!(crate::application::tools()
            .into_iter()
            .all(|tool| tool.name != "unica.source.apply"));
        let resources_schema = input_schema_for_tool(&resources);
        let read_schema = input_schema_for_tool(&read);

        assert_eq!(resources_schema["additionalProperties"], false);
        assert_eq!(
            resources_schema["properties"]["scope"]["enum"],
            json!(["self", "aggregate", "registrations"])
        );
        assert_eq!(resources_schema["properties"]["limit"]["maximum"], 50);
        assert_eq!(read_schema["additionalProperties"], false);
        assert_eq!(read_schema["required"], json!(["snapshotId", "resourceId"]));
        assert_eq!(read_schema["properties"]["offset"]["minimum"], 0);
        assert_eq!(read_schema["properties"]["limit"]["maximum"], 65_536);
        for forbidden in [
            "path",
            "sourceDir",
            "handle",
            "provider",
            "providerRevision",
            "expectedHash",
            "content",
        ] {
            for (name, schema) in [
                ("source.resources", &resources_schema),
                ("source.read", &read_schema),
            ] {
                assert!(
                    schema["properties"].get(forbidden).is_none(),
                    "{name} must not publish {forbidden}"
                );
            }
        }
    }

    #[test]
    fn source_navigation_arguments_reject_fuzzy_modes_and_unbounded_limits() {
        let resolve = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.source.resolve")
            .unwrap();
        for args in [
            json!({"sourceSet": "main", "query": "Catalog.Items", "mode": "fuzzy"}),
            json!({"sourceSet": "main", "query": "Catalog.Items", "mode": 1}),
            json!({"sourceSet": "main", "query": "Catalog.Items", "targetKind": "sourceRoot"}),
            json!({"sourceSet": "main", "query": "Catalog.Items", "targetKind": 1}),
            json!({"sourceSet": "main", "query": "Catalog.Items", "limit": -1}),
            json!({"sourceSet": "main", "query": "Catalog.Items", "limit": 0}),
            json!({"sourceSet": "main", "query": "Catalog.Items", "limit": 51}),
        ] {
            let error = validate_tool_arguments(resolve, args.as_object().unwrap(), false)
                .expect_err("invalid source navigation input must be rejected");
            assert!(
                error.contains("mode") || error.contains("limit") || error.contains("targetKind"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn runtime_job_schemas_keep_execution_typed_and_controls_narrow() {
        let runtime_jobs = tools()
            .into_iter()
            .filter(|tool| tool.name.starts_with("unica.runtime.job."))
            .map(|tool| tool.name)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            runtime_jobs,
            [
                "unica.runtime.job.cancel",
                "unica.runtime.job.list",
                "unica.runtime.job.logs",
                "unica.runtime.job.start",
                "unica.runtime.job.status",
                "unica.runtime.job.wait",
            ]
            .into_iter()
            .collect()
        );
        let job_start = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.job.start")
            .expect("runtime job start is registered");
        let job_wait = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.job.wait")
            .expect("runtime job wait is registered");
        let job_logs = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.job.logs")
            .expect("runtime job logs is registered");

        let start_schema = input_schema_for_tool(&job_start);
        assert_eq!(start_schema["additionalProperties"], false);
        assert!(start_schema["properties"].get("operation").is_some());
        assert!(start_schema["properties"].get("args").is_none());

        let wait_schema = input_schema_for_tool(&job_wait);
        assert_eq!(wait_schema["required"], json!(["jobId"]));
        assert_eq!(
            wait_schema["properties"]["timeoutSeconds"]["type"],
            "integer"
        );
        assert!(wait_schema["properties"].get("operation").is_none());

        let logs_schema = input_schema_for_tool(&job_logs);
        assert_eq!(logs_schema["required"], json!(["jobId"]));
        assert_eq!(logs_schema["properties"]["tailChars"]["type"], "integer");
        runtime_job_controls_reject_invalid_ids_bounds_and_execution_arguments();
    }

    #[test]
    fn runtime_job_start_excludes_bounded_external_epf_arguments() {
        let job_start = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.job.start")
            .expect("runtime job start is registered");
        let schema = input_schema_for_tool(&job_start);

        for name in ["waitForExit", "waitTimeoutMs", "stderrOutput"] {
            assert!(
                schema["properties"].get(name).is_none(),
                "{name} must remain exclusive to synchronous runtime.execute"
            );

            let mut args = json!({
                "operation": "launch",
                "clientMode": "thin"
            })
            .as_object()
            .unwrap()
            .clone();
            args.insert(
                name.to_string(),
                match name {
                    "waitForExit" => json!(true),
                    "waitTimeoutMs" => json!(30_000),
                    "stderrOutput" => json!("build/stderr.log"),
                    _ => unreachable!(),
                },
            );

            let error = validate_tool_arguments(job_start, &args, false)
                .expect_err("bounded execution arguments must be rejected by runtime jobs");
            assert!(error.contains(&format!("does not accept argument `{name}`")));
        }

        validate_tool_arguments(
            job_start,
            json!({
                "operation": "launch",
                "clientMode": "thin",
                "c": "StartFeaturePlayer"
            })
            .as_object()
            .unwrap(),
            false,
        )
        .expect("ordinary runtime job launch arguments must remain supported");
    }

    #[test]
    pub(crate) fn code_patch_schema_accepts_each_documented_selector_variant() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.patch")
            .expect("code patch tool is registered");
        let schema = input_schema_for_tool(&tool);
        let selector = &schema["properties"]["selector"];

        assert!(schema["properties"].get("sourceSet").is_some());
        assert!(schema["properties"].get("metadataPath").is_some());
        assert!(schema["properties"].get("path").is_none());
        assert!(schema["properties"].get("sourceDir").is_none());
        assert_eq!(selector["type"], "object");
        assert_eq!(selector["additionalProperties"], false);
        assert_eq!(selector["properties"]["method"]["type"], "string");
        assert_eq!(selector["properties"]["anchor"]["type"], "string");
        assert_eq!(selector["oneOf"].as_array().map(Vec::len), Some(2));
        for required in ["sourceSet", "metadataPath", "operation", "content"] {
            assert!(schema["required"]
                .as_array()
                .is_some_and(|items| { items.iter().any(|value| value == required) }));
        }
        // `position` belongs to a selector-bearing `insert` only, so it is
        // offered but not demanded of every call.
        assert_eq!(
            schema["properties"]["operation"]["enum"],
            serde_json::json!(["insert", "replace"])
        );
        assert!(schema["properties"]["position"].is_object());
        assert!(schema["required"]
            .as_array()
            .is_some_and(|items| { items.iter().all(|value| value != "position") }));
    }

    #[test]
    pub(crate) fn code_patch_tail_insert_public_contract_is_closed() {
        code_patch_contract_is_narrow_and_requires_one_typed_selector();
        code_patch_schema_accepts_each_documented_selector_variant();
        public_code_mutator_inventory_is_exact();

        let patch = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.patch")
            .expect("code patch is registered");
        let schema = input_schema_for_tool(&patch);
        assert_eq!(
            schema["properties"]["operation"]["enum"],
            json!(["insert", "replace"])
        );
    }

    #[test]
    fn public_code_mutator_inventory_is_exact() {
        let mut mutators = tools()
            .into_iter()
            .filter(|tool| tool.name.starts_with("unica.code."))
            .filter(|tool| tool.execution.is_mutating())
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        mutators.sort_unstable();
        assert_eq!(mutators, ["unica.code.patch"]);
    }

    #[test]
    fn runtime_job_controls_reject_invalid_ids_bounds_and_execution_arguments() {
        let wait = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.job.wait")
            .expect("runtime job wait is registered");
        let cancel = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.job.cancel")
            .expect("runtime job cancel is registered");
        let logs = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.job.logs")
            .expect("runtime job logs is registered");
        let valid_id = "00000000-0000-4000-8000-000000000001";

        assert!(validate_tool_arguments(wait, &Map::new(), false).is_err());
        assert!(validate_tool_arguments(
            wait,
            json!({"jobId":"not-a-uuid"}).as_object().unwrap(),
            false
        )
        .is_err());
        assert!(validate_tool_arguments(
            wait,
            json!({"jobId":valid_id,"timeoutSeconds":0})
                .as_object()
                .unwrap(),
            false
        )
        .is_err());
        assert!(validate_tool_arguments(
            wait,
            json!({"jobId":valid_id,"timeoutSeconds":61})
                .as_object()
                .unwrap(),
            false
        )
        .is_err());
        assert!(validate_tool_arguments(
            logs,
            json!({"jobId":valid_id,"tailChars":32769})
                .as_object()
                .unwrap(),
            false
        )
        .is_err());
        for tail_chars in [-1, 0, 32_769] {
            assert!(
                validate_tool_arguments(
                    logs,
                    json!({"jobId":valid_id,"tailChars":tail_chars})
                        .as_object()
                        .unwrap(),
                    false
                )
                .is_err(),
                "tailChars={tail_chars}"
            );
        }
        for tail_chars in [1, 32_768] {
            validate_tool_arguments(
                logs,
                json!({"jobId":valid_id,"tailChars":tail_chars})
                    .as_object()
                    .unwrap(),
                false,
            )
            .unwrap_or_else(|error| panic!("tailChars={tail_chars}: {error}"));
        }
        assert!(validate_tool_arguments(
            cancel,
            json!({"jobId":valid_id,"operation":"build"})
                .as_object()
                .unwrap(),
            true
        )
        .is_err());
    }

    #[test]
    fn code_navigation_contracts_expose_typed_arguments_without_raw_args() {
        let definition = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.definition")
            .expect("unica.code.definition must be registered");
        let outline = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.outline")
            .expect("unica.code.outline must be registered");
        let search = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.search")
            .expect("unica.code.search must be registered");

        let definition_schema = input_schema_for_tool(&definition);
        assert_eq!(definition_schema["additionalProperties"], false);
        assert!(definition_schema["properties"].get("name").is_some());
        assert!(definition_schema["properties"].get("moduleHint").is_some());
        assert!(definition_schema["properties"].get("args").is_none());
        assert_eq!(definition_schema["properties"]["limit"]["type"], "integer");
        assert_eq!(definition_schema["required"], json!(["name"]));

        let outline_schema = input_schema_for_tool(&outline);
        assert_eq!(outline_schema["additionalProperties"], false);
        assert!(outline_schema["properties"].get("path").is_some());
        assert_eq!(
            outline_schema["properties"]["includeMethods"]["type"],
            "boolean"
        );
        assert_eq!(outline_schema["required"], json!(["path"]));

        let search_schema = input_schema_for_tool(&search);
        assert_eq!(search_schema["additionalProperties"], false);
        assert!(search_schema["properties"].get("query").is_some());
        assert_eq!(search_schema["properties"]["query"]["minLength"], 1);
        assert_eq!(search_schema["properties"]["limit"]["minimum"], 1);
        assert_eq!(search_schema["properties"]["limit"]["maximum"], 50);
        assert!(search_schema["properties"].get("sourceSet").is_some());
        assert!(search_schema["properties"].get("metadataPath").is_some());
        assert!(search_schema["properties"].get("sourceDir").is_some());
        let source_dir_description = search_schema["properties"]["sourceDir"]["description"]
            .as_str()
            .expect("code.search sourceDir must describe its migration branch");
        assert!(source_dir_description.contains("migration selector"));
        assert!(source_dir_description.contains("entire resolved source root"));
        assert!(source_dir_description.contains("cannot be combined with metadataPath"));
        assert_eq!(
            search_schema["oneOf"],
            json!([
                {
                    "required": ["sourceSet"],
                    "not": {"required": ["sourceDir"]}
                },
                {
                    "required": ["sourceDir"],
                    "not": {"anyOf": [
                        {"required": ["sourceSet"]},
                        {"required": ["metadataPath"]}
                    ]}
                }
            ])
        );
        for removed in [
            "excludePath",
            "fileTypes",
            "ignoreCase",
            "mode",
            "path",
            "regex",
        ] {
            assert!(
                search_schema["properties"].get(removed).is_none(),
                "{removed} must not leak from removed unica.code.grep"
            );
        }
        assert_eq!(search_schema["required"], json!(["query"]));
    }

    #[test]
    fn code_search_rejects_blank_queries_and_out_of_range_limits() {
        let search = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.search")
            .unwrap();

        for args in [
            json!({"sourceSet":"main", "query": "   "}),
            json!({"sourceSet":"main", "query": 42}),
            json!({"sourceSet":"main", "query": null}),
            json!({"sourceSet":"main", "query": true}),
            json!({"sourceSet":"main", "query": {}}),
            json!({"sourceSet":"main", "query": "Post", "limit": 0}),
            json!({"sourceSet":"main", "query": "Post", "limit": 51}),
        ] {
            assert!(
                validate_tool_arguments(search, args.as_object().unwrap(), false).is_err(),
                "payload must be rejected: {args}"
            );
        }
        validate_tool_arguments(
            search,
            json!({"sourceSet":"main", "query": "Post", "limit": 50})
                .as_object()
                .unwrap(),
            false,
        )
        .unwrap();
    }

    #[test]
    fn code_search_requires_one_fail_closed_logical_or_migration_selector() {
        let search = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.search")
            .unwrap();

        for rejected in [
            json!({"query":"Post"}),
            json!({"query":"Post", "metadataPath":"CommonModule.X.Module"}),
            json!({"query":"Post", "sourceSet":"main", "sourceDir":"src"}),
            json!({"query":"Post", "sourceDir":"src", "metadataPath":"CommonModule.X.Module"}),
        ] {
            assert!(
                validate_tool_arguments(search, rejected.as_object().unwrap(), false).is_err(),
                "payload must fail closed: {rejected}"
            );
        }

        for accepted in [
            json!({"query":"Post", "sourceSet":"main"}),
            json!({"query":"Post", "sourceSet":"main", "metadataPath":"CommonModule.X.Module"}),
            json!({"query":"Post", "sourceDir":"src"}),
        ] {
            validate_tool_arguments(search, accepted.as_object().unwrap(), false)
                .unwrap_or_else(|error| panic!("valid selector rejected: {accepted}: {error}"));
        }
    }

    #[test]
    fn code_navigation_contracts_reject_raw_args_and_require_real_payloads() {
        let definition = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.definition")
            .unwrap();
        let mut args = Map::new();
        args.insert("args".to_string(), json!(["--unsafe"]));

        let error = validate_tool_arguments(definition, &args, false).unwrap_err();
        assert!(error.contains("does not accept argument `args`"));

        let args = Map::new();
        let error = validate_tool_arguments(definition, &args, false).unwrap_err();
        assert!(error.contains("requires `name`"));
        let error = validate_tool_arguments(definition, &args, true).unwrap_err();
        assert!(
            error.contains("requires `name`"),
            "reader validation cannot be weakened by a preview boolean: {error}"
        );
    }

    /// The typed reader answers with every section at once, so the selectors
    /// that used to trim its report select nothing. Publishing them promised a
    /// behaviour the handler no longer has (ADR-0023).
    #[test]
    fn dcs_info_contract_publishes_only_what_the_typed_reader_reads() {
        let dcs_info = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.dcs.info")
            .expect("unica.dcs.info must be registered");

        let schema = input_schema_for_tool(&dcs_info);
        assert_eq!(schema["additionalProperties"], false);
        for retired in ["Raw", "Mode", "Name", "Limit", "Offset"] {
            assert!(
                schema["properties"].get(retired).is_none(),
                "{retired} no longer selects anything: {schema}"
            );
        }
        // ADR-0049 moved the requirement into the selector branches: the path
        // is still required, just not unconditionally — a logical call carries
        // `sourceSet` + `metadataPath` instead.
        assert_eq!(schema["required"], json!([]));
        assert_eq!(
            schema["oneOf"][1]["required"],
            json!(["TemplatePath"]),
            "the physical branch still requires the path: {schema}"
        );
        assert!(schema.get("allOf").is_none());

        let mut args = Map::new();
        args.insert(
            "TemplatePath".to_string(),
            json!("Reports/Sales/Templates/Main"),
        );
        validate_tool_arguments(dcs_info, &args, false).unwrap();
    }

    /// The eight readers ADR-0023 narrowed publish exactly this set and accept
    /// exactly that one. The table is the contract: dropping a functional
    /// selector by accident fails here, publishing an unreachable one fails
    /// here, and so does letting a reader fall back to the historical catch-all.
    /// Published names are canonical (ADR-0019 collapses path aliases in
    /// `tools/list`); accepted names include the aliases validation still takes.
    /// Six of them are also ADR-0049 bridges, so their logical selector belongs
    /// to the pinned set: losing it would be as invisible as losing the path.
    #[test]
    fn every_narrowed_reader_publishes_its_exact_argument_set() {
        let cases: [(&str, &[&str], &[&str]); 8] = [
            (
                "unica.cf.info",
                &["ConfigPath", "confirm", "cwd", "sourceSet"],
                &[
                    "ConfigPath",
                    "Path",
                    "configPath",
                    "confirm",
                    "cwd",
                    "path",
                    "sourceSet",
                ],
            ),
            (
                "unica.role.info",
                &[
                    "RightsPath",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "resultRef",
                    "section",
                    "sourceSet",
                ],
                &[
                    "Path",
                    "RightsPath",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "path",
                    "resultRef",
                    "rightsPath",
                    "section",
                    "sourceSet",
                ],
            ),
            (
                "unica.subsystem.info",
                &[
                    "SubsystemPath",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "resultRef",
                    "section",
                    "sourceSet",
                ],
                &[
                    "Path",
                    "SubsystemPath",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "path",
                    "resultRef",
                    "section",
                    "sourceSet",
                    "subsystemPath",
                ],
            ),
            (
                "unica.dcs.info",
                &[
                    "TemplatePath",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "resultRef",
                    "section",
                    "sourceSet",
                ],
                &[
                    "Path",
                    "TemplatePath",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "path",
                    "resultRef",
                    "section",
                    "sourceSet",
                    "templatePath",
                ],
            ),
            (
                "unica.form.info",
                &[
                    "FormPath",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "resultRef",
                    "section",
                    "sourceSet",
                ],
                &[
                    "FormPath",
                    "Path",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "formPath",
                    "metadataPath",
                    "page",
                    "path",
                    "resultRef",
                    "section",
                    "sourceSet",
                ],
            ),
            (
                "unica.mxl.info",
                &[
                    "TemplatePath",
                    "WithText",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "resultRef",
                    "section",
                    "sourceSet",
                    "withText",
                ],
                &[
                    "Path",
                    "TemplatePath",
                    "WithText",
                    "confirm",
                    "cwd",
                    "delivery",
                    "filter",
                    "metadataPath",
                    "page",
                    "path",
                    "resultRef",
                    "section",
                    "sourceSet",
                    "templatePath",
                    "withText",
                ],
            ),
            (
                "unica.cfe.diff",
                &["ConfigPath", "ExtensionPath", "confirm", "cwd"],
                &[
                    "ConfigPath",
                    "ExtensionPath",
                    "configPath",
                    "confirm",
                    "cwd",
                    "extensionPath",
                ],
            ),
            (
                "unica.meta.info",
                &["cwd", "limit", "metadataPath", "sections", "sourceSet"],
                // The metadata surface has no path aliases and validates its
                // own closed shape, so `allowed_args` stays empty by design.
                &[],
            ),
        ];

        for (name, published, accepted) in cases {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("{name} must be registered"));
            let schema = input_schema_for_tool(&tool);
            assert_eq!(schema["additionalProperties"], false, "{name}");
            assert_eq!(sorted_property_names(&schema), published, "{name}");
            assert_eq!(allowed_args(&tool), accepted, "{name}");
        }
    }

    /// The historical catch-all is what made a narrowing invisible: every native
    /// XML tool accepted every name, so removing one from a handler changed
    /// nothing observable. No reader may reach it again.
    #[test]
    fn no_narrowed_reader_falls_back_to_the_native_catch_all() {
        for name in [
            "unica.cf.info",
            "unica.role.info",
            "unica.subsystem.info",
            "unica.dcs.info",
            "unica.form.info",
            "unica.mxl.info",
            "unica.cfe.diff",
        ] {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("{name} must be registered"));
            let published = allowed_args(&tool);
            assert!(
                published.len() < NATIVE_XML_DSL_ARGS.len(),
                "{name} publishes the catch-all argument list"
            );
            // One representative name from every family the catch-all carried.
            for foreign in ["Mode", "Limit", "Offset", "Name", "Format", "OutputDir"] {
                assert!(
                    !published.contains(&foreign),
                    "{name} accepts `{foreign}` again"
                );
            }
        }
    }

    /// `SrcDir` only ever addressed a template together with `ProcessorName`
    /// and `TemplateName`, and `TemplatePath` has been required throughout, so
    /// the composite address was never reachable. Publishing `SrcDir` alone
    /// advertised a lever that could not select anything.
    #[test]
    fn mxl_info_publishes_one_reachable_template_address() {
        let mxl_info = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.mxl.info")
            .expect("unica.mxl.info must be registered");

        let schema = input_schema_for_tool(&mxl_info);
        // ADR-0049 moved the requirement into the selector branches: the
        // physical branch still requires the path, and the logical one requires
        // the address pair instead.
        assert_eq!(schema["required"], json!([]));
        assert_eq!(
            schema["oneOf"][1]["required"],
            json!(["TemplatePath"]),
            "the physical branch still requires one reachable address: {schema}"
        );
        for unreachable in ["SrcDir", "srcDir", "ProcessorName", "TemplateName"] {
            assert!(
                schema["properties"].get(unreachable).is_none(),
                "{unreachable} cannot address a template on its own: {schema}"
            );
            let args = Map::from_iter([
                (
                    "TemplatePath".to_string(),
                    json!("Reports/Sales/Templates/Main"),
                ),
                (unreachable.to_string(), json!("src")),
            ]);
            let error = validate_tool_arguments(mxl_info, &args, false).unwrap_err();
            assert!(
                error.contains(&format!("does not accept argument `{unreachable}`")),
                "{unreachable}: {error}"
            );
        }

        // `WithText` stays: it selects cell content, not report formatting.
        let mut args = Map::new();
        args.insert(
            "TemplatePath".to_string(),
            json!("Reports/Sales/Templates/Main"),
        );
        args.insert("WithText".to_string(), json!(true));
        validate_tool_arguments(mxl_info, &args, false).unwrap();
    }

    #[test]
    fn retired_meta_tools_are_absent_from_the_contract_registry() {
        let names = tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        for retired in [
            "unica.meta.compile",
            "unica.meta.profile",
            "unica.meta.validate",
        ] {
            assert!(!names.contains(&retired), "retired {retired} is public");
        }
    }

    #[test]
    fn bsl_graph_contract_exposes_typed_arguments_without_raw_args() {
        let graph = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.graph")
            .expect("unica.code.graph must be registered");

        let schema = input_schema_for_tool(&graph);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["mode"]));
        assert!(schema["properties"].get("args").is_none());
        assert!(schema["properties"].get("argv").is_none());
        assert!(schema["properties"].get("query").is_some());
        assert_eq!(schema["properties"]["ids"]["type"], "array");
        assert_eq!(schema["properties"]["edgeKinds"]["type"], "array");
        assert_eq!(schema["properties"]["maxOutputTokens"]["type"], "integer");
        assert!(schema["properties"]["mode"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("callers")));

        let mut args = Map::new();
        args.insert("mode".to_string(), json!("callers"));
        args.insert("args".to_string(), json!(["--raw"]));
        let error = validate_tool_arguments(graph, &args, false).unwrap_err();
        assert!(error.contains("does not accept argument `args`"));

        let mut args = Map::new();
        args.insert("mode".to_string(), json!("raw"));
        let error = validate_tool_arguments(graph, &args, false).unwrap_err();
        assert!(error.contains("must be one of"));
    }

    fn diagnostic_tool() -> ToolSpec {
        tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.diagnostics")
            .expect("unica.code.diagnostics must be registered")
    }

    #[test]
    fn diagnostics_contract_is_a_strict_discriminated_action_union() {
        let schema = input_schema_for_tool(&diagnostic_tool());
        let text = serde_json::to_string(&schema).unwrap();

        for expected in [
            "\"action\"",
            "\"sourceSet\"",
            "\"analyze\"",
            "\"findings\"",
            "\"status\"",
            "\"catalog\"",
        ] {
            assert!(text.contains(expected), "schema misses {expected}: {text}");
        }
        let branches = schema["oneOf"]
            .as_array()
            .expect("diagnostics must publish one strict branch per action");
        assert_eq!(branches.len(), 4);
        for branch in branches {
            assert_eq!(branch["additionalProperties"], false);
            let properties = branch["properties"].as_object().unwrap();
            assert!(
                !properties.contains_key("providers"),
                "provider execution is routed internally: {branch}"
            );
            for removed in [
                "mode",
                "sourceDir",
                "path",
                "config",
                "format",
                "detail",
                "maxFiles",
                "rangeStart",
                "rangeEnd",
            ] {
                assert!(!properties.contains_key(removed), "{removed}");
            }
        }
    }

    #[test]
    fn diagnostics_timeout_description_uses_the_action_contract() {
        let schema = input_schema_for_tool(&diagnostic_tool());
        let description = schema["properties"]["timeoutSeconds"]["description"]
            .as_str()
            .expect("timeoutSeconds is described at the summary object");

        assert!(description.contains("action=analyze"), "{description}");
        assert!(!description.contains("mode analyze"), "{description}");
    }

    #[test]
    fn diagnostics_contract_accepts_only_fields_of_the_selected_action() {
        let schema = input_schema_for_tool(&diagnostic_tool());
        let validator = jsonschema::validator_for(&schema).unwrap();
        for valid in [
            json!({
                "action": "analyze",
                "sourceSet": "main",
                "filter": {
                    "minSeverity": "warning",
                    "codes": [{"provider": "bsl-analyzer", "code": "LineLength"}]
                },
                "limit": 200,
                "timeoutSeconds": 30
            }),
            json!({
                "action": "findings",
                "sourceSet": "main",
                "metadataPath": "CommonModule.Проверка.Module",
                "range": {
                    "startLine": 0,
                    "startColumn": 0,
                    "endLine": 1,
                    "endColumn": 0
                }
            }),
            json!({"action": "status", "sourceSet": "main"}),
            json!({
                "action": "catalog",
                "sourceSet": "main",
                "filter": {"codes": [{"provider": "bsl-analyzer", "code": "LineLength"}]}
            }),
        ] {
            assert!(validator.is_valid(&valid), "schema rejected {valid}");
        }

        for invalid in [
            json!({"action": "analyze"}),
            json!({"action": "analyze", "sourceSet": " main "}),
            json!({"action": "findings", "sourceSet": "main"}),
            json!({"action": "analyze", "sourceSet": "main", "metadataPath": "Catalog.Товары"}),
            json!({"action": "status", "sourceSet": "main", "range": {}}),
            json!({"action": "findings", "sourceSet": "main", "metadataPath": "Catalog.Товары", "timeoutSeconds": 30}),
            json!({"action": "catalog", "sourceSet": "main", "filter": {"minSeverity": "warning"}}),
            json!({"action": "status", "sourceSet": "main", "providers": ["bsl-analyzer"]}),
            json!({"action": "status", "sourceSet": "main", "providers": []}),
            json!({"action": "status", "sourceSet": "main", "providers": ["bsl-analyzer", "bsl-analyzer"]}),
            json!({"action": "analyze", "sourceSet": "main", "filter": {"codes": [
                {"provider": "bsl-analyzer", "code": "LineLength"},
                {"provider": "bsl-analyzer", "code": "LineLength"}
            ]}}),
        ] {
            assert!(!validator.is_valid(&invalid), "schema accepted {invalid}");
        }
    }

    #[test]
    fn diagnostics_legacy_target_removed_precedes_unknown_argument_validation() {
        let tool = diagnostic_tool();
        for legacy in ["mode", "sourceDir", "path"] {
            let args = Map::from_iter([
                ("action".into(), json!("analyze")),
                ("sourceSet".into(), json!("main")),
                (legacy.into(), json!("legacy")),
            ]);
            let error = validate_tool_argument_shape(tool, &args).unwrap_err();
            assert!(error.starts_with("legacy_target_removed:"), "{error}");
            assert!(error.contains("action + sourceSet"), "{error}");
        }
    }

    #[test]
    fn diagnostics_contract_semantics_reject_malformed_action_payloads() {
        let tool = diagnostic_tool();
        for valid in [
            json!({"action": "analyze", "sourceSet": "main", "limit": 1}),
            json!({
                "action": "findings",
                "sourceSet": "main",
                "metadataPath": "CommonModule.Проверка.Module",
                "range": {"startLine": 0, "startColumn": 0, "endLine": 0, "endColumn": 1}
            }),
        ] {
            validate_tool_arguments(tool, valid.as_object().unwrap(), false)
                .unwrap_or_else(|error| panic!("valid request {valid} failed: {error}"));
        }

        for invalid in [
            json!({"action": "analyze", "sourceSet": " main "}),
            json!({"action": "findings", "sourceSet": "main"}),
            json!({"action": "findings", "sourceSet": "main", "metadataPath": "Module", "timeoutSeconds": 30}),
            json!({"action": "catalog", "sourceSet": "main", "filter": {"minSeverity": "warning"}}),
            json!({"action": "status", "sourceSet": "main", "providers": []}),
            json!({"action": "findings", "sourceSet": "main", "metadataPath": "Module", "range": {
                "startLine": 1, "startColumn": 0, "endLine": 1, "endColumn": 0
            }}),
            json!({"action": "analyze", "sourceSet": "main", "filter": {"codes": [
                {"provider": "bsl-analyzer", "code": "LineLength"},
                {"provider": "bsl-analyzer", "code": "LineLength"}
            ]}}),
        ] {
            assert!(
                validate_tool_arguments(tool, invalid.as_object().unwrap(), false).is_err(),
                "semantic validator accepted {invalid}"
            );
        }
    }

    #[test]
    fn role_edit_schema_and_runtime_share_the_closed_logical_contract() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.role.edit")
            .expect("role.edit is registered");
        let schema = input_schema_for_tool(&tool);
        assert_eq!(schema["additionalProperties"], false);
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(
            properties
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["dryRun", "metadataPath", "operations", "sourceSet"])
        );
        assert_eq!(
            schema["required"],
            json!(["sourceSet", "metadataPath", "operations"])
        );
        assert!(schema["properties"]["dryRun"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("typed role edit")));
        assert_eq!(
            schema["properties"]["operations"]["items"]["additionalProperties"],
            false
        );
        let validator = jsonschema::validator_for(&schema).unwrap();
        let valid = json!({
            "sourceSet": "main",
            "metadataPath": "Role.Δοκιμή",
            "operations": [{
                "op": "setRight",
                "objectName": "Catalog.Δοκιμή",
                "right": "Delete",
                "value": false
            }]
        });
        assert!(validator.is_valid(&valid), "schema rejected {valid}");
        validate_tool_arguments(tool, valid.as_object().unwrap(), true).unwrap();
        let mut cyrillic = valid.clone();
        cyrillic["metadataPath"] = json!("Role.Демо");
        cyrillic["operations"][0]["objectName"] = json!("Catalog.Товары");
        assert!(validator.is_valid(&cyrillic), "schema rejected {cyrillic}");
        validate_tool_arguments(tool, cyrillic.as_object().unwrap(), true).unwrap();

        // Публикуемый `pattern` читает ECMA-262, где `\p{...}` без флага `u`
        // не класс символов. Наружу поэтому уходит сегментная форма соседних
        // `meta.*`-схем: она ловит форму адреса, а идентификатор 1С остаётся
        // за парсером — тем же разделением обязанностей, что и в ADR-0025.
        for (field, invalid) in [
            ("metadataPath", "Role.123"),
            ("objectName", "Catalog.123"),
            ("metadataPath", "Role.Ⅻ"),
            ("objectName", "Catalog.Ⅻ"),
        ] {
            let mut call = valid.clone();
            if field == "metadataPath" {
                call[field] = json!(invalid);
            } else {
                call["operations"][0][field] = json!(invalid);
            }
            assert!(
                validator.is_valid(&call),
                "coarse schema shape must accept `{invalid}`"
            );
            assert!(
                validate_tool_arguments(tool, call.as_object().unwrap(), true).is_err(),
                "parser must reject `{invalid}`"
            );
        }

        // Форму адреса схема по-прежнему держит сама.
        for (field, invalid) in [
            ("metadataPath", "Role"),
            ("metadataPath", "Catalog.Demo"),
            ("objectName", "Catalog"),
            ("objectName", "Catalog.Demo.Attribute"),
        ] {
            let mut call = valid.clone();
            if field == "metadataPath" {
                call[field] = json!(invalid);
            } else {
                call["operations"][0][field] = json!(invalid);
            }
            assert!(!validator.is_valid(&call), "schema accepted `{invalid}`");
            assert!(validate_tool_arguments(tool, call.as_object().unwrap(), true).is_err());
        }

        for legacy in ["RightsPath", "Path", "ObjectName", "Name", "Value"] {
            let mut call = valid.clone();
            call[legacy] = json!("legacy");
            assert!(!validator.is_valid(&call), "schema accepted `{legacy}`");
            let error = validate_tool_arguments(tool, call.as_object().unwrap(), true).unwrap_err();
            assert!(error.contains(legacy), "{error}");
        }
        let physical = json!({
            "sourceSet": "main",
            "metadataPath": "src/Roles/Demo/Ext/Rights.xml",
            "operations": valid["operations"].clone()
        });
        assert!(!validator.is_valid(&physical));
        assert!(validate_tool_arguments(tool, physical.as_object().unwrap(), true).is_err());

        let mut unknown_operation_field = valid.clone();
        unknown_operation_field["operations"][0]["Path"] = json!("legacy");
        assert!(!validator.is_valid(&unknown_operation_field));
        assert!(
            validate_tool_arguments(tool, unknown_operation_field.as_object().unwrap(), true)
                .is_err()
        );
    }
}

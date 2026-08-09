use super::operation_descriptors::{native_operation_descriptor, native_path_alias_groups};
use super::source_navigation::SOURCE_NAVIGATION_LIMIT_MAX;
use super::{
    CodeIntelligenceOperation, RuntimeJobAction, SourceNavigationOperation,
    SourceResourceOperation, ToolHandler, ToolSpec,
};
use crate::domain::form_edit::{form_edit_definition_schema, validate_form_edit_definition};
use crate::domain::source_resources::{SOURCE_READ_LIMIT_MAX, SOURCE_RESOURCE_PAGE_LIMIT_MAX};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

const COMMON_ARGS: &[&str] = &["cwd", "dryRun", "confirm"];
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
/// `subsystem.info` answers with typed data: its `Mode` picked which slice of
/// one subsystem to print, and the tree projection belongs to a separate ask.
const SUBSYSTEM_INFO_ARGS: &[&str] = &["SubsystemPath", "subsystemPath", "Path", "path"];

/// A typed reader publishes only what it reads. `dcs.info` and `form.info`
/// answer with every section at once, so `Mode`, `Raw`, `Name`, `Limit` and
/// `Expand` no longer select or trim anything (ADR-0023).
const DCS_INFO_ARGS: &[&str] = &["TemplatePath", "templatePath", "Path", "path"];
const FORM_INFO_ARGS: &[&str] = &["FormPath", "formPath", "Path", "path"];
/// `mxl.info` answers with typed data. `WithText` stays: it selects cell
/// content, the way `includeMethods` selects methods in ADR-0020. `Format`,
/// `MaxParams`, `Limit` and `Offset` only shaped a printed report.
const MXL_INFO_ARGS: &[&str] = &[
    "TemplatePath",
    "templatePath",
    "Path",
    "path",
    "SrcDir",
    "srcDir",
    "WithText",
    "withText",
];
/// `cfe.diff` answers with typed data: `Mode` chose between two views of one
/// extension, and both are now reported together.
const CFE_DIFF_ARGS: &[&str] = &["ExtensionPath", "extensionPath", "ConfigPath", "configPath"];
const XDTO_INFO_ARGS: &[&str] = &["sourceSet", "metadataPath", "typeName", "limit", "cursor"];
const XDTO_EDIT_ARGS: &[&str] = &[
    "sourceSet",
    "metadataPath",
    "operation",
    "name",
    "base",
    "typeName",
    "propertyPath",
    "property",
];
const XDTO_EDIT_OPERATIONS: &[&str] = &[
    "add-value-type",
    "add-object-type",
    "add-property",
    "remove-type",
    "remove-property",
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
    "BaseForm",
    "Batch",
    "BodyLimit",
    "BorrowMainAttribute",
    "Capability",
    "Child",
    "Children",
    "CIPath",
    "Columns",
    "Command",
    "CommandName",
    "CompatibilityMode",
    "ConfigDir",
    "ConfigPath",
    "Context",
    "CreateIfMissing",
    "DataSet",
    "DataPath",
    "DefinitionFile",
    "Detailed",
    "EmitDsl",
    "ExtensionPath",
    "Expand",
    "Field",
    "Fields",
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
    "MaxParams",
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
    "Preset",
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
    "Type",
    "Value",
    "Variant",
    "Vendor",
    "Version",
    "WithText",
    "baseForm",
    "batch",
    "bodyLimit",
    "borrowMainAttribute",
    "capability",
    "child",
    "children",
    "ciPath",
    "columns",
    "command",
    "commandName",
    "compatibilityMode",
    "configDir",
    "configPath",
    "context",
    "createIfMissing",
    "dataSet",
    "dataPath",
    "definitionFile",
    "detailed",
    "emitDsl",
    "extensionPath",
    "expand",
    "field",
    "fields",
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
    "maxParams",
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
    "preset",
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
    "type",
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

const RUNTIME_OPERATIONS: &[&str] = &[
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
const RUNTIME_SYNTAX_MODES: &[&str] = &["designer-config", "designer-modules", "edt"];

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
const CODE_SEARCH_ARGS: &[&str] = &["limit", "query", "sourceDir"];
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
const CODE_DIAGNOSTICS_ARGS: &[&str] = &[
    "codes",
    "config",
    "detail",
    "format",
    "limit",
    "maxFiles",
    "minSeverity",
    "mode",
    "path",
    "rangeEnd",
    "rangeStart",
    "sourceDir",
    "timeoutSeconds",
];
const CODE_DIAGNOSTIC_MODES: &[&str] = &["analyze", "status", "catalog", "file", "workspace"];
const CODE_DIAGNOSTIC_SEVERITIES: &[&str] = &["error", "warning", "info", "hint"];
const CODE_DIAGNOSTIC_DETAIL: &[&str] = &["concise", "detailed"];
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
const DOCUMENTATION_ARGS: &[&str] = &["language", "limit", "platformVersion", "query"];

pub fn input_schema_for_tool(tool: &ToolSpec) -> Value {
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
    if tool.name == "unica.xdto.edit" {
        schema["oneOf"] = json!([
            xdto_edit_schema_branch(
                "add-value-type",
                &["name", "base"],
                &["typeName", "propertyPath", "property"],
            ),
            xdto_edit_schema_branch(
                "add-object-type",
                &["name"],
                &["base", "typeName", "propertyPath", "property"],
            ),
            xdto_edit_schema_branch("add-property", &["typeName", "property"], &["name", "base"],),
            xdto_edit_schema_branch(
                "remove-type",
                &["name"],
                &["base", "typeName", "propertyPath", "property"],
            ),
            xdto_edit_schema_branch(
                "remove-property",
                &["typeName", "name"],
                &["base", "property"],
            ),
        ]);
    }
    schema
}

fn xdto_edit_schema_branch(operation: &str, required: &[&str], forbidden: &[&str]) -> Value {
    let mut branch_required = vec!["operation"];
    branch_required.extend_from_slice(required);
    json!({
        "properties": {"operation": {"const": operation}},
        "required": branch_required,
        "not": {
            "anyOf": forbidden
                .iter()
                .map(|name| json!({"required": [name]}))
                .collect::<Vec<_>>()
        }
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

pub fn validate_tool_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
) -> Result<(), String> {
    if let ToolHandler::Metadata { operation } = tool.handler {
        return super::metadata::parse_metadata_request(operation, args)
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
    validate_removed_target_arguments(tool, args)?;
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

    if !dry_run || is_external_init_tool(tool) {
        for required in required_args(&tool) {
            if !args.contains_key(required) {
                return Err(format!("{} requires `{required}` argument", tool.name));
            }
        }
    }

    Ok(())
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

    let operation = xdto_required_string(tool.name, args, "operation")?;
    if !XDTO_EDIT_OPERATIONS.contains(&operation) {
        return Err(format!(
            "{} argument `operation` must be one of: {}",
            tool.name,
            XDTO_EDIT_OPERATIONS.join(", ")
        ));
    }
    let (required, forbidden): (&[&str], &[&str]) = match operation {
        "add-value-type" => (&["name", "base"], &["typeName", "propertyPath", "property"]),
        "add-object-type" => (&["name"], &["base", "typeName", "propertyPath", "property"]),
        "add-property" => (&["typeName", "property"], &["name", "base"]),
        "remove-type" => (&["name"], &["base", "typeName", "propertyPath", "property"]),
        "remove-property" => (&["typeName", "name"], &["base", "property"]),
        _ => unreachable!("operation was checked against the closed set"),
    };
    for field in required {
        if !args.contains_key(*field) {
            return Err(format!(
                "{} operation `{operation}` requires `{field}` argument",
                tool.name
            ));
        }
    }
    for field in forbidden {
        if args.contains_key(*field) {
            return Err(format!(
                "{} operation `{operation}` does not accept `{field}` argument",
                tool.name
            ));
        }
    }

    for field in ["name", "typeName"] {
        if let Some(value) = xdto_optional_string(tool.name, args, field)? {
            if !is_xml_ncname(value) {
                return Err(format!(
                    "{} argument `{field}` must be an XML NCName",
                    tool.name
                ));
            }
        }
    }
    if let Some(base) = xdto_optional_string(tool.name, args, "base")? {
        if !is_xml_prefixed_qname(base) {
            return Err(format!(
                "{} argument `base` must be a prefixed XML QName without surrounding whitespace",
                tool.name
            ));
        }
    }
    if let Some(path) = xdto_optional_string(tool.name, args, "propertyPath")? {
        if !is_xdto_property_path(path) {
            return Err(format!(
                "{} argument `propertyPath` must contain dot-separated XML NCNames with literal dots escaped as `\\.`",
                tool.name
            ));
        }
    }
    if let Some(property) = args.get("property") {
        let property = property
            .as_object()
            .ok_or_else(|| format!("{} argument `property` must be object", tool.name))?;
        if property
            .keys()
            .any(|key| !matches!(key.as_str(), "name" | "type" | "minOccurs"))
        {
            return Err(format!(
                "{} argument `property` accepts only name, type, minOccurs",
                tool.name
            ));
        }
        let name = property
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| is_xml_ncname(value))
            .ok_or_else(|| {
                format!(
                    "{} argument `property.name` must be an XML NCName",
                    tool.name
                )
            })?;
        debug_assert!(!name.is_empty());
        let property_type = property
            .get("type")
            .and_then(Value::as_str)
            .filter(|value| is_xml_prefixed_qname(value))
            .ok_or_else(|| {
                format!(
                    "{} argument `property.type` must be a prefixed XML QName without surrounding whitespace",
                    tool.name
                )
            })?;
        debug_assert!(!property_type.is_empty());
        if property
            .get("minOccurs")
            .is_some_and(|value| !matches!(value.as_u64(), Some(0 | 1)))
        {
            return Err(format!(
                "{} argument `property.minOccurs` must be 0 or 1",
                tool.name
            ));
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
    dry_run: bool,
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
        }
        "unica.code.graph" => {
            validate_enum_argument(tool.name, args, "mode", CODE_GRAPH_MODES)?;
            validate_enum_argument(tool.name, args, "dir", CODE_GRAPH_DIRECTIONS)?;
            validate_enum_argument(tool.name, args, "detail", CODE_GRAPH_DETAIL)?;
        }
        "unica.code.diagnostics" => {
            validate_enum_argument(tool.name, args, "mode", CODE_DIAGNOSTIC_MODES)?;
            validate_enum_argument(tool.name, args, "minSeverity", CODE_DIAGNOSTIC_SEVERITIES)?;
            validate_enum_argument(tool.name, args, "detail", CODE_DIAGNOSTIC_DETAIL)?;
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("analyze");
            // `path` scopes a single-file read, which only mode `file` performs.
            // Every other mode dropped it silently: `analyze` then scanned the
            // whole source set although the caller had named one file.
            if mode != "file" && args.contains_key("path") {
                return Err(format!(
                    "{} mode `{mode}` does not support `path`; use mode `file` for one file",
                    tool.name
                ));
            }
            if args.contains_key("timeoutSeconds") {
                if mode != "analyze" {
                    return Err(format!(
                        "{} argument `timeoutSeconds` is only supported for mode `analyze`",
                        tool.name
                    ));
                }
                validate_integer_bound(
                    tool.name,
                    args,
                    "timeoutSeconds",
                    DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
                    DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS,
                )?;
            }
            if !dry_run && mode == "file" && !args.contains_key("path") {
                return Err(format!(
                    "{} mode `file` requires `path` argument",
                    tool.name
                ));
            }
        }
        _ => {}
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
        if COMMON_ARGS.contains(&key.as_str()) {
            continue;
        }
        if !allowed.contains(&key.as_str()) {
            let mut accepted = allowed.to_vec();
            accepted.extend_from_slice(COMMON_ARGS);
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
    match tool.handler {
        ToolHandler::Metadata { .. } => names.clear(),
        ToolHandler::NativeOperation { operation, .. } => {
            match operation {
                "code-patch" => names.extend(CODE_PATCH_ARGS),
                "xdto-info" => names.extend(XDTO_INFO_ARGS),
                "xdto-edit" => names.extend(XDTO_EDIT_ARGS),
                "cf-info" => names.extend(CF_INFO_ARGS),
                "role-info" => names.extend(ROLE_INFO_ARGS),
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
        }
        ToolHandler::BuildRuntime { .. } => names.extend(BUILD_ARGS),
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
        ToolHandler::CodeAdapter { .. } => names.extend(code_args_for(tool.name)),
        ToolHandler::StandardsAdapter { .. } => names.extend(STANDARDS_ARGS),
        ToolHandler::Documentation { .. } => names.extend(DOCUMENTATION_ARGS),
        ToolHandler::ProjectStatus | ToolHandler::ProjectMap => {}
    }
    if tool.name == "unica.mxl.decompile" {
        names.retain(|name| *name != "OutputPath" && *name != "outputPath");
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
        ToolHandler::CodeAdapter { .. } => match tool.name {
            "unica.code.graph" => vec!["mode"],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn code_args_for(tool_name: &str) -> &'static [&'static str] {
    match tool_name {
        "unica.code.search" => CODE_SEARCH_ARGS,
        "unica.code.definition" => CODE_DEFINITION_ARGS,
        "unica.code.outline" => CODE_OUTLINE_ARGS,
        "unica.code.graph" => CODE_GRAPH_ARGS,
        "unica.code.diagnostics" => CODE_DIAGNOSTICS_ARGS,
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
    if name == "waitTimeoutMs" {
        return json!({
            "type": "integer",
            "minimum": 1,
            "maximum": 86_400_000
        });
    }

    let value_type = if matches!(
        name,
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
        "base",
        "Prefixed lexical QName naming the base type of a new XDTO valueType in `unica.xdto.edit`, for example `xs:string`; an unprefixed name or surrounding whitespace is rejected.",
    ),
    (
        "allExtensions",
        "Boolean --all-extensions covering every extension in operation syntax; only for the designer-* modes, since mode edt rejects it together with extension",
    ),
    (
        "baseForm",
        "Declared string argument that no handler reads; `<BaseForm>` is an element inside a borrowed Form.xml marking extension mode, never a call argument",
    ),
    (
        "batch",
        "Declared argument that no handler reads; `unica.dcs.info` with `Mode=query` always prints every packet and narrows only by `Name`.",
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
        "property",
        "New XDTO property object for `unica.xdto.edit`: `name` must be an XML NCName and `type` a prefixed lexical QName; `minOccurs` is optional and must be 0 or 1.",
    ),
    (
        "propertyPath",
        "Property path to a nested XDTO `typeDef`: an unescaped dot separates segments and `\\.` denotes a literal dot inside one NCName, for example `A\\.B.Child`.",
    ),
    (
        "typeName",
        "Name of the XDTO valueType or objectType, or of the target objectType for a property operation.",
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
        "child",
        "Declared string argument that no handler reads; a child subsystem is added via `unica.subsystem.edit` with `operation: \"add-child\"` and the name in `value`",
    ),
    (
        "children",
        "Declared array-of-strings argument that no handler reads; nested form elements are a `children` key inside the form DSL definition, not a call argument",
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
        "Array of diagnostic codes such as \"АПК:142\" or \"LineLength\"; on standards.explain it selects diagnostics mode and outranks snippet/id/query, on code.diagnostics it filters the catalog, and standards.search ignores it.",
    ),
    (
        "columns",
        "Declared string argument that no handler reads; table columns are a `columns` key inside the form DSL definition, not a call argument",
    ),
    (
        "command",
        "Declared string argument that no handler reads; commands are described in the `commands` section of the form DSL definition instead",
    ),
    (
        "commandName",
        "Declared string argument that no handler reads; `CommandName` is an element inside Form.xml, not a call argument",
    ),
    (
        "compatibilityMode",
        "Platform compatibility mode for the generated `Configuration.xml`, e.g. `Version8_3_27`; default `Version8_3_27` in `unica.cf.init` and `Version8_3_24` in `unica.cfe.init`, which infers it from `configPath`",
    ),
    (
        "config",
        "Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config.",
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
        "Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does",
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
        "dataPath",
        "Declared string argument that no handler reads; `DataPath` is a Form.xml binding element, expressed as a `path` key in the form DSL",
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
        "How much detail to return, with a per-tool enum: names, signatures or bodies for unica.code.graph; concise or detailed for unica.code.diagnostics",
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
        "distributiveModules",
        "Boolean Designer syntax-check option (--distributive-modules) accepted only by operation syntax with a designer-* mode",
    ),
    (
        "dryRun",
        "Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution.",
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
        "field",
        "Declared string argument that no handler reads; DCS fields are added via `unica.dcs.edit` with `operation: \"add-field\"` and the shorthand in `value`",
    ),
    (
        "fields",
        "Declared array-of-strings argument that no handler reads; data-set fields are a `fields` key inside the DCS JSON definition, not a call argument",
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
        "Boolean for operation build that forces a complete rebuild instead of the incremental one; use it after branch switches, rebases, large object moves or suspect incremental state",
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
        "Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained; on `unica.documentation.search` it picks the locale of the platform help containers to read and of the signature returned with each hit, defaulting to ru, and each corpus falls back on its own to the installed locale (the English `root` container first) when the installation ships no containers in the requested one, so every section reports the locale that actually answered",
    ),
    (
        "limit",
        "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, code.graph nodes, code.diagnostics findings, standards results).",
    ),
    (
        "maxErrors",
        "Stop a validate tool after this many errors: default 30 for `unica.cf.validate`, `cfe.validate`, `form.validate` and `interface.validate`, 20 for `unica.dcs.validate` and `unica.mxl.validate`; `unica.role.validate` and `unica.subsystem.validate` accept the key but ignore it.",
    ),
    (
        "maxFiles",
        "Integer cap on how many files one unica.code.diagnostics read covers, forwarded to the analyzer as max_files",
    ),
    (
        "maxOutputTokens",
        "Integer output budget for unica.code.graph, forwarded as max_output_tokens; use it to keep a large graph answer within context",
    ),
    (
        "maxParams",
        "`unica.mxl.info` only: maximum number of parameters listed per area, default 10",
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
        "Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full|incremental|partial for dump, load|merge for load, designer-config|designer-modules|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze|status|catalog|file|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema.",
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
        "Name of the object being created (`cf.init`, `cfe.init`, `epf.init`, `erf.init`), or the drill-down target for `subsystem.info` and `dcs.info`; on `cf.info` it is an alias of `section`",
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
        "Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; `add-value-type`, `add-object-type`, `add-property`, `remove-type` or `remove-property` for `unica.xdto.edit` — read the enum published in the tool's own schema.",
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
        "Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*.",
    ),
    (
        "platformVersion",
        "Requested platform installation version for unica.documentation.search, matched against an installation directory name exactly, for example 8.3.27.2074; when omitted the project's own tools.platform.version constrains the choice, and without that the numerically newest installation found under a configured platform root wins; a tools.platform.path pin names the installation directly instead of walking the roots, with the same version constraints applied to it.",
    ),
    (
        "position",
        "Where unica.code.patch places the content relative to the selector: before or after. Accepted only when insert names a selector",
    ),
    (
        "preset",
        "Declared string argument that no handler reads; role presets are a `preset` key (`view`, `edit`) inside the role JSON definition, not a call argument",
    ),
    (
        "processorName",
        "Name of the owning object, used together with `templateName` and `srcDir` instead of a direct `templatePath` by `unica.mxl.info` and `unica.mxl.validate`; `unica.help.add` also accepts it as an alias of `objectName`.",
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
        "rangeEnd",
        "Integer end of the source line range for unica.code.diagnostics, forwarded as range_end; pair it with rangeStart to scope a mode=file read",
    ),
    (
        "rangeStart",
        "Integer start of the source line range for unica.code.diagnostics, forwarded as range_start; pair it with rangeEnd to scope a mode=file read",
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
        "section",
        "`unica.cf.info` only: drill-down section of `Configuration.xml`, currently just `\"home-page\"`; `name` is accepted as an alias for it",
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
        "sourceSet",
        "Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set",
    ),
    (
        "sourceSets",
        "Array of source-set names for operation extensions when several extensions are synchronized at once; use the singular sourceSet for one",
    ),
    (
        "sources",
        "Boolean that also downloads tool sources on operation tools-download; supported only for tool yaxunit or client-mcp and rejected for vanessa",
    ),
    (
        "srcDir",
        "Directory holding `<objectName>.xml`, default `src`; for `unica.form.remove` and `unica.template.add`/`remove` point it at the type folder such as `src/Reports`, and `unica.mxl.info`/`help.add` use it too",
    ),
    (
        "stderrOutput",
        "Workspace-relative file capturing stderr of the 1C client process in a bounded launch; requires waitForExit true, must differ from output, and unica.runtime.job.start rejects it",
    ),
    (
        "subsystemPath",
        "Path to a subsystem's XML, its directory, or the whole `Subsystems/` folder for `Mode=tree`, used by `unica.subsystem.info`/`edit`/`validate`, relative to `cwd`",
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
        "Name of the template to create with `unica.template.add`, delete with `unica.template.remove`, or read with `unica.mxl.info` together with `processorName`",
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
        "Integer seconds bounding a blocking call: 1..60 (default 30) for unica.runtime.job.wait, and 30..3600 (default 120) for unica.code.diagnostics, which accepts it only with mode analyze.",
    ),
    (
        "tool",
        "Runner tool payload to fetch with operation tools-download: yaxunit, vanessa or client-mcp",
    ),
    (
        "type",
        "Declared string argument that no handler reads",
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
    if matches!(tool.name, "unica.xdto.info" | "unica.xdto.edit") {
        return match name {
            "sourceSet" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": r"^\S(?:.*\S)?$"
            }),
            "typeName" | "name" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": xml_ncname_pattern()
            }),
            "base" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": xml_qname_pattern()
            }),
            "propertyPath" => json!({
                "type": "string",
                "minLength": 1,
                "pattern": xml_property_path_pattern()
            }),
            "metadataPath" => json!({
                "type": "string",
                "pattern": format!(
                    r"^(?:XDTOPackage|ПакетXDTO)\.{}$",
                    xml_property_path_segment_pattern_body()
                ),
                "description": "Logical address of an XDTO package in the form `XDTOPackage.<name>`; the physical `Package.bin` path is rejected."
            }),
            "operation" => json!({
                "type": "string",
                "enum": XDTO_EDIT_OPERATIONS
            }),
            "limit" => json!({
                "type": "integer",
                "minimum": 1,
                "maximum": SOURCE_NAVIGATION_LIMIT_MAX
            }),
            "cursor" => json!({"type": "string", "minLength": 1, "pattern": r"^\S+$"}),
            "property" => json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": { "type": "string", "minLength": 1, "pattern": xml_ncname_pattern() },
                    "type": { "type": "string", "minLength": 1, "pattern": xml_qname_pattern() },
                    "minOccurs": { "type": "integer", "minimum": 0, "maximum": 1 }
                },
                "required": ["name", "type"]
            }),
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
        "unica.code.diagnostics" => match name {
            "mode" => return json!({ "type": "string", "enum": CODE_DIAGNOSTIC_MODES }),
            "timeoutSeconds" => {
                return json!({
                    "type": "integer",
                    "minimum": DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
                    "maximum": DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS,
                    "description": "Only supported for mode analyze. Defaults to 120 seconds."
                });
            }
            "minSeverity" => {
                return json!({ "type": "string", "enum": CODE_DIAGNOSTIC_SEVERITIES });
            }
            "detail" => return json!({ "type": "string", "enum": CODE_DIAGNOSTIC_DETAIL }),
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
    ) {
        Some("array")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::metadata::MetadataOperation;
    use crate::application::tools;

    fn metadata_tool(operation: MetadataOperation) -> ToolSpec {
        ToolSpec {
            name: match operation {
                MetadataOperation::Info => "unica.meta.info",
                MetadataOperation::Add => "unica.meta.add",
                MetadataOperation::Edit => "unica.meta.edit",
                MetadataOperation::Remove => "unica.meta.remove",
            },
            description: "direct metadata contract test",
            mutating: !matches!(operation, MetadataOperation::Info),
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
        let mut names = value["properties"]
            .as_object()
            .expect("schema node must publish properties")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
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
                vec!["limit", "metadataPath", "sections", "sourceSet"],
                json!(["sourceSet", "metadataPath"]),
            ),
            (
                MetadataOperation::Add,
                vec!["dryRun", "kind", "name", "operations", "sourceSet"],
                json!(["sourceSet", "kind", "name"]),
            ),
            (
                MetadataOperation::Edit,
                vec!["dryRun", "metadataPath", "operations", "sourceSet"],
                json!(["sourceSet", "metadataPath", "operations"]),
            ),
            (
                MetadataOperation::Remove,
                vec!["confirm", "dryRun", "force", "metadataPath", "sourceSet"],
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
        assert_eq!(variants.len(), 5);
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
        let mut relations = variants[4]["properties"]["relation"]["enum"]
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
            variants[4]["properties"]["mode"]["enum"],
            json!(["add", "remove", "replace"])
        );
        let targets = &variants[4]["properties"]["targets"];
        assert_eq!(targets["uniqueItems"], true);
        assert!(
            targets.get("minItems").is_none(),
            "relation-specific branches own target cardinality"
        );
        let relation_variants = variants[4]["oneOf"]
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
        let source_arrays = source["properties"]["targets"]["oneOf"]
            .as_array()
            .expect("source targets publish a closed array oneOf");
        assert_eq!(source_arrays.len(), 3);
        assert!(source_arrays.iter().any(|branch| branch["maxItems"] == 0));
        let non_storage = source_arrays
            .iter()
            .find(|branch| branch["minItems"] == 1 && branch.get("maxItems").is_none())
            .expect("source publishes a non-storage array branch");
        let target_variants = non_storage["items"]["oneOf"]
            .as_array()
            .expect("non-storage source targets publish a closed oneOf");
        assert_eq!(target_variants.len(), 8);
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
                "string",
                "number",
                "boolean",
                "date",
                "object",
                "reference",
                "recordSet",
                "definedType",
            ]
        );
        let value_storage = source_arrays
            .iter()
            .find(|branch| branch["minItems"] == 1 && branch["maxItems"] == 1)
            .expect("source publishes a sole ValueStorage branch");
        assert_eq!(
            value_storage["items"]["properties"]["kind"]["const"],
            "valueStorage"
        );
        assert_eq!(target_variants[0]["properties"]["length"]["const"], 0);
        assert_eq!(
            target_variants[0]["properties"]["allowedLength"]["const"],
            "variable"
        );
        assert_eq!(
            target_variants[3]["properties"]["fractions"]["enum"],
            json!(["date", "dateTime"])
        );
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
        assert!(variants[4]["description"]
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
            let properties = schema["properties"].as_object().unwrap();
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

        // A model inspects the schema before it reaches the skills, so an
        // argument without a description has to be guessed at the call site.
        assert!(
            undescribed.is_empty(),
            "arguments published without a description: {undescribed:?}"
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
    fn config_description_excludes_tools_that_stopped_accepting_it() {
        let (_, description) = ARG_DESCRIPTIONS
            .iter()
            .find(|(name, _)| *name == "config")
            .expect("config must have a shared description");

        assert!(!CODE_SEARCH_ARGS.contains(&"config"));
        assert!(!description.contains("unica.code.search"), "{description}");
        assert!(CODE_DIAGNOSTICS_ARGS.contains(&"config"));
        assert!(
            description.contains("unica.code.diagnostics"),
            "{description}"
        );
    }

    #[test]
    fn described_arguments_are_still_reachable() {
        let published: std::collections::BTreeSet<String> = tools()
            .into_iter()
            .flat_map(|tool| allowed_args(&tool))
            .map(|name| {
                let mut chars = name.chars();
                match chars.next() {
                    Some(first) => first.to_lowercase().chain(chars).collect(),
                    None => String::new(),
                }
            })
            .collect();
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
            let published = schema["properties"].as_object().unwrap();

            for name in published.keys() {
                assert!(
                    error.contains(name.as_str()),
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
            error.contains(
                "accepted arguments: confirm, cwd, dryRun, limit, moduleHint, name, sourceDir"
            ),
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

        let valid_operations = [
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-value-type",
                "name": "Document",
                "base": "xs:string"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-object-type",
                "name": "Document"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": "AnyRef",
                "propertyPath": "ObjectRef.Nested",
                "property": {"name": "Document", "type": "tns:Document", "minOccurs": 0}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "remove-type",
                "name": "Document"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "remove-property",
                "typeName": "AnyRef",
                "propertyPath": "ObjectRef.Nested",
                "name": "Document"
            }),
        ];
        assert_eq!(edit_schema["oneOf"].as_array().unwrap().len(), 5);
        for call in &valid_operations {
            assert!(edit_validator.is_valid(call), "schema rejected {call}");
            for dry_run in [true, false] {
                validate_tool_arguments(edit, call.as_object().unwrap(), dry_run)
                    .unwrap_or_else(|error| panic!("dryRun={dry_run} rejected {call}: {error}"));
            }
        }
        let unicode_ncname = json!({
            "sourceSet": "configuration",
            "metadataPath": "ПакетXDTO.Обмен",
            "operation": "add-object-type",
            "name": "A·B́"
        });
        assert!(edit_validator.is_valid(&unicode_ncname));
        validate_tool_arguments(edit, unicode_ncname.as_object().unwrap(), true).unwrap();

        let branch_required = [
            &["name", "base"][..],
            &["name"][..],
            &["typeName", "property"][..],
            &["name"][..],
            &["typeName", "name"][..],
        ];
        let branch_forbidden = [
            &["typeName", "propertyPath", "property"][..],
            &["base", "typeName", "propertyPath", "property"][..],
            &["name", "base"][..],
            &["base", "typeName", "propertyPath", "property"][..],
            &["base", "property"][..],
        ];
        assert_eq!(valid_operations.len(), XDTO_EDIT_OPERATIONS.len());
        assert_eq!(branch_required.len(), valid_operations.len());
        assert_eq!(branch_forbidden.len(), valid_operations.len());
        let field_value = |field: &str| match field {
            "name" => json!("Document"),
            "base" => json!("xs:string"),
            "typeName" => json!("AnyRef"),
            "propertyPath" => json!("Nested"),
            "property" => json!({"name":"Document", "type":"tns:Document"}),
            _ => unreachable!(),
        };
        for ((call, required), forbidden) in valid_operations
            .iter()
            .zip(branch_required)
            .zip(branch_forbidden)
        {
            for field in required {
                let mut missing = call.as_object().unwrap().clone();
                missing.remove(*field);
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
            for field in forbidden {
                let mut incompatible = call.as_object().unwrap().clone();
                incompatible.insert((*field).to_string(), field_value(field));
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
        }
        for field in ["sourceSet", "metadataPath", "operation"] {
            let mut missing = valid_operations[0].as_object().unwrap().clone();
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
                "operation": "add-object-type"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-object-type",
                "name": "Document",
                "base": "xs:string"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": "AnyRef",
                "property": {"name": "Document"}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": "AnyRef",
                "propertyPath": "ObjectRef..Nested",
                "property": {"name": "Document", "type": "tns:Document"}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": "AnyRef",
                "property": {"name": "Document", "type": "tns:Document", "minOccurs": 2}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-value-type",
                "name": "bad:name",
                "base": "xs:string"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-value-type",
                "name": "Document",
                "base": "xs::string"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-object-type",
                "name": 1
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-value-type",
                "name": "Document",
                "base": 1
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": 1,
                "property": {"name": "Document", "type": "tns:Document"}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "remove-property",
                "typeName": "AnyRef",
                "name": "Document",
                "propertyPath": 1
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": "AnyRef",
                "property": {"name": "Document", "type": "tns:Document", "minOccurs": -1}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": "AnyRef",
                "property": {"name": "Document", "type": "tns:Document", "minOccurs": "0"}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": "AnyRef",
                "property": {"name": "Document", "type": "tns:Document", "extra": true}
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "rename-type",
                "name": "Document"
            }),
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": " add-object-type",
                "name": "Document"
            }),
            json!({
                "sourceSet": " configuration ",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-object-type",
                "name": "Document"
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
            "operation": "add-property",
            "property": {"name": "Document", "type": "Document", "upperBound": 1}
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
                "operation": "add-value-type",
                "name": "Document",
                "base": base
            })
        };
        let property = |type_ref: &str| {
            json!({
                "sourceSet": "configuration",
                "metadataPath": "XDTOPackage.EnterpriseData_1_17_3",
                "operation": "add-property",
                "typeName": "AnyRef",
                "property": {"name": "Document", "type": type_ref}
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
        let patterns = [
            &info_schema["properties"]["metadataPath"]["pattern"],
            &info_schema["properties"]["typeName"]["pattern"],
            &edit_schema["properties"]["metadataPath"]["pattern"],
            &edit_schema["properties"]["name"]["pattern"],
            &edit_schema["properties"]["typeName"]["pattern"],
            &edit_schema["properties"]["base"]["pattern"],
            &edit_schema["properties"]["propertyPath"]["pattern"],
            &edit_schema["properties"]["property"]["properties"]["name"]["pattern"],
            &edit_schema["properties"]["property"]["properties"]["type"]["pattern"],
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
            "operation": "add-property",
            "typeName": astral_name,
            "propertyPath": format!("{astral_name}.{astral_name}"),
            "property": {
                "name": astral_name,
                "type": format!("{astral_name}:{astral_name}")
            }
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
                "operation": "add-property",
                "typeName": "AnyRef",
                "propertyPath": property_path,
                "property": {"name": "Document", "type": "tns:Document"}
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
        let operation_tags = metadata_operation_union(&schema)["oneOf"]
            .as_array()
            .expect("closed operation union")
            .iter()
            .map(|variant| variant["properties"]["op"]["enum"][0].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            Value::Array(operation_tags),
            json!(["setProperties", "add", "update", "remove", "editRelations"])
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
            .find(|tool| tool.name == "unica.cf.info")
            .unwrap();
        let mut args = Map::new();
        args.insert("ConfigPath".to_string(), json!("Configuration.xml"));
        args.insert("dryRun".to_string(), json!("false"));

        let error = validate_tool_arguments(tool, &args, false).unwrap_err();

        assert!(error.contains("dryRun"));
        assert!(error.contains("boolean"));
    }

    #[test]
    fn form_and_template_boolean_flags_are_boolean_in_mcp_contract() {
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

        let template_add = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.template.add")
            .unwrap();
        let schema = input_schema_for_tool(&template_add);
        assert_eq!(schema["properties"]["SetMainSKD"]["type"], "boolean");
        assert_eq!(schema["properties"]["setMainSKD"]["type"], "boolean");

        let mut args = Map::new();
        args.insert("ObjectName".to_string(), json!("Report"));
        args.insert("TemplateName".to_string(), json!("MainSchema"));
        args.insert("TemplateType".to_string(), json!("DataCompositionSchema"));
        args.insert("SetMainSKD".to_string(), json!(false));
        args.insert("setMainSKD".to_string(), json!(true));
        let error = validate_tool_arguments(template_add, &args, false).unwrap_err();
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
    fn code_patch_schema_accepts_each_documented_selector_variant() {
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
            json!({"query": "   "}),
            json!({"query": 42}),
            json!({"query": null}),
            json!({"query": true}),
            json!({"query": {}}),
            json!({"query": "Post", "limit": 0}),
            json!({"query": "Post", "limit": 51}),
        ] {
            assert!(
                validate_tool_arguments(search, args.as_object().unwrap(), false).is_err(),
                "payload must be rejected: {args}"
            );
        }
        validate_tool_arguments(
            search,
            json!({"query": "Post", "limit": 50}).as_object().unwrap(),
            false,
        )
        .unwrap();
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
        validate_tool_arguments(definition, &args, true).unwrap();
    }

    #[test]
    fn help_add_contract_exposes_typed_arguments_without_raw_args() {
        let help_add = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.help.add")
            .expect("unica.help.add must be registered");

        let schema = input_schema_for_tool(&help_add);
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("ObjectName").is_some());
        assert!(schema["properties"].get("Lang").is_some());
        assert!(schema["properties"].get("SrcDir").is_some());
        assert!(schema["properties"].get("args").is_none());
        assert_eq!(schema["required"], json!(["ObjectName"]));

        let mut args = Map::new();
        args.insert("args".to_string(), json!(["scripts/add-help.py"]));
        let error = validate_tool_arguments(help_add, &args, false).unwrap_err();
        assert!(error.contains("does not accept argument `args`"));

        let args = Map::new();
        let error = validate_tool_arguments(help_add, &args, false).unwrap_err();
        assert!(error.contains("requires `ObjectName`"));
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
        assert_eq!(schema["required"], json!(["TemplatePath"]));
        assert!(schema.get("allOf").is_none());

        let mut args = Map::new();
        args.insert(
            "TemplatePath".to_string(),
            json!("Reports/Sales/Templates/Main"),
        );
        validate_tool_arguments(dcs_info, &args, false).unwrap();
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

    #[test]
    fn bsl_diagnostics_contract_exposes_modes_and_keeps_analyze_default() {
        let diagnostics = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.diagnostics")
            .expect("unica.code.diagnostics must be registered");

        let schema = input_schema_for_tool(&diagnostics);
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("args").is_none());
        assert!(schema["properties"].get("argv").is_none());
        assert!(schema["properties"].get("cwd").is_some());
        assert!(schema["properties"].get("sourceDir").is_some());
        assert_eq!(schema["properties"]["codes"]["type"], "array");
        assert_eq!(schema["properties"]["rangeStart"]["type"], "integer");
        assert_eq!(schema["properties"]["maxFiles"]["type"], "integer");
        assert_eq!(schema["properties"]["timeoutSeconds"]["type"], "integer");
        assert_eq!(schema["properties"]["timeoutSeconds"]["minimum"], 30);
        assert_eq!(schema["properties"]["timeoutSeconds"]["maximum"], 3600);
        assert!(schema.get("oneOf").is_none());
        assert!(schema["properties"]["mode"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("workspace")));

        let mut args = Map::new();
        args.insert("mode".to_string(), json!("file"));
        let error = validate_tool_arguments(diagnostics, &args, false).unwrap_err();
        assert!(error.contains("requires `path`"));

        let mut args = Map::new();
        args.insert("mode".to_string(), json!("analyze"));
        args.insert(
            "path".to_string(),
            json!("src/CommonModules/Probe/Ext/Module.bsl"),
        );
        let error = validate_tool_arguments(diagnostics, &args, false).unwrap_err();
        assert!(error.contains("does not support `path`"));

        let mut args = Map::new();
        args.insert(
            "path".to_string(),
            json!("src/CommonModules/Probe/Ext/Module.bsl"),
        );
        let error = validate_tool_arguments(diagnostics, &args, false).unwrap_err();
        assert!(error.contains("does not support `path`"));

        for mode in ["status", "catalog", "workspace"] {
            let mut args = Map::new();
            args.insert("mode".to_string(), json!(mode));
            args.insert(
                "path".to_string(),
                json!("src/CommonModules/Probe/Ext/Module.bsl"),
            );
            let error = validate_tool_arguments(diagnostics, &args, false).unwrap_err();
            assert!(
                error.contains(&format!("mode `{mode}` does not support `path`")),
                "mode {mode} must reject `path` instead of dropping it: {error}"
            );
        }

        let mut args = Map::new();
        args.insert("mode".to_string(), json!("file"));
        args.insert(
            "path".to_string(),
            json!("src/CommonModules/Probe/Ext/Module.bsl"),
        );
        validate_tool_arguments(diagnostics, &args, false).unwrap();

        let mut args = Map::new();
        args.insert("mode".to_string(), json!("raw"));
        let error = validate_tool_arguments(diagnostics, &args, false).unwrap_err();
        assert!(error.contains("must be one of"));

        let args = Map::new();
        validate_tool_arguments(diagnostics, &args, false).unwrap();

        for timeout in [30, 900, 3600] {
            let mut args = Map::new();
            args.insert("timeoutSeconds".to_string(), json!(timeout));
            validate_tool_arguments(diagnostics, &args, false).unwrap();
        }

        for mode in ["status", "catalog", "file", "workspace"] {
            let mut args = Map::new();
            args.insert("mode".to_string(), json!(mode));
            args.insert("timeoutSeconds".to_string(), json!(900));
            let error = validate_tool_arguments(diagnostics, &args, false).unwrap_err();
            assert!(
                error.contains("only supported for mode `analyze`"),
                "{mode}: {error}"
            );
        }

        for timeout in [json!("900"), json!(29), json!(3601), json!(-1), json!(30.5)] {
            let mut args = Map::new();
            args.insert("timeoutSeconds".to_string(), timeout);
            assert!(validate_tool_arguments(diagnostics, &args, false).is_err());
        }
    }
}

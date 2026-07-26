use super::operation_descriptors::{native_operation_descriptor, native_path_alias_groups};
use super::{RuntimeJobAction, ToolHandler, ToolSpec};
use crate::domain::form_edit::{form_edit_definition_schema, validate_form_edit_definition};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

const COMMON_ARGS: &[&str] = &["cwd", "dryRun", "confirm"];
const CODE_PATCH_ARGS: &[&str] = &[
    "path",
    "operation",
    "selector",
    "content",
    "position",
    "sourceDir",
];
const RUNTIME_JOB_STATUS_ARGS: &[&str] = &["jobId"];
const RUNTIME_JOB_WAIT_ARGS: &[&str] = &["jobId", "timeoutSeconds"];
const RUNTIME_JOB_LOGS_ARGS: &[&str] = &["jobId", "tailChars"];

pub(crate) const DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS: u64 = 30;
pub(crate) const DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS: u64 = 3600;

const META_EDIT_OPERATIONS: &[&str] = &[
    "modify-property",
    "add-attribute",
    "add-ts",
    "add-dimension",
    "add-resource",
    "add-enumValue",
    "add-column",
    "add-form",
    "add-template",
    "add-command",
    "add-owner",
    "add-registerRecord",
    "add-basedOn",
    "add-inputByString",
    "remove-attribute",
    "remove-ts",
    "remove-dimension",
    "remove-resource",
    "remove-enumValue",
    "remove-column",
    "remove-form",
    "remove-template",
    "remove-command",
    "remove-owner",
    "remove-registerRecord",
    "remove-basedOn",
    "remove-inputByString",
    "add-ts-attribute",
    "modify-attribute",
    "modify-dimension",
    "modify-resource",
    "modify-enumValue",
    "modify-column",
    "modify-ts",
    "modify-ts-attribute",
    "remove-ts-attribute",
    "set-owners",
    "set-registerRecords",
    "set-basedOn",
    "set-inputByString",
];
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
    "KeepFiles",
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
    "keepFiles",
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
const CODE_GREP_ARGS: &[&str] = &[
    "excludePath",
    "fileTypes",
    "ignoreCase",
    "limit",
    "mode",
    "path",
    "query",
    "regex",
    "sourceDir",
];
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
const META_PROFILE_ARGS: &[&str] = &["limit", "name", "sections", "sourceDir"];
const META_PROFILE_SECTIONS: &[&str] = &[
    "structure",
    "modules",
    "roles",
    "subscriptions",
    "functionalOptions",
    "predefinedItems",
];

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

pub fn input_schema_for_tool(tool: &ToolSpec) -> Value {
    let property_names = allowed_args(tool);
    let mut properties = Map::new();
    for name in property_names {
        let mut property = property_schema_for_tool(tool, name);
        // Attached here rather than inside property_schema so that the
        // tool-specific overrides above, which return their own enums and
        // patterns, are described too.
        if let Some(description) = description_for_arg(name) {
            if let Some(object) = property.as_object_mut() {
                object.insert("description".to_string(), json!(description));
            }
        }
        properties.insert(name.to_string(), property);
    }

    let mut required = required_args(tool);
    let mut required_path_aliases = Vec::new();
    if let ToolHandler::NativeOperation { operation, .. } = tool.handler {
        for group in native_path_alias_groups(operation) {
            if let Some(index) = required
                .iter()
                .position(|required| *required == group.canonical)
            {
                required.remove(index);
                required_path_aliases.push(json!({
                    "anyOf": group
                        .aliases
                        .iter()
                        .map(|alias| json!({"required": [alias]}))
                        .collect::<Vec<_>>()
                }));
            }
        }
    }

    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    });
    if !required_path_aliases.is_empty() {
        schema["allOf"] = Value::Array(required_path_aliases);
    }
    if tool.name == "unica.form.edit" {
        schema["anyOf"] = json!([
            {"required": ["JsonPath"]},
            {"required": ["jsonPath"]},
            {"required": ["definition"]}
        ]);
    }
    schema
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
    let allowed = allowed_args(&tool).into_iter().collect::<BTreeSet<_>>();
    for key in args.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(format!(
                "{} does not accept argument `{key}`; use typed MCP arguments only",
                tool.name
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
    validate_code_patch_arguments(tool, args)?;
    validate_meta_edit_arguments(tool, args)?;
    validate_form_add_arguments(tool, args)?;
    validate_form_edit_arguments(tool, args, dry_run)?;
    validate_template_add_arguments(tool, args)?;
    validate_support_arguments(tool, args, dry_run)?;
    validate_external_init_arguments(tool, args)?;
    validate_cfe_patch_method_arguments(tool, args)?;

    if !dry_run || is_external_init_tool(tool) {
        for required in required_args(&tool) {
            if !args.contains_key(required) {
                return Err(format!("{} requires `{required}` argument", tool.name));
            }
        }
    }

    Ok(())
}

fn validate_code_patch_arguments(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    if tool.name != "unica.code.patch" {
        return Ok(());
    }
    for key in ["path", "operation", "content", "position"] {
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
    if args.get("operation").and_then(Value::as_str) != Some("insert") {
        return Err(format!("{} supports only operation `insert`", tool.name));
    }
    if !matches!(
        args.get("position").and_then(Value::as_str),
        Some("before" | "after")
    ) {
        return Err(format!(
            "{} argument `position` must be `before` or `after`",
            tool.name
        ));
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

fn validate_meta_edit_arguments(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    if tool.name != "unica.meta.edit" {
        return Ok(());
    }

    validate_unique_alias_group(tool.name, args, &["Operation", "operation"])?;
    validate_unique_alias_group(tool.name, args, &["DefinitionFile", "definitionFile"])?;

    if contains_any(args, &["Operation", "operation"])
        && contains_any(args, &["DefinitionFile", "definitionFile"])
    {
        return Err(format!(
            "{} accepts either Operation or DefinitionFile, not both",
            tool.name
        ));
    }

    for name in ["Operation", "operation"] {
        let Some(value) = args.get(name) else {
            continue;
        };
        let Some(operation) = value.as_str() else {
            return Err(format!("{} argument `{name}` must be string", tool.name));
        };
        if !META_EDIT_OPERATIONS.contains(&operation) {
            return Err(format!(
                "{} unsupported Operation `{operation}`; supported: {}",
                tool.name,
                META_EDIT_OPERATIONS.join(", ")
            ));
        }
    }

    Ok(())
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
        "unica.code.graph" => {
            validate_enum_argument(tool.name, args, "mode", CODE_GRAPH_MODES)?;
            validate_enum_argument(tool.name, args, "dir", CODE_GRAPH_DIRECTIONS)?;
            validate_enum_argument(tool.name, args, "detail", CODE_GRAPH_DETAIL)?;
        }
        "unica.code.diagnostics" => {
            validate_enum_argument(tool.name, args, "mode", CODE_DIAGNOSTIC_MODES)?;
            validate_enum_argument(tool.name, args, "minSeverity", CODE_DIAGNOSTIC_SEVERITIES)?;
            validate_enum_argument(tool.name, args, "detail", CODE_DIAGNOSTIC_DETAIL)?;
            if args.contains_key("timeoutSeconds") {
                let mode = args
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("analyze");
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
            if !dry_run
                && args
                    .get("mode")
                    .and_then(Value::as_str)
                    .is_some_and(|mode| mode == "file")
                && !args.contains_key("path")
            {
                return Err(format!(
                    "{} mode `file` requires `path` argument",
                    tool.name
                ));
            }
        }
        "unica.meta.profile" => {
            validate_array_enum_argument(tool.name, args, "sections", META_PROFILE_SECTIONS)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_array_enum_argument(
    tool_name: &str,
    args: &Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<(), String> {
    let Some(value) = args.get(key) else {
        return Ok(());
    };
    let Some(items) = value.as_array() else {
        return Err(format!("{tool_name} argument `{key}` must be array"));
    };
    for item in items {
        let Some(item) = item.as_str() else {
            return Err(format!("{tool_name} argument `{key}` must contain strings"));
        };
        if !allowed.contains(&item) {
            return Err(format!(
                "{tool_name} argument `{key}` values must be one of: {}",
                allowed.join(", ")
            ));
        }
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
            return Err(format!(
                "{tool_name} operation `{operation}` does not accept `{key}`"
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

fn allowed_args(tool: &ToolSpec) -> Vec<&'static str> {
    let mut names = COMMON_ARGS.to_vec();
    match tool.handler {
        ToolHandler::NativeOperation { operation, .. } => {
            if operation == "code-patch" {
                names.extend(CODE_PATCH_ARGS);
            } else {
                names.extend(native_args_for(operation));
            }
            if operation == "form-edit" {
                names.push("definition");
            }
        }
        ToolHandler::BuildRuntime { .. } => names.extend(BUILD_ARGS),
        ToolHandler::RuntimeAdapter => names.extend(RUNTIME_ARGS),
        ToolHandler::RuntimeJob { action } => names.extend(runtime_job_args(action)),
        ToolHandler::CodeAdapter { .. } => names.extend(code_args_for(tool.name)),
        ToolHandler::StandardsAdapter { .. } => names.extend(STANDARDS_ARGS),
        ToolHandler::ProjectStatus | ToolHandler::ProjectMap => {}
    }
    names.sort_unstable();
    names.dedup();
    names
}

fn native_args_for(operation: &str) -> &'static [&'static str] {
    match operation {
        "epf-init" | "erf-init" => EXTERNAL_INIT_ARGS,
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
        ToolHandler::RuntimeAdapter => runtime_required_args(tool),
        ToolHandler::RuntimeJob { action } => runtime_job_required_args(action),
        ToolHandler::CodeAdapter { .. } => match tool.name {
            "unica.code.definition" => vec!["name"],
            "unica.code.outline" => vec!["path"],
            "unica.code.grep" => vec!["query"],
            "unica.code.graph" => vec!["mode"],
            "unica.meta.profile" => vec!["name"],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn code_args_for(tool_name: &str) -> &'static [&'static str] {
    match tool_name {
        "unica.code.definition" => CODE_DEFINITION_ARGS,
        "unica.code.outline" => CODE_OUTLINE_ARGS,
        "unica.code.grep" => CODE_GREP_ARGS,
        "unica.code.graph" => CODE_GRAPH_ARGS,
        "unica.code.diagnostics" => CODE_DIAGNOSTICS_ARGS,
        "unica.meta.profile" => META_PROFILE_ARGS,
        _ => CODE_ARGS,
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
            | "KeepFiles"
            | "keepFiles"
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
            | "ignoreCase"
            | "regex"
    ) {
        "boolean"
    } else if name == "definition" {
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
        "Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.search and unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config.",
    ),
    (
        "configDir",
        "Root of the configuration dump (the directory holding `Configuration.xml`) for `unica.meta.remove`, relative to `cwd`; that tool takes `configDir`, not `configPath`",
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
        "Non-empty BSL text that unica.code.patch inserts at the selector; its line endings are normalized to the target module's EOL before writing",
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
        "Path to a JSON file holding a batch of operations or a full definition, for `unica.cf.edit`, `meta.edit`, `interface.edit`, `subsystem.edit`/`compile` and `dcs.compile`; relative to `cwd`",
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
        "excludePath",
        "One workspace path to exclude from unica.code.grep, resolved against cwd (or placed under sourceDir when relative) and applied as a git-grep exclude pathspec",
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
        "fileTypes",
        "String, not an array: bare file extensions for unica.code.grep separated by commas, semicolons, or spaces, such as \"bsl\" or \"bsl,xml\"; each must be alphanumeric",
    ),
    (
        "filterTags",
        "Array of Vanessa Automation tags to include for operation test with testRunner va; each entry becomes one --filter-tag",
    ),
    (
        "force",
        "Boolean --force: on unica.runtime.execute it overwrites an existing project config for config-init and re-downloads the payload for tools-download, and no other runtime operation accepts it; the native XML tools expose their own Force, for example unica.meta.remove, where it removes an object despite discovered references.",
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
        "ignoreCase",
        "Boolean for unica.code.grep; true makes the match case-insensitive (git grep -i), and it defaults to false",
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
        "Path to the JSON DSL file, relative to `cwd`, for `unica.form.compile`, `unica.form.edit`, `unica.meta.compile`, `unica.mxl.compile` and `unica.role.compile`",
    ),
    (
        "keepFiles",
        "`unica.meta.remove` only: boolean that deregisters the object from `Configuration.xml` but leaves its files on disk (requires typing `KeepFiles` as boolean in `property_schema`, which currently declares it a string).",
    ),
    ("kind", "Declared string argument that no tool handler reads"),
    (
        "lang",
        "`unica.help.add` only: language code of the help page to create, default `\"ru\"`; `language` is accepted as an alias for it",
    ),
    (
        "language",
        "Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained",
    ),
    (
        "limit",
        "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20, code.definition 50, code.grep 200, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results).",
    ),
    (
        "maxErrors",
        "Stop a validate tool after this many errors: default 30 for `unica.cf.validate`, `cfe.validate`, `form.validate`, `meta.validate` and `interface.validate`, 20 for `unica.dcs.validate` and `unica.mxl.validate`; `unica.role.validate` and `unica.subsystem.validate` accept the key but ignore it.",
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
        "Declared string argument that no handler reads; `unica.role.validate` derives the role metadata XML (`Roles/<Name>.xml`) from `rightsPath` itself and checks its UUID, name and synonym whenever that file exists.",
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
        "Name of the object being created (`cf.init`, `cfe.init`, `epf.init`, `erf.init`), or the drill-down target for `meta.info`, `subsystem.info` and `dcs.info`; on `cf.info` it is an alias of `section`",
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
        "Path to an object's metadata XML — a directory resolves to `<name>/<name>.xml` — for `unica.meta.edit`/`info`/`validate` and `unica.form.add`, relative to `cwd`; `meta.validate` accepts several joined by `|`",
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
        "Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` for unica.code.patch; the metadata edit verbs for unica.meta.edit — read the enum published in the tool's own schema.",
    ),
    (
        "output",
        "Workspace-relative destination: the artifact file for make (a publish directory for external source-sets), the conversion directory for convert, and the platform /Out log for a direct-client launch",
    ),
    (
        "outputDir",
        "Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `meta.compile`/`role.compile`/`subsystem.compile`",
    ),
    (
        "outputPath",
        "Path of the single file to generate: the `Form.xml` for `unica.form.compile`, the `Template.xml` for `dcs.compile` and `mxl.compile`, or the JSON for `mxl.decompile`, where omitting it prints to stdout",
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
        "Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), the target *Module.bsl or module-relative file for unica.code.patch and the other unica.code.* tools, the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*.",
    ),
    (
        "position",
        "Where unica.code.patch places the content relative to the selector: before or after",
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
        "Search text: FTS phrase for unica.code.search, git-grep pattern for unica.code.grep, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback",
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
        "Declared boolean that no handler reads: `unica.dcs.info` with `Mode=query` always emits the `=== Query: <dataset> ===` header and is still truncated by `limit`/`offset`, so passing it changes nothing (the skill documents it, the code does not implement it).",
    ),
    (
        "rawKeys",
        "Array of extra non-reserved platform launch keys such as /TESTMANAGER for a direct-client operation launch; never repeat /C, /Execute or /Out here",
    ),
    (
        "regex",
        "Boolean for unica.code.grep; false (the default) matches query as a fixed literal, true treats query as a regular expression",
    ),
    (
        "rightsPath",
        "Path to a role's `Rights.xml`, or the role directory that resolves to it, for `unica.role.info` and `unica.role.validate`, relative to `cwd`",
    ),
    (
        "scenarioFilters",
        "Array of Vanessa Automation scenario filters for operation test with testRunner va; each entry becomes one --scenario-filter",
    ),
    (
        "section",
        "`unica.cf.info` only: drill-down section of `Configuration.xml`, currently just `\"home-page\"`; `name` is accepted as an alias for it",
    ),
    (
        "sections",
        "Array of profile sections unica.meta.profile returns, from structure, modules, roles, subscriptions, functionalOptions, predefinedItems; omit it for all sections except predefinedItems, which must be listed explicitly",
    ),
    (
        "selector",
        "Object naming the unica.code.patch insertion point: exactly one of {\"method\": \"Name\"} for a whole procedure or function, or {\"anchor\": \"text\"} for a fragment that occurs once inside one method",
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
        "sourceDir",
        "Workspace-relative source root to work in: on unica.code.* , unica.code.patch and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.runtime.execute has no sourceDir and selects sources by configured sourceSet name instead.",
    ),
    (
        "sourceSet",
        "Name of one source-set declared in v8project.yaml, such as main or external-processors; accepted by config-init, build, dump, convert, make and extensions",
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
        "Declared string argument that no handler reads; an object's `type` is a key inside the meta-compile JSON definition, not a call argument",
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
    if tool.name == "unica.form.edit" && name == "definition" {
        return form_edit_definition_schema();
    }
    if tool.name == "unica.code.patch" {
        return match name {
            "operation" => json!({ "type": "string", "enum": ["insert"] }),
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
    if tool.name == "unica.meta.edit" && matches!(name, "Operation" | "operation") {
        return json!({ "type": "string", "enum": META_EDIT_OPERATIONS });
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
        "unica.meta.profile" if name == "sections" => {
            return json!({
                "type": "array",
                "items": {"type": "string", "enum": META_PROFILE_SECTIONS}
            });
        }
        _ => {}
    }
    property_schema(name)
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
            | "KeepFiles"
            | "keepFiles"
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
            | "ignoreCase"
            | "regex"
    ) {
        Some("boolean")
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
    use crate::application::tools;

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

    #[test]
    fn read_only_native_tools_reject_out_file_arguments() {
        let required_path = |name: &str| match name {
            "unica.cf.info" | "unica.cf.validate" => ("ConfigPath", "src"),
            "unica.cfe.validate" => ("ExtensionPath", "src"),
            "unica.meta.info" | "unica.meta.validate" => ("ObjectPath", "src/Object.xml"),
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
            "unica.meta.info",
            "unica.meta.validate",
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
        args.insert(
            "path".to_string(),
            json!("src/CommonModules/X/Ext/Module.bsl"),
        );
        args.insert("operation".to_string(), json!("insert"));
        args.insert("selector".to_string(), json!({"method": "ПриСоздании"}));
        args.insert("content".to_string(), json!("Сообщить(\"ok\");"));
        args.insert("position".to_string(), json!("after"));
        args.insert("sourceDir".to_string(), json!("src"));
        validate_tool_arguments(tool, &args, false).unwrap();

        args.insert(
            "selector".to_string(),
            json!({"method": "A", "anchor": "B"}),
        );
        assert!(validate_tool_arguments(tool, &args, false).is_err());
        args.insert("rawArgs".to_string(), json!(["--unsafe"]));
        assert!(validate_tool_arguments(tool, &args, false).is_err());
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
            "path": "src/CommonModules/X/Ext/Module.bsl",
            "operation": "insert",
            "content": "Сообщить(\"ok\");",
            "position": "after",
            "sourceDir": "src",
        });

        for selector in [
            json!({"method": "ПриСоздании"}),
            json!({"anchor": "Сообщить"}),
        ] {
            let mut instance = base.clone();
            instance["selector"] = selector;
            assert!(validator.is_valid(&instance), "{instance}");
        }

        let mut invalid = base;
        invalid["selector"] = json!({"method": "A", "anchor": "B"});
        assert!(!validator.is_valid(&invalid));
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
    fn meta_remove_keep_files_contract_is_boolean() {
        let remove = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.meta.remove")
            .expect("unica.meta.remove must be registered");
        let schema = input_schema_for_tool(&remove);

        assert_eq!(schema["properties"]["KeepFiles"]["type"], "boolean");
        assert_eq!(schema["properties"]["keepFiles"]["type"], "boolean");
        validate_tool_arguments(
            remove,
            json!({
                "ConfigDir": "src",
                "Object": "Catalog.Legacy",
                "KeepFiles": true,
            })
            .as_object()
            .expect("test arguments must be an object"),
            false,
        )
        .expect("meta.remove must accept boolean KeepFiles");
    }

    #[test]
    fn native_required_path_aliases_are_honest_json_schema_alternatives() {
        let cases = [
            (
                "unica.cf.info",
                vec![
                    json!({"ConfigPath": "src"}),
                    json!({"configPath": "src"}),
                    json!({"Path": "src"}),
                    json!({"path": "src"}),
                    json!({"ConfigPath": "src", "configPath": "src"}),
                ],
            ),
            (
                "unica.meta.edit",
                vec![
                    json!({"ObjectPath": "Catalogs/Items.xml"}),
                    json!({"objectPath": "Catalogs/Items.xml"}),
                    json!({"Path": "Catalogs/Items.xml"}),
                    json!({"path": "Catalogs/Items.xml"}),
                ],
            ),
            (
                "unica.form.edit",
                vec![
                    json!({"FormPath": "Ext/Form.xml", "definition": {}}),
                    json!({"formPath": "Ext/Form.xml", "definition": {}}),
                    json!({"Path": "Ext/Form.xml", "definition": {}}),
                    json!({"path": "Ext/Form.xml", "definition": {}}),
                    json!({
                        "FormPath": "Ext/Form.xml",
                        "JsonPath": "edit.json",
                        "jsonPath": "edit.json"
                    }),
                ],
            ),
            (
                "unica.interface.edit",
                vec![
                    json!({"CIPath": "Ext/CommandInterface.xml"}),
                    json!({"ciPath": "Ext/CommandInterface.xml"}),
                    json!({"Path": "Ext/CommandInterface.xml"}),
                    json!({"path": "Ext/CommandInterface.xml"}),
                ],
            ),
            (
                "unica.subsystem.edit",
                vec![
                    json!({"SubsystemPath": "Subsystems/Sales.xml"}),
                    json!({"subsystemPath": "Subsystems/Sales.xml"}),
                    json!({"Path": "Subsystems/Sales.xml"}),
                    json!({"path": "Subsystems/Sales.xml"}),
                ],
            ),
            (
                "unica.dcs.edit",
                vec![
                    json!({"TemplatePath": "Ext/Template.xml"}),
                    json!({"templatePath": "Ext/Template.xml"}),
                    json!({"Path": "Ext/Template.xml"}),
                    json!({"path": "Ext/Template.xml"}),
                ],
            ),
            (
                "unica.form.compile",
                vec![
                    json!({"OutputPath": "Ext/Form.xml"}),
                    json!({"outputPath": "Ext/Form.xml"}),
                    json!({"OutputPath": "Ext/Form.xml", "outputPath": "Ext/Form.xml"}),
                ],
            ),
        ];

        for (tool_name, instances) in cases {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == tool_name)
                .unwrap();
            let schema = input_schema_for_tool(&tool);
            let validator = jsonschema::validator_for(&schema).unwrap();
            for instance in instances {
                assert!(
                    validator.is_valid(&instance),
                    "{tool_name} schema rejected documented path aliases: {instance}; schema={schema}"
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
                    assert!(
                        properties.contains_key(*alias),
                        "{operation} path alias {alias} is not public in its MCP schema"
                    );
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
        assert_eq!(schema["required"], json!([]));
        assert_eq!(
            schema["allOf"],
            json!([{
                "anyOf": [
                    {"required": ["FormPath"]},
                    {"required": ["formPath"]},
                    {"required": ["Path"]},
                    {"required": ["path"]}
                ]
            }])
        );
        assert_eq!(
            schema["anyOf"],
            json!([
                {"required": ["JsonPath"]},
                {"required": ["jsonPath"]},
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
    fn meta_edit_contract_accepts_definition_file_and_extended_operations() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.meta.edit")
            .unwrap();
        let schema = input_schema_for_tool(&tool);
        assert!(schema["properties"]["Operation"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("add-dimension")));
        assert!(schema["properties"]["Operation"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("set-owners")));

        let mut args = Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!("src/Catalogs/Items/Items.xml"),
        );
        args.insert("DefinitionFile".to_string(), json!("edit.json"));
        validate_tool_arguments(tool, &args, false).unwrap();

        args.insert("Operation".to_string(), json!("add-attribute"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("either Operation or DefinitionFile"));

        let mut args = Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!("src/Catalogs/Items/Items.xml"),
        );
        args.insert("Operation".to_string(), json!("add-unknown"));
        let error = validate_tool_arguments(tool, &args, false).unwrap_err();
        assert!(error.contains("unsupported Operation"));
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

        assert_eq!(selector["type"], "object");
        assert_eq!(selector["additionalProperties"], false);
        assert_eq!(selector["properties"]["method"]["type"], "string");
        assert_eq!(selector["properties"]["anchor"]["type"], "string");
        assert_eq!(selector["oneOf"].as_array().map(Vec::len), Some(2));
        for required in ["path", "operation", "selector", "content", "position"] {
            assert!(schema["required"]
                .as_array()
                .is_some_and(|items| { items.iter().any(|value| value == required) }));
        }
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
        let grep = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.grep")
            .expect("unica.code.grep must be registered");

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

        let grep_schema = input_schema_for_tool(&grep);
        assert_eq!(grep_schema["additionalProperties"], false);
        assert!(grep_schema["properties"].get("query").is_some());
        assert!(grep_schema["properties"].get("excludePath").is_some());
        assert_eq!(grep_schema["properties"]["regex"]["type"], "boolean");
        assert_eq!(grep_schema["properties"]["ignoreCase"]["type"], "boolean");
        assert_eq!(grep_schema["required"], json!(["query"]));
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

    #[test]
    fn dcs_info_contract_exposes_raw_query_export() {
        let dcs_info = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.dcs.info")
            .expect("unica.dcs.info must be registered");

        let schema = input_schema_for_tool(&dcs_info);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["Raw"]["type"], "boolean");
        assert_eq!(schema["required"], json!([]));
        assert_eq!(
            schema["allOf"],
            json!([{
                "anyOf": [
                    {"required": ["TemplatePath"]},
                    {"required": ["templatePath"]},
                    {"required": ["Path"]},
                    {"required": ["path"]}
                ]
            }])
        );

        let mut args = Map::new();
        args.insert(
            "TemplatePath".to_string(),
            json!("Reports/Sales/Templates/Main"),
        );
        args.insert("Mode".to_string(), json!("query"));
        args.insert("Name".to_string(), json!("Sales"));
        args.insert("Raw".to_string(), json!(true));
        validate_tool_arguments(dcs_info, &args, false).unwrap();
    }

    #[test]
    fn meta_profile_contract_exposes_typed_arguments_without_raw_args() {
        let profile = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.meta.profile")
            .expect("unica.meta.profile must be registered");

        let schema = input_schema_for_tool(&profile);
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("name").is_some());
        assert_eq!(schema["properties"]["sections"]["type"], "array");
        assert_eq!(schema["properties"]["limit"]["type"], "integer");
        assert!(schema["properties"].get("args").is_none());
        assert!(schema["properties"].get("rlm_execute").is_none());
        assert_eq!(schema["required"], json!(["name"]));

        let mut args = Map::new();
        args.insert("args".to_string(), json!(["get_object_profile"]));
        let error = validate_tool_arguments(profile, &args, false).unwrap_err();
        assert!(error.contains("does not accept argument `args`"));

        let args = Map::new();
        let error = validate_tool_arguments(profile, &args, false).unwrap_err();
        assert!(error.contains("requires `name`"));
        validate_tool_arguments(profile, &args, true).unwrap();
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

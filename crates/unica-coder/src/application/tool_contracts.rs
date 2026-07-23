use super::operation_descriptors::native_operation_descriptor;
use super::{RuntimeJobAction, ToolHandler, ToolSpec};
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
    "OutFile",
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
    "outFile",
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
        properties.insert(name.to_string(), property_schema_for_tool(tool, name));
    }

    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required_args(tool),
    });
    if tool.name == "unica.form.edit" {
        schema["oneOf"] = json!([
            {"required": ["JsonPath"]},
            {"required": ["jsonPath"]},
            {"required": ["definition"]}
        ]);
    }
    schema
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

fn property_schema_for_tool(tool: &ToolSpec, name: &str) -> Value {
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
    fn mutating_dry_run_does_not_require_payload() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.form.edit")
            .unwrap();
        let args = Map::new();

        validate_tool_arguments(tool, &args, true).unwrap();
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
        assert_eq!(
            schema["oneOf"],
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
        assert_eq!(schema["required"], json!(["TemplatePath"]));

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

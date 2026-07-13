use super::operation_descriptors::native_operation_descriptor;
use super::{ToolHandler, ToolSpec};
use crate::domain::project_sources::{discover_project_source_map, SourceFormat};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

const COMMON_ARGS: &[&str] = &["cwd", "dryRun", "confirm"];

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
const PROJECT_DISCOVERY_ARGS: &[&str] = &[
    "limit",
    "objects",
    "proposedExtensionPoints",
    "sourceDir",
    "task",
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

    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required_args(tool),
    })
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
    validate_code_arguments(tool, args, dry_run)?;
    validate_project_discovery_arguments(tool, args)?;
    validate_meta_edit_arguments(tool, args)?;
    validate_form_add_arguments(tool, args)?;
    validate_template_add_arguments(tool, args)?;
    validate_support_arguments(tool, args, dry_run)?;

    if !dry_run {
        for required in required_args(&tool) {
            if !args.contains_key(required) {
                return Err(format!("{} requires `{required}` argument", tool.name));
            }
        }
    }

    Ok(())
}

fn validate_project_discovery_arguments(
    tool: ToolSpec,
    args: &Map<String, Value>,
) -> Result<(), String> {
    if !matches!(tool.handler, ToolHandler::ProjectDiscovery) {
        return Ok(());
    }

    let task = args
        .get("task")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} requires `task` argument", tool.name))?;
    let task_len = task.chars().count();
    if task.trim().is_empty() || task_len > 4096 || task.chars().any(char::is_control) {
        return Err(format!(
            "{} argument `task` must contain 1..=4096 printable characters",
            tool.name
        ));
    }

    if let Some(cwd) = args.get("cwd") {
        let cwd = cwd
            .as_str()
            .ok_or_else(|| format!("{} argument `cwd` must be string", tool.name))?;
        if cwd.trim().is_empty() || cwd.chars().count() > 4096 || cwd.chars().any(char::is_control)
        {
            return Err(format!(
                "{} argument `cwd` must contain 1..=4096 printable characters",
                tool.name
            ));
        }
    }

    if let Some(source_dir) = args.get("sourceDir") {
        let source_dir = source_dir
            .as_str()
            .ok_or_else(|| format!("{} argument `sourceDir` must be string", tool.name))?;
        if source_dir.trim().is_empty()
            || source_dir.chars().count() > 4096
            || source_dir.chars().any(char::is_control)
        {
            return Err(format!(
                "{} argument `sourceDir` must contain 1..=4096 printable characters",
                tool.name
            ));
        }
    }

    for key in ["objects", "proposedExtensionPoints"] {
        let Some(value) = args.get(key) else {
            continue;
        };
        let items = value
            .as_array()
            .ok_or_else(|| format!("{} argument `{key}` must be array", tool.name))?;
        if items.is_empty() || items.len() > 32 {
            return Err(format!(
                "{} argument `{key}` must contain 1..=32 strings",
                tool.name
            ));
        }
        for item in items {
            let item = item
                .as_str()
                .ok_or_else(|| format!("{} argument `{key}` must contain strings", tool.name))?;
            if item.trim().is_empty() || item.chars().count() > 512 {
                return Err(format!(
                    "{} argument `{key}` items must contain 1..=512 characters",
                    tool.name
                ));
            }
            if item.chars().any(char::is_control)
                || item.contains('/')
                || item.contains('\\')
                || !is_metadata_reference(item)
            {
                return Err(format!(
                    "{} argument `{key}` items must be canonical dotted metadata references, not paths",
                    tool.name
                ));
            }
        }
    }

    if let Some(limit) = args.get("limit") {
        let limit = limit
            .as_i64()
            .ok_or_else(|| format!("{} argument `limit` must be integer", tool.name))?;
        if !(1..=50).contains(&limit) {
            return Err(format!(
                "{} argument `limit` must be between 1 and 50",
                tool.name
            ));
        }
    }
    Ok(())
}

fn is_metadata_reference(value: &str) -> bool {
    let segments = value.split('.').collect::<Vec<_>>();
    segments.len() >= 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|character| character.is_alphanumeric() || character == '_')
        })
}

fn validate_form_add_arguments(tool: ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    if tool.name != "unica.form.add" {
        return Ok(());
    }
    validate_unique_alias_group(tool.name, args, &["SetDefault", "setDefault"])
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

pub fn validate_workspace_paths(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    context: &WorkspaceContext,
) -> Result<(), String> {
    if dry_run {
        return Ok(());
    }
    if !is_native_xml_tool(tool) && !matches!(tool.handler, ToolHandler::RuntimeAdapter) {
        return Ok(());
    }

    let write_args = write_path_args(tool);
    if write_args.is_empty() {
        return Ok(());
    }

    let policy = WorkspacePathPolicy::new(context);
    for key in write_args {
        if let Some(Value::String(path)) = args.get(*key) {
            policy.resolve_write(path.as_str())?;
        }
    }
    Ok(())
}

pub fn validate_native_source_set_format(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    context: &WorkspaceContext,
) -> Result<(), String> {
    if dry_run || !is_native_xml_tool(tool) {
        return Ok(());
    }

    let source_map = discover_project_source_map(&context.workspace_root)?;
    if source_map.source_sets.is_empty() {
        return Ok(());
    }

    for key in native_source_path_args(tool) {
        let Some(Value::String(raw_path)) = args.get(*key) else {
            continue;
        };
        let target = resolve_read_path(&context.cwd, raw_path);
        let Some(source_set) = source_map
            .source_sets
            .iter()
            .filter(|source_set| {
                let source_root = normalize_lexical(&context.workspace_root.join(&source_set.path));
                target.starts_with(source_root)
            })
            .max_by_key(|source_set| source_set.path.len())
        else {
            continue;
        };

        match source_set.source_format {
            SourceFormat::Edt => {
                return Err(format!(
                    "{} targets source-set `{}` with sourceFormat=edt; native platform XML tools require sourceFormat=platform_xml",
                    tool.name, source_set.name
                ));
            }
            SourceFormat::Invalid => {
                return Err(format!(
                    "{} targets source-set `{}` with invalid/ambiguous format; native platform XML tools require sourceFormat=platform_xml",
                    tool.name, source_set.name
                ));
            }
            SourceFormat::PlatformXml | SourceFormat::Unknown => {}
        }
    }

    Ok(())
}

fn write_path_args(tool: ToolSpec) -> &'static [&'static str] {
    match tool.handler {
        ToolHandler::NativeOperation { operation, .. } => native_operation_descriptor(operation)
            .map(|descriptor| descriptor.write_path_args)
            .unwrap_or(&[]),
        ToolHandler::RuntimeAdapter => &["config", "path", "output", "settings", "mcpConfig"],
        _ => &[],
    }
}

fn is_native_xml_tool(tool: ToolSpec) -> bool {
    matches!(tool.handler, ToolHandler::NativeOperation { .. })
}

fn native_source_path_args(tool: ToolSpec) -> &'static [&'static str] {
    match tool.handler {
        ToolHandler::NativeOperation { operation, .. } => native_operation_descriptor(operation)
            .map(|descriptor| descriptor.source_path_args)
            .unwrap_or(&[]),
        _ => &[],
    }
}

fn resolve_read_path(cwd: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        normalize_lexical(&path)
    } else {
        normalize_lexical(&cwd.join(path))
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn allowed_args(tool: &ToolSpec) -> Vec<&'static str> {
    let mut names = COMMON_ARGS.to_vec();
    match tool.handler {
        ToolHandler::NativeOperation { operation, .. } => {
            names.extend(native_args_for(operation));
        }
        ToolHandler::BuildRuntime { .. } => names.extend(BUILD_ARGS),
        ToolHandler::RuntimeAdapter => names.extend(RUNTIME_ARGS),
        ToolHandler::CodeAdapter { .. } => names.extend(code_args_for(tool.name)),
        ToolHandler::StandardsAdapter { .. } => names.extend(STANDARDS_ARGS),
        ToolHandler::ProjectDiscovery => names.extend(PROJECT_DISCOVERY_ARGS),
        ToolHandler::ProjectStatus | ToolHandler::ProjectMap => {}
    }
    names.sort_unstable();
    names.dedup();
    names
}

fn native_args_for(_operation: &str) -> &'static [&'static str] {
    NATIVE_XML_DSL_ARGS
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
        ToolHandler::ProjectDiscovery => vec!["task"],
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

fn property_schema(name: &str) -> Value {
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
            | "webClient"
            | "includeMethods"
            | "ignoreCase"
            | "regex"
    ) {
        "boolean"
    } else if matches!(
        name,
        "limit"
            | "Offset"
            | "offset"
            | "MaxParams"
            | "maxParams"
            | "mcpPort"
            | "maxOutputTokens"
            | "maxFiles"
            | "rangeStart"
            | "rangeEnd"
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
    if matches!(tool.handler, ToolHandler::ProjectDiscovery) {
        return match name {
            "task" => json!({ "type": "string", "minLength": 1, "maxLength": 4096 }),
            "objects" | "proposedExtensionPoints" => json!({
                "type": "array",
                "minItems": 1,
                "maxItems": 32,
                "items": { "type": "string", "minLength": 1, "maxLength": 512 }
            }),
            "limit" => json!({ "type": "integer", "minimum": 1, "maximum": 50 }),
            _ => property_schema(name),
        };
    }
    if tool.name == "unica.meta.edit" && matches!(name, "Operation" | "operation") {
        return json!({ "type": "string", "enum": META_EDIT_OPERATIONS });
    }
    if matches!(tool.handler, ToolHandler::RuntimeAdapter) {
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
            | "webClient"
            | "includeMethods"
            | "ignoreCase"
            | "regex"
    ) {
        Some("boolean")
    } else if matches!(
        key,
        "limit"
            | "Offset"
            | "offset"
            | "MaxParams"
            | "maxParams"
            | "mcpPort"
            | "maxOutputTokens"
            | "maxFiles"
            | "rangeStart"
            | "rangeEnd"
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
    fn project_discovery_contract_is_typed_and_rejects_ambiguous_input() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.project.discover")
            .expect("project discovery tool exists");

        let schema = input_schema_for_tool(&tool);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["task"]));
        assert_eq!(schema["properties"]["task"]["type"], "string");
        assert_eq!(schema["properties"]["objects"]["type"], "array");
        assert_eq!(schema["properties"]["objects"]["items"]["type"], "string");
        assert_eq!(
            schema["properties"]["proposedExtensionPoints"]["items"]["type"],
            "string"
        );
        assert!(schema["properties"].get("args").is_none());

        for invalid in [
            json!({}),
            json!({"task": "   "}),
            json!({"task": "Контроль серий", "objects": [1]}),
            json!({"task": "Контроль серий", "objects": [""]}),
            json!({"task": "Контроль серий", "objects": ["Catalog"]}),
            json!({"task": "Контроль серий", "objects": ["Catalog.Secret!"]}),
            json!({"task": "Контроль серий", "objects": ["../Catalog.Secret"]}),
            json!({"task": "Контроль серий", "sourceDir": 1}),
            json!({"task": "Контроль серий", "sourceDir": " "}),
            json!({"task": "Контроль серий", "cwd": 1}),
            json!({"task": "Контроль серий", "cwd": " "}),
            json!({"task": "Контроль серий", "proposedExtensionPoints": [" "]}),
            json!({"task": "Контроль серий", "limit": 0}),
            json!({"task": "Контроль серий", "limit": 51}),
            json!({"task": "Контроль серий", "limit": "20"}),
        ] {
            let invalid = invalid.as_object().unwrap().clone();
            assert!(validate_tool_arguments(tool, &invalid, false).is_err());
        }

        let valid = json!({
            "task": "Контроль остаточного срока годности при поступлении",
            "objects": ["Document.ПриобретениеТоваровУслуг"],
            "proposedExtensionPoints": ["Document.ПриобретениеТоваровУслуг.TabularSection.Товары.Серия"],
            "limit": 20
        });
        validate_tool_arguments(tool, valid.as_object().unwrap(), false).unwrap();

        let empty = Map::new();
        assert!(
            validate_tool_arguments(tool, &empty, true).is_err(),
            "dryRun must not bypass discovery input validation"
        );

        let too_many = json!({
            "task": "Контроль серий",
            "objects": (0..33).map(|index| format!("Catalog.Item{index}")).collect::<Vec<_>>()
        });
        assert!(validate_tool_arguments(tool, too_many.as_object().unwrap(), false).is_err());
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
    fn mutating_dry_run_does_not_require_payload() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.form.edit")
            .unwrap();
        let args = Map::new();

        validate_tool_arguments(tool, &args, true).unwrap();
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
    fn skd_info_contract_exposes_raw_query_export() {
        let skd_info = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.skd.info")
            .expect("unica.skd.info must be registered");

        let schema = input_schema_for_tool(&skd_info);
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
        validate_tool_arguments(skd_info, &args, false).unwrap();
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
        assert_eq!(schema["properties"]["codes"]["type"], "array");
        assert_eq!(schema["properties"]["rangeStart"]["type"], "integer");
        assert_eq!(schema["properties"]["maxFiles"]["type"], "integer");
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
    }
}

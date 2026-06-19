use super::{ToolHandler, ToolSpec};
use crate::domain::project_sources::{discover_project_source_map, SourceFormat};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

const COMMON_ARGS: &[&str] = &["cwd", "dryRun", "confirm"];

const NATIVE_XML_DSL_ARGS: &[&str] = &[
    "BaseForm",
    "Batch",
    "BodyLimit",
    "BorrowMainAttribute",
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
    "DataPath",
    "DefinitionFile",
    "Detailed",
    "ExtensionPath",
    "Expand",
    "Field",
    "Fields",
    "Force",
    "FormName",
    "FormPath",
    "Format",
    "FromObject",
    "InterceptorType",
    "JsonPath",
    "KeepFiles",
    "Kind",
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
    "ProcessorName",
    "Purpose",
    "RightsPath",
    "Section",
    "SetDefault",
    "SetMainSKD",
    "ShowDenied",
    "SrcDir",
    "SubsystemPath",
    "Synonym",
    "TemplateName",
    "TemplatePath",
    "TemplateType",
    "Type",
    "Value",
    "Vendor",
    "Version",
    "WithText",
    "baseForm",
    "batch",
    "bodyLimit",
    "borrowMainAttribute",
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
    "dataPath",
    "definitionFile",
    "detailed",
    "extensionPath",
    "expand",
    "field",
    "fields",
    "force",
    "formName",
    "formPath",
    "format",
    "fromObject",
    "interceptorType",
    "jsonPath",
    "keepFiles",
    "kind",
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
    "processorName",
    "purpose",
    "rightsPath",
    "section",
    "setDefault",
    "setMainSKD",
    "showDenied",
    "srcDir",
    "subsystemPath",
    "synonym",
    "templateName",
    "templatePath",
    "templateType",
    "type",
    "value",
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
    "builder",
    "clientMode",
    "config",
    "connection",
    "extension",
    "format",
    "fullRebuild",
    "mcpConfig",
    "mcpPort",
    "mode",
    "module",
    "object",
    "operation",
    "output",
    "path",
    "server",
    "settings",
    "sourceSet",
    "testRunner",
    "testScope",
    "thinClient",
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
];

const RUNTIME_STRING_ARGS: &[&str] = &[
    "builder",
    "clientMode",
    "config",
    "connection",
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
    "workdir",
];

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

    if !dry_run {
        for required in required_args(&tool) {
            if !args.contains_key(required) {
                return Err(format!("{} requires `{required}` argument", tool.name));
            }
        }
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
    if !RUNTIME_OPERATIONS.contains(&operation) {
        return Err(format!(
            "{tool_name} argument `operation` must be one of: {}",
            RUNTIME_OPERATIONS.join(", ")
        ));
    }

    if dry_run {
        return Ok(());
    }

    let required = match operation {
        "load" => &["path"][..],
        "make" => &["output"][..],
        "syntax" => &["mode"][..],
        "test" => &["testRunner"][..],
        "launch" => &["clientMode"][..],
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

pub fn validate_workspace_paths(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    context: &WorkspaceContext,
) -> Result<(), String> {
    if dry_run || !tool.mutating {
        return Ok(());
    }
    if !matches!(tool.handler, ToolHandler::NativeOperation { .. })
        && !matches!(tool.handler, ToolHandler::RuntimeAdapter)
    {
        return Ok(());
    }

    let policy = WorkspacePathPolicy::new(context);
    for key in write_path_args(tool) {
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
    if dry_run || !matches!(tool.handler, ToolHandler::NativeOperation { .. }) {
        return Ok(());
    }

    let source_map = discover_project_source_map(&context.workspace_root)?;
    if source_map.source_sets.is_empty() {
        return Ok(());
    }

    for key in native_source_path_args() {
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
        ToolHandler::NativeOperation { operation, .. } => match operation {
            "cf-init" | "cfe-init" => &["OutputDir", "outputDir"],
            "cf-edit" => &["ConfigPath", "configPath", "Path", "path"],
            "meta-compile" => &["OutputDir", "outputDir"],
            "meta-edit" => &["ObjectPath", "objectPath", "Path", "path"],
            "meta-remove" => &["ConfigDir", "configDir"],
            "form-add" => &["ObjectPath", "objectPath"],
            "form-compile" => &["OutputPath", "outputPath"],
            "form-edit" => &["FormPath", "formPath"],
            "form-remove" => &["SrcDir", "srcDir"],
            "interface-edit" => &["CIPath", "ciPath"],
            "subsystem-compile" => &["OutputDir", "outputDir", "Parent", "parent"],
            "subsystem-edit" => &["SubsystemPath", "subsystemPath"],
            "template-add" | "template-remove" => &["SrcDir", "srcDir"],
            "skd-compile" | "mxl-compile" => &["OutputPath", "outputPath"],
            "skd-edit" => &["TemplatePath", "templatePath"],
            "role-compile" => &["OutputDir", "outputDir"],
            "cfe-borrow" | "cfe-patch-method" => &["ExtensionPath", "extensionPath"],
            _ => &[],
        },
        ToolHandler::RuntimeAdapter => &["config", "path", "output", "settings", "mcpConfig"],
        _ => &[],
    }
}

fn native_source_path_args() -> &'static [&'static str] {
    &[
        "CIPath",
        "ConfigDir",
        "ConfigPath",
        "DataPath",
        "ExtensionPath",
        "FormPath",
        "JsonPath",
        "MetadataPath",
        "ModulePath",
        "ObjectPath",
        "OutFile",
        "OutputDir",
        "OutputPath",
        "Path",
        "RightsPath",
        "SrcDir",
        "SubsystemPath",
        "TemplatePath",
        "ciPath",
        "configDir",
        "configPath",
        "dataPath",
        "extensionPath",
        "formPath",
        "jsonPath",
        "metadataPath",
        "modulePath",
        "objectPath",
        "outFile",
        "outputDir",
        "outputPath",
        "path",
        "rightsPath",
        "srcDir",
        "subsystemPath",
        "templatePath",
    ]
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
        ToolHandler::ProjectStatus | ToolHandler::ProjectMap => {}
        ToolHandler::LegacyScript { .. } => names.extend(NATIVE_XML_DSL_ARGS),
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
        ToolHandler::NativeOperation { operation, .. } => match operation {
            "cf-info" | "cf-validate" => vec!["ConfigPath"],
            "cfe-diff" => vec!["ExtensionPath", "ConfigPath"],
            "cfe-validate" => vec!["ExtensionPath"],
            "meta-info" | "meta-validate" | "meta-edit" => vec!["ObjectPath"],
            "form-info" | "form-validate" | "form-edit" => vec!["FormPath"],
            "interface-validate" | "interface-edit" => vec!["CIPath"],
            "subsystem-info" | "subsystem-validate" | "subsystem-edit" => vec!["SubsystemPath"],
            "skd-info" | "skd-validate" | "skd-edit" => vec!["TemplatePath"],
            "mxl-info" | "mxl-validate" | "mxl-decompile" => vec!["TemplatePath"],
            "role-info" | "role-validate" => vec!["RightsPath"],
            _ => Vec::new(),
        },
        ToolHandler::StandardsAdapter {
            operation: "search",
            ..
        } => vec!["query"],
        ToolHandler::RuntimeAdapter => runtime_required_args(tool),
        ToolHandler::CodeAdapter { .. } => match tool.name {
            "unica.code.definition" => vec!["name"],
            "unica.code.outline" => vec!["path"],
            "unica.code.grep" => vec!["query"],
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
            | "NoValidate"
            | "noValidate"
            | "NoRole"
            | "noRole"
            | "WithText"
            | "withText"
            | "CreateIfMissing"
            | "createIfMissing"
            | "IsFunction"
            | "isFunction"
            | "fullRebuild"
            | "server"
            | "thinClient"
            | "includeMethods"
            | "ignoreCase"
            | "regex"
    ) {
        "boolean"
    } else if matches!(
        name,
        "limit" | "Offset" | "offset" | "MaxParams" | "maxParams" | "mcpPort"
    ) {
        "integer"
    } else if matches!(
        name,
        "codes" | "types" | "Fields" | "fields" | "Children" | "children"
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
    if matches!(tool.handler, ToolHandler::RuntimeAdapter) {
        match name {
            "operation" => return json!({ "type": "string", "enum": RUNTIME_OPERATIONS }),
            "clientMode" => {
                return json!({
                    "type": "string",
                    "enum": ["designer", "thin", "thick", "ordinary", "mcp", "mcp-va"]
                });
            }
            "testRunner" => return json!({ "type": "string", "enum": ["yaxunit", "va"] }),
            "testScope" => return json!({ "type": "string", "enum": ["all", "module"] }),
            _ => {}
        }
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
            | "NoValidate"
            | "noValidate"
            | "NoRole"
            | "noRole"
            | "WithText"
            | "withText"
            | "CreateIfMissing"
            | "createIfMissing"
            | "IsFunction"
            | "isFunction"
            | "fullRebuild"
            | "server"
            | "thinClient"
            | "includeMethods"
            | "ignoreCase"
            | "regex"
    ) {
        Some("boolean")
    } else if matches!(
        key,
        "limit" | "Offset" | "offset" | "MaxParams" | "maxParams" | "mcpPort"
    ) {
        Some("integer")
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
    fn mutating_dry_run_does_not_require_payload() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.form.edit")
            .unwrap();
        let args = Map::new();

        validate_tool_arguments(tool, &args, true).unwrap();
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
        assert_eq!(schema["properties"]["fullRebuild"]["type"], "boolean");
        assert_eq!(schema["properties"]["mcpPort"]["type"], "integer");
        assert!(schema["properties"]["operation"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("build")));
        assert!(schema["properties"]["clientMode"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("mcp-va")));
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
}

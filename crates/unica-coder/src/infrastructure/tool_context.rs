use crate::application::operation_descriptors::native_operation_descriptor;
use crate::application::{RuntimeJobAction, ToolHandler, ToolSpec};
use crate::domain::project_sources::{ProjectSourceMap, SourceFormat, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::project_sources::discover_project_source_map;
use serde_json::{Map, Value};
use std::path::{Component, Path, PathBuf};

pub(crate) fn validate_tool_context(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    context: &WorkspaceContext,
) -> Result<(), String> {
    validate_workspace_paths(tool, args, dry_run, context)?;
    validate_native_source_set_format(tool, args, dry_run, context)
}

fn validate_workspace_paths(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    context: &WorkspaceContext,
) -> Result<(), String> {
    if dry_run && !validates_compile_preview_like_apply(tool) && !is_external_init_tool(tool) {
        return Ok(());
    }
    if !is_native_xml_tool(tool)
        && !matches!(
            tool.handler,
            ToolHandler::RuntimeAdapter
                | ToolHandler::RuntimeJob {
                    action: RuntimeJobAction::Start
                }
        )
    {
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

fn validate_native_source_set_format(
    tool: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    context: &WorkspaceContext,
) -> Result<(), String> {
    if (dry_run && !validates_compile_preview_like_apply(tool) && !is_external_init_tool(tool))
        || !is_native_xml_tool(tool)
    {
        return Ok(());
    }

    let source_map = discover_project_source_map(&context.workspace_root)?;
    if source_map.source_sets.is_empty() && !is_external_init_tool(tool) {
        return Ok(());
    }

    if is_external_init_tool(tool) {
        validate_external_project_format(tool, &source_map)?;
    }

    for key in native_source_path_args(tool) {
        let Some(Value::String(raw_path)) = args.get(*key) else {
            continue;
        };
        let target = resolve_read_path(&context.cwd, raw_path);
        if is_external_init_tool(tool) {
            validate_external_init_destination(tool, &target, context, &source_map)?;
            continue;
        }
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

fn validates_compile_preview_like_apply(tool: ToolSpec) -> bool {
    matches!(
        tool.handler,
        ToolHandler::NativeOperation {
            operation: "form-compile" | "meta-compile" | "role-compile" | "subsystem-compile",
            ..
        }
    )
}

fn validate_external_project_format(
    tool: ToolSpec,
    source_map: &ProjectSourceMap,
) -> Result<(), String> {
    match source_map.configured_format_raw.as_deref() {
        None | Some("DESIGNER") => Ok(()),
        Some("EDT") => Err(format!(
            "{} requires v8project.yaml format=DESIGNER; format=EDT uses a different external-project layout",
            tool.name
        )),
        Some(other) => Err(format!(
            "{} requires v8project.yaml format to be exact `DESIGNER` (or omitted for the Designer default); got {other:?}",
            tool.name
        )),
    }
}

fn validate_external_init_destination(
    tool: ToolSpec,
    target: &Path,
    context: &WorkspaceContext,
    source_map: &ProjectSourceMap,
) -> Result<(), String> {
    reject_symlink_components(target, &context.workspace_root)?;
    let Some(expected_kind) = external_init_source_set_kind(tool) else {
        return Ok(());
    };

    let matching_source_set = source_map
        .source_sets
        .iter()
        .filter_map(|source_set| {
            let source_root = normalize_lexical(&context.workspace_root.join(&source_set.path));
            path_starts_with_case_insensitive(target, &source_root)
                .then_some((source_set, source_root))
        })
        .max_by_key(|(_, source_root)| source_root.components().count());
    let Some((source_set, source_root)) = matching_source_set else {
        return Ok(());
    };

    if target != source_root {
        let aliases_source_root = target.components().count() == source_root.components().count();
        if aliases_source_root
            || matches!(
                source_set.kind,
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
            )
        {
            return Err(format!(
                "{} must target the exact source-set root {} so v8-runner can discover top-level external descriptors; got {}",
                tool.name,
                source_root.display(),
                target.display()
            ));
        }
        return Ok(());
    }
    if source_set.kind != expected_kind {
        return Err(format!(
            "{} targets source-set `{}` of kind {:?}; expected {:?}",
            tool.name, source_set.name, source_set.kind, expected_kind
        ));
    }
    match source_set.source_format {
        SourceFormat::PlatformXml | SourceFormat::Unknown => Ok(()),
        SourceFormat::Edt => Err(format!(
            "{} targets source-set `{}` with sourceFormat=edt; native platform XML tools require sourceFormat=platform_xml",
            tool.name, source_set.name
        )),
        SourceFormat::Invalid => Err(format!(
            "{} targets source-set `{}` with invalid/ambiguous format; native platform XML tools require sourceFormat=platform_xml",
            tool.name, source_set.name
        )),
    }
}

fn reject_symlink_components(target: &Path, workspace_root: &Path) -> Result<(), String> {
    let workspace_root = normalize_lexical(workspace_root);
    let relative = target.strip_prefix(&workspace_root).map_err(|_| {
        format!(
            "external scaffold target is outside workspace root: {}",
            target.display()
        )
    })?;
    let mut current = workspace_root;
    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => {
                return Err(format!(
                    "external scaffold OutputDir must not traverse symlink: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!("failed to inspect {}: {error}", current.display()));
            }
        }
    }
    Ok(())
}

fn path_starts_with_case_insensitive(path: &Path, base: &Path) -> bool {
    let path_components = path.components().collect::<Vec<_>>();
    let base_components = base.components().collect::<Vec<_>>();
    path_components.len() >= base_components.len()
        && path_components
            .iter()
            .zip(base_components.iter())
            .all(|(left, right)| {
                left.as_os_str().to_string_lossy().to_lowercase()
                    == right.as_os_str().to_string_lossy().to_lowercase()
            })
}

fn write_path_args(tool: ToolSpec) -> &'static [&'static str] {
    match tool.handler {
        ToolHandler::NativeOperation { operation, .. } => native_operation_descriptor(operation)
            .map(|descriptor| descriptor.write_path_args)
            .unwrap_or(&[]),
        ToolHandler::RuntimeAdapter => &["config", "path", "output", "settings", "mcpConfig"],
        ToolHandler::RuntimeJob {
            action: RuntimeJobAction::Start,
        } => &["config", "path", "output", "settings", "mcpConfig"],
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

fn is_external_init_tool(tool: ToolSpec) -> bool {
    matches!(tool.name, "unica.epf.init" | "unica.erf.init")
}

fn external_init_source_set_kind(tool: ToolSpec) -> Option<SourceSetKind> {
    match tool.name {
        "unica.epf.init" => Some(SourceSetKind::ExternalProcessor),
        "unica.erf.init" => Some(SourceSetKind::ExternalReport),
        _ => None,
    }
}

use crate::application::operation_descriptors::native_operation_descriptor;
use crate::application::{RuntimeJobAction, ToolHandler, ToolSpec};
use crate::domain::project_sources::{ProjectSourceMap, SourceFormat, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::project_sources::discover_project_source_map;
use crate::infrastructure::source_roots::deepest_source_set_matches;
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
    if dry_run && !validates_compile_preview_like_apply(tool) && !is_initializer_tool(tool) {
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
    if (dry_run && !validates_compile_preview_like_apply(tool) && !is_initializer_tool(tool))
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

    if let Some(expected_kind) = initializer_source_set_kind(tool) {
        for key in write_path_args(tool) {
            let Some(Value::String(raw_path)) = args.get(*key) else {
                continue;
            };
            let target = resolve_read_path(&context.cwd, raw_path);
            validate_initializer_destination(tool, &target, context, &source_map, expected_kind)?;
        }
    }

    for key in native_source_path_args(tool) {
        let Some(Value::String(raw_path)) = args.get(*key) else {
            continue;
        };
        if initializer_source_set_kind(tool).is_some() && write_path_args(tool).contains(key) {
            continue;
        }
        let target = resolve_read_path(&context.cwd, raw_path);
        let matches = source_map
            .source_sets
            .iter()
            .filter_map(|source_set| {
                let source_root = normalize_lexical(&context.workspace_root.join(&source_set.path));
                target
                    .starts_with(&source_root)
                    .then_some((source_set, source_root))
            })
            .collect::<Vec<_>>();
        for (source_set, _) in deepest_source_set_matches(matches) {
            validate_platform_xml_source_format(tool, source_set)?;
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

fn validate_initializer_destination(
    tool: ToolSpec,
    target: &Path,
    context: &WorkspaceContext,
    source_map: &ProjectSourceMap,
    expected_kind: SourceSetKind,
) -> Result<(), String> {
    reject_symlink_components(target, &context.workspace_root)?;
    let matching_source_sets = source_map
        .source_sets
        .iter()
        .filter_map(|source_set| {
            let source_root = normalize_lexical(&context.workspace_root.join(&source_set.path));
            path_starts_with_case_insensitive(target, &source_root)
                .then_some((source_set, source_root))
        })
        .collect::<Vec<_>>();
    let matching_source_sets = deepest_source_set_matches(matching_source_sets);
    if matching_source_sets.is_empty() {
        return Ok(());
    }

    for (source_set, source_root) in &matching_source_sets {
        validate_platform_xml_source_format(tool, source_set)?;
        let aliases_source_root = target.components().count() == source_root.components().count();
        let targets_source_root = target == source_root || aliases_source_root;
        if is_external_init_tool(tool) && aliases_source_root && target != source_root {
            return Err(format!(
                "{} must target the exact source-set root {} so v8-runner can discover top-level external descriptors; got {}",
                tool.name,
                source_root.display(),
                target.display()
            ));
        }
        let nested_in_external_artifact_set = !targets_source_root
            && matches!(
                source_set.kind,
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
            );
        if source_set.kind != expected_kind
            && (targets_source_root || nested_in_external_artifact_set)
        {
            return Err(format!(
                "{} targets source-set `{}` of kind {:?}; expected {:?}",
                tool.name, source_set.name, source_set.kind, expected_kind
            ));
        }
        if source_set.kind == expected_kind && is_external_init_tool(tool) && target != source_root
        {
            return Err(format!(
                "{} must target the exact source-set root {} so v8-runner can discover top-level external descriptors; got {}",
                tool.name,
                source_root.display(),
                target.display()
            ));
        }
    }
    Ok(())
}

fn validate_platform_xml_source_format(
    tool: ToolSpec,
    source_set: &crate::domain::project_sources::ProjectSourceSet,
) -> Result<(), String> {
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
        ToolHandler::RuntimeAdapter => &[
            "config",
            "path",
            "output",
            "stderrOutput",
            "settings",
            "mcpConfig",
        ],
        ToolHandler::RuntimeJob {
            action: RuntimeJobAction::Start,
        } => &[
            "config",
            "path",
            "output",
            "stderrOutput",
            "settings",
            "mcpConfig",
        ],
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

fn is_initializer_tool(tool: ToolSpec) -> bool {
    initializer_source_set_kind(tool).is_some()
}

fn initializer_source_set_kind(tool: ToolSpec) -> Option<SourceSetKind> {
    match tool.handler {
        ToolHandler::NativeOperation {
            operation: "cf-init",
            ..
        } => Some(SourceSetKind::Configuration),
        ToolHandler::NativeOperation {
            operation: "cfe-init",
            ..
        } => Some(SourceSetKind::Extension),
        ToolHandler::NativeOperation {
            operation: "epf-init",
            ..
        } => Some(SourceSetKind::ExternalProcessor),
        ToolHandler::NativeOperation {
            operation: "erf-init",
            ..
        } => Some(SourceSetKind::ExternalReport),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::tools;
    use crate::domain::cache::CacheAccess;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(label: &str) -> (PathBuf, WorkspaceContext) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-tool-context-{label}-{}-{nonce}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        (root, context)
    }

    fn runtime_write_tools() -> Vec<ToolSpec> {
        tools()
            .into_iter()
            .filter(|tool| {
                matches!(
                    tool.name,
                    "unica.runtime.execute" | "unica.runtime.job.start"
                )
            })
            .collect()
    }

    #[test]
    fn runtime_stderr_output_rejects_lexical_workspace_escape() {
        let (root, context) = fixture("stderr-lexical-escape");
        let args = json!({"stderrOutput": "../outside/stderr.log"})
            .as_object()
            .unwrap()
            .clone();

        for tool in runtime_write_tools() {
            let error = validate_tool_context(tool, &args, false, &context)
                .expect_err("stderrOutput must be protected by workspace write policy");
            assert!(error.contains("outside workspace root"), "{error}");
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_stderr_output_rejects_symlink_workspace_escape() {
        let (root, context) = fixture("stderr-symlink-escape");
        let outside = root.join("outside.log");
        let link = context.workspace_root.join("stderr.log");
        std::fs::write(&outside, "outside").unwrap();
        match create_file_link_fixture_for_test(&outside, &link).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                let _ = std::fs::remove_dir_all(root);
                return;
            }
        }
        let args = json!({"stderrOutput": "stderr.log"})
            .as_object()
            .unwrap()
            .clone();

        for tool in runtime_write_tools() {
            let error = validate_tool_context(tool, &args, false, &context)
                .expect_err("stderrOutput must not traverse a symlink outside the workspace");
            assert!(
                error.contains("through symlink outside workspace root"),
                "{error}"
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-tool-context-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temporary workspace must be created");
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn mutating_meta_edit_tool() -> ToolSpec {
        ToolSpec {
            name: "unica.meta.edit",
            description: "test tool",
            mutating: true,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::NativeOperation {
                operation: "meta-edit",
                event: None,
            },
        }
    }

    fn meta_edit_args(path: &str) -> Map<String, Value> {
        Map::from_iter([("ObjectPath".to_string(), Value::String(path.to_string()))])
    }

    #[test]
    fn mutating_tool_context_defers_equal_depth_owner_ambiguity_to_format_guard() {
        let context = temp_context("ambiguous-same-root");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src/Demo/Ext")).unwrap();
        fs::write(
            context.cwd.join("src/Configuration.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            context.cwd.join("src/Demo.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();

        validate_tool_context(
            mutating_meta_edit_tool(),
            &meta_edit_args("src/Demo/Ext/ObjectModule.bsl"),
            false,
            &context,
        )
        .expect("ownership ambiguity belongs to the structured format guard");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mutating_tool_context_keeps_source_format_guard_for_equal_depth_matches() {
        let context = temp_context("ambiguous-same-root-edt");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src/Demo/Ext")).unwrap();
        fs::write(context.cwd.join("src/.project"), "<projectDescription/>").unwrap();
        fs::write(
            context.cwd.join("src/Demo.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();

        let error = validate_tool_context(
            mutating_meta_edit_tool(),
            &meta_edit_args("src/Demo/Ext/ObjectModule.bsl"),
            false,
            &context,
        )
        .expect_err("ownership deferral must not bypass EDT/invalid source-format guards");

        assert!(
            error.contains("sourceFormat=edt") || error.contains("invalid/ambiguous format"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn mutating_tool_context_keeps_the_unique_deepest_nested_source_set() {
        let context = temp_context("unique-deepest");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: outer\n    type: CONFIGURATION\n    path: src\n  - name: nested\n    type: CONFIGURATION\n    path: src/nested\n",
        )
        .unwrap();
        fs::create_dir_all(context.cwd.join("src/nested/Catalogs")).unwrap();
        fs::write(context.cwd.join("src/.project"), "<projectDescription/>").unwrap();
        fs::write(
            context.cwd.join("src/nested/Configuration.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();

        validate_tool_context(
            mutating_meta_edit_tool(),
            &meta_edit_args("src/nested/Catalogs/Items.xml"),
            false,
            &context,
        )
        .expect("unique deepest platform XML source-set must remain valid");

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

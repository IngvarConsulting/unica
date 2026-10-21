#![allow(dead_code, unused_imports)]

use crate::application::operation_descriptors::TEMPLATE_PATH;
use crate::infrastructure::form_composition::*;
use crate::infrastructure::interface_composition::*;
use crate::infrastructure::native_operations::logical_selector::{
    logical_selection, physical_selection, typed_reader_metadata_target, AttachedResource,
    ResolvedReadTarget,
};
use crate::infrastructure::subsystem_composition::*;
use crate::infrastructure::support_state::WorkspaceSupportStateReader;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use unica_address::address::QualifiedAddress;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_forms::facts::*;
use unica_forms_designer::codec::*;
use unica_metadata::interface::*;
use unica_metadata::subsystem::*;
use unica_metadata::support_state::{
    ObjectSupportData as DomainObjectSupportData, SupportStateReader,
};
use unica_metadata_designer::interface::*;
use unica_metadata_designer::subsystem::*;
use unica_source_contract::discovery::{FollowMode, ProjectDiscoveryRead};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::owner::DCS_ROOT;
use unica_templates::mxl::TEMPLATE_KINDS;
use unica_workspace::discovery::ProjectDiscoveryReader;

use super::common::*;

use super::{cf::*, cfe::*, meta::*, role::*};

pub(crate) fn typed_dcs_reader_target(
    address: &QualifiedAddress,
) -> Option<unica_address::source_target::MetadataAddress> {
    typed_reader_metadata_target(address, TEMPLATE_KINDS)
}

pub(crate) struct DcsInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<unica_dcs::facts::DcsFacts>,
}

pub(crate) fn analyze_dcs_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> AdapterOutcome {
    analyze_dcs_info_with_data(args, context, support_reader).outcome
}

pub(crate) fn analyze_dcs_info_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> DcsInfoExecution {
    let result = (|| -> Result<(unica_dcs::facts::DcsFacts, PathBuf), String> {
        let selection = resolve_dcs_info_target(args, context)?;
        let template_path = selection.resource_path;
        let reader = ProjectDiscoveryReader::new(&context.cwd);
        let resolved_path = reader
            .canonicalize(&template_path)
            .unwrap_or_else(|_| template_path.clone());
        let text = unica_source_designer::read::read_utf8_sig(&reader, &resolved_path)?;
        let support = support_reader
            .object_support(&selection.target)
            .map_err(|error| error.to_string())?;
        let data = crate::infrastructure::family_codecs::dcs_codec().read(&text, support)?;
        Ok((data, resolved_path))
    })();

    match result {
        Ok((data, artifact)) => DcsInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.dcs.info described {} data set(s) and {} parameter(s)",
                    data.data_sets.len(),
                    data.parameters.len()
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![artifact.display().to_string()],
                stdout: None,
                stderr: None,
                command: None,
            },
            data: Some(data),
        },
        Err(error) => DcsInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.dcs.info failed in native DCS inspector".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

/// Inert read-path inspection facts: the resolved schema and every file the
/// resolution read, so the format guard inspects exactly what the reader uses.
pub(crate) struct DcsInfoPathInspection {
    pub(crate) resolution: Result<PathBuf, String>,
    pub(crate) dependencies: Vec<PathBuf>,
}

fn is_xml(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("xml"))
}

fn is_directory(reader: &dyn ProjectDiscoveryRead, path: &Path) -> bool {
    reader
        .inspect_input(path, FollowMode::Follow)
        .is_ok_and(|metadata| metadata.is_dir())
}

fn exists(reader: &dyn ProjectDiscoveryRead, path: &Path) -> bool {
    reader.inspect_input(path, FollowMode::Follow).is_ok()
}

fn inspect_dcs_info_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> DcsInfoPathInspection {
    // A logical target is already proven down to the file, so none of the
    // `Ext/Template.xml` probing below applies. The resolved resource is still
    // a format dependency: leaving the list empty would let a logical call
    // reach the reader without the guard ever inspecting the file it reads.
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        let resolution = selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
        let dependencies = resolution.as_ref().ok().cloned().into_iter().collect();
        return DcsInfoPathInspection {
            resolution,
            dependencies,
        };
    }
    let raw_path =
        match unica_operation_contract::input::required_path(args, TEMPLATE_PATH, "TemplatePath") {
            Ok(path) => path,
            Err(error) => {
                return DcsInfoPathInspection {
                    resolution: Err(error),
                    dependencies: Vec::new(),
                };
            }
        };
    let reader = ProjectDiscoveryReader::new(&context.cwd);
    let absolute = |path: PathBuf| unica_operation_contract::input::absolutize(path, &context.cwd);
    let original_path = raw_path.clone();
    let mut template_path = raw_path;
    let mut dependencies = Vec::new();
    if !is_xml(&template_path) {
        let candidate = template_path.join("Ext").join("Template.xml");
        if reader.path_is_file(&absolute(candidate.clone())) {
            template_path = candidate;
        }
    }

    if !reader.path_is_file(&absolute(template_path.clone())) && !is_xml(&template_path) {
        let templates_dir = absolute(original_path.join("Templates"));
        if is_directory(&reader, &templates_dir) {
            let failed =
                |err: std::io::Error| format!("failed to read {}: {err}", templates_dir.display());
            let entries = match reader
                .read_entries(&templates_dir)
                .and_then(|entries| entries.collect::<Result<Vec<_>, _>>())
            {
                Ok(entries) => entries,
                Err(err) => {
                    return DcsInfoPathInspection {
                        resolution: Err(failed(err)),
                        dependencies,
                    };
                }
            };
            let mut entries = entries;
            entries.sort_by_key(|entry| entry.file_name());
            let mut dcs_templates = Vec::<PathBuf>::new();
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("xml") {
                    continue;
                }
                dependencies.push(path.clone());
                let Ok(text) = unica_source_designer::read::read_utf8_sig(&reader, &path) else {
                    continue;
                };
                let template_type =
                    unica_templates_designer::descriptor::template_type_of(text.as_bytes());
                if template_type.as_deref() == Some("DataCompositionSchema") {
                    if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                        let template = templates_dir.join(stem).join("Ext").join("Template.xml");
                        if reader.path_is_file(&template) {
                            dcs_templates.push(template);
                        }
                    }
                }
            }
            if dcs_templates.len() == 1 {
                let resolved_path = dcs_templates.remove(0);
                dependencies.push(resolved_path.clone());
                return DcsInfoPathInspection {
                    resolution: Ok(resolved_path),
                    dependencies,
                };
            }
            let resolution = if dcs_templates.len() > 1 {
                format!(
                    "Multiple DCS templates found in: {}",
                    original_path.display()
                )
            } else {
                format!("No DCS templates found in: {}", original_path.display())
            };
            return DcsInfoPathInspection {
                resolution: Err(resolution),
                dependencies,
            };
        }
    }

    let abs_template = absolute(template_path);
    if !reader.path_is_file(&abs_template) {
        return DcsInfoPathInspection {
            resolution: Err(format!("File not found: {}", abs_template.display())),
            dependencies,
        };
    }
    dependencies.push(abs_template.clone());
    DcsInfoPathInspection {
        resolution: Ok(abs_template),
        dependencies,
    }
}

pub(crate) fn resolve_dcs_info_path_for_script(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    inspect_dcs_info_path(args, context).resolution
}

pub(crate) fn resolve_dcs_info_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedReadTarget, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        return selection.map_err(|failure| failure.to_string());
    }
    let template_path = resolve_dcs_info_path_for_script(args, context)?;
    physical_selection(&template_path, context, AttachedResource::Template)
        .map_err(|failure| failure.to_string())
}

pub(crate) fn dcs_info_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Vec<PathBuf> {
    inspect_dcs_info_path(args, context).dependencies
}

pub(crate) fn validate_dcs(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| {
        let template_path = resolve_dcs_validate_path(args, context)?;
        let reader = ProjectDiscoveryReader::new(&context.cwd);
        let resolved_path = reader.canonicalize(&template_path).unwrap_or(template_path);
        let text = unica_source_designer::read::read_utf8_sig(&reader, &resolved_path)?;
        Ok(check_dcs_text(args, &text, resolved_path))
    })();
    dcs_validation_outcome(result)
}

/// Validates the exact input already read through the canonical source authority.
/// The path is an artifact label; this entry point never reopens it.
pub(crate) fn validate_dcs_input(
    args: &Map<String, Value>,
    text: &str,
    artifact: PathBuf,
) -> AdapterOutcome {
    dcs_validation_outcome(Ok(check_dcs_text(args, text, artifact)))
}

fn check_dcs_text(
    args: &Map<String, Value>,
    text: &str,
    artifact: PathBuf,
) -> (unica_dcs::codec::DcsCheckReport, PathBuf) {
    let label = artifact
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let report = crate::infrastructure::family_codecs::dcs_codec().check(
        text,
        unica_dcs::codec::DcsCheckOptions::from_args(args),
        label,
    );
    (report, artifact)
}

fn dcs_validation_outcome(
    result: Result<(unica_dcs::codec::DcsCheckReport, PathBuf), String>,
) -> AdapterOutcome {
    match result {
        Ok((run, artifact)) => AdapterOutcome {
            ok: run.ok,
            summary: if run.ok {
                "unica.dcs.validate completed with native DCS validator".to_string()
            } else {
                "unica.dcs.validate failed in native DCS validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: run.errors,
            artifacts: vec![artifact.display().to_string()],
            stdout: Some(run.report),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.dcs.validate failed in native DCS validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn resolve_dcs_validate_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let reader = ProjectDiscoveryReader::new(&context.cwd);
    let raw_path =
        unica_operation_contract::input::required_path(args, TEMPLATE_PATH, "TemplatePath")?;
    let mut display_path = raw_path.clone();
    let mut template_path = unica_operation_contract::input::absolutize(raw_path, &context.cwd);

    if is_directory(&reader, &template_path) {
        display_path = display_path.join("Ext").join("Template.xml");
        template_path = template_path.join("Ext").join("Template.xml");
    }
    if !exists(&reader, &template_path)
        && display_path.file_name().and_then(|value| value.to_str()) == Some("Template.xml")
    {
        let display_candidate = display_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("Template.xml");
        let candidate = template_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("Ext")
            .join("Template.xml");
        if exists(&reader, &candidate) {
            display_path = display_candidate;
            template_path = candidate;
        }
    }
    if !exists(&reader, &template_path) && is_xml(&display_path) {
        if let Some(stem) = display_path.file_stem().and_then(|value| value.to_str()) {
            let display_candidate = display_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(stem)
                .join("Ext")
                .join("Template.xml");
            let candidate = template_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(stem)
                .join("Ext")
                .join("Template.xml");
            if exists(&reader, &candidate) {
                display_path = display_candidate;
                template_path = candidate;
            }
        }
    }
    if !exists(&reader, &template_path) {
        return Err(format!("File not found: {}", display_path.display()));
    }
    Ok(template_path)
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "dcs-info" => Some(Ok(analyze_dcs_info(
            args,
            context,
            &WorkspaceSupportStateReader::new(context),
        ))),
        "dcs-validate" => Some(Ok(validate_dcs(args, context))),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    //! Адаптер composition: путь `TemplatePath`, состояние поддержки владельца
    //! и метка проверенного входа. Факты и проверку схемы покрывают тесты
    //! кодека в `unica-dcs-designer`.
    use super::*;
    use serde_json::{json, Map};
    use std::fs;

    /// Пустая схема с верным корнем: кодек принимает её без замечаний.
    const MINIMAL_SCHEMA: &str =
        r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#;

    fn dcs_info_context() -> (tempfile::TempDir, WorkspaceContext, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: cwd.clone(),
            workspace_root: cwd.clone(),
            cache_root: cwd.join(".build/unica"),
            workspace_epoch: 0,
        };
        let source = context.workspace_root.join("src");
        let template_path = source.join("Reports/Sales/Templates/Dcs/Ext/Template.xml");
        fs::create_dir_all(template_path.parent().unwrap()).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Report>Sales</Report></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            source.join("Reports/Sales.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties><Name>Sales</Name></Properties><ChildObjects><Template>Dcs</Template></ChildObjects></Report></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            source.join("Reports/Sales/Templates/Dcs.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Dcs</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#,
        )
        .unwrap();
        (root, context, template_path)
    }

    #[test]
    fn dcs_info_rejects_wrong_root_namespace_without_output() {
        let (_root, context, template_path) = dcs_info_context();
        let out_file = context.cwd.join("info.txt");
        fs::write(
            &template_path,
            MINIMAL_SCHEMA.replace(
                "http://v8.1c.ru/8.1/data-composition-system/schema",
                "urn:not-dcs",
            ),
        )
        .unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!(template_path)),
            ("OutFile".to_string(), json!("info.txt")),
        ]);

        let outcome =
            analyze_dcs_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

        assert!(!outcome.ok, "{outcome:?}");
        assert!(!out_file.exists());
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("urn:not-dcs") && error.contains("DataCompositionSchema")));
    }

    /// The adapter passes the owner's support state read by the product
    /// support reader into the facts; the fixture has no support data.
    #[test]
    fn dcs_info_carries_the_owner_support_state_from_the_product_reader() {
        let (_root, context, template_path) = dcs_info_context();
        fs::write(&template_path, MINIMAL_SCHEMA).unwrap();

        let execution = analyze_dcs_info_with_data(
            &Map::from_iter([("TemplatePath".to_string(), json!(template_path))]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let data = execution.data.expect("dcs.info answers with data");
        assert_eq!(
            data.support.state,
            unica_metadata::support_state::ObjectSupportState::NotSupported,
            "{data:?}"
        );
    }

    #[test]
    fn verified_dcs_input_is_validated_without_reopening_the_artifact_path() {
        let root = tempfile::tempdir().unwrap();
        let artifact = root.path().join("Template.xml");
        let replacement = b"not the retained DCS input";
        fs::write(&artifact, replacement).unwrap();
        let result = validate_dcs_input(&Map::new(), MINIMAL_SCHEMA, artifact.clone());
        assert!(result.ok, "{result:?}");
        assert_eq!(fs::read(artifact).unwrap(), replacement);
    }
}

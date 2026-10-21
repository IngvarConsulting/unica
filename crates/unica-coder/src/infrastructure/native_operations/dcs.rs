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
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
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
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::owner::DCS_ROOT;
use unica_templates::mxl::TEMPLATE_KINDS;

use super::common::*;

use super::{cf::*, cfe::*, meta::*, role::*};

pub(crate) fn typed_dcs_reader_target(
    address: &QualifiedAddress,
) -> Option<unica_address::source_target::MetadataAddress> {
    typed_reader_metadata_target(address, TEMPLATE_KINDS)
}

pub(crate) struct DcsInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<unica_dcs_designer::schema::DcsInfoData>,
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
    let result = (|| -> Result<(unica_dcs_designer::schema::DcsInfoData, PathBuf), String> {
        let selection = resolve_dcs_info_target(args, context)?;
        let template_path = selection.resource_path;
        let resolved_path = template_path
            .canonicalize()
            .unwrap_or_else(|_| template_path.clone());
        let text = read_utf8_sig(&resolved_path)?;
        let support = support_reader
            .object_support(&selection.target)
            .map_err(|error| error.to_string())?;
        let data = unica_dcs_designer::schema::parse_dcs_info_xml(&text, support)?;
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

fn inspect_dcs_info_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> unica_dcs_designer::schema::DcsInfoPathInspection {
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
        return unica_dcs_designer::schema::DcsInfoPathInspection {
            resolution,
            dependencies,
        };
    }
    let raw_path =
        match unica_operation_contract::input::required_path(args, TEMPLATE_PATH, "TemplatePath") {
            Ok(path) => path,
            Err(error) => {
                return unica_dcs_designer::schema::DcsInfoPathInspection {
                    resolution: Err(error),
                    dependencies: Vec::new(),
                };
            }
        };
    let original_path = raw_path.clone();
    let mut template_path = raw_path.clone();
    let mut dependencies = Vec::new();
    if template_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| !value.eq_ignore_ascii_case("xml"))
        .unwrap_or(true)
    {
        let candidate = template_path.join("Ext").join("Template.xml");
        if unica_operation_contract::input::absolutize(candidate.clone(), &context.cwd).is_file() {
            template_path = candidate;
        }
    }

    let abs_template =
        unica_operation_contract::input::absolutize(template_path.clone(), &context.cwd);
    if !abs_template.is_file()
        && template_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| !value.eq_ignore_ascii_case("xml"))
            .unwrap_or(true)
    {
        let templates_dir = unica_operation_contract::input::absolutize(
            original_path.join("Templates"),
            &context.cwd,
        );
        if templates_dir.is_dir() {
            let mut dcs_templates = Vec::<PathBuf>::new();
            let entries = match fs::read_dir(&templates_dir) {
                Ok(entries) => entries,
                Err(err) => {
                    return unica_dcs_designer::schema::DcsInfoPathInspection {
                        resolution: Err(format!(
                            "failed to read {}: {err}",
                            templates_dir.display()
                        )),
                        dependencies,
                    };
                }
            };
            let mut entries = match entries.collect::<Result<Vec<_>, _>>() {
                Ok(entries) => entries,
                Err(err) => {
                    return unica_dcs_designer::schema::DcsInfoPathInspection {
                        resolution: Err(format!(
                            "failed to read {}: {err}",
                            templates_dir.display()
                        )),
                        dependencies,
                    };
                }
            };
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("xml") {
                    continue;
                }
                dependencies.push(path.clone());
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(doc) = Document::parse(text.trim_start_matches('\u{feff}')) else {
                    continue;
                };
                let template_type = doc
                    .descendants()
                    .find(|node| node.is_element() && node.tag_name().name() == "TemplateType")
                    .and_then(|node| node.text())
                    .unwrap_or("")
                    .trim();
                if template_type == "DataCompositionSchema" {
                    if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                        let template = templates_dir.join(stem).join("Ext").join("Template.xml");
                        if template.is_file() {
                            dcs_templates.push(template);
                        }
                    }
                }
            }
            if dcs_templates.len() == 1 {
                let resolved_path = dcs_templates.remove(0);
                dependencies.push(resolved_path.clone());
                return unica_dcs_designer::schema::DcsInfoPathInspection {
                    resolution: Ok(resolved_path),
                    dependencies,
                };
            }
            if dcs_templates.len() > 1 {
                return unica_dcs_designer::schema::DcsInfoPathInspection {
                    resolution: Err(format!(
                        "Multiple DCS templates found in: {}",
                        original_path.display()
                    )),
                    dependencies,
                };
            }
            return unica_dcs_designer::schema::DcsInfoPathInspection {
                resolution: Err(format!(
                    "No DCS templates found in: {}",
                    original_path.display()
                )),
                dependencies,
            };
        }
    }

    let abs_template = unica_operation_contract::input::absolutize(template_path, &context.cwd);
    if !abs_template.is_file() {
        return unica_dcs_designer::schema::DcsInfoPathInspection {
            resolution: Err(format!("File not found: {}", abs_template.display())),
            dependencies,
        };
    }
    dependencies.push(abs_template.clone());
    unica_dcs_designer::schema::DcsInfoPathInspection {
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
        let resolved_path = template_path.canonicalize().unwrap_or(template_path);
        let text = read_utf8_sig(&resolved_path)?;
        unica_dcs_designer::schema::validate_dcs_text(args, &text, resolved_path)
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
    dcs_validation_outcome(unica_dcs_designer::schema::validate_dcs_text(
        args, text, artifact,
    ))
}

fn dcs_validation_outcome(
    result: Result<unica_dcs_designer::schema::DcsValidationRun, String>,
) -> AdapterOutcome {
    match result {
        Ok(run) => AdapterOutcome {
            ok: run.ok,
            summary: if run.ok {
                "unica.dcs.validate completed with native DCS validator".to_string()
            } else {
                "unica.dcs.validate failed in native DCS validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: run.errors,
            artifacts: vec![run.artifact.display().to_string()],
            stdout: Some(run.stdout),
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
    let raw_path =
        unica_operation_contract::input::required_path(args, TEMPLATE_PATH, "TemplatePath")?;
    let mut display_path = raw_path.clone();
    let mut template_path = unica_operation_contract::input::absolutize(raw_path, &context.cwd);

    if template_path.is_dir() {
        display_path = display_path.join("Ext").join("Template.xml");
        template_path = template_path.join("Ext").join("Template.xml");
    }
    if !template_path.exists()
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
        if candidate.exists() {
            display_path = display_candidate;
            template_path = candidate;
        }
    }
    if !template_path.exists()
        && display_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("xml"))
            .unwrap_or(false)
    {
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
            if candidate.exists() {
                display_path = display_candidate;
                template_path = candidate;
            }
        }
    }
    if !template_path.exists() {
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
    use super::*;
    use serde_json::{json, Map};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    const TEST_DCS_SETTINGS_NS: &str = unica_dcs_designer::schema::DCS_SETTINGS_NS;
    const TEST_DCS_CORE_NS: &str = unica_dcs_designer::schema::DCS_CORE_NS;
    const TEST_DCS_COMMON_NS: &str = unica_dcs_designer::schema::DCS_COMMON_NS;
    fn dcs_info_context(name: &str) -> (WorkspaceContext, PathBuf) {
        let context = temp_context(name);
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
        (context, template_path)
    }

    #[test]
    fn dcs_info_rejects_wrong_root_namespace_without_output() {
        let (context, template_path) = dcs_info_context("dcs-info-wrong-root-ns");
        let out_file = context.cwd.join("info.txt");
        fs::write(
            &template_path,
            base_dcs_xml().replace(
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
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_info_raw_returns_exact_unpaginated_query_text() {
        let (context, template_path) = dcs_info_context("dcs-info-raw-query");
        let query = "  ВЫБРАТЬ 1 КАК Значение  \n|ОБЪЕДИНИТЬ ВСЕ\n|ВЫБРАТЬ 2  ";
        fs::write(
            &template_path,
            base_dcs_xml().replace(
                "<query>ВЫБРАТЬ Amount КАК Amount</query>",
                &format!("<query>{query}</query>"),
            ),
        )
        .unwrap();
        let args = Map::from_iter([
            ("TemplatePath".to_string(), json!(template_path)),
            ("Mode".to_string(), json!("query")),
            ("Name".to_string(), json!("НаборДанных1")),
            ("Raw".to_string(), json!(true)),
            ("Limit".to_string(), json!(1)),
            ("Offset".to_string(), json!(100)),
        ]);

        let execution = analyze_dcs_info_with_data(
            &args,
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        // `Raw` existed because pagination mangled the query; data carries the
        // exact text always, so Limit and Offset cannot truncate it and the
        // flag has nothing left to switch on.
        let data = execution.data.expect("dcs.info answers with data");
        let queries = data
            .data_sets
            .iter()
            .filter_map(|data_set| data_set.query.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(queries, vec![query], "{data:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Support has its own structured field and is not duplicated in overview prose.

    #[test]
    fn dcs_info_overview_does_not_duplicate_structured_support() {
        let xml = complete_dcs_xml();
        let doc = Document::parse(xml).unwrap();
        let mut lines = Vec::new();

        unica_dcs_designer::schema::dcs_info_overview(
            doc.root_element(),
            Path::new("Template.xml"),
            xml,
            &mut lines,
            unica_dcs_designer::schema::DCS_SCHEMA_NS,
            TEST_DCS_SETTINGS_NS,
        );

        assert!(
            lines.iter().all(|line| !line.starts_with("Поддержка:")),
            "support belongs to the typed `support` field, not overview prose: {lines:?}"
        );
    }

    #[test]
    fn dcs_info_data_carries_every_fact_the_retired_modes_printed() {
        let (context, template_path) = dcs_info_context("dcs-info-complete-read-model");
        fs::write(&template_path, complete_dcs_xml()).unwrap();

        let execution = analyze_dcs_info_with_data(
            &Map::from_iter([("TemplatePath".to_string(), json!(template_path))]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let data = execution.data.expect("dcs.info answers with data");

        // `overview` printed the owner's support state and the data sources.
        assert_eq!(
            data.support.state,
            unica_metadata::support_state::ObjectSupportState::NotSupported,
            "{data:?}"
        );
        assert_eq!(
            data.data_sources
                .iter()
                .map(|source| (source.name.as_str(), source.kind.as_deref()))
                .collect::<Vec<_>>(),
            vec![("ИсточникДанных1", Some("Local"))],
            "{data:?}"
        );

        // `overview` distinguished an Object dataset by its objectName, and
        // every dataset named the source it reads.
        let object_set = data
            .data_sets
            .iter()
            .find(|set| set.kind == "Object")
            .expect("the fixture declares an Object dataset");
        assert_eq!(object_set.object_name.as_deref(), Some("Справочник.Товары"));
        assert_eq!(
            data.data_sets[0].data_source.as_deref(),
            Some("ИсточникДанных1")
        );

        // `resources` printed the grouping a total belongs to; without it an
        // overall total and a per-group total look identical.
        assert_eq!(
            data.total_fields
                .iter()
                .map(|total| (total.data_path.as_str(), total.group.as_deref()))
                .collect::<Vec<_>>(),
            vec![("Сумма", Some("Товар")), ("Количество", None)],
            "{data:?}"
        );

        // `calculated -Name` printed which uses a field is barred from; the
        // element is structured, so a boolean cannot carry that answer.
        assert_eq!(
            data.calculated_fields[0].restrictions.as_deref(),
            Some(&["condition".to_string(), "order".to_string()][..]),
            "{data:?}"
        );

        // `params` marked a parameter that cannot be used as a field.
        assert_eq!(
            data.parameters[0].available_as_field,
            Some(false),
            "{data:?}"
        );

        // `variant` printed the name, presentation, selection, filters and the
        // structure with its grouping fields.
        let variant = &data.variants[0];
        assert_eq!(variant.name, "Основной");
        assert_eq!(variant.presentation.as_deref(), Some("Основной вариант"));
        assert_eq!(variant.selection, vec!["Товар".to_string()]);
        assert_eq!(variant.filters, 1);
        assert_eq!(
            variant
                .order
                .iter()
                .map(|item| (item.field.as_str(), item.direction.as_deref()))
                .collect::<Vec<_>>(),
            vec![("Сумма", Some("Desc"))]
        );
        assert_eq!(
            variant
                .structure
                .iter()
                .map(|item| (item.kind.as_str(), item.group_by.clone()))
                .collect::<Vec<_>>(),
            vec![("Group", vec!["Товар".to_string()])]
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn dcs_info_structure_items_preserve_names_unnamed_parents_duplicates_and_axes() {
        let xml = r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:s="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
        <settingsVariant><s:name>Variant</s:name><s:settings>
          <s:item xsi:type="s:StructureItemGroup"><s:item xsi:type="s:StructureItemGroup"><s:name>Nested</s:name></s:item></s:item>
          <s:item xsi:type="s:StructureItemGroup"/>
          <s:item xsi:type="s:StructureItemTable"><s:row><s:item xsi:type="s:StructureItemGroup"><s:name>Repeated</s:name></s:item></s:row><s:column><s:item xsi:type="s:StructureItemGroup"><s:name>Repeated</s:name></s:item></s:column></s:item>
          <s:item xsi:type="s:StructureItemChart"><s:point><s:item xsi:type="s:StructureItemGroup"><s:name>name with spaces</s:name></s:item></s:point><s:series><s:item xsi:type="s:StructureItemGroup"><s:name>Series</s:name></s:item></s:series></s:item>
          <s:selection><s:item xsi:type="s:SelectedItemField"><s:name>Decoy</s:name></s:item></s:selection>
          <s:item xsi:type="s:StructureItemNestedObject"><s:name>NestedObject</s:name><s:item xsi:type="s:StructureItemGroup"><s:name>NestedChild</s:name></s:item></s:item>
          <s:item xsi:type="s:StructureItemFuture"><s:name>Unknown</s:name><s:item xsi:type="s:StructureItemGroup"><s:name>FutureChild</s:name></s:item></s:item>
        </s:settings></settingsVariant></DataCompositionSchema>"#;
        let data = unica_dcs_designer::schema::parse_dcs_info_xml(
            xml,
            DomainObjectSupportData {
                state: unica_metadata::support_state::ObjectSupportState::NotSupported,
                direct_edit_safe: None,
            },
        )
        .unwrap();
        let value = serde_json::to_value(data).unwrap();
        let rows = value["variants"][0]["structureItems"]
            .as_array()
            .expect("structure names must be reachable without reading XML");
        assert_eq!(
            rows.iter()
                .map(|row| row["name"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!(null),
                json!("Nested"),
                json!(null),
                json!(null),
                json!("Repeated"),
                json!("Repeated"),
                json!(null),
                json!("name with spaces"),
                json!("Series"),
                json!("NestedObject"),
                json!("NestedChild"),
                json!("Unknown"),
                json!("FutureChild")
            ]
        );
        assert_eq!(rows[1]["parentIndex"], json!(0));
        assert_eq!(rows[4]["parentIndex"], json!(3));
        assert_eq!(rows[4]["axis"], json!("row"));
        assert_eq!(rows[5]["axis"], json!("column"));
        assert_eq!(rows[7]["axis"], json!("point"));
        assert_eq!(rows[8]["axis"], json!("series"));
        assert_eq!(rows[9]["kind"], json!("NestedObject"));
        assert_eq!(rows[10]["parentIndex"], json!(9));
        assert_eq!(rows[11]["kind"], json!("StructureItemFuture"));
        assert_eq!(rows[12]["parentIndex"], json!(11));
        assert!(rows.iter().all(|row| row.get("at").is_none()));
    }

    #[test]
    fn verified_dcs_input_is_validated_without_reopening_the_artifact_path() {
        let root = tempfile::tempdir().unwrap();
        let artifact = root.path().join("Template.xml");
        let replacement = b"not the retained DCS input";
        fs::write(&artifact, replacement).unwrap();
        let result = validate_dcs_input(&Map::new(), base_dcs_xml(), artifact.clone());
        assert!(result.ok, "{result:?}");
        assert_eq!(fs::read(artifact).unwrap(), replacement);
    }

    /// #311. A `SettingsParameterValue` with an empty `<dcscor:parameter>` is
    /// well-formed XML that names no parameter, so the variant carries no value
    /// for it. Validation reported `Validation OK` on exactly that shape.

    #[test]
    fn native_dcs_validate_rejects_a_data_parameter_without_a_name() {
        let context = temp_context("dcs-validate-empty-data-parameter");
        let template_path = context.cwd.join("Template.xml");
        fs::write(
            &template_path,
            base_dcs_xml().replace(
                "\t\t\t<dcsset:order>",
                "\t\t\t<dcsset:dataParameters>\n\t\t\t\t<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\">\n\t\t\t\t\t<dcscor:parameter></dcscor:parameter>\n\t\t\t\t</dcscor:item>\n\t\t\t</dcsset:dataParameters>\n\t\t\t<dcsset:order>",
            ),
        )
        .unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        let outcome = validate_dcs(&args, &context);

        let stdout = outcome.stdout.unwrap_or_default();
        assert!(!outcome.ok, "{stdout}");
        assert!(
            stdout.contains("SettingsParameterValue has empty parameter name"),
            "{stdout}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn native_dcs_validate_rejects_ref_type_bound_to_unexpected_namespace() {
        let context = temp_context("dcs-validate-bad-prefix");
        let template_path = context.cwd.join("Template.xml");
        fs::write(
            &template_path,
            base_dcs_xml().replace(
                "<field>Amount</field>",
                "<field>Amount</field>\n\t\t\t<valueType>\n\t\t\t\t<v8:Type xmlns:bad=\"http://example.com\">bad:CatalogRef.X</v8:Type>\n\t\t\t</valueType>",
            ),
        )
        .unwrap();

        let mut args = Map::new();
        args.insert("TemplatePath".to_string(), json!("Template.xml"));
        let outcome = validate_dcs(&args, &context);
        let stdout = outcome.stdout.unwrap_or_default();
        assert!(!outcome.ok, "{stdout}");
        assert!(
            stdout.contains("uses prefix 'bad' bound to unexpected namespace 'http://example.com'"),
            "{stdout}"
        );

        let _ = fs::remove_dir_all(&context.cwd);
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cwd = std::env::temp_dir().join(format!("unica-{name}-{nanos}"));
        fs::create_dir_all(&cwd).unwrap();
        WorkspaceContext {
            cwd: cwd.clone(),
            workspace_root: cwd.clone(),
            cache_root: cwd.join(".build/unica"),
            workspace_epoch: 0,
        }
    }

    /// One schema carrying a representative of every fact the retired `Mode`
    /// reports used to print. The settings elements deliberately sit in the
    /// `dcsset` namespace, which is where the platform puts them.
    fn complete_dcs_xml() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"
		xmlns:dcscor="http://v8.1c.ru/8.1/data-composition-system/core"
		xmlns:dcsset="http://v8.1c.ru/8.1/data-composition-system/settings"
		xmlns:xs="http://www.w3.org/2001/XMLSchema"
		xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
	<dataSource>
		<name>ИсточникДанных1</name>
		<dataSourceType>Local</dataSourceType>
	</dataSource>
	<dataSet xsi:type="DataSetQuery">
		<name>НаборЗапрос</name>
		<field xsi:type="DataSetFieldField">
			<dataPath>Товар</dataPath>
			<field>Товар</field>
		</field>
		<dataSource>ИсточникДанных1</dataSource>
		<query>ВЫБРАТЬ Товар КАК Товар</query>
	</dataSet>
	<dataSet xsi:type="DataSetObject">
		<name>НаборОбъект</name>
		<objectName>Справочник.Товары</objectName>
		<field xsi:type="DataSetFieldField">
			<dataPath>Наименование</dataPath>
			<field>Наименование</field>
		</field>
	</dataSet>
	<calculatedField>
		<dataPath>Наценка</dataPath>
		<expression>Сумма - Себестоимость</expression>
		<useRestriction>
			<condition>true</condition>
			<order>true</order>
		</useRestriction>
	</calculatedField>
	<totalField>
		<dataPath>Сумма</dataPath>
		<expression>Сумма(Сумма)</expression>
		<group>Товар</group>
	</totalField>
	<totalField>
		<dataPath>Количество</dataPath>
		<expression>Сумма(Количество)</expression>
	</totalField>
	<parameter>
		<name>Период</name>
		<availableAsField>false</availableAsField>
	</parameter>
	<settingsVariant>
		<dcsset:name>Основной</dcsset:name>
		<dcsset:presentation xsi:type="xs:string">Основной вариант</dcsset:presentation>
		<dcsset:settings>
			<dcsset:selection>
				<dcsset:item xsi:type="dcsset:SelectedItemField">
					<dcsset:field>Товар</dcsset:field>
				</dcsset:item>
			</dcsset:selection>
			<dcsset:filter>
				<dcsset:item xsi:type="dcsset:FilterItemComparison">
					<dcsset:left xsi:type="dcscor:Field">Товар</dcsset:left>
					<dcsset:comparisonType>Equal</dcsset:comparisonType>
				</dcsset:item>
			</dcsset:filter>
			<dcsset:order>
				<dcsset:item xsi:type="dcsset:OrderItemField">
					<dcsset:field>Сумма</dcsset:field>
					<dcsset:orderType>Desc</dcsset:orderType>
				</dcsset:item>
			</dcsset:order>
			<dcsset:item xsi:type="dcsset:StructureItemGroup">
				<dcsset:groupItems>
					<dcsset:item xsi:type="dcsset:GroupItemField">
						<dcsset:field>Товар</dcsset:field>
					</dcsset:item>
				</dcsset:groupItems>
			</dcsset:item>
		</dcsset:settings>
	</settingsVariant>
</DataCompositionSchema>
"#
    }

    fn base_dcs_xml() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"
		xmlns:dcscom="http://v8.1c.ru/8.1/data-composition-system/common"
		xmlns:dcscor="http://v8.1c.ru/8.1/data-composition-system/core"
		xmlns:dcsset="http://v8.1c.ru/8.1/data-composition-system/settings"
		xmlns:v8="http://v8.1c.ru/8.1/data/core"
		xmlns:v8ui="http://v8.1c.ru/8.1/data/ui"
		xmlns:xs="http://www.w3.org/2001/XMLSchema"
		xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
	<dataSource>
		<name>ИсточникДанных1</name>
		<dataSourceType>Local</dataSourceType>
	</dataSource>
	<dataSet xsi:type="DataSetQuery">
		<name>НаборДанных1</name>
		<field xsi:type="DataSetFieldField">
			<dataPath>Amount</dataPath>
			<field>Amount</field>
		</field>
		<dataSource>ИсточникДанных1</dataSource>
		<query>ВЫБРАТЬ Amount КАК Amount</query>
	</dataSet>
	<settingsVariant>
		<dcsset:name>Основной</dcsset:name>
		<dcsset:settings>
			<dcsset:selection>
			</dcsset:selection>
			<dcsset:filter>
			</dcsset:filter>
			<dcsset:order>
			</dcsset:order>
			<dcsset:item xsi:type="dcsset:StructureItemGroup">
				<dcsset:selection>
					<dcsset:item xsi:type="dcsset:SelectedItemAuto"/>
				</dcsset:selection>
			</dcsset:item>
		</dcsset:settings>
	</settingsVariant>
</DataCompositionSchema>
"#
    }
}

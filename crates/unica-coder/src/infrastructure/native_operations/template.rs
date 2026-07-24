#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use super::common::*;
use super::compile_transaction::CompileTransaction;
use super::{cf::*, cfe::*, dcs::*, form::*, interface::*, meta::*, mxl::*, role::*, subsystem::*};

struct TemplateAddResult {
    stdout: String,
    changes: Vec<String>,
    artifacts: Vec<String>,
    warnings: Vec<String>,
}

pub(crate) fn add_template(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<TemplateAddResult, String> {
        let object_name = required_string(
            args,
            &["objectName", "ObjectName", "processorName", "ProcessorName"],
            "ObjectName",
        )?;
        let template_name =
            required_string(args, &["templateName", "TemplateName"], "TemplateName")?;
        validate_template_metadata_name("ObjectName", object_name)?;
        validate_template_metadata_name("TemplateName", template_name)?;
        let template_type =
            required_string(args, &["templateType", "TemplateType"], "TemplateType")?;
        let (metadata_type, extension) = template_type_info(template_type)?;
        let synonym = string_arg(args, &["synonym", "Synonym"]).unwrap_or(template_name);
        let set_main_dcs = bool_arg(args, &["setMainSKD", "SetMainSKD"]);
        let mut src_dir_display =
            path_arg(args, &["srcDir", "SrcDir"]).unwrap_or_else(|| PathBuf::from("src"));
        let mut src_dir_abs = absolutize(src_dir_display.clone(), &context.cwd);
        let mut stdout = String::new();

        let mut root_xml_display = src_dir_display.join(format!("{object_name}.xml"));
        let mut root_xml_path = src_dir_abs.join(format!("{object_name}.xml"));
        if !root_xml_path.exists() {
            let mut candidates = Vec::<(PathBuf, PathBuf)>::new();
            for folder in template_add_object_type_folders() {
                let display = src_dir_display.join(folder);
                let probe = absolutize(display.join(format!("{object_name}.xml")), &context.cwd);
                if probe.exists() {
                    candidates.push((display, probe));
                }
            }

            if candidates.len() == 1 {
                let (display, probe) = candidates.remove(0);
                src_dir_display = display;
                src_dir_abs = absolutize(src_dir_display.clone(), &context.cwd);
                root_xml_display = src_dir_display.join(format!("{object_name}.xml"));
                root_xml_path = probe;
                stdout.push_str(&format!(
                    "[INFO] SrcDir расширен до: {}\n",
                    src_dir_display.display()
                ));
            } else if candidates.len() > 1 {
                let joined = candidates
                    .iter()
                    .map(|(display, _)| display.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Объект '{object_name}' найден в нескольких подпапках: {joined}\nУкажи SrcDir явно"
                ));
            } else {
                return Err(format!(
                    "Корневой файл объекта не найден: {}\nОжидается: <SrcDir>/<ObjectName>.xml\nПодсказка: SrcDir должен указывать на папку типа объектов (например Reports), а не на корень конфигурации",
                    root_xml_display.display()
                ));
            }
        }

        let processor_dir_display = src_dir_display.join(object_name);
        let processor_dir_abs = src_dir_abs.join(object_name);
        let templates_dir_display = processor_dir_display.join("Templates");
        let templates_dir_abs = processor_dir_abs.join("Templates");
        let template_meta_display = templates_dir_display.join(format!("{template_name}.xml"));
        let template_meta_path = templates_dir_abs.join(format!("{template_name}.xml"));
        if template_meta_path.exists() {
            return Err(format!(
                "Макет уже существует: {}",
                template_meta_display.display()
            ));
        }
        require_metadata_8_3_27_validation(&root_xml_path, context, "template.add")?;

        let format_version = detect_format_version(&root_xml_path, context)?.to_string();
        let template_ext_dir = templates_dir_abs.join(template_name).join("Ext");
        let template_uuid = fresh_uuid();
        let template_meta_xml = template_metadata_xml(
            template_name,
            synonym,
            metadata_type,
            &format_version,
            &template_uuid,
        );

        let template_file_display = templates_dir_display
            .join(template_name)
            .join("Ext")
            .join(format!("Template{extension}"));
        let template_file_path = template_ext_dir.join(format!("Template{extension}"));
        let template_content = if template_type == "HTML" {
            html_template_descriptor(&format_version)
        } else {
            template_content_xml(template_type, extension)?
        };
        let html_page_path =
            (template_type == "HTML").then(|| template_ext_dir.join("Template/ru.html"));
        let html_page_display = (template_type == "HTML").then(|| {
            templates_dir_display
                .join(template_name)
                .join("Ext/Template/ru.html")
        });
        let source_snapshot = read_utf8_sig_snapshot(&root_xml_path)?;
        let source_xml_text = source_snapshot.text;
        let xml_text = lxml_parser_normalized_text(&source_xml_text);
        let mut xml_text = append_metadata_child_text(&xml_text, "Template", template_name)
            .ok_or_else(|| {
                format!(
                    "Не найден элемент ChildObjects в {}",
                    root_xml_display.display()
                )
            })?;

        let mut main_dcs_updated = false;
        let mut main_dcs_value = String::new();
        if template_type == "DataCompositionSchema" {
            let (new_text, updated, value) =
                update_main_data_composition_schema_text(&xml_text, template_name, set_main_dcs);
            xml_text = new_text;
            main_dcs_updated = updated;
            main_dcs_value = value;
        }
        if !xml_text.ends_with('\n') {
            xml_text.push('\n');
        }
        let owner_replacement = utf8_bom_bytes(&lxml_tree_serialized_text_like_source(
            &xml_text,
            &source_xml_text,
        ));

        let mut transaction = CompileTransaction::new();
        transaction.create_utf8_bom_text(&template_meta_path, &template_meta_xml)?;
        if template_type == "BinaryData" {
            transaction.create_bytes(&template_file_path, Vec::new())?;
        } else {
            transaction.create_utf8_bom_text(&template_file_path, &template_content)?;
        }
        if let Some(path) = &html_page_path {
            transaction.create_utf8_bom_text(path, html_template_page())?;
        }
        transaction.replace_bytes(&root_xml_path, &source_snapshot.raw, owner_replacement)?;
        guard_active_format_owner(&mut transaction, &root_xml_path, context)?;
        let validation_path = root_xml_path.clone();
        let report = transaction.commit_with_post_validation(|| {
            require_metadata_8_3_27_validation(&validation_path, context, "template.add")
        })?;

        stdout.push_str(&format!(
            "[OK] Создан макет: {template_name} ({template_type})\n"
        ));
        stdout.push_str(&format!(
            "     Метаданные: {}\n",
            template_meta_display.display()
        ));
        stdout.push_str(&format!(
            "     Содержимое: {}\n",
            template_file_display.display()
        ));
        if main_dcs_updated {
            stdout.push_str(&format!(
                "     MainDataCompositionSchema: {main_dcs_value}\n"
            ));
        }

        let mut changes = vec![
            format!("created {}", template_meta_path.display()),
            format!("created {}", template_file_path.display()),
            format!("updated {}", root_xml_path.display()),
        ];
        let mut artifacts = vec![
            template_meta_path.display().to_string(),
            template_file_path.display().to_string(),
            root_xml_path.display().to_string(),
        ];
        if let (Some(path), Some(display)) = (&html_page_path, &html_page_display) {
            stdout.push_str(&format!("     HTML page: {}\n", display.display()));
            changes.insert(2, format!("created {}", path.display()));
            artifacts.insert(2, path.display().to_string());
        }

        Ok(TemplateAddResult {
            stdout,
            changes,
            artifacts,
            warnings: report.cleanup_warnings,
        })
    })();

    match result {
        Ok(result) => AdapterOutcome {
            ok: true,
            summary: "unica.template.add completed with native template writer".to_string(),
            changes: result.changes,
            warnings: result.warnings,
            errors: Vec::new(),
            artifacts: result.artifacts,
            stdout: Some(result.stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.template.add failed in native template writer".to_string(),
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

pub(crate) fn remove_template(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(String, Vec<String>, Vec<String>), String> {
        let object_name = required_string(
            args,
            &["objectName", "ObjectName", "processorName", "ProcessorName"],
            "ObjectName",
        )?;
        let template_name =
            required_string(args, &["templateName", "TemplateName"], "TemplateName")?;
        validate_template_metadata_name("ObjectName", object_name)?;
        validate_template_metadata_name("TemplateName", template_name)?;
        let src_dir_raw = string_arg(args, &["srcDir", "SrcDir"]).unwrap_or("src");
        let src_dir_display = PathBuf::from(src_dir_raw);
        let src_dir_abs = absolutize(src_dir_display.clone(), &context.cwd);

        let root_xml_display = src_dir_display.join(format!("{object_name}.xml"));
        let root_xml_path = src_dir_abs.join(format!("{object_name}.xml"));
        if !root_xml_path.exists() {
            return Err(format!(
                "Корневой файл обработки не найден: {}",
                root_xml_display.display()
            ));
        }

        let processor_dir_display = src_dir_display.join(object_name);
        let processor_dir_abs = src_dir_abs.join(object_name);
        let templates_dir_display = processor_dir_display.join("Templates");
        let templates_dir_abs = processor_dir_abs.join("Templates");
        let template_meta_display = templates_dir_display.join(format!("{template_name}.xml"));
        let template_meta_path = templates_dir_abs.join(format!("{template_name}.xml"));
        let template_dir_display = templates_dir_display.join(template_name);
        let template_dir_path = templates_dir_abs.join(template_name);

        if !template_meta_path.exists() {
            return Err(format!(
                "Метаданные макета не найдены: {}",
                template_meta_display.display()
            ));
        }
        require_metadata_8_3_27_validation(&root_xml_path, context, "template.remove")?;

        let source_snapshot = read_utf8_sig_snapshot(&root_xml_path)?;
        let source_xml_text = source_snapshot.text;
        let xml_text = lxml_parser_normalized_text(&source_xml_text);
        let (xml_text, _) =
            remove_owner_template_child_text(&xml_text, template_name).ok_or_else(|| {
                format!(
                    "Не найден элемент ChildObjects в {}",
                    root_xml_display.display()
                )
            })?;
        let (mut xml_text, main_dcs_cleared) =
            clear_main_data_composition_schema_text(&xml_text, template_name);
        if !xml_text.ends_with('\n') {
            xml_text.push('\n');
        }
        let owner_replacement = utf8_bom_bytes(&lxml_tree_serialized_text_like_source(
            &xml_text,
            &source_xml_text,
        ));

        let template_payload_exists = match fs::symlink_metadata(&template_dir_path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(format!(
                    "failed to inspect {}: {error}",
                    template_dir_path.display()
                ));
            }
        };
        let collection_targets = if template_payload_exists {
            vec![template_meta_path.as_path(), template_dir_path.as_path()]
        } else {
            vec![template_meta_path.as_path()]
        };

        let mut transaction = CompileTransaction::new();
        transaction.replace_bytes(&root_xml_path, &source_snapshot.raw, owner_replacement)?;
        let remove_templates_collection = transaction.remove_directory_if_only_direct_entries(
            &templates_dir_abs,
            collection_targets
                .iter()
                .map(|path| {
                    path.file_name()
                        .expect("template collection target must have a file name")
                        .to_os_string()
                })
                .collect(),
        )?;
        if !remove_templates_collection {
            if template_payload_exists {
                transaction.remove_path(&template_dir_path)?;
            } else {
                transaction.guard_path_absent(&template_dir_path)?;
            }
            transaction.remove_path(&template_meta_path)?;
        }
        let trees = if template_payload_exists {
            vec![template_meta_path.as_path(), template_dir_path.as_path()]
        } else {
            vec![template_meta_path.as_path()]
        };
        guard_active_format_dependencies_and_xml_trees(
            &mut transaction,
            &[root_xml_path.as_path()],
            &trees,
            context,
        )?;
        let validation_path = root_xml_path.clone();
        let validation_template_meta = template_meta_path.clone();
        let validation_template_dir = template_dir_path.clone();
        let report = transaction.commit_with_post_validation(move || {
            require_metadata_8_3_27_validation(&validation_path, context, "template.remove")?;
            for path in [&validation_template_meta, &validation_template_dir] {
                match fs::symlink_metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Ok(_) => {
                        return Err(format!(
                            "template.remove post-write validation found removed pair member still present: {}",
                            path.display()
                        ));
                    }
                    Err(error) => {
                        return Err(format!(
                            "template.remove post-write validation failed to inspect {}: {error}",
                            path.display()
                        ));
                    }
                }
            }
            Ok(())
        })?;

        let mut stdout = String::new();
        let mut changes = Vec::new();
        if template_payload_exists {
            stdout.push_str(&format!(
                "[OK] Удалён каталог: {}\n",
                template_dir_display.display()
            ));
            changes.push(format!("removed directory {}", template_dir_path.display()));
        }
        if remove_templates_collection {
            changes.push(format!(
                "removed empty collection directory {}",
                templates_dir_abs.display()
            ));
        }

        stdout.push_str(&format!(
            "[OK] Удалён файл: {}\n",
            template_meta_display.display()
        ));
        changes.push(format!("removed file {}", template_meta_path.display()));
        if main_dcs_cleared {
            stdout.push_str("[OK] Очищён MainDataCompositionSchema\n");
            changes.push("cleared MainDataCompositionSchema".to_string());
        }
        changes.push(format!("updated {}", root_xml_path.display()));

        stdout.push_str(&format!(
            "[OK] Макет {template_name} удалён из {}\n",
            root_xml_display.display()
        ));
        Ok((stdout, changes, report.cleanup_warnings))
    })();

    match result {
        Ok((stdout, changes, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.template.remove completed with native template remover".to_string(),
            changes,
            warnings,
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.template.remove failed in native template remover".to_string(),
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

pub(crate) fn template_type_info(
    template_type: &str,
) -> Result<(&'static str, &'static str), String> {
    match template_type {
        "HTML" => Ok(("HTMLDocument", ".xml")),
        "Text" => Ok(("TextDocument", ".txt")),
        "SpreadsheetDocument" => Ok(("SpreadsheetDocument", ".xml")),
        "BinaryData" => Ok(("BinaryData", ".bin")),
        "DataCompositionSchema" => Ok(("DataCompositionSchema", ".xml")),
        other => Err(format!(
            "argument -TemplateType: invalid choice: '{other}' (choose from 'HTML', 'Text', 'SpreadsheetDocument', 'BinaryData', 'DataCompositionSchema')"
        )),
    }
}

fn validate_template_metadata_name(argument: &str, value: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(Component::Normal(component)) if component == OsStr::new(value)
    ) && components.next().is_none();

    if form_is_xml_ncname(value) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid Unicode XML NCName and a single path component: {value:?}"
        ))
    }
}

pub(crate) fn template_add_object_type_folders() -> &'static [&'static str] {
    &[
        "Reports",
        "DataProcessors",
        "Documents",
        "Catalogs",
        "InformationRegisters",
        "AccumulationRegisters",
        "ChartsOfCharacteristicTypes",
        "ChartsOfAccounts",
        "ChartsOfCalculationTypes",
        "BusinessProcesses",
        "Tasks",
        "ExchangePlans",
    ]
}

pub(crate) fn full_md_namespace_declarations() -> &'static str {
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
}

pub(crate) fn fresh_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn template_metadata_xml(
    template_name: &str,
    synonym: &str,
    metadata_type: &str,
    format_version: &str,
    template_uuid: &str,
) -> String {
    let template_name = escape_xml(template_name);
    let synonym = escape_xml(synonym).replace('\r', "&#13;");
    let metadata_type = escape_xml(metadata_type);
    let format_version = escape_xml(format_version);
    let template_uuid = escape_xml(template_uuid);
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"",
            " xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\"",
            " xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
            " xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\"",
            " xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\"",
            " xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\"",
            " xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\"",
            " xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\"",
            " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
            " xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"",
            " xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
            " xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"",
            " xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\"",
            " xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\"",
            " xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
            " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"",
            " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
            " version=\"{format_version}\">\n",
            "\t<Template uuid=\"{template_uuid}\">\n",
            "\t\t<Properties>\n",
            "\t\t\t<Name>{template_name}</Name>\n",
            "\t\t\t<Synonym>\n",
            "\t\t\t\t<v8:item>\n",
            "\t\t\t\t\t<v8:lang>ru</v8:lang>\n",
            "\t\t\t\t\t<v8:content>{synonym}</v8:content>\n",
            "\t\t\t\t</v8:item>\n",
            "\t\t\t</Synonym>\n",
            "\t\t\t<Comment/>\n",
            "\t\t\t<TemplateType>{metadata_type}</TemplateType>\n",
            "\t\t</Properties>\n",
            "\t</Template>\n",
            "</MetaDataObject>"
        ),
        format_version = format_version,
        template_uuid = template_uuid,
        template_name = template_name,
        synonym = synonym,
        metadata_type = metadata_type,
    )
}

fn html_template_descriptor(format_version: &str) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Help xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" ",
            "xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "version=\"{}\">\n",
            "\t<Page>ru</Page>\n",
            "</Help>"
        ),
        escape_xml(format_version)
    )
}

fn html_template_page() -> &'static str {
    concat!(
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">",
        "<html><head>",
        "<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></meta>",
        "<link rel=\"stylesheet\" type=\"text/css\" ",
        "href=\"v8help://service_book/service_style\"></link>",
        "</head><body>\n",
        "</body></html>"
    )
}

pub(crate) fn template_content_xml(
    template_type: &str,
    _extension: &str,
) -> Result<String, String> {
    match template_type {
        "HTML" => Ok(concat!(
            "<!DOCTYPE html>\n",
            "<html>\n",
            "<head>\n",
            "\t<meta charset=\"UTF-8\">\n",
            "\t<title></title>\n",
            "</head>\n",
            "<body>\n",
            "</body>\n",
            "</html>"
        )
        .to_string()),
        "Text" => Ok(String::new()),
        "SpreadsheetDocument" => Ok(super::mxl::empty_spreadsheet_document_xml()),
        "DataCompositionSchema" => Ok(concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\"\n",
            "\t\txmlns:dcscom=\"http://v8.1c.ru/8.1/data-composition-system/common\"\n",
            "\t\txmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\"\n",
            "\t\txmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\"\n",
            "\t\txmlns:v8=\"http://v8.1c.ru/8.1/data/core\"\n",
            "\t\txmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"\n",
            "\t\txmlns:xs=\"http://www.w3.org/2001/XMLSchema\"\n",
            "\t\txmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "\t<dataSource>\n",
            "\t\t<name>ИсточникДанных1</name>\n",
            "\t\t<dataSourceType>Local</dataSourceType>\n",
            "\t</dataSource>\n",
            "\t<settingsVariant>\n",
            "\t\t<dcsset:name>Основной</dcsset:name>\n",
            "\t\t<dcsset:presentation xsi:type=\"xs:string\">Основной</dcsset:presentation>\n",
            "\t\t<dcsset:settings xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"/>\n",
            "\t</settingsVariant>\n",
            "</DataCompositionSchema>"
        )
        .to_string()),
        "BinaryData" => Ok(String::new()),
        other => Err(format!("unsupported template type: {other}")),
    }
}

pub(crate) fn append_metadata_child_text(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> Option<String> {
    let doc = Document::parse(xml_text).ok()?;
    let object_node = doc
        .root_element()
        .children()
        .find(|node| node.is_element())?;
    let child_objects_node = object_node
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "ChildObjects")?;
    let range = child_objects_node.range();
    let element_text = &xml_text[range.clone()];
    let prefix = if element_text.trim_start().starts_with("<md:") {
        "md:"
    } else {
        ""
    };
    let item_name = escape_xml(item_name);

    let empty_tag = format!("<{prefix}ChildObjects/>");
    if element_text.trim() == empty_tag {
        let line_start = xml_text[..range.start].rfind('\n').map_or(0, |pos| pos + 1);
        let indent_candidate = &xml_text[line_start..range.start];
        let indent = if indent_candidate
            .chars()
            .all(|character| character == ' ' || character == '\t')
        {
            indent_candidate
        } else {
            ""
        };
        let replacement = format!(
            "<{prefix}ChildObjects>\n{indent}\t<{prefix}{local_name}>{item_name}</{prefix}{local_name}>\n{indent}</{prefix}ChildObjects>"
        );
        let mut result = String::with_capacity(xml_text.len() + replacement.len());
        result.push_str(&xml_text[..range.start]);
        result.push_str(&replacement);
        result.push_str(&xml_text[range.end..]);
        return Some(result);
    }

    let close = format!("</{prefix}ChildObjects>");
    let close_rel = element_text.rfind(&close)?;
    let index = range.start + close_rel;
    let line_start = xml_text[..index].rfind('\n').map_or(0, |pos| pos + 1);
    let closing_indent_candidate = &xml_text[line_start..index];
    let closing_indent = if closing_indent_candidate
        .chars()
        .all(|character| character == ' ' || character == '\t')
    {
        closing_indent_candidate
    } else {
        ""
    };
    let line =
        format!("\t<{prefix}{local_name}>{item_name}</{prefix}{local_name}>\n{closing_indent}");
    let mut result = String::with_capacity(xml_text.len() + line.len());
    result.push_str(&xml_text[..index]);
    result.push_str(&line);
    result.push_str(&xml_text[index..]);
    Some(result)
}

pub(crate) fn update_main_data_composition_schema_text(
    xml_text: &str,
    template_name: &str,
    set_main_dcs: bool,
) -> (String, bool, String) {
    let Some((object_type, object_start)) = ["ExternalReport", "Report"]
        .iter()
        .find_map(|name| find_open_tag(xml_text, name).map(|index| (*name, index)))
    else {
        return (xml_text.to_string(), false, String::new());
    };
    let object_name = first_tag_text_after(xml_text, "Name", object_start);
    let value = format!("{object_type}.{object_name}.Template.{template_name}");
    if let Some((open_start, content_start, close_start, close_end, open_tag, close_tag)) =
        find_element_bounds(xml_text, "MainDataCompositionSchema", object_start)
    {
        let content = xml_text[content_start..close_start].trim();
        if !content.is_empty() && !set_main_dcs {
            return (xml_text.to_string(), false, String::new());
        }
        let replacement = format!("{open_tag}{value}{close_tag}");
        let mut result = String::with_capacity(xml_text.len() + value.len());
        result.push_str(&xml_text[..open_start]);
        result.push_str(&replacement);
        result.push_str(&xml_text[close_end..]);
        return (result, true, value);
    }

    let Some((open_start, open_end, tag)) =
        find_self_closing_element_bounds(xml_text, "MainDataCompositionSchema", object_start)
    else {
        return (xml_text.to_string(), false, String::new());
    };
    let replacement = format!("<{tag}>{value}</{tag}>");
    let mut result = String::with_capacity(xml_text.len() + replacement.len());
    result.push_str(&xml_text[..open_start]);
    result.push_str(&replacement);
    result.push_str(&xml_text[open_end..]);
    (result, true, value)
}

fn find_self_closing_element_bounds(
    xml_text: &str,
    local_name: &str,
    start: usize,
) -> Option<(usize, usize, String)> {
    for tag in [local_name.to_string(), format!("md:{local_name}")] {
        let open_needle = format!("<{tag}");
        let mut search_start = start;
        while let Some(open_rel) = xml_text[search_start..].find(&open_needle) {
            let open_start = search_start + open_rel;
            let name_end = open_start + open_needle.len();
            let Some(boundary) = xml_text[name_end..].chars().next() else {
                break;
            };
            if !boundary.is_ascii_whitespace() && boundary != '/' && boundary != '>' {
                search_start = name_end;
                continue;
            }
            let open_end = open_start + xml_text[open_start..].find('>')? + 1;
            if xml_text[open_start..open_end]
                .trim_end_matches('>')
                .trim_end()
                .ends_with('/')
            {
                return Some((open_start, open_end, tag));
            }
            search_start = open_end;
        }
    }
    None
}

pub(crate) fn find_open_tag(xml_text: &str, local_name: &str) -> Option<usize> {
    [format!("<{local_name}"), format!("<md:{local_name}")]
        .iter()
        .filter_map(|needle| xml_text.find(needle))
        .min()
}

pub(crate) fn first_tag_text_after(xml_text: &str, local_name: &str, start: usize) -> String {
    let Some((_, content_start, close_start, _, _, _)) =
        find_element_bounds(xml_text, local_name, start)
    else {
        return String::new();
    };
    xml_text[content_start..close_start].trim().to_string()
}

pub(crate) fn find_element_bounds(
    xml_text: &str,
    local_name: &str,
    start: usize,
) -> Option<(usize, usize, usize, usize, String, String)> {
    for tag in [local_name.to_string(), format!("md:{local_name}")] {
        let open_needle = format!("<{tag}");
        let Some(open_rel) = xml_text[start..].find(&open_needle) else {
            continue;
        };
        let open_start = start + open_rel;
        let Some(open_end_rel) = xml_text[open_start..].find('>') else {
            continue;
        };
        let content_start = open_start + open_end_rel + 1;
        let close_tag = format!("</{tag}>");
        let Some(close_rel) = xml_text[content_start..].find(&close_tag) else {
            continue;
        };
        let close_start = content_start + close_rel;
        let close_end = close_start + close_tag.len();
        let open_tag = xml_text[open_start..content_start].to_string();
        return Some((
            open_start,
            content_start,
            close_start,
            close_end,
            open_tag,
            close_tag,
        ));
    }
    None
}

pub(crate) fn remove_template_child_text_lxml(xml_text: &str, template_name: &str) -> String {
    remove_metadata_child_text_lxml(xml_text, "Template", template_name)
}

fn remove_owner_template_child_text(xml_text: &str, template_name: &str) -> Option<(String, bool)> {
    let document = Document::parse(xml_text).ok()?;
    let object = document
        .root_element()
        .children()
        .find(roxmltree::Node::is_element)?;
    let child_objects = object
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "ChildObjects")?;
    let Some(template) = child_objects.children().find(|node| {
        node.is_element()
            && node.tag_name().name() == "Template"
            && node.text().is_some_and(|text| text.trim() == template_name)
    }) else {
        return Some((xml_text.to_string(), false));
    };

    if child_objects
        .children()
        .filter(roxmltree::Node::is_element)
        .count()
        == 1
    {
        let range = child_objects.range();
        let qualified_name = xml_text[range.start + 1..]
            .split(|character: char| character.is_whitespace() || matches!(character, '/' | '>'))
            .next()?;
        let replacement = format!("<{qualified_name}/>");
        let mut updated = String::with_capacity(xml_text.len() - range.len() + replacement.len());
        updated.push_str(&xml_text[..range.start]);
        updated.push_str(&replacement);
        updated.push_str(&xml_text[range.end..]);
        return Some((updated, true));
    }

    let range = template.range();
    let line_start = xml_text[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let leading_is_indent = xml_text[line_start..range.start]
        .chars()
        .all(|character| character == ' ' || character == '\t');
    let remove_range = if leading_is_indent && xml_text[range.end..].starts_with('\n') {
        line_start..range.end + 1
    } else {
        range
    };
    let mut updated = String::with_capacity(xml_text.len() - remove_range.len());
    updated.push_str(&xml_text[..remove_range.start]);
    updated.push_str(&xml_text[remove_range.end..]);
    Some((updated, true))
}

pub(crate) fn invoke_read(
    _operation: &str,
    _tool_name: &str,
    _args: &Map<String, Value>,
    _context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    None
}

pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    match operation {
        "template-add" => Some(add_template(args, context)),
        "template-remove" => Some(remove_template(args, context)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use super::super::single_file_publisher::with_before_commit_hook;
    use super::*;
    use crate::application::UnicaApplication;

    fn path_text(path: &Path) -> String {
        crate::infrastructure::platform::testing::path_text_for_test(path)
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!(
            "unica-template-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn valid_report_owner_xml(with_template: bool) -> String {
        let (xml, _) = meta_compile_object_xml(&Map::new(), "Report", "Sales", "2.20")
            .expect("report fixture must satisfy the fixed 8.3.27 contract");
        if with_template {
            xml.replacen(
                "\t\t<ChildObjects/>",
                "\t\t<ChildObjects>\n\t\t\t<Template>Layout</Template>\n\t\t</ChildObjects>",
                1,
            )
        } else {
            xml
        }
    }

    fn valid_template_descriptor_bytes() -> Vec<u8> {
        br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template/></MetaDataObject>"#.to_vec()
    }

    fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        fn visit(root: &Path, current: &Path, snapshot: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
            let mut entries = fs::read_dir(current)
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                if path.is_dir() {
                    snapshot.push((relative, None));
                    visit(root, &path, snapshot);
                } else {
                    snapshot.push((relative, Some(fs::read(&path).unwrap())));
                }
            }
        }

        let mut snapshot = Vec::new();
        visit(root, root, &mut snapshot);
        snapshot
    }

    #[test]
    fn update_main_dcs_expands_the_self_closing_property_emitted_by_meta_compile() {
        let source = valid_report_owner_xml(false);
        assert!(
            source.contains("<MainDataCompositionSchema/>"),
            "fixture must exercise the exact meta.compile output: {source}"
        );

        let (updated, changed, value) =
            update_main_data_composition_schema_text(&source, "MainSchema", false);

        assert!(changed, "{updated}");
        assert_eq!(value, "Report.Sales.Template.MainSchema");
        assert!(
            updated.contains(
                "<MainDataCompositionSchema>Report.Sales.Template.MainSchema</MainDataCompositionSchema>"
            ),
            "{updated}"
        );
        assert!(
            !updated.contains("<MainDataCompositionSchema/>"),
            "{updated}"
        );
    }

    #[test]
    fn template_add_rejects_platform_invalid_owner_boolean_without_creating_files() {
        let context = temp_context("add-invalid-owner-boolean");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let owner = reports.join("Sales.xml");
        let invalid = valid_report_owner_xml(false).replace(
            "<IncludeHelpInContents>false</IncludeHelpInContents>",
            "<IncludeHelpInContents>truthy</IncludeHelpInContents>",
        );
        fs::write(&owner, invalid.as_bytes()).unwrap();
        let owner_before = fs::read(&owner).unwrap();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = add_template(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("IncludeHelpInContents")
                    && error.contains("true or false")),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), owner_before);
        assert!(!reports.join("Sales/Templates").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_rejects_platform_invalid_owner_boolean_without_deleting_files() {
        let context = temp_context("remove-invalid-owner-boolean");
        let reports = context.cwd.join("src/Reports");
        let template_dir = reports.join("Sales/Templates/Layout/Ext");
        fs::create_dir_all(&template_dir).unwrap();
        let owner = reports.join("Sales.xml");
        let invalid = valid_report_owner_xml(true).replace(
            "<IncludeHelpInContents>false</IncludeHelpInContents>",
            "<IncludeHelpInContents>truthy</IncludeHelpInContents>",
        );
        let descriptor = reports.join("Sales/Templates/Layout.xml");
        let content = template_dir.join("Template.txt");
        fs::write(&owner, invalid.as_bytes()).unwrap();
        fs::write(&descriptor, b"descriptor-before").unwrap();
        fs::write(&content, b"content-before").unwrap();
        let before = snapshot_tree(&context.cwd);
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = remove_template(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("IncludeHelpInContents")
                    && error.contains("true or false")),
            "{outcome:?}"
        );
        assert_eq!(snapshot_tree(&context.cwd), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_rejects_newer_descriptor_without_deleting_payload() {
        let context = temp_context("remove-newer-descriptor");
        let reports = context.cwd.join("src/Reports");
        let template_dir = reports.join("Sales/Templates/Layout/Ext");
        fs::create_dir_all(&template_dir).unwrap();
        let owner = reports.join("Sales.xml");
        let descriptor = reports.join("Sales/Templates/Layout.xml");
        let content = template_dir.join("Template.txt");
        fs::write(&owner, valid_report_owner_xml(true)).unwrap();
        fs::write(
            &descriptor,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Template/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(&content, b"content-before").unwrap();
        let before = snapshot_tree(&context.cwd);
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = remove_template(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostics = outcome.errors.join("\n");
        assert!(diagnostics.contains("2.21"), "{diagnostics}");
        assert!(diagnostics.contains("1C 8.5"), "{diagnostics}");
        assert_eq!(snapshot_tree(&context.cwd), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_rejects_unsupported_owner_format_before_creating_tree() {
        for (case, configuration) in [
            (
                "unsupported-format",
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
            ),
            ("malformed-format", "<MetaDataObject"),
        ] {
            let context = temp_context(case);
            let src = context.cwd.join("src");
            let reports = src.join("Reports");
            fs::create_dir_all(reports.join("Sales")).unwrap();
            fs::write(src.join("Configuration.xml"), configuration).unwrap();
            let owner = reports.join("Sales.xml");
            let owner_bytes = valid_report_owner_xml(false).into_bytes();
            fs::write(&owner, &owner_bytes).unwrap();
            let args = json!({
                "ObjectName": "Sales",
                "TemplateName": "Layout",
                "TemplateType": "Text",
                "SrcDir": "src/Reports"
            })
            .as_object()
            .unwrap()
            .clone();

            let outcome = add_template(&args, &context);

            assert!(!outcome.ok, "{case}: {outcome:?}");
            assert_eq!(fs::read(&owner).unwrap(), owner_bytes, "{case}");
            assert!(!reports.join("Sales/Templates").exists(), "{case}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn public_template_add_prioritizes_newer_existing_target_over_older_object_owner() {
        let context = temp_context("public-add-existing-newer-target");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales/Templates")).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let configuration_path = src.join("Configuration.xml");
        let configuration = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#.to_vec();
        fs::write(&configuration_path, &configuration).unwrap();
        let owner_path = reports.join("Sales.xml");
        let older_owner = valid_report_owner_xml(false)
            .replacen(r#"version="2.20""#, r#"version="2.19""#, 1)
            .into_bytes();
        fs::write(&owner_path, &older_owner).unwrap();
        let descriptor_path = reports.join("Sales/Templates/ExistingLayout.xml");
        let newer_descriptor = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Template/></MetaDataObject>"#.to_vec();
        fs::write(&descriptor_path, &newer_descriptor).unwrap();
        let before = snapshot_tree(&context.cwd);
        let mut args = json!({
            "ObjectName": "Sales",
            "TemplateName": "ExistingLayout",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();
        args.insert("cwd".to_string(), json!(context.cwd.display().to_string()));
        args.insert("dryRun".to_string(), json!(false));

        let outcome = UnicaApplication::new()
            .call_tool("unica.template.add", &args)
            .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostic = &outcome.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let warning = outcome.warnings.join("\n");
        assert!(warning.contains("1С 8.5"), "{warning}");
        assert!(!warning.contains("миграц"), "{warning}");
        assert!(!warning.contains("повторно выгруз"), "{warning}");
        assert!(!warning.contains("re-export"), "{warning}");
        assert_eq!(fs::read(&configuration_path).unwrap(), configuration);
        assert_eq!(fs::read(&owner_path).unwrap(), older_owner);
        assert_eq!(fs::read(&descriptor_path).unwrap(), newer_descriptor);
        assert_eq!(snapshot_tree(&context.cwd), before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_descriptor_uses_active_format() {
        let context = temp_context("active-format");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(reports.join("Sales.xml"), valid_report_owner_xml(false)).unwrap();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = add_template(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let generated = fs::read_to_string(reports.join("Sales/Templates/Layout.xml")).unwrap();
        assert!(generated.contains(r#"version="2.20""#), "{generated}");
        assert!(!generated.contains(r#"version="2.17""#), "{generated}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_collapses_last_child_to_canonical_self_closing_child_objects() {
        let context = temp_context("remove-last-child-canonical");
        let reports = context.cwd.join("src/Reports");
        fs::create_dir_all(reports.join("Sales/Templates")).unwrap();
        let owner = reports.join("Sales.xml");
        let descriptor = reports.join("Sales/Templates/Layout.xml");
        fs::write(&owner, valid_report_owner_xml(true)).unwrap();
        fs::write(&descriptor, valid_template_descriptor_bytes()).unwrap();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = remove_template(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let owner_after = fs::read_to_string(&owner).unwrap();
        assert!(
            owner_after.contains("\t\t<ChildObjects/>\n"),
            "{owner_after}"
        );
        assert!(
            !owner_after.contains("<ChildObjects>\n\t\t</ChildObjects>"),
            "{owner_after}"
        );
        assert!(
            !reports.join("Sales/Templates").exists(),
            "the platform removes an empty Templates collection directory"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_invalid_owner_child_objects_does_not_leave_orphan_files() {
        let context = temp_context("add-invalid-owner-child-objects");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let owner = reports.join("Sales.xml");
        let owner_bytes = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties/></Report></MetaDataObject>"#.to_vec();
        fs::write(&owner, &owner_bytes).unwrap();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = add_template(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(fs::read(&owner).unwrap(), owner_bytes);
        assert!(!reports.join("Sales/Templates/Layout.xml").exists());
        assert!(!reports.join("Sales/Templates/Layout").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_final_failure_restores_owner_and_removes_orphans() {
        let context = temp_context("add-final-failure");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let owner = reports.join("Sales.xml");
        let owner_bytes = valid_report_owner_xml(false).into_bytes();
        fs::write(&owner, &owner_bytes).unwrap();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            add_template(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(fs::read(&owner).unwrap(), owner_bytes);
        assert!(!reports.join("Sales/Templates/Layout.xml").exists());
        assert!(!reports.join("Sales/Templates/Layout").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_invalid_owner_child_objects_does_not_delete_payload() {
        let context = temp_context("remove-invalid-owner-child-objects");
        let reports = context.cwd.join("src/Reports");
        let template_dir = reports.join("Sales/Templates/Layout/Ext");
        fs::create_dir_all(&template_dir).unwrap();
        let owner = reports.join("Sales.xml");
        let owner_bytes = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties/></Report></MetaDataObject>"#.to_vec();
        let descriptor = reports.join("Sales/Templates/Layout.xml");
        let descriptor_bytes = b"descriptor-before".to_vec();
        let content = template_dir.join("Template.txt");
        let content_bytes = b"content-before".to_vec();
        fs::write(&owner, &owner_bytes).unwrap();
        fs::write(&descriptor, &descriptor_bytes).unwrap();
        fs::write(&content, &content_bytes).unwrap();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = remove_template(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(fs::read(&owner).unwrap(), owner_bytes);
        assert_eq!(fs::read(&descriptor).unwrap(), descriptor_bytes);
        assert_eq!(fs::read(&content).unwrap(), content_bytes);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_final_failure_restores_owner_and_payload_bytes() {
        let context = temp_context("remove-final-failure");
        let reports = context.cwd.join("src/Reports");
        let template_dir = reports.join("Sales/Templates/Layout/Ext");
        fs::create_dir_all(&template_dir).unwrap();
        let owner = reports.join("Sales.xml");
        let owner_bytes = valid_report_owner_xml(true).into_bytes();
        let descriptor = reports.join("Sales/Templates/Layout.xml");
        let descriptor_bytes = valid_template_descriptor_bytes();
        let content = template_dir.join("Template.txt");
        let content_bytes = b"content-before".to_vec();
        fs::write(&owner, &owner_bytes).unwrap();
        fs::write(&descriptor, &descriptor_bytes).unwrap();
        fs::write(&content, &content_bytes).unwrap();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            remove_template(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(fs::read(&owner).unwrap(), owner_bytes);
        assert_eq!(fs::read(&descriptor).unwrap(), descriptor_bytes);
        assert_eq!(fs::read(&content).unwrap(), content_bytes);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_preserves_concurrent_owner_replacement_and_removes_planned_files() {
        let context = temp_context("add-concurrent-owner-change");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let owner = reports.join("Sales.xml");
        let original = valid_report_owner_xml(false);
        let concurrent = original
            .replace("<Comment/>", "<Comment>concurrent</Comment>")
            .into_bytes();
        assert_ne!(concurrent, original.as_bytes());
        fs::write(&owner, original.as_bytes()).unwrap();
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent.clone();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &concurrent_for_hook).unwrap(),
            || add_template(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("changed")
                || outcome.errors.join("\n").contains("preimage"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent);
        assert!(!reports.join("Sales/Templates").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_preserves_concurrent_owner_replacement_and_payload() {
        let context = temp_context("remove-concurrent-owner-change");
        let reports = context.cwd.join("src/Reports");
        let template_dir = reports.join("Sales/Templates/Layout/Ext");
        fs::create_dir_all(&template_dir).unwrap();
        let owner = reports.join("Sales.xml");
        let original = valid_report_owner_xml(true);
        let concurrent = original
            .replace("<Comment/>", "<Comment>concurrent</Comment>")
            .into_bytes();
        assert_ne!(concurrent, original.as_bytes());
        let descriptor = reports.join("Sales/Templates/Layout.xml");
        let descriptor_bytes = valid_template_descriptor_bytes();
        let content = template_dir.join("Template.txt");
        let content_bytes = b"content-before".to_vec();
        fs::write(&owner, original.as_bytes()).unwrap();
        fs::write(&descriptor, &descriptor_bytes).unwrap();
        fs::write(&content, &content_bytes).unwrap();
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent.clone();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &concurrent_for_hook).unwrap(),
            || remove_template(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("changed")
                || outcome.errors.join("\n").contains("preimage"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent);
        assert_eq!(fs::read(&descriptor).unwrap(), descriptor_bytes);
        assert_eq!(fs::read(&content).unwrap(), content_bytes);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_rejects_payload_directory_that_appears_after_absent_probe() {
        let context = temp_context("remove-late-payload-directory");
        let reports = context.cwd.join("src/Reports");
        let templates = reports.join("Sales/Templates");
        fs::create_dir_all(&templates).unwrap();
        let owner = reports.join("Sales.xml");
        let owner_before = valid_report_owner_xml(true).into_bytes();
        let descriptor = templates.join("Layout.xml");
        let descriptor_before = valid_template_descriptor_bytes();
        let sibling = templates.join("Other.xml");
        let late_payload = templates.join("Layout");
        fs::write(&owner, &owner_before).unwrap();
        fs::write(&descriptor, &descriptor_before).unwrap();
        fs::write(&sibling, valid_template_descriptor_bytes()).unwrap();
        let late_content_for_hook = late_payload.join("Ext/Template.txt");
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = with_before_commit_hook(
            move |_| {
                fs::create_dir_all(late_content_for_hook.parent().unwrap()).unwrap();
                fs::write(&late_content_for_hook, b"late payload").unwrap();
            },
            || remove_template(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("pair member"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), owner_before);
        assert_eq!(fs::read(&descriptor).unwrap(), descriptor_before);
        assert_eq!(
            fs::read(late_payload.join("Ext/Template.txt")).unwrap(),
            b"late payload"
        );
        assert!(sibling.is_file());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_rolls_back_if_configuration_owner_changes_during_publication() {
        let context = temp_context("add-configuration-owner-race");
        let source = context.cwd.join("src");
        let reports = source.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let owner = source.join("Configuration.xml");
        let owner_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#.to_vec();
        fs::write(&owner, &owner_before).unwrap();
        let report = reports.join("Sales.xml");
        let report_before = valid_report_owner_xml(false).into_bytes();
        fs::write(&report, &report_before).unwrap();
        let mut concurrent_owner = owner_before.clone();
        concurrent_owner.extend_from_slice(b" ");
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent_owner.clone();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &concurrent_for_hook).unwrap(),
            || add_template(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent_owner);
        assert_eq!(fs::read(&report).unwrap(), report_before);
        assert!(!reports.join("Sales/Templates").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_rolls_back_if_configuration_owner_changes_during_publication() {
        let context = temp_context("remove-configuration-owner-race");
        let source = context.cwd.join("src");
        let reports = source.join("Reports");
        let template_dir = reports.join("Sales/Templates/Layout/Ext");
        fs::create_dir_all(&template_dir).unwrap();
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let owner = source.join("Configuration.xml");
        let owner_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#.to_vec();
        fs::write(&owner, &owner_before).unwrap();
        let report = reports.join("Sales.xml");
        let report_before = valid_report_owner_xml(true).into_bytes();
        fs::write(&report, &report_before).unwrap();
        let descriptor = reports.join("Sales/Templates/Layout.xml");
        let descriptor_before = valid_template_descriptor_bytes();
        fs::write(&descriptor, &descriptor_before).unwrap();
        let content = template_dir.join("Template.txt");
        let content_before = b"content-before".to_vec();
        fs::write(&content, &content_before).unwrap();
        let mut concurrent_owner = owner_before.clone();
        concurrent_owner.extend_from_slice(b" ");
        let owner_for_hook = owner.clone();
        let concurrent_for_hook = concurrent_owner.clone();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &concurrent_for_hook).unwrap(),
            || remove_template(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&owner).unwrap(), concurrent_owner);
        assert_eq!(fs::read(&report).unwrap(), report_before);
        assert_eq!(fs::read(&descriptor).unwrap(), descriptor_before);
        assert_eq!(fs::read(&content).unwrap(), content_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_rejects_template_name_path_traversal_without_mutation() {
        let context = temp_context("add-template-name-traversal");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(reports.join("Sales.xml"), valid_report_owner_xml(false)).unwrap();
        let before = snapshot_tree(&context.cwd);
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "../Escape",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = add_template(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(snapshot_tree(&context.cwd), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_rejects_object_name_path_traversal_without_mutation() {
        let context = temp_context("add-object-name-traversal");
        let src = context.cwd.join("src");
        fs::create_dir_all(src.join("Reports")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            src.join("Sales.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties/><ChildObjects/></Report></MetaDataObject>"#,
        )
        .unwrap();
        let before = snapshot_tree(&context.cwd);
        let args = json!({
            "ObjectName": "../Sales",
            "TemplateName": "Layout",
            "TemplateType": "Text",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = add_template(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(snapshot_tree(&context.cwd), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_remove_rejects_path_components_without_mutation() {
        for (case, object_name, template_name) in [
            ("template", "Sales", "../Victim"),
            ("object", "../Sales", "Layout"),
        ] {
            let context = temp_context(&format!("remove-{case}-name-traversal"));
            let src = context.cwd.join("src");
            let reports = src.join("Reports");
            fs::create_dir_all(&reports).unwrap();
            let sales = if case == "object" {
                src.join("Sales")
            } else {
                reports.join("Sales")
            };
            fs::create_dir_all(sales.join("Templates/Layout/Ext")).unwrap();
            let owner = if case == "object" {
                src.join("Sales.xml")
            } else {
                reports.join("Sales.xml")
            };
            fs::write(
                &owner,
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report><Properties/><ChildObjects><Template>Layout</Template></ChildObjects></Report></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(sales.join("Templates/Layout.xml"), b"descriptor").unwrap();
            fs::write(sales.join("Templates/Layout/Ext/Template.txt"), b"content").unwrap();
            if case == "template" {
                fs::create_dir_all(sales.join("Victim/Ext")).unwrap();
                fs::write(sales.join("Victim.xml"), b"victim descriptor").unwrap();
                fs::write(sales.join("Victim/Ext/Template.txt"), b"victim").unwrap();
            }
            let before = snapshot_tree(&context.cwd);
            let args = json!({
                "ObjectName": object_name,
                "TemplateName": template_name,
                "SrcDir": "src/Reports"
            })
            .as_object()
            .unwrap()
            .clone();

            let outcome = remove_template(&args, &context);

            assert!(!outcome.ok, "{case}: {outcome:?}");
            assert_eq!(snapshot_tree(&context.cwd), before, "{case}");
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn template_add_accepts_unicode_ncname_and_escapes_synonym() {
        let context = temp_context("add-unicode-name-special-synonym");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(reports.join("Sales.xml"), valid_report_owner_xml(false)).unwrap();
        let template_name = "模板_Δ";
        let synonym = "A&B <C> \"quoted\"";
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": template_name,
            "TemplateType": "Text",
            "Synonym": synonym,
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = add_template(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let descriptor =
            fs::read(reports.join(format!("Sales/Templates/{template_name}.xml"))).unwrap();
        let descriptor = std::str::from_utf8(&descriptor)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let document = Document::parse(descriptor).unwrap();
        let properties = document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "Properties")
            .unwrap();
        assert_eq!(
            properties
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "Name")
                .and_then(|node| node.text()),
            Some(template_name)
        );
        assert_eq!(
            properties
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "content")
                .and_then(|node| node.text()),
            Some(synonym)
        );
        let owner = fs::read(reports.join("Sales.xml")).unwrap();
        let owner = std::str::from_utf8(&owner)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let owner = Document::parse(owner).unwrap();
        assert!(owner.descendants().any(|node| {
            node.is_element()
                && node.tag_name().name() == "Template"
                && node.text() == Some(template_name)
        }));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_html_uses_the_8_3_27_page_set_layout() {
        let context = temp_context("add-html-platform-layout");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(reports.join("Sales.xml"), valid_report_owner_xml(false)).unwrap();
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "HtmlLayout",
            "TemplateType": "HTML",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = add_template(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        let ext = reports.join("Sales/Templates/HtmlLayout/Ext");
        let descriptor = fs::read(ext.join("Template.xml")).unwrap();
        assert!(descriptor.starts_with(&[0xef, 0xbb, 0xbf]));
        let descriptor = std::str::from_utf8(&descriptor[3..]).unwrap();
        let document = Document::parse(descriptor).unwrap();
        let root = document.root_element();
        assert_eq!(
            (root.tag_name().namespace(), root.tag_name().name()),
            (Some("http://v8.1c.ru/8.3/xcf/extrnprops"), "Help")
        );
        assert_eq!(root.attribute("version"), Some("2.20"));
        assert_eq!(
            root.children()
                .find(|node| node.is_element() && node.tag_name().name() == "Page")
                .and_then(|node| node.text()),
            Some("ru")
        );
        let page = fs::read(ext.join("Template/ru.html")).unwrap();
        assert!(page.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(std::str::from_utf8(&page[3..])
            .unwrap()
            .starts_with("<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">"));
        assert!(!ext.join("Template.html").exists());
        let artifacts = outcome
            .artifacts
            .iter()
            .map(|path| {
                crate::infrastructure::platform::testing::normalize_path_text_for_test(path)
            })
            .collect::<Vec<_>>();
        assert!(artifacts.contains(&path_text(&ext.join("Template.xml"))));
        assert!(artifacts.contains(&path_text(&ext.join("Template/ru.html"))));
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn template_add_rejects_xml_control_in_synonym_without_mutation() {
        let context = temp_context("add-invalid-synonym-control");
        let src = context.cwd.join("src");
        let reports = src.join("Reports");
        fs::create_dir_all(reports.join("Sales")).unwrap();
        fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(reports.join("Sales.xml"), valid_report_owner_xml(false)).unwrap();
        let before = snapshot_tree(&context.cwd);
        let args = json!({
            "ObjectName": "Sales",
            "TemplateName": "Layout",
            "TemplateType": "Text",
            "Synonym": "invalid\u{0001}synonym",
            "SrcDir": "src/Reports"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = add_template(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert_eq!(snapshot_tree(&context.cwd), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn append_metadata_child_text_uses_root_child_objects() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses">
	<Document uuid="00000000-0000-0000-0000-000000000001">
		<Properties>
			<Name>NestedChildObjectsDoc</Name>
		</Properties>
		<ChildObjects>
			<TabularSection uuid="00000000-0000-0000-0000-000000000002">
				<Properties>
					<Name>Goods</Name>
				</Properties>
				<ChildObjects>
					<Attribute uuid="00000000-0000-0000-0000-000000000003">
						<Properties>
							<Name>Item</Name>
						</Properties>
					</Attribute>
				</ChildObjects>
			</TabularSection>
		</ChildObjects>
	</Document>
</MetaDataObject>
"#;

        let updated = append_metadata_child_text(xml, "Template", "ПФ_MXL_КШ").unwrap();

        assert_eq!(updated.matches("<Template>ПФ_MXL_КШ</Template>").count(), 1);
        assert!(updated.contains(
            "\t\t\t</TabularSection>\n\t\t\t<Template>ПФ_MXL_КШ</Template>\n\t\t</ChildObjects>"
        ));
        assert!(
            !updated.contains("\t\t\t\t<Template>ПФ_MXL_КШ</Template>\n\t\t\t\t</ChildObjects>")
        );
    }

    #[test]
    fn fresh_uuid_generates_uuid_v4() {
        let value = fresh_uuid();
        let uuid = uuid::Uuid::parse_str(&value).expect(&value);

        assert!(!uuid.is_nil(), "{value}");
        assert_eq!(uuid.get_version(), Some(uuid::Version::Random), "{value}");
    }

    #[test]
    fn template_add_spreadsheet_matches_platform_8_3_27_fixture() {
        let xml = template_content_xml("SpreadsheetDocument", ".xml").unwrap();
        let expected =
            include_str!("../../../../../tests/fixtures/platform_8_3_27/mxl/Template.xml");

        assert_eq!(xml.replace("\r\n", "\n"), expected.replace("\r\n", "\n"));
    }

    #[test]
    fn template_add_empty_dcs_matches_platform_8_3_27_settings_variant() {
        const SETTINGS_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/settings";
        const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

        let xml = template_content_xml("DataCompositionSchema", ".xml").unwrap();
        let document = Document::parse(&xml).unwrap();
        let root = document.root_element();
        let root_children = root
            .children()
            .filter(roxmltree::Node::is_element)
            .collect::<Vec<_>>();

        assert_eq!(
            root_children
                .iter()
                .map(|node| node.tag_name().name())
                .collect::<Vec<_>>(),
            ["dataSource", "settingsVariant"]
        );
        let variant = root_children[1];
        let variant_children = variant
            .children()
            .filter(roxmltree::Node::is_element)
            .collect::<Vec<_>>();
        assert_eq!(
            variant_children
                .iter()
                .map(|node| (node.tag_name().namespace(), node.tag_name().name()))
                .collect::<Vec<_>>(),
            [
                (Some(SETTINGS_NS), "name"),
                (Some(SETTINGS_NS), "presentation"),
                (Some(SETTINGS_NS), "settings"),
            ]
        );
        assert_eq!(variant_children[0].text(), Some("Основной"));
        assert_eq!(
            variant_children[1].attribute((XSI_NS, "type")),
            Some("xs:string")
        );
        assert_eq!(variant_children[1].text(), Some("Основной"));
        assert_eq!(
            variant_children[2]
                .children()
                .filter(roxmltree::Node::is_element)
                .count(),
            0
        );
    }
}

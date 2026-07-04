#![allow(dead_code, unused_imports)]

use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::AdapterOutcome;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::common::*;
use super::{cf::*, cfe::*, form::*, interface::*, meta::*, mxl::*, role::*, skd::*, subsystem::*};
pub(crate) fn add_template(
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
        let template_type =
            required_string(args, &["templateType", "TemplateType"], "TemplateType")?;
        let (metadata_type, extension) = template_type_info(template_type)?;
        let synonym = string_arg(args, &["synonym", "Synonym"]).unwrap_or(template_name);
        let set_main_skd = bool_arg(args, &["setMainSKD", "SetMainSKD"]);
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

        let template_ext_dir = templates_dir_abs.join(template_name).join("Ext");
        fs::create_dir_all(&template_ext_dir)
            .map_err(|err| format!("failed to create {}: {err}", template_ext_dir.display()))?;

        let format_version = detect_format_version(&src_dir_abs);
        let template_uuid = fresh_uuid();
        let template_meta_xml = template_metadata_xml(
            template_name,
            synonym,
            metadata_type,
            &format_version,
            &template_uuid,
        );
        write_utf8_bom(&template_meta_path, &template_meta_xml)?;

        let template_file_display = templates_dir_display
            .join(template_name)
            .join("Ext")
            .join(format!("Template{extension}"));
        let template_file_path = template_ext_dir.join(format!("Template{extension}"));
        if template_type == "BinaryData" {
            fs::File::create(&template_file_path).map_err(|err| {
                format!("failed to write {}: {err}", template_file_path.display())
            })?;
        } else {
            write_utf8_bom(
                &template_file_path,
                &template_content_xml(template_type, extension)?,
            )?;
        }

        let xml_text = read_utf8_sig(&root_xml_path)?;
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
                update_main_data_composition_schema_text(&xml_text, template_name, set_main_skd);
            xml_text = new_text;
            main_dcs_updated = updated;
            main_dcs_value = value;
        }
        if !xml_text.ends_with('\n') {
            xml_text.push('\n');
        }
        write_utf8_bom(&root_xml_path, &xml_text)?;

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

        Ok((
            stdout,
            vec![
                format!("created {}", template_meta_path.display()),
                format!("created {}", template_file_path.display()),
                format!("updated {}", root_xml_path.display()),
            ],
            vec![
                template_meta_path.display().to_string(),
                template_file_path.display().to_string(),
                root_xml_path.display().to_string(),
            ],
        ))
    })();

    match result {
        Ok((stdout, changes, artifacts)) => AdapterOutcome {
            ok: true,
            summary: "unica.template.add completed with native template writer".to_string(),
            changes,
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts,
            stdout: Some(stdout),
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
    let result = (|| -> Result<(String, Vec<String>), String> {
        let object_name = required_string(
            args,
            &["objectName", "ObjectName", "processorName", "ProcessorName"],
            "ObjectName",
        )?;
        let template_name =
            required_string(args, &["templateName", "TemplateName"], "TemplateName")?;
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

        let mut stdout = String::new();
        let mut changes = Vec::new();
        if template_dir_path.is_dir() {
            fs::remove_dir_all(&template_dir_path).map_err(|err| {
                format!("failed to remove {}: {err}", template_dir_path.display())
            })?;
            stdout.push_str(&format!(
                "[OK] Удалён каталог: {}\n",
                template_dir_display.display()
            ));
            changes.push(format!("removed directory {}", template_dir_path.display()));
        }

        fs::remove_file(&template_meta_path)
            .map_err(|err| format!("failed to remove {}: {err}", template_meta_path.display()))?;
        stdout.push_str(&format!(
            "[OK] Удалён файл: {}\n",
            template_meta_display.display()
        ));
        changes.push(format!("removed file {}", template_meta_path.display()));

        let xml_text = read_utf8_sig(&root_xml_path)?;
        let xml_text = remove_template_child_text_lxml(&xml_text, template_name);
        let (mut xml_text, main_dcs_cleared) =
            clear_main_data_composition_schema_text(&xml_text, template_name);
        if !xml_text.ends_with('\n') {
            xml_text.push('\n');
        }
        if main_dcs_cleared {
            stdout.push_str("[OK] Очищён MainDataCompositionSchema\n");
            changes.push("cleared MainDataCompositionSchema".to_string());
        }
        write_utf8_bom(&root_xml_path, &xml_text)?;
        changes.push(format!("updated {}", root_xml_path.display()));

        stdout.push_str(&format!(
            "[OK] Макет {template_name} удалён из {}\n",
            root_xml_display.display()
        ));
        Ok((stdout, changes))
    })();

    match result {
        Ok((stdout, changes)) => AdapterOutcome {
            ok: true,
            summary: "unica.template.remove completed with native template remover".to_string(),
            changes,
            warnings: Vec::new(),
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
        "HTML" => Ok(("HTMLDocument", ".html")),
        "Text" => Ok(("TextDocument", ".txt")),
        "SpreadsheetDocument" => Ok(("SpreadsheetDocument", ".xml")),
        "BinaryData" => Ok(("BinaryData", ".bin")),
        "DataCompositionSchema" => Ok(("DataCompositionSchema", ".xml")),
        other => Err(format!(
            "argument -TemplateType: invalid choice: '{other}' (choose from 'HTML', 'Text', 'SpreadsheetDocument', 'BinaryData', 'DataCompositionSchema')"
        )),
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
        "SpreadsheetDocument" => Ok(concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<SpreadsheetDocument xmlns=\"http://v8.1c.ru/spreadsheet/document\"",
            " xmlns:ss=\"http://v8.1c.ru/spreadsheet/document\"",
            " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
            " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\">\n",
            "</SpreadsheetDocument>"
        )
        .to_string()),
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
    for close in ["</ChildObjects>", "</md:ChildObjects>"] {
        if let Some(index) = xml_text.find(close) {
            let prefix = if close.starts_with("</md:") {
                "md:"
            } else {
                ""
            };
            let line_start = xml_text[..index].rfind('\n').map_or(0, |pos| pos + 1);
            let closing_indent = &xml_text[line_start..index];
            let line = format!(
                "\t<{prefix}{local_name}>{item_name}</{prefix}{local_name}>\n{closing_indent}"
            );
            let mut result = String::with_capacity(xml_text.len() + line.len());
            result.push_str(&xml_text[..index]);
            result.push_str(&line);
            result.push_str(&xml_text[index..]);
            return Some(result);
        }
    }

    for empty in ["<ChildObjects/>", "<md:ChildObjects/>"] {
        if let Some(index) = xml_text.find(empty) {
            let prefix = if empty.starts_with("<md:") { "md:" } else { "" };
            let replacement = format!(
                "<{prefix}ChildObjects>\n\t\t\t<{prefix}{local_name}>{item_name}</{prefix}{local_name}>\n\t\t</{prefix}ChildObjects>"
            );
            let mut result = String::with_capacity(xml_text.len() + replacement.len());
            result.push_str(&xml_text[..index]);
            result.push_str(&replacement);
            result.push_str(&xml_text[index + empty.len()..]);
            return Some(result);
        }
    }

    None
}

pub(crate) fn update_main_data_composition_schema_text(
    xml_text: &str,
    template_name: &str,
    set_main_skd: bool,
) -> (String, bool, String) {
    let Some((object_type, object_start)) = ["ExternalReport", "Report"]
        .iter()
        .find_map(|name| find_open_tag(xml_text, name).map(|index| (*name, index)))
    else {
        return (xml_text.to_string(), false, String::new());
    };
    let object_name = first_tag_text_after(xml_text, "Name", object_start);
    let Some((open_start, content_start, close_start, close_end, open_tag, close_tag)) =
        find_element_bounds(xml_text, "MainDataCompositionSchema", object_start)
    else {
        return (xml_text.to_string(), false, String::new());
    };
    let content = xml_text[content_start..close_start].trim();
    if !content.is_empty() && !set_main_skd {
        return (xml_text.to_string(), false, String::new());
    }
    let value = format!("{object_type}.{object_name}.Template.{template_name}");
    let replacement = format!("{open_tag}{value}{close_tag}");
    let mut result = String::with_capacity(xml_text.len() + value.len());
    result.push_str(&xml_text[..open_start]);
    result.push_str(&replacement);
    result.push_str(&xml_text[close_end..]);
    (result, true, value)
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
    use super::*;

    #[test]
    fn fresh_uuid_generates_uuid_v4() {
        let value = fresh_uuid();
        let uuid = uuid::Uuid::parse_str(&value).expect(&value);

        assert!(!uuid.is_nil(), "{value}");
        assert_eq!(uuid.get_version(), Some(uuid::Version::Random), "{value}");
    }
}

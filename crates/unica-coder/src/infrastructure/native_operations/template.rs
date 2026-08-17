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
    mutation: MutationData,
    stdout: String,
    changes: Vec<String>,
    artifacts: Vec<String>,
    warnings: Vec<String>,
}

pub(crate) struct TemplateExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<MutationData>,
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

pub(crate) fn html_template_page() -> &'static str {
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

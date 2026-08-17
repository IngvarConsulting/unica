use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use roxmltree::Document;
use std::path::Path;

use super::common::*;

pub(crate) fn help_metadata_xml(lang: &str, format_version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Help xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{}\">\n\
\t<Page>{}</Page>\n\
</Help>",
        escape_xml(format_version),
        escape_xml(lang)
    )
}

pub(crate) fn help_page_html(object_name: &str) -> String {
    format!(
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">\
<html><head>\
<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></meta>\
<link rel=\"stylesheet\" type=\"text/css\" href=\"v8help://service_book/service_style\"></link>\
</head><body>\n    <h1>{}</h1>\n    <p>Описание.</p>\n</body></html>",
        escape_xml(object_name)
    )
}

pub(crate) fn validate_help_xml(path: &Path, text: &str) -> Result<(), String> {
    Document::parse(text.trim_start_matches('\u{feff}'))
        .map(|_| ())
        .map_err(|error| format!("XML parse error in {}: {error}", path.display()))
}

pub(crate) fn validate_help_form_owner_8_3_27(path: &Path, text: &str) -> Result<(), String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error in {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject" || root.tag_name().namespace() != Some(MD_NS) {
        return Err(format!(
            "help.add form owner {} must have the fixed 8.3.27 MetaDataObject root",
            path.display()
        ));
    }
    if root.attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format) {
        return Err(format!(
            "help.add form owner {} must use export format {}",
            path.display(),
            ACTIVE_FORMAT_PROFILE.export_format
        ));
    }

    let forms = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == "Form"
        })
        .collect::<Vec<_>>();
    if forms.len() != 1 {
        return Err(format!(
            "help.add form owner {} must contain exactly one MDClasses Form element",
            path.display()
        ));
    }
    let form = forms[0];
    let properties = form
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == "Properties"
        })
        .ok_or_else(|| {
            format!(
                "help.add form owner {} is missing Form/Properties",
                path.display()
            )
        })?;
    let property = |name: &str| {
        properties.children().find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_NS)
                && node.tag_name().name() == name
        })
    };

    let form_type = property("FormType")
        .and_then(|node| node.text())
        .unwrap_or("");
    if !matches!(form_type, "Managed" | "Ordinary") {
        return Err(format!(
            "help.add form owner {} property Form.FormType value '{form_type}' is not valid for the fixed 8.3.27 contract; expected Managed or Ordinary",
            path.display()
        ));
    }
    if let Some(include_help) = property("IncludeHelpInContents") {
        let value = include_help.text().unwrap_or("");
        if !matches!(value, "true" | "false") {
            return Err(format!(
                "help.add form owner {} property Form.IncludeHelpInContents value '{value}' is not a canonical xs:boolean for the fixed 8.3.27 contract; expected true or false",
                path.display()
            ));
        }
    }

    Ok(())
}

pub(crate) fn form_with_include_help(text: &str) -> Option<String> {
    if text.contains("<IncludeHelpInContents>") {
        return None;
    }
    let insert_at = text
        .find("</FormType>")
        .map(|index| index + "</FormType>".len())?;
    let mut updated = text.to_string();
    updated.insert_str(
        insert_at,
        "\n\t\t\t<IncludeHelpInContents>false</IncludeHelpInContents>",
    );
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    Some(updated)
}

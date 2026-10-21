//! Existing template descriptor format observation.
/// `TemplateType` of a template descriptor.
pub fn template_type_of(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).ok()?;
    let document = roxmltree::Document::parse(text).ok()?;
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "TemplateType")
        .and_then(|node| node.text())
        .map(|text| text.trim().to_string())
}

pub fn html_template_descriptor(format_version: &str) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<Help xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" ",
            "xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "version=\"{}\">\r\n",
            "\t<Page>ru</Page>\r\n",
            "</Help>"
        ),
        unica_xml_format::text::escape_xml(format_version)
    )
}

pub fn html_template_page() -> &'static str {
    concat!(
        "\u{feff}",
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.0 Transitional//EN\">",
        "<html><head>",
        "<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></meta>",
        "<link rel=\"stylesheet\" type=\"text/css\" ",
        "href=\"v8help://service_book/service_style\"></link>",
        "</head><body>\n",
        "</body></html>"
    )
}

pub fn template_content_xml(template_type: &str, _extension: &str) -> Result<String, String> {
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
        "SpreadsheetDocument" => Ok(crate::mxl_root::empty_spreadsheet_document_xml()),
        "DataCompositionSchema" => Ok(concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\"",
            " xmlns:dcscom=\"http://v8.1c.ru/8.1/data-composition-system/common\"",
            " xmlns:dcscor=\"http://v8.1c.ru/8.1/data-composition-system/core\"",
            " xmlns:dcsset=\"http://v8.1c.ru/8.1/data-composition-system/settings\"",
            " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
            " xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"",
            " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"",
            " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\r\n",
            "\t<dataSource>\r\n",
            "\t\t<name>ИсточникДанных1</name>\r\n",
            "\t\t<dataSourceType>Local</dataSourceType>\r\n",
            "\t</dataSource>\r\n",
            "\t<settingsVariant>\r\n",
            "\t\t<dcsset:name>Основной</dcsset:name>\r\n",
            "\t\t<dcsset:presentation xsi:type=\"xs:string\">Основной</dcsset:presentation>\r\n",
            "\t\t<dcsset:settings xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"/>\r\n",
            "\t</settingsVariant>\r\n",
            "</DataCompositionSchema>"
        )
        .to_string()),
        "BinaryData" => Ok(String::new()),
        other => Err(format!("unsupported template type: {other}")),
    }
}

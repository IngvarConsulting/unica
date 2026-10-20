//! Existing template descriptor emission and exact native argument validation.
use std::ffi::OsStr;
use std::path::{Component, Path};
pub fn template_type_info(template_type: &str) -> Result<(&'static str, &'static str), String> {
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

pub fn validate_template_metadata_name(argument: &str, value: &str) -> Result<(), String> {
    let mut components = Path::new(value).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(Component::Normal(component)) if component == OsStr::new(value)
    ) && components.next().is_none();

    if unica_xml_format::names::xml_ncname_is_valid(value) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "{argument} must be a valid Unicode XML NCName and a single path component: {value:?}"
        ))
    }
}

pub fn template_metadata_xml(
    template_name: &str,
    synonym: &str,
    metadata_type: &str,
    format_version: &str,
    template_uuid: &str,
) -> String {
    let template_name = unica_xml_format::text::escape_xml(template_name);
    let synonym = unica_xml_format::text::escape_xml(synonym).replace('\r', "&#13;");
    let metadata_type = unica_xml_format::text::escape_xml(metadata_type);
    let format_version = unica_xml_format::text::escape_xml(format_version);
    let template_uuid = unica_xml_format::text::escape_xml(template_uuid);
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

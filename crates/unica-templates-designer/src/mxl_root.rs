//! Existing spreadsheet XML root and empty document image.
pub const MXL_DOCUMENT_NS: &str = "http://v8.1c.ru/8.2/data/spreadsheet";

pub fn require_mxl_document_root(root: roxmltree::Node<'_, '_>) -> Result<(), String> {
    let local_name = root.tag_name().name();
    if local_name != "document" {
        return Err(format!(
            "Root element is '{local_name}', expected 'document'"
        ));
    }
    let namespace = root.tag_name().namespace().unwrap_or("");
    if namespace != MXL_DOCUMENT_NS {
        return Err(format!(
            "Root namespace is '{namespace}', expected '{MXL_DOCUMENT_NS}'"
        ));
    }
    Ok(())
}

pub fn empty_spreadsheet_document_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<document xmlns=\"{MXL_DOCUMENT_NS}\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\r\n\t<languageSettings>\r\n\t\t<currentLanguage>ru</currentLanguage>\r\n\t\t<defaultLanguage>ru</defaultLanguage>\r\n\t\t<languageInfo>\r\n\t\t\t<id>ru</id>\r\n\t\t\t<code>Русский</code>\r\n\t\t\t<description>Русский</description>\r\n\t\t</languageInfo>\r\n\t</languageSettings>\r\n\t<columns>\r\n\t\t<size>0</size>\r\n\t</columns>\r\n\t<rowsItem>\r\n\t\t<index>0</index>\r\n\t\t<row>\r\n\t\t\t<empty>true</empty>\r\n\t\t</row>\r\n\t</rowsItem>\r\n\t<templateMode>true</templateMode>\r\n\t<vgRows>0</vgRows>\r\n</document>"
    )
}

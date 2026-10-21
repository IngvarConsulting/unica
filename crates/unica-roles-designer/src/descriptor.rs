//! Role descriptor validation from already-read bytes.
use crate::writer::{
    decode_role_xml, direct_role_children, role_text_content, ROLE_METADATA_NAMESPACE,
};
use roxmltree::Document;

pub fn validate_role_edit_descriptor(raw: &[u8], role_name: &str) -> Result<(), String> {
    let (_, text) = decode_role_xml(raw)?;
    let document =
        Document::parse(&text).map_err(|_| "role descriptor is not well-formed XML".to_string())?;
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject"
        || root.tag_name().namespace() != Some(ROLE_METADATA_NAMESPACE)
        || unica_source_designer::roots::root_version_literal(&text, root).as_deref()
            != Some("2.20")
    {
        return Err("role descriptor must use exact MDClasses format 2.20".to_string());
    }
    let roles = direct_role_children(root, "Role", ROLE_METADATA_NAMESPACE);
    if roles.len() != 1 {
        return Err("role descriptor must contain exactly one direct Role".to_string());
    }
    let properties = direct_role_children(roles[0], "Properties", ROLE_METADATA_NAMESPACE);
    if properties.len() != 1 {
        return Err("Role must contain exactly one direct Properties element".to_string());
    }
    let names = direct_role_children(properties[0], "Name", ROLE_METADATA_NAMESPACE);
    if names.len() != 1 || role_text_content(names[0]) != role_name {
        return Err("Role.Properties.Name does not match metadataPath".to_string());
    }
    Ok(())
}

/// `lang` is the `LanguageCode` the synonym is written in.
pub fn role_metadata_xml(
    role_name: &str,
    synonym: &str,
    comment: &str,
    format_version: &str,
    uid: &str,
    lang: &str,
) -> String {
    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"".to_string());
    lines.push("        xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\"".to_string());
    lines.push(
        "        xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"".to_string(),
    );
    lines.push("        xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\"".to_string());
    lines.push("        xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\"".to_string());
    lines.push("        xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\"".to_string());
    lines.push("        xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\"".to_string());
    lines.push("        xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\"".to_string());
    lines.push("        xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"".to_string());
    lines.push("        xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\"".to_string());
    lines.push("        xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"".to_string());
    lines.push("        xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\"".to_string());
    lines.push("        xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\"".to_string());
    lines.push("        xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\"".to_string());
    lines.push("        xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"".to_string());
    lines.push("        xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"".to_string());
    lines.push("        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"".to_string());
    lines.push(format!("        version=\"{format_version}\">"));
    lines.push(format!("    <Role uuid=\"{uid}\">"));
    lines.push("        <Properties>".to_string());
    lines.push(format!(
        "            <Name>{}</Name>",
        unica_xml_format::text::escape_xml(role_name)
    ));
    lines.push("            <Synonym>".to_string());
    lines.push("                <v8:item>".to_string());
    lines.push(format!(
        "                    <v8:lang>{}</v8:lang>",
        unica_xml_format::text::escape_xml(lang)
    ));
    lines.push(format!(
        "                    <v8:content>{}</v8:content>",
        unica_xml_format::text::escape_xml(synonym)
    ));
    lines.push("                </v8:item>".to_string());
    lines.push("            </Synonym>".to_string());
    if comment.is_empty() {
        lines.push("            <Comment/>".to_string());
    } else {
        lines.push(format!(
            "            <Comment>{}</Comment>",
            unica_xml_format::text::escape_xml(comment)
        ));
    }
    lines.push("        </Properties>".to_string());
    lines.push("    </Role>".to_string());
    lines.push("</MetaDataObject>".to_string());
    format!("{}\n", lines.join("\n"))
}

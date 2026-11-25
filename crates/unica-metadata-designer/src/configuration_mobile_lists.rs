// module: private

use roxmltree::{Document, Node};
use unica_metadata::configuration::{ConfigurationMobileList, IncomingShareRequestType};
use unica_metadata::descriptor_operations::DescriptorProperty;
use unica_source_designer::owner::MD_CLASSES_NS;
use unica_xml_format::read::{
    child_element_in_namespace, child_elements_in_namespace, direct_text,
};

const CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";
const APP_NS: &str = "http://v8.1c.ru/8.2/managed-application/core";
const XS_NS: &str = "http://www.w3.org/2001/XMLSchema";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn non_nilled(node: Node<'_, '_>) -> bool {
    node.attribute((XSI_NS, "nil"))
        .is_none_or(|value| matches!(value.trim(), "false" | "0"))
}

fn has_type(node: Node<'_, '_>, namespace: &str, local: &str) -> bool {
    let Some(name) = node.attribute((XSI_NS, "type")).map(str::trim) else {
        return false;
    };
    match name.split_once(':') {
        Some((prefix, name)) => {
            !prefix.is_empty()
                && name == local
                && node.lookup_namespace_uri(Some(prefix)) == Some(namespace)
        }
        None => name == local && node.lookup_namespace_uri(None) == Some(namespace),
    }
}

fn property_node<'a, 'input>(
    properties: Node<'a, 'input>,
    property: DescriptorProperty,
) -> Result<Option<Node<'a, 'input>>, String> {
    let tag = property.names()[0];
    match child_elements_in_namespace(properties, tag, MD_CLASSES_NS).as_slice() {
        [] => Ok(None),
        [node] => Ok(Some(*node)),
        _ => Err(format!("duplicate Configuration property {tag}")),
    }
}

fn scalar(node: Node<'_, '_>) -> Option<String> {
    (non_nilled(node) && !node.children().any(|child| child.is_element()))
        .then(|| direct_text(node))
}

fn read_share_type(node: Node<'_, '_>) -> Option<IncomingShareRequestType> {
    if !non_nilled(node)
        || !has_type(node, APP_NS, "AllowedIncomingShareRequestType")
        || !direct_text(node).trim().is_empty()
        || node.children().filter(Node::is_element).count() != 5
    {
        return None;
    }
    let member = |key| match child_elements_in_namespace(node, key, APP_NS).as_slice() {
        [field] => Some(*field),
        _ => None,
    };
    let processing = member("processingVariant")?;
    if !has_type(processing, XS_NS, "decimal") {
        return None;
    }
    let processing_variant = match scalar(processing)?.trim() {
        "0" => 0,
        "1" => 1,
        _ => return None,
    };
    let is_custom = match scalar(member("isCustom")?)?.trim() {
        "false" | "0" => false,
        "true" | "1" => true,
        _ => return None,
    };
    Some(IncomingShareRequestType {
        mime: scalar(member("mime")?)?,
        uti: scalar(member("uti")?)?,
        ext: scalar(member("ext")?)?,
        processing_variant,
        is_custom,
    })
}

pub(crate) fn read(
    properties: Node<'_, '_>,
    property: DescriptorProperty,
) -> Result<Option<ConfigurationMobileList>, String> {
    let Some(node) = property_node(properties, property)? else {
        return Ok(None);
    };
    let invalid = || format!("{} contains an invalid list item", property.names()[0]);
    if !non_nilled(node) || !direct_text(node).trim().is_empty() {
        return Err(invalid());
    }
    let values = node.children().filter(Node::is_element).map(|value| {
        if value.tag_name().name() == "Value" && value.tag_name().namespace() == Some(CORE_NS) {
            Ok(value)
        } else {
            Err(invalid())
        }
    });
    let value = match property {
        DescriptorProperty::MobileApplicationUrls => {
            let items = values
                .map(|item| {
                    let item = item?;
                    if !has_type(item, XS_NS, "string") {
                        return Err(invalid());
                    }
                    scalar(item).ok_or_else(invalid)
                })
                .collect::<Result<Vec<_>, _>>()?;
            ConfigurationMobileList::Urls(items)
        }
        DescriptorProperty::AllowedIncomingShareRequestTypes => {
            let items = values
                .map(|item| read_share_type(item?).ok_or_else(invalid))
                .collect::<Result<Vec<_>, _>>()?;
            ConfigurationMobileList::ShareTypes(items)
        }
        _ => return Err(invalid()),
    };
    Ok(Some(value))
}

fn escape(text: &str) -> String {
    unica_xml_format::text::escape_xml_text(text).replace('\r', "&#13;")
}

pub(crate) fn replace(
    source: &str,
    property: DescriptorProperty,
    value: &ConfigurationMobileList,
) -> Result<String, String> {
    let doc = Document::parse(source).map_err(|error| error.to_string())?;
    let configuration =
        child_element_in_namespace(doc.root_element(), "Configuration", MD_CLASSES_NS)
            .ok_or("the source set has no Configuration descriptor")?;
    let properties = child_elements_in_namespace(configuration, "Properties", MD_CLASSES_NS);
    let [properties] = properties.as_slice() else {
        return Err("the configuration root must have one Properties element".into());
    };
    let name = property.names()[0];
    let existing = property_node(*properties, property)?;
    // Read the current image before replacing it to preserve unchanged XML
    // and to distinguish edit from recovery of damaged input.
    if read(*properties, property)?.as_ref() == Some(value) {
        return Ok(source.to_string());
    }
    let (range, place) = match existing {
        Some(node) => (node.range(), node),
        None => {
            let anchor = [
                "AllowedIncomingShareRequestTypes",
                "MainClientApplicationWindowInterfaceVariant",
                "MainClientApplicationWindowMode",
            ]
            .into_iter()
            .filter(|tag| *tag != name)
            .find_map(|tag| child_element_in_namespace(*properties, tag, MD_CLASSES_NS))
            .ok_or_else(|| format!("no following platform property to place {name} before"))?;
            let position = anchor.range().start;
            (position..position, anchor)
        }
    };
    let spelling = source[place.range().start + 1..place.range().end]
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '>' | '/'))
        .next()
        .expect("parsed XML tag");
    let prefix = spelling.split_once(':').map(|(prefix, _)| prefix);
    let tag = prefix.map_or_else(|| name.to_string(), |prefix| format!("{prefix}:{name}"));
    let mut namespace = String::new();
    if properties.lookup_namespace_uri(prefix) != Some(MD_CLASSES_NS) {
        namespace = prefix.map_or_else(
            || format!(" xmlns=\"{MD_CLASSES_NS}\""),
            |prefix| format!(" xmlns:{prefix}=\"{MD_CLASSES_NS}\""),
        );
    }
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let indent = unica_xml_format::text::form_edit_line_indent_at(source, range.start);
    let members = match value {
        ConfigurationMobileList::Urls(items) => items.len(),
        ConfigurationMobileList::ShareTypes(items) => items.len(),
    };
    let mut xml = if members == 0 {
        format!("<{tag}{namespace}/>")
    } else {
        let mut item_ns = String::new();
        for (item_prefix, uri) in [
            ("v8", CORE_NS),
            ("app", APP_NS),
            ("xs", XS_NS),
            ("xsi", XSI_NS),
        ] {
            if prefix == Some(item_prefix)
                || properties.lookup_namespace_uri(Some(item_prefix)) != Some(uri)
            {
                item_ns.push_str(&format!(" xmlns:{item_prefix}=\"{uri}\""));
            }
        }
        let mut xml = format!("<{tag}{namespace}>");
        match value {
            ConfigurationMobileList::Urls(items) => {
                for item in items {
                    xml.push_str(&format!(
                        "{eol}{indent}\t<v8:Value{item_ns} xsi:type=\"xs:string\">{}</v8:Value>",
                        escape(item)
                    ));
                }
            }
            ConfigurationMobileList::ShareTypes(items) => {
                for item in items {
                    xml.push_str(&format!(
                        "{eol}{indent}\t<v8:Value{item_ns} xsi:type=\"app:AllowedIncomingShareRequestType\">"
                    ));
                    for (tag, value) in
                        [("mime", &item.mime), ("uti", &item.uti), ("ext", &item.ext)]
                    {
                        if value.is_empty() {
                            xml.push_str(&format!("{eol}{indent}\t\t<app:{tag}/>"));
                        } else {
                            xml.push_str(&format!(
                                "{eol}{indent}\t\t<app:{tag}>{}</app:{tag}>",
                                escape(value)
                            ));
                        }
                    }
                    xml.push_str(&format!(
                        "{eol}{indent}\t\t<app:processingVariant xsi:type=\"xs:decimal\">{}</app:processingVariant>{eol}{indent}\t\t<app:isCustom>{}</app:isCustom>{eol}{indent}\t</v8:Value>",
                        item.processing_variant, item.is_custom,
                    ));
                }
            }
        }
        xml.push_str(&format!("{eol}{indent}</{tag}>"));
        xml
    };
    if existing.is_none() {
        xml.push_str(eol);
        xml.push_str(&indent);
    }
    let mut updated = source.to_string();
    updated.replace_range(range, &xml);
    let postimage =
        Document::parse(&updated).map_err(|error| format!("invalid XML in {name}: {error}"))?;
    let configuration =
        child_element_in_namespace(postimage.root_element(), "Configuration", MD_CLASSES_NS)
            .ok_or("the postimage has no Configuration descriptor")?;
    let properties = child_element_in_namespace(configuration, "Properties", MD_CLASSES_NS)
        .ok_or("the postimage has no Properties element")?;
    if read(properties, property)?.as_ref() != Some(value) {
        return Err(format!(
            "the postimage does not contain the requested {name}"
        ));
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_list_writer_keeps_xml_type_when_the_application_prefix_is_shadowed() {
        let source = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" xmlns:app="{APP_NS}"><Configuration><Properties>
                <app:AllowedIncomingShareRequestTypes xmlns:app="{MD_CLASSES_NS}"/>
            </Properties></Configuration></MetaDataObject>"#
        );
        let value = ConfigurationMobileList::ShareTypes(vec![IncomingShareRequestType {
            mime: "text/plain".into(),
            uti: String::new(),
            ext: String::new(),
            processing_variant: 0,
            is_custom: false,
        }]);
        let updated = replace(
            &source,
            DescriptorProperty::AllowedIncomingShareRequestTypes,
            &value,
        )
        .unwrap();
        assert!(updated.contains(&format!(
            "<app:AllowedIncomingShareRequestTypes xmlns:app=\"{MD_CLASSES_NS}\""
        )));
    }

    #[test]
    fn mobile_list_edits_preserve_adjacent_bytes_and_escape_text() {
        let input = format!(
            "<MetaDataObject xmlns=\"{MD_CLASSES_NS}\"><Configuration><Properties>\r\n             <MobileApplicationURLs/>\r\n<Neighbor>keep</Neighbor>\n             <AllowedIncomingShareRequestTypes/>\r\n</Properties></Configuration></MetaDataObject>"
        );
        let urls = ConfigurationMobileList::Urls(vec!["https://example.test/?a=1&b=2".into()]);
        let share = ConfigurationMobileList::ShareTypes(vec![IncomingShareRequestType {
            mime: "text/plain".into(),
            uti: String::new(),
            ext: String::new(),
            processing_variant: 1,
            is_custom: true,
        }]);
        let first = replace(&input, DescriptorProperty::MobileApplicationUrls, &urls).unwrap();
        let second = replace(
            &first,
            DescriptorProperty::AllowedIncomingShareRequestTypes,
            &share,
        )
        .unwrap();
        assert!(second.contains("\r\n<Neighbor>keep</Neighbor>\n"));
        assert!(second.contains("?a=1&amp;b=2"));
        let doc = Document::parse(&second).unwrap();
        let properties = doc
            .descendants()
            .find(|node| node.has_tag_name("Properties"))
            .unwrap();
        assert_eq!(
            read(properties, DescriptorProperty::MobileApplicationUrls).unwrap(),
            Some(urls)
        );
        assert_eq!(
            read(
                properties,
                DescriptorProperty::AllowedIncomingShareRequestTypes
            )
            .unwrap(),
            Some(share)
        );
    }
}

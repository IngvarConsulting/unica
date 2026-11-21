// module: public

use roxmltree::Node;
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_metadata::descriptor_operations::DescriptorProperty;
use unica_source_designer::layout::ORDINARY_INTERFACE_XML_TAG;
use unica_source_designer::owner::MD_CLASSES_NS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationReferenceError {
    Malformed,
    UnsupportedKind,
}

pub fn xml_kind(kind: NodeKind) -> &'static str {
    if kind == NodeKind::OrdinaryInterface {
        ORDINARY_INTERFACE_XML_TAG
    } else {
        kind.as_str()
    }
}

fn property_node<'a, 'input>(
    properties: Node<'a, 'input>,
    property: DescriptorProperty,
) -> Result<Option<Node<'a, 'input>>, ConfigurationReferenceError> {
    let mut matches = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == property.names()[0]);
    let first = matches.next();
    if matches.next().is_some()
        || first.is_some_and(|node| node.tag_name().namespace() != Some(MD_CLASSES_NS))
    {
        return Err(ConfigurationReferenceError::Malformed);
    }
    Ok(first)
}

fn decode_node(
    node: Node<'_, '_>,
    property: DescriptorProperty,
) -> Result<Option<String>, ConfigurationReferenceError> {
    if node.children().any(|child| child.is_element()) || node.attributes().len() != 0 {
        return Err(ConfigurationReferenceError::Malformed);
    }
    let text = unica_xml_format::read::direct_text(node);
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let (tag, name) = text
        .split_once('.')
        .ok_or(ConfigurationReferenceError::Malformed)?;
    let kind = property.reference_kinds()[0];
    let logical = format!("{}.{name}", kind.as_str());
    let address = QualifiedAddress::parse(&format!("source:{logical}"))
        .map_err(|_| ConfigurationReferenceError::Malformed)?;
    if !matches!(address.segments(), [segment] if segment.name() == Some(name)) {
        return Err(ConfigurationReferenceError::Malformed);
    }
    if tag != xml_kind(kind) {
        return Err(ConfigurationReferenceError::UnsupportedKind);
    }
    Ok(Some(logical))
}

pub fn read(
    properties: Node<'_, '_>,
    property: DescriptorProperty,
) -> Result<Option<String>, ConfigurationReferenceError> {
    match property_node(properties, property)? {
        Some(node) => decode_node(node, property),
        None => Ok(None),
    }
}

pub(crate) fn replace(
    source: &str,
    property: DescriptorProperty,
    target: Option<&QualifiedAddress>,
) -> Result<String, String> {
    let document = roxmltree::Document::parse(source).map_err(|error| error.to_string())?;
    let properties = crate::configuration_reference_lists::properties(&document)?;
    let tag = property.names()[0];
    let invalid = |error| format!("{tag} has an invalid metadata reference: {error:?}");
    let existing = property_node(properties, property).map_err(invalid)?;
    let target_value = target.map(QualifiedAddress::logical_path);
    if read(properties, property).map_err(invalid)? == target_value {
        return Ok(source.to_string());
    }
    let (range, place) = match existing {
        Some(node) => (node.range(), node),
        None => {
            let anchors: &[&str] = match property {
                DescriptorProperty::DefaultReportAppearanceTemplate => {
                    &["DefaultDynamicListSettingsForm", "DefaultSearchForm"]
                }
                DescriptorProperty::DefaultInterface => &[
                    "Caption",
                    "ShortCaption",
                    "DefaultStyle",
                    "DefaultLanguage",
                    "BriefInformation",
                ],
                DescriptorProperty::DefaultStyle => &["DefaultLanguage", "BriefInformation"],
                _ => return Err(format!("no platform position for {tag}")),
            };
            let place = anchors
                .iter()
                .find_map(|anchor| {
                    unica_xml_format::read::child_element_in_namespace(
                        properties,
                        anchor,
                        MD_CLASSES_NS,
                    )
                })
                .ok_or_else(|| format!("no following platform property for {tag}"))?;
            (place.range().start..place.range().start, place)
        }
    };
    let spelling = source[place.range().start + 1..place.range().end]
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '>' | '/'))
        .next()
        .expect("parsed XML element");
    let prefix = spelling.split_once(':').map(|(prefix, _)| prefix);
    let mut element = prefix.map_or_else(|| tag.to_string(), |prefix| format!("{prefix}:{tag}"));
    let closing = element.clone();
    if properties.lookup_namespace_uri(prefix) != Some(MD_CLASSES_NS) {
        match prefix {
            Some(prefix) => element.push_str(&format!(" xmlns:{prefix}=\"{MD_CLASSES_NS}\"")),
            None => element.push_str(&format!(" xmlns=\"{MD_CLASSES_NS}\"")),
        }
    }
    let mut replacement = if let Some(target) = target {
        let owner = &target.segments()[0];
        let kind = xml_kind(owner.kind());
        let name = unica_xml_format::text::escape_xml_text(owner.name().expect("named target"));
        format!("<{element}>{kind}.{name}</{closing}>")
    } else {
        format!("<{element}/>")
    };
    if existing.is_none() {
        let indent = &source[source[..range.start].rfind('\n').map_or(0, |i| i + 1)..range.start];
        if indent.chars().all(|ch| matches!(ch, '\t' | ' ')) {
            replacement.push_str(if source.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            });
            replacement.push_str(indent);
        }
    }
    let mut updated = source.to_string();
    updated.replace_range(range, &replacement);
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_interface_assignment_and_clear_only_replace_the_platform_property() {
        let source = format!(
            "<m:MetaDataObject xmlns:m=\"{MD_CLASSES_NS}\"><m:Configuration><m:Properties>\r\n\t<m:DefaultInterface/>\r\n\t<m:DefaultLanguage/>\r\n</m:Properties></m:Configuration></m:MetaDataObject>"
        );
        let target = QualifiedAddress::parse("main:OrdinaryInterface.Probe").unwrap();
        let field = DescriptorProperty::DefaultInterface;
        let assigned = replace(&source, field, Some(&target)).unwrap();
        assert_eq!(
            assigned,
            source.replace(
                "<m:DefaultInterface/>",
                "<m:DefaultInterface>Interface.Probe</m:DefaultInterface>"
            )
        );
        assert_eq!(replace(&assigned, field, None).unwrap(), source);
    }

    #[test]
    fn missing_reference_uses_the_next_native_property_when_its_usual_neighbor_is_absent() {
        for (field, address, xml_target, anchor) in [
            (
                DescriptorProperty::DefaultReportAppearanceTemplate,
                "main:CommonTemplate.Probe",
                "CommonTemplate.Probe",
                "DefaultSearchForm",
            ),
            (
                DescriptorProperty::DefaultStyle,
                "main:Style.Probe",
                "Style.Probe",
                "BriefInformation",
            ),
            (
                DescriptorProperty::DefaultInterface,
                "main:OrdinaryInterface.Probe",
                "Interface.Probe",
                "BriefInformation",
            ),
        ] {
            let source = format!(
                "<m:MetaDataObject xmlns:m=\"{MD_CLASSES_NS}\"><m:Configuration><m:Properties>\r\n\t<m:Name>Probe</m:Name>\r\n\t<!-- neighbor -->\r\n\t<m:{anchor}/>\r\n\t<m:DetailedInformation/>\r\n</m:Properties></m:Configuration></m:MetaDataObject>"
            );
            assert_eq!(replace(&source, field, None).unwrap(), source);
            let target = QualifiedAddress::parse(address).unwrap();
            let tag = field.names()[0];
            let expected = source.replace(
                &format!("<m:{anchor}/>"),
                &format!("<m:{tag}>{xml_target}</m:{tag}>\r\n\t<m:{anchor}/>"),
            );
            assert_eq!(replace(&source, field, Some(&target)).unwrap(), expected);
            assert_eq!(replace(&expected, field, Some(&target)).unwrap(), expected);
        }
    }

    #[test]
    fn single_reference_distinguishes_empty_malformed_and_unsupported_values() {
        let field = DescriptorProperty::DefaultInterface;
        for (element, expected) in [
            ("", Ok(None)),
            ("<DefaultInterface/>", Ok(None)),
            (
                "<DefaultInterface>Interface.Probe</DefaultInterface>",
                Ok(Some("OrdinaryInterface.Probe".into())),
            ),
            (
                "<DefaultInterface><Broken/></DefaultInterface>",
                Err(ConfigurationReferenceError::Malformed),
            ),
            (
                "<DefaultInterface>Style.Probe</DefaultInterface>",
                Err(ConfigurationReferenceError::UnsupportedKind),
            ),
        ] {
            let source = format!("<Properties xmlns=\"{MD_CLASSES_NS}\">{element}</Properties>");
            let document = roxmltree::Document::parse(&source).unwrap();
            assert_eq!(read(document.root_element(), field), expected);
        }
    }
}

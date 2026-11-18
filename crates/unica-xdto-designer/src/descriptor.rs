//! XDTO descriptor identity from already-read bytes.
use roxmltree::{Document, Node};
use unica_source_designer::owner::MD_CLASSES_NS;

pub struct XdtoDescriptorIdentity {
    pub name: String,
    pub namespace: String,
}

pub struct XdtoDescriptorFields {
    pub name: Option<String>,
    pub namespace: String,
}

pub fn descriptor_identity(bytes: &[u8]) -> Result<XdtoDescriptorIdentity, String> {
    let fields = descriptor_fields(bytes)?;
    let name = fields
        .name
        .ok_or_else(|| "not_an_xdto_package: XDTO descriptor has no Name".to_string())?;
    Ok(XdtoDescriptorIdentity {
        name,
        namespace: fields.namespace,
    })
}

pub fn descriptor_fields(bytes: &[u8]) -> Result<XdtoDescriptorFields, String> {
    let document = descriptor_document(bytes)?;
    let properties = descriptor_properties(&document)?;
    let name = properties
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Name")))
        .and_then(|name| name.text())
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let namespace = properties
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Namespace")))
        .and_then(|namespace| namespace.text())
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "namespace_mismatch: XDTO descriptor has no Namespace".to_string())?;
    Ok(XdtoDescriptorFields { name, namespace })
}

fn descriptor_document(bytes: &[u8]) -> Result<Document<'_>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "not_an_xdto_package: XDTO descriptor is not UTF-8".to_string())?;
    Document::parse(text)
        .map_err(|_| "not_an_xdto_package: XDTO descriptor is not valid XML".to_string())
}

fn descriptor_properties<'a, 'input>(
    document: &'a Document<'input>,
) -> Result<Node<'a, 'input>, String> {
    let root = document.root_element();
    if !root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")) {
        return Err(
            "not_an_xdto_package: descriptor root must be an MDClasses MetaDataObject".into(),
        );
    }
    let package = root
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "XDTOPackage")))
        .ok_or_else(|| "not_an_xdto_package: descriptor must contain an XDTOPackage".to_string())?;
    package
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Properties")))
        .ok_or_else(|| "namespace_mismatch: XDTO descriptor has no Properties".to_string())
}

/// The descriptor's preferred-language synonym, independent of Package.bin.
/// As in metadata views, Russian is preferred, then the first nonempty text.
/// An absent property is None; an explicitly empty localized value is Some("").
pub fn descriptor_synonym(bytes: &[u8]) -> Result<Option<String>, String> {
    const CORE: &str = unica_xml_format::localized::DATA_CORE_NS;
    let document = descriptor_document(bytes)?;
    let properties = descriptor_properties(&document)?;
    let owners = document
        .root_element()
        .children()
        .filter(Node::is_element)
        .count();
    let containers = properties
        .parent()
        .expect("Properties has an owner")
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .count();
    if owners != 1 || containers != 1 {
        return Err("Synonym requires one XDTOPackage owner and one Properties container".into());
    }
    let synonyms = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Synonym")
        .collect::<Vec<_>>();
    let synonym = match synonyms.as_slice() {
        [] => return Ok(None),
        [node] if node.has_tag_name((MD_CLASSES_NS, "Synonym")) => *node,
        _ => return Err("duplicate or foreign Synonym".into()),
    };
    let formatting = |node: Node<'_, '_>| {
        node.attributes().len() == 0
            && node
                .children()
                .filter(Node::is_text)
                .all(|text| text.text().is_none_or(|text| text.trim().is_empty()))
    };
    if !formatting(synonym) {
        return Err("malformed localized Synonym".into());
    }
    let mut entries = Vec::new();
    let mut languages = std::collections::HashSet::new();
    for item in synonym.children().filter(Node::is_element) {
        if !item.has_tag_name((CORE, "item")) || !formatting(item) {
            return Err("malformed Synonym item".into());
        }
        let children = item.children().filter(Node::is_element).collect::<Vec<_>>();
        let lang = children
            .iter()
            .filter(|node| node.has_tag_name((CORE, "lang")))
            .collect::<Vec<_>>();
        let content = children
            .iter()
            .filter(|node| node.has_tag_name((CORE, "content")))
            .collect::<Vec<_>>();
        let ([lang], [content]) = (lang.as_slice(), content.as_slice()) else {
            return Err("Synonym item requires one lang and one content".into());
        };
        if children.len() != 2
            || children.iter().any(|node| {
                node.attributes().len() != 0 || node.children().any(|child| child.is_element())
            })
        {
            return Err("malformed Synonym language or content".into());
        }
        let language = unica_xml_format::read::direct_text(**lang);
        if language.trim().is_empty() || !languages.insert(language.clone()) {
            return Err("empty or duplicate Synonym language".into());
        }
        entries.push((language, unica_xml_format::read::direct_text(**content)));
    }
    Ok(Some(
        entries
            .iter()
            .filter(|(_, content)| !content.is_empty())
            .find(|(language, _)| language == "ru")
            .or_else(|| entries.iter().find(|(_, content)| !content.is_empty()))
            .map(|(_, content)| content.clone())
            .unwrap_or_default(),
    ))
}

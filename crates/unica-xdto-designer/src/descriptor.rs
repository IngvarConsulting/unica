//! XDTO descriptor identity from already-read bytes.
use roxmltree::Document;
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
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "not_an_xdto_package: XDTO descriptor is not UTF-8".to_string())?;
    let document = Document::parse(text)
        .map_err(|_| "not_an_xdto_package: XDTO descriptor is not valid XML".to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")) {
        return Err(
            "not_an_xdto_package: descriptor root must be an MDClasses MetaDataObject".to_string(),
        );
    }
    let package = root
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "XDTOPackage")))
        .ok_or_else(|| "not_an_xdto_package: descriptor must contain an XDTOPackage".to_string())?;
    let properties = package
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Properties")))
        .ok_or_else(|| "namespace_mismatch: XDTO descriptor has no Properties".to_string())?;
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

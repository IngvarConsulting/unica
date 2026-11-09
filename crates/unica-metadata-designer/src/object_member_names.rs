//! Имена реквизитов и табличных частей объекта из его дескриптора: то, что
//! код модуля объекта видит как свои члены.

use roxmltree::Node;
use unica_metadata::read_codec::ObjectMemberNames;

/// Реквизиты и табличные части с их реквизитами из `ChildObjects` первого
/// объекта дескриптора. Член без имени пропускается.
pub(crate) fn read(descriptor: &[u8]) -> Result<ObjectMemberNames, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(descriptor)?;
    let object = document
        .root_element()
        .children()
        .find(Node::is_element)
        .ok_or_else(|| "metadata descriptor has no object".to_owned())?;
    Ok(ObjectMemberNames {
        attributes: children(object, "Attribute")
            .into_iter()
            .filter_map(name)
            .collect(),
        tabular_sections: children(object, "TabularSection")
            .into_iter()
            .filter_map(|section| {
                Some((
                    name(section)?,
                    children(section, "Attribute")
                        .into_iter()
                        .filter_map(name)
                        .collect(),
                ))
            })
            .collect(),
    })
}

/// The `kind` elements in a node's `ChildObjects`.
fn children<'a, 'input>(node: Node<'a, 'input>, kind: &'static str) -> Vec<Node<'a, 'input>> {
    node.children()
        .find(|child| child.has_tag_name("ChildObjects"))
        .into_iter()
        .flat_map(|list| list.children().filter(|child| child.has_tag_name(kind)))
        .collect()
}

/// The `Properties/Name` of a child object.
fn name(node: Node<'_, '_>) -> Option<String> {
    node.children()
        .find(|child| child.has_tag_name("Properties"))?
        .children()
        .find(|child| child.has_tag_name("Name"))?
        .text()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_descriptor_gives_its_attributes_and_sections_with_columns() {
        let descriptor = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="1"><Properties><Name>Items</Name></Properties><ChildObjects><Attribute uuid="2"><Properties><Name>Article</Name></Properties></Attribute><TabularSection uuid="3"><Properties><Name>Barcodes</Name></Properties><ChildObjects><Attribute uuid="4"><Properties><Name>Barcode</Name></Properties></Attribute></ChildObjects></TabularSection></ChildObjects></Catalog></MetaDataObject>"#;
        assert_eq!(
            read(descriptor.as_bytes()),
            Ok(ObjectMemberNames {
                attributes: vec!["Article".to_owned()],
                tabular_sections: vec![("Barcodes".to_owned(), vec!["Barcode".to_owned()])],
            })
        );
    }

    #[test]
    fn a_descriptor_that_is_no_xml_or_has_no_object_is_refused() {
        assert!(read(b"not xml").is_err());
        assert!(read(br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"/>"#).is_err());
    }
}

//! Observe and replace the sequence's own boundary mode without reserializing XML.
use roxmltree::{Document, Node};
use unica_metadata::descriptor_operations::DescriptorProperty;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const FIELD: &str = "MoveBoundaryOnPosting";

fn direct<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let nodes = parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
        .collect::<Vec<_>>();
    match nodes.as_slice() {
        [] => Ok(None),
        [node] if node.has_tag_name((MD, name)) => Ok(Some(*node)),
        _ => Err(format!(
            "Sequence requires one direct {name} in the metadata namespace"
        )),
    }
}

fn scalar(node: Node<'_, '_>) -> Result<String, String> {
    if node.attributes().len() != 0 || node.children().any(|n| n.is_element()) {
        return Err(format!(
            "Sequence contains a malformed {}",
            node.tag_name().name()
        ));
    }
    Ok(node
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect())
}

fn boundary<'a, 'input>(
    document: &'a Document<'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let root = document.root_element();
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("Sequence must have exactly one owner".into());
    };
    if !root.has_tag_name((MD, "MetaDataObject")) || !owner.has_tag_name((MD, "Sequence")) {
        return Err("Sequence descriptor has a different owner kind".into());
    }
    let props = direct(*owner, "Properties")?.ok_or("Sequence Properties is absent")?;
    let observed_name = scalar(direct(props, "Name")?.ok_or("Sequence Name is absent")?)?;
    if observed_name != name {
        return Err("Sequence identity differs from its address".into());
    }
    let node = direct(props, FIELD)?;
    if let Some(node) = node {
        if !DescriptorProperty::MoveBoundaryOnPosting
            .allowed_values()
            .contains(&scalar(node)?.as_str())
        {
            return Err("Sequence contains an invalid MoveBoundaryOnPosting".into());
        }
    }
    Ok(node)
}

/// Missing is absence, never an invented default. Malformed observations stay local.
pub fn read(source: &str, name: &str) -> Result<Option<String>, String> {
    let document =
        Document::parse(source.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    boundary(&document, name)?.map(scalar).transpose()
}

pub(crate) fn patch(source: &str, name: &str, value: &str) -> Result<String, String> {
    if !DescriptorProperty::MoveBoundaryOnPosting
        .allowed_values()
        .contains(&value)
    {
        return Err("invalid MoveBoundaryOnPosting argument".into());
    }
    let relative = std::path::PathBuf::from("Sequences").join(format!("{name}.xml"));
    let owner = unica_source_designer::owner::prove_already_read_metadata_owner(
        &relative,
        source.as_bytes(),
    )
    .map_err(|e| e.message)?;
    if owner.version() != Some("2.20") {
        return Err("Sequence requires literal Platform XML 2.20".into());
    }
    let document = Document::parse(source).map_err(|e| e.to_string())?;
    let node = boundary(&document, name)?.ok_or("Sequence MoveBoundaryOnPosting is absent")?;
    if scalar(node)? == value {
        return Ok(source.to_string());
    }
    let range = node.range();
    let element = &source[range.clone()];
    let start = range.start + element.find('>').ok_or("scalar opening tag is absent")? + 1;
    let end = range.start + element.rfind("</").ok_or("scalar closing tag is absent")?;
    let mut result = source.to_string();
    result.replace_range(start..end, value);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    const IMAGE: &str = concat!(
        "<m:MetaDataObject xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><m:Sequence uuid=\"16770000-0000-4000-8000-000000000001\">\r\n",
        "<m:InternalInfo/><m:Properties><m:Name>Sales</m:Name><m:Comment>keep</m:Comment>",
        "<m:MoveBoundaryOnPosting>DontMove</m:MoveBoundaryOnPosting><m:Documents/></m:Properties>",
        "<m:ChildObjects/><m:Future><m:MoveBoundaryOnPosting>keep</m:MoveBoundaryOnPosting></m:Future></m:Sequence></m:MetaDataObject>"
    );

    #[test]
    fn sequence_boundary_patch_preserves_all_bytes_outside_the_direct_scalar_and_noops() {
        let changed = patch(IMAGE, "Sales", "Move").unwrap();
        assert_eq!(changed, IMAGE.replace(">DontMove<", ">Move<"));
        assert_eq!(read(&changed, "Sales").unwrap().as_deref(), Some("Move"));
        assert_eq!(patch(&changed, "Sales", "Move").unwrap(), changed);
        assert_eq!(patch(&changed, "Sales", "DontMove").unwrap(), IMAGE);
    }

    #[test]
    fn sequence_boundary_refuses_damaged_scalar_owner_identity_and_inexact_version() {
        for (old, new) in [
            ("m:Sequence", "m:Catalog"), ("<m:Name>Sales</m:Name>", "<m:Name>Other</m:Name>"),
            ("2.20", "2.&#50;0"), ("2.20", "2.21"),
            ("<m:MoveBoundaryOnPosting>DontMove</m:MoveBoundaryOnPosting>", ""),
            ("DontMove", "Unknown"), ("DontMove", "<m:Value>DontMove</m:Value>"),
            ("<m:MoveBoundaryOnPosting>", "<m:MoveBoundaryOnPosting flag=\"true\">"),
            ("<m:MoveBoundaryOnPosting>", "<m:MoveBoundaryOnPosting xmlns:m=\"urn:foreign\">"),
            ("</m:MoveBoundaryOnPosting><m:Documents/>", "</m:MoveBoundaryOnPosting><m:MoveBoundaryOnPosting>Move</m:MoveBoundaryOnPosting><m:Documents/>"),
        ] {
            let source = IMAGE.replace(old, new);
            assert!(patch(&source, "Sales", "Move").is_err(), "{source}");
            assert!(patch(&source, "Sales", "DontMove").is_err(), "invalid no-op: {source}");
        }
        assert!(patch(IMAGE, "Sales", "Unknown").is_err());
        assert_eq!(
            read(
                &IMAGE.replace(
                    "<m:MoveBoundaryOnPosting>DontMove</m:MoveBoundaryOnPosting>",
                    ""
                ),
                "Sales"
            )
            .unwrap(),
            None
        );
    }
}

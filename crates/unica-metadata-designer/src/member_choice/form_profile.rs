//! The field reference type admitted by the standard choice-form profile.
use roxmltree::Node;

const CORE: &str = "http://v8.1c.ru/8.1/data/core";
const CONFIG: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";

fn one<'a, 'i>(parent: Node<'a, 'i>, ns: &str, name: &str) -> Result<Node<'a, 'i>, String> {
    let mut nodes = parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let node = nodes
        .next()
        .ok_or_else(|| format!("standard choice form requires {name}"))?;
    if nodes.next().is_some() || !node.has_tag_name((ns, name)) {
        return Err(format!(
            "standard choice form requires one qualified {name}"
        ));
    }
    Ok(node)
}

fn scalar<'a, 'i>(node: Node<'a, 'i>) -> Result<&'a str, String> {
    if node.attributes().len() != 0 || node.children().any(|n| !n.is_text()) {
        return Err(format!("{} must be a scalar", node.tag_name().name()));
    }
    Ok(node.text().unwrap_or_default())
}

fn configured_type<'a, 'i>(parent: Node<'a, 'i>, ns: &str) -> Result<&'a str, String> {
    let ty = one(parent, ns, "Type")?;
    let node = one(ty, CORE, "Type")?;
    if ty.attributes().len() != 0 || ty.children().filter(|n| n.is_element()).count() != 1 {
        return Err("choiceForm requires a single reference type".into());
    }
    let text = scalar(node)?;
    qualified_name(node, text, CONFIG)
}

fn qualified_name<'a>(
    node: Node<'_, '_>,
    text: &'a str,
    namespace: &str,
) -> Result<&'a str, String> {
    let (prefix, name) = text
        .split_once(':')
        .map_or((None, text), |(prefix, name)| (Some(prefix), name));
    if node.lookup_namespace_uri(prefix) != Some(namespace) {
        return Err("unexpected type namespace in standard choice form".into());
    }
    Ok(name)
}

pub(super) fn reference_table(properties: Node<'_, '_>) -> Result<String, String> {
    const UNSUPPORTED: &str = "choiceForm supports only a single CatalogRef or DocumentRef";
    let name = configured_type(properties, super::MD).map_err(|_| UNSUPPORTED.to_string())?;
    for (prefix, kind) in [("CatalogRef.", "Catalog"), ("DocumentRef.", "Document")] {
        if let Some(name) = name
            .strip_prefix(prefix)
            .filter(|name| !name.is_empty() && !name.contains('.'))
        {
            return Ok(format!("{kind}.{name}"));
        }
    }
    Err(UNSUPPORTED.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    #[test]
    fn reference_profile_requires_one_native_catalog_or_document_reference() {
        let properties = |types: &str| {
            format!("<Properties xmlns=\"{}\" xmlns:v8=\"{CORE}\" xmlns:cfg=\"{CONFIG}\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"><Type>{types}</Type></Properties>", super::super::MD)
        };
        for (types, expected) in [
            (
                "<v8:Type>cfg:CatalogRef.Валюты</v8:Type>",
                Some("Catalog.Валюты"),
            ),
            (
                "<v8:Type>cfg:DocumentRef.Приход</v8:Type>",
                Some("Document.Приход"),
            ),
            ("<v8:Type>xs:string</v8:Type>", None),
            ("<v8:Type>xs:boolean</v8:Type>", None),
            ("<v8:Type>cfg:EnumRef.Mode</v8:Type>", None),
            (
                "<v8:Type>cfg:CatalogRef.Валюты</v8:Type><v8:Type>cfg:DocumentRef.Приход</v8:Type>",
                None,
            ),
            (
                "<v8:Type xmlns:v8=\"foreign\">cfg:CatalogRef.Валюты</v8:Type>",
                None,
            ),
        ] {
            let text = properties(types);
            let doc = Document::parse(&text).unwrap();
            let actual = reference_table(doc.root_element());
            assert_eq!(actual.as_ref().ok().map(String::as_str), expected);
            if expected.is_none() {
                assert_eq!(
                    actual.unwrap_err(),
                    "choiceForm supports only a single CatalogRef or DocumentRef"
                );
            }
        }
    }
}

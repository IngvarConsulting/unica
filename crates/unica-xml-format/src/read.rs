//! Existing DOM/text reads, with exact namespace and text policies.

pub fn attribute_by_local_name<'a>(
    node: roxmltree::Node<'a, '_>,
    local_name: &str,
) -> Option<&'a str> {
    node.attributes()
        .find(|attr| attr.name() == local_name)
        .map(|attr| attr.value())
}

pub fn child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> String {
    node.children()
        .find(|child| crate::names::element_has_name(*child, local_name, namespace))
        .and_then(|child| child.text())
        .unwrap_or("")
        .to_string()
}

pub fn multilang_text(node: roxmltree::Node<'_, '_>) -> String {
    for item in node.children().filter(|child| child.is_element()) {
        let mut lang = "";
        let mut content = "";
        for child in item.children().filter(|child| child.is_element()) {
            match child.tag_name().name() {
                "lang" => lang = child.text().unwrap_or(""),
                "content" => content = child.text().unwrap_or(""),
                _ => {}
            }
        }
        if lang == "ru" && !content.is_empty() {
            return content.to_string();
        }
    }
    for item in node.children().filter(|child| child.is_element()) {
        for child in item.children().filter(|child| child.is_element()) {
            if child.tag_name().name() == "content" {
                if let Some(text) = child.text() {
                    if !text.is_empty() {
                        return text.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

pub fn parse_metadata_image(bytes: &[u8]) -> Result<(&str, roxmltree::Document<'_>), String> {
    let source = std::str::from_utf8(bytes)
        .map_err(|error| format!("metadata image is not UTF-8: {error}"))?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(source)
        .map_err(|error| format!("metadata XML parse failed: {error}"))?;
    Ok((source, document))
}

pub fn child_element<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| crate::names::element_has_name(*child, local_name, None))
}

pub fn child_elements<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| crate::names::element_has_name(*child, local_name, None))
        .collect()
}

pub fn child_direct_text(node: roxmltree::Node<'_, '_>, local_name: &str) -> Option<String> {
    child_element(node, local_name).map(direct_text)
}

pub fn direct_text(node: roxmltree::Node<'_, '_>) -> String {
    node.children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|child| child.text())
        .collect()
}

pub fn first_text(doc: &roxmltree::Document<'_>, local_name: &str) -> Option<String> {
    doc.descendants()
        .find(|node| node.is_element() && node.tag_name().name() == local_name)
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn child_elements_in_namespace<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| crate::names::element_has_name(*child, local_name, Some(namespace)))
        .collect()
}

pub fn child_element_in_namespace<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    namespace: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| crate::names::element_has_name(*child, local_name, Some(namespace)))
}

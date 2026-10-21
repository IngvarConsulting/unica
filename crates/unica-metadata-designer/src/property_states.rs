use roxmltree::Node;
use std::collections::BTreeMap;

const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

/// Read the direct scalar state entries once for borrowing, writing and checking.
/// Whether a property permits Notify, MultiState or requires Extended belongs
/// to its caller; malformed XML and duplicate property names never do.
pub fn read_property_states<'a>(
    internal: Node<'a, '_>,
) -> Result<BTreeMap<&'a str, &'a str>, String> {
    let mut result = BTreeMap::new();
    for state in internal
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "PropertyState")
    {
        let children: Vec<_> = state.children().filter(Node::is_element).collect();
        if !state.has_tag_name((XR, "PropertyState"))
            || children.len() != 2
            || !children[0].has_tag_name((XR, "Property"))
            || !children[1].has_tag_name((XR, "State"))
            || state.children().any(|node| {
                node.is_text() && node.text().is_some_and(|text| !text.trim().is_empty())
            })
            || children.iter().any(|node| {
                node.children().any(|child| child.is_element())
                    || node.children().filter(|child| child.is_text()).count() != 1
                    || node.text().is_none_or(|text| text.trim().is_empty())
            })
        {
            return Err(
                "Malformed PropertyState: expected scalar xr:Property followed by xr:State".into(),
            );
        }
        let property = children[0].text().expect("validated scalar property");
        let value = children[1].text().expect("validated scalar state");
        if result.insert(property, value).is_some() {
            return Err(format!("Duplicate PropertyState for {property}"));
        }
    }
    Ok(result)
}

use roxmltree::Document;
use std::path::Path;
use unica_xml_format::text::{
    cfe_borrow_ensure_namespace_declaration, cfe_borrow_find_start_tag, form_edit_line_indent_at,
};
const CFE_PATCH_MD_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const CFE_PATCH_XR_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/readable";
pub fn cfe_patch_precondition_error(module_path: &str, detail: impl AsRef<str>) -> String {
    format!(
        "ModulePath '{module_path}' is not a borrowed extension object: {}",
        detail.as_ref()
    )
}
pub fn cfe_patch_direct_md_child<'a>(
    document: &'a Document<'a>,
    expected_type: &str,
    module_path: &str,
    path: &Path,
) -> Result<roxmltree::Node<'a, 'a>, String> {
    let root = document.root_element();
    if root.tag_name().name() != "MetaDataObject"
        || root.tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE)
    {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{} must have root {{{CFE_PATCH_MD_NAMESPACE}}}MetaDataObject",
                path.display()
            ),
        ));
    }
    let direct = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if direct.len() != 1
        || direct[0].tag_name().name() != expected_type
        || direct[0].tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE)
    {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{} must contain exactly one direct {{{CFE_PATCH_MD_NAMESPACE}}}{expected_type}",
                path.display()
            ),
        ));
    }
    Ok(direct[0])
}
pub fn cfe_patch_exact_md_child<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    local_name: &str,
    module_path: &str,
    path: &Path,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let children = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == local_name)
        .collect::<Vec<_>>();
    if children.len() != 1 || children[0].tag_name().namespace() != Some(CFE_PATCH_MD_NAMESPACE) {
        return Err(cfe_patch_precondition_error(
            module_path,
            format!(
                "{} must contain exactly one direct {{{CFE_PATCH_MD_NAMESPACE}}}{local_name}",
                path.display()
            ),
        ));
    }
    Ok(children[0])
}
pub fn cfe_patch_mark_extended_property(
    module_path: &str,
    path: &Path,
    raw: &[u8],
    property: &str,
) -> Result<Vec<u8>, String> {
    let raw_text = std::str::from_utf8(raw)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
    let (bom, text) = raw_text
        .strip_prefix('\u{feff}')
        .map_or(("", raw_text), |text| ("\u{feff}", text));
    let document = Document::parse(text)
        .map_err(|error| format!("failed to parse platform XML {}: {error}", path.display()))?;
    let root = document.root_element();
    if let Some(namespace) = root.lookup_namespace_uri(Some("xr")) {
        if namespace != CFE_PATCH_XR_NAMESPACE {
            return Err(cfe_patch_precondition_error(
                module_path,
                format!(
                    "{} binds the xr prefix to unsupported namespace {namespace:?}",
                    path.display()
                ),
            ));
        }
    }
    let object = root
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| {
            cfe_patch_precondition_error(
                module_path,
                format!("{} has no metadata object", path.display()),
            )
        })?;
    let internal_info = cfe_patch_exact_md_child(object, "InternalInfo", module_path, path)?;
    let states = read_property_states(internal_info)
        .map_err(|error| cfe_patch_precondition_error(module_path, error))?;
    match states.get(property) {
        None => {}
        Some(&"Extended") => return Ok(raw.to_vec()),
        Some(state) => {
            return Err(cfe_patch_precondition_error(
                module_path,
                format!(
                    "{} has incompatible PropertyState for {property}: {state:?}",
                    path.display()
                ),
            ));
        }
    }

    let range = internal_info.range();
    let fragment = &text[range.clone()];
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let indent = form_edit_line_indent_at(text, range.start);
    let child_indent = format!("{indent}\t");
    let value_indent = format!("{child_indent}\t");
    let state_xml = format!(
        "{child_indent}<xr:PropertyState>{newline}{value_indent}<xr:Property>{property}</xr:Property>{newline}{value_indent}<xr:State>Extended</xr:State>{newline}{child_indent}</xr:PropertyState>{newline}"
    );
    let replacement = if fragment.trim_end().ends_with("/>") {
        let close = fragment
            .rfind("/>")
            .expect("trimmed empty XML element must have a closing marker");
        let tag_name = fragment
            .strip_prefix('<')
            .and_then(|value| {
                value
                    .split(|character: char| character.is_ascii_whitespace() || character == '/')
                    .next()
            })
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                cfe_patch_precondition_error(
                    module_path,
                    format!("cannot identify InternalInfo tag in {}", path.display()),
                )
            })?;
        format!(
            "{}>{newline}{state_xml}{indent}</{tag_name}>",
            fragment[..close].trim_end()
        )
    } else {
        let close = fragment.rfind("</").ok_or_else(|| {
            cfe_patch_precondition_error(
                module_path,
                format!(
                    "cannot locate InternalInfo closing tag in {}",
                    path.display()
                ),
            )
        })?;
        let insertion = fragment[..close]
            .rfind('\n')
            .map(|line_break| line_break + 1)
            // A closing tag may share its line with the preceding child.
            // Only indentation belongs before the new sibling state.
            .filter(|start| fragment[*start..close].trim().is_empty())
            .unwrap_or(close);
        format!(
            "{}{}{}",
            &fragment[..insertion],
            state_xml,
            &fragment[insertion..]
        )
    };

    let mut updated = format!(
        "{}{}{}",
        &text[..range.start],
        replacement,
        &text[range.end..]
    );
    let root_start = cfe_borrow_find_start_tag(&updated, "MetaDataObject", 0).ok_or_else(|| {
        cfe_patch_precondition_error(
            module_path,
            format!("cannot locate MetaDataObject root in {}", path.display()),
        )
    })?;
    let root_end = updated[root_start..]
        .find('>')
        .map(|offset| root_start + offset + 1)
        .ok_or_else(|| {
            cfe_patch_precondition_error(
                module_path,
                format!(
                    "cannot locate MetaDataObject root end in {}",
                    path.display()
                ),
            )
        })?;
    let root_open = cfe_borrow_ensure_namespace_declaration(
        updated[root_start..root_end].to_string(),
        "xr",
        CFE_PATCH_XR_NAMESPACE,
    );
    updated.replace_range(root_start..root_end, &root_open);
    let updated_with_bom = format!("{bom}{updated}");
    let verified = Document::parse(&updated)
        .map_err(|error| format!("failed to build platform XML {}: {error}", path.display()))?;
    let verified_object =
        cfe_patch_direct_md_child(&verified, object.tag_name().name(), module_path, path)?;
    let verified_info =
        cfe_patch_exact_md_child(verified_object, "InternalInfo", module_path, path)?;
    if read_property_states(verified_info)?.get(property) != Some(&"Extended") {
        return Err(format!(
            "failed to build exactly one Extended PropertyState for {property} in {}",
            path.display()
        ));
    }
    Ok(updated_with_bom.into_bytes())
}

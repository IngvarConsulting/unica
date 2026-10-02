use roxmltree::Node;
use std::collections::BTreeMap;

const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

/// Read the direct scalar state entries once for borrowing, writing and checking.
/// Whether a property permits Notify, MultiState or requires Extended belongs
/// to its caller; malformed XML and duplicate property names never do.
pub(super) fn read_property_states<'a>(
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

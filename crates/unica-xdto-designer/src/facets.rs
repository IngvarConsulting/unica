use roxmltree::Node;
use serde_json::{Map, Value};
pub use unica_xdto::facets::{ValueFacets, NAMES};

pub fn from_node(node: Node<'_, '_>) -> Result<ValueFacets, &'static str> {
    let mut object = Map::new();
    for name in NAMES {
        if let Some(text) = node.attribute(name) {
            let value = if matches!(name, "minInclusive" | "maxInclusive") {
                Value::String(text.to_owned())
            } else {
                Value::from(text.parse::<u64>().map_err(|_| "invalid integer facet")?)
            };
            object.insert(name.to_owned(), value);
        }
    }
    ValueFacets::from_json(Some(&Value::Object(object)))
}

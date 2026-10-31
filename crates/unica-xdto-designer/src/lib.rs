//! Existing Designer XDTO XML model, validation and byte-preserving edits.
//! This owner proposes source images and has no workspace writer authority.
use model::XDTO_NS;
use roxmltree::Document;
use serde_json::{Map, Value};

pub mod model;
pub mod validation;
pub mod writer;

pub fn decode(raw: &[u8]) -> Result<String, String> {
    let value = std::str::from_utf8(raw)
        .map_err(|_| "unsupported_node: Package.bin is not UTF-8".to_string())?;
    let body = value.strip_prefix('\u{feff}').unwrap_or(value);
    if body.starts_with('\u{feff}') {
        return Err(
            "unsupported_node: Package.bin must begin with at most one UTF-8 BOM".to_string(),
        );
    }
    Ok(body.to_string())
}

pub fn encode_like(before: &[u8], text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    if before.starts_with(&[0xef, 0xbb, 0xbf]) {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

pub fn parse(text: &str) -> Result<Document<'_>, String> {
    let doc = Document::parse(text)
        .map_err(|error| format!("unsupported_node: invalid XDTO XML: {error}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "package" || root.tag_name().namespace() != Some(XDTO_NS) {
        return Err("not_an_xdto_package: Package.bin root is not an XDTO package".to_string());
    }
    Ok(doc)
}

pub fn required<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} must be a non-empty string"))
}

pub mod descriptor;
pub mod planner;

pub mod info;

pub mod edit_plan;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub mod facets;
mod serialization;

#[cfg(test)]
mod serialization_tests;

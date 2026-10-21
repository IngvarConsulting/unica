//! Pure descriptor planning for explicit top-level borrowing.
use crate::extension::{cfe_borrow_generated_types, cfe_borrow_object_xml, CfeBorrowIdentity};
use unica_source_designer::configuration_layout::cfe_borrow_type_dir;

use roxmltree::{Document, Node};
use std::collections::BTreeSet;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

#[derive(Debug)]
pub struct BorrowObjectPlan {
    pub parent_uuid: String,
    pub kind: String,
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowObjectErrorKind {
    BadValue,
    InvalidSource,
}

#[derive(Debug)]
pub struct BorrowObjectError {
    pub kind: BorrowObjectErrorKind,
    message: String,
}

impl BorrowObjectError {
    fn bad_value(message: impl Into<String>) -> Self {
        Self {
            kind: BorrowObjectErrorKind::BadValue,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for BorrowObjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for BorrowObjectError {}
impl From<String> for BorrowObjectError {
    fn from(message: String) -> Self {
        Self {
            kind: BorrowObjectErrorKind::InvalidSource,
            message,
        }
    }
}
impl From<&str> for BorrowObjectError {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

pub fn validate_requested_overrides(
    items: &[String],
) -> Result<BTreeSet<String>, BorrowObjectError> {
    let set = items.iter().cloned().collect::<BTreeSet<_>>();
    if set.len() != items.len() {
        return Err(BorrowObjectError::bad_value(
            "Duplicate requested overrides",
        ));
    }
    for name in &set {
        if matches!(name.as_str(), "Comment" | "Synonym") {
            return Err(BorrowObjectError::bad_value(format!(
                "{name} is extension-owned, not a borrowed control property; edit it separately"
            )));
        }
        if reserved(name) {
            return Err(BorrowObjectError::bad_value(format!(
                "Property {name} cannot be requested as an override"
            )));
        }
    }
    Ok(set)
}

pub fn plan_borrow_object_with_identity(
    parent: &[u8],
    existing: Option<&[u8]>,
    requested_overrides: Option<&[String]>,
    identity: &CfeBorrowIdentity,
    fresh_uuid: fn() -> String,
) -> Result<BorrowObjectPlan, BorrowObjectError> {
    let parent_text = utf8(parent)?;
    let parent_doc =
        Document::parse(parent_text).map_err(|e| format!("Invalid parent XML: {e}"))?;
    let parent_object = object(&parent_doc)?;
    let kind = parent_object.tag_name().name();
    if cfe_borrow_type_dir(kind).is_none() || cfe_borrow_generated_types(kind).is_none() {
        return Err(BorrowObjectError::bad_value(format!(
            "Unsupported top-level borrow profile: {kind}"
        )));
    }
    let parent_uuid = identifier(parent_object.attribute("uuid"), "parent uuid")?;
    let parent_props = required(parent_object, MD, "Properties")?;
    validate_properties(parent_props)?;
    let name = required(parent_props, MD, "Name")?
        .text()
        .unwrap_or_default();
    if name.is_empty() {
        return Err("Parent object has no Name".into());
    }
    let version = parent_doc
        .root_element()
        .attribute("version")
        .ok_or("Parent descriptor has no version")?;
    let template = cfe_borrow_object_xml(
        kind,
        name,
        &parent_uuid,
        Some(parent_props),
        version,
        identity,
        fresh_uuid,
    )?;
    let template_doc = Document::parse(&template).map_err(|e| e.to_string())?;
    let template_object = object(&template_doc)?;
    let template_props = required(template_object, MD, "Properties")?;
    let requested = requested_overrides
        .map(|items| {
            let set = validate_requested_overrides(items)?;
            for name in &set {
                if existing.is_none() && override_state(kind, name).is_none() {
                    return Err(BorrowObjectError::bad_value(format!(
                        "No platform-proven override profile for {kind}.{name}"
                    )));
                }
                if optional(parent_props, MD, name)?.is_none() {
                    return Err(BorrowObjectError::bad_value(format!(
                        "Property {name} cannot be requested as an override"
                    )));
                }
            }
            Ok(set)
        })
        .transpose()?;
    let initial = existing.is_none();
    let original = existing.map(utf8).transpose()?.unwrap_or(&template);
    let original_doc =
        Document::parse(original).map_err(|e| format!("Damaged extension descriptor: {e}"))?;
    let target = object(&original_doc)?;
    let target_props = required(target, MD, "Properties")?;
    validate_properties(target_props)?;
    let internal = required(target, MD, "InternalInfo")?;
    let existing_overrides = property_states(internal)?;
    if !initial {
        if target.tag_name().name() != kind
            || required(target_props, MD, "Name")?.text() != Some(name)
            || required(target_props, MD, "ObjectBelonging")?.text() != Some("Adopted")
            || identifier(
                required(target_props, MD, "ExtendedConfigurationObject")?.text(),
                "parent reference",
            )? != parent_uuid
        {
            return Err("Existing object is owned locally or belongs to a different parent; borrowing cannot replace it".into());
        }
        if original_doc.root_element().attribute("version") != Some(version) {
            return Err("Parent and extension descriptor versions differ".into());
        }
        validate_identity(target, kind, name)?;
        if identifier(target.attribute("uuid"), "object uuid")? == parent_uuid {
            return Err("Borrowed object UUID must differ from its parent UUID".into());
        }
        let requested_controls = existing_overrides
            .iter()
            .filter(|name| optional(target_props, MD, name).is_ok_and(|node| node.is_some()))
            .cloned()
            .collect::<BTreeSet<_>>();
        if requested
            .as_ref()
            .is_some_and(|set| set != &requested_controls)
        {
            return Err(BorrowObjectError::bad_value(
                "Changing the existing override set is not part of repeated borrowing",
            ));
        }
    }
    let overrides = if initial {
        requested.unwrap_or_default()
    } else {
        existing_overrides
    };
    let mut patches: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    // The emitter defines transferred properties. Unknown extension properties
    // remain extension-owned even if the parent later adds an equally named field.
    for expected in template_props.children().filter(|n| n.is_element()) {
        let prop = expected.tag_name().name();
        if reserved(prop) || prop == "Comment" || (!initial && overrides.contains(prop)) {
            continue;
        }
        let source = optional(parent_props, MD, prop)?.unwrap_or(expected);
        let target_prop = required(target_props, MD, prop)?;
        if !equivalent(target_prop, source) {
            patches.push((target_prop.range(), standalone_fragment(source)));
        }
    }
    if !initial && kind == "Catalog" {
        // These controlled properties were independently retained by 8.3.27
        // without PropertyState. Do not infer control for unknown properties.
        for prop in ["CodeLength", "DescriptionLength", "Hierarchical"] {
            if overrides.contains(prop) {
                continue;
            }
            if let Some(target_prop) = optional(target_props, MD, prop)? {
                let source = required(parent_props, MD, prop)?;
                if !equivalent(target_prop, source) {
                    patches.push((target_prop.range(), standalone_fragment(source)));
                }
            }
        }
    }
    if initial {
        for source in parent_props.children().filter(|n| {
            n.is_element()
                && n.tag_name().namespace() == Some(MD)
                && (overrides.contains(n.tag_name().name())
                    || (kind == "Catalog"
                        && matches!(n.tag_name().name(), "CodeLength" | "Hierarchical")))
        }) {
            let prop = source.tag_name().name();
            if optional(template_props, MD, prop)?.is_some() {
                continue;
            }
            let source = required(parent_props, MD, prop)?;
            // Keep the platform property order from the parent. Identity fields
            // are supplied by the extension template, never copied from source.
            let before = source
                .next_siblings()
                .filter(|n| n.is_element())
                .find_map(|n| {
                    optional(target_props, MD, n.tag_name().name())
                        .ok()
                        .flatten()
                });
            let pos = before
                .map(|n| n.range().start)
                .unwrap_or_else(|| closing_start(original, target_props));
            patches.push((pos..pos, standalone_fragment(source)));
        }
        if !overrides.is_empty() {
            let states = overrides.iter().map(|name| format!("<xr:PropertyState xmlns:xr=\"{XR}\"><xr:Property>{}</xr:Property><xr:State>{}</xr:State></xr:PropertyState>", unica_xml_format::text::escape_xml(name), override_state(kind, name).expect("validated initial override"))).collect::<String>();
            if internal.children().next().is_none() {
                patches.push((
                    internal.range(),
                    format!("<InternalInfo>{states}</InternalInfo>"),
                ));
            } else {
                let pos = closing_start(original, internal);
                patches.push((pos..pos, states));
            }
        }
    }
    patches.sort_by_key(|(range, _)| (range.start, range.end));
    let mut changed = original.to_string();
    for (range, replacement) in patches.into_iter().rev() {
        changed.replace_range(range, &replacement);
    }
    // Byte offsets above refer to BOM-free text; retain the original encoding marker.
    if existing.is_some_and(|bytes| bytes.starts_with(b"\xef\xbb\xbf")) {
        changed.insert(0, '\u{feff}');
    }
    let check = Document::parse(changed.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("Borrowed descriptor is not well formed: {e}"))?;
    validate_identity(object(&check)?, kind, name)?;
    Ok(BorrowObjectPlan {
        parent_uuid,
        kind: kind.into(),
        name: name.into(),
        bytes: changed.into_bytes(),
    })
}

fn utf8(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes)
        .map(|s| s.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("Descriptor is not UTF-8: {e}"))
}

fn object<'a>(doc: &'a Document<'a>) -> Result<Node<'a, 'a>, String> {
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("Expected MDClasses MetaDataObject descriptor".into());
    }
    let mut objects = root.children().filter(|n| n.is_element());
    let result = objects.next().ok_or("Missing metadata object")?;
    if objects.next().is_some() || result.tag_name().namespace() != Some(MD) {
        return Err("Descriptor must contain exactly one metadata object".into());
    }
    Ok(result)
}

fn optional<'a>(node: Node<'a, 'a>, ns: &str, name: &str) -> Result<Option<Node<'a, 'a>>, String> {
    let mut found = node.children().filter(|n| n.has_tag_name((ns, name)));
    let result = found.next();
    if found.next().is_some() {
        return Err(format!("Duplicate {name} in descriptor"));
    }
    Ok(result)
}

fn required<'a>(node: Node<'a, 'a>, ns: &str, name: &str) -> Result<Node<'a, 'a>, String> {
    optional(node, ns, name)?.ok_or_else(|| format!("Damaged descriptor: missing {name}; restore a confirmed working state before borrowing"))
}

fn identifier(value: Option<&str>, label: &str) -> Result<String, String> {
    let id = value.and_then(|s| uuid::Uuid::parse_str(s).ok()).filter(|id| !id.is_nil())
        .ok_or_else(|| format!("Damaged descriptor: missing or invalid {label}; restore a confirmed working state before borrowing"))?;
    Ok(id.hyphenated().to_string())
}

fn validate_identity(object: Node<'_, '_>, kind: &str, name: &str) -> Result<(), String> {
    let mut identifiers = BTreeSet::from([identifier(object.attribute("uuid"), "object uuid")?]);
    let internal = required(object, MD, "InternalInfo")?;
    let mut categories = BTreeSet::new();
    let mut names = BTreeSet::new();
    for generated in internal
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "GeneratedType")
    {
        if !generated.has_tag_name((XR, "GeneratedType")) {
            return Err("Invalid GeneratedType namespace".into());
        }
        let generated_name = generated
            .attribute("name")
            .ok_or("GeneratedType has no name")?;
        let category = generated
            .attribute("category")
            .ok_or("GeneratedType has no category")?;
        if !names.insert(generated_name) || !categories.insert(category) {
            return Err("Duplicate GeneratedType identity".into());
        }
        for label in ["TypeId", "ValueId"] {
            let value = identifier(required(generated, XR, label)?.text(), label)?;
            if !identifiers.insert(value) {
                return Err("Reused UUID in borrowed object identity".into());
            }
        }
    }
    for (prefix, category) in
        cfe_borrow_generated_types(kind).ok_or("Unsupported generated type profile")?
    {
        if !names.contains(format!("{prefix}.{name}").as_str()) || !categories.contains(category) {
            return Err(format!("Damaged descriptor: missing GeneratedType {prefix}.{name}/{category}; restore a confirmed working state before borrowing"));
        }
        let generated = internal
            .children()
            .find(|n| {
                n.has_tag_name((XR, "GeneratedType"))
                    && n.attribute("name") == Some(format!("{prefix}.{name}").as_str())
            })
            .unwrap();
        if generated.attribute("category") != Some(*category) {
            return Err("GeneratedType category does not match its name".into());
        }
    }
    let this_node = optional(internal, XR, "ThisNode")?;
    if kind == "ExchangePlan" || this_node.is_some() {
        let value = identifier(this_node.and_then(|n| n.text()), "ThisNode")?;
        if !identifiers.insert(value) {
            return Err("ThisNode reuses another identity UUID".into());
        }
    }
    Ok(())
}

fn property_states(internal: Node<'_, '_>) -> Result<BTreeSet<String>, String> {
    let states = crate::property_states::read_property_states(internal)?;
    for (name, state) in &states {
        if !matches!(*state, "Extended" | "Notify" | "MultiState") {
            return Err(format!("Unsupported PropertyState for {name}"));
        }
    }
    Ok(states.keys().map(|name| (*name).to_string()).collect())
}

fn validate_properties(properties: Node<'_, '_>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for node in properties.children().filter(|n| n.is_element()) {
        if !seen.insert((node.tag_name().namespace(), node.tag_name().name())) {
            return Err(format!("Duplicate property {}", node.tag_name().name()));
        }
    }
    Ok(())
}

fn override_state(kind: &str, name: &str) -> Option<&'static str> {
    // The platform retained these combinations. In particular, Server/Extended
    // was discarded on export, whereas Server/Notify survived the round trip.
    match (kind, name) {
        ("Catalog", "DescriptionLength") => Some("Extended"),
        ("CommonModule", "Server") => Some("Notify"),
        _ => None,
    }
}

fn reserved(name: &str) -> bool {
    matches!(
        name,
        "Name" | "ObjectBelonging" | "ExtendedConfigurationObject"
    )
}

fn closing_start(text: &str, node: Node<'_, '_>) -> usize {
    let range = node.range();
    range.start
        + text[range.clone()]
            .rfind("</")
            .expect("nonempty validated element")
}

// Expanded names and values drive no-op detection; formatting and equivalent
// namespace prefixes do not trigger a rewrite of the user's descriptor.
pub fn equivalent(a: Node<'_, '_>, b: Node<'_, '_>) -> bool {
    if a.node_type() != b.node_type() {
        return false;
    }
    if a.is_element() {
        if a.tag_name() != b.tag_name() {
            return false;
        }
        let attrs = |n: Node<'_, '_>| {
            n.attributes()
                .map(|a| {
                    (
                        a.namespace().unwrap_or("").to_string(),
                        a.name().to_string(),
                        resolved_value(n, a.value()),
                    )
                })
                .collect::<BTreeSet<_>>()
        };
        if attrs(a) != attrs(b) {
            return false;
        }
        let significant = |n: &Node<'_, '_>| {
            n.is_element()
                || (n.is_text()
                    && (!n.text().unwrap_or("").trim().is_empty()
                        || !n
                            .parent()
                            .unwrap()
                            .children()
                            .any(|child| child.is_element())))
        };
        let aa = a.children().filter(significant).collect::<Vec<_>>();
        let bb = b.children().filter(significant).collect::<Vec<_>>();
        aa.len() == bb.len() && aa.into_iter().zip(bb).all(|(a, b)| equivalent(a, b))
    } else {
        resolved_value(a, a.text().unwrap_or_default())
            == resolved_value(b, b.text().unwrap_or_default())
    }
}

fn resolved_value(node: Node<'_, '_>, value: &str) -> (Option<String>, String) {
    let context = if node.is_element() {
        node
    } else {
        node.parent().unwrap_or(node)
    };
    if let Some((prefix, local)) = value.split_once(':') {
        if !prefix.is_empty()
            && !local.is_empty()
            && prefix
                .chars()
                .chain(local.chars())
                .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        {
            if let Some(namespace) = context.lookup_namespace_uri(Some(prefix)) {
                return (Some(namespace.to_string()), local.to_string());
            }
        }
    }
    (None, value.to_string())
}

// Preserve a copied subtree's meaning, making inherited namespace bindings
// explicit. Each element is serialized with its own in-scope bindings so a
// nested rebind and QName-valued content retain their original meaning.
pub fn standalone_fragment(node: Node<'_, '_>) -> String {
    if node.is_text() {
        return unica_xml_format::text::escape_xml(node.text().unwrap_or_default());
    }
    if !node.is_element() {
        return node.document().input_text()[node.range()].to_string();
    }
    let qname = |namespace: Option<&str>, name: &str| match namespace
        .and_then(|ns| node.lookup_prefix(ns))
    {
        Some(prefix) => format!("{prefix}:{name}"),
        None => name.to_string(),
    };
    let tag = qname(node.tag_name().namespace(), node.tag_name().name());
    let mut result = format!("<{tag}");
    for ns in node.namespaces() {
        match ns.name() {
            Some(prefix) => result.push_str(&format!(
                " xmlns:{prefix}=\"{}\"",
                unica_xml_format::text::escape_xml(ns.uri())
            )),
            None => result.push_str(&format!(
                " xmlns=\"{}\"",
                unica_xml_format::text::escape_xml(ns.uri())
            )),
        }
    }
    for attribute in node.attributes() {
        result.push_str(&format!(
            " {}=\"{}\"",
            qname(attribute.namespace(), attribute.name()),
            unica_xml_format::text::escape_xml(attribute.value())
        ));
    }
    if node.children().next().is_none() {
        result.push_str("/>");
        return result;
    }
    result.push('>');
    for child in node.children() {
        result.push_str(&standalone_fragment(child));
    }
    result.push_str(&format!("</{tag}>"));
    result
}

#[cfg(any(test, feature = "test-support"))]
#[path = "../tests/support/borrowed_object.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "../tests/platform/borrowed_object.rs"]
mod tests;

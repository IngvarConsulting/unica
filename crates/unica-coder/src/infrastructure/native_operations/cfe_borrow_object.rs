//! Pure descriptor planning for explicit top-level borrowing.
use super::cfe::{
    cfe_borrow_generated_types, cfe_borrow_object_xml, cfe_borrow_type_dir, CfeBorrowIdentity,
};
use super::common::escape_xml;
use roxmltree::{Document, Node};
use std::collections::BTreeSet;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

#[derive(Debug)]
pub(crate) struct BorrowObjectPlan {
    pub(crate) parent_uuid: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BorrowObjectErrorKind {
    BadValue,
    InvalidSource,
}

#[derive(Debug)]
pub(crate) struct BorrowObjectError {
    pub(crate) kind: BorrowObjectErrorKind,
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

pub(crate) fn validate_requested_overrides(
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

/// Uses the existing platform profile for the first descriptor. Refreshes are
/// surgical: extension-owned structure is never regenerated from the parent.
pub(crate) fn plan_borrow_object(
    parent: &[u8],
    existing: Option<&[u8]>,
    requested_overrides: Option<&[String]>,
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
        &CfeBorrowIdentity::default(),
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
            let states = overrides.iter().map(|name| format!("<xr:PropertyState xmlns:xr=\"{XR}\"><xr:Property>{}</xr:Property><xr:State>{}</xr:State></xr:PropertyState>", escape_xml(name), override_state(kind, name).expect("validated initial override"))).collect::<String>();
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
    let mut result = BTreeSet::new();
    for state in internal
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "PropertyState")
    {
        if !state.has_tag_name((XR, "PropertyState")) {
            return Err("Invalid PropertyState namespace".into());
        }
        let name = required(state, XR, "Property")?
            .text()
            .filter(|s| !s.is_empty())
            .ok_or("Empty PropertyState property")?;
        if !matches!(
            required(state, XR, "State")?.text(),
            Some("Extended" | "Notify" | "MultiState")
        ) {
            return Err(format!("Unsupported PropertyState for {name}"));
        }
        if !result.insert(name.into()) {
            return Err(format!("Duplicate PropertyState for {name}"));
        }
    }
    Ok(result)
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
fn equivalent(a: Node<'_, '_>, b: Node<'_, '_>) -> bool {
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
fn standalone_fragment(node: Node<'_, '_>) -> String {
    if node.is_text() {
        return escape_xml(node.text().unwrap_or_default());
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
            Some(prefix) => {
                result.push_str(&format!(" xmlns:{prefix}=\"{}\"", escape_xml(ns.uri())))
            }
            None => result.push_str(&format!(" xmlns=\"{}\"", escape_xml(ns.uri()))),
        }
    }
    for attribute in node.attributes() {
        result.push_str(&format!(
            " {}=\"{}\"",
            qname(attribute.namespace(), attribute.name()),
            escape_xml(attribute.value())
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

#[cfg(test)]
mod tests {
    use super::*;
    fn parent(kind: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" version="2.20"><{kind} uuid="00000000-0000-4000-8000-000000000001"><Properties><Name>Orders</Name><Comment>parent comment</Comment><Synonym>parent label</Synonym><DescriptionLength>10</DescriptionLength><Server>true</Server></Properties><ChildObjects/></{kind}></MetaDataObject>"#
        )
    }
    fn borrow(kind: &str, overrides: &[String]) -> Vec<u8> {
        plan_borrow_object(parent(kind).as_bytes(), None, Some(overrides))
            .unwrap()
            .bytes
    }
    #[test]
    fn comment_is_extension_owned_without_a_property_state() {
        let parent = parent("CommonModule");
        let first = plan_borrow_object(parent.as_bytes(), None, None).unwrap();
        let text = String::from_utf8(first.bytes).unwrap();
        let doc = Document::parse(&text).unwrap();
        assert_eq!(
            doc.descendants()
                .find(|n| n.has_tag_name((MD, "Comment")))
                .unwrap()
                .text(),
            None
        );
        let local = text.replace("<Comment/>", "<Comment>local comment</Comment>");
        let changed = parent
            .replace("parent comment", "changed parent comment")
            .replace("<Server>true</Server>", "<Server>false</Server>");
        let next = plan_borrow_object(changed.as_bytes(), Some(local.as_bytes()), None).unwrap();
        let next = String::from_utf8(next.bytes).unwrap();
        assert!(next.contains("<Comment>local comment</Comment>"));
        assert!(!next.contains("changed parent comment"));
    }

    #[test]
    fn refresh_updates_transferred_properties_preserving_local_xml_and_noop_bytes() {
        let old = borrow("CommonModule", &[]);
        let old = String::from_utf8(old)
            .unwrap()
            .replace("<Comment/>", "<Comment>local comment</Comment>")
            .replace(
                "</Properties>",
                "<Future xmlns=\"urn:local\" a=\"keep\"/></Properties>",
            );
        let changed = parent("CommonModule")
            .replace("<Server>true</Server>", "<Server>false</Server>")
            .replace("parent comment", "changed parent");
        let next = plan_borrow_object(changed.as_bytes(), Some(old.as_bytes()), None).unwrap();
        let next_text = String::from_utf8(next.bytes.clone()).unwrap();
        assert!(next_text.contains("local comment"));
        assert!(next_text.contains("<Future xmlns=\"urn:local\" a=\"keep\"/>"));
        let old_doc = Document::parse(&old).unwrap();
        let new_doc = Document::parse(&next_text).unwrap();
        assert_eq!(
            old_doc
                .root_element()
                .first_element_child()
                .unwrap()
                .attribute("uuid"),
            new_doc
                .root_element()
                .first_element_child()
                .unwrap()
                .attribute("uuid")
        );
        assert_eq!(
            new_doc
                .descendants()
                .find(|n| n.has_tag_name((MD, "Server")))
                .unwrap()
                .text(),
            Some("false")
        );
        assert_eq!(
            plan_borrow_object(changed.as_bytes(), Some(&next.bytes), None)
                .unwrap()
                .bytes,
            next.bytes
        );
    }

    #[test]
    fn catalog_refresh_keeps_platform_identity_children_and_overridden_property_state() {
        let parent = include_str!(
            "../../../../../tests/fixtures/platform_8_3_27/cfe_borrow/parent-catalog.xml"
        );
        let old = include_str!(
            "../../../../../tests/fixtures/platform_8_3_27/cfe_borrow/extension-catalog.xml"
        );
        let changed = parent.replace(
            "<DescriptionLength>10</DescriptionLength>",
            "<DescriptionLength>20</DescriptionLength>",
        );
        assert_ne!(parent, changed);
        let new = plan_borrow_object(
            changed.as_bytes(),
            Some(old.as_bytes()),
            Some(&["DescriptionLength".into()]),
        )
        .unwrap();
        assert_eq!(new.bytes, old.as_bytes());
        assert_eq!(new.parent_uuid, "1d6b8425-360c-4ab1-9bab-cc9a3b590bb2");
        let first =
            plan_borrow_object(parent.as_bytes(), None, Some(&["DescriptionLength".into()]))
                .unwrap();
        let text = std::str::from_utf8(&first.bytes).unwrap();
        let doc = Document::parse(text).unwrap();
        let obj = doc.root_element().first_element_child().unwrap();
        assert_ne!(obj.attribute("uuid"), Some(first.parent_uuid.as_str()));
        assert_eq!(
            doc.descendants()
                .find(|n| n.has_tag_name((XR, "Property")))
                .unwrap()
                .text(),
            Some("DescriptionLength")
        );
        assert_eq!(
            doc.descendants()
                .find(|n| n.has_tag_name((MD, "DescriptionLength")))
                .unwrap()
                .text(),
            Some("10")
        );
    }

    #[test]
    fn catalog_refresh_updates_platform_control_properties_and_keeps_override() {
        let parent = include_str!(
            "../../../../../tests/fixtures/platform_8_3_27/cfe_borrow/parent-catalog.xml"
        );
        let old = include_str!(
            "../../../../../tests/fixtures/platform_8_3_27/cfe_borrow/extension-catalog.xml"
        );
        let changed = parent
            .replace("<CodeLength>3</CodeLength>", "<CodeLength>5</CodeLength>")
            .replace(
                "<DescriptionLength>10</DescriptionLength>",
                "<DescriptionLength>20</DescriptionLength>",
            )
            .replace(
                "<Hierarchical>false</Hierarchical>",
                "<Hierarchical>true</Hierarchical>",
            );
        assert_ne!(parent, changed);
        let new = plan_borrow_object(changed.as_bytes(), Some(old.as_bytes()), None).unwrap();
        let text = std::str::from_utf8(&new.bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let doc = Document::parse(text).unwrap();
        let old_doc = Document::parse(old.trim_start_matches('\u{feff}')).unwrap();
        for (prop, value) in [
            ("CodeLength", "5"),
            ("Hierarchical", "true"),
            ("DescriptionLength", "50"),
        ] {
            assert_eq!(
                doc.descendants()
                    .find(|n| n.has_tag_name((MD, prop)))
                    .unwrap()
                    .text(),
                Some(value)
            );
        }
        for prop in ["InternalInfo", "ChildObjects", "Synonym", "Comment"] {
            let before = old_doc
                .descendants()
                .find(|n| n.has_tag_name((MD, prop)))
                .unwrap();
            let after = doc
                .descendants()
                .find(|n| n.has_tag_name((MD, prop)))
                .unwrap();
            assert_eq!(
                &old.trim_start_matches('\u{feff}')[before.range()],
                &text[after.range()]
            );
        }
        assert_eq!(
            plan_borrow_object(changed.as_bytes(), Some(&new.bytes), None)
                .unwrap()
                .bytes,
            new.bytes
        );
    }

    #[test]
    fn exchange_plan_identity_is_fresh_then_stable_after_untransferred_parent_changes() {
        let parent_uuid = "00000000-0000-4000-8000-000000000001";
        let parent_node = "00000000-0000-4000-8000-000000000002";
        let categories = ["Object", "Ref", "Selection", "List", "Manager"];
        let generated = categories.iter().enumerate().map(|(index, category)| {
            format!("<xr:GeneratedType name=\"ExchangePlan{category}.Orders\" category=\"{category}\"><xr:TypeId>00000000-0000-4000-8000-{:012x}</xr:TypeId><xr:ValueId>00000000-0000-4000-8000-{:012x}</xr:ValueId></xr:GeneratedType>", 100 + index * 2, 101 + index * 2)
        }).collect::<String>();
        let parent = format!("<MetaDataObject xmlns=\"{MD}\" xmlns:xr=\"{XR}\" version=\"2.20\"><ExchangePlan uuid=\"{parent_uuid}\"><InternalInfo><xr:ThisNode>{parent_node}</xr:ThisNode>{generated}</InternalInfo><Properties><Name>Orders</Name><Comment>before</Comment><CodeLength>9</CodeLength></Properties><ChildObjects/></ExchangePlan></MetaDataObject>");
        let identity = |text: &str| {
            let doc = Document::parse(text).unwrap();
            let object = doc.root_element().first_element_child().unwrap();
            let internal = object
                .children()
                .find(|n| n.has_tag_name((MD, "InternalInfo")))
                .unwrap();
            let node = internal
                .children()
                .find(|n| n.has_tag_name((XR, "ThisNode")))
                .unwrap()
                .text()
                .unwrap()
                .to_string();
            let pairs = internal
                .children()
                .filter(|n| n.has_tag_name((XR, "GeneratedType")))
                .map(|generated| {
                    let type_id = generated
                        .children()
                        .find(|n| n.has_tag_name((XR, "TypeId")))
                        .unwrap()
                        .text()
                        .unwrap()
                        .to_string();
                    let value_id = generated
                        .children()
                        .find(|n| n.has_tag_name((XR, "ValueId")))
                        .unwrap()
                        .text()
                        .unwrap()
                        .to_string();
                    (
                        generated.attribute("name").unwrap().to_string(),
                        generated.attribute("category").unwrap().to_string(),
                        type_id,
                        value_id,
                    )
                })
                .collect::<Vec<_>>();
            (
                object.attribute("uuid").unwrap().to_string(),
                node,
                pairs,
                text[internal.range()].to_string(),
            )
        };
        let parent_identity = identity(&parent);
        let mut parent_ids = BTreeSet::from([parent_identity.0.clone(), parent_identity.1.clone()]);
        for (_, _, type_id, value_id) in &parent_identity.2 {
            parent_ids.extend([type_id.clone(), value_id.clone()]);
        }
        let first = plan_borrow_object(parent.as_bytes(), None, None).unwrap();
        let first_identity = identity(std::str::from_utf8(&first.bytes).unwrap());
        assert_eq!(first_identity.2.len(), categories.len());
        assert_eq!(
            first_identity
                .2
                .iter()
                .map(|(_, category, _, _)| category.as_str())
                .collect::<BTreeSet<_>>(),
            categories.into_iter().collect::<BTreeSet<_>>()
        );
        let mut own_ids = BTreeSet::from([first_identity.0.clone(), first_identity.1.clone()]);
        for (name, category, type_id, value_id) in &first_identity.2 {
            assert!(categories.contains(&category.as_str()));
            assert_eq!(name, &format!("ExchangePlan{category}.Orders"));
            assert!(own_ids.insert(type_id.clone()));
            assert!(own_ids.insert(value_id.clone()));
        }
        assert_eq!(own_ids.len(), 12);
        assert!(parent_ids.is_disjoint(&own_ids));
        for value in &own_ids {
            assert!(!uuid::Uuid::parse_str(value).unwrap().is_nil());
        }
        // This profile does not transfer Comment or CodeLength. Their changes
        // exercise identity stability, while the Catalog test proves refresh of
        // transferred fields together with GeneratedType preservation.
        let changed = parent
            .replace("<Comment>before</Comment>", "<Comment>after</Comment>")
            .replace("<CodeLength>9</CodeLength>", "<CodeLength>12</CodeLength>");
        assert_ne!(changed, parent);
        let repeated = plan_borrow_object(changed.as_bytes(), Some(&first.bytes), None).unwrap();
        assert_eq!(
            identity(std::str::from_utf8(&repeated.bytes).unwrap()),
            first_identity
        );
        assert_eq!(repeated.bytes, first.bytes);
    }

    #[test]
    fn extension_identity_damage_is_refused_instead_of_repaired() {
        let original = String::from_utf8(borrow("ExchangePlan", &[])).unwrap();
        let doc = Document::parse(&original).unwrap();
        for tag in ["ThisNode", "TypeId", "ValueId"] {
            let node = doc
                .descendants()
                .find(|n| n.has_tag_name((XR, tag)))
                .unwrap();
            let mut damaged = original.clone();
            damaged.replace_range(node.range(), "");
            assert!(
                plan_borrow_object(
                    parent("ExchangePlan").as_bytes(),
                    Some(damaged.as_bytes()),
                    None
                )
                .is_err(),
                "{tag}"
            );
        }
        let wrapper = doc
            .root_element()
            .first_element_child()
            .unwrap()
            .attribute("uuid")
            .unwrap();
        for invalid in [
            "",
            "not-a-uuid",
            "00000000-0000-0000-0000-000000000000",
            "00000000-0000-4000-8000-000000000001",
        ] {
            let damaged = original.replace(wrapper, invalid);
            assert!(plan_borrow_object(
                parent("ExchangePlan").as_bytes(),
                Some(damaged.as_bytes()),
                None
            )
            .is_err());
        }
        assert_eq!(
            plan_borrow_object(
                parent("ExchangePlan").as_bytes(),
                Some(original.as_bytes()),
                None
            )
            .unwrap()
            .bytes,
            original.as_bytes()
        );
    }

    #[test]
    fn known_state_variants_and_module_states_do_not_erase_local_overrides() {
        let parent = include_str!(
            "../../../../../tests/fixtures/platform_8_3_27/cfe_borrow/parent-catalog.xml"
        );
        let original = include_str!(
            "../../../../../tests/fixtures/platform_8_3_27/cfe_borrow/extension-catalog.xml"
        );
        for state in ["Extended", "Notify", "MultiState"] {
            let existing = original.replace("<xr:State>Extended</xr:State>", &format!("<xr:State>{state}</xr:State>")).replace("</InternalInfo>", "<xr:PropertyState><xr:Property>ObjectModule</xr:Property><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo>");
            let next = plan_borrow_object(
                parent.as_bytes(),
                Some(existing.as_bytes()),
                Some(&["DescriptionLength".into()]),
            )
            .unwrap();
            assert_eq!(next.bytes, existing.as_bytes());
        }
        for local in ["Comment", "Synonym"] {
            assert!(
                plan_borrow_object(parent.as_bytes(), None, Some(&[local.into()]))
                    .unwrap_err()
                    .to_string()
                    .contains("extension-owned")
            );
        }
    }

    #[test]
    fn common_module_server_override_uses_platform_notify_and_survives_refresh() {
        let parent = include_str!(
            "../../../../../tests/fixtures/platform_8_3_27/cfe_borrow/parent-common-module.xml"
        )
        .replace("<Global>false</Global>", "<Global>true</Global>");
        let existing = include_str!(
            "../../../../../tests/fixtures/platform_8_3_27/cfe_borrow/extension-common-module.xml"
        );
        let next = plan_borrow_object(
            parent.as_bytes(),
            Some(existing.as_bytes()),
            Some(&["Server".into()]),
        )
        .unwrap();
        let next_text = std::str::from_utf8(&next.bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let next_doc = Document::parse(next_text).unwrap();
        assert_eq!(
            next_doc
                .descendants()
                .find(|n| n.has_tag_name((MD, "Server")))
                .unwrap()
                .text(),
            Some("false")
        );
        assert_eq!(
            next_doc
                .descendants()
                .find(|n| n.has_tag_name((MD, "Global")))
                .unwrap()
                .text(),
            Some("true")
        );
        assert_eq!(
            next_doc
                .descendants()
                .find(|n| n.has_tag_name((XR, "State")))
                .unwrap()
                .text(),
            Some("Notify")
        );
        let first = plan_borrow_object(parent.as_bytes(), None, Some(&["Server".into()])).unwrap();
        let first_text = std::str::from_utf8(&first.bytes).unwrap();
        let doc = Document::parse(first_text).unwrap();
        assert_eq!(
            doc.descendants()
                .find(|n| n.has_tag_name((XR, "State")))
                .unwrap()
                .text(),
            Some("Notify")
        );
        assert_eq!(
            doc.descendants()
                .find(|n| n.has_tag_name((MD, "Server")))
                .unwrap()
                .text(),
            Some("true")
        );
    }

    #[test]
    fn changed_qname_binding_refreshes_type_even_when_its_text_is_unchanged() {
        let parent = parent("DefinedType").replace("version=\"2.20\"", "xmlns:p=\"urn:parent:one\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\"").replace("</Properties>", "<Type><v8:Type>p:Example</v8:Type></Type></Properties>");
        let first = plan_borrow_object(parent.as_bytes(), None, None).unwrap();
        let first_text = std::str::from_utf8(&first.bytes).unwrap();
        let first_doc = Document::parse(first_text).unwrap();
        let first_type = first_doc
            .descendants()
            .find(|n| n.has_tag_name(("http://v8.1c.ru/8.1/data/core", "Type")))
            .unwrap();
        assert_eq!(
            first_type.lookup_namespace_uri(Some("p")),
            Some("urn:parent:one")
        );
        let changed = parent.replace("urn:parent:one", "urn:parent:two");
        let next = plan_borrow_object(changed.as_bytes(), Some(&first.bytes), None).unwrap();
        let text = std::str::from_utf8(&next.bytes).unwrap();
        let doc = Document::parse(text).unwrap();
        let ty = doc
            .descendants()
            .find(|n| n.has_tag_name(("http://v8.1c.ru/8.1/data/core", "Type")))
            .unwrap();
        assert_eq!(ty.text(), Some("p:Example"));
        assert_eq!(ty.lookup_namespace_uri(Some("p")), Some("urn:parent:two"));
        assert_eq!(
            plan_borrow_object(changed.as_bytes(), Some(&next.bytes), None)
                .unwrap()
                .bytes,
            next.bytes
        );
    }

    #[test]
    fn refuses_damaged_identity_own_object_changed_parent_and_changed_overrides() {
        let original = String::from_utf8(borrow("Catalog", &[])).unwrap();
        let doc = Document::parse(&original).unwrap();
        let value_id = doc
            .descendants()
            .find(|n| n.has_tag_name((XR, "ValueId")))
            .unwrap();
        let mut damaged = original.clone();
        damaged.replace_range(value_id.range(), "");
        for bad in [
            damaged,
            original.replace("Adopted", "Own"),
            original.replace(
                "00000000-0000-4000-8000-000000000001",
                "00000000-0000-4000-8000-000000000002",
            ),
        ] {
            assert!(
                plan_borrow_object(parent("Catalog").as_bytes(), Some(bad.as_bytes()), None)
                    .is_err()
            );
        }
        assert!(plan_borrow_object(
            parent("Catalog").as_bytes(),
            Some(original.as_bytes()),
            Some(&["DescriptionLength".into()])
        )
        .is_err());
    }
}

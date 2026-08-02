use super::model::{XDTO_NS, XSI_NS};
use crate::infrastructure::native_operations::text_snapshot::{
    resolve_line_ending, EolPolicy, LineEnding, SourceTextSnapshot,
};
use roxmltree::Node;
use serde::Serialize;
use serde_json::{Map, Value};
use std::ops::Range;

#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct TextEdit {
    pub(super) range: Range<usize>,
    pub(super) replacement: String,
}

#[allow(dead_code)]
#[derive(Debug)]
pub(super) struct WriterPlan {
    pub(super) after: String,
    pub(super) edits: Vec<TextEdit>,
    pub(super) finding: Option<WriterFinding>,
}

impl WriterPlan {
    pub(super) fn blocks(&self) -> bool {
        self.finding
            .as_ref()
            .is_some_and(|finding| finding.severity == WriterFindingSeverity::Error)
    }

    #[cfg(test)]
    fn duplicate_code(&self) -> Option<&'static str> {
        self.finding.as_ref().map(|finding| finding.code)
    }

    #[cfg(test)]
    fn conflict(&self) -> bool {
        self.blocks()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WriterFindingSeverity {
    Info,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WriterFindingState {
    PreExisting,
    Introduced,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct WriterSourceSpan {
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct WriterFindingLocation {
    pub(super) key: String,
    pub(super) span: WriterSourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct WriterFinding {
    pub(super) code: &'static str,
    pub(super) severity: WriterFindingSeverity,
    pub(super) state: WriterFindingState,
    pub(super) message: String,
    pub(super) location: WriterFindingLocation,
}

pub(super) fn plan(
    before: &str,
    args: &Map<String, Value>,
    operation: &str,
) -> Result<WriterPlan, String> {
    let document = super::parse(before)?;
    let root = document.root_element();
    let mut finding = None;
    let edit = match operation {
        "add-value-type" => {
            let name = super::required(args, "name")?;
            let base = super::required(args, "base")?;
            let existing = named_types(root, name);
            if !existing.is_empty() {
                let exact = existing.len() == 1
                    && compatible_value_type(existing[0], &semantic_qname(root, base));
                finding = Some(duplicate_finding(
                    "duplicate_type",
                    existing[0],
                    exact,
                    "type",
                ));
                None
            } else {
                let fragment = format!(
                    "<valueType name=\"{}\" base=\"{}\"/>",
                    escape_attribute(name),
                    escape_attribute(base)
                );
                Some(insert_top_level(before, root, TopLevelInsert::ValueType, &fragment)?)
            }
        }
        "add-object-type" => {
            let name = super::required(args, "name")?;
            let existing = named_types(root, name);
            if !existing.is_empty() {
                let exact = existing.len() == 1 && compatible_empty_object_type(existing[0]);
                finding = Some(duplicate_finding(
                    "duplicate_type",
                    existing[0],
                    exact,
                    "type",
                ));
                None
            } else {
                let fragment = format!("<objectType name=\"{}\"/>", escape_attribute(name));
                Some(insert_top_level(before, root, TopLevelInsert::ObjectType, &fragment)?)
            }
        }
        "add-property" => {
            let type_name = super::required(args, "typeName")?;
            let property = args
                .get("property")
                .and_then(Value::as_object)
                .ok_or_else(|| "property must be an object".to_string())?;
            let name = object_string(property, "name")?;
            let type_name_value = object_string(property, "type")?;
            let target = property_target(
                root,
                type_name,
                args.get("propertyPath").and_then(Value::as_str),
            )?;
            let desired_qname = desired_property_qname(root, target, type_name_value)?;
            let desired_lower = property
                .get("minOccurs")
                .and_then(Value::as_u64)
                .map_or_else(|| "1".to_string(), |value| value.to_string());
            let existing = direct_named_properties(target, name);
            if !existing.is_empty() {
                let exact = existing.len() == 1
                    && compatible_property(existing[0], &desired_qname.semantic, &desired_lower);
                finding = Some(duplicate_finding(
                    "duplicate_property",
                    existing[0],
                    exact,
                    "property",
                ));
                None
            } else {
                let lower = property
                    .get("minOccurs")
                    .and_then(Value::as_u64)
                    .map(|value| format!(" lowerBound=\"{value}\""))
                    .unwrap_or_default();
                let rendered_qname = render_property_qname(root, target, &desired_qname)?;
                let fragment = format!(
                    "<property{} name=\"{}\" type=\"{}\"{lower}/>",
                    rendered_qname.namespace_attribute,
                    escape_attribute(name),
                    escape_attribute(&rendered_qname.value)
                );
                Some(insert_child(before, target, &fragment)?)
            }
        }
        "remove-type" => {
            let name = super::required(args, "name")?;
            let matches = named_types(root, name);
            let node = exactly_one(matches, "type")?;
            Some(remove_node(before, node))
        }
        "remove-property" => {
            let target = property_target(
                root,
                super::required(args, "typeName")?,
                args.get("propertyPath").and_then(Value::as_str),
            )?;
            let name = super::required(args, "name")?;
            let matches = direct_named_properties(target, name);
            let node = exactly_one(matches, "property")?;
            Some(remove_node(before, node))
        }
        _ => {
            return Err("unsupported_node: supported operations are add-value-type, add-object-type, add-property, remove-type, remove-property".to_string())
        }
    };
    let edits = edit.into_iter().collect::<Vec<_>>();
    let after = apply_edits(before, &edits)?;
    Ok(WriterPlan {
        after,
        edits,
        finding,
    })
}

#[derive(Clone, Copy)]
enum TopLevelInsert {
    ValueType,
    ObjectType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SemanticQName {
    namespace: Option<String>,
    local: String,
    lexical_valid: bool,
}

struct DesiredPropertyQName {
    raw: String,
    semantic: SemanticQName,
    bare_local: bool,
}

struct RenderedPropertyQName {
    value: String,
    namespace_attribute: String,
}

fn desired_property_qname(
    root: Node<'_, '_>,
    target: Node<'_, '_>,
    raw: &str,
) -> Result<DesiredPropertyQName, String> {
    if raw.contains(':') {
        return Ok(DesiredPropertyQName {
            raw: raw.to_string(),
            semantic: semantic_qname(target, raw),
            bare_local: false,
        });
    }
    let target_namespace = root
        .attribute("targetNamespace")
        .filter(|namespace| !namespace.is_empty())
        .ok_or_else(|| {
            "unsupported_node: cannot resolve a bare property type without targetNamespace"
                .to_string()
        })?;
    Ok(DesiredPropertyQName {
        raw: raw.to_string(),
        semantic: SemanticQName {
            namespace: Some(target_namespace.to_string()),
            local: raw.to_string(),
            lexical_valid: !raw.is_empty(),
        },
        bare_local: true,
    })
}

fn render_property_qname(
    root: Node<'_, '_>,
    target: Node<'_, '_>,
    desired: &DesiredPropertyQName,
) -> Result<RenderedPropertyQName, String> {
    if !desired.bare_local {
        return Ok(RenderedPropertyQName {
            value: desired.raw.clone(),
            namespace_attribute: String::new(),
        });
    }
    let namespace = desired
        .semantic
        .namespace
        .as_deref()
        .expect("a bare local QName carries targetNamespace");
    let (prefix, in_scope) = existing_non_default_prefix(root, target, namespace).ok_or_else(|| {
        "unsupported_node: bare local property type requires an existing non-default prefix for targetNamespace"
            .to_string()
    })?;
    Ok(RenderedPropertyQName {
        value: format!("{prefix}:{}", desired.semantic.local),
        namespace_attribute: if in_scope {
            String::new()
        } else {
            format!(
                " xmlns:{}=\"{}\"",
                escape_attribute(&prefix),
                escape_attribute(namespace)
            )
        },
    })
}

fn existing_non_default_prefix(
    root: Node<'_, '_>,
    target: Node<'_, '_>,
    namespace: &str,
) -> Option<(String, bool)> {
    for node in target.ancestors().filter(Node::is_element) {
        for declaration in node.namespaces() {
            if declaration.uri() == namespace {
                if let Some(prefix) = declaration.name().filter(|prefix| !prefix.is_empty()) {
                    if target.lookup_namespace_uri(Some(prefix)) == Some(namespace) {
                        return Some((prefix.to_string(), true));
                    }
                }
            }
        }
    }
    for node in root.descendants().filter(Node::is_element) {
        for declaration in node.namespaces() {
            if declaration.uri() == namespace {
                if let Some(prefix) = declaration.name().filter(|prefix| !prefix.is_empty()) {
                    return Some((prefix.to_string(), false));
                }
            }
        }
    }
    None
}

fn semantic_qname(node: Node<'_, '_>, raw: &str) -> SemanticQName {
    let parts = raw.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [prefix, local] if !prefix.is_empty() && !local.is_empty() => SemanticQName {
            namespace: node.lookup_namespace_uri(Some(prefix)).map(str::to_string),
            local: (*local).to_string(),
            lexical_valid: true,
        },
        [local] if !local.is_empty() => SemanticQName {
            namespace: None,
            local: (*local).to_string(),
            lexical_valid: true,
        },
        _ => SemanticQName {
            namespace: None,
            local: raw.to_string(),
            lexical_valid: false,
        },
    }
}

fn compatible_value_type(node: Node<'_, '_>, base: &SemanticQName) -> bool {
    node.has_tag_name((XDTO_NS, "valueType"))
        && has_only_plain_attributes(node, &["name", "base"])
        && has_exact_compatible_content(node)
        && node
            .attribute("base")
            .is_some_and(|existing| semantic_qname(node, existing) == *base)
}

fn compatible_empty_object_type(node: Node<'_, '_>) -> bool {
    node.has_tag_name((XDTO_NS, "objectType"))
        && has_only_plain_attributes(node, &["name"])
        && has_exact_compatible_content(node)
}

fn compatible_property(node: Node<'_, '_>, type_ref: &SemanticQName, desired_lower: &str) -> bool {
    node.has_tag_name((XDTO_NS, "property"))
        && has_only_plain_attributes(node, &["name", "type", "lowerBound", "upperBound"])
        && has_exact_compatible_content(node)
        && node
            .attribute("type")
            .is_some_and(|existing| semantic_qname(node, existing) == *type_ref)
        && node.attribute("lowerBound").unwrap_or("1") == desired_lower
        && node.attribute("upperBound").unwrap_or("1") == "1"
}

fn has_only_plain_attributes(node: Node<'_, '_>, allowed: &[&str]) -> bool {
    node.attributes()
        .all(|attribute| attribute.namespace().is_none() && allowed.contains(&attribute.name()))
}

fn has_exact_compatible_content(node: Node<'_, '_>) -> bool {
    node.children().all(|child| {
        if child.is_element() {
            return false;
        }
        !child.is_text()
            || child.text().is_none_or(|text| {
                text.chars()
                    .all(|character| matches!(character, ' ' | '\t' | '\r' | '\n'))
            })
    })
}

fn duplicate_finding(
    code: &'static str,
    node: Node<'_, '_>,
    exact: bool,
    noun: &str,
) -> WriterFinding {
    let identity = node.attribute("name").unwrap_or("<unknown>");
    let range = node.range();
    WriterFinding {
        code,
        severity: if exact {
            WriterFindingSeverity::Info
        } else {
            WriterFindingSeverity::Error
        },
        state: if exact {
            WriterFindingState::PreExisting
        } else {
            WriterFindingState::Introduced
        },
        message: if exact {
            format!("the requested {noun} `{identity}` already exists with equivalent semantics")
        } else {
            format!("the requested {noun} `{identity}` conflicts with the existing identity")
        },
        location: WriterFindingLocation {
            key: format!("$operation/{noun}:{identity}/@unique"),
            span: WriterSourceSpan {
                start: range.start,
                end: range.end,
            },
        },
    }
}

fn insert_top_level(
    text: &str,
    root: Node<'_, '_>,
    kind: TopLevelInsert,
    fragment: &str,
) -> Result<TextEdit, String> {
    if matches!(kind, TopLevelInsert::ValueType) {
        if let Some(object) = root
            .children()
            .find(|node| node.has_tag_name((XDTO_NS, "objectType")))
        {
            return insert_line_before_node(text, object, fragment);
        }
    }
    let sibling = root.children().filter(Node::is_element).next_back();
    insert_before_explicit_close(text, root, sibling, fragment)
}

fn insert_line_before_node(
    text: &str,
    node: Node<'_, '_>,
    fragment: &str,
) -> Result<TextEdit, String> {
    let line_start = line_start(text, node.range().start);
    let indent = exact_line_indent(text, node.range().start)?;
    let eol = observed_eol(text, local_eol_before(text, line_start))?;
    Ok(TextEdit {
        range: line_start..line_start,
        replacement: format!("{indent}{fragment}{eol}"),
    })
}

fn insert_child(text: &str, target: Node<'_, '_>, fragment: &str) -> Result<TextEdit, String> {
    let range = target.range();
    let source = &text[range.clone()];
    if source.ends_with("/>") {
        let target_indent = exact_line_indent(text, range.start)?;
        let child_indent = infer_child_indent(text, target)?;
        let eol = observed_eol(
            text,
            local_eol_after(text, range.end)
                .or_else(|| local_eol_before(text, line_start(text, range.start))),
        )?;
        let lexical_name = lexical_element_name(source)?;
        return Ok(TextEdit {
            range: range.end - 2..range.end,
            replacement: format!(
                ">{eol}{child_indent}{fragment}{eol}{target_indent}</{lexical_name}>"
            ),
        });
    }
    let sibling = target.children().filter(Node::is_element).next_back();
    insert_before_explicit_close(text, target, sibling, fragment)
}

fn insert_before_explicit_close(
    text: &str,
    target: Node<'_, '_>,
    sibling: Option<Node<'_, '_>>,
    fragment: &str,
) -> Result<TextEdit, String> {
    let range = target.range();
    let close = text[range.clone()]
        .rfind("</")
        .map(|offset| range.start + offset)
        .ok_or_else(|| {
            "unsupported_node: insertion target has no explicit closing tag".to_string()
        })?;
    let child_indent = match sibling {
        Some(sibling) => exact_line_indent(text, sibling.range().start)?.to_string(),
        None => infer_child_indent(text, target)?,
    };
    let close_line_start = line_start(text, close);
    if text[close_line_start..close]
        .chars()
        .all(|character| matches!(character, ' ' | '\t'))
    {
        let eol = observed_eol(text, local_eol_before(text, close_line_start))?;
        return Ok(TextEdit {
            range: close_line_start..close_line_start,
            replacement: format!("{child_indent}{fragment}{eol}"),
        });
    }

    let target_indent = exact_line_indent(text, range.start)?;
    let eol = observed_eol(
        text,
        local_eol_after(text, range.end)
            .or_else(|| local_eol_before(text, line_start(text, range.start))),
    )?;
    Ok(TextEdit {
        range: close..close,
        replacement: format!("{eol}{child_indent}{fragment}{eol}{target_indent}"),
    })
}

fn infer_child_indent(text: &str, target: Node<'_, '_>) -> Result<String, String> {
    if let Some(child) = target.children().find(Node::is_element) {
        return exact_line_indent(text, child.range().start).map(str::to_string);
    }
    let target_indent = exact_line_indent(text, target.range().start)?;
    let parent = target.parent_element().ok_or_else(unsupported_indent)?;
    let parent_indent = exact_line_indent(text, parent.range().start)?;
    let unit = target_indent
        .strip_prefix(parent_indent)
        .filter(|unit| !unit.is_empty())
        .ok_or_else(unsupported_indent)?;
    Ok(format!("{target_indent}{unit}"))
}

fn unsupported_indent() -> String {
    "unsupported_node: cannot prove the local indentation profile".to_string()
}

fn observed_eol(text: &str, local: Option<LineEnding>) -> Result<&'static str, String> {
    let snapshot = SourceTextSnapshot::from_bytes(text.as_bytes())
        .map_err(|error| format!("unsupported_node: {error}"))?;
    let ending = resolve_line_ending(EolPolicy::Preserve, &snapshot, local)
        .map_err(|error| format!("unsupported_node: {error}"))?;
    match ending {
        LineEnding::Lf | LineEnding::CrLf => Ok(ending.as_str()),
        LineEnding::Cr => {
            Err("unsupported_node: standalone CR line endings are unsupported".into())
        }
    }
}

fn local_eol_before(text: &str, offset: usize) -> Option<LineEnding> {
    let prefix = text.as_bytes().get(..offset)?;
    if prefix.ends_with(b"\r\n") {
        Some(LineEnding::CrLf)
    } else if prefix.ends_with(b"\n") {
        Some(LineEnding::Lf)
    } else if prefix.ends_with(b"\r") {
        Some(LineEnding::Cr)
    } else {
        None
    }
}

fn local_eol_after(text: &str, offset: usize) -> Option<LineEnding> {
    let suffix = text.as_bytes().get(offset..)?;
    if suffix.starts_with(b"\r\n") {
        Some(LineEnding::CrLf)
    } else if suffix.starts_with(b"\n") {
        Some(LineEnding::Lf)
    } else if suffix.starts_with(b"\r") {
        Some(LineEnding::Cr)
    } else {
        None
    }
}

fn line_start(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset]
        .iter()
        .rposition(|byte| matches!(byte, b'\n' | b'\r'))
        .map_or(0, |index| index + 1)
}

fn exact_line_indent(text: &str, offset: usize) -> Result<&str, String> {
    let start = line_start(text, offset);
    let indent = &text[start..offset];
    indent
        .chars()
        .all(|character| matches!(character, ' ' | '\t'))
        .then_some(indent)
        .ok_or_else(unsupported_indent)
}

fn lexical_element_name(source: &str) -> Result<&str, String> {
    let name = source
        .strip_prefix('<')
        .and_then(|source| {
            let end = source
                .find(|character: char| {
                    character.is_ascii_whitespace() || matches!(character, '/' | '>')
                })
                .unwrap_or(source.len());
            source.get(..end)
        })
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "unsupported_node: cannot preserve the element QName".to_string())?;
    Ok(name)
}

fn remove_node(text: &str, node: Node<'_, '_>) -> TextEdit {
    let range = node.range();
    let start = line_start(text, range.start);
    let line_prefix_is_whitespace = text[start..range.start]
        .chars()
        .all(|character| matches!(character, ' ' | '\t'));
    let mut trailing = range.end;
    while text
        .as_bytes()
        .get(trailing)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        trailing += 1;
    }
    let (end, owns_line) = if text
        .as_bytes()
        .get(trailing..)
        .is_some_and(|tail| tail.starts_with(b"\r\n"))
    {
        (trailing + 2, true)
    } else if text
        .as_bytes()
        .get(trailing)
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        (trailing + 1, true)
    } else {
        (range.end, false)
    };
    TextEdit {
        range: if line_prefix_is_whitespace && owns_line {
            start..end
        } else {
            range
        },
        replacement: String::new(),
    }
}

fn property_target<'a>(
    root: Node<'a, 'a>,
    type_name: &str,
    property_path: Option<&str>,
) -> Result<Node<'a, 'a>, String> {
    let mut matching = root.children().filter(|node| {
        node.has_tag_name((XDTO_NS, "objectType")) && node.attribute("name") == Some(type_name)
    });
    let mut target = matching.next().ok_or_else(|| {
        if !named_types(root, type_name).is_empty() {
            "unsupported_node: properties can be added only to objectType".to_string()
        } else {
            "target_not_found: type does not exist".to_string()
        }
    })?;
    if matching.next().is_some() {
        return Err("unsupported_node: objectType identity is ambiguous".to_string());
    }
    let segments = strict_property_path(property_path)?;
    for segment in segments {
        let properties = direct_named_properties(target, segment);
        let property = exactly_one(properties, "property path segment")?;
        let type_defs = property
            .children()
            .filter(|child| child.has_tag_name((XDTO_NS, "typeDef")))
            .collect::<Vec<_>>();
        target = match type_defs.as_slice() {
            [] => {
                return Err(format!(
                    "target_not_found: property path segment `{segment}` has no nested typeDef:ObjectType"
                ))
            }
            [type_def] if type_def.attribute((XSI_NS, "type")) == Some("ObjectType") => *type_def,
            _ => {
                return Err(format!(
                    "unsupported_node: property path segment `{segment}` must own exactly one typeDef with xsi:type=\"ObjectType\""
                ))
            }
        };
    }
    Ok(target)
}

fn strict_property_path(property_path: Option<&str>) -> Result<Vec<&str>, String> {
    let Some(path) = property_path else {
        return Ok(Vec::new());
    };
    let segments = path.split('.').collect::<Vec<_>>();
    if segments.is_empty()
        || segments.iter().any(|segment| {
            segment.is_empty()
                || segment.trim().is_empty()
                || segment.chars().any(char::is_whitespace)
        })
    {
        return Err(
            "property_path_invalid: propertyPath must contain non-empty property names".to_string(),
        );
    }
    Ok(segments)
}

fn named_types<'a>(root: Node<'a, 'a>, name: &str) -> Vec<Node<'a, 'a>> {
    root.children()
        .filter(|node| {
            (node.has_tag_name((XDTO_NS, "valueType"))
                || node.has_tag_name((XDTO_NS, "objectType")))
                && node.attribute("name") == Some(name)
        })
        .collect()
}

fn direct_named_properties<'a>(target: Node<'a, 'a>, name: &str) -> Vec<Node<'a, 'a>> {
    target
        .children()
        .filter(|node| {
            node.has_tag_name((XDTO_NS, "property")) && node.attribute("name") == Some(name)
        })
        .collect()
}

fn exactly_one<'a>(nodes: Vec<Node<'a, 'a>>, noun: &str) -> Result<Node<'a, 'a>, String> {
    match nodes.as_slice() {
        [] => Err(format!("target_not_found: {noun} does not exist")),
        [node] => Ok(*node),
        _ => Err(format!("unsupported_node: {noun} identity is ambiguous")),
    }
}

fn apply_edits(before: &str, edits: &[TextEdit]) -> Result<String, String> {
    match edits {
        [] => Ok(before.to_string()),
        [edit]
            if edit.range.start <= edit.range.end
                && edit.range.end <= before.len()
                && before.is_char_boundary(edit.range.start)
                && before.is_char_boundary(edit.range.end) =>
        {
            Ok(format!(
                "{}{}{}",
                &before[..edit.range.start],
                edit.replacement,
                &before[edit.range.end..]
            ))
        }
        [_] => Err("unsupported_node: writer produced an invalid text edit range".to_string()),
        _ => Err("unsupported_node: writer produced more than one text edit".to_string()),
    }
}

fn object_string<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    object
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("property.{name} must be a non-empty string"))
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::plan;
    use serde_json::{json, Map, Value};

    const ROOT: &str = r#"xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test""#;

    fn package(body: &str) -> String {
        format!("<package {ROOT}>\n{body}</package>")
    }

    fn args(entries: &[(&str, Value)]) -> Map<String, Value> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn assert_edit_applies(before: &str, plan: &super::WriterPlan) {
        match plan.edits.as_slice() {
            [] => assert_eq!(plan.after, before),
            [edit] => assert_eq!(
                plan.after,
                format!(
                    "{}{}{}",
                    &before[..edit.range.start],
                    edit.replacement,
                    &before[edit.range.end..]
                )
            ),
            edits => panic!(
                "writer returned {} edits instead of zero or one",
                edits.len()
            ),
        }
    }

    #[test]
    fn xdto_writer_places_value_type_before_the_first_object_type() {
        let before = package("\t<objectType name=\"Existing\"></objectType>\n");
        let plan = plan(
            &before,
            &args(&[("name", json!("Added")), ("base", json!("xs:string"))]),
            "add-value-type",
        )
        .unwrap();

        let value = plan.after.find("<valueType name=\"Added\"").unwrap();
        let object = plan.after.find("<objectType name=\"Existing\"").unwrap();
        assert!(value < object, "valueType was emitted after objectType");
    }

    #[test]
    fn xdto_writer_expands_a_self_closing_object_for_its_first_property() {
        let before = package("\t<objectType name=\"Target\"/>\n");
        let plan = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                (
                    "property",
                    json!({"name":"Added", "type":"xs:string", "minOccurs":0}),
                ),
            ]),
            "add-property",
        )
        .expect("a supported self-closing objectType must be expanded locally");

        assert!(plan.after.contains(
            "\t<objectType name=\"Target\">\n\t\t<property name=\"Added\" type=\"xs:string\" lowerBound=\"0\"/>\n\t</objectType>"
        ));
    }

    #[test]
    fn xdto_writer_rejects_every_invalid_property_path_shape() {
        let before = package("\t<objectType name=\"Target\"></objectType>\n");
        for property_path in [".", ".Foo", "Foo.", "Foo..Bar", "Foo. .Bar"] {
            let error = plan(
                &before,
                &args(&[
                    ("typeName", json!("Target")),
                    ("propertyPath", json!(property_path)),
                    ("property", json!({"name":"Added", "type":"xs:string"})),
                ]),
                "add-property",
            )
            .unwrap_err();

            assert!(
                error.contains("property_path_invalid"),
                "{property_path:?}: {error}"
            );
        }
    }

    #[test]
    fn xdto_writer_distinguishes_exact_and_conflicting_type_duplicates() {
        let before = package("\t<valueType name=\"Existing\" base=\"xs:string\"/>\n");
        let exact = plan(
            &before,
            &args(&[("name", json!("Existing")), ("base", json!("xs:string"))]),
            "add-value-type",
        )
        .unwrap();
        assert_eq!(exact.after, before);
        assert_eq!(exact.duplicate_code(), Some("duplicate_type"));
        assert!(!exact.conflict());

        let conflict = plan(
            &before,
            &args(&[("name", json!("Existing")), ("base", json!("xs:integer"))]),
            "add-value-type",
        )
        .unwrap();
        assert_eq!(conflict.after, before);
        assert_eq!(conflict.duplicate_code(), Some("duplicate_type"));
        assert!(conflict.conflict());
    }

    #[test]
    fn xdto_writer_returns_one_local_patch_and_preserves_outside_bytes() {
        let before = package(
            "\t<valueType name=\"Existing\" base=\"xs:string\"/>\n\t<objectType name=\"Object\"/>\n",
        );
        let object_start = before.find("\t<objectType").unwrap();
        let plan = plan(
            &before,
            &args(&[("name", json!("Added")), ("base", json!("xs:string"))]),
            "add-value-type",
        )
        .unwrap();

        assert_eq!(plan.edits.len(), 1);
        let edit = &plan.edits[0];
        assert_eq!(edit.range, object_start..object_start);
        assert_eq!(&plan.after[..edit.range.start], &before[..edit.range.start]);
        assert_eq!(
            &plan.after[edit.range.start + edit.replacement.len()..],
            &before[edit.range.end..]
        );
        assert_edit_applies(&before, &plan);
    }

    #[test]
    fn xdto_writer_compares_property_qname_effective_bounds_and_structure() {
        let before = package(
            "\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<objectType name=\"Target\">\n\t\t<property name=\"Existing\" type=\"tns:Local\" lowerBound=\"1\" upperBound=\"1\"/>\n\t\t<property name=\"Structured\"><typeDef xsi:type=\"ObjectType\"/></property>\n\t</objectType>\n",
        );
        let exact = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                (
                    "property",
                    json!({"name":"Existing", "type":"Local", "minOccurs":1}),
                ),
            ]),
            "add-property",
        )
        .unwrap();
        assert_eq!(exact.after, before);
        assert_eq!(exact.duplicate_code(), Some("duplicate_property"));
        assert!(!exact.conflict());

        for property in [
            json!({"name":"Existing", "type":"xs:string", "minOccurs":1}),
            json!({"name":"Existing", "type":"Local", "minOccurs":0}),
            json!({"name":"Structured", "type":"Local", "minOccurs":1}),
        ] {
            let conflict = plan(
                &before,
                &args(&[("typeName", json!("Target")), ("property", property)]),
                "add-property",
            )
            .unwrap();
            assert_eq!(conflict.after, before);
            assert_eq!(conflict.duplicate_code(), Some("duplicate_property"));
            assert!(conflict.conflict());
        }
    }

    #[test]
    fn xdto_writer_distinguishes_empty_object_duplicate_from_kind_and_structure_conflicts() {
        let empty = package("\t<objectType name=\"Existing\"></objectType>\n");
        let exact = plan(
            &empty,
            &args(&[("name", json!("Existing"))]),
            "add-object-type",
        )
        .unwrap();
        assert_eq!(exact.duplicate_code(), Some("duplicate_type"));
        assert!(!exact.conflict());

        for before in [
            package("\t<valueType name=\"Existing\" base=\"xs:string\"/>\n"),
            package(
                "\t<objectType name=\"Existing\">\n\t\t<property name=\"P\" type=\"xs:string\"/>\n\t</objectType>\n",
            ),
        ] {
            let conflict = plan(
                &before,
                &args(&[("name", json!("Existing"))]),
                "add-object-type",
            )
            .unwrap();
            assert_eq!(conflict.after, before);
            assert_eq!(conflict.duplicate_code(), Some("duplicate_type"));
            assert!(conflict.conflict());
        }
    }

    #[test]
    fn xdto_writer_treats_significant_character_data_as_duplicate_structure_conflict() {
        let cases = [
            (
                "valueType text",
                package("\t<valueType name=\"Existing\" base=\"xs:string\">text</valueType>\n"),
                "add-value-type",
                args(&[
                    ("name", json!("Existing")),
                    ("base", json!("xs:string")),
                ]),
                "duplicate_type",
            ),
            (
                "objectType CDATA",
                package("\t<objectType name=\"Existing\"><![CDATA[text]]></objectType>\n"),
                "add-object-type",
                args(&[("name", json!("Existing"))]),
                "duplicate_type",
            ),
            (
                "property text",
                package(
                    "\t<objectType name=\"Target\">\n\t\t<property name=\"Existing\" type=\"xs:string\">text</property>\n\t</objectType>\n",
                ),
                "add-property",
                args(&[
                    ("typeName", json!("Target")),
                    (
                        "property",
                        json!({"name":"Existing", "type":"xs:string"}),
                    ),
                ]),
                "duplicate_property",
            ),
        ];

        for (label, before, operation, arguments, code) in cases {
            let conflict = plan(&before, &arguments, operation).unwrap();
            assert_eq!(conflict.after, before, "{label}");
            assert_eq!(conflict.duplicate_code(), Some(code), "{label}");
            assert!(conflict.conflict(), "{label}");
        }
    }

    #[test]
    fn xdto_writer_rejects_value_type_as_a_property_container() {
        let before = package("\t<valueType name=\"Target\" base=\"xs:string\"></valueType>\n");
        let error = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"xs:string"})),
            ]),
            "add-property",
        )
        .unwrap_err();

        assert!(error.contains("unsupported_node"), "{error}");
    }

    #[test]
    fn xdto_writer_rejects_a_nested_type_def_without_exact_object_discriminator() {
        let before = package(
            "\t<objectType name=\"Target\">\n\t\t<property name=\"Nested\"><typeDef xsi:type=\"ValueType\"/></property>\n\t</objectType>\n",
        );
        let error = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("propertyPath", json!("Nested")),
                ("property", json!({"name":"Added", "type":"xs:string"})),
            ]),
            "add-property",
        )
        .unwrap_err();

        assert!(error.contains("unsupported_node"), "{error}");
    }

    #[test]
    fn xdto_writer_inserts_into_explicit_and_self_closing_nested_object_type_defs() {
        for (label, type_def, expected) in [
            (
                "explicit",
                "<typeDef xsi:type=\"ObjectType\">\n\t\t\t</typeDef>",
                "<typeDef xsi:type=\"ObjectType\">\n\t\t\t\t<property name=\"Added\" type=\"xs:string\"/>\n\t\t\t</typeDef>",
            ),
            (
                "self-closing",
                "<typeDef xsi:type=\"ObjectType\"/>",
                "<typeDef xsi:type=\"ObjectType\">\n\t\t\t\t<property name=\"Added\" type=\"xs:string\"/>\n\t\t\t</typeDef>",
            ),
        ] {
            let before = package(&format!(
                "\t<objectType name=\"Target\">\n\t\t<property name=\"Nested\">\n\t\t\t{type_def}\n\t\t</property>\n\t</objectType>\n"
            ));
            let plan = plan(
                &before,
                &args(&[
                    ("typeName", json!("Target")),
                    ("propertyPath", json!("Nested")),
                    (
                        "property",
                        json!({"name":"Added", "type":"xs:string"}),
                    ),
                ]),
                "add-property",
            )
            .unwrap_or_else(|error| panic!("{label}: {error}"));
            assert!(plan.after.contains(expected), "{label}: {}", plan.after);
            assert_edit_applies(&before, &plan);
        }
    }

    #[test]
    fn xdto_writer_preserves_crlf_and_uses_local_eol_in_a_mixed_document() {
        let crlf = package("\t<objectType name=\"Target\"/>\n").replace('\n', "\r\n");
        let crlf_plan = plan(
            &crlf,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"xs:string"})),
            ]),
            "add-property",
        )
        .unwrap();
        assert!(!crlf_plan.after.replace("\r\n", "").contains('\n'));

        let mixed = format!(
            "<package {ROOT}>\n\t<objectType name=\"Other\"/>\n\t<objectType name=\"Target\"/>\r\n</package>"
        );
        let mixed_plan = plan(
            &mixed,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"xs:string"})),
            ]),
            "add-property",
        )
        .unwrap();
        assert!(mixed_plan.after.contains(
            "<objectType name=\"Target\">\r\n\t\t<property name=\"Added\" type=\"xs:string\"/>\r\n\t</objectType>"
        ));
    }

    #[test]
    fn xdto_writer_fails_closed_when_mixed_eol_has_no_local_context() {
        let before = format!(
            "<?xml version=\"1.0\"?>\n<!-- profile -->\r\n<package {ROOT}><objectType name=\"Target\"></objectType></package>"
        );
        let error = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"xs:string"})),
            ]),
            "add-property",
        )
        .unwrap_err();

        assert!(error.contains("unsupported_node"), "{error}");
    }

    #[test]
    fn xdto_writer_remove_keeps_neighboring_blank_lines_byte_identical() {
        let before = package(
            "\t<valueType name=\"Keep\" base=\"xs:string\"/>\n\n\t<valueType name=\"Remove\" base=\"xs:string\"/>\n\n\t<objectType name=\"Object\"/>\n",
        );
        let expected = package(
            "\t<valueType name=\"Keep\" base=\"xs:string\"/>\n\n\n\t<objectType name=\"Object\"/>\n",
        );
        let plan = plan(&before, &args(&[("name", json!("Remove"))]), "remove-type").unwrap();

        assert_eq!(plan.after, expected);
        assert_eq!(plan.edits.len(), 1);
        assert_edit_applies(&before, &plan);
    }

    #[test]
    fn xdto_writer_reuses_an_arbitrary_self_namespace_prefix_for_a_bare_local_type() {
        let root = ROOT.replace("xmlns:tns=\"urn:test\"", "xmlns:self=\"urn:test\"");
        let before = format!(
            "<package {root}>\n\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<objectType name=\"Target\"/>\n</package>"
        );
        let plan = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"Local"})),
            ]),
            "add-property",
        )
        .unwrap();

        assert!(plan.after.contains("name=\"Added\" type=\"self:Local\""));
        assert!(!plan.after.contains("type=\"tns:Local\""));
    }

    #[test]
    fn xdto_writer_fails_closed_for_bare_local_type_without_a_non_default_self_prefix() {
        let root = ROOT.replace(" xmlns:tns=\"urn:test\"", "");
        let before = format!(
            "<package {root}>\n\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<objectType name=\"Target\"/>\n</package>"
        );
        let error = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"Local"})),
            ]),
            "add-property",
        )
        .unwrap_err();

        assert!(error.contains("unsupported_node"), "{error}");
    }

    #[test]
    fn xdto_writer_real_fixture_supports_the_full_add_flow_and_repeat_no_ops() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"
        ));
        let mut text = super::super::decode(bytes).unwrap();
        let operations = [
            (
                "add-value-type",
                args(&[
                    ("name", json!("ДобавленныйТип")),
                    ("base", json!("xs:string")),
                ]),
                "duplicate_type",
            ),
            (
                "add-object-type",
                args(&[("name", json!("ДобавленныйОбъект"))]),
                "duplicate_type",
            ),
            (
                "add-property",
                args(&[
                    ("typeName", json!("ЛюбаяСсылка")),
                    ("propertyPath", json!("СсылкаНаОбъект")),
                    (
                        "property",
                        json!({"name":"ДобавленноеВложенное", "type":"xs:string", "minOccurs":0}),
                    ),
                ]),
                "duplicate_property",
            ),
            (
                "add-property",
                args(&[
                    ("typeName", json!("СоставнойЛюбойОбъект")),
                    (
                        "property",
                        json!({"name":"ДобавленноеПлоское", "type":"xs:string"}),
                    ),
                ]),
                "duplicate_property",
            ),
        ];

        for (operation, arguments, _) in &operations {
            let planned = plan(&text, arguments, operation).unwrap();
            assert_edit_applies(&text, &planned);
            assert_eq!(planned.edits.len(), 1, "{operation}");
            text = planned.after;
            let model = super::super::model::PackageModel::parse(&text).unwrap();
            assert!(
                super::super::validation::validate(&model).is_empty(),
                "{operation}"
            );
        }
        let value_position = text.find("<valueType name=\"ДобавленныйТип\"").unwrap();
        let first_object = text.find("<objectType").unwrap();
        assert!(value_position < first_object);

        for (operation, arguments, duplicate_code) in &operations {
            let repeated = plan(&text, arguments, operation).unwrap();
            assert_eq!(repeated.after, text, "{operation}");
            assert!(repeated.edits.is_empty(), "{operation}");
            assert_eq!(
                repeated.duplicate_code(),
                Some(*duplicate_code),
                "{operation}"
            );
            assert!(!repeated.conflict(), "{operation}");
        }
    }
}

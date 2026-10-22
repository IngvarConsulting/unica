use super::model::{XDTO_NS, XSI_NS};
use roxmltree::Node;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::ops::Range;
use unica_xdto::facets::ValueFacets;
use unica_xdto::operations::{XdtoPropertyDefinition, XdtoPropertySpec};
use unica_xml_format::text::{resolve_line_ending, EolPolicy, LineEnding, SourceTextSnapshot};

#[allow(dead_code)]
#[derive(Debug)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub replacement: String,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct WriterPlan {
    pub after: String,
    pub edits: Vec<TextEdit>,
    pub finding: Option<WriterFinding>,
}

impl WriterPlan {
    pub fn blocks(&self) -> bool {
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
pub enum WriterFindingSeverity {
    Info,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterFindingState {
    PreExisting,
    Introduced,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WriterSourceSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WriterFindingLocation {
    pub key: String,
    pub span: WriterSourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WriterFinding {
    pub code: &'static str,
    pub severity: WriterFindingSeverity,
    pub state: WriterFindingState,
    pub message: String,
    pub location: WriterFindingLocation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterErrorCause {
    BadValue,
    NotFound,
    AmbiguousTarget,
    InvalidSource,
    Postcondition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterErrorField {
    Source,
    TypeTarget,
    PropertyPath,
    PropertyName,
    Base,
    Before,
    After,
}

#[derive(Debug)]
pub struct WriterError {
    cause: WriterErrorCause,
    field: WriterErrorField,
    legacy_message: String,
}

impl WriterError {
    fn new(
        cause: WriterErrorCause,
        field: WriterErrorField,
        legacy_message: impl Into<String>,
    ) -> Self {
        Self {
            cause,
            field,
            legacy_message: legacy_message.into(),
        }
    }

    pub const fn cause(&self) -> WriterErrorCause {
        self.cause
    }

    pub const fn field(&self) -> WriterErrorField {
        self.field
    }
}

impl std::fmt::Display for WriterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.legacy_message.fmt(formatter)
    }
}

impl std::error::Error for WriterError {}

#[derive(Clone, Copy, Debug)]
pub enum TypedWriterOperation<'a> {
    AddValueType {
        name: &'a str,
        base: &'a str,
        facets: Option<&'a ValueFacets>,
    },
    AddObjectType {
        name: &'a str,
        base: Option<&'a str>,
        before: Option<&'a str>,
        after: Option<&'a str>,
    },
    AddProperty {
        type_name: &'a str,
        name: &'a str,
        type_ref: &'a str,
        min_occurs: Option<u64>,
        property_path: Option<&'a str>,
    },
    AddPropertyTree {
        type_name: &'a str,
        property: &'a XdtoPropertySpec,
        property_path: Option<&'a str>,
    },
    RemoveType {
        name: &'a str,
    },
    RemoveProperty {
        type_name: &'a str,
        name: &'a str,
        property_path: Option<&'a str>,
    },
}

pub fn plan_typed(
    before: &str,
    operation: TypedWriterOperation<'_>,
) -> Result<WriterPlan, WriterError> {
    let document = super::parse(before).map_err(|error| {
        WriterError::new(
            WriterErrorCause::InvalidSource,
            WriterErrorField::Source,
            error,
        )
    })?;
    let root = document.root_element();
    let mut finding = None;
    let edit = match operation {
        TypedWriterOperation::AddValueType { name, base, facets } => {
            let desired_base = desired_qname(root, root, base);
            let empty_facets = ValueFacets::default();
            let facets = facets.unwrap_or(&empty_facets);
            facets
                .validate_base(
                    desired_base.semantic.namespace.as_deref(),
                    &desired_base.semantic.local,
                )
                .map_err(|message| writer_invalid_source(message.to_owned()))?;
            let existing = named_types(root, name);
            if !existing.is_empty() {
                let exact = existing.len() == 1
                    && compatible_value_type(existing[0], &desired_base.semantic, facets);
                finding = Some(duplicate_finding(
                    "duplicate_type",
                    existing[0],
                    exact,
                    "type",
                ));
                None
            } else {
                let element_name = lexical_xdto_child_name(before, root, "valueType")
                    .map_err(writer_invalid_source)?;
                let fragment = format!(
                    "<{element_name}{} name=\"{}\" base=\"{}\"{}/>",
                    desired_base.namespace_declaration(),
                    escape_attribute(name),
                    escape_attribute(&desired_base.raw),
                    facet_attributes(facets)
                );
                Some(
                    insert_top_level(before, root, TopLevelInsert::ValueType, &fragment)
                        .map_err(writer_invalid_source)?,
                )
            }
        }
        TypedWriterOperation::AddObjectType {
            name,
            base,
            before: preceding,
            after: following,
        } => {
            let desired_base = base.map(|value| desired_qname(root, root, value));
            if let Some(base) = &desired_base {
                prove_local_object_base(root, name, &base.semantic)?;
            }
            let anchor = object_position_anchor(root, name, preceding, following)?;
            let existing = named_types(root, name);
            if !existing.is_empty() {
                let exact = existing.len() == 1
                    && compatible_object_type(
                        existing[0],
                        desired_base.as_ref().map(|base| &base.semantic),
                    )
                    && object_position_matches(root, existing[0], anchor);
                finding = Some(duplicate_finding(
                    "duplicate_type",
                    existing[0],
                    exact,
                    "type",
                ));
                None
            } else {
                let element_name = lexical_xdto_child_name(before, root, "objectType")
                    .map_err(writer_invalid_source)?;
                let declaration = desired_base
                    .as_ref()
                    .map_or_else(String::new, DesiredQName::namespace_declaration);
                let base_attribute = desired_base.as_ref().map_or_else(String::new, |base| {
                    format!(" base=\"{}\"", escape_attribute(&base.raw))
                });
                let fragment = format!(
                    "<{element_name}{declaration} name=\"{}\"{base_attribute}/>",
                    escape_attribute(name)
                );
                let next = anchor.and_then(|(node, before_anchor)| {
                    if before_anchor {
                        Some(node)
                    } else {
                        root.children()
                            .skip_while(|sibling| *sibling != node)
                            .skip(1)
                            .find(|sibling| sibling.has_tag_name((XDTO_NS, "objectType")))
                    }
                });
                Some(
                    if let Some(next) = next {
                        insert_line_before_node(before, next, &fragment)
                    } else {
                        insert_top_level(before, root, TopLevelInsert::ObjectType, &fragment)
                    }
                    .map_err(writer_invalid_source)?,
                )
            }
        }
        TypedWriterOperation::AddProperty {
            type_name,
            name,
            type_ref,
            min_occurs,
            property_path,
        } => {
            let target = property_target(root, type_name, property_path)?;
            let desired_qname = desired_qname(root, target, type_ref);
            let desired_lower =
                min_occurs.map_or_else(|| "1".to_string(), |value| value.to_string());
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
                let lower = min_occurs
                    .map(|value| format!(" lowerBound=\"{value}\""))
                    .unwrap_or_default();
                let element_name = lexical_xdto_child_name(before, target, "property")
                    .map_err(writer_invalid_source)?;
                let fragment = format!(
                    "<{element_name}{} name=\"{}\" type=\"{}\"{lower}/>",
                    desired_qname.namespace_declaration(),
                    escape_attribute(name),
                    escape_attribute(&desired_qname.raw)
                );
                Some(insert_child(before, target, &fragment).map_err(writer_invalid_source)?)
            }
        }
        TypedWriterOperation::AddPropertyTree {
            type_name,
            property,
            property_path,
        } => {
            let target = property_target(root, type_name, property_path)?;
            validate_property_tree_bases(root, target, property)?;
            let existing = direct_named_properties(target, property.name.as_str());
            if !existing.is_empty() {
                let exact = existing.len() == 1
                    && compatible_property_tree(existing[0], root, target, property);
                finding = Some(duplicate_finding(
                    "duplicate_property",
                    existing[0],
                    exact,
                    "property",
                ));
                None
            } else {
                let property_name = lexical_xdto_child_name(before, target, "property")
                    .map_err(writer_invalid_source)?;
                let fragment = if matches!(
                    property.definition,
                    XdtoPropertyDefinition::Named(_) | XdtoPropertyDefinition::Value { .. }
                ) {
                    // A leaf property needs only the existing insertion layout.
                    // Requiring a multiline ObjectType layout here rejects legacy inputs.
                    {
                        let definition_name = lexical_xdto_child_name(before, target, "typeDef")
                            .map_err(writer_invalid_source)?;
                        render_property_tree(
                            root,
                            target,
                            property,
                            &property_name,
                            &definition_name,
                            "",
                            "",
                            "",
                        )
                    }
                } else {
                    let indent =
                        infer_child_indent(before, target).map_err(writer_invalid_source)?;
                    let parent_indent = exact_line_indent(before, target.range().start)
                        .map_err(writer_invalid_source)?;
                    let unit = indent
                        .strip_prefix(parent_indent)
                        .filter(|v| !v.is_empty())
                        .unwrap_or("\t");
                    let eol = observed_eol(
                        before,
                        local_eol_after(before, target.range().end).or_else(|| {
                            local_eol_before(before, line_start(before, target.range().start))
                        }),
                    )
                    .map_err(writer_invalid_source)?;
                    let definition_name = lexical_xdto_child_name(before, target, "typeDef")
                        .map_err(writer_invalid_source)?;
                    render_property_tree(
                        root,
                        target,
                        property,
                        &property_name,
                        &definition_name,
                        &indent,
                        unit,
                        eol,
                    )
                };
                Some(insert_child(before, target, &fragment).map_err(writer_invalid_source)?)
            }
        }
        TypedWriterOperation::RemoveType { name } => {
            let node = exactly_one(
                named_types(root, name),
                "type",
                WriterErrorField::TypeTarget,
            )?;
            Some(remove_node(before, node))
        }
        TypedWriterOperation::RemoveProperty {
            type_name,
            name,
            property_path,
        } => {
            let target = property_target(root, type_name, property_path)?;
            let node = exactly_one(
                direct_named_properties(target, name),
                "property",
                WriterErrorField::PropertyName,
            )?;
            Some(remove_node(before, node))
        }
    };
    let edits = edit.into_iter().collect::<Vec<_>>();
    let after = apply_edits(before, &edits).map_err(writer_postcondition)?;
    Ok(WriterPlan {
        after,
        edits,
        finding,
    })
}

pub fn plan(
    before: &str,
    args: &Map<String, Value>,
    operation: &str,
) -> Result<WriterPlan, String> {
    let typed = match operation {
        "add-value-type" => {
            let name = super::required(args, "name")?;
            let base = super::required(args, "base")?;
            TypedWriterOperation::AddValueType { name, base, facets: None }
        }
        "add-object-type" => {
            let name = super::required(args, "name")?;
            TypedWriterOperation::AddObjectType {
                name,
                base: args.get("base").and_then(Value::as_str),
                before: args.get("before").and_then(Value::as_str),
                after: args.get("after").and_then(Value::as_str),
            }
        }
        "add-property" => {
            let type_name = super::required(args, "typeName")?;
            let property = args
                .get("property")
                .and_then(Value::as_object)
                .ok_or_else(|| "property must be an object".to_string())?;
            let name = object_string(property, "name")?;
            let type_ref = object_string(property, "type")?;
            TypedWriterOperation::AddProperty {
                type_name,
                name,
                type_ref,
                min_occurs: property.get("minOccurs").and_then(Value::as_u64),
                property_path: args.get("propertyPath").and_then(Value::as_str),
            }
        }
        "remove-type" => {
            let name = super::required(args, "name")?;
            TypedWriterOperation::RemoveType { name }
        }
        "remove-property" => {
            let type_name = super::required(args, "typeName")?;
            let name = super::required(args, "name")?;
            TypedWriterOperation::RemoveProperty {
                type_name,
                name,
                property_path: args.get("propertyPath").and_then(Value::as_str),
            }
        }
        _ => {
            return Err("unsupported_node: supported operations are add-value-type, add-object-type, add-property, remove-type, remove-property".to_string())
        }
    };
    plan_typed(before, typed).map_err(|error| error.to_string())
}

fn writer_postcondition(message: String) -> WriterError {
    WriterError::new(
        WriterErrorCause::Postcondition,
        WriterErrorField::Source,
        message,
    )
}

fn writer_invalid_source(message: String) -> WriterError {
    WriterError::new(
        WriterErrorCause::InvalidSource,
        WriterErrorField::Source,
        message,
    )
}

#[derive(Clone, Copy)]
enum TopLevelInsert {
    ValueType,
    ObjectType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SemanticQName {
    namespace: Option<String>,
    unresolved_prefix: Option<String>,
    local: String,
    lexical_valid: bool,
}

struct NamespaceBinding {
    prefix: String,
    uri: String,
}

struct DesiredQName {
    raw: String,
    semantic: SemanticQName,
    local_binding: Option<NamespaceBinding>,
}

impl DesiredQName {
    fn namespace_declaration(&self) -> String {
        self.local_binding
            .as_ref()
            .map_or_else(String::new, |binding| {
                format!(
                    " xmlns:{}=\"{}\"",
                    binding.prefix,
                    escape_attribute(&binding.uri)
                )
            })
    }
}

fn desired_qname(root: Node<'_, '_>, target: Node<'_, '_>, raw: &str) -> DesiredQName {
    let mut semantic = semantic_qname(target, raw);
    let local_binding = semantic
        .unresolved_prefix
        .as_deref()
        .and_then(|prefix| unique_package_namespace_binding(root, prefix));
    if let Some(binding) = &local_binding {
        semantic.namespace = Some(binding.uri.clone());
        semantic.unresolved_prefix = None;
    }
    DesiredQName {
        raw: raw.to_string(),
        semantic,
        local_binding,
    }
}

fn unique_package_namespace_binding(root: Node<'_, '_>, prefix: &str) -> Option<NamespaceBinding> {
    let uris = root
        .descendants()
        .filter(Node::is_element)
        .flat_map(|node| node.namespaces())
        .filter(|namespace| namespace.name() == Some(prefix))
        .map(|namespace| namespace.uri().to_string())
        .collect::<BTreeSet<_>>();
    let mut uris = uris.into_iter();
    let uri = uris.next()?;
    uris.next().is_none().then(|| NamespaceBinding {
        prefix: prefix.to_string(),
        uri,
    })
}

fn semantic_qname(node: Node<'_, '_>, raw: &str) -> SemanticQName {
    let parts = raw.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [prefix, local] if !prefix.is_empty() && !local.is_empty() => {
            let namespace = node.lookup_namespace_uri(Some(prefix)).map(str::to_string);
            SemanticQName {
                unresolved_prefix: namespace.is_none().then(|| (*prefix).to_string()),
                namespace,
                local: (*local).to_string(),
                lexical_valid: true,
            }
        }
        [local] if !local.is_empty() => SemanticQName {
            namespace: None,
            unresolved_prefix: None,
            local: (*local).to_string(),
            lexical_valid: true,
        },
        _ => SemanticQName {
            namespace: None,
            unresolved_prefix: None,
            local: raw.to_string(),
            lexical_valid: false,
        },
    }
}

fn facet_attributes(facets: &ValueFacets) -> String {
    facets
        .attributes()
        .into_iter()
        .map(|(name, value)| format!(" {name}=\"{}\"", escape_attribute(&value)))
        .collect()
}

fn compatible_value_type(node: Node<'_, '_>, base: &SemanticQName, facets: &ValueFacets) -> bool {
    node.has_tag_name((XDTO_NS, "valueType"))
        && has_only_plain_attributes(
            node,
            &[
                "name",
                "base",
                "maxLength",
                "totalDigits",
                "fractionDigits",
                "minInclusive",
                "maxInclusive",
            ],
        )
        && crate::facets::from_node(node).is_ok_and(|existing| existing.equivalent(facets))
        && has_exact_compatible_content(node)
        && node.attribute("base").is_some_and(|existing| {
            let existing = semantic_qname(node, existing);
            existing.lexical_valid && base.lexical_valid && existing == *base
        })
}

fn prove_local_object_base(
    root: Node<'_, '_>,
    name: &str,
    base: &SemanticQName,
) -> Result<(), WriterError> {
    let mut visited = BTreeSet::from([name.to_string()]);
    let mut current = base.clone();
    loop {
        if !current.lexical_valid
            || current.unresolved_prefix.is_some()
            || current.namespace.as_deref() != root.attribute("targetNamespace")
        {
            return Err(WriterError::new(WriterErrorCause::BadValue, WriterErrorField::Base,
                "base must resolve to a local ObjectType; imported inheritance is not supported for this addition"));
        }
        if !visited.insert(current.local.clone()) {
            return Err(WriterError::new(
                WriterErrorCause::BadValue,
                WriterErrorField::Base,
                "base introduces or reaches an inheritance cycle",
            ));
        }
        let candidates = named_types(root, &current.local);
        let parent = match candidates.as_slice() {
            [node] if node.has_tag_name((XDTO_NS, "objectType")) => *node,
            [] => {
                return Err(WriterError::new(
                    WriterErrorCause::NotFound,
                    WriterErrorField::Base,
                    "base ObjectType does not exist",
                ))
            }
            [_] => {
                return Err(WriterError::new(
                    WriterErrorCause::BadValue,
                    WriterErrorField::Base,
                    "base must name an ObjectType, not a ValueType",
                ))
            }
            _ => {
                return Err(WriterError::new(
                    WriterErrorCause::AmbiguousTarget,
                    WriterErrorField::Base,
                    "base has multiple declarations",
                ))
            }
        };
        let Some(next) = parent.attribute("base") else {
            return Ok(());
        };
        current = semantic_qname(parent, next);
    }
}

fn object_position_anchor<'a>(
    root: Node<'a, 'a>,
    name: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<Option<(Node<'a, 'a>, bool)>, WriterError> {
    let (anchor, is_before, field) = match (before, after) {
        (None, None) => return Ok(None),
        (Some(anchor), None) => (anchor, true, WriterErrorField::Before),
        (None, Some(anchor)) => (anchor, false, WriterErrorField::After),
        _ => {
            return Err(WriterError::new(
                WriterErrorCause::BadValue,
                WriterErrorField::After,
                "before and after are mutually exclusive",
            ))
        }
    };
    if anchor == name {
        return Err(WriterError::new(
            WriterErrorCause::BadValue,
            field,
            "a new ObjectType cannot use itself as its neighbor",
        ));
    }
    let candidates = named_types(root, anchor);
    match candidates.as_slice() {
        [node] if node.has_tag_name((XDTO_NS, "objectType")) => Ok(Some((*node, is_before))),
        [] => Err(WriterError::new(
            WriterErrorCause::NotFound,
            field,
            "position must name an existing local ObjectType",
        )),
        [_] => Err(WriterError::new(
            WriterErrorCause::BadValue,
            field,
            "position must name an ObjectType, not a ValueType",
        )),
        _ => Err(WriterError::new(
            WriterErrorCause::AmbiguousTarget,
            field,
            "position names multiple declarations",
        )),
    }
}

fn object_position_matches(
    root: Node<'_, '_>,
    existing: Node<'_, '_>,
    anchor: Option<(Node<'_, '_>, bool)>,
) -> bool {
    let Some((anchor, before)) = anchor else {
        return true;
    };
    let objects = root
        .children()
        .filter(|node| node.has_tag_name((XDTO_NS, "objectType")))
        .collect::<Vec<_>>();
    let existing_index = objects.iter().position(|node| *node == existing);
    let anchor_index = objects.iter().position(|node| *node == anchor);
    match (existing_index, anchor_index) {
        (Some(existing), Some(anchor)) if before => existing + 1 == anchor,
        (Some(existing), Some(anchor)) => anchor + 1 == existing,
        _ => false,
    }
}

fn compatible_object_type(node: Node<'_, '_>, base: Option<&SemanticQName>) -> bool {
    node.has_tag_name((XDTO_NS, "objectType"))
        && has_only_plain_attributes(node, &["name", "base"])
        && has_exact_compatible_content(node)
        && match (node.attribute("base"), base) {
            (None, None) => true,
            (Some(existing), Some(base)) => semantic_qname(node, existing) == *base,
            _ => false,
        }
}

fn compatible_property_tree(
    node: Node<'_, '_>,
    root: Node<'_, '_>,
    context: Node<'_, '_>,
    spec: &XdtoPropertySpec,
) -> bool {
    if !node.has_tag_name((XDTO_NS, "property"))
        || node.attribute("name") != Some(spec.name.as_str())
        || !has_only_plain_attributes(node, &["name", "type", "lowerBound", "upperBound"])
        || node
            .attribute("lowerBound")
            .unwrap_or("1")
            .parse::<u8>()
            .ok()
            != Some(spec.min_occurs.unwrap_or(1))
        || node
            .attribute("upperBound")
            .unwrap_or("1")
            .parse::<i64>()
            .ok()
            != Some(spec.max_occurs.unwrap_or(1))
        || !whitespace_character_data(node)
    {
        return false;
    }
    match &spec.definition {
        XdtoPropertyDefinition::Named(name) => {
            let desired = desired_qname(root, context, name.as_str());
            has_exact_compatible_content(node)
                && node
                    .attribute("type")
                    .is_some_and(|raw| semantic_qname(node, raw) == desired.semantic)
        }
        XdtoPropertyDefinition::Value { base, facets } => {
            if node.attribute("type").is_some() {
                return false;
            }
            let definitions = node.children().filter(Node::is_element).collect::<Vec<_>>();
            let [definition] = definitions.as_slice() else {
                return false;
            };
            let desired = desired_qname(root, context, base.as_str());
            definition.has_tag_name((XDTO_NS, "typeDef"))
                && definition.attribute((XSI_NS, "type")) == Some("ValueType")
                && definition.attributes().all(|attribute| {
                    (attribute.namespace() == Some(XSI_NS) && attribute.name() == "type")
                        || (attribute.namespace().is_none()
                            && (attribute.name() == "base"
                                || crate::facets::NAMES.contains(&attribute.name())))
                })
                && has_exact_compatible_content(*definition)
                && definition
                    .attribute("base")
                    .is_some_and(|raw| semantic_qname(*definition, raw) == desired.semantic)
                && crate::facets::from_node(*definition)
                    .is_ok_and(|actual| actual.equivalent(facets))
        }
        XdtoPropertyDefinition::Object(properties) => {
            if node.attribute("type").is_some() {
                return false;
            }
            let definitions = node.children().filter(Node::is_element).collect::<Vec<_>>();
            let [definition] = definitions.as_slice() else {
                return false;
            };
            if !definition.has_tag_name((XDTO_NS, "typeDef"))
                || definition.attribute((XSI_NS, "type")) != Some("ObjectType")
                || !definition
                    .attributes()
                    .all(|a| a.namespace() == Some(XSI_NS) && a.name() == "type")
                || !whitespace_character_data(*definition)
            {
                return false;
            }
            let children = definition
                .children()
                .filter(Node::is_element)
                .collect::<Vec<_>>();
            children.len() == properties.len()
                && children
                    .into_iter()
                    .zip(properties)
                    .all(|(child, spec)| compatible_property_tree(child, root, context, spec))
        }
    }
}

fn validate_property_tree_bases(
    root: Node<'_, '_>,
    context: Node<'_, '_>,
    spec: &XdtoPropertySpec,
) -> Result<(), WriterError> {
    match &spec.definition {
        XdtoPropertyDefinition::Named(_) => Ok(()),
        XdtoPropertyDefinition::Object(children) => children
            .iter()
            .try_for_each(|child| validate_property_tree_bases(root, context, child)),
        XdtoPropertyDefinition::Value { base, facets } => {
            let desired = desired_qname(root, context, base.as_str());
            facets
                .validate_anonymous_base(
                    desired.semantic.namespace.as_deref(),
                    &desired.semantic.local,
                )
                .map_err(|message| writer_invalid_source(message.to_string()))
        }
    }
}

fn whitespace_character_data(node: Node<'_, '_>) -> bool {
    node.children()
        .filter(Node::is_text)
        .all(|n| n.text().is_none_or(|s| s.chars().all(char::is_whitespace)))
}

fn property_requests_prefix(spec: &XdtoPropertySpec, prefix: &str) -> bool {
    match &spec.definition {
        XdtoPropertyDefinition::Named(name) => name
            .as_str()
            .split_once(':')
            .is_some_and(|(used, _)| used == prefix),
        XdtoPropertyDefinition::Value { base, .. } => base
            .as_str()
            .split_once(':')
            .is_some_and(|(used, _)| used == prefix),
        XdtoPropertyDefinition::Object(children) => children
            .iter()
            .any(|child| property_requests_prefix(child, prefix)),
    }
}

// A fresh prefix does not shadow any QName supplied for a child property.
fn xsi_binding(
    root: Node<'_, '_>,
    context: Node<'_, '_>,
    spec: &XdtoPropertySpec,
) -> (String, String) {
    if let Some(prefix) = context.lookup_prefix(XSI_NS).filter(|p| !p.is_empty()) {
        return (prefix.to_string(), String::new());
    }
    let mut prefix = "xsi".to_string();
    let mut index = 0;
    while root
        .descendants()
        .filter(Node::is_element)
        .any(|n| n.namespaces().any(|ns| ns.name() == Some(prefix.as_str())))
        || property_requests_prefix(spec, &prefix)
    {
        index += 1;
        prefix = format!("xdtoXsi{index}");
    }
    let declaration = format!(" xmlns:{prefix}=\"{XSI_NS}\"");
    (prefix, declaration)
}

#[allow(clippy::too_many_arguments)]
fn render_property_tree(
    root: Node<'_, '_>,
    context: Node<'_, '_>,
    spec: &XdtoPropertySpec,
    property_name: &str,
    definition_name: &str,
    indent: &str,
    unit: &str,
    eol: &str,
) -> String {
    let bounds = format!(
        "{}{}",
        spec.min_occurs
            .map(|v| format!(" lowerBound=\"{v}\""))
            .unwrap_or_default(),
        spec.max_occurs
            .map(|v| format!(" upperBound=\"{v}\""))
            .unwrap_or_default()
    );
    match &spec.definition {
        XdtoPropertyDefinition::Named(name) => {
            let desired = desired_qname(root, context, name.as_str());
            format!(
                "<{property_name}{} name=\"{}\" type=\"{}\"{bounds}/>",
                desired.namespace_declaration(),
                escape_attribute(spec.name.as_str()),
                escape_attribute(&desired.raw)
            )
        }
        XdtoPropertyDefinition::Value { base, facets } => {
            let desired = desired_qname(root, context, base.as_str());
            let (prefix, declaration) = xsi_binding(root, context, spec);
            let default_namespace = if context.lookup_namespace_uri(None) == Some(XDTO_NS) {
                String::new()
            } else {
                format!(" xmlns=\"{XDTO_NS}\"")
            };
            format!("<{property_name} name=\"{}\"{bounds}><{definition_name}{declaration}{default_namespace} {prefix}:type=\"ValueType\"{} base=\"{}\"{}/></{property_name}>", escape_attribute(spec.name.as_str()), desired.namespace_declaration(), escape_attribute(&desired.raw), facet_attributes(facets))
        }
        XdtoPropertyDefinition::Object(properties) => {
            let (prefix, declaration) = xsi_binding(root, context, spec);
            let default_namespace = if context.lookup_namespace_uri(None) == Some(XDTO_NS) {
                String::new()
            } else {
                format!(" xmlns=\"{XDTO_NS}\"")
            };
            let child_indent = format!("{indent}{unit}{unit}");
            let mut fragment = format!("<{property_name} name=\"{}\"{bounds}>{eol}{indent}{unit}<{definition_name}{declaration}{default_namespace} {prefix}:type=\"ObjectType\">", escape_attribute(spec.name.as_str()));
            for child in properties {
                fragment.push_str(eol);
                fragment.push_str(&child_indent);
                fragment.push_str(&render_property_tree(
                    root,
                    context,
                    child,
                    property_name,
                    definition_name,
                    &child_indent,
                    unit,
                    eol,
                ));
            }
            fragment.push_str(&format!(
                "{eol}{indent}{unit}</{definition_name}>{eol}{indent}</{property_name}>"
            ));
            fragment
        }
    }
}

fn compatible_property(node: Node<'_, '_>, type_ref: &SemanticQName, desired_lower: &str) -> bool {
    node.has_tag_name((XDTO_NS, "property"))
        && has_only_plain_attributes(node, &["name", "type", "lowerBound", "upperBound"])
        && has_exact_compatible_content(node)
        && node.attribute("type").is_some_and(|existing| {
            let existing = semantic_qname(node, existing);
            existing.lexical_valid && type_ref.lexical_valid && existing == *type_ref
        })
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
    let sibling = root.children().rfind(Node::is_element);
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
    let sibling = target.children().rfind(Node::is_element);
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
    if sibling.is_none()
        && target.parent_element().is_none()
        && line_start(text, range.start) == line_start(text, close)
        && target.children().all(|child| {
            child.is_text()
                && child.text().is_none_or(|value| {
                    value
                        .chars()
                        .all(|character| matches!(character, ' ' | '\t' | '\r' | '\n'))
                })
        })
    {
        return Ok(TextEdit {
            range: close..close,
            replacement: fragment.to_string(),
        });
    }
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
    let parent = target
        .parent_element()
        .ok_or_else(unsupported_childless_root_indent)?;
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

fn unsupported_childless_root_indent() -> String {
    "unsupported_node: childless root has no indentation evidence for its first child".to_string()
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

fn lexical_xdto_child_name(
    text: &str,
    parent: Node<'_, '_>,
    child_local: &str,
) -> Result<String, String> {
    let parent_name = lexical_element_name(&text[parent.range()])?;
    Ok(parent_name.rsplit_once(':').map_or_else(
        || child_local.to_string(),
        |(prefix, _)| format!("{prefix}:{child_local}"),
    ))
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
) -> Result<Node<'a, 'a>, WriterError> {
    let matching = named_types(root, type_name);
    let mut target = match matching.as_slice() {
        [] => {
            return Err(WriterError::new(
                WriterErrorCause::NotFound,
                WriterErrorField::TypeTarget,
                "target_not_found: type does not exist",
            ))
        }
        [node] if node.has_tag_name((XDTO_NS, "objectType")) => *node,
        [node] if node.has_tag_name((XDTO_NS, "valueType")) => {
            return Err(WriterError::new(
                WriterErrorCause::BadValue,
                WriterErrorField::TypeTarget,
                "unsupported_node: properties can be added only to objectType",
            ))
        }
        [_] => {
            return Err(WriterError::new(
                WriterErrorCause::InvalidSource,
                WriterErrorField::TypeTarget,
                "unsupported_node: property target kind is unsupported",
            ))
        }
        _ => {
            return Err(WriterError::new(
                WriterErrorCause::AmbiguousTarget,
                WriterErrorField::TypeTarget,
                "unsupported_node: named type identity is ambiguous",
            ))
        }
    };
    let segments = strict_property_path(property_path)?;
    for segment in segments {
        let properties = direct_named_properties(target, &segment);
        let property = exactly_one(
            properties,
            "property path segment",
            WriterErrorField::PropertyPath,
        )?;
        let type_defs = property
            .children()
            .filter(|child| child.has_tag_name((XDTO_NS, "typeDef")))
            .collect::<Vec<_>>();
        target = match type_defs.as_slice() {
            [] => {
                return Err(WriterError::new(
                    WriterErrorCause::NotFound,
                    WriterErrorField::PropertyPath,
                    format!(
                        "target_not_found: property path segment `{segment}` has no nested typeDef:ObjectType"
                    ),
                ))
            }
            [type_def] if type_def.attribute((XSI_NS, "type")) == Some("ObjectType") => *type_def,
            _ => {
                return Err(WriterError::new(
                    WriterErrorCause::InvalidSource,
                    WriterErrorField::PropertyPath,
                    format!(
                        "unsupported_node: property path segment `{segment}` must own exactly one typeDef with xsi:type=\"ObjectType\""
                    ),
                ))
            }
        };
    }
    Ok(target)
}

fn strict_property_path(property_path: Option<&str>) -> Result<Vec<String>, WriterError> {
    let Some(path) = property_path else {
        return Ok(Vec::new());
    };
    let invalid = || {
        WriterError::new(
        WriterErrorCause::BadValue,
        WriterErrorField::PropertyPath,
        "property_path_invalid: propertyPath must contain dot-separated XML NCNames and escape literal dots as `\\.`",
    )
    };
    if path.is_empty() {
        return Err(invalid());
    }
    let mut segments = Vec::new();
    let mut segment = String::new();
    let mut characters = path.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                if characters.next() != Some('.') {
                    return Err(invalid());
                }
                segment.push('.');
            }
            '.' => {
                if !super::validation::is_ncname(&segment) {
                    return Err(invalid());
                }
                segments.push(std::mem::take(&mut segment));
            }
            _ => segment.push(character),
        }
    }
    if !super::validation::is_ncname(&segment) {
        return Err(invalid());
    }
    segments.push(segment);
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

fn exactly_one<'a>(
    nodes: Vec<Node<'a, 'a>>,
    noun: &str,
    field: WriterErrorField,
) -> Result<Node<'a, 'a>, WriterError> {
    match nodes.as_slice() {
        [] => Err(WriterError::new(
            WriterErrorCause::NotFound,
            field,
            format!("target_not_found: {noun} does not exist"),
        )),
        [node] => Ok(*node),
        _ => Err(WriterError::new(
            WriterErrorCause::AmbiguousTarget,
            field,
            format!("unsupported_node: {noun} identity is ambiguous"),
        )),
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
        .filter(|value| !value.trim().is_empty() && *value == value.trim())
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
    use crate::test_support::args;
    use serde_json::json;

    const ROOT: &str = r#"xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test""#;

    fn package(body: &str) -> String {
        format!("<package {ROOT}>\n{body}</package>")
    }

    #[test]
    fn xdto_object_base_position_repeats_aliases_and_rejects_moving_duplicates() {
        let before = package("\t<objectType name=\"Base\"/>\n\t<objectType name=\"A\"/>\n\t<!-- keep -->\n\t<objectType name=\"C\"/>\n")
            .replace("xmlns:tns=\"urn:test\"", "xmlns:tns=\"urn:test\" xmlns:same=\"urn:test\"");
        let request = args(&[
            ("name", json!("B")),
            ("base", json!("tns:Base")),
            ("after", json!("A")),
        ]);
        let added = plan(&before, &request, "add-object-type").unwrap();
        assert_edit_applies(&before, &added);
        assert_eq!(
            added
                .after
                .replace("\t<objectType name=\"B\" base=\"tns:Base\"/>\n", ""),
            before
        );
        assert!(added.after.find("name=\"A\"").unwrap() < added.after.find("name=\"B\"").unwrap());
        assert!(added.after.find("name=\"B\"").unwrap() < added.after.find("name=\"C\"").unwrap());
        let mut alias = request.clone();
        alias.insert("base".into(), json!("same:Base"));
        let repeated = plan(&added.after, &alias, "add-object-type").unwrap();
        assert!(!repeated.blocks());
        assert!(repeated.edits.is_empty());
        assert_eq!(repeated.after, added.after);
        for (key, value) in [("base", json!("tns:A")), ("after", json!("C"))] {
            let mut conflict = request.clone();
            conflict.insert(key.into(), value);
            let refused = plan(&added.after, &conflict, "add-object-type").unwrap();
            assert!(refused.blocks());
            assert_eq!(refused.after, added.after);
        }
        let tail = plan(
            &before,
            &args(&[("name", json!("Tail")), ("after", json!("C"))]),
            "add-object-type",
        )
        .unwrap();
        assert!(tail.after.find("name=\"C\"").unwrap() < tail.after.find("name=\"Tail\"").unwrap());
    }

    #[test]
    fn xdto_object_base_position_refuses_invalid_chain_and_neighbors() {
        let before = package("\t<valueType name=\"Scalar\" base=\"xs:string\"/>\n\t<objectType name=\"Base\"/>\n\t<objectType name=\"CycleA\" base=\"tns:CycleB\"/>\n\t<objectType name=\"CycleB\" base=\"tns:CycleA\"/>\n\t<objectType name=\"External\" base=\"ext:Unknown\"/>\n");
        for base in [
            "tns:Missing",
            "tns:Scalar",
            "tns:New",
            "tns:CycleA",
            "tns:External",
            "unknown:Base",
        ] {
            let request = args(&[("name", json!("New")), ("base", json!(base))]);
            assert!(
                plan(&before, &request, "add-object-type").is_err(),
                "{base}"
            );
        }
        for fields in [
            args(&[("before", json!("Missing"))]),
            args(&[("after", json!("Scalar"))]),
            args(&[("before", json!("New"))]),
            args(&[("before", json!("Base")), ("after", json!("Base"))]),
        ] {
            let mut request = fields;
            request.insert("name".into(), json!("New"));
            assert!(plan(&before, &request, "add-object-type").is_err());
        }
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

    fn assert_removal(
        before: &str,
        plan: &super::WriterPlan,
        expected_range: std::ops::Range<usize>,
        expected_after: &str,
    ) {
        assert_eq!(plan.edits.len(), 1);
        let edit = &plan.edits[0];
        assert_eq!(edit.range, expected_range);
        assert!(edit.replacement.is_empty());
        assert_eq!(plan.after, expected_after);
        assert_eq!(&plan.after[..edit.range.start], &before[..edit.range.start]);
        assert_eq!(&plan.after[edit.range.start..], &before[edit.range.end..]);
        assert_edit_applies(before, plan);
    }

    #[test]
    fn xdto_writer_preserves_the_lexical_xdto_prefix_for_all_adds_and_nested_closing_tag() {
        let mut text = r#"<x:package xmlns:x="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" targetNamespace="urn:test">
	<x:objectType name="Target">
		<x:property name="Nested">
			<x:typeDef xsi:type="ObjectType"/>
		</x:property>
	</x:objectType>
</x:package>"#
            .to_string();
        let operations = [
            (
                "add-value-type",
                args(&[("name", json!("AddedValue")), ("base", json!("xs:string"))]),
            ),
            ("add-object-type", args(&[("name", json!("AddedObject"))])),
            (
                "add-property",
                args(&[
                    ("typeName", json!("Target")),
                    ("propertyPath", json!("Nested")),
                    (
                        "property",
                        json!({"name":"AddedProperty", "type":"xs:string"}),
                    ),
                ]),
            ),
        ];

        for (operation, arguments) in operations {
            text = plan(&text, &arguments, operation).unwrap().after;
            let model = super::super::model::PackageModel::parse(&text).unwrap();
            assert!(
                super::super::validation::validate(&model).is_empty(),
                "{operation}: {text}"
            );
        }
        assert!(text.contains("<x:valueType name=\"AddedValue\""));
        assert!(text.contains("<x:objectType name=\"AddedObject\""));
        assert!(text.contains("<x:property name=\"AddedProperty\""));
        assert!(text.contains("</x:typeDef>"));
        assert!(!text.contains("</typeDef>"));
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
    fn xdto_writer_adds_the_first_value_type_without_inventing_inline_root_layout() {
        let before = format!("<package {ROOT}></package>");
        let planned = plan(
            &before,
            &args(&[("name", json!("Added")), ("base", json!("xs:string"))]),
            "add-value-type",
        )
        .expect("an inline childless package must accept its first valueType");

        assert_eq!(
            planned.after,
            format!("<package {ROOT}><valueType name=\"Added\" base=\"xs:string\"/></package>")
        );
        assert_edit_applies(&before, &planned);
    }

    #[test]
    fn xdto_writer_names_missing_indent_evidence_for_a_multiline_childless_root() {
        let before = package("");
        let error = plan(
            &before,
            &args(&[("name", json!("Added")), ("base", json!("xs:string"))]),
            "add-value-type",
        )
        .expect_err("a multiline childless root has no observed child indentation");

        assert_eq!(
            error,
            "unsupported_node: childless root has no indentation evidence for its first child"
        );
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
        for property_path in [
            ".",
            ".Foo",
            "Foo.",
            "Foo..Bar",
            "Foo. .Bar",
            r"\Foo",
            r"Foo\Bar",
            "Foo\\",
            r"Foo\\.Bar",
            r"Foo\.Bar\Baz",
        ] {
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
    fn xdto_writer_rejects_padded_property_fields_before_planning() {
        let before = package("\t<objectType name=\"Target\"/>\n");
        for (field, property) in [
            ("name", json!({"name":" Added", "type":"xs:string"})),
            ("type", json!({"name":"Added", "type":"xs:string "})),
        ] {
            let error = plan(
                &before,
                &args(&[("typeName", json!("Target")), ("property", property)]),
                "add-property",
            )
            .expect_err("padded property fields must fail argument parsing");

            assert_eq!(
                error,
                format!("property.{field} must be a non-empty string")
            );
        }
    }

    #[test]
    fn xdto_writer_addresses_a_dotted_property_identity_with_an_escape() {
        let before = package(
            "\t<objectType name=\"Target\">\n\t\t<property name=\"A.B\">\n\t\t\t<typeDef xsi:type=\"ObjectType\"/>\n\t\t</property>\n\t</objectType>\n",
        );
        let before_model = super::super::model::PackageModel::parse(&before).unwrap();
        assert_eq!(
            before_model.types[0].properties[0].logical_identity(),
            "A.B"
        );

        let planned = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("propertyPath", json!(r"A\.B")),
                ("property", json!({"name":"Added", "type":"xs:string"})),
            ]),
            "add-property",
        )
        .expect("escaped dot must select the A.B property identity");

        let after_model = super::super::model::PackageModel::parse(&planned.after).unwrap();
        let dotted = &after_model.types[0].properties[0];
        assert_eq!(dotted.logical_identity(), "A.B");
        assert_eq!(
            dotted.type_defs[0].properties[0].logical_identity(),
            "Added"
        );
        assert!(super::super::validation::validate(&after_model).is_empty());
    }

    #[test]
    fn xdto_writer_separates_only_unescaped_property_path_dots() {
        let before = package(
            "\t<objectType name=\"Target\">\n\t\t<property name=\"A.B\">\n\t\t\t<typeDef xsi:type=\"ObjectType\">\n\t\t\t\t<property name=\"Child\">\n\t\t\t\t\t<typeDef xsi:type=\"ObjectType\"/>\n\t\t\t\t</property>\n\t\t\t</typeDef>\n\t\t</property>\n\t</objectType>\n",
        );

        let planned = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("propertyPath", json!(r"A\.B.Child")),
                ("property", json!({"name":"Added", "type":"xs:string"})),
            ]),
            "add-property",
        )
        .expect("A\\.B.Child must resolve as the A.B and Child segments");

        let model = super::super::model::PackageModel::parse(&planned.after).unwrap();
        let dotted = &model.types[0].properties[0];
        let child = &dotted.type_defs[0].properties[0];
        assert_eq!(child.logical_identity(), "Child");
        assert_eq!(child.type_defs[0].properties[0].logical_identity(), "Added");
        assert!(super::super::validation::validate(&model).is_empty());
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
    fn xdto_writer_qname_duplicates_require_resolved_or_lexically_identical_identity() {
        let declared = ROOT.replace(
            "targetNamespace=\"urn:test\"",
            "xmlns:a=\"urn:external\" xmlns:b=\"urn:external\" targetNamespace=\"urn:test\"",
        );
        let value_before = format!(
            "<package {declared}>\n\t<import namespace=\"urn:external\"/>\n\t<valueType name=\"Existing\" base=\"a:T\"/>\n</package>"
        );
        let value_exact = plan(
            &value_before,
            &args(&[("name", json!("Existing")), ("base", json!("b:T"))]),
            "add-value-type",
        )
        .unwrap();
        assert!(!value_exact.conflict());
        assert_eq!(value_exact.duplicate_code(), Some("duplicate_type"));

        let property_before = format!(
            "<package {declared}>\n\t<import namespace=\"urn:external\"/>\n\t<objectType name=\"Target\">\n\t\t<property name=\"Existing\" type=\"a:T\"/>\n\t</objectType>\n</package>"
        );
        let property_exact = plan(
            &property_before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Existing", "type":"b:T"})),
            ]),
            "add-property",
        )
        .unwrap();
        assert!(!property_exact.conflict());
        assert_eq!(property_exact.duplicate_code(), Some("duplicate_property"));

        for (label, existing, requested) in [
            ("different unresolved prefixes", "foo:T", "bar:T"),
            ("bare versus unresolved", "foo:T", "T"),
            ("same lexical-invalid QName", "foo:T:bad", "foo:T:bad"),
        ] {
            let value_before = package(&format!(
                "\t<valueType name=\"Existing\" base=\"{existing}\"/>\n"
            ));
            let value_conflict = plan(
                &value_before,
                &args(&[("name", json!("Existing")), ("base", json!(requested))]),
                "add-value-type",
            )
            .unwrap();
            assert!(value_conflict.conflict(), "valueType: {label}");

            let property_before = package(&format!(
                "\t<objectType name=\"Target\">\n\t\t<property name=\"Existing\" type=\"{existing}\"/>\n\t</objectType>\n"
            ));
            let property_conflict = plan(
                &property_before,
                &args(&[
                    ("typeName", json!("Target")),
                    ("property", json!({"name":"Existing", "type":requested})),
                ]),
                "add-property",
            )
            .unwrap();
            assert!(property_conflict.conflict(), "property: {label}");
        }
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
                    json!({"name":"Existing", "type":"tns:Local", "minOccurs":1}),
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
            json!({"name":"Existing", "type":"tns:Local", "minOccurs":0}),
            json!({"name":"Structured", "type":"tns:Local", "minOccurs":1}),
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
    fn xdto_writer_rejects_a_joint_mixed_kind_property_target_as_ambiguous() {
        let before = package(
            "\t<valueType name=\"Target\" base=\"xs:string\"/>\n\t<objectType name=\"Target\"/>\n",
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
        assert!(error.contains("ambiguous"), "{error}");
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
    fn xdto_writer_removal_matrix_preserves_exact_outer_slices() {
        let inline_node = "<valueType name=\"Remove\" base=\"xs:string\"/>";
        let inline = format!("<package {ROOT}>{inline_node}<objectType name=\"Keep\"/></package>");
        let inline_start = inline.find(inline_node).unwrap();
        let inline_plan =
            plan(&inline, &args(&[("name", json!("Remove"))]), "remove-type").unwrap();
        assert_removal(
            &inline,
            &inline_plan,
            inline_start..inline_start + inline_node.len(),
            &inline.replacen(inline_node, "", 1),
        );

        let lf_line = "\t<valueType name=\"Remove\" base=\"xs:string\"/>\n";
        let own_line_lf = package(&format!(
            "\t<valueType name=\"Keep\" base=\"xs:string\"/>\n{lf_line}\t<objectType name=\"Object\"/>\n"
        ));
        let lf_start = own_line_lf.find(lf_line).unwrap();
        let lf_plan = plan(
            &own_line_lf,
            &args(&[("name", json!("Remove"))]),
            "remove-type",
        )
        .unwrap();
        assert_removal(
            &own_line_lf,
            &lf_plan,
            lf_start..lf_start + lf_line.len(),
            &own_line_lf.replacen(lf_line, "", 1),
        );

        let crlf_line = "\t<valueType name=\"Remove\" base=\"xs:string\"/>\r\n";
        let own_line_crlf = package(
            "\t<valueType name=\"Keep\" base=\"xs:string\"/>\n\t<valueType name=\"Remove\" base=\"xs:string\"/>\n\t<objectType name=\"Object\"/>\n",
        )
        .replace('\n', "\r\n");
        let crlf_start = own_line_crlf.find(crlf_line).unwrap();
        let crlf_plan = plan(
            &own_line_crlf,
            &args(&[("name", json!("Remove"))]),
            "remove-type",
        )
        .unwrap();
        assert_removal(
            &own_line_crlf,
            &crlf_plan,
            crlf_start..crlf_start + crlf_line.len(),
            &own_line_crlf.replacen(crlf_line, "", 1),
        );

        let last_line = "\t<valueType name=\"Remove\" base=\"xs:string\"/>\n";
        let last_before_close =
            format!("<package {ROOT}>\n\t<objectType name=\"Keep\"/>\n{last_line}</package>");
        let last_start = last_before_close.find(last_line).unwrap();
        let last_plan = plan(
            &last_before_close,
            &args(&[("name", json!("Remove"))]),
            "remove-type",
        )
        .unwrap();
        assert_removal(
            &last_before_close,
            &last_plan,
            last_start..last_start + last_line.len(),
            &last_before_close.replacen(last_line, "", 1),
        );

        let nested_line = "\t\t\t\t<property name=\"Remove\" type=\"xs:string\"/>\n";
        let nested = package(&format!(
            "\t<objectType name=\"Target\">\n\t\t<property name=\"Nested\">\n\t\t\t<typeDef xsi:type=\"ObjectType\">\n{nested_line}\t\t\t\t<property name=\"Keep\" type=\"xs:string\"/>\n\t\t\t</typeDef>\n\t\t</property>\n\t</objectType>\n"
        ));
        let nested_start = nested.find(nested_line).unwrap();
        let nested_plan = plan(
            &nested,
            &args(&[
                ("typeName", json!("Target")),
                ("propertyPath", json!("Nested")),
                ("name", json!("Remove")),
            ]),
            "remove-property",
        )
        .unwrap();
        assert_removal(
            &nested,
            &nested_plan,
            nested_start..nested_start + nested_line.len(),
            &nested.replacen(nested_line, "", 1),
        );

        let blank_line = "\t<valueType name=\"Remove\" base=\"xs:string\"/>\n";
        let neighboring_blanks = package(&format!(
            "\t<valueType name=\"Keep\" base=\"xs:string\"/>\n\n{blank_line}\n\t<objectType name=\"Object\"/>\n"
        ));
        let blank_start = neighboring_blanks.find(blank_line).unwrap();
        let blank_plan = plan(
            &neighboring_blanks,
            &args(&[("name", json!("Remove"))]),
            "remove-type",
        )
        .unwrap();
        assert_removal(
            &neighboring_blanks,
            &blank_plan,
            blank_start..blank_start + blank_line.len(),
            &neighboring_blanks.replacen(blank_line, "", 1),
        );
    }

    #[test]
    fn xdto_writer_preserves_bare_qnames_for_validation_to_reject() {
        let root = ROOT.replace("xmlns:tns=\"urn:test\"", "xmlns:self=\"urn:test\"");
        let before = format!(
            "<package {root}>\n\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<objectType name=\"Target\"/>\n</package>"
        );
        let value_plan = plan(
            &before,
            &args(&[("name", json!("AddedValue")), ("base", json!("Local"))]),
            "add-value-type",
        )
        .unwrap();
        assert!(value_plan
            .after
            .contains("name=\"AddedValue\" base=\"Local\""));
        let value_model = super::super::model::PackageModel::parse(&value_plan.after).unwrap();
        let value_findings = super::super::validation::validate(&value_model);
        assert_eq!(value_findings.len(), 1);
        assert_eq!(value_findings[0].code, "invalid_qname");

        let property_plan = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"Local"})),
            ]),
            "add-property",
        )
        .unwrap();

        assert!(property_plan
            .after
            .contains("name=\"Added\" type=\"Local\""));
        assert!(!property_plan.after.contains("type=\"self:Local\""));
        let property_model =
            super::super::model::PackageModel::parse(&property_plan.after).unwrap();
        let property_findings = super::super::validation::validate(&property_model);
        assert_eq!(property_findings.len(), 1);
        assert_eq!(property_findings[0].code, "invalid_qname");
    }

    #[test]
    fn xdto_writer_preserves_declared_prefixed_qnames_exactly() {
        let root = ROOT.replace("xmlns:tns=\"urn:test\"", "xmlns:self=\"urn:test\"");
        let before = format!(
            "<package {root}>\n\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<objectType name=\"Target\"/>\n</package>"
        );
        let value_plan = plan(
            &before,
            &args(&[("name", json!("Alias")), ("base", json!("self:Local"))]),
            "add-value-type",
        )
        .unwrap();
        assert!(value_plan
            .after
            .contains("name=\"Alias\" base=\"self:Local\""));
        assert_eq!(
            value_plan.after.matches("xmlns:self=\"urn:test\"").count(),
            1
        );
        let value_model = super::super::model::PackageModel::parse(&value_plan.after).unwrap();
        assert!(super::super::validation::validate(&value_model).is_empty());

        let property_plan = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"self:Local"})),
            ]),
            "add-property",
        )
        .unwrap();

        assert!(property_plan
            .after
            .contains("name=\"Added\" type=\"self:Local\""));
        assert_eq!(
            property_plan
                .after
                .matches("xmlns:self=\"urn:test\"")
                .count(),
            1
        );
        let property_model =
            super::super::model::PackageModel::parse(&property_plan.after).unwrap();
        assert!(super::super::validation::validate(&property_model).is_empty());
    }

    #[test]
    fn xdto_writer_leaves_unbound_qnames_for_validation_to_reject() {
        let before = package(
            "\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<objectType name=\"Target\"/>\n",
        );
        let value_plan = plan(
            &before,
            &args(&[("name", json!("Alias")), ("base", json!("missing:Local"))]),
            "add-value-type",
        )
        .unwrap();
        assert!(!value_plan.after.contains("xmlns:missing="));
        let value_model = super::super::model::PackageModel::parse(&value_plan.after).unwrap();
        let value_findings = super::super::validation::validate(&value_model);
        assert_eq!(value_findings.len(), 1);
        assert_eq!(value_findings[0].code, "undeclared_prefix");

        let property_plan = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"missing:Local"})),
            ]),
            "add-property",
        )
        .unwrap();
        assert!(!property_plan.after.contains("xmlns:missing="));
        let property_model =
            super::super::model::PackageModel::parse(&property_plan.after).unwrap();
        let property_findings = super::super::validation::validate(&property_model);
        assert_eq!(property_findings.len(), 1);
        assert_eq!(property_findings[0].code, "undeclared_prefix");
    }

    #[test]
    fn xdto_writer_repeats_one_package_wide_binding_on_each_new_qname_element() {
        let root = ROOT.replace(" xmlns:tns=\"urn:test\"", "");
        let before = format!(
            "<package {root}>\n\t<property xmlns:self=\"urn:test\" name=\"Anchor\" type=\"xs:string\"/>\n\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<objectType name=\"Target\"/>\n</package>"
        );

        let value_plan = plan(
            &before,
            &args(&[("name", json!("Alias")), ("base", json!("self:Local"))]),
            "add-value-type",
        )
        .expect("one package-wide prefix identity must be reusable");
        assert!(value_plan
            .after
            .contains("<valueType xmlns:self=\"urn:test\" name=\"Alias\" base=\"self:Local\"/>"));
        let value_model = super::super::model::PackageModel::parse(&value_plan.after).unwrap();
        assert!(super::super::validation::validate(&value_model).is_empty());

        let property_plan = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                ("property", json!({"name":"Added", "type":"self:Local"})),
            ]),
            "add-property",
        )
        .expect("one package-wide prefix identity must be reusable");
        assert!(property_plan
            .after
            .contains("<property xmlns:self=\"urn:test\" name=\"Added\" type=\"self:Local\"/>"));
        let property_model =
            super::super::model::PackageModel::parse(&property_plan.after).unwrap();
        assert!(super::super::validation::validate(&property_model).is_empty());
    }

    #[test]
    fn xdto_writer_rejects_ambiguous_package_wide_prefix_identity() {
        let root = ROOT.replace(" xmlns:tns=\"urn:test\"", "");
        let before = format!(
            "<package {root}>\n\t<property xmlns:self=\"urn:first\" name=\"First\" type=\"xs:string\"/>\n\t<property xmlns:self=\"urn:second\" name=\"Second\" type=\"xs:string\"/>\n\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<objectType name=\"Target\"/>\n</package>"
        );

        for (operation, arguments) in [
            (
                "add-value-type",
                args(&[("name", json!("Alias")), ("base", json!("self:Local"))]),
            ),
            (
                "add-property",
                args(&[
                    ("typeName", json!("Target")),
                    ("property", json!({"name":"Added", "type":"self:Local"})),
                ]),
            ),
        ] {
            let planned = plan(&before, &arguments, operation).unwrap();
            assert!(!planned
                .after
                .contains("xmlns:self=\"urn:first\" name=\"Alias\""));
            assert!(!planned
                .after
                .contains("xmlns:self=\"urn:second\" name=\"Alias\""));
            let model = super::super::model::PackageModel::parse(&planned.after).unwrap();
            let findings = super::super::validation::validate(&model);
            assert!(
                findings
                    .iter()
                    .any(|finding| finding.code == "undeclared_prefix"),
                "{operation}: {findings:?}"
            );
        }
    }

    #[test]
    fn xdto_writer_compares_duplicates_against_the_proven_package_binding() {
        let root = ROOT.replace(" xmlns:tns=\"urn:test\"", "");
        let before = format!(
            "<package {root}>\n\t<property xmlns:self=\"urn:test\" name=\"Anchor\" type=\"xs:string\"/>\n\t<valueType name=\"Local\" base=\"xs:string\"/>\n\t<valueType xmlns:self=\"urn:test\" name=\"ExistingValue\" base=\"self:Local\"/>\n\t<objectType name=\"Target\">\n\t\t<property xmlns:self=\"urn:test\" name=\"ExistingProperty\" type=\"self:Local\"/>\n\t</objectType>\n</package>"
        );

        let value = plan(
            &before,
            &args(&[
                ("name", json!("ExistingValue")),
                ("base", json!("self:Local")),
            ]),
            "add-value-type",
        )
        .unwrap();
        assert_eq!(value.after, before);
        assert!(!value.conflict());

        let property = plan(
            &before,
            &args(&[
                ("typeName", json!("Target")),
                (
                    "property",
                    json!({"name":"ExistingProperty", "type":"self:Local"}),
                ),
            ]),
            "add-property",
        )
        .unwrap();
        assert_eq!(property.after, before);
        assert!(!property.conflict());
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

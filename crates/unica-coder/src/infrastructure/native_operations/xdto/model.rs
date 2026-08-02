use roxmltree::{Document, Node};
use std::ops::Range;

pub(super) const XDTO_NS: &str = "http://v8.1c.ru/8.1/xdto";
pub(super) const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
pub(super) const XML_SCHEMA_NS: &str = "http://www.w3.org/2001/XMLSchema";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SourceSpan {
    pub(super) start: usize,
    pub(super) end: usize,
}

impl From<Range<usize>> for SourceSpan {
    fn from(range: Range<usize>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct LocatedValue {
    pub(super) value: String,
    pub(super) span: SourceSpan,
}

#[derive(Clone, Debug)]
pub(super) struct QNameRef {
    pub(super) raw: String,
    pub(super) prefix: Option<String>,
    pub(super) local: String,
    pub(super) namespace: Option<String>,
    pub(super) lexical_valid: bool,
    pub(super) span: SourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TypeKind {
    Value,
    Object,
}

impl TypeKind {
    pub(super) fn element_name(self) -> &'static str {
        match self {
            Self::Value => "valueType",
            Self::Object => "objectType",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TopLevelKind {
    Import,
    Property,
    ValueType,
    ObjectType,
    Unsupported,
}

#[derive(Clone, Debug)]
pub(super) struct TopLevelNode {
    pub(super) kind: TopLevelKind,
    pub(super) key: String,
    pub(super) span: SourceSpan,
}

#[derive(Clone, Debug)]
pub(super) struct UnsupportedSyntax {
    pub(super) key: String,
    pub(super) description: String,
    pub(super) span: SourceSpan,
}

#[derive(Clone, Debug)]
pub(super) struct Import {
    pub(super) namespace: Option<LocatedValue>,
    pub(super) key: String,
    pub(super) span: SourceSpan,
}

#[derive(Clone, Debug)]
pub(super) struct Property {
    pub(super) key: String,
    pub(super) name: Option<LocatedValue>,
    pub(super) reference: Option<QNameRef>,
    pub(super) type_ref: Option<QNameRef>,
    pub(super) type_defs: Vec<AnonymousObject>,
    pub(super) lower_bound: Option<LocatedValue>,
    pub(super) upper_bound: Option<LocatedValue>,
    pub(super) span: SourceSpan,
}

impl Property {
    pub(super) fn logical_identity(&self) -> String {
        if let Some(name) = &self.name {
            return name.value.clone();
        }
        if let Some(reference) = &self.reference {
            return reference
                .namespace
                .as_deref()
                .map(|namespace| format!("{{{namespace}}}{}", reference.local))
                .unwrap_or_else(|| reference.raw.clone());
        }
        "<missing>".to_string()
    }
}

#[derive(Clone, Debug)]
pub(super) struct AnonymousObject {
    pub(super) key: String,
    pub(super) discriminator: Option<LocatedValue>,
    pub(super) properties: Vec<Property>,
    pub(super) span: SourceSpan,
}

#[derive(Clone, Debug)]
pub(super) struct NamedType {
    pub(super) kind: TypeKind,
    pub(super) key: String,
    pub(super) name: Option<LocatedValue>,
    pub(super) base: Option<QNameRef>,
    pub(super) properties: Vec<Property>,
    pub(super) span: SourceSpan,
}

#[derive(Clone, Debug)]
pub(super) struct PackageModel {
    pub(super) root_span: SourceSpan,
    pub(super) target_namespace: Option<LocatedValue>,
    pub(super) top_level: Vec<TopLevelNode>,
    pub(super) imports: Vec<Import>,
    pub(super) global_properties: Vec<Property>,
    pub(super) types: Vec<NamedType>,
    pub(super) unsupported: Vec<UnsupportedSyntax>,
}

#[derive(Clone, Debug)]
pub(super) struct ModelError {
    pub(super) code: &'static str,
    pub(super) message: String,
    pub(super) span: SourceSpan,
}

impl PackageModel {
    pub(super) fn parse(text: &str) -> Result<Self, ModelError> {
        let document = Document::parse(text).map_err(|error| ModelError {
            code: "unsupported_node",
            message: format!("invalid XDTO XML: {error}"),
            span: SourceSpan { start: 0, end: 0 },
        })?;
        let root = document.root_element();
        if !root.has_tag_name((XDTO_NS, "package")) {
            return Err(ModelError {
                code: "not_an_xdto_package",
                message: "Package.bin root must be {http://v8.1c.ru/8.1/xdto}package".to_string(),
                span: root.range().into(),
            });
        }

        let root_span = root.range().into();
        let target_namespace = attribute(root, None, "targetNamespace");
        let mut model = Self {
            root_span,
            target_namespace,
            top_level: Vec::new(),
            imports: Vec::new(),
            global_properties: Vec::new(),
            types: Vec::new(),
            unsupported: Vec::new(),
        };
        collect_unsupported_attributes(
            root,
            &[AttrName::plain("targetNamespace")],
            "$package",
            &mut model.unsupported,
        );

        for child in root.children().filter(Node::is_element) {
            let (kind, key) = if child.has_tag_name((XDTO_NS, "import")) {
                let namespace = attribute(child, None, "namespace");
                let key = format!(
                    "$package/import:{}",
                    namespace
                        .as_ref()
                        .map_or("<missing>", |value| value.value.as_str())
                );
                collect_unsupported_attributes(
                    child,
                    &[AttrName::plain("namespace")],
                    &key,
                    &mut model.unsupported,
                );
                collect_element_children_as_unsupported(child, &key, &mut model.unsupported);
                model.imports.push(Import {
                    namespace,
                    key: key.clone(),
                    span: child.range().into(),
                });
                (TopLevelKind::Import, key)
            } else if child.has_tag_name((XDTO_NS, "property")) {
                let property = parse_property(child, "$package", &mut model.unsupported);
                let key = property.key.clone();
                model.global_properties.push(property);
                (TopLevelKind::Property, key)
            } else if child.has_tag_name((XDTO_NS, "valueType")) {
                let named = parse_named_type(child, TypeKind::Value, &mut model.unsupported);
                let key = named.key.clone();
                model.types.push(named);
                (TopLevelKind::ValueType, key)
            } else if child.has_tag_name((XDTO_NS, "objectType")) {
                let named = parse_named_type(child, TypeKind::Object, &mut model.unsupported);
                let key = named.key.clone();
                model.types.push(named);
                (TopLevelKind::ObjectType, key)
            } else {
                let key = format!(
                    "$package/unsupported:{{{}}}{}",
                    child.tag_name().namespace().unwrap_or(""),
                    child.tag_name().name()
                );
                model.unsupported.push(UnsupportedSyntax {
                    key: key.clone(),
                    description: format!(
                        "unsupported package child {{{}}}{}",
                        child.tag_name().namespace().unwrap_or(""),
                        child.tag_name().name()
                    ),
                    span: child.range().into(),
                });
                (TopLevelKind::Unsupported, key)
            };
            model.top_level.push(TopLevelNode {
                kind,
                key,
                span: child.range().into(),
            });
        }
        Ok(model)
    }

    pub(super) fn target_namespace(&self) -> Option<&str> {
        self.target_namespace
            .as_ref()
            .map(|value| value.value.as_str())
    }
}

#[derive(Clone, Copy)]
struct AttrName {
    namespace: Option<&'static str>,
    local: &'static str,
}

impl AttrName {
    const fn plain(local: &'static str) -> Self {
        Self {
            namespace: None,
            local,
        }
    }

    const fn namespaced(namespace: &'static str, local: &'static str) -> Self {
        Self {
            namespace: Some(namespace),
            local,
        }
    }
}

fn parse_named_type(
    node: Node<'_, '_>,
    kind: TypeKind,
    unsupported: &mut Vec<UnsupportedSyntax>,
) -> NamedType {
    let name = attribute(node, None, "name");
    let key = format!(
        "$package/{}:{}",
        kind.element_name(),
        name.as_ref()
            .map_or("<missing>", |value| value.value.as_str())
    );
    collect_unsupported_attributes(
        node,
        &[AttrName::plain("name"), AttrName::plain("base")],
        &key,
        unsupported,
    );
    let base = qname_attribute(node, "base");
    let mut properties = Vec::new();
    for child in node.children().filter(Node::is_element) {
        if kind == TypeKind::Object && child.has_tag_name((XDTO_NS, "property")) {
            properties.push(parse_property(child, &key, unsupported));
        } else {
            unsupported.push(UnsupportedSyntax {
                key: format!("{key}/unsupported:{}", child.tag_name().name()),
                description: format!(
                    "{} cannot contain {}",
                    kind.element_name(),
                    child.tag_name().name()
                ),
                span: child.range().into(),
            });
        }
    }
    NamedType {
        kind,
        key,
        name,
        base,
        properties,
        span: node.range().into(),
    }
}

fn parse_property(
    node: Node<'_, '_>,
    parent_key: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
) -> Property {
    let name = attribute(node, None, "name");
    let reference = qname_attribute(node, "ref");
    let identity = name
        .as_ref()
        .map(|value| value.value.clone())
        .or_else(|| reference.as_ref().map(|value| value.raw.clone()))
        .unwrap_or_else(|| "<missing>".to_string());
    let key = format!("{parent_key}/property:{identity}");
    collect_unsupported_attributes(
        node,
        &[
            AttrName::plain("name"),
            AttrName::plain("ref"),
            AttrName::plain("type"),
            AttrName::plain("lowerBound"),
            AttrName::plain("upperBound"),
        ],
        &key,
        unsupported,
    );
    let mut type_defs = Vec::new();
    for child in node.children().filter(Node::is_element) {
        if child.has_tag_name((XDTO_NS, "typeDef")) {
            type_defs.push(parse_type_def(child, &key, unsupported));
        } else {
            unsupported.push(UnsupportedSyntax {
                key: format!("{key}/unsupported:{}", child.tag_name().name()),
                description: format!("property cannot contain {}", child.tag_name().name()),
                span: child.range().into(),
            });
        }
    }
    Property {
        key,
        name,
        reference,
        type_ref: qname_attribute(node, "type"),
        type_defs,
        lower_bound: attribute(node, None, "lowerBound"),
        upper_bound: attribute(node, None, "upperBound"),
        span: node.range().into(),
    }
}

fn parse_type_def(
    node: Node<'_, '_>,
    property_key: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
) -> AnonymousObject {
    let key = format!("{property_key}/typeDef");
    collect_unsupported_attributes(
        node,
        &[AttrName::namespaced(XSI_NS, "type")],
        &key,
        unsupported,
    );
    let discriminator = attribute(node, Some(XSI_NS), "type");
    let mut properties = Vec::new();
    for child in node.children().filter(Node::is_element) {
        if child.has_tag_name((XDTO_NS, "property")) {
            properties.push(parse_property(child, &key, unsupported));
        } else {
            unsupported.push(UnsupportedSyntax {
                key: format!("{key}/unsupported:{}", child.tag_name().name()),
                description: format!(
                    "typeDef:ObjectType cannot contain {}",
                    child.tag_name().name()
                ),
                span: child.range().into(),
            });
        }
    }
    AnonymousObject {
        key,
        discriminator,
        properties,
        span: node.range().into(),
    }
}

fn attribute(node: Node<'_, '_>, namespace: Option<&str>, local: &str) -> Option<LocatedValue> {
    node.attributes()
        .find(|attribute| attribute.namespace() == namespace && attribute.name() == local)
        .map(|attribute| LocatedValue {
            value: attribute.value().to_string(),
            span: attribute.range_value().into(),
        })
}

fn qname_attribute(node: Node<'_, '_>, local: &str) -> Option<QNameRef> {
    let attribute = node
        .attributes()
        .find(|attribute| attribute.namespace().is_none() && attribute.name() == local)?;
    let raw = attribute.value();
    let parts = raw.split(':').collect::<Vec<_>>();
    let (prefix, local, lexical_valid) = match parts.as_slice() {
        [prefix, local] if !prefix.is_empty() && !local.is_empty() => {
            (Some((*prefix).to_string()), (*local).to_string(), true)
        }
        [local] if !local.is_empty() => (None, (*local).to_string(), true),
        _ => (None, raw.to_string(), false),
    };
    let namespace = prefix
        .as_deref()
        .and_then(|prefix| node.lookup_namespace_uri(Some(prefix)))
        .map(str::to_string);
    Some(QNameRef {
        raw: raw.to_string(),
        prefix,
        local,
        namespace,
        lexical_valid,
        span: attribute.range_value().into(),
    })
}

fn collect_unsupported_attributes(
    node: Node<'_, '_>,
    allowed: &[AttrName],
    key: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
) {
    for attribute in node.attributes() {
        if allowed.iter().any(|allowed| {
            allowed.namespace == attribute.namespace() && allowed.local == attribute.name()
        }) {
            continue;
        }
        unsupported.push(UnsupportedSyntax {
            key: format!(
                "{key}/@{{{}}}{}",
                attribute.namespace().unwrap_or(""),
                attribute.name()
            ),
            description: format!(
                "unsupported attribute {{{}}}{}",
                attribute.namespace().unwrap_or(""),
                attribute.name()
            ),
            span: attribute.range().into(),
        });
    }
}

fn collect_element_children_as_unsupported(
    node: Node<'_, '_>,
    key: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
) {
    for child in node.children().filter(Node::is_element) {
        unsupported.push(UnsupportedSyntax {
            key: format!("{key}/unsupported:{}", child.tag_name().name()),
            description: format!("{} cannot contain elements", node.tag_name().name()),
            span: child.range().into(),
        });
    }
}

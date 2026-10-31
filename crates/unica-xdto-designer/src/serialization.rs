//! One observer for the explicitly supported serialization attributes.
use crate::model::{SourceSpan, UnsupportedSyntax};
use roxmltree::Node;
use unica_xdto::results::{
    XdtoObjectSerialization, XdtoPackageSerialization, XdtoPropertyForm, XdtoPropertySerialization,
};
pub(crate) const PACKAGE_NAMES: &[&str] = &["elementFormQualified", "attributeFormQualified"];
pub(crate) const OBJECT_NAMES: &[&str] = &["open", "mixed"];
pub(crate) const PROPERTY_NAMES: &[&str] = &["form", "nillable", "fixed", "default"];

fn foreign_limits(node: Node<'_, '_>, names: &[&str], key: &str) -> Vec<String> {
    node.attributes()
        .filter(|a| a.namespace().is_some() && names.contains(&a.name()))
        .map(|a| {
            format!(
                "{key}: serialization attribute {{{}}}{} is outside the plain-attribute profile",
                a.namespace().unwrap(),
                a.name()
            )
        })
        .collect()
}
fn invalid(
    node: Node<'_, '_>,
    name: &str,
    key: &str,
    expected: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
    limits: &mut Vec<String>,
) {
    let attribute = node
        .attributes()
        .find(|a| a.namespace().is_none() && a.name() == name)
        .expect("an invalid observed attribute exists");
    let message = format!("{key}/@{name}: expected {expected}");
    limits.push(message.clone());
    unsupported.push(UnsupportedSyntax {
        key: format!("{key}/@{{}}{name}"),
        description: message,
        span: SourceSpan::from(attribute.range()),
    });
}
fn boolean(
    node: Node<'_, '_>,
    name: &str,
    key: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
    limits: &mut Vec<String>,
) -> Option<bool> {
    let raw = node.attribute(name)?;
    // XML Schema boolean whitespace is ASCII collapse; Unicode whitespace is not normalized.
    match raw.trim_matches([' ', '\t', '\r', '\n']) {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => {
            invalid(
                node,
                name,
                key,
                "an XML boolean (true, false, 1 or 0)",
                unsupported,
                limits,
            );
            None
        }
    }
}
pub(crate) fn package(
    node: Node<'_, '_>,
    key: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
) -> XdtoPackageSerialization {
    let mut facts = XdtoPackageSerialization {
        serialization_limits: foreign_limits(node, PACKAGE_NAMES, key),
        ..Default::default()
    };
    facts.element_form_qualified = boolean(
        node,
        "elementFormQualified",
        key,
        unsupported,
        &mut facts.serialization_limits,
    );
    facts.attribute_form_qualified = boolean(
        node,
        "attributeFormQualified",
        key,
        unsupported,
        &mut facts.serialization_limits,
    );
    facts
}
pub(crate) fn object(
    node: Node<'_, '_>,
    key: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
) -> XdtoObjectSerialization {
    let mut facts = XdtoObjectSerialization {
        serialization_limits: foreign_limits(node, OBJECT_NAMES, key),
        ..Default::default()
    };
    facts.open = boolean(
        node,
        "open",
        key,
        unsupported,
        &mut facts.serialization_limits,
    );
    facts.mixed = boolean(
        node,
        "mixed",
        key,
        unsupported,
        &mut facts.serialization_limits,
    );
    facts
}
pub(crate) fn property(
    node: Node<'_, '_>,
    key: &str,
    unsupported: &mut Vec<UnsupportedSyntax>,
) -> XdtoPropertySerialization {
    let mut facts = XdtoPropertySerialization {
        serialization_limits: foreign_limits(node, PROPERTY_NAMES, key),
        ..Default::default()
    };
    facts.form = match node.attribute("form") {
        Some("Element") => Some(XdtoPropertyForm::Element),
        Some("Attribute") => Some(XdtoPropertyForm::Attribute),
        Some("Text") => Some(XdtoPropertyForm::Text),
        Some(_) => {
            invalid(
                node,
                "form",
                key,
                "Element, Attribute or Text",
                unsupported,
                &mut facts.serialization_limits,
            );
            None
        }
        None => None,
    };
    facts.nillable = boolean(
        node,
        "nillable",
        key,
        unsupported,
        &mut facts.serialization_limits,
    );
    facts.fixed = boolean(
        node,
        "fixed",
        key,
        unsupported,
        &mut facts.serialization_limits,
    );
    facts.default = node.attribute("default").map(str::to_owned);
    facts
}

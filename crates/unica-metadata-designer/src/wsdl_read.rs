//! Closed observations of a local WSDL 1.1 / SOAP 1.1 / XSD companion.
//! No URI in this model is dereferenced.
use roxmltree::{Document, Node};
use std::collections::HashMap;
use unica_metadata::wsdl::{Definition, Fact, QName, Row};
use unica_xml_format::names::xml_ncname_is_valid;

const WSDL: &str = "http://schemas.xmlsoap.org/wsdl/";
const SOAP: &str = "http://schemas.xmlsoap.org/wsdl/soap/";
const XSD: &str = "http://www.w3.org/2001/XMLSchema";
mod links;

struct Observation {
    order: usize,
    limits: Vec<String>,
}

impl Observation {
    fn problem(&mut self, field: &str, message: &str) {
        self.limits.push(format!(
            "WSDL declaration {} {field}: {message}",
            self.order
        ));
    }

    fn required_text(&mut self, node: Node<'_, '_>, field: &str) -> Option<String> {
        let value = text(node, field);
        if value.is_none() {
            self.problem(field, "required declaration is absent");
        }
        value
    }

    fn name(&mut self, node: Node<'_, '_>, required: bool) -> Option<String> {
        let token_name = (node.tag_name().namespace() == Some(WSDL)
            && matches!(
                node.tag_name().name(),
                "part" | "input" | "output" | "fault"
            ))
            || node.has_tag_name((SOAP, "fault"));
        match node
            .attribute("name")
            .map(|name| name.trim_matches([' ', '\t', '\r', '\n']))
        {
            Some(name)
                if if token_name {
                    !name.is_empty() && xml_ncname_is_valid(&format!("_{}", name.replace(':', "_")))
                } else {
                    xml_ncname_is_valid(name)
                } =>
            {
                Some(name.to_owned())
            }
            Some(_) => {
                self.problem("name", "invalid XML name token");
                None
            }
            None => {
                if required {
                    self.problem("name", "required name is absent");
                }
                None
            }
        }
    }

    fn qname(&mut self, node: Node<'_, '_>, field: &str, required: bool) -> Option<QName> {
        let Some(raw) = node.attribute(field) else {
            if required {
                self.problem(field, "required QName is absent");
            }
            return None;
        };
        let raw = raw.trim_matches([' ', '\t', '\r', '\n']);
        let (prefix, local) = match raw.split_once(':') {
            Some((prefix, local)) if xml_ncname_is_valid(prefix) && xml_ncname_is_valid(local) => {
                (Some(prefix), local)
            }
            None if xml_ncname_is_valid(raw) => (None, raw),
            _ => {
                self.problem(field, "malformed QName");
                return None;
            }
        };
        let namespace = match node.lookup_namespace_uri(prefix) {
            Some(namespace) => namespace,
            None if prefix.is_none() => "",
            None => {
                self.problem(field, "QName prefix is not declared");
                return None;
            }
        };
        Some(QName {
            namespace: namespace.to_owned(),
            local_name: local.to_owned(),
        })
    }

    fn choice(&mut self, node: Node<'_, '_>, field: &str, choices: &[&str]) -> Option<String> {
        let raw = node.attribute(field)?;
        // XSD form/use are token enums. SOAP style/use restrict xs:string,
        // whose whitespace is significant (WSDL 1.1 appendix A.4.2).
        let raw = if node.tag_name().namespace() == Some(XSD) {
            raw.trim_matches([' ', '\t', '\r', '\n'])
        } else {
            raw
        };
        if choices.contains(&raw) {
            Some(raw.to_owned())
        } else {
            self.problem(field, "value is outside the local read profile");
            None
        }
    }

    fn boolean(&mut self, node: Node<'_, '_>, field: &str) -> Option<bool> {
        match node.attribute(field)?.trim_matches([' ', '\t', '\r', '\n']) {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => {
                self.problem(field, "invalid XML boolean");
                None
            }
        }
    }

    fn occurs(&mut self, node: Node<'_, '_>, field: &str) -> Option<String> {
        let raw = node.attribute(field)?;
        let lexical = raw.trim_matches([' ', '\t', '\r', '\n']);
        let (negative, number) = match lexical.strip_prefix('-') {
            Some(number) => (true, number),
            None => (false, lexical.strip_prefix('+').unwrap_or(lexical)),
        };
        if (field == "maxOccurs" && lexical == "unbounded")
            || (!number.is_empty()
                && number.bytes().all(|byte| byte.is_ascii_digit())
                && (!negative || number.bytes().all(|byte| byte == b'0')))
        {
            Some(raw.to_owned())
        } else {
            self.problem(field, "invalid occurrence bound");
            None
        }
    }

    fn attributes(&mut self, node: Node<'_, '_>, allowed: &[&str]) {
        for attribute in node.attributes() {
            if attribute.namespace().is_some() || !allowed.contains(&attribute.name()) {
                self.problem(
                    attribute.name(),
                    "attribute is outside the local read profile",
                );
            }
        }
        if node.children().filter(Node::is_text).any(|child| {
            child
                .text()
                .is_some_and(|text| !text.trim_matches([' ', '\t', '\r', '\n']).is_empty())
        }) {
            self.problem(
                "content",
                "character content is outside the local read profile",
            );
        }
    }
}

fn text(node: Node<'_, '_>, name: &str) -> Option<String> {
    node.attribute(name).map(str::to_owned)
}

/// The caller supplies cancellation/deadline checks from the admitted read.
pub fn read<E>(
    bytes: &[u8],
    checkpoint: &mut dyn FnMut() -> Result<(), E>,
) -> Result<Definition, E> {
    checkpoint()?;
    let Ok(xml_text) = std::str::from_utf8(bytes) else {
        return Ok(Definition::unavailable("local WSDL is not UTF-8"));
    };
    let document = match Document::parse(xml_text) {
        Ok(document) => document,
        Err(error) => {
            return Ok(Definition::unavailable(format!(
                "local WSDL XML is unreadable: {error}"
            )))
        }
    };
    checkpoint()?;
    let root = document.root_element();
    if !root.has_tag_name((WSDL, "definitions")) {
        return Ok(Definition::unavailable(
            "local WSDL root must be wsdl:definitions",
        ));
    }
    let namespace = root.attribute("targetNamespace").unwrap_or("");
    let mut observation = Observation {
        order: 0,
        limits: Vec::new(),
    };
    let mut rows: Vec<Row> = Vec::new();
    let mut positions = HashMap::new();
    for (index, node) in document.descendants().filter(Node::is_element).enumerate() {
        checkpoint()?;
        observation.order = index + 1;
        let parent_index = node
            .parent()
            .and_then(|parent| positions.get(&parent.id()))
            .copied();
        if node != root && parent_index.is_none() {
            continue;
        }
        let parent = parent_index.map(|index: usize| &rows[index].fact);
        let binding_context = node
            .ancestors()
            .any(|ancestor| ancestor.has_tag_name((WSDL, "binding")));
        let (fact, attributes): (Fact, &[&str]) =
            match (node.tag_name().namespace(), node.tag_name().name(), parent) {
                (Some(WSDL), "definitions", None) => (
                    Fact::Definitions {
                        name: observation.name(node, false),
                        target_namespace: text(node, "targetNamespace"),
                    },
                    &["name", "targetNamespace"],
                ),
                (Some(WSDL), "types", Some(Fact::Definitions { .. })) => (Fact::Types {}, &[]),
                (Some(WSDL), "message", Some(Fact::Definitions { .. })) => (
                    Fact::Message {
                        name: observation.name(node, true),
                        namespace: namespace.to_owned(),
                    },
                    &["name"],
                ),
                (Some(WSDL), "part", Some(Fact::Message { .. })) => {
                    if node.attribute("element").is_some() == node.attribute("type").is_some() {
                        observation
                            .problem("part", "exactly one element or type reference is required");
                    }
                    (
                        Fact::Part {
                            name: observation.name(node, true),
                            element: observation.qname(node, "element", false),
                            r#type: observation.qname(node, "type", false),
                        },
                        &["name", "element", "type"],
                    )
                }
                (Some(WSDL), "portType", Some(Fact::Definitions { .. })) => (
                    Fact::PortType {
                        name: observation.name(node, true),
                        namespace: namespace.to_owned(),
                    },
                    &["name"],
                ),
                (Some(WSDL), "operation", Some(Fact::PortType { .. })) => (
                    Fact::Operation {
                        name: observation.name(node, true),
                        parameter_order: text(node, "parameterOrder"),
                    },
                    &["name", "parameterOrder"],
                ),
                (Some(WSDL), "operation", Some(Fact::Binding { .. })) => (
                    Fact::Operation {
                        name: observation.name(node, true),
                        parameter_order: None,
                    },
                    &["name"],
                ),
                (Some(WSDL), "input", Some(Fact::Operation { .. })) => (
                    Fact::Input {
                        name: observation.name(node, false),
                        message: observation.qname(node, "message", !binding_context),
                    },
                    if binding_context {
                        &["name"]
                    } else {
                        &["name", "message"]
                    },
                ),
                (Some(WSDL), "output", Some(Fact::Operation { .. })) => (
                    Fact::Output {
                        name: observation.name(node, false),
                        message: observation.qname(node, "message", !binding_context),
                    },
                    if binding_context {
                        &["name"]
                    } else {
                        &["name", "message"]
                    },
                ),
                (Some(WSDL), "fault", Some(Fact::Operation { .. })) => (
                    Fact::Fault {
                        name: observation.name(node, true),
                        message: observation.qname(node, "message", !binding_context),
                    },
                    if binding_context {
                        &["name"]
                    } else {
                        &["name", "message"]
                    },
                ),
                (Some(WSDL), "binding", Some(Fact::Definitions { .. })) => (
                    Fact::Binding {
                        name: observation.name(node, true),
                        namespace: namespace.to_owned(),
                        r#type: observation.qname(node, "type", true),
                    },
                    &["name", "type"],
                ),
                (Some(SOAP), "binding", Some(Fact::Binding { .. })) => (
                    Fact::SoapBinding {
                        style: observation.choice(node, "style", &["document", "rpc"]),
                        transport: text(node, "transport"),
                    },
                    &["style", "transport"],
                ),
                (Some(SOAP), "operation", Some(Fact::Operation { .. })) if binding_context => (
                    Fact::SoapOperation {
                        soap_action: text(node, "soapAction"),
                        style: observation.choice(node, "style", &["document", "rpc"]),
                    },
                    &["soapAction", "style"],
                ),
                (Some(SOAP), "body", Some(Fact::Input { .. } | Fact::Output { .. }))
                    if binding_context =>
                {
                    (
                        Fact::SoapBody {
                            r#use: observation.choice(node, "use", &["literal", "encoded"]),
                            namespace: text(node, "namespace"),
                            encoding_style: text(node, "encodingStyle"),
                            parts: text(node, "parts"),
                        },
                        &["use", "namespace", "encodingStyle", "parts"],
                    )
                }
                (Some(SOAP), "fault", Some(Fact::Fault { .. })) if binding_context => (
                    Fact::SoapFault {
                        name: observation.name(node, true),
                        r#use: observation.choice(node, "use", &["literal", "encoded"]),
                        namespace: text(node, "namespace"),
                        encoding_style: text(node, "encodingStyle"),
                    },
                    &["name", "use", "namespace", "encodingStyle"],
                ),
                (Some(WSDL), "service", Some(Fact::Definitions { .. })) => (
                    Fact::Service {
                        name: observation.name(node, true),
                        namespace: namespace.to_owned(),
                    },
                    &["name"],
                ),
                (Some(WSDL), "port", Some(Fact::Service { .. })) => (
                    Fact::Port {
                        name: observation.name(node, true),
                        binding: observation.qname(node, "binding", true),
                    },
                    &["name", "binding"],
                ),
                (Some(SOAP), "address", Some(Fact::Port { .. })) => (
                    Fact::SoapAddress {
                        location: observation.required_text(node, "location"),
                    },
                    &["location"],
                ),
                (Some(XSD), "schema", Some(Fact::Types { .. })) => (
                    Fact::Schema {
                        target_namespace: text(node, "targetNamespace"),
                        element_form_default: observation.choice(
                            node,
                            "elementFormDefault",
                            &["qualified", "unqualified"],
                        ),
                        attribute_form_default: observation.choice(
                            node,
                            "attributeFormDefault",
                            &["qualified", "unqualified"],
                        ),
                    },
                    &[
                        "targetNamespace",
                        "elementFormDefault",
                        "attributeFormDefault",
                    ],
                ),
                (Some(XSD), "complexType", Some(Fact::Schema { .. } | Fact::Element { .. })) => (
                    Fact::ComplexType {
                        name: observation.name(node, matches!(parent, Some(Fact::Schema { .. }))),
                        mixed: observation.boolean(node, "mixed"),
                        r#abstract: observation.boolean(node, "abstract"),
                    },
                    &["name", "mixed", "abstract"],
                ),
                (Some(XSD), "sequence", Some(Fact::ComplexType { .. } | Fact::Sequence { .. })) => {
                    (
                        Fact::Sequence {
                            min_occurs: observation.occurs(node, "minOccurs"),
                            max_occurs: observation.occurs(node, "maxOccurs"),
                        },
                        &["minOccurs", "maxOccurs"],
                    )
                }
                (Some(XSD), "element", Some(Fact::Schema { .. } | Fact::Sequence { .. })) => (
                    Fact::Element {
                        name: observation.name(node, node.attribute("ref").is_none()),
                        r#ref: observation.qname(node, "ref", false),
                        r#type: observation.qname(node, "type", false),
                        min_occurs: observation.occurs(node, "minOccurs"),
                        max_occurs: observation.occurs(node, "maxOccurs"),
                        nillable: observation.boolean(node, "nillable"),
                        form: observation.choice(node, "form", &["qualified", "unqualified"]),
                    },
                    &[
                        "name",
                        "ref",
                        "type",
                        "minOccurs",
                        "maxOccurs",
                        "nillable",
                        "form",
                    ],
                ),
                (Some(XSD), "attribute", Some(Fact::ComplexType { .. })) => (
                    Fact::Attribute {
                        name: observation.name(node, node.attribute("ref").is_none()),
                        r#ref: observation.qname(node, "ref", false),
                        r#type: observation.qname(node, "type", false),
                        r#use: observation.choice(
                            node,
                            "use",
                            &["optional", "prohibited", "required"],
                        ),
                        form: observation.choice(node, "form", &["qualified", "unqualified"]),
                    },
                    &["name", "ref", "type", "use", "form"],
                ),
                (Some(WSDL), "import", Some(Fact::Definitions { .. })) => {
                    observation.problem(
                        "import",
                        "external dependency is declared; no resource was loaded",
                    );
                    (
                        Fact::WsdlImport {
                            namespace: text(node, "namespace"),
                            location: text(node, "location"),
                        },
                        &["namespace", "location"],
                    )
                }
                (Some(XSD), "import", Some(Fact::Schema { .. })) => {
                    observation.problem(
                        "import",
                        "external dependency is declared; no resource was loaded",
                    );
                    (
                        Fact::SchemaImport {
                            namespace: text(node, "namespace"),
                            schema_location: text(node, "schemaLocation"),
                        },
                        &["namespace", "schemaLocation"],
                    )
                }
                (Some(XSD), "include", Some(Fact::Schema { .. })) => {
                    observation.problem(
                        "include",
                        "external dependency is declared; no resource was loaded",
                    );
                    (
                        Fact::SchemaInclude {
                            schema_location: text(node, "schemaLocation"),
                        },
                        &["schemaLocation"],
                    )
                }
                _ => {
                    observation.problem(
                        "construct",
                        "element or its context is outside the local read profile",
                    );
                    continue;
                }
            };
        observation.attributes(node, attributes);
        positions.insert(node.id(), rows.len());
        rows.push(Row {
            source_order: index + 1,
            parent_order: parent_index.map(|index| rows[index].source_order),
            fact,
        });
    }
    checkpoint()?;
    let mut definition = Definition {
        rows,
        limits: observation.limits,
    };
    links::validate(&mut definition, checkpoint)?;
    Ok(definition)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsdl_read_keeps_cancellation_during_declaration_walk() {
        let xml = format!(
            "<definitions xmlns=\"{WSDL}\">{}</definitions>",
            "<message name=\"M\"/>".repeat(100)
        );
        let mut checkpoints = 0;
        let result = read(xml.as_bytes(), &mut || {
            checkpoints += 1;
            if checkpoints == 25 {
                Err("cancelled")
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err(), "cancelled");
        assert_eq!(checkpoints, 25);
    }

    #[test]
    fn wsdl_read_rejects_dtd_without_loading_external_resources() {
        let xml = format!("<!DOCTYPE definitions SYSTEM 'https://example.invalid/must-not-load'><definitions xmlns=\"{WSDL}\"/>");
        let result = read(xml.as_bytes(), &mut || Ok::<_, ()>(())).unwrap();
        assert!(result.rows.is_empty());
        assert!(!result.limits.is_empty());
    }

    #[test]
    fn wsdl_observed_bounds_keep_literals_and_boolean_xml_whitespace() {
        for bound in ["+000", "-000", "000", " 0 "] {
            let xml = format!("<definitions xmlns=\"{WSDL}\" xmlns:xs=\"{XSD}\"><types><xs:schema><xs:element name=\"Value\" minOccurs=\"{bound}\" nillable=\" false \"/></xs:schema></types></definitions>");
            let result = read(xml.as_bytes(), &mut || Ok::<_, ()>(())).unwrap();
            assert!(result.limits.is_empty(), "{:?}", result.limits);
            assert!(
                matches!(&result.rows.last().unwrap().fact, Fact::Element { min_occurs: Some(value), nillable: Some(false), .. } if value == bound)
            );
        }
    }
}

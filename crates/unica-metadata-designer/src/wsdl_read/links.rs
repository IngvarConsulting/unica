use super::{Definition, Fact, QName, Row, XSD};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum Target {
    Message,
    PortType,
    Binding,
    Type,
    Element,
    Attribute,
}

fn schema_namespace(row: &Row, rows: &HashMap<usize, &Row>) -> Option<String> {
    let mut parent = row.parent_order;
    while let Some(order) = parent {
        let row = rows.get(&order)?;
        if let Fact::Schema {
            target_namespace, ..
        } = &row.fact
        {
            return Some(target_namespace.clone().unwrap_or_default());
        }
        parent = row.parent_order;
    }
    None
}

// XML Schema 1.0 built-in types; this does not validate instances or fetch schemas.
fn builtin(name: &QName) -> bool {
    name.namespace == XSD
        && matches!(
            name.local_name.as_str(),
            "anyType"
                | "anySimpleType"
                | "string"
                | "boolean"
                | "decimal"
                | "float"
                | "double"
                | "duration"
                | "dateTime"
                | "time"
                | "date"
                | "gYearMonth"
                | "gYear"
                | "gMonthDay"
                | "gDay"
                | "gMonth"
                | "hexBinary"
                | "base64Binary"
                | "anyURI"
                | "QName"
                | "NOTATION"
                | "normalizedString"
                | "token"
                | "language"
                | "NMTOKEN"
                | "NMTOKENS"
                | "Name"
                | "NCName"
                | "ID"
                | "IDREF"
                | "IDREFS"
                | "ENTITY"
                | "ENTITIES"
                | "integer"
                | "nonPositiveInteger"
                | "negativeInteger"
                | "long"
                | "int"
                | "short"
                | "byte"
                | "nonNegativeInteger"
                | "unsignedLong"
                | "unsignedInt"
                | "unsignedShort"
                | "unsignedByte"
                | "positiveInteger"
        )
}

pub(super) fn validate<E>(
    definition: &mut Definition,
    checkpoint: &mut dyn FnMut() -> Result<(), E>,
) -> Result<(), E> {
    let rows = definition
        .rows
        .iter()
        .map(|row| (row.source_order, row))
        .collect::<HashMap<_, _>>();
    let mut targets = HashMap::<(Target, QName), usize>::new();
    let mut local_names = std::collections::HashSet::new();
    for row in &definition.rows {
        checkpoint()?;
        let local_name = match &row.fact {
            Fact::Part {
                name: Some(name), ..
            } => Some(("part", name)),
            Fact::Port {
                name: Some(name), ..
            } => Some(("port", name)),
            _ => None,
        };
        if let Some((kind, name)) = local_name {
            if !local_names.insert((row.parent_order, kind, name)) {
                definition.limits.push(format!(
                    "WSDL declaration {}: duplicate local {kind} identity is ambiguous",
                    row.source_order
                ));
            }
        }
        let identity = match &row.fact {
            Fact::Message {
                name: Some(name),
                namespace,
            } => Some((Target::Message, namespace.clone(), name)),
            Fact::PortType {
                name: Some(name),
                namespace,
            } => Some((Target::PortType, namespace.clone(), name)),
            Fact::Binding {
                name: Some(name),
                namespace,
                ..
            } => Some((Target::Binding, namespace.clone(), name)),
            Fact::ComplexType {
                name: Some(name), ..
            } if row
                .parent_order
                .and_then(|order| rows.get(&order))
                .is_some_and(|parent| matches!(parent.fact, Fact::Schema { .. })) =>
            {
                schema_namespace(row, &rows).map(|namespace| (Target::Type, namespace, name))
            }
            Fact::Element {
                name: Some(name), ..
            } if row
                .parent_order
                .and_then(|order| rows.get(&order))
                .is_some_and(|parent| matches!(parent.fact, Fact::Schema { .. })) =>
            {
                schema_namespace(row, &rows).map(|namespace| (Target::Element, namespace, name))
            }
            _ => None,
        };
        if let Some((kind, namespace, name)) = identity {
            let count = targets
                .entry((
                    kind,
                    QName {
                        namespace,
                        local_name: name.clone(),
                    },
                ))
                .or_default();
            *count += 1;
            if *count > 1 {
                definition.limits.push(format!(
                    "WSDL declaration {}: duplicate global {kind:?} identity is ambiguous",
                    row.source_order
                ));
            }
        }
    }
    for row in &definition.rows {
        checkpoint()?;
        let mut references = Vec::new();
        match &row.fact {
            Fact::Part {
                element, r#type, ..
            } => {
                references.extend(
                    element
                        .as_ref()
                        .map(|name| ("element", Target::Element, name)),
                );
                references.extend(r#type.as_ref().map(|name| ("type", Target::Type, name)));
            }
            Fact::Input { message, .. }
            | Fact::Output { message, .. }
            | Fact::Fault { message, .. } => references.extend(
                message
                    .as_ref()
                    .map(|name| ("message", Target::Message, name)),
            ),
            Fact::Binding { r#type, .. } => {
                references.extend(r#type.as_ref().map(|name| ("type", Target::PortType, name)))
            }
            Fact::Port { binding, .. } => references.extend(
                binding
                    .as_ref()
                    .map(|name| ("binding", Target::Binding, name)),
            ),
            Fact::Element { r#ref, r#type, .. } => {
                references.extend(r#ref.as_ref().map(|name| ("ref", Target::Element, name)));
                references.extend(r#type.as_ref().map(|name| ("type", Target::Type, name)));
            }
            Fact::Attribute { r#ref, r#type, .. } => {
                references.extend(r#ref.as_ref().map(|name| ("ref", Target::Attribute, name)));
                references.extend(r#type.as_ref().map(|name| ("type", Target::Type, name)));
            }
            _ => {}
        }
        for (field, kind, name) in references {
            if kind == Target::Type && builtin(name) {
                continue;
            }
            if targets.get(&(kind, name.clone())) != Some(&1) {
                definition.limits.push(format!("WSDL declaration {} {field}: QName target is missing, ambiguous, or outside the local document",row.source_order));
            }
        }
    }
    Ok(())
}

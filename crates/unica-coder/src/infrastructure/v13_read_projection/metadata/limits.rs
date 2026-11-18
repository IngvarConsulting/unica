use super::{metadata_collection, metadata_missing_relation_limits};
use serde_json::Value;
use unica_address::address::{NodeKind, QualifiedAddress};

pub(super) fn metadata_limits(payload: &Value) -> Vec<String> {
    let mut limits = unreadable_limits(payload);
    limits.extend(metadata_missing_relation_limits(payload));
    limits.extend(wsdl_limits(payload));
    limits
}

pub(super) fn wsdl_limits(payload: &Value) -> Vec<String> {
    payload
        .pointer("/wsdlDefinition/limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Поля, которые читатель не смог спроецировать, — строками для `limits`.
fn unreadable_limits(payload: &Value) -> Vec<String> {
    unreadable_entries(payload)
        .map(|(field, message)| format!("{field} не прочитано: {message}"))
        .collect()
}

/// Причина, по которой коллекция из пофактовой части вида осталась без данных.
///
/// Без неё `URLTemplate` сервиса, чей шаблон читатель не разобрал, отвечал бы
/// «нет такой коллекции», и агент счёл бы сервис пустым.
pub(super) fn unreadable_collection(payload: &Value, kind: NodeKind) -> Option<String> {
    let (prefix, property) = match kind {
        NodeKind::AddressingAttribute => ("collections.addressingAttributes", None),
        NodeKind::PredefinedItem => ("predefinedItems", None),
        NodeKind::UrlTemplate => ("details.urlTemplates", None),
        NodeKind::Operation => ("details.operations", None),
        NodeKind::StandardAttribute => {
            ("standardAttributes", Some("properties.StandardAttributes"))
        }
        NodeKind::Characteristic => ("characteristics", Some("properties.Characteristics")),
        NodeKind::StandardTabularSection => (
            "standardTabularSections",
            Some("properties.StandardTabularSections"),
        ),
        _ => return None,
    };
    // У применимой декларации есть ключ, даже если её состав не прочитан.
    // Чужое одноимённое свойство не делает декларацию применимой к объекту.
    if property.is_some() && metadata_collection(payload, kind).is_none() {
        return None;
    }
    let reasons = unreadable_entries(payload)
        .filter(|(field, _)| {
            property == Some(*field)
                || field
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
        })
        .map(|(field, message)| format!("{field}: {message}"))
        .collect::<Vec<_>>();
    (!reasons.is_empty()).then(|| reasons.join("; "))
}

pub(super) fn task_addressing_applies(address: &QualifiedAddress, payload: &Value) -> bool {
    address
        .segments()
        .first()
        .is_some_and(|owner| owner.kind() == NodeKind::Task && owner.name().is_some())
        && payload.get("kind").and_then(Value::as_str) == Some("Task")
}

// The optional native collection distinguishes an unreadable/absent container
// from a proved empty collection. Never run the generic empty fallback for it.
pub(super) fn task_addressing_issue(payload: &Value) -> Option<String> {
    if let Some(reason) = unreadable_collection(payload, NodeKind::AddressingAttribute) {
        return Some(format!(
            "metadata AddressingAttribute collection is unreadable: {reason}"
        ));
    }
    if metadata_collection(payload, NodeKind::AddressingAttribute)
        .is_none_or(|value| !value.is_array())
    {
        return Some("collections.addressingAttributes is unknown: Task ChildObjects is absent or unreadable".into());
    }
    None
}

pub(super) fn unreadable_entries(payload: &Value) -> impl Iterator<Item = (&str, &str)> {
    payload
        .get("unreadable")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry.get("field")?.as_str()?,
                entry.get("message")?.as_str()?,
            ))
        })
}

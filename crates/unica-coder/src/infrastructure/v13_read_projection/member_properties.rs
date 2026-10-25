use super::place_prop;
use serde_json::{Map, Value};
use unica_metadata::{
    MetaMemberReadContext, MetaMemberReadProfile, MetaObservedProperty, MetaObservedPropertyValue,
};

/// Both the XML observer and this transfer consume the same closed profile.
/// A serialized provider property is not, by itself, permission to publish it.
pub(super) fn project(
    profile: MetaMemberReadProfile,
    context: MetaMemberReadContext,
    item: &Value,
) -> (Map<String, Value>, Vec<String>) {
    let observations = item.get("properties").and_then(Value::as_array);
    let mut props = Map::new();
    let mut limits = item
        .get("propertyLimits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for spec in profile.properties(context) {
        let matching = observations
            .into_iter()
            .flatten()
            .filter(|value| value.get("name").and_then(Value::as_str) == Some(spec.name))
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }
        let value = match matching.as_slice() {
            [observed] => serde_json::from_value::<MetaObservedProperty>((*observed).clone())
                .ok()
                .filter(|observed| spec.accepts(&observed.value))
                .map(|observed| observed.value),
            _ => None,
        };
        let value = match value {
            Some(MetaObservedPropertyValue::UnsignedInteger { value }) => Some(Value::from(value)),
            Some(MetaObservedPropertyValue::Text { value }) => Some(Value::String(value)),
            // Пустой набор переводов — значения нет, свойство не публикуется.
            Some(MetaObservedPropertyValue::LocalizedString { values }) if values.is_empty() => {
                continue;
            }
            // Обёртку снимает `place_prop`: публикуется массив переводов.
            Some(value @ MetaObservedPropertyValue::LocalizedString { .. }) => {
                serde_json::to_value(value).ok()
            }
            _ => None,
        };
        match value {
            Some(value) => place_prop(&mut props, &mut limits, spec.name, &value, true, spec.name),
            None => limits.push(format!(
                "{} has an invalid or duplicated observed value",
                spec.name
            )),
        }
    }
    (props, limits)
}

#[cfg(test)]
mod tests;

use super::{place_prop, safe_prop};
use serde_json::{json, Map, Value};
use unica_address::address::QualifiedAddress;

/// Пофактовая часть вида раскладывается в `props` по ролям.
///
/// Составное значение из двух-трёх полей не помещается в `props` объектом и
/// не заслуживает ветви: адресовать внутри него нечего. Роль каждого поля
/// называет ключ, и родительный падеж уходит в имя: `handlerModule`, а не
/// `method.metadataPath`.
pub(super) fn metadata_detail_props(
    address: &QualifiedAddress,
    payload: &Value,
    limits: &mut Vec<String>,
) -> Map<String, Value> {
    let mut props = Map::new();
    let Some(details) = payload
        .get("details")
        .and_then(|adjacent| adjacent.get("details"))
    else {
        return props;
    };
    for (source, roles) in [
        (
            "method",
            &[
                ("metadataPath", "handlerModule"),
                ("method", "handlerMethod"),
            ][..],
        ),
        (
            "schedule",
            &[
                ("register", "scheduleRegister"),
                ("valueField", "scheduleValueField"),
                ("dateField", "scheduleDateField"),
            ][..],
        ),
    ] {
        let Some(composite) = details.get(source) else {
            continue;
        };
        for (field, role) in roles {
            if let Some(value) = composite.get(field) {
                let subject = format!("metadata {role}");
                place_prop(&mut props, limits, address, role, value, false, &subject);
            }
        }
    }
    // Тип корня — свойство платформы `Type`, под тем же именем его
    // принимает запись.
    if let Some(value) = details.get("type") {
        place_prop(
            &mut props,
            limits,
            address,
            "Type",
            value,
            false,
            "metadata type",
        );
    }
    props
}

pub(super) fn metadata_property_props(
    address: &QualifiedAddress,
    payload: &Value,
    limits: &mut Vec<String>,
) -> Map<String, Value> {
    let mut props = Map::new();
    for property in payload
        .get("properties")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(key), Some(value)) = (
            property.get("key").and_then(Value::as_str),
            property.get("value"),
        ) else {
            continue;
        };
        let subject = format!("metadata property {key}");
        place_prop(&mut props, limits, address, key, value, true, &subject);
    }
    props
}

pub(crate) fn borrowing_props(
    borrowing: &unica_metadata::borrowing::MetaBorrowing,
) -> Map<String, Value> {
    let mut props = Map::new();
    props.insert(
        "belonging".into(),
        json!(if borrowing.extends.is_some() {
            "borrowed"
        } else {
            "own"
        }),
    );
    if let Some(parent_id) = &borrowing.extends {
        if safe_prop("parentId", &json!(parent_id)) {
            props.insert("parentId".into(), json!(parent_id));
        }
        props.insert("parentStatus".into(), json!("unavailable"));
        let overrides = borrowing.overrides.join(", ");
        let complete = safe_prop("overrides", &json!(overrides));
        props.insert("overridesCount".into(), json!(borrowing.overrides.len()));
        props.insert("overridesComplete".into(), json!(complete));
        if complete && !overrides.is_empty() {
            props.insert("overrides".into(), json!(overrides));
        }
    }
    props
}

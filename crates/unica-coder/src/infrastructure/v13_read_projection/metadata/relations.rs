use super::{metadata_details, unreadable_entries};
use serde_json::{json, Value};
use unica_address::address::{AddressSegment, QualifiedAddress};

const METADATA_DETAIL_RELATIONS: &[(&str, &str, &str)] = &[
    (
        "registeredDocuments",
        "registeredDocument",
        "RegisteredDocuments",
    ),
    (
        "baseCalculationTypes",
        "baseCalculationType",
        "BaseCalculationTypes",
    ),
    ("xdtoPackages", "xdtoPackage", "XDTOPackages"),
];

fn metadata_relation_error_field(field: &str) -> Option<String> {
    if field.starts_with("relations.") {
        return Some(field.to_string());
    }
    for (name, _, xml_name) in METADATA_DETAIL_RELATIONS {
        let detail = format!("details.{name}");
        if field == format!("properties.{xml_name}") {
            return Some(detail);
        }
        if field
            .strip_prefix(&detail)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
        {
            return Some(field.to_string());
        }
    }
    None
}

pub(super) fn metadata_relation_limits(payload: &Value) -> Vec<String> {
    let mut limits = unreadable_entries(payload)
        .filter_map(|(field, message)| {
            metadata_relation_error_field(field)
                .map(|field| format!("{field} не прочитано: {message}"))
        })
        .collect::<Vec<_>>();
    limits.extend(metadata_missing_relation_limits(payload));
    limits
}

pub(super) fn metadata_missing_relation_limits(payload: &Value) -> Vec<String> {
    let mut missing = payload
        .get("relations")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, value)| value.is_null())
        .map(|(name, _)| format!("relations.{name}"))
        .collect::<Vec<_>>();
    if let Some(details) = metadata_details(payload) {
        missing.extend(
            METADATA_DETAIL_RELATIONS
                .iter()
                .filter(|(name, _, _)| details.get(*name).is_some_and(Value::is_null))
                .map(|(name, _, _)| format!("details.{name}")),
        );
    }
    missing
        .into_iter()
        .filter_map(|field| {
            let damaged = unreadable_entries(payload).any(|(candidate, _)| {
                metadata_relation_error_field(candidate).is_some_and(|candidate| {
                    candidate
                        .strip_prefix(&field)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
                })
            });
            (!damaged).then(|| format!("{field} is absent; the relation list may be incomplete"))
        })
        .collect()
}

/// Все ссылки объекта наружу — одной ветвью.
///
/// Платформа держит их в разных местах: владельцы и движения — в `relations`,
/// зарегистрированные документы журнала и базовые виды расчёта — в пофактовой
/// части вида. Вопрос при этом один: на что этот объект показывает и почему.
/// Поэтому имя связи становится полем элемента, а не отдельной ветвью на
/// каждое имя: девять ветвей ради девяти имён — это девять новых видов узлов
/// там, где хватает поля.
pub(super) fn metadata_relations(address: &QualifiedAddress, payload: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    let addressed = |relation: &str, target: &str| {
        let at = QualifiedAddress::parse(&format!("{}:{target}", address.source_set())).ok()?;
        let kind = at.segments().last().map(AddressSegment::kind)?;
        Some(json!({
            "relation": relation,
            "at": at.to_string(),
            "kind": kind.as_str(),
        }))
    };
    let relations = payload.get("relations");
    for (field, relation) in [
        ("owners", "owner"),
        ("registerRecords", "registerRecord"),
        ("basedOn", "basedOn"),
        ("inputByString", "inputByString"),
        ("dataLockFields", "dataLockField"),
    ] {
        let targets = relations
            .and_then(|relations| relations.get(field))
            .and_then(Value::as_array);
        for target in targets.into_iter().flatten() {
            if let Some(value) = target.get("value").and_then(Value::as_str) {
                items.extend(addressed(relation, value));
            }
        }
    }
    // Источник события адресует объект, но один адрес даёт разные классы:
    // объект и менеджер справочника, менеджер константы и менеджер её
    // значения. Элемент поэтому несёт `target` — ту же логическую цель, что
    // принимают `targets` записи, и чтение возвращает записанный выбор.
    // Семейство адреса не имеет и показывается видом `family`, как `typeId`.
    let sources = relations
        .and_then(|relations| relations.get("source"))
        .and_then(Value::as_array);
    for source in sources.into_iter().flatten() {
        let item = match source.get("metadataPath").and_then(Value::as_str) {
            Some(target) => addressed("source", target),
            None => source
                .get("kind")
                .and_then(Value::as_str)
                .map(|kind| json!({"relation": "source", "kind": kind})),
        };
        if let Some(mut item) = item {
            item["target"] = source.clone();
            items.push(item);
        }
    }
    // Пакеты XDTO названы платформой по-своему, но показывают ровно наружу
    // и ложатся сюда же. Пространство имён пакета сюда не попадает: ветвь
    // адресует объекты, а не строки.
    let details = metadata_details(payload);
    for (field, relation, _) in METADATA_DETAIL_RELATIONS {
        let entries = details
            .and_then(|details| details.get(*field))
            .and_then(Value::as_array);
        for entry in entries.into_iter().flatten() {
            let target = entry
                .as_str()
                .or_else(|| entry.get("metadataPath").and_then(Value::as_str));
            if let Some(target) = target {
                items.extend(addressed(relation, target));
            }
        }
    }
    for item in relations
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(target) = item.get("target").and_then(Value::as_str) else {
            continue;
        };
        let Ok(at) = QualifiedAddress::parse(&format!("{}:{target}", address.source_set())) else {
            continue;
        };
        let Some(kind) = at.segments().last().map(AddressSegment::kind) else {
            continue;
        };
        items.push(json!({ "relation": "content", "at": at.to_string(), "kind": kind.as_str(), "autoRecord": item.get("autoRecord") }));
    }
    // Источник подписки расширения из основной конфигурации платформа пишет
    // идентификатором типа (`v8:TypeId`, #1361): адреса у него нет, но
    // скрыть его нельзя — замена источников сохраняет его как есть.
    let type_ids = relations
        .and_then(|relations| relations.get("sourceTypeIds"))
        .and_then(Value::as_array);
    for type_id in type_ids.into_iter().flatten().filter_map(Value::as_str) {
        items.push(json!({
            "relation": "source",
            "kind": "typeId",
            "typeId": type_id,
            "readOnly": true,
        }));
    }
    // Подсистемы, в состав которых входит объект: `direct` у той, что
    // перечисляет его, и не у её родителей. Корень библиотеки — признак
    // соглашения стандартов разработки, а не платформы, поэтому элемент называет его источник
    // и модуль, на котором он держится.
    let subsystem_at = |name: &str| {
        format!(
            "{}:Subsystem.{}",
            address.source_set(),
            name.split('.').collect::<Vec<_>>().join(".Subsystem.")
        )
    };
    for membership in payload
        .get("subsystemMemberships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(name) = membership.get("subsystem").and_then(Value::as_str) {
            items.push(json!({
                "relation": "subsystem",
                "at": subsystem_at(name),
                "kind": "Subsystem",
                "direct": membership.get("direct"),
            }));
        }
    }
    for root in payload
        .get("libraryRoots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(module)) = (
            root.get("subsystem").and_then(Value::as_str),
            root.get("module").and_then(Value::as_str),
        ) else {
            continue;
        };
        items.push(json!({
            "relation": "library",
            "at": subsystem_at(name),
            "kind": "Subsystem",
            "source": "standardLibraryModule",
            "module": format!("{}:{module}", address.source_set()),
        }));
    }
    items
}

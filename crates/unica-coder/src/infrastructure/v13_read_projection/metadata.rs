use super::{
    bounded_prop, member_properties, safe_prop, select_named, selected_scalar_props,
    MAX_COMPACT_PROP_BYTES,
};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use unica_execution_contract::view_error::ViewError;

use serde_json::{json, Map, Value};
use unica_address::address::{AddressSegment, NodeKind, QualifiedAddress};
use unica_refusal::{RefusalCode, RefusalDetail};

pub(super) const PAYLOAD_KEYS: &[&str] = &[
    "name",
    "synonym",
    "kind",
    "details",
    "support",
    "properties",
    "declarations",
    "relations",
    "collections",
    "predefinedItems",
    "memberReadProfile",
    // Заимствование расширением: только в наборе вида `extension`.
    "belonging",
    "parentId",
    "parentStatus",
    "overridesCount",
    "overridesComplete",
    "overrides",
    // Поля, которые читатель не смог спроецировать (см. `metadata_payload`).
    "unreadable",
];

pub(super) fn project_metadata_for_source(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
    source_kind: Option<unica_source_contract::SourceSetKind>,
) -> Result<NodeViewData, ViewError> {
    let profile = source_kind
        .filter(|kind| *kind == unica_source_contract::SourceSetKind::Configuration)
        .filter(|_| {
            address
                .segments()
                .first()
                .is_some_and(|part| part.kind() == NodeKind::Catalog)
        })
        .filter(|_| payload.get("kind").and_then(Value::as_str) == Some("Catalog"))
        .and_then(|_| payload.get("memberReadProfile"))
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    project_metadata(address, payload, suffix, profile)
}

pub(super) fn project_form_metadata(
    address: &QualifiedAddress,
    payload: &Value,
) -> Result<NodeViewData, ViewError> {
    let props = payload
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| {
            ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "form metadata properties are not an object",
            )
        })?;
    let title = props
        .get("Name")
        .and_then(Value::as_str)
        .unwrap_or("Metadata")
        .to_string();
    Ok(NodeViewData::Node(
        NodeView::new(address.to_string(), "Metadata", title, props)
            .with_limits(metadata_limits(payload)),
    ))
}

fn project_metadata(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
    member_profile: Option<unica_metadata::MetaMemberReadProfile>,
) -> Result<NodeViewData, ViewError> {
    if suffix.is_empty() {
        let mut props = selected_scalar_props(
            payload,
            &[
                "kind",
                "synonym",
                "support",
                // Заимствование расширением: ключи есть только в наборе
                // расширения и только у заимствованного объекта, кроме
                // `belonging` — он отвечает у всякого объекта расширения.
                "belonging",
                "parentId",
                "parentStatus",
                "overridesCount",
                "overridesComplete",
                "overrides",
            ],
        );
        props.extend(metadata_property_props(payload));
        props.extend(metadata_detail_props(payload));
        // Счёт предопределённых элементов берётся из ответа читателя, а не из
        // длины страницы: при урезанной странице они расходятся, и ветвь
        // обещала бы меньше, чем есть.
        let predefined_total = payload
            .get("predefinedItems")
            .and_then(|predefined| predefined.get("total"))
            .and_then(Value::as_u64);
        let mut branches = metadata_branch_kinds()
            .iter()
            .filter_map(|kind| {
                let value = metadata_collection(payload, *kind)?;
                let count = match (kind, predefined_total) {
                    (NodeKind::PredefinedItem, Some(total)) => {
                        usize::try_from(total).unwrap_or(usize::MAX)
                    }
                    _ => value.as_array().map_or(0, Vec::len),
                };
                (count > 0).then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
            })
            .collect::<Vec<_>>();
        let relations = metadata_relations(address, payload);
        if matches!(address.segments(), [owner] if owner.kind() == NodeKind::ScheduledJob) {
            branches.push(BranchRef::new(format!("{address}.Schedule"), 1));
        }
        if !relations.is_empty() {
            branches.push(BranchRef::new(
                format!("{}.{}", address, NodeKind::Relation.as_str()),
                relations.len(),
            ));
        }
        let title = payload
            .get("synonym")
            .and_then(Value::as_str)
            .or_else(|| payload.get("name").and_then(Value::as_str))
            .unwrap_or_else(|| {
                address
                    .segments()
                    .last()
                    .and_then(AddressSegment::name)
                    .unwrap_or("Metadata")
            });
        return Ok(NodeViewData::Node(
            NodeView::new(
                address.to_string(),
                address
                    .segments()
                    .last()
                    .map(AddressSegment::kind)
                    .unwrap_or(NodeKind::Configuration)
                    .as_str(),
                title,
                props,
            )
            .with_branches(branches)
            .with_limits(metadata_limits(payload)),
        ));
    }
    let first = &suffix[0];
    if matches!(address.segments(), [owner, schedule]
        if owner.kind() == NodeKind::ScheduledJob && schedule.kind() == NodeKind::Schedule)
    {
        let properties = payload
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "schedule reader returned a non-object payload",
                )
            })?;
        let props = selected_scalar_props(
            &Value::Object(properties.clone()),
            &[
                "state",
                "BeginDate",
                "EndDate",
                "BeginTime",
                "EndTime",
                "CompletionTime",
                "CompletionInterval",
                "RepeatPeriodInDay",
                "RepeatPause",
                "WeekDayInMonth",
                "DayInMonth",
                "WeeksPeriod",
                "DaysRepeatPeriod",
                "WeekDays",
                "Months",
            ],
        );
        return Ok(NodeViewData::Node(NodeView::new(
            address.to_string(),
            "Schedule",
            "Schedule",
            props,
        )));
    }
    if first.kind() == NodeKind::Relation {
        if suffix.len() > 1 || first.name().is_some() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "the relation branch lists targets; read the target at its own address",
            ));
        }
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(
                address.to_string(),
                NodeKind::Relation.as_str(),
                "Relations",
                Map::new(),
            )
            .with_limits(metadata_relation_limits(payload)),
            metadata_relations(address, payload),
        )));
    }
    // Непрочитанная коллекция приходит `null`; без этой проверки она читалась
    // бы как пустая.
    if metadata_collection(payload, first.kind()).is_none_or(Value::is_null) {
        if let Some(unread) = unreadable_collection(payload, first.kind()) {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                format!(
                    "metadata {} collection is unreadable: {unread}",
                    first.kind().as_str()
                ),
            ));
        }
    }
    let collection = metadata_collection(payload, first.kind()).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("metadata has no {} collection", first.kind().as_str()),
        )
    })?;
    if first.kind() == NodeKind::Characteristic {
        // Прикладного имени у характеристики нет: она названа парой источников,
        // а не словом. Адресовать в ней нечего, поэтому элементы остаются
        // строками данных — как строки макета и строки исходника.
        if first.name().is_some() || suffix.len() > 1 {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "a characteristic carries no applied name; the branch lists them as rows",
            ));
        }
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(
                address.to_string(),
                NodeKind::Characteristic.as_str(),
                "Characteristics",
                Map::new(),
            ),
            collection.as_array().cloned().unwrap_or_default(),
        )));
    }
    if first.name().is_none() && suffix.len() == 1 {
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(
                address.to_string(),
                first.kind().as_str(),
                first.kind().as_str(),
                Map::new(),
            ),
            metadata_items(address, first.kind(), collection, payload, member_profile),
        )));
    }
    let name = first.name().ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            "metadata collection name is required",
        )
    })?;
    let mut item = select_named(collection, name).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("metadata node `{name}` was not found"),
        )
    })?;
    let mut kind = first.kind();
    for segment in &suffix[1..] {
        if kind == NodeKind::TabularSection
            && segment.kind() == NodeKind::StandardAttribute
            && item.get("standardAttributes").is_some_and(Value::is_null)
        {
            let reasons = tabular_standard_attribute_limits(payload, item);
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                format!(
                    "tabular section StandardAttributes is unreadable: {}",
                    reasons.join("; ")
                ),
            ));
        }
        kind = segment.kind();
        let next = match kind {
            NodeKind::Attribute | NodeKind::Column => item.get("attributes"),
            NodeKind::Method => item.get("methods"),
            NodeKind::Parameter => item.get("parameters"),
            NodeKind::StandardAttribute => item.get("standardAttributes"),
            _ => None,
        }
        .ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                "metadata child collection was not found",
            )
        })?;
        if let Some(name) = segment.name() {
            item = select_named(next, name).ok_or_else(|| {
                ViewError::new(
                    RefusalCode::NotFound,
                    format!("metadata child `{name}` was not found"),
                )
            })?;
        } else {
            return Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(
                    address.to_string(),
                    kind.as_str(),
                    kind.as_str(),
                    Map::new(),
                ),
                metadata_items(address, kind, next, payload, member_profile),
            )));
        }
    }
    Ok(NodeViewData::Node(metadata_item_with_context(
        address,
        kind,
        item,
        payload,
        member_profile,
    )))
}

fn metadata_limits(payload: &Value) -> Vec<String> {
    let mut limits = unreadable_limits(payload);
    limits.extend(metadata_missing_relation_limits(payload));
    limits
}

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

fn metadata_relation_limits(payload: &Value) -> Vec<String> {
    let mut limits = unreadable_entries(payload)
        .filter_map(|(field, message)| {
            metadata_relation_error_field(field)
                .map(|field| format!("{field} не прочитано: {message}"))
        })
        .collect::<Vec<_>>();
    limits.extend(metadata_missing_relation_limits(payload));
    limits
}

fn metadata_missing_relation_limits(payload: &Value) -> Vec<String> {
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
fn unreadable_collection(payload: &Value, kind: NodeKind) -> Option<String> {
    let (prefix, property) = match kind {
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

fn unreadable_entries(payload: &Value) -> impl Iterator<Item = (&str, &str)> {
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

fn metadata_branch_kinds() -> &'static [NodeKind] {
    &[
        NodeKind::Attribute,
        NodeKind::StandardAttribute,
        NodeKind::TabularSection,
        NodeKind::Dimension,
        NodeKind::Resource,
        NodeKind::Recalculation,
        NodeKind::EnumValue,
        NodeKind::Column,
        NodeKind::Form,
        NodeKind::Template,
        NodeKind::Command,
        NodeKind::UrlTemplate,
        NodeKind::Operation,
        NodeKind::Characteristic,
        NodeKind::StandardTabularSection,
        NodeKind::PredefinedItem,
    ]
}

fn metadata_collection(payload: &Value, kind: NodeKind) -> Option<&Value> {
    let collections = payload.get("collections")?;
    let declarations = payload.get("declarations");
    match kind {
        NodeKind::Attribute => collections.get("attributes"),
        NodeKind::StandardAttribute => declarations?.get("standardAttributes"),
        NodeKind::Characteristic => declarations?.get("characteristics"),
        NodeKind::StandardTabularSection => declarations?.get("standardTabularSections"),
        // Страница элементов лежит в `items`, а правдивый счёт — в `total`
        // рядом: при урезанной странице длина массива соврала бы.
        NodeKind::PredefinedItem => payload.get("predefinedItems")?.get("items"),
        NodeKind::TabularSection => collections.get("tabularSections"),
        NodeKind::Dimension => collections.get("dimensions"),
        NodeKind::Resource => collections.get("resources"),
        NodeKind::Recalculation => collections.get("recalculations"),
        NodeKind::EnumValue => collections.get("enumValues"),
        NodeKind::Column => collections.get("columns"),
        NodeKind::Form => collections.get("forms"),
        NodeKind::Template => collections.get("templates"),
        NodeKind::Command => collections.get("commands"),
        // Последовательности пофактовой части вида: платформа держит их не в
        // `collections`, но для читателя это такая же адресуемая коллекция.
        NodeKind::UrlTemplate => metadata_details(payload)?.get("urlTemplates"),
        NodeKind::Operation => metadata_details(payload)?.get("operations"),
        _ => None,
    }
}

/// Все ссылки объекта наружу — одной ветвью.
///
/// Платформа держит их в разных местах: владельцы и движения — в `relations`,
/// зарегистрированные документы журнала и базовые виды расчёта — в пофактовой
/// части вида. Вопрос при этом один: на что этот объект показывает и почему.
/// Поэтому имя связи становится полем элемента, а не отдельной ветвью на
/// каждое имя: девять ветвей ради девяти имён — это девять новых видов узлов
/// там, где хватает поля.
fn metadata_relations(address: &QualifiedAddress, payload: &Value) -> Vec<Value> {
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
    items
}

/// Пофактовая часть вида: `details` приходит соседним тегом `{kind, details}`.
fn metadata_details(payload: &Value) -> Option<&Value> {
    payload
        .get("details")
        .and_then(|adjacent| adjacent.get("details"))
}

fn metadata_items(
    address: &QualifiedAddress,
    kind: NodeKind,
    value: &Value,
    payload: &Value,
    member_profile: Option<unica_metadata::MetaMemberReadProfile>,
) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let child = address.member(name).ok()?;
            serde_json::to_value(metadata_item_with_context(
                &child,
                kind,
                item,
                payload,
                member_profile,
            ))
            .ok()
        })
        .collect()
}

fn metadata_item_with_context(
    address: &QualifiedAddress,
    kind: NodeKind,
    item: &Value,
    payload: &Value,
    member_profile: Option<unica_metadata::MetaMemberReadProfile>,
) -> NodeView {
    let node = metadata_item_node(address, kind, item, member_profile);
    if kind == NodeKind::TabularSection {
        node.with_additional_limits(tabular_standard_attribute_limits(payload, item))
    } else {
        node
    }
}

fn tabular_standard_attribute_limits(payload: &Value, item: &Value) -> Vec<String> {
    let Some(sections) =
        metadata_collection(payload, NodeKind::TabularSection).and_then(Value::as_array)
    else {
        return Vec::new();
    };
    // Both routes borrow items directly from this array, so the native
    // diagnostic index belongs to this exact section, even for equal values.
    let Some(index) = sections
        .iter()
        .position(|section| std::ptr::eq(section, item))
    else {
        return Vec::new();
    };
    let prefix = format!("collections.tabularSections[{index}].standardAttributes");
    unreadable_entries(payload)
        .filter(|(field, _)| {
            field
                .strip_prefix(&prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
        })
        .map(|(field, message)| format!("source_unreadable: {field}: {message}"))
        .collect()
}

fn metadata_item_node(
    address: &QualifiedAddress,
    kind: NodeKind,
    item: &Value,
    member_profile: Option<unica_metadata::MetaMemberReadProfile>,
) -> NodeView {
    let mut limits = Vec::new();
    let mut props = selected_scalar_props(
        item,
        &[
            "synonym",
            "comment",
            "required",
            "fillValue",
            "addressingDimension",
            "incomplete",
            "template",
            "httpMethod",
            "handler",
            "procedure",
            "nillable",
            "transactioned",
            "direction",
            // Предопределённый элемент: собственные скаляры и поля по виду
            // владельца. Без них ветвь отвечала бы одним заголовком, то есть
            // домом без содержимого.
            "id",
            "parentId",
            "code",
            "description",
            "isFolder",
            "accountType",
            "offBalance",
            "order",
            "actionPeriodIsBase",
        ],
    );
    if kind == NodeKind::StandardAttribute {
        let tabular = address
            .segments()
            .iter()
            .rev()
            .nth(1)
            .is_some_and(|segment| segment.kind() == NodeKind::TabularSection);
        let allowed = if tabular {
            unica_metadata::META_STANDARD_ATTRIBUTE_PROPERTY_NAMES
        } else {
            &["Synonym", "ToolTip", "FillChecking"]
        };
        for property in item
            .get("properties")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(key) = property
                .get("name")
                .and_then(Value::as_str)
                .filter(|key| allowed.contains(key))
            else {
                continue;
            };
            let Some(value) = property.get("value") else {
                continue;
            };
            if let Some(value) = bounded_prop(key, value) {
                props.insert(key.to_string(), value);
            } else if tabular
                && (value.is_object() || value.is_array())
                && value.to_string().len() > MAX_COMPACT_PROP_BYTES
            {
                limits.push(format!("metadata standard attribute property {key} exceeds the compact property limit of {MAX_COMPACT_PROP_BYTES} bytes"));
            }
        }
    }
    for key in ["accountingFlags", "extDimensionTypes"] {
        if let Some(value) = item.get(key).and_then(|value| bounded_prop(key, value)) {
            props.insert(key.to_string(), value);
        }
    }
    if let Some(value) = item
        .get("returnType")
        .and_then(|value| bounded_prop("returnType", value))
    {
        props.insert("returnType".to_string(), value);
    }
    if let Some(value) = item.get("type") {
        if let Some(compact) = bounded_prop("type", value) {
            props.insert("type".to_string(), compact);
        } else if matches!(value, Value::Object(_) | Value::Array(_))
            && serde_json::to_string(value)
                .is_ok_and(|rendered| rendered.len() > MAX_COMPACT_PROP_BYTES)
        {
            limits.push(format!(
                "metadata type exceeds the compact property limit of {MAX_COMPACT_PROP_BYTES} bytes"
            ));
        }
    }
    if let Some(value) = item
        .get("fillValue")
        .and_then(|value| bounded_prop("fillValue", value))
    {
        props.insert("fillValue".to_string(), value);
    }
    let mut branches = [
        ("attributes", NodeKind::Attribute),
        ("methods", NodeKind::Method),
        ("parameters", NodeKind::Parameter),
        ("standardAttributes", NodeKind::StandardAttribute),
    ]
    .into_iter()
    .filter_map(|(field, kind)| {
        let items = item.get(field)?.as_array()?;
        (!items.is_empty())
            .then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), items.len()))
    })
    .collect::<Vec<_>>();
    branches.extend(
        item.get("logicalBranches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|branch| {
                let kind = branch.get("kind")?.as_str()?;
                let count = usize::try_from(branch.get("count")?.as_u64()?).ok()?;
                let kind = NodeKind::parse(kind).ok()?;
                (count > 0).then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
            }),
    );
    // Пределы элемента приходят от читателя: профиль свойств переносит их
    // вместе со своими, а без профиля они переносятся как есть, иначе узел
    // отвечал бы `incomplete` без причины (#1933).
    let (member_props, member_limits) = member_profile
        .and_then(|profile| {
            profile
                .context_for_address(address, kind)
                .map(|context| (profile, context))
        })
        .map_or_else(
            || {
                let limits = item
                    .get("propertyLimits")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                (Map::new(), limits)
            },
            |(profile, context)| member_properties::project(profile, context, item),
        );
    props.extend(member_props);
    limits.extend(member_limits);
    NodeView::new(
        address.to_string(),
        kind.as_str(),
        item.get("synonym")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .unwrap_or(kind.as_str()),
        props,
    )
    .with_branches(branches)
    .with_limits(limits)
}

/// Пофактовая часть вида раскладывается в `props` по ролям.
///
/// Составное значение из двух-трёх полей не помещается в `props` объектом и
/// не заслуживает ветви: адресовать внутри него нечего. Роль каждого поля
/// называет ключ, и родительный падеж уходит в имя: `handlerModule`, а не
/// `method.metadataPath`.
fn metadata_detail_props(payload: &Value) -> Map<String, Value> {
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
            if let Some(value) = composite
                .get(field)
                .and_then(|value| bounded_prop(role, value))
            {
                props.insert((*role).to_string(), value);
            }
        }
    }
    if let Some(value) = details
        .get("type")
        .and_then(|value| bounded_prop("type", value))
    {
        props.insert("type".to_string(), value);
    }
    props
}

fn metadata_property_props(payload: &Value) -> Map<String, Value> {
    payload
        .get("properties")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|property| {
            let key = property.get("key")?.as_str()?;
            // Синоним узел уже назвал сам — заголовком и собственным `props`.
            if key == "Synonym" {
                return None;
            }
            let value = bounded_prop(key, property.get("value")?)?;
            Some((key.to_string(), value))
        })
        .collect()
}

pub(crate) fn borrowing_props(
    borrowing: &unica_metadata_designer::xml_model::MetaBorrowing,
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

#[cfg(test)]
mod tabular_standard_attribute_property_tests {
    use super::*;

    #[test]
    fn tabular_standard_properties_keep_a_closed_utf8_bound_and_independent_values() {
        for extra in [0, 1] {
            let overhead = json!({"kind":"text","value":""}).to_string().len();
            let room = MAX_COMPACT_PROP_BYTES + extra - overhead;
            let text = format!("{}{}", "я".repeat(room / 2), "a".repeat(room % 2));
            let comment = json!({"kind":"text","value":text});
            assert_eq!(comment.to_string().len(), MAX_COMPACT_PROP_BYTES + extra);
            let item = json!({"name":"LineNumber","properties":[
                {"name":"Comment","value":comment},
                {"name":"Synonym","value":{"kind":"localizedString","values":[]}},
                {"name":"MultiLine","value":{"kind":"boolean","value":false}},
                {"name":"UnknownProviderKey","value":{"kind":"text","value":"x".repeat(3000)}}
            ]});
            for (raw, tabular) in [
                ("main:Catalog.Items.TabularSection.Rows.StandardAttribute.LineNumber", true),
                ("main:Catalog.Items.StandardAttribute.Code", false),
                ("main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes.StandardAttribute.ExtDimensionType", false),
            ] {
                let at = QualifiedAddress::parse(raw).unwrap();
                let node = serde_json::to_value(metadata_item_node(&at, NodeKind::StandardAttribute, &item, None)).unwrap();
                assert!(node["props"].get("UnknownProviderKey").is_none());
                assert!(node["props"].get("Synonym").is_some());
                assert_eq!(node["props"].get("Comment").is_some(), tabular && extra == 0, "{raw}: {node}");
                assert_eq!(node["props"].get("MultiLine").is_some(), tabular);
                if tabular {
                    assert_eq!(serde_json::from_str::<Value>(node["props"]["MultiLine"].as_str().unwrap()).unwrap(), json!({"kind":"boolean","value":false}));
                    if extra == 0 {
                        assert_eq!(serde_json::from_str::<Value>(node["props"]["Comment"].as_str().unwrap()).unwrap(), comment);
                    }
                }
                assert_eq!(node["limits"].to_string().contains("Comment"), tabular && extra == 1);
                assert!(!node["limits"].to_string().contains("UnknownProviderKey"));
            }
        }
    }
}

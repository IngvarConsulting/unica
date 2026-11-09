use super::{
    dropped_by_size, member_properties, place_prop, safe_prop, select_named, selected_scalar_props,
};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use unica_execution_contract::view_error::ViewError;

use serde_json::{Map, Value};
use unica_address::address::{AddressSegment, NodeKind, QualifiedAddress};
use unica_refusal::{RefusalCode, RefusalDetail};

mod collections;
mod items;
mod limits;
mod props;
mod relations;
use collections::{metadata_branch_kinds, metadata_collection, metadata_details};
use items::{metadata_item_with_context, metadata_items, tabular_standard_attribute_limits};
use limits::{
    metadata_limits, task_addressing_applies, task_addressing_issue, unreadable_collection,
    unreadable_entries, wsdl_limits,
};
pub use props::borrowing_props;
use props::{metadata_detail_props, metadata_property_props};
use relations::{metadata_missing_relation_limits, metadata_relation_limits, metadata_relations};

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
    // Подсистемы объекта и корни его библиотек (ветвь `Relation`).
    "subsystemMemberships",
    "libraryRoots",
    // Поля, которые читатель не смог спроецировать (см. `metadata_payload`).
    "unreadable",
    "wsdlDefinition",
];

/// Скаляры корня объекта метаданных в его `props`.
const ROOT_SCALAR_KEYS: &[&str] = &[
    "kind",
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
];

pub(super) fn project_metadata_for_source(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
    source_kind: Option<unica_source_contract::SourceSetKind>,
) -> Result<NodeViewData, ViewError> {
    let profile = source_kind
        .filter(|kind| *kind == unica_source_contract::SourceSetKind::Configuration)
        .and_then(|_| payload.get("memberReadProfile"))
        .and_then(|value| {
            serde_json::from_value::<unica_metadata::MetaMemberReadProfile>(value.clone()).ok()
        })
        .filter(|profile| {
            let owner = profile.owner_kind().as_str();
            payload.get("kind").and_then(Value::as_str) == Some(owner)
                && address
                    .segments()
                    .first()
                    .is_some_and(|part| part.kind().as_str() == owner)
        });
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
        let mut props = selected_scalar_props(payload, ROOT_SCALAR_KEYS);
        let size_limits = dropped_by_size(payload, &props, address, |probe| {
            selected_scalar_props(probe, ROOT_SCALAR_KEYS)
        });
        let mut limits = metadata_limits(payload);
        limits.extend(size_limits);
        props.extend(metadata_property_props(address, payload, &mut limits));
        // Синоним — свойство платформы и публикуется под её именем, тем же,
        // что принимает `props.set`. У видов, чей профиль не перечисляет
        // его среди свойств, значение берётся из сведений читателя; если
        // профиль его перечисляет, предел или порча уже сказаны там.
        let profiled = payload
            .get("properties")
            .and_then(Value::as_array)
            .is_some_and(|properties| {
                properties
                    .iter()
                    .any(|property| property.get("key").and_then(Value::as_str) == Some("Synonym"))
            });
        if !profiled {
            if let Some(synonym) = payload.get("synonym").filter(|value| value.is_string()) {
                place_prop(
                    &mut props,
                    &mut limits,
                    address,
                    "Synonym",
                    synonym,
                    true,
                    "metadata property Synonym",
                );
            }
        }
        props.extend(metadata_detail_props(address, payload, &mut limits));
        // Счёт предопределённых элементов берётся из ответа читателя, а не из
        // длины страницы: при урезанной странице они расходятся, и ветвь
        // обещала бы меньше, чем есть.
        let predefined_total = payload
            .get("predefinedItems")
            .and_then(|predefined| predefined.get("total"))
            .and_then(Value::as_u64);
        // Виды `ChildObjects`, которые профиль платформы допускает у вида
        // объекта, выдаются и пустыми (`count: 0`): «пусто» отличается от
        // «вид не поддержан», и первый элемент можно создать (#2153).
        let platform_kinds = match address.segments() {
            [owner] => unica_source_designer::registration::platform_child_object_order(
                owner.kind().as_str(),
            )
            .unwrap_or_default(),
            _ => &[],
        };
        let addressing_issue = task_addressing_applies(address, payload)
            .then(|| task_addressing_issue(payload))
            .flatten();
        if let Some(reason) = &addressing_issue {
            limits.push(reason.clone());
        }
        let mut branches = metadata_branch_kinds()
            .iter()
            .filter_map(|kind| {
                if *kind == NodeKind::AddressingAttribute
                    && (!task_addressing_applies(address, payload) || addressing_issue.is_some())
                {
                    return None;
                }
                let value = metadata_collection(payload, *kind);
                let count = match (kind, predefined_total) {
                    (NodeKind::PredefinedItem, Some(total)) => {
                        usize::try_from(total).unwrap_or(usize::MAX)
                    }
                    _ => value.and_then(Value::as_array).map_or(0, Vec::len),
                };
                let declared = platform_kinds.contains(&kind.as_str());
                (count > 0 || declared)
                    .then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
            })
            .collect::<Vec<_>>();
        let relations = metadata_relations(address, payload);
        if payload.get("kind").and_then(Value::as_str) == Some("WSReference") {
            if let Some(rows) = payload
                .pointer("/wsdlDefinition/rows")
                .and_then(Value::as_array)
            {
                branches.push(BranchRef::new(format!("{address}.Definition"), rows.len()));
            }
        }
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
            .with_limits(limits),
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
        let mut props = selected_scalar_props(
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
        // Детальные расписания дня — массив тех же фактов, вложенным JSON
        // без упаковки в строку и в общем пределе размера свойства (#2031).
        let mut limits = Vec::new();
        if let Some(detailed) = properties
            .get("detailedDailySchedules")
            .filter(|value| value.is_array())
        {
            place_prop(
                &mut props,
                &mut limits,
                address,
                "detailedDailySchedules",
                detailed,
                false,
                "schedule detailedDailySchedules",
            );
        }
        return Ok(NodeViewData::Node(
            NodeView::new(address.to_string(), "Schedule", "Schedule", props).with_limits(limits),
        ));
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
    if first.kind() == NodeKind::AddressingAttribute {
        if !task_addressing_applies(address, payload) {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "AddressingAttribute belongs only to a named Task",
            ));
        }
        if let Some(reason) = task_addressing_issue(payload) {
            return Err(ViewError::detailed(RefusalDetail::SourceUnreadable, reason));
        }
    }
    if first.kind() == NodeKind::Definition
        && first.name().is_none()
        && suffix.len() == 1
        && payload.get("kind").and_then(Value::as_str) == Some("WSReference")
    {
        let rows = payload
            .pointer("/wsdlDefinition/rows")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "local WSDL observation is unavailable",
                )
            })?;
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Definition", "Local WSDL", Map::new())
                .with_limits(wsdl_limits(payload)),
            rows.clone(),
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
    // Вид, который профиль выдаёт пустой ветвью, читается пустой коллекцией,
    // даже если читатель не прислал для него массива (#2153).
    let empty = Value::Array(Vec::new());
    let declared = matches!(address.segments(), [owner] if
        unica_source_designer::registration::platform_child_object_order(owner.kind().as_str())
            .is_some_and(|kinds| kinds.contains(&first.kind().as_str())));
    let collection = match metadata_collection(payload, first.kind()) {
        Some(collection) => collection,
        None if declared => &empty,
        None => {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                format!("metadata has no {} collection", first.kind().as_str()),
            ))
        }
    };
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

#[cfg(test)]
mod tabular_standard_attribute_property_tests {
    use super::items::metadata_item_node;
    use super::*;
    use crate::infrastructure::v13_read_projection::MAX_COMPACT_PROP_BYTES;
    use serde_json::json;

    #[test]
    fn tabular_standard_properties_keep_a_closed_utf8_bound_and_independent_values() {
        for extra in [0, 1] {
            // The text wrapper is removed: the bound applies to the
            // published string itself, counted in UTF-8 bytes.
            let room = MAX_COMPACT_PROP_BYTES + extra;
            let text = format!("{}{}", "я".repeat(room / 2), "a".repeat(room % 2));
            assert_eq!(text.len(), MAX_COMPACT_PROP_BYTES + extra);
            let comment = json!({"kind":"text","value":text});
            let synonym = json!([{"language":"ru","content":"Номер строки"}]);
            let item = json!({"name":"LineNumber","properties":[
                {"name":"Comment","value":comment},
                {"name":"Synonym","value":{"kind":"localizedString","values":synonym.clone()}},
                {"name":"ToolTip","value":{"kind":"localizedString","values":[]}},
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
                assert_eq!(node["props"]["Synonym"], synonym, "{raw}: {node}");
                // An empty translation set is no value: no key and no limit.
                assert!(node["props"].get("ToolTip").is_none(), "{raw}: {node}");
                assert!(!node["limits"].to_string().contains("ToolTip"));
                assert_eq!(node["props"].get("Comment").is_some(), tabular && extra == 0, "{raw}: {node}");
                assert_eq!(node["props"].get("MultiLine").is_some(), tabular);
                if tabular {
                    assert_eq!(node["props"]["MultiLine"], json!(false));
                    if extra == 0 {
                        assert_eq!(node["props"]["Comment"], json!(text));
                    }
                }
                assert_eq!(node["limits"].to_string().contains("Comment"), tabular && extra == 1);
                assert!(!node["limits"].to_string().contains("UnknownProviderKey"));
            }
        }
    }
}

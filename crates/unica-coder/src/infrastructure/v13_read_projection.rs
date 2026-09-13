use crate::application::v13::view::ViewError;
use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::module_projection::EventProjection;
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use crate::domain::refusal::RefusalCode;
use crate::infrastructure::logical_tree::{LogicalReader, LogicalTreeRoute};
use serde_json::{json, Map, Value};

mod role;
use role::project_role;

const MAX_PROJECTED_BRANCHES: usize = 256;
const MAX_COMPACT_PROP_BYTES: usize = 2_048;

pub(super) fn project_typed_payload(
    route: &LogicalTreeRoute,
    payload: Value,
) -> Result<NodeViewData, ViewError> {
    project_typed_payload_inner(route, payload, None)
}

pub(super) fn project_typed_payload_with_form_events(
    route: &LogicalTreeRoute,
    payload: Value,
    events: &[EventProjection],
) -> Result<NodeViewData, ViewError> {
    project_typed_payload_inner(route, payload, Some(events))
}

fn project_typed_payload_inner(
    route: &LogicalTreeRoute,
    payload: Value,
    form_events: Option<&[EventProjection]>,
) -> Result<NodeViewData, ViewError> {
    validate_reader_payload(route.reader(), &payload)?;
    let root_depth = route
        .reader_metadata_path()
        .map(|target| target.as_str().split('.').count().div_ceil(2))
        .unwrap_or_else(|| {
            usize::from(
                route.reader() == LogicalReader::Configuration
                    || (route.reader() == LogicalReader::Metadata
                        && route
                            .at()
                            .segments()
                            .first()
                            .is_some_and(|segment| segment.name().is_some())),
            )
        });
    let suffix = &route.at().segments()[root_depth.min(route.at().segments().len())..];
    match route.reader() {
        LogicalReader::Configuration => return project_configuration(route.at(), &payload, suffix),
        LogicalReader::Metadata => return project_metadata(route.at(), &payload, suffix),
        LogicalReader::Form => return project_form(route.at(), &payload, suffix, form_events),
        LogicalReader::Role => return project_role(route.at(), &payload, suffix),
        LogicalReader::Interface => {
            return project_subsystem_interface(route.at(), &payload, suffix)
        }
        LogicalReader::Xdto => return project_xdto(route.at(), &payload, suffix),
        LogicalReader::Dcs => return project_dcs(route.at(), &payload, suffix),
        LogicalReader::Mxl => return project_mxl(route.at(), &payload, suffix),
        _ => {}
    }
    if suffix.is_empty() {
        return Ok(NodeViewData::Node(known_reader_node(
            route.reader(),
            route.at(),
            route
                .at()
                .segments()
                .last()
                .map(AddressSegment::kind)
                .unwrap_or(NodeKind::Configuration),
            &payload,
        )));
    }
    project_known_suffix(route.reader(), route.at(), &payload, suffix)
}

fn project_form(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
    semantic_events: Option<&[EventProjection]>,
) -> Result<NodeViewData, ViewError> {
    if let Some(events) = semantic_events.filter(|_| {
        suffix
            .last()
            .is_some_and(|part| part.kind() == NodeKind::Event)
    }) {
        return project_semantic_form_event(address, suffix, events);
    }
    if suffix.is_empty() {
        let props = selected_scalar_props(
            payload,
            &[
                "title",
                "objectContext",
                "isExtension",
                "baseFormVersion",
                "commandBarLocation",
            ],
        );
        let branches = [
            NodeKind::Item,
            NodeKind::Attribute,
            NodeKind::Parameter,
            NodeKind::Command,
            NodeKind::Event,
        ]
        .into_iter()
        .filter_map(|kind| {
            if kind == NodeKind::Event {
                if let Some(events) = semantic_events {
                    let count = form_owner_events(&address.to_string(), events).count();
                    return (count > 0)
                        .then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count));
                }
            }
            let values = form_child_values(payload, kind)?;
            let count = values.as_array().map_or(0, Vec::len);
            (count > 0).then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
        })
        .collect::<Vec<_>>();
        return Ok(NodeViewData::Node(
            NodeView::new(
                address.to_string(),
                "Form",
                payload
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("name").and_then(Value::as_str))
                    .unwrap_or("Form"),
                props,
            )
            .with_branches(branches),
        ));
    }
    let first = &suffix[0];
    let mut values = form_child_values(payload, first.kind()).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("form has no {} collection", first.kind().as_str()),
        )
    })?;
    if first.name().is_none() {
        if suffix.len() != 1 {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "form collection did not consume the complete suffix",
            ));
        }
        return form_collection(address, first.kind(), values, semantic_events);
    }
    let mut kind = first.kind();
    let mut item = select_named(values, first.name().unwrap()).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!(
                "form {} `{}` was not found",
                first.kind().as_str(),
                first.name().unwrap()
            ),
        )
    })?;
    for (index, segment) in suffix[1..].iter().enumerate() {
        kind = segment.kind();
        values = form_nested_values(item, kind).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("form node has no {} collection", kind.as_str()),
            )
        })?;
        let terminal = index + 2 == suffix.len();
        let Some(name) = segment.name() else {
            if !terminal {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "form collection did not consume the complete suffix",
                ));
            }
            return form_collection(address, kind, values, semantic_events);
        };
        item = select_named(values, name).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("form {} `{name}` was not found", kind.as_str()),
            )
        })?;
    }
    Ok(NodeViewData::Node(form_node(
        address,
        kind,
        item,
        semantic_events,
    )))
}

fn project_semantic_form_event(
    address: &QualifiedAddress,
    suffix: &[AddressSegment],
    events: &[EventProjection],
) -> Result<NodeViewData, ViewError> {
    let event = suffix
        .last()
        .expect("the semantic event route has an Event suffix");
    if event.name().is_none() {
        let address_text = address.to_string();
        let owner_at = address_text.strip_suffix(".Event").ok_or_else(|| {
            ViewError::new(RefusalCode::NotFound, "event collection owner is invalid")
        })?;
        let items = form_owner_events(owner_at, events)
            .map(crate::infrastructure::v13_read::event_node_value)
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "form owner has no applicable events",
            ));
        }
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(
                address.to_string(),
                NodeKind::Event.as_str(),
                "Event",
                Map::new(),
            ),
            items,
        )));
    }
    events
        .iter()
        .find(|candidate| candidate.at == address.to_string())
        .map(crate::infrastructure::v13_read::event_node)
        .map(NodeViewData::Node)
        .ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!(
                    "form Event `{}` was not found",
                    event.name().unwrap_or_default()
                ),
            )
        })
}

fn form_owner_events<'a>(
    owner_at: &'a str,
    events: &'a [EventProjection],
) -> impl Iterator<Item = &'a EventProjection> + 'a {
    events.iter().filter(move |event| {
        event
            .at
            .rsplit_once(".Event.")
            .is_some_and(|(candidate, _)| candidate == owner_at)
    })
}

fn form_child_values(payload: &Value, kind: NodeKind) -> Option<&Value> {
    match kind {
        NodeKind::Item => payload.get("elements"),
        NodeKind::Attribute => payload.get("attributes"),
        NodeKind::Parameter => payload.get("parameters"),
        NodeKind::Command => payload.get("commands"),
        NodeKind::Event => payload.get("events"),
        _ => None,
    }
}

fn form_nested_values(item: &Value, kind: NodeKind) -> Option<&Value> {
    match kind {
        NodeKind::Item => item.get("children"),
        NodeKind::Column => item.get("columns"),
        NodeKind::Event => item.get("events").or_else(|| item.get("actions")),
        _ => None,
    }
}

fn form_collection(
    address: &QualifiedAddress,
    kind: NodeKind,
    values: &Value,
    semantic_events: Option<&[EventProjection]>,
) -> Result<NodeViewData, ViewError> {
    let items = values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item_identity(item)?;
            let child = QualifiedAddress::parse(&format!("{address}.{name}")).ok()?;
            serde_json::to_value(form_node(&child, kind, item, semantic_events)).ok()
        })
        .collect();
    Ok(NodeViewData::Collection(CollectionView::new(
        NodeView::new(
            address.to_string(),
            kind.as_str(),
            kind.as_str(),
            Map::new(),
        ),
        items,
    )))
}

fn form_node(
    address: &QualifiedAddress,
    kind: NodeKind,
    item: &Value,
    semantic_events: Option<&[EventProjection]>,
) -> NodeView {
    let mut node = NodeView::new(
        address.to_string(),
        kind.as_str(),
        item.get("title")
            .and_then(Value::as_str)
            .or_else(|| item_identity(item))
            .unwrap_or(kind.as_str()),
        reader_node_props(LogicalReader::Form, kind, item),
    );
    let branch_kinds: &[NodeKind] = match kind {
        NodeKind::Item => &[NodeKind::Item, NodeKind::Event],
        NodeKind::Attribute => &[NodeKind::Column],
        NodeKind::Command => &[NodeKind::Event],
        _ => &[],
    };
    node = node.with_branches(
        branch_kinds
            .iter()
            .filter_map(|child_kind| {
                if *child_kind == NodeKind::Event {
                    if let Some(events) = semantic_events {
                        let count = form_owner_events(&address.to_string(), events).count();
                        return (count > 0).then(|| {
                            BranchRef::new(format!("{}.{}", address, child_kind.as_str()), count)
                        });
                    }
                }
                let values = form_nested_values(item, *child_kind)?;
                let count = values.as_array().map_or(0, Vec::len);
                (count > 0)
                    .then(|| BranchRef::new(format!("{}.{}", address, child_kind.as_str()), count))
            })
            .collect(),
    );
    node
}

fn project_dcs(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    if suffix.is_empty() {
        return Ok(NodeViewData::Node(known_reader_node(
            LogicalReader::Dcs,
            address,
            NodeKind::Template,
            payload,
        )));
    }
    let dataset_segment = &suffix[0];
    if dataset_segment.kind() != NodeKind::DataSet {
        return project_known_suffix(LogicalReader::Dcs, address, payload, suffix);
    }
    let datasets = payload
        .get("dataSets")
        .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "DCS has no datasets"))?;
    let Some(dataset_name) = dataset_segment.name() else {
        if suffix.len() != 1 {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "DCS dataset collection cannot have a child suffix",
            ));
        }
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "DataSet", "DataSet", Map::new()),
            collection_items(LogicalReader::Dcs, address, NodeKind::DataSet, datasets),
        )));
    };
    let dataset = select_named(datasets, dataset_name).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("DCS dataset `{dataset_name}` was not found"),
        )
    })?;
    if suffix.len() == 1 {
        let node = node_from_value(LogicalReader::Dcs, address, NodeKind::DataSet, dataset);
        let mut branches = known_reader_branches(LogicalReader::Dcs, address, dataset);
        let parameter_count = payload
            .get("parameters")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if parameter_count > 0 {
            branches.push(BranchRef::new(
                format!("{}.Parameter", address),
                parameter_count,
            ));
        }
        return Ok(NodeViewData::Node(node.with_branches(branches)));
    }
    let [detail] = &suffix[1..] else {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "DCS projection did not consume the complete suffix",
        ));
    };
    let selected = match detail.kind() {
        NodeKind::Field => dataset.get("fields"),
        NodeKind::Parameter => payload.get("parameters"),
        NodeKind::Query => dataset.get("query"),
        _ => None,
    }
    .ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("DCS dataset has no {} projection", detail.kind().as_str()),
        )
    })?;
    if detail.name().is_none() {
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(
                address.to_string(),
                detail.kind().as_str(),
                detail.kind().as_str(),
                Map::new(),
            ),
            collection_items(LogicalReader::Dcs, address, detail.kind(), selected),
        )));
    }
    let name = detail.name().unwrap();
    let item = select_named(selected, name).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("DCS {} `{name}` was not found", detail.kind().as_str()),
        )
    })?;
    Ok(NodeViewData::Node(NodeView::new(
        address.to_string(),
        detail.kind().as_str(),
        name,
        reader_node_props(LogicalReader::Dcs, detail.kind(), item),
    )))
}

fn project_mxl(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    if suffix.is_empty() {
        return Ok(NodeViewData::Node(known_reader_node(
            LogicalReader::Mxl,
            address,
            NodeKind::Template,
            payload,
        )));
    }
    let area_segment = &suffix[0];
    if area_segment.kind() != NodeKind::Area {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "MXL projection starts at Area",
        ));
    }
    let areas = payload
        .get("areas")
        .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "MXL has no named areas"))?;
    let Some(area_name) = area_segment.name() else {
        if suffix.len() != 1 {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "MXL area collection cannot have a child suffix",
            ));
        }
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Area", "Area", Map::new()),
            collection_items(LogicalReader::Mxl, address, NodeKind::Area, areas),
        )));
    };
    let area = select_named(areas, area_name).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("MXL area `{area_name}` was not found"),
        )
    })?;
    if suffix.len() == 1 {
        let mut node = node_from_value(LogicalReader::Mxl, address, NodeKind::Area, area);
        let parameter_count = area
            .get("params")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let mut branches = Vec::new();
        if parameter_count > 0 {
            branches.push(BranchRef::new(
                format!("{}.Parameter", address),
                parameter_count,
            ));
        }
        // Счёт непустых ячеек известен и без текста, поэтому ветвь честно
        // объявляет свою длину, а текст читается только по её адресу.
        let content_count = area
            .get("contentCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if content_count > 0 {
            branches.push(BranchRef::new(
                format!("{}.{}", address, NodeKind::Body.as_str()),
                content_count as usize,
            ));
        }
        if !branches.is_empty() {
            node = node.with_branches(branches);
        }
        return Ok(NodeViewData::Node(node));
    }
    if let [body] = &suffix[1..] {
        if body.kind() == NodeKind::Body {
            if body.name().is_some() {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "MXL area body is a collection and takes no name",
                ));
            }
            let content = area
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    ViewError::new(
                        RefusalCode::ProviderUnavailable,
                        "MXL area body was projected without cell content",
                    )
                })?;
            return Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(
                    address.to_string(),
                    NodeKind::Body.as_str(),
                    NodeKind::Body.as_str(),
                    Map::new(),
                ),
                content
                    .iter()
                    .enumerate()
                    .map(|(index, cell)| {
                        json!({
                            "index": index + 1,
                            "template": cell.get("template").and_then(Value::as_bool).unwrap_or(false),
                            "text": cell.get("text").and_then(Value::as_str).unwrap_or_default(),
                        })
                    })
                    .collect(),
            )));
        }
    }
    let [parameter] = &suffix[1..] else {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "MXL projection did not consume the complete suffix",
        ));
    };
    if parameter.kind() != NodeKind::Parameter {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "MXL area has only Parameter child nodes",
        ));
    }
    let params = area
        .get("params")
        .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "MXL area has no parameters"))?;
    let Some(name) = parameter.name() else {
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Parameter", "Parameter", Map::new()),
            params
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|name| {
                    json!({
                        "at": format!("{address}.{name}"),
                        "kind": "Parameter",
                        "title": name,
                    })
                })
                .collect(),
        )));
    };
    if !params
        .as_array()
        .into_iter()
        .flatten()
        .any(|value| value.as_str() == Some(name))
    {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            format!("MXL area parameter `{name}` was not found"),
        ));
    }
    Ok(NodeViewData::Node(NodeView::new(
        address.to_string(),
        "Parameter",
        name,
        Map::new(),
    )))
}

fn project_configuration(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    if !suffix.is_empty() {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "configuration projection does not accept an address suffix",
        ));
    }
    // The root carries what the retired `unica.cf.info` reported: the
    // identity of the export, the vendor facts, the support state, the
    // closed set of root properties and the home page layout.
    let mut props = selected_scalar_props(
        payload,
        &[
            "format",
            "name",
            "synonym",
            "version",
            "vendor",
            "extensionPurpose",
            "totalObjects",
        ],
    );
    // `interface` идёт рядом с `homePage` не по сходству имён: оба живут
    // отдельным документом рядом с `Configuration.xml` и оба описывают корень
    // целиком. Командного интерфейса как узла у конфигурации нет — внутри
    // корневого документа только порядок подсистем, листать там нечего.
    if let Some(Value::Object(interface)) = payload.get("interface") {
        let mut interface = interface.clone();
        // Ссылка `Subsystem.Планирование` дополняется до адреса: читателю
        // нужен адрес, по которому можно спуститься, а не строка платформы.
        if let Some(Value::Array(order)) = interface.get("subsystemOrder") {
            let qualified: Vec<Value> = order
                .iter()
                .filter_map(Value::as_str)
                .map(|reference| Value::String(format!("{}:{reference}", address.source_set())))
                .collect();
            interface.insert("subsystemOrder".to_string(), Value::Array(qualified));
        }
        props.insert("interface".to_string(), Value::Object(interface));
    }
    for key in ["support", "properties", "homePage"] {
        match payload.get(key) {
            Some(Value::Object(object)) => {
                // Typed, bounded structures of the root parser travel whole.
                props.insert(key.to_string(), Value::Object(object.clone()));
            }
            Some(Value::Null) => {
                props.insert(key.to_string(), Value::Null);
            }
            _ => {}
        }
    }
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for item in payload
        .get("registeredObjects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(kind) = item.get("kind").and_then(Value::as_str) {
            *counts.entry(kind.to_string()).or_default() += 1;
        }
    }
    let branches = counts
        .into_iter()
        .map(|(kind, count)| BranchRef::new(format!("{}:{kind}", address.source_set()), count))
        .collect();
    Ok(NodeViewData::Node(
        NodeView::new(
            address.to_string(),
            NodeKind::Configuration.as_str(),
            payload
                .get("synonym")
                .and_then(Value::as_str)
                .or_else(|| payload.get("name").and_then(Value::as_str))
                .unwrap_or("Configuration"),
            props,
        )
        .with_branches(branches),
    ))
}

pub(super) fn project_registered_metadata_branch(
    address: &QualifiedAddress,
    payload: &Value,
) -> Result<NodeViewData, ViewError> {
    validate_reader_payload(LogicalReader::Configuration, payload)?;
    let kind = address
        .segments()
        .last()
        .map(AddressSegment::kind)
        .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "metadata branch kind is missing"))?;
    let items = payload
        .get("registeredObjects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("kind").and_then(Value::as_str) == Some(kind.as_str()))
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .map(|name| {
            serde_json::to_value(NodeView::new(
                format!("{}.{name}", address),
                kind.as_str(),
                name,
                Map::new(),
            ))
            .expect("NodeView always serializes")
        })
        .collect();
    Ok(NodeViewData::Collection(CollectionView::new(
        NodeView::new(
            address.to_string(),
            kind.as_str(),
            kind.as_str(),
            Map::new(),
        ),
        items,
    )))
}

fn validate_reader_payload(reader: LogicalReader, payload: &Value) -> Result<(), ViewError> {
    let object = payload.as_object().ok_or_else(|| {
        ViewError::new(
            RefusalCode::ProviderUnavailable,
            "typed reader returned a non-object root payload",
        )
    })?;
    let allowed: &[&str] = match reader {
        LogicalReader::Configuration => &[
            "format",
            "name",
            "synonym",
            "version",
            "vendor",
            "extensionPurpose",
            "support",
            "properties",
            "childObjects",
            "registeredObjects",
            "totalObjects",
            "homePage",
            "interface",
        ],
        LogicalReader::Metadata => &[
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
        ],
        LogicalReader::Form => &[
            "name",
            "title",
            "objectContext",
            "isExtension",
            "baseFormVersion",
            "support",
            "properties",
            "events",
            "autoCommandBar",
            "commandBarLocation",
            "elements",
            "attributes",
            "parameters",
            "commands",
        ],
        LogicalReader::Role => &[
            "name",
            "synonym",
            "support",
            "defaults",
            "allowed",
            "denied",
            "totals",
            "restrictedObjects",
            "templates",
        ],
        LogicalReader::Subsystem | LogicalReader::Interface => &[
            "name",
            "synonym",
            "comment",
            "explanation",
            "picture",
            "includeInCommandInterface",
            "useOneCommand",
            "support",
            "content",
            "groups",
            "children",
            "tree",
            "commandInterface",
        ],
        LogicalReader::Dcs => &[
            "support",
            "dataSources",
            "dataSets",
            "links",
            "calculatedFields",
            "totalFields",
            "parameters",
            "variants",
            "templates",
        ],
        LogicalReader::Mxl => &[
            "name",
            "support",
            "rows",
            "columns",
            "columnSets",
            "areas",
            "outside",
            "mergeCount",
            "drawingCount",
        ],
        LogicalReader::Xdto => &[
            "sourceSet",
            "metadataPath",
            "location",
            "targetNamespace",
            "imports",
            "counts",
            "globalProperties",
            "types",
            "typeDetail",
            "findings",
            "nextCursor",
        ],
        LogicalReader::Module => {
            return Err(ViewError::new(
                RefusalCode::ProviderUnavailable,
                "module projection bypassed its Task 13 adapter",
            ));
        }
    };
    const EXPLICITLY_DISCARDED_PROVIDER_KEYS: &[&str] = &[
        "sourceSet",
        "metadataPath",
        "location",
        "fileExists",
        "provider",
        "sourceState",
        "layout",
        "set",
        "nextCursor",
    ];
    if let Some(key) = object.keys().find(|key| {
        !allowed.contains(&key.as_str())
            && !EXPLICITLY_DISCARDED_PROVIDER_KEYS.contains(&key.as_str())
    }) {
        return Err(ViewError::new(
            RefusalCode::ProviderUnavailable,
            format!("typed {reader:?} reader returned unknown field `{key}`"),
        ));
    }
    Ok(())
}

pub(super) fn project_known_suffix(
    reader: LogicalReader,
    address: &QualifiedAddress,
    root: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    let mut current = root;
    for (index, segment) in suffix.iter().enumerate() {
        let terminal = index + 1 == suffix.len();
        let selected = select_kind_value(reader, current, segment.kind()).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("typed reader has no {} projection", segment.kind().as_str()),
            )
        })?;
        if let Some(name) = segment.name() {
            current = select_named(selected, name).ok_or_else(|| {
                ViewError::new(
                    RefusalCode::NotFound,
                    format!(
                        "typed reader has no {} named {name}",
                        segment.kind().as_str()
                    ),
                )
            })?;
            if terminal {
                return Ok(NodeViewData::Node(known_reader_node(
                    reader,
                    address,
                    segment.kind(),
                    current,
                )));
            }
        } else if terminal {
            let items = collection_items(reader, address, segment.kind(), selected);
            let node = NodeView::new(
                address.to_string(),
                segment.kind().as_str(),
                segment.kind().as_str(),
                Map::new(),
            );
            return Ok(NodeViewData::Collection(CollectionView::new(node, items)));
        } else {
            current = selected;
        }
    }
    Err(ViewError::new(
        RefusalCode::NotFound,
        "typed reader did not resolve the complete logical address",
    ))
}

fn select_kind_value(reader: LogicalReader, value: &Value, kind: NodeKind) -> Option<&Value> {
    if kind == NodeKind::Interface {
        return value.get("commandInterface");
    }
    if kind == NodeKind::Type {
        if let Some(detail) = value.get("typeDetail").filter(|value| !value.is_null()) {
            return Some(detail);
        }
    }
    first_field(value, reader_branch_fields(reader, kind))
}

fn reader_branch_fields(reader: LogicalReader, kind: NodeKind) -> &'static [&'static str] {
    match (reader, kind) {
        (LogicalReader::Form, NodeKind::Item) => &["elements", "children"],
        (LogicalReader::Form, NodeKind::Attribute) => &["attributes"],
        (LogicalReader::Form, NodeKind::Parameter) => &["parameters"],
        (LogicalReader::Form, NodeKind::Command) => &["commands"],
        (LogicalReader::Form, NodeKind::Event) => &["events", "actions"],
        (LogicalReader::Dcs, NodeKind::DataSet) => &["dataSets", "items"],
        (LogicalReader::Dcs, NodeKind::Field) => &["fields"],
        (LogicalReader::Dcs, NodeKind::Query) => &["query"],
        (LogicalReader::Dcs, NodeKind::Calculation) => &["calculatedFields", "totalFields"],
        (LogicalReader::Dcs, NodeKind::Parameter) => &["parameters"],
        (LogicalReader::Dcs, NodeKind::Setting) => &["variants"],
        (LogicalReader::Mxl, NodeKind::Area) => &["areas"],
        _ => branch_field_names(kind),
    }
}

fn project_metadata(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    if suffix.is_empty() {
        let mut props = selected_scalar_props(payload, &["kind", "synonym", "support"]);
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
            .with_branches(branches),
        ));
    }
    let first = &suffix[0];
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
            ),
            metadata_relations(address, payload),
        )));
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
            metadata_items(address, first.kind(), collection),
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
                metadata_items(address, kind, next),
            )));
        }
    }
    Ok(NodeViewData::Node(metadata_item_node(address, kind, item)))
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
    let mut push = |relation: &str, target: &str| {
        let Ok(at) = QualifiedAddress::parse(&format!("{}:{target}", address.source_set())) else {
            return;
        };
        let Some(kind) = at.segments().last().map(AddressSegment::kind) else {
            return;
        };
        items.push(json!({
            "relation": relation,
            "at": at.to_string(),
            "kind": kind.as_str(),
        }));
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
                push(relation, value);
            }
        }
    }
    // Источник события и пакеты XDTO названы платформой по-своему, но
    // показывают ровно наружу и ложатся сюда же. Вариант без адреса —
    // `family` у источника, пространство имён у пакета — сюда не попадает:
    // ветвь адресует объекты, а не строки.
    let details = metadata_details(payload);
    for (holder, field, relation) in [
        (relations, "source", "source"),
        (details, "registeredDocuments", "registeredDocument"),
        (details, "baseCalculationTypes", "baseCalculationType"),
        (details, "xdtoPackages", "xdtoPackage"),
    ] {
        let entries = holder
            .and_then(|holder| holder.get(field))
            .and_then(Value::as_array);
        for entry in entries.into_iter().flatten() {
            let target = entry
                .as_str()
                .or_else(|| entry.get("metadataPath").and_then(Value::as_str));
            if let Some(target) = target {
                push(relation, target);
            }
        }
    }
    items
}

/// Пофактовая часть вида: `details` приходит соседним тегом `{kind, details}`.
fn metadata_details(payload: &Value) -> Option<&Value> {
    payload
        .get("details")
        .and_then(|adjacent| adjacent.get("details"))
}

fn metadata_items(address: &QualifiedAddress, kind: NodeKind, value: &Value) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let child = QualifiedAddress::parse(&format!("{address}.{name}")).ok()?;
            serde_json::to_value(metadata_item_node(&child, kind, item)).ok()
        })
        .collect()
}

fn metadata_item_node(address: &QualifiedAddress, kind: NodeKind, item: &Value) -> NodeView {
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
    if let Some(value) = item
        .get("type")
        .and_then(|value| bounded_prop("type", value))
    {
        props.insert("type".to_string(), value);
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
}

fn project_subsystem_interface(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    if !matches!(suffix.first(), Some(segment) if segment.kind() == NodeKind::Interface) {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "subsystem interface projection is missing",
        ));
    }
    let interface = payload
        .get("commandInterface")
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                "subsystem has no command interface document",
            )
        })?;
    let commands = interface_commands(interface);
    // Имя команды в CommandInterface.xml — это ссылка на команду, живущую в
    // другом месте: `CommonCommand.Печать` или `Catalog.Валюты.Command.Создать`.
    // Приписывать её к адресу интерфейса нельзя — получается адрес, которого
    // грамматика не знает. Команда адресуется по своему настоящему месту.
    let (addressable, dangling) = split_interface_commands(address, &commands);
    if suffix.len() > 2 {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "subsystem interface projection did not consume the complete suffix",
        ));
    }
    // Порядок — это последовательность, а не скаляр, поэтому он живёт
    // ветвями, а не свойствами: `props` держат только ограниченные скаляры.
    let groups = interface_listed(interface, "groupsOrder");
    let subsystems = interface_listed(interface, "subsystemOrder");
    if suffix.len() == 1 {
        let mut branches = Vec::new();
        if !addressable.is_empty() {
            branches.push(BranchRef::new(
                format!("{}.Command", address),
                addressable.len(),
            ));
        }
        if !groups.is_empty() {
            branches.push(BranchRef::new(format!("{}.Group", address), groups.len()));
        }
        if !subsystems.is_empty() {
            branches.push(BranchRef::new(
                format!("{}.Subsystem", address),
                subsystems.len(),
            ));
        }
        return Ok(NodeViewData::Node(
            NodeView::new(
                address.to_string(),
                "Interface",
                "Command interface",
                Map::new(),
            )
            .with_branches(branches)
            .with_limits(dangling_limits(&dangling)),
        ));
    }
    let command = &suffix[1];
    if command.kind() == NodeKind::Group {
        return project_interface_groups(address, interface, &groups, command.name());
    }
    if command.kind() == NodeKind::Subsystem {
        if command.name().is_some() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "the interface lists child subsystems in order; read the subsystem at its own address",
            ));
        }
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Subsystem", "Subsystems", Map::new()),
            subsystems
                .iter()
                .enumerate()
                .map(|(index, reference)| {
                    json!({
                        "order": index + 1,
                        // Ссылка платформы дополняется до адреса: читателю
                        // нужен адрес, по которому можно спуститься.
                        "at": format!("{}:{reference}", address.source_set()),
                    })
                })
                .collect(),
        )));
    }
    match (command.kind(), command.name()) {
        (NodeKind::Command, None) => Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Command", "Commands", Map::new())
                .with_limits(dangling_limits(&dangling)),
            addressable
                .iter()
                .filter_map(|(at, item)| {
                    let name = item.get("name")?.as_str()?;
                    serde_json::to_value(
                        NodeView::new(
                            at.to_string(),
                            at.segments().last()?.kind().as_str(),
                            name,
                            reader_node_props(LogicalReader::Interface, NodeKind::Command, item),
                        )
                        .with_limits(Vec::new()),
                    )
                    .ok()
                })
                .collect(),
        ))),
        (NodeKind::Command, Some(name)) => commands
            .iter()
            .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
            .map(|item| {
                NodeViewData::Node(NodeView::new(
                    address.to_string(),
                    "Command",
                    name,
                    reader_node_props(LogicalReader::Interface, NodeKind::Command, item),
                ))
            })
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::NotFound,
                    format!("interface command `{name}` was not found"),
                )
            }),
        _ => Err(ViewError::new(
            RefusalCode::NotFound,
            "interface projection was not found",
        )),
    }
}

/// Разводит записи командного интерфейса на адресуемые и повисшие.
///
/// Ссылка `CommonCommand.Печать` или `Catalog.Валюты.Command.Создать` читается
/// как адрес в том же наборе исходников. Ссылка по идентификатору
/// (`0:78b4152e-…`) логического адреса не имеет: платформа так пишет указатель
/// на исчезнувший объект.
fn split_interface_commands(
    address: &QualifiedAddress,
    commands: &[Value],
) -> (Vec<(QualifiedAddress, Value)>, Vec<String>) {
    let source_set = address.source_set();
    let mut addressable = Vec::new();
    let mut dangling = Vec::new();
    for item in commands {
        let Some(reference) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        match QualifiedAddress::parse(&format!("{source_set}:{reference}")) {
            Ok(at) => addressable.push((at, item.clone())),
            Err(_) => dangling.push(reference.to_string()),
        }
    }
    (addressable, dangling)
}

/// Повисшие ссылки называются слотом `limits`, а не выбрасываются молча: иначе
/// счёт ветви и страница расходятся, а читатель считает, что видел всё.
fn dangling_limits(dangling: &[String]) -> Vec<String> {
    if dangling.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "{} команд заданы ссылкой по идентификатору и логического адреса не имеют: {}",
        dangling.len(),
        dangling.join(", ")
    )]
}

fn interface_listed(interface: &Value, key: &str) -> Vec<String> {
    interface
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// Группы панели в объявленном порядке, а внутри группы — её команды в том
/// порядке, который задаёт `CommandsOrder`. Группа без единой команды законна
/// и остаётся видимой: порядок объявляется отдельно от наполнения.
fn project_interface_groups(
    address: &QualifiedAddress,
    interface: &Value,
    groups: &[String],
    requested: Option<&str>,
) -> Result<NodeViewData, ViewError> {
    let commands_of = |group: &str| -> Vec<String> {
        interface
            .get("order")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|entry| entry.get("name").and_then(Value::as_str) == Some(group))
            .and_then(|entry| entry.get("items"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    };
    let Some(name) = requested else {
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Group", "Groups", Map::new()),
            groups
                .iter()
                .enumerate()
                .map(|(index, group)| {
                    json!({
                        "at": format!("{address}.{group}"),
                        "kind": "Group",
                        "title": group,
                        "order": index + 1,
                        "commands": commands_of(group).len(),
                    })
                })
                .collect(),
        )));
    };
    if !groups.iter().any(|group| group == name) {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            format!("the command interface declares no group `{name}`"),
        ));
    }
    Ok(NodeViewData::Collection(CollectionView::new(
        NodeView::new(address.to_string(), "Group", name, Map::new()),
        commands_of(name)
            .into_iter()
            .enumerate()
            .map(|(index, command)| json!({"order": index + 1, "command": command}))
            .collect(),
    )))
}

fn interface_commands(interface: &Value) -> Vec<Value> {
    let mut commands = std::collections::BTreeMap::<String, Map<String, Value>>::new();
    for item in interface
        .get("visibility")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(name) = item.get("command").and_then(Value::as_str) {
            let entry = commands.entry(name.to_string()).or_default();
            entry.insert("name".to_string(), json!(name));
            entry.insert(
                "visible".to_string(),
                item.get("visible").cloned().unwrap_or(Value::Null),
            );
            // Закрытый признак вместо умолчания: ноль говорит «переопределений
            // нет», а не «не смотрели». Без него `visible` читался бы как вся
            // правда о видимости команды.
            entry.insert(
                "roleOverrides".to_string(),
                item.get("roleOverrides").cloned().unwrap_or(json!(0)),
            );
        }
    }
    for item in interface
        .get("placement")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(name) = item.get("command").and_then(Value::as_str) {
            let entry = commands.entry(name.to_string()).or_default();
            entry.insert("name".to_string(), json!(name));
            for key in ["group", "placement"] {
                entry.insert(
                    key.to_string(),
                    item.get(key).cloned().unwrap_or(Value::Null),
                );
            }
        }
    }
    commands.into_values().map(Value::Object).collect()
}

fn project_xdto(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    if suffix.is_empty() {
        let mut props = selected_scalar_props(payload, &["targetNamespace"]);
        if let Some(counts) = payload.get("counts") {
            props.extend(selected_scalar_props(
                counts,
                &["total", "valueTypes", "objectTypes", "globalProperties"],
            ));
        }
        let type_count = payload
            .get("types")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let property_count = payload
            .get("globalProperties")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let mut branches = Vec::new();
        if payload
            .get("targetNamespace")
            .and_then(Value::as_str)
            .is_some()
        {
            branches.push(BranchRef::new(format!("{}.Namespace", address), 1));
        }
        if type_count > 0 {
            branches.push(BranchRef::new(format!("{}.Type", address), type_count));
        }
        if property_count > 0 {
            branches.push(BranchRef::new(
                format!("{}.Property", address),
                property_count,
            ));
        }
        return Ok(NodeViewData::Node(
            NodeView::new(
                address.to_string(),
                "XDTOPackage",
                address
                    .segments()
                    .first()
                    .and_then(AddressSegment::name)
                    .unwrap_or("XDTO"),
                props,
            )
            .with_branches(branches),
        ));
    }
    let segment = &suffix[0];
    if segment.kind() == NodeKind::Namespace && segment.name().is_none() {
        let namespace = payload
            .get("targetNamespace")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ViewError::new(RefusalCode::NotFound, "XDTO target namespace is absent")
            })?;
        return Ok(NodeViewData::Node(NodeView::new(
            address.to_string(),
            "Namespace",
            namespace,
            Map::from_iter([("namespace".to_string(), json!(namespace))]),
        )));
    }
    let selected = match segment.kind() {
        NodeKind::Type => payload
            .get("typeDetail")
            .filter(|value| !value.is_null())
            .or_else(|| payload.get("types")),
        NodeKind::Property => payload.get("globalProperties"),
        _ => None,
    }
    .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "XDTO projection was not found"))?;
    if segment.name().is_none() {
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(
                address.to_string(),
                segment.kind().as_str(),
                segment.kind().as_str(),
                Map::new(),
            ),
            collection_items(LogicalReader::Xdto, address, segment.kind(), selected),
        )));
    }
    let name = segment.name().unwrap();
    let item = select_named(selected, name).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("XDTO node `{name}` was not found"),
        )
    })?;
    if suffix.len() == 1 {
        let mut props = reader_node_props(LogicalReader::Xdto, segment.kind(), item);
        if let Some(base) = item.get("base") {
            props.insert(
                "base".to_string(),
                Value::String(serde_json::to_string(base).unwrap_or_default()),
            );
        }
        let branches = item
            .get("properties")
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
            .map(|items| vec![BranchRef::new(format!("{}.Property", address), items.len())])
            .unwrap_or_default();
        return Ok(NodeViewData::Node(
            NodeView::new(address.to_string(), segment.kind().as_str(), name, props)
                .with_branches(branches),
        ));
    }
    let property = &suffix[1];
    if suffix.len() != 2 {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            "XDTO property projection did not consume the complete suffix",
        ));
    }
    let properties = item
        .get("properties")
        .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "XDTO type has no properties"))?;
    if property.name().is_none() {
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Property", "Properties", Map::new()),
            collection_items(LogicalReader::Xdto, address, NodeKind::Property, properties),
        )));
    }
    let property_name = property.name().unwrap();
    let property = select_named(properties, property_name).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("XDTO property `{property_name}` was not found"),
        )
    })?;
    Ok(NodeViewData::Node(NodeView::new(
        address.to_string(),
        "Property",
        property_name,
        reader_node_props(LogicalReader::Xdto, NodeKind::Property, property),
    )))
}

/// Свойства объекта метаданных — это `props` его узла.
///
/// Читатель отдаёт их списком пар `{key, value}`, где ключ взят из закрытого
/// словаря `META_INFO_PROPERTY_NAMES`, а профиль вида уже решил, что из него
/// наблюдаемо. Развернуть список — значит снять обёртку, а не открыть дверь
/// произвольным ключам.
/// Значение, годное в `props`: скаляр как есть, составное — компактной
/// строкой.
///
/// Механизм один на всю проекцию: так уже отвечал тип реквизита, и второго
/// заводить не за чем. Граница у обоих путей общая — `MAX_COMPACT_PROP_BYTES`.
fn bounded_prop(key: &str, value: &Value) -> Option<Value> {
    match value {
        Value::Object(_) | Value::Array(_) => {
            let rendered = serde_json::to_string(value).ok()?;
            (rendered.len() <= MAX_COMPACT_PROP_BYTES).then_some(Value::String(rendered))
        }
        scalar => safe_prop(key, scalar).then(|| scalar.clone()),
    }
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

fn selected_scalar_props(value: &Value, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter_map(|key| {
            value
                .get(*key)
                .filter(|value| safe_prop(key, value))
                .map(|value| ((*key).to_string(), value.clone()))
        })
        .collect()
}

fn branch_field_names(kind: NodeKind) -> &'static [&'static str] {
    match kind {
        NodeKind::Attribute => &["attributes"],
        NodeKind::StandardAttribute => &["standardAttributes"],
        NodeKind::TabularSection => &["tabularSections"],
        NodeKind::Column => &["columns", "children"],
        NodeKind::Item => &["elements", "children", "items"],
        NodeKind::Parameter => &["parameters"],
        NodeKind::Command => &["commands"],
        NodeKind::Right => &["allowed", "denied", "rights"],
        NodeKind::Rls => &["restrictedObjects"],
        NodeKind::DataSet => &["dataSets", "items"],
        NodeKind::Field => &["fields", "calculatedFields", "totalFields"],
        NodeKind::Query => &["query"],
        NodeKind::Calculation => &["calculatedFields", "totalFields"],
        NodeKind::Setting => &["variants", "settings"],
        NodeKind::Area => &["areas"],
        NodeKind::Namespace => &["imports", "targetNamespace"],
        NodeKind::Type => &["types", "typeDetail"],
        NodeKind::Property => &["properties", "globalProperties"],
        NodeKind::Interface => &["commandInterface"],
        _ => &[],
    }
}

fn first_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| value.get(*name))
}

fn select_named<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Array(items) => items
            .iter()
            .find(|item| item_identity(item).is_some_and(|identity| identity == name)),
        Value::Object(_) if item_identity(value).is_some_and(|identity| identity == name) => {
            Some(value)
        }
        Value::String(text) if text == name => Some(value),
        _ => None,
    }
}

fn item_identity(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    ["name", "dataPath", "id", "command", "role", "kind"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
}

fn node_from_value(
    reader: LogicalReader,
    address: &QualifiedAddress,
    kind: NodeKind,
    value: &Value,
) -> NodeView {
    let title = value
        .as_object()
        .and_then(|object| {
            ["title", "synonym", "name", "dataPath", "id"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(Value::as_str))
        })
        .unwrap_or_else(|| kind.as_str());
    let props = reader_node_props(reader, kind, value);
    NodeView::new(address.to_string(), kind.as_str(), title, props)
}

fn reader_node_props(reader: LogicalReader, kind: NodeKind, value: &Value) -> Map<String, Value> {
    let keys: &[&str] = match (reader, kind) {
        (LogicalReader::Form, NodeKind::Item) => {
            &["tag", "title", "visible", "enabled", "readOnly"]
        }
        (LogicalReader::Form, NodeKind::Event) => &["handler", "callType"],
        (LogicalReader::Form, NodeKind::Attribute | NodeKind::Column) => {
            &["type", "isMain", "mainTable"]
        }
        (LogicalReader::Form, NodeKind::Parameter) => &["type", "isKey"],
        (LogicalReader::Form, NodeKind::Command) => &["shortcut"],
        (LogicalReader::Dcs, NodeKind::DataSet) => &["kind", "objectName", "dataSource"],
        (LogicalReader::Dcs, NodeKind::Field) => &["field", "title"],
        (LogicalReader::Dcs, NodeKind::Parameter) => {
            &["type", "value", "restricted", "availableAsField"]
        }
        (LogicalReader::Dcs, NodeKind::Setting) => &["presentation", "filters"],
        (LogicalReader::Mxl, NodeKind::Area) => &[
            "kind",
            "beginRow",
            "endRow",
            "beginCol",
            "endCol",
            "columnsId",
            "drawingId",
            "contentCount",
        ],
        (LogicalReader::Subsystem, _) => &[
            "synonym",
            "comment",
            "explanation",
            "picture",
            "includeInCommandInterface",
            "useOneCommand",
        ],
        (LogicalReader::Interface, NodeKind::Command) => {
            &["visible", "roleOverrides", "group", "placement"]
        }
        (LogicalReader::Xdto, NodeKind::Type) => &["kind", "abstract", "mixed"],
        (LogicalReader::Xdto, NodeKind::Property) => {
            &["type", "minOccurs", "maxOccurs", "nillable"]
        }
        _ => &[],
    };
    selected_scalar_props(value, keys)
}

fn known_reader_node(
    reader: LogicalReader,
    address: &QualifiedAddress,
    kind: NodeKind,
    value: &Value,
) -> NodeView {
    let mut node = node_from_value(reader, address, kind, value);
    let branches = known_reader_branches(reader, address, value);
    node = node.with_branches(branches);
    node
}

fn known_reader_branches(
    reader: LogicalReader,
    address: &QualifiedAddress,
    value: &Value,
) -> Vec<BranchRef> {
    let kinds: &[NodeKind] = match reader {
        LogicalReader::Configuration => &[],
        LogicalReader::Form => &[
            NodeKind::Item,
            NodeKind::Attribute,
            NodeKind::Parameter,
            NodeKind::Command,
            NodeKind::Event,
        ],
        LogicalReader::Dcs => &[
            NodeKind::DataSet,
            NodeKind::Field,
            NodeKind::Query,
            NodeKind::Calculation,
            NodeKind::Parameter,
            NodeKind::Setting,
        ],
        LogicalReader::Mxl => &[NodeKind::Area],
        LogicalReader::Subsystem => &[NodeKind::Subsystem, NodeKind::Interface],
        _ => &[],
    };
    kinds
        .iter()
        .filter_map(|kind| {
            let selected = first_field(value, reader_branch_fields(reader, *kind))?;
            let count = match selected {
                Value::Array(items) => items.len(),
                Value::Null => 0,
                _ => 1,
            };
            (count > 0).then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
        })
        .take(MAX_PROJECTED_BRANCHES)
        .collect()
}

fn safe_prop(_key: &str, value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
        Value::String(text) => text.len() <= MAX_COMPACT_PROP_BYTES,
        Value::Object(_) => false,
        Value::Array(_) => false,
    }
}

fn collection_items(
    reader: LogicalReader,
    address: &QualifiedAddress,
    kind: NodeKind,
    value: &Value,
) -> Vec<Value> {
    if matches!(kind, NodeKind::Query | NodeKind::Body) {
        return value
            .as_str()
            .unwrap_or_default()
            .lines()
            .enumerate()
            .map(|(index, text)| json!({"line": index + 1, "text": text}))
            .collect();
    }
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| collection_item(reader, address, kind, item))
            .collect(),
        Value::Object(_) => vec![collection_item(reader, address, kind, value)],
        Value::Null => Vec::new(),
        scalar => vec![data_row(scalar)],
    }
}

fn collection_item(
    reader: LogicalReader,
    address: &QualifiedAddress,
    kind: NodeKind,
    value: &Value,
) -> Value {
    let Some(name) = item_identity(value) else {
        return if value.is_object() {
            Value::Object(reader_node_props(reader, kind, value))
        } else {
            data_row(value)
        };
    };
    let at = format!("{address}.{name}");
    let Ok(at) = QualifiedAddress::parse(&at) else {
        return Value::Object(reader_node_props(reader, kind, value));
    };
    serde_json::to_value(known_reader_node(reader, &at, kind, value))
        .unwrap_or_else(|_| Value::Object(reader_node_props(reader, kind, value)))
}

fn data_row(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => json!({"value": value}),
        Value::String(text) if text.len() <= MAX_COMPACT_PROP_BYTES => json!({"value": text}),
        Value::String(_) | Value::Array(_) | Value::Object(_) => json!({"value": "<omitted>"}),
    }
}

#[cfg(test)]
mod interface_command_tests {
    use super::*;

    /// Формы ссылок взяты из настоящего `CommandInterface.xml` конфигурации
    /// класса УТ: двухсегментная общая команда, четырёхсегментная команда
    /// объекта и повисшая ссылка по идентификатору — платформа так пишет
    /// указатель на исчезнувший объект.
    /// Адрес коллекции — тот самый, который объявляет ветвь узла. Прежде он
    /// нёс `Command` дважды: разбор шёл парами `<Вид>.<Имя>`, и безымянный
    /// `Interface` забирал первый `Command` себе под имя. С правилом
    /// безымянных видов объявленная ветвь и адрес коллекции — одно и то же.
    const COLLECTION: &str = "main:Subsystem.Администрирование.Interface.Command";

    fn payload() -> Value {
        json!({
            "commandInterface": {
                "visibility": [
                    {"command": "CommonCommand.ПанельОтчетовCRM", "visible": true},
                    {"command": "Catalog.Валюты.Command.СоздатьНаОсновании", "visible": true},
                    {"command": "0:78b4152e-8a04-45f1-a08f-397423e76fc8", "visible": false}
                ]
            }
        })
    }

    fn view(at: &str) -> NodeViewData {
        let address = QualifiedAddress::parse(at).expect("адрес разбирается");
        let suffix: Vec<AddressSegment> = address
            .segments()
            .iter()
            .skip_while(|segment| segment.kind() != NodeKind::Interface)
            .cloned()
            .collect();
        project_subsystem_interface(&address, &payload(), &suffix).expect("проекция отвечает")
    }

    #[test]
    fn a_command_item_is_addressed_where_the_command_actually_lives() {
        let NodeViewData::Collection(page) = view(COLLECTION) else {
            panic!("ожидалась страница");
        };
        let page = serde_json::to_value(&page).expect("страница сериализуется");
        let addresses: Vec<&str> = page["items"]
            .as_array()
            .expect("страница несёт элементы")
            .iter()
            .filter_map(|item| item["at"].as_str())
            .collect();
        assert_eq!(
            addresses,
            vec![
                "main:Catalog.Валюты.Command.СоздатьНаОсновании",
                "main:CommonCommand.ПанельОтчетовCRM",
            ],
            "команда живёт вне интерфейса, и адресуется по своему месту"
        );
    }

    #[test]
    fn the_branch_count_equals_what_the_page_can_show() {
        let NodeViewData::Node(node) = view("main:Subsystem.Администрирование.Interface")
        else {
            panic!("ожидался узел");
        };
        let declared = serde_json::to_value(&node).expect("узел сериализуется")["branches"][0]
            ["count"]
            .as_u64()
            .expect("ветвь объявляет счёт");

        let NodeViewData::Collection(page) = view(COLLECTION) else {
            panic!("ожидалась страница");
        };
        let page = serde_json::to_value(&page).expect("страница сериализуется");
        assert_eq!(
            declared as usize,
            page["items"].as_array().expect("элементы").len(),
            "счёт ветви и страница обязаны сойтись: расхождение и было дефектом"
        );
    }

    /// Ветвь, которую объявляет узел, обязана вести к коллекции. Это и был
    /// дефект целиком: счёт называл команды, а спуск по объявленному адресу
    /// возвращал тот же узел, потому что безымянный `Interface` забирал
    /// сегмент `Command` себе под имя.
    #[test]
    fn the_branch_the_node_declares_leads_to_the_collection() {
        let NodeViewData::Node(node) = view("main:Subsystem.Администрирование.Interface")
        else {
            panic!("ожидался узел");
        };
        let declared = serde_json::to_value(&node).expect("узел сериализуется")["branches"][0]
            ["at"]
            .as_str()
            .expect("ветвь объявляет адрес")
            .to_string();
        assert_eq!(
            declared, COLLECTION,
            "объявленный адрес — это адрес коллекции"
        );

        let NodeViewData::Collection(_) = view(&declared) else {
            panic!("спуск по объявленному адресу обязан дать страницу, а не тот же узел");
        };
    }

    #[test]
    fn a_reference_without_a_logical_address_is_named_not_dropped() {
        let NodeViewData::Node(node) = view("main:Subsystem.Администрирование.Interface")
        else {
            panic!("ожидался узел");
        };
        let limits = serde_json::to_value(&node).expect("узел сериализуется")["limits"].clone();
        let named = limits[0].as_str().unwrap_or_default().to_string();
        assert!(
            named.contains("78b4152e-8a04-45f1-a08f-397423e76fc8"),
            "повисшая ссылка обязана быть названа, иначе читатель считает, что \
             видел всё: {limits}"
        );
    }
}

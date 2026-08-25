use crate::application::v13::view::ViewError;
use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::module_projection::EventProjection;
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
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
            "not_found",
            format!("form has no {} collection", first.kind().as_str()),
        )
    })?;
    if first.name().is_none() {
        if suffix.len() != 1 {
            return Err(ViewError::new(
                "not_found",
                "form collection did not consume the complete suffix",
            ));
        }
        return form_collection(address, first.kind(), values, semantic_events);
    }
    let mut kind = first.kind();
    let mut item = select_named(values, first.name().unwrap()).ok_or_else(|| {
        ViewError::new(
            "not_found",
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
                "not_found",
                format!("form node has no {} collection", kind.as_str()),
            )
        })?;
        let terminal = index + 2 == suffix.len();
        let Some(name) = segment.name() else {
            if !terminal {
                return Err(ViewError::new(
                    "not_found",
                    "form collection did not consume the complete suffix",
                ));
            }
            return form_collection(address, kind, values, semantic_events);
        };
        item = select_named(values, name).ok_or_else(|| {
            ViewError::new(
                "not_found",
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
        let owner_at = address_text
            .strip_suffix(".Event")
            .ok_or_else(|| ViewError::new("not_found", "event collection owner is invalid"))?;
        let items = form_owner_events(owner_at, events)
            .map(crate::infrastructure::v13_read::event_node_value)
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err(ViewError::new(
                "not_found",
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
                "not_found",
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
        .ok_or_else(|| ViewError::new("not_found", "DCS has no datasets"))?;
    let Some(dataset_name) = dataset_segment.name() else {
        if suffix.len() != 1 {
            return Err(ViewError::new(
                "not_found",
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
            "not_found",
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
            "not_found",
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
            "not_found",
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
            "not_found",
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
        return Err(ViewError::new("not_found", "MXL projection starts at Area"));
    }
    let areas = payload
        .get("areas")
        .ok_or_else(|| ViewError::new("not_found", "MXL has no named areas"))?;
    let Some(area_name) = area_segment.name() else {
        if suffix.len() != 1 {
            return Err(ViewError::new(
                "not_found",
                "MXL area collection cannot have a child suffix",
            ));
        }
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Area", "Area", Map::new()),
            collection_items(LogicalReader::Mxl, address, NodeKind::Area, areas),
        )));
    };
    let area = select_named(areas, area_name).ok_or_else(|| {
        ViewError::new("not_found", format!("MXL area `{area_name}` was not found"))
    })?;
    if suffix.len() == 1 {
        let mut node = node_from_value(LogicalReader::Mxl, address, NodeKind::Area, area);
        let parameter_count = area
            .get("params")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if parameter_count > 0 {
            node = node.with_branches(vec![BranchRef::new(
                format!("{}.Parameter", address),
                parameter_count,
            )]);
        }
        return Ok(NodeViewData::Node(node));
    }
    let [parameter] = &suffix[1..] else {
        return Err(ViewError::new(
            "not_found",
            "MXL projection did not consume the complete suffix",
        ));
    };
    if parameter.kind() != NodeKind::Parameter {
        return Err(ViewError::new(
            "not_found",
            "MXL area has only Parameter child nodes",
        ));
    }
    let params = area
        .get("params")
        .ok_or_else(|| ViewError::new("not_found", "MXL area has no parameters"))?;
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
            "not_found",
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
            "not_found",
            "configuration projection does not accept an address suffix",
        ));
    }
    let props = selected_scalar_props(payload, &["format", "name", "synonym"]);
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
        .ok_or_else(|| ViewError::new("not_found", "metadata branch kind is missing"))?;
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
            "provider_unavailable",
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
                "provider_unavailable",
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
            "provider_unavailable",
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
                "not_found",
                format!("typed reader has no {} projection", segment.kind().as_str()),
            )
        })?;
        if let Some(name) = segment.name() {
            current = select_named(selected, name).ok_or_else(|| {
                ViewError::new(
                    "not_found",
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
        "not_found",
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
        if let Some(details) = payload.get("details") {
            props.extend(selected_scalar_props(
                details,
                &[
                    "hierarchical",
                    "codeLength",
                    "descriptionLength",
                    "numberLength",
                    "periodicity",
                    "registerRecords",
                ],
            ));
        }
        let branches = metadata_branch_kinds()
            .iter()
            .filter_map(|kind| {
                let value = metadata_collection(payload, *kind)?;
                let count = value.as_array().map_or(0, Vec::len);
                (count > 0).then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
            })
            .collect::<Vec<_>>();
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
    let collection = metadata_collection(payload, first.kind()).ok_or_else(|| {
        ViewError::new(
            "not_found",
            format!("metadata has no {} collection", first.kind().as_str()),
        )
    })?;
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
    let name = first
        .name()
        .ok_or_else(|| ViewError::new("not_found", "metadata collection name is required"))?;
    let mut item = select_named(collection, name).ok_or_else(|| {
        ViewError::new("not_found", format!("metadata node `{name}` was not found"))
    })?;
    let mut kind = first.kind();
    for segment in &suffix[1..] {
        kind = segment.kind();
        let next = if matches!(kind, NodeKind::Attribute | NodeKind::Column) {
            item.get("attributes")
        } else {
            None
        }
        .ok_or_else(|| ViewError::new("not_found", "metadata child collection was not found"))?;
        if let Some(name) = segment.name() {
            item = select_named(next, name).ok_or_else(|| {
                ViewError::new(
                    "not_found",
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
    ]
}

fn metadata_collection(payload: &Value, kind: NodeKind) -> Option<&Value> {
    let collections = payload.get("collections")?;
    let declarations = payload.get("declarations");
    match kind {
        NodeKind::Attribute => collections.get("attributes"),
        NodeKind::StandardAttribute => declarations?.get("standardAttributes"),
        NodeKind::TabularSection => collections.get("tabularSections"),
        NodeKind::Dimension => collections.get("dimensions"),
        NodeKind::Resource => collections.get("resources"),
        NodeKind::Recalculation => collections.get("recalculations"),
        NodeKind::EnumValue => collections.get("enumValues"),
        NodeKind::Column => collections.get("columns"),
        NodeKind::Form => collections.get("forms"),
        NodeKind::Template => collections.get("templates"),
        NodeKind::Command => collections.get("commands"),
        _ => None,
    }
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
        ],
    );
    if let Some(value) = item.get("type") {
        if let Ok(rendered) = serde_json::to_string(value) {
            if rendered.len() <= MAX_COMPACT_PROP_BYTES {
                props.insert("type".to_string(), Value::String(rendered));
            }
        }
    }
    let mut branches = item
        .get("attributes")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .map(|items| {
            vec![BranchRef::new(
                format!("{}.Attribute", address),
                items.len(),
            )]
        })
        .unwrap_or_default();
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
            "not_found",
            "subsystem interface projection is missing",
        ));
    }
    let interface = payload
        .get("commandInterface")
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            ViewError::new("not_found", "subsystem has no command interface document")
        })?;
    let commands = interface_commands(interface);
    if suffix.len() > 2 {
        return Err(ViewError::new(
            "not_found",
            "subsystem interface projection did not consume the complete suffix",
        ));
    }
    if suffix.len() == 1 {
        return Ok(NodeViewData::Node(
            NodeView::new(
                address.to_string(),
                "Interface",
                "Command interface",
                Map::new(),
            )
            .with_branches(
                (!commands.is_empty())
                    .then(|| BranchRef::new(format!("{}.Command", address), commands.len()))
                    .into_iter()
                    .collect(),
            ),
        ));
    }
    let command = &suffix[1];
    match (command.kind(), command.name()) {
        (NodeKind::Command, None) => Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Command", "Commands", Map::new()),
            commands
                .iter()
                .filter_map(|item| {
                    let name = item.get("name")?.as_str()?;
                    let at = QualifiedAddress::parse(&format!("{address}.{name}")).ok()?;
                    serde_json::to_value(NodeView::new(
                        at.to_string(),
                        "Command",
                        name,
                        reader_node_props(LogicalReader::Interface, NodeKind::Command, item),
                    ))
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
                    "not_found",
                    format!("interface command `{name}` was not found"),
                )
            }),
        _ => Err(ViewError::new(
            "not_found",
            "interface projection was not found",
        )),
    }
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
            .ok_or_else(|| ViewError::new("not_found", "XDTO target namespace is absent"))?;
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
    .ok_or_else(|| ViewError::new("not_found", "XDTO projection was not found"))?;
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
    let item = select_named(selected, name)
        .ok_or_else(|| ViewError::new("not_found", format!("XDTO node `{name}` was not found")))?;
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
            "not_found",
            "XDTO property projection did not consume the complete suffix",
        ));
    }
    let properties = item
        .get("properties")
        .ok_or_else(|| ViewError::new("not_found", "XDTO type has no properties"))?;
    if property.name().is_none() {
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Property", "Properties", Map::new()),
            collection_items(LogicalReader::Xdto, address, NodeKind::Property, properties),
        )));
    }
    let property_name = property.name().unwrap();
    let property = select_named(properties, property_name).ok_or_else(|| {
        ViewError::new(
            "not_found",
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
        ],
        (LogicalReader::Subsystem, _) => &[
            "synonym",
            "comment",
            "explanation",
            "picture",
            "includeInCommandInterface",
            "useOneCommand",
        ],
        (LogicalReader::Interface, NodeKind::Command) => &["visible", "group", "placement"],
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

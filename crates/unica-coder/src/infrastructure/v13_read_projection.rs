use crate::application::v13::view::ViewError;
use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use crate::infrastructure::logical_tree::{LogicalReader, LogicalTreeRoute};
use serde_json::{json, Map, Value};

const MAX_PROJECTED_BRANCHES: usize = 256;
const MAX_COMPACT_PROP_BYTES: usize = 2_048;

pub(super) fn project_typed_payload(
    route: &LogicalTreeRoute,
    payload: Value,
) -> Result<NodeViewData, ViewError> {
    validate_reader_payload(route.reader(), &payload)?;
    let root_depth = route
        .reader_metadata_path()
        .map(|target| target.as_str().split('.').count().div_ceil(2))
        .unwrap_or_else(|| usize::from(route.reader() == LogicalReader::Configuration));
    let suffix = &route.at().segments()[root_depth.min(route.at().segments().len())..];
    match route.reader() {
        LogicalReader::Metadata => return project_metadata(route.at(), &payload, suffix),
        LogicalReader::Role => return project_role(route.at(), &payload, suffix),
        LogicalReader::Interface => {
            return project_subsystem_interface(route.at(), &payload, suffix)
        }
        LogicalReader::Xdto => return project_xdto(route.at(), &payload, suffix),
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
        return find_field(value, &["commandInterface"]);
    }
    if kind == NodeKind::Type {
        if let Some(detail) = find_field(value, &["typeDetail"]).filter(|value| !value.is_null()) {
            return Some(detail);
        }
    }
    find_field(value, reader_branch_fields(reader, kind))
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
            props.extend(compact_props(details));
        }
        let branches = metadata_branch_kinds()
            .iter()
            .filter_map(|kind| {
                let value = metadata_collection(payload, *kind)?;
                let count = value.as_array().map_or(0, Vec::len);
                (count > 0).then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
            })
            .collect();
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
        let next = if kind == NodeKind::Column {
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
    let branches = item
        .get("attributes")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .map(|items| vec![BranchRef::new(format!("{}.Column", address), items.len())])
        .unwrap_or_default();
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

fn project_role(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    let objects = role_objects(payload);
    let restricted = payload
        .get("restrictedObjects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if suffix.is_empty() {
        let mut props = selected_scalar_props(payload, &["synonym"]);
        if let Some(totals) = payload.get("totals") {
            props.extend(
                compact_props(totals)
                    .into_iter()
                    .map(|(key, value)| (format!("total{key}"), value)),
            );
        }
        let mut branches = Vec::new();
        if !objects.is_empty() {
            branches.push(BranchRef::new(format!("{}.Right", address), objects.len()));
        }
        if !restricted.is_empty() {
            branches.push(BranchRef::new(format!("{}.RLS", address), restricted.len()));
        }
        return Ok(NodeViewData::Node(
            NodeView::new(
                address.to_string(),
                NodeKind::Role.as_str(),
                payload
                    .get("synonym")
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("name").and_then(Value::as_str))
                    .unwrap_or("Role"),
                props,
            )
            .with_branches(branches),
        ));
    }
    let segment = &suffix[0];
    match (segment.kind(), segment.name()) {
        (NodeKind::Right, None) => Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Right", "Rights", Map::new()),
            objects
                .iter()
                .filter_map(|object| role_object_value(address, object))
                .collect(),
        ))),
        (NodeKind::Right, Some(name)) => objects
            .iter()
            .find(|object| object.get("name").and_then(Value::as_str) == Some(name))
            .map(|object| role_object_node(address, object))
            .map(NodeViewData::Node)
            .ok_or_else(|| {
                ViewError::new("not_found", format!("role object `{name}` was not found"))
            }),
        (NodeKind::Rls, None) => Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "RLS", "RLS", Map::new()),
            restricted
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|name| {
                    let at = QualifiedAddress::parse(&format!("{address}.{name}")).ok()?;
                    serde_json::to_value(NodeView::new(at.to_string(), "RLS", name, Map::new()))
                        .ok()
                })
                .collect(),
        ))),
        (NodeKind::Rls, Some(name))
            if restricted.iter().any(|value| value.as_str() == Some(name)) =>
        {
            Ok(NodeViewData::Node(NodeView::new(
                address.to_string(),
                "RLS",
                name,
                Map::new(),
            )))
        }
        _ => Err(ViewError::new("not_found", "role projection was not found")),
    }
}

fn role_objects(payload: &Value) -> Vec<Value> {
    let mut result = Vec::new();
    for (access, key) in [("allowed", "allowed"), ("denied", "denied")] {
        for group in payload
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let group_kind = group.get("kind").cloned().unwrap_or(Value::Null);
            for object in group
                .get("objects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let mut object = object.as_object().cloned().unwrap_or_default();
                object.insert("access".to_string(), json!(access));
                object.insert("objectKind".to_string(), group_kind.clone());
                result.push(Value::Object(object));
            }
        }
    }
    result
}

fn role_object_value(address: &QualifiedAddress, object: &Value) -> Option<Value> {
    let name = object.get("name")?.as_str()?;
    let at = QualifiedAddress::parse(&format!("{address}.{name}")).ok()?;
    serde_json::to_value(role_object_node(&at, object)).ok()
}

fn role_object_node(address: &QualifiedAddress, object: &Value) -> NodeView {
    NodeView::new(
        address.to_string(),
        "Right",
        object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Right"),
        Map::from_iter([
            (
                "access".to_string(),
                object.get("access").cloned().unwrap_or(Value::Null),
            ),
            (
                "objectKind".to_string(),
                object.get("objectKind").cloned().unwrap_or(Value::Null),
            ),
            (
                "rights".to_string(),
                Value::String(
                    serde_json::to_string(object.get("rights").unwrap_or(&Value::Null))
                        .unwrap_or_default(),
                ),
            ),
        ]),
    )
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
                        compact_props(item),
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
                    compact_props(item),
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
            props.extend(compact_props(counts));
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
        let mut props = compact_props(item);
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
        compact_props(property),
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

fn find_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    let object = value.as_object()?;
    for name in names {
        if let Some(found) = object.get(*name) {
            return Some(found);
        }
    }
    for nested in object.values().filter(|value| value.is_object()) {
        if let Some(found) = find_field(nested, names) {
            return Some(found);
        }
    }
    None
}

fn select_named<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Array(items) => items.iter().find_map(|item| {
            if item_identity(item).is_some_and(|identity| identity == name) {
                Some(item)
            } else {
                select_named(item, name)
            }
        }),
        Value::Object(object) => {
            if item_identity(value).is_some_and(|identity| identity == name) {
                Some(value)
            } else {
                object
                    .values()
                    .find_map(|nested| select_named(nested, name))
            }
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

fn node_from_value(address: &QualifiedAddress, kind: NodeKind, value: &Value) -> NodeView {
    let title = value
        .as_object()
        .and_then(|object| {
            ["title", "synonym", "name", "dataPath", "id"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(Value::as_str))
        })
        .unwrap_or_else(|| kind.as_str());
    let props = compact_props(value);
    NodeView::new(address.to_string(), kind.as_str(), title, props)
}

fn known_reader_node(
    reader: LogicalReader,
    address: &QualifiedAddress,
    kind: NodeKind,
    value: &Value,
) -> NodeView {
    let mut node = node_from_value(address, kind, value);
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
            let selected = find_field(value, reader_branch_fields(reader, *kind))?;
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

fn compact_props(value: &Value) -> Map<String, Value> {
    let Some(object) = value.as_object() else {
        return Map::from_iter([("value".to_string(), value.clone())]);
    };
    object
        .iter()
        .filter(|(key, value)| safe_prop(key, value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn safe_prop(key: &str, value: &Value) -> bool {
    if [
        "sourceSet",
        "metadataPath",
        "location",
        "path",
        "fileExists",
        "layout",
        "provider",
        "sourceState",
        "set",
        "query",
        "body",
        "text",
        "texts",
        "templates",
        "content",
        "stdout",
        "stderr",
        "nextCursor",
    ]
    .contains(&key)
    {
        return false;
    }
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
        scalar => vec![json!({"value": scalar})],
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
            Value::Object(compact_props(value))
        } else {
            json!({"value": value})
        };
    };
    let at = format!("{address}.{name}");
    let Ok(at) = QualifiedAddress::parse(&at) else {
        return Value::Object(compact_props(value));
    };
    serde_json::to_value(known_reader_node(reader, &at, kind, value))
        .unwrap_or_else(|_| Value::Object(compact_props(value)))
}

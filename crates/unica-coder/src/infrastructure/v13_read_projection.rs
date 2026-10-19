use crate::application::v13::view::ViewError;
use crate::domain::address::{member_address, AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::module_projection::EventProjection;
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use crate::domain::refusal::{RefusalCode, RefusalDetail};
use crate::infrastructure::logical_tree::{LogicalReader, LogicalTreeRoute};
use serde_json::{json, Map, Value};

mod form;
mod member_properties;
mod metadata;
mod role;
pub(crate) use metadata::borrowing_props;
use role::project_role;

const MAX_PROJECTED_BRANCHES: usize = 256;
const MAX_COMPACT_PROP_BYTES: usize = 2_048;

pub(super) fn project_typed_payload(
    route: &LogicalTreeRoute,
    payload: Value,
) -> Result<NodeViewData, ViewError> {
    project_typed_payload_inner(route, payload, None, None)
}

pub(super) fn project_typed_payload_with_form_events(
    route: &LogicalTreeRoute,
    payload: Value,
    events: &[EventProjection],
) -> Result<NodeViewData, ViewError> {
    project_typed_payload_inner(route, payload, Some(events), None)
}

pub(super) fn project_typed_payload_for_source(
    route: &LogicalTreeRoute,
    payload: Value,
    source_kind: crate::domain::project_sources::SourceSetKind,
) -> Result<NodeViewData, ViewError> {
    project_typed_payload_inner(route, payload, None, Some(source_kind))
}

fn project_typed_payload_inner(
    route: &LogicalTreeRoute,
    payload: Value,
    form_events: Option<&[EventProjection]>,
    source_kind: Option<crate::domain::project_sources::SourceSetKind>,
) -> Result<NodeViewData, ViewError> {
    validate_reader_payload(route.reader(), &payload)?;
    if route.at().form_metadata_owner().is_some() {
        return metadata::project_form_metadata(route.at(), &payload);
    }
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
    let root_depth = root_depth + route.nested_subsystems().len();
    let suffix = &route.at().segments()[root_depth.min(route.at().segments().len())..];
    match route.reader() {
        LogicalReader::Configuration => return project_configuration(route.at(), &payload, suffix),
        LogicalReader::Metadata => {
            return metadata::project_metadata_for_source(
                route.at(),
                &payload,
                suffix,
                source_kind,
            );
        }
        LogicalReader::Form => {
            return form::project_form(route.at(), &payload, suffix, form_events)
        }
        LogicalReader::Role => return project_role(route.at(), &payload, suffix),
        LogicalReader::Subsystem => return project_subsystem(route.at(), &payload, suffix),
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
    if suffix.len() == 1 && suffix[0].kind() == NodeKind::Item && suffix[0].name().is_none() {
        // Связи наборов не имеют устойчивой идентичности: они остаются
        // строками, выбираемыми составным ключом. Источники данных
        // адресуются сами: `DataSource.<имя>` (#1357).
        let rows = payload
            .get("links")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, link)| {
                let mut row = link.as_object().cloned().unwrap_or_default();
                row.insert("kind".into(), serde_json::json!("DataSetLink"));
                row.insert("index".into(), serde_json::json!(index));
                Value::Object(row)
            })
            .collect();
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Item", "Item", Map::new()),
            rows,
        )));
    }
    if let [setting, property] = suffix {
        if setting.kind() == NodeKind::Setting
            && property.kind() == NodeKind::Property
            && property.name().is_none()
        {
            let variant = setting
                .name()
                .and_then(|name| {
                    payload
                        .get("variants")
                        .and_then(|items| select_named(items, name))
                })
                .ok_or_else(|| {
                    ViewError::new(RefusalCode::NotFound, "DCS setting was not found")
                })?;
            let rows = variant
                .get("settingItems")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            return Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(address.to_string(), "Property", "Property", Map::new()),
                rows,
            )));
        }
    }
    if let [total] = suffix {
        if total.kind() == NodeKind::Total {
            return project_dcs_totals(address, payload, total.name());
        }
    }
    let dataset_segment = &suffix[0];
    if dataset_segment.kind() == NodeKind::Setting && suffix.len() > 1 {
        return project_dcs_structure(address, payload, &suffix[0], &suffix[1..]);
    }
    if dataset_segment.kind() != NodeKind::DataSet {
        return project_known_suffix(LogicalReader::Dcs, address, payload, suffix);
    }
    let mut datasets = payload
        .get("dataSets")
        .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "DCS has no datasets"))?;
    let mut consumed = 0;
    let mut current = None;
    for segment in suffix
        .iter()
        .take_while(|segment| segment.kind() == NodeKind::DataSet)
    {
        consumed += 1;
        let Some(name) = segment.name() else {
            if consumed != suffix.len() {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "dataset collection cannot have children",
                ));
            }
            return Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(address.to_string(), "DataSet", "DataSet", Map::new()),
                collection_items(LogicalReader::Dcs, address, NodeKind::DataSet, datasets),
            )));
        };
        let matches = datasets
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item_identity(item) == Some(name))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                format!("dataset {name} was not found uniquely"),
            ));
        }
        let selected = matches[0];
        current = Some(selected);
        datasets = selected.get("items").unwrap_or(&Value::Null);
    }
    let dataset =
        current.ok_or_else(|| ViewError::new(RefusalCode::NotFound, "dataset was not found"))?;
    if consumed == suffix.len() {
        let node = node_from_value(LogicalReader::Dcs, address, NodeKind::DataSet, dataset);
        let mut branches = known_reader_branches(LogicalReader::Dcs, address, dataset);
        let count = payload
            .get("parameters")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if count > 0 {
            branches.push(BranchRef::new(format!("{address}.Parameter"), count));
        }
        return Ok(NodeViewData::Node(node.with_branches(branches)));
    }
    let [detail] = &suffix[consumed..] else {
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

/// Строки структуры варианта, чей ближайший именованный предок — элемент
/// `scope` (`None` — сам вариант). Безымянный элемент прозрачен: его потомки
/// принадлежат ближайшему именованному предку, поэтому именованная группа под
/// безымянной адресуется от варианта (#1357).
pub(crate) fn dcs_structure_scope(rows: &[Value], scope: Option<usize>) -> Vec<usize> {
    let named = |index: usize| {
        rows.get(index)
            .and_then(|row| row.get("name"))
            .and_then(Value::as_str)
            .is_some_and(|name| !name.is_empty())
    };
    let parent = |index: usize| {
        rows.get(index)
            .and_then(|row| row.get("parentIndex"))
            .and_then(Value::as_u64)
            .and_then(|parent| usize::try_from(parent).ok())
    };
    (0..rows.len())
        .filter(|index| {
            let mut current = parent(*index);
            // Глубина ограничена числом строк: испорченный `parentIndex`
            // не зацикливает обход.
            for _ in 0..rows.len() {
                match current {
                    Some(ancestor) if !named(ancestor) => current = parent(ancestor),
                    _ => break,
                }
            }
            current == scope
        })
        .collect()
}

/// Имя строки структуры, которое адресует её в области: непустое и
/// единственное среди строк этой области.
fn dcs_structure_address_name<'a>(
    rows: &'a [Value],
    scope: &[usize],
    index: usize,
) -> Option<&'a str> {
    let name = rows[index]
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())?;
    (scope
        .iter()
        .filter(|other| rows[**other].get("name").and_then(Value::as_str) == Some(name))
        .count()
        == 1)
        .then_some(name)
}

/// Строка в области `scope` по имени сегмента адреса. Отсутствующее и
/// повторённое имя одинаково не дают узла: адрес не выбирает за читателя.
pub(crate) fn dcs_structure_child(
    rows: &[Value],
    scope: Option<usize>,
    name: &str,
) -> Option<usize> {
    let members = dcs_structure_scope(rows, scope);
    members
        .iter()
        .copied()
        .find(|index| dcs_structure_address_name(rows, &members, *index) == Some(name))
}

/// Пути от варианта до элементов структуры с именем `name`: цепочки имён
/// именованных предков и самого элемента. Пусто — элемента нет; в ответ
/// входят только те, чей путь адресует именно их (имя единственно на каждом
/// уровне), а `found` — сколько элементов с этим именем есть всего.
pub(crate) fn dcs_structure_paths(rows: &[Value], name: &str) -> (Vec<Vec<String>>, usize) {
    let row_name = |index: usize| {
        rows.get(index)
            .and_then(|row| row.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
    };
    let parent = |index: usize| {
        rows.get(index)
            .and_then(|row| row.get("parentIndex"))
            .and_then(Value::as_u64)
            .and_then(|parent| usize::try_from(parent).ok())
    };
    let mut found = 0;
    let mut paths = Vec::new();
    for index in (0..rows.len()).filter(|index| row_name(*index) == Some(name)) {
        found += 1;
        let mut chain = vec![name.to_string()];
        let mut current = parent(index);
        for _ in 0..rows.len() {
            let Some(ancestor) = current else { break };
            if let Some(ancestor_name) = row_name(ancestor) {
                chain.push(ancestor_name.to_string());
            }
            current = parent(ancestor);
        }
        chain.reverse();
        let mut scope = None;
        for step in &chain {
            scope = dcs_structure_child(rows, scope, step);
            if scope.is_none() {
                break;
            }
        }
        if scope == Some(index) {
            paths.push(chain);
        }
    }
    (paths, found)
}

fn dcs_structure_props(row: &Value) -> Map<String, Value> {
    row.as_object()
        .into_iter()
        .flatten()
        .filter(|(key, value)| {
            !value.is_null() && !matches!(key.as_str(), "index" | "parentIndex" | "name")
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn dcs_structure_node(address: &QualifiedAddress, rows: &[Value], index: usize) -> NodeView {
    let name = rows[index]
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Item");
    let count = dcs_structure_scope(rows, Some(index)).len();
    let branches = (count > 0)
        .then(|| BranchRef::new(format!("{address}.Item"), count))
        .into_iter()
        .collect();
    NodeView::new(
        address.to_string(),
        "Item",
        name,
        dcs_structure_props(&rows[index]),
    )
    .with_branches(branches)
}

/// Число строк структуры, которые показывает ветвь `Setting.V.Item`.
fn dcs_structure_root_count(variant: &Value) -> usize {
    variant
        .get("structureItems")
        .and_then(Value::as_array)
        .map_or(0, |rows| dcs_structure_scope(rows, None).len())
}

/// Элементы структуры варианта: `Setting.V.Item[.<имя>[.Item.<имя>…]]`.
/// Именованный элемент, единственный по имени в своей области, — узел с
/// адресом; безымянные и повторённые остаются строками данных.
fn project_dcs_structure(
    address: &QualifiedAddress,
    payload: &Value,
    setting: &AddressSegment,
    rest: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    let variant = setting
        .name()
        .and_then(|name| {
            payload
                .get("variants")
                .and_then(|variants| select_named(variants, name))
        })
        .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "DCS setting was not found"))?;
    let rows = variant
        .get("structureItems")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut scope = None;
    for (position, segment) in rest.iter().enumerate() {
        if segment.kind() != NodeKind::Item {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "DCS setting structure is addressed by `Item.<name>` segments",
            ));
        }
        let Some(name) = segment.name() else {
            if position + 1 != rest.len() {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "the structure collection has no addressed children",
                ));
            }
            let members = dcs_structure_scope(rows, scope);
            let items = members
                .iter()
                .map(|index| {
                    dcs_structure_address_name(rows, &members, *index)
                        .and_then(|name| address.member(name).ok())
                        .and_then(|at| {
                            serde_json::to_value(dcs_structure_node(&at, rows, *index)).ok()
                        })
                        .unwrap_or_else(|| rows[*index].clone())
                })
                .collect();
            return Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(address.to_string(), "Item", "Item", Map::new()),
                items,
            )));
        };
        scope = Some(dcs_structure_child(rows, scope, name).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!(
                    "DCS structure item `{name}` was not found uniquely; read the Item collection of its parent"
                ),
            )
        })?);
    }
    let index = scope.ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            "DCS setting structure is addressed by `Item.<name>` segments",
        )
    })?;
    Ok(NodeViewData::Node(dcs_structure_node(address, rows, index)))
}

/// Итоги схемы: `Template.T.Total[.<dataPath>]`. Итог адресуется путём
/// поля, только если этот путь единственный: итоги одного поля по разным
/// группировкам остаются строками и адрес за читателя не выбирают (#1357).
fn project_dcs_totals(
    address: &QualifiedAddress,
    payload: &Value,
    name: Option<&str>,
) -> Result<NodeViewData, ViewError> {
    let totals = payload
        .get("totalFields")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let path = |item: &Value| {
        item.get("dataPath")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let unique = |wanted: &str| {
        totals
            .iter()
            .filter(|item| path(item).as_deref() == Some(wanted))
            .count()
            == 1
    };
    let Some(name) = name else {
        let items = totals
            .iter()
            .map(|item| match path(item) {
                Some(data_path) if unique(&data_path) => {
                    collection_item(LogicalReader::Dcs, address, NodeKind::Total, item)
                }
                _ => Value::Object(reader_node_props_with_path(item)),
            })
            .collect();
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Total", "Total", Map::new()),
            items,
        )));
    };
    let item = totals
        .iter()
        .find(|item| path(item).as_deref() == Some(name))
        .filter(|_| unique(name))
        .ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("DCS total `{name}` was not found uniquely; read the Total collection"),
            )
        })?;
    Ok(NodeViewData::Node(known_reader_node(
        LogicalReader::Dcs,
        address,
        NodeKind::Total,
        item,
    )))
}

fn reader_node_props_with_path(item: &Value) -> Map<String, Value> {
    let mut props = reader_node_props(LogicalReader::Dcs, NodeKind::Total, item);
    if let Some(path) = item.get("dataPath") {
        props.insert("dataPath".into(), path.clone());
    }
    props
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
                    // Параметр шаблона показывается как `Имя [tpl]`: это
                    // подпись, а не имя, и адрес из неё грамматика отвергает.
                    // Такой элемент выдаётся без `at`, как и другие
                    // неадресуемые элементы (#1467).
                    let at = member_address(address, name);
                    if QualifiedAddress::parse(&at).is_ok() {
                        json!({
                            "at": at,
                            "kind": "Parameter",
                            "title": name,
                        })
                    } else {
                        json!({
                            "kind": "Parameter",
                            "title": name,
                            "addressable": false,
                        })
                    }
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
    project_configuration_with_counts(address, payload, suffix, counts)
}

pub(super) fn project_configuration_with_counts(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
    counts: std::collections::BTreeMap<String, usize>,
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
    let names = payload
        .get("registeredObjects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("kind").and_then(Value::as_str) == Some(kind.as_str()))
        .filter_map(|item| item.get("name").and_then(Value::as_str));
    project_registered_metadata_branch_names_inner(address, kind, names)
}

pub(super) fn project_registered_metadata_branch_names(
    address: &QualifiedAddress,
    names: &[String],
) -> Result<NodeViewData, ViewError> {
    let kind = address
        .segments()
        .last()
        .map(AddressSegment::kind)
        .ok_or_else(|| ViewError::new(RefusalCode::NotFound, "metadata branch kind is missing"))?;
    project_registered_metadata_branch_names_inner(address, kind, names.iter().map(String::as_str))
}

fn project_registered_metadata_branch_names_inner<'a>(
    address: &QualifiedAddress,
    kind: NodeKind,
    names: impl Iterator<Item = &'a str>,
) -> Result<NodeViewData, ViewError> {
    let items = names
        .map(|name| {
            serde_json::to_value(NodeView::new(
                member_address(address, name),
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
        LogicalReader::Metadata => metadata::PAYLOAD_KEYS,
        LogicalReader::Form => form::PAYLOAD_KEYS,
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
        (LogicalReader::Form, _) => form::branch_fields(kind),
        (LogicalReader::Dcs, NodeKind::DataSet) => &["dataSets", "items"],
        (LogicalReader::Dcs, NodeKind::Field) => &["fields"],
        (LogicalReader::Dcs, NodeKind::Query) => &["query"],
        (LogicalReader::Dcs, NodeKind::DataSource) => &["dataSources"],
        (LogicalReader::Dcs, NodeKind::CalculatedField) => &["calculatedFields"],
        (LogicalReader::Dcs, NodeKind::Total) => &["totalFields"],
        (LogicalReader::Dcs, NodeKind::Parameter) => &["parameters"],
        (LogicalReader::Dcs, NodeKind::Setting) => &["variants"],
        (LogicalReader::Mxl, NodeKind::Area) => &["areas"],
        _ => branch_field_names(kind),
    }
}

fn project_subsystem(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    let (content, dangling) = subsystem_content(address, payload);
    let children = subsystem_children(address, payload, suffix);
    if suffix.is_empty() {
        let mut branches = Vec::new();
        if !content.is_empty() {
            branches.push(BranchRef::new(
                format!("{address}.{}", NodeKind::Relation.as_str()),
                content.len(),
            ));
        }
        if !children.is_empty() {
            branches.push(BranchRef::new(
                format!("{address}.{}", NodeKind::Subsystem.as_str()),
                children.len(),
            ));
        }
        if payload
            .get("commandInterface")
            .is_some_and(|value| !value.is_null())
        {
            branches.push(BranchRef::new(
                format!("{address}.{}", NodeKind::Interface.as_str()),
                1,
            ));
        }
        return Ok(NodeViewData::Node(
            node_from_value(
                LogicalReader::Subsystem,
                address,
                NodeKind::Subsystem,
                payload,
            )
            .with_branches(branches)
            .with_limits(content_limits(&dangling)),
        ));
    }
    let first = &suffix[0];
    match first.kind() {
        // Состав — ссылки на чужие объекты, а не потомки подсистемы:
        // они живут в общей ветви ссылок, и цель читается по своему адресу.
        NodeKind::Relation => {
            if suffix.len() > 1 || first.name().is_some() {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "the relation branch lists targets; read the target at its own address",
                ));
            }
            Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(
                    address.to_string(),
                    NodeKind::Relation.as_str(),
                    "Relations",
                    Map::new(),
                )
                .with_limits(content_limits(&dangling)),
                content,
            )))
        }
        NodeKind::Subsystem if suffix.len() == 1 && first.name().is_none() => {
            Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(address.to_string(), "Subsystem", "Subsystems", Map::new()),
                children
                    .iter()
                    .filter_map(|(child, name)| {
                        serde_json::to_value(NodeView::new(
                            child.to_string(),
                            NodeKind::Subsystem.as_str(),
                            name,
                            Map::new(),
                        ))
                        .ok()
                    })
                    .collect(),
            )))
        }
        kind => Err(ViewError::new(
            RefusalCode::NotFound,
            format!("subsystem has no {} branch", kind.as_str()),
        )),
    }
}

/// Разводит состав подсистемы на адресуемые ссылки и повисшие.
///
/// `Catalog.Товары` читается как адрес в том же наборе исходников. Ссылка
/// по идентификатору или строка, которой грамматика адресов не знает, адреса
/// не имеет; её называет слот `limits`, а чтение подсистемы продолжается.
fn subsystem_content(address: &QualifiedAddress, payload: &Value) -> (Vec<Value>, Vec<String>) {
    let mut content = Vec::new();
    let mut dangling = Vec::new();
    let items = payload.get("content").and_then(Value::as_array);
    for reference in items.into_iter().flatten().filter_map(Value::as_str) {
        let target = QualifiedAddress::parse(&format!("{}:{reference}", address.source_set()))
            .ok()
            .and_then(|at| {
                let kind = at.segments().last()?.kind();
                Some((at, kind))
            });
        match target {
            Some((at, kind)) => content.push(json!({
                "relation": "content",
                "at": at.to_string(),
                "kind": kind.as_str(),
            })),
            None => dangling.push(reference.to_string()),
        }
    }
    (content, dangling)
}

/// Дочерние подсистемы с адресами. Счёт ветви и страница строятся из одного
/// списка, поэтому имя, которому не собрать адрес, не попадает ни туда, ни туда.
fn subsystem_children(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Vec<(QualifiedAddress, String)> {
    // Узел подсистемы или её ветвь `Subsystem`: адрес ветви — префикс детей.
    let branch = if suffix.is_empty() {
        format!("{address}.{}", NodeKind::Subsystem.as_str())
    } else {
        address.to_string()
    };
    payload
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|name| {
            let child = QualifiedAddress::parse(&member_address(&branch, name)).ok()?;
            Some((child, name.to_string()))
        })
        .collect()
}

fn content_limits(dangling: &[String]) -> Vec<String> {
    if dangling.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "{} элементов состава не имеют логического адреса: {}",
        dangling.len(),
        dangling.join(", ")
    )]
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
                        "at": member_address(address, group),
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
        if segment.kind() == NodeKind::Property {
            return Ok(NodeViewData::Node(
                xdto_property_node(address, name, item)
                    .with_branches(xdto_property_branch(address, item)?),
            ));
        }
        let mut props = reader_node_props(LogicalReader::Xdto, segment.kind(), item);
        if let Some(base) = item.get("base") {
            props.insert(
                "base".to_string(),
                Value::String(serde_json::to_string(base).unwrap_or_default()),
            );
        }
        let branches = xdto_property_branch(address, item)?;
        return Ok(NodeViewData::Node(
            NodeView::new(address.to_string(), segment.kind().as_str(), name, props)
                .with_branches(branches),
        ));
    }
    // Свойство любой вложенности: `Type.T.Property.A.Property.B` спускается
    // через вложенный анонимный тип (`typeDef`) каждого свойства цепочки.
    let mut current = item;
    for (index, property) in suffix[1..].iter().enumerate() {
        if property.kind() != NodeKind::Property {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "XDTO type has only Property child nodes",
            ));
        }
        let properties = xdto_nested_properties(current)?.ok_or_else(|| {
            ViewError::new(RefusalCode::NotFound, "XDTO node has no nested properties")
        })?;
        let last = index + 2 == suffix.len();
        let Some(property_name) = property.name() else {
            if !last {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "XDTO property collection cannot have children",
                ));
            }
            return Ok(NodeViewData::Collection(CollectionView::new(
                NodeView::new(address.to_string(), "Property", "Properties", Map::new()),
                collection_items(LogicalReader::Xdto, address, NodeKind::Property, properties),
            )));
        };
        current = select_named(properties, property_name).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("XDTO property `{property_name}` was not found"),
            )
        })?;
        if last {
            return Ok(NodeViewData::Node(
                xdto_property_node(address, property_name, current)
                    .with_branches(xdto_property_branch(address, current)?),
            ));
        }
    }
    Err(ViewError::new(
        RefusalCode::NotFound,
        "XDTO property projection did not consume the complete suffix",
    ))
}

/// Свойства узла XDTO: у типа — свои, у свойства — его вложенного
/// анонимного типа. `None` — вложенных свойств нет. Два вложенных
/// определения у одного свойства — конфликт источника, который не
/// разрешается выбором одного из них.
fn xdto_nested_properties(node: &Value) -> Result<Option<&Value>, ViewError> {
    if let Some(properties) = node.get("properties").filter(|value| !value.is_null()) {
        return Ok(Some(properties));
    }
    match node
        .get("typeDefs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    {
        None | Some([]) => Ok(None),
        Some([definition]) => Ok(definition
            .get("properties")
            .filter(|value| !value.is_null())),
        Some(_) => Err(ViewError::detailed(
            RefusalDetail::SourceUnreadable,
            "XDTO property has more than one nested type definition",
        )),
    }
}

fn xdto_property_branch(
    address: &QualifiedAddress,
    node: &Value,
) -> Result<Vec<BranchRef>, ViewError> {
    Ok(xdto_nested_properties(node)?
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .map(|items| vec![BranchRef::new(format!("{address}.Property"), items.len())])
        .unwrap_or_default())
}

fn xdto_property_node(address: &QualifiedAddress, name: &str, property: &Value) -> NodeView {
    let props = reader_node_props(LogicalReader::Xdto, NodeKind::Property, property);
    let limits = ["type", "reference", "bounds"]
        .into_iter()
        .filter(|key| property.get(*key).is_some() && !props.contains_key(*key))
        .map(|key| format!("XDTO property {key} exceeds the compact property limit of {MAX_COMPACT_PROP_BYTES} bytes"))
        .collect();
    NodeView::new(address.to_string(), "Property", name, props).with_limits(limits)
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
        NodeKind::DataSource => &["dataSources"],
        NodeKind::CalculatedField => &["calculatedFields"],
        NodeKind::Total => &["totalFields"],
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
    if reader == LogicalReader::Form {
        return form::node_props(kind, value);
    }
    let keys: &[&str] = match (reader, kind) {
        (LogicalReader::Dcs, NodeKind::DataSet) => &["kind", "objectName", "dataSource"],
        (LogicalReader::Dcs, NodeKind::Field) => &["field", "title"],
        (LogicalReader::Dcs, NodeKind::Parameter) => {
            &["type", "value", "restricted", "availableAsField"]
        }
        (LogicalReader::Dcs, NodeKind::Setting) => &["presentation", "filters"],
        (LogicalReader::Dcs, NodeKind::DataSource) => &["kind"],
        (LogicalReader::Dcs, NodeKind::CalculatedField) => &["expression", "title"],
        (LogicalReader::Dcs, NodeKind::Total) => &["expression", "group"],
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
            return ["type", "reference", "bounds"]
                .into_iter()
                .filter_map(|key| {
                    value
                        .get(key)
                        .and_then(|value| bounded_prop(key, value))
                        .map(|value| (key.to_string(), value))
                })
                .collect();
        }
        _ => &[],
    };
    let mut props = selected_scalar_props(value, keys);
    if reader == LogicalReader::Subsystem && kind == NodeKind::Subsystem {
        if let Some(support) = value.get("support") {
            for (source, key) in [("state", "support"), ("directEditSafe", "directEditSafe")] {
                if let Some(value) = support.get(source).filter(|value| safe_prop(key, value)) {
                    props.insert(key.to_string(), value.clone());
                }
            }
        }
    }
    if kind == NodeKind::Template {
        let format = match reader {
            LogicalReader::Dcs => Some("DataCompositionSchema"),
            LogicalReader::Mxl => Some("SpreadsheetDocument"),
            _ => None,
        };
        if let Some(format) = format {
            props.insert("format".into(), Value::String(format.into()));
        }
    }
    props
}

/// Ветви макета-схемы: адресуемые источники, наборы, вычисляемые поля,
/// итоги, параметры и варианты; связи наборов — строки ветви `Item`.
pub(crate) fn dcs_template_branches(payload: &Value) -> Vec<(NodeKind, usize)> {
    let count = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    vec![
        (NodeKind::DataSource, count("dataSources")),
        (NodeKind::DataSet, count("dataSets")),
        (NodeKind::CalculatedField, count("calculatedFields")),
        (NodeKind::Total, count("totalFields")),
        (NodeKind::Parameter, count("parameters")),
        (NodeKind::Setting, count("variants")),
        (NodeKind::Item, count("links")),
    ]
}

fn known_reader_node(
    reader: LogicalReader,
    address: &QualifiedAddress,
    kind: NodeKind,
    value: &Value,
) -> NodeView {
    let mut node = node_from_value(reader, address, kind, value);
    let mut branches = known_reader_branches(reader, address, value);
    if matches!(reader, LogicalReader::Dcs) && kind == NodeKind::Template {
        branches = dcs_template_branches(value)
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(kind, count)| BranchRef::new(format!("{address}.{}", kind.as_str()), count))
            .collect();
    }
    node = node.with_branches(branches);
    node
}

fn known_reader_branches(
    reader: LogicalReader,
    address: &QualifiedAddress,
    value: &Value,
) -> Vec<BranchRef> {
    if matches!(reader, LogicalReader::Dcs)
        && address
            .segments()
            .last()
            .is_some_and(|segment| segment.kind() == NodeKind::Setting && segment.name().is_some())
    {
        let mut branches = Vec::new();
        let structure = dcs_structure_root_count(value);
        if structure > 0 {
            branches.push(BranchRef::new(format!("{address}.Item"), structure));
        }
        if let Some(items) = value
            .get("settingItems")
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
        {
            branches.push(BranchRef::new(format!("{address}.Property"), items.len()));
        }
        return branches;
    }
    let kinds: &[NodeKind] = match reader {
        LogicalReader::Configuration => &[],
        LogicalReader::Form => form::BRANCH_KINDS,
        LogicalReader::Dcs => &[
            NodeKind::DataSet,
            NodeKind::Field,
            NodeKind::Query,
            NodeKind::Parameter,
            NodeKind::Setting,
        ],
        LogicalReader::Mxl => &[NodeKind::Area],
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
    let Ok(at) = address.member(name) else {
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
mod dcs_structure_tests {
    use super::*;
    /// Итоги одного поля по разным группировкам адреса не получают: строка
    /// ветви без `at`, а адрес `Total.<путь>` их не выбирает (#1357).
    #[test]
    fn repeated_total_paths_stay_rows_and_are_not_chosen() {
        let base = "cf:Report.F05Report.Template.F05Schema";
        let payload = json!({"totalFields":[
            {"dataPath":"Amount","expression":"Сумма(Amount)","group":null},
            {"dataPath":"Amount","expression":"Сумма(Amount)","group":"Category"},
            {"dataPath":"Owner.Count","expression":"Сумма(Owner.Count)","group":null}
        ]});
        let read = |suffix: &str| {
            let address = QualifiedAddress::parse(&format!("{base}.{suffix}")).unwrap();
            let tail: Vec<_> = address.segments().iter().skip(2).cloned().collect();
            project_dcs(&address, &payload, &tail)
        };
        let branch = serde_json::to_value(read("Total").unwrap()).unwrap();
        let items = branch["items"].as_array().unwrap();
        assert!(items[0].get("at").is_none());
        assert!(items[1].get("at").is_none());
        assert_eq!(items[1]["dataPath"], "Amount");
        assert_eq!(items[2]["at"], json!(format!("{base}.Total.Owner\\.Count")));
        let node = serde_json::to_value(read("Total.Owner\\.Count").unwrap()).unwrap();
        assert_eq!(node["props"]["expression"], "Сумма(Owner.Count)");
        assert_eq!(
            read("Total.Amount").unwrap_err().code(),
            RefusalCode::NotFound
        );
    }

    /// Именованный элемент, единственный по имени на своём уровне, — узел
    /// с адресом; безымянный прозрачен и остаётся строкой, а одноимённые
    /// элементы одного уровня адреса не получают и не выбираются (#1357).
    #[test]
    fn setting_structure_rows_never_fabricate_or_choose_named_addresses() {
        let base = "cf:Report.F05Report.Template.F05Schema";
        let rows = json!([
            {"index":0,"parentIndex":null,"axis":null,"kind":"Group","name":null,"groupBy":[]},
            {"index":1,"parentIndex":0,"axis":null,"kind":"Group","name":"Repeated","groupBy":["Amount"]},
            {"index":2,"parentIndex":null,"axis":"row","kind":"Group","name":"Repeated","groupBy":[]},
            {"index":3,"parentIndex":0,"axis":null,"kind":"Group","name":"Unique (group)","groupBy":["Category"]},
            {"index":4,"parentIndex":3,"axis":null,"kind":"Group","name":"Nested.Name","groupBy":[]}
        ]);
        let payload = json!({"variants":[{"name":"Variant","structureItems":rows}]});
        let read = |suffix: &str| {
            let address = QualifiedAddress::parse(&format!("{base}.{suffix}")).unwrap();
            let tail: Vec<_> = address.segments().iter().skip(2).cloned().collect();
            project_dcs(&address, &payload, &tail)
        };
        let projected = serde_json::to_value(read("Setting.Variant.Item").unwrap()).unwrap();
        let unique = format!("{base}.Setting.Variant.Item.Unique (group)");
        assert_eq!(
            projected["items"],
            json!([
                rows[0],
                rows[1],
                rows[2],
                {"at":unique,"kind":"Item","title":"Unique (group)","props":{"kind":"Group","groupBy":["Category"]},
                 "branches":[{"at":format!("{unique}.Item"),"count":1}]}
            ])
        );
        let setting = serde_json::to_value(read("Setting.Variant").unwrap()).unwrap();
        assert_eq!(
            setting["branches"],
            json!([{"at":format!("{base}.Setting.Variant.Item"),"count":4}])
        );
        let nested =
            serde_json::to_value(read("Setting.Variant.Item.Unique (group).Item").unwrap())
                .unwrap();
        assert_eq!(
            nested["items"][0]["at"],
            json!(format!("{unique}.Item.Nested\\.Name"))
        );
        let node = serde_json::to_value(
            read("Setting.Variant.Item.Unique (group).Item.Nested\\.Name").unwrap(),
        )
        .unwrap();
        assert_eq!(node["title"], "Nested.Name");
        for suffix in [
            "Setting.Variant.Item.Repeated",
            "Setting.Variant.Item.Group",
            "Setting.Variant.Item.Nested\\.Name",
            "Setting.Variant.Item.Unique (group).Item.Repeated",
            "Setting.Variant.Item.Repeated.Item",
            "Setting.Missing.Item",
        ] {
            assert_eq!(
                read(suffix).unwrap_err().code(),
                RefusalCode::NotFound,
                "{suffix}"
            );
        }
        // Пути элементов для отказа по снятому аргументу: имя на двух
        // уровнях даёт пути обоих адресуемых, одноимённые на одном уровне —
        // ни одного.
        let rows = rows.as_array().unwrap();
        assert_eq!(
            dcs_structure_paths(rows, "Nested.Name"),
            (
                vec![vec![
                    "Unique (group)".to_string(),
                    "Nested.Name".to_string()
                ]],
                1
            )
        );
        assert_eq!(dcs_structure_paths(rows, "Repeated"), (Vec::new(), 2));
        assert_eq!(dcs_structure_paths(rows, "Missing"), (Vec::new(), 0));
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

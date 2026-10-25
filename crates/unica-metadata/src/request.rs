//! Typed metadata request decoding and the canonical argument schemas.
use crate::contracts::{
    metadata_element_input_schema, metadata_host_element_schema,
    metadata_predefined_element_input_schema,
};
use crate::*;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use unica_address::source_target::{
    metadata_address_kind_spellings, MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use unica_operation_contract::ArgumentShape;
const INFO_SECTIONS: &[(&str, MetaInfoSection)] = &[
    ("roles", MetaInfoSection::Roles),
    ("subscriptions", MetaInfoSection::Subscriptions),
    ("functionalOptions", MetaInfoSection::FunctionalOptions),
    ("predefinedItems", MetaInfoSection::PredefinedItems),
];
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaFailure {
    pub diagnostics: Vec<MetaDiagnostic>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataRequest {
    Info(MetaInfoRequest),
    Add(MetaAddRequest),
    Edit(MetaEditRequest),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaEditRequest {
    pub source_set: String,
    pub metadata_path: MetadataAddress,
    pub operations: Vec<MetaEditOperation>,
    pub dry_run: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaAddRequest {
    pub source_set: String,
    pub kind: MetadataKind,
    pub name: String,
    pub operations: Vec<MetaEditOperation>,
    pub dry_run: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaInfoRequest {
    pub source_set: String,
    pub metadata_path: MetadataAddress,
    pub sections: Vec<MetaInfoSection>,
    pub limit: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaInfoSection {
    Roles,
    Subscriptions,
    FunctionalOptions,
    PredefinedItems,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataOperation {
    Info,
    Add,
    Edit,
}
pub fn parse_metadata_request(
    operation: MetadataOperation,
    args: &Map<String, Value>,
) -> Result<MetadataRequest, MetaFailure> {
    validate_metadata_argument_shape(operation, args)?;
    parse_metadata_request_after_shape(operation, args)
}
pub fn parse_metadata_request_after_shape(
    operation: MetadataOperation,
    args: &Map<String, Value>,
) -> Result<MetadataRequest, MetaFailure> {
    let source_set = required_string(args, "sourceSet")?;
    match operation {
        MetadataOperation::Info => {
            let metadata_path = required_metadata_path(args, "metadataPath")?;
            let sections = parse_info_sections(args.get("sections"))?;
            if sections.contains(&MetaInfoSection::PredefinedItems) {
                let kind = metadata_kind_for_address(&metadata_path)?;
                if !crate::metadata_kind_collections(kind)
                    .contains(&MetaCollection::PredefinedItems)
                {
                    return Err(MetaDiagnostic::error(
                        MetaDiagnosticCode::UnsupportedKind,
                        format!(
                            "predefinedItems is not supported for metadata kind `{}`",
                            kind.as_str()
                        ),
                    )
                    .with_field("sections")
                    .with_metadata_path(metadata_path)
                    .into());
                }
            }
            let limit = parse_bounded_usize(args.get("limit"), "limit", 20, 50)?;
            Ok(MetadataRequest::Info(MetaInfoRequest {
                source_set,
                metadata_path,
                sections,
                limit,
            }))
        }
        MetadataOperation::Add => {
            let raw_kind = required_string(args, "kind")?;
            let kind = MetadataKind::parse(&raw_kind)?;
            let name = required_string(args, "name")?;
            let owner = metadata_owner(kind, &name)?;
            let operations = parse_operations(args.get("operations"), false, kind, &owner)?;
            let dry_run = optional_bool(args, "dryRun", true)?;
            Ok(MetadataRequest::Add(MetaAddRequest {
                source_set,
                kind,
                name,
                operations,
                dry_run,
            }))
        }
        MetadataOperation::Edit => {
            let metadata_path = required_metadata_path(args, "metadataPath")?;
            let kind = metadata_kind_for_address(&metadata_path)?;
            let operations = parse_operations(args.get("operations"), true, kind, &metadata_path)?;
            let dry_run = optional_bool(args, "dryRun", true)?;
            Ok(MetadataRequest::Edit(MetaEditRequest {
                source_set,
                metadata_path,
                operations,
                dry_run,
            }))
        }
    }
}
pub fn validate_metadata_argument_shape(
    operation: MetadataOperation,
    args: &Map<String, Value>,
) -> Result<(), MetaFailure> {
    reject_unknown_top_level(operation, args)?;
    let schema = metadata_input_schema(operation);
    let properties = schema["properties"]
        .as_object()
        .expect("metadata schema properties are an object");
    for (name, value) in args {
        let expected = properties[name]["type"]
            .as_str()
            .expect("metadata top-level property declares one JSON type");
        let matches = match expected {
            "array" => value.is_array(),
            "boolean" => value.is_boolean(),
            "integer" => value.as_number().is_some_and(|number| {
                number.as_i64().is_some()
                    || number.as_u64().is_some()
                    || number
                        .as_f64()
                        .is_some_and(|number| number.is_finite() && number.fract() == 0.0)
            }),
            "object" => value.is_object(),
            "string" => value.is_string(),
            other => panic!("unsupported metadata top-level JSON type: {other}"),
        };
        if !matches {
            return Err(invalid(name, format!("`{name}` must be {expected}")).into());
        }
    }
    Ok(())
}
pub fn parse_operations(
    value: Option<&Value>,
    required: bool,
    kind: MetadataKind,
    owner: &MetadataAddress,
) -> Result<Vec<MetaEditOperation>, MetaFailure> {
    let Some(value) = value else {
        return if required {
            Err(invalid("operations", "`operations` is required").into())
        } else {
            Ok(Vec::new())
        };
    };
    let raw_operations = value
        .as_array()
        .ok_or_else(|| invalid("operations", "`operations` must be an array"))?;
    if raw_operations.is_empty() {
        return Err(invalid("operations", "operations must not be empty").into());
    }
    raw_operations
        .iter()
        .enumerate()
        .map(|(index, raw_operation)| {
            let operation = parse_edit_operation(raw_operation, kind)
                .map_err(|diagnostic| diagnostic.with_operation_index(index))?;
            validate_metadata_operation_capabilities(kind, owner, &operation)
                .map_err(|diagnostic| diagnostic.with_operation_index(index))?;
            Ok(operation)
        })
        .collect::<Result<Vec<_>, MetaDiagnostic>>()
        .map_err(Into::into)
}
pub fn metadata_owner(kind: MetadataKind, name: &str) -> Result<MetadataAddress, MetaFailure> {
    let owner = parse_address(&format!("{}.{name}", kind.as_str()), "name")?;
    if owner.segments().count() != 2 {
        return Err(invalid("name", "metadata object name must be one address segment").into());
    }
    Ok(owner)
}
pub fn reject_unknown_top_level(
    operation: MetadataOperation,
    args: &Map<String, Value>,
) -> Result<(), MetaDiagnostic> {
    let allowed = metadata_top_level_fields(operation);
    if let Some(unknown) = args.keys().find(|name| !allowed.contains(&name.as_str())) {
        let accepted = allowed.join(", ");
        return Err(invalid(
            unknown,
            format!(
                "metadata operation does not accept argument `{unknown}`; accepted arguments: {accepted}"
            ),
        ));
    }
    Ok(())
}
pub fn metadata_top_level_fields(operation: MetadataOperation) -> &'static [&'static str] {
    match operation {
        MetadataOperation::Info => &["cwd", "sourceSet", "metadataPath", "sections", "limit"],
        MetadataOperation::Add => &["cwd", "sourceSet", "kind", "name", "operations", "dryRun"],
        MetadataOperation::Edit => &["cwd", "sourceSet", "metadataPath", "operations", "dryRun"],
    }
}
pub fn parse_info_sections(value: Option<&Value>) -> Result<Vec<MetaInfoSection>, MetaDiagnostic> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| invalid("sections", "sections must be an array"))?;
    let mut sections = Vec::with_capacity(values.len());
    for value in values {
        let name = value
            .as_str()
            .ok_or_else(|| invalid("sections", "section names must be strings"))?;
        let section = registry_value(INFO_SECTIONS, name)
            .ok_or_else(|| invalid("sections", format!("unknown related section `{name}`")))?;
        if sections.contains(&section) {
            return Err(invalid(
                "sections",
                format!("section `{name}` is duplicated"),
            ));
        }
        sections.push(section);
    }
    Ok(sections)
}
pub fn parse_edit_operation(
    value: &Value,
    kind: MetadataKind,
) -> Result<MetaEditOperation, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("operations", "each operation must be an object"))?;
    reject_unknown_fields(object, operation_property_names(), "")?;
    let tag = required_string_diagnostic(object, "op")?;
    let operation = MetaEditOperationTag::parse(&tag)?;
    match operation {
        MetaEditOperationTag::SetProperties => {
            reject_forbidden_fields(
                object,
                &[
                    "collection",
                    "scope",
                    "elements",
                    "names",
                    "ids",
                    "relation",
                    "mode",
                    "targets",
                    "lang",
                ],
            )?;
            let values = parse_property_changes(required_object(object, "values")?, kind)?;
            Ok(MetaEditOperation::SetProperties { values })
        }
        MetaEditOperationTag::Add => {
            reject_forbidden_fields(
                object,
                &[
                    "values", "names", "ids", "relation", "mode", "targets", "lang",
                ],
            )?;
            let collection = parse_collection(object, kind)?;
            if collection == MetaCollection::PredefinedItems {
                reject_forbidden_fields(object, &["scope"])?;
                let elements = required_array(object, "elements")?
                    .iter()
                    .enumerate()
                    .map(|(index, element)| parse_predefined_add(element, index))
                    .collect::<Result<Vec<_>, _>>()?;
                return MetaEditOperation::add_predefined_items(elements);
            }
            let scope = parse_scope(object.get("scope"))?;
            let elements = required_array(object, "elements")?
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    parse_element_input(element, &format!("elements[{index}]"), true)
                })
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::add(collection, scope, elements)
        }
        MetaEditOperationTag::Update => {
            reject_forbidden_fields(
                object,
                &[
                    "values", "names", "ids", "relation", "mode", "targets", "lang",
                ],
            )?;
            let collection = parse_collection(object, kind)?;
            if collection == MetaCollection::PredefinedItems {
                reject_forbidden_fields(object, &["scope"])?;
                let elements = required_array(object, "elements")?
                    .iter()
                    .enumerate()
                    .map(|(index, element)| parse_predefined_update(element, index))
                    .collect::<Result<Vec<_>, _>>()?;
                return MetaEditOperation::update_predefined_items(elements);
            }
            let scope = parse_scope(object.get("scope"))?;
            let elements = required_array(object, "elements")?
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    parse_element_update(element, &format!("elements[{index}]"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::update(collection, scope, elements)
        }
        MetaEditOperationTag::Remove => {
            reject_forbidden_fields(
                object,
                &["values", "elements", "relation", "mode", "targets", "lang"],
            )?;
            let collection = parse_collection(object, kind)?;
            if collection == MetaCollection::PredefinedItems {
                reject_forbidden_fields(object, &["scope", "names"])?;
                let ids = required_array(object, "ids")?
                    .iter()
                    .enumerate()
                    .map(|(index, value)| nonempty_string(value, &format!("ids[{index}]")))
                    .collect::<Result<Vec<_>, _>>()?;
                return MetaEditOperation::remove_predefined_items(ids);
            }
            reject_forbidden_fields(object, &["ids"])?;
            let scope = parse_scope(object.get("scope"))?;
            let names = required_array(object, "names")?
                .iter()
                .enumerate()
                .map(|(index, value)| nonempty_string(value, &format!("names[{index}]")))
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::remove(collection, scope, names)
        }
        MetaEditOperationTag::AddHelp => {
            reject_forbidden_fields(
                object,
                &[
                    "values",
                    "collection",
                    "scope",
                    "elements",
                    "names",
                    "ids",
                    "relation",
                    "mode",
                    "targets",
                ],
            )?;
            let lang = match object.get("lang") {
                None => "ru".to_string(),
                Some(value) => {
                    let lang = value
                        .as_str()
                        .ok_or_else(|| invalid("lang", "`lang` must be a string"))?;
                    if lang.is_empty()
                        || lang
                            .chars()
                            .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
                    {
                        return Err(invalid(
                            "lang",
                            "`lang` must be a simple language code, for example ru or en",
                        ));
                    }
                    lang.to_string()
                }
            };
            Ok(MetaEditOperation::AddHelp { lang })
        }
        MetaEditOperationTag::EditRelations => {
            reject_forbidden_fields(
                object,
                &[
                    "values",
                    "collection",
                    "scope",
                    "elements",
                    "names",
                    "ids",
                    "lang",
                ],
            )?;
            let reader =
                MetadataArgumentReader::new(object, metadata_relation_operation_input_schema(), "");
            let relation = reader.read("relation", |object, key, field| {
                MetaRelation::parse(&required_string_at(object, key, field)?)
            })?;
            let mode = reader.read("mode", |object, key, field| {
                RelationEditMode::parse(&required_string_at(object, key, field)?)
            })?;
            let targets = reader
                .read("targets", required_array_at)?
                .iter()
                .enumerate()
                .map(|(index, target)| {
                    let field = format!("targets[{index}]");
                    match relation {
                        MetaRelation::InputByString => {
                            parse_field_reference(target, &field).map(MetaRelationTarget::Field)
                        }
                        MetaRelation::Source => {
                            parse_event_source(target, &field).map(MetaRelationTarget::EventSource)
                        }
                        MetaRelation::Owners
                        | MetaRelation::RegisterRecords
                        | MetaRelation::BasedOn => {
                            parse_reference(target, &field).map(MetaRelationTarget::Object)
                        }
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::edit_relation_targets(relation, mode, targets)
        }
    }
}
pub fn operation_property_names() -> &'static [&'static str] {
    &[
        "op",
        "values",
        "collection",
        "scope",
        "elements",
        "names",
        "ids",
        "relation",
        "mode",
        "targets",
        "lang",
    ]
}
pub fn parse_predefined_add(
    value: &Value,
    index: usize,
) -> Result<MetaPredefinedItemAdd, MetaDiagnostic> {
    let prefix = format!("elements[{index}]");
    let schema = metadata_predefined_element_input_schema(false);
    let object = predefined_object(value, &prefix, ArgumentShape::new(schema))?;
    let reader = MetadataArgumentReader::new(object, schema, &prefix);
    Ok(MetaPredefinedItemAdd {
        id: reader.read("id", |object, key, _| canonical_uuid(object, key, &prefix))?,
        name: reader.read("name", required_string_at)?,
        fields: parse_predefined_fields(&reader)?,
    })
}
pub fn parse_predefined_update(
    value: &Value,
    index: usize,
) -> Result<MetaPredefinedItemUpdate, MetaDiagnostic> {
    let prefix = format!("elements[{index}]");
    let schema = metadata_predefined_element_input_schema(true);
    let object = predefined_object(value, &prefix, ArgumentShape::new(schema))?;
    let reader = MetadataArgumentReader::new(object, schema, &prefix);
    Ok(MetaPredefinedItemUpdate {
        id: reader.read("id", |object, key, _| canonical_uuid(object, key, &prefix))?,
        name: reader.read("name", |object, key, field| {
            object
                .get(key)
                .map(|value| nonempty_string(value, field))
                .transpose()
        })?,
        fields: parse_predefined_fields(&reader)?,
    })
}
pub fn predefined_object<'a>(
    value: &'a Value,
    prefix: &str,
    shape: ArgumentShape<'_>,
) -> Result<&'a Map<String, Value>, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(prefix, "predefined item must be an object"))?;
    reject_unknown_shape_fields(object, shape, &format!("{prefix}."))?;
    Ok(object)
}
pub fn canonical_uuid(
    object: &Map<String, Value>,
    name: &str,
    prefix: &str,
) -> Result<String, MetaDiagnostic> {
    let field = format!("{prefix}.{name}");
    let raw = required_string_at(object, name, &field)?;
    crate::canonical_predefined_uuid(&raw)
        .ok_or_else(|| invalid(field, "predefined item id must be a UUID"))
}
fn parse_predefined_fields(
    reader: &MetadataArgumentReader<'_>,
) -> Result<MetaPredefinedFields, MetaDiagnostic> {
    let optional_string = |name: &str| {
        reader.read(name, |object, key, field| {
            object
                .get(key)
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| invalid(field, "field must be a string"))
                })
                .transpose()
        })
    };
    let optional_bool = |name: &str| {
        reader.read(name, |object, key, field| {
            object
                .get(key)
                .map(|value| {
                    value
                        .as_bool()
                        .ok_or_else(|| invalid(field, "field must be a boolean"))
                })
                .transpose()
        })
    };
    Ok(MetaPredefinedFields {
        code: optional_string("code")?,
        description: optional_string("description")?,
        is_folder: optional_bool("isFolder")?,
        r#type: reader.read("type", |object, key, field| {
            object
                .get(key)
                .map(|value| parse_metadata_type(value, field))
                .transpose()
        })?,
        account_type: reader.read("accountType", |object, key, field| {
            object
                .get(key)
                .map(|value| {
                    let value = value
                        .as_str()
                        .ok_or_else(|| invalid(field, "accountType must be a string"))?;
                    MetaPredefinedAccountType::parse(value, field)
                })
                .transpose()
        })?,
        off_balance: optional_bool("offBalance")?,
        order: optional_string("order")?,
        accounting_flags: reader.read("accountingFlags", |object, key, field| {
            object
                .get(key)
                .map(|value| {
                    parse_predefined_flags(
                        value,
                        field,
                        reader.shape.property(key).expect("predefined flags"),
                    )
                })
                .transpose()
        })?,
        ext_dimension_types: reader.read("extDimensionTypes", |object, key, field| {
            object
                .get(key)
                .map(|value| {
                    parse_ext_dimension_types(
                        value,
                        field,
                        reader
                            .shape
                            .property(key)
                            .expect("predefined ext dimension types"),
                    )
                })
                .transpose()
        })?,
        action_period_is_base: optional_bool("actionPeriodIsBase")?,
    })
}
pub fn parse_predefined_flags(
    value: &Value,
    field: &str,
    shape: ArgumentShape<'_>,
) -> Result<BTreeMap<String, bool>, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "accounting flags must be an object of name:boolean"))?;
    let mut flags = BTreeMap::new();
    for (name, value) in object {
        if name.is_empty() {
            return Err(invalid(field, "accounting flag name must not be empty"));
        }
        let converted = value
            .as_bool()
            .ok_or_else(|| invalid(format!("{field}.{name}"), "accounting flag must be boolean"))?;
        shape
            .additional_properties()
            .expect("predefined accounting flag value")
            .validate_field(value, &format!("{field}.{name}"))
            .map_err(|error| invalid(error.path, error.message))?;
        flags.insert(name.clone(), converted);
    }
    Ok(flags)
}
pub fn parse_ext_dimension_types(
    value: &Value,
    field: &str,
    shape: ArgumentShape<'_>,
) -> Result<Vec<MetaPredefinedExtDimensionType>, MetaDiagnostic> {
    let values = value
        .as_array()
        .ok_or_else(|| invalid(field, "extDimensionTypes must be an array"))?;
    let mut result = Vec::with_capacity(values.len());
    let mut names = std::collections::HashSet::new();
    for (index, value) in values.iter().enumerate() {
        let item_field = format!("{field}[{index}]");
        let object = value
            .as_object()
            .ok_or_else(|| invalid(&item_field, "ext-dimension type must be an object"))?;
        let reader = MetadataArgumentReader {
            object,
            shape: shape.items().expect("predefined ext dimension type"),
            field: &item_field,
        };
        reader.reject_unknown()?;
        let name = reader.read("name", required_string_at)?;
        if !names.insert(name.to_lowercase()) {
            return Err(invalid(
                format!("{item_field}.name"),
                "ext-dimension type name is duplicated",
            ));
        }
        let turnover = reader.read("turnover", |object, key, field| {
            object
                .get(key)
                .map(|value| {
                    value
                        .as_bool()
                        .ok_or_else(|| invalid(field, "turnover must be boolean"))
                })
                .transpose()
        })?;
        let accounting_flags = reader.read("accountingFlags", |object, key, field| {
            object
                .get(key)
                .map(|value| {
                    parse_predefined_flags(
                        value,
                        field,
                        reader
                            .shape
                            .property(key)
                            .expect("ext dimension accounting flags"),
                    )
                })
                .transpose()
        })?;
        result.push(MetaPredefinedExtDimensionType {
            name,
            turnover,
            accounting_flags,
        });
    }
    Ok(result)
}
pub fn parse_collection(
    object: &Map<String, Value>,
    kind: MetadataKind,
) -> Result<MetaCollection, MetaDiagnostic> {
    let name = required_string_diagnostic(object, "collection")?;
    let collection = MetaCollection::parse(&name)?;
    validate_metadata_kind_collection(kind, collection)?;
    Ok(collection)
}
pub fn parse_scope(value: Option<&Value>) -> Result<Option<MetaScope>, MetaDiagnostic> {
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| invalid("scope", "scope must be an object"))?;
    reject_unknown_fields(object, &["tabularSection"], "scope.")?;
    Ok(Some(MetaScope {
        tabular_section: required_string_at(object, "tabularSection", "scope.tabularSection")?,
    }))
}
pub fn parse_property_changes(
    object: &Map<String, Value>,
    kind: MetadataKind,
) -> Result<MetaPropertyChanges, MetaDiagnostic> {
    if object.is_empty() {
        return Err(invalid("values", "property changes must not be empty"));
    }
    let mut inputs = Vec::with_capacity(object.len());
    for (name, value) in object {
        let field = format!("values.{name}");
        let spec = METADATA_PROPERTY_SPECS
            .iter()
            .find(|spec| spec.public_name == name)
            .ok_or_else(|| invalid(&field, unknown_metadata_property_message(kind, name)))?;
        let value = match spec.value_kind {
            MetaPropertyValueKind::Type => {
                MetaPropertyValue::Type(parse_metadata_type(value, &field)?)
            }
            MetaPropertyValueKind::String => value
                .as_str()
                .map(|value| MetaPropertyValue::String(value.to_string()))
                .ok_or_else(|| invalid(&field, "property value must be a string"))?,
            MetaPropertyValueKind::Boolean => value
                .as_bool()
                .map(MetaPropertyValue::Boolean)
                .ok_or_else(|| invalid(&field, "property value must be a boolean"))?,
            MetaPropertyValueKind::UnsignedInteger => json_u32(value)
                .map(MetaPropertyValue::UnsignedInteger)
                .ok_or_else(|| {
                    invalid(&field, "property value must be an unsigned 32-bit integer")
                })?,
        };
        inputs.push(MetaPropertyInput::new(name, value));
    }
    MetaPropertyChanges::convert(kind, inputs)
}
pub fn reject_unknown_shape_fields(
    object: &Map<String, Value>,
    shape: ArgumentShape<'_>,
    prefix: &str,
) -> Result<(), MetaDiagnostic> {
    if let Some(name) = object.keys().find(|name| shape.property(name).is_none()) {
        return Err(invalid(
            format!("{prefix}{name}"),
            format!("unknown field `{name}`"),
        ));
    }
    Ok(())
}
pub fn parse_element_input(
    value: &Value,
    field: &str,
    allows_attributes: bool,
) -> Result<MetaElementInput, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "element must be an object"))?;
    let reader = MetadataArgumentReader::new(
        object,
        metadata_element_input_schema(false, allows_attributes),
        field,
    );
    reader.reject_unknown()?;
    let attributes_field = format!("{field}.attributes");
    let attributes = reader.read("attributes", |object, key, _| {
        object
            .get(key)
            .map(|_| {
                let attributes = required_array_at(object, "attributes", &attributes_field)?;
                if attributes.is_empty() {
                    return Err(invalid(
                        &attributes_field,
                        "nested attributes must not be empty",
                    ));
                }
                attributes
                    .iter()
                    .enumerate()
                    .map(|(index, element)| {
                        parse_element_input(element, &format!("{attributes_field}[{index}]"), false)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
    })?;
    Ok(MetaElementInput {
        name: reader.read("name", required_string_at)?,
        synonym: reader.read("synonym", optional_string_at)?,
        comment: reader.read("comment", optional_string_at)?,
        r#type: reader.read("type", |object, key, field| {
            object
                .get(key)
                .map(|value| parse_metadata_type(value, field))
                .transpose()
        })?,
        required: reader.read("required", optional_bool_at)?,
        fill_value: reader.read("fillValue", |object, key, field| {
            object
                .get(key)
                .map(|value| parse_fill_value(value, field))
                .transpose()
        })?,
        attributes,
        position: reader.read("position", |object, key, field| {
            object
                .get(key)
                .map(|value| parse_position(value, field))
                .transpose()
        })?,
        template_type: reader.read("templateType", |object, key, field| {
            object
                .get(key)
                .map(|value| {
                    let value = value
                        .as_str()
                        .ok_or_else(|| invalid(field, "`templateType` must be a string"))?;
                    MetaTemplateKind::parse(value)
                        .map_err(|diagnostic| diagnostic.with_field(field))
                })
                .transpose()
        })?,
        indexing: reader.read("indexing", optional_string_at)?,
    })
}
pub fn parse_element_update(
    value: &Value,
    field: &str,
) -> Result<MetaElementUpdateInput, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "element must be an object"))?;
    let reader =
        MetadataArgumentReader::new(object, metadata_element_input_schema(true, false), field);
    reader.reject_unknown()?;
    Ok(MetaElementUpdateInput {
        indexing: reader.read("indexing", optional_string_at)?,
        deny_incomplete_values: reader.read("denyIncompleteValues", optional_bool_at)?,
        name: reader.read("name", required_string_at)?,
        new_name: reader.read("newName", optional_string_at)?,
        synonym: reader.read("synonym", optional_string_at)?,
        comment: reader.read("comment", optional_string_at)?,
        r#type: reader.read("type", |object, key, field| {
            object
                .get(key)
                .map(|value| parse_metadata_type(value, field))
                .transpose()
        })?,
        required: reader.read("required", optional_bool_at)?,
        fill_value: reader.read("fillValue", |object, key, field| {
            object
                .get(key)
                .map(|value| parse_fill_value(value, field))
                .transpose()
        })?,
        position: reader.read("position", |object, key, field| {
            object
                .get(key)
                .map(|value| parse_position(value, field))
                .transpose()
        })?,
    })
}
pub fn parse_position(value: &Value, field: &str) -> Result<MetaPosition, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "position must be an object"))?;
    let schema = position_schema();
    let reader = MetadataArgumentReader::new(object, &schema, field);
    reader.reject_unknown()?;
    MetaPosition::new_at(
        reader.read("before", optional_string_at)?,
        reader.read("after", optional_string_at)?,
        field,
    )
}
pub fn parse_metadata_type(value: &Value, field: &str) -> Result<MetadataType, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "type must be a structured object"))?;
    let schema = metadata_type_schema();
    let reader = MetadataArgumentReader::new(object, &schema, field);
    reader.reject_unknown()?;
    let variants = reader.read("variants", |object, key, path| {
        required_array_at(object, key, path).and_then(|values| {
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    parse_type_variant(
                        value,
                        index,
                        field,
                        &schema["properties"]["variants"]["items"],
                    )
                })
                .collect::<Result<Vec<_>, _>>()
        })
    })?;
    MetadataType::new(variants).map_err(|mut diagnostic| {
        if let Some(domain_field) = diagnostic.field.as_deref() {
            if let Some(suffix) = domain_field.strip_prefix("type") {
                diagnostic.field = Some(format!("{field}{suffix}"));
            }
        }
        diagnostic
    })
}
pub fn parse_type_variant(
    value: &Value,
    index: usize,
    field: &str,
    schema: &Value,
) -> Result<MetadataTypeVariant, MetaDiagnostic> {
    let variant_field = format!("{field}.variants[{index}]");
    let object = value
        .as_object()
        .ok_or_else(|| invalid(&variant_field, "type variant must be an object"))?;
    let kind = required_string_at(object, "kind", &format!("{variant_field}.kind"))?;
    let variant = match kind.as_str() {
        "string" => MetadataTypeVariant::String {
            length: required_u32(object, "length", &variant_field)?,
            allowed_length: match required_string_at(
                object,
                "allowedLength",
                &format!("{variant_field}.allowedLength"),
            )?
            .as_str()
            {
                "variable" => StringLengthMode::Variable,
                "fixed" => StringLengthMode::Fixed,
                value => {
                    return Err(invalid(
                        format!("{variant_field}.allowedLength"),
                        format!("unsupported allowedLength `{value}`"),
                    ))
                }
            },
        },
        "number" => MetadataTypeVariant::Number {
            digits: required_u32(object, "digits", &variant_field)?,
            fraction: required_u32(object, "fraction", &variant_field)?,
            sign: match required_string_at(object, "sign", &format!("{variant_field}.sign"))?
                .as_str()
            {
                "any" => NumberSign::Any,
                "nonNegative" => NumberSign::NonNegative,
                value => {
                    return Err(invalid(
                        format!("{variant_field}.sign"),
                        format!("unsupported number sign `{value}`"),
                    ))
                }
            },
        },
        "boolean" => MetadataTypeVariant::Boolean,
        "uuid" => MetadataTypeVariant::Uuid,
        "date" => MetadataTypeVariant::Date {
            fractions: match required_string_at(
                object,
                "fractions",
                &format!("{variant_field}.fractions"),
            )?
            .as_str()
            {
                "date" => DateFractions::Date,
                "time" => DateFractions::Time,
                "dateTime" => DateFractions::DateTime,
                value => {
                    return Err(invalid(
                        format!("{variant_field}.fractions"),
                        format!("unsupported date fractions `{value}`"),
                    ))
                }
            },
        },
        "binaryData" => MetadataTypeVariant::BinaryData {
            length: required_u32(object, "length", &variant_field)?,
            allowed_length: match required_string_at(
                object,
                "allowedLength",
                &format!("{variant_field}.allowedLength"),
            )?
            .as_str()
            {
                "variable" => StringLengthMode::Variable,
                "fixed" => StringLengthMode::Fixed,
                value => {
                    return Err(invalid(
                        format!("{variant_field}.allowedLength"),
                        format!("unsupported allowedLength `{value}`"),
                    ))
                }
            },
        },
        "valueStorage" => MetadataTypeVariant::ValueStorage,
        "reference" | "definedType" => {
            let raw = required_string_at(
                object,
                "metadataPath",
                &format!("{variant_field}.metadataPath"),
            )?;
            let metadata_path = parse_address(&raw, &format!("{variant_field}.metadataPath"))?;
            if kind == "reference" {
                MetadataTypeVariant::Reference { metadata_path }
            } else {
                MetadataTypeVariant::DefinedType { metadata_path }
            }
        }
        _ => {
            return Err(invalid(
                format!("{variant_field}.kind"),
                format!("unsupported type kind `{kind}`"),
            ))
        }
    };
    let shape = ArgumentShape::new(schema)
        .variant("kind", &kind)
        .expect("converted metadata type variant");
    reject_unknown_shape_fields(object, shape, &format!("{variant_field}."))?;
    for (key, value) in object {
        shape
            .property(key)
            .expect("known variant field")
            .validate_type(value, &format!("{variant_field}.{key}"))
            .map_err(|error| invalid(error.path, error.message))?;
    }
    Ok(variant)
}
pub fn parse_fill_value(value: &Value, field: &str) -> Result<MetaFillValue, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "fillValue must be a structured object"))?;
    let kind = required_string_at(object, "kind", &format!("{field}.kind"))?;
    let converted = match kind.as_str() {
        "string" => MetaFillValue::String(required_string_value(object, "value", field)?),
        "number" => MetaFillValue::Number(required_string_value(object, "value", field)?),
        "boolean" => {
            MetaFillValue::Boolean(object.get("value").and_then(Value::as_bool).ok_or_else(
                || {
                    invalid(
                        format!("{field}.value"),
                        "boolean fill value must be a boolean",
                    )
                },
            )?)
        }
        "dateTime" => MetaFillValue::DateTime(required_string_value(object, "value", field)?),
        "reference" => {
            let raw = required_string_at(object, "metadataPath", &format!("{field}.metadataPath"))?;
            MetaFillValue::Reference(MetadataReference {
                metadata_path: parse_address(&raw, &format!("{field}.metadataPath"))?,
            })
        }
        _ => {
            return Err(invalid(
                format!("{field}.kind"),
                format!("unsupported fill value kind `{kind}`"),
            ))
        }
    };
    let schema = fill_value_schema();
    let shape = ArgumentShape::new(&schema)
        .variant("kind", &kind)
        .expect("converted fill value variant");
    reject_unknown_shape_fields(object, shape, &format!("{field}."))?;
    for (key, value) in object {
        shape
            .property(key)
            .expect("known fill value field")
            .validate_type(value, &format!("{field}.{key}"))
            .map_err(|error| invalid(error.path, error.message))?;
    }
    Ok(converted)
}
pub fn parse_reference(value: &Value, field: &str) -> Result<MetadataReference, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "relation target must be an object"))?;
    let reader = MetadataArgumentReader::new(
        object,
        metadata_relation_object_target_input_schema(),
        field,
    );
    reader.reject_unknown()?;
    reader.read("metadataPath", |object, key, field| {
        Ok(MetadataReference {
            metadata_path: parse_address(&required_string_at(object, key, field)?, field)?,
        })
    })
}
pub fn parse_field_reference(
    value: &Value,
    field: &str,
) -> Result<MetadataFieldPath, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "inputByString target must be an object"))?;
    let reader =
        MetadataArgumentReader::new(object, metadata_relation_field_target_input_schema(), field);
    reader.reject_unknown()?;
    reader.read("fieldPath", |object, key, field| {
        MetadataFieldPath::parse(&required_string_at(object, key, field)?).map_err(
            |mut diagnostic| {
                diagnostic.field = Some(field.to_string());
                diagnostic
            },
        )
    })
}
pub fn parse_event_source(value: &Value, field: &str) -> Result<MetaEventSource, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "event source target must be an object"))?;
    let kind = required_string_at(object, "kind", &format!("{field}.kind"))?;
    let Some(shape) =
        ArgumentShape::new(metadata_event_source_input_schema()).variant("kind", &kind)
    else {
        return Err(invalid(
            format!("{field}.kind"),
            format!("unsupported event source kind `{kind}`"),
        ));
    };
    let reader = MetadataArgumentReader {
        object,
        shape,
        field,
    };
    reader.read("kind", required_string_at)?;
    let source = match kind.as_str() {
        "object" | "manager" | "recordSet" | "definedType" => {
            let metadata_path = reader.read("metadataPath", |object, key, field| {
                parse_address(&required_string_at(object, key, field)?, field)
            })?;
            match kind.as_str() {
                "object" => MetaEventSource::Object { metadata_path },
                "manager" => {
                    let source_class = reader.read("sourceClass", |object, key, field| {
                        object
                            .get(key)
                            .map(|value| {
                                EventSourceClass::parse(value.as_str().ok_or_else(|| {
                                    invalid(field, "sourceClass must be a string")
                                })?)
                                .map_err(|mut diagnostic| {
                                    diagnostic.field = Some(field.to_string());
                                    diagnostic
                                })
                            })
                            .transpose()
                    })?;
                    MetaEventSource::Manager {
                        metadata_path,
                        source_class,
                    }
                }
                "recordSet" => MetaEventSource::RecordSet { metadata_path },
                "definedType" => MetaEventSource::DefinedType { metadata_path },
                _ => unreachable!("event source kind match is closed"),
            }
        }
        "family" => {
            let source_class = reader.read("sourceClass", |object, key, field| {
                EventSourceClass::parse(&required_string_at(object, key, field)?).map_err(
                    |mut diagnostic| {
                        diagnostic.field = Some(field.to_string());
                        diagnostic
                    },
                )
            })?;
            MetaEventSource::Family { source_class }
        }
        _ => unreachable!("the shared event source union has closed kinds"),
    };
    reader.reject_unknown()?;
    Ok(source)
}
pub fn metadata_kind_for_address(
    address: &MetadataAddress,
) -> Result<MetadataKind, MetaDiagnostic> {
    let mut segments = address.segments();
    let Some(kind) = segments.next() else {
        return Err(invalid(
            "metadataPath",
            "meta.edit requires a top-level metadata object path",
        ));
    };
    let Some(_name) = segments.next() else {
        return Err(invalid(
            "metadataPath",
            "meta.edit requires a top-level metadata object path",
        ));
    };
    if segments.next().is_some() {
        return Err(invalid(
            "metadataPath",
            "meta.edit requires a top-level metadata object path",
        ));
    }
    MetadataKind::parse(kind).map_err(|_| {
        invalid(
            "metadataPath",
            format!("setProperties is unsupported for metadata kind `{kind}`"),
        )
    })
}
pub fn reject_forbidden_fields(
    object: &Map<String, Value>,
    forbidden: &[&str],
) -> Result<(), MetaDiagnostic> {
    if let Some(name) = forbidden.iter().find(|name| object.contains_key(**name)) {
        return Err(invalid(
            *name,
            format!("field `{name}` is not legal for this operation"),
        ));
    }
    Ok(())
}
pub fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    prefix: &str,
) -> Result<(), MetaDiagnostic> {
    if let Some(name) = object.keys().find(|name| !allowed.contains(&name.as_str())) {
        return Err(invalid(
            format!("{prefix}{name}"),
            format!("unknown field `{name}`"),
        ));
    }
    Ok(())
}
pub fn registry_value<T: Copy>(registry: &[(&str, T)], name: &str) -> Option<T> {
    registry
        .iter()
        .find_map(|(candidate, value)| (*candidate == name).then_some(*value))
}
pub fn required_metadata_path(
    object: &Map<String, Value>,
    field: &str,
) -> Result<MetadataAddress, MetaDiagnostic> {
    let raw = required_string_diagnostic(object, field)?;
    parse_address(&raw, field)
}
pub fn parse_address(raw: &str, field: &str) -> Result<MetadataAddress, MetaDiagnostic> {
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).map_err(|error| {
        invalid(
            field,
            format!("invalid logical metadata address: {}", error.message),
        )
    })
}
pub fn required_string(object: &Map<String, Value>, field: &str) -> Result<String, MetaFailure> {
    required_string_diagnostic(object, field).map_err(Into::into)
}
pub fn required_string_diagnostic(
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, MetaDiagnostic> {
    let value = object
        .get(field)
        .ok_or_else(|| invalid(field, format!("`{field}` is required")))?;
    nonempty_string(value, field)
}
pub fn required_string_at(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<String, MetaDiagnostic> {
    let value = object
        .get(key)
        .ok_or_else(|| invalid(field, format!("`{key}` is required")))?;
    nonempty_string(value, field)
}
pub fn nonempty_string(value: &Value, field: &str) -> Result<String, MetaDiagnostic> {
    let value = value
        .as_str()
        .ok_or_else(|| invalid(field, format!("`{field}` must be a string")))?;
    if value.is_empty() || value != value.trim() {
        return Err(invalid(
            field,
            format!("`{field}` must be non-empty without surrounding whitespace"),
        ));
    }
    Ok(value.to_string())
}
pub fn optional_string_at(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<Option<String>, MetaDiagnostic> {
    object
        .get(key)
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid(field, format!("`{key}` must be a string")))
        })
        .transpose()
}
pub fn required_string_value(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<String, MetaDiagnostic> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            invalid(
                format!("{field}.{key}"),
                format!("`{key}` must be a string"),
            )
        })
}
pub fn required_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Map<String, Value>, MetaDiagnostic> {
    object
        .get(field)
        .ok_or_else(|| invalid(field, format!("`{field}` is required")))?
        .as_object()
        .ok_or_else(|| invalid(field, format!("`{field}` must be an object")))
}
pub fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Vec<Value>, MetaDiagnostic> {
    object
        .get(field)
        .ok_or_else(|| invalid(field, format!("`{field}` is required")))?
        .as_array()
        .ok_or_else(|| invalid(field, format!("`{field}` must be an array")))
}
pub fn required_array_at<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<&'a Vec<Value>, MetaDiagnostic> {
    object
        .get(key)
        .ok_or_else(|| invalid(field, format!("`{key}` is required")))?
        .as_array()
        .ok_or_else(|| invalid(field, format!("`{key}` must be an array")))
}
pub fn optional_bool(
    object: &Map<String, Value>,
    field: &str,
    default: bool,
) -> Result<bool, MetaFailure> {
    optional_bool_diagnostic(object, field)
        .map(|value| value.unwrap_or(default))
        .map_err(Into::into)
}
pub fn optional_bool_diagnostic(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, MetaDiagnostic> {
    object
        .get(field)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| invalid(field, format!("`{field}` must be a boolean")))
        })
        .transpose()
}
pub fn optional_bool_at(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<Option<bool>, MetaDiagnostic> {
    object
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| invalid(field, format!("`{key}` must be a boolean")))
        })
        .transpose()
}
pub fn parse_bounded_usize(
    value: Option<&Value>,
    field: &str,
    default: usize,
    maximum: usize,
) -> Result<usize, MetaFailure> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = json_u32(value)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| (1..=maximum).contains(value))
        .ok_or_else(|| {
            invalid(
                field,
                format!("`{field}` must be an integer between 1 and {maximum}"),
            )
        })?;
    Ok(value)
}
pub fn required_u32(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<u32, MetaDiagnostic> {
    object.get(key).and_then(json_u32).ok_or_else(|| {
        invalid(
            format!("{field}.{key}"),
            format!("`{key}` must be an unsigned 32-bit integer"),
        )
    })
}
pub fn json_u32(value: &Value) -> Option<u32> {
    let number = value.as_number()?;
    if let Some(value) = number.as_u64() {
        return u32::try_from(value).ok();
    }
    if let Some(value) = number.as_i64() {
        return u32::try_from(value).ok();
    }
    let value = number.as_f64()?;
    (value.is_finite() && value.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&value))
        .then_some(value as u32)
}
pub fn invalid(field: impl Into<String>, message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message).with_field(field)
}
pub fn metadata_input_schema(operation: MetadataOperation) -> Value {
    let string =
        |description: &str| json!({"type": "string", "minLength": 1, "description": description});
    let mut properties = Map::new();
    properties.insert(
        "sourceSet".into(),
        string("Exact Configuration source-set name from v8project.yaml."),
    );
    properties.insert(
        "cwd".into(),
        string(
            "Absolute path to the workspace root holding v8project.yaml; it selects the workspace and never narrows the logical address.",
        ),
    );
    let required = match operation {
        MetadataOperation::Info => {
            properties.insert(
                "metadataPath".into(),
                string("Logical metadata path of the object to inspect."),
            );
            properties.insert(
                "sections".into(),
                json!({
                    "type": "array",
                    "uniqueItems": true,
                    "items": {
                        "type": "string",
                        "enum": INFO_SECTIONS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
                    },
                    "default": [],
                    "description": "Extra sections to compute, all read from the source tree: `roles`, `subscriptions` and `functionalOptions` land in `usage`, `predefinedItems` in its own field. Omit or pass [] to inspect the object alone.",
                }),
            );
            properties.insert(
                "limit".into(),
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 20,
                    "description": "Maximum `predefinedItems` returned (1 through 50). Usage lists are read from the source tree, so they are exact and complete and the limit does not apply to them."
                }),
            );
            vec!["sourceSet", "metadataPath"]
        }
        MetadataOperation::Add => {
            properties.insert(
                "kind".into(),
                json!({
                    "type": "string",
                    "enum": MetadataKind::ALL
                        .iter()
                        .map(|kind| kind.as_str())
                        .collect::<Vec<_>>(),
                    "description": "Supported metadata object kind for the minimal template.",
                }),
            );
            properties.insert(
                "name".into(),
                json!({
                    "type": "string",
                    "minLength": 1,
                    "pattern": "^[^.]+$",
                    "description": "Metadata object name using a valid 1C identifier.",
                }),
            );
            properties.insert(
                "operations".into(),
                json!({
                    "type": "array",
                    "minItems": 1,
                    "items": host_visible_operation_schema(),
                    "description": "Optional ordered typed operations applied to the private creation image before one atomic publication.",
                }),
            );
            properties.insert(
                "dryRun".into(),
                json!({
                    "type": "boolean",
                    "default": true,
                    "description": "Preview the mutation without writing workspace files."
                }),
            );
            vec!["sourceSet", "kind", "name"]
        }
        MetadataOperation::Edit => {
            properties.insert(
                "metadataPath".into(),
                json!({
                    "type": "string",
                    "minLength": 1,
                    "pattern": metadata_edit_path_pattern(),
                    "description": "Logical metadata path of the object to edit.",
                }),
            );
            properties.insert(
                "operations".into(),
                json!({
                    "type": "array",
                    "minItems": 1,
                    "items": host_visible_operation_schema(),
                    "description": "Ordered typed edit operations applied as one atomic change.",
                }),
            );
            properties.insert(
                "dryRun".into(),
                json!({
                    "type": "boolean",
                    "default": true,
                    "description": "Preview the mutation without writing workspace files."
                }),
            );
            vec!["sourceSet", "metadataPath", "operations"]
        }
    };
    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    });
    match operation {
        MetadataOperation::Add | MetadataOperation::Edit => {
            schema
                .as_object_mut()
                .expect("metadata input schema is always an object")
                .insert("$defs".into(), Value::Object(metadata_schema_definitions()));
        }
        MetadataOperation::Info => {}
    }
    schema
}
/// Shared shapes the host-visible operation union refers to.
///
/// The union in [`host_visible_operation_schema`] is the whole published
/// contract, so only the definitions it actually reaches belong here. The
/// per-kind narrowing that used to live in `allOf`/`$defs` never reached a host
/// that renders `properties` alone, while it consumed the entire reviewed
/// context budget; per-kind legality is enforced by the writer, which answers a
/// violation with `unsupported_kind` naming the exact field.
pub fn metadata_schema_definitions() -> Map<String, Value> {
    let mut definitions = Map::new();
    definitions.insert("scope".into(), scope_schema());
    definitions.insert("position".into(), position_schema());
    definitions.insert("metadataType".into(), metadata_type_schema());
    definitions.insert("fillValue".into(), fill_value_schema());
    definitions
}
pub fn schema_reference(definition: impl AsRef<str>) -> Value {
    json!({"$ref": format!("#/$defs/{}", definition.as_ref())})
}
pub fn metadata_edit_path_pattern() -> String {
    format!(
        r"^({})\.[^.]+$",
        MetadataKind::ALL
            .iter()
            .flat_map(|kind| {
                metadata_address_kind_spellings(kind.as_str())
                    .expect("metadata kind must have a source-target spelling registry")
            })
            .collect::<Vec<_>>()
            .join("|")
    )
}
/// Kind-agnostic union of every typed operation shape, published directly as
/// `properties.operations.items`.
///
/// The per-kind `allOf`/`if`/`then` branches narrow this union to the exact
/// variant a concrete `kind` or `metadataPath` allows. A host that renders only
/// `properties` never evaluates a conditional, so without a direct `items` the
/// model is shown `array of anything` and the whole typed contract is lost on
/// the way to the caller. The union therefore ships inline, and the conditional
/// branches only tighten it: every branch variant is a subset of this union, so
/// the language the schema accepts is unchanged.
pub fn host_visible_operation_schema() -> Value {
    let collections = MetaCollection::ALL
        .iter()
        .copied()
        .filter(|collection| *collection != MetaCollection::PredefinedItems)
        .map(MetaCollection::as_str)
        .collect::<Vec<_>>();
    let root_properties = metadata_property_schemas(None);
    let element = metadata_host_element_schema();
    let collection_operation = |tag: MetaEditOperationTag, payload_name: &str, payload: Value| {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["op", "collection", payload_name],
            "properties": {
                "op": {"type": "string", "enum": [tag.as_str()]},
                "collection": {"type": "string", "enum": collections},
                "scope": schema_reference("scope"),
                payload_name: payload,
            },
        })
    };
    let elements = || json!({"type": "array", "minItems": 1, "items": element});
    let predefined_fields = || {
        let mut fields = metadata_predefined_element_input_schema(false)["properties"].clone();
        fields["id"]
            .as_object_mut()
            .expect("predefined id field")
            .remove("minLength");
        fields["id"]["format"] = json!("uuid");
        fields["id"]["pattern"] =
            json!("^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$");
        fields["type"] = schema_reference("metadataType");
        fields
    };
    let predefined_operation = |tag: MetaEditOperationTag, update: bool| {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["op", "collection", "elements"],
            "properties": {
                "op": {"type": "string", "enum": [tag.as_str()]},
                "collection": {"type": "string", "enum": [MetaCollection::PredefinedItems.as_str()]},
                "elements": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "minProperties": if update { 2 } else { 0 },
                        "required": metadata_predefined_element_input_schema(update)["required"].clone(),
                        "properties": predefined_fields(),
                    },
                },
            },
        })
    };
    json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["op", "values"],
                "properties": {
                    "op": {"type": "string", "enum": [MetaEditOperationTag::SetProperties.as_str()]},
                    "values": {
                        "type": "object",
                        "additionalProperties": false,
                        "minProperties": 1,
                        "properties": root_properties,
                        "description": "Root scalar properties; the legal subset depends on the object kind.",
                    },
                },
            },
            collection_operation(MetaEditOperationTag::Add, "elements", elements()),
            collection_operation(MetaEditOperationTag::Update, "elements", elements()),
            collection_operation(
                MetaEditOperationTag::Remove,
                "names",
                json!({
                    "type": "array",
                    "minItems": 1,
                    "uniqueItems": true,
                    "items": {"type": "string", "minLength": 1},
                }),
            ),
            predefined_operation(MetaEditOperationTag::Add, false),
            predefined_operation(MetaEditOperationTag::Update, true),
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["op", "collection", "ids"],
                "properties": {
                    "op": {"type": "string", "enum": [MetaEditOperationTag::Remove.as_str()]},
                    "collection": {"type": "string", "enum": [MetaCollection::PredefinedItems.as_str()]},
                    "ids": {
                        "type": "array",
                        "minItems": 1,
                        "uniqueItems": true,
                        "items": {
                            "type": "string",
                            "format": "uuid",
                            "pattern": "^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$"
                        },
                    },
                },
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["op"],
                "properties": {
                    "op": {"type": "string", "enum": [MetaEditOperationTag::AddHelp.as_str()]},
                    "lang": {
                        "type": "string",
                        "minLength": 1,
                        "pattern": "^[A-Za-z0-9_-]+$",
                        "default": "ru",
                        "description": "Help language code naming Ext/Help/<lang>.html; defaults to ru.",
                    },
                },
                "description": "Create the owner's embedded help facet: Ext/Help.xml, Ext/Help/<lang>.html and IncludeHelpInContents on the owner's forms. Create-only; a repeat is rejected (ADR-0072).",
            },
            metadata_relation_operation_schema(),
        ],
        "description": "Exactly one typed metadata edit operation. Legal collections, root properties and relations are narrowed by the object kind. EventSubscription source supports add/remove preserving existing members and replace with deterministic ordering; requests require at least one logical event source.",
    })
}
pub fn scope_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "tabularSection": {
                "type": "string",
                "minLength": 1,
                "description": "Tabular section that owns the selected collection.",
            },
        },
        "required": ["tabularSection"],
        "description": "Optional scope for a collection owned by a tabular section.",
    })
}
impl From<MetaDiagnostic> for MetaFailure {
    fn from(diagnostic: MetaDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }
}

/// Read shared shape facts at the existing conversion stages. Primitive
/// conversion retains the established diagnostics; the contract also checks
/// the value, so changing only Can's field type changes productive admission.
struct MetadataArgumentReader<'a> {
    object: &'a Map<String, Value>,
    shape: ArgumentShape<'a>,
    field: &'a str,
}
impl<'a> MetadataArgumentReader<'a> {
    fn new(object: &'a Map<String, Value>, schema: &'a Value, field: &'a str) -> Self {
        Self {
            object,
            shape: ArgumentShape::new(schema),
            field,
        }
    }

    fn reject_unknown(&self) -> Result<(), MetaDiagnostic> {
        let prefix = if self.field.is_empty() {
            String::new()
        } else {
            format!("{}.", self.field)
        };
        reject_unknown_shape_fields(self.object, self.shape, &prefix)
    }

    fn read<T>(
        &self,
        key: &str,
        convert: impl FnOnce(&'a Map<String, Value>, &str, &str) -> Result<T, MetaDiagnostic>,
    ) -> Result<T, MetaDiagnostic> {
        let field = if self.field.is_empty() {
            key.to_string()
        } else {
            format!("{}.{key}", self.field)
        };
        let converted = convert(self.object, key, &field)?;
        if let Some(value) = self.object.get(key) {
            let shape = self.shape.property(key).expect("known typed input field");
            shape.validate_field(value, &field).map_err(|error| {
                let message = if key == "indexing" {
                    "`indexing` must be DontIndex, Index or IndexWithAdditionalOrder".into()
                } else {
                    error.message
                };
                invalid(error.path, message)
            })?;
        } else if self.shape.requires(key) {
            return Err(invalid(field, format!("`{key}` is required")));
        }
        Ok(converted)
    }
}

#[cfg(test)]
mod shared_primitive_contract_tests {
    use super::*;
    #[test]
    fn predefined_fields_consume_the_shared_primitive_and_nested_shapes() {
        let item = json!({"code":"C","description":"D","isFolder":false,
        "accountType":"Active","offBalance":false,"order":"O",
        "accountingFlags":{"Flag":true},"extDimensionTypes":[{"name":"Dim","turnover":false,"accountingFlags":{"Flag":true}}],
        "actionPeriodIsBase":false});
        let object = item.as_object().unwrap();
        assert!(parse_predefined_fields(&MetadataArgumentReader::new(
            object,
            metadata_predefined_element_input_schema(false),
            "elements[3]"
        ))
        .is_ok());
        for (pointer, field) in [
        ("/properties/code/type", "elements[3].code"),
        ("/properties/description/type", "elements[3].description"),
        ("/properties/isFolder/type", "elements[3].isFolder"),
        ("/properties/accountType/type", "elements[3].accountType"),
        ("/properties/offBalance/type", "elements[3].offBalance"),
        ("/properties/order/type", "elements[3].order"),
        ("/properties/accountingFlags/additionalProperties/type", "elements[3].accountingFlags.Flag"),
        ("/properties/extDimensionTypes/items/properties/name/type", "elements[3].extDimensionTypes[0].name"),
        ("/properties/extDimensionTypes/items/properties/turnover/type", "elements[3].extDimensionTypes[0].turnover"),
        ("/properties/extDimensionTypes/items/properties/accountingFlags/additionalProperties/type", "elements[3].extDimensionTypes[0].accountingFlags.Flag"),
        ("/properties/actionPeriodIsBase/type", "elements[3].actionPeriodIsBase"),
    ] {
        let mut schema = metadata_predefined_element_input_schema(false).clone();
        *schema.pointer_mut(pointer).unwrap() = json!("integer");
        let reader = MetadataArgumentReader::new(object, &schema, "elements[3]");
        let error = parse_predefined_fields(&reader).unwrap_err();
        assert_eq!(error.field.as_deref(), Some(field), "{pointer}: {error:?}");
    }
    }
}

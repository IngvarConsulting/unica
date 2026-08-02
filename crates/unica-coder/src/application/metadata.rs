#![allow(dead_code)] // The handler remains off-registry until the coordinated public switch.

use crate::domain::metadata::{
    DateFractions, MetaCollection, MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation,
    MetaEditOperationTag, MetaElementInput, MetaElementUpdateInput, MetaFillValue, MetaPosition,
    MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetaPropertyValueKind, MetaRelation,
    MetaScope, MetadataKind, MetadataReference, MetadataType, MetadataTypeVariant, NumberSign,
    RelationEditMode, StringLengthMode, METADATA_PROPERTY_SPECS,
};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataOperation {
    Info,
    Add,
    Edit,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaInfoSection {
    Modules,
    Roles,
    Subscriptions,
    FunctionalOptions,
    PredefinedItems,
}

const INFO_SECTIONS: &[(&str, MetaInfoSection)] = &[
    ("modules", MetaInfoSection::Modules),
    ("roles", MetaInfoSection::Roles),
    ("subscriptions", MetaInfoSection::Subscriptions),
    ("functionalOptions", MetaInfoSection::FunctionalOptions),
    ("predefinedItems", MetaInfoSection::PredefinedItems),
];
const DEFAULT_INFO_SECTIONS: &[(&str, MetaInfoSection)] = &[
    ("modules", MetaInfoSection::Modules),
    ("roles", MetaInfoSection::Roles),
    ("subscriptions", MetaInfoSection::Subscriptions),
    ("functionalOptions", MetaInfoSection::FunctionalOptions),
];
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaInfoRequest {
    pub(crate) source_set: String,
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) sections: Vec<MetaInfoSection>,
    pub(crate) limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaAddRequest {
    pub(crate) source_set: String,
    pub(crate) kind: MetadataKind,
    pub(crate) name: String,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaEditRequest {
    pub(crate) source_set: String,
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) operations: Vec<MetaEditOperation>,
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaRemoveRequest {
    pub(crate) source_set: String,
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) dry_run: bool,
    pub(crate) force: bool,
    pub(crate) confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetadataRequest {
    Info(MetaInfoRequest),
    Add(MetaAddRequest),
    Edit(MetaEditRequest),
    Remove(MetaRemoveRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaFailure {
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
}

impl From<MetaDiagnostic> for MetaFailure {
    fn from(diagnostic: MetaDiagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }
}

pub(crate) fn parse_metadata_request(
    operation: MetadataOperation,
    args: &Map<String, Value>,
) -> Result<MetadataRequest, MetaFailure> {
    reject_unknown_top_level(operation, args)?;
    let source_set = required_string(args, "sourceSet")?;
    match operation {
        MetadataOperation::Info => {
            let metadata_path = required_metadata_path(args, "metadataPath")?;
            let sections = parse_info_sections(args.get("sections"))?;
            let limit = parse_positive_usize(args.get("limit"), "limit", 20)?;
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
            let dry_run = optional_bool(args, "dryRun", true)?;
            Ok(MetadataRequest::Add(MetaAddRequest {
                source_set,
                kind,
                name,
                dry_run,
            }))
        }
        MetadataOperation::Edit => {
            let metadata_path = required_metadata_path(args, "metadataPath")?;
            let raw_operations = required_array(args, "operations")?;
            if raw_operations.is_empty() {
                return Err(invalid("operations", "operations must not be empty").into());
            }
            let mut operations = Vec::with_capacity(raw_operations.len());
            for (index, raw_operation) in raw_operations.iter().enumerate() {
                let converted = parse_edit_operation(raw_operation, &metadata_path)
                    .map_err(|diagnostic| diagnostic.with_operation_index(index))?;
                operations.push(converted);
            }
            let dry_run = optional_bool(args, "dryRun", true)?;
            Ok(MetadataRequest::Edit(MetaEditRequest {
                source_set,
                metadata_path,
                operations,
                dry_run,
            }))
        }
        MetadataOperation::Remove => {
            let metadata_path = required_metadata_path(args, "metadataPath")?;
            let dry_run = optional_bool(args, "dryRun", true)?;
            let force = optional_bool(args, "force", false)?;
            let confirm = optional_bool(args, "confirm", false)?;
            if (force && (dry_run || !confirm)) || (confirm && !force) {
                return Err(invalid(
                    "force",
                    "forced remove apply requires force=true, confirm=true, and dryRun=false",
                )
                .into());
            }
            Ok(MetadataRequest::Remove(MetaRemoveRequest {
                source_set,
                metadata_path,
                dry_run,
                force,
                confirm,
            }))
        }
    }
}

fn reject_unknown_top_level(
    operation: MetadataOperation,
    args: &Map<String, Value>,
) -> Result<(), MetaDiagnostic> {
    let schema = metadata_input_schema(operation);
    let allowed = schema["properties"]
        .as_object()
        .expect("metadata schemas always publish an object property registry");
    if let Some(unknown) = args.keys().find(|name| !allowed.contains_key(*name)) {
        return Err(invalid(
            unknown,
            format!("metadata operation does not accept argument `{unknown}`"),
        ));
    }
    Ok(())
}

fn parse_info_sections(value: Option<&Value>) -> Result<Vec<MetaInfoSection>, MetaDiagnostic> {
    let Some(value) = value else {
        return Ok(DEFAULT_INFO_SECTIONS
            .iter()
            .map(|(_, section)| *section)
            .collect());
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

fn parse_edit_operation(
    value: &Value,
    metadata_path: &MetadataAddress,
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
                    "relation",
                    "mode",
                    "targets",
                ],
            )?;
            let kind = metadata_kind_for_address(metadata_path)?;
            let values = parse_property_changes(required_object(object, "values")?, kind)?;
            Ok(MetaEditOperation::SetProperties { values })
        }
        MetaEditOperationTag::Add => {
            reject_forbidden_fields(object, &["values", "names", "relation", "mode", "targets"])?;
            let collection = parse_collection(object)?;
            let scope = parse_scope(object.get("scope"))?;
            let elements = required_array(object, "elements")?
                .iter()
                .map(|element| parse_element_input(element, false))
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::add(collection, scope, elements)
        }
        MetaEditOperationTag::Update => {
            reject_forbidden_fields(object, &["values", "names", "relation", "mode", "targets"])?;
            let collection = parse_collection(object)?;
            let scope = parse_scope(object.get("scope"))?;
            let elements = required_array(object, "elements")?
                .iter()
                .map(parse_element_update)
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::update(collection, scope, elements)
        }
        MetaEditOperationTag::Remove => {
            reject_forbidden_fields(
                object,
                &["values", "elements", "relation", "mode", "targets"],
            )?;
            let collection = parse_collection(object)?;
            let scope = parse_scope(object.get("scope"))?;
            let names = required_array(object, "names")?
                .iter()
                .map(|value| nonempty_string(value, "names"))
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::remove(collection, scope, names)
        }
        MetaEditOperationTag::EditRelations => {
            reject_forbidden_fields(
                object,
                &["values", "collection", "scope", "elements", "names"],
            )?;
            let relation_name = required_string_diagnostic(object, "relation")?;
            let relation = MetaRelation::parse(&relation_name)?;
            let mode_name = required_string_diagnostic(object, "mode")?;
            let mode = RelationEditMode::parse(&mode_name)?;
            let targets = required_array(object, "targets")?
                .iter()
                .map(parse_reference)
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::edit_relations(relation, mode, targets)
        }
    }
}

fn operation_property_names() -> &'static [&'static str] {
    &[
        "op",
        "values",
        "collection",
        "scope",
        "elements",
        "names",
        "relation",
        "mode",
        "targets",
    ]
}

fn parse_collection(object: &Map<String, Value>) -> Result<MetaCollection, MetaDiagnostic> {
    let name = required_string_diagnostic(object, "collection")?;
    MetaCollection::parse(&name)
}

fn parse_scope(value: Option<&Value>) -> Result<Option<MetaScope>, MetaDiagnostic> {
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

fn parse_property_changes(
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
            .ok_or_else(|| invalid(&field, format!("unknown metadata property `{name}`")))?;
        let value = match spec.value_kind {
            MetaPropertyValueKind::String => value
                .as_str()
                .map(|value| MetaPropertyValue::String(value.to_string()))
                .ok_or_else(|| invalid(&field, "property value must be a string"))?,
            MetaPropertyValueKind::Boolean => value
                .as_bool()
                .map(MetaPropertyValue::Boolean)
                .ok_or_else(|| invalid(&field, "property value must be a boolean"))?,
            MetaPropertyValueKind::UnsignedInteger => value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .map(MetaPropertyValue::UnsignedInteger)
                .ok_or_else(|| {
                    invalid(&field, "property value must be an unsigned 32-bit integer")
                })?,
        };
        inputs.push(MetaPropertyInput::new(name, value));
    }
    MetaPropertyChanges::convert(kind, inputs)
}

fn parse_element_input(value: &Value, nested: bool) -> Result<MetaElementInput, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("elements", "elements must contain objects"))?;
    let allowed = if nested {
        &[
            "name",
            "synonym",
            "comment",
            "type",
            "required",
            "fillValue",
            "position",
        ][..]
    } else {
        &[
            "name",
            "synonym",
            "comment",
            "type",
            "required",
            "fillValue",
            "attributes",
            "position",
        ][..]
    };
    reject_unknown_fields(object, allowed, "elements.")?;
    let attributes = object
        .get("attributes")
        .map(|_| {
            let attributes = required_array_at(object, "attributes", "elements.attributes")?;
            if attributes.is_empty() {
                return Err(invalid(
                    "elements.attributes",
                    "nested attributes must not be empty",
                ));
            }
            attributes
                .iter()
                .map(|element| parse_element_input(element, true))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    Ok(MetaElementInput {
        name: required_string_at(object, "name", "elements.name")?,
        synonym: optional_string_at(object, "synonym", "elements.synonym")?,
        comment: optional_string_at(object, "comment", "elements.comment")?,
        r#type: object
            .get("type")
            .map(|value| parse_metadata_type(value, "elements.type"))
            .transpose()?,
        required: optional_bool_at(object, "required", "elements.required")?,
        fill_value: object
            .get("fillValue")
            .map(|value| parse_fill_value(value, "elements.fillValue"))
            .transpose()?,
        attributes,
        position: object.get("position").map(parse_position).transpose()?,
    })
}

fn parse_element_update(value: &Value) -> Result<MetaElementUpdateInput, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("elements", "elements must contain objects"))?;
    reject_unknown_fields(
        object,
        &[
            "name",
            "newName",
            "synonym",
            "comment",
            "type",
            "required",
            "fillValue",
            "position",
        ],
        "elements.",
    )?;
    Ok(MetaElementUpdateInput {
        name: required_string_at(object, "name", "elements.name")?,
        new_name: optional_string_at(object, "newName", "elements.newName")?,
        synonym: optional_string_at(object, "synonym", "elements.synonym")?,
        comment: optional_string_at(object, "comment", "elements.comment")?,
        r#type: object
            .get("type")
            .map(|value| parse_metadata_type(value, "elements.type"))
            .transpose()?,
        required: optional_bool_at(object, "required", "elements.required")?,
        fill_value: object
            .get("fillValue")
            .map(|value| parse_fill_value(value, "elements.fillValue"))
            .transpose()?,
        position: object.get("position").map(parse_position).transpose()?,
    })
}

fn parse_position(value: &Value) -> Result<MetaPosition, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("position", "position must be an object"))?;
    reject_unknown_fields(object, &["before", "after"], "position.")?;
    MetaPosition::new(
        optional_string_at(object, "before", "position.before")?,
        optional_string_at(object, "after", "position.after")?,
    )
}

fn parse_metadata_type(value: &Value, field: &str) -> Result<MetadataType, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "type must be a structured object"))?;
    reject_unknown_fields(object, &["variants"], &format!("{field}."))?;
    let variants = required_array_at(object, "variants", &format!("{field}.variants"))?
        .iter()
        .enumerate()
        .map(|(index, value)| parse_type_variant(value, index, field))
        .collect::<Result<Vec<_>, _>>()?;
    MetadataType::new(variants).map_err(|mut diagnostic| {
        if let Some(domain_field) = diagnostic.field.as_deref() {
            if let Some(suffix) = domain_field.strip_prefix("type") {
                diagnostic.field = Some(format!("{field}{suffix}"));
            }
        }
        diagnostic
    })
}

fn parse_type_variant(
    value: &Value,
    index: usize,
    field: &str,
) -> Result<MetadataTypeVariant, MetaDiagnostic> {
    let variant_field = format!("{field}.variants[{index}]");
    let object = value
        .as_object()
        .ok_or_else(|| invalid(&variant_field, "type variant must be an object"))?;
    let kind = required_string_at(object, "kind", &format!("{variant_field}.kind"))?;
    let (allowed, variant) = match kind.as_str() {
        "string" => (
            &["kind", "length", "allowedLength"][..],
            MetadataTypeVariant::String {
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
        ),
        "number" => (
            &["kind", "digits", "fraction", "sign"][..],
            MetadataTypeVariant::Number {
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
        ),
        "boolean" => (&["kind"][..], MetadataTypeVariant::Boolean),
        "date" => (
            &["kind", "fractions"][..],
            MetadataTypeVariant::Date {
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
        ),
        "valueStorage" => (&["kind"][..], MetadataTypeVariant::ValueStorage),
        "reference" | "definedType" => {
            let raw = required_string_at(
                object,
                "metadataPath",
                &format!("{variant_field}.metadataPath"),
            )?;
            let metadata_path = parse_address(&raw, &format!("{variant_field}.metadataPath"))?;
            let variant = if kind == "reference" {
                MetadataTypeVariant::Reference { metadata_path }
            } else {
                MetadataTypeVariant::DefinedType { metadata_path }
            };
            (&["kind", "metadataPath"][..], variant)
        }
        _ => {
            return Err(invalid(
                format!("{variant_field}.kind"),
                format!("unsupported type kind `{kind}`"),
            ))
        }
    };
    reject_unknown_fields(object, allowed, &format!("{variant_field}."))?;
    Ok(variant)
}

fn parse_fill_value(value: &Value, field: &str) -> Result<MetaFillValue, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "fillValue must be a structured object"))?;
    let kind = required_string_at(object, "kind", &format!("{field}.kind"))?;
    let (allowed, converted) = match kind.as_str() {
        "string" => (
            &["kind", "value"][..],
            MetaFillValue::String(required_string_value(object, "value", field)?),
        ),
        "number" => (
            &["kind", "value"][..],
            MetaFillValue::Number(required_string_value(object, "value", field)?),
        ),
        "boolean" => (
            &["kind", "value"][..],
            MetaFillValue::Boolean(object.get("value").and_then(Value::as_bool).ok_or_else(
                || {
                    invalid(
                        format!("{field}.value"),
                        "boolean fill value must be a boolean",
                    )
                },
            )?),
        ),
        "dateTime" => (
            &["kind", "value"][..],
            MetaFillValue::DateTime(required_string_value(object, "value", field)?),
        ),
        "reference" => {
            let raw = required_string_at(object, "metadataPath", &format!("{field}.metadataPath"))?;
            (
                &["kind", "metadataPath"][..],
                MetaFillValue::Reference(MetadataReference {
                    metadata_path: parse_address(&raw, &format!("{field}.metadataPath"))?,
                }),
            )
        }
        _ => {
            return Err(invalid(
                format!("{field}.kind"),
                format!("unsupported fill value kind `{kind}`"),
            ))
        }
    };
    reject_unknown_fields(object, allowed, &format!("{field}."))?;
    Ok(converted)
}

fn parse_reference(value: &Value) -> Result<MetadataReference, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("targets", "relation targets must be objects"))?;
    reject_unknown_fields(object, &["metadataPath"], "targets.")?;
    let raw = required_string_at(object, "metadataPath", "targets.metadataPath")?;
    Ok(MetadataReference {
        metadata_path: parse_address(&raw, "targets.metadataPath")?,
    })
}

fn metadata_kind_for_address(address: &MetadataAddress) -> Result<MetadataKind, MetaDiagnostic> {
    let kind = address
        .segments()
        .next()
        .expect("validated metadata address has a root kind");
    MetadataKind::parse(kind).map_err(|_| {
        invalid(
            "metadataPath",
            format!("setProperties is unsupported for metadata kind `{kind}`"),
        )
    })
}

fn reject_forbidden_fields(
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

fn reject_unknown_fields(
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

fn registry_value<T: Copy>(registry: &[(&str, T)], name: &str) -> Option<T> {
    registry
        .iter()
        .find_map(|(candidate, value)| (*candidate == name).then_some(*value))
}

fn required_metadata_path(
    object: &Map<String, Value>,
    field: &str,
) -> Result<MetadataAddress, MetaDiagnostic> {
    let raw = required_string_diagnostic(object, field)?;
    parse_address(&raw, field)
}

fn parse_address(raw: &str, field: &str) -> Result<MetadataAddress, MetaDiagnostic> {
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).map_err(|error| {
        invalid(
            field,
            format!("invalid logical metadata address: {}", error.message),
        )
    })
}

fn required_string(object: &Map<String, Value>, field: &str) -> Result<String, MetaFailure> {
    required_string_diagnostic(object, field).map_err(Into::into)
}

fn required_string_diagnostic(
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, MetaDiagnostic> {
    let value = object
        .get(field)
        .ok_or_else(|| invalid(field, format!("`{field}` is required")))?;
    nonempty_string(value, field)
}

fn required_string_at(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<String, MetaDiagnostic> {
    let value = object
        .get(key)
        .ok_or_else(|| invalid(field, format!("`{key}` is required")))?;
    nonempty_string(value, field)
}

fn nonempty_string(value: &Value, field: &str) -> Result<String, MetaDiagnostic> {
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

fn optional_string_at(
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

fn required_string_value(
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

fn required_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Map<String, Value>, MetaDiagnostic> {
    object
        .get(field)
        .ok_or_else(|| invalid(field, format!("`{field}` is required")))?
        .as_object()
        .ok_or_else(|| invalid(field, format!("`{field}` must be an object")))
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Vec<Value>, MetaDiagnostic> {
    object
        .get(field)
        .ok_or_else(|| invalid(field, format!("`{field}` is required")))?
        .as_array()
        .ok_or_else(|| invalid(field, format!("`{field}` must be an array")))
}

fn required_array_at<'a>(
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

fn optional_bool(
    object: &Map<String, Value>,
    field: &str,
    default: bool,
) -> Result<bool, MetaFailure> {
    optional_bool_diagnostic(object, field)
        .map(|value| value.unwrap_or(default))
        .map_err(Into::into)
}

fn optional_bool_diagnostic(
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

fn optional_bool_at(
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

fn parse_positive_usize(
    value: Option<&Value>,
    field: &str,
    default: usize,
) -> Result<usize, MetaFailure> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| invalid(field, format!("`{field}` must be a positive integer")))?;
    Ok(value)
}

fn required_u32(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<u32, MetaDiagnostic> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            invalid(
                format!("{field}.{key}"),
                format!("`{key}` must be an unsigned 32-bit integer"),
            )
        })
}

fn invalid(field: impl Into<String>, message: impl Into<String>) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message).with_field(field)
}

pub(crate) fn metadata_input_schema(operation: MetadataOperation) -> Value {
    let string = || json!({"type": "string", "minLength": 1});
    let mut properties = Map::new();
    properties.insert("sourceSet".into(), string());
    let required = match operation {
        MetadataOperation::Info => {
            properties.insert("metadataPath".into(), string());
            properties.insert(
                "sections".into(),
                json!({
                    "type": "array",
                    "uniqueItems": true,
                    "items": {
                        "type": "string",
                        "enum": INFO_SECTIONS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
                    },
                    "default": DEFAULT_INFO_SECTIONS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
                }),
            );
            properties.insert(
                "limit".into(),
                json!({"type": "integer", "minimum": 1, "default": 20}),
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
                }),
            );
            properties.insert("name".into(), string());
            properties.insert("dryRun".into(), json!({"type": "boolean", "default": true}));
            vec!["sourceSet", "kind", "name"]
        }
        MetadataOperation::Edit => {
            properties.insert("metadataPath".into(), string());
            properties.insert(
                "operations".into(),
                json!({
                    "type": "array",
                    "minItems": 1,
                    "items": operation_schema(),
                }),
            );
            properties.insert("dryRun".into(), json!({"type": "boolean", "default": true}));
            vec!["sourceSet", "metadataPath", "operations"]
        }
        MetadataOperation::Remove => {
            properties.insert("metadataPath".into(), string());
            properties.insert("dryRun".into(), json!({"type": "boolean", "default": true}));
            properties.insert("force".into(), json!({"type": "boolean", "default": false}));
            properties.insert(
                "confirm".into(),
                json!({"type": "boolean", "default": false}),
            );
            vec!["sourceSet", "metadataPath"]
        }
    };
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

fn operation_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "op": {
                "type": "string",
                "enum": MetaEditOperationTag::ALL.iter().copied().map(MetaEditOperationTag::as_str).collect::<Vec<_>>(),
            },
            "values": property_values_schema(),
            "collection": {
                "type": "string",
                "enum": MetaCollection::ALL.iter().copied().map(MetaCollection::as_str).collect::<Vec<_>>(),
            },
            "scope": scope_schema(),
            "elements": {
                "type": "array",
                "minItems": 1,
                "items": element_schema(),
            },
            "names": {
                "type": "array",
                "minItems": 1,
                "uniqueItems": true,
                "items": {"type": "string", "minLength": 1},
            },
            "relation": {
                "type": "string",
                "enum": MetaRelation::ALL.iter().copied().map(MetaRelation::as_str).collect::<Vec<_>>(),
            },
            "mode": {
                "type": "string",
                "enum": RelationEditMode::ALL.iter().copied().map(RelationEditMode::as_str).collect::<Vec<_>>(),
            },
            "targets": {
                "type": "array",
                "minItems": 1,
                "items": reference_schema(),
            },
        },
        "required": ["op"],
    })
}

fn property_values_schema() -> Value {
    let properties = METADATA_PROPERTY_SPECS
        .iter()
        .map(|spec| {
            let schema = match spec.value_kind {
                MetaPropertyValueKind::String => json!({"type": "string"}),
                MetaPropertyValueKind::Boolean => json!({"type": "boolean"}),
                MetaPropertyValueKind::UnsignedInteger => {
                    json!({"type": "integer", "minimum": 0, "maximum": u32::MAX})
                }
            };
            (spec.public_name.to_string(), schema)
        })
        .collect::<Map<_, _>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "minProperties": 1,
    })
}

fn scope_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {"tabularSection": {"type": "string", "minLength": 1}},
        "required": ["tabularSection"],
    })
}

fn reference_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {"metadataPath": {"type": "string", "minLength": 1}},
        "required": ["metadataPath"],
    })
}

fn position_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "before": {"type": "string", "minLength": 1},
            "after": {"type": "string", "minLength": 1},
        },
    })
}

fn metadata_type_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "variants": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["string", "number", "boolean", "date", "valueStorage", "reference", "definedType"],
                        },
                        "length": {"type": "integer", "minimum": 0, "maximum": 1024},
                        "allowedLength": {"type": "string", "enum": ["variable", "fixed"]},
                        "digits": {"type": "integer", "minimum": 0, "maximum": 38},
                        "fraction": {"type": "integer", "minimum": 0, "maximum": 38},
                        "sign": {"type": "string", "enum": ["any", "nonNegative"]},
                        "fractions": {"type": "string", "enum": ["date", "time", "dateTime"]},
                        "metadataPath": {"type": "string", "minLength": 1},
                    },
                    "required": ["kind"],
                },
            },
        },
        "required": ["variants"],
    })
}

fn fill_value_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": {"type": "string", "enum": ["string", "number", "boolean", "dateTime", "reference"]},
            "value": {},
            "metadataPath": {"type": "string", "minLength": 1},
        },
        "required": ["kind"],
    })
}

fn element_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": {"type": "string", "minLength": 1},
            "newName": {"type": "string", "minLength": 1},
            "synonym": {"type": "string"},
            "comment": {"type": "string"},
            "type": metadata_type_schema(),
            "required": {"type": "boolean"},
            "fillValue": fill_value_schema(),
            "attributes": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "name": {"type": "string", "minLength": 1},
                        "synonym": {"type": "string"},
                        "comment": {"type": "string"},
                        "type": metadata_type_schema(),
                        "required": {"type": "boolean"},
                        "fillValue": fill_value_schema(),
                        "position": position_schema(),
                    },
                    "required": ["name"],
                },
            },
            "position": position_schema(),
        },
        "required": ["name"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::{MetaDiagnosticCode, MetaEditOperation, MetadataKind};

    fn object(value: Value) -> Map<String, Value> {
        value
            .as_object()
            .expect("test input must be an object")
            .clone()
    }

    fn diagnostic(
        operation: MetadataOperation,
        value: Value,
    ) -> crate::domain::metadata::MetaDiagnostic {
        parse_metadata_request(operation, &object(value))
            .expect_err("input must fail")
            .diagnostics
            .into_iter()
            .next()
            .expect("failure must contain a diagnostic")
    }

    fn edit(operation: Value) -> Value {
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [operation]
        })
    }

    #[test]
    fn parse_info_and_add_apply_defaults_without_accepting_aliases() {
        let MetadataRequest::Info(info) = parse_metadata_request(
            MetadataOperation::Info,
            &object(json!({"sourceSet": "main", "metadataPath": "Документ.Заказ"})),
        )
        .unwrap() else {
            panic!("expected info request")
        };
        assert_eq!(info.source_set, "main");
        assert_eq!(info.metadata_path.as_str(), "Document.Заказ");
        assert_eq!(
            info.sections,
            vec![
                MetaInfoSection::Modules,
                MetaInfoSection::Roles,
                MetaInfoSection::Subscriptions,
                MetaInfoSection::FunctionalOptions,
            ]
        );
        assert_eq!(info.limit, 20);

        for kind in MetadataKind::ALL {
            let MetadataRequest::Add(add) = parse_metadata_request(
                MetadataOperation::Add,
                &object(json!({"sourceSet": "main", "kind": kind.as_str(), "name": "Created"})),
            )
            .unwrap() else {
                panic!("expected add request")
            };
            assert_eq!(add.kind, *kind);
            assert!(add.dry_run);
        }

        let error = diagnostic(
            MetadataOperation::Info,
            json!({"sourceSet": "main", "metadataPath": "Document.Order", "ObjectPath": "x"}),
        );
        assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(error.field.as_deref(), Some("ObjectPath"));
    }

    #[test]
    fn parse_all_five_edit_operations_and_every_collection_into_closed_domain_variants() {
        let MetadataRequest::Edit(request) = parse_metadata_request(
            MetadataOperation::Edit,
            &object(edit(json!({
                "op": "setProperties",
                "values": {"NumberLength": 12, "CheckUnique": true}
            }))),
        )
        .unwrap() else {
            panic!("expected edit request")
        };
        assert!(matches!(
            request.operations[0],
            MetaEditOperation::SetProperties { .. }
        ));
        assert!(request.dry_run);

        for collection in MetaCollection::ALL {
            let collection_name = collection.as_str();
            for (op, payload) in [
                ("add", json!({"elements": [{"name": "Element"}]})),
                (
                    "update",
                    json!({"elements": [{"name": "Element", "comment": "changed"}]}),
                ),
                ("remove", json!({"names": ["Element"]})),
            ] {
                let mut operation = payload.as_object().unwrap().clone();
                operation.insert("op".into(), json!(op));
                operation.insert("collection".into(), json!(collection_name));
                let MetadataRequest::Edit(request) = parse_metadata_request(
                    MetadataOperation::Edit,
                    &object(edit(Value::Object(operation))),
                )
                .unwrap() else {
                    panic!("expected edit request")
                };
                match (&request.operations[0], op) {
                    (
                        MetaEditOperation::Add {
                            collection: actual, ..
                        },
                        "add",
                    )
                    | (
                        MetaEditOperation::Update {
                            collection: actual, ..
                        },
                        "update",
                    )
                    | (
                        MetaEditOperation::Remove {
                            collection: actual, ..
                        },
                        "remove",
                    ) => {
                        assert_eq!(*actual, *collection)
                    }
                    (actual, _) => {
                        panic!("wrong conversion for {op}/{collection_name}: {actual:?}")
                    }
                }
            }
        }

        for relation in MetaRelation::ALL {
            let relation_name = relation.as_str();
            for mode in RelationEditMode::ALL {
                let mode_name = mode.as_str();
                let MetadataRequest::Edit(request) = parse_metadata_request(
                    MetadataOperation::Edit,
                    &object(edit(json!({
                        "op": "editRelations",
                        "relation": relation_name,
                        "mode": mode_name,
                        "targets": [{"metadataPath": "Catalog.Items"}]
                    }))),
                )
                .unwrap() else {
                    panic!("expected edit request")
                };
                assert!(matches!(
                    &request.operations[0],
                    MetaEditOperation::EditRelations { relation: actual_relation, mode: actual_mode, .. }
                        if *actual_relation == *relation && *actual_mode == *mode
                ));
            }
        }
    }

    #[test]
    fn parse_rejects_conditional_shapes_with_indexed_field_diagnostics() {
        let cases = [
            (json!({"op": "setProperties"}), "values"),
            (
                json!({"op": "setProperties", "values": {"Comment": "x"}, "collection": "attributes"}),
                "collection",
            ),
            (json!({"op": "add", "collection": "attributes"}), "elements"),
            (
                json!({"op": "remove", "collection": "attributes", "names": []}),
                "names",
            ),
            (
                json!({"op": "editRelations", "relation": "owners", "mode": "add", "targets": []}),
                "targets",
            ),
            (
                json!({"op": "add", "collection": "forms", "scope": {"tabularSection": "Lines"}, "elements": [{"name": "F"}]}),
                "scope",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "position": {"before": "B", "after": "C"}}]}),
                "position",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": "String(10) | req"}]}),
                "elements.type",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": {"variants": [{"kind": "number", "digits": 10, "fraction": 2}]}}]}),
                "elements.type.variants[0].sign",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": {"variants": "number"}}]}),
                "elements.type.variants",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "fillValue": {"kind": "reference"}}]}),
                "elements.fillValue.metadataPath",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "unknown": true}]}),
                "elements.unknown",
            ),
            (
                json!({"op": "add", "collection": "tabularSections", "elements": [{"name": "Lines", "attributes": []}]}),
                "elements.attributes",
            ),
            (
                json!({"op": "add", "collection": "tabularSections", "elements": [{"name": "Lines", "attributes": "Quantity"}]}),
                "elements.attributes",
            ),
            (
                json!({"op": "setProperties", "values": {"UnknownProperty": true}}),
                "values.UnknownProperty",
            ),
        ];

        for (operation, expected_field) in cases {
            let error = diagnostic(MetadataOperation::Edit, edit(operation));
            assert_eq!(
                error.code,
                MetaDiagnosticCode::InvalidArguments,
                "{error:?}"
            );
            assert_eq!(error.operation_index, Some(0), "{error:?}");
            assert_eq!(error.field.as_deref(), Some(expected_field), "{error:?}");
        }

        let error = diagnostic(
            MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": "add attribute A;;add attribute B"
            }),
        );
        assert_eq!(error.field.as_deref(), Some("operations"));
    }

    #[test]
    fn parse_rejects_unknown_top_level_fields_missing_required_values_and_wrong_scalars() {
        let cases = [
            (
                MetadataOperation::Info,
                json!({"sourceSet": "main", "metadataPath": "Document.Order", "path": "legacy"}),
                "path",
            ),
            (
                MetadataOperation::Add,
                json!({"sourceSet": "main", "name": "A"}),
                "kind",
            ),
            (
                MetadataOperation::Edit,
                json!({"sourceSet": "main", "metadataPath": "Document.Order", "operations": []}),
                "operations",
            ),
            (
                MetadataOperation::Remove,
                json!({"sourceSet": " main ", "metadataPath": "Document.Order"}),
                "sourceSet",
            ),
        ];
        for (operation, input, field) in cases {
            let error = diagnostic(operation, input);
            assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
            assert_eq!(error.field.as_deref(), Some(field), "{error:?}");
        }
    }

    #[test]
    fn parse_remove_requires_the_complete_force_apply_gate() {
        let base = || {
            json!({
                "sourceSet": "main",
                "metadataPath": "Catalog.Items"
            })
        };
        let MetadataRequest::Remove(preview) =
            parse_metadata_request(MetadataOperation::Remove, &object(base())).unwrap()
        else {
            panic!("expected remove request")
        };
        assert!(preview.dry_run);
        assert!(!preview.force);
        assert!(!preview.confirm);

        let MetadataRequest::Remove(forced) = parse_metadata_request(
            MetadataOperation::Remove,
            &object(json!({
                "sourceSet": "main",
                "metadataPath": "Catalog.Items",
                "dryRun": false,
                "force": true,
                "confirm": true
            })),
        )
        .unwrap() else {
            panic!("expected remove request")
        };
        assert!(!forced.dry_run && forced.force && forced.confirm);

        for incomplete in [
            json!({"sourceSet": "main", "metadataPath": "Catalog.Items", "force": true}),
            json!({"sourceSet": "main", "metadataPath": "Catalog.Items", "dryRun": false, "force": true}),
            json!({"sourceSet": "main", "metadataPath": "Catalog.Items", "dryRun": false, "confirm": true}),
        ] {
            let error = diagnostic(MetadataOperation::Remove, incomplete);
            assert_eq!(error.field.as_deref(), Some("force"), "{error:?}");
        }
    }
}

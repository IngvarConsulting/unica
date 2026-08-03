#![allow(dead_code)] // The handler remains off-registry until the coordinated public switch.

use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::metadata::{
    DateFractions, MetaCollection, MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation,
    MetaEditOperationTag, MetaElementInput, MetaElementUpdateInput, MetaFillValue, MetaPosition,
    MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetaPropertyValueKind, MetaRelation,
    MetaRelationTarget, MetaScope, MetaValidationStatus, MetadataFieldPath, MetadataKind,
    MetadataReference, MetadataType, MetadataTypeVariant, NumberSign, RelationEditMode,
    StringLengthMode, METADATA_PROPERTY_SPECS,
};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::workspace::WorkspaceContext;
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

pub(crate) fn invoke(
    operation: MetadataOperation,
    ports: &dyn ApplicationPorts,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let request = match parse_metadata_request(operation, args) {
        Ok(request) => request,
        Err(failure) => {
            return Ok(metadata_failure(
                "metadata arguments are invalid",
                failure,
                None,
            ))
        }
    };
    match request {
        MetadataRequest::Info(request) => invoke_info(ports, &request, context, cancellation),
        request => invoke_mutation(ports, request, context, cancellation),
    }
}

fn invoke_info(
    ports: &dyn ApplicationPorts,
    request: &MetaInfoRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let read = match ports.read_metadata_local(request, context, cancellation) {
        Ok(read) => read,
        Err(failure) => return Ok(metadata_failure("metadata read failed", failure, None)),
    };
    let validation = ports.validate_metadata(&read.validation_subject, context, cancellation);
    let related = ports.read_metadata_related(request, &read.local, context, cancellation);
    let failed = validation.status == MetaValidationStatus::Failed;
    let diagnostics = validation.diagnostics.clone();
    let data = serde_json::to_value(read.local.into_info(validation, related))
        .map_err(|error| format!("cannot serialize metadata info result: {error}"))?;
    if failed {
        return Ok(metadata_failure(
            "metadata validation failed",
            MetaFailure { diagnostics },
            Some(data),
        ));
    }
    Ok(metadata_success(
        "metadata information inspected",
        data,
        Vec::new(),
        Vec::new(),
    ))
}

fn invoke_mutation(
    ports: &dyn ApplicationPorts,
    request: MetadataRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let dry_run = mutation_dry_run(&request);
    let prepared = match ports.prepare_metadata_mutation(&request, context, cancellation) {
        Ok(prepared) => prepared,
        Err(failure) => {
            return Ok(metadata_failure(
                "metadata mutation is unavailable",
                failure,
                None,
            ))
        }
    };
    let validation = ports.validate_metadata(prepared.validation_subject(), context, cancellation);
    if validation.status == MetaValidationStatus::Failed {
        let diagnostics = validation.diagnostics.clone();
        let mut data = prepared.preview().clone();
        data.validation = validation;
        let data = serde_json::to_value(data)
            .map_err(|error| format!("cannot serialize metadata mutation result: {error}"))?;
        return Ok(metadata_failure(
            "metadata validation failed",
            MetaFailure { diagnostics },
            Some(data),
        ));
    }

    if dry_run {
        let mut data = prepared.preview().clone();
        data.validation = validation;
        let projected_events = metadata_change_event(&data, true);
        let data = serde_json::to_value(data)
            .map_err(|error| format!("cannot serialize metadata mutation preview: {error}"))?;
        return Ok(metadata_success(
            "metadata mutation preview prepared",
            data,
            Vec::new(),
            projected_events,
        ));
    }

    if cancellation.is_cancelled() {
        return Ok(HandlerOutcome::plain(AdapterOutcome::cancelled(
            "metadata mutation stopped before publication",
        )));
    }

    let report = match prepared.publish(cancellation) {
        Ok(report) => report,
        Err(failure) => {
            return Ok(metadata_failure(
                "metadata publication failed",
                failure,
                None,
            ))
        }
    };
    let mut data = report.data;
    data.validation = validation;
    let events = metadata_change_event(&data, false);
    let data = serde_json::to_value(data)
        .map_err(|error| format!("cannot serialize metadata mutation result: {error}"))?;
    Ok(metadata_success(
        "metadata mutation published",
        data,
        events,
        Vec::new(),
    ))
}

fn mutation_dry_run(request: &MetadataRequest) -> bool {
    match request {
        MetadataRequest::Info(_) => false,
        MetadataRequest::Add(request) => request.dry_run,
        MetadataRequest::Edit(request) => request.dry_run,
        MetadataRequest::Remove(request) => request.dry_run,
    }
}

fn metadata_change_event(
    data: &crate::domain::metadata::MetaMutationData,
    projected: bool,
) -> Vec<DomainEvent> {
    if !data.changed {
        return Vec::new();
    }
    let artifact = if projected {
        format!("preview:{}", data.metadata_path.as_str())
    } else {
        data.metadata_path.as_str().to_string()
    };
    vec![DomainEvent::new(DomainEventKind::MetadataChanged, artifact)]
}

fn metadata_success(
    summary: &str,
    data: Value,
    events: Vec<DomainEvent>,
    projected_events: Vec<DomainEvent>,
) -> HandlerOutcome {
    HandlerOutcome {
        adapter: AdapterOutcome::ok(summary),
        data: Some(data),
        job: None,
        events,
        projected_events,
        recorded_cache: None,
        diagnostics: None,
    }
}

fn metadata_failure(summary: &str, failure: MetaFailure, data: Option<Value>) -> HandlerOutcome {
    let errors = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    let diagnostics = serde_json::to_value(&failure.diagnostics)
        .expect("metadata diagnostics are always serializable");
    HandlerOutcome {
        adapter: AdapterOutcome {
            ok: false,
            summary: summary.to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors,
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
        data,
        job: None,
        events: Vec::new(),
        projected_events: Vec::new(),
        recorded_cache: None,
        diagnostics: Some(diagnostics),
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
                .enumerate()
                .map(|(index, element)| {
                    parse_element_input(element, &format!("elements[{index}]"), true)
                })
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::add(collection, scope, elements)
        }
        MetaEditOperationTag::Update => {
            reject_forbidden_fields(object, &["values", "names", "relation", "mode", "targets"])?;
            let collection = parse_collection(object)?;
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
                &["values", "elements", "relation", "mode", "targets"],
            )?;
            let collection = parse_collection(object)?;
            let scope = parse_scope(object.get("scope"))?;
            let names = required_array(object, "names")?
                .iter()
                .enumerate()
                .map(|(index, value)| nonempty_string(value, &format!("names[{index}]")))
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
                .enumerate()
                .map(|(index, target)| {
                    let field = format!("targets[{index}]");
                    if relation == MetaRelation::InputByString {
                        parse_field_reference(target, &field).map(MetaRelationTarget::Field)
                    } else {
                        parse_reference(target, &field).map(MetaRelationTarget::Object)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            MetaEditOperation::edit_relation_targets(relation, mode, targets)
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

fn parse_element_input(
    value: &Value,
    field: &str,
    allows_attributes: bool,
) -> Result<MetaElementInput, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "element must be an object"))?;
    let allowed = if allows_attributes {
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
    } else {
        &[
            "name",
            "synonym",
            "comment",
            "type",
            "required",
            "fillValue",
            "position",
        ][..]
    };
    reject_unknown_fields(object, allowed, &format!("{field}."))?;
    let attributes_field = format!("{field}.attributes");
    let attributes = object
        .get("attributes")
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
        .transpose()?;
    Ok(MetaElementInput {
        name: required_string_at(object, "name", &format!("{field}.name"))?,
        synonym: optional_string_at(object, "synonym", &format!("{field}.synonym"))?,
        comment: optional_string_at(object, "comment", &format!("{field}.comment"))?,
        r#type: object
            .get("type")
            .map(|value| parse_metadata_type(value, &format!("{field}.type")))
            .transpose()?,
        required: optional_bool_at(object, "required", &format!("{field}.required"))?,
        fill_value: object
            .get("fillValue")
            .map(|value| parse_fill_value(value, &format!("{field}.fillValue")))
            .transpose()?,
        attributes,
        position: object
            .get("position")
            .map(|value| parse_position(value, &format!("{field}.position")))
            .transpose()?,
    })
}

fn parse_element_update(
    value: &Value,
    field: &str,
) -> Result<MetaElementUpdateInput, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "element must be an object"))?;
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
        &format!("{field}."),
    )?;
    Ok(MetaElementUpdateInput {
        name: required_string_at(object, "name", &format!("{field}.name"))?,
        new_name: optional_string_at(object, "newName", &format!("{field}.newName"))?,
        synonym: optional_string_at(object, "synonym", &format!("{field}.synonym"))?,
        comment: optional_string_at(object, "comment", &format!("{field}.comment"))?,
        r#type: object
            .get("type")
            .map(|value| parse_metadata_type(value, &format!("{field}.type")))
            .transpose()?,
        required: optional_bool_at(object, "required", &format!("{field}.required"))?,
        fill_value: object
            .get("fillValue")
            .map(|value| parse_fill_value(value, &format!("{field}.fillValue")))
            .transpose()?,
        position: object
            .get("position")
            .map(|value| parse_position(value, &format!("{field}.position")))
            .transpose()?,
    })
}

fn parse_position(value: &Value, field: &str) -> Result<MetaPosition, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "position must be an object"))?;
    reject_unknown_fields(object, &["before", "after"], &format!("{field}."))?;
    MetaPosition::new_at(
        optional_string_at(object, "before", &format!("{field}.before"))?,
        optional_string_at(object, "after", &format!("{field}.after"))?,
        field,
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
        "binaryData" => (
            &["kind", "length", "allowedLength"][..],
            MetadataTypeVariant::BinaryData {
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

fn parse_reference(value: &Value, field: &str) -> Result<MetadataReference, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "relation target must be an object"))?;
    reject_unknown_fields(object, &["metadataPath"], &format!("{field}."))?;
    let path_field = format!("{field}.metadataPath");
    let raw = required_string_at(object, "metadataPath", &path_field)?;
    Ok(MetadataReference {
        metadata_path: parse_address(&raw, &path_field)?,
    })
}

fn parse_field_reference(value: &Value, field: &str) -> Result<MetadataFieldPath, MetaDiagnostic> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(field, "inputByString target must be an object"))?;
    reject_unknown_fields(object, &["fieldPath"], &format!("{field}."))?;
    let path_field = format!("{field}.fieldPath");
    let raw = required_string_at(object, "fieldPath", &path_field)?;
    MetadataFieldPath::parse(&raw).map_err(|mut diagnostic| {
        diagnostic.field = Some(path_field);
        diagnostic
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
                "items": relation_target_schema(),
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

fn relation_target_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "minProperties": 1,
        "maxProperties": 1,
        "properties": {
            "metadataPath": {"type": "string", "minLength": 1},
            "fieldPath": {"type": "string", "minLength": 1},
        },
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
                            "enum": ["string", "number", "boolean", "date", "binaryData", "valueStorage", "reference", "definedType"],
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
    use crate::application::ports::{
        ApplicationPorts, HandlerOutcome, MetaLocalInfo, MetaPublishReport, MetadataRead,
        MetadataResourceImage, MetadataResourceRole, MetadataValidationSubject,
        PreparedMetadataMutation, SupportGuardCheck,
    };
    use crate::application::ToolSpec;
    use crate::domain::cache::{CacheAccess, CacheReport};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::events::{DomainEvent, DomainEventKind};
    use crate::domain::metadata::{
        MetaCollectionsData, MetaCompleteness, MetaDiagnosticCode, MetaEditOperation,
        MetaFreshness, MetaMutationData, MetaRelatedItem, MetaRelatedSection, MetaRelatedSections,
        MetaRelatedStatus, MetaSupportStatus, MetaValidationData, MetaValidationStatus,
        MetadataKind,
    };
    use crate::domain::workspace::WorkspaceContext;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

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

    #[derive(Default)]
    struct CoordinatorState {
        calls: Vec<&'static str>,
        publish_calls: usize,
    }

    struct FakePreparedMutation {
        state: Arc<Mutex<CoordinatorState>>,
        preview: MetaMutationData,
        validation_subject: MetadataValidationSubject,
        publication: Result<MetaPublishReport, MetaFailure>,
    }

    impl PreparedMetadataMutation for FakePreparedMutation {
        fn preview(&self) -> &MetaMutationData {
            &self.preview
        }

        fn validation_subject(&self) -> &MetadataValidationSubject {
            &self.validation_subject
        }

        fn publish(
            self: Box<Self>,
            _cancellation: &CancellationToken,
        ) -> Result<MetaPublishReport, MetaFailure> {
            let mut state = self.state.lock().unwrap();
            state.calls.push("publish");
            state.publish_calls += 1;
            drop(state);
            self.publication
        }
    }

    struct FakeMetadataPorts {
        state: Arc<Mutex<CoordinatorState>>,
        cache_events: Arc<Mutex<Vec<DomainEvent>>>,
        read: Mutex<Option<Result<MetadataRead, MetaFailure>>>,
        related: MetaRelatedSections,
        validation: MetaValidationData,
        prepared: Mutex<Option<Result<Box<dyn PreparedMetadataMutation>, MetaFailure>>>,
        cancel_after_validation: Option<CancellationToken>,
    }

    impl ApplicationPorts for FakeMetadataPorts {
        fn discover_workspace(
            &self,
            _requested_cwd: Option<PathBuf>,
        ) -> Result<WorkspaceContext, String> {
            Ok(coordinator_context())
        }

        fn validate_tool_context(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _dry_run: bool,
            _context: &WorkspaceContext,
        ) -> Result<(), String> {
            Ok(())
        }

        fn read_metadata_local(
            &self,
            _request: &MetaInfoRequest,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> Result<MetadataRead, MetaFailure> {
            self.state.lock().unwrap().calls.push("read");
            self.read
                .lock()
                .unwrap()
                .take()
                .expect("read is called exactly once")
        }

        fn read_metadata_related(
            &self,
            _request: &MetaInfoRequest,
            _local: &MetaLocalInfo,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> MetaRelatedSections {
            self.state.lock().unwrap().calls.push("related");
            self.related.clone()
        }

        fn validate_metadata(
            &self,
            _subject: &MetadataValidationSubject,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> MetaValidationData {
            self.state.lock().unwrap().calls.push("validate");
            if let Some(cancellation) = self.cancel_after_validation.as_ref() {
                cancellation.cancel();
            }
            self.validation.clone()
        }

        fn prepare_metadata_mutation(
            &self,
            _request: &MetadataRequest,
            _context: &WorkspaceContext,
            _cancellation: &CancellationToken,
        ) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
            self.state.lock().unwrap().calls.push("prepare");
            self.prepared
                .lock()
                .unwrap()
                .take()
                .expect("mutation is prepared exactly once")
        }

        fn evaluate_support_guard(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
        ) -> Result<SupportGuardCheck, String> {
            Ok(SupportGuardCheck::Allow)
        }

        fn invoke_handler(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _dry_run: bool,
            _cancellation: &CancellationToken,
        ) -> Result<HandlerOutcome, String> {
            unreachable!("metadata has a dedicated coordinator")
        }

        fn cache_report(
            &self,
            context: &WorkspaceContext,
            events: &[DomainEvent],
            dry_run: bool,
            _cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            *self.cache_events.lock().unwrap() = events.to_vec();
            Ok(CacheReport {
                mode: if dry_run { "preview" } else { "apply" }.to_string(),
                root: context.cache_root.display().to_string(),
                workspace_epoch: context.workspace_epoch,
                events: events
                    .iter()
                    .map(|event| event.name().to_string())
                    .collect(),
                invalidated: Vec::new(),
                refreshed: Vec::new(),
                lazy_rebuilt: Vec::new(),
                stale: Vec::new(),
                fresh: Vec::new(),
                publication_warnings: Vec::new(),
            })
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
    }

    fn coordinator_context() -> WorkspaceContext {
        WorkspaceContext {
            cwd: PathBuf::from("/workspace"),
            workspace_root: PathBuf::from("/workspace"),
            cache_root: PathBuf::from("/workspace/.unica/cache"),
            workspace_epoch: 1,
        }
    }

    fn passed_validation() -> MetaValidationData {
        MetaValidationData {
            status: MetaValidationStatus::Passed,
            diagnostics: Vec::new(),
        }
    }

    fn unavailable_section() -> MetaRelatedSection<MetaRelatedItem> {
        MetaRelatedSection {
            status: MetaRelatedStatus::Unavailable,
            freshness: MetaFreshness::Unknown,
            completeness: MetaCompleteness::Unknown,
            total: 0,
            returned: 0,
            truncated: false,
            items: Vec::new(),
            diagnostics: vec![MetaDiagnostic::error(
                MetaDiagnosticCode::CapabilityUnavailable,
                "related metadata is unavailable",
            )],
        }
    }

    fn unavailable_related() -> MetaRelatedSections {
        MetaRelatedSections {
            modules: unavailable_section(),
            roles: unavailable_section(),
            subscriptions: unavailable_section(),
            functional_options: unavailable_section(),
            predefined_items: Some(unavailable_section()),
        }
    }

    fn empty_collections() -> MetaCollectionsData {
        MetaCollectionsData {
            attributes: Vec::new(),
            tabular_sections: Vec::new(),
            dimensions: Vec::new(),
            resources: Vec::new(),
            enum_values: Vec::new(),
            columns: Vec::new(),
            forms: Vec::new(),
            templates: Vec::new(),
            commands: Vec::new(),
        }
    }

    fn validation_subject() -> MetadataValidationSubject {
        MetadataValidationSubject {
            target: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Document.Order")
                .unwrap(),
            resources: vec![MetadataResourceImage {
                role: MetadataResourceRole::Descriptor,
                bytes: b"<MetaDataObject/>".to_vec(),
            }],
        }
    }

    fn mutation_data(changed: bool) -> MetaMutationData {
        MetaMutationData {
            metadata_path: MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "Document.Order",
            )
            .unwrap(),
            changed,
            publication_plan: Vec::new(),
            validation: passed_validation(),
            diagnostics: Vec::new(),
        }
    }

    fn fake_ports(
        validation: MetaValidationData,
        preview_changed: bool,
        publication: Result<MetaPublishReport, MetaFailure>,
    ) -> (FakeMetadataPorts, Arc<Mutex<CoordinatorState>>) {
        let state = Arc::new(Mutex::new(CoordinatorState::default()));
        let plan = FakePreparedMutation {
            state: Arc::clone(&state),
            preview: mutation_data(preview_changed),
            validation_subject: validation_subject(),
            publication,
        };
        (
            FakeMetadataPorts {
                state: Arc::clone(&state),
                cache_events: Arc::new(Mutex::new(Vec::new())),
                read: Mutex::new(None),
                related: unavailable_related(),
                validation,
                prepared: Mutex::new(Some(Ok(Box::new(plan)))),
                cancel_after_validation: None,
            },
            state,
        )
    }

    fn add_args(dry_run: bool) -> Map<String, Value> {
        object(json!({
            "sourceSet": "main",
            "kind": "Document",
            "name": "Order",
            "dryRun": dry_run
        }))
    }

    #[test]
    fn coordinator_info_reads_validates_then_enriches_and_keeps_local_data_when_related_is_unavailable(
    ) {
        let state = Arc::new(Mutex::new(CoordinatorState::default()));
        let subject = validation_subject();
        let ports = FakeMetadataPorts {
            state: Arc::clone(&state),
            cache_events: Arc::new(Mutex::new(Vec::new())),
            read: Mutex::new(Some(Ok(MetadataRead {
                local: MetaLocalInfo {
                    metadata_path: subject.target.clone(),
                    kind: MetadataKind::Document,
                    name: "Order".to_string(),
                    synonym: Some("Order".to_string()),
                    support: MetaSupportStatus::Supported,
                    properties: Vec::new(),
                    owners: Vec::new(),
                    collections: empty_collections(),
                },
                validation_subject: subject,
            }))),
            related: unavailable_related(),
            validation: passed_validation(),
            prepared: Mutex::new(None),
            cancel_after_validation: None,
        };

        let outcome = invoke(
            MetadataOperation::Info,
            &ports,
            &object(json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "sections": ["modules", "roles", "subscriptions", "functionalOptions", "predefinedItems"]
            })),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert_eq!(outcome.data.as_ref().unwrap()["name"], "Order");
        assert_eq!(
            outcome.data.as_ref().unwrap()["related"]["modules"]["status"],
            "unavailable"
        );
        assert_eq!(state.lock().unwrap().calls, ["read", "validate", "related"]);
    }

    #[test]
    fn coordinator_validation_failure_blocks_publication_and_returns_typed_diagnostics() {
        let diagnostic = MetaDiagnostic::error(
            MetaDiagnosticCode::ValidationFailed,
            "post-image validation failed",
        );
        let failed_validation = MetaValidationData {
            status: MetaValidationStatus::Failed,
            diagnostics: vec![diagnostic.clone()],
        };
        let (ports, state) = fake_ports(
            failed_validation,
            true,
            Ok(MetaPublishReport {
                data: mutation_data(true),
            }),
        );

        let outcome = invoke(
            MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert_eq!(outcome.diagnostics, Some(json!([diagnostic])));
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        let state = state.lock().unwrap();
        assert_eq!(state.calls, ["prepare", "validate"]);
        assert_eq!(state.publish_calls, 0);
    }

    #[test]
    fn coordinator_preview_never_publishes_and_emits_only_a_projected_metadata_event() {
        let (ports, state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport {
                data: mutation_data(true),
            }),
        );

        let outcome = invoke(
            MetadataOperation::Add,
            &ports,
            &add_args(true),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert!(outcome.events.is_empty());
        assert_eq!(outcome.projected_events.len(), 1);
        assert_eq!(
            outcome.projected_events[0].kind,
            DomainEventKind::MetadataChanged
        );
        let state = state.lock().unwrap();
        assert_eq!(state.calls, ["prepare", "validate"]);
        assert_eq!(state.publish_calls, 0);
    }

    #[test]
    fn coordinator_apply_publishes_once_and_emits_only_an_actual_metadata_event() {
        let (ports, state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport {
                data: mutation_data(true),
            }),
        );

        let outcome = invoke(
            MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert_eq!(outcome.events.len(), 1);
        assert_eq!(outcome.events[0].kind, DomainEventKind::MetadataChanged);
        assert!(outcome.projected_events.is_empty());
        let state = state.lock().unwrap();
        assert_eq!(state.calls, ["prepare", "validate", "publish"]);
        assert_eq!(state.publish_calls, 1);
    }

    #[test]
    fn coordinator_internal_dispatch_preserves_the_actual_event_for_cache_publication() {
        let (ports, _state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport {
                data: mutation_data(true),
            }),
        );
        let cache_events = Arc::clone(&ports.cache_events);
        let spec = ToolSpec {
            name: "unica.meta.add",
            description: "internal metadata coordinator test",
            mutating: true,
            cache_access: CacheAccess::default(),
            handler: crate::application::ToolHandler::Metadata {
                operation: MetadataOperation::Add,
            },
        };

        let result = crate::application::call_tool(
            spec,
            &add_args(false),
            &ports,
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(result.ok);
        assert_eq!(result.stdout, None);
        assert_eq!(result.data.as_ref().unwrap()["changed"], true);
        assert_eq!(result.cache.events, ["MetadataChanged"]);
        assert_eq!(
            cache_events.lock().unwrap()[0].kind,
            DomainEventKind::MetadataChanged
        );
    }

    #[test]
    fn coordinator_cancellation_after_validation_never_calls_publication() {
        let cancellation = CancellationToken::new();
        let (mut ports, state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport {
                data: mutation_data(true),
            }),
        );
        ports.cancel_after_validation = Some(cancellation.clone());

        let outcome = invoke(
            MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &cancellation,
        )
        .unwrap();

        assert!(!outcome.adapter.ok);
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        let state = state.lock().unwrap();
        assert_eq!(state.calls, ["prepare", "validate"]);
        assert_eq!(state.publish_calls, 0);
    }

    #[test]
    fn coordinator_apply_noop_uses_the_publication_report_and_emits_no_event() {
        let (ports, state) = fake_ports(
            passed_validation(),
            true,
            Ok(MetaPublishReport {
                data: mutation_data(false),
            }),
        );

        let outcome = invoke(
            MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(outcome.adapter.ok);
        assert_eq!(outcome.data.as_ref().unwrap()["changed"], false);
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        assert_eq!(state.lock().unwrap().publish_calls, 1);
    }

    #[test]
    fn coordinator_rollback_failure_stays_a_typed_error_without_events() {
        let diagnostic = MetaDiagnostic::error(
            MetaDiagnosticCode::RollbackFailed,
            "publication failed and rollback did not restore the preimage",
        );
        let (ports, state) = fake_ports(
            passed_validation(),
            true,
            Err(MetaFailure {
                diagnostics: vec![diagnostic.clone()],
            }),
        );

        let outcome = invoke(
            MetadataOperation::Add,
            &ports,
            &add_args(false),
            &coordinator_context(),
            &CancellationToken::new(),
        )
        .unwrap();

        assert!(!outcome.adapter.ok);
        assert_eq!(outcome.adapter.stdout, None);
        assert_eq!(outcome.diagnostics, Some(json!([diagnostic])));
        assert!(outcome.events.is_empty());
        assert!(outcome.projected_events.is_empty());
        assert_eq!(state.lock().unwrap().publish_calls, 1);
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
            let target = if *relation == MetaRelation::InputByString {
                json!({"fieldPath": "Catalog.Items.StandardAttribute.Code"})
            } else {
                json!({"metadataPath": "Catalog.Items"})
            };
            for mode in RelationEditMode::ALL {
                let mode_name = mode.as_str();
                let MetadataRequest::Edit(request) = parse_metadata_request(
                    MetadataOperation::Edit,
                    &object(edit(json!({
                        "op": "editRelations",
                        "relation": relation_name,
                        "mode": mode_name,
                        "targets": [target]
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
                "elements[0].position",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": "String(10) | req"}]}),
                "elements[0].type",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": {"variants": [{"kind": "number", "digits": 10, "fraction": 2}]}}]}),
                "elements[0].type.variants[0].sign",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": {"variants": "number"}}]}),
                "elements[0].type.variants",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "fillValue": {"kind": "reference"}}]}),
                "elements[0].fillValue.metadataPath",
            ),
            (
                json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "unknown": true}]}),
                "elements[0].unknown",
            ),
            (
                json!({"op": "add", "collection": "tabularSections", "elements": [{"name": "Lines", "attributes": []}]}),
                "elements[0].attributes",
            ),
            (
                json!({"op": "add", "collection": "tabularSections", "elements": [{"name": "Lines", "attributes": "Quantity"}]}),
                "elements[0].attributes",
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
    fn parse_accepts_binary_data_type_qualifiers() {
        let MetadataRequest::Edit(request) = parse_metadata_request(
            MetadataOperation::Edit,
            &object(edit(json!({
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Payload",
                    "type": {"variants": [{
                        "kind": "binaryData",
                        "length": 512,
                        "allowedLength": "fixed"
                    }]}
                }]
            }))),
        )
        .unwrap() else {
            panic!("expected edit request")
        };
        let MetaEditOperation::Add { elements, .. } = &request.operations[0] else {
            panic!("expected add operation")
        };
        assert!(matches!(
            elements[0].r#type.as_ref().unwrap().variants[0],
            MetadataTypeVariant::BinaryData {
                length: 512,
                allowed_length: StringLengthMode::Fixed
            }
        ));
    }

    #[test]
    fn parse_reports_exact_paths_for_later_and_nested_array_items() {
        let cases = [
            (
                json!({
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{"name": "First"}, {"comment": "missing name"}]
                }),
                "elements[1].name",
            ),
            (
                json!({
                    "op": "add",
                    "collection": "forms",
                    "elements": [
                        {"name": "First"},
                        {"name": "Second", "type": {"variants": [{"kind": "boolean"}]}}
                    ]
                }),
                "elements[1].type",
            ),
            (
                json!({
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{"name": "Duplicate"}, {"name": "Duplicate"}]
                }),
                "elements[1].name",
            ),
            (
                json!({
                    "op": "update",
                    "collection": "attributes",
                    "elements": [
                        {"name": "First", "comment": "changed"},
                        {"name": "Second", "position": {"before": "A", "after": "B"}}
                    ]
                }),
                "elements[1].position",
            ),
            (
                json!({
                    "op": "add",
                    "collection": "tabularSections",
                    "elements": [{
                        "name": "Lines",
                        "attributes": [{"name": "First"}, {"type": {"variants": []}}]
                    }]
                }),
                "elements[0].attributes[1].name",
            ),
            (
                json!({
                    "op": "add",
                    "collection": "tabularSections",
                    "elements": [{
                        "name": "Lines",
                        "attributes": [{"name": "Duplicate"}, {"name": "Duplicate"}]
                    }]
                }),
                "elements[0].attributes[1].name",
            ),
            (
                json!({
                    "op": "remove",
                    "collection": "attributes",
                    "names": ["First", ""]
                }),
                "names[1]",
            ),
            (
                json!({
                    "op": "remove",
                    "collection": "attributes",
                    "names": ["Duplicate", "Duplicate"]
                }),
                "names[1]",
            ),
            (
                json!({
                    "op": "editRelations",
                    "relation": "owners",
                    "mode": "add",
                    "targets": [
                        {"metadataPath": "Catalog.Items"},
                        {"metadataPath": ""}
                    ]
                }),
                "targets[1].metadataPath",
            ),
            (
                json!({
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Typed",
                        "type": {"variants": [
                            {"kind": "boolean"},
                            {"kind": "number", "digits": 10, "fraction": 2}
                        ]}
                    }]
                }),
                "elements[0].type.variants[1].sign",
            ),
        ];

        for (operation, expected_field) in cases {
            let error = diagnostic(MetadataOperation::Edit, edit(operation));
            assert_eq!(error.operation_index, Some(0), "{error:?}");
            assert_eq!(error.field.as_deref(), Some(expected_field), "{error:?}");
        }

        let error = diagnostic(
            MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [
                    {"op": "setProperties", "values": {"Comment": "valid"}},
                    {"op": "remove", "collection": "attributes", "names": ["First", ""]}
                ]
            }),
        );
        assert_eq!(error.operation_index, Some(1));
        assert_eq!(error.field.as_deref(), Some("names[1]"));
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

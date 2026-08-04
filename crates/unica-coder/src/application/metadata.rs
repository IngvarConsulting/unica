use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::metadata::{
    metadata_collection_spec, metadata_kind_collections, validate_metadata_kind_collection,
    DateFractions, MetaCollection, MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation,
    MetaEditOperationTag, MetaElementInput, MetaElementUpdateInput, MetaFillValue, MetaPosition,
    MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetaPropertyValueKind, MetaRelation,
    MetaRelationTarget, MetaScope, MetaValidationStatus, MetadataFieldPath, MetadataKind,
    MetadataReference, MetadataType, MetadataTypeVariant, NumberSign, RelationEditMode,
    StringLengthMode, METADATA_PROPERTY_SPECS,
};
use crate::domain::source_target::{
    metadata_address_kind_spellings, MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
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
    pub(crate) operations: Vec<MetaEditOperation>,
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
    let mut validation =
        ports.validate_metadata_read(&read.validation_subject, context, cancellation);
    if read.local.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == crate::domain::metadata::MetaDiagnosticSeverity::Error
    }) {
        validation.status = MetaValidationStatus::Failed;
    }
    validation
        .diagnostics
        .extend(read.local.diagnostics.iter().cloned());
    let related = if request.sections.is_empty() {
        empty_related_sections()
    } else {
        ports.read_metadata_related(request, &read.local, context, cancellation)
    };
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

fn empty_related_sections() -> crate::domain::metadata::MetaRelatedSections {
    crate::domain::metadata::MetaRelatedSections {
        modules: None,
        roles: None,
        subscriptions: None,
        functional_options: None,
        predefined_items: None,
    }
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
            let operations = parse_operations(args.get("operations"), false, kind)?;
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
            let operations = parse_operations(args.get("operations"), true, kind)?;
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
            if (force && !confirm) || (confirm && !force) {
                return Err(invalid(
                    "force",
                    "forced remove requires force=true and confirm=true; dryRun=false applies it",
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

fn parse_operations(
    value: Option<&Value>,
    required: bool,
    kind: MetadataKind,
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
            parse_edit_operation(raw_operation, kind)
                .map_err(|diagnostic| MetaFailure::from(diagnostic.with_operation_index(index)))
        })
        .collect()
}

fn reject_unknown_top_level(
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

fn metadata_top_level_fields(operation: MetadataOperation) -> &'static [&'static str] {
    match operation {
        MetadataOperation::Info => &["sourceSet", "metadataPath", "sections", "limit"],
        MetadataOperation::Add => &["sourceSet", "kind", "name", "operations", "dryRun"],
        MetadataOperation::Edit => &["sourceSet", "metadataPath", "operations", "dryRun"],
        MetadataOperation::Remove => &["sourceSet", "metadataPath", "dryRun", "force", "confirm"],
    }
}

fn parse_info_sections(value: Option<&Value>) -> Result<Vec<MetaInfoSection>, MetaDiagnostic> {
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

fn parse_edit_operation(
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
                    "relation",
                    "mode",
                    "targets",
                ],
            )?;
            let values = parse_property_changes(required_object(object, "values")?, kind)?;
            Ok(MetaEditOperation::SetProperties { values })
        }
        MetaEditOperationTag::Add => {
            reject_forbidden_fields(object, &["values", "names", "relation", "mode", "targets"])?;
            let collection = parse_collection(object, kind)?;
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
            let collection = parse_collection(object, kind)?;
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
            let collection = parse_collection(object, kind)?;
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

fn parse_collection(
    object: &Map<String, Value>,
    kind: MetadataKind,
) -> Result<MetaCollection, MetaDiagnostic> {
    let name = required_string_diagnostic(object, "collection")?;
    let collection = MetaCollection::parse(&name)?;
    validate_metadata_kind_collection(kind, collection)?;
    Ok(collection)
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
    let mut segments = address.segments();
    let kind = segments
        .next()
        .expect("validated metadata address has a root kind");
    let _name = segments
        .next()
        .expect("validated metadata address has an object name");
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

fn parse_bounded_usize(
    value: Option<&Value>,
    field: &str,
    default: usize,
    maximum: usize,
) -> Result<usize, MetaFailure> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = value
        .as_u64()
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
    let string =
        |description: &str| json!({"type": "string", "minLength": 1, "description": description});
    let mut properties = Map::new();
    properties.insert(
        "sourceSet".into(),
        string("Exact Configuration source-set name from v8project.yaml."),
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
                    "description": "Related metadata sections to include in the typed answer; omit or pass [] for local-only inspection.",
                }),
            );
            properties.insert(
                "limit".into(),
                json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 20,
                    "description": "Maximum related items returned for each requested section (1 through 50)."
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
                string("Metadata object name using a valid 1C identifier."),
            );
            properties.insert(
                "operations".into(),
                json!({
                    "type": "array",
                    "minItems": 1,
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
        MetadataOperation::Remove => {
            properties.insert(
                "metadataPath".into(),
                string("Logical metadata path of the object to remove."),
            );
            properties.insert(
                "dryRun".into(),
                json!({
                    "type": "boolean",
                    "default": true,
                    "description": "Preview the mutation without writing workspace files."
                }),
            );
            properties.insert(
                "force".into(),
                json!({
                    "type": "boolean",
                    "default": false,
                    "description": "Allow removal despite discovered references when confirmed."
                }),
            );
            properties.insert(
                "confirm".into(),
                json!({
                    "type": "boolean",
                    "default": false,
                    "description": "Explicitly confirm a forced metadata object removal."
                }),
            );
            vec!["sourceSet", "metadataPath"]
        }
    };
    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    });
    if matches!(operation, MetadataOperation::Add | MetadataOperation::Edit) {
        let schema = schema
            .as_object_mut()
            .expect("metadata input schema is always an object");
        schema.insert(
            "allOf".into(),
            Value::Array(owner_operation_branches(operation)),
        );
        schema.insert("$defs".into(), Value::Object(metadata_schema_definitions()));
    }
    schema
}

fn metadata_schema_definitions() -> Map<String, Value> {
    let mut definitions = Map::new();
    definitions.insert("scope".into(), scope_schema());
    definitions.insert("position".into(), position_schema());
    definitions.insert("metadataType".into(), metadata_type_schema());
    definitions.insert("fillValue".into(), fill_value_schema());
    for collection in MetaCollection::ALL.iter().copied() {
        definitions.insert(
            add_element_definition_name(collection),
            add_element_schema(collection),
        );
        definitions.insert(
            update_element_definition_name(collection),
            update_element_schema(collection),
        );
    }
    definitions.insert("editRelationsOperation".into(), edit_relations_schema());
    for kind in MetadataKind::ALL.iter().copied() {
        if metadata_kind_collections(kind).is_empty() {
            continue;
        }
        for tag in [
            MetaEditOperationTag::Add,
            MetaEditOperationTag::Update,
            MetaEditOperationTag::Remove,
        ] {
            definitions
                .entry(collection_operation_definition_name(kind, tag))
                .or_insert_with(|| collection_operation_schema(kind, tag));
        }
    }
    for kind in MetadataKind::ALL.iter().copied() {
        definitions.insert(operation_definition_name(kind), operation_schema(kind));
    }
    definitions
}

fn schema_reference(definition: impl AsRef<str>) -> Value {
    json!({"$ref": format!("#/$defs/{}", definition.as_ref())})
}

fn operation_definition_name(kind: MetadataKind) -> String {
    format!("operationsFor{}", kind.as_str())
}

fn add_element_definition_name(collection: MetaCollection) -> String {
    let profile_owner = MetaCollection::ALL
        .iter()
        .copied()
        .find(|candidate| same_add_element_profile(*candidate, collection))
        .expect("metadata add-element profile must have an owner");
    format!("addElementFor{}", profile_owner.as_str())
}

fn update_element_definition_name(collection: MetaCollection) -> String {
    let profile_owner = MetaCollection::ALL
        .iter()
        .copied()
        .find(|candidate| same_update_element_profile(*candidate, collection))
        .expect("metadata update-element profile must have an owner");
    format!("updateElementFor{}", profile_owner.as_str())
}

fn same_add_element_profile(left: MetaCollection, right: MetaCollection) -> bool {
    let left = metadata_collection_spec(left);
    let right = metadata_collection_spec(right);
    left.allows_type == right.allows_type
        && left.allows_required == right.allows_required
        && left.allows_fill_value == right.allows_fill_value
        && left.allows_nested_attributes == right.allows_nested_attributes
        && left.allows_position == right.allows_position
}

fn same_update_element_profile(left: MetaCollection, right: MetaCollection) -> bool {
    let left = metadata_collection_spec(left);
    let right = metadata_collection_spec(right);
    left.allows_type == right.allows_type
        && left.allows_required == right.allows_required
        && left.allows_fill_value == right.allows_fill_value
        && left.allows_position == right.allows_position
}

fn collection_operation_definition_name(kind: MetadataKind, tag: MetaEditOperationTag) -> String {
    let profile_owner = MetadataKind::ALL
        .iter()
        .copied()
        .find(|candidate| metadata_kind_collections(*candidate) == metadata_kind_collections(kind))
        .expect("metadata collection profile must have an owner");
    format!("{}OperationFor{}", tag.as_str(), profile_owner.as_str())
}

fn owner_operation_branches(operation: MetadataOperation) -> Vec<Value> {
    MetadataKind::ALL
        .iter()
        .copied()
        .map(|kind| {
            let (selector_name, selector) = match operation {
                MetadataOperation::Add => (
                    "kind",
                    json!({
                        "kind": {
                            "type": "string",
                            "enum": [kind.as_str()],
                        },
                    }),
                ),
                MetadataOperation::Edit => (
                    "metadataPath",
                    json!({
                        "metadataPath": {
                            "type": "string",
                            "pattern": metadata_kind_edit_path_pattern(kind),
                        },
                    }),
                ),
                MetadataOperation::Info | MetadataOperation::Remove => {
                    unreachable!("only mutation inputs have operation branches")
                }
            };
            json!({
                "if": {
                    "properties": selector,
                    "required": [selector_name],
                },
                "then": {
                    "properties": {
                        "operations": {
                    "type": "array",
                    "minItems": 1,
                    "items": schema_reference(operation_definition_name(kind)),
                        },
                    },
                },
            })
        })
        .collect()
}

fn metadata_edit_path_pattern() -> String {
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

fn metadata_kind_edit_path_pattern(kind: MetadataKind) -> String {
    format!(
        r"^({})\.[^.]+$",
        metadata_address_kind_spellings(kind.as_str())
            .expect("metadata kind must have a source-target spelling registry")
            .join("|")
    )
}

fn operation_schema(kind: MetadataKind) -> Value {
    let mut set_properties = Map::new();
    set_properties.insert("values".into(), property_values_schema(kind));

    let mut variants = vec![tagged_operation_variant(
        MetaEditOperationTag::SetProperties,
        set_properties,
        &["op", "values"],
    )];
    if !metadata_kind_collections(kind).is_empty() {
        variants.extend(
            [
                MetaEditOperationTag::Add,
                MetaEditOperationTag::Update,
                MetaEditOperationTag::Remove,
            ]
            .map(|tag| schema_reference(collection_operation_definition_name(kind, tag))),
        );
    }
    variants.push(schema_reference("editRelationsOperation"));

    json!({
        "oneOf": variants,
        "description": "Exactly one typed metadata edit operation.",
    })
}

fn tagged_operation_variant(
    tag: MetaEditOperationTag,
    mut properties: Map<String, Value>,
    required: &[&str],
) -> Value {
    properties.insert(
        "op".into(),
        json!({
            "type": "string",
            "enum": [tag.as_str()],
            "description": "Discriminator for this metadata edit operation.",
        }),
    );
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

fn collection_operation_schema(kind: MetadataKind, tag: MetaEditOperationTag) -> Value {
    let collections = metadata_kind_collections(kind);
    let mut properties = Map::new();
    properties.insert("collection".into(), collection_schema(collections));
    if collections.contains(&MetaCollection::Attributes) {
        properties.insert("scope".into(), schema_reference("scope"));
    }
    let required = match tag {
        MetaEditOperationTag::Add | MetaEditOperationTag::Update => {
            properties.insert(
                "elements".into(),
                json!({
                    "type": "array",
                    "minItems": 1,
                    "description": "Elements to add or update in the selected collection.",
                }),
            );
            &["op", "collection", "elements"][..]
        }
        MetaEditOperationTag::Remove => {
            properties.insert(
                "names".into(),
                json!({
                    "type": "array",
                    "minItems": 1,
                    "uniqueItems": true,
                    "description": "Names of existing elements to remove.",
                    "items": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Name of one existing metadata element.",
                    },
                }),
            );
            &["op", "collection", "names"][..]
        }
        MetaEditOperationTag::SetProperties | MetaEditOperationTag::EditRelations => {
            unreachable!("collection schema requires a collection operation tag")
        }
    };
    let mut schema = tagged_operation_variant(tag, properties, required);
    let collection_branches = collections
        .iter()
        .copied()
        .map(|collection| collection_operation_branch(tag, collection))
        .collect::<Vec<_>>();
    schema
        .as_object_mut()
        .expect("collection operation schema is always an object")
        .insert("oneOf".into(), Value::Array(collection_branches));
    schema
}

fn collection_operation_branch(tag: MetaEditOperationTag, collection: MetaCollection) -> Value {
    let mut properties = Map::new();
    properties.insert(
        "collection".into(),
        json!({
            "type": "string",
            "enum": [collection.as_str()],
        }),
    );
    if matches!(
        tag,
        MetaEditOperationTag::Add | MetaEditOperationTag::Update
    ) {
        let item_schema = if tag == MetaEditOperationTag::Add {
            schema_reference(add_element_definition_name(collection))
        } else {
            schema_reference(update_element_definition_name(collection))
        };
        properties.insert(
            "elements".into(),
            json!({
                "type": "array",
                "minItems": 1,
                "items": item_schema,
            }),
        );
    }
    let mut branch = json!({"properties": properties});
    if collection != MetaCollection::Attributes {
        branch
            .as_object_mut()
            .unwrap()
            .insert("not".into(), json!({"required": ["scope"]}));
    }
    branch
}

fn collection_schema(collections: &[MetaCollection]) -> Value {
    json!({
        "type": "string",
        "enum": collections.iter().copied().map(MetaCollection::as_str).collect::<Vec<_>>(),
        "description": "Metadata child collection to change.",
    })
}

fn edit_relations_schema() -> Value {
    let mut properties = Map::new();
    properties.insert(
        "relation".into(),
        json!({
            "type": "string",
            "enum": MetaRelation::ALL.iter().copied().map(MetaRelation::as_str).collect::<Vec<_>>(),
            "description": "Metadata relation to edit.",
        }),
    );
    properties.insert(
        "mode".into(),
        json!({
            "type": "string",
            "enum": RelationEditMode::ALL.iter().copied().map(RelationEditMode::as_str).collect::<Vec<_>>(),
            "description": "Whether relation targets are added, removed, or replaced.",
        }),
    );
    properties.insert(
        "targets".into(),
        json!({
            "type": "array",
            "minItems": 1,
            "description": "Target metadata objects or field paths for the relation.",
            "items": relation_target_schema(),
        }),
    );
    let mut schema = tagged_operation_variant(
        MetaEditOperationTag::EditRelations,
        properties,
        &["op", "relation", "mode", "targets"],
    );
    schema
        .as_object_mut()
        .expect("edit relation operation schema is always an object")
        .insert(
            "oneOf".into(),
            json!([
                {
                    "properties": {
                        "relation": {
                            "type": "string",
                            "enum": ["owners", "registerRecords", "basedOn"],
                        },
                        "targets": {
                            "type": "array",
                            "items": metadata_relation_target_schema(),
                        },
                    },
                },
                {
                    "properties": {
                        "relation": {
                            "type": "string",
                            "enum": ["inputByString"],
                        },
                        "targets": {
                            "type": "array",
                            "items": field_relation_target_schema(),
                        },
                    },
                },
            ]),
        );
    schema
}

fn property_values_schema(kind: MetadataKind) -> Value {
    let mut properties = Map::new();
    for spec in METADATA_PROPERTY_SPECS
        .iter()
        .filter(|spec| spec.allowed_kinds.contains(&kind))
    {
        let mut schema = match spec.value_kind {
            MetaPropertyValueKind::String => json!({
                "type": "string",
                "description": format!("New value for metadata property {}.", spec.public_name),
            }),
            MetaPropertyValueKind::Boolean => json!({
                "type": "boolean",
                "description": format!("New value for metadata property {}.", spec.public_name),
            }),
            MetaPropertyValueKind::UnsignedInteger => {
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "maximum": u32::MAX,
                    "description": format!("New value for metadata property {}.", spec.public_name),
                })
            }
        };
        if !spec.enum_values.is_empty() {
            schema
                .as_object_mut()
                .expect("property schema is always an object")
                .insert("enum".into(), json!(spec.enum_values));
        }
        properties.insert(spec.public_name.to_string(), schema);
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "minProperties": 1,
        "description": "One or more supported metadata property values to set.",
    })
}

fn scope_schema() -> Value {
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

fn relation_target_schema() -> Value {
    json!({
        "oneOf": [
            metadata_relation_target_schema(),
            field_relation_target_schema(),
        ],
        "description": "One relation target, expressed as an object or field reference.",
    })
}

fn metadata_relation_target_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "metadataPath": {
                "type": "string",
                "minLength": 1,
                "description": "Logical metadata path of the related object.",
            },
        },
        "required": ["metadataPath"],
    })
}

fn field_relation_target_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "fieldPath": {
                "type": "string",
                "minLength": 1,
                "description": "Logical field path used by an input-by-string relation.",
            },
        },
        "required": ["fieldPath"],
    })
}

fn position_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "before": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Name of the existing element that follows this element.",
                    },
                },
                "required": ["before"],
            },
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "after": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Name of the existing element that precedes this element.",
                    },
                },
                "required": ["after"],
            },
        ],
        "description": "Exact insertion or movement anchor.",
    })
}

fn metadata_type_schema() -> Value {
    let at_most_one = [
        "string",
        "number",
        "boolean",
        "date",
        "binaryData",
        "valueStorage",
    ]
    .into_iter()
    .map(|kind| {
        json!({
            "contains": {
                "type": "object",
                "properties": {"kind": {"enum": [kind]}},
                "required": ["kind"],
            },
            "minContains": 0,
            "maxContains": 1,
        })
    })
    .collect::<Vec<_>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "variants": {
                "type": "array",
                "minItems": 1,
                "uniqueItems": true,
                "allOf": at_most_one,
                "if": {
                    "contains": {
                        "type": "object",
                        "properties": {"kind": {"enum": ["valueStorage"]}},
                        "required": ["kind"],
                    },
                },
                "then": {"maxItems": 1},
                "description": "One or more platform type variants.",
                "items": type_variant_schema(),
            },
        },
        "required": ["variants"],
        "description": "Structured 1C metadata type.",
    })
}

fn type_variant_schema() -> Value {
    let mut string = tagged_type_variant(
        "string",
        json!({
            "length": {"type": "integer", "minimum": 0, "maximum": 1024, "description": "Maximum string length."},
            "allowedLength": {"type": "string", "enum": ["variable", "fixed"], "description": "Whether the string length is variable or fixed."},
        }),
        &["kind", "length", "allowedLength"],
    );
    require_positive_fixed_length(&mut string);
    let mut binary_data = tagged_type_variant(
        "binaryData",
        json!({
            "length": {"type": "integer", "minimum": 0, "maximum": u32::MAX, "description": "Maximum binary data length."},
            "allowedLength": {"type": "string", "enum": ["variable", "fixed"], "description": "Whether the binary data length is variable or fixed."},
        }),
        &["kind", "length", "allowedLength"],
    );
    require_positive_fixed_length(&mut binary_data);
    json!({
        "oneOf": [
            string,
            tagged_type_variant(
                "number",
                json!({
                    "digits": {"type": "integer", "minimum": 0, "maximum": 38, "description": "Total number of decimal digits."},
                    "fraction": {"type": "integer", "minimum": 0, "maximum": 38, "description": "Number of fractional decimal digits."},
                    "sign": {"type": "string", "enum": ["any", "nonNegative"], "description": "Whether negative values are allowed."},
                }),
                &["kind", "digits", "fraction", "sign"],
            ),
            tagged_type_variant("boolean", json!({}), &["kind"]),
            tagged_type_variant(
                "date",
                json!({
                    "fractions": {"type": "string", "enum": ["date", "time", "dateTime"], "description": "Date and time fractions stored by the value."},
                }),
                &["kind", "fractions"],
            ),
            binary_data,
            tagged_type_variant("valueStorage", json!({}), &["kind"]),
            tagged_type_variant(
                "reference",
                json!({
                    "metadataPath": {"type": "string", "minLength": 1, "description": "Logical metadata path of the referenced object."},
                }),
                &["kind", "metadataPath"],
            ),
            tagged_type_variant(
                "definedType",
                json!({
                    "metadataPath": {"type": "string", "minLength": 1, "description": "Logical metadata path of the defined type."},
                }),
                &["kind", "metadataPath"],
            ),
        ],
        "description": "One closed platform type variant.",
    })
}

fn require_positive_fixed_length(schema: &mut Value) {
    schema
        .as_object_mut()
        .expect("type variant schema is always an object")
        .insert(
            "allOf".into(),
            json!([{
                "if": {
                    "properties": {"allowedLength": {"enum": ["fixed"]}},
                    "required": ["allowedLength"],
                },
                "then": {"properties": {"length": {"minimum": 1}}},
            }]),
        );
}

fn tagged_type_variant(kind: &str, fields: Value, required: &[&str]) -> Value {
    let mut properties = fields
        .as_object()
        .expect("type variant fields are always an object")
        .clone();
    properties.insert(
        "kind".into(),
        json!({
            "type": "string",
            "enum": [kind],
            "description": "Discriminator for this platform type variant.",
        }),
    );
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

fn fill_value_schema() -> Value {
    json!({
        "oneOf": [
            tagged_fill_value_variant(
                "string",
                json!({"value": {"type": "string", "description": "String fill value."}}),
                &["kind", "value"],
            ),
            tagged_fill_value_variant(
                "number",
                json!({"value": {"type": "string", "description": "Platform-formatted numeric fill value."}}),
                &["kind", "value"],
            ),
            tagged_fill_value_variant(
                "boolean",
                json!({"value": {"type": "boolean", "description": "Boolean fill value."}}),
                &["kind", "value"],
            ),
            tagged_fill_value_variant(
                "dateTime",
                json!({"value": {"type": "string", "description": "Platform-formatted date-time fill value."}}),
                &["kind", "value"],
            ),
            tagged_fill_value_variant(
                "reference",
                json!({"metadataPath": {"type": "string", "minLength": 1, "description": "Logical metadata path of the reference fill value."}}),
                &["kind", "metadataPath"],
            ),
        ],
        "description": "One closed fill-value variant.",
    })
}

fn tagged_fill_value_variant(kind: &str, fields: Value, required: &[&str]) -> Value {
    let mut properties = fields
        .as_object()
        .expect("fill-value variant fields are always an object")
        .clone();
    properties.insert(
        "kind".into(),
        json!({
            "type": "string",
            "enum": [kind],
            "description": "Discriminator for this fill-value variant.",
        }),
    );
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

fn add_element_schema(collection: MetaCollection) -> Value {
    let spec = metadata_collection_spec(collection);
    let mut properties = Map::new();
    properties.insert(
        "name".into(),
        json!({"type": "string", "minLength": 1, "description": "Name of the new metadata element."}),
    );
    properties.insert(
        "synonym".into(),
        json!({"type": "string", "description": "Optional display synonym for the new element."}),
    );
    properties.insert(
        "comment".into(),
        json!({"type": "string", "description": "Optional comment for the new element."}),
    );
    if spec.allows_type {
        properties.insert("type".into(), schema_reference("metadataType"));
    }
    if spec.allows_required {
        properties.insert(
            "required".into(),
            json!({"type": "boolean", "description": "Whether the new element is required."}),
        );
    }
    if spec.allows_fill_value {
        properties.insert("fillValue".into(), schema_reference("fillValue"));
    }
    if spec.allows_position {
        properties.insert("position".into(), schema_reference("position"));
    }
    if spec.allows_nested_attributes {
        properties.insert(
            "attributes".into(),
            json!({
                "type": "array",
                "minItems": 1,
                "description": "Attributes nested under a new tabular section.",
                "items": schema_reference(add_element_definition_name(MetaCollection::Attributes)),
            }),
        );
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": ["name"],
        "description": "Definition of one metadata element to add.",
    })
}

fn update_element_schema(collection: MetaCollection) -> Value {
    let spec = metadata_collection_spec(collection);
    let mut properties = json!({
        "name": {"type": "string", "minLength": 1, "description": "Name of the existing metadata element to update."},
        "newName": {"type": "string", "minLength": 1, "description": "Optional replacement name for the existing element."},
        "synonym": {"type": "string", "description": "Optional replacement display synonym."},
        "comment": {"type": "string", "description": "Optional replacement comment."},
    })
    .as_object()
    .unwrap()
    .clone();
    if spec.allows_type {
        properties.insert("type".into(), schema_reference("metadataType"));
    }
    if spec.allows_required {
        properties.insert(
            "required".into(),
            json!({"type": "boolean", "description": "Optional replacement required flag."}),
        );
    }
    if spec.allows_fill_value {
        properties.insert("fillValue".into(), schema_reference("fillValue"));
    }
    if spec.allows_position {
        properties.insert("position".into(), schema_reference("position"));
    }
    let mutation_fields = properties
        .keys()
        .filter(|name| name.as_str() != "name")
        .map(|name| json!({"required": [name]}))
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": ["name"],
        "anyOf": mutation_fields,
        "description": "Patch for one existing metadata element.",
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
        MetaFreshness, MetaMutationData, MetaRelatedSection, MetaRelatedSections,
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

    fn resolve_definition<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
        match schema.get("$ref").and_then(Value::as_str) {
            Some(reference) => {
                let definition = reference
                    .strip_prefix("#/$defs/")
                    .expect("test schema reference must target the root definitions");
                &root["$defs"][definition]
            }
            None => schema,
        }
    }

    fn operation_schema_for_kind(root: &Value, kind: MetadataKind) -> &Value {
        &root["$defs"][operation_definition_name(kind)]
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

    fn unavailable_section() -> MetaRelatedSection<Value> {
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
            modules: Some(unavailable_section()),
            roles: Some(unavailable_section()),
            subscriptions: Some(unavailable_section()),
            functional_options: Some(unavailable_section()),
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

    fn empty_relations() -> crate::domain::metadata::MetaRelationsData {
        crate::domain::metadata::MetaRelationsData::default()
    }

    fn validation_subject() -> MetadataValidationSubject {
        MetadataValidationSubject {
            target: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Document.Order")
                .unwrap(),
            resources: vec![MetadataResourceImage {
                role: MetadataResourceRole::Descriptor,
                bytes: b"<MetaDataObject/>".to_vec(),
            }],
            child_footprints: Vec::new(),
            registrar_evidence: Default::default(),
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
            effects: Vec::new(),
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

    fn info_ports() -> (FakeMetadataPorts, Arc<Mutex<CoordinatorState>>) {
        let state = Arc::new(Mutex::new(CoordinatorState::default()));
        let subject = validation_subject();
        (
            FakeMetadataPorts {
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
                        relations: empty_relations(),
                        collections: empty_collections(),
                        diagnostics: Vec::new(),
                    },
                    validation_subject: subject,
                }))),
                related: unavailable_related(),
                validation: passed_validation(),
                prepared: Mutex::new(None),
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
    fn preview_effects_follow_operation_order() {
        let workspace = std::env::temp_dir().join(format!(
            "unica-meta-effect-order-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let initialized = crate::application::UnicaApplication::new()
            .call_tool(
                "unica.cf.init",
                &object(json!({
                    "cwd": workspace.display().to_string(),
                    "Name": "EffectOrder",
                    "OutputDir": "src",
                    "dryRun": false
                })),
            )
            .unwrap();
        assert!(initialized.ok, "{:?}", initialized.errors);
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workspace).unwrap();
        let application = crate::application::UnicaApplication::new();
        for name in ["Owner", "Subject"] {
            let added = application
                .call_tool(
                    "unica.meta.add",
                    &object(json!({
                        "sourceSet": "main",
                        "kind": "Catalog",
                        "name": name,
                        "dryRun": false
                    })),
                )
                .unwrap();
            assert!(added.ok, "{name}: {:?}", added.errors);
        }
        let operations = json!([
            {"op": "setProperties", "values": {"Comment": "ordered"}},
            {"op": "add", "collection": "attributes", "elements": [{"name": "Transient"}]},
            {"op": "update", "collection": "attributes", "elements": [{"name": "Transient", "newName": "Updated"}]},
            {"op": "remove", "collection": "attributes", "names": ["Updated"]},
            {"op": "editRelations", "relation": "owners", "mode": "replace", "targets": [{"metadataPath": "Catalog.Owner"}]}
        ]);
        let preview = application
            .call_tool(
                "unica.meta.edit",
                &object(json!({
                    "sourceSet": "main",
                    "metadataPath": "Catalog.Subject",
                    "operations": operations,
                    "dryRun": true
                })),
            )
            .unwrap();
        let template = application
            .call_tool(
                "unica.meta.add",
                &object(json!({
                    "sourceSet": "main",
                    "kind": "Catalog",
                    "name": "TemplateOnly",
                    "dryRun": true
                })),
            )
            .unwrap();
        std::env::set_current_dir(previous).unwrap();
        let _ = std::fs::remove_dir_all(&workspace);

        assert!(preview.ok, "{:?}", preview.errors);
        let effects = preview.data.unwrap()["effects"].clone();
        assert_eq!(
            effects
                .as_array()
                .unwrap()
                .iter()
                .map(|effect| effect["operation"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["setProperties", "add", "update", "remove", "editRelations"]
        );
        assert!(effects
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .all(|(index, effect)| effect["operationIndex"] == index as u64));
        assert_eq!(
            template.data.unwrap()["effects"][0]["operation"],
            "createTemplate"
        );
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
                    relations: empty_relations(),
                    collections: empty_collections(),
                    diagnostics: Vec::new(),
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
                "sections": ["modules", "roles", "subscriptions", "functionalOptions", "predefinedItems"],
                "limit": 50,
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
    fn coordinator_info_with_no_selected_sections_skips_related_provider() {
        for sections in [None, Some(json!([]))] {
            let (ports, state) = info_ports();
            let mut args = object(json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
            }));
            if let Some(sections) = sections {
                args.insert("sections".to_string(), sections);
            }

            let outcome = invoke(
                MetadataOperation::Info,
                &ports,
                &args,
                &coordinator_context(),
                &CancellationToken::new(),
            )
            .unwrap();

            assert!(outcome.adapter.ok);
            assert_eq!(outcome.data.as_ref().unwrap()["related"], json!({}));
            assert_eq!(state.lock().unwrap().calls, ["read", "validate"]);
        }
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
        assert!(info.sections.is_empty());
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
    fn info_limit_is_bounded() {
        let schema = metadata_input_schema(MetadataOperation::Info);
        assert_eq!(schema["properties"]["sections"]["default"], json!([]));
        assert_eq!(schema["properties"]["limit"]["maximum"], json!(50));

        let MetadataRequest::Info(info) = parse_metadata_request(
            MetadataOperation::Info,
            &object(json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "limit": 50,
            })),
        )
        .unwrap() else {
            panic!("expected info request")
        };
        assert_eq!(info.limit, 50);

        let error = diagnostic(
            MetadataOperation::Info,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "limit": 51,
            }),
        );
        assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(error.field.as_deref(), Some("limit"));
    }

    #[test]
    fn edit_schema() {
        let root = metadata_input_schema(MetadataOperation::Edit);
        let schema = operation_schema_for_kind(&root, MetadataKind::Document);
        let variants = schema["oneOf"]
            .as_array()
            .expect("metadata edit operations must publish a oneOf union");
        let expected = [
            ("setProperties", &["op", "values"][..]),
            ("add", &["op", "collection", "elements"][..]),
            ("update", &["op", "collection", "elements"][..]),
            ("remove", &["op", "collection", "names"][..]),
            ("editRelations", &["op", "relation", "mode", "targets"][..]),
        ];

        assert_eq!(variants.len(), expected.len());
        for (op, required) in expected {
            let variant = variants
                .iter()
                .map(|variant| resolve_definition(&root, variant))
                .find(|variant| variant["properties"]["op"]["enum"] == json!([op]))
                .unwrap_or_else(|| panic!("missing {op} operation schema variant"));
            assert_eq!(variant["type"], json!("object"));
            assert_eq!(variant["additionalProperties"], json!(false));
            assert_eq!(variant["required"], json!(required));
        }
    }

    #[test]
    fn property_schema_publishes_closed_enum_domains_for_retired_scalars() {
        let catalog = property_values_schema(MetadataKind::Catalog);
        let catalog_properties = catalog["properties"].as_object().unwrap();

        assert_eq!(
            catalog_properties["HierarchyType"]["enum"],
            json!(["HierarchyFoldersAndItems", "HierarchyOfItems"]),
        );
        let document = property_values_schema(MetadataKind::Document);
        let document_properties = document["properties"].as_object().unwrap();
        assert_eq!(
            document_properties["RegisterRecordsDeletion"]["enum"],
            json!(["AutoDelete", "AutoDeleteOnUnpost", "AutoDeleteOff"]),
        );
        let information_register = property_values_schema(MetadataKind::InformationRegister);
        let information_register_properties =
            information_register["properties"].as_object().unwrap();
        assert_eq!(
            information_register_properties["Periodicity"]["enum"],
            json!([
                "Nonperiodical",
                "Second",
                "Day",
                "Month",
                "Quarter",
                "Year",
                "RecorderPosition",
            ]),
        );
        let calculation_register = property_values_schema(MetadataKind::CalculationRegister);
        assert_eq!(
            calculation_register["properties"]["Periodicity"]["enum"],
            json!(["Day", "Month", "Quarter", "Year"]),
        );
    }

    #[test]
    fn property_schema_excludes_removed_register_type() {
        let schema = property_values_schema(MetadataKind::Document);
        let properties = schema["properties"].as_object().unwrap();

        assert!(!properties.contains_key("RegisterType"));
    }

    #[test]
    fn property_parser_rejects_an_enum_outside_the_exact_kind_domain() {
        let error = diagnostic(
            MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "CalculationRegister.Payroll",
                "operations": [{
                    "op": "setProperties",
                    "values": {"Periodicity": "Nonperiodical"},
                }],
            }),
        );

        assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(error.field.as_deref(), Some("values.Periodicity"));
        assert!(error.message.contains("expected one of"), "{error:?}");
    }

    #[test]
    fn schema_rejects_operations_the_closed_domain_rejects() {
        let cases = [
            json!({
                "sourceSet": "main",
                "metadataPath": "CalculationRegister.Payroll",
                "operations": [{
                    "op": "setProperties",
                    "values": {"Periodicity": "Nonperiodical"},
                }],
            }),
            json!({
                "sourceSet": "main",
                "metadataPath": "Enum.Statuses",
                "operations": [{
                    "op": "update",
                    "collection": "enumValues",
                    "elements": [{
                        "name": "Open",
                        "type": {"variants": [{"kind": "boolean"}]},
                    }],
                }],
            }),
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "remove",
                    "collection": "forms",
                    "scope": {"tabularSection": "Lines"},
                    "names": ["MainForm"],
                }],
            }),
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "update",
                    "collection": "attributes",
                    "elements": [{"name": "Customer"}],
                }],
            }),
            json!({
                "sourceSet": "main",
                "metadataPath": "Enum.Statuses",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{"name": "Code"}],
                }],
            }),
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Value",
                        "type": {"variants": [
                            {"kind": "string", "length": 10, "allowedLength": "variable"},
                            {"kind": "string", "length": 20, "allowedLength": "variable"},
                        ]},
                    }],
                }],
            }),
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Value",
                        "type": {"variants": [
                            {"kind": "valueStorage"},
                            {"kind": "boolean"},
                        ]},
                    }],
                }],
            }),
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Value",
                        "type": {"variants": [{
                            "kind": "string",
                            "length": 0,
                            "allowedLength": "fixed",
                        }]},
                    }],
                }],
            }),
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Value",
                        "type": {"variants": [{
                            "kind": "binaryData",
                            "length": 0,
                            "allowedLength": "fixed",
                        }]},
                    }],
                }],
            }),
        ];
        let schema = metadata_input_schema(MetadataOperation::Edit);
        let validator = jsonschema::validator_for(&schema).unwrap();

        for call in cases {
            assert!(
                parse_metadata_request(MetadataOperation::Edit, call.as_object().unwrap()).is_err(),
                "closed conversion unexpectedly accepted {call}"
            );
            assert!(
                !validator.is_valid(&call),
                "public schema accepted a call rejected by closed conversion: {call}"
            );
        }
    }

    #[test]
    fn binary_data_length_schema_and_parser_follow_the_closed_domain_bounds() {
        let schema = metadata_input_schema(MetadataOperation::Edit);
        let validator = jsonschema::validator_for(&schema).unwrap();
        let call = |length| {
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Payload",
                        "type": {"variants": [{
                            "kind": "binaryData",
                            "length": length,
                            "allowedLength": "fixed",
                        }]},
                    }],
                }],
            })
        };

        for length in [2_000_u64, u64::from(u32::MAX)] {
            let valid = call(length);
            assert!(
                validator.is_valid(&valid),
                "schema rejected runtime-valid binaryData length: {valid}"
            );
            parse_metadata_request(MetadataOperation::Edit, valid.as_object().unwrap())
                .unwrap_or_else(|error| panic!("parser rejected {valid}: {error:?}"));
        }

        let too_large = call(u64::from(u32::MAX) + 1);
        assert!(
            !validator.is_valid(&too_large),
            "schema accepted binaryData length above u32: {too_large}"
        );
        assert!(
            parse_metadata_request(MetadataOperation::Edit, too_large.as_object().unwrap())
                .is_err(),
            "parser accepted binaryData length above u32: {too_large}"
        );

        let invalid = call(0);
        assert!(
            !validator.is_valid(&invalid),
            "schema accepted zero-length fixed binaryData: {invalid}"
        );
        assert!(
            parse_metadata_request(MetadataOperation::Edit, invalid.as_object().unwrap()).is_err(),
            "parser accepted zero-length fixed binaryData: {invalid}"
        );
    }

    #[test]
    fn parser_rejects_a_collection_outside_the_owner_kind_registry() {
        let call = json!({
            "sourceSet": "main",
            "metadataPath": "Enum.Statuses",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{"name": "Code"}],
            }],
        });

        let error = diagnostic(MetadataOperation::Edit, call);
        assert_eq!(error.code, MetaDiagnosticCode::UnsupportedKind);
        assert_eq!(error.field.as_deref(), Some("collection"));
    }

    #[test]
    fn edit_schema_and_parser_share_the_top_level_address_alias_contract() {
        let schema = metadata_input_schema(MetadataOperation::Edit);
        let validator = jsonschema::validator_for(&schema).unwrap();
        let call = |metadata_path: &str| {
            json!({
                "sourceSet": "main",
                "metadataPath": metadata_path,
                "operations": [{
                    "op": "setProperties",
                    "values": {"Comment": "typed"},
                }],
            })
        };

        for metadata_path in ["Catalog.Items", "Справочник.Товары"] {
            let call = call(metadata_path);
            assert!(validator.is_valid(&call), "schema rejected {call}");
            parse_metadata_request(MetadataOperation::Edit, call.as_object().unwrap())
                .unwrap_or_else(|error| panic!("parser rejected {call}: {error:?}"));
        }

        for metadata_path in [
            "Catalogs.Items",
            "Справочники.Товары",
            "Catalog.Items.Form.List",
        ] {
            let call = call(metadata_path);
            assert!(!validator.is_valid(&call), "schema accepted {call}");
            assert!(
                parse_metadata_request(MetadataOperation::Edit, call.as_object().unwrap()).is_err(),
                "parser accepted {call}"
            );
        }
    }

    #[test]
    fn schema_and_parser_accept_every_registered_kind_collection_and_operation_tag() {
        fn add_element(collection: MetaCollection) -> Value {
            let mut element = json!({
                "name": "Element",
                "synonym": "Element synonym",
                "comment": "Element comment",
                "position": {"after": "Previous"},
            })
            .as_object()
            .unwrap()
            .clone();
            match collection {
                MetaCollection::Attributes
                | MetaCollection::Dimensions
                | MetaCollection::Resources => {
                    element.insert("type".into(), json!({"variants": [{"kind": "boolean"}]}));
                    element.insert("required".into(), json!(true));
                    element.insert(
                        "fillValue".into(),
                        json!({"kind": "boolean", "value": false}),
                    );
                }
                MetaCollection::TabularSections => {
                    element.insert(
                        "attributes".into(),
                        json!([{
                            "name": "Nested",
                            "type": {"variants": [{"kind": "boolean"}]},
                            "required": true,
                            "fillValue": {"kind": "boolean", "value": false},
                        }]),
                    );
                }
                MetaCollection::Columns => {
                    element.insert("type".into(), json!({"variants": [{"kind": "boolean"}]}));
                }
                MetaCollection::EnumValues
                | MetaCollection::Forms
                | MetaCollection::Templates
                | MetaCollection::Commands => {}
            }
            Value::Object(element)
        }

        fn update_element(collection: MetaCollection) -> Value {
            let mut element = json!({
                "name": "Element",
                "newName": "Renamed",
                "synonym": "Replacement synonym",
                "comment": "Replacement comment",
                "position": {"before": "Next"},
            })
            .as_object()
            .unwrap()
            .clone();
            match collection {
                MetaCollection::Attributes
                | MetaCollection::Dimensions
                | MetaCollection::Resources => {
                    element.insert("type".into(), json!({"variants": [{"kind": "boolean"}]}));
                    element.insert("required".into(), json!(false));
                    element.insert(
                        "fillValue".into(),
                        json!({"kind": "boolean", "value": true}),
                    );
                }
                MetaCollection::Columns => {
                    element.insert("type".into(), json!({"variants": [{"kind": "boolean"}]}));
                }
                MetaCollection::TabularSections
                | MetaCollection::EnumValues
                | MetaCollection::Forms
                | MetaCollection::Templates
                | MetaCollection::Commands => {}
            }
            Value::Object(element)
        }

        let add_schema = metadata_input_schema(MetadataOperation::Add);
        let add_validator = jsonschema::validator_for(&add_schema).unwrap();
        let edit_schema = metadata_input_schema(MetadataOperation::Edit);
        let edit_validator = jsonschema::validator_for(&edit_schema).unwrap();
        for kind in MetadataKind::ALL {
            let common_operations = [
                json!({"op": "setProperties", "values": {"Comment": "typed"}}),
                json!({
                    "op": "editRelations",
                    "relation": "basedOn",
                    "mode": "add",
                    "targets": [{"metadataPath": "Catalog.Items"}],
                }),
            ];
            let collection_operations = crate::domain::metadata::metadata_kind_collections(*kind)
                .iter()
                .flat_map(|collection| {
                    let collection_name = collection.as_str();
                    [
                        json!({
                            "op": "add",
                            "collection": collection_name,
                            "elements": [add_element(*collection)],
                        }),
                        json!({
                            "op": "update",
                            "collection": collection_name,
                            "elements": [update_element(*collection)],
                        }),
                        json!({
                            "op": "remove",
                            "collection": collection_name,
                            "names": ["Element"],
                        }),
                    ]
                });

            for operation in common_operations.into_iter().chain(collection_operations) {
                let add_call = json!({
                    "sourceSet": "main",
                    "kind": kind.as_str(),
                    "name": "Object",
                    "operations": [operation.clone()],
                });
                assert!(
                    add_validator.is_valid(&add_call),
                    "published add schema rejected {add_call}"
                );
                parse_metadata_request(MetadataOperation::Add, add_call.as_object().unwrap())
                    .unwrap_or_else(|error| panic!("conversion rejected {add_call}: {error:?}"));

                let edit_call = json!({
                    "sourceSet": "main",
                    "metadataPath": format!("{}.Object", kind.as_str()),
                    "operations": [operation],
                });
                assert!(
                    edit_validator.is_valid(&edit_call),
                    "published edit schema rejected {edit_call}"
                );
                parse_metadata_request(MetadataOperation::Edit, edit_call.as_object().unwrap())
                    .unwrap_or_else(|error| panic!("conversion rejected {edit_call}: {error:?}"));
            }

            if crate::domain::metadata::metadata_kind_collections(*kind)
                .contains(&MetaCollection::Attributes)
            {
                for operation in [
                    json!({
                        "op": "add",
                        "collection": "attributes",
                        "scope": {"tabularSection": "Lines"},
                        "elements": [{"name": "Nested"}],
                    }),
                    json!({
                        "op": "update",
                        "collection": "attributes",
                        "scope": {"tabularSection": "Lines"},
                        "elements": [{"name": "Nested", "comment": "changed"}],
                    }),
                    json!({
                        "op": "remove",
                        "collection": "attributes",
                        "scope": {"tabularSection": "Lines"},
                        "names": ["Nested"],
                    }),
                ] {
                    let call = json!({
                        "sourceSet": "main",
                        "metadataPath": format!("{}.Object", kind.as_str()),
                        "operations": [operation],
                    });
                    assert!(
                        edit_validator.is_valid(&call),
                        "published edit schema rejected scoped operation {call}"
                    );
                    parse_metadata_request(MetadataOperation::Edit, call.as_object().unwrap())
                        .unwrap_or_else(|error| panic!("conversion rejected {call}: {error:?}"));
                }
            }
        }
    }

    #[test]
    fn edit_schema_rejects_relation_target_cross_products() {
        let schema = metadata_input_schema(MetadataOperation::Edit);
        let validator = jsonschema::validator_for(&schema).unwrap();
        let invalid_owners_target = json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "editRelations",
                "relation": "owners",
                "mode": "add",
                "targets": [{"fieldPath": "Catalog.Items.StandardAttribute.Code"}],
            }],
        });
        let invalid_input_by_string_target = json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "editRelations",
                "relation": "inputByString",
                "mode": "add",
                "targets": [{"metadataPath": "Catalog.Items"}],
            }],
        });

        assert!(
            !validator.is_valid(&invalid_owners_target),
            "owners must reject fieldPath targets"
        );
        assert!(
            !validator.is_valid(&invalid_input_by_string_target),
            "inputByString must reject metadataPath targets"
        );
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
            let metadata_path = match collection {
                MetaCollection::Dimensions | MetaCollection::Resources => {
                    "InformationRegister.Entries"
                }
                MetaCollection::EnumValues => "Enum.Statuses",
                MetaCollection::Columns => "DocumentJournal.Documents",
                MetaCollection::Attributes
                | MetaCollection::TabularSections
                | MetaCollection::Forms
                | MetaCollection::Templates
                | MetaCollection::Commands => "Document.Order",
            };
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
                    &object(json!({
                        "sourceSet": "main",
                        "metadataPath": metadata_path,
                        "operations": [Value::Object(operation)],
                    })),
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

        let MetadataRequest::Remove(forced_preview) = parse_metadata_request(
            MetadataOperation::Remove,
            &object(json!({
                "sourceSet": "main",
                "metadataPath": "Catalog.Items",
                "force": true,
                "confirm": true
            })),
        )
        .unwrap() else {
            panic!("expected forced remove preview request")
        };
        assert!(forced_preview.dry_run && forced_preview.force && forced_preview.confirm);

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

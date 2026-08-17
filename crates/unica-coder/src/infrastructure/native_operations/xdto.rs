use crate::application::ports::{FormatGuardError, XdtoPublicErrorCode};
use crate::application::source_navigation::{
    authenticate_cursor, page_bounds_from_offset, SOURCE_NAVIGATION_LIMIT_DEFAULT,
    SOURCE_NAVIGATION_LIMIT_MAX,
};
use crate::application::{AdapterOutcome, SupportGuardRequirement};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, SourceTargetError, SourceTargetErrorCode,
    SourceTargetErrorReason, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::guard_resolved_platform_xml_target_dependencies;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    PlatformXmlResourceEvidence, TargetKindPolicy,
};
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::support_guard::{
    evaluate_resolved_support_guard, ResolvedSupportGuardCheck,
};
use roxmltree::Document;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

mod model;
mod validation;
mod writer;

use model::{PackageModel, XDTO_NS};
use validation::{validate, ValidationDiff};

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

pub(crate) struct XdtoExecution<T> {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<T>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum XdtoLocation {
    Addressed {
        source_set: String,
        metadata_path: String,
        target_kind: TargetKind,
    },
    Unaddressable {
        source_set: String,
        owner_metadata_path: String,
        node_key: String,
        body_byte_range: XdtoByteRange,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoByteRange {
    /// UTF-8 byte offset in the decoded XML body. A leading BOM is excluded.
    start: usize,
    /// Exclusive UTF-8 byte offset in the decoded XML body.
    end: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoTypeCounts {
    total: usize,
    value_types: usize,
    object_types: usize,
    global_properties: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoImportInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    namespace: Option<String>,
    location: XdtoLocation,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) enum XdtoTypeKind {
    #[serde(rename = "valueType")]
    Value,
    #[serde(rename = "objectType")]
    Object,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoBoundsInfo {
    lower: String,
    upper: String,
    lower_explicit: bool,
    upper_explicit: bool,
    unbounded: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoAnonymousTypeInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    discriminator: Option<String>,
    properties: Vec<XdtoPropertyInfo>,
    location: XdtoLocation,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoPropertyInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference: Option<XdtoQNameInfo>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    type_name: Option<XdtoQNameInfo>,
    bounds: XdtoBoundsInfo,
    type_defs: Vec<XdtoAnonymousTypeInfo>,
    location: XdtoLocation,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoQNameInfo {
    raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    prefix: Option<String>,
    local: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    namespace: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoTypeInfo {
    kind: XdtoTypeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<XdtoQNameInfo>,
    properties: Vec<XdtoPropertyInfo>,
    location: XdtoLocation,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoTypeSummary {
    kind: XdtoTypeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<XdtoQNameInfo>,
    location: XdtoLocation,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoInfoData {
    source_set: String,
    metadata_path: String,
    location: XdtoLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_namespace: Option<String>,
    imports: Vec<XdtoImportInfo>,
    counts: XdtoTypeCounts,
    global_properties: Vec<XdtoPropertyInfo>,
    types: Vec<XdtoTypeSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    type_detail: Option<XdtoTypeInfo>,
    findings: Vec<XdtoFinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum XdtoEditOperation {
    AddValueType,
    AddObjectType,
    AddProperty,
    RemoveType,
    RemoveProperty,
}

impl XdtoEditOperation {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "addValueType" => Ok(Self::AddValueType),
            "addObjectType" => Ok(Self::AddObjectType),
            "addProperty" => Ok(Self::AddProperty),
            "removeType" => Ok(Self::RemoveType),
            "removeProperty" => Ok(Self::RemoveProperty),
            _ => Err("unsupported_node: supported operations are addValueType, addObjectType, addProperty, removeType, removeProperty".to_string()),
        }
    }

    /// The writer keeps its historical kebab-case dispatch keys: ADR-0071
    /// changed the published payload shape, not the write semantics.
    fn writer_key(self) -> &'static str {
        match self {
            Self::AddValueType => "add-value-type",
            Self::AddObjectType => "add-object-type",
            Self::AddProperty => "add-property",
            Self::RemoveType => "remove-type",
            Self::RemoveProperty => "remove-property",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum XdtoFindingSeverity {
    Info,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum XdtoFindingState {
    PreExisting,
    Introduced,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum XdtoFindingPhase {
    Before,
    After,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoFinding {
    code: String,
    severity: XdtoFindingSeverity,
    state: XdtoFindingState,
    phase: XdtoFindingPhase,
    message: String,
    location: XdtoLocation,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum XdtoChangeKind {
    PackageTextEdit,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoProjectedChange {
    kind: XdtoChangeKind,
    location: XdtoLocation,
    body_byte_range: XdtoByteRange,
    removed_byte_count: usize,
    replacement_byte_count: usize,
}

/// The effect of one element of the `operations` array (ADR-0071). Byte
/// ranges in `change` are relative to the document state the operation was
/// applied to — operations are sequential, so element `i` sees the text
/// produced by element `i - 1`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoOperationEffect {
    operation_index: usize,
    op: XdtoEditOperation,
    no_op: bool,
    change: Option<XdtoProjectedChange>,
    findings: Vec<XdtoFinding>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XdtoEditData {
    source_set: String,
    metadata_path: String,
    location: XdtoLocation,
    no_op: bool,
    effects: Vec<XdtoOperationEffect>,
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    (operation == "xdto-info").then(|| info(args, context).map(|execution| execution.outcome))
}

pub(crate) fn info_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<XdtoExecution<XdtoInfoData>, String> {
    info(args, context)
}

pub(crate) fn resolve_xdto_guard_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, FormatGuardError> {
    resolve_package(args, context)
        .map(|package| package.path)
        .map_err(XdtoResolutionError::into_format_guard)
}

pub(crate) fn preview_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> XdtoExecution<XdtoEditData> {
    edit(args, context, true)
}
pub(crate) fn apply_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> XdtoExecution<XdtoEditData> {
    edit(args, context, false)
}

fn info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<XdtoExecution<XdtoInfoData>, String> {
    let package = resolve_package(args, context).map_err(|error| error.to_string())?;
    let raw =
        fs::read(&package.path).map_err(|error| format!("package_resource_missing: {error}"))?;
    let requested = optional_string(args, "typeName")?;
    if requested.is_some() && (args.contains_key("limit") || args.contains_key("cursor")) {
        return Err("xdto info `typeName` detail does not accept `limit` or `cursor`".to_string());
    }
    if requested
        .as_deref()
        .is_some_and(|name| !validation::is_ncname(name))
    {
        return Err("xdto info `typeName` must be an XML NCName".to_string());
    }
    let pagination = if requested.is_none() {
        let limit = xdto_info_limit(args)?;
        let cursor = optional_cursor(args)?;
        let cursor_key = xdto_info_cursor_key(&package, &raw, limit);
        let offset =
            authenticate_cursor(cursor.as_deref(), &cursor_key).map_err(xdto_cursor_error)?;
        Some((limit, cursor_key, offset))
    } else {
        None
    };
    let text = decode(&raw)?;
    let model = PackageModel::parse(&text).map_err(model_error)?;
    let descriptor_namespace = descriptor_namespace(&package.descriptor_path)?;
    if model.target_namespace() != Some(descriptor_namespace.as_str()) {
        return Err(
            "namespace_mismatch: descriptor Namespace must equal package targetNamespace"
                .to_string(),
        );
    }
    let value_types = model
        .types
        .iter()
        .filter(|named| named.kind == model::TypeKind::Value)
        .count();
    let counts = XdtoTypeCounts {
        total: model.types.len(),
        value_types,
        object_types: model.types.len() - value_types,
        global_properties: model.global_properties.len(),
    };
    let (selected, type_detail, next_cursor) = if let Some(name) = requested.as_deref() {
        let matches = model
            .types
            .iter()
            .filter(|named| named.name.as_ref().is_some_and(|value| value.value == name))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(if matches.is_empty() {
                format!("target_not_found: XDTO type `{name}` was not found")
            } else {
                format!("duplicate_type: XDTO type `{name}` is ambiguous")
            });
        }
        (Vec::new(), Some(type_info(&package, matches[0])), None)
    } else {
        let (limit, cursor_key, offset) =
            pagination.expect("list-mode pagination was authenticated before parsing");
        let (start, end, next_cursor) =
            page_bounds_from_offset(offset, &cursor_key, limit, model.types.len())
                .map_err(xdto_cursor_error)?;
        (
            model.types[start..end].iter().collect::<Vec<_>>(),
            None,
            next_cursor,
        )
    };
    let location = addressed_location(&package);
    let data = XdtoInfoData {
        source_set: package.source_set.clone(),
        metadata_path: package.metadata_path.clone(),
        location,
        target_namespace: model
            .target_namespace
            .as_ref()
            .map(|value| value.value.clone()),
        imports: model
            .imports
            .iter()
            .map(|import| XdtoImportInfo {
                namespace: import
                    .namespace
                    .as_ref()
                    .map(|namespace| namespace.value.clone()),
                location: internal_location(&package, &import.key, &import.span),
            })
            .collect(),
        counts,
        global_properties: model
            .global_properties
            .iter()
            .map(|property| property_info(&package, property))
            .collect(),
        types: selected
            .into_iter()
            .map(|named| type_summary(&package, named))
            .collect(),
        type_detail,
        findings: validate(&model)
            .into_iter()
            .map(|finding| baseline_finding(&package, finding))
            .collect(),
        next_cursor,
    };
    Ok(XdtoExecution {
        outcome: AdapterOutcome {
            ok: true,
            summary: "unica.xdto.info inspected XDTO package".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![format!(
                "{} + {}",
                package.source_set, package.metadata_path
            )],
            stdout: None,
            stderr: None,
            command: None,
        },
        data: Some(data),
    })
}

fn addressed_location(package: &Package) -> XdtoLocation {
    XdtoLocation::Addressed {
        source_set: package.source_set.clone(),
        metadata_path: package.metadata_path.clone(),
        target_kind: TargetKind::MetadataObject,
    }
}

fn internal_location(package: &Package, node_key: &str, span: &model::SourceSpan) -> XdtoLocation {
    XdtoLocation::Unaddressable {
        source_set: package.source_set.clone(),
        owner_metadata_path: package.metadata_path.clone(),
        node_key: node_key.to_string(),
        body_byte_range: XdtoByteRange {
            start: span.start,
            end: span.end,
        },
    }
}

fn qname_info(qname: &model::QNameRef) -> XdtoQNameInfo {
    XdtoQNameInfo {
        raw: qname.raw.clone(),
        prefix: qname.prefix.clone(),
        local: qname.local.clone(),
        namespace: qname.namespace.clone(),
    }
}

fn type_info(package: &Package, named: &model::NamedType) -> XdtoTypeInfo {
    XdtoTypeInfo {
        kind: match named.kind {
            model::TypeKind::Value => XdtoTypeKind::Value,
            model::TypeKind::Object => XdtoTypeKind::Object,
        },
        name: named.name.as_ref().map(|name| name.value.clone()),
        base: named.base.as_ref().map(qname_info),
        properties: named
            .properties
            .iter()
            .map(|property| property_info(package, property))
            .collect(),
        location: internal_location(package, &named.key, &named.span),
    }
}

fn type_summary(package: &Package, named: &model::NamedType) -> XdtoTypeSummary {
    XdtoTypeSummary {
        kind: match named.kind {
            model::TypeKind::Value => XdtoTypeKind::Value,
            model::TypeKind::Object => XdtoTypeKind::Object,
        },
        name: named.name.as_ref().map(|name| name.value.clone()),
        base: named.base.as_ref().map(qname_info),
        location: internal_location(package, &named.key, &named.span),
    }
}

fn property_info(package: &Package, property: &model::Property) -> XdtoPropertyInfo {
    let lower = property
        .lower_bound
        .as_ref()
        .map_or("1", |value| value.value.as_str());
    let upper = property
        .upper_bound
        .as_ref()
        .map_or("1", |value| value.value.as_str());
    XdtoPropertyInfo {
        name: property.name.as_ref().map(|name| name.value.clone()),
        reference: property.reference.as_ref().map(qname_info),
        type_name: property.type_ref.as_ref().map(qname_info),
        bounds: XdtoBoundsInfo {
            lower: lower.to_string(),
            upper: upper.to_string(),
            lower_explicit: property.lower_bound.is_some(),
            upper_explicit: property.upper_bound.is_some(),
            unbounded: upper == "-1",
        },
        type_defs: property
            .type_defs
            .iter()
            .map(|anonymous| anonymous_type_info(package, anonymous))
            .collect(),
        location: internal_location(package, &property.key, &property.span),
    }
}

fn anonymous_type_info(
    package: &Package,
    anonymous: &model::AnonymousObject,
) -> XdtoAnonymousTypeInfo {
    XdtoAnonymousTypeInfo {
        discriminator: anonymous
            .discriminator
            .as_ref()
            .map(|value| value.value.clone()),
        properties: anonymous
            .properties
            .iter()
            .map(|property| property_info(package, property))
            .collect(),
        location: internal_location(package, &anonymous.key, &anonymous.span),
    }
}

fn baseline_finding(package: &Package, finding: validation::Finding) -> XdtoFinding {
    XdtoFinding {
        code: finding.code,
        severity: XdtoFindingSeverity::Error,
        state: XdtoFindingState::PreExisting,
        phase: XdtoFindingPhase::Before,
        message: finding.message,
        location: XdtoLocation::Unaddressable {
            source_set: package.source_set.clone(),
            owner_metadata_path: package.metadata_path.clone(),
            node_key: finding.location.key,
            body_byte_range: XdtoByteRange {
                start: finding.location.span.start,
                end: finding.location.span.end,
            },
        },
    }
}

fn classified_finding(package: &Package, finding: &validation::ClassifiedFinding) -> XdtoFinding {
    XdtoFinding {
        code: finding.code.clone(),
        severity: XdtoFindingSeverity::Error,
        state: match finding.state {
            validation::FindingState::PreExisting => XdtoFindingState::PreExisting,
            validation::FindingState::Introduced => XdtoFindingState::Introduced,
        },
        phase: match finding.state {
            validation::FindingState::PreExisting => XdtoFindingPhase::Before,
            validation::FindingState::Introduced => XdtoFindingPhase::After,
        },
        message: finding.message.clone(),
        location: XdtoLocation::Unaddressable {
            source_set: package.source_set.clone(),
            owner_metadata_path: package.metadata_path.clone(),
            node_key: finding.location.key.clone(),
            body_byte_range: XdtoByteRange {
                start: finding.location.span.start,
                end: finding.location.span.end,
            },
        },
    }
}

fn writer_finding(package: &Package, finding: &writer::WriterFinding) -> XdtoFinding {
    XdtoFinding {
        code: finding.code.to_string(),
        severity: match finding.severity {
            writer::WriterFindingSeverity::Info => XdtoFindingSeverity::Info,
            writer::WriterFindingSeverity::Error => XdtoFindingSeverity::Error,
        },
        state: match finding.state {
            writer::WriterFindingState::PreExisting => XdtoFindingState::PreExisting,
            writer::WriterFindingState::Introduced => XdtoFindingState::Introduced,
        },
        phase: match finding.state {
            writer::WriterFindingState::PreExisting => XdtoFindingPhase::Before,
            writer::WriterFindingState::Introduced => XdtoFindingPhase::After,
        },
        message: finding.message.clone(),
        location: XdtoLocation::Unaddressable {
            source_set: package.source_set.clone(),
            owner_metadata_path: package.metadata_path.clone(),
            node_key: finding.location.key.clone(),
            body_byte_range: XdtoByteRange {
                start: finding.location.span.start,
                end: finding.location.span.end,
            },
        },
    }
}

fn xdto_info_limit(args: &Map<String, Value>) -> Result<usize, String> {
    let limit = args
        .get("limit")
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| "xdto info `limit` must be an integer".to_string())
        })
        .transpose()?
        .unwrap_or(SOURCE_NAVIGATION_LIMIT_DEFAULT);
    if !(1..=SOURCE_NAVIGATION_LIMIT_MAX).contains(&limit) {
        return Err(format!(
            "xdto info `limit` must be between 1 and {SOURCE_NAVIGATION_LIMIT_MAX}"
        ));
    }
    Ok(limit)
}

fn optional_string(args: &Map<String, Value>, name: &str) -> Result<Option<String>, String> {
    args.get(name)
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty() && *value == value.trim())
                .map(str::to_string)
                .ok_or_else(|| format!("xdto info `{name}` must be a non-empty string"))
        })
        .transpose()
}

fn optional_cursor(args: &Map<String, Value>) -> Result<Option<String>, String> {
    args.get("cursor")
        .map(|value| {
            value
                .as_str()
                .filter(|value| {
                    !value.is_empty() && value.chars().all(|character| !character.is_whitespace())
                })
                .map(str::to_string)
                .ok_or_else(|| {
                    "cursor_invalid: cursor must be a non-empty token without whitespace"
                        .to_string()
                })
        })
        .transpose()
}

fn xdto_info_cursor_key(package: &Package, raw: &[u8], limit: usize) -> String {
    let digest = Sha256::digest(raw)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "xdto-info-v1:{}:{}:list:limit={limit}:sha256={digest}",
        package.source_set, package.metadata_path
    )
}

fn xdto_cursor_error(error: String) -> String {
    format!(
        "cursor_invalid: {}",
        error.replacen("source navigation", "xdto info", 1)
    )
}

/// One parsed element of the `operations` array: the operation kind plus the
/// element's own fields as that operation's writer argument map.
type ParsedXdtoOperation = (XdtoEditOperation, Map<String, Value>);

/// Split the typed `operations` array into per-element writer inputs. The
/// element's own fields are exactly the argument names the writer has always
/// read, so each element minus its `op` tag is that operation's argument map.
fn parse_operations(args: &Map<String, Value>) -> Result<Vec<ParsedXdtoOperation>, String> {
    let operations = args
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| "operations must be a non-empty array".to_string())?;
    if operations.is_empty() {
        return Err("operations must be a non-empty array".to_string());
    }
    operations
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let item = item
                .as_object()
                .ok_or_else(|| format!("operations[{index}]: must be an object"))?;
            let op = item
                .get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("operations[{index}]: requires `op`"))?;
            let operation = XdtoEditOperation::parse(op)
                .map_err(|error| format!("operations[{index}]: {error}"))?;
            let mut operation_args = item.clone();
            operation_args.remove("op");
            Ok((operation, operation_args))
        })
        .collect()
}

fn edit(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    preview: bool,
) -> XdtoExecution<XdtoEditData> {
    let planned = (|| -> Result<MutationPlan, PlanningFailure> {
        let package = resolve_package(args, context).map_err(|error| error.to_string())?;
        let before = fs::read(&package.path)
            .map_err(|error| format!("package_resource_missing: {error}"))?;
        let text = decode(&before)?;
        let before_model = PackageModel::parse(&text).map_err(model_error)?;
        let descriptor_namespace = descriptor_namespace(&package.descriptor_path)?;
        if before_model.target_namespace() != Some(descriptor_namespace.as_str()) {
            return Err(PlanningFailure::plain(
                "namespace_mismatch: descriptor Namespace must equal package targetNamespace",
            ));
        }
        let operations = parse_operations(args)?;
        // ADR-0071: operations apply in order against the accumulated text, so
        // element `i` sees what element `i - 1` produced; the file is written
        // once after the whole batch plans cleanly.
        let mut effects: Vec<XdtoOperationEffect> = Vec::new();
        let mut current_text = text;
        let mut step_baseline = validate(&before_model);
        let partial_data =
            |effects: &[XdtoOperationEffect], package: &Package, no_op: bool| XdtoEditData {
                source_set: package.source_set.clone(),
                metadata_path: package.metadata_path.clone(),
                location: addressed_location(package),
                no_op,
                effects: effects.to_vec(),
            };
        for (index, (operation, operation_args)) in operations.iter().enumerate() {
            let writer_plan = writer::plan(&current_text, operation_args, operation.writer_key())
                .map_err(|error| PlanningFailure {
                error: format!("operations[{index}]: {error}"),
                data: Some(Box::new(partial_data(&effects, &package, false))),
            })?;
            let after_model =
                PackageModel::parse(&writer_plan.after).map_err(|error| PlanningFailure {
                    error: format!("operations[{index}]: {}", model_error(error)),
                    data: Some(Box::new(partial_data(&effects, &package, false))),
                })?;
            let validation = ValidationDiff::between(&step_baseline, validate(&after_model));
            let blocked = writer_plan.blocks() || validation.blocks();
            let step_no_op = current_text == writer_plan.after && !blocked;
            debug_assert!(writer_plan.edits.len() <= 1);
            let change = writer_plan.edits.first().map(|edit| XdtoProjectedChange {
                kind: XdtoChangeKind::PackageTextEdit,
                location: addressed_location(&package),
                body_byte_range: XdtoByteRange {
                    start: edit.range.start,
                    end: edit.range.end,
                },
                removed_byte_count: edit.range.end - edit.range.start,
                replacement_byte_count: edit.replacement.len(),
            });
            let mut findings = validation
                .findings
                .iter()
                .map(|finding| classified_finding(&package, finding))
                .collect::<Vec<_>>();
            if let Some(finding) = &writer_plan.finding {
                findings.push(writer_finding(&package, finding));
            }
            effects.push(XdtoOperationEffect {
                operation_index: index,
                op: *operation,
                no_op: step_no_op,
                change,
                findings,
            });
            if blocked {
                let mut codes = validation
                    .findings
                    .iter()
                    .filter(|finding| finding.state == validation::FindingState::Introduced)
                    .map(|finding| finding.code.as_str())
                    .collect::<Vec<_>>();
                if let Some(finding) = writer_plan
                    .finding
                    .as_ref()
                    .filter(|_| writer_plan.blocks())
                {
                    codes.push(finding.code);
                }
                return Err(PlanningFailure {
                    error: format!(
                        "operations[{index}]: xdto_validation_failed: introduced findings: {}",
                        codes.join(", ")
                    ),
                    data: Some(Box::new(partial_data(&effects, &package, false))),
                });
            }
            step_baseline = validate(&after_model);
            current_text = writer_plan.after;
        }
        let after = encode_like(&before, &current_text);
        let no_op = before == after;
        let data = partial_data(&effects, &package, no_op);
        Ok(MutationPlan {
            package,
            before,
            after,
            data,
            no_op,
        })
    })();
    let MutationPlan {
        package,
        before,
        after,
        data,
        no_op,
    } = match planned {
        Ok(plan) => plan,
        Err(failure) => {
            return XdtoExecution {
                outcome: AdapterOutcome {
                    ok: false,
                    summary: "unica.xdto.edit rejected XDTO mutation".to_string(),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![failure.error.clone()],
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: Some(format!("{}\n", failure.error)),
                    command: None,
                },
                data: failure.data.map(|data| *data),
            }
        }
    };
    let publication_warnings = if !preview && !no_op {
        let publish_result = (|| -> Result<Vec<String>, XdtoPublicationFailure> {
            let mut transaction = CompileTransaction::new();
            transaction
                .replace_bytes(&package.path, &before, after.clone())
                .map_err(|error| XdtoPublicationFailure::new(XdtoPublicationStage::Plan, error))?;
            let descriptor = guard_resolved_platform_xml_target_dependencies(
                &mut transaction,
                &package.handle,
                context,
            )
            .map_err(|error| {
                XdtoPublicationFailure::new(XdtoPublicationStage::DependencyGuard, error)
            })?;
            if descriptor != package.descriptor_path {
                return Err(XdtoPublicationFailure::new(
                    XdtoPublicationStage::TargetIdentity,
                    "resolved XDTO descriptor changed",
                ));
            }
            let resource = resolve_xdto_resource(&package.handle, context)
                .map_err(XdtoPublicationFailure::from_resolution)?;
            if resource != package.path {
                return Err(XdtoPublicationFailure::new(
                    XdtoPublicationStage::TargetIdentity,
                    "resolved XDTO resource changed",
                ));
            }
            guard_resolved_support(&resource, context).map_err(|error| {
                XdtoPublicationFailure::new(XdtoPublicationStage::SupportGuard, error)
            })?;
            let report = transaction.commit().map_err(|error| {
                XdtoPublicationFailure::new(XdtoPublicationStage::Commit, error)
            })?;
            Ok(xdto_publication_cleanup_warnings(
                &package,
                !report.cleanup_warnings.is_empty(),
            ))
        })();
        match publish_result {
            Ok(warnings) => warnings,
            Err(error) => {
                return XdtoExecution {
                    outcome: AdapterOutcome {
                        ok: false,
                        summary: "unica.xdto.edit could not publish XDTO mutation".to_string(),
                        changes: Vec::new(),
                        warnings: Vec::new(),
                        errors: vec![error.public_projection(&package)],
                        artifacts: Vec::new(),
                        stdout: None,
                        stderr: None,
                        command: None,
                    },
                    data: Some(data),
                };
            }
        }
    } else {
        Vec::new()
    };
    XdtoExecution {
        outcome: AdapterOutcome {
            ok: true,
            summary: if no_op {
                "unica.xdto.edit is already applied".to_string()
            } else if preview {
                "dry run: unica.xdto.edit planned XDTO mutation".to_string()
            } else {
                "unica.xdto.edit applied XDTO mutation".to_string()
            },
            changes: (!preview && !no_op)
                .then(|| {
                    format!(
                        "{} + {}: XDTO package updated",
                        package.source_set, package.metadata_path
                    )
                })
                .into_iter()
                .collect(),
            warnings: publication_warnings,
            errors: Vec::new(),
            artifacts: vec![format!(
                "{} + {}",
                package.source_set, package.metadata_path
            )],
            stdout: None,
            stderr: None,
            command: None,
        },
        data: Some(data),
    }
}

#[derive(Clone, Copy, Debug)]
enum XdtoPublicationFailureCode {
    PublicationFailed,
}

impl XdtoPublicationFailureCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PublicationFailed => "publication_failed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum XdtoPublicationStage {
    Plan,
    DependencyGuard,
    TargetIdentity,
    ResourceResolution,
    SupportGuard,
    Commit,
}

#[derive(Debug)]
struct XdtoPublicationFailure {
    code: XdtoPublicationFailureCode,
    stage: XdtoPublicationStage,
    internal_cause: String,
}

impl XdtoPublicationFailure {
    fn new(stage: XdtoPublicationStage, internal_cause: impl Into<String>) -> Self {
        Self {
            code: XdtoPublicationFailureCode::PublicationFailed,
            stage,
            internal_cause: internal_cause.into(),
        }
    }

    fn from_resolution(error: XdtoResolutionError) -> Self {
        Self::new(
            XdtoPublicationStage::ResourceResolution,
            format!("{}: {}", error.code.as_str(), error.internal_cause),
        )
    }

    fn public_projection(&self, package: &Package) -> String {
        format!(
            "{}: XDTO package {} + {} could not be published",
            self.code.as_str(),
            package.source_set,
            package.metadata_path
        )
    }
}

impl std::fmt::Display for XdtoPublicationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.stage, self.internal_cause)
    }
}

impl std::error::Error for XdtoPublicationFailure {}

fn xdto_publication_cleanup_warnings(package: &Package, cleanup_incomplete: bool) -> Vec<String> {
    cleanup_incomplete
        .then(|| {
            format!(
                "publication_cleanup_incomplete: XDTO package {} + {} was committed; private recovery cleanup is incomplete",
                package.source_set, package.metadata_path
            )
        })
        .into_iter()
        .collect()
}

#[derive(Debug)]
struct XdtoResolutionError {
    code: XdtoPublicErrorCode,
    public_message: String,
    internal_cause: String,
}

impl XdtoResolutionError {
    fn new(code: XdtoPublicErrorCode, internal_cause: impl Into<String>) -> Self {
        let public_message = match code {
            XdtoPublicErrorCode::SourceSetUnknown => "the selected XDTO source set is unavailable",
            XdtoPublicErrorCode::TargetNotFound => {
                "the logical XDTO package target could not be resolved"
            }
            XdtoPublicErrorCode::NotAnXdtoPackage => "the logical target is not an XDTO package",
            XdtoPublicErrorCode::PackageResourceMissing => {
                "the logical XDTO package resource is missing"
            }
            XdtoPublicErrorCode::ContainmentDenied => {
                "the logical XDTO package target is outside the permitted source boundary"
            }
        };
        Self {
            code,
            public_message: public_message.to_string(),
            internal_cause: internal_cause.into(),
        }
    }

    fn from_source_target(error: SourceTargetError) -> Self {
        let code = match (error.code, error.reason()) {
            (
                SourceTargetErrorCode::SourceRootNotAddressable,
                SourceTargetErrorReason::SourceFormatUnsupported,
            ) => XdtoPublicErrorCode::NotAnXdtoPackage,
            (SourceTargetErrorCode::SourceSetRequired, _)
            | (SourceTargetErrorCode::SourceSetNotFound, _)
            | (SourceTargetErrorCode::SourceRootNotAddressable, _) => {
                XdtoPublicErrorCode::SourceSetUnknown
            }
            (SourceTargetErrorCode::TargetKindMismatch, _) => XdtoPublicErrorCode::NotAnXdtoPackage,
            (SourceTargetErrorCode::ContainmentDenied, _) => XdtoPublicErrorCode::ContainmentDenied,
            (SourceTargetErrorCode::MetadataAddressInvalid, _)
            | (SourceTargetErrorCode::MetadataAddressNotFound, _)
            | (SourceTargetErrorCode::AddressProfileUnsupported, _) => {
                XdtoPublicErrorCode::TargetNotFound
            }
        };
        Self::new(code, error.to_string())
    }

    fn into_format_guard(self) -> FormatGuardError {
        FormatGuardError::xdto(self.code, self.public_message, self.internal_cause)
    }
}

impl std::fmt::Display for XdtoResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.public_message)
    }
}

impl std::error::Error for XdtoResolutionError {}

struct Package {
    path: PathBuf,
    descriptor_path: PathBuf,
    handle: ClosedPlatformXmlTarget,
    source_set: String,
    metadata_path: String,
}

struct MutationPlan {
    package: Package,
    before: Vec<u8>,
    after: Vec<u8>,
    data: XdtoEditData,
    no_op: bool,
}

struct PlanningFailure {
    error: String,
    data: Option<Box<XdtoEditData>>,
}

impl PlanningFailure {
    fn plain(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            data: None,
        }
    }
}

impl From<String> for PlanningFailure {
    fn from(error: String) -> Self {
        Self::plain(error)
    }
}

fn resolve_package(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Package, XdtoResolutionError> {
    let source_set = required(args, "sourceSet")
        .map_err(|error| XdtoResolutionError::new(XdtoPublicErrorCode::SourceSetUnknown, error))?;
    let raw_address = required(args, "metadataPath")
        .map_err(|error| XdtoResolutionError::new(XdtoPublicErrorCode::TargetNotFound, error))?;
    let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw_address)
        .map_err(XdtoResolutionError::from_source_target)?;
    let parts = address.as_str().split('.').collect::<Vec<_>>();
    if parts.len() != 2 || parts[0] != "XDTOPackage" {
        return Err(XdtoResolutionError::new(
            XdtoPublicErrorCode::NotAnXdtoPackage,
            "metadataPath must be XDTOPackage.<name>",
        ));
    }
    if parts[1].is_empty() || parts[1].contains(['/', '\\']) || parts[1] == "." || parts[1] == ".."
    {
        return Err(XdtoResolutionError::new(
            XdtoPublicErrorCode::ContainmentDenied,
            "XDTO package name is not a path segment",
        ));
    }
    let target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(address),
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(XdtoResolutionError::from_source_target)?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err(XdtoResolutionError::new(
            XdtoPublicErrorCode::NotAnXdtoPackage,
            "metadataPath must identify an XDTO package",
        ));
    }
    let evidence = platform_xml_resource_evidence(context, &resolution.handle)
        .map_err(XdtoResolutionError::from_source_target)?;
    let path = prove_xdto_resource(&evidence, context)?;
    Ok(Package {
        path,
        descriptor_path: evidence.target_path,
        handle: resolution.handle,
        source_set: resolution.resolved.source_set,
        metadata_path: resolution
            .resolved
            .metadata_path
            .expect("an XDTO object resolution carries its address")
            .as_str()
            .to_string(),
    })
}

fn resolve_xdto_resource(
    handle: &ClosedPlatformXmlTarget,
    context: &WorkspaceContext,
) -> Result<PathBuf, XdtoResolutionError> {
    let evidence = platform_xml_resource_evidence(context, handle)
        .map_err(XdtoResolutionError::from_source_target)?;
    prove_xdto_resource(&evidence, context)
}

fn prove_xdto_resource(
    evidence: &PlatformXmlResourceEvidence,
    context: &WorkspaceContext,
) -> Result<PathBuf, XdtoResolutionError> {
    let descriptor_stem = evidence
        .target_path
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| {
            XdtoResolutionError::new(
                XdtoPublicErrorCode::TargetNotFound,
                "XDTO descriptor has no file stem",
            )
        })?;
    let descriptor_parent = evidence.target_path.parent().ok_or_else(|| {
        XdtoResolutionError::new(
            XdtoPublicErrorCode::ContainmentDenied,
            "XDTO descriptor has no parent",
        )
    })?;
    let resource = WorkspacePathPolicy::new(context)
        .resolve_write(
            descriptor_parent
                .join(descriptor_stem)
                .join("Ext")
                .join("Package.bin"),
        )
        .map_err(|error| {
            XdtoResolutionError::new(
                XdtoPublicErrorCode::ContainmentDenied,
                format!("cannot resolve XDTO package resource: {error}"),
            )
        })?;
    ensure_no_link_components(&evidence.source_root, &resource)
        .map_err(|error| XdtoResolutionError::new(XdtoPublicErrorCode::ContainmentDenied, error))?;
    let metadata = match fs::symlink_metadata(&resource) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(XdtoResolutionError::new(
                XdtoPublicErrorCode::PackageResourceMissing,
                format!("{}: {error}", resource.display()),
            ));
        }
        Err(error) => {
            return Err(XdtoResolutionError::new(
                XdtoPublicErrorCode::ContainmentDenied,
                format!("cannot inspect {}: {error}", resource.display()),
            ));
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(XdtoResolutionError::new(
            XdtoPublicErrorCode::ContainmentDenied,
            format!(
                "XDTO package resource must not be a link: {}",
                resource.display()
            ),
        ));
    }
    if !metadata.is_file() {
        return Err(XdtoResolutionError::new(
            XdtoPublicErrorCode::PackageResourceMissing,
            format!(
                "XDTO package resource is not a regular file: {}",
                resource.display()
            ),
        ));
    }
    let source_root = normalize_path_identity(&evidence.source_root).map_err(|error| {
        XdtoResolutionError::new(
            XdtoPublicErrorCode::ContainmentDenied,
            format!("cannot resolve XDTO sourceSet identity: {error}"),
        )
    })?;
    let resource_identity = normalize_path_identity(&resource).map_err(|error| {
        XdtoResolutionError::new(
            XdtoPublicErrorCode::ContainmentDenied,
            format!("cannot resolve XDTO package resource identity: {error}"),
        )
    })?;
    if !resource_identity.starts_with(&source_root) {
        return Err(XdtoResolutionError::new(
            XdtoPublicErrorCode::ContainmentDenied,
            format!(
                "XDTO package resource {} escapes sourceSet {}",
                resource_identity.display(),
                source_root.display()
            ),
        ));
    }
    Ok(resource)
}

fn ensure_no_link_components(source_root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(source_root)
        .map_err(|_| "containment_denied: XDTO package resource escapes sourceSet".to_string())?;
    let mut current = source_root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => break,
            Err(_) => {
                return Err("containment_denied: cannot inspect XDTO package path".to_string())
            }
        };
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err("containment_denied: XDTO package path contains a link".to_string());
        }
    }
    Ok(())
}

fn guard_resolved_support(target: &Path, context: &WorkspaceContext) -> Result<(), String> {
    match evaluate_resolved_support_guard(target, SupportGuardRequirement::Editable, context) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => Ok(()),
        ResolvedSupportGuardCheck::Block(violation) => {
            Err(format!("support_locked: {}", violation.reason))
        }
    }
}

fn decode(raw: &[u8]) -> Result<String, String> {
    let value = std::str::from_utf8(raw)
        .map_err(|_| "unsupported_node: Package.bin is not UTF-8".to_string())?;
    let body = value.strip_prefix('\u{feff}').unwrap_or(value);
    if body.starts_with('\u{feff}') {
        return Err(
            "unsupported_node: Package.bin must begin with at most one UTF-8 BOM".to_string(),
        );
    }
    Ok(body.to_string())
}
fn encode_like(before: &[u8], text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    if before.starts_with(&[0xef, 0xbb, 0xbf]) {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(text.as_bytes());
    bytes
}
fn model_error(error: model::ModelError) -> String {
    format!(
        "{}: {} at byte {}",
        error.code, error.message, error.span.start
    )
}
fn descriptor_namespace(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|_| "target_not_found: cannot read proven XDTO descriptor".to_string())?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "not_an_xdto_package: XDTO descriptor is not UTF-8".to_string())?;
    let document = Document::parse(text)
        .map_err(|_| "not_an_xdto_package: XDTO descriptor is not valid XML".to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")) {
        return Err(
            "not_an_xdto_package: descriptor root must be an MDClasses MetaDataObject".to_string(),
        );
    }
    let package = root
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "XDTOPackage")))
        .ok_or_else(|| "not_an_xdto_package: descriptor must contain an XDTOPackage".to_string())?;
    let properties = package
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Properties")))
        .ok_or_else(|| "namespace_mismatch: XDTO descriptor has no Properties".to_string())?;
    properties
        .children()
        .find(|child| child.has_tag_name((MD_CLASSES_NS, "Namespace")))
        .and_then(|namespace| namespace.text())
        .filter(|namespace| !namespace.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "namespace_mismatch: XDTO descriptor has no Namespace".to_string())
}
fn parse(text: &str) -> Result<Document<'_>, String> {
    let doc = Document::parse(text)
        .map_err(|error| format!("unsupported_node: invalid XDTO XML: {error}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "package" || root.tag_name().namespace() != Some(XDTO_NS) {
        return Err("not_an_xdto_package: Package.bin root is not an XDTO package".to_string());
    }
    Ok(doc)
}
fn required<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} must be a non-empty string"))
}
#[cfg(test)]
mod tests {
    use super::{apply_with_data, decode, encode_like, info_with_data, preview_with_data, writer};
    use crate::application::{SupportGuardRequirement, UnicaApplication};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::common::support_guard_violation;
    use crate::infrastructure::native_operations::single_file_publisher::{
        with_before_commit_hook, with_publish_failpoints, PublishCheckpoint,
    };
    use crate::infrastructure::platform::filesystem::{
        create_dir_symlink_for_test, remove_dir_symlink_for_test,
    };
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use serde::Serialize;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    const PACKAGE: &str = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test">
	<objectType name="ЛюбаяСсылка">
		<property name="СсылкаНаОбъект">
			<typeDef xsi:type="ObjectType">
			</typeDef>
		</property>
	</objectType>
	<objectType name="СоставнойЛюбойОбъект"/>
</package>"#;

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    fn args(entries: &[(&str, Value)]) -> Map<String, Value> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn add_property_descends_through_property_path_and_is_idempotent() {
        let args = args(&[
            ("typeName", json!("ЛюбаяСсылка")),
            ("propertyPath", json!("СсылкаНаОбъект")),
            (
                "property",
                json!({"name":"Документ_Новый", "type":"tns:Документ_Новый", "minOccurs":0}),
            ),
        ]);
        let once = writer::plan(PACKAGE, &args, "add-property").unwrap();
        assert_eq!(
            once.after,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" targetNamespace="urn:test">
	<objectType name="ЛюбаяСсылка">
		<property name="СсылкаНаОбъект">
			<typeDef xsi:type="ObjectType">
				<property name="Документ_Новый" type="tns:Документ_Новый" lowerBound="0"/>
			</typeDef>
		</property>
	</objectType>
	<objectType name="СоставнойЛюбойОбъект"/>
</package>"#
        );
        let repeated = writer::plan(&once.after, &args, "add-property").unwrap();
        assert_eq!(repeated.after, once.after);
        assert!(repeated.edits.is_empty());
        assert_eq!(repeated.finding.unwrap().code, "duplicate_property");
    }

    #[test]
    fn byte_encoding_keeps_bom_and_crlf() {
        let before = format!("\u{feff}{}", PACKAGE.replace('\n', "\r\n")).into_bytes();
        let text = decode(&before).unwrap();
        let after = writer::plan(
            &text,
            &args(&[("name", json!("Новый")), ("base", json!("xs:string"))]),
            "add-value-type",
        )
        .unwrap()
        .after;
        let bytes = encode_like(&before, &after);
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(decode(&bytes).unwrap().contains("\r\n"));
    }

    #[test]
    fn enterprise_data_public_edit_reuses_local_tns_binding_byte_exactly_and_repeats() {
        let (context, arguments, package) = enterprise_xdto_fixture("local-tns");
        let before = fs::read(&package).unwrap();
        let marker = b"\t\t\t</typeDef>\r\n";
        let insertion = concat!(
            "\t\t\t\t<property xmlns:tns=\"http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3\" ",
            "name=\"Документ_НовыйДокумент\" type=\"tns:Документ_ЗаказКлиента\" ",
            "lowerBound=\"0\"/>\r\n"
        )
        .as_bytes();
        let marker_offset = before
            .windows(marker.len())
            .position(|window| window == marker)
            .expect("tracked EnterpriseData fixture must contain the nested typeDef close");
        assert_eq!(
            before
                .windows(marker.len())
                .filter(|window| *window == marker)
                .count(),
            1
        );
        let mut expected = Vec::with_capacity(before.len() + insertion.len());
        expected.extend_from_slice(&before[..marker_offset]);
        expected.extend_from_slice(insertion);
        expected.extend_from_slice(&before[marker_offset..]);

        let preview = UnicaApplication::new()
            .call_tool(
                "unica.xdto.edit",
                &public_edit_args(&context, &arguments, true),
            )
            .unwrap();
        assert!(preview.ok, "{preview:?}");
        assert_eq!(preview.data.as_ref().unwrap()["noOp"], false);
        assert_eq!(preview.cache.events, vec!["MetadataChanged"]);
        assert_eq!(fs::read(&package).unwrap(), before);

        let applied = UnicaApplication::new()
            .call_tool(
                "unica.xdto.edit",
                &public_edit_args(&context, &arguments, false),
            )
            .unwrap();
        assert!(applied.ok, "{applied:?}");
        assert_eq!(applied.cache.events, vec!["MetadataChanged"]);
        assert_eq!(fs::read(&package).unwrap(), expected);

        let repeated = UnicaApplication::new()
            .call_tool(
                "unica.xdto.edit",
                &public_edit_args(&context, &arguments, true),
            )
            .unwrap();
        assert!(repeated.ok, "{repeated:?}");
        assert_eq!(repeated.data.as_ref().unwrap()["noOp"], true);
        assert!(repeated.cache.events.is_empty());
        assert_eq!(fs::read(&package).unwrap(), expected);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    fn xdto_guard_fixture(
        name: &str,
    ) -> (
        WorkspaceContext,
        Map<String, Value>,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let root = std::env::temp_dir().join(format!(
            "unica-xdto-guard-{name}-{}-{}",
            std::process::id(),
            TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let descriptor = root.join("src/XDTOPackages/Sample.xml");
        let package = root.join("src/XDTOPackages/Sample/Ext/Package.bin");
        fs::create_dir_all(package.parent().unwrap()).unwrap();
        fs::create_dir_all(root.join("src/Ext")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><XDTOPackage>Sample</XDTOPackage></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Sample</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(&package, PACKAGE).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let args = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            (
                "operations",
                json!([{"op": "addObjectType", "name": "Added"}]),
            ),
        ]);
        (context, args, package, descriptor)
    }

    fn finding_codes(
        execution: &super::XdtoExecution<super::XdtoEditData>,
        state: &str,
    ) -> Vec<String> {
        serde_json::to_value(execution.data.as_ref().expect("edit data"))
            .unwrap()
            .get("effects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|effect| effect.get("findings").and_then(Value::as_array))
            .flatten()
            .filter(|finding| finding.get("state").and_then(Value::as_str) == Some(state))
            .filter_map(|finding| finding.get("code").and_then(Value::as_str))
            .map(str::to_string)
            .collect()
    }

    fn data_json<T: Serialize>(data: Option<T>) -> Value {
        serde_json::to_value(data.expect("typed XDTO execution must carry data")).unwrap()
    }

    fn public_edit_args(
        context: &WorkspaceContext,
        base: &Map<String, Value>,
        dry_run: bool,
    ) -> Map<String, Value> {
        let mut arguments = base.clone();
        arguments.insert(
            "cwd".to_string(),
            json!(context.workspace_root.to_string_lossy()),
        );
        arguments.insert("dryRun".to_string(), json!(dry_run));
        arguments
    }

    fn transaction_debris(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        fn visit(path: &std::path::Path, debris: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".unica-recovery-"))
                {
                    debris.push(path.clone());
                }
                if path.is_dir() {
                    visit(&path, debris);
                }
            }
        }

        let mut debris = Vec::new();
        visit(root, &mut debris);
        debris
    }

    fn enterprise_xdto_fixture(
        name: &str,
    ) -> (WorkspaceContext, Map<String, Value>, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-xdto-enterprise-{name}-{}-{}",
            std::process::id(),
            TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let source = root.join("src");
        let descriptor = source.join("XDTOPackages/EnterpriseData_1_17_3.xml");
        let package = source.join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
        fs::create_dir_all(package.parent().unwrap()).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            source.join("Configuration.xml"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/xdto/enterprise-data-minimal/Configuration.xml"
            )),
        )
        .unwrap();
        fs::write(
            &descriptor,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3.xml"
            )),
        )
        .unwrap();
        fs::write(
            &package,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"
            )),
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let arguments = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.EnterpriseData_1_17_3")),
            (
                "operations",
                json!([{
                    "op": "addProperty",
                    "typeName": "ЛюбаяСсылка",
                    "propertyPath": "СсылкаНаОбъект",
                    "property": {
                        "name":"Документ_НовыйДокумент",
                        "type":"tns:Документ_ЗаказКлиента",
                        "minOccurs":0
                    }
                }]),
            ),
        ]);
        (context, arguments, package)
    }

    #[test]
    fn xdto_info_returns_typed_summary_nested_detail_and_logical_locations() {
        let (context, _, package, _) = xdto_guard_fixture("info-detail");
        fs::write(
            &package,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" xmlns:ext="urn:external" targetNamespace="urn:test">
	<import namespace="urn:external"/>
	<property name="Global" type="xs:string"/>
	<valueType name="Scalar" base="xs:string"/>
	<objectType name="NestedHolder">
		<property name="Items" type="ext:Remote" lowerBound="0" upperBound="-1"/>
		<property name="Nested">
			<typeDef xsi:type="ObjectType">
				<property name="Leaf" type="xs:string" upperBound="123456789012345678901234567890"/>
			</typeDef>
		</property>
	</objectType>
	<objectType name="Last"/>
</package>"#,
        )
        .unwrap();
        let request = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("ПакетXDTO.Sample")),
            ("typeName", json!("NestedHolder")),
        ]);

        let data = data_json(info_with_data(&request, &context).unwrap().data);

        assert_eq!(data["sourceSet"], "main");
        assert_eq!(data["metadataPath"], "XDTOPackage.Sample");
        assert_eq!(data["location"]["kind"], "addressed");
        assert_eq!(data["location"]["sourceSet"], "main");
        assert_eq!(data["location"]["metadataPath"], "XDTOPackage.Sample");
        assert_eq!(data["location"]["targetKind"], "metadataObject");
        assert_eq!(data["targetNamespace"], "urn:test");
        assert_eq!(
            data["counts"],
            json!({"total":3,"valueTypes":1,"objectTypes":2,"globalProperties":1})
        );
        assert_eq!(data["globalProperties"][0]["name"], "Global");
        assert_eq!(data["globalProperties"][0]["type"]["raw"], "xs:string");
        assert_eq!(data["imports"][0]["namespace"], "urn:external");
        assert_eq!(data["imports"][0]["location"]["kind"], "unaddressable");
        assert_eq!(
            data["imports"][0]["location"]["ownerMetadataPath"],
            "XDTOPackage.Sample"
        );
        assert!(data["types"].as_array().unwrap().is_empty());
        let detail = &data["typeDetail"];
        assert_eq!(detail["kind"], "objectType");
        assert_eq!(detail["name"], "NestedHolder");
        assert_eq!(detail["location"]["kind"], "unaddressable");
        assert_eq!(detail["properties"][0]["bounds"]["lower"], "0");
        assert_eq!(detail["properties"][0]["bounds"]["upper"], "-1");
        assert_eq!(detail["properties"][0]["bounds"]["lowerExplicit"], true);
        assert_eq!(detail["properties"][0]["bounds"]["upperExplicit"], true);
        assert_eq!(detail["properties"][0]["bounds"]["unbounded"], true);
        let leaf = &detail["properties"][1]["typeDefs"][0]["properties"][0];
        assert_eq!(leaf["name"], "Leaf");
        assert_eq!(leaf["bounds"]["lower"], "1");
        assert_eq!(leaf["bounds"]["upper"], "123456789012345678901234567890");
        assert_eq!(leaf["bounds"]["lowerExplicit"], false);
        assert_eq!(leaf["bounds"]["upperExplicit"], true);
        assert_eq!(leaf["location"]["kind"], "unaddressable");
        let rendered = serde_json::to_string(&data).unwrap();
        assert!(!rendered.contains(package.to_string_lossy().as_ref()));
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_info_and_edit_share_the_escaped_dotted_property_identity() {
        let (context, _, package, _) = xdto_guard_fixture("dotted-property-identity");
        let before = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" targetNamespace="urn:test">
	<objectType name="Holder">
		<property name="A.B">
			<typeDef xsi:type="ObjectType">
			</typeDef>
		</property>
	</objectType>
</package>"#;
        fs::write(&package, before).unwrap();
        let app = UnicaApplication::new();
        let public_args = |entries: &[(&str, Value)]| {
            let mut call = args(entries);
            call.insert(
                "cwd".to_string(),
                json!(context.workspace_root.to_string_lossy()),
            );
            call.insert("sourceSet".to_string(), json!("main"));
            call.insert("metadataPath".to_string(), json!("XDTOPackage.Sample"));
            call
        };

        let info = app
            .call_tool(
                "unica.xdto.info",
                &public_args(&[("typeName", json!("Holder"))]),
            )
            .unwrap();
        assert!(info.ok, "{info:?}");
        let dotted = &info.data.as_ref().unwrap()["typeDetail"]["properties"][0];
        assert_eq!(dotted["name"], "A.B");
        assert!(info.data.as_ref().unwrap()["findings"]
            .as_array()
            .unwrap()
            .is_empty());

        let edit = app
            .call_tool(
                "unica.xdto.edit",
                &public_args(&[
                    ("dryRun", json!(false)),
                    (
                        "operations",
                        json!([{
                            "op": "addProperty",
                            "typeName": "Holder",
                            "propertyPath": r"A\.B",
                            "property": {"name":"Child", "type":"xs:string"}
                        }]),
                    ),
                ]),
            )
            .unwrap();
        assert!(edit.ok, "{edit:?}");
        assert_eq!(edit.data.as_ref().unwrap()["noOp"], false);
        let after = fs::read_to_string(&package).unwrap();
        assert!(after.contains(r#"<property name="A.B">"#), "{after}");
        assert!(
            after.contains(r#"<property name="Child" type="xs:string"/>"#),
            "{after}"
        );

        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_info_pages_in_document_order_and_binds_cursor_to_request_and_snapshot() {
        let (context, _, package, _) = xdto_guard_fixture("info-cursor");
        let snapshot = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:test">
	<valueType name="First" base="xs:string"/>
	<objectType name="Second"/>
	<objectType name="Third"/>
</package>"#;
        fs::write(&package, snapshot).unwrap();
        let mut first_request = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            ("limit", json!(1)),
        ]);
        let first = data_json(info_with_data(&first_request, &context).unwrap().data);
        assert_eq!(first["types"][0]["name"], "First");
        let cursor = first["nextCursor"].as_str().unwrap().to_string();

        first_request.insert("cursor".to_string(), json!(cursor));
        let second = data_json(info_with_data(&first_request, &context).unwrap().data);
        assert_eq!(second["types"][0]["name"], "Second");
        let terminal_cursor = second["nextCursor"].as_str().unwrap().to_string();
        first_request.insert("cursor".to_string(), json!(terminal_cursor));
        let terminal = data_json(info_with_data(&first_request, &context).unwrap().data);
        assert_eq!(terminal["types"][0]["name"], "Third");
        assert!(terminal.get("nextCursor").is_none());

        for limit in [None, Some(50)] {
            let mut request = args(&[
                ("sourceSet", json!("main")),
                ("metadataPath", json!("XDTOPackage.Sample")),
            ]);
            if let Some(limit) = limit {
                request.insert("limit".to_string(), json!(limit));
            }
            let page = data_json(info_with_data(&request, &context).unwrap().data);
            assert_eq!(page["types"].as_array().unwrap().len(), 3);
            assert!(page.get("nextCursor").is_none());
        }

        let mut foreign_request = first_request.clone();
        foreign_request.insert("limit".to_string(), json!(2));
        assert!(info_with_data(&foreign_request, &context)
            .err()
            .expect("foreign cursor must fail")
            .starts_with("cursor_invalid:"));

        let mut malformed_request = first_request.clone();
        malformed_request.insert("cursor".to_string(), json!("not-a-cursor"));
        assert!(info_with_data(&malformed_request, &context)
            .err()
            .expect("malformed cursor must fail")
            .starts_with("cursor_invalid:"));

        for whitespace_cursor in [" nav1-token", "nav1 token", "nav1-token "] {
            let mut whitespace_request = first_request.clone();
            whitespace_request.insert("cursor".to_string(), json!(whitespace_cursor));
            assert!(
                info_with_data(&whitespace_request, &context)
                    .err()
                    .expect("cursor whitespace must fail in the handler")
                    .starts_with("cursor_invalid:"),
                "{whitespace_cursor:?}"
            );
        }

        fs::write(&package, b"<package").unwrap();
        assert!(info_with_data(&first_request, &context)
            .err()
            .expect("malformed replacement snapshot must invalidate the cursor before parsing")
            .starts_with("cursor_invalid:"));

        fs::write(
            &package,
            snapshot.replace(
                "targetNamespace=\"urn:test\"",
                "targetNamespace=\"urn:other\"",
            ),
        )
        .unwrap();
        assert!(info_with_data(&first_request, &context)
            .err()
            .expect("namespace-changing replacement must invalidate the cursor before validation")
            .starts_with("cursor_invalid:"));

        fs::write(&package, format!("{snapshot}\n")).unwrap();
        assert!(info_with_data(&first_request, &context)
            .err()
            .expect("stale cursor must fail")
            .starts_with("cursor_invalid:"));

        for incompatible in [
            args(&[
                ("sourceSet", json!("main")),
                ("metadataPath", json!("XDTOPackage.Sample")),
                ("typeName", json!("First")),
                ("limit", json!(1)),
            ]),
            args(&[
                ("sourceSet", json!("main")),
                ("metadataPath", json!("XDTOPackage.Sample")),
                ("limit", json!(51)),
            ]),
        ] {
            assert!(info_with_data(&incompatible, &context).is_err());
        }
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_info_rejects_missing_or_ambiguous_detail_and_reports_unsupported_baseline() {
        let (context, _, package, _) = xdto_guard_fixture("info-detail-errors");
        let request = |name| {
            args(&[
                ("sourceSet", json!("main")),
                ("metadataPath", json!("XDTOPackage.Sample")),
                ("typeName", json!(name)),
            ])
        };
        let missing = info_with_data(&request("Missing"), &context)
            .err()
            .expect("missing detail must fail");
        assert!(missing.starts_with("target_not_found:"), "{missing}");

        fs::write(
            &package,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" targetNamespace="urn:test">
	<valueType name="Duplicate" base="xs:string"/>
	<objectType name="Duplicate"/>
</package>"#,
        )
        .unwrap();
        let ambiguous = info_with_data(&request("Duplicate"), &context)
            .err()
            .expect("ambiguous joint identity must fail");
        assert!(ambiguous.starts_with("duplicate_type:"), "{ambiguous}");

        fs::write(
            &package,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" targetNamespace="urn:test">
	<objectType name="Container">
		<property name="Nested">
			<typeDef xsi:type="ObjectType"><property name="First" type="xs:string"/></typeDef>
			<typeDef xsi:type="ObjectType"><property name="Second" type="xs:string"/></typeDef>
		</property>
	</objectType>
</package>"#,
        )
        .unwrap();
        let data = data_json(
            info_with_data(&request("Container"), &context)
                .unwrap()
                .data,
        );
        assert_eq!(
            data["typeDetail"]["properties"][0]["typeDefs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "type_definition_conflict"
                && finding["state"] == "pre_existing"
                && finding["location"]["kind"] == "unaddressable"));
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_info_body_byte_ranges_exclude_bom_and_count_multibyte_names_as_utf8() {
        let (context, _, package, _) = xdto_guard_fixture("info-byte-range");
        let body = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" targetNamespace="urn:test">
	<objectType name="Тип"/>
</package>"#;
        fs::write(&package, format!("\u{feff}{body}")).unwrap();
        let request = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            ("typeName", json!("Тип")),
        ]);

        let data = data_json(info_with_data(&request, &context).unwrap().data);
        let range = &data["typeDetail"]["location"]["bodyByteRange"];
        assert_eq!(
            range["start"].as_u64().unwrap() as usize,
            body.find("<objectType").unwrap()
        );
        assert_eq!(
            range["end"].as_u64().unwrap() as usize,
            body.find("<objectType").unwrap() + "<objectType name=\"Тип\"/>".len()
        );
        assert_eq!(data["typeDetail"]["location"]["kind"], "unaddressable");
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_events_follow_changed_plan_and_exact_noop_for_preview_and_apply() {
        let (context, base_args, package, _) = xdto_guard_fixture("events");
        let before = fs::read(&package).unwrap();
        let call_args = |dry_run| {
            let mut args = base_args.clone();
            args.insert(
                "cwd".to_string(),
                json!(context.workspace_root.to_string_lossy()),
            );
            args.insert("dryRun".to_string(), json!(dry_run));
            args
        };

        let preview = UnicaApplication::new()
            .call_tool("unica.xdto.edit", &call_args(true))
            .unwrap();
        assert!(preview.ok, "{:?}", preview.errors);
        assert_eq!(preview.cache.mode, "dry-run");
        assert_eq!(preview.cache.events, vec!["MetadataChanged"]);
        assert!(preview
            .cache
            .invalidated
            .contains(&"metadata_graph".to_string()));
        assert_eq!(preview.data.as_ref().unwrap()["noOp"], false);
        assert!(preview.data.as_ref().unwrap()["effects"][0]["change"].is_object());
        assert_eq!(fs::read(&package).unwrap(), before);

        let applied = UnicaApplication::new()
            .call_tool("unica.xdto.edit", &call_args(false))
            .unwrap();
        assert!(applied.ok, "{:?}", applied.errors);
        assert_eq!(applied.cache.mode, "applied");
        assert_eq!(applied.cache.events, vec!["MetadataChanged"]);
        assert!(applied
            .cache
            .invalidated
            .contains(&"metadata_graph".to_string()));
        let after = fs::read(&package).unwrap();
        assert_ne!(after, before);

        for dry_run in [true, false] {
            let repeated = UnicaApplication::new()
                .call_tool("unica.xdto.edit", &call_args(dry_run))
                .unwrap();
            assert!(repeated.ok, "{:?}", repeated.errors);
            assert_eq!(repeated.data.as_ref().unwrap()["noOp"], true);
            assert!(repeated.data.as_ref().unwrap()["effects"][0]["change"].is_null());
            assert!(repeated.cache.events.is_empty(), "dryRun={dry_run}");
            assert!(repeated.cache.invalidated.is_empty(), "dryRun={dry_run}");
            assert_eq!(fs::read(&package).unwrap(), after);
        }
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_public_edit_rejects_two_or_more_boms_without_bytes_or_cache_events() {
        let (context, mut arguments, package, _) = xdto_guard_fixture("multiple-bom");
        arguments.insert(
            "operations".to_string(),
            json!([{"op": "addObjectType", "name": "СоставнойЛюбойОбъект"}]),
        );

        for bom_count in [2, 3] {
            let mut before = Vec::new();
            for _ in 0..bom_count {
                before.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            }
            before.extend_from_slice(PACKAGE.as_bytes());
            fs::write(&package, &before).unwrap();

            for dry_run in [true, false] {
                let result = UnicaApplication::new()
                    .call_tool(
                        "unica.xdto.edit",
                        &public_edit_args(&context, &arguments, dry_run),
                    )
                    .unwrap();
                assert!(
                    !result.ok,
                    "bomCount={bom_count}, dryRun={dry_run}: {result:?}"
                );
                assert_eq!(
                    result.errors,
                    ["unsupported_node: Package.bin must begin with at most one UTF-8 BOM"]
                );
                assert!(
                    result.cache.events.is_empty(),
                    "bomCount={bom_count}, dryRun={dry_run}"
                );
                assert!(
                    result.cache.invalidated.is_empty(),
                    "bomCount={bom_count}, dryRun={dry_run}"
                );
                assert_eq!(
                    fs::read(&package).unwrap(),
                    before,
                    "bomCount={bom_count}, dryRun={dry_run}"
                );
            }
        }
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_public_edit_surfaces_logical_cleanup_warning_after_committed_bytes() {
        let (context, arguments, package, _) = xdto_guard_fixture("cleanup-warning");
        let before = fs::read(&package).unwrap();

        let result = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
            apply_with_data(&arguments, &context)
        });

        assert!(result.outcome.ok, "{:?}", result.outcome);
        let expected_warning = concat!(
            "publication_cleanup_incomplete: XDTO package main + XDTOPackage.Sample ",
            "was committed; private recovery cleanup is incomplete"
        );
        assert_eq!(result.outcome.warnings, [expected_warning]);
        let committed = fs::read(&package).unwrap();
        assert_ne!(committed, before);
        assert!(String::from_utf8_lossy(&committed).contains("name=\"Added\""));
        let debris = transaction_debris(&context.workspace_root);
        assert_eq!(debris.len(), 1, "{debris:?}");
        assert_eq!(fs::read(debris[0].join("original")).unwrap(), before);
        let public = serde_json::to_string(&json!({
            "outcome": result.outcome,
            "data": result.data,
        }))
        .unwrap();
        for forbidden in [
            context.workspace_root.to_string_lossy().as_ref(),
            package.to_string_lossy().as_ref(),
            "Package.bin",
            ".unica-recovery-",
            "injected publication cleanup failure",
        ] {
            assert!(
                !public.contains(forbidden),
                "leaked {forbidden:?}: {public}"
            );
        }
        fs::remove_dir_all(&debris[0]).unwrap();
        assert!(transaction_debris(&context.workspace_root).is_empty());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_publication_failure_from_symlinked_cwd_is_closed_and_logical() {
        let (context, arguments, package, _) = xdto_guard_fixture("symlink-publication");
        let alias = context.workspace_root.parent().unwrap().join(format!(
            "{}-alias",
            context
                .workspace_root
                .file_name()
                .unwrap()
                .to_string_lossy()
        ));
        let Some(link) = create_dir_symlink_for_test(&context.workspace_root, &alias) else {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        };
        link.unwrap();
        let support = context
            .workspace_root
            .join("src/Ext/ParentConfigurations.bin");
        let support_for_hook = support.clone();
        let mut public_arguments = arguments.clone();
        public_arguments.insert("cwd".to_string(), json!(alias.to_string_lossy()));
        public_arguments.insert("dryRun".to_string(), json!(false));

        let result = with_before_commit_hook(
            move |_| fs::write(&support_for_hook, b"concurrent support state").unwrap(),
            || {
                UnicaApplication::new()
                    .call_tool("unica.xdto.edit", &public_arguments)
                    .unwrap()
            },
        );

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            result.errors,
            [concat!(
                "publication_failed: XDTO package main + XDTOPackage.Sample ",
                "could not be published"
            )]
        );
        assert!(result.cache.events.is_empty());
        assert!(result.cache.invalidated.is_empty());
        let public = serde_json::to_string(&json!({
            "summary": result.summary,
            "changes": result.changes,
            "warnings": result.warnings,
            "errors": result.errors,
            "artifacts": result.artifacts,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "command": result.command,
            "diagnostics": result.diagnostics,
            "data": result.data,
            "job": result.job,
        }))
        .unwrap();
        for forbidden in [
            context.workspace_root.to_string_lossy().as_ref(),
            alias.to_string_lossy().as_ref(),
            package.to_string_lossy().as_ref(),
            support.to_string_lossy().as_ref(),
            "ParentConfigurations.bin",
            "Package.bin",
            "Configuration.xml",
            "provider",
            "absence guard",
            "compile transaction",
            ".unica-recovery-",
        ] {
            assert!(
                !public.contains(forbidden),
                "leaked {forbidden:?}: {public}"
            );
        }
        assert!(support.is_file());
        remove_dir_symlink_for_test(&alias).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_events_rejected_or_commit_failed_plans_never_emit_or_write_package() {
        let (context, base_args, package, descriptor) = xdto_guard_fixture("events-failures");
        let before = fs::read(&package).unwrap();
        let call = |mut args: Map<String, Value>, dry_run| {
            args.insert(
                "cwd".to_string(),
                json!(context.workspace_root.to_string_lossy()),
            );
            args.insert("dryRun".to_string(), json!(dry_run));
            UnicaApplication::new()
                .call_tool("unica.xdto.edit", &args)
                .unwrap()
        };

        let conflicting = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            (
                "operations",
                json!([{"op": "addValueType", "name": "ЛюбаяСсылка", "base": "xs:string"}]),
            ),
        ]);
        for dry_run in [true, false] {
            let rejected = call(conflicting.clone(), dry_run);
            assert!(!rejected.ok);
            assert_eq!(rejected.data.as_ref().unwrap()["noOp"], false);
            assert!(rejected.data.as_ref().unwrap()["effects"][0]["change"].is_null());
            assert!(rejected.cache.events.is_empty());
            assert_eq!(fs::read(&package).unwrap(), before);
        }

        let semantic = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            (
                "operations",
                json!([{
                    "op": "addProperty",
                    "typeName": "ЛюбаяСсылка",
                    "property": {"name":"Broken", "type":"tns:Missing"}
                }]),
            ),
        ]);
        for dry_run in [true, false] {
            let rejected = call(semantic.clone(), dry_run);
            assert!(!rejected.ok);
            assert_eq!(rejected.data.as_ref().unwrap()["noOp"], false);
            assert!(rejected.data.as_ref().unwrap()["effects"][0]["change"].is_object());
            assert!(rejected.cache.events.is_empty());
            assert_eq!(fs::read(&package).unwrap(), before);
        }

        let descriptor_for_hook = descriptor.clone();
        let failed_publish = with_before_commit_hook(
            move |_| fs::write(&descriptor_for_hook, "<concurrent/>").unwrap(),
            || call(base_args, false),
        );
        assert!(!failed_publish.ok);
        assert!(failed_publish.cache.events.is_empty());
        assert_eq!(fs::read(&package).unwrap(), before);
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), "<concurrent/>");
        assert!(
            !failed_publish
                .errors
                .join("\n")
                .contains(context.workspace_root.to_string_lossy().as_ref()),
            "{:?}",
            failed_publish.errors
        );
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_operations_apply_in_order_within_one_transaction() {
        let (context, _, package, _) = xdto_guard_fixture("batch-transaction");
        let before = fs::read(&package).unwrap();
        let arguments = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            (
                "operations",
                json!([
                    {"op": "addObjectType", "name": "Order"},
                    {"op": "addProperty", "typeName": "Order",
                     "property": {"name": "Ref", "type": "xs:string"}}
                ]),
            ),
        ]);

        let preview = preview_with_data(&arguments, &context);
        assert!(preview.outcome.ok, "{:?}", preview.outcome);
        let preview_data = data_json(preview.data);
        assert_eq!(preview_data["noOp"], json!(false));
        let effects = preview_data["effects"].as_array().unwrap();
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0]["operationIndex"], json!(0));
        assert_eq!(effects[0]["op"], json!("addObjectType"));
        assert_eq!(effects[1]["operationIndex"], json!(1));
        assert_eq!(effects[1]["op"], json!("addProperty"));
        // The second operation targets the type the first one creates, so it
        // plans a real change: element `i` sees the text element `i - 1` made.
        assert!(effects[1]["change"].is_object());
        assert_eq!(fs::read(&package).unwrap(), before);

        let applied = apply_with_data(&arguments, &context);
        assert!(applied.outcome.ok, "{:?}", applied.outcome);
        let after = fs::read_to_string(&package).unwrap();
        assert!(after.contains("\"Order\""), "{after}");
        assert!(
            after.contains(r#"<property name="Ref" type="xs:string"/>"#),
            "{after}"
        );
        assert_eq!(preview_data["effects"], data_json(applied.data)["effects"]);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_failed_batch_element_names_its_index_and_writes_nothing() {
        let (context, _, package, _) = xdto_guard_fixture("batch-atomic");
        let before = fs::read(&package).unwrap();
        let arguments = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            (
                "operations",
                json!([
                    {"op": "addObjectType", "name": "Order"},
                    {"op": "removeType", "name": "Missing"}
                ]),
            ),
        ]);

        for execution in [
            preview_with_data(&arguments, &context),
            apply_with_data(&arguments, &context),
        ] {
            assert!(!execution.outcome.ok, "{:?}", execution.outcome);
            let errors = execution.outcome.errors.join("\n");
            assert!(errors.contains("operations[1]"), "{errors}");
            let data = data_json(execution.data);
            let effects = data["effects"].as_array().unwrap();
            assert_eq!(effects.len(), 1, "{effects:?}");
            assert_eq!(effects[0]["op"], json!("addObjectType"));
            assert_eq!(
                fs::read(&package).unwrap(),
                before,
                "a failed element must leave no partial write"
            );
        }
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_writer_orchestration_reports_exact_and_conflicting_duplicates_without_bytes() {
        let (context, args, package, _) = xdto_guard_fixture("writer-duplicates");
        let applied = apply_with_data(&args, &context);
        assert!(applied.outcome.ok, "{:?}", applied.outcome);
        let after_first_apply = fs::read(&package).unwrap();

        let exact = preview_with_data(&args, &context);
        assert!(exact.outcome.ok, "{:?}", exact.outcome);
        let exact_data = data_json(exact.data);
        assert_eq!(exact_data["noOp"], json!(true));
        let exact_finding = exact_data["effects"][0]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["code"] == "duplicate_type")
            .unwrap();
        assert_eq!(exact_finding["severity"], "info");
        assert_eq!(exact_finding["state"], "pre_existing");

        let mut conflicting_args = args.clone();
        conflicting_args.insert(
            "operations".to_string(),
            json!([{"op": "addValueType", "name": "Added", "base": "xs:string"}]),
        );
        let conflict = preview_with_data(&conflicting_args, &context);
        assert!(!conflict.outcome.ok, "{:?}", conflict.outcome);
        let conflict_data = data_json(conflict.data);
        assert_eq!(conflict_data["noOp"], json!(false));
        assert!(conflict_data["effects"][0]["change"].is_null());
        let conflict_finding = conflict_data["effects"][0]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| finding["code"] == "duplicate_type")
            .unwrap();
        assert_eq!(conflict_finding["severity"], "error");
        assert_eq!(conflict_finding["state"], "introduced");
        assert_eq!(fs::read(&package).unwrap(), after_first_apply);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_writer_orchestration_preserves_bom_crlf_and_repeat_preview_is_no_op() {
        let (context, args, package, _) = xdto_guard_fixture("writer-bom-crlf");
        let original = format!("\u{feff}{}", PACKAGE.replace('\n', "\r\n")).into_bytes();
        fs::write(&package, &original).unwrap();

        let applied = apply_with_data(&args, &context);
        assert!(applied.outcome.ok, "{:?}", applied.outcome);
        let after = fs::read(&package).unwrap();
        assert!(after.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(!decode(&after).unwrap().replace("\r\n", "").contains('\n'));

        let repeated = preview_with_data(&args, &context);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome);
        assert_eq!(
            finding_codes(&repeated, "pre_existing"),
            vec!["duplicate_type"]
        );
        assert_eq!(data_json(repeated.data)["noOp"], json!(true));
        assert_eq!(fs::read(&package).unwrap(), after);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_writer_orchestration_rejects_mixed_kind_target_in_preview_and_apply_without_write() {
        let (context, mut args, package, _) = xdto_guard_fixture("writer-ambiguous-target");
        let before = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:test">
	<valueType name="Target" base="xs:string"/>
	<objectType name="Target"/>
</package>"#
            .as_bytes()
            .to_vec();
        fs::write(&package, &before).unwrap();
        let model =
            super::model::PackageModel::parse(std::str::from_utf8(&before).unwrap()).unwrap();
        assert!(super::validation::validate(&model)
            .iter()
            .any(|finding| finding.code == "duplicate_type"));
        args.insert(
            "operations".to_string(),
            json!([{
                "op": "addProperty",
                "typeName": "Target",
                "property": {"name":"Added", "type":"xs:string"}
            }]),
        );

        for execution in [
            preview_with_data(&args, &context),
            apply_with_data(&args, &context),
        ] {
            assert!(!execution.outcome.ok, "{:?}", execution.outcome);
            let errors = execution.outcome.errors.join("\n");
            assert!(errors.contains("unsupported_node"), "{errors}");
            assert!(errors.contains("ambiguous"), "{errors}");
            assert_eq!(fs::read(&package).unwrap(), before);
        }
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_validation_reports_unrelated_baseline_findings_without_blocking() {
        let (context, mut args, package, _) = xdto_guard_fixture("validation-baseline");
        fs::write(
            &package,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:tns="urn:test" xmlns:ext="urn:external" targetNamespace="urn:test">
	<property name="Global" type="xs:string"/>
	<valueType name="Duplicate" base="xs:string"><enumeration value="x"/></valueType>
	<import namespace="urn:other"/>
	<objectType name="Duplicate">
		<property name="bad:name" type="ext:Remote" lowerBound="2" upperBound="1"/>
		<property name="bad:name" type="xs:string"><typeDef xsi:type="ObjectType"/></property>
		<property name="Nested"><typeDef xsi:type="ValueType"/></property>
		<property name="Undeclared" type="ghost:Remote"/>
		<property name="MissingLocal" type="tns:Missing"/>
		<property name="Both" ref="tns:Global"/>
		<property ref="tns:MissingGlobal"/>
	</objectType>
</package>"#,
        )
        .unwrap();
        args.insert(
            "operations".to_string(),
            json!([{"op": "addObjectType", "name": "Safe"}]),
        );

        let execution = preview_with_data(&args, &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let codes = finding_codes(&execution, "pre_existing");
        for expected in [
            "invalid_group_order",
            "unsupported_node",
            "duplicate_type",
            "duplicate_property",
            "invalid_ncname",
            "missing_import",
            "undeclared_prefix",
            "unknown_type_reference",
            "unknown_property_reference",
            "invalid_property_identity",
            "type_definition_conflict",
            "invalid_bounds",
        ] {
            assert!(
                codes.iter().any(|code| code == expected),
                "{expected}: {codes:?}"
            );
        }
        let rendered = serde_json::to_string(&execution.data).unwrap();
        assert!(!rendered.contains(package.to_string_lossy().as_ref()));
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_validation_accepts_the_untouched_enterprise_data_fixture() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"
        ));
        let text = decode(bytes).unwrap();
        let model = super::model::PackageModel::parse(&text).unwrap();

        let findings = super::validation::validate(&model);

        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn xdto_validation_diff_uses_code_and_logical_key_not_message_or_span() {
        use super::validation::{
            Finding, FindingLocation, FindingSeverity, FindingState, SourceSpanDto, ValidationDiff,
        };
        let baseline = vec![Finding {
            code: "unknown_type_reference".to_string(),
            severity: FindingSeverity::Error,
            message: "old wording".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@type".to_string(),
                span: SourceSpanDto { start: 10, end: 18 },
            },
        }];
        let candidate = vec![Finding {
            code: "unknown_type_reference".to_string(),
            severity: FindingSeverity::Error,
            message: "new wording".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@type".to_string(),
                span: SourceSpanDto {
                    start: 110,
                    end: 118,
                },
            },
        }];

        let diff = ValidationDiff::between(&baseline, candidate);

        assert_eq!(diff.findings.len(), 1);
        assert_eq!(diff.findings[0].state, FindingState::PreExisting);
        assert_eq!(diff.findings[0].message, "old wording");
        assert_eq!(diff.findings[0].location.span.start, 10);
        assert!(!diff.blocks());
    }

    #[test]
    fn xdto_validation_diff_classifies_multiplicity_removal_and_fresh_keys() {
        use super::validation::{
            Finding, FindingLocation, FindingSeverity, FindingState, SourceSpanDto, ValidationDiff,
        };
        let finding = |start| Finding {
            code: "duplicate_property".to_string(),
            severity: FindingSeverity::Error,
            message: "duplicate".to_string(),
            location: FindingLocation {
                key: "$package/objectType:Consumer/property:Value/@unique".to_string(),
                span: SourceSpanDto {
                    start,
                    end: start + 1,
                },
            },
        };

        let diff = ValidationDiff::between(&[finding(10)], vec![finding(20), finding(30)]);

        assert_eq!(diff.findings.len(), 2);
        assert_eq!(diff.findings[0].state, FindingState::PreExisting);
        assert_eq!(diff.findings[1].state, FindingState::Introduced);
        assert!(diff.blocks());

        let removed = ValidationDiff::between(&[finding(40)], Vec::new());
        assert!(removed.findings.is_empty());
        assert!(!removed.blocks());

        let fresh = ValidationDiff::between(&[], vec![finding(50)]);
        assert_eq!(fresh.findings[0].state, FindingState::Introduced);
        assert!(fresh.blocks());
    }

    #[test]
    fn xdto_validation_requires_exact_root_and_target_namespace() {
        let wrong_root = super::model::PackageModel::parse(
            r#"<package xmlns="urn:not-xdto" targetNamespace="urn:test"/>"#,
        )
        .unwrap_err();
        assert_eq!(wrong_root.code, "not_an_xdto_package");

        let missing_namespace =
            super::model::PackageModel::parse(r#"<package xmlns="http://v8.1c.ru/8.1/xdto"/>"#)
                .unwrap();
        let findings = super::validation::validate(&missing_namespace);
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].code, "target_namespace_required");
        assert_eq!(findings[0].location.key, "$package/@targetNamespace");
    }

    #[test]
    fn xdto_validation_uses_xml_ncname_character_ranges() {
        let model = super::model::PackageModel::parse(
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" targetNamespace="urn:test">
	<objectType name="_ok"/>
	<objectType name="Имя"/>
	<objectType name="A·B"/>
	<objectType name="Á"/>
	<objectType name="1bad"/>
	<objectType name="bad:name"/>
</package>"#,
        )
        .unwrap();

        let invalid_names = super::validation::validate(&model)
            .into_iter()
            .filter(|finding| finding.code == "invalid_ncname")
            .map(|finding| finding.location.key)
            .collect::<Vec<_>>();

        assert_eq!(invalid_names.len(), 2, "{invalid_names:#?}");
        assert!(invalid_names.iter().any(|key| key.contains("1bad")));
        assert!(invalid_names.iter().any(|key| key.contains("bad:name")));
    }

    #[test]
    fn xdto_validation_rejects_bare_property_qname_without_rewriting() {
        let (context, mut args, _, _) = xdto_guard_fixture("validation-bare-qname");
        args.insert(
            "operations".to_string(),
            json!([{
                "op": "addProperty",
                "typeName": "ЛюбаяСсылка",
                "property": {"name":"BareSelf", "type":"Missing"}
            }]),
        );

        let execution = preview_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(
            finding_codes(&execution, "introduced"),
            vec!["invalid_qname"]
        );
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_validation_blocks_candidate_unknown_type_and_invalid_ncname() {
        let (context, mut args, package, _) = xdto_guard_fixture("validation-candidate");
        let before = fs::read(&package).unwrap();
        args.insert(
            "operations".to_string(),
            json!([{
                "op": "addProperty",
                "typeName": "ЛюбаяСсылка",
                "property": {"name":"bad:name", "type":"tns:Missing"}
            }]),
        );

        let execution = preview_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        let codes = finding_codes(&execution, "introduced");
        assert!(
            codes.iter().any(|code| code == "invalid_ncname"),
            "{codes:?}"
        );
        assert!(
            codes.iter().any(|code| code == "unknown_type_reference"),
            "{codes:?}"
        );
        assert_eq!(fs::read(&package).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_validation_blocks_removing_a_referenced_local_type() {
        let (context, mut args, package, _) = xdto_guard_fixture("validation-remove-type");
        fs::write(
            &package,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:tns="urn:test" targetNamespace="urn:test">
	<valueType name="Used" base="xs:string"/>
	<objectType name="Consumer"><property name="Value" type="tns:Used"/></objectType>
</package>"#,
        )
        .unwrap();
        args.insert(
            "operations".to_string(),
            json!([{"op": "removeType", "name": "Used"}]),
        );

        let execution = preview_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(
            finding_codes(&execution, "introduced"),
            vec!["unknown_type_reference"]
        );
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_validation_rejects_descriptor_namespace_mismatch_without_path_leak() {
        let (context, args, package, descriptor) = xdto_guard_fixture("validation-descriptor");
        fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Sample</Name><Namespace>urn:other</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
        )
        .unwrap();

        let execution = preview_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        let rendered = format!("{:?}", execution.outcome.errors);
        assert!(rendered.contains("namespace_mismatch"), "{rendered}");
        assert!(!rendered.contains(package.to_string_lossy().as_ref()));
        assert!(!rendered.contains(descriptor.to_string_lossy().as_ref()));
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_descriptor_identity_drift_before_commit() {
        let (context, args, package, descriptor) = xdto_guard_fixture("descriptor-drift");
        let before = fs::read(&package).unwrap();
        let descriptor_for_hook = descriptor.clone();

        let execution = with_before_commit_hook(
            move |_| fs::write(&descriptor_for_hook, "<concurrent/>").unwrap(),
            || apply_with_data(&args, &context),
        );

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(fs::read(&package).unwrap(), before);
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), "<concurrent/>");
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_source_identity_drift_before_commit() {
        let (context, args, package, _) = xdto_guard_fixture("source-drift");
        let before = fs::read(&package).unwrap();
        let project = context.workspace_root.join("v8project.yaml");
        let project_for_hook = project.clone();
        let concurrent = "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: moved\n";

        let execution = with_before_commit_hook(
            move |_| fs::write(&project_for_hook, concurrent).unwrap(),
            || apply_with_data(&args, &context),
        );

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(fs::read(&package).unwrap(), before);
        assert_eq!(fs::read_to_string(&project).unwrap(), concurrent);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_support_state_drift_without_public_path_leak() {
        let (context, args, package, _) = xdto_guard_fixture("support-drift");
        let before = fs::read(&package).unwrap();
        let support = context
            .workspace_root
            .join("src/Ext/ParentConfigurations.bin");
        let support_for_hook = support.clone();
        let concurrent_support = concat!(
            "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
            "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
            "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
            "cccccccc-cccc-cccc-cccc-cccccccccccc,",
            "cccccccc-cccc-cccc-cccc-cccccccccccc}"
        )
        .as_bytes()
        .to_vec();
        let concurrent_support_for_hook = concurrent_support.clone();
        assert!(
            support_guard_violation(&package, SupportGuardRequirement::Editable).is_none(),
            "the initial absent support state must allow editing"
        );

        let execution = with_before_commit_hook(
            move |_| fs::write(&support_for_hook, &concurrent_support_for_hook).unwrap(),
            || apply_with_data(&args, &context),
        );

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        let error = execution.outcome.errors.join("\n");
        assert_eq!(
            error,
            concat!(
                "publication_failed: XDTO package main + XDTOPackage.Sample ",
                "could not be published"
            )
        );
        assert!(!error.contains("ParentConfigurations.bin"), "{error}");
        assert!(!error.contains(context.workspace_root.to_string_lossy().as_ref()));
        assert_eq!(fs::read(&package).unwrap(), before);
        assert_eq!(fs::read(&support).unwrap(), concurrent_support);
        let violation = support_guard_violation(&package, SupportGuardRequirement::Editable)
            .expect("concurrent support state must change the XDTO package verdict to locked");
        assert_eq!(violation.code, "locked");
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_resource_preimage_drift_before_commit() {
        let (context, args, package, _) = xdto_guard_fixture("preimage-drift");
        let concurrent = b"concurrent package bytes".to_vec();
        let concurrent_for_hook = concurrent.clone();

        let execution = with_before_commit_hook(
            move |target| fs::write(target, &concurrent_for_hook).unwrap(),
            || apply_with_data(&args, &context),
        );

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(fs::read(&package).unwrap(), concurrent);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn xdto_guard_rejects_resource_outside_selected_source_set() {
        let (context, args, package, _) = xdto_guard_fixture("outside-source-set");
        let outside = context.workspace_root.join("outside/Package.bin");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, PACKAGE).unwrap();
        fs::remove_file(&package).unwrap();
        let outcome = create_file_link_fixture_for_test(&outside, &package)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            fs::remove_dir_all(context.workspace_root).unwrap();
            return;
        }

        let execution = apply_with_data(&args, &context);

        assert!(!execution.outcome.ok, "{:?}", execution.outcome);
        assert!(
            execution
                .outcome
                .errors
                .join("\n")
                .contains("containment_denied"),
            "{:?}",
            execution.outcome.errors
        );
        assert_eq!(fs::read(&outside).unwrap(), PACKAGE.as_bytes());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }
}

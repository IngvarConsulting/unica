use crate::application::ports::{FormatGuardError, XdtoPublicErrorCode};
use crate::application::SupportGuardRequirement;
use crate::infrastructure::native_operations::common::guard_resolved_platform_xml_target_dependencies;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    TargetKindPolicy,
};
use crate::infrastructure::support_guard::{
    evaluate_resolved_support_guard, ResolvedSupportGuardCheck,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use unica_address::source_target::{
    MetadataAddress, SourceTarget, SourceTargetError, SourceTargetErrorCode,
    SourceTargetErrorReason, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_navigation::source_navigation::{
    authenticate_cursor, page_bounds_from_offset, SOURCE_NAVIGATION_LIMIT_DEFAULT,
    SOURCE_NAVIGATION_LIMIT_MAX,
};
#[cfg(test)]
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, IndexedPlanOperation, ProvisionalApplyEffect,
};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::owner::PlatformXmlResourceEvidence;
#[cfg(test)]
use unica_workspace::actor::ProviderRootBinding;
use unica_workspace::compile_transaction::CompileTransaction;
use unica_workspace::path_policy::WorkspacePathPolicy;
use unica_workspace::paths::metadata_is_link_or_reparse_point;
use unica_workspace::paths::normalize_path_identity;
#[cfg(test)]
use unica_xdto::operations::{
    PrefixedXmlQName, XdtoAddObjectTypeArgs, XdtoAddPropertyArgs, XdtoAddValueTypeArgs,
    XdtoPropertySpec, XdtoRemovePropertyArgs, XdtoRemoveTypeArgs, XmlNcName,
};
#[cfg(test)]
use unica_xdto::operations::{XdtoOperation, XdtoPlanOperation, XdtoPropertyPath};
use unica_xdto::results::*;
use unica_xdto_designer::descriptor::descriptor_fields;

use unica_xdto_designer::edit_plan::{plan_xdto_edit_operations, PlanningFailure};
#[cfg(test)]
use unica_xdto_designer::info::parse_xdto_info_bytes;
use unica_xdto_designer::info::{
    build_xdto_info, model_error, parse_xdto_info_model, validate_xdto_type_name,
};
#[cfg(test)]
use unica_xdto_designer::planner::{
    decode_xdto_plan_operation, plan_xdto_batch, validate_final_xdto_resources,
    XdtoValidationLedger,
};
#[cfg(test)]
use unica_xdto_designer::planner::{staged_writer_error, staged_writer_operation};

use unica_xdto_designer::{decode, model, required};
#[cfg(test)]
use unica_xdto_designer::{validation, writer};

use model::PackageModel;
#[cfg(test)]
use validation::validate;

#[cfg(test)]
fn parse_xdto_plan_operation(
    operation: &str,
    value: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<XdtoPlanOperation, ApplyPlanError> {
    let operation = XdtoOperation::named(operation).ok_or_else(|| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "unsupported XDTO operation")
            .at_path(format!("ops[{op_index}].args"))
    })?;
    decode_xdto_plan_operation(operation, value, op_index, binding.source_set_name())
}

#[cfg(test)]
fn plan_unindexed_xdto_batch(
    staged: unica_workspace::apply::ApplyStagedState,
    authority: unica_workspace::actor::XdtoApplyAuthority<'_>,
    operations: &[XdtoPlanOperation],
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        unica_source_contract::proposal::PlannedApplyEffects,
    ),
    ApplyPlanError,
> {
    let indexed = operations
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, operation)| IndexedPlanOperation::new(index, operation))
        .collect::<Vec<_>>();
    let mut validation_ledger = XdtoValidationLedger::default();
    let (mut staged, provisional) =
        plan_xdto_batch_owned_for_test(staged, &authority, &indexed, &mut validation_ledger)?;
    validate_final_xdto_resources(
        &mut staged.proposal(),
        &authority,
        &validation_ledger,
        indexed.iter(),
    )?;
    let effects =
        unica_source_contract::proposal::reconcile_effects(staged.planned_changes(), provisional);
    Ok((staged, effects))
}

pub(crate) struct XdtoExecution<T> {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<T>,
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
    validate_xdto_type_name(requested.as_deref())?;
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
    let descriptor = fs::read(&package.descriptor_path)
        .map_err(|_| "target_not_found: cannot read proven XDTO descriptor".to_string())?;
    let model = parse_xdto_info_model(&raw, &descriptor, &package)?;
    let page = if let Some((limit, cursor_key, offset)) = pagination {
        let (start, end, next_cursor) =
            page_bounds_from_offset(offset, &cursor_key, limit, model.types.len())
                .map_err(xdto_cursor_error)?;
        Some((start, end, next_cursor))
    } else {
        None
    };
    let data = build_xdto_info(&package, &model, requested.as_deref(), page)
        .map_err(|error| error.to_string())?;
    Ok(xdto_info_execution(&package, data))
}

fn xdto_info_execution(package: &Package, data: XdtoInfoData) -> XdtoExecution<XdtoInfoData> {
    XdtoExecution {
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
    }
}

impl XdtoIdentity for Package {
    fn source_set(&self) -> &str {
        &self.source_set
    }

    fn metadata_path(&self) -> &str {
        &self.metadata_path
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
        let image = plan_xdto_edit_operations(&package, &before, text, &before_model, args)?;
        Ok(MutationPlan {
            package,
            before,
            after: image.after,
            data: image.data,
            no_op: image.no_op,
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

fn descriptor_namespace(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|_| "target_not_found: cannot read proven XDTO descriptor".to_string())?;
    descriptor_fields(&bytes).map(|fields| fields.namespace)
}

#[cfg(test)]
pub(crate) fn plan_xdto_batch_owned_for_test(
    mut staged: unica_workspace::apply::ApplyStagedState,
    authority: &unica_workspace::actor::XdtoApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<XdtoPlanOperation>],
    validation_ledger: &mut XdtoValidationLedger,
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        Vec<ProvisionalApplyEffect>,
    ),
    ApplyPlanError,
> {
    let effects = plan_xdto_batch(
        &mut staged.proposal(),
        authority,
        operations,
        validation_ledger,
    )?;
    Ok((staged, effects))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        apply_with_data, decode, info_with_data, model, parse_xdto_info_bytes,
        parse_xdto_plan_operation, plan_unindexed_xdto_batch, plan_xdto_batch_owned_for_test,
        preview_with_data, staged_writer_error, staged_writer_operation,
        validate_final_xdto_resources, writer, IndexedPlanOperation, PrefixedXmlQName,
        XdtoAddObjectTypeArgs, XdtoAddPropertyArgs, XdtoAddValueTypeArgs, XdtoPlanOperation,
        XdtoPropertyPath, XdtoPropertySpec, XdtoRemovePropertyArgs, XdtoRemoveTypeArgs,
        XdtoValidationLedger, XmlNcName,
    };
    use crate::application::SupportGuardRequirement;
    use unica_source_contract::events::DomainEventKind;
    use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
    use unica_xdto_designer::test_support::{args, PACKAGE};

    use crate::infrastructure::native_operations::common::support_guard_violation;
    use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_workspace::single_file_publisher::with_before_commit_hook;

    use crate::infrastructure::source_workspace::WorkspaceActor;
    use serde::Serialize;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use unica_workspace::actor::{ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput};
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn xdto_v12_wrapper_and_shared_core_share_projection_and_reject_wrong_owner() {
        let (context, _, package, descriptor) = xdto_guard_fixture("info-shared-core-parity");
        let request = args(&[
            ("sourceSet", json!("main")),
            ("metadataPath", json!("XDTOPackage.Sample")),
            ("typeName", json!("ЛюбаяСсылка")),
        ]);
        let wrapper = info_with_data(&request, &context).unwrap().data.unwrap();
        let address =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "XDTOPackage.Sample").unwrap();
        let core = parse_xdto_info_bytes(
            &fs::read(&package).unwrap(),
            &fs::read(&descriptor).unwrap(),
            "main",
            &address,
            Some("ЛюбаяСсылка"),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(wrapper).unwrap(),
            serde_json::to_value(core).unwrap()
        );

        fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><XDTOPackage><Properties><Name>Other</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
        )
        .unwrap();
        let error = info_with_data(&request, &context)
            .err()
            .expect("wrong descriptor owner must fail");
        assert!(error.starts_with("target_not_found:"), "{error}");
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
    pub(crate) fn xdto_guard_rejects_descriptor_identity_drift_before_commit() {
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

    const REFERENCED_PACKAGE: &str = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:tns="urn:test" targetNamespace="urn:test">
	<valueType name="Used" base="xs:string"/>
	<objectType name="Consumer"><property name="Value" type="tns:Used"/></objectType>
</package>"#;

    struct StagedXdtoFixture {
        context: WorkspaceContext,
        source: PathBuf,
        package: PathBuf,
        actor: Arc<WorkspaceActor>,
        binding: ProviderRootBinding,
    }

    impl StagedXdtoFixture {
        fn cleanup(self) {
            let _ = fs::remove_dir_all(self.context.workspace_root);
        }
    }

    fn staged_xdto_fixture(name: &str, package_bytes: &[u8]) -> StagedXdtoFixture {
        let (context, _, package, _) = xdto_guard_fixture(&format!("staged-{name}"));
        fs::write(&package, package_bytes).unwrap();
        let source = context.workspace_root.join("src");
        let actor = staged_xdto_actor(&context, &source);
        let binding = actor.bind_provider_root("main", &source).unwrap();
        StagedXdtoFixture {
            context,
            source,
            package,
            actor,
            binding,
        }
    }

    fn staged_xdto_actor(context: &WorkspaceContext, source: &Path) -> Arc<WorkspaceActor> {
        let identity = WorkspaceIdentity::new(
            context,
            [WorkspaceSourceSetInput::new(
                "main",
                source,
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "staged-xdto-test-provider",
        )
        .unwrap();
        Arc::new(
            crate::infrastructure::source_workspace::new_actor(identity, context.clone()).unwrap(),
        )
    }

    fn staged_xdto_admission(
        fixture: &StagedXdtoFixture,
        if_rev: Option<&str>,
        dry_run: bool,
    ) -> unica_workspace::actor::ApplyAdmission {
        fixture
            .actor
            .admit_apply(
                &fixture.binding,
                if_rev,
                dry_run,
                unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                    Duration::from_secs(7),
                ),
                &unica_work_control::cancellation::CancellationToken::new(),
            )
            .unwrap()
    }

    fn plan_admitted_xdto(
        admission: &unica_workspace::actor::ApplyAdmission,
        binding: &ProviderRootBinding,
        operations: &[XdtoPlanOperation],
    ) -> Result<
        (
            unica_workspace::apply::ApplyStagedState,
            unica_source_contract::proposal::PlannedApplyEffects,
        ),
        ApplyPlanError,
    > {
        let staged = admission.staged_state().unwrap();
        let authority = admission.xdto_planning_authority(binding)?;
        plan_unindexed_xdto_batch(staged, authority, operations)
    }

    fn qaddr(value: &str) -> unica_address::address::QualifiedAddress {
        unica_address::address::QualifiedAddress::parse(value).unwrap()
    }

    fn ncname(value: &str) -> XmlNcName {
        XmlNcName::for_test(value)
    }

    fn qname(value: &str) -> PrefixedXmlQName {
        PrefixedXmlQName::for_test(value)
    }

    fn add_object(at: &str, name: &str) -> XdtoPlanOperation {
        XdtoPlanOperation::AddObjectType(XdtoAddObjectTypeArgs {
            at: qaddr(at),
            name: ncname(name),
        })
    }

    fn add_value(at: &str, name: &str, base: &str) -> XdtoPlanOperation {
        XdtoPlanOperation::AddValueType(XdtoAddValueTypeArgs {
            at: qaddr(at),
            name: ncname(name),
            base: qname(base),
        })
    }

    fn add_property(
        at: &str,
        name: &str,
        type_ref: &str,
        min_occurs: Option<u8>,
        property_path: Option<&str>,
    ) -> XdtoPlanOperation {
        XdtoPlanOperation::AddProperty(XdtoAddPropertyArgs {
            at: qaddr(at),
            property: XdtoPropertySpec {
                name: ncname(name),
                definition: unica_xdto::operations::XdtoPropertyDefinition::Named(qname(type_ref)),
                min_occurs,
                max_occurs: None,
            },
            property_path: property_path.map(XdtoPropertyPath::for_test),
        })
    }

    fn remove_type(at: &str) -> XdtoPlanOperation {
        XdtoPlanOperation::RemoveType(XdtoRemoveTypeArgs { at: qaddr(at) })
    }

    fn remove_property(at: &str, name: &str, property_path: Option<&str>) -> XdtoPlanOperation {
        XdtoPlanOperation::RemoveProperty(XdtoRemovePropertyArgs {
            at: qaddr(at),
            name: ncname(name),
            property_path: property_path.map(XdtoPropertyPath::for_test),
        })
    }

    fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        if !root.exists() {
            return Vec::new();
        }
        let mut pending = vec![root.to_path_buf()];
        let mut observed = Vec::new();
        while let Some(path) = pending.pop() {
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                observed.push((relative, None));
                let mut children = fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                children.sort();
                pending.extend(children.into_iter().rev());
            } else {
                observed.push((relative, Some(fs::read(path).unwrap())));
            }
        }
        observed
    }

    /// The typed spec of one `property.add` payload, as the planner decodes it.
    fn decoded_property_spec(property: &Value) -> XdtoPropertySpec {
        let args = json!({"at":"main:XDTOPackage.Sample.Type.T", "values":{"property":property}});
        match unica_xdto::operations::XdtoOperation::AddProperty
            .decode(&args, "ops[0].args", "main")
            .unwrap()
        {
            XdtoPlanOperation::AddProperty(args) => args.property,
            other => panic!("property.add decodes to {other:?}"),
        }
    }

    #[test]
    fn staged_xdto_named_property_preserves_legacy_inline_and_mixed_eol_layouts() {
        let header = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:test">"#;
        for ending in ["\n", "\r\n"] {
            let before = format!("{header}<objectType name=\"T\">\n\t\t<property name=\"Old\" type=\"xs:string\"/>\n\t</objectType></package>{ending}");
            let legacy = writer::plan(
                &before,
                &args(&[
                    ("typeName", json!("T")),
                    ("property", json!({"name":"Added", "type":"xs:string"})),
                ]),
                "add-property",
            )
            .expect("legacy layout is supported");
            assert!(!legacy.blocks());
            let fixture = staged_xdto_fixture("named-legacy-layout", before.as_bytes());
            let operation = add_property(
                "main:XDTOPackage.Sample.Type.T",
                "Added",
                "xs:string",
                None,
                None,
            );
            let admission = staged_xdto_admission(&fixture, None, true);
            let (mut state, _) = plan_admitted_xdto(&admission, &fixture.binding, &[operation])
                .expect("plain properties must retain the legacy layout support");
            assert_eq!(
                state
                    .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
                    .unwrap()
                    .unwrap(),
                legacy.after.as_bytes()
            );
            assert_eq!(fs::read(&fixture.package).unwrap(), before.as_bytes());
            fixture.cleanup();
        }
    }

    #[test]
    fn staged_xdto_anonymous_property_tree_is_atomic_and_repeatable() {
        use super::{model, staged_writer_operation};
        use unica_xdto_designer::parse;
        let fixture = staged_xdto_fixture("anonymous-tree", PACKAGE.as_bytes());
        let request = json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка", "values": {
            "property": {"name":"Строки", "minOccurs":0, "maxOccurs":-1,
                "objectType":{"properties":[
                    {"name":"Основание", "type":"tns:ЛюбаяСсылка", "minOccurs":0, "maxOccurs":1},
                    {"name":"Получатель", "type":"tns:СоставнойЛюбойОбъект", "minOccurs":0, "maxOccurs":1}
                ]}}
        }});
        let operation =
            parse_xdto_plan_operation("property.add", &request, 0, &fixture.binding).unwrap();
        let first = writer::plan_typed(PACKAGE, staged_writer_operation(&operation)).unwrap();
        assert!(!first.blocks());
        let doc = parse(&first.after).unwrap();
        let rows = doc
            .descendants()
            .find(|n| n.attribute("name") == Some("Строки"))
            .unwrap();
        assert_eq!(rows.attribute("lowerBound"), Some("0"));
        assert_eq!(rows.attribute("upperBound"), Some("-1"));
        assert_eq!(rows.attribute("type"), None);
        let definition = rows
            .children()
            .find(|n| n.has_tag_name((model::XDTO_NS, "typeDef")))
            .unwrap();
        assert_eq!(definition.attribute("name"), None);
        assert_eq!(
            definition.attribute((model::XSI_NS, "type")),
            Some("ObjectType")
        );
        assert_eq!(
            definition
                .children()
                .filter(|n| n.is_element())
                .map(|n| n.attribute("name").unwrap())
                .collect::<Vec<_>>(),
            ["Основание", "Получатель"]
        );
        let repeated =
            writer::plan_typed(&first.after, staged_writer_operation(&operation)).unwrap();
        assert_eq!(repeated.after, first.after);
        assert!(repeated.edits.is_empty());
        assert!(!repeated.blocks());
        let admission = staged_xdto_admission(&fixture, None, true);
        assert!(plan_admitted_xdto(&admission, &fixture.binding, &[operation]).is_ok());
        let mut invalid = request.clone();
        invalid["values"]["property"]["objectType"]["properties"][1]["type"] = json!("tns:Missing");
        let invalid =
            parse_xdto_plan_operation("property.add", &invalid, 0, &fixture.binding).unwrap();
        let error = match plan_admitted_xdto(&admission, &fixture.binding, &[invalid]) {
            Err(error) => error,
            Ok(_) => panic!("an unresolved child type is refused"),
        };
        // The refusal names the field that declares the children.
        assert_eq!(error.path(), Some("ops[0].args.values.property.objectType"));
        assert_eq!(fs::read(&fixture.package).unwrap(), PACKAGE.as_bytes());
        fixture.cleanup();
    }

    #[test]
    fn anonymous_property_tree_preserves_prefixes_crlf_and_conflicting_children() {
        use super::model;
        use unica_xdto_designer::parse;
        let before = "<d:package xmlns:d=\"http://v8.1c.ru/8.1/xdto\" xmlns=\"urn:other\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"urn:occupied\" targetNamespace=\"urn:test\">\r\n\t<d:objectType name=\"T\"/>\r\n</d:package>";
        let property = json!({"name":"Rows", "minOccurs":0, "maxOccurs":-1, "objectType":{"properties":[
            {"name":"Nested", "objectType":{"properties":[{"name":"Code","type":"xs:string"}]}},
            {"name":"Other","type":"xs:decimal"}
        ]}});
        let spec = decoded_property_spec(&property);
        let operation = writer::TypedWriterOperation::AddPropertyTree {
            type_name: "T",
            property: &spec,
            property_path: None,
        };
        let result = writer::plan_typed(before, operation).unwrap();
        assert!(result
            .after
            .starts_with(before.split("<d:objectType").next().unwrap()));
        assert!(result.after.ends_with("\r\n</d:package>"));
        assert!(!result.after.replace("\r\n", "").contains('\n'));
        let document = parse(&result.after).unwrap();
        let definitions = document
            .descendants()
            .filter(|n| n.has_tag_name((model::XDTO_NS, "typeDef")))
            .collect::<Vec<_>>();
        assert_eq!(definitions.len(), 2);
        for node in definitions {
            assert_eq!(node.attribute((model::XSI_NS, "type")), Some("ObjectType"));
            assert_eq!(node.lookup_namespace_uri(None), Some(model::XDTO_NS));
            assert_eq!(node.lookup_namespace_uri(Some("xsi")), Some("urn:occupied"));
        }
        let repeated = writer::plan_typed(&result.after, operation).unwrap();
        assert!(repeated.edits.is_empty() && !repeated.blocks());
        for change in ["type", "order", "bounds", "extra"] {
            let mut changed = property.clone();
            let children = changed["objectType"]["properties"].as_array_mut().unwrap();
            match change {
                "type" => children[1]["type"] = json!("xs:string"),
                "order" => children.reverse(),
                "bounds" => children[1]["maxOccurs"] = json!(2),
                "extra" => children.push(json!({"name":"Extra","type":"xs:string"})),
                _ => unreachable!(),
            }
            let spec = decoded_property_spec(&changed);
            let refusal = writer::plan_typed(
                &result.after,
                writer::TypedWriterOperation::AddPropertyTree {
                    type_name: "T",
                    property: &spec,
                    property_path: None,
                },
            )
            .unwrap();
            assert!(refusal.blocks(), "{change}");
            assert_eq!(refusal.after, result.after);
        }
    }

    #[test]
    fn anonymous_discriminator_binding_never_resolves_requested_unbound_prefixes() {
        for (label, prefix, declaration) in [
            ("direct", "xsi", ""),
            ("nested", "xdtoXsi1", r#" xmlns:xsi="urn:occupied""#),
        ] {
            let before = format!(
                r#"<package xmlns="http://v8.1c.ru/8.1/xdto"{declaration} targetNamespace="urn:test">
    <import namespace="http://www.w3.org/2001/XMLSchema-instance"/>
    <objectType name="T"/>
</package>"#
            );
            let fixture = staged_xdto_fixture(
                &format!("anonymous-prefix-capture-{label}"),
                before.as_bytes(),
            );
            let mut child = json!({"name":"Value","type":format!("{prefix}:Remote")});
            if label == "nested" {
                child = json!({"name":"Nested","objectType":{"properties":[child]}});
            }
            let request = json!({"at":"main:XDTOPackage.Sample.Type.T", "values":{"property":{
                "name":"Rows", "objectType":{"properties":[child]}
            }}});
            let operation =
                parse_xdto_plan_operation("property.add", &request, 0, &fixture.binding).unwrap();
            let admission = staged_xdto_admission(&fixture, None, true);
            let before_tree = snapshot_tree(&fixture.source);
            let error = match plan_admitted_xdto(&admission, &fixture.binding, &[operation]) {
                Err(error) => error,
                Ok(_) => panic!("the sole undeclared requested prefix must remain invalid"),
            };
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
            assert_eq!(snapshot_tree(&fixture.source), before_tree);
            fixture.cleanup();
        }
    }

    #[test]
    fn staged_xdto_add_type_then_property_reads_prior_postimage() {
        let fixture = staged_xdto_fixture("compose", PACKAGE.as_bytes());
        let operations = [
            add_object("main:XDTOPackage.Sample", "Order"),
            add_property(
                "main:XDTOPackage.Sample.Type.Order",
                "Ref",
                "tns:ЛюбаяСсылка",
                Some(0),
                None,
            ),
        ];
        let legacy_first = writer::plan(
            PACKAGE,
            &args(&[("name", json!("Order"))]),
            "add-object-type",
        )
        .unwrap();
        let legacy_second = writer::plan(
            &legacy_first.after,
            &args(&[
                ("typeName", json!("Order")),
                (
                    "property",
                    json!({"name":"Ref", "type":"tns:ЛюбаяСсылка", "minOccurs":0}),
                ),
            ]),
            "add-property",
        )
        .unwrap();
        let admission = staged_xdto_admission(&fixture, None, true);

        let (mut staged, effects) =
            plan_admitted_xdto(&admission, &fixture.binding, &operations).unwrap();

        assert_eq!(
            staged
                .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
                .unwrap()
                .unwrap(),
            legacy_second.after.as_bytes()
        );
        assert_eq!(effects.events().len(), 1);
        assert_eq!(effects.events()[0].kind, DomainEventKind::MetadataChanged);
        assert_eq!(effects.events()[0].artifact, "main:XDTOPackage.Sample");
        assert_eq!(fs::read(&fixture.package).unwrap(), PACKAGE.as_bytes());
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_poisoned_second_op_publishes_nothing() {
        let referenced = REFERENCED_PACKAGE;
        for (name, before, poison) in [
            (
                "missing",
                PACKAGE.as_bytes(),
                remove_type("main:XDTOPackage.Sample.Type.Missing"),
            ),
            (
                "referenced",
                referenced.as_bytes(),
                remove_type("main:XDTOPackage.Sample.Type.Used"),
            ),
        ] {
            for dry_run in [true, false] {
                let fixture = staged_xdto_fixture(&format!("poison-{name}-{dry_run}"), before);
                let source_before = snapshot_tree(&fixture.source);
                let cache_before = snapshot_tree(&fixture.context.cache_root);
                let admission = staged_xdto_admission(&fixture, None, dry_run);
                let result = plan_admitted_xdto(
                    &admission,
                    &fixture.binding,
                    &[
                        add_object("main:XDTOPackage.Sample", "Added"),
                        poison.clone(),
                    ],
                );
                let error = result.expect_err("poisoned second operation was accepted");
                assert!(error.path().is_some_and(|path| path.starts_with("ops[1]")));
                assert_eq!(snapshot_tree(&fixture.source), source_before);
                assert_eq!(snapshot_tree(&fixture.context.cache_root), cache_before);
                assert!(!fixture.context.cache_root.join("state.json").exists());
                fixture.cleanup();
            }
        }
    }

    #[test]
    fn staged_xdto_dry_and_real_share_postimage_effects_and_revision() {
        let fixture = staged_xdto_fixture("dry-real-plan", PACKAGE.as_bytes());
        let operation = [add_value("main:XDTOPackage.Sample", "Added", "xs:string")];
        let dry_admission = staged_xdto_admission(&fixture, None, true);
        let (mut dry_state, dry_effects) =
            plan_admitted_xdto(&dry_admission, &fixture.binding, &operation).unwrap();
        let dry_bytes = dry_state
            .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
            .unwrap()
            .unwrap();
        let dry_result = fixture
            .actor
            .publish_prepared_apply(
                dry_admission
                    .prepare_with_effects(dry_state, dry_effects)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(dry_result.commit_count_for_test(), 0);
        assert_eq!(fs::read(&fixture.package).unwrap(), PACKAGE.as_bytes());

        let reconstructed = staged_xdto_actor(&fixture.context, &fixture.source);
        let binding = reconstructed
            .bind_provider_root("main", &fixture.source)
            .unwrap();
        let real_admission = reconstructed
            .admit_apply(
                &binding,
                Some(dry_result.rev()),
                false,
                unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                    Duration::from_secs(7),
                ),
                &unica_work_control::cancellation::CancellationToken::new(),
            )
            .unwrap();
        let (mut real_state, real_effects) =
            plan_admitted_xdto(&real_admission, &binding, &operation).unwrap();
        assert_eq!(
            real_state
                .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
                .unwrap()
                .unwrap(),
            dry_bytes
        );
        let real_result = reconstructed
            .publish_prepared_apply(
                real_admission
                    .prepare_with_effects(real_state, real_effects)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(real_result.commit_count_for_test(), 1);
        assert_eq!(real_result.rev(), dry_result.rev());
        assert_eq!(fs::read(&fixture.package).unwrap(), dry_bytes);
        assert_eq!(real_result.effects().events().len(), 1);
        assert_eq!(
            dry_result.effects().events(),
            real_result.effects().events()
        );

        let next = staged_xdto_admission(&fixture, Some(real_result.rev()), true);
        let (state, effects) = plan_admitted_xdto(
            &next,
            &fixture.binding,
            &[add_value("main:XDTOPackage.Sample", "Another", "xs:string")],
        )
        .unwrap();
        let error = next.prepare_with_effects(state, effects).unwrap_err();
        assert_eq!(
            error.kind(),
            unica_source_contract::proposal::ApplyStagingErrorKind::ConcurrentRevision
        );
        assert_eq!(fs::read(&fixture.package).unwrap(), dry_bytes);
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_reuses_actor_authority_and_race_fences() {
        let fixture = staged_xdto_fixture("authority", PACKAGE.as_bytes());
        let operation = [add_object("main:XDTOPackage.Sample", "Added")];
        let admitted = staged_xdto_admission(&fixture, None, true);
        let (state, effects) = plan_admitted_xdto(&admitted, &fixture.binding, &operation).unwrap();
        let preview = fixture
            .actor
            .publish_prepared_apply(admitted.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        let exact = staged_xdto_admission(&fixture, Some(preview.rev()), true);
        let (state, effects) = plan_admitted_xdto(&exact, &fixture.binding, &operation).unwrap();
        let repeated = fixture
            .actor
            .publish_prepared_apply(exact.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        assert_eq!(repeated.rev(), preview.rev());
        for stale in ["stale", "not-a-revision"] {
            let admission = staged_xdto_admission(&fixture, Some(stale), true);
            let (state, effects) =
                plan_admitted_xdto(&admission, &fixture.binding, &operation).unwrap();
            let error = admission.prepare_with_effects(state, effects).unwrap_err();
            assert_eq!(
                error.kind(),
                unica_source_contract::proposal::ApplyStagingErrorKind::ConcurrentRevision
            );
            assert!(error.to_string().contains("stale"));
        }
        let cancelled = unica_work_control::cancellation::CancellationToken::new();
        cancelled.cancel();
        assert!(fixture
            .actor
            .admit_apply(
                &fixture.binding,
                None,
                true,
                unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                    Duration::from_secs(7),
                ),
                &cancelled,
            )
            .unwrap_err()
            .to_string()
            .contains("cancel"));
        assert!(fixture
            .actor
            .admit_apply(
                &fixture.binding,
                None,
                true,
                unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                    Duration::ZERO
                ),
                &unica_work_control::cancellation::CancellationToken::new(),
            )
            .unwrap_err()
            .to_string()
            .contains("deadline"));

        let foreign_actor = staged_xdto_actor(&fixture.context, &fixture.source);
        let foreign_binding = foreign_actor
            .bind_provider_root("main", &fixture.source)
            .unwrap();
        let first = staged_xdto_admission(&fixture, None, true);
        let foreign_error = first
            .xdto_planning_authority(&foreign_binding)
            .err()
            .expect("foreign actor binding was accepted");
        assert_eq!(foreign_error.kind(), ApplyPlanErrorKind::InvalidState);
        let second = staged_xdto_admission(&fixture, None, true);
        let authority = first.xdto_planning_authority(&fixture.binding).unwrap();
        let error = plan_unindexed_xdto_batch(
            second.staged_state().unwrap(),
            authority,
            &[add_object("main:XDTOPackage.Sample", "Added")],
        )
        .expect_err("foreign writer state was accepted");
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_args_are_closed_typed_and_report_exact_paths() {
        let fixture = staged_xdto_fixture("args", PACKAGE.as_bytes());
        let mut failures = Vec::new();
        for (operation, value) in [
            (
                "valueType.add",
                json!({"at":"main:XDTOPackage.Sample", "values":{"name":"Added", "base":"xs:string"}}),
            ),
            (
                "objectType.add",
                json!({"at":"main:XDTOPackage.Sample", "values":{"name":"Added"}}),
            ),
            (
                "property.add",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка.Property.СсылкаНаОбъект", "values":{"property":{"name":"Added", "type":"xs:string", "minOccurs":0}}}),
            ),
            (
                "type.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.СоставнойЛюбойОбъект"}),
            ),
            (
                "property.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка.Property.СсылкаНаОбъект"}),
            ),
        ] {
            parse_xdto_plan_operation(operation, &value, 0, &fixture.binding)
                .unwrap_or_else(|error| panic!("valid {operation} rejected: {error:?}"));
        }

        for (literal, expected) in [
            (None, None),
            (Some("0"), Some(0)),
            (Some("1"), Some(1)),
            (Some("0.0"), Some(0)),
            (Some("0e0"), Some(0)),
            (Some("1.0"), Some(1)),
            (Some("1e0"), Some(1)),
        ] {
            let mut value = json!({
                "at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка",
                "values":{"property":{"name":"Added", "type":"xs:string"}}
            });
            if let Some(literal) = literal {
                value["values"]["property"]["minOccurs"] = serde_json::from_str(literal).unwrap();
            }
            match parse_xdto_plan_operation("property.add", &value, 2, &fixture.binding) {
                Ok(XdtoPlanOperation::AddProperty(args)) => {
                    if args.property.min_occurs != expected {
                        failures.push(format!(
                            "minOccurs {literal:?}: expected {expected:?}, got {:?}",
                            args.property.min_occurs
                        ));
                    }
                }
                result => failures.push(format!(
                    "mathematical integer minOccurs {literal:?} rejected: {result:?}"
                )),
            }
        }

        // Flat add payloads are not a second grammar beside `values`.
        for (operation, value) in [
            (
                "valueType.add",
                json!({"at":"main:XDTOPackage.Sample", "name":"Added", "base":"xs:string"}),
            ),
            (
                "objectType.add",
                json!({"at":"main:XDTOPackage.Sample", "name":"Added"}),
            ),
            (
                "property.add",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка", "property":{"name":"Added", "type":"xs:string"}}),
            ),
        ] {
            match parse_xdto_plan_operation(operation, &value, 2, &fixture.binding) {
                Err(error) if error.kind() == ApplyPlanErrorKind::BadValue => {}
                result => failures.push(format!(
                    "flat {operation} must refuse without an alias: {result:?}"
                )),
            }
        }
        let invalid = [
            ("valueType.add", json!(null), "ops[2].args"),
            (
                "valueType.add",
                json!({"values":{"name":"A", "base":"xs:string"}}),
                "ops[2].args.at",
            ),
            (
                "valueType.add",
                json!({"at":"main:XDTOPackage.Sample", "values":{"name":"bad:name", "base":"xs:string"}}),
                "ops[2].args.values.name",
            ),
            (
                "valueType.add",
                json!({"at":"main:XDTOPackage.Sample", "values":{"name":"A", "base":"xs::string"}}),
                "ops[2].args.values.base",
            ),
            (
                "objectType.add",
                json!({"at":"main:XDTOPackage.Sample", "values":{"name":"A"}, "sourceSet":"main"}),
                "ops[2].args.sourceSet",
            ),
            (
                "property.add",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка", "values":{"property":{"name":"A", "type":"xs:string", "minOccurs":2}}}),
                "ops[2].args.values.property.minOccurs",
            ),
            (
                "property.add",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка", "values":{"property":{"name":"A", "type":"xs:string", "legacy":1}}}),
                "ops[2].args.values.property.legacy",
            ),
            (
                "valueType.add",
                json!({"at":"main:XDTOPackage.Sample", "values":{"name":"A", "base":"xs:string", "at":false}}),
                "ops[2].args.values.at",
            ),
            (
                "type.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка", "values":{}}),
                "ops[2].args.values",
            ),
            (
                "property.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка.Property.A", "values":{}}),
                "ops[2].args.values",
            ),
            (
                "property.remove",
                json!({"at":"foreign:XDTOPackage.Sample.Type.ЛюбаяСсылка.Property.A"}),
                "ops[2].args.at",
            ),
            // Удаляемое свойство и путь к нему задаёт адрес, а не аргументы.
            (
                "property.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка", "name":"A"}),
                "ops[2].args.name",
            ),
            (
                "property.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка"}),
                "ops[2].args.at",
            ),
            (
                "property.add",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка", "values":{"property":{"name":"A", "type":"xs:string"}}, "propertyPath":"СсылкаНаОбъект"}),
                "ops[2].args.propertyPath",
            ),
            (
                "type.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка.Property.A"}),
                "ops[2].args.at",
            ),
        ];
        for (operation, value, path) in invalid {
            match parse_xdto_plan_operation(operation, &value, 2, &fixture.binding) {
                Err(error)
                    if error.kind() == ApplyPlanErrorKind::BadValue
                        && error.path() == Some(path) => {}
                result => failures.push(format!(
                    "{operation} {value}: expected bad_value at {path}, got {result:?}"
                )),
            }
        }
        for literal in [
            "-1", "2", "0.25", "0.5", "1.5", "1.0001", "null", "true", "\"1\"",
        ] {
            let mut value = json!({
                "at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка",
                "values":{"property":{"name":"Added", "type":"xs:string"}}
            });
            value["values"]["property"]["minOccurs"] = serde_json::from_str(literal).unwrap();
            match parse_xdto_plan_operation("property.add", &value, 2, &fixture.binding) {
                Err(error)
                    if error.kind() == ApplyPlanErrorKind::BadValue
                        && error.path() == Some("ops[2].args.values.property.minOccurs") => {}
                result => failures.push(format!(
                    "minOccurs {literal} must refuse without rounding at its caller path: {result:?}"
                )),
            }
        }
        fixture.cleanup();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn staged_xdto_preserves_owner_namespace_format_support_and_identity_guards() {
        for (name, mutate, expected) in [
            ("unregistered", 0usize, ApplyPlanErrorKind::NotFound),
            ("descriptor-name", 1usize, ApplyPlanErrorKind::InvalidSource),
            (
                "descriptor-version",
                2usize,
                ApplyPlanErrorKind::InvalidSource,
            ),
            ("namespace", 3usize, ApplyPlanErrorKind::InvalidSource),
            ("resource-missing", 4usize, ApplyPlanErrorKind::NotFound),
            ("support-deny", 5usize, ApplyPlanErrorKind::InvalidState),
        ] {
            let fixture = staged_xdto_fixture(name, PACKAGE.as_bytes());
            match mutate {
                0 => fs::write(
                    fixture.source.join("Configuration.xml"),
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
                )
                .unwrap(),
                1 => fs::write(
                    fixture.source.join("XDTOPackages/Sample.xml"),
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Other</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
                )
                .unwrap(),
                2 => fs::write(
                    fixture.source.join("XDTOPackages/Sample.xml"),
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><XDTOPackage uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Sample</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
                )
                .unwrap(),
                3 => fs::write(
                    fixture.source.join("XDTOPackages/Sample.xml"),
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Sample</Name><Namespace>urn:other</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
                )
                .unwrap(),
                4 => fs::remove_file(&fixture.package).unwrap(),
                5 => fs::write(
                    fixture.source.join("Ext/ParentConfigurations.bin"),
                    b"{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd}",
                )
                .unwrap(),
                _ => unreachable!(),
            }
            let admission = staged_xdto_admission(&fixture, None, true);
            let error = plan_admitted_xdto(
                &admission,
                &fixture.binding,
                &[add_object("main:XDTOPackage.Sample", "Added")],
            )
            .expect_err("invalid owner/resource/support evidence was accepted");
            assert_eq!(error.kind(), expected, "{name}: {error:?}");
            assert!(!format!("{error:?}").contains(fixture.source.to_string_lossy().as_ref()));
            fixture.cleanup();
        }
    }

    #[test]
    pub(crate) fn staged_xdto_v12_parity_preserves_exact_bytes_errors_and_noop() {
        let before = r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:tns="urn:test" targetNamespace="urn:test">
	<valueType name="Used" base="xs:string"/>
	<objectType name="Order">
		<property name="Existing" type="xs:string"/>
	</objectType>
</package>"#;
        let cases = vec![
            (
                "add-value",
                add_value("main:XDTOPackage.Sample", "AddedValue", "xs:string"),
                "add-value-type",
                args(&[("name", json!("AddedValue")), ("base", json!("xs:string"))]),
            ),
            (
                "add-object",
                add_object("main:XDTOPackage.Sample", "AddedObject"),
                "add-object-type",
                args(&[("name", json!("AddedObject"))]),
            ),
            (
                "add-property",
                add_property(
                    "main:XDTOPackage.Sample.Type.Order",
                    "Added",
                    "xs:string",
                    Some(0),
                    None,
                ),
                "add-property",
                args(&[
                    ("typeName", json!("Order")),
                    (
                        "property",
                        json!({"name":"Added", "type":"xs:string", "minOccurs":0}),
                    ),
                ]),
            ),
            (
                "remove-type",
                remove_type("main:XDTOPackage.Sample.Type.Used"),
                "remove-type",
                args(&[("name", json!("Used"))]),
            ),
            (
                "remove-property",
                remove_property("main:XDTOPackage.Sample.Type.Order", "Existing", None),
                "remove-property",
                args(&[("typeName", json!("Order")), ("name", json!("Existing"))]),
            ),
        ];
        for (name, operation, legacy_key, legacy_args) in cases {
            let fixture = staged_xdto_fixture(name, before.as_bytes());
            let expected = writer::plan(before, &legacy_args, legacy_key)
                .unwrap()
                .after;
            let admission = staged_xdto_admission(&fixture, None, true);
            let (mut state, _) = plan_admitted_xdto(
                &admission,
                &fixture.binding,
                std::slice::from_ref(&operation),
            )
            .unwrap();
            assert_eq!(
                state
                    .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
                    .unwrap()
                    .unwrap(),
                expected.as_bytes(),
                "{name}"
            );
            fixture.cleanup();
        }
        let bom_crlf = format!("\u{feff}{}", before.replace('\n', "\r\n"));
        let fixture = staged_xdto_fixture("bom-crlf", bom_crlf.as_bytes());
        let operation = add_object("main:XDTOPackage.Sample", "AddedObject");
        let legacy = writer::plan(
            bom_crlf.trim_start_matches('\u{feff}'),
            &args(&[("name", json!("AddedObject"))]),
            "add-object-type",
        )
        .unwrap();
        let mut expected = vec![0xef, 0xbb, 0xbf];
        expected.extend_from_slice(legacy.after.as_bytes());
        let admission = staged_xdto_admission(&fixture, None, true);
        let (mut state, _) =
            plan_admitted_xdto(&admission, &fixture.binding, &[operation]).unwrap();
        assert_eq!(
            state
                .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
                .unwrap()
                .unwrap(),
            expected
        );
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_effects_follow_final_package_postimages() {
        let fixture = staged_xdto_fixture("effects-dedup", PACKAGE.as_bytes());
        let admission = staged_xdto_admission(&fixture, None, true);
        let (state, effects) = plan_admitted_xdto(
            &admission,
            &fixture.binding,
            &[
                add_object("main:XDTOPackage.Sample", "Added"),
                add_object("main:XDTOPackage.Sample", "Added"),
            ],
        )
        .unwrap();
        assert_eq!(state.planned_changes().len(), 1);
        assert_eq!(effects.events().len(), 1);
        fixture.cleanup();

        let restored = staged_xdto_fixture("effects-restored", PACKAGE.as_bytes());
        let admission = staged_xdto_admission(&restored, None, true);
        let (state, effects) = plan_admitted_xdto(
            &admission,
            &restored.binding,
            &[
                add_object("main:XDTOPackage.Sample", "Transient"),
                remove_type("main:XDTOPackage.Sample.Type.Transient"),
            ],
        )
        .unwrap();
        assert!(state.planned_changes().is_empty());
        assert!(effects.events().is_empty());
        restored.cleanup();

        let cross = staged_xdto_fixture("effects-cross", PACKAGE.as_bytes());
        fs::write(
            cross.source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><XDTOPackage>Sample</XDTOPackage><XDTOPackage>Other</XDTOPackage></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::create_dir_all(cross.source.join("XDTOPackages/Other/Ext")).unwrap();
        fs::write(
            cross.source.join("XDTOPackages/Other.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><XDTOPackage uuid="cccccccc-cccc-cccc-cccc-cccccccccccc"><Properties><Name>Other</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            cross.source.join("XDTOPackages/Other/Ext/Package.bin"),
            PACKAGE,
        )
        .unwrap();
        let admission = staged_xdto_admission(&cross, None, true);
        let error = plan_admitted_xdto(
            &admission,
            &cross.binding,
            &[
                add_object("main:XDTOPackage.Sample", "One"),
                add_object("main:XDTOPackage.Other", "Two"),
            ],
        )
        .expect_err("cross-package XDTO batch was accepted");
        assert_eq!(error.path(), Some("ops[1].args.at"));
        cross.cleanup();
    }

    /// Вложенное свойство задаётся адресом: цепочка `Property` после типа
    /// становится путём писателя, точка в имени (`\.` в адресе) остаётся
    /// точкой имени, а удаляемое свойство — последним сегментом. Писатель
    /// получает тот же путь, что прежде давал `propertyPath`.
    #[test]
    fn staged_xdto_property_chain_comes_from_the_address() {
        let fixture = staged_xdto_fixture("chain", PACKAGE.as_bytes());
        let before = "<package xmlns=\"http://v8.1c.ru/8.1/xdto\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" targetNamespace=\"urn:test\">\n\t<objectType name=\"Target\">\n\t\t<property name=\"A.B\">\n\t\t\t<typeDef xsi:type=\"ObjectType\">\n\t\t\t\t<property name=\"Child\">\n\t\t\t\t\t<typeDef xsi:type=\"ObjectType\">\n\t\t\t\t\t\t<property name=\"Leaf\" type=\"xs:string\"/>\n\t\t\t\t\t</typeDef>\n\t\t\t\t</property>\n\t\t\t</typeDef>\n\t\t</property>\n\t</objectType>\n</package>";
        let added = parse_xdto_plan_operation(
            "property.add",
            &json!({"at": r"main:XDTOPackage.Sample.Type.Target.Property.A\.B.Property.Child", "values": {"property": {"name": "Added", "type": "xs:string"}}}),
            0,
            &fixture.binding,
        )
        .unwrap();
        let XdtoPlanOperation::AddProperty(args) = &added else {
            panic!("property.add parses into an addition");
        };
        assert_eq!(args.at.to_string(), "main:XDTOPackage.Sample.Type.Target");
        assert_eq!(
            args.property_path.as_ref().map(XdtoPropertyPath::as_str),
            Some(r"A\.B.Child")
        );
        let planned = writer::plan_typed(before, staged_writer_operation(&added)).unwrap();
        let model = model::PackageModel::parse(&planned.after).unwrap();
        let child = &model.types[0].properties[0].type_defs[0].properties[0];
        assert_eq!(
            child.type_defs[0]
                .properties
                .iter()
                .map(|property| property.logical_identity())
                .collect::<Vec<_>>(),
            vec!["Leaf", "Added"]
        );

        let removed = parse_xdto_plan_operation(
            "property.remove",
            &json!({"at": r"main:XDTOPackage.Sample.Type.Target.Property.A\.B.Property.Child.Property.Leaf"}),
            0,
            &fixture.binding,
        )
        .unwrap();
        let XdtoPlanOperation::RemoveProperty(args) = &removed else {
            panic!("property.remove parses into a removal");
        };
        assert_eq!(args.name.as_str(), "Leaf");
        assert_eq!(
            args.property_path.as_ref().map(XdtoPropertyPath::as_str),
            Some(r"A\.B.Child")
        );
        let planned = writer::plan_typed(before, staged_writer_operation(&removed)).unwrap();
        assert!(!planned.after.contains("Leaf"));
        assert!(planned.after.contains("name=\"Child\""));

        // Свойство верхнего уровня: пути нет, удаляется член самого типа.
        let top = parse_xdto_plan_operation(
            "property.remove",
            &json!({"at": r"main:XDTOPackage.Sample.Type.Target.Property.A\.B"}),
            0,
            &fixture.binding,
        )
        .unwrap();
        let XdtoPlanOperation::RemoveProperty(args) = &top else {
            panic!("property.remove parses into a removal");
        };
        assert_eq!(
            (args.name.as_str(), args.property_path.is_none()),
            ("A.B", true)
        );

        // Свойство без вложенного типа — узел есть, но свойства в него не
        // кладутся: отказ называет адрес и причину, а не общий отказ писателя.
        let leaf = parse_xdto_plan_operation(
            "property.add",
            &json!({"at": r"main:XDTOPackage.Sample.Type.Target.Property.A\.B.Property.Child.Property.Leaf", "values": {"property": {"name": "X", "type": "xs:string"}}}),
            0,
            &fixture.binding,
        )
        .unwrap();
        let error = writer::plan_typed(before, staged_writer_operation(&leaf)).unwrap_err();
        let refused = staged_writer_error(error, &leaf, 0);
        assert_eq!(refused.path(), Some("ops[0].args.at"));
        assert!(refused.to_string().contains("nested anonymous type"));
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_package_mapping_is_logical_and_single_resource() {
        let fixture = staged_xdto_fixture("mapping", PACKAGE.as_bytes());
        for (operation, value) in [
            (
                "valueType.add",
                json!({"at":"main:XDTOPackage.Sample", "values":{"name":"Value", "base":"xs:string"}}),
            ),
            (
                "objectType.add",
                json!({"at":"main:XDTOPackage.Sample", "values":{"name":"Object"}}),
            ),
            (
                "property.add",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка", "values":{"property":{"name":"Added", "type":"xs:string"}}}),
            ),
            (
                "type.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.СоставнойЛюбойОбъект"}),
            ),
            (
                "property.remove",
                json!({"at":"main:XDTOPackage.Sample.Type.ЛюбаяСсылка.Property.СсылкаНаОбъект"}),
            ),
        ] {
            let parsed = parse_xdto_plan_operation(operation, &value, 0, &fixture.binding).unwrap();
            let admission = staged_xdto_admission(&fixture, None, true);
            let (state, effects) =
                plan_admitted_xdto(&admission, &fixture.binding, &[parsed]).unwrap();
            assert!(state
                .planned_changes()
                .iter()
                .all(|change| change.relative_path
                    == Path::new("XDTOPackages/Sample/Ext/Package.bin")));
            assert!(effects
                .events()
                .iter()
                .all(|event| event.artifact == "main:XDTOPackage.Sample"));
        }
        for raw in [
            "main:XDTOPackage.Sample.Namespace.urn",
            "main:XDTOPackage.Sample.Property.Value",
            "main:XDTOPackage.Sample.Type.Order.Property.Value",
            "foreign:XDTOPackage.Sample",
            "main:Type.Sample",
        ] {
            let error = parse_xdto_plan_operation(
                "objectType.add",
                &json!({"at":raw, "values":{"name":"Added"}}),
                3,
                &fixture.binding,
            )
            .expect_err("invalid logical alias selected Package.bin");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{raw}");
            assert_eq!(error.path(), Some("ops[3].args.at"), "{raw}");
        }
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_final_validation_keeps_original_baseline_and_origin_across_runs() {
        let fixture = staged_xdto_fixture("final-origin", PACKAGE.as_bytes());
        let admission = staged_xdto_admission(&fixture, None, true);
        let authority = admission.xdto_planning_authority(&fixture.binding).unwrap();
        let operations = [
            IndexedPlanOperation::new(
                4,
                add_value("main:XDTOPackage.Sample", "Bad", "tns:Missing"),
            ),
            IndexedPlanOperation::new(9, add_object("main:XDTOPackage.Sample", "Unrelated")),
        ];
        let mut ledger = XdtoValidationLedger::default();
        let (state, first_effects) = plan_xdto_batch_owned_for_test(
            admission.staged_state().unwrap(),
            &authority,
            &operations[..1],
            &mut ledger,
        )
        .expect("a later operation may complete a forward reference");
        assert_eq!(first_effects.len(), 1);
        let (mut state, _) =
            plan_xdto_batch_owned_for_test(state, &authority, &operations[1..], &mut ledger)
                .unwrap();
        let error = validate_final_xdto_resources(
            &mut state.proposal(),
            &authority,
            &ledger,
            operations.iter(),
        )
        .expect_err("a subsequent run must not adopt an introduced finding as its baseline");
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert_eq!(error.path(), Some("ops[4].args.values.base"));
        assert_eq!(fs::read(&fixture.package).unwrap(), PACKAGE.as_bytes());
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_final_validation_preserves_original_findings() {
        let original = REFERENCED_PACKAGE.replace("tns:Used", "tns:Missing");
        let fixture = staged_xdto_fixture("final-preexisting", original.as_bytes());
        let admission = staged_xdto_admission(&fixture, None, true);
        let (mut state, effects) = plan_admitted_xdto(
            &admission,
            &fixture.binding,
            &[add_object("main:XDTOPackage.Sample", "Unrelated")],
        )
        .expect("an unchanged finding in the retained original package is not introduced");
        let after = state
            .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
            .unwrap()
            .unwrap();
        let text = decode(&after).unwrap();
        let model = model::PackageModel::parse(&text).unwrap();
        let findings = super::validate(&model);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "unknown_type_reference");
        assert_eq!(effects.events().len(), 1);
        assert_eq!(fs::read(&fixture.package).unwrap(), original.as_bytes());
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_final_validation_allows_removing_the_reference_later() {
        let fixture = staged_xdto_fixture("final-remove-reference", REFERENCED_PACKAGE.as_bytes());
        let admission = staged_xdto_admission(&fixture, None, true);
        let (mut state, effects) = plan_admitted_xdto(
            &admission,
            &fixture.binding,
            &[
                remove_type("main:XDTOPackage.Sample.Type.Used"),
                remove_property("main:XDTOPackage.Sample.Type.Consumer", "Value", None),
            ],
        )
        .expect("the final package no longer references the removed type");
        let after = state
            .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
            .unwrap()
            .unwrap();
        let text = decode(&after).unwrap();
        let xml = roxmltree::Document::parse(&text).unwrap();
        assert!(!xml
            .descendants()
            .any(|node| node.is_element() && node.attribute("name") == Some("Used")));
        assert!(!xml
            .descendants()
            .any(|node| node.has_tag_name((unica_xdto_designer::model::XDTO_NS, "property"))));
        assert!(super::validate(&model::PackageModel::parse(&text).unwrap()).is_empty());
        assert_eq!(effects.events().len(), 1);
        assert_eq!(
            fs::read(&fixture.package).unwrap(),
            REFERENCED_PACKAGE.as_bytes()
        );
        fixture.cleanup();
    }

    #[test]
    fn staged_xdto_original_preimage_restoration_emits_no_effect() {
        let fixture = staged_xdto_fixture("restoration", PACKAGE.as_bytes());
        let admission = staged_xdto_admission(&fixture, None, true);
        let (mut state, effects) = plan_admitted_xdto(
            &admission,
            &fixture.binding,
            &[
                add_object("main:XDTOPackage.Sample", "Transient"),
                remove_type("main:XDTOPackage.Sample.Type.Transient"),
            ],
        )
        .unwrap();
        assert_eq!(
            state
                .read(Path::new("XDTOPackages/Sample/Ext/Package.bin"))
                .unwrap()
                .unwrap(),
            PACKAGE.as_bytes()
        );
        assert!(state.planned_changes().is_empty());
        assert!(effects.events().is_empty());
        assert_eq!(fs::read(&fixture.package).unwrap(), PACKAGE.as_bytes());
        fixture.cleanup();
    }
}

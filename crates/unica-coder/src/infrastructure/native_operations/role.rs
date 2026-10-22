#![allow(dead_code, unused_imports)]
use unica_roles::info::*;
use unica_roles_designer::info::{
    parse_role_info_xml, role_info_metadata_xml, validate_role_source,
};
use unica_source_designer::layout::role_read_layout;
use unica_source_designer::owner::PlatformXmlResourceEvidence;

use crate::infrastructure::form_composition::*;
use crate::infrastructure::interface_composition::*;
use crate::infrastructure::native_operations::logical_selector::prove_attached_resource;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, ClosedPlatformXmlTarget,
    TargetKindPolicy,
};
use crate::infrastructure::subsystem_composition::*;
use crate::infrastructure::support_guard::{
    bind_resolved_support_guard_evidence, evaluate_resolved_support_guard,
    ResolvedSupportGuardCheck,
};
use crate::infrastructure::support_state::WorkspaceSupportStateReader;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use unica_address::source_target::{
    MetadataAddress, SourceTarget, SourceTargetErrorCode, TargetKind,
    PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_forms::facts::*;
use unica_forms_designer::codec::*;
use unica_metadata::interface::*;
use unica_metadata::subsystem::*;
use unica_metadata::support_state::{
    ObjectSupportData as DomainObjectSupportData, SupportStateReader,
};
use unica_metadata_designer::interface::*;
use unica_metadata_designer::subsystem::*;
use unica_roles::{
    known_role_rights, nested_right_shape_is_modelled, parse_role_edit_request,
    validate_nested_right, RoleEditData, RoleEditEffect, RoleEditEffectAction,
};
use unica_roles_designer::descriptor::validate_role_edit_descriptor;
use unica_roles_designer::writer::{
    apply_role_edit_operation, decode_role_xml, direct_role_children, encode_role_xml,
    right_insertion, role_text_content, validate_role_rights_document, RoleValueScope,
    ROLE_METADATA_NAMESPACE, ROLE_RIGHTS_NAMESPACE, XSI_NAMESPACE,
};
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::roots::{platform_xml_publication_policy, PlatformXmlPublicationPolicy};
use unica_workspace::cache::{CacheAccess, CacheReport};
use unica_workspace::path_policy::WorkspacePathPolicy;
use unica_workspace::paths::metadata_is_link_or_reparse_point;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::state::WorkspaceStateRepository;
use unica_xml_format::text::{
    resolve_line_ending, EolPolicy, LineEndingProfile, SourceTextSnapshot,
};

use super::common::*;
use super::{cf::*, cfe::*, dcs::*, meta::*};
use unica_workspace::compile_transaction::{
    CommitFailure, CommitFailureKind, CompileTransaction, RegistrationStatus,
};

#[cfg(test)]
thread_local! {
    static ROLE_EDIT_BEFORE_NATIVE_GUARD_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static ROLE_EDIT_BEFORE_PUBLISH_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static ROLE_EDIT_AFTER_RIGHTS_REREAD_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    static ROLE_EDIT_POST_VALIDATION_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
fn with_role_edit_before_native_guard_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Box<dyn FnOnce()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_EDIT_BEFORE_NATIVE_GUARD_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        ROLE_EDIT_BEFORE_NATIVE_GUARD_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_role_edit_before_native_guard_hook() {
    ROLE_EDIT_BEFORE_NATIVE_GUARD_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn with_role_edit_before_publish_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Box<dyn FnOnce()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_EDIT_BEFORE_PUBLISH_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = ROLE_EDIT_BEFORE_PUBLISH_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_role_edit_before_publish_hook() {
    ROLE_EDIT_BEFORE_PUBLISH_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn with_role_edit_after_rights_reread_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Box<dyn FnOnce()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_EDIT_AFTER_RIGHTS_REREAD_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        ROLE_EDIT_AFTER_RIGHTS_REREAD_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_role_edit_after_rights_reread_hook() {
    ROLE_EDIT_AFTER_RIGHTS_REREAD_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn with_role_edit_post_validation_failure<T>(action: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            ROLE_EDIT_POST_VALIDATION_FAILURE.with(|slot| slot.set(self.0));
        }
    }
    let previous = ROLE_EDIT_POST_VALIDATION_FAILURE.with(|slot| slot.replace(true));
    let _reset = Reset(previous);
    action()
}

pub(crate) fn role_read_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    operation: &str,
) -> Result<Vec<PathBuf>, String> {
    let rights_path = resolve_role_read_rights_path(args, context)?;
    let layout = role_read_layout(&rights_path);

    let mut paths = vec![rights_path];
    if layout.metadata_path.is_file() {
        paths.push(layout.metadata_path);
    }
    if operation == "role-validate" && layout.configuration_path.is_file() {
        paths.push(layout.configuration_path);
    }
    Ok(paths)
}

pub(crate) struct RoleInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<RoleInfoData>,
}

pub(crate) fn analyze_role_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> RoleInfoExecution {
    let result = (|| -> Result<(RoleInfoData, PathBuf), String> {
        let selection = resolve_role_info_target(args, context)?;
        let rights_path = selection.resource_path;
        if !rights_path.is_file() {
            return Err(format!("[ERROR] File not found: {}", rights_path.display()));
        }

        let layout = role_read_layout(&rights_path);
        let descriptor_text = fs::read_to_string(&layout.metadata_path).ok();
        let rights_text = fs::read_to_string(&rights_path)
            .map_err(|err| format!("failed to read {}: {err}", rights_path.display()))?;
        let data = parse_role_info_xml(
            &rights_text,
            descriptor_text.as_deref(),
            &layout.role_dir_name,
            support_reader
                .object_support(&selection.target)
                .map_err(|error| error.to_string())?,
        )?;
        Ok((data, rights_path))
    })();

    match result {
        Ok((data, rights_path)) => RoleInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.role.info described {} with {} allowed and {} denied right(s)",
                    data.name, data.totals.allowed, data.totals.denied
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![rights_path.display().to_string()],
                stdout: None,
                stderr: Some(String::new()),
                command: None,
            },
            data: Some(data),
        },
        Err(error) => RoleInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.role.info failed in native role analyzer".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

pub(crate) fn role_info_metadata(rights_path: &Path) -> (String, String) {
    let layout = role_read_layout(rights_path);
    let descriptor = fs::read_to_string(layout.metadata_path).ok();
    role_info_metadata_xml(descriptor.as_deref(), &layout.role_dir_name)
}

pub(crate) fn validate_role(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<(bool, String, PathBuf), String> {
        let rights_path = resolve_role_read_rights_path(args, context)?;
        let detailed = unica_operation_contract::input::bool_arg(args, &["detailed", "Detailed"]);

        let reader =
            unica_workspace::discovery::ProjectDiscoveryReader::new(&context.workspace_root);
        validate_role_source(&reader, rights_path, detailed)
    })();

    match result {
        Ok((ok, text, rights_path)) => AdapterOutcome {
            ok,
            summary: if ok {
                "unica.role.validate completed with native role validator".to_string()
            } else {
                "unica.role.validate failed in native role validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: if ok {
                Vec::new()
            } else {
                vec![text.trim().to_string()]
            },
            artifacts: vec![rights_path.display().to_string()],
            stdout: Some(format!("{text}\n")),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.role.validate failed in native role validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        // Typed answer: the registry keeps the prose-shaped signature, and the
        // data reaches the envelope through typed_result.rs.
        "role-info" => Some(Ok(analyze_role_info(
            args,
            context,
            &WorkspaceSupportStateReader::new(context),
        )
        .outcome)),
        "role-validate" => Some(Ok(validate_role(args, context))),
        _ => None,
    }
}

pub(crate) struct RoleEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<RoleEditData>,
    pub(crate) events: Vec<DomainEvent>,
    pub(crate) recorded_cache: Option<CacheReport>,
}

struct RoleEditTarget {
    source_set: String,
    metadata_path: String,
    descriptor_path: PathBuf,
    descriptor_preimage: Vec<u8>,
    registration_path: PathBuf,
    registration_preimage: Vec<u8>,
    rights_path: PathBuf,
    rights_preimage: Vec<u8>,
    support_warning: Option<String>,
    /// Пары «объект, право», которых касался вызов: только их значения
    /// пост-образ обязан иметь булевыми.
    written_rights: Vec<(String, String)>,
}

struct RoleEditPlan {
    target: RoleEditTarget,
    postimage: Vec<u8>,
    data: RoleEditData,
}

struct RoleEditPublication {
    warnings: Vec<String>,
    event: DomainEvent,
    cache: CacheReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleResolutionCode {
    SourceSetUnknown,
    TargetNotFound,
    NotARole,
    ProviderUnavailable,
    ContainmentDenied,
    ProfileUnsupported,
}

impl RoleResolutionCode {
    const fn code(self) -> &'static str {
        match self {
            Self::SourceSetUnknown => "source_set_unknown",
            Self::TargetNotFound => "target_not_found",
            Self::NotARole => "not_a_role",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ContainmentDenied => "containment_denied",
            Self::ProfileUnsupported => "profile_unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoleResolutionFailure {
    code: RoleResolutionCode,
    reason: &'static str,
}

impl RoleResolutionFailure {
    const fn new(code: RoleResolutionCode, reason: &'static str) -> Self {
        Self { code, reason }
    }

    const fn code(self) -> &'static str {
        self.code.code()
    }

    const fn reason(self) -> &'static str {
        self.reason
    }
}

impl std::fmt::Display for RoleResolutionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.reason())
    }
}

fn role_support_warning() -> String {
    "support_guard_warning: the role is protected by support policy; warn mode allows the mutation"
        .to_string()
}

pub(crate) fn resolve_role_edit_guard_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    let request = parse_role_edit_request(args).map_err(|error| error.to_string())?;
    resolve_role_descriptor(&request.source_set, &request.metadata_path, context)
        .map(|(_, evidence)| evidence.target_path)
        .map_err(|failure| failure.to_string())
}

fn resolve_role_descriptor(
    source_set: &str,
    metadata_path: &unica_address::source_target::MetadataAddress,
    context: &WorkspaceContext,
) -> Result<(ClosedPlatformXmlTarget, PlatformXmlResourceEvidence), RoleResolutionFailure> {
    if !metadata_path.as_str().starts_with("Role.") {
        return Err(RoleResolutionFailure::new(
            RoleResolutionCode::NotARole,
            "metadataPath must identify a Role",
        ));
    }
    let target = SourceTarget {
        source_set: source_set.to_string(),
        metadata_path: Some(metadata_path.clone()),
    };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| role_resolution_failure(error.code))?;
    if resolution.resolved.target_kind != TargetKind::MetadataObject {
        return Err(RoleResolutionFailure::new(
            RoleResolutionCode::NotARole,
            "metadataPath must identify a role object",
        ));
    }
    let evidence = platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
        RoleResolutionFailure::new(
            RoleResolutionCode::ProviderUnavailable,
            "the role descriptor evidence is unavailable",
        )
    })?;
    Ok((resolution.handle, evidence))
}

fn role_resolution_failure(code: SourceTargetErrorCode) -> RoleResolutionFailure {
    match code {
        SourceTargetErrorCode::SourceSetRequired | SourceTargetErrorCode::SourceSetNotFound => {
            RoleResolutionFailure::new(
                RoleResolutionCode::SourceSetUnknown,
                "the requested source set is unavailable",
            )
        }
        SourceTargetErrorCode::MetadataAddressNotFound => RoleResolutionFailure::new(
            RoleResolutionCode::TargetNotFound,
            "the logical role target was not found",
        ),
        SourceTargetErrorCode::TargetKindMismatch
        | SourceTargetErrorCode::MetadataAddressInvalid => RoleResolutionFailure::new(
            RoleResolutionCode::NotARole,
            "metadataPath does not identify a role",
        ),
        SourceTargetErrorCode::ContainmentDenied => RoleResolutionFailure::new(
            RoleResolutionCode::ContainmentDenied,
            "the logical role target failed containment checks",
        ),
        SourceTargetErrorCode::AddressProfileUnsupported => RoleResolutionFailure::new(
            RoleResolutionCode::ProfileUnsupported,
            "the logical address profile is unsupported",
        ),
        SourceTargetErrorCode::SourceRootNotAddressable => RoleResolutionFailure::new(
            RoleResolutionCode::ProviderUnavailable,
            "the logical source provider is unavailable",
        ),
    }
}

pub(crate) fn preview_edit_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> RoleEditExecution {
    edit_with_data(args, context, true)
}

pub(crate) fn apply_edit_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> RoleEditExecution {
    edit_with_data(args, context, false)
}

fn edit_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    preview: bool,
) -> RoleEditExecution {
    let request = match parse_role_edit_request(args) {
        Ok(request) => request,
        Err(error) => {
            let failed_data = args.get("metadataPath").and_then(Value::as_str).map(|raw| {
                RoleEditData::failed(
                    raw,
                    error.code,
                    error.message.clone(),
                    error.operation_index,
                )
            });
            return role_edit_failure(
                if preview {
                    "dry run: unica.role.edit rejected invalid arguments"
                } else {
                    "unica.role.edit rejected invalid arguments"
                },
                error.code,
                &error.message,
                failed_data,
            );
        }
    };
    let metadata_path = request.metadata_path.as_str().to_string();
    let planned = plan_role_edit(&request, context);
    let RoleEditPlan {
        target,
        postimage,
        data,
    } = match planned {
        Ok(plan) => plan,
        Err((code, message, operation_index)) => {
            let failed_data =
                RoleEditData::failed(metadata_path, code, message.clone(), operation_index);
            return role_edit_failure(
                if preview {
                    "dry run: unica.role.edit rejected role mutation"
                } else {
                    "unica.role.edit rejected role mutation"
                },
                code,
                &message,
                Some(failed_data),
            );
        }
    };

    let mut warnings = target
        .support_warning
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    let mut events = Vec::new();
    let mut recorded_cache = None;
    if !preview && data.changed {
        #[cfg(test)]
        run_role_edit_before_native_guard_hook();
        match publish_role_edit(&target, &postimage, context) {
            Ok(publication) => {
                for warning in publication.warnings {
                    if !warnings.contains(&warning) {
                        warnings.push(warning);
                    }
                }
                events.push(publication.event);
                recorded_cache = Some(publication.cache);
            }
            Err((code, warning)) => {
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
                let failed = RoleEditData::failed(
                    target.metadata_path.clone(),
                    code,
                    "the role mutation could not be published atomically",
                    None,
                );
                return RoleEditExecution {
                    outcome: AdapterOutcome {
                        ok: false,
                        summary: "unica.role.edit could not publish role mutation".to_string(),
                        changes: Vec::new(),
                        warnings,
                        errors: vec![format!(
                            "{code}: the role mutation could not be published atomically"
                        )],
                        artifacts: Vec::new(),
                        stdout: None,
                        stderr: None,
                        command: None,
                    },
                    data: Some(failed),
                    events: Vec::new(),
                    recorded_cache: None,
                };
            }
        }
    }

    let changed = data.changed;
    let logical_target = format!("{} + {}", target.source_set, target.metadata_path);
    RoleEditExecution {
        outcome: AdapterOutcome {
            ok: true,
            summary: if !changed {
                "unica.role.edit is already applied"
            } else if preview {
                "dry run: unica.role.edit planned role mutation"
            } else {
                "unica.role.edit applied role mutation"
            }
            .to_string(),
            changes: changed
                .then(|| {
                    format!(
                        "{}: role rights {}",
                        logical_target,
                        if preview { "would change" } else { "changed" }
                    )
                })
                .into_iter()
                .collect(),
            warnings,
            errors: Vec::new(),
            artifacts: vec![logical_target],
            stdout: None,
            stderr: None,
            command: None,
        },
        data: Some(data),
        events,
        recorded_cache,
    }
}

fn role_edit_failure(
    summary: &str,
    code: &str,
    message: &str,
    data: Option<RoleEditData>,
) -> RoleEditExecution {
    RoleEditExecution {
        outcome: AdapterOutcome {
            ok: false,
            summary: summary.to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![format!("{code}: {message}")],
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
        data,
        events: Vec::new(),
        recorded_cache: None,
    }
}

fn plan_role_edit(
    request: &unica_roles::RoleEditRequest,
    context: &WorkspaceContext,
) -> Result<RoleEditPlan, (&'static str, String, Option<usize>)> {
    let (_handle, evidence) =
        resolve_role_descriptor(&request.source_set, &request.metadata_path, context)
            .map_err(|failure| (failure.code(), failure.reason().to_string(), None))?;
    let role_name = request
        .metadata_path
        .as_str()
        .strip_prefix("Role.")
        .expect("the domain parser accepts only Role metadata paths");
    let descriptor_preimage = read_regular_file(&evidence.target_path)
        .map_err(|message| ("target_unavailable", message, None))?;
    validate_role_edit_descriptor(&descriptor_preimage, role_name)
        .map_err(|message| ("format_incompatible", message, None))?;
    let registration_preimage = read_regular_file(&evidence.registration_path)
        .map_err(|message| ("target_unavailable", message, None))?;
    let rights_path = prove_role_rights_path(&evidence, role_name, context)
        .map_err(|message| ("containment_denied", message, None))?;
    let rights_preimage =
        read_regular_file(&rights_path).map_err(|message| ("target_unavailable", message, None))?;
    let (bom, mut body) = decode_role_xml(&rights_preimage)
        .map_err(|message| ("format_incompatible", message, None))?;
    validate_role_rights_document(&body, RoleValueScope::SourceImage)
        .map_err(|message| ("role_validation_failed", message, None))?;

    let support_warning = match evaluate_resolved_support_guard(
        &evidence.target_path,
        crate::application::SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow => None,
        ResolvedSupportGuardCheck::Warn(_) => Some(role_support_warning()),
        ResolvedSupportGuardCheck::Block(_) => {
            return Err((
                "support_locked",
                "the logical role target is protected by support policy".to_string(),
                None,
            ));
        }
    };

    let mut effects = Vec::with_capacity(request.operations.len());
    for (operation_index, operation) in request.operations.iter().enumerate() {
        let (updated, effect) = apply_role_edit_operation(&body, operation, operation_index)
            .map_err(|message| {
                // Ненастроенный объект — отдельный отказ: writer вставляет
                // недостающее право, но не создаёт объектный блок, и вызывающая
                // сторона должна отличать это ограничение от порчи документа.
                let code = if message.contains("` was not found") {
                    "object_not_listed"
                } else {
                    "role_validation_failed"
                };
                (code, message, Some(operation_index))
            })?;
        body = updated;
        effects.push(effect);
    }
    let written_rights = request
        .operations
        .iter()
        .map(|operation| (operation.object_name.clone(), operation.right.clone()))
        .collect::<Vec<_>>();
    validate_role_rights_document(&body, RoleValueScope::WrittenRights(&written_rights))
        .map_err(|message| ("role_validation_failed", message, None))?;
    let postimage = encode_role_xml(bom, &body);
    let changed = rights_preimage != postimage;
    let metadata_path = request.metadata_path.as_str().to_string();

    Ok(RoleEditPlan {
        target: RoleEditTarget {
            source_set: request.source_set.clone(),
            metadata_path: metadata_path.clone(),
            descriptor_path: evidence.target_path,
            descriptor_preimage,
            registration_path: evidence.registration_path,
            registration_preimage,
            rights_path,
            rights_preimage,
            support_warning,
            written_rights,
        },
        postimage,
        data: RoleEditData::passed(metadata_path, changed, effects),
    })
}

fn publish_role_edit(
    target: &RoleEditTarget,
    postimage: &[u8],
    context: &WorkspaceContext,
) -> Result<RoleEditPublication, (&'static str, Option<String>)> {
    let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &target.metadata_path)
        .map_err(|_| ("concurrent_modification", None))?;
    let (current_handle, current_evidence) =
        resolve_role_descriptor(&target.source_set, &address, context).map_err(|failure| {
            match failure.code {
                RoleResolutionCode::ContainmentDenied => ("containment_denied", None),
                RoleResolutionCode::SourceSetUnknown
                | RoleResolutionCode::TargetNotFound
                | RoleResolutionCode::NotARole
                | RoleResolutionCode::ProviderUnavailable
                | RoleResolutionCode::ProfileUnsupported => ("concurrent_modification", None),
            }
        })?;
    if current_evidence.target_path != target.descriptor_path
        || current_evidence.registration_path != target.registration_path
    {
        return Err(("concurrent_modification", None));
    }
    let role_name = target
        .metadata_path
        .strip_prefix("Role.")
        .expect("planned role address stays canonical");
    let current_rights = prove_role_rights_path(&current_evidence, role_name, context)
        .map_err(|_| ("containment_denied", None))?;
    let current_rights_preimage =
        read_regular_file(&current_rights).map_err(|_| ("concurrent_modification", None))?;
    if current_rights != target.rights_path || current_rights_preimage != target.rights_preimage {
        return Err(("concurrent_modification", None));
    }

    #[cfg(test)]
    run_role_edit_after_rights_reread_hook();

    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes_classified(
            &target.rights_path,
            &target.rights_preimage,
            postimage.to_vec(),
        )
        .map_err(|failure| (role_commit_failure_code(failure.kind()), None))?;
    transaction
        .guard_or_verify_exact_preimage(&target.descriptor_path, &target.descriptor_preimage)
        .map_err(|_| ("concurrent_modification", None))?;
    transaction
        .guard_or_verify_exact_preimage(&target.registration_path, &target.registration_preimage)
        .map_err(|_| ("concurrent_modification", None))?;
    bind_resolved_support_guard_evidence(&mut transaction, &target.descriptor_path, context)
        .map_err(|_| ("concurrent_modification", None))?;
    let current_descriptor =
        guard_resolved_platform_xml_target_dependencies(&mut transaction, &current_handle, context)
            .map_err(|_| ("concurrent_modification", None))?;
    if current_descriptor != target.descriptor_path {
        return Err(("concurrent_modification", None));
    }
    let mut publication_warnings = Vec::new();
    match evaluate_resolved_support_guard(
        &target.descriptor_path,
        crate::application::SupportGuardRequirement::Editable,
        context,
    ) {
        ResolvedSupportGuardCheck::Allow => {}
        ResolvedSupportGuardCheck::Warn(_) => publication_warnings.push(role_support_warning()),
        ResolvedSupportGuardCheck::Block(_) => return Err(("support_locked", None)),
    }

    let event = DomainEvent::new(DomainEventKind::RoleChanged, "unica.role.edit");
    let cache = WorkspaceStateRepository::new(context)
        .stage_report_in_transaction(
            &mut transaction,
            context,
            std::slice::from_ref(&event),
            false,
            CacheAccess {
                reads: &[],
                writes: &["metadata_graph", "rights_graph"],
            },
        )
        .map_err(|_| ("cache_publication_failed", None))?;

    let expected = postimage.to_vec();
    let written_rights = target.written_rights.clone();
    let handle = current_handle;
    let descriptor = target.descriptor_path.clone();
    let rights = target.rights_path.clone();
    let role_name = role_name.to_string();
    #[cfg(test)]
    run_role_edit_before_publish_hook();
    let report = transaction.commit_with_classified_post_validation(|| {
        #[cfg(test)]
        if ROLE_EDIT_POST_VALIDATION_FAILURE.with(|slot| slot.get()) {
            return Err(CommitFailure::provider(
                "injected role.edit post-validation failure",
            ));
        }
        let current =
            crate::infrastructure::platform_xml_source_targets::revalidate_platform_xml_target(
                context, &handle,
            )
            .map_err(|_| CommitFailure::concurrent("logical role target changed"))?;
        if current.path != descriptor {
            return Err(CommitFailure::concurrent("logical role target changed"));
        }
        let descriptor_bytes = fs::read(&descriptor)
            .map_err(|_| CommitFailure::provider("role descriptor is unavailable"))?;
        validate_role_edit_descriptor(&descriptor_bytes, &role_name)
            .map_err(CommitFailure::provider)?;
        let published =
            fs::read(&rights).map_err(|_| CommitFailure::provider("Rights.xml is unavailable"))?;
        if published != expected {
            return Err(CommitFailure::concurrent(
                "published Rights.xml differs from the planned postimage",
            ));
        }
        let (_, body) = decode_role_xml(&published).map_err(CommitFailure::provider)?;
        validate_role_rights_document(&body, RoleValueScope::WrittenRights(&written_rights))
            .map_err(CommitFailure::provider)
    });
    match report {
        Ok(report) => {
            if !report.cleanup_warnings.is_empty() {
                publication_warnings.push(
                    "publication_cleanup_incomplete: role rights were committed; private recovery cleanup is incomplete"
                        .to_string(),
                );
            }
            Ok(RoleEditPublication {
                warnings: publication_warnings,
                event,
                cache,
            })
        }
        Err(error) => Err((role_commit_failure_code(error.kind()), None)),
    }
}

fn role_commit_failure_code(kind: CommitFailureKind) -> &'static str {
    match kind {
        CommitFailureKind::ConcurrentModification => "concurrent_modification",
        CommitFailureKind::ProviderUnavailable => "provider_unavailable",
        CommitFailureKind::RollbackFailed => "rollback_failed",
    }
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "required role resource is unavailable".to_string())?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err("required role resource is not a direct regular file".to_string());
    }
    fs::read(path).map_err(|_| "required role resource is unavailable".to_string())
}

/// The identity check is role-specific; the proof that `<Roles>/<R>/Ext/Rights.xml`
/// is a direct regular file inside the selected source set is the general one
/// every subject reader needs, and lives in `logical_selector`.
fn prove_role_rights_path(
    evidence: &PlatformXmlResourceEvidence,
    role_name: &str,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if evidence
        .target_path
        .file_stem()
        .and_then(|value| value.to_str())
        != Some(role_name)
    {
        return Err("role descriptor identity does not match metadataPath".to_string());
    }
    prove_attached_resource(evidence, "Rights.xml", context).map_err(|failure| failure.to_string())
}

#[cfg(test)]
pub(crate) mod role_edit_contract_tests {
    use super::*;
    use unica_roles_designer::test_support::{operation, rights_xml, validate_every_role_value};
    use unica_workspace::single_file_publisher::{with_publish_failpoints, PublishCheckpoint};

    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use unica_workspace::test_support::{
        create_dir_symlink_for_test, create_file_symlink_for_test, remove_dir_symlink_for_test,
    };

    static NONCE: AtomicU64 = AtomicU64::new(0);

    fn fixture(name: &str) -> (WorkspaceContext, Map<String, Value>, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-role-edit-{name}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let rights = root.join("src/Roles/Demo/Ext/Rights.xml");
        fs::create_dir_all(rights.parent().unwrap()).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><Role>Demo</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Roles/Demo.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Demo</Name></Properties></Role></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &rights,
            encode_role_xml(true, &rights_xml("\r\n", "<value>true</value>")),
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let args = json!({
            "sourceSet": "main",
            "metadataPath": "Role.Demo",
            "operations": [
                {"op":"setRight", "objectName":"Catalog.Demo", "right":"Delete", "value":false},
                {"op":"setRight", "objectName":"DataProcessor.Worker", "right":"Use", "value":false}
            ]
        })
        .as_object()
        .unwrap()
        .clone();
        (context, args, rights)
    }

    fn replace_format_version(raw: &[u8], replacement: Option<&str>) -> Vec<u8> {
        let marker = b" version=\"2.20\"";
        let offset = raw
            .windows(marker.len())
            .position(|window| window == marker)
            .expect("fixture carries exact format version");
        let mut updated = raw.to_vec();
        let replacement = replacement
            .map(|version| format!(" version=\"{version}\""))
            .unwrap_or_default();
        updated.splice(
            offset..offset + marker.len(),
            replacement.as_bytes().iter().copied(),
        );
        updated
    }

    #[test]
    fn a_mid_sequence_failure_publishes_nothing_and_names_its_operation() {
        // Последовательность операций — одна транзакция: отказ на второй не
        // должен оставить применённой первую.
        let (context, mut args, rights) = fixture("mid-sequence-failure");
        let before = fs::read(&rights).unwrap();
        args.insert(
            "operations".to_string(),
            json!([
                {"op":"setRight", "objectName":"Catalog.Demo", "right":"Delete", "value":false},
                {"op":"setRight", "objectName":"Catalog.Missing", "right":"Read", "value":true},
                {"op":"setRight", "objectName":"DataProcessor.Worker", "right":"Use", "value":false}
            ]),
        );

        let applied = apply_edit_with_data(&args, &context);

        assert!(!applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read(&rights).unwrap(),
            before,
            "провалившийся вызов не имеет права публиковать первую операцию"
        );
        let data = applied.data.expect("typed failure data");
        assert_eq!(data.diagnostics.len(), 1);
        assert_eq!(data.diagnostics[0].code, "object_not_listed");
        assert_eq!(
            data.diagnostics[0].operation_index,
            Some(1),
            "отказ назван своим индексом: {:?}",
            data.diagnostics
        );
        assert!(!data.changed);
        assert_eq!(
            data.validation.status,
            unica_roles::RoleEditValidationStatus::Failed
        );
        let encoded = serde_json::to_string(&data).unwrap();
        assert!(!encoded.contains("Rights.xml"), "{encoded}");
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn parse_failure_never_fabricates_a_role_metadata_path() {
        let (context, mut args, _) = fixture("invalid-metadata-path");

        args.remove("metadataPath");
        let missing = preview_edit_with_data(&args, &context);
        assert!(!missing.outcome.ok);
        assert!(missing.data.is_none());

        args.insert("metadataPath".to_string(), json!(42));
        let non_string = preview_edit_with_data(&args, &context);
        assert!(!non_string.outcome.ok);
        assert!(non_string.data.is_none());

        args.insert("metadataPath".to_string(), json!("Role.123"));
        let supplied = preview_edit_with_data(&args, &context);
        assert!(!supplied.outcome.ok);
        assert_eq!(supplied.data.unwrap().metadata_path, "Role.123");

        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    pub(crate) fn role_edit_without_vendor_support_is_logically_addressed_and_preserves_identity() {
        use unica_workspace::test_support::file_identity_for_test;

        let (context, args, rights) = fixture("roundtrip");
        assert!(
            !context
                .workspace_root
                .join("src/Ext/ParentConfigurations.bin")
                .exists(),
            "the no-support case must not accidentally inherit vendor support evidence"
        );
        let before = fs::read(&rights).unwrap();
        let preview = preview_edit_with_data(&args, &context);
        assert!(preview.outcome.ok, "{:?}", preview.outcome);
        assert!(preview.outcome.stdout.is_none());
        assert!(preview.outcome.stderr.is_none());
        assert_eq!(preview.outcome.artifacts, ["main + Role.Demo"]);
        let preview_data = preview.data.unwrap();
        assert!(preview_data.changed);
        assert_eq!(preview_data.effects.len(), 2);
        assert_eq!(preview_data.effects[0].operation_index, 0);
        assert_eq!(preview_data.effects[1].operation_index, 1);
        assert_eq!(fs::read(&rights).unwrap(), before);

        let applied = apply_edit_with_data(&args, &context);
        assert!(applied.outcome.ok, "{:?}", applied.outcome);
        assert_eq!(applied.events.len(), 1);
        assert_eq!(applied.events[0].kind, DomainEventKind::RoleChanged);
        let cache = applied
            .recorded_cache
            .as_ref()
            .expect("applied source and cache state commit together");
        assert_eq!(cache.events, ["RoleChanged"]);
        assert!(cache.invalidated.contains(&"rights_graph".to_string()));
        assert!(cache.refreshed.contains(&"rights_graph".to_string()));
        assert!(applied.data.unwrap().changed);
        let after = fs::read(&rights).unwrap();
        assert_ne!(after, before);
        assert!(after.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert!(!String::from_utf8_lossy(&after).contains("DataProcessor.Worker"));
        let identity = file_identity_for_test(&rights).unwrap();

        let repeated = apply_edit_with_data(&args, &context);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome);
        assert!(repeated.events.is_empty());
        assert!(repeated.recorded_cache.is_none());
        assert!(!repeated.data.unwrap().changed);
        assert_eq!(fs::read(&rights).unwrap(), after);
        assert_eq!(file_identity_for_test(&rights).unwrap(), identity);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn exact_profile_rejects_rights_and_descriptor_versions_outside_2_20() {
        let (context, args, rights) = fixture("exact-profile");
        let original = fs::read(&rights).unwrap();

        for version in [Some("2.19"), Some("2.21"), Some("2.2&#48;"), None] {
            let incompatible = replace_format_version(&original, version);
            fs::write(&rights, &incompatible).unwrap();
            let rejected = apply_edit_with_data(&args, &context);
            assert!(!rejected.outcome.ok, "version={version:?}");
            assert_eq!(fs::read(&rights).unwrap(), incompatible);
        }

        fs::write(&rights, &original).unwrap();
        let descriptor = context.workspace_root.join("src/Roles/Demo.xml");
        let original_descriptor = fs::read(&descriptor).unwrap();
        for version in [Some("2.19"), Some("2.21"), Some("2.2&#48;"), None] {
            let incompatible = replace_format_version(&original_descriptor, version);
            fs::write(&descriptor, &incompatible).unwrap();
            let rejected = apply_edit_with_data(&args, &context);
            assert!(!rejected.outcome.ok, "descriptor version={version:?}");
            assert_eq!(fs::read(&rights).unwrap(), original);
            assert_eq!(fs::read(&descriptor).unwrap(), incompatible);
        }
        fs::write(&descriptor, original_descriptor).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    pub(crate) fn rights_drift_in_the_staging_window_is_classified_as_concurrent() {
        let (context, args, rights) = fixture("staging-byte-drift");
        let changed_rights = rights.clone();
        let changed = with_role_edit_after_rights_reread_hook(
            move || {
                let mut bytes = fs::read(&changed_rights).unwrap();
                bytes.extend_from_slice(b"<!-- concurrent -->");
                fs::write(&changed_rights, bytes).unwrap();
            },
            || apply_edit_with_data(&args, &context),
        );
        assert!(!changed.outcome.ok);
        assert!(changed.outcome.errors[0].starts_with("concurrent_modification:"));
        assert!(fs::read(&rights).unwrap().ends_with(b"<!-- concurrent -->"));
        assert!(changed.events.is_empty());
        assert!(changed.recorded_cache.is_none());
        fs::remove_dir_all(context.workspace_root).unwrap();

        let (context, args, rights) = fixture("staging-disappearance");
        let removed_rights = rights.clone();
        let removed = with_role_edit_after_rights_reread_hook(
            move || fs::remove_file(&removed_rights).unwrap(),
            || apply_edit_with_data(&args, &context),
        );
        assert!(!removed.outcome.ok);
        assert!(removed.outcome.errors[0].starts_with("concurrent_modification:"));
        assert!(!rights.exists());
        assert!(removed.events.is_empty());
        assert!(removed.recorded_cache.is_none());
        fs::remove_dir_all(context.workspace_root).unwrap();

        let (context, args, rights) = fixture("staging-symlink-swap");
        let original = fs::read(&rights).unwrap();
        let saved = rights.with_extension("saved");
        let outside = context.workspace_root.join("outside-rights.xml");
        fs::write(&outside, &original).unwrap();
        let probe = create_file_symlink_for_test(&outside, &saved)
            .expect("symlink fixtures exist on Unix and Windows");
        if probe.is_err() {
            fs::remove_dir_all(context.workspace_root).unwrap();
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        fs::remove_file(&saved).unwrap();
        let swapped_rights = rights.clone();
        let swapped_saved = saved.clone();
        let swapped_outside = outside.clone();
        let swapped = with_role_edit_after_rights_reread_hook(
            move || {
                fs::rename(&swapped_rights, &swapped_saved).unwrap();
                create_file_symlink_for_test(&swapped_outside, &swapped_rights)
                    .expect("file links are available on this test host")
                    .expect("test host permits a file link");
            },
            || apply_edit_with_data(&args, &context),
        );
        assert!(!swapped.outcome.ok);
        assert!(swapped.outcome.errors[0].starts_with("concurrent_modification:"));
        assert_eq!(fs::read(&outside).unwrap(), original);
        assert_eq!(fs::read(&saved).unwrap(), original);
        assert!(swapped.events.is_empty());
        assert!(swapped.recorded_cache.is_none());
        fs::remove_file(&rights).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn descriptor_configuration_and_rights_preimage_drift_and_post_validation_are_failure_atomic() {
        let (context, args, rights) = fixture("transaction-guards");
        let original = fs::read(&rights).unwrap();

        let concurrent = {
            let rights = rights.clone();
            with_role_edit_before_publish_hook(
                move || {
                    let mut changed = fs::read(&rights).unwrap();
                    changed.extend_from_slice(b"<!-- concurrent -->");
                    fs::write(&rights, changed).unwrap();
                },
                || apply_edit_with_data(&args, &context),
            )
        };
        assert!(!concurrent.outcome.ok);
        assert!(
            concurrent.outcome.errors[0].starts_with("concurrent_modification:"),
            "{:?}",
            concurrent.outcome
        );
        let concurrent_bytes = fs::read(&rights).unwrap();
        assert!(concurrent_bytes.ends_with(b"<!-- concurrent -->"));

        fs::write(&rights, &original).unwrap();
        let descriptor = context.workspace_root.join("src/Roles/Demo.xml");
        let descriptor_before = fs::read(&descriptor).unwrap();
        let descriptor_drift = {
            let descriptor = descriptor.clone();
            with_role_edit_before_publish_hook(
                move || {
                    let mut changed = fs::read(&descriptor).unwrap();
                    changed.extend_from_slice(b"<!-- descriptor drift -->");
                    fs::write(&descriptor, changed).unwrap();
                },
                || apply_edit_with_data(&args, &context),
            )
        };
        assert!(!descriptor_drift.outcome.ok);
        assert!(
            descriptor_drift.outcome.errors[0].starts_with("concurrent_modification:"),
            "{:?}",
            descriptor_drift.outcome
        );
        assert_eq!(fs::read(&rights).unwrap(), original);
        assert_ne!(fs::read(&descriptor).unwrap(), descriptor_before);
        fs::write(&descriptor, descriptor_before).unwrap();

        let owner = context.workspace_root.join("src/Configuration.xml");
        let owner_before = fs::read(&owner).unwrap();
        let owner_drift = {
            let owner = owner.clone();
            with_role_edit_before_publish_hook(
                move || {
                    let mut changed = fs::read(&owner).unwrap();
                    changed.extend_from_slice(b"<!-- owner drift -->");
                    fs::write(&owner, changed).unwrap();
                },
                || apply_edit_with_data(&args, &context),
            )
        };
        assert!(!owner_drift.outcome.ok);
        assert!(
            owner_drift.outcome.errors[0].starts_with("concurrent_modification:"),
            "{:?}",
            owner_drift.outcome
        );
        assert_eq!(fs::read(&rights).unwrap(), original);
        assert_ne!(fs::read(&owner).unwrap(), owner_before);
        fs::write(&owner, owner_before).unwrap();

        fs::write(&rights, &original).unwrap();
        let rolled_back =
            with_role_edit_post_validation_failure(|| apply_edit_with_data(&args, &context));
        assert!(!rolled_back.outcome.ok);
        assert!(
            rolled_back.outcome.errors[0].starts_with("provider_unavailable:"),
            "{:?}",
            rolled_back.outcome
        );
        assert_eq!(fs::read(&rights).unwrap(), original);
        assert!(!context.cache_root.join("state.json").exists());
        assert!(!context
            .cache_root
            .join("caches/metadata_graph.json")
            .exists());
        assert!(!context.cache_root.join("caches/rights_graph.json").exists());
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn cache_planning_failure_leaves_role_source_unchanged() {
        let (context, args, rights) = fixture("cache-planning-failure");
        let before = fs::read(&rights).unwrap();
        fs::create_dir_all(context.cache_root.join("state.json")).unwrap();
        let rejected = apply_edit_with_data(&args, &context);
        assert!(!rejected.outcome.ok, "{:?}", rejected.outcome);
        assert!(rejected.outcome.errors[0].contains("cache_publication_failed"));
        assert_eq!(fs::read(&rights).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_edit_rejects_invalid_right_for_nested_object_without_changing_rights() {
        let (context, args, rights) = fixture("invalid-nested-right");
        let before = fs::read(&rights).unwrap();

        let mut nested = args.clone();
        nested.insert(
            "operations".to_string(),
            json!([{
                "op":"setRight",
                "objectName":"DataProcessor.Worker.Command.Run",
                "right":"Use",
                "value":false
            }]),
        );
        let invalid = apply_edit_with_data(&nested, &context);
        assert!(!invalid.outcome.ok, "{:?}", invalid.outcome);
        let data = invalid.data.expect("typed failure data");
        assert_eq!(data.diagnostics.len(), 1);
        assert_eq!(data.diagnostics[0].code, "unsupported_right");
        assert_eq!(data.diagnostics[0].operation_index, Some(0));
        assert!(
            data.diagnostics[0]
                .message
                .contains("DataProcessor.Worker.Command.Run"),
            "{:?}",
            data.diagnostics
        );
        assert_eq!(fs::read(&rights).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_edit_rejects_locked_vendor_supported_role_without_changing_rights() {
        let (context, args, rights) = fixture("locked-vendor-supported-role");
        let before = fs::read(&rights).unwrap();

        fs::create_dir_all(context.workspace_root.join("src/Ext")).unwrap();
        fs::write(
            context
                .workspace_root
                .join("src/Ext/ParentConfigurations.bin"),
            concat!(
                "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc}"
            )
            .as_bytes(),
        )
        .unwrap();
        let denied = apply_edit_with_data(&args, &context);
        assert!(!denied.outcome.ok, "{:?}", denied.outcome);
        assert!(denied.outcome.errors[0].contains("support_locked"));
        assert_eq!(fs::read(&rights).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_edit_changes_editable_vendor_supported_role_addressed_logically() {
        let (context, args, rights) = fixture("editable-vendor-supported-role");
        fs::create_dir_all(context.workspace_root.join("src/Ext")).unwrap();
        fs::write(
            context
                .workspace_root
                .join("src/Ext/ParentConfigurations.bin"),
            concat!(
                "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,1,0,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc}"
            )
            .as_bytes(),
        )
        .unwrap();

        let support = WorkspaceSupportStateReader::new(&context)
            .object_support(&unica_address::source_target::ResolvedTarget {
                source_set: "main".to_string(),
                metadata_path: Some(
                    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Role.Demo").unwrap(),
                ),
                target_kind: TargetKind::MetadataObject,
            })
            .unwrap();
        assert_eq!(
            support.state,
            unica_metadata::support_state::ObjectSupportState::EditableWithSupport
        );
        assert_eq!(support.direct_edit_safe, Some(true));

        let expected_body = [
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:r="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="Rights" version="2.20">"#,
            "\t<setForNewObjects>false</setForNewObjects>",
            "\t<setForAttributesByDefault>true</setForAttributesByDefault>",
            "\t<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
            "\t<object>",
            "\t\t<name>Catalog.Demo</name>",
            "\t\t<future><name>Delete</name><value>false</value></future>",
            "\t\t<right><name>FuturePlatformRight</name><value>true</value><future/></right>",
            "\t</object>",
            "\t<restrictionTemplate><name>Keep</name><condition>TRUE</condition></restrictionTemplate>",
            "</Rights>",
        ]
        .join("\r\n");
        let expected = encode_role_xml(true, &expected_body);

        let changed = apply_edit_with_data(&args, &context);

        assert!(changed.outcome.ok, "{:?}", changed.outcome);
        assert_eq!(fs::read(&rights).unwrap(), expected);
        assert!(changed.data.expect("typed success data").changed);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn publish_time_allow_to_warn_transition_is_reported_once() {
        let (context, args, _rights) = fixture("allow-to-warn");
        let workspace = context.workspace_root.clone();
        let changed = with_role_edit_before_native_guard_hook(
            move || {
                fs::create_dir_all(workspace.join("src/Ext")).unwrap();
                fs::write(
                    workspace.join("src/Ext/ParentConfigurations.bin"),
                    concat!(
                        "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                        "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
                        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
                        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
                        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
                        "cccccccc-cccc-cccc-cccc-cccccccccccc,",
                        "cccccccc-cccc-cccc-cccc-cccccccccccc}"
                    )
                    .as_bytes(),
                )
                .unwrap();
                fs::write(
                    workspace.join(".v8-project.json"),
                    r#"{"editingAllowedCheck":"warn"}"#,
                )
                .unwrap();
            },
            || apply_edit_with_data(&args, &context),
        );

        assert!(changed.outcome.ok, "{:?}", changed.outcome);
        assert_eq!(
            changed
                .outcome
                .warnings
                .iter()
                .filter(|warning| warning.contains("support_guard_warning"))
                .count(),
            1
        );
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn cleanup_warning_is_success_after_validated_committed_bytes() {
        let (context, args, rights) = fixture("cleanup-warning");
        let before = fs::read(&rights).unwrap();
        let result = with_publish_failpoints(&[PublishCheckpoint::Cleanup], || {
            apply_edit_with_data(&args, &context)
        });
        assert!(result.outcome.ok, "{:?}", result.outcome);
        assert!(result
            .outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("publication_cleanup_incomplete")));
        assert_ne!(fs::read(&rights).unwrap(), before);
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn role_resource_symlink_swap_after_planning_is_rejected_before_publication() {
        let (context, args, rights) = fixture("symlink-swap");
        let role_dir = rights.parent().unwrap().parent().unwrap().to_path_buf();
        let saved_role = context.workspace_root.join("saved-role");
        let outside = context.workspace_root.join("outside-role");
        let outside_rights = outside.join("Ext/Rights.xml");
        fs::create_dir_all(outside_rights.parent().unwrap()).unwrap();
        let before = fs::read(&rights).unwrap();
        fs::write(&outside_rights, &before).unwrap();
        let probe_source = context.workspace_root.join("link-probe-source");
        let probe_link = context.workspace_root.join("link-probe");
        fs::create_dir(&probe_source).unwrap();
        let probe_result = create_dir_symlink_for_test(&probe_source, &probe_link)
            .expect("symlink fixtures exist on Unix and Windows");
        if probe_result.is_err() {
            fs::remove_dir_all(context.workspace_root).unwrap();
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        remove_dir_symlink_for_test(&probe_link).unwrap();

        let swapped_role = role_dir.clone();
        let swapped_outside = outside.clone();
        let swapped_saved = saved_role.clone();
        let rejected = with_role_edit_before_publish_hook(
            move || {
                fs::rename(&swapped_role, &swapped_saved).unwrap();
                create_dir_symlink_for_test(&swapped_outside, &swapped_role)
                    .expect("directory links are available on supported hosts")
                    .expect("test host must permit a directory link");
            },
            || apply_edit_with_data(&args, &context),
        );
        assert!(!rejected.outcome.ok, "{:?}", rejected.outcome);
        assert_eq!(fs::read(&outside_rights).unwrap(), before);
        assert_eq!(fs::read(saved_role.join("Ext/Rights.xml")).unwrap(), before);
        remove_dir_symlink_for_test(&role_dir).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }

    #[test]
    fn symlinked_role_resource_is_rejected_without_following_it() {
        let (context, args, rights) = fixture("symlink");
        let role_dir = rights.parent().unwrap().parent().unwrap().to_path_buf();
        let outside = context.workspace_root.join("outside-role");
        fs::create_dir_all(outside.join("Ext")).unwrap();
        let outside_rights = outside.join("Ext/Rights.xml");
        let before = fs::read(&rights).unwrap();
        fs::write(&outside_rights, &before).unwrap();
        fs::remove_dir_all(&role_dir).unwrap();
        let link_result = create_dir_symlink_for_test(&outside, &role_dir)
            .expect("symlink fixtures exist on Unix and Windows");
        if link_result.is_err() {
            fs::remove_dir_all(context.workspace_root).unwrap();
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }

        let rejected = apply_edit_with_data(&args, &context);
        assert!(!rejected.outcome.ok);
        assert_eq!(fs::read(&outside_rights).unwrap(), before);
        remove_dir_symlink_for_test(&role_dir).unwrap();
        fs::remove_dir_all(context.workspace_root).unwrap();
    }
}

#[cfg(test)]
pub(super) mod role_info_typed_result_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn workspace(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!(
            "unica-role-info-typed-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src/Roles/Reader/Ext")).unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Role>Reader</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Roles/Reader.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role><Properties><Name>Reader</Name></Properties></Role></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Roles/Reader/Ext/Rights.xml"),
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" setForNewObjects="false" setForAttributesByDefault="true" independentRightsOfChildObjects="false">
  <object><name>Catalog.Goods</name>
    <right><name>Read</name><value>true</value><restrictionByCondition><condition>ГДЕ Ложь</condition></restrictionByCondition></right>
    <right><name>Insert</name><value>false</value></right>
  </object>
</Rights>"#,
        )
        .unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    /// The support state belongs to the object, and `Rights.xml` sits two
    /// directories below the configuration root. Reading it from the leaf path
    /// answered `notSupported` for a configuration that is on support.
    #[test]
    fn role_info_reads_the_support_state_from_the_configuration_root() {
        let context = workspace("support");
        fs::write(
            context.workspace_root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Role>Reader</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([(
            "RightsPath".to_string(),
            json!("src/Roles/Reader/Ext/Rights.xml"),
        )]);

        let execution =
            analyze_role_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let data = execution.data.expect("role info answers with data");
        // No ParentConfigurations.bin here, so the honest answer is
        // `notSupported` — but it must come from the resolved configuration
        // root, not from a directory the walk never reached.
        assert_eq!(
            data.support.state,
            unica_metadata::support_state::ObjectSupportState::NotSupported,
            "{data:?}"
        );
        assert_eq!(data.support.direct_edit_safe, None, "{data:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// `ShowDenied` used to decide whether denied rights appeared at all, so an
    /// answer without them could mean "none" or "you did not ask".
    #[test]
    fn role_info_reports_allowed_and_denied_rights_without_a_flag() {
        let context = workspace("both");
        let args = Map::from_iter([(
            "RightsPath".to_string(),
            json!("src/Roles/Reader/Ext/Rights.xml"),
        )]);

        let execution =
            analyze_role_info(&args, &context, &WorkspaceSupportStateReader::new(&context));

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        assert!(execution.outcome.stdout.is_none());
        let data = execution.data.expect("role info answers with data");
        assert_eq!(data.name, "Reader");
        assert_eq!(data.totals.allowed, 1);
        assert_eq!(data.totals.denied, 1);
        assert_eq!(data.defaults.set_for_new_objects.as_deref(), Some("false"));
        let allowed = &data.allowed[0].objects[0];
        assert_eq!(allowed.name, "Goods");
        assert!(allowed.rights.iter().any(|right| right.restricted));
        assert!(!data.denied.is_empty(), "denied rights are always reported");
        assert_eq!(data.restricted_objects.len(), 1);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// The same dump, made addressable: a project map naming the source set and
    /// a configuration that registers the role.
    fn addressable_workspace(name: &str) -> WorkspaceContext {
        let context = workspace(name);
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            context.workspace_root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Role>Reader</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        context
    }

    #[test]
    pub(crate) fn role_info_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = addressable_workspace("bridge");

        let physical = analyze_role_info(
            &Map::from_iter([(
                "RightsPath".to_string(),
                json!("src/Roles/Reader/Ext/Rights.xml"),
            )]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );
        let logical = analyze_role_info(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("Role.Reader")),
            ]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(logical.outcome.ok, "{:?}", logical.outcome);
        assert!(physical.outcome.ok, "{:?}", physical.outcome);
        // Compare what the caller actually receives: the serialized
        // answer, not a struct the wire never carries.
        assert_eq!(
            serde_json::to_value(&logical.data).unwrap(),
            serde_json::to_value(&physical.data).unwrap()
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn role_info_accepts_the_russian_kind_alias() {
        let context = addressable_workspace("bridge-alias");

        let logical = analyze_role_info(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("Роль.Reader")),
            ]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(logical.outcome.ok, "{:?}", logical.outcome);
        assert_eq!(
            logical.data.expect("role info answers with data").name,
            "Reader"
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn role_info_refuses_an_address_that_is_not_a_role() {
        let context = addressable_workspace("bridge-wrong-kind");

        let outcome = analyze_role_info(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("Catalog.Goods")),
            ]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        )
        .outcome;

        assert!(!outcome.ok);
        assert!(
            outcome
                .errors
                .iter()
                .chain(std::iter::once(&outcome.summary))
                .any(|message| message.contains("target_kind_unsupported")),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    pub(crate) fn role_validate_answers_identically_for_a_logical_and_a_physical_selector() {
        let context = addressable_workspace("bridge-validate");

        let physical = validate_role(
            &Map::from_iter([(
                "RightsPath".to_string(),
                json!("src/Roles/Reader/Ext/Rights.xml"),
            )]),
            &context,
        );
        let logical = validate_role(
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!("Role.Reader")),
            ]),
            &context,
        );

        assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
        assert_eq!(logical.errors, physical.errors);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}

#[cfg(test)]
pub(crate) mod role_validate_contract_tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-role-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn context(root: &Path) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    #[test]
    fn role_validate_reports_validation_failures_in_errors() {
        let workspace = temp_root("validate-errors");
        let rights_path = workspace.join("missing-rights.xml");
        let outcome = validate_role(
            &Map::from_iter([(
                "RightsPath".to_string(),
                Value::String(rights_path.display().to_string()),
            )]),
            &context(&workspace),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("File not found"),
            "{outcome:?}"
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    fn validate_role_stdout(rights_xml: &str) -> String {
        let workspace = temp_root("role-validate-predefined-data");
        let ext_dir = workspace.join("Roles/PredefinedDataEditor/Ext");
        fs::create_dir_all(&ext_dir).unwrap();
        let rights_path = ext_dir.join("Rights.xml");
        fs::write(&rights_path, rights_xml).unwrap();

        let args = Map::from_iter([
            (
                "RightsPath".to_string(),
                Value::String(rights_path.display().to_string()),
            ),
            ("Detailed".to_string(), Value::Bool(true)),
        ]);
        let outcome = validate_role(&args, &context(&workspace));
        let stdout = outcome.stdout.clone().unwrap_or_default();
        let _ = fs::remove_dir_all(&workspace);
        assert!(outcome.ok, "{outcome:?}");
        stdout
    }

    const PREDEFINED_DATA_WARNING: &str =
        "grants interactive changes to predefined data (predefined data is part of the configuration and should not be available to end users)";

    #[test]
    fn validate_role_warns_on_interactive_predefined_data_right_set_true() {
        let rights = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xsi:type=\"Rights\" version=\"2.20\"\n",
            "        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "    <setForNewObjects>false</setForNewObjects>\n",
            "    <setForAttributesByDefault>false</setForAttributesByDefault>\n",
            "    <independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
            "    <object>\n",
            "        <name>Catalog.Products</name>\n",
            "        <right><name>Read</name><value>true</value></right>\n",
            "        <right><name>InteractiveDeletePredefinedData</name><value>true</value></right>\n",
            "    </object>\n",
            "</Rights>\n",
        );
        let stdout = validate_role_stdout(rights);
        assert!(
            stdout.contains(&format!(
                "Catalog.Products: 'InteractiveDeletePredefinedData' = true {PREDEFINED_DATA_WARNING}"
            )),
            "{stdout}"
        );
    }

    #[test]
    fn validate_role_allows_interactive_predefined_data_right_set_false() {
        let rights = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xsi:type=\"Rights\" version=\"2.20\"\n",
            "        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "    <setForNewObjects>false</setForNewObjects>\n",
            "    <setForAttributesByDefault>false</setForAttributesByDefault>\n",
            "    <independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
            "    <object>\n",
            "        <name>Catalog.Products</name>\n",
            "        <right><name>InteractiveClearDeletionMarkPredefinedData</name><value>false</value></right>\n",
            "    </object>\n",
            "</Rights>\n",
        );
        let stdout = validate_role_stdout(rights);
        assert!(!stdout.contains(PREDEFINED_DATA_WARNING), "{stdout}");
    }

    #[test]
    fn validate_role_allows_ordinary_right_without_predefined_data_warning() {
        let rights = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" xsi:type=\"Rights\" version=\"2.20\"\n",
            "        xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "    <setForNewObjects>false</setForNewObjects>\n",
            "    <setForAttributesByDefault>false</setForAttributesByDefault>\n",
            "    <independentRightsOfChildObjects>false</independentRightsOfChildObjects>\n",
            "    <object>\n",
            "        <name>Catalog.Products</name>\n",
            "        <right><name>InteractiveDelete</name><value>true</value></right>\n",
            "    </object>\n",
            "</Rights>\n",
        );
        let stdout = validate_role_stdout(rights);
        assert!(!stdout.contains(PREDEFINED_DATA_WARNING), "{stdout}");
    }
}

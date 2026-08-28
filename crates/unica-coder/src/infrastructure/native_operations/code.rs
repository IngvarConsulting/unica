use crate::application::AdapterOutcome;
use crate::application::SupportGuardRequirement;
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::project_sources::SourceSetKind;
use crate::domain::source_target::{ResolvedTarget, TargetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::logical_event_source::{module_relative, module_source_address};
use crate::infrastructure::native_operations::apply::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyStagedState, ApplyStagingError, PlannedApplyEffects,
};
use crate::infrastructure::platform_xml_owner::{
    prove_already_read_metadata_owner, prove_already_read_source_set_owner,
    PlatformXmlSourceSetOwnerEvidence,
};
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_module_identity, resolve_platform_xml_target, revalidate_platform_xml_target,
    ClosedPlatformXmlTarget, PlatformXmlModuleIdentity, TargetKindPolicy,
};
use crate::infrastructure::support_guard::{
    evaluate_resolved_support_guard, ResolvedSupportGuardCheck,
};
use crate::infrastructure::workspace_actor::{CodeApplyAuthority, ProviderRootBinding};
use bsl_syntax::ast::{AstNode, FunctionDef, ProcedureDef};
use diffy::{apply, DiffOptions, Patch};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::infrastructure::platform_xml_source_targets::platform_xml_module_identity as module_identity;

use super::common::{
    code_patch_source_target, guard_code_patch_resolved_target, parse_support_state_compat_bytes,
    support_root_uuid_from_bytes,
};
use super::compile_transaction::CompileTransaction;
use super::text_snapshot::{
    resolve_observed_line_ending, LineEnding, LineEndingProfile, SourceTextSnapshot,
};

pub(crate) fn apply_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> CodePatchExecution {
    patch_inner(args, context, PatchMode::Apply)
}

pub(crate) fn preview_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> CodePatchExecution {
    patch_inner(args, context, PatchMode::Preview)
}

pub(crate) struct CodePatchExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<CodePatchData>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodePatchData {
    source_set: String,
    metadata_path: String,
    target_kind: TargetKind,
    pre_hash: String,
    post_hash: String,
    no_op: bool,
    changed_ranges: Vec<ChangedRange>,
    diff: String,
    affected_target: AffectedTarget,
    validation: SourceValidation,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChangedRange {
    start_byte: usize,
    end_byte: usize,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AffectedTarget {
    source_set: String,
    metadata_path: String,
    target_kind: TargetKind,
    owner: String,
    module_role: String,
    raw_hash: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceValidation {
    kind: ValidationKind,
    status: ValidationStatus,
    validated_post_hash: String,
    diagnostics: Vec<ValidationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum ValidationKind {
    #[serde(rename = "bsl-analyzer-parser")]
    BslAnalyzerParser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ValidationStatus {
    Passed,
    Failed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationDiagnostic {
    code: &'static str,
    message: String,
    start_byte: usize,
    end_byte: usize,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchMode {
    Apply,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Before,
    After,
}

impl Position {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "before" => Ok(Self::Before),
            "after" => Ok(Self::After),
            _ => Err("position must be before or after".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selector {
    Method(String),
    Anchor(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectorResolutionCause {
    ZeroMatches,
    MultipleMatches,
    InvalidSource,
}

#[derive(Debug)]
struct SelectorResolutionError {
    cause: SelectorResolutionCause,
    legacy_message: String,
}

impl SelectorResolutionError {
    fn zero_matches(selector: &Selector) -> Self {
        Self::match_count(selector, 0, SelectorResolutionCause::ZeroMatches)
    }

    fn multiple_matches(selector: &Selector, count: usize) -> Self {
        Self::match_count(selector, count, SelectorResolutionCause::MultipleMatches)
    }

    fn match_count(selector: &Selector, count: usize, cause: SelectorResolutionCause) -> Self {
        let name = match selector {
            Selector::Method(_) => "method",
            Selector::Anchor(_) => "anchor",
        };
        Self {
            cause,
            legacy_message: format!(
                "{name} selector must match exactly once; matched {count} times"
            ),
        }
    }

    fn invalid_source(message: impl Into<String>) -> Self {
        Self {
            cause: SelectorResolutionCause::InvalidSource,
            legacy_message: message.into(),
        }
    }

    const fn cause(&self) -> SelectorResolutionCause {
        self.cause
    }
}

impl std::fmt::Display for SelectorResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.legacy_message.fmt(formatter)
    }
}

impl Selector {
    /// `insert` names a place only when it has one to name. An absent selector
    /// is not an error: it means the end of the module, which is the one place
    /// every module has, including a module that holds no method yet.
    fn parse_optional(args: &Map<String, Value>) -> Result<Option<Self>, String> {
        if !args.contains_key("selector") {
            return Ok(None);
        }
        Self::parse(args).map(Some)
    }

    fn parse(args: &Map<String, Value>) -> Result<Self, String> {
        let selector = args
            .get("selector")
            .and_then(Value::as_object)
            .ok_or_else(|| "selector must be an object".to_string())?;
        if selector.len() != 1 {
            return Err("selector must contain exactly one of method or anchor".to_string());
        }
        match (
            selector.get("method").and_then(Value::as_str),
            selector.get("anchor").and_then(Value::as_str),
        ) {
            (Some(name), None) if !name.is_empty() => Ok(Self::Method(name.to_string())),
            (None, Some(anchor)) if !anchor.is_empty() => {
                Ok(Self::Anchor(canonicalize_eol(anchor)))
            }
            _ => Err("selector must contain exactly one non-empty method or anchor".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeadingSeparator {
    None,
    LocalEol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchOperation {
    Insert,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodePosition {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodeSelector {
    Method(String),
    Anchor(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeInsertArgs {
    pub(crate) at: crate::domain::address::QualifiedAddress,
    pub(crate) text: String,
    pub(crate) selector: Option<CodeSelector>,
    pub(crate) position: Option<CodePosition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeReplaceArgs {
    pub(crate) at: crate::domain::address::QualifiedAddress,
    pub(crate) text: String,
    pub(crate) selector: CodeSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodePlanOperation {
    Insert(CodeInsertArgs),
    Replace(CodeReplaceArgs),
}

pub(crate) fn parse_code_plan_operation(
    operation: &str,
    value: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<CodePlanOperation, ApplyPlanError> {
    let base = format!("ops[{op_index}].args");
    let object = value
        .as_object()
        .ok_or_else(|| bad_code_arg(&base, "code operation args must be an object"))?;
    let allowed = match operation {
        "code.insert" => &["at", "text", "selector", "position"][..],
        "code.replace" => &["at", "text", "selector"][..],
        _ => {
            return Err(bad_code_arg(
                &base,
                "code planner accepts only the closed code.insert/code.replace catalog",
            ))
        }
    };
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(bad_code_arg(
            format!("{base}.{unknown}"),
            "unknown staged code argument",
        ));
    }
    let at_path = format!("{base}.at");
    let at_raw = non_empty_code_string(object.get("at"), &at_path, "at")?;
    let at = crate::domain::address::QualifiedAddress::parse(at_raw)
        .map_err(|_| bad_code_arg(&at_path, "at must be a qualified logical address"))?;
    if at.source_set() != binding.source_set_name() {
        return Err(bad_code_arg(
            &at_path,
            "at belongs to another actor-admitted source set",
        ));
    }
    let text_path = format!("{base}.text");
    let text = non_empty_code_string(object.get("text"), &text_path, "text")?.to_string();

    match operation {
        "code.insert" => {
            let selector = object
                .get("selector")
                .map(|value| parse_code_selector(value, &format!("{base}.selector")))
                .transpose()?;
            let position_path = format!("{base}.position");
            let position = match (selector.is_some(), object.get("position")) {
                (true, Some(value)) => Some(parse_code_position(value, &position_path)?),
                (true, None) => {
                    return Err(bad_code_arg(
                        position_path,
                        "position is required when selector is present",
                    ))
                }
                (false, Some(_)) => {
                    return Err(bad_code_arg(
                        position_path,
                        "position is unavailable without selector",
                    ))
                }
                (false, None) => None,
            };
            Ok(CodePlanOperation::Insert(CodeInsertArgs {
                at,
                text,
                selector,
                position,
            }))
        }
        "code.replace" => {
            let selector_path = format!("{base}.selector");
            let selector = object
                .get("selector")
                .ok_or_else(|| bad_code_arg(&selector_path, "selector is required"))
                .and_then(|value| parse_code_selector(value, &selector_path))?;
            Ok(CodePlanOperation::Replace(CodeReplaceArgs {
                at,
                text,
                selector,
            }))
        }
        _ => unreachable!("closed operation checked above"),
    }
}

fn non_empty_code_string<'a>(
    value: Option<&'a Value>,
    path: &str,
    name: &str,
) -> Result<&'a str, ApplyPlanError> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| bad_code_arg(path, format!("{name} must be a non-empty string")))
}

fn parse_code_selector(value: &Value, path: &str) -> Result<CodeSelector, ApplyPlanError> {
    let object = value
        .as_object()
        .ok_or_else(|| bad_code_arg(path, "selector must be an object"))?;
    if object.len() != 1 {
        return Err(bad_code_arg(
            path,
            "selector must contain exactly one method or anchor",
        ));
    }
    let (name, value) = object.iter().next().expect("one selector field");
    let field_path = format!("{path}.{name}");
    let value = value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| bad_code_arg(&field_path, "selector value must be a non-empty string"))?;
    match name.as_str() {
        "method" => Ok(CodeSelector::Method(value.to_string())),
        "anchor" => Ok(CodeSelector::Anchor(canonicalize_eol(value))),
        _ => Err(bad_code_arg(
            field_path,
            "selector accepts only method or anchor",
        )),
    }
}

fn parse_code_position(value: &Value, path: &str) -> Result<CodePosition, ApplyPlanError> {
    match value.as_str() {
        Some("before") => Ok(CodePosition::Before),
        Some("after") => Ok(CodePosition::After),
        _ => Err(bad_code_arg(path, "position must be before or after")),
    }
}

fn bad_code_arg(path: impl Into<String>, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(path)
}

pub(crate) fn plan_code_batch(
    mut staged: ApplyStagedState,
    authority: CodeApplyAuthority<'_>,
    operations: &[CodePlanOperation],
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    if !authority.owns_staged_state(&staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "staged source belongs to another apply admission",
        ));
    }
    let mut provisional_effects = Vec::new();
    for (op_index, operation) in operations.iter().enumerate() {
        plan_one_code_operation(
            &mut staged,
            &mut provisional_effects,
            &authority,
            operation,
            op_index,
        )?;
    }
    let final_changes = staged.planned_changes();
    let mut effects = PlannedApplyEffects::default();
    for (relative, event) in provisional_effects {
        if final_changes
            .iter()
            .any(|change| change.relative_path == relative)
        {
            effects.append(event);
        }
    }
    Ok((staged, effects))
}

fn plan_one_code_operation(
    staged: &mut ApplyStagedState,
    effects: &mut Vec<(PathBuf, DomainEvent)>,
    authority: &CodeApplyAuthority<'_>,
    operation: &CodePlanOperation,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    let at = code_operation_at(operation);
    let at_path = format!("ops[{op_index}].args.at");
    if at.source_set() != authority.source_set_name() {
        return Err(bad_code_arg(
            &at_path,
            "at belongs to another actor-admitted source set",
        ));
    }
    let capability = authority.profile().module_capability(at).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "at does not identify one exact writable module terminal",
        )
        .at_path(&at_path)
    })?;
    let target = module_source_address(at, capability).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::ProviderUnavailable,
            "module layout is unavailable for the actor-issued profile",
        )
        .at_path(&at_path)
    })?;
    let relative = module_relative(&target, authority.source_kind()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::ProviderUnavailable,
            "module source layout is unavailable for the actor-issued source kind",
        )
        .at_path(&at_path)
    })?;
    let identity = platform_xml_module_identity(&relative).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "module layout does not round-trip through the Platform XML mapper",
        )
        .at_path(&at_path)
    })?;
    if identity.address != target {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "module layout identity does not match its logical target",
        )
        .at_path(&at_path));
    }

    let owner_bytes = prove_staged_code_owner(
        staged,
        authority.source_kind(),
        authority.expected_format(),
        &identity,
        &at_path,
    )?;
    prove_staged_code_support(
        staged,
        authority.support_policy_mode(),
        &owner_bytes,
        &at_path,
    )?;
    let before = staged
        .read(&relative)
        .map_err(|error| staged_code_error(error, &at_path))?;
    let postimage =
        plan_code_postimage(before.as_deref().unwrap_or_default(), operation, op_index)?;
    if postimage.no_op {
        return Ok(());
    }
    match before.as_deref() {
        Some(before) => staged
            .replace(&relative, before, postimage.after.clone())
            .map_err(|error| staged_code_error(error, &at_path))?,
        None => staged
            .create_leaf_below_retained_parent(&relative, postimage.after.clone())
            .map_err(|error| staged_code_error(error, &at_path))?,
    }
    if staged
        .read(&relative)
        .map_err(|error| staged_code_error(error, &at_path))?
        .as_deref()
        != Some(postimage.after.as_slice())
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "staged module postimage did not remain exact",
        )
        .at_path(at_path));
    }
    effects.push((
        relative,
        DomainEvent::new(DomainEventKind::ModuleChanged, at.to_string()),
    ));
    Ok(())
}

fn code_operation_at(operation: &CodePlanOperation) -> &crate::domain::address::QualifiedAddress {
    match operation {
        CodePlanOperation::Insert(args) => &args.at,
        CodePlanOperation::Replace(args) => &args.at,
    }
}

fn prove_staged_code_owner(
    staged: &mut ApplyStagedState,
    source_kind: SourceSetKind,
    expected_format: &str,
    identity: &PlatformXmlModuleIdentity,
    at_path: &str,
) -> Result<Vec<Vec<u8>>, ApplyPlanError> {
    let root_relative = Path::new("Configuration.xml");
    let root = staged
        .read(root_relative)
        .map_err(|error| staged_code_error(error, at_path))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "source-set owner descriptor is absent",
            )
            .at_path(at_path)
        })?;
    let mut evidence = prove_already_read_source_set_owner(root_relative, &root, source_kind)
        .map_err(|_| invalid_code_owner(at_path))?;
    require_code_owner_format(&evidence, expected_format, at_path)?;
    let mut owner_bytes = vec![root];
    let owner_parts = identity.address.as_str().split('.').collect::<Vec<_>>();
    let (owner_pairs, remainder) =
        owner_parts[..owner_parts.len().saturating_sub(1)].as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "module logical owner has an incomplete kind/name pair",
        )
        .at_path(at_path));
    }
    let mut owner_index = 0usize;
    for relative in &identity.descriptors {
        if relative == root_relative {
            continue;
        }
        let expected = owner_pairs.get(owner_index).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::Postcondition,
                "module descriptor depth exceeds its logical owner",
            )
            .at_path(at_path)
        })?;
        if !evidence.registers(expected[0], expected[1]) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "logical module owner is not registered by its parent descriptor",
            )
            .at_path(at_path));
        }
        let bytes = staged
            .read(relative)
            .map_err(|error| staged_code_error(error, at_path))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    "logical module owner descriptor is absent",
                )
                .at_path(at_path)
            })?;
        let current = prove_already_read_metadata_owner(relative, &bytes)
            .map_err(|_| invalid_code_owner(at_path))?;
        require_code_owner_format(&current, expected_format, at_path)?;
        if current.artifact_kind() != expected[0] || current.artifact_name() != Some(expected[1]) {
            return Err(invalid_code_owner(at_path));
        }
        owner_bytes.push(bytes);
        evidence = current;
        owner_index += 1;
    }
    if owner_index != owner_pairs.len() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "module descriptor chain is incomplete for its logical owner",
        )
        .at_path(at_path));
    }
    Ok(owner_bytes)
}

fn require_code_owner_format(
    evidence: &PlatformXmlSourceSetOwnerEvidence,
    expected_format: &str,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if evidence.version() == Some(expected_format) {
        Ok(())
    } else {
        Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "module owner does not match the actor-issued serialization profile",
        )
        .at_path(at_path))
    }
}

fn invalid_code_owner(at_path: &str) -> ApplyPlanError {
    ApplyPlanError::new(
        ApplyPlanErrorKind::InvalidSource,
        "logical module owner evidence is invalid",
    )
    .at_path(at_path)
}

fn prove_staged_code_support(
    staged: &mut ApplyStagedState,
    support_policy: crate::infrastructure::support_policy_evidence::SupportPolicyMode,
    owner_bytes: &[Vec<u8>],
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    let marker = staged
        .read(Path::new("Ext/ParentConfigurations.bin"))
        .map_err(|error| staged_code_error(error, at_path))?;
    if support_policy != crate::infrastructure::support_policy_evidence::SupportPolicyMode::Deny {
        return Ok(());
    }
    let state = parse_support_state_compat_bytes(marker.as_deref());
    let Some(state) = state else {
        return Ok(());
    };
    if state.removed() {
        return Ok(());
    }
    if !state.global_editing_enabled() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "actor support policy denies editing this protected source",
        )
        .at_path(at_path));
    }
    let owner_uuid = owner_bytes
        .iter()
        .rev()
        .find_map(|bytes| support_root_uuid_from_bytes(bytes));
    if owner_uuid
        .as_deref()
        .and_then(|uuid| state.object_rule(uuid))
        == Some(0)
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "actor support policy denies editing this protected module owner",
        )
        .at_path(at_path));
    }
    Ok(())
}

fn staged_code_error(error: ApplyStagingError, path: &str) -> ApplyPlanError {
    ApplyPlanError::staging(error, path)
}

struct StagedCodePostimage {
    after: Vec<u8>,
    no_op: bool,
}

fn plan_code_postimage(
    before: &[u8],
    operation: &CodePlanOperation,
    op_index: usize,
) -> Result<StagedCodePostimage, ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let text_path = format!("ops[{op_index}].args.text");
    let snapshot = SourceTextSnapshot::from_bytes(before).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "original BSL module is not a supported text snapshot",
        )
        .at_path(&at_path)
    })?;
    let indexed = analyze_module(snapshot.decoded_text()).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "original BSL module cannot be analyzed",
        )
        .at_path(&at_path)
    })?;
    if !indexed.diagnostics.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "original BSL module contains syntax errors",
        )
        .at_path(&at_path));
    }
    let (selector, content, site, no_op, after) = match operation {
        CodePlanOperation::Insert(args) => {
            if args.text.is_empty() {
                return Err(bad_code_arg(&text_path, "text must be a non-empty string"));
            }
            let selector = args.selector.as_ref().map(legacy_selector);
            let site = match (&selector, args.position) {
                (Some(selector), Some(position)) => locate_selector_typed(
                    &snapshot,
                    legacy_position(position),
                    selector,
                    &indexed.methods,
                )
                .map_err(|error| code_selector_error(error, op_index, selector))?,
                (Some(_), None) => {
                    return Err(bad_code_arg(
                        format!("ops[{op_index}].args.position"),
                        "position is required when selector is present",
                    ))
                }
                (None, Some(_)) => {
                    return Err(bad_code_arg(
                        format!("ops[{op_index}].args.position"),
                        "position is unavailable without selector",
                    ))
                }
                (None, None) => locate_module_tail(&snapshot).map_err(|_| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::InvalidSource,
                        "module tail cannot be resolved from the observed source",
                    )
                    .at_path(&at_path)
                })?,
            };
            let insertion = normalized_content(&args.text, site.eol, site.leading_separator);
            let no_op = insertion_is_present(snapshot.raw(), site, &insertion);
            let mut after = snapshot.raw().to_vec();
            if !no_op {
                after.splice(site.offset..site.offset, insertion.iter().copied());
            }
            (
                selector,
                args.text.as_str(),
                PatchSite::Insertion(site),
                no_op,
                after,
            )
        }
        CodePlanOperation::Replace(args) => {
            if args.text.is_empty() {
                return Err(bad_code_arg(&text_path, "text must be a non-empty string"));
            }
            let selector = legacy_selector(&args.selector);
            let site = locate_replacement_typed(&snapshot, &selector, &indexed.methods)
                .map_err(|error| code_selector_error(error, op_index, &selector))?;
            let replacement = normalized_replacement(&args.text, site.eol, site.trailing_eol);
            let no_op = snapshot.raw().get(site.start..site.end) == Some(replacement.as_slice());
            let mut after = snapshot.raw().to_vec();
            if !no_op {
                after.splice(site.start..site.end, replacement.iter().copied());
            }
            (
                Some(selector),
                args.text.as_str(),
                PatchSite::Replacement(site),
                no_op,
                after,
            )
        }
    };
    let postimage = std::str::from_utf8(&after).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "patched BSL module is not UTF-8",
        )
        .at_path(&text_path)
    })?;
    let post = analyze_module(postimage).map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "patched BSL module cannot be analyzed",
        )
        .at_path(&text_path)
    })?;
    if !post.diagnostics.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::Postcondition,
            "patched BSL module contains syntax errors",
        )
        .at_path(&text_path));
    }
    prove_repeat_is_noop_parts(postimage, selector.as_ref(), content, site, &post.methods)
        .map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::Postcondition,
                "patched BSL module does not prove repeat-noop behavior",
            )
            .at_path(&text_path)
        })?;
    Ok(StagedCodePostimage { after, no_op })
}

fn legacy_selector(selector: &CodeSelector) -> Selector {
    match selector {
        CodeSelector::Method(value) => Selector::Method(value.clone()),
        CodeSelector::Anchor(value) => Selector::Anchor(value.clone()),
    }
}

const fn legacy_position(position: CodePosition) -> Position {
    match position {
        CodePosition::Before => Position::Before,
        CodePosition::After => Position::After,
    }
}

fn code_selector_error(
    error: SelectorResolutionError,
    op_index: usize,
    selector: &Selector,
) -> ApplyPlanError {
    let kind = match error.cause() {
        SelectorResolutionCause::ZeroMatches => ApplyPlanErrorKind::NotFound,
        SelectorResolutionCause::MultipleMatches => ApplyPlanErrorKind::InvalidState,
        SelectorResolutionCause::InvalidSource => ApplyPlanErrorKind::InvalidSource,
    };
    let field = match selector {
        Selector::Method(_) => "method",
        Selector::Anchor(_) => "anchor",
    };
    ApplyPlanError::new(kind, "code selector cannot identify one stable BSL site")
        .at_path(format!("ops[{op_index}].args.selector.{field}"))
}

impl PatchOperation {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "insert" => Ok(Self::Insert),
            "replace" => Ok(Self::Replace),
            _ => Err("unica.code.patch supports operation `insert` or `replace`".to_string()),
        }
    }
}

/// The span a replacement overwrites, so an edit costs the selected method or
/// anchor rather than a rewrite of the whole module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplacementSite {
    start: usize,
    end: usize,
    eol: LineEnding,
    trailing_eol: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchSite {
    Insertion(InsertionSite),
    Replacement(ReplacementSite),
}

impl PatchSite {
    /// First byte the patch writes, in postimage coordinates.
    fn changed_start(self) -> usize {
        match self {
            Self::Insertion(site) => site.offset,
            Self::Replacement(site) => site.start,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InsertionSite {
    offset: usize,
    position: Position,
    eol: LineEnding,
    leading_separator: LeadingSeparator,
}

fn patch_inner(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: PatchMode,
) -> CodePatchExecution {
    match build_patch(args, context) {
        Ok(plan) => finish_patch(plan, mode, context),
        Err(error) => CodePatchExecution::failure(error, None),
    }
}

struct CodePatchPlan {
    target: CodePatchTarget,
    before: Vec<u8>,
    after: Vec<u8>,
    selector: Option<Selector>,
    content: String,
    insertion: Vec<u8>,
    site: PatchSite,
    no_op: bool,
}

fn build_patch(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<CodePatchPlan, String> {
    let target = resolve_target(args, context)?;
    // A module the platform never exported reads as no bytes at all, which is
    // exactly the preimage a first body is appended to.
    let before = if target.existed {
        fs::read(&target.path)
            .map_err(|error| format!("failed to read {}: {error}", target.path.display()))?
    } else {
        Vec::new()
    };
    let snapshot = SourceTextSnapshot::from_bytes(&before)
        .map_err(|error| format!("BSL module snapshot: {error}"))?;
    let text = snapshot.decoded_text();
    let operation = PatchOperation::parse(string_arg(args, "operation")?)?;
    let indexed = analyze_module(text)?;
    reject_parse_diagnostics(&indexed.diagnostics, "validate original BSL module")?;
    let content = string_arg(args, "content")?.to_string();
    let (selector, site, insertion, no_op, after) = match operation {
        PatchOperation::Insert => {
            let selector = Selector::parse_optional(args)?;
            let site = match &selector {
                Some(selector) => {
                    let position = Position::parse(string_arg(args, "position")?)?;
                    locate_selector(&snapshot, position, selector, &indexed.methods)?
                }
                // Without a selector there is nothing to place content relative
                // to, so `position` is refused rather than silently ignored.
                None => {
                    if args.contains_key("position") {
                        return Err(
                            "unica.code.patch does not accept `position` without a `selector`; content goes to the end of the module"
                                .to_string(),
                        );
                    }
                    locate_module_tail(&snapshot)?
                }
            };
            let insertion = normalized_content(&content, site.eol, site.leading_separator);
            let no_op = insertion_is_present(snapshot.raw(), site, &insertion);
            let mut after = snapshot.raw().to_vec();
            if !no_op {
                after.splice(site.offset..site.offset, insertion.iter().copied());
            }
            (
                selector,
                PatchSite::Insertion(site),
                insertion,
                no_op,
                after,
            )
        }
        PatchOperation::Replace => {
            let selector = Selector::parse(args)?;
            let site = locate_replacement(&snapshot, &selector, &indexed.methods)?;
            let replacement = normalized_replacement(&content, site.eol, site.trailing_eol);
            let no_op = snapshot.raw().get(site.start..site.end) == Some(replacement.as_slice());
            let mut after = snapshot.raw().to_vec();
            if !no_op {
                after.splice(site.start..site.end, replacement.iter().copied());
            }
            (
                Some(selector),
                PatchSite::Replacement(site),
                replacement,
                no_op,
                after,
            )
        }
    };
    Ok(CodePatchPlan {
        target,
        before,
        after,
        selector,
        content,
        insertion,
        site,
        no_op,
    })
}

fn finish_patch(
    plan: CodePatchPlan,
    mode: PatchMode,
    context: &WorkspaceContext,
) -> CodePatchExecution {
    let postimage = match std::str::from_utf8(&plan.after) {
        Ok(postimage) => postimage,
        Err(_) => {
            return CodePatchExecution::failure(
                "patched BSL module must remain UTF-8".to_string(),
                None,
            )
        }
    };
    let post_hash = hash(&plan.after);
    let analysis = match analyze_module(postimage) {
        Ok(analysis) => analysis,
        Err(error) => return CodePatchExecution::failure(error, None),
    };
    let validation_status = if analysis.diagnostics.is_empty() {
        ValidationStatus::Passed
    } else {
        ValidationStatus::Failed
    };
    let validation = SourceValidation {
        kind: ValidationKind::BslAnalyzerParser,
        status: validation_status,
        validated_post_hash: post_hash.clone(),
        diagnostics: analysis.diagnostics,
    };
    let data = match patch_data(&plan, postimage, post_hash, validation) {
        Ok(data) => data,
        Err(error) => return CodePatchExecution::failure(error, None),
    };
    if data.validation.status == ValidationStatus::Failed {
        let details = data
            .validation
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .take(5)
            .collect::<Vec<_>>()
            .join("; ");
        return CodePatchExecution::failure(
            format!("validate patched BSL module: {details}"),
            Some(data),
        );
    }
    if let Err(error) = prove_repeat_is_noop(postimage, &plan, &analysis.methods) {
        return CodePatchExecution::failure(error, Some(data));
    }
    if mode == PatchMode::Apply && !plan.no_op {
        let publish_result = (|| -> Result<(), String> {
            let mut transaction = CompileTransaction::new();
            if plan.target.existed {
                transaction.replace_bytes(&plan.target.path, &plan.before, plan.after.clone())?;
            } else {
                // `create_bytes` refuses if the file appeared meanwhile, so a
                // concurrent creator is not overwritten either.
                transaction.create_bytes(&plan.target.path, plan.after.clone())?;
            }
            let revalidated =
                guard_code_patch_resolved_target(&mut transaction, &plan.target.handle, context)?;
            if revalidated != plan.target.path {
                return Err("resolved code.patch target changed before publication".to_string());
            }
            guard_resolved_support(&revalidated, context)?;
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = publish_result {
            return CodePatchExecution::failure(format!("publish BSL module: {error}"), Some(data));
        }
    }
    let (edit, verb) = match plan.site {
        PatchSite::Insertion(_) => ("insertion", "inserted"),
        PatchSite::Replacement(_) => ("replacement", "replaced"),
    };
    let outcome = AdapterOutcome {
        ok: true,
        summary: if plan.no_op {
            "unica.code.patch is already applied".to_string()
        } else if mode == PatchMode::Preview {
            format!("dry run: unica.code.patch planned one {edit}")
        } else {
            format!("unica.code.patch applied one {edit}")
        },
        changes: (mode == PatchMode::Apply && !plan.no_op)
            .then(|| {
                format!(
                    "{} + {}: {verb} BSL content",
                    plan.target.resolved.source_set,
                    plan.target.metadata_path()
                )
            })
            .into_iter()
            .collect(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: vec![format!(
            "{} + {}",
            plan.target.resolved.source_set,
            plan.target.metadata_path()
        )],
        stdout: None,
        stderr: None,
        command: None,
    };
    CodePatchExecution {
        outcome,
        data: Some(data),
    }
}

fn patch_data(
    plan: &CodePatchPlan,
    postimage: &str,
    post_hash: String,
    validation: SourceValidation,
) -> Result<CodePatchData, String> {
    let changed_ranges = if plan.no_op {
        Vec::new()
    } else {
        let start = plan.site.changed_start();
        let end = start + plan.insertion.len();
        let (start_line, start_column) = line_column(postimage, start)?;
        let (end_line, end_column) = line_column(postimage, end)?;
        vec![ChangedRange {
            start_byte: start,
            end_byte: end,
            start_line,
            start_column,
            end_line,
            end_column,
        }]
    };
    let diff = if plan.no_op {
        String::new()
    } else {
        let preimage = std::str::from_utf8(&plan.before)
            .map_err(|_| "original BSL module must be UTF-8".to_string())?;
        unified_diff(plan.target.metadata_path(), preimage, postimage)?
    };
    Ok(CodePatchData {
        source_set: plan.target.resolved.source_set.clone(),
        metadata_path: plan.target.metadata_path().to_string(),
        target_kind: plan.target.resolved.target_kind,
        pre_hash: hash(&plan.before),
        post_hash: post_hash.clone(),
        no_op: plan.no_op,
        changed_ranges,
        diff,
        affected_target: AffectedTarget {
            source_set: plan.target.resolved.source_set.clone(),
            metadata_path: plan.target.metadata_path().to_string(),
            target_kind: plan.target.resolved.target_kind,
            owner: plan.target.owner.clone(),
            module_role: plan.target.module_role.clone(),
            raw_hash: post_hash,
        },
        validation,
    })
}

impl CodePatchExecution {
    fn failure(error: String, data: Option<CodePatchData>) -> Self {
        Self {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.code.patch failed".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data,
        }
    }
}

struct CodePatchTarget {
    path: PathBuf,
    resolved: ResolvedTarget,
    handle: ClosedPlatformXmlTarget,
    owner: String,
    module_role: String,
    /// Whether the module file existed when the patch was planned. The platform
    /// omits an empty object or manager module on export, so a legitimate
    /// address can point at a file that is not there yet.
    existed: bool,
}

impl CodePatchTarget {
    fn metadata_path(&self) -> &str {
        self.resolved
            .metadata_path
            .as_ref()
            .expect("code.patch resolves one module metadataPath")
            .as_str()
    }
}

fn resolve_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<CodePatchTarget, String> {
    let source_target = code_patch_source_target(args).map_err(|error| error.to_string())?;
    // `ModuleOnly` is the write surface's declaration: a descriptor or any other
    // metadata object address is refused here, not deeper.
    let resolution = resolve_platform_xml_target(
        context,
        &source_target,
        TargetKindPolicy::ModuleOnlyAllowingAbsent,
    )
    .map_err(|error| error.to_string())?;
    let path = revalidate_platform_xml_target(context, &resolution.handle)
        .map_err(|error| error.to_string())?
        .path;
    let mut authorization = CompileTransaction::new();
    let guarded_path =
        guard_code_patch_resolved_target(&mut authorization, &resolution.handle, context)?;
    if guarded_path != path {
        return Err("resolved code.patch target changed during planning".to_string());
    }
    guard_resolved_support(&path, context)?;
    let metadata_path = resolution
        .resolved
        .metadata_path
        .as_ref()
        .expect("module resolver returns metadataPath");
    let segments = metadata_path.segments().collect::<Vec<_>>();
    let (owner, module_role) = match segments.as_slice() {
        [module_role] => ("Configuration".to_string(), (*module_role).to_string()),
        [kind, name, .., module_role] => (format!("{kind}.{name}"), (*module_role).to_string()),
        _ => {
            return Err("resolved module metadataPath has an invalid shape".to_string());
        }
    };
    let existed = path.is_file();
    Ok(CodePatchTarget {
        path,
        resolved: resolution.resolved,
        handle: resolution.handle,
        owner,
        module_role,
        existed,
    })
}

#[cfg(test)]
fn locate_insertion(text: &str, args: &Map<String, Value>) -> Result<InsertionSite, String> {
    if string_arg(args, "operation")? != "insert" {
        return Err("unica.code.patch v1 supports only operation=insert".to_string());
    }
    let position = Position::parse(string_arg(args, "position")?)?;
    let selector = Selector::parse(args)?;
    let snapshot = SourceTextSnapshot::from_bytes(text.as_bytes())
        .map_err(|error| format!("BSL module snapshot: {error}"))?;
    let indexed = analyze_module(text)?;
    reject_parse_diagnostics(&indexed.diagnostics, "validate original BSL module")?;
    locate_selector(&snapshot, position, &selector, &indexed.methods)
}

fn locate_selector_typed(
    snapshot: &SourceTextSnapshot,
    position: Position,
    selector: &Selector,
    methods: &[Method],
) -> Result<InsertionSite, SelectorResolutionError> {
    reject_lone_cr_line_endings(snapshot).map_err(SelectorResolutionError::invalid_source)?;
    let text = snapshot.decoded_text();
    let offset = match selector {
        Selector::Method(name) => {
            let folded_name = name.to_lowercase();
            let found = methods
                .iter()
                .filter(|method| method.name.to_lowercase() == folded_name)
                .collect::<Vec<_>>();
            let method = match found.as_slice() {
                [method] => *method,
                [] => return Err(SelectorResolutionError::zero_matches(selector)),
                _ => {
                    return Err(SelectorResolutionError::multiple_matches(
                        selector,
                        found.len(),
                    ))
                }
            };
            match position {
                Position::Before => safe_line_start(text, method.start),
                Position::After => line_end(text, method.end),
            }
        }
        Selector::Anchor(anchor) => {
            let found = anchor_occurrences(text, anchor, methods);
            let selected = match found.as_slice() {
                [selected] => *selected,
                [] => return Err(SelectorResolutionError::zero_matches(selector)),
                _ => {
                    return Err(SelectorResolutionError::multiple_matches(
                        selector,
                        found.len(),
                    ))
                }
            };
            match position {
                Position::Before => safe_line_start(text, selected.start),
                Position::After => anchor_line_end(text, selected.end),
            }
        }
    };
    let local = local_line_ending_at(text, offset, position);
    let eol = resolve_observed_line_ending(snapshot, local).map_err(|error| {
        SelectorResolutionError::invalid_source(format!("resolve code.patch EOL: {error}"))
    })?;
    let leading_separator = if position == Position::After
        && offset == text.len()
        && !text.is_empty()
        && !text.as_bytes().ends_with(b"\n")
    {
        LeadingSeparator::LocalEol
    } else {
        LeadingSeparator::None
    };
    Ok(InsertionSite {
        offset,
        position,
        eol,
        leading_separator,
    })
}

fn locate_selector(
    snapshot: &SourceTextSnapshot,
    position: Position,
    selector: &Selector,
    methods: &[Method],
) -> Result<InsertionSite, String> {
    locate_selector_typed(snapshot, position, selector, methods).map_err(|error| error.to_string())
}

/// Resolves the one place every module has: its end.
///
/// The site is `Before` the end-of-text offset rather than `After` it, because
/// that is what makes the repeat provable. On the next identical call the offset
/// has moved to the new end, and `insertion_is_present` then asks whether the
/// text already ends with the same bytes — which is exactly the question. An
/// `After` site would look at the empty tail past the end and never match.
///
/// A module holding no method yet is not a special case here: its text is empty,
/// so the offset is zero and no separator is owed.
fn locate_module_tail(snapshot: &SourceTextSnapshot) -> Result<InsertionSite, String> {
    reject_lone_cr_line_endings(snapshot)?;
    let text = snapshot.decoded_text();
    let offset = text.len();
    let local = local_line_ending_at(text, offset, Position::Before);
    let eol = resolve_observed_line_ending(snapshot, local)
        .map_err(|error| format!("resolve code.patch EOL: {error}"))?;
    // The separator question is about content, not bytes: a module holding only
    // a byte order mark has nothing to be separated from, so it owes no blank
    // line. `text()` is the same span as `decoded_text()` without that mark.
    let content = snapshot.text();
    let leading_separator = if content.is_empty() || content.as_bytes().ends_with(b"\n") {
        LeadingSeparator::None
    } else {
        LeadingSeparator::LocalEol
    };
    Ok(InsertionSite {
        offset,
        position: Position::Before,
        eol,
        leading_separator,
    })
}

/// Resolves the span a replacement overwrites. A method selector takes the whole
/// method including the line it starts and ends on; an anchor selector takes the
/// exact occurrence, so an inline edit stays inline.
fn locate_replacement_typed(
    snapshot: &SourceTextSnapshot,
    selector: &Selector,
    methods: &[Method],
) -> Result<ReplacementSite, SelectorResolutionError> {
    reject_lone_cr_line_endings(snapshot).map_err(SelectorResolutionError::invalid_source)?;
    let text = snapshot.decoded_text();
    let (start, end) = match selector {
        Selector::Method(name) => {
            let folded_name = name.to_lowercase();
            let found = methods
                .iter()
                .filter(|method| method.name.to_lowercase() == folded_name)
                .collect::<Vec<_>>();
            let method = match found.as_slice() {
                [method] => *method,
                [] => return Err(SelectorResolutionError::zero_matches(selector)),
                _ => {
                    return Err(SelectorResolutionError::multiple_matches(
                        selector,
                        found.len(),
                    ))
                }
            };
            (
                safe_line_start(text, method.start),
                line_end(text, method.end),
            )
        }
        Selector::Anchor(anchor) => {
            let found = anchor_occurrences(text, anchor, methods);
            let selected = match found.as_slice() {
                [selected] => *selected,
                [] => return Err(SelectorResolutionError::zero_matches(selector)),
                _ => {
                    return Err(SelectorResolutionError::multiple_matches(
                        selector,
                        found.len(),
                    ))
                }
            };
            (selected.start, selected.end)
        }
    };
    let local = local_line_ending_at(text, start, Position::Before);
    let eol = resolve_observed_line_ending(snapshot, local).map_err(|error| {
        SelectorResolutionError::invalid_source(format!("resolve code.patch EOL: {error}"))
    })?;
    // The replacement keeps a trailing newline only where the span it overwrites
    // had one, so replacing an inline anchor does not break the line.
    let trailing_eol = text
        .get(..end)
        .is_some_and(|head| head.ends_with('\n') || head.ends_with('\r'));
    Ok(ReplacementSite {
        start,
        end,
        eol,
        trailing_eol,
    })
}

fn locate_replacement(
    snapshot: &SourceTextSnapshot,
    selector: &Selector,
    methods: &[Method],
) -> Result<ReplacementSite, String> {
    locate_replacement_typed(snapshot, selector, methods).map_err(|error| error.to_string())
}

fn normalized_replacement(content: &str, eol: LineEnding, trailing_eol: bool) -> Vec<u8> {
    let eol = eol.as_str();
    let mut bytes = canonicalize_eol(content).replace('\n', eol).into_bytes();
    if trailing_eol && !bytes.ends_with(eol.as_bytes()) {
        bytes.extend_from_slice(eol.as_bytes());
    }
    bytes
}

fn reject_lone_cr_line_endings(snapshot: &SourceTextSnapshot) -> Result<(), String> {
    match snapshot.line_endings() {
        LineEndingProfile::Uniform(LineEnding::Cr) | LineEndingProfile::Mixed { cr: 1.., .. } => {
            Err(
                "unica.code.patch v1 does not support source containing lone CR line endings"
                    .to_string(),
            )
        }
        LineEndingProfile::None
        | LineEndingProfile::Uniform(LineEnding::Lf | LineEnding::CrLf)
        | LineEndingProfile::Mixed { cr: 0, .. } => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnchorOccurrence {
    start: usize,
    end: usize,
}

fn anchor_occurrences(text: &str, anchor: &str, methods: &[Method]) -> Vec<AnchorOccurrence> {
    canonical_occurrences(text, anchor)
        .into_iter()
        .filter(|occurrence| {
            methods
                .iter()
                .filter(|method| {
                    occurrence.start >= method.start && occurrence.end <= line_end(text, method.end)
                })
                .count()
                == 1
        })
        .collect()
}

fn canonical_occurrences(text: &str, needle: &str) -> Vec<AnchorOccurrence> {
    if needle.is_empty() {
        return Vec::new();
    }
    text.char_indices()
        .filter_map(|(start, _)| {
            if text.as_bytes().get(start) == Some(&b'\n')
                && start > 0
                && text.as_bytes().get(start - 1) == Some(&b'\r')
            {
                return None;
            }
            canonical_match_end(text, start, needle).map(|end| AnchorOccurrence { start, end })
        })
        .collect()
}

fn canonical_match_end(text: &str, start: usize, needle: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut offset = start;
    for expected in needle.chars() {
        if expected == '\n' {
            match bytes.get(offset) {
                Some(b'\r') if bytes.get(offset + 1) == Some(&b'\n') => offset += 2,
                Some(b'\r' | b'\n') => offset += 1,
                _ => return None,
            }
            continue;
        }
        let actual = text.get(offset..)?.chars().next()?;
        if actual != expected {
            return None;
        }
        offset += actual.len_utf8();
    }
    Some(offset)
}

fn prove_repeat_is_noop(
    postimage: &str,
    plan: &CodePatchPlan,
    methods: &[Method],
) -> Result<(), String> {
    prove_repeat_is_noop_parts(
        postimage,
        plan.selector.as_ref(),
        &plan.content,
        plan.site,
        methods,
    )
}

fn prove_repeat_is_noop_parts(
    postimage: &str,
    selector: Option<&Selector>,
    content: &str,
    site: PatchSite,
    methods: &[Method],
) -> Result<(), String> {
    let snapshot = SourceTextSnapshot::from_bytes(postimage.as_bytes())
        .map_err(|error| format!("patched BSL module snapshot: {error}"))?;
    let stale =
        |error: String| format!("patch cannot be applied idempotently on the next call: {error}");
    let repeated_is_noop = match site {
        PatchSite::Insertion(site) => {
            let repeat_site = match selector {
                Some(selector) => {
                    locate_selector(&snapshot, site.position, selector, methods).map_err(stale)?
                }
                // The end of the module is still the end of the module, so the
                // repeat resolves; whether it writes is decided just below.
                None => locate_module_tail(&snapshot).map_err(stale)?,
            };
            let repeat_insertion =
                normalized_content(content, repeat_site.eol, repeat_site.leading_separator);
            insertion_is_present(postimage.as_bytes(), repeat_site, &repeat_insertion)
        }
        PatchSite::Replacement(_) => match locate_replacement(
            &snapshot,
            selector.expect("replacement has a selector"),
            methods,
        ) {
            // The edit consumed its own selector — an anchor rewritten to new
            // text, a method renamed. A repeated identical call then resolves
            // nothing and fails closed without writing, which is exactly the
            // double application the guard exists to prevent.
            Err(_) => true,
            Ok(repeat_site) => {
                let repeat_replacement =
                    normalized_replacement(content, repeat_site.eol, repeat_site.trailing_eol);
                snapshot.raw().get(repeat_site.start..repeat_site.end)
                    == Some(repeat_replacement.as_slice())
            }
        },
    };
    if repeated_is_noop {
        Ok(())
    } else {
        Err(
            "patch cannot be applied idempotently on the next call: repeated planning would change bytes"
                .to_string(),
        )
    }
}

fn guard_resolved_support(
    target: &std::path::Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    match evaluate_resolved_support_guard(target, SupportGuardRequirement::Editable, context) {
        ResolvedSupportGuardCheck::Allow | ResolvedSupportGuardCheck::Warn(_) => Ok(()),
        ResolvedSupportGuardCheck::Block(violation) => {
            Err(format!("support guard: {}", violation.reason))
        }
    }
}

#[derive(Debug)]
struct Method {
    name: String,
    start: usize,
    end: usize,
}

struct ModuleAnalysis {
    methods: Vec<Method>,
    diagnostics: Vec<ValidationDiagnostic>,
}

fn analyze_module(text: &str) -> Result<ModuleAnalysis, String> {
    if text.len() > u32::MAX as usize {
        return Err("BSL module is too large for the analyzer parser".to_string());
    }
    let parsed = bsl_parser::parse(text);
    let diagnostics = parsed
        .errors()
        .iter()
        .map(|error| validation_diagnostic(text, error))
        .collect::<Result<Vec<_>, _>>()?;
    let root = parsed.syntax_node();
    let mut methods = Vec::new();
    for node in root.descendants() {
        let method = if let Some(procedure) = ProcedureDef::cast(node.clone()) {
            method_from_ast(
                procedure
                    .name_or_keyword()
                    .map(|token| token.text().to_string()),
                procedure.syntax().text_range(),
            )
        } else if let Some(function) = FunctionDef::cast(node) {
            method_from_ast(
                function
                    .name_or_keyword()
                    .map(|token| token.text().to_string()),
                function.syntax().text_range(),
            )
        } else {
            None
        };
        if let Some(method) = method {
            methods.push(method);
        }
    }
    methods.sort_by_key(|method| method.start);
    Ok(ModuleAnalysis {
        methods,
        diagnostics,
    })
}

fn method_from_ast(name: Option<String>, range: bsl_syntax::TextRange) -> Option<Method> {
    name.map(|name| Method {
        name,
        start: text_offset(range.start()),
        end: text_offset(range.end()),
    })
}

fn text_offset(offset: bsl_syntax::TextSize) -> usize {
    u32::from(offset) as usize
}

fn validation_diagnostic(
    text: &str,
    error: &bsl_syntax::SyntaxError,
) -> Result<ValidationDiagnostic, String> {
    let range = error.range();
    let start_byte = text_offset(range.start());
    let end_byte = text_offset(range.end());
    let (start_line, start_column) = line_column(text, start_byte)?;
    let (end_line, end_column) = line_column(text, end_byte)?;
    Ok(ValidationDiagnostic {
        code: "bsl-parse-error",
        message: error.message().to_string(),
        start_byte,
        end_byte,
        start_line,
        start_column,
        end_line,
        end_column,
    })
}

fn reject_parse_diagnostics(
    diagnostics: &[ValidationDiagnostic],
    context: &str,
) -> Result<(), String> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    let details = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .take(5)
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!("{context}: {details}"))
}

fn safe_line_start(text: &str, from: usize) -> usize {
    let start = text
        .as_bytes()
        .get(..from)
        .and_then(|prefix| prefix.iter().rposition(|byte| *byte == b'\n'))
        .map_or(0, |position| position + 1);
    if start == 0 && text.as_bytes().starts_with(b"\xef\xbb\xbf") {
        3
    } else {
        start
    }
}

fn line_end(text: &str, from: usize) -> usize {
    text.as_bytes()
        .get(from..)
        .and_then(|suffix| suffix.iter().position(|byte| *byte == b'\n'))
        .map_or(text.len(), |position| from + position + 1)
}

fn anchor_line_end(text: &str, anchor_end: usize) -> usize {
    if anchor_end > 0 && text.as_bytes().get(anchor_end - 1) == Some(&b'\n') {
        anchor_end
    } else {
        line_end(text, anchor_end)
    }
}

fn line_column(text: &str, offset: usize) -> Result<(usize, usize), String> {
    let prefix = text
        .get(..offset)
        .ok_or_else(|| format!("byte offset {offset} is not a UTF-8 boundary in BSL module"))?;
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, current)| current)
        .chars()
        .count()
        + 1;
    Ok((line, column))
}

fn local_line_ending_at(text: &str, offset: usize, position: Position) -> Option<LineEnding> {
    let bytes = text.as_bytes();
    let before = bytes
        .get(..offset)
        .and_then(|prefix| prefix.iter().rposition(|byte| *byte == b'\n'))
        .map(|newline| line_ending_at_newline(bytes, newline));
    let after = bytes
        .get(offset..)
        .and_then(|suffix| suffix.iter().position(|byte| *byte == b'\n'))
        .map(|newline| line_ending_at_newline(bytes, offset + newline));
    match position {
        Position::Before => after.or(before),
        Position::After => before.or(after),
    }
}

fn line_ending_at_newline(bytes: &[u8], newline: usize) -> LineEnding {
    if newline > 0 && bytes.get(newline - 1) == Some(&b'\r') {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

fn canonicalize_eol(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

fn normalized_content(
    content: &str,
    eol: LineEnding,
    leading_separator: LeadingSeparator,
) -> Vec<u8> {
    let eol = eol.as_str();
    let normalized = canonicalize_eol(content).replace('\n', eol);
    let mut bytes = Vec::new();
    if leading_separator == LeadingSeparator::LocalEol {
        bytes.extend_from_slice(eol.as_bytes());
    }
    bytes.extend_from_slice(normalized.as_bytes());
    if !bytes.ends_with(eol.as_bytes()) {
        bytes.extend_from_slice(eol.as_bytes());
    }
    bytes
}

fn insertion_is_present(text: &[u8], site: InsertionSite, insertion: &[u8]) -> bool {
    match site.position {
        Position::Before => text
            .get(..site.offset)
            .is_some_and(|head| head.ends_with(insertion)),
        Position::After => text
            .get(site.offset..)
            .is_some_and(|tail| tail.starts_with(insertion)),
    }
}

fn string_arg<'a>(args: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("unica.code.patch requires non-empty `{name}`"))
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn unified_diff(path: &str, before: &str, after: &str) -> Result<String, String> {
    let mut options = DiffOptions::new();
    options
        .set_original_filename(format!("a/{path}"))
        .set_modified_filename(format!("b/{path}"));
    let patch = options.create_patch(before, after);
    let rendered = patch.to_string();
    let reparsed = Patch::from_str(&rendered)
        .map_err(|error| format!("generated unified diff cannot be parsed: {error}"))?;
    let rebuilt = apply(before, &reparsed)
        .map_err(|error| format!("generated unified diff cannot be applied: {error}"))?;
    if rebuilt.as_bytes() != after.as_bytes() {
        return Err("generated unified diff does not reproduce the exact postimage".to_string());
    }
    Ok(rendered)
}

#[cfg(test)]
pub(super) mod tests {
    use super::{
        analyze_module, hash, insertion_is_present, line_column, local_line_ending_at,
        locate_insertion, module_identity, normalized_content, parse_code_plan_operation,
        patch_inner, plan_code_batch, unified_diff, CodeInsertArgs, CodePlanOperation,
        CodePosition, CodeReplaceArgs, CodeSelector, LeadingSeparator, PatchMode, Position,
        ValidationStatus,
    };
    use crate::application::SupportGuardRequirement;
    use crate::domain::workspace::WorkspaceContext;
    use crate::domain::{
        events::{DomainEvent, DomainEventKind},
        project_sources::{SourceFormat, SourceProfile, SourceSetKind},
    };
    use crate::infrastructure::native_operations::apply::{
        ApplyPlanError, ApplyPlanErrorKind, ApplyStagingErrorKind, StagedFileState,
    };
    use crate::infrastructure::native_operations::common::support_guard_violation;
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use crate::infrastructure::native_operations::text_snapshot::{
        resolve_line_ending, EolPolicy, LineEnding, SourceTextSnapshot,
    };
    use crate::infrastructure::platform::testing::{
        attempt_retained_directory_replacement_for_test, create_file_link_fixture_for_test,
        path_identity_for_test, FileLinkFixtureOutcome, RetainedDirectoryReplacementOutcome,
    };
    use crate::infrastructure::support_policy_evidence::SupportPolicyMode;
    use crate::infrastructure::workspace_actor::{
        ApplyEffectDisposition, ProviderRootBinding, WorkspaceActor, WorkspaceIdentity,
        WorkspaceSourceSetInput,
    };
    use diffy::{apply, Patch};
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    const MODULE: &str = "Процедура Первая()\n    Сообщить(\"один\");\nКонецПроцедуры\n\nФункция Вторая()\n    Возврат Истина;\nКонецФункции\n";
    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn method_selector_places_after_the_complete_method() {
        let args = arguments(json!({"method": "Первая"}), "after");
        let site = locate_insertion(MODULE, &args).unwrap();
        assert!(MODULE[site.offset..].starts_with("\nФункция Вторая()"));
    }

    #[test]
    fn method_selector_is_case_insensitive_for_bsl_identifiers() {
        let args = arguments(json!({"method": "пЕРВАЯ"}), "after");

        let site = locate_insertion(MODULE, &args).unwrap();

        assert!(MODULE[site.offset..].starts_with("\nФункция Вторая()"));
    }

    #[test]
    fn method_before_keeps_bom_first_and_includes_its_annotation() {
        let module = "\u{feff}&НаКлиенте\nProcedure Run()\nEndProcedure\n";
        let args = arguments(json!({"method": "Run"}), "before");
        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, "\u{feff}".len());
        assert!(module[site.offset..].starts_with("&НаКлиенте"));
    }

    #[test]
    fn anchor_must_be_unique_and_inside_one_method() {
        let args = arguments(json!({"anchor": "Сообщить(\"один\");"}), "before");
        assert!(locate_insertion(MODULE, &args).is_ok());

        let args = arguments(json!({"anchor": "КонецПроцедуры"}), "before");
        assert!(locate_insertion(MODULE, &args).is_ok());

        let args = arguments(json!({"anchor": "отсутствует"}), "before");
        assert!(locate_insertion(MODULE, &args).is_err());

        let args = arguments(json!({"anchor": "\n\n"}), "before");
        assert!(locate_insertion(MODULE, &args).is_err());
    }

    #[test]
    fn anchor_before_uses_the_start_of_the_anchored_line() {
        let module = "Procedure Run()\n    Message(\"ok\");\nEndProcedure\n";
        let args = arguments(json!({"anchor": "Message(\"ok\");"}), "before");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Message").unwrap());
    }

    #[test]
    fn multiline_anchor_ending_with_eol_does_not_skip_the_following_line() {
        let module = "Procedure Run()\n    First();\n    Second();\n    Third();\nEndProcedure\n";
        let anchor = "    First();\n    Second();\n";
        let args = arguments(json!({"anchor": anchor}), "after");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Third();").unwrap());
        assert_eq!(site.eol, LineEnding::Lf);
    }

    #[test]
    fn multiline_anchor_matches_mixed_source_eol_and_uses_real_byte_range() {
        let module =
            "Procedure Run()\r\n    First();\n    Second();\r\n    Third();\nEndProcedure\n";
        let anchor = "    First();\n    Second();\n";
        let args = arguments(json!({"anchor": anchor}), "after");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Third();").unwrap());
        assert_eq!(site.eol, LineEnding::CrLf);
    }

    #[test]
    fn overlapping_anchor_occurrences_are_counted() {
        let module = "Procedure Run()\n    aaaa = 1;\nEndProcedure\n";
        let args = arguments(json!({"anchor": "aaa"}), "before");

        let error = locate_insertion(module, &args).unwrap_err();

        assert!(error.contains("matched 2 times"), "{error}");
    }

    #[test]
    fn anchor_cardinality_ignores_non_method_decoys() {
        let module = "// Target();\nProcedure Run()\n    Target();\nEndProcedure\n";
        let args = arguments(json!({"anchor": "Target();"}), "before");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Target();").unwrap());
    }

    #[test]
    fn mixed_eol_uses_the_target_method_line_ending() {
        let module = "Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\n";
        let args = arguments(json!({"method": "Second"}), "after");
        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.eol, LineEnding::Lf);
    }

    #[test]
    fn code_patch_rejects_lone_cr_instead_of_inventing_or_gaining_an_eol_policy() {
        let args = arguments(json!({"method": "First"}), "after");

        for module in [
            "Procedure First()\rEndProcedure\r",
            "Procedure First()\rEndProcedure\rProcedure Second()\nEndProcedure\n",
        ] {
            let error = locate_insertion(module, &args).unwrap_err();

            assert!(error.contains("lone CR line endings"), "{error}");
        }
    }

    #[test]
    fn local_eol_observation_is_resolved_by_shared_preserve_policy() {
        let snapshot = SourceTextSnapshot::from_bytes(
            b"Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\n",
        )
        .unwrap();
        let offset = snapshot.decoded_text().len();
        let local = local_line_ending_at(snapshot.decoded_text(), offset, Position::After);

        assert_eq!(
            resolve_line_ending(EolPolicy::Preserve, &snapshot, local),
            Ok(LineEnding::Lf)
        );
    }

    #[test]
    fn inserted_content_uses_local_line_ending_once() {
        assert_eq!(
            normalized_content("A\r\nB", LineEnding::Lf, LeadingSeparator::None),
            b"A\nB\n"
        );
        assert_eq!(
            normalized_content("A\nB", LineEnding::CrLf, LeadingSeparator::None),
            b"A\r\nB\r\n"
        );
        assert_eq!(
            normalized_content("A", LineEnding::Lf, LeadingSeparator::LocalEol),
            b"\nA\n"
        );
    }

    #[test]
    fn repeated_before_or_after_insertion_is_a_noop() {
        let before = b"// marker\nProcedure First()";
        let before_site = super::InsertionSite {
            offset: 10,
            position: Position::Before,
            eol: LineEnding::Lf,
            leading_separator: LeadingSeparator::None,
        };
        assert!(insertion_is_present(before, before_site, b"// marker\n"));

        let after = b"Procedure First()\n// marker\n";
        let after_site = super::InsertionSite {
            offset: after.len() - b"// marker\n".len(),
            position: Position::After,
            eol: LineEnding::Lf,
            leading_separator: LeadingSeparator::None,
        };
        assert!(insertion_is_present(after, after_site, b"// marker\n"));
    }

    #[test]
    fn analyzer_index_accepts_bom_english_case_and_common_bsl_regions() {
        let module = "\u{feff}&MyAnnotation\nprocedure Run()\n#Region R\nif true then\nMessage(\"ok\");\n#EndRegion\nendif;\nendprocedure\n";
        let analysis = analyze_module(module).unwrap();

        assert!(analysis.diagnostics.is_empty());
        assert_eq!(analysis.methods.len(), 1);
        assert_eq!(analysis.methods[0].name, "Run");
        assert_eq!(analysis.methods[0].start, "\u{feff}".len());
    }

    #[test]
    fn analyzer_index_ignores_declaration_words_in_comments_and_strings() {
        let module =
            "// Procedure Fake()\nValue = \"Function AlsoFake()\";\nProcedure Run()\nEndProcedure\n";
        let analysis = analyze_module(module).unwrap();

        assert!(analysis.diagnostics.is_empty());
        assert_eq!(analysis.methods.len(), 1);
        assert_eq!(analysis.methods[0].name, "Run");
    }

    #[test]
    fn analyzer_rejects_invalid_control_flow_and_unclosed_methods() {
        let invalid_if =
            analyze_module("Procedure Run()\nIf True Then\nMessage(\"bad\");\nEndProcedure\n")
                .unwrap();
        assert!(!invalid_if.diagnostics.is_empty());

        let unclosed = analyze_module("Procedure Run()\n").unwrap();
        assert!(!unclosed.diagnostics.is_empty());
    }

    #[test]
    fn line_column_reports_utf8_character_columns() {
        assert_eq!(line_column("Процедура Run()\n", 0).unwrap(), (1, 1));
        assert_eq!(
            line_column("Процедура Run()\n", "Процедура ".len()).unwrap(),
            (1, 11)
        );
        assert_eq!(line_column("A\nBC", 3).unwrap(), (2, 2));
        assert!(line_column("Я", 1).is_err());
    }

    #[test]
    fn unified_diff_round_trips_crlf_and_missing_terminal_eol() {
        let before = "Procedure Run()\r\nEndProcedure";
        let after = "Procedure Run()\r\nEndProcedure\r\nProcedure Added()\r\nEndProcedure\r\n";
        let diff = unified_diff("src/CommonModules/X/Ext/Module.bsl", before, after).unwrap();
        let patch = Patch::from_str(&diff).unwrap();
        let rebuilt = apply(before, &patch).unwrap();

        assert_eq!(rebuilt.as_bytes(), after.as_bytes());
        assert!(diff.contains("\\ No newline at end of file"));
        assert!(diff.starts_with("--- a/src/CommonModules/X/Ext/Module.bsl\n"));
    }

    #[test]
    fn emitted_diff_is_accepted_by_git_and_reproduces_postimage() {
        let root = temp_root("git-diff-roundtrip");
        let relative = "src/CommonModules/X/Ext/Module.bsl";
        let target = root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\r\nEndProcedure";
        let after = b"Procedure Run()\r\nEndProcedure\r\nProcedure Added()\r\nEndProcedure\r\n";
        fs::write(&target, before).unwrap();
        let diff = unified_diff(
            relative,
            std::str::from_utf8(before).unwrap(),
            std::str::from_utf8(after).unwrap(),
        )
        .unwrap();
        fs::write(root.join("change.diff"), diff).unwrap();
        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["config", "core.autocrlf", "false"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["apply", "--check", "change.diff"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["apply", "change.diff"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        assert_eq!(fs::read(&target).unwrap(), after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    pub(crate) fn applied_patch_returns_typed_data_and_repeated_apply_is_noop_with_stable_identity()
    {
        use crate::infrastructure::platform::testing::file_identity_for_test;

        let context = temp_context("applied-patch");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n// untouched suffix\r\n";
        let expected = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\nProcedure Added()\r\nEndProcedure\r\n// untouched suffix\r\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);
        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        assert_eq!(fs::read(&module).unwrap(), before);

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(fs::read(&module).unwrap(), expected);
        assert!(applied.outcome.stdout.is_none());
        let data = applied.data.unwrap();
        assert_eq!(data.source_set, "main");
        assert_eq!(data.affected_target.owner, "CommonModule.Sample");
        assert_eq!(data.affected_target.module_role, "Module");
        assert_eq!(data.validation.status, ValidationStatus::Passed);
        assert_eq!(data.validation.validated_post_hash, data.post_hash);
        assert_eq!(data.changed_ranges[0].start_line, 3);
        assert!(data.diff.starts_with("--- a/"));
        let identity = file_identity_for_test(&module).unwrap();

        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
        assert_eq!(fs::read(&module).unwrap(), expected);
        assert_eq!(file_identity_for_test(&module).unwrap(), identity);
        assert!(repeated.outcome.changes.is_empty());
        let data = repeated.data.unwrap();
        assert_eq!(data.pre_hash, data.post_hash);
        assert!(data.changed_ranges.is_empty());
        assert!(data.diff.is_empty());
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_configuration_and_extension_preview_apply_preserve_unrelated_bytes() {
        let root = temp_root("code-patch-config-extension");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: cfg\n",
                "  - name: addOn\n",
                "    type: EXTENSION\n",
                "    path: ext\n",
            ),
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let before = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n// untouched\r\n";
        let expected = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\nProcedure Added()\r\nEndProcedure\r\n// untouched\r\n";
        let mut protected = Vec::new();
        for source_root in ["cfg", "ext"] {
            let source_root = root.join(source_root);
            let module = source_root.join("CommonModules/Shared/Ext/Module.bsl");
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(
                source_root.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(
                source_root.join("CommonModules/Shared.xml"),
                b"\xef\xbb\xbf<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><CommonModule><Properties><Name>Shared</Name></Properties></CommonModule></MetaDataObject>\r\n",
            )
            .unwrap();
            fs::write(source_root.join("unrelated.bin"), [0, 1, 2, 255]).unwrap();
            fs::write(&module, before).unwrap();
            protected.push(source_root.join("Configuration.xml"));
            protected.push(source_root.join("CommonModules/Shared.xml"));
            protected.push(source_root.join("unrelated.bin"));
        }
        let protected_before = protected
            .iter()
            .map(|path| (path.clone(), fs::read(path).unwrap()))
            .collect::<Vec<_>>();

        for (source_set, source_root) in [("main", "cfg"), ("addOn", "ext")] {
            let module = root
                .join(source_root)
                .join("CommonModules/Shared/Ext/Module.bsl");
            let args = patch_args(
                source_set,
                "CommonModule.Shared.Module",
                "Run",
                "Procedure Added()\nEndProcedure",
            );

            let preview = patch_inner(&args, &context, PatchMode::Preview);
            assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
            assert_eq!(fs::read(&module).unwrap(), before);
            let preview_data = serde_json::to_value(preview.data.unwrap()).unwrap();
            assert_eq!(preview_data["sourceSet"], source_set);
            assert_eq!(preview_data["metadataPath"], "CommonModule.Shared.Module");
            assert_eq!(preview_data["targetKind"], "module");

            let applied = patch_inner(&args, &context, PatchMode::Apply);
            assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
            assert_eq!(fs::read(&module).unwrap(), expected);

            let repeated = patch_inner(&args, &context, PatchMode::Apply);
            assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
            assert!(repeated.data.unwrap().no_op);
            assert_eq!(fs::read(&module).unwrap(), expected);
        }
        for (path, bytes) in protected_before {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn code_patch_data_has_an_exact_stable_serialization_contract() {
        let context = temp_context("typed-serialization");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = "Procedure Run()\nEndProcedure\n";
        let inserted = "Procedure Added()\nEndProcedure\n";
        let after = format!("{before}{inserted}");
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);

        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        let serialized = serde_json::to_value(preview.data.unwrap()).unwrap();
        let pre_hash = hash(before.as_bytes());
        let post_hash = hash(after.as_bytes());
        let diff = concat!(
            "--- a/CommonModule.Sample.Module\n",
            "+++ b/CommonModule.Sample.Module\n",
            "@@ -1,2 +1,4 @@\n",
            " Procedure Run()\n",
            " EndProcedure\n",
            "+Procedure Added()\n",
            "+EndProcedure\n",
        );
        assert_eq!(
            serialized,
            json!({
                "sourceSet": "main",
                "metadataPath": "CommonModule.Sample.Module",
                "targetKind": "module",
                "preHash": pre_hash,
                "postHash": post_hash.clone(),
                "noOp": false,
                "changedRanges": [{
                    "startByte": before.len(),
                    "endByte": after.len(),
                    "startLine": 3,
                    "startColumn": 1,
                    "endLine": 5,
                    "endColumn": 1
                }],
                "diff": diff,
                "affectedTarget": {
                    "sourceSet": "main",
                    "metadataPath": "CommonModule.Sample.Module",
                    "targetKind": "module",
                    "owner": "CommonModule.Sample",
                    "moduleRole": "Module",
                    "rawHash": post_hash.clone()
                },
                "validation": {
                    "kind": "bsl-analyzer-parser",
                    "status": "passed",
                    "validatedPostHash": post_hash,
                    "diagnostics": []
                }
            })
        );
        let serialized_text = serialized.to_string();
        assert!(!serialized_text.contains("src/CommonModules"));
        assert!(!serialized_text.contains("Module.bsl"));
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn anchor_before_preserves_indentation_byte_for_byte() {
        let context = temp_context("anchor-indentation");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(
            &module,
            "Procedure Run()\n    Message(\"old\");\nEndProcedure\n",
        )
        .unwrap();
        let args = patch_args_for_selector(
            "main",
            "CommonModule.Sample.Module",
            json!({"anchor": "Message(\"old\");"}),
            "before",
            "    Message(\"new\");",
        );

        let applied = patch_inner(&args, &context, PatchMode::Apply);

        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read_to_string(&module).unwrap(),
            "Procedure Run()\n    Message(\"new\");\n    Message(\"old\");\nEndProcedure\n"
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn closing_token_anchor_matches_crlf_and_repeats_as_noop() {
        let context = temp_context("closing-token-anchor");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = concat!(
            "Процедура Цель()\r\n",
            "\tЕсли Истина Тогда\r\n",
            "\tКонецЕсли;\r\n",
            "КонецПроцедуры\r\n",
            "// frame\r\n"
        );
        fs::write(&module, before).unwrap();
        let args = patch_args_for_selector(
            "main",
            "CommonModule.Sample.Module",
            json!({"anchor": "\tКонецЕсли;\nКонецПроцедуры\n"}),
            "after",
            "// inserted",
        );

        let applied = patch_inner(&args, &context, PatchMode::Apply);

        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read_to_string(&module).unwrap(),
            before.replacen(
                "КонецПроцедуры\r\n// frame",
                "КонецПроцедуры\r\n// inserted\r\n// frame",
                1
            )
        );
        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
        assert!(repeated.data.unwrap().no_op);
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn patch_rejects_content_that_would_break_anchor_idempotence() {
        let context = temp_context("unstable-anchor");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\n    Target();\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args_for_selector(
            "main",
            "CommonModule.Sample.Module",
            json!({"anchor": "Target();"}),
            "before",
            "    Target();",
        );

        let result = patch_inner(&args, &context, PatchMode::Apply);

        assert!(!result.outcome.ok);
        assert!(result.outcome.errors[0].contains("cannot be applied idempotently"));
        assert_eq!(fs::read(&module).unwrap(), before);
        assert_eq!(
            result.data.unwrap().validation.status,
            ValidationStatus::Passed
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn patch_rejects_content_that_duplicates_the_selected_method() {
        let context = temp_context("unstable-method");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Run()\nEndProcedure",
        );

        let result = patch_inner(&args, &context, PatchMode::Apply);

        assert!(!result.outcome.ok);
        assert!(result.outcome.errors[0].contains("cannot be applied idempotently"));
        assert_eq!(fs::read(&module).unwrap(), before);
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn method_after_at_eof_inserts_a_separator_and_is_idempotent() {
        let context = temp_context("missing-terminal-eol");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure").unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read_to_string(&module).unwrap(),
            "Procedure Run()\nEndProcedure\nProcedure Added()\nEndProcedure\n"
        );
        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeated.outcome.ok);
        assert!(repeated.data.unwrap().no_op);
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    fn replace_args(metadata_path: &str, selector: Value, content: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("sourceSet".to_string(), json!("main"));
        args.insert("metadataPath".to_string(), json!(metadata_path));
        args.insert("operation".to_string(), json!("replace"));
        args.insert("selector".to_string(), selector);
        args.insert("content".to_string(), json!(content));
        args
    }

    #[test]
    fn code_patch_replaces_one_method_and_leaves_its_neighbours_untouched() {
        let context = temp_context("replace-method");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = "Процедура Первая()\r\nКонецПроцедуры\r\n\
                      Процедура Цель()\r\n\tСтарое = 1;\r\nКонецПроцедуры\r\n\
                      Процедура Третья()\r\nКонецПроцедуры\r\n";
        fs::write(&module, before).unwrap();
        let args = replace_args(
            "CommonModule.Sample.Module",
            json!({"method": "Цель"}),
            "Процедура Цель()\n\tНовое = 2;\nКонецПроцедуры",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);
        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        assert_eq!(fs::read_to_string(&module).unwrap(), before);

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        let after = fs::read_to_string(&module).unwrap();
        assert_eq!(
            after,
            "Процедура Первая()\r\nКонецПроцедуры\r\n\
             Процедура Цель()\r\n\tНовое = 2;\r\nКонецПроцедуры\r\n\
             Процедура Третья()\r\nКонецПроцедуры\r\n",
            "only the selected method changes and the source CRLF survives"
        );

        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
        assert!(repeated.data.unwrap().no_op);
        assert_eq!(fs::read_to_string(&module).unwrap(), after);
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_replace_that_consumes_its_selector_cannot_apply_twice() {
        let context = temp_context("replace-consumes-selector");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Процедура Цель()\nКонецПроцедуры\n").unwrap();
        let args = replace_args(
            "CommonModule.Sample.Module",
            json!({"method": "Цель"}),
            "Процедура Переименована()\nКонецПроцедуры",
        );

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        let after = fs::read_to_string(&module).unwrap();
        assert_eq!(after, "Процедура Переименована()\nКонецПроцедуры\n");

        // The selector no longer resolves, so the repeat fails closed rather
        // than writing the edit a second time.
        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(!repeated.outcome.ok);
        assert_eq!(
            fs::read_to_string(&module).unwrap(),
            after,
            "a refused repeat must leave the module byte-identical"
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_replace_keeps_an_anchor_edit_inline() {
        let context = temp_context("replace-anchor");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(
            &module,
            "Процедура Цель()\n\tЗначение = 1;\nКонецПроцедуры\n",
        )
        .unwrap();
        let args = replace_args(
            "CommonModule.Sample.Module",
            json!({"anchor": "Значение = 1;"}),
            "Значение = 42;",
        );

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read_to_string(&module).unwrap(),
            "Процедура Цель()\n\tЗначение = 42;\nКонецПроцедуры\n",
            "an inline anchor replacement must not add a line break"
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_without_any_source_eol_uses_lf_for_preview_apply_and_repeat_noop() {
        let context = temp_context("no-source-eol");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = "Процедура Тест() КонецПроцедуры";
        let expected = "Процедура Тест() КонецПроцедуры\nПроцедура Добавлена() КонецПроцедуры\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Тест",
            "Процедура Добавлена() КонецПроцедуры",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);
        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        assert_eq!(fs::read_to_string(&module).unwrap(), before);

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(fs::read_to_string(&module).unwrap(), expected);

        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
        assert!(repeated.data.unwrap().no_op);
        assert_eq!(fs::read_to_string(&module).unwrap(), expected);
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_preserves_two_leading_boms_for_preview_apply_and_repeat_noop() {
        let context = temp_context("two-leading-boms");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"\xef\xbb\xbf\xef\xbb\xbfProcedure Run()\nEndProcedure\n";
        let expected = b"\xef\xbb\xbf\xef\xbb\xbfProcedure Run()\nEndProcedure\nProcedure Added()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);
        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        assert_eq!(fs::read(&module).unwrap(), before);

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(fs::read(&module).unwrap(), expected);

        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
        assert!(repeated.data.unwrap().no_op);
        assert_eq!(fs::read(&module).unwrap(), expected);
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn mixed_eol_apply_preserves_untouched_bytes_and_uses_target_eol() {
        let context = temp_context("mixed-eol");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Second",
            "Procedure Added()\r\nEndProcedure",
        );

        let applied = patch_inner(&args, &context, PatchMode::Apply);

        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read(&module).unwrap(),
            b"Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\nProcedure Added()\nEndProcedure\n"
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_observed_eol_policy_is_closed() {
        code_patch_rejects_lone_cr_instead_of_inventing_or_gaining_an_eol_policy();
        code_patch_without_any_source_eol_uses_lf_for_preview_apply_and_repeat_noop();
        mixed_eol_apply_preserves_untouched_bytes_and_uses_target_eol();
        unified_diff_round_trips_crlf_and_missing_terminal_eol();
    }

    #[test]
    fn object_and_manager_modules_report_owner_and_role() {
        let context = temp_context("module-roles");
        fs::create_dir_all(context.workspace_root.join("src/Catalogs")).unwrap();
        fs::write(
            context.workspace_root.join("src/Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        for role in ["ObjectModule", "ManagerModule"] {
            let relative = format!("src/Catalogs/Items/Ext/{role}.bsl");
            let module = context.workspace_root.join(&relative);
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
            let args = patch_args(
                "main",
                &format!("Catalog.Items.{role}"),
                "Run",
                "Procedure Added()\nEndProcedure",
            );

            let preview = patch_inner(&args, &context, PatchMode::Preview);
            assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
            let target = preview.data.unwrap().affected_target;
            assert_eq!(target.owner, "Catalog.Items");
            assert_eq!(target.module_role, role);
        }
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn canonical_module_layouts_have_stable_owner_and_role() {
        let direct_cases = [
            (
                "Ext/ManagedApplicationModule.bsl",
                "Configuration",
                "ManagedApplicationModule",
            ),
            (
                "Ext/OrdinaryApplicationModule.bsl",
                "Configuration",
                "OrdinaryApplicationModule",
            ),
            ("Ext/SessionModule.bsl", "Configuration", "SessionModule"),
            (
                "Ext/ExternalConnectionModule.bsl",
                "Configuration",
                "ExternalConnectionModule",
            ),
            (
                "CommonModules/Service/Ext/Module.bsl",
                "CommonModule.Service",
                "Module",
            ),
            (
                "HTTPServices/Api/Ext/Module.bsl",
                "HTTPService.Api",
                "Module",
            ),
            ("WebServices/Api/Ext/Module.bsl", "WebService.Api", "Module"),
            (
                "IntegrationServices/Bus/Ext/Module.bsl",
                "IntegrationService.Bus",
                "Module",
            ),
            (
                "CommonForms/Main/Ext/Form/Module.bsl",
                "CommonForm.Main",
                "FormModule",
            ),
            (
                "CommonCommands/Print/Ext/CommandModule.bsl",
                "CommonCommand.Print",
                "CommandModule",
            ),
            (
                "Catalogs/Items/Ext/ObjectModule.bsl",
                "Catalog.Items",
                "ObjectModule",
            ),
            (
                "DocumentJournals/Sales/Ext/ManagerModule.bsl",
                "DocumentJournal.Sales",
                "ManagerModule",
            ),
            (
                "FilterCriteria/ByPartner/Ext/ManagerModule.bsl",
                "FilterCriterion.ByPartner",
                "ManagerModule",
            ),
            (
                "SettingsStorages/Ui/Ext/ManagerModule.bsl",
                "SettingsStorage.Ui",
                "ManagerModule",
            ),
            (
                "InformationRegisters/Prices/Ext/RecordSetModule.bsl",
                "InformationRegister.Prices",
                "RecordSetModule",
            ),
            (
                "Constants/Mode/Ext/ValueManagerModule.bsl",
                "Constant.Mode",
                "ValueManagerModule",
            ),
        ];
        for (path, owner, role) in direct_cases {
            assert_module_identity(path, owner, role);
        }

        let nested_kinds = [
            ("Catalogs", "Catalog"),
            ("Documents", "Document"),
            ("ExchangePlans", "ExchangePlan"),
            ("ChartsOfAccounts", "ChartOfAccounts"),
            ("ChartsOfCharacteristicTypes", "ChartOfCharacteristicTypes"),
            ("ChartsOfCalculationTypes", "ChartOfCalculationTypes"),
            ("BusinessProcesses", "BusinessProcess"),
            ("Tasks", "Task"),
            ("Reports", "Report"),
            ("DataProcessors", "DataProcessor"),
            ("InformationRegisters", "InformationRegister"),
            ("AccumulationRegisters", "AccumulationRegister"),
            ("AccountingRegisters", "AccountingRegister"),
            ("CalculationRegisters", "CalculationRegister"),
            ("DocumentJournals", "DocumentJournal"),
            ("Enums", "Enum"),
            ("Constants", "Constant"),
            ("Sequences", "Sequence"),
            ("DocumentNumerators", "DocumentNumerator"),
        ];
        for (directory, tag) in nested_kinds {
            assert_module_identity(
                &format!("{directory}/Owner/Forms/Main/Ext/Form/Module.bsl"),
                &format!("{tag}.Owner"),
                "FormModule",
            );
            assert_module_identity(
                &format!("{directory}/Owner/Commands/Print/Ext/CommandModule.bsl"),
                &format!("{tag}.Owner"),
                "CommandModule",
            );
        }
    }

    #[test]
    fn noncanonical_or_unsupported_module_layouts_are_rejected() {
        for path in [
            "Catalogs/Items/Trash/Ext/FakeModule.bsl",
            "Catalogs/Items/Ext/Module.bsl",
            "CommonModules/X/Ext/ObjectModule.bsl",
            "Languages/Ru/Ext/ManagerModule.bsl",
            "Catalogs/Items/Forms/Main/Ext/Module.bsl",
            "Catalogs/Items/Commands/Print/Ext/Module.bsl",
            "Catalogs/Items/Ext/FakeModule.bsl",
        ] {
            let error = module_identity(Path::new(path)).unwrap_err();
            assert!(error.contains("supported canonical"), "{path}: {error}");
        }
    }

    #[test]
    fn nested_form_and_command_modules_report_the_metadata_owner_and_role() {
        let context = temp_context("nested-module-roles");
        let src = context.workspace_root.join("src");
        fs::create_dir_all(src.join("Catalogs/Items/Forms/Main/Ext/Form")).unwrap();
        fs::create_dir_all(src.join("Catalogs/Items/Commands/Print/Ext")).unwrap();
        fs::write(
            src.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            src.join("Catalogs/Items/Forms/Main.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            src.join("Catalogs/Items/Commands/Print.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Command><Properties><Name>Print</Name></Properties></Command></MetaDataObject>"#,
        )
        .unwrap();
        let cases = [
            (
                "src/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
                "Catalog.Items.Form.Main.FormModule",
                "FormModule",
            ),
            (
                "src/Catalogs/Items/Commands/Print/Ext/CommandModule.bsl",
                "Catalog.Items.Command.Print.CommandModule",
                "CommandModule",
            ),
        ];
        for (relative, metadata_path, role) in cases {
            let module = context.workspace_root.join(relative);
            fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
            let args = patch_args(
                "main",
                metadata_path,
                "Run",
                "Procedure Added()\nEndProcedure",
            );

            let preview = patch_inner(&args, &context, PatchMode::Preview);

            assert!(
                preview.outcome.ok,
                "{relative}: {:?}",
                preview.outcome.errors
            );
            let target = preview.data.unwrap().affected_target;
            assert_eq!(target.owner, "Catalog.Items");
            assert_eq!(target.module_role, role);
        }
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_requires_descriptor_evidence_and_a_proven_module_role() {
        let context = temp_context("target-contract");

        let missing_descriptor_module = context
            .workspace_root
            .join("src/CommonModules/Missing/Ext/Module.bsl");
        fs::create_dir_all(missing_descriptor_module.parent().unwrap()).unwrap();
        fs::write(
            &missing_descriptor_module,
            "Procedure Run()\nEndProcedure\n",
        )
        .unwrap();
        let missing_descriptor = patch_args(
            "main",
            "CommonModule.Missing.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        let missing_result = patch_inner(&missing_descriptor, &context, PatchMode::Preview);
        assert!(!missing_result.outcome.ok);
        assert!(missing_result.outcome.errors[0].contains("metadata owner evidence"));

        fs::create_dir_all(context.workspace_root.join("src/Languages/Ru/Ext")).unwrap();
        fs::write(
            context.workspace_root.join("src/Languages/Ru.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();
        fs::write(
            context
                .workspace_root
                .join("src/Languages/Ru/Ext/ManagerModule.bsl"),
            "Procedure Run()\nEndProcedure\n",
        )
        .unwrap();
        let unsupported_role = patch_args(
            "main",
            "Language.Ru.ManagerModule",
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        let unsupported_result = patch_inner(&unsupported_role, &context, PatchMode::Preview);
        assert!(!unsupported_result.outcome.ok);
        assert!(
            unsupported_result.outcome.errors[0].contains("supported canonical"),
            "{:?}",
            unsupported_result.outcome.errors
        );

        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    /// The resolver also answers metadata object addresses for the read-only
    /// surface. `code.patch` must keep refusing them by declaration, so a
    /// descriptor can never become a write target.
    #[test]
    fn code_patch_refuses_a_metadata_object_address() {
        let context = temp_context("object-address");
        let descriptor = context.workspace_root.join("src/Catalogs/Items.xml");
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        let before = fs::read(&descriptor).unwrap();

        let result = patch_inner(
            &patch_args(
                "main",
                "Catalog.Items",
                "Run",
                "Procedure Added()\nEndProcedure",
            ),
            &context,
            PatchMode::Preview,
        );

        assert!(!result.outcome.ok);
        assert!(
            result.outcome.errors[0].contains("module terminal"),
            "{:?}",
            result.outcome.errors
        );
        assert_eq!(fs::read(&descriptor).unwrap(), before);

        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    /// The public writer refuses an absent address before the resolver is asked,
    /// and the resolver's `ModuleOnly` policy refuses the source root that such
    /// an address would name. Neither barrier lets a write plan see the root.
    #[test]
    fn code_patch_refuses_a_missing_or_empty_metadata_path_without_touching_the_workspace() {
        let context = temp_context("absent-address");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
        let module_before = fs::read(&module).unwrap();
        let root_before = fs::read(context.workspace_root.join("src/Configuration.xml")).unwrap();

        for (label, address) in [
            ("missing", None),
            ("empty", Some("")),
            ("blank", Some("  ")),
        ] {
            let mut args = patch_args(
                "main",
                "CommonModule.Sample.Module",
                "Run",
                "Procedure Added()\nEndProcedure",
            );
            match address {
                Some(value) => {
                    args.insert("metadataPath".to_string(), json!(value));
                }
                None => {
                    args.remove("metadataPath");
                }
            }

            for mode in [PatchMode::Preview, PatchMode::Apply] {
                let result = patch_inner(&args, &context, mode);

                assert!(!result.outcome.ok, "{label} address must not be accepted");
                assert!(
                    result.outcome.errors[0]
                        .contains("metadataPath must identify one existing module"),
                    "{label}: {:?}",
                    result.outcome.errors
                );
                assert!(
                    result.data.is_none(),
                    "{label} address must publish no plan"
                );
            }
        }

        assert_eq!(fs::read(&module).unwrap(), module_before);
        assert_eq!(
            fs::read(context.workspace_root.join("src/Configuration.xml")).unwrap(),
            root_before
        );

        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_symlink_module_is_rejected_during_preview() {
        let context = temp_context("symlink-target");
        let real = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/RealModule.bsl");
        let target = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, "Procedure Run()\nEndProcedure\n").unwrap();
        let outcome = create_file_link_fixture_for_test(&real, &target)
            .expect("unexpected file-link creation error must fail the fixture test");
        match outcome {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported => {
                fs::remove_dir_all(&context.workspace_root).unwrap();
                return;
            }
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&context.workspace_root).unwrap();
                return;
            }
        }
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let result = patch_inner(&args, &context, PatchMode::Preview);

        assert!(!result.outcome.ok);
        assert!(result.outcome.errors[0].contains("containment"));
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_invalid_postimage_returns_failed_validation_without_writing() {
        let context = temp_context("validation-failure");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\n    Message(\"ok\");\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args_for_selector(
            "main",
            "CommonModule.Sample.Module",
            json!({"anchor": "Message(\"ok\");"}),
            "after",
            "    If True Then",
        );

        let result = patch_inner(&args, &context, PatchMode::Apply);

        assert!(!result.outcome.ok);
        assert_eq!(fs::read(&module).unwrap(), before);
        let data = result.data.unwrap();
        let serialized = serde_json::to_value(&data).unwrap();
        let validation = data.validation;
        assert_eq!(validation.status, ValidationStatus::Failed);
        assert!(!validation.diagnostics.is_empty());
        let diagnostic = serialized["validation"]["diagnostics"][0]
            .as_object()
            .unwrap();
        assert_eq!(
            diagnostic.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "code",
                "message",
                "startByte",
                "endByte",
                "startLine",
                "startColumn",
                "endLine",
                "endColumn"
            ]
        );
        assert_eq!(serialized["validation"]["status"], "failed");
        assert_eq!(serialized["validation"]["kind"], "bsl-analyzer-parser");
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    pub(crate) fn code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat() {
        let context = temp_context("tail-append");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = "Procedure Run()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = tail_args(
            "main",
            "CommonModule.Sample.Module",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);
        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        assert_eq!(fs::read(&module).unwrap(), before.as_bytes());

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        let expected = "Procedure Run()\nEndProcedure\nProcedure Added()\nEndProcedure\n";
        assert_eq!(fs::read(&module).unwrap(), expected.as_bytes());

        // The end of the module is still addressable afterwards, so the repeat
        // is a proven no-op rather than a refusal after the write landed.
        let repeat = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeat.outcome.ok, "{:?}", repeat.outcome.errors);
        assert!(repeat.outcome.summary.contains("already applied"));
        assert_eq!(fs::read(&module).unwrap(), expected.as_bytes());
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    pub(crate) fn code_patch_creates_a_module_file_the_platform_never_exported() {
        let context = temp_context("tail-absent");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        assert!(!module.exists());
        let args = tail_args(
            "main",
            "CommonModule.Sample.Module",
            "Procedure Run()\nEndProcedure",
        );

        // Preview stays read-only: an absent file is not created to be shown.
        let preview = patch_inner(&args, &context, PatchMode::Preview);
        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        assert!(!module.exists(), "preview created the module file");
        let data = preview.data.unwrap();
        assert_eq!(data.pre_hash, hash(b""));
        assert_eq!(data.validation.status, ValidationStatus::Passed);

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read(&module).unwrap(),
            b"Procedure Run()\nEndProcedure\n"
        );

        // Once materialised the module is ordinary, so the repeat is the same
        // proven no-op as for a module that was exported all along.
        let repeat = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeat.outcome.ok, "{:?}", repeat.outcome.errors);
        assert_eq!(
            fs::read(&module).unwrap(),
            b"Procedure Run()\nEndProcedure\n"
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    pub(crate) fn code_patch_refuses_a_module_role_the_metadata_kind_never_owns() {
        let context = temp_context("tail-absent-role");
        // A common module owns `Module`; it never owns an object module, so the
        // absent file is not an omitted empty one and must stay unaddressable.
        let args = tail_args(
            "main",
            "CommonModule.Sample.ObjectModule",
            "Procedure Run()\nEndProcedure",
        );
        let refused = patch_inner(&args, &context, PatchMode::Apply);
        assert!(!refused.outcome.ok);
        assert!(!context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/ObjectModule.bsl")
            .exists());
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    pub(crate) fn code_patch_writes_the_first_body_of_an_empty_or_bom_only_module() {
        for (label, before, expected) in [
            (
                "bom-only",
                b"\xef\xbb\xbf".to_vec(),
                b"\xef\xbb\xbfProcedure Run()\nEndProcedure\n".to_vec(),
            ),
            (
                "byte-empty",
                Vec::new(),
                b"Procedure Run()\nEndProcedure\n".to_vec(),
            ),
        ] {
            let context = temp_context(&format!("tail-first-{label}"));
            let module = context
                .workspace_root
                .join("src/CommonModules/Sample/Ext/Module.bsl");
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(&module, &before).unwrap();
            let args = tail_args(
                "main",
                "CommonModule.Sample.Module",
                "Procedure Run()\nEndProcedure",
            );

            let preview = patch_inner(&args, &context, PatchMode::Preview);
            assert!(preview.outcome.ok, "{label}: {:?}", preview.outcome.errors);
            assert_eq!(fs::read(&module).unwrap(), before, "{label} preview wrote");
            let data = preview.data.unwrap();
            assert_eq!(data.pre_hash, hash(&before), "{label}");
            assert_eq!(data.post_hash, hash(&expected), "{label}");
            assert_eq!(data.validation.status, ValidationStatus::Passed, "{label}");

            let applied = patch_inner(&args, &context, PatchMode::Apply);
            assert!(applied.outcome.ok, "{label}: {:?}", applied.outcome.errors);
            // No blank line is owed: a byte order mark is not content.
            assert_eq!(fs::read(&module).unwrap(), expected, "{label}");

            let repeat = patch_inner(&args, &context, PatchMode::Apply);
            assert!(repeat.outcome.ok, "{label}: {:?}", repeat.outcome.errors);
            assert_eq!(fs::read(&module).unwrap(), expected, "{label} repeat wrote");
            fs::remove_dir_all(&context.workspace_root).unwrap();
        }
    }

    #[test]
    fn code_patch_separates_an_appended_method_from_a_module_without_a_trailing_eol() {
        let context = temp_context("tail-separator");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure").unwrap();
        let args = tail_args(
            "main",
            "CommonModule.Sample.Module",
            "Procedure Added()\nEndProcedure",
        );

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(
            fs::read(&module).unwrap(),
            b"Procedure Run()\nEndProcedure\nProcedure Added()\nEndProcedure\n"
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    /// #420. The writer must not be the source of a dirty diff: content that
    /// carries no trailing whitespace must not gain any on the way in, and the
    /// separator the writer adds itself is a bare EOL, never an indented blank
    /// line. What the caller wrote is preserved verbatim — the writer neither
    /// adds whitespace nor silently edits the body it was given.
    #[test]
    fn code_patch_insert_adds_no_trailing_whitespace_of_its_own() {
        let context = temp_context("insert-trailing-whitespace");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(
            &module,
            "Процедура Первая()\r\nКонецПроцедуры\r\n\
             Процедура Цель()\r\n\tСтарое = 1;\r\nКонецПроцедуры\r\n",
        )
        .unwrap();
        let mut args = tail_args(
            "main",
            "CommonModule.Sample.Module",
            "Процедура Помощник()\n\tЗначение = 1;\n\n\tВозврат Значение;\nКонецПроцедуры",
        );
        args.insert("selector".to_string(), json!({"method": "Цель"}));
        args.insert("position".to_string(), json!("before"));

        let applied = patch_inner(&args, &context, PatchMode::Apply);

        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        let after = fs::read_to_string(&module).unwrap();
        let dirty = after
            .split("\r\n")
            .enumerate()
            .filter(|(_, line)| line.len() != line.trim_end().len())
            .collect::<Vec<_>>();
        assert!(dirty.is_empty(), "git diff --check would flag {dirty:?}");
        assert!(after.contains("\tВозврат Значение;"), "{after}");
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    fn tail_args(source_set: &str, metadata_path: &str, content: &str) -> Map<String, Value> {
        Map::from_iter([
            ("sourceSet".to_string(), json!(source_set)),
            ("metadataPath".to_string(), json!(metadata_path)),
            ("operation".to_string(), json!("insert")),
            ("content".to_string(), json!(content)),
        ])
    }

    #[test]
    fn code_patch_dry_run_reports_the_same_postimage_without_writing() {
        let context = temp_context("dry-run");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);

        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        assert!(preview.outcome.changes.is_empty());
        assert!(preview.outcome.stdout.is_none());
        let data = preview.data.unwrap();
        assert_ne!(data.pre_hash, data.post_hash);
        assert_eq!(data.validation.status, ValidationStatus::Passed);
        assert_eq!(fs::read(&module).unwrap(), before);
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_refuses_a_stale_preimage_without_overwriting_concurrent_change() {
        let context = temp_context("stale-preimage");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        let replacement = "Procedure Run()\n    Message(\"concurrent\");\nEndProcedure\n";

        let result = with_before_commit_hook(
            move |path| fs::write(path, replacement).unwrap(),
            || patch_inner(&args, &context, PatchMode::Apply),
        );

        assert!(!result.outcome.ok);
        assert!(result.outcome.errors[0].contains("publish BSL module"));
        assert_eq!(fs::read_to_string(&module).unwrap(), replacement);
        assert_eq!(
            result.data.unwrap().validation.status,
            ValidationStatus::Passed
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_rolls_back_if_v8project_changes_the_owner_source_set_before_commit() {
        let context = temp_context("source-map-race");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        fs::write(
            context.workspace_root.join("src/CommonModules.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        let v8project = context.workspace_root.join("v8project.yaml");
        let concurrent_source_map = "format: DESIGNER\nsource-set:\n  - name: main\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n";
        let v8project_for_hook = v8project.clone();

        let result = with_before_commit_hook(
            move |_| fs::write(&v8project_for_hook, concurrent_source_map).unwrap(),
            || patch_inner(&args, &context, PatchMode::Apply),
        );

        assert!(!result.outcome.ok);
        let error = result.outcome.errors.join("\n");
        assert!(error.contains("read guard"), "{error}");
        assert!(error.contains("v8project.yaml"), "{error}");
        assert_eq!(fs::read(&module).unwrap(), before);
        assert_eq!(
            fs::read_to_string(&v8project).unwrap(),
            concurrent_source_map
        );
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    pub(crate) fn code_patch_rolls_back_if_owner_descriptor_changes_before_commit() {
        let context = temp_context("owner-descriptor-race");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        let descriptor = context.workspace_root.join("src/CommonModules/Sample.xml");
        let replacement = "<MetaDataObject concurrent=\"true\"/>";
        let descriptor_for_hook = descriptor.clone();

        let result = with_before_commit_hook(
            move |_| fs::write(&descriptor_for_hook, replacement).unwrap(),
            || patch_inner(&args, &context, PatchMode::Apply),
        );

        assert!(!result.outcome.ok);
        assert!(
            result.outcome.errors.join("\n").contains("read guard"),
            "{:?}",
            result.outcome.errors
        );
        assert_eq!(fs::read(&module).unwrap(), before);
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), replacement);
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn code_patch_rolls_back_if_absent_support_state_appears_before_commit() {
        let context = temp_context("support-state-appearance-race");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::create_dir_all(context.workspace_root.join("src/Ext")).unwrap();
        fs::write(
            context.workspace_root.join("src/Configuration.xml"),
            concat!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">",
                "<Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">",
                "<Properties><Name>Main</Name></Properties>",
                "</Configuration>",
                "</MetaDataObject>"
            ),
        )
        .unwrap();
        fs::write(
            context.workspace_root.join("src/CommonModules/Sample.xml"),
            concat!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">",
                "<CommonModule uuid=\"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb\">",
                "<Properties><Name>Sample</Name></Properties>",
                "</CommonModule>",
                "</MetaDataObject>"
            ),
        )
        .unwrap();
        let before = b"Procedure Run()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        let support_path = context
            .workspace_root
            .join("src/Ext/ParentConfigurations.bin");
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
        let support_path_for_hook = support_path.clone();
        let concurrent_support_for_hook = concurrent_support.clone();

        let result = with_before_commit_hook(
            move |_| fs::write(&support_path_for_hook, &concurrent_support_for_hook).unwrap(),
            || patch_inner(&args, &context, PatchMode::Apply),
        );

        assert!(!result.outcome.ok);
        assert!(
            result.outcome.errors.join("\n").contains("absence guard"),
            "{:?}",
            result.outcome.errors
        );
        assert_eq!(fs::read(&module).unwrap(), before);
        assert_eq!(fs::read(&support_path).unwrap(), concurrent_support);
        let violation =
            support_guard_violation(&module, SupportGuardRequirement::Editable).unwrap();
        assert_eq!(violation.code, "locked");
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn staged_code_insert_then_replace_reads_prior_postimage() {
        let fixture =
            staged_code_fixture("insert-then-replace", b"Procedure Base()\nEndProcedure\n");
        let operations = vec![
            staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                Some(CodeSelector::Method("Base".to_string())),
                Some(CodePosition::After),
            ),
            staged_replace(
                "main:CommonModule.Sample",
                "Procedure Final()\nEndProcedure",
                CodeSelector::Method("Added".to_string()),
            ),
        ];
        let admitted = staged_code_admission(&fixture, true);
        let (mut state, effects) = plan_admitted_code(&admitted, &fixture.binding, &operations)
            .expect("placeholder cannot compose the staged insert and replace postimages");

        assert_eq!(
            state
                .read(Path::new("CommonModules/Sample/Ext/Module.bsl"))
                .unwrap()
                .unwrap(),
            b"Procedure Base()\nEndProcedure\nProcedure Final()\nEndProcedure\n"
        );
        assert_eq!(
            effects.events(),
            &[DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:CommonModule.Sample",
            )]
        );

        let reverse = vec![operations[1].clone(), operations[0].clone()];
        let reverse_admission = staged_code_admission(&fixture, true);
        let error = plan_admitted_code(&reverse_admission, &fixture.binding, &reverse)
            .expect_err("reverse order unexpectedly resolved an unstaged method");
        assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
        fixture.cleanup();
    }

    #[test]
    fn staged_code_poisoned_second_op_publishes_nothing() {
        for dry_run in [true, false] {
            let fixture = staged_code_fixture(
                if dry_run { "poison-dry" } else { "poison-real" },
                b"Procedure Base()\nEndProcedure\n",
            );
            let service = fixture
                .actor
                .source_revision_service(&fixture.binding)
                .unwrap();
            let source_before = snapshot_staged_code_tree(&fixture.source);
            let cache = fixture.root.join(".build/unica");
            let cache_before = snapshot_staged_code_tree(&cache);
            let machine_before = service.machine_state_for_test();
            let admitted = fixture
                .actor
                .admit_apply(
                    &fixture.binding,
                    None,
                    dry_run,
                    crate::domain::code_intelligence::ProviderDeadline::from_budget(
                        Duration::from_secs(5),
                    ),
                    &crate::domain::cancellation::CancellationToken::new(),
                )
                .unwrap();
            let operations = vec![
                staged_insert(
                    "main:CommonModule.Sample",
                    "Procedure Added()\nEndProcedure",
                    None,
                    None,
                ),
                staged_replace(
                    "main:CommonModule.Sample",
                    "Procedure Never()\nEndProcedure",
                    CodeSelector::Method("Missing".to_string()),
                ),
            ];

            let error = plan_admitted_code(&admitted, &fixture.binding, &operations)
                .expect_err("poisoned staged code batch unexpectedly returned effects");

            assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
            assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
            assert_eq!(snapshot_staged_code_tree(&cache), cache_before);
            assert_eq!(service.machine_state_for_test(), machine_before);
            assert!(!cache.join("state.json").exists());
            fixture.cleanup();
        }
    }

    #[test]
    fn staged_code_dry_run_and_real_apply_share_postimages_and_effect_receipt() {
        let dry = staged_code_fixture("dry-real-dry", b"Procedure Base()\nEndProcedure\n");
        let real = staged_code_fixture("dry-real-real", b"Procedure Base()\nEndProcedure\n");
        let operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )];
        let dry_admission = staged_code_admission(&dry, true);
        let real_admission = staged_code_admission(&real, false);
        let admitted_rev = real_admission.revision_identity();
        let (mut dry_state, dry_effects) =
            plan_admitted_code(&dry_admission, &dry.binding, &operation)
                .expect("dry staged code planning is unavailable");
        let (mut real_state, real_effects) =
            plan_admitted_code(&real_admission, &real.binding, &operation)
                .expect("real staged code planning is unavailable");
        let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
        assert_eq!(
            dry_state.read(relative).unwrap(),
            real_state.read(relative).unwrap()
        );
        assert_eq!(dry_effects.events(), real_effects.events());
        let dry_result = dry
            .actor
            .publish_prepared_apply(
                dry_admission
                    .prepare_with_effects(dry_state, dry_effects)
                    .unwrap(),
            )
            .unwrap();
        let real_result = real
            .actor
            .publish_prepared_apply(
                real_admission
                    .prepare_with_effects(real_state, real_effects)
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(
            dry_result.effects().disposition(),
            ApplyEffectDisposition::Projected
        );
        assert_eq!(dry_result.commit_count_for_test(), 0);
        assert_eq!(
            fs::read(dry.source.join(relative)).unwrap(),
            b"Procedure Base()\nEndProcedure\n"
        );
        assert_eq!(
            real_result.effects().disposition(),
            ApplyEffectDisposition::Committed
        );
        assert_eq!(real_result.commit_count_for_test(), 1);
        assert_ne!(real_result.rev(), admitted_rev);
        assert_eq!(
            dry_result.effects().events(),
            real_result.effects().events()
        );
        dry.cleanup();
        real.cleanup();
    }

    #[test]
    fn staged_code_reuses_actor_revision_and_race_fences() {
        use crate::infrastructure::workspace_actor::ApplyPublicationErrorKind;

        let fixture = staged_code_fixture("actor-fences", b"Procedure Base()\nEndProcedure\n");
        let observed = staged_code_admission(&fixture, true);
        let rev = observed.revision_identity();
        let exact = fixture
            .actor
            .admit_apply(
                &fixture.binding,
                Some(&rev),
                true,
                crate::domain::code_intelligence::ProviderDeadline::from_budget(
                    Duration::from_secs(5),
                ),
                &crate::domain::cancellation::CancellationToken::new(),
            )
            .unwrap();
        let operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )];
        let (state, effects) = plan_admitted_code(&exact, &fixture.binding, &operation)
            .expect("planner did not reuse the actor-admitted revision");
        let prepared = exact.prepare_with_effects(state, effects).unwrap();

        let stale = fixture.actor.admit_apply(
            &fixture.binding,
            Some("sha256:999:stale"),
            true,
            crate::domain::code_intelligence::ProviderDeadline::from_budget(Duration::from_secs(5)),
            &crate::domain::cancellation::CancellationToken::new(),
        );
        assert!(stale.unwrap_err().contains("ifRev is stale"));

        fs::write(
            fixture.source.join("CommonModules/Sample/Ext/Module.bsl"),
            b"Procedure Foreign()\nEndProcedure\n",
        )
        .unwrap();
        let raced = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(raced.kind(), ApplyPublicationErrorKind::ConcurrentRevision);

        let cancelled = crate::domain::cancellation::CancellationToken::new();
        cancelled.cancel();
        assert!(fixture
            .actor
            .admit_apply(
                &fixture.binding,
                None,
                true,
                crate::domain::code_intelligence::ProviderDeadline::from_budget(
                    Duration::from_secs(5),
                ),
                &cancelled,
            )
            .unwrap_err()
            .contains("cancelled"));
        assert!(fixture
            .actor
            .admit_apply(
                &fixture.binding,
                None,
                true,
                crate::domain::code_intelligence::ProviderDeadline::from_budget(Duration::ZERO),
                &crate::domain::cancellation::CancellationToken::new(),
            )
            .unwrap_err()
            .contains("deadline"));
        assert!(!fixture.root.join(".build/unica/state.json").exists());
        fixture.cleanup();
    }

    #[test]
    fn staged_code_args_report_exact_paths_and_reject_legacy_fields() {
        let fixture = staged_code_fixture("args", b"Procedure Base()\nEndProcedure\n");
        let valid = parse_code_plan_operation(
            "code.insert",
            &json!({
                "at": "main:CommonModule.Sample",
                "text": "Procedure Added()\nEndProcedure"
            }),
            0,
            &fixture.binding,
        )
        .expect("placeholder parser rejected the valid closed code.insert shape");
        assert!(matches!(valid, CodePlanOperation::Insert(_)));

        let cases = [
            ("code.insert", json!(null), "ops[1].args"),
            ("code.insert", json!({"text": "x"}), "ops[1].args.at"),
            (
                "code.insert",
                json!({"at": "", "text": "x"}),
                "ops[1].args.at",
            ),
            (
                "code.insert",
                json!({"at": 42, "text": "x"}),
                "ops[1].args.at",
            ),
            (
                "code.insert",
                json!({"at": "not-an-address", "text": "x"}),
                "ops[1].args.at",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample"}),
                "ops[1].args.text",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": ""}),
                "ops[1].args.text",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": 1}),
                "ops[1].args.text",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": "Run"}),
                "ops[1].args.selector",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {}}),
                "ops[1].args.selector",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": "Run", "anchor": "x"}, "position": "after"}),
                "ops[1].args.selector",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": ""}, "position": "after"}),
                "ops[1].args.selector.method",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"unknown": "Run"}, "position": "after"}),
                "ops[1].args.selector.unknown",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": "Run"}}),
                "ops[1].args.position",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "position": "after"}),
                "ops[1].args.position",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": "Run"}, "position": 1}),
                "ops[1].args.position",
            ),
            (
                "code.insert",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": "Run"}, "position": "middle"}),
                "ops[1].args.position",
            ),
            (
                "code.replace",
                json!({"at": "main:CommonModule.Sample", "text": "x"}),
                "ops[1].args.selector",
            ),
            (
                "code.replace",
                json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": "Run"}, "position": "after"}),
                "ops[1].args.position",
            ),
            (
                "code.insert",
                json!({"at": "foreign:CommonModule.Sample", "text": "x"}),
                "ops[1].args.at",
            ),
        ];
        for (operation, args, expected_path) in cases {
            let error = parse_code_plan_operation(operation, &args, 1, &fixture.binding)
                .expect_err("invalid staged code arguments were accepted");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{args}");
            assert_eq!(error.path(), Some(expected_path), "{args}");
        }
        for legacy in [
            "operation",
            "content",
            "sourceSet",
            "sourceKind",
            "path",
            "metadataPath",
            "targetPath",
            "jsonPath",
            "provider",
            "providerIdentity",
            "providerProfile",
        ] {
            let mut args = json!({
                "at": "main:CommonModule.Sample",
                "text": "Procedure Added()\nEndProcedure"
            });
            args.as_object_mut()
                .unwrap()
                .insert(legacy.to_string(), json!("legacy"));
            let error = parse_code_plan_operation("code.insert", &args, 2, &fixture.binding)
                .expect_err("legacy/physical code argument was accepted");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some(format!("ops[2].args.{legacy}").as_str()));
        }
        fixture.cleanup();
    }

    #[test]
    fn staged_code_rejects_raw_policy_and_same_root_foreign_binding_substitution() {
        let locked = staged_code_fixture(
            "authority-policy-substitution",
            b"Procedure Base()\nEndProcedure\n",
        );
        write_locked_support_state(&locked.source);
        let denied = staged_code_admission(&locked, true);
        assert_eq!(denied.support_policy_mode(), SupportPolicyMode::Deny);
        fs::write(
            locked.root.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"off"}"#,
        )
        .unwrap();
        let operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )];
        let source_before = snapshot_staged_code_tree(&locked.source);
        let cache_before = snapshot_staged_code_tree(&locked.root.join(".build/unica"));

        let authority = denied
            .code_planning_authority(&locked.binding)
            .expect("the original actor binding must remain admitted");
        let substituted = plan_code_batch(denied.staged_state().unwrap(), authority, &operation);

        let error = substituted.expect_err(
            "a Deny admission followed ambient Off policy instead of its sealed evidence",
        );
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert_eq!(snapshot_staged_code_tree(&locked.source), source_before);
        assert_eq!(
            snapshot_staged_code_tree(&locked.root.join(".build/unica")),
            cache_before
        );
        assert!(!locked.root.join(".build/unica/state.json").exists());
        locked.cleanup();

        let fixture = staged_code_fixture(
            "authority-foreign-binding",
            b"Procedure Base()\nEndProcedure\n",
        );
        let foreign_actor = staged_code_actor(&fixture.root, &fixture.source);
        let foreign_binding = foreign_actor
            .bind_provider_root("main", &fixture.source)
            .unwrap();
        let admission = staged_code_admission(&fixture, true);
        let source_before = snapshot_staged_code_tree(&fixture.source);
        let cache_before = snapshot_staged_code_tree(&fixture.root.join(".build/unica"));

        let substituted = admission.code_planning_authority(&foreign_binding);

        let error = substituted
            .err()
            .expect("same-root binding from another actor instance was accepted");
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
        assert_eq!(
            snapshot_staged_code_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        assert!(!fixture.root.join(".build/unica/state.json").exists());
        fixture.cleanup();

        let writer = staged_code_fixture(
            "authority-writer-substitution",
            b"Procedure Base()\nEndProcedure\n",
        );
        let first = staged_code_admission(&writer, true);
        let second = staged_code_admission(&writer, true);
        let authority = first.code_planning_authority(&writer.binding).unwrap();
        let error = plan_code_batch(second.staged_state().unwrap(), authority, &operation)
            .expect_err("staged state from another admission writer was accepted");
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
        assert!(!writer.root.join(".build/unica/state.json").exists());
        writer.cleanup();
    }

    #[test]
    fn staged_code_rejects_absent_leaf_below_missing_parent_topology() {
        for (name, missing_parent) in [
            ("missing-ext", "CommonModules/Sample/Ext"),
            ("missing-owner-directory", "CommonModules/Sample"),
        ] {
            let fixture = staged_code_fixture(name, b"Procedure Base()\nEndProcedure\n");
            fs::remove_dir_all(fixture.source.join(missing_parent)).unwrap();
            let admission = staged_code_admission(&fixture, true);
            let source_before = snapshot_staged_code_tree(&fixture.source);
            let cache_before = snapshot_staged_code_tree(&fixture.root.join(".build/unica"));
            let operation = [staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                None,
                None,
            )];

            let result = plan_admitted_code(&admission, &fixture.binding, &operation);

            let error = result.expect_err(&format!(
                "planner returned effects for missing parent topology: {missing_parent}"
            ));
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::Staging(ApplyStagingErrorKind::MissingParent)
            );
            assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
            assert_eq!(
                snapshot_staged_code_tree(&fixture.root.join(".build/unica")),
                cache_before
            );
            assert!(!fixture.root.join(".build/unica/state.json").exists());
            fixture.cleanup();
        }
    }

    #[test]
    fn staged_code_support_guard_matches_v12_compatibility_for_all_marker_bytes() {
        let cases: Vec<(&str, Option<Vec<u8>>, bool)> = vec![
            ("absent", None, false),
            ("empty", Some(Vec::new()), false),
            ("short", Some(b"short marker".to_vec()), false),
            ("malformed-non-utf8", Some(vec![0xff; 40]), false),
            (
                "malformed-header",
                Some(b"not a support header but longer than thirty-two bytes".to_vec()),
                false,
            ),
            (
                "removed-zero-vendor",
                Some(b"{6,0,0,dddddddd-dddd-dddd-dddd-dddddddddddd}".to_vec()),
                false,
            ),
            (
                "global-deny",
                Some(b"{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd}".to_vec()),
                true,
            ),
            (
                "global-allow",
                Some(b"{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd}".to_vec()),
                false,
            ),
            (
                "owner-deny",
                Some(
                    b"{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}"
                        .to_vec(),
                ),
                true,
            ),
            (
                "owner-allow",
                Some(
                    b"{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,1,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}"
                        .to_vec(),
                ),
                false,
            ),
        ];
        let operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )];
        for (name, marker, expected_blocked) in cases {
            let fixture = staged_code_fixture(
                &format!("support-compat-{name}"),
                b"Procedure Base()\nEndProcedure\n",
            );
            let marker_path = fixture.source.join("Ext/ParentConfigurations.bin");
            match marker {
                Some(bytes) => fs::write(&marker_path, bytes).unwrap(),
                None => assert!(!marker_path.exists()),
            }
            let module = fixture.source.join("CommonModules/Sample/Ext/Module.bsl");
            let v12_blocked =
                support_guard_violation(&module, SupportGuardRequirement::Editable).is_some();
            assert_eq!(v12_blocked, expected_blocked, "invalid V12 oracle: {name}");
            let v12_context = WorkspaceContext {
                cwd: fixture.root.clone(),
                workspace_root: fixture.root.clone(),
                cache_root: fixture.root.join(".build/unica"),
                workspace_epoch: 1,
            };
            let v12 = patch_inner(
                &patch_args(
                    "main",
                    "CommonModule.Sample.Module",
                    "Base",
                    "Procedure Added()\nEndProcedure",
                ),
                &v12_context,
                PatchMode::Preview,
            );
            assert_eq!(
                !v12.outcome.ok, v12_blocked,
                "V12 patch path disagrees with its support guard for {name}: {:?}",
                v12.outcome.errors
            );
            let admission = staged_code_admission(&fixture, true);

            let c1 = plan_admitted_code(&admission, &fixture.binding, &operation);

            assert_eq!(c1.is_err(), v12_blocked, "V12/C1 mismatch for {name}");
            if let Err(error) = c1 {
                assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState, "{name}");
            }
            fixture.cleanup();
        }
    }

    #[test]
    fn staged_code_selector_kind_does_not_depend_on_error_message_words() {
        let error = super::code_selector_error(
            super::SelectorResolutionError::invalid_source(
                "invalid source contains the word matched but has no match count",
            ),
            3,
            &super::Selector::Method("Base".to_string()),
        );

        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
        assert_eq!(error.path(), Some("ops[3].args.selector.method"));

        for (name, before, selector, expected) in [
            (
                "zero-method",
                b"Procedure Base()\nEndProcedure\n".as_slice(),
                CodeSelector::Method("Missing".to_string()),
                ApplyPlanErrorKind::NotFound,
            ),
            (
                "multiple-method",
                b"Procedure Base()\nEndProcedure\nProcedure Base()\nEndProcedure\n".as_slice(),
                CodeSelector::Method("Base".to_string()),
                ApplyPlanErrorKind::InvalidState,
            ),
            (
                "zero-anchor",
                b"Procedure Base()\nEndProcedure\n".as_slice(),
                CodeSelector::Anchor("// marker".to_string()),
                ApplyPlanErrorKind::NotFound,
            ),
            (
                "multiple-anchor",
                b"Procedure Base()\n    // marker\n    // marker\nEndProcedure\n".as_slice(),
                CodeSelector::Anchor("// marker".to_string()),
                ApplyPlanErrorKind::InvalidState,
            ),
        ] {
            for operation_kind in ["insert", "replace"] {
                let fixture =
                    staged_code_fixture(&format!("selector-cause-{operation_kind}-{name}"), before);
                let admission = staged_code_admission(&fixture, true);
                let operation = [match operation_kind {
                    "insert" => staged_insert(
                        "main:CommonModule.Sample",
                        "Procedure Added()\nEndProcedure",
                        Some(selector.clone()),
                        Some(CodePosition::After),
                    ),
                    "replace" => staged_replace(
                        "main:CommonModule.Sample",
                        "Procedure Added()\nEndProcedure",
                        selector.clone(),
                    ),
                    _ => unreachable!(),
                }];
                let failure = format!("{operation_kind} {name}");
                let projected = plan_admitted_code(&admission, &fixture.binding, &operation)
                    .expect_err(&failure);
                assert_eq!(projected.kind(), expected, "{operation_kind} {name}");
                fixture.cleanup();
            }
        }
    }

    #[test]
    fn v12_selector_adapter_preserves_exact_zero_and_multiple_messages() {
        let cases = [
            (
                "zero-method",
                "Procedure Base()\nEndProcedure\n",
                super::Selector::Method("Missing".to_string()),
                "method selector must match exactly once; matched 0 times",
            ),
            (
                "multiple-method",
                "Procedure Base()\nEndProcedure\nProcedure Base()\nEndProcedure\n",
                super::Selector::Method("Base".to_string()),
                "method selector must match exactly once; matched 2 times",
            ),
            (
                "zero-anchor",
                "Procedure Base()\nEndProcedure\n",
                super::Selector::Anchor("// marker".to_string()),
                "anchor selector must match exactly once; matched 0 times",
            ),
            (
                "multiple-anchor",
                "Procedure Base()\n    // marker\n    // marker\nEndProcedure\n",
                super::Selector::Anchor("// marker".to_string()),
                "anchor selector must match exactly once; matched 2 times",
            ),
        ];
        for (name, source, selector, expected) in cases {
            let snapshot = SourceTextSnapshot::from_bytes(source.as_bytes()).unwrap();
            let indexed = analyze_module(source).unwrap();
            assert!(indexed.diagnostics.is_empty(), "{name}");
            assert_eq!(
                super::locate_selector(&snapshot, Position::After, &selector, &indexed.methods,)
                    .unwrap_err(),
                expected,
                "insert {name}"
            );
            assert_eq!(
                super::locate_replacement(&snapshot, &selector, &indexed.methods).unwrap_err(),
                expected,
                "replace {name}"
            );
        }
    }

    #[test]
    fn staged_code_preserves_module_owner_support_format_and_preimage_guards() {
        let valid = staged_code_fixture("guards-valid", b"Procedure Base()\nEndProcedure\n");
        let operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )];
        let admission = staged_code_admission(&valid, true);
        let (state, effects) = plan_admitted_code(&admission, &valid.binding, &operation)
            .expect("valid actor-issued profile was rejected by the placeholder planner");
        assert_eq!(effects.events().len(), 1);
        drop(state);
        drop(admission);

        let descriptor_target = [staged_insert(
            "main:CommonModule.Sample.Module",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )];
        let admission = staged_code_admission(&valid, true);
        assert_eq!(
            plan_admitted_code(&admission, &valid.binding, &descriptor_target)
                .unwrap_err()
                .kind(),
            ApplyPlanErrorKind::InvalidSource
        );

        fs::write(
            valid.source.join("CommonModules/Sample/Ext/Module.bsl"),
            b"Procedure Broken(\n",
        )
        .unwrap();
        let malformed_admission = staged_code_admission(&valid, true);
        assert_eq!(
            plan_admitted_code(&malformed_admission, &valid.binding, &operation)
                .unwrap_err()
                .kind(),
            ApplyPlanErrorKind::InvalidSource
        );
        valid.cleanup();

        let locked = staged_code_fixture("guards-locked", b"Procedure Base()\nEndProcedure\n");
        write_locked_support_state(&locked.source);
        let denied = staged_code_admission(&locked, true);
        assert_eq!(denied.support_policy_mode(), SupportPolicyMode::Deny);
        assert_eq!(
            plan_admitted_code(&denied, &locked.binding, &operation)
                .unwrap_err()
                .kind(),
            ApplyPlanErrorKind::InvalidState
        );
        fs::write(
            locked.root.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"warn"}"#,
        )
        .unwrap();
        let warned = staged_code_admission(&locked, true);
        assert_eq!(warned.support_policy_mode(), SupportPolicyMode::Warn);
        assert!(plan_admitted_code(&warned, &locked.binding, &operation).is_ok());
        fs::write(
            locked.root.join(".v8-project.json"),
            br#"{"editingAllowedCheck":"off"}"#,
        )
        .unwrap();
        let off = staged_code_admission(&locked, true);
        assert_eq!(off.support_policy_mode(), SupportPolicyMode::Off);
        assert!(plan_admitted_code(&off, &locked.binding, &operation).is_ok());
        locked.cleanup();

        let absent = staged_code_fixture("guards-absent", b"Procedure Base()\nEndProcedure\n");
        fs::remove_file(absent.source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap();
        let absent_admission = staged_code_admission(&absent, true);
        let (state, _) = plan_admitted_code(&absent_admission, &absent.binding, &operation)
            .expect("profile-permitted absent module was rejected");
        assert!(matches!(
            state.planned_changes()[0].original,
            StagedFileState::Absent
        ));
        absent.cleanup();

        let raced = staged_code_fixture("guards-owner-race", b"Procedure Base()\nEndProcedure\n");
        let raced_admission = staged_code_admission(&raced, false);
        let (state, effects) =
            plan_admitted_code(&raced_admission, &raced.binding, &operation).unwrap();
        fs::write(
            raced.source.join("CommonModules/Sample.xml"),
            b"<MetaDataObject concurrent=\"true\"/>",
        )
        .unwrap();
        assert!(raced_admission
            .prepare_with_effects(state, effects)
            .is_err());
        raced.cleanup();

        let replaced_root =
            staged_code_fixture("guards-root-race", b"Procedure Base()\nEndProcedure\n");
        let root_admission = staged_code_admission(&replaced_root, false);
        let (state, effects) =
            plan_admitted_code(&root_admission, &replaced_root.binding, &operation).unwrap();
        let prepared = root_admission.prepare_with_effects(state, effects).unwrap();
        let retained_source = replaced_root.root.join("retained-source");
        let retained_identity = path_identity_for_test(&replaced_root.source)
            .unwrap()
            .expect("source root identity must be available on supported CI platforms");
        let replacement = attempt_retained_directory_replacement_for_test(
            &replaced_root.source,
            &retained_source,
        )
        .unwrap();
        match replacement {
            RetainedDirectoryReplacementOutcome::Replaced => {
                fs::create_dir(&replaced_root.source).unwrap();
                assert!(replaced_root
                    .actor
                    .publish_prepared_apply(prepared)
                    .is_err());
                assert_eq!(
                    fs::read(retained_source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap(),
                    b"Procedure Base()\nEndProcedure\n"
                );
                assert!(fs::read_dir(&replaced_root.source)
                    .unwrap()
                    .next()
                    .is_none());
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&replaced_root.source)
                        .unwrap()
                        .as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!retained_source.exists());
                let result = replaced_root
                    .actor
                    .publish_prepared_apply(prepared)
                    .unwrap();
                assert_eq!(result.effects().events().len(), 1);
                let module = fs::read(
                    replaced_root
                        .source
                        .join("CommonModules/Sample/Ext/Module.bsl"),
                )
                .unwrap();
                assert!(module
                    .windows(b"Procedure Added()".len())
                    .any(|window| { window == b"Procedure Added()" }));
            }
        }
        replaced_root.cleanup();

        let linked = staged_code_fixture("guards-link", b"Procedure Base()\nEndProcedure\n");
        let link_admission = staged_code_admission(&linked, true);
        let module = linked.source.join("CommonModules/Sample/Ext/Module.bsl");
        let real = linked
            .source
            .join("CommonModules/Sample/Ext/RealModule.bsl");
        fs::rename(&module, &real).unwrap();
        if create_file_link_fixture_for_test(&real, &module).unwrap()
            == FileLinkFixtureOutcome::Created
        {
            let error =
                plan_admitted_code(&link_admission, &linked.binding, &operation).unwrap_err();
            assert!(
                matches!(
                    error.kind(),
                    ApplyPlanErrorKind::Staging(
                        ApplyStagingErrorKind::ContainmentIdentity
                            | ApplyStagingErrorKind::UnsupportedProvider
                    )
                ),
                "{error:?}"
            );
        }
        linked.cleanup();

        let linked_support =
            staged_code_fixture("guards-support-link", b"Procedure Base()\nEndProcedure\n");
        let support_admission = staged_code_admission(&linked_support, true);
        let real_support = linked_support.root.join("retained-support-marker.bin");
        fs::write(&real_support, b"short marker").unwrap();
        let support_marker = linked_support.source.join("Ext/ParentConfigurations.bin");
        if create_file_link_fixture_for_test(&real_support, &support_marker).unwrap()
            == FileLinkFixtureOutcome::Created
        {
            let error = plan_admitted_code(&support_admission, &linked_support.binding, &operation)
                .unwrap_err();
            assert!(
                matches!(
                    error.kind(),
                    ApplyPlanErrorKind::Staging(
                        ApplyStagingErrorKind::ContainmentIdentity
                            | ApplyStagingErrorKind::UnsupportedProvider
                    )
                ),
                "{error:?}"
            );
            assert!(!linked_support.root.join(".build/unica/state.json").exists());
        }
        linked_support.cleanup();

        let generated =
            staged_code_fixture("guards-generated", b"Procedure Base()\nEndProcedure\n");
        let mut staged = staged_code_admission(&generated, true)
            .staged_state()
            .unwrap();
        assert_eq!(
            staged
                .read(Path::new(".build/unica/foreign"))
                .unwrap_err()
                .kind(),
            ApplyStagingErrorKind::ContainmentIdentity
        );
        generated.cleanup();
    }

    #[test]
    fn staged_code_v12_parity_preserves_bytes_errors_and_noop() {
        let before = b"\xef\xbb\xbfProcedure Base()\r\nEndProcedure\r\n// untouched\r\n";
        let v12 = staged_code_fixture("v12-parity-v12", before);
        let staged = staged_code_fixture("v12-parity-v13", before);
        let v12_context = WorkspaceContext {
            cwd: v12.root.clone(),
            workspace_root: v12.root.clone(),
            cache_root: v12.root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let v12_args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Base",
            "Procedure Added()\nEndProcedure",
        );
        let v12_result = patch_inner(&v12_args, &v12_context, PatchMode::Apply);
        assert!(v12_result.outcome.ok, "{:?}", v12_result.outcome.errors);
        let operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            Some(CodeSelector::Method("Base".to_string())),
            Some(CodePosition::After),
        )];
        let admission = staged_code_admission(&staged, true);
        let (mut state, effects) = plan_admitted_code(&admission, &staged.binding, &operation)
            .expect("placeholder planner cannot produce the V12 parity postimage");
        let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
        assert_eq!(
            state.read(relative).unwrap().unwrap(),
            fs::read(v12.source.join(relative)).unwrap()
        );
        assert_eq!(effects.events().len(), 1);

        let invalid_v12 = patch_inner(
            &patch_args(
                "main",
                "CommonModule.Sample.Module",
                "Missing",
                "Procedure Added()\nEndProcedure",
            ),
            &v12_context,
            PatchMode::Preview,
        );
        assert!(!invalid_v12.outcome.ok);
        let invalid_operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            Some(CodeSelector::Method("Missing".to_string())),
            Some(CodePosition::After),
        )];
        let invalid_admission = staged_code_admission(&staged, true);
        assert_eq!(
            plan_admitted_code(&invalid_admission, &staged.binding, &invalid_operation)
                .unwrap_err()
                .kind(),
            ApplyPlanErrorKind::NotFound
        );

        let repeated_v12 = patch_inner(&v12_args, &v12_context, PatchMode::Apply);
        assert!(repeated_v12.outcome.ok);
        assert!(repeated_v12.data.unwrap().no_op);
        let real_admission = staged_code_admission(&staged, false);
        let (state, effects) = plan_admitted_code(&real_admission, &staged.binding, &operation)
            .expect("first staged parity publication planning failed");
        staged
            .actor
            .publish_prepared_apply(real_admission.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        let repeat_admission = staged_code_admission(&staged, true);
        let (repeat_state, repeat_effects) =
            plan_admitted_code(&repeat_admission, &staged.binding, &operation).unwrap();
        assert!(repeat_state.planned_changes().is_empty());
        assert!(repeat_effects.events().is_empty());

        let malformed_v12 = staged_code_fixture("v12-malformed", b"Procedure Broken(\n");
        let malformed_context = WorkspaceContext {
            cwd: malformed_v12.root.clone(),
            workspace_root: malformed_v12.root.clone(),
            cache_root: malformed_v12.root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let malformed_result = patch_inner(&v12_args, &malformed_context, PatchMode::Preview);
        assert!(!malformed_result.outcome.ok);
        let malformed_admission = staged_code_admission(&malformed_v12, true);
        assert_eq!(
            plan_admitted_code(&malformed_admission, &malformed_v12.binding, &operation)
                .unwrap_err()
                .kind(),
            ApplyPlanErrorKind::InvalidSource
        );
        malformed_v12.cleanup();
        v12.cleanup();
        staged.cleanup();
    }

    #[test]
    fn staged_code_v12_parity_covers_insert_replace_method_and_anchor() {
        assert_v12_c1_operation_parity(
            "insert-method",
            b"Procedure Base()\nEndProcedure\n",
            patch_args(
                "main",
                "CommonModule.Sample.Module",
                "Base",
                "Procedure Added()\nEndProcedure",
            ),
            staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                Some(CodeSelector::Method("Base".to_string())),
                Some(CodePosition::After),
            ),
            true,
        );
        assert_v12_c1_operation_parity(
            "insert-anchor",
            b"Procedure Base()\n    // marker\nEndProcedure\n",
            patch_args_for_selector(
                "main",
                "CommonModule.Sample.Module",
                json!({"anchor": "// marker"}),
                "after",
                "Message(\"added\");",
            ),
            staged_insert(
                "main:CommonModule.Sample",
                "Message(\"added\");",
                Some(CodeSelector::Anchor("// marker".to_string())),
                Some(CodePosition::After),
            ),
            true,
        );
        assert_v12_c1_operation_parity(
            "replace-method",
            b"Procedure Base()\nEndProcedure\n",
            replace_args(
                "CommonModule.Sample.Module",
                json!({"method": "Base"}),
                "Procedure Base()\n    Message(\"changed\");\nEndProcedure",
            ),
            staged_replace(
                "main:CommonModule.Sample",
                "Procedure Base()\n    Message(\"changed\");\nEndProcedure",
                CodeSelector::Method("Base".to_string()),
            ),
            true,
        );
        assert_v12_c1_operation_parity(
            "replace-anchor",
            "Procedure Base()\n    Значение = 1;\nEndProcedure\n".as_bytes(),
            replace_args(
                "CommonModule.Sample.Module",
                json!({"anchor": "Значение = 1;"}),
                "Значение = 2;",
            ),
            staged_replace(
                "main:CommonModule.Sample",
                "Значение = 2;",
                CodeSelector::Anchor("Значение = 1;".to_string()),
            ),
            false,
        );
    }

    #[test]
    fn staged_code_v12_parity_rejects_malformed_postimage_and_owner_evidence() {
        let malformed_v12 =
            staged_code_fixture("parity-post-v12", b"Procedure Base()\nEndProcedure\n");
        let malformed_c1 =
            staged_code_fixture("parity-post-c1", b"Procedure Base()\nEndProcedure\n");
        let context = WorkspaceContext {
            cwd: malformed_v12.root.clone(),
            workspace_root: malformed_v12.root.clone(),
            cache_root: malformed_v12.root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let v12 = patch_inner(
            &patch_args(
                "main",
                "CommonModule.Sample.Module",
                "Base",
                "Procedure Broken(",
            ),
            &context,
            PatchMode::Preview,
        );
        assert!(!v12.outcome.ok);
        let admission = staged_code_admission(&malformed_c1, true);
        let c1 = plan_admitted_code(
            &admission,
            &malformed_c1.binding,
            &[staged_insert(
                "main:CommonModule.Sample",
                "Procedure Broken(",
                Some(CodeSelector::Method("Base".to_string())),
                Some(CodePosition::After),
            )],
        )
        .unwrap_err();
        assert_eq!(c1.kind(), ApplyPlanErrorKind::Postcondition);
        malformed_v12.cleanup();
        malformed_c1.cleanup();

        for case in ["absent", "malformed", "wrong-format"] {
            let v12 = staged_code_fixture(
                &format!("owner-{case}-v12"),
                b"Procedure Base()\nEndProcedure\n",
            );
            let c1 = staged_code_fixture(
                &format!("owner-{case}-c1"),
                b"Procedure Base()\nEndProcedure\n",
            );
            for fixture in [&v12, &c1] {
                let descriptor = fixture.source.join("CommonModules/Sample.xml");
                match case {
                    "absent" => fs::remove_file(descriptor).unwrap(),
                    "malformed" => fs::write(descriptor, b"<not-xml").unwrap(),
                    "wrong-format" => {
                        let before = fs::read_to_string(&descriptor).unwrap();
                        fs::write(descriptor, before.replace("2.20", "2.21")).unwrap();
                    }
                    _ => unreachable!(),
                }
            }
            let context = WorkspaceContext {
                cwd: v12.root.clone(),
                workspace_root: v12.root.clone(),
                cache_root: v12.root.join(".build/unica"),
                workspace_epoch: 1,
            };
            let v12_result = patch_inner(
                &patch_args(
                    "main",
                    "CommonModule.Sample.Module",
                    "Base",
                    "Procedure Added()\nEndProcedure",
                ),
                &context,
                PatchMode::Preview,
            );
            assert!(!v12_result.outcome.ok, "V12 accepted owner case {case}");
            let admission = staged_code_admission(&c1, true);
            assert!(
                plan_admitted_code(
                    &admission,
                    &c1.binding,
                    &[staged_insert(
                        "main:CommonModule.Sample",
                        "Procedure Added()\nEndProcedure",
                        Some(CodeSelector::Method("Base".to_string())),
                        Some(CodePosition::After),
                    )],
                )
                .is_err(),
                "C1 accepted owner case {case}"
            );
            v12.cleanup();
            c1.cleanup();
        }
    }

    #[test]
    fn staged_code_effects_track_changed_modules_only_in_stable_order() {
        use crate::infrastructure::native_operations::compile_transaction::{
            set_retained_apply_failpoint, RetainedApplyFailpoint,
        };

        let fixture = staged_code_fixture("effects", b"Procedure Base()\nEndProcedure\n");
        let operations = [
            staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                None,
                None,
            ),
            staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                None,
                None,
            ),
            staged_insert(
                "main:CommonModule.Other",
                "Procedure OtherAdded()\nEndProcedure",
                None,
                None,
            ),
        ];
        let admission = staged_code_admission(&fixture, false);
        let (state, effects) = plan_admitted_code(&admission, &fixture.binding, &operations)
            .expect("placeholder planner cannot produce stable changed-module effects");
        assert_eq!(
            effects.events(),
            &[
                DomainEvent::new(DomainEventKind::ModuleChanged, "main:CommonModule.Sample"),
                DomainEvent::new(DomainEventKind::ModuleChanged, "main:CommonModule.Other"),
            ]
        );
        let prepared = admission.prepare_with_effects(state, effects).unwrap();
        let source_before = snapshot_staged_code_tree(&fixture.source);
        let cache_before = snapshot_staged_code_tree(&fixture.root.join(".build/unica"));
        set_retained_apply_failpoint(RetainedApplyFailpoint::AfterAllPostimages);
        assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
        assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
        assert_eq!(
            snapshot_staged_code_tree(&fixture.root.join(".build/unica")),
            cache_before
        );

        let poisoned = staged_code_admission(&fixture, true);
        let poisoned_ops = [
            operations[0].clone(),
            staged_replace(
                "main:CommonModule.Sample",
                "Procedure Never()\nEndProcedure",
                CodeSelector::Method("Missing".to_string()),
            ),
        ];
        assert!(plan_admitted_code(&poisoned, &fixture.binding, &poisoned_ops).is_err());
        fixture.cleanup();
    }

    #[test]
    fn staged_code_effects_omit_a_module_restored_to_its_original_preimage() {
        let fixture = staged_code_fixture(
            "effects-restored-preimage",
            b"Procedure Base()\nEndProcedure\n",
        );
        let admission = staged_code_admission(&fixture, true);
        let operations = [
            staged_replace(
                "main:CommonModule.Sample",
                "Procedure Base()\n    Message(\"changed\");\nEndProcedure",
                CodeSelector::Method("Base".to_string()),
            ),
            staged_replace(
                "main:CommonModule.Sample",
                "Procedure Base()\nEndProcedure",
                CodeSelector::Method("Base".to_string()),
            ),
        ];

        let (state, effects) =
            plan_admitted_code(&admission, &fixture.binding, &operations).unwrap();

        assert!(state.planned_changes().is_empty());
        assert!(effects.events().is_empty());
        fixture.cleanup();
    }

    #[test]
    fn shared_apply_plan_error_and_effect_contract_is_complete() {
        let fixture = staged_code_fixture("error-projections", b"Procedure Base()\nEndProcedure\n");
        let bad_value =
            parse_code_plan_operation("code.insert", &json!({"text": "x"}), 0, &fixture.binding)
                .unwrap_err();
        assert_closed_code_error(
            &bad_value,
            ApplyPlanErrorKind::BadValue,
            Some("ops[0].args.at"),
            &fixture.root,
        );

        let missing = [staged_replace(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            CodeSelector::Method("Missing".to_string()),
        )];
        let admission = staged_code_admission(&fixture, true);
        let not_found = plan_admitted_code(&admission, &fixture.binding, &missing).unwrap_err();
        assert_closed_code_error(
            &not_found,
            ApplyPlanErrorKind::NotFound,
            Some("ops[0].args.selector.method"),
            &fixture.root,
        );

        write_locked_support_state(&fixture.source);
        let admission = staged_code_admission(&fixture, true);
        let invalid_state = plan_admitted_code(
            &admission,
            &fixture.binding,
            &[staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                None,
                None,
            )],
        )
        .unwrap_err();
        assert_closed_code_error(
            &invalid_state,
            ApplyPlanErrorKind::InvalidState,
            Some("ops[0].args.at"),
            &fixture.root,
        );
        fs::remove_file(fixture.source.join("Ext/ParentConfigurations.bin")).unwrap();

        fs::write(
            fixture.source.join("CommonModules/Sample/Ext/Module.bsl"),
            b"Procedure Broken(\n",
        )
        .unwrap();
        let admission = staged_code_admission(&fixture, true);
        let invalid_source = plan_admitted_code(
            &admission,
            &fixture.binding,
            &[staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                None,
                None,
            )],
        )
        .unwrap_err();
        assert_closed_code_error(
            &invalid_source,
            ApplyPlanErrorKind::InvalidSource,
            Some("ops[0].args.at"),
            &fixture.root,
        );

        fs::write(
            fixture.source.join("CommonModules/Sample/Ext/Module.bsl"),
            b"Procedure Base()\nEndProcedure\n",
        )
        .unwrap();
        let admission = staged_code_admission(&fixture, true);
        let postcondition = plan_admitted_code(
            &admission,
            &fixture.binding,
            &[staged_insert(
                "main:CommonModule.Sample",
                "Procedure Base()\nEndProcedure",
                Some(CodeSelector::Method("Base".to_string())),
                Some(CodePosition::After),
            )],
        )
        .unwrap_err();
        assert_closed_code_error(
            &postcondition,
            ApplyPlanErrorKind::Postcondition,
            Some("ops[0].args.text"),
            &fixture.root,
        );

        fs::remove_dir_all(fixture.source.join("CommonModules/Sample/Ext")).unwrap();
        let admission = staged_code_admission(&fixture, true);
        let staging = plan_admitted_code(
            &admission,
            &fixture.binding,
            &[staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                None,
                None,
            )],
        )
        .unwrap_err();
        assert_closed_code_error(
            &staging,
            ApplyPlanErrorKind::Staging(ApplyStagingErrorKind::MissingParent),
            Some("ops[0].args.at"),
            &fixture.root,
        );
        fixture.cleanup();

        let unsupported = staged_code_fixture(
            "error-provider-unavailable",
            b"Procedure Base()\nEndProcedure\n",
        );
        fs::write(
            unsupported.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: unsupported.root.clone(),
            workspace_root: unsupported.root.clone(),
            cache_root: unsupported.root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                "main",
                &unsupported.source,
                SourceSetKind::ExternalProcessor,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "staged-code-test-provider",
        )
        .unwrap();
        let actor = WorkspaceActor::new(identity, context).unwrap();
        let binding = actor
            .bind_provider_root("main", &unsupported.source)
            .unwrap();
        let admission = actor
            .admit_apply(
                &binding,
                None,
                true,
                crate::domain::code_intelligence::ProviderDeadline::from_budget(
                    Duration::from_secs(5),
                ),
                &crate::domain::cancellation::CancellationToken::new(),
            )
            .unwrap();
        let provider = admission
            .code_planning_authority(&binding)
            .err()
            .expect("unsupported admitted source unexpectedly issued Code planning authority");
        assert_closed_code_error(
            &provider,
            ApplyPlanErrorKind::ProviderUnavailable,
            None,
            &unsupported.root,
        );
        unsupported.cleanup();

        let mut effects =
            crate::infrastructure::native_operations::apply::PlannedApplyEffects::default();
        effects.append(DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "main:CommonModule.Sample",
        ));
        effects.append(DomainEvent::new(
            DomainEventKind::FormChanged,
            "main:CommonForm.Main",
        ));
        effects.append(DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "main:CommonModule.Sample",
        ));
        assert_eq!(
            effects.into_events(),
            vec![
                DomainEvent::new(DomainEventKind::ModuleChanged, "main:CommonModule.Sample"),
                DomainEvent::new(DomainEventKind::FormChanged, "main:CommonForm.Main"),
            ]
        );
    }

    fn assert_closed_code_error(
        error: &ApplyPlanError,
        kind: ApplyPlanErrorKind,
        path: Option<&str>,
        physical_root: &Path,
    ) {
        assert_eq!(error.kind(), kind);
        assert_eq!(error.path(), path);
        let rendered = error.to_string();
        let canonical_root = fs::canonicalize(physical_root).unwrap();
        for forbidden in [
            physical_root.display().to_string(),
            canonical_root.display().to_string(),
            "staged-code-test-provider".to_owned(),
            "unica.code.patch".to_owned(),
            "provider profile".to_owned(),
        ] {
            assert!(
                !rendered.contains(&forbidden),
                "closed error leaked `{forbidden}`: {rendered}"
            );
        }
    }

    struct StagedCodeFixture {
        root: PathBuf,
        source: PathBuf,
        actor: Arc<WorkspaceActor>,
        binding: ProviderRootBinding,
    }

    impl StagedCodeFixture {
        fn cleanup(self) {
            let _ = fs::remove_dir_all(self.root);
        }
    }

    fn staged_code_fixture(name: &str, sample_module: &[u8]) -> StagedCodeFixture {
        const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
        let root = temp_root(&format!("staged-code-{name}"));
        let source = root.join("src");
        fs::create_dir_all(source.join("Ext")).unwrap();
        fs::create_dir_all(source.join("CommonModules/Sample/Ext")).unwrap();
        fs::create_dir_all(source.join("CommonModules/Other/Ext")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            source.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Sample</CommonModule><CommonModule>Other</CommonModule></ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
        for (module, uuid) in [
            ("Sample", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            ("Other", "cccccccc-cccc-cccc-cccc-cccccccccccc"),
        ] {
            fs::write(
                source.join(format!("CommonModules/{module}.xml")),
                format!(
                    "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><CommonModule uuid=\"{uuid}\"><Properties><Name>{module}</Name></Properties></CommonModule></MetaDataObject>"
                ),
            )
            .unwrap();
        }
        fs::write(
            source.join("CommonModules/Sample/Ext/Module.bsl"),
            sample_module,
        )
        .unwrap();
        fs::write(
            source.join("CommonModules/Other/Ext/Module.bsl"),
            b"Procedure Other()\nEndProcedure\n",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                "main",
                &source,
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "staged-code-test-provider",
        )
        .unwrap();
        let actor = Arc::new(WorkspaceActor::new(identity, context).unwrap());
        let binding = actor.bind_provider_root("main", &source).unwrap();
        StagedCodeFixture {
            root,
            source,
            actor,
            binding,
        }
    }

    fn staged_code_actor(root: &Path, source: &Path) -> Arc<WorkspaceActor> {
        let context = WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [WorkspaceSourceSetInput::new(
                "main",
                source,
                SourceSetKind::Configuration,
                SourceFormat::PlatformXml,
                SourceProfile::platform_xml_8_3_27_format_2_20(),
            )],
            "staged-code-test-provider",
        )
        .unwrap();
        Arc::new(WorkspaceActor::new(identity, context).unwrap())
    }

    fn staged_code_admission(
        fixture: &StagedCodeFixture,
        dry_run: bool,
    ) -> crate::infrastructure::workspace_actor::ApplyAdmission {
        fixture
            .actor
            .admit_apply(
                &fixture.binding,
                None,
                dry_run,
                crate::domain::code_intelligence::ProviderDeadline::from_budget(
                    Duration::from_secs(5),
                ),
                &crate::domain::cancellation::CancellationToken::new(),
            )
            .unwrap()
    }

    fn plan_admitted_code(
        admission: &crate::infrastructure::workspace_actor::ApplyAdmission,
        binding: &ProviderRootBinding,
        operations: &[CodePlanOperation],
    ) -> Result<
        (
            crate::infrastructure::native_operations::apply::ApplyStagedState,
            crate::infrastructure::native_operations::apply::PlannedApplyEffects,
        ),
        ApplyPlanError,
    > {
        let staged = admission.staged_state().unwrap();
        let authority = admission.code_planning_authority(binding)?;
        plan_code_batch(staged, authority, operations)
    }

    fn assert_v12_c1_operation_parity(
        name: &str,
        before: &[u8],
        v12_args: Map<String, Value>,
        operation: CodePlanOperation,
        repeat_is_noop: bool,
    ) {
        let v12 = staged_code_fixture(&format!("parity-{name}-v12"), before);
        let c1 = staged_code_fixture(&format!("parity-{name}-c1"), before);
        let context = WorkspaceContext {
            cwd: v12.root.clone(),
            workspace_root: v12.root.clone(),
            cache_root: v12.root.join(".build/unica"),
            workspace_epoch: 1,
        };

        let v12_first = patch_inner(&v12_args, &context, PatchMode::Apply);
        assert!(
            v12_first.outcome.ok,
            "{name}: {:?}",
            v12_first.outcome.errors
        );
        assert!(!v12_first.data.as_ref().unwrap().no_op, "{name}");

        let admission = staged_code_admission(&c1, false);
        let (mut state, effects) =
            plan_admitted_code(&admission, &c1.binding, std::slice::from_ref(&operation))
                .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
        assert_eq!(
            state.read(relative).unwrap().unwrap(),
            fs::read(v12.source.join(relative)).unwrap(),
            "{name}"
        );
        assert_eq!(effects.events().len(), 1, "{name}");
        c1.actor
            .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
            .unwrap();

        let v12_repeat = patch_inner(&v12_args, &context, PatchMode::Apply);
        let repeat_admission = staged_code_admission(&c1, true);
        let c1_repeat = plan_admitted_code(
            &repeat_admission,
            &c1.binding,
            std::slice::from_ref(&operation),
        );
        if repeat_is_noop {
            assert!(
                v12_repeat.outcome.ok,
                "{name}: {:?}",
                v12_repeat.outcome.errors
            );
            assert!(v12_repeat.data.unwrap().no_op, "{name}");
            let (state, effects) = c1_repeat.unwrap_or_else(|error| panic!("{name}: {error:?}"));
            assert!(state.planned_changes().is_empty(), "{name}");
            assert!(effects.events().is_empty(), "{name}");
        } else {
            assert!(!v12_repeat.outcome.ok, "{name}");
            assert_eq!(
                c1_repeat.unwrap_err().kind(),
                ApplyPlanErrorKind::NotFound,
                "{name}"
            );
        }
        assert_eq!(
            fs::read(v12.source.join(relative)).unwrap(),
            fs::read(c1.source.join(relative)).unwrap(),
            "{name}"
        );
        v12.cleanup();
        c1.cleanup();
    }

    fn staged_insert(
        at: &str,
        text: &str,
        selector: Option<CodeSelector>,
        position: Option<CodePosition>,
    ) -> CodePlanOperation {
        CodePlanOperation::Insert(CodeInsertArgs {
            at: crate::domain::address::QualifiedAddress::parse(at).unwrap(),
            text: text.to_string(),
            selector,
            position,
        })
    }

    fn staged_replace(at: &str, text: &str, selector: CodeSelector) -> CodePlanOperation {
        CodePlanOperation::Replace(CodeReplaceArgs {
            at: crate::domain::address::QualifiedAddress::parse(at).unwrap(),
            text: text.to_string(),
            selector,
        })
    }

    fn write_locked_support_state(source: &Path) {
        fs::create_dir_all(source.join("Ext")).unwrap();
        fs::write(
            source.join("Ext/ParentConfigurations.bin"),
            concat!(
                "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc,",
                "cccccccc-cccc-cccc-cccc-cccccccccccc}"
            ),
        )
        .unwrap();
    }

    fn snapshot_staged_code_tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
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

    #[test]
    fn parser_library_commit_matches_the_bundled_analyzer_contract() {
        let tools: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/unica/third-party/tools.lock.json"
        )))
        .unwrap();
        let expected = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "bsl-analyzer")
            .and_then(|tool| tool["sourceCommit"].as_str())
            .unwrap();
        let cargo_lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"));

        for package in ["parser", "syntax"] {
            let block = cargo_lock
                .split("[[package]]")
                .find(|block| block.contains(&format!("name = \"{package}\"")))
                .unwrap();
            assert!(
                block.contains(expected),
                "{package} must use the bundled bsl-analyzer sourceCommit {expected}"
            );
        }
    }

    #[test]
    fn diff_and_post_hash_are_derived_from_the_same_postimage() {
        let before = "Procedure Run()\nEndProcedure\n";
        let after = "Procedure Run()\nEndProcedure\nProcedure Added()\nEndProcedure\n";
        let diff = unified_diff("src/CommonModules/X/Ext/Module.bsl", before, after).unwrap();
        let rebuilt = apply(before, &Patch::from_str(&diff).unwrap()).unwrap();

        assert_eq!(hash(rebuilt.as_bytes()), hash(after.as_bytes()));
    }

    fn assert_module_identity(path: &str, owner: &str, role: &str) {
        let identity = module_identity(Path::new(path)).unwrap_or_else(|error| {
            panic!("expected canonical module identity for {path}: {error}")
        });
        assert_eq!(identity.owner, owner, "{path}");
        assert_eq!(identity.role.as_str(), role, "{path}");
    }

    fn arguments(selector: Value, position: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("insert"));
        args.insert("selector".to_string(), selector);
        args.insert("position".to_string(), json!(position));
        args
    }

    fn patch_args(
        source_set: &str,
        metadata_path: &str,
        method: &str,
        content: &str,
    ) -> Map<String, Value> {
        patch_args_for_selector(
            source_set,
            metadata_path,
            json!({"method": method}),
            "after",
            content,
        )
    }

    fn patch_args_for_selector(
        source_set: &str,
        metadata_path: &str,
        selector: Value,
        position: &str,
        content: &str,
    ) -> Map<String, Value> {
        let mut args = arguments(selector, position);
        args.insert("sourceSet".to_string(), json!(source_set));
        args.insert("metadataPath".to_string(), json!(metadata_path));
        args.insert("content".to_string(), json!(content));
        args
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let root = temp_root(&format!("code-patch-{name}"));
        fs::create_dir_all(root.join("src/CommonModules")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/CommonModules/Sample.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Sample</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-{name}-{}-{nanos}-{nonce}",
            std::process::id()
        ))
    }
}

use crate::application::AdapterOutcome;
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::metadata_kind_by_directory;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::project_sources::discover_project_source_map;
use crate::infrastructure::source_roots::{normalize_path_identity, resolve_source_root};
use bsl_syntax::ast::{AstNode, FunctionDef, ProcedureDef};
use diffy::{apply, DiffOptions, Patch};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::single_file_publisher::{publish, PublishMode, PublishRequest};

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
    path: String,
    source_set: String,
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
    path: String,
    source_set: String,
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

impl Selector {
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
enum Eol {
    Lf,
    CrLf,
}

impl Eol {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeadingSeparator {
    None,
    LocalEol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InsertionSite {
    offset: usize,
    position: Position,
    eol: Eol,
    leading_separator: LeadingSeparator,
}

fn patch_inner(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    mode: PatchMode,
) -> CodePatchExecution {
    match build_patch(args, context) {
        Ok(plan) => finish_patch(plan, mode),
        Err(error) => CodePatchExecution::failure(error, None),
    }
}

struct PatchPlan {
    target: ResolvedTarget,
    before: Vec<u8>,
    after: Vec<u8>,
    selector: Selector,
    content: String,
    insertion: Vec<u8>,
    site: InsertionSite,
    no_op: bool,
}

fn build_patch(args: &Map<String, Value>, context: &WorkspaceContext) -> Result<PatchPlan, String> {
    let target = resolve_target(args, context)?;
    let before = fs::read(&target.path)
        .map_err(|error| format!("failed to read {}: {error}", target.path.display()))?;
    let text = std::str::from_utf8(&before).map_err(|_| "BSL module must be UTF-8".to_string())?;
    let selector = Selector::parse(args)?;
    let position = Position::parse(string_arg(args, "position")?)?;
    if string_arg(args, "operation")? != "insert" {
        return Err("unica.code.patch v1 supports only operation=insert".to_string());
    }
    let indexed = analyze_module(text)?;
    reject_parse_diagnostics(&indexed.diagnostics, "validate original BSL module")?;
    let site = locate_selector(text, position, &selector, &indexed.methods)?;
    let content = string_arg(args, "content")?.to_string();
    let insertion = normalized_content(&content, site.eol, site.leading_separator);
    let no_op = insertion_is_present(text.as_bytes(), site, &insertion);
    let mut after = before.clone();
    if !no_op {
        after.splice(site.offset..site.offset, insertion.iter().copied());
    }
    Ok(PatchPlan {
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

fn finish_patch(plan: PatchPlan, mode: PatchMode) -> CodePatchExecution {
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
        if let Err(error) = publish(PublishRequest {
            target: &plan.target.path,
            replacement: &plan.after,
            mode: PublishMode::ReplaceExisting {
                expected_preimage: &plan.before,
            },
        }) {
            return CodePatchExecution::failure(format!("publish BSL module: {error}"), Some(data));
        }
    }
    let outcome = AdapterOutcome {
        ok: true,
        summary: if plan.no_op {
            "unica.code.patch is already applied".to_string()
        } else if mode == PatchMode::Preview {
            "dry run: unica.code.patch planned one insertion".to_string()
        } else {
            "unica.code.patch applied one insertion".to_string()
        },
        changes: (mode == PatchMode::Apply && !plan.no_op)
            .then(|| format!("{}: inserted BSL content", plan.target.path.display()))
            .into_iter()
            .collect(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: vec![plan.target.path.display().to_string()],
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
    plan: &PatchPlan,
    postimage: &str,
    post_hash: String,
    validation: SourceValidation,
) -> Result<CodePatchData, String> {
    let changed_ranges = if plan.no_op {
        Vec::new()
    } else {
        let (start_line, start_column) = line_column(postimage, plan.site.offset)?;
        let (end_line, end_column) =
            line_column(postimage, plan.site.offset + plan.insertion.len())?;
        vec![ChangedRange {
            start_byte: plan.site.offset,
            end_byte: plan.site.offset + plan.insertion.len(),
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
        unified_diff(&plan.target.relative_path, preimage, postimage)?
    };
    Ok(CodePatchData {
        path: plan.target.relative_path.clone(),
        source_set: plan.target.source_set.clone(),
        pre_hash: hash(&plan.before),
        post_hash: post_hash.clone(),
        no_op: plan.no_op,
        changed_ranges,
        diff,
        affected_target: AffectedTarget {
            path: plan.target.relative_path.clone(),
            source_set: plan.target.source_set.clone(),
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

struct ResolvedTarget {
    path: PathBuf,
    relative_path: String,
    source_set: String,
    owner: String,
    module_role: String,
}

#[derive(Debug)]
struct ModuleIdentity {
    owner: String,
    role: ModuleRole,
    descriptors: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleRole {
    Module,
    ObjectModule,
    ManagerModule,
    RecordSetModule,
    ValueManagerModule,
    FormModule,
    CommandModule,
    ManagedApplicationModule,
    OrdinaryApplicationModule,
    SessionModule,
    ExternalConnectionModule,
}

impl ModuleRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Module => "Module",
            Self::ObjectModule => "ObjectModule",
            Self::ManagerModule => "ManagerModule",
            Self::RecordSetModule => "RecordSetModule",
            Self::ValueManagerModule => "ValueManagerModule",
            Self::FormModule => "FormModule",
            Self::CommandModule => "CommandModule",
            Self::ManagedApplicationModule => "ManagedApplicationModule",
            Self::OrdinaryApplicationModule => "OrdinaryApplicationModule",
            Self::SessionModule => "SessionModule",
            Self::ExternalConnectionModule => "ExternalConnectionModule",
        }
    }
}

fn resolve_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<ResolvedTarget, String> {
    let requested = Path::new(string_arg(args, "path")?);
    if requested.is_absolute() {
        return Err("unica.code.patch v1 requires a workspace-relative `path`".to_string());
    }
    let target = WorkspacePathPolicy::new(context).resolve_write(requested)?;
    let target_metadata = fs::symlink_metadata(&target)
        .map_err(|error| format!("failed to inspect BSL module {}: {error}", target.display()))?;
    if metadata_is_link_or_reparse_point(&target_metadata) || !target_metadata.is_file() {
        return Err("unica.code.patch v1 accepts only an existing regular *Module.bsl".to_string());
    }
    let source_root = resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))?;
    let source_name = source_root
        .source_set
        .as_deref()
        .ok_or_else(|| "sourceDir must select a configured Configuration source set".to_string())?;
    let source_map = discover_project_source_map(&context.workspace_root)?;
    let source_set = source_map
        .source_sets
        .iter()
        .find(|set| set.name == source_name)
        .ok_or_else(|| "effective source set is unavailable".to_string())?;
    if source_set.kind != SourceSetKind::Configuration
        || source_set.source_format != SourceFormat::PlatformXml
    {
        return Err(
            "unica.code.patch v1 requires a platform XML Configuration source set".to_string(),
        );
    }
    let target_identity = normalize_path_identity(&target)?;
    if !target_identity.starts_with(&source_root.path) {
        return Err("BSL module is outside the selected Configuration source set".to_string());
    }
    let source_relative = target_identity
        .strip_prefix(&source_root.path)
        .map_err(|_| "failed to derive BSL module identity".to_string())?;
    let identity = module_identity(source_relative)?;
    validate_identity_descriptors(&source_root.path, &identity.descriptors)?;
    let workspace_identity = normalize_path_identity(&context.workspace_root)?;
    let workspace_relative = target_identity
        .strip_prefix(&workspace_identity)
        .map_err(|_| "BSL module is outside the normalized workspace root".to_string())?;
    let relative_path = portable_relative_path(workspace_relative)?;
    Ok(ResolvedTarget {
        path: target,
        relative_path,
        source_set: source_name.to_string(),
        owner: identity.owner,
        module_role: identity.role.as_str().to_string(),
    })
}

fn module_identity(relative: &Path) -> Result<ModuleIdentity, String> {
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "BSL module path is not valid UTF-8".to_string()),
            _ => Err("BSL module path must be relative to its source set".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let parts = components.iter().map(String::as_str).collect::<Vec<_>>();
    match parts.as_slice() {
        ["Ext", file] => configuration_module_identity(file),
        [directory, name, "Ext", "Module.bsl"]
            if matches!(
                *directory,
                "CommonModules" | "HTTPServices" | "WebServices" | "IntegrationServices"
            ) =>
        {
            metadata_module_identity(directory, name, ModuleRole::Module)
        }
        ["CommonForms", name, "Ext", "Form", "Module.bsl"] => Ok(ModuleIdentity {
            owner: format!("CommonForm.{name}"),
            role: ModuleRole::FormModule,
            descriptors: vec![metadata_descriptor("CommonForms", name)],
        }),
        ["CommonCommands", name, "Ext", "CommandModule.bsl"] => Ok(ModuleIdentity {
            owner: format!("CommonCommand.{name}"),
            role: ModuleRole::CommandModule,
            descriptors: vec![metadata_descriptor("CommonCommands", name)],
        }),
        [directory, name, "Ext", file] => {
            let role = direct_module_role(file).ok_or_else(unsupported_module_layout)?;
            let kind =
                metadata_kind_by_directory(directory).ok_or_else(unsupported_module_layout)?;
            if !direct_role_is_supported(kind.tag, role) {
                return Err(unsupported_module_layout());
            }
            Ok(ModuleIdentity {
                owner: format!("{}.{name}", kind.tag),
                role,
                descriptors: vec![metadata_descriptor(directory, name)],
            })
        }
        [directory, name, "Forms", form, "Ext", "Form", "Module.bsl"] => {
            nested_module_identity(directory, name, "Form", form, ModuleRole::FormModule)
        }
        [directory, name, "Commands", command, "Ext", "CommandModule.bsl"] => {
            nested_module_identity(
                directory,
                name,
                "Command",
                command,
                ModuleRole::CommandModule,
            )
        }
        _ => Err(unsupported_module_layout()),
    }
}

fn configuration_module_identity(file: &str) -> Result<ModuleIdentity, String> {
    let role = match file {
        "ManagedApplicationModule.bsl" => ModuleRole::ManagedApplicationModule,
        "OrdinaryApplicationModule.bsl" => ModuleRole::OrdinaryApplicationModule,
        "SessionModule.bsl" => ModuleRole::SessionModule,
        "ExternalConnectionModule.bsl" => ModuleRole::ExternalConnectionModule,
        _ => return Err(unsupported_module_layout()),
    };
    Ok(ModuleIdentity {
        owner: "Configuration".to_string(),
        role,
        descriptors: vec![PathBuf::from("Configuration.xml")],
    })
}

fn metadata_module_identity(
    directory: &str,
    name: &str,
    role: ModuleRole,
) -> Result<ModuleIdentity, String> {
    let kind = metadata_kind_by_directory(directory).ok_or_else(unsupported_module_layout)?;
    Ok(ModuleIdentity {
        owner: format!("{}.{name}", kind.tag),
        role,
        descriptors: vec![metadata_descriptor(directory, name)],
    })
}

fn nested_module_identity(
    directory: &str,
    name: &str,
    nested_kind: &str,
    nested_name: &str,
    role: ModuleRole,
) -> Result<ModuleIdentity, String> {
    let kind = metadata_kind_by_directory(directory).ok_or_else(unsupported_module_layout)?;
    if !nested_modules_are_supported(kind.tag) {
        return Err(unsupported_module_layout());
    }
    let child_directory = match nested_kind {
        "Form" => "Forms",
        "Command" => "Commands",
        _ => return Err(unsupported_module_layout()),
    };
    Ok(ModuleIdentity {
        owner: format!("{}.{name}", kind.tag),
        role,
        descriptors: vec![
            metadata_descriptor(directory, name),
            PathBuf::from(directory)
                .join(name)
                .join(child_directory)
                .join(nested_name)
                .join("Ext")
                .join(format!("{nested_kind}.xml")),
        ],
    })
}

fn direct_module_role(file: &str) -> Option<ModuleRole> {
    match file {
        "ObjectModule.bsl" => Some(ModuleRole::ObjectModule),
        "ManagerModule.bsl" => Some(ModuleRole::ManagerModule),
        "RecordSetModule.bsl" => Some(ModuleRole::RecordSetModule),
        "ValueManagerModule.bsl" => Some(ModuleRole::ValueManagerModule),
        _ => None,
    }
}

fn direct_role_is_supported(kind: &str, role: ModuleRole) -> bool {
    match role {
        ModuleRole::ObjectModule => matches!(
            kind,
            "Catalog"
                | "Document"
                | "ExchangePlan"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "Report"
                | "DataProcessor"
        ),
        ModuleRole::ManagerModule => matches!(
            kind,
            "Catalog"
                | "Document"
                | "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
                | "ChartOfAccounts"
                | "ChartOfCharacteristicTypes"
                | "ChartOfCalculationTypes"
                | "BusinessProcess"
                | "Task"
                | "ExchangePlan"
                | "Enum"
                | "Report"
                | "DataProcessor"
                | "Constant"
                | "DocumentJournal"
                | "FilterCriterion"
                | "SettingsStorage"
        ),
        ModuleRole::RecordSetModule => matches!(
            kind,
            "InformationRegister"
                | "AccumulationRegister"
                | "AccountingRegister"
                | "CalculationRegister"
        ),
        ModuleRole::ValueManagerModule => kind == "Constant",
        ModuleRole::Module
        | ModuleRole::FormModule
        | ModuleRole::CommandModule
        | ModuleRole::ManagedApplicationModule
        | ModuleRole::OrdinaryApplicationModule
        | ModuleRole::SessionModule
        | ModuleRole::ExternalConnectionModule => false,
    }
}

fn nested_modules_are_supported(kind: &str) -> bool {
    matches!(
        kind,
        "Document"
            | "Catalog"
            | "DataProcessor"
            | "Report"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
            | "DocumentJournal"
            | "Enum"
            | "Constant"
            | "Sequence"
            | "DocumentNumerator"
    )
}

fn metadata_descriptor(directory: &str, name: &str) -> PathBuf {
    PathBuf::from(directory).join(format!("{name}.xml"))
}

fn validate_identity_descriptors(
    source_root: &Path,
    descriptors: &[PathBuf],
) -> Result<(), String> {
    for descriptor in descriptors {
        let path = source_root.join(descriptor);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "BSL module metadata descriptor is unavailable {}: {error}",
                path.display()
            )
        })?;
        if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
            return Err(format!(
                "BSL module metadata descriptor must be a regular file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn unsupported_module_layout() -> String {
    "unica.code.patch v1 accepts only a supported canonical platform XML BSL module path"
        .to_string()
}

fn portable_relative_path(path: &Path) -> Result<String, String> {
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "workspace-relative BSL path is not valid UTF-8".to_string()),
            _ => Err("workspace-relative BSL path contains an invalid component".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|components| components.join("/"))
}

#[cfg(test)]
fn locate_insertion(text: &str, args: &Map<String, Value>) -> Result<InsertionSite, String> {
    if string_arg(args, "operation")? != "insert" {
        return Err("unica.code.patch v1 supports only operation=insert".to_string());
    }
    let position = Position::parse(string_arg(args, "position")?)?;
    let selector = Selector::parse(args)?;
    let indexed = analyze_module(text)?;
    reject_parse_diagnostics(&indexed.diagnostics, "validate original BSL module")?;
    locate_selector(text, position, &selector, &indexed.methods)
}

fn locate_selector(
    text: &str,
    position: Position,
    selector: &Selector,
    methods: &[Method],
) -> Result<InsertionSite, String> {
    let offset = match selector {
        Selector::Method(name) => {
            let folded_name = name.to_lowercase();
            let found = methods
                .iter()
                .filter(|method| method.name.to_lowercase() == folded_name)
                .collect::<Vec<_>>();
            let method = match found.as_slice() {
                [method] => *method,
                _ => {
                    return Err(format!(
                        "method selector must match exactly once; matched {} times",
                        found.len()
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
                _ => {
                    return Err(format!(
                        "anchor selector must match exactly once; matched {} times",
                        found.len()
                    ))
                }
            };
            match position {
                Position::Before => safe_line_start(text, selected.start),
                Position::After => anchor_line_end(text, selected.end),
            }
        }
    };
    let eol = local_eol_at(text, offset, position);
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
    plan: &PatchPlan,
    methods: &[Method],
) -> Result<(), String> {
    let repeat_site = locate_selector(postimage, plan.site.position, &plan.selector, methods)
        .map_err(|error| {
            format!("patch cannot be applied idempotently on the next call: {error}")
        })?;
    let repeat_insertion = normalized_content(
        &plan.content,
        repeat_site.eol,
        repeat_site.leading_separator,
    );
    if insertion_is_present(postimage.as_bytes(), repeat_site, &repeat_insertion) {
        Ok(())
    } else {
        Err(
            "patch cannot be applied idempotently on the next call: repeated planning would change bytes"
                .to_string(),
        )
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

fn local_eol_at(text: &str, offset: usize, position: Position) -> Eol {
    let bytes = text.as_bytes();
    let before = bytes
        .get(..offset)
        .and_then(|prefix| prefix.iter().rposition(|byte| *byte == b'\n'))
        .map(|newline| eol_at_newline(bytes, newline));
    let after = bytes
        .get(offset..)
        .and_then(|suffix| suffix.iter().position(|byte| *byte == b'\n'))
        .map(|newline| eol_at_newline(bytes, offset + newline));
    match position {
        Position::Before => after.or(before).unwrap_or(Eol::Lf),
        Position::After => before.or(after).unwrap_or(Eol::Lf),
    }
}

fn eol_at_newline(bytes: &[u8], newline: usize) -> Eol {
    if newline > 0 && bytes.get(newline - 1) == Some(&b'\r') {
        Eol::CrLf
    } else {
        Eol::Lf
    }
}

fn canonicalize_eol(content: &str) -> String {
    content.replace("\r\n", "\n").replace('\r', "\n")
}

fn normalized_content(content: &str, eol: Eol, leading_separator: LeadingSeparator) -> Vec<u8> {
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
mod tests {
    use super::{
        analyze_module, hash, insertion_is_present, line_column, locate_insertion, module_identity,
        normalized_content, patch_inner, unified_diff, Eol, LeadingSeparator, PatchMode, Position,
        ValidationStatus,
    };
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use diffy::{apply, Patch};
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
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
        assert_eq!(site.eol, Eol::Lf);
    }

    #[test]
    fn multiline_anchor_matches_mixed_source_eol_and_uses_real_byte_range() {
        let module =
            "Procedure Run()\r\n    First();\n    Second();\r\n    Third();\nEndProcedure\n";
        let anchor = "    First();\n    Second();\n";
        let args = arguments(json!({"anchor": anchor}), "after");

        let site = locate_insertion(module, &args).unwrap();

        assert_eq!(site.offset, module.find("    Third();").unwrap());
        assert_eq!(site.eol, Eol::CrLf);
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

        assert_eq!(site.eol, Eol::Lf);
    }

    #[test]
    fn inserted_content_uses_local_line_ending_once() {
        assert_eq!(
            normalized_content("A\r\nB", Eol::Lf, LeadingSeparator::None),
            b"A\nB\n"
        );
        assert_eq!(
            normalized_content("A\nB", Eol::CrLf, LeadingSeparator::None),
            b"A\r\nB\r\n"
        );
        assert_eq!(
            normalized_content("A", Eol::Lf, LeadingSeparator::LocalEol),
            b"\nA\n"
        );
    }

    #[test]
    fn repeated_before_or_after_insertion_is_a_noop() {
        let before = b"// marker\nProcedure First()";
        let before_site = super::InsertionSite {
            offset: 10,
            position: Position::Before,
            eol: Eol::Lf,
            leading_separator: LeadingSeparator::None,
        };
        assert!(insertion_is_present(before, before_site, b"// marker\n"));

        let after = b"Procedure First()\n// marker\n";
        let after_site = super::InsertionSite {
            offset: after.len() - b"// marker\n".len(),
            position: Position::After,
            eol: Eol::Lf,
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
    fn applied_patch_returns_typed_data_and_repeated_apply_is_noop() {
        let context = temp_context("applied-patch");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "\u{feff}Procedure Run()\r\nEndProcedure\r\n").unwrap();
        let args = patch_args(
            "src/CommonModules/Sample/Ext/Module.bsl",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert!(applied.outcome.stdout.is_none());
        let data = applied.data.unwrap();
        assert_eq!(data.source_set, "main");
        assert_eq!(data.affected_target.owner, "CommonModule.Sample");
        assert_eq!(data.affected_target.module_role, "Module");
        assert_eq!(data.validation.status, ValidationStatus::Passed);
        assert_eq!(data.validation.validated_post_hash, data.post_hash);
        assert_eq!(data.changed_ranges[0].start_line, 3);
        assert!(data.diff.starts_with("--- a/"));

        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
        assert!(repeated.outcome.changes.is_empty());
        let data = repeated.data.unwrap();
        assert_eq!(data.pre_hash, data.post_hash);
        assert!(data.changed_ranges.is_empty());
        assert!(data.diff.is_empty());
        fs::remove_dir_all(&context.workspace_root).unwrap();
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
            "src/CommonModules/Sample/Ext/Module.bsl",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);

        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        let serialized = serde_json::to_value(preview.data.unwrap()).unwrap();
        let pre_hash = hash(before.as_bytes());
        let post_hash = hash(after.as_bytes());
        let diff = unified_diff("src/CommonModules/Sample/Ext/Module.bsl", before, &after).unwrap();
        assert_eq!(
            serialized,
            json!({
                "path": "src/CommonModules/Sample/Ext/Module.bsl",
                "sourceSet": "main",
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
                    "path": "src/CommonModules/Sample/Ext/Module.bsl",
                    "sourceSet": "main",
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
            "src/CommonModules/Sample/Ext/Module.bsl",
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
            "src/CommonModules/Sample/Ext/Module.bsl",
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
            "src/CommonModules/Sample/Ext/Module.bsl",
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
            "src/CommonModules/Sample/Ext/Module.bsl",
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
            "src/CommonModules/Sample/Ext/Module.bsl",
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
            "src/CommonModules/Sample/Ext/Module.bsl",
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
    fn object_and_manager_modules_report_owner_and_role() {
        let context = temp_context("module-roles");
        fs::create_dir_all(context.workspace_root.join("src/Catalogs")).unwrap();
        fs::write(
            context.workspace_root.join("src/Catalogs/Items.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();
        for role in ["ObjectModule", "ManagerModule"] {
            let relative = format!("src/Catalogs/Items/Ext/{role}.bsl");
            let module = context.workspace_root.join(&relative);
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
            let args = patch_args(&relative, "Run", "Procedure Added()\nEndProcedure");

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
        fs::write(src.join("Catalogs/Items.xml"), "<MetaDataObject/>").unwrap();
        fs::write(
            src.join("Catalogs/Items/Forms/Main/Ext/Form.xml"),
            "<Form/>",
        )
        .unwrap();
        fs::write(
            src.join("Catalogs/Items/Commands/Print/Ext/Command.xml"),
            "<Command/>",
        )
        .unwrap();
        let cases = [
            (
                "src/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
                "FormModule",
            ),
            (
                "src/Catalogs/Items/Commands/Print/Ext/CommandModule.bsl",
                "CommandModule",
            ),
        ];
        for (relative, role) in cases {
            let module = context.workspace_root.join(relative);
            fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
            let args = patch_args(relative, "Run", "Procedure Added()\nEndProcedure");

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
    fn target_requires_relative_regular_file_and_metadata_descriptors() {
        let context = temp_context("target-contract");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();

        let absolute = patch_args(
            module.to_str().unwrap(),
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        let absolute_result = patch_inner(&absolute, &context, PatchMode::Preview);
        assert!(!absolute_result.outcome.ok);
        assert!(absolute_result.outcome.errors[0].contains("workspace-relative"));

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
            "src/CommonModules/Missing/Ext/Module.bsl",
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        let missing_result = patch_inner(&missing_descriptor, &context, PatchMode::Preview);
        assert!(!missing_result.outcome.ok);
        assert!(missing_result.outcome.errors[0].contains("descriptor is unavailable"));

        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn symlink_module_is_rejected_during_preview() {
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
            "src/CommonModules/Sample/Ext/Module.bsl",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let result = patch_inner(&args, &context, PatchMode::Preview);

        assert!(!result.outcome.ok);
        assert!(result.outcome.errors[0].contains("regular *Module.bsl"));
        fs::remove_dir_all(&context.workspace_root).unwrap();
    }

    #[test]
    fn invalid_postimage_returns_failed_validation_without_writing() {
        let context = temp_context("validation-failure");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\n    Message(\"ok\");\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args_for_selector(
            "src/CommonModules/Sample/Ext/Module.bsl",
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
    fn dry_run_reports_the_same_postimage_without_writing() {
        let context = temp_context("dry-run");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let before = b"Procedure Run()\nEndProcedure\n";
        fs::write(&module, before).unwrap();
        let args = patch_args(
            "src/CommonModules/Sample/Ext/Module.bsl",
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
    fn patch_refuses_a_stale_preimage_without_overwriting_concurrent_change() {
        let context = temp_context("stale-preimage");
        let module = context
            .workspace_root
            .join("src/CommonModules/Sample/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
        let args = patch_args(
            "src/CommonModules/Sample/Ext/Module.bsl",
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

    fn patch_args(path: &str, method: &str, content: &str) -> Map<String, Value> {
        patch_args_for_selector(path, json!({"method": method}), "after", content)
    }

    fn patch_args_for_selector(
        path: &str,
        selector: Value,
        position: &str,
        content: &str,
    ) -> Map<String, Value> {
        let mut args = arguments(selector, position);
        args.insert("path".to_string(), json!(path));
        args.insert("content".to_string(), json!(content));
        args.insert("sourceDir".to_string(), json!("src"));
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
        fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
        fs::write(
            root.join("src/CommonModules/Sample.xml"),
            "<MetaDataObject/>",
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

#![allow(clippy::result_large_err)]
//! `make` — сборка файла CF, CFE, EPF или ERF
//! из исходников рабочего пространства силами `v8-runner make` (A-7 зонтика
//! #871). В отличие от `download`, который выносит конфигурацию из базы, тут
//! источник — исходники, а база служит раннеру временной площадкой.
//!
//! Аргументы закрыты: `output` — относительный путь к `.cf` или `.cfe` внутри
//! рабочего пространства; `sourceSet` — имя объявленного набора, если их
//! несколько; `extension` — имя расширения в базе, обязательное для `.cfe` и
//! недопустимое для других видов. EPF/ERF требуют singleton sourceSet и
//! основной конфигурации. Их каталог раннера приватен; наружу публикуется
//! только независимо проверенный файл.
//!
//! Превью зовёт `make --dry-run`: раннер называет вид артефакта, набор и
//! выход, ничего не публикуя. Раннер принимает выход и вне проекта — Unica
//! ограничивает его рабочим пространством до вызова. Применение повторяет
//! превью, сверяет забор ревизии, собирает и снимает с выхода независимую
//! квитанцию: размер и дайджест файла. Путь к платформе наружу не идёт.

use super::protocol::InvocationRequest;
use super::runner_015::Runner015ProcessRunner;
use super::v13_infobase_exports::{
    closed_workspace_relative_path, configuration_source_set, digest_optional_workspace_file,
    digest_required_workspace_file, missing_runner_rejection, resolve_bundled_runner,
    runner_rejection, runner_start_rejection, valid_1c_identifier, RunnerAbsence, CONFIG_NAME,
    LOCAL_CONFIG_NAME, RUNNER_OUTPUT_LIMIT,
};
use super::v13_source_set_name::{source_set_name_guidance, valid_source_set_name};
use crate::infrastructure::platform_xml_owner::resolve_platform_xml_owners;
use crate::infrastructure::source_roots::resolve_named_source_set;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unica_artifact_delivery::engine::BundledTool;
use unica_execution::invocation_store::ToolIdentity;
use unica_execution_contract::invocation::{DomainResult, SafeIdentityHash};
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
use unica_redaction::redactor;
use unica_refusal::RefusalCode;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::SourceSetKind;
use unica_source_designer::owner::prove_already_read_source_set_owner;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::compile_transaction::{CommitFailure, CommitFailureKind, CompileTransaction};
use unica_workspace::discovery::inspect_declared_source_root_route;
use unica_workspace::path_policy::WorkspacePathPolicy;
use unica_workspace::paths::normalize_path_identity;

pub(super) const OPERATION: &str = "make";
/// Имя команды в конверте раннера: словарь читается как слой и направление,
/// раннер называет свои команды по-своему.
const RUNNER_COMMAND: &str = "make";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    Cf,
    Cfe,
    Epf,
    Erf,
}

impl ArtifactKind {
    const fn is_external(self) -> bool {
        matches!(self, Self::Epf | Self::Erf)
    }

    const fn suffix(self) -> &'static str {
        match self {
            Self::Cf => "cf",
            Self::Cfe => "cfe",
            Self::Epf => "epf",
            Self::Erf => "erf",
        }
    }

    /// Как раннер называет вид артефакта в `mode` и `payload.artifact_type`.
    const fn runner_mode(self) -> &'static str {
        match self {
            Self::Cf => "configuration_cf",
            Self::Cfe => "extension_cfe",
            Self::Epf => "external_data_processor_epf",
            Self::Erf => "external_report_erf",
        }
    }
}

#[derive(Debug, Clone)]
struct BuildArguments {
    kind: ArtifactKind,
    source_set: Option<String>,
    extension: Option<String>,
    output_relative: PathBuf,
    output: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedArtifactBuild {
    arguments: BuildArguments,
    dry_run: bool,
    context: WorkspaceContext,
}

pub(super) enum Preparation {
    NotApplicable,
    Rejected(Box<DomainResult>),
    Ready(Arc<PreparedArtifactBuild>),
}

pub(super) fn prepare(request: &InvocationRequest) -> Preparation {
    if request.tool() != ToolIdentity::Run
        || request.arguments().get("op").and_then(Value::as_str) != Some(OPERATION)
    {
        return Preparation::NotApplicable;
    }
    match PreparedArtifactBuild::parse(request) {
        Ok(prepared) => Preparation::Ready(Arc::new(prepared)),
        Err(result) => Preparation::Rejected(Box::new(result)),
    }
}

impl PreparedArtifactBuild {
    fn parse(request: &InvocationRequest) -> Result<Self, DomainResult> {
        let arguments = request.arguments();
        let args = arguments
            .get("args")
            .and_then(Value::as_object)
            .ok_or_else(|| reject(RefusalCode::BadValue, "run args must be an object"))?;
        let dry_run = arguments
            .get("dryRun")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                reject(
                    RefusalCode::BadValue,
                    "make requires dryRun: true to preview or dryRun: false to execute",
                )
            })?;
        let context =
            discover_workspace(Some(PathBuf::from(request.workspace_hint()))).map_err(|error| {
                reject(
                    RefusalCode::ProviderUnavailable,
                    format!("workspace discovery failed: {error}"),
                )
            })?;
        let arguments = parse_build_arguments(args, &context)?;
        Ok(Self {
            arguments,
            dry_run,
            context,
        })
    }

    pub(super) fn workspace_identity_hash(&self) -> SafeIdentityHash {
        let mut hasher = Sha256::new();
        hasher.update(b"unica-v13-artifact-build-workspace-v1\0");
        hasher.update(self.context.workspace_root.as_os_str().as_encoded_bytes());
        SafeIdentityHash::from_sha256(hasher.finalize().into())
    }

    pub(super) fn execute(&self, cancellation: CancellationToken) -> DomainResult {
        execute_with_runner(self, &Runner015ProcessRunner, cancellation)
    }
}

fn parse_build_arguments(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<BuildArguments, DomainResult> {
    const ACCEPTED: [&str; 3] = ["output", "sourceSet", "extension"];
    if let Some(unknown) = args.keys().find(|key| !ACCEPTED.contains(&key.as_str())) {
        return Err(reject(
            RefusalCode::BadValue,
            format!("make does not accept `{unknown}`; the closed args are output, sourceSet and extension"),
        ));
    }
    let source_set = match args.get("sourceSet") {
        None => None,
        Some(Value::String(value)) if valid_source_set_name(value) => Some(value.clone()),
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                source_set_name_guidance(OPERATION),
            ))
        }
    };
    let extension = match args.get("extension") {
        None => None,
        Some(Value::String(value)) if valid_1c_identifier(value) => Some(value.clone()),
        Some(_) => {
            return Err(reject(
                RefusalCode::BadValue,
                "make extension must be a non-empty 1C identifier",
            ))
        }
    };
    let output = args
        .get("output")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| reject(RefusalCode::BadValue, "make output must be non-empty text"))?;
    let output_relative = closed_workspace_relative_path(output)
        .map_err(|message| reject(RefusalCode::BadValue, format!("make output {message}")))?;
    let kind = match output_relative
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("cf") => ArtifactKind::Cf,
        Some("cfe") => ArtifactKind::Cfe,
        Some("epf") => ArtifactKind::Epf,
        Some("erf") => ArtifactKind::Erf,
        _ => {
            return Err(reject(
                RefusalCode::BadValue,
                "make output must end in .cf for a configuration or .cfe for an extension, .epf for a processor or .erf for a report",
            ))
        }
    };
    match (kind, extension.is_some()) {
        (ArtifactKind::Cfe, false) => {
            return Err(reject(
                RefusalCode::BadValue,
                "make output .cfe requires extension: the name the infobase knows it by",
            ))
        }
        (_, true) if kind != ArtifactKind::Cfe => {
            return Err(reject(
                RefusalCode::BadValue,
                "make extension is only for a .cfe output; a .cf is the main configuration",
            ))
        }
        _ => {}
    }
    if kind.is_external() && source_set.is_none() {
        return Err(reject(
            RefusalCode::BadValue,
            "make external output requires sourceSet naming one external object set",
        ));
    }
    let root_context = WorkspaceContext {
        cwd: context.workspace_root.clone(),
        workspace_root: context.workspace_root.clone(),
        cache_root: context.cache_root.clone(),
        workspace_epoch: context.workspace_epoch,
    };
    // Раннер принимает выход и вне проекта; поверхность — нет: та же политика,
    // что у выгрузок, путь внутри пространства и без ссылок.
    let output_path = WorkspacePathPolicy::new(&root_context)
        .resolve_write(&output_relative)
        .map_err(|error| reject(RefusalCode::BadValue, error))?;
    Ok(BuildArguments {
        kind,
        source_set,
        extension,
        output_relative,
        output: output_path,
    })
}

/// Проектный файл с локальным дополнением, объявленный состав и состояние
/// выхода: план действителен, пока они прежние. Содержимое исходников забором
/// не держится: сборка читает их целиком, и другой результат — другой файл,
/// а не отказ.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StableInputs {
    config_sha256: String,
    local_config_sha256: Option<String>,
    declared: Vec<String>,
    output_sha256: Option<String>,
    external: Option<ExternalPlan>,
}

fn capture_inputs(prepared: &PreparedArtifactBuild) -> Result<StableInputs, DomainResult> {
    let root = &prepared.context.workspace_root;
    let config_sha256 =
        digest_required_workspace_file(root, Path::new(CONFIG_NAME)).map_err(|error| {
            let mut result = reject(RefusalCode::InvalidState, error);
            result.next.push(json!({
                "tool": "unica.view",
                "args": {},
                "reason": "inspect workspace setup and the required v8project.yaml recipe"
            }));
            result
        })?;
    let local_config_sha256 = digest_optional_workspace_file(root, Path::new(LOCAL_CONFIG_NAME))
        .map_err(|error| reject(RefusalCode::InvalidState, error))?
        .map(|(digest, _)| digest);
    let declared = declared_source_sets(&root.join(CONFIG_NAME))?;
    if let Some(source_set) = &prepared.arguments.source_set {
        if !declared.contains(source_set) {
            return Err(reject(
                RefusalCode::BadValue,
                format!(
                    "make sourceSet `{source_set}` is not declared in v8project.yaml; declared source sets: {}",
                    declared.join(", ")
                ),
            ));
        }
    }
    let output_sha256 = digest_optional_workspace_file(root, &prepared.arguments.output_relative)
        .map_err(|error| reject(RefusalCode::BadValue, error))?
        .map(|(digest, _)| digest);
    Ok(StableInputs {
        config_sha256,
        local_config_sha256,
        declared,
        output_sha256,
        external: external_plan(prepared)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalPlan {
    at: String,
    configuration_set: String,
    file_name: String,
    descriptor_sha256: String,
}

fn external_plan(prepared: &PreparedArtifactBuild) -> Result<Option<ExternalPlan>, DomainResult> {
    if !prepared.arguments.kind.is_external() {
        return Ok(None);
    }
    let fail = |message| reject(RefusalCode::InvalidState, message);
    let name = prepared
        .arguments
        .source_set
        .as_deref()
        .ok_or_else(|| fail("external sourceSet is required".to_string()))?;
    let source =
        resolve_named_source_set(&prepared.context, name).map_err(|e| fail(e.to_string()))?;
    let expected_kind = if prepared.arguments.kind == ArtifactKind::Epf {
        SourceSetKind::ExternalProcessor
    } else {
        SourceSetKind::ExternalReport
    };
    if source.source_set.kind != expected_kind {
        return Err(reject(
            RefusalCode::BadValue,
            "make sourceSet kind does not match the external output",
        ));
    }
    if !source.source_set.source_format.is_supported() {
        return Err(reject(
            RefusalCode::InvalidState,
            "external make requires readable Designer platform XML sources",
        ));
    }
    let configuration_set = configuration_source_set(&prepared.context.workspace_root)
        .map_err(fail)?
        .ok_or_else(|| {
            fail("external make requires a declared main configuration source-set".to_string())
        })?;
    let map = crate::infrastructure::project_sources::discover_project_source_map(
        &prepared.context.workspace_root,
    )
    .map_err(fail)?;
    // Compare physical identities: on a case-insensitive volume a lexical
    // `External/x.epf` names the `external` source root.
    let output_identity = normalize_path_identity(&prepared.arguments.output).map_err(fail)?;
    for set in map.source_sets {
        let resolved =
            inspect_declared_source_root_route(&prepared.context.workspace_root, &set.path)
                .map_err(|e| fail(e.to_string()))?;
        if output_identity.starts_with(&resolved.identity_path) {
            return Err(reject(
                RefusalCode::BadValue,
                "make output must not overlap any source-set",
            ));
        }
    }
    let owners = resolve_platform_xml_owners(&source.path, &prepared.context)
        .map_err(|e| fail(e.message))?;
    if owners.len() != 1 {
        return Err(reject(RefusalCode::BadValue, "make external sourceSet must contain exactly one object; multiple owners are ambiguous"));
    }
    let owner = &owners[0];
    let evidence = prove_already_read_source_set_owner(&owner.path, &owner.raw, expected_kind)
        .map_err(|e| fail(e.message))?;
    let logical_name = evidence
        .artifact_name()
        .filter(|name| valid_1c_identifier(name))
        .ok_or_else(|| fail("external descriptor must declare a valid 1C Name".to_string()))?;
    Ok(Some(ExternalPlan {
        at: format!("{name}:{}.{logical_name}", evidence.artifact_kind()),
        configuration_set,
        file_name: format!("{logical_name}.{}", prepared.arguments.kind.suffix()),
        descriptor_sha256: format!("{:x}", Sha256::digest(&owner.raw)),
    }))
}

/// The runner's private output root for an external artifact. Like the full
/// export stage it is owner-only from creation (Unix mode 0700, a protected
/// owner DACL on Windows), not tightened afterwards; dropping it removes it.
struct PrivateStaging(std::path::PathBuf);

impl PrivateStaging {
    fn create() -> std::io::Result<Self> {
        let parent =
            normalize_path_identity(&std::env::temp_dir()).map_err(std::io::Error::other)?;
        let name = format!("unica-make-{}", uuid::Uuid::new_v4());
        let parent_handle = unica_workspace::state_fs::open_state_directory_nofollow(&parent)?;
        let created = unica_workspace::state_fs::create_state_directory_child(
            &parent_handle,
            std::ffi::OsStr::new(&name),
        )?;
        let staging = Self(parent.join(name));
        // Unconfirmed privacy refuses before dispatch; dropping cleans it up.
        unica_workspace::state_fs::verify_state_owner_only(&created)?;
        Ok(staging)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateStaging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runner replaces its whole output directory. It never sees the public
/// destination; this adapter publishes one checked regular file atomically.
fn publish_external(
    prepared: &PreparedArtifactBuild,
    staging: &Path,
    before: &StableInputs,
    cancellation: &CancellationToken,
) -> Result<(), DomainResult> {
    let plan = before.external.as_ref().expect("external plan");
    let invalid = |message| reject(RefusalCode::InvalidResult, message);
    let directory = unica_workspace::read::open_source_directory(staging)
        .map_err(|_| invalid("external build output directory is missing or linked"))?;
    let names = directory
        .read_immediate_names_with_limit(None, || Ok(()))
        .map_err(|_| invalid("cannot inspect external build output"))?;
    if names.len() != 1 || names[0] != std::ffi::OsStr::new(&plan.file_name) {
        return Err(invalid(
            "external build must produce exactly the planned artifact and no other entries",
        ));
    }
    let mut file = directory
        .open_relative_regular_nofollow(Path::new(&plan.file_name))
        .map_err(|_| invalid("external artifact is not a regular file or is linked"))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes)
        .map_err(|_| invalid("cannot read external artifact"))?;
    if bytes.is_empty() {
        return Err(invalid("external artifact is empty"));
    }
    let mut publication = CompileTransaction::new();
    publication
        .create_or_replace_bytes(&prepared.arguments.output, bytes)
        .map_err(|_| invalid("cannot prepare external artifact publication"))?;
    if capture_inputs(prepared)? != *before {
        return Err(reject(
            RefusalCode::ConcurrentChange,
            "make inputs changed before publication; preview again",
        ));
    }
    if cancellation.is_cancelled() {
        return Err(reject(
            RefusalCode::Cancelled,
            "make cancelled before external artifact publication",
        ));
    }
    let mut cancelled_before_publication = false;
    publication
        .commit_with_classified_pre_and_post_validation(
            || {
                if cancellation.is_cancelled() {
                    cancelled_before_publication = true;
                    Err(CommitFailure::provider(
                        "external artifact publication cancelled",
                    ))
                } else {
                    Ok(())
                }
            },
            || Ok(()),
        )
        .map_err(|error| {
            reject(
                match error.kind() {
                    kind if cancelled_before_publication
                        && kind != CommitFailureKind::RollbackFailed =>
                    {
                        RefusalCode::Cancelled
                    }
                    CommitFailureKind::ConcurrentModification => RefusalCode::ConcurrentChange,
                    CommitFailureKind::ProviderUnavailable => RefusalCode::InvalidState,
                    CommitFailureKind::RollbackFailed => RefusalCode::InvalidResult,
                },
                if error.kind() == CommitFailureKind::ProviderUnavailable
                    && !cancelled_before_publication
                {
                    "external artifact target is not writable or publication is unavailable"
                } else {
                    "external artifact publication failed"
                },
            )
        })?;
    Ok(())
}

fn declared_source_sets(config: &Path) -> Result<Vec<String>, DomainResult> {
    let text = std::fs::read_to_string(config)
        .map_err(|error| reject(RefusalCode::InvalidState, format!("{CONFIG_NAME}: {error}")))?;
    let document: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| {
        reject(
            RefusalCode::InvalidState,
            format!(
                "{CONFIG_NAME} is not valid YAML: {}",
                redactor(&error.to_string())
            ),
        )
    })?;
    let mut names = Vec::new();
    for entry in document
        .get("source-set")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
    {
        let Some(name) = entry.get("name").and_then(serde_yaml::Value::as_str) else {
            return Err(reject(
                RefusalCode::InvalidState,
                format!("{CONFIG_NAME} declares a source-set without a name"),
            ));
        };
        names.push(name.to_string());
    }
    if names.is_empty() {
        return Err(reject(
            RefusalCode::InvalidState,
            format!("{CONFIG_NAME} declares no source-set; there is nothing to build"),
        ));
    }
    Ok(names)
}

fn execute_with_runner(
    prepared: &PreparedArtifactBuild,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
) -> DomainResult {
    let runner_tool = match resolve_bundled_runner(&prepared.context.cwd) {
        Ok(resolved) => resolved,
        Err(message) => return reject_absent_runner(message),
    };
    let tool = runner_tool.tool;
    let runner_version = runner_tool.version;
    execute_with_resolved_runner(prepared, runner, cancellation, &tool, &runner_version)
}

fn execute_with_resolved_runner(
    prepared: &PreparedArtifactBuild,
    runner: &dyn ProcessRunner,
    cancellation: CancellationToken,
    tool: &BundledTool,
    _runner_version: &str,
) -> DomainResult {
    if cancellation.is_cancelled() {
        return reject(RefusalCode::Cancelled, "make cancelled before preflight");
    }
    let before = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    // Набор заказывается один раз: превью, применение и обе квитанции видят
    // один и тот же план, даже если проектный файл тем временем сменится.
    let planned = match planned_source_set(prepared) {
        Ok(planned) => planned,
        Err(result) => return result,
    };
    let mut runner_prepared = prepared.clone();
    let staging = if before.external.is_some() && !prepared.dry_run {
        match PrivateStaging::create() {
            Ok(directory) => Some(directory),
            Err(_) => {
                return reject(
                    RefusalCode::InvalidState,
                    "cannot create private artifact staging",
                )
            }
        }
    } else {
        None
    };
    if before.external.is_some() {
        let private_root = staging
            .as_ref()
            .map(|dir| dir.path().to_path_buf())
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("unica-make-preview-{}", uuid::Uuid::new_v4()))
            });
        // Normalize the trusted temp root before dispatch (macOS /var is an alias).
        // Never canonicalize the runner-created artifact directory: links there must fail.
        runner_prepared.arguments.output = match normalize_path_identity(&private_root) {
            Ok(root) => root.join("artifacts"),
            Err(_) => {
                return reject(
                    RefusalCode::InvalidState,
                    "cannot resolve private artifact staging",
                )
            }
        };
    }
    let preview = match invoke_runner(
        &runner_prepared,
        planned.as_deref(),
        tool,
        runner,
        &cancellation,
        true,
    ) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let source_set = match validate_envelope(
        &runner_prepared,
        planned.as_deref(),
        &before,
        &preview,
        false,
    ) {
        Ok(source_set) => source_set,
        Err(result) => return result,
    };
    let after = match capture_inputs(prepared) {
        Ok(inputs) => inputs,
        Err(result) => return result,
    };
    if before != after {
        return reject(
            RefusalCode::ConcurrentChange,
            "make inputs changed during preview; run dryRun: true again",
        );
    }

    if prepared.dry_run {
        let mut result = DomainResult::success(format!(
            "make planned building the {} from source set `{source_set}` without publishing anything",
            prepared.arguments.kind.suffix()
        ));
        result.data = Some(json!({
            "op": OPERATION,
            "dryRun": true,
            "plan": {
                "artifact": {
                    "kind": prepared.arguments.kind.suffix(),
                    "path": path_text(&prepared.arguments.output_relative),
                },
                "sourceSet": source_set,
                "source": before.external.as_ref().map(|plan| &plan.at),
                "configurationSourceSet": before.external.as_ref().map(|plan| &plan.configuration_set),
                "extension": prepared.arguments.extension,
                "outputExists": before.output_sha256.is_some(),
            },
            "providerDispatched": false,
            "requiresPlatform": true,
        }));

        result.next.push(json!({
            "tool": "unica.run",
            "args": {
                "op": OPERATION,
                "args": public_arguments(prepared),
                "dryRun": false,
            },
            "reason": "execute with the current arguments"
        }));
        return result;
    }
    if cancellation.is_cancelled() {
        return reject(
            RefusalCode::Cancelled,
            "make cancelled before provider launch",
        );
    }
    let applied = match invoke_runner(
        &runner_prepared,
        planned.as_deref(),
        tool,
        runner,
        &cancellation,
        false,
    ) {
        Ok(envelope) => envelope,
        Err(result) => return result,
    };
    let built_from = match validate_envelope(
        &runner_prepared,
        planned.as_deref(),
        &before,
        &applied,
        true,
    ) {
        Ok(source_set) => source_set,
        Err(result) => return result,
    };
    if built_from != source_set {
        return reject(
            RefusalCode::InvalidResult,
            "v8-runner built the artifact from a different source set than previewed",
        );
    }
    if before.external.is_some() {
        if let Err(result) = publish_external(
            prepared,
            &runner_prepared.arguments.output,
            &before,
            &cancellation,
        ) {
            return result;
        }
    }
    // Квитанция не со слов провайдера: файл на выходе Unica читает сама.
    let (sha256, size) = match digest_optional_workspace_file(
        &prepared.context.workspace_root,
        &prepared.arguments.output_relative,
    ) {
        Ok(Some(receipt)) if receipt.1 > 0 => receipt,
        Ok(Some(_)) => {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner reported the build but the artifact is empty",
            )
        }
        Ok(None) => {
            return reject(
                RefusalCode::InvalidResult,
                "v8-runner reported the build but the artifact is missing",
            )
        }
        Err(error) => return reject(RefusalCode::InvalidResult, error),
    };
    let state = if before.output_sha256.is_some() {
        "replaced"
    } else {
        "created"
    };
    let artifact = json!({
        "kind": prepared.arguments.kind.suffix(),
        "path": path_text(&prepared.arguments.output_relative),
        "size": size,
        "sha256": sha256,
    });
    let mut result = DomainResult::success(format!(
        "make built the {} from source set `{source_set}` and independently verified it",
        prepared.arguments.kind.suffix()
    ));
    result.data = Some(json!({
        "op": OPERATION,
        "dryRun": false,
        "artifact": artifact,
        "sourceSet": source_set,
        "extension": prepared.arguments.extension,
        "providerDispatched": true,
    }));
    result.changed.push(json!({
        "path": path_text(&prepared.arguments.output_relative),
        "kind": state,
    }));
    result.artifacts.push(artifact);

    result
}

/// Сверка конверта с планом; возвращает набор, который раннер выбрал.
fn validate_envelope(
    prepared: &PreparedArtifactBuild,
    planned: Option<&str>,
    inputs: &StableInputs,
    envelope: &Value,
    applied: bool,
) -> Result<String, DomainResult> {
    let data = &envelope["data"];
    let phase = if applied { "apply result" } else { "preview" };
    if data["provider_dispatched"] != applied
        || data["execution"]["payload"]["published"] != applied
        || data["execution"]["status"] != "succeeded"
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            if applied {
                "v8-runner reported success without publishing the artifact"
            } else {
                "v8-runner preview did not prove that nothing was published"
            },
        ));
    }
    let mode = prepared.arguments.kind.runner_mode();
    if data["mode"] != mode
        || data["execution"]["payload"]["artifact_type"] != mode
        || data["extension"].as_str() != prepared.arguments.extension.as_deref()
    {
        return Err(reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} answered for a different artifact kind or extension"),
        ));
    }
    let source_set = data["source_set"]
        .as_str()
        .filter(|name| inputs.declared.iter().any(|declared| declared == name))
        .ok_or_else(|| {
            reject(
                RefusalCode::InvalidResult,
                format!(
                    "v8-runner {phase} named a source set that v8project.yaml does not declare"
                ),
            )
        })?
        .to_string();
    // The receipt is checked against the set the runner was asked for, also
    // when the request left it to the declared configuration set.
    if let Some(requested) = planned {
        if requested != source_set {
            return Err(reject(
                RefusalCode::InvalidResult,
                format!("v8-runner {phase} answered for a different source set than requested"),
            ));
        }
    }
    let reported = data["output_path"].as_str().ok_or_else(|| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} omitted the output it acts on"),
        )
    })?;
    let reported = normalize_path_identity(Path::new(reported)).map_err(|error| {
        reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} returned an invalid output path: {error}"),
        )
    })?;
    let expected = normalize_path_identity(&prepared.arguments.output).map_err(|error| {
        reject(
            RefusalCode::InvalidResult,
            format!("failed to resolve the planned output: {error}"),
        )
    })?;
    if reported != expected {
        return Err(reject(
            RefusalCode::InvalidResult,
            format!("v8-runner {phase} named a different output than the requested one"),
        ));
    }
    Ok(source_set)
}

/// The set the runner is asked to build from: the requested one, or for a
/// `.cf` without `sourceSet` the declared configuration set. A `.cfe` without
/// it is left to the runner and its `--extension`.
fn planned_source_set(prepared: &PreparedArtifactBuild) -> Result<Option<String>, DomainResult> {
    Ok(match (&prepared.arguments.source_set, prepared.arguments.kind) {
        (Some(source_set), _) => Some(source_set.clone()),
        (None, ArtifactKind::Cf) => Some(
            configuration_source_set(&prepared.context.workspace_root)
                .map_err(|error| reject(RefusalCode::InvalidState, error))?
                .ok_or_else(|| {
                    reject(
                        RefusalCode::InvalidState,
                        format!("{CONFIG_NAME} declares no configuration source-set; name the source set to build the .cf from with sourceSet"),
                    )
                })?,
        ),
        (None, ArtifactKind::Cfe) => None,
        (None, ArtifactKind::Epf | ArtifactKind::Erf) => return Err(reject(RefusalCode::BadValue, "external sourceSet is required")),
    })
}

fn invoke_runner(
    prepared: &PreparedArtifactBuild,
    planned: Option<&str>,
    tool: &BundledTool,
    runner: &dyn ProcessRunner,
    cancellation: &CancellationToken,
    dry_run: bool,
) -> Result<Value, DomainResult> {
    let mut args = vec![
        "--config".to_string(),
        prepared
            .context
            .workspace_root
            .join(CONFIG_NAME)
            .display()
            .to_string(),
        "--json-message".to_string(),
        RUNNER_COMMAND.to_string(),
    ];
    // Набор — позиционный аргумент: `--source-set` у раннера 0.12 скрытый синоним.
    // Без набора и без `--extension` раннер 0.14 собирает все наборы в каталог
    // и путь к одному файлу отклоняет, поэтому `.cf` без набора заказывается по
    // имени набора основной конфигурации.
    args.extend(planned.map(str::to_string));
    args.extend([
        "--output".to_string(),
        prepared.arguments.output.display().to_string(),
    ]);
    if let Some(extension) = &prepared.arguments.extension {
        args.extend(["--extension".to_string(), extension.clone()]);
    }
    if dry_run {
        args.push("--dry-run".to_string());
    }
    let output = runner
        .run_staged(&ProcessCommand {
            program: tool.program.clone(),
            args,
            cwd: prepared.context.workspace_root.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: Some((RUNNER_OUTPUT_LIMIT, RUNNER_OUTPUT_LIMIT)),
            timeout: None,
            cancellation: cancellation.clone(),
        })
        .map_err(|error| {
            runner_start_rejection(Some(OPERATION.to_string()), &error, &tool.program)
        })?;
    parse_runner_output(output)
}

fn parse_runner_output(output: ProcessOutput) -> Result<Value, DomainResult> {
    if output.cancelled {
        return Err(reject(RefusalCode::Cancelled, "v8-runner was cancelled"));
    }
    if output.timed_out {
        return Err(reject(
            RefusalCode::DeadlineExceeded,
            "v8-runner exceeded its execution deadline",
        ));
    }
    if output.stdout_truncated || output.stdout_had_invalid_utf8 {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner returned an unreadable or oversized JSON result",
        ));
    }
    let envelope: Value = serde_json::from_str(&output.stdout).map_err(|_| {
        reject(
            RefusalCode::InvalidResult,
            "v8-runner returned an invalid JSON result",
        )
    })?;
    if envelope["command"] != RUNNER_COMMAND {
        return Err(reject(
            RefusalCode::InvalidResult,
            "v8-runner returned a result for a different operation",
        ));
    }
    if !output.status_success || envelope["ok"] != true {
        // Снимок отказа у `make` пишет `provider_dispatched: true` и до
        // запуска платформы (v8-runner 0.9.0): отказ идёт по коду раннера.
        let code = envelope["error"]["code"]
            .as_str()
            .unwrap_or("provider_failed");
        let message = envelope["error"]["message"]
            .as_str()
            .map(redactor)
            .unwrap_or_else(|| "v8-runner failed without a typed message".to_string());
        return Err(runner_rejection(Some(OPERATION.to_string()), code, message));
    }
    Ok(envelope)
}

fn public_arguments(prepared: &PreparedArtifactBuild) -> Value {
    let mut args = Map::new();
    args.insert(
        "output".to_string(),
        Value::String(path_text(&prepared.arguments.output_relative)),
    );
    if let Some(source_set) = &prepared.arguments.source_set {
        args.insert("sourceSet".to_string(), Value::String(source_set.clone()));
    }
    if let Some(extension) = &prepared.arguments.extension {
        args.insert("extension".to_string(), Value::String(extension.clone()));
    }
    Value::Object(args)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn reject(code: RefusalCode, message: impl Into<String>) -> DomainResult {
    DomainResult::canonical_rejection(Some(OPERATION.to_string()), code, message)
}

/// Поставляемого раннера нет: маршрут один на все операции `run`, уточнение
/// `provider_absent` назначает общий помощник.
fn reject_absent_runner(absence: RunnerAbsence) -> DomainResult {
    missing_runner_rejection(Some(OPERATION.to_string()), absence)
}

#[cfg(test)]
mod tests {
    use super::super::v13_infobase_exports::map_runner_code;
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use unica_refusal::Outcome;

    struct SequenceRunner {
        outputs: Mutex<Vec<ProcessOutput>>,
        calls: Mutex<Vec<ProcessCommand>>,
        publish_on_apply: Option<Vec<u8>>,
    }

    impl SequenceRunner {
        fn new(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                publish_on_apply: None,
            }
        }

        fn publishing(outputs: Vec<ProcessOutput>, bytes: &[u8]) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().rev().collect()),
                calls: Mutex::new(Vec::new()),
                publish_on_apply: Some(bytes.to_vec()),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }

        fn joined_args(&self, index: usize) -> String {
            self.calls.lock().unwrap()[index].args.join(" ")
        }
    }

    impl ProcessRunner for SequenceRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.lock().unwrap().push(command.clone());
            if !command.args.iter().any(|argument| argument == "--dry-run") {
                if let Some(bytes) = &self.publish_on_apply {
                    let index = command
                        .args
                        .iter()
                        .position(|argument| argument == "--output")
                        .expect("output argument");
                    let output = PathBuf::from(&command.args[index + 1]);
                    fs::create_dir_all(output.parent().unwrap()).unwrap();
                    fs::write(output, bytes).unwrap();
                }
            }
            Ok(self.outputs.lock().unwrap().pop().expect("runner output"))
        }
    }

    fn process(envelope: Value, success: bool) -> ProcessOutput {
        ProcessOutput {
            status_success: success,
            status: if success {
                "exit status: 0"
            } else {
                "exit status: 2"
            }
            .to_string(),
            stdout: serde_json::to_string(&envelope).unwrap(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        }
    }

    fn context(root: &Path) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(CONFIG_NAME),
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: ext-sales\n    type: EXTENSION\n    path: ext-sales\n",
        )
        .unwrap();
        root
    }

    fn build_of(
        root: &Path,
        output: &str,
        source_set: Option<&str>,
        extension: Option<&str>,
        dry_run: bool,
    ) -> PreparedArtifactBuild {
        let mut args = Map::new();
        args.insert("output".to_string(), json!(output));
        if let Some(source_set) = source_set {
            args.insert("sourceSet".to_string(), json!(source_set));
        }
        if let Some(extension) = extension {
            args.insert("extension".to_string(), json!(extension));
        }
        PreparedArtifactBuild {
            arguments: parse_build_arguments(&args, &context(root)).expect("valid arguments"),
            dry_run,
            context: context(root),
        }
    }

    fn tool(root: &Path) -> BundledTool {
        BundledTool {
            program: root.join("v8-runner"),
            warnings: Vec::new(),
            missing: None,
        }
    }

    /// Конверт `make` раннера 0.9.0, снятый с живой пробы: вид артефакта,
    /// набор, выход как передан и `published` в полезной нагрузке.
    fn envelope(
        output: &Path,
        kind: ArtifactKind,
        set: &str,
        extension: Option<&str>,
        published: bool,
    ) -> Value {
        let mut data = json!({
            "ok": true,
            "provider_dispatched": published,
            "mode": kind.runner_mode(),
            "source_set": set,
            "output_path": output,
            "message": format!(
                "would build into '{}' via /opt/1cv8/8.3.27.2074/1cv8; nothing published",
                output.display()
            ),
            "execution": {
                "status": "succeeded",
                "diagnostics": ["via /opt/1cv8/8.3.27.2074/1cv8"],
                "payload": {
                    "artifact_type": kind.runner_mode(),
                    "output_path": output,
                    "file_names": [output.file_name().unwrap().to_string_lossy()],
                    "published": published
                }
            }
        });
        if let Some(extension) = extension {
            data["extension"] = json!(extension);
        }
        json!({
            "ok": true,
            "command": "make",
            "duration_ms": 3,
            "data": data,
            "warnings": [],
            "steps": [],
        })
    }

    fn run(root: &Path, prepared: &PreparedArtifactBuild, runner: &SequenceRunner) -> DomainResult {
        execute_with_resolved_runner(
            prepared,
            runner,
            CancellationToken::new(),
            &tool(root),
            "0.9.0",
        )
    }

    fn external_workspace(kind: ArtifactKind) -> tempfile::TempDir {
        let root = workspace();
        let (source_type, tag) = if kind == ArtifactKind::Epf {
            ("EXTERNAL_DATA_PROCESSORS", "ExternalDataProcessor")
        } else {
            ("EXTERNAL_REPORTS", "ExternalReport")
        };
        let config = fs::read_to_string(root.path().join(CONFIG_NAME)).unwrap();
        fs::write(
            root.path().join(CONFIG_NAME),
            format!("{config}  - name: external\n    type: {source_type}\n    path: external\n"),
        )
        .unwrap();
        fs::create_dir(root.path().join("external")).unwrap();
        fs::write(root.path().join("external/Тест.xml"), format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{tag}><Properties><Name>Тест</Name></Properties></{tag}></MetaDataObject>"#)).unwrap();
        root
    }

    struct ExternalRunner {
        kind: ArtifactKind,
        behavior: &'static str,
        calls: Mutex<Vec<ProcessCommand>>,
    }
    impl ProcessRunner for ExternalRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.calls.lock().unwrap().push(command.clone());
            let output = PathBuf::from(
                &command.args[command
                    .args
                    .iter()
                    .position(|arg| arg == "--output")
                    .unwrap()
                    + 1],
            );
            assert!(
                !output.starts_with(&command.cwd),
                "runner must never replace a workspace directory"
            );
            let applied = !command.args.iter().any(|arg| arg == "--dry-run");
            if applied {
                // The private root exists before dispatch and is owner-only
                // from creation (mode 0700, a protected owner DACL on Windows).
                let private_root = unica_workspace::state_fs::open_state_directory_nofollow(
                    output
                        .parent()
                        .expect("artifact directory has a private root"),
                )
                .expect("private artifact staging exists before the runner");
                unica_workspace::state_fs::verify_state_owner_only(&private_root)
                    .expect("private artifact staging is owner-only");
                fs::create_dir_all(&output).unwrap();
                let target = output.join(format!("Тест.{}", self.kind.suffix()));
                match self.behavior {
                    "missing" => {}
                    "empty" => fs::write(&target, b"").unwrap(),
                    "wrong-kind" => fs::write(output.join("Тест.cf"), b"wrong").unwrap(),
                    "directory" => fs::create_dir(&target).unwrap(),
                    "linked" => {
                        let outside = command.cwd.join("keep.bin");
                        fs::write(&outside, b"outside").unwrap();
                        if let Some(result) =
                            unica_workspace::state_fs::test_support::create_file_symlink_for_test(
                                &outside, &target,
                            )
                        {
                            result.unwrap();
                        } else {
                            // A directory is still an invalid regular-file output on hosts without links.
                            fs::create_dir(&target).unwrap();
                        }
                    }
                    _ => fs::write(&target, b"external artifact").unwrap(),
                }
                if self.behavior == "multiple" {
                    fs::write(output.join("other.epf"), b"other").unwrap();
                }
                if self.behavior == "cancel" {
                    command.cancellation.cancel();
                }
                if self.behavior == "race" {
                    fs::write(command.cwd.join("dist/renamed.epf"), b"concurrent").unwrap();
                }
            }
            let mut result = envelope(&output, self.kind, "external", None, applied);
            if applied && self.behavior == "forged" {
                result["data"]["output_path"] = json!(command.cwd.join("dist"));
            }
            Ok(process(result, true))
        }
    }

    #[test]
    fn external_build_publishes_one_verified_file_and_preserves_neighbors() {
        for (kind, replacing) in [
            (ArtifactKind::Epf, false),
            (ArtifactKind::Epf, true),
            (ArtifactKind::Erf, false),
            (ArtifactKind::Erf, true),
        ] {
            let root = external_workspace(kind);
            let output = format!("dist/renamed.{}", kind.suffix());
            fs::create_dir(root.path().join("dist")).unwrap();
            fs::write(root.path().join("dist/neighbor.cf"), b"keep").unwrap();
            if replacing {
                fs::write(root.path().join(&output), b"previous").unwrap();
            }
            let original = fs::read(root.path().join("external/Тест.xml")).unwrap();
            let prepared = build_of(root.path(), &output, Some("external"), None, false);
            let runner = ExternalRunner {
                kind,
                behavior: "ok",
                calls: Mutex::new(vec![]),
            };
            let result = execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                &tool(root.path()),
                "0.15.2",
            );
            assert!(result.ok, "{result:?}");
            assert_eq!(
                fs::read(root.path().join(&output)).unwrap(),
                b"external artifact"
            );
            assert_eq!(
                fs::read(root.path().join("dist/neighbor.cf")).unwrap(),
                b"keep"
            );
            assert_eq!(
                fs::read(root.path().join("external/Тест.xml")).unwrap(),
                original
            );
            assert_eq!(
                result.artifacts[0]["sha256"],
                format!("{:x}", Sha256::digest(b"external artifact"))
            );
            assert_eq!(result.artifacts[0]["size"], 17);
            assert_eq!(
                result.changed[0]["kind"],
                if replacing { "replaced" } else { "created" }
            );
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            let staging = Path::new(
                &calls[1].args[calls[1]
                    .args
                    .iter()
                    .position(|arg| arg == "--output")
                    .unwrap()
                    + 1],
            );
            assert!(!staging.exists(), "private staging must be cleaned up");
        }
    }

    #[test]
    fn external_preview_names_owner_and_configuration_without_creating_output() {
        let root = external_workspace(ArtifactKind::Epf);
        let prepared = build_of(
            root.path(),
            "dist/renamed.epf",
            Some("external"),
            None,
            true,
        );
        let runner = ExternalRunner {
            kind: ArtifactKind::Epf,
            behavior: "ok",
            calls: Mutex::new(vec![]),
        };
        let result = execute_with_resolved_runner(
            &prepared,
            &runner,
            CancellationToken::new(),
            &tool(root.path()),
            "0.15.2",
        );
        assert!(result.ok, "{result:?}");
        let data = result.data.unwrap();
        assert_eq!(
            data["plan"]["source"],
            "external:ExternalDataProcessor.Тест"
        );
        assert_eq!(data["plan"]["configurationSourceSet"], "main");
        assert_eq!(data["plan"]["artifact"]["path"], "dist/renamed.epf");
        assert!(!root.path().join("dist").exists());
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn external_invalid_builds_and_cancellation_preserve_previous_output() {
        for behavior in [
            "missing",
            "empty",
            "wrong-kind",
            "directory",
            "linked",
            "multiple",
            "forged",
            "cancel",
            "race",
        ] {
            let root = external_workspace(ArtifactKind::Epf);
            fs::create_dir(root.path().join("dist")).unwrap();
            fs::write(root.path().join("dist/renamed.epf"), b"previous").unwrap();
            let prepared = build_of(
                root.path(),
                "dist/renamed.epf",
                Some("external"),
                None,
                false,
            );
            let runner = ExternalRunner {
                kind: ArtifactKind::Epf,
                behavior,
                calls: Mutex::new(vec![]),
            };
            let result = execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                &tool(root.path()),
                "0.15.2",
            );
            assert!(!result.ok, "{behavior}: {result:?}");
            let expected_code = match behavior {
                "race" => "concurrent_change",
                "cancel" => "cancelled",
                _ => "invalid_result",
            };
            assert_eq!(
                result.diagnostics[0]["code"], expected_code,
                "{behavior}: {result:?}"
            );
            assert_eq!(
                fs::read(root.path().join("dist/renamed.epf")).unwrap(),
                if behavior == "race" {
                    &b"concurrent"[..]
                } else {
                    &b"previous"[..]
                }
            );
        }
    }

    #[test]
    fn external_publication_io_failure_preserves_output_and_requires_environment_repair() {
        use unica_workspace::single_file_publisher::{with_publish_failpoints, PublishCheckpoint};
        let root = external_workspace(ArtifactKind::Epf);
        fs::create_dir(root.path().join("dist")).unwrap();
        let output = root.path().join("dist/renamed.epf");
        fs::write(&output, b"previous").unwrap();
        fs::write(root.path().join("dist/neighbor.cf"), b"keep").unwrap();
        let source = fs::read(root.path().join("external/Тест.xml")).unwrap();
        let prepared = build_of(
            root.path(),
            "dist/renamed.epf",
            Some("external"),
            None,
            false,
        );
        let runner = ExternalRunner {
            kind: ArtifactKind::Epf,
            behavior: "ok",
            calls: Mutex::new(vec![]),
        };
        let result = with_publish_failpoints(&[PublishCheckpoint::Write], || {
            execute_with_resolved_runner(
                &prepared,
                &runner,
                CancellationToken::new(),
                &tool(root.path()),
                "0.15.2",
            )
        });
        assert!(!result.ok, "{result:?}");
        assert_eq!(result.diagnostics[0]["code"], "invalid_state", "{result:?}");
        assert_eq!(result.diagnostics[0]["outcome"], "needsHuman", "{result:?}");
        assert_eq!(fs::read(&output).unwrap(), b"previous");
        assert_eq!(
            fs::read(root.path().join("dist/neighbor.cf")).unwrap(),
            b"keep"
        );
        assert_eq!(
            fs::read(root.path().join("external/Тест.xml")).unwrap(),
            source
        );
        assert_eq!(fs::read_dir(root.path().join("dist")).unwrap().count(), 2);
    }

    #[test]
    fn external_cancellation_after_publication_lock_never_commits() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Barrier,
        };
        use unica_workspace::single_file_publisher::{
            with_before_commit_hook, with_publication_lock_pause,
        };
        for replacing in [false, true] {
            let root = external_workspace(ArtifactKind::Epf);
            fs::create_dir(root.path().join("dist")).unwrap();
            let output = root.path().join("dist/renamed.epf");
            if replacing {
                fs::write(&output, b"previous").unwrap();
            }
            let prepared = build_of(
                root.path(),
                "dist/renamed.epf",
                Some("external"),
                None,
                false,
            );
            let cancellation = CancellationToken::new();
            let worker_cancellation = cancellation.clone();
            let acquired = Arc::new(Barrier::new(2));
            let release = Arc::new(Barrier::new(2));
            let worker_acquired = acquired.clone();
            let worker_release = release.clone();
            let commits = Arc::new(AtomicUsize::new(0));
            let worker_commits = commits.clone();
            let runner_tool = tool(root.path());
            let worker = std::thread::spawn(move || {
                let runner = ExternalRunner {
                    kind: ArtifactKind::Epf,
                    behavior: "ok",
                    calls: Mutex::new(vec![]),
                };
                with_publication_lock_pause(worker_acquired, worker_release, || {
                    with_before_commit_hook(
                        move |_| {
                            worker_commits.fetch_add(1, Ordering::SeqCst);
                        },
                        || {
                            execute_with_resolved_runner(
                                &prepared,
                                &runner,
                                worker_cancellation,
                                &runner_tool,
                                "0.15.2",
                            )
                        },
                    )
                })
            });
            acquired.wait();
            cancellation.cancel();
            release.wait();
            let result = worker.join().unwrap();
            assert_eq!(result.diagnostics[0]["code"], "cancelled", "{result:?}");
            assert_eq!(
                commits.load(Ordering::SeqCst),
                0,
                "cancellation must prevent publication, not roll it back later"
            );
            if replacing {
                assert_eq!(fs::read(&output).unwrap(), b"previous");
            } else {
                assert!(!output.exists());
            }
            assert_eq!(
                fs::read_dir(root.path().join("dist")).unwrap().count(),
                usize::from(replacing),
                "no staged or recovery files may remain"
            );
        }
    }

    #[test]
    fn external_prerequisites_and_ambiguity_are_refused_before_dispatch() {
        for case in ["missing-config", "multiple", "wrong-kind", "overlap"] {
            let root = external_workspace(ArtifactKind::Epf);
            match case {
                "missing-config" => {
                    let config = fs::read_to_string(root.path().join(CONFIG_NAME)).unwrap();
                    fs::write(
                        root.path().join(CONFIG_NAME),
                        config.replace("type: CONFIGURATION", "type: EXTENSION"),
                    )
                    .unwrap();
                }
                "multiple" => {
                    fs::copy(
                        root.path().join("external/Тест.xml"),
                        root.path().join("external/Other.xml"),
                    )
                    .unwrap();
                }
                _ => {}
            }
            let output = match case {
                "wrong-kind" => "dist/renamed.erf",
                "overlap" => "external/renamed.epf",
                _ => "dist/renamed.epf",
            };
            let prepared = build_of(root.path(), output, Some("external"), None, true);
            let runner = SequenceRunner::new(vec![]);
            let result = run(root.path(), &prepared, &runner);
            assert!(!result.ok, "{case}: {result:?}");
            assert_eq!(runner.call_count(), 0);
        }
    }

    #[test]
    fn external_output_overlap_is_physical_on_case_insensitive_volumes() {
        let root = external_workspace(ArtifactKind::Epf);
        if !root.path().join("EXTERNAL").is_dir() {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::CaseInsensitiveFilesystem,
                "a case variant of the source root names another directory on this volume",
            );
            return;
        }
        let prepared = build_of(
            root.path(),
            "EXTERNAL/renamed.epf",
            Some("external"),
            None,
            true,
        );
        let runner = SequenceRunner::new(vec![]);
        let result = run(root.path(), &prepared, &runner);
        assert!(!result.ok, "{result:?}");
        assert_eq!(result.diagnostics[0]["code"], "bad_value", "{result:?}");
        assert_eq!(runner.call_count(), 0);
        assert!(!root.path().join("external/renamed.epf").exists());
    }

    #[test]
    fn external_artifact_arguments_accept_single_file_outputs() {
        let root = workspace();
        for suffix in ["epf", "erf"] {
            let args = json!({"output": format!("dist/tool.{suffix}"), "sourceSet": "external"});
            let parsed = parse_build_arguments(args.as_object().unwrap(), &context(root.path()));
            assert!(
                parsed.is_ok(),
                "external output must be accepted: {parsed:?}"
            );
        }
    }

    #[test]
    fn arguments_are_closed_and_each_refusal_names_the_fix() {
        let root = workspace();
        for (args, code, expected) in [
            (
                json!({"output": "dist/main.cf", "publish": true}),
                "bad_value",
                "does not accept `publish`",
            ),
            (json!({}), "bad_value", "output must be non-empty text"),
            (
                json!({"output": root.path().join("main.cf").to_string_lossy()}),
                "bad_value",
                "workspace-relative",
            ),
            (
                json!({"output": "dist/main.dt"}),
                "bad_value",
                ".cf for a configuration or .cfe",
            ),
            (
                json!({"output": "dist/report.epf"}),
                "bad_value",
                "requires sourceSet",
            ),
            (
                json!({"output": "dist/sales.cfe"}),
                "bad_value",
                "requires extension",
            ),
            (
                json!({"output": "dist/main.cf", "extension": "Sales"}),
                "bad_value",
                "only for a .cfe output",
            ),
            (
                json!({"output": "dist/sales.cfe", "extension": "1Sales"}),
                "bad_value",
                "1C identifier",
            ),
            (
                json!({"output": "dist/main.cf", "sourceSet": "../x"}),
                "bad_value",
                "must be the name of a source set",
            ),
        ] {
            let result = parse_build_arguments(args.as_object().unwrap(), &context(root.path()))
                .err()
                .unwrap_or_else(|| panic!("{args} must be refused"));
            assert_eq!(result.diagnostics[0]["code"], code, "{args}: {result:?}");
            assert!(
                result.diagnostics[0]["message"]
                    .as_str()
                    .unwrap()
                    .contains(expected),
                "{args}: {result:?}"
            );
        }
        let accepted = parse_build_arguments(
            json!({"output": "dist/sales.cfe", "sourceSet": "ext-sales", "extension": "Sales"})
                .as_object()
                .unwrap(),
            &context(root.path()),
        )
        .unwrap();
        assert_eq!(accepted.kind, ArtifactKind::Cfe);
        assert_eq!(accepted.output_relative, PathBuf::from("dist/sales.cfe"));

        let runner = SequenceRunner::new(Vec::new());
        let result = run(
            root.path(),
            &build_of(
                root.path(),
                "dist/main.cf",
                Some("ext-purchases"),
                None,
                true,
            ),
            &runner,
        );
        assert_eq!(result.diagnostics[0]["code"], "bad_value", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("declared source sets: main, ext-sales"));
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn preview_names_the_artifact_and_the_set_without_publishing() {
        let root = workspace();
        let prepared = build_of(root.path(), "dist/main.cf", None, None, true);
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cf,
                "main",
                None,
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);

        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["providerDispatched"], false);
        assert_eq!(data["plan"]["artifact"]["kind"], "cf");
        assert_eq!(data["plan"]["artifact"]["path"], "dist/main.cf");
        assert_eq!(data["plan"]["sourceSet"], "main");
        assert_eq!(data["plan"]["outputExists"], false);
        assert!(result.rev.is_none());
        assert_eq!(result.next[0]["args"]["dryRun"], false);
        assert!(result.next[0]["args"].get("ifRev").is_none());
        assert_eq!(
            result.next[0]["args"]["args"],
            json!({"output": "dist/main.cf"})
        );
        assert!(result.changed.is_empty());
        assert!(!prepared.arguments.output.exists());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        let root_text = normalize_path_identity(root.path())
            .unwrap()
            .display()
            .to_string();
        assert!(
            !encoded.contains(&root_text),
            "absolute path leaked: {encoded}"
        );
        // Без набора раннер 0.14 собрал бы все наборы в каталог и путь к файлу
        // отклонил бы: `.cf` заказывается по имени набора основной конфигурации.
        let args = runner.joined_args(0);
        assert!(
            args.contains("--json-message make main --output ") && args.ends_with("--dry-run"),
            "{args}"
        );
    }

    /// Проект без набора основной конфигурации: `.cf` без `sourceSet` собирать
    /// не из чего, и отказ приходит до запуска раннера.
    #[test]
    fn a_cf_without_a_configuration_set_is_refused_before_the_runner_starts() {
        let root = workspace();
        fs::write(
            root.path().join(CONFIG_NAME),
            "format: DESIGNER\nsource-set:\n  - name: ext-sales\n    type: EXTENSION\n    path: ext-sales\n",
        )
        .unwrap();
        let prepared = build_of(root.path(), "dist/main.cf", None, None, true);
        let runner = SequenceRunner::new(vec![]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(result.diagnostics[0]["code"], "invalid_state", "{result:?}");
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("declares no configuration source-set"));
        assert_eq!(runner.call_count(), 0);
    }

    #[test]
    fn preview_of_an_extension_passes_the_set_and_the_extension() {
        let root = workspace();
        let prepared = build_of(
            root.path(),
            "dist/sales.cfe",
            Some("ext-sales"),
            Some("Sales"),
            true,
        );
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cfe,
                "ext-sales",
                Some("Sales"),
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert!(result.ok, "{result:?}");
        assert_eq!(result.data.as_ref().unwrap()["plan"]["extension"], "Sales");
        assert!(runner
            .joined_args(0)
            .contains("--json-message make ext-sales --output "));
        assert!(runner
            .joined_args(0)
            .ends_with("--extension Sales --dry-run"));
    }

    #[test]
    fn preview_accepts_cyrillic_extension_source_set_from_public_run_request() {
        let root = workspace();
        let config = root.path().join(CONFIG_NAME);
        let yaml = fs::read_to_string(&config)
            .unwrap()
            .replace("ext-sales", "Доработки");
        fs::write(config, yaml).unwrap();
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            json!({
                "op": "make",
                "args": {"output": "dist/Доработки.cfe", "sourceSet": "Доработки", "extension": "Доработки"},
                "dryRun": true
            }),
            root.path().display().to_string(),
            7000,
        )
        .unwrap();
        let prepared = PreparedArtifactBuild::parse(&request).expect("Cyrillic source set");
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cfe,
                "Доработки",
                Some("Доработки"),
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.data.as_ref().unwrap()["plan"]["sourceSet"],
            "Доработки"
        );
        assert_eq!(
            result.data.as_ref().unwrap()["plan"]["artifact"]["path"],
            "dist/Доработки.cfe"
        );
        assert!(runner
            .joined_args(0)
            .contains("--json-message make Доработки --output "));
        assert!(runner
            .joined_args(0)
            .ends_with("--extension Доработки --dry-run"));
        assert!(result.changed.is_empty());
        assert!(!prepared.arguments.output.exists());
    }

    #[test]
    fn preview_refuses_another_output_kind_or_set_and_a_published_preview() {
        let root = workspace();
        let prepared = build_of(root.path(), "dist/main.cf", None, None, true);

        let mut other = envelope(
            &prepared.arguments.output,
            ArtifactKind::Cf,
            "main",
            None,
            false,
        );
        other["data"]["output_path"] = json!(root.path().join("dist/other.cf"));
        let runner = SequenceRunner::new(vec![process(other, true)]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("different output"));

        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cfe,
                "main",
                None,
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );

        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cf,
                "ext-purchases",
                None,
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );

        // Без sourceSet `.cf` заказан по набору конфигурации `main`; ответ
        // по другому объявленному набору — не та квитанция.
        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cf,
                "ext-sales",
                None,
                false,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("different source set than requested"));

        let runner = SequenceRunner::new(vec![process(
            envelope(
                &prepared.arguments.output,
                ArtifactKind::Cf,
                "main",
                None,
                true,
            ),
            true,
        )]);
        let result = run(root.path(), &prepared, &runner);
        assert_eq!(
            result.diagnostics[0]["code"], "invalid_result",
            "{result:?}"
        );
        assert!(result.diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("nothing was published"));
    }

    #[test]
    fn apply_without_prior_preview_returns_an_independent_file_receipt() {
        let root = workspace();

        let prepared = build_of(root.path(), "dist/main.cf", None, None, false);
        let runner = SequenceRunner::publishing(
            vec![
                process(
                    envelope(
                        &prepared.arguments.output,
                        ArtifactKind::Cf,
                        "main",
                        None,
                        false,
                    ),
                    true,
                ),
                process(
                    envelope(
                        &prepared.arguments.output,
                        ArtifactKind::Cf,
                        "main",
                        None,
                        true,
                    ),
                    true,
                ),
            ],
            b"built cf",
        );

        let result = run(root.path(), &prepared, &runner);

        assert!(result.ok, "{result:?}");
        assert_eq!(runner.call_count(), 2);
        assert!(runner.joined_args(0).ends_with("--dry-run"));
        assert!(!runner.joined_args(1).contains("--dry-run"));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["artifact"]["path"], "dist/main.cf");
        assert_eq!(data["artifact"]["size"], 8);
        assert_eq!(
            data["artifact"]["sha256"],
            "949a1f41c225a214233e51f1afb728b03e6add23ac0df2bfd40c09d56057ccbe"
        );
        assert_eq!(data["sourceSet"], "main");
        assert_eq!(result.changed[0]["path"], "dist/main.cf");
        assert_eq!(result.changed[0]["kind"], "created");
        assert_eq!(result.artifacts[0]["kind"], "cf");
        assert_eq!(result.artifacts[0]["size"], 8);
        assert!(result.rev.is_none());
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("1cv8"), "platform path leaked: {encoded}");
        assert!(!encoded.contains("--config"));
    }

    #[test]
    fn apply_refuses_a_missing_or_empty_artifact() {
        for published in [None, Some(&b""[..])] {
            let root = workspace();

            let prepared = build_of(root.path(), "dist/main.cf", None, None, false);
            let outputs = vec![
                process(
                    envelope(
                        &prepared.arguments.output,
                        ArtifactKind::Cf,
                        "main",
                        None,
                        false,
                    ),
                    true,
                ),
                process(
                    envelope(
                        &prepared.arguments.output,
                        ArtifactKind::Cf,
                        "main",
                        None,
                        true,
                    ),
                    true,
                ),
            ];
            let runner = match published {
                Some(bytes) => SequenceRunner::publishing(outputs, bytes),
                None => SequenceRunner::new(outputs),
            };
            let result = run(root.path(), &prepared, &runner);
            assert_eq!(
                result.diagnostics[0]["code"], "invalid_result",
                "{result:?}"
            );
            assert!(result.diagnostics[0]["message"].as_str().unwrap().contains(
                if published.is_some() {
                    "artifact is empty"
                } else {
                    "artifact is missing"
                }
            ));
        }
    }

    #[test]
    fn runner_refusals_keep_their_outcome() {
        let root = workspace();
        let prepared = build_of(
            root.path(),
            "dist/sales.cfe",
            Some("ext-sales"),
            Some("Sales"),
            true,
        );
        let failure = |code: &str| {
            json!({
                "ok": false,
                "command": "make",
                "duration_ms": 0,
                "data": {"ok": false, "provider_dispatched": true, "mode": "extension_cfe", "source_set": "ext-sales", "output_path": prepared.arguments.output, "message": "refused",
                    "execution": {"status": "failed", "diagnostics": ["refused"], "errors": [{"code": "artifacts_failed", "message": "refused"}],
                        "payload": {"artifact_type": "extension_cfe", "output_path": prepared.arguments.output, "file_names": ["sales.cfe"], "published": false}}},
                "warnings": [],
                "steps": [],
                "error": {"code": code, "kind": "validation", "message": "refused"}
            })
        };
        for (code, expected, outcome) in [
            ("invalid_argument", "bad_value", Outcome::FixCall),
            (
                "platform_failure",
                "provider_unavailable",
                Outcome::NeedsHuman,
            ),
        ] {
            let runner = SequenceRunner::new(vec![process(failure(code), false)]);
            let result = run(root.path(), &prepared, &runner);
            assert_eq!(result.diagnostics[0]["code"], expected, "{code}");
            assert_eq!(map_runner_code(code).outcome(), outcome, "{code}");
        }
    }
}

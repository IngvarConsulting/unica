mod inspection;
pub(crate) use inspection::{
    NativeOperationAdapter, PlatformInspectionSession, PlatformXmlInspector,
};

use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use unica_format_core::{
    commands::{
        BorrowScope, DiagnosticCode, DiagnosticDetail, ExecutionContext,
        ExtensionPatchEmissionPlan, FormElementKind, InspectionPort, InspectionRequest,
        InspectionResult, InterceptorKind, MetadataKindName, MetadataObjectReference, MutationMode,
        SemanticArtifact, SemanticArtifactRef, SemanticChange, SemanticObjectIdentity,
        SupportCapability, SupportObjectRule, TemplateKind, WriterCommand, WriterDiagnostic,
        WriterEvidence, WriterFailureKind, WriterFamily, WriterLifecycle, WriterResult,
        WriterSourceRole,
    },
    ports::{WriterPort, WriterRequest},
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

#[cfg(test)]
pub(crate) use crate::versions::v2_20::writers::testing;
pub(crate) use crate::versions::v2_20::writers::{
    cf, cfe, common, compile_transaction, dcs, external, filesystem, form, form_edit,
    form_event_registry, help, interface, meta, mxl, operation_descriptors, platform_xml_owner,
    project_source_types, project_sources, registry, role, single_file_publisher,
    source_root_types, source_roots, subsystem, support, template,
};
pub(crate) use crate::versions::v2_20::writers::{
    is_1c_identifier, is_1c_identifier_part, is_1c_identifier_start, FormatProfile,
    ACTIVE_FORMAT_PROFILE,
};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NativeWriterResult {
    pub(crate) ok: bool,
    pub(crate) summary: String,
    pub(crate) changes: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) errors: Vec<String>,
    pub(crate) artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stderr: Option<String>,
}

impl NativeWriterResult {
    pub(crate) fn ok(summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            summary: summary.into(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceContext {
    pub(crate) cwd: PathBuf,
    pub(crate) workspace_root: PathBuf,
    pub(crate) cache_root: PathBuf,
    pub(crate) workspace_epoch: u64,
}

#[derive(Clone)]
pub(crate) struct PlatformWriterSession {
    sources: BTreeMap<WriterSourceRole, PathBuf>,
    context: WorkspaceContext,
    extension_emitter: Option<Arc<ExtensionEmitter>>,
}

pub(crate) type ExtensionEmitter =
    dyn Fn(&ExtensionPatchEmissionPlan, Option<&[u8]>) -> Result<Vec<u8>, String> + Send + Sync;

impl PlatformWriterSession {
    pub(crate) fn new<I>(sources: I, context: WorkspaceContext) -> Result<Self, String>
    where
        I: IntoIterator<Item = (WriterSourceRole, PathBuf)>,
    {
        let mut captured = BTreeMap::new();
        for (role, source) in sources {
            if captured.insert(role, source).is_some() {
                return Err("writer source role was bound more than once".to_string());
            }
        }
        Ok(Self {
            sources: captured,
            context,
            extension_emitter: None,
        })
    }

    pub(crate) fn with_extension_emitter(mut self, emitter: Arc<ExtensionEmitter>) -> Self {
        self.extension_emitter = Some(emitter);
        self
    }

    pub(crate) fn source_bindings(&self) -> &BTreeMap<WriterSourceRole, PathBuf> {
        &self.sources
    }

    pub(crate) fn source(&self, role: WriterSourceRole) -> Option<&std::path::Path> {
        self.sources.get(&role).map(PathBuf::as_path)
    }

    pub(crate) fn required_source(
        &self,
        role: WriterSourceRole,
        purpose: &'static str,
    ) -> Result<&std::path::Path, String> {
        self.source(role)
            .ok_or_else(|| format!("semantic source is required for {purpose}"))
    }

    pub(crate) fn context(&self) -> &WorkspaceContext {
        &self.context
    }

    pub(crate) fn extension_emitter(&self) -> Option<&ExtensionEmitter> {
        self.extension_emitter.as_deref()
    }
}

pub(crate) struct PlatformXmlWriter;

impl WriterPort for PlatformXmlWriter {
    fn families(&self) -> &'static [WriterFamily] {
        &WriterFamily::ALL
    }

    fn execute(&self, request: &WriterRequest) -> Result<WriterResult, SourceAdapterError> {
        let session = request
            .session()
            .adapter_state::<PlatformWriterSession>()
            .ok_or_else(|| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::CapabilityBlocked,
                    "writer command has no bound Platform XML execution session",
                )
            })?;
        if request.cancellation().is_cancelled() {
            return Ok(WriterResult::cancelled());
        }
        crate::versions::v2_20::writers::cancellation::with_cancellation(
            request.cancellation(),
            || {
                let scoped = crate::versions::v2_20::writers::single_file_publisher::
                    with_writer_publication_scope(|| {
                        let (outcome, evidence) = registry::execute(
                            request.command(),
                            request.mode(),
                            session,
                        )
                        .unwrap_or_else(|message| {
                            (
                                NativeWriterResult {
                                    ok: false,
                                    summary: "writer command was rejected".to_string(),
                                    changes: Vec::new(),
                                    warnings: Vec::new(),
                                    errors: vec![message],
                                    artifacts: Vec::new(),
                                    stdout: None,
                                    stderr: None,
                                },
                                None,
                            )
                        });
                        let result = core_writer_result(
                            request.command(),
                            request.mode(),
                            &outcome,
                            evidence,
                        );
                        Ok(observed_cancellation_result().unwrap_or(result))
                    });
                match scoped {
                    Ok(result) => result,
                    Err(_) => observed_cancellation_result().map(Ok).unwrap_or_else(|| {
                        Err(SourceAdapterError::new(
                            SourceAdapterErrorKind::CapabilityBlocked,
                            "writer publication scope is unavailable",
                        ))
                    }),
                }
            },
        )
    }
}

fn observed_cancellation_result() -> Option<WriterResult> {
    match crate::versions::v2_20::writers::cancellation::outcome()? {
        crate::versions::v2_20::writers::cancellation::CancellationOutcome::DuringExecution => {
            Some(WriterResult::cancelled_during_execution())
        }
        crate::versions::v2_20::writers::cancellation::CancellationOutcome::
            DuringPublicationRolledBack => Some(WriterResult::cancelled_during_publication()),
        crate::versions::v2_20::writers::cancellation::CancellationOutcome::RecoveryRequired => {
            Some(WriterResult::publication_recovery_required())
        }
    }
}

fn core_writer_result(
    command: &WriterCommand,
    mode: MutationMode,
    outcome: &NativeWriterResult,
    evidence: Option<unica_format_core::commands::FormEditEvidence>,
) -> WriterResult {
    if !outcome.ok {
        let diagnostic = classify_writer_diagnostic(command, outcome);
        let result = if diagnostic == DiagnosticCode::Cancelled {
            let message = outcome.errors.join("\n").to_ascii_lowercase();
            if message.contains("rollback") {
                WriterResult::cancelled_during_publication()
            } else {
                WriterResult::cancelled_during_execution()
            }
        } else {
            let detail = planner_diagnostic_detail(command, outcome, diagnostic);
            WriterResult::rejected_with_diagnostic(
                WriterDiagnostic::new(diagnostic, detail),
                classify_writer_failure(diagnostic),
            )
        };
        return result.with_evidence(evidence.map(WriterEvidence::FormEdit));
    }
    let changes = semantic_changes(command, outcome);
    if mode.is_preview() {
        let artifacts = external_artifact_refs(command);
        return WriterResult::new(WriterLifecycle::Previewed, changes, artifacts, [])
            .expect("native preview maps to a valid semantic result")
            .with_evidence(evidence.map(WriterEvidence::FormEdit));
    }
    let changed = changes != [SemanticChange::NoChange];
    let artifact = semantic_artifact(command);
    let artifact_count = if changed {
        outcome.artifacts.len().max(1)
    } else {
        0
    };
    let artifacts = {
        let external = external_artifact_refs(command);
        if changed && !external.is_empty() {
            external
        } else {
            std::iter::repeat_n(SemanticArtifactRef::unidentified(artifact), artifact_count)
                .collect()
        }
    };
    WriterResult::new(WriterLifecycle::Applied, changes, artifacts, [])
        .expect("native writer success maps to a valid semantic result")
        .with_evidence(evidence.map(WriterEvidence::FormEdit))
}

fn external_artifact_refs(command: &WriterCommand) -> Vec<SemanticArtifactRef> {
    use unica_format_core::commands::ExternalArtifactKind;

    let (command, kind, artifact) = match command {
        WriterCommand::ExternalProcessorInitialize(command) => (
            command,
            ExternalArtifactKind::Processor,
            SemanticArtifact::ExternalProcessor,
        ),
        WriterCommand::ExternalReportInitialize(command) => (
            command,
            ExternalArtifactKind::Report,
            SemanticArtifact::ExternalReport,
        ),
        _ => return Vec::new(),
    };
    let mut artifacts = vec![SemanticArtifactRef::new(
        artifact,
        SemanticObjectIdentity::ExternalObject {
            kind,
            name: command.name().clone(),
        },
    )];
    artifacts.push(match command.primary_form() {
        Some(form) => SemanticArtifactRef::new(
            SemanticArtifact::Form,
            SemanticObjectIdentity::ExternalPrimaryForm {
                kind,
                owner: command.name().clone(),
                form: form.name().clone(),
            },
        ),
        None => SemanticArtifactRef::new(
            SemanticArtifact::Module,
            SemanticObjectIdentity::ExternalObjectModule {
                kind,
                owner: command.name().clone(),
            },
        ),
    });
    artifacts
}

fn semantic_changes(command: &WriterCommand, outcome: &NativeWriterResult) -> Vec<SemanticChange> {
    if outcome.changes.is_empty() {
        return vec![SemanticChange::NoChange];
    }
    outcome
        .changes
        .iter()
        .map(|native_change| {
            let normalized = native_change.to_ascii_lowercase();
            if normalized.contains("configuration.xml")
                && matches!(
                    command,
                    WriterCommand::MetadataCreate(_)
                        | WriterCommand::MetadataRemove(_)
                        | WriterCommand::FormCreate(_)
                        | WriterCommand::FormRemove(_)
                        | WriterCommand::TemplateCreate(_)
                        | WriterCommand::TemplateRemove(_)
                        | WriterCommand::RoleCreate(_)
                        | WriterCommand::SubsystemCreate(_)
                )
            {
                return SemanticChange::RegistrationUpdated;
            }
            match command {
                WriterCommand::ConfigurationInitialize(_)
                | WriterCommand::ExtensionInitialize(_)
                | WriterCommand::ExternalProcessorInitialize(_)
                | WriterCommand::ExternalReportInitialize(_)
                | WriterCommand::MetadataCreate(_)
                | WriterCommand::FormCreate(_)
                | WriterCommand::FormCompile(_)
                | WriterCommand::TemplateCreate(_)
                | WriterCommand::HelpCreate(_)
                | WriterCommand::RoleCreate(_)
                | WriterCommand::SubsystemCreate(_)
                | WriterCommand::DataCompositionCreate(_)
                | WriterCommand::SpreadsheetCreate(_) => SemanticChange::SourceCreated,
                WriterCommand::MetadataRemove(_)
                | WriterCommand::FormRemove(_)
                | WriterCommand::TemplateRemove(_) => SemanticChange::SourceRemoved,
                WriterCommand::SupportEdit(_) => SemanticChange::SupportUpdated,
                WriterCommand::ExtensionPatchMethod(_) => SemanticChange::ModuleUpdated,
                _ => SemanticChange::SourceUpdated,
            }
        })
        .collect()
}

fn semantic_artifact(command: &WriterCommand) -> SemanticArtifact {
    match command {
        WriterCommand::ConfigurationInitialize(_) | WriterCommand::ConfigurationEdit(_) => {
            SemanticArtifact::Configuration
        }
        WriterCommand::ExtensionInitialize(_)
        | WriterCommand::ExtensionBorrow(_)
        | WriterCommand::ExtensionPatchMethod(_) => SemanticArtifact::Extension,
        WriterCommand::ExternalProcessorInitialize(_) => SemanticArtifact::ExternalProcessor,
        WriterCommand::ExternalReportInitialize(_) => SemanticArtifact::ExternalReport,
        WriterCommand::MetadataCreate(_)
        | WriterCommand::MetadataEdit(_)
        | WriterCommand::MetadataRemove(_) => SemanticArtifact::MetadataObject,
        WriterCommand::FormCreate(_)
        | WriterCommand::FormCompile(_)
        | WriterCommand::FormEdit(_)
        | WriterCommand::FormRemove(_) => SemanticArtifact::Form,
        WriterCommand::TemplateCreate(_) | WriterCommand::TemplateRemove(_) => {
            SemanticArtifact::Template
        }
        WriterCommand::HelpCreate(_) => SemanticArtifact::Help,
        WriterCommand::InterfaceEdit(_) => SemanticArtifact::Interface,
        WriterCommand::RoleCreate(_) => SemanticArtifact::Role,
        WriterCommand::SubsystemCreate(_) | WriterCommand::SubsystemEdit(_) => {
            SemanticArtifact::Subsystem
        }
        WriterCommand::SupportEdit(_) => SemanticArtifact::SupportState,
        WriterCommand::DataCompositionCreate(_) | WriterCommand::DataCompositionEdit(_) => {
            SemanticArtifact::DataComposition
        }
        WriterCommand::SpreadsheetCreate(_) => SemanticArtifact::Spreadsheet,
    }
}

fn classify_writer_diagnostic(
    command: &WriterCommand,
    outcome: &NativeWriterResult,
) -> DiagnosticCode {
    let text = outcome.errors.join("\n").to_ascii_lowercase();
    if text.contains("cancel") || text.contains("отмен") {
        DiagnosticCode::Cancelled
    } else if matches!(command, WriterCommand::MetadataCreate(_))
        && unsupported_metadata_kind(outcome).is_some()
    {
        DiagnosticCode::PlannerRejected
    } else if text.contains("differs from the expected preimage")
        || text.contains("changed after planning")
        || text.contains("changed while planning")
    {
        DiagnosticCode::Conflict
    } else if text.contains("read-only") {
        DiagnosticCode::ReadOnlyArtifact
    } else if text.contains("hard link") {
        DiagnosticCode::AliasedArtifact
    } else if text.contains("invalid property format") {
        DiagnosticCode::InvalidMutation
    } else if text.contains("invalid format") && text.contains("type.name") {
        DiagnosticCode::InvalidObjectReference
    } else if text.contains("unknown type") {
        DiagnosticCode::UnknownObjectKind
    } else if text.contains("missing companion") {
        DiagnosticCode::MissingFormCompanion
    } else if text.contains("valid 1c identifier") || text.contains("single path component") {
        DiagnosticCode::InvalidModuleReference
    } else if text.contains("not a borrowed extension object") {
        DiagnosticCode::ObjectNotBorrowed
    } else if text.contains("capability=on") || text.contains("пообъектное переключение недоступно")
    {
        DiagnosticCode::SupportCapabilityDisabled
    } else if text.contains("newer than supported")
        || text.contains("unsupported")
        || text.contains("не поддерж")
    {
        DiagnosticCode::UnsupportedState
    } else if text.contains("already exists") || text.contains("уже существ") {
        DiagnosticCode::AlreadyExists
    } else if text.contains("not found") || text.contains("не найден") {
        DiagnosticCode::NotFound
    } else if text.contains("planner") || text.contains("plan ") {
        DiagnosticCode::PlannerRejected
    } else if text.contains("owner") || text.contains("владел") {
        DiagnosticCode::OwnerResolutionFailed
    } else if text.contains("validate") || text.contains("root element") {
        DiagnosticCode::ValidationFailed
    } else {
        DiagnosticCode::InvalidRequest
    }
}

fn classify_writer_failure(diagnostic: DiagnosticCode) -> WriterFailureKind {
    match diagnostic {
        DiagnosticCode::UnsupportedState | DiagnosticCode::UnsupportedFormat => {
            WriterFailureKind::UnsupportedState
        }
        DiagnosticCode::AlreadyExists | DiagnosticCode::Conflict => WriterFailureKind::Conflict,
        DiagnosticCode::ValidationFailed | DiagnosticCode::InvalidDefinition => {
            WriterFailureKind::Validation
        }
        DiagnosticCode::MissingFormCompanion => WriterFailureKind::Validation,
        DiagnosticCode::PlannerRejected | DiagnosticCode::OwnerResolutionFailed => {
            WriterFailureKind::Planning
        }
        DiagnosticCode::InvalidModuleReference | DiagnosticCode::ObjectNotBorrowed => {
            WriterFailureKind::Planning
        }
        _ => WriterFailureKind::InvalidRequest,
    }
}

fn planner_diagnostic_detail(
    command: &WriterCommand,
    outcome: &NativeWriterResult,
    diagnostic: DiagnosticCode,
) -> Option<DiagnosticDetail> {
    if diagnostic == DiagnosticCode::PlannerRejected
        && matches!(command, WriterCommand::MetadataCreate(_))
    {
        return unsupported_metadata_kind(outcome).map(DiagnosticDetail::MetadataKind);
    }
    let text = outcome.errors.join("\n");
    if diagnostic == DiagnosticCode::UnknownObjectKind {
        return quoted_value_after(&text, "Unknown type '")
            .and_then(|value| MetadataKindName::new(value).ok())
            .map(DiagnosticDetail::MetadataKind);
    }
    if diagnostic == DiagnosticCode::InvalidObjectReference {
        return quoted_value_after(&text, "Invalid format '")
            .and_then(|value| MetadataObjectReference::new(value).ok())
            .map(DiagnosticDetail::Object);
    }
    if diagnostic == DiagnosticCode::MissingFormCompanion {
        return text
            .split("] '")
            .nth(1)
            .and_then(|value| value.split("':").next())
            .and_then(|value| unica_format_core::commands::FormElementName::new(value).ok())
            .map(DiagnosticDetail::FormElement);
    }
    None
}

fn quoted_value_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.split_once(prefix)?
        .1
        .split_once('\'')
        .map(|(value, _)| value)
}

fn unsupported_metadata_kind(outcome: &NativeWriterResult) -> Option<MetadataKindName> {
    outcome.errors.iter().find_map(|message| {
        let value = message
            .split_once("Unsupported type: ")
            .map(|(_, value)| value)?
            .split(['.', ','])
            .next()?
            .trim();
        MetadataKindName::new(value).ok()
    })
}

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::PathBuf,
};

use unica_format_core::{
    commands::{
        DiagnosticCode, MutationMode, SemanticArtifact, SemanticChange, WriterFailureKind,
        WriterLifecycle, WriterResult,
    },
    ports::{
        ArtifactContent, ArtifactReadRequest, ArtifactReadResult, ArtifactWriteIntent,
        ArtifactWritePort, ArtifactWriteRequest, OperationCancellation,
    },
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

use super::{
    cancellation::{self, CancellationOutcome},
    compile_transaction::{
        CompileTransaction, DirectoryMembershipSelector as NativeMembershipSelector,
    },
    module_locator::PlatformModuleArtifactLease,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArtifactMembershipSelector {
    StructuredDescriptors,
    ConfigurationArtifacts,
    DirectEntries,
}

#[derive(Debug, Clone)]
pub(crate) struct StagedArtifactReplacement {
    pub(crate) path: PathBuf,
    pub(crate) expected_preimage: Vec<u8>,
    pub(crate) replacement: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PlatformArtifactWriteSession {
    pub(crate) replacement: Option<StagedArtifactReplacement>,
    pub(crate) exact_guards: BTreeMap<PathBuf, Vec<u8>>,
    pub(crate) absence_guards: BTreeSet<PathBuf>,
    pub(crate) membership_guards: BTreeMap<(PathBuf, ArtifactMembershipSelector), Vec<OsString>>,
}

pub(crate) struct PlatformArtifactWriter;

impl ArtifactWritePort for PlatformArtifactWriter {
    fn read(
        &self,
        request: &ArtifactReadRequest,
    ) -> Result<ArtifactReadResult, SourceAdapterError> {
        if request.cancellation().is_cancelled() {
            return Err(cancelled_error());
        }
        let lease = request
            .lease()
            .adapter_state::<PlatformModuleArtifactLease>()
            .ok_or_else(|| capability_error("artifact lease does not belong to this adapter"))?;
        verify_module_lease(lease)?;
        let bytes = fs::read(&lease.target)
            .map_err(|_| capability_error("module artifact is no longer readable"))?;
        if request.cancellation().is_cancelled() {
            return Err(cancelled_error());
        }
        ArtifactContent::new(bytes)
            .map(ArtifactReadResult::new)
            .map_err(|_| capability_error("module artifact exceeds the neutral content limit"))
    }

    fn write(&self, request: &ArtifactWriteRequest) -> Result<WriterResult, SourceAdapterError> {
        if request.cancellation().is_cancelled() {
            return Ok(WriterResult::cancelled());
        }
        if request.mode() == MutationMode::Preview {
            return Ok(WriterResult::previewed(false));
        }
        match request.intent() {
            ArtifactWriteIntent::ModulePatch | ArtifactWriteIntent::ExtensionMethodPatch(_) => {
                write_module(request)
            }
            ArtifactWriteIntent::SemanticSourceTransaction => write_transaction(request),
        }
    }
}

fn write_module(request: &ArtifactWriteRequest) -> Result<WriterResult, SourceAdapterError> {
    let lease = request
        .lease()
        .and_then(|lease| lease.adapter_state::<PlatformModuleArtifactLease>())
        .ok_or_else(|| capability_error("module write requires an adapter-owned artifact lease"))?;
    let replacement = request
        .replacement()
        .ok_or_else(|| capability_error("module write requires replacement content"))?;
    verify_module_lease(lease)?;

    let mut transaction = CompileTransaction::new();
    transaction
        .replace_bytes(
            &lease.target,
            &lease.expected_preimage,
            replacement.as_bytes(),
        )
        .map_err(publication_error)?;
    for (path, preimage) in &lease.descriptor_preimages {
        transaction
            .guard_or_verify_exact_preimage(path, preimage)
            .map_err(publication_error)?;
    }
    for (path, preimage) in &lease.source_declaration_preimages {
        transaction
            .guard_or_verify_exact_preimage(path, preimage)
            .map_err(publication_error)?;
    }
    commit_transaction(
        transaction,
        request.cancellation(),
        SemanticArtifact::Module,
    )
}

fn write_transaction(request: &ArtifactWriteRequest) -> Result<WriterResult, SourceAdapterError> {
    let session = request
        .session()
        .adapter_state::<PlatformArtifactWriteSession>()
        .ok_or_else(|| capability_error("artifact transaction has no adapter-owned session"))?;
    let mut transaction = CompileTransaction::new();
    if let Some(replacement) = &session.replacement {
        transaction
            .replace_bytes(
                &replacement.path,
                &replacement.expected_preimage,
                replacement.replacement.clone(),
            )
            .map_err(publication_error)?;
    }
    for (path, preimage) in &session.exact_guards {
        transaction
            .guard_or_verify_exact_preimage(path, preimage)
            .map_err(publication_error)?;
    }
    for path in &session.absence_guards {
        transaction
            .guard_path_absent(path)
            .map_err(publication_error)?;
    }
    for ((directory, selector), expected) in &session.membership_guards {
        transaction
            .guard_or_verify_directory_membership(
                directory,
                match selector {
                    ArtifactMembershipSelector::StructuredDescriptors => {
                        NativeMembershipSelector::XmlFiles
                    }
                    ArtifactMembershipSelector::ConfigurationArtifacts => {
                        NativeMembershipSelector::CfFilesAsciiCaseInsensitive
                    }
                    ArtifactMembershipSelector::DirectEntries => {
                        NativeMembershipSelector::AllDirectEntries
                    }
                },
                expected.clone(),
            )
            .map_err(publication_error)?;
    }
    commit_transaction(
        transaction,
        request.cancellation(),
        SemanticArtifact::Module,
    )
}

fn commit_transaction(
    transaction: CompileTransaction,
    cancellation: &OperationCancellation,
    artifact: SemanticArtifact,
) -> Result<WriterResult, SourceAdapterError> {
    if cancellation.is_cancelled() {
        return Ok(WriterResult::cancelled());
    }
    cancellation::with_cancellation(cancellation, || match transaction.commit() {
        Ok(report) => {
            let changed = !(report.created.is_empty() && report.updated.is_empty());
            WriterResult::new(
                WriterLifecycle::Applied,
                [if changed {
                    SemanticChange::ModuleUpdated
                } else {
                    SemanticChange::NoChange
                }],
                changed.then_some(artifact),
                [],
            )
            .map_err(|_| capability_error("adapter produced an invalid publication lifecycle"))
        }
        Err(_) => Ok(match cancellation::outcome() {
            Some(CancellationOutcome::DuringExecution) => {
                WriterResult::cancelled_during_execution()
            }
            Some(CancellationOutcome::DuringPublicationRolledBack) => {
                WriterResult::cancelled_during_publication()
            }
            Some(CancellationOutcome::RecoveryRequired) => {
                WriterResult::publication_recovery_required()
            }
            None => WriterResult::rejected(
                DiagnosticCode::PublicationFailed,
                WriterFailureKind::Publication,
            ),
        }),
    })
}

fn verify_module_lease(lease: &PlatformModuleArtifactLease) -> Result<(), SourceAdapterError> {
    let actual = fs::read(&lease.target)
        .map_err(|_| capability_error("module artifact is no longer readable"))?;
    if actual != lease.expected_preimage {
        return Err(capability_error(
            "module artifact changed after its lease was issued",
        ));
    }
    for (path, expected) in &lease.descriptor_preimages {
        let actual = fs::read(path)
            .map_err(|_| capability_error("module owner evidence is no longer readable"))?;
        if &actual != expected {
            return Err(capability_error(
                "module owner evidence changed after its lease was issued",
            ));
        }
    }
    for (path, expected) in &lease.source_declaration_preimages {
        let actual = fs::read(path)
            .map_err(|_| capability_error("source declaration evidence is no longer readable"))?;
        if &actual != expected {
            return Err(capability_error(
                "source declaration evidence changed after its lease was issued",
            ));
        }
    }
    Ok(())
}

fn capability_error(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::CapabilityBlocked, message)
}

fn publication_error(_message: String) -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::CapabilityBlocked,
        "atomic artifact publication could not be prepared",
    )
}

fn cancelled_error() -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::CapabilityBlocked,
        "cancelled: artifact access was cancelled",
    )
}

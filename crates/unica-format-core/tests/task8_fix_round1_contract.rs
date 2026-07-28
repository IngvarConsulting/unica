use std::collections::BTreeSet;

use unica_format_core::{
    commands::{
        ConfigurationInitialize, ConfigurationName, DiagnosticCode, SemanticArtifact,
        SemanticChange, WriterCommand, WriterCommandKind, WriterLifecycle, WriterMessageCode,
        WriterResult,
    },
    ports::OperationCancellation,
};

#[test]
fn every_writer_operation_is_a_closed_typed_variant() {
    let actual = WriterCommandKind::ALL.into_iter().collect::<BTreeSet<_>>();
    let expected = [
        WriterCommandKind::ConfigurationInitialize,
        WriterCommandKind::ConfigurationEdit,
        WriterCommandKind::ExtensionInitialize,
        WriterCommandKind::ExtensionBorrow,
        WriterCommandKind::ExtensionPatchMethod,
        WriterCommandKind::ExternalProcessorInitialize,
        WriterCommandKind::ExternalReportInitialize,
        WriterCommandKind::MetadataCreate,
        WriterCommandKind::MetadataEdit,
        WriterCommandKind::MetadataRemove,
        WriterCommandKind::FormCreate,
        WriterCommandKind::FormCompile,
        WriterCommandKind::FormEdit,
        WriterCommandKind::FormRemove,
        WriterCommandKind::TemplateCreate,
        WriterCommandKind::TemplateRemove,
        WriterCommandKind::HelpCreate,
        WriterCommandKind::InterfaceEdit,
        WriterCommandKind::RoleCreate,
        WriterCommandKind::SubsystemCreate,
        WriterCommandKind::SubsystemEdit,
        WriterCommandKind::SupportEdit,
        WriterCommandKind::DataCompositionCreate,
        WriterCommandKind::DataCompositionEdit,
        WriterCommandKind::SpreadsheetCreate,
    ]
    .into_iter()
    .collect();

    assert_eq!(actual, expected);

    let name = ConfigurationName::new("Sales").unwrap();
    let command = WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(name));
    assert_eq!(command.kind(), WriterCommandKind::ConfigurationInitialize);
    assert!(ConfigurationName::new(" \n").is_err());
}

#[test]
fn writer_outcomes_are_invariant_safe_and_contain_only_closed_semantics() {
    let result = WriterResult::new(
        WriterLifecycle::Applied,
        [SemanticChange::SourceCreated],
        [SemanticArtifact::Configuration],
        [],
    )
    .unwrap();

    assert_eq!(result.lifecycle(), WriterLifecycle::Applied);
    assert_eq!(result.message_code(), WriterMessageCode::Applied);
    assert_eq!(result.changes(), &[SemanticChange::SourceCreated]);
    assert_eq!(result.artifacts(), &[SemanticArtifact::Configuration]);

    assert!(WriterResult::new(
        WriterLifecycle::cancelled_before_execution(),
        [SemanticChange::SourceCreated],
        [SemanticArtifact::Configuration],
        [DiagnosticCode::Cancelled],
    )
    .is_err());

    let preview = WriterResult::previewed_with_changes([
        SemanticChange::SourceCreated,
        SemanticChange::RegistrationUpdated,
    ])
    .unwrap();
    assert_eq!(preview.message_code(), WriterMessageCode::PreviewPlanned);
    assert_eq!(
        preview.changes(),
        &[
            SemanticChange::SourceCreated,
            SemanticChange::RegistrationUpdated
        ]
    );
}

#[test]
fn cancellation_clones_share_the_actual_request_state() {
    let public_request = OperationCancellation::new();
    let adapter_request = public_request.clone();
    public_request.cancel();
    assert!(adapter_request.is_cancelled());
}

#[test]
fn core_writer_contract_has_no_transport_native_or_free_form_envelopes() {
    let source = include_str!("../src/commands/mod.rs");
    for forbidden in [
        "WriterArgument",
        "WriterArguments",
        "serde_json",
        "AdapterOutcome",
        "PathBuf",
        "std::path",
        "operation: String",
        "tool_name",
        "stdout",
        "stderr",
        "summary: String",
        "changes: Vec<String>",
        "artifacts: Vec<String>",
        "MetaDataObject",
        "Configuration.xml",
        "2.20",
        "unica.",
    ] {
        assert!(
            !source.contains(forbidden),
            "core writer contract retains forbidden escape hatch `{forbidden}`"
        );
    }
}

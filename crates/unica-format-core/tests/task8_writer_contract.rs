use std::collections::BTreeSet;

use unica_format_core::commands::{
    ConfigurationInitialize, ConfigurationName, ModuleArtifactLocation, ModuleOwner, ModuleRole,
    MutationMode, WriterCommand, WriterCommandKind, WriterFamily, WriterSourceRole,
};
use unica_format_core::semantic_ids::SemanticObjectKind;

#[test]
fn task8_writer_language_is_closed_and_covers_every_existing_family() {
    let actual = WriterFamily::ALL.into_iter().collect::<BTreeSet<_>>();
    let expected = [
        WriterFamily::Configuration,
        WriterFamily::Extension,
        WriterFamily::ExternalArtifact,
        WriterFamily::Metadata,
        WriterFamily::Form,
        WriterFamily::Template,
        WriterFamily::Help,
        WriterFamily::Interface,
        WriterFamily::Role,
        WriterFamily::Subsystem,
        WriterFamily::Support,
        WriterFamily::DataComposition,
        WriterFamily::Spreadsheet,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
    assert!(MutationMode::Preview.is_preview());
    assert!(!MutationMode::Apply.is_preview());
}

#[test]
fn task8_writer_variants_cover_each_existing_mutation_intent() {
    let actual = WriterCommandKind::ALL.into_iter().collect::<BTreeSet<_>>();

    assert_eq!(actual.len(), 25);
    assert_eq!(actual.len(), WriterCommandKind::ALL.len());
}

#[test]
fn task8_writer_command_owns_closed_immutable_semantic_arguments() {
    let name = ConfigurationName::new("Sales").unwrap();
    let command = WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(name));

    assert_eq!(command.kind(), WriterCommandKind::ConfigurationInitialize);
    assert_eq!(command.family(), WriterFamily::Configuration);
    assert!(ConfigurationName::new(" \n").is_err());

    let roles = [
        WriterSourceRole::Configuration,
        WriterSourceRole::Extension,
        WriterSourceRole::Definition,
        WriterSourceRole::DestinationArtifact,
    ];
    assert_eq!(roles.len(), 4);
}

#[test]
fn task8_module_locator_result_is_closed_and_semantic() {
    let location = ModuleArtifactLocation::new(
        ModuleOwner::object(SemanticObjectKind::Catalog, "Items").unwrap(),
        ModuleRole::Object,
    );

    assert_eq!(location.owner().kind(), SemanticObjectKind::Catalog);
    assert_eq!(location.owner().name(), Some("Items"));
    assert_eq!(location.role().public_label(), "ObjectModule");
}

#[test]
fn task8_command_sources_do_not_expose_native_or_transport_vocabulary() {
    let sources = [
        include_str!("../src/commands/mod.rs"),
        include_str!("../src/commands/inspection.rs"),
        include_str!("../src/commands/module_locator.rs"),
    ]
    .join("\n");

    for forbidden in [
        "PathBuf",
        "std::path",
        "serde_json",
        "AdapterOutcome",
        "NativeOperationResult",
        "MetaDataObject",
        "Configuration.xml",
        "2.20",
        "8.3.27",
    ] {
        assert!(
            !sources.contains(forbidden),
            "core command DTOs expose forbidden vocabulary `{forbidden}`"
        );
    }
}

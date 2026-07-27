use std::collections::BTreeSet;

use unica_format_core::commands::{
    ConfigurationCommand, DataCompositionCommand, ExtensionCommand, ExternalArtifactCommand,
    FormCommand, HelpCommand, InterfaceCommand, MetadataCommand, ModuleArtifactLocation,
    ModuleOwner, ModuleRole, MutationMode, RoleCommand, SpreadsheetCommand, SubsystemCommand,
    SupportCommand, TemplateCommand, WriterCommand, WriterFamily,
};
use unica_format_core::semantic_ids::SemanticObjectKind;

#[test]
fn task8_writer_language_is_closed_and_covers_every_existing_family() {
    let commands = [
        WriterCommand::configuration(ConfigurationCommand::Initialize),
        WriterCommand::extension(ExtensionCommand::Initialize),
        WriterCommand::external_artifact(ExternalArtifactCommand::InitializeProcessor),
        WriterCommand::metadata(MetadataCommand::Create),
        WriterCommand::form(FormCommand::Create),
        WriterCommand::template(TemplateCommand::Create),
        WriterCommand::help(HelpCommand::Create),
        WriterCommand::interface(InterfaceCommand::Edit),
        WriterCommand::role(RoleCommand::Create),
        WriterCommand::subsystem(SubsystemCommand::Create),
        WriterCommand::support(SupportCommand::Edit),
        WriterCommand::data_composition(DataCompositionCommand::Create),
        WriterCommand::spreadsheet(SpreadsheetCommand::Create),
    ];
    let actual = commands
        .iter()
        .map(|command| command.family())
        .collect::<BTreeSet<_>>();
    let expected = WriterFamily::ALL.into_iter().collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
    assert_eq!(MutationMode::Preview.is_preview(), true);
    assert_eq!(MutationMode::Apply.is_preview(), false);
}

#[test]
fn task8_writer_variants_cover_each_existing_mutation_intent() {
    let commands = [
        WriterCommand::configuration(ConfigurationCommand::Initialize),
        WriterCommand::configuration(ConfigurationCommand::Edit),
        WriterCommand::extension(ExtensionCommand::Initialize),
        WriterCommand::extension(ExtensionCommand::Borrow),
        WriterCommand::extension(ExtensionCommand::PatchMethod),
        WriterCommand::external_artifact(ExternalArtifactCommand::InitializeProcessor),
        WriterCommand::external_artifact(ExternalArtifactCommand::InitializeReport),
        WriterCommand::metadata(MetadataCommand::Create),
        WriterCommand::metadata(MetadataCommand::Edit),
        WriterCommand::metadata(MetadataCommand::Remove),
        WriterCommand::form(FormCommand::Create),
        WriterCommand::form(FormCommand::Compile),
        WriterCommand::form(FormCommand::Edit),
        WriterCommand::form(FormCommand::Remove),
        WriterCommand::template(TemplateCommand::Create),
        WriterCommand::template(TemplateCommand::Remove),
        WriterCommand::help(HelpCommand::Create),
        WriterCommand::interface(InterfaceCommand::Edit),
        WriterCommand::role(RoleCommand::Create),
        WriterCommand::subsystem(SubsystemCommand::Create),
        WriterCommand::subsystem(SubsystemCommand::Edit),
        WriterCommand::support(SupportCommand::Edit),
        WriterCommand::data_composition(DataCompositionCommand::Create),
        WriterCommand::data_composition(DataCompositionCommand::Edit),
        WriterCommand::spreadsheet(SpreadsheetCommand::Create),
    ];

    assert_eq!(commands.len(), 25);
    assert!(commands.iter().all(|command| !command.intent().is_empty()));
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

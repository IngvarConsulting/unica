use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value};
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    commands::*,
    ports::{OperationCancellation, WriterRequest},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scenario {
    Success,
    DryRun,
    Idempotent,
    Denied,
    Cancelled,
    Concurrent,
}

impl Scenario {
    const ALL: [Self; 6] = [
        Self::Success,
        Self::DryRun,
        Self::Idempotent,
        Self::Denied,
        Self::Cancelled,
        Self::Concurrent,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticFacts {
    effect: bool,
    invariant: bool,
    reader_ok: Option<bool>,
}

#[derive(Clone)]
struct MatrixCase {
    kind: WriterCommandKind,
    command: WriterCommand,
}

#[test]
fn every_writer_variant_preserves_independent_semantics_in_every_required_scenario() {
    let mut covered = BTreeSet::new();
    for case in matrix_cases() {
        for scenario in Scenario::ALL {
            let root = fixture_root(case.kind, scenario);
            fs::create_dir_all(&root).unwrap();
            prepare_fixture(case.kind, &root);
            exercise(&case, scenario, &root);
            covered.insert((case.kind, scenario));
            fs::remove_dir_all(root).unwrap();
        }
    }

    let expected = WriterCommandKind::ALL
        .into_iter()
        .flat_map(|kind| {
            Scenario::ALL
                .into_iter()
                .map(move |scenario| (kind, scenario))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(covered, expected);
    assert_eq!(covered.len(), 25 * 6);
}

fn exercise(case: &MatrixCase, scenario: Scenario, root: &Path) {
    let before = observe(case.kind, root);
    assert_eq!(
        before,
        expected_before(case.kind),
        "{:?} {scenario:?}: unexpected initial semantic facts",
        case.kind
    );

    match scenario {
        Scenario::Success => {
            let result = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert!(
                matches!(result.lifecycle(), WriterLifecycle::Applied),
                "{:?}: {result:?}",
                case.kind
            );
            assert_eq!(
                observe(case.kind, root),
                expected_after(case.kind),
                "{:?} {scenario:?}: unexpected post-write semantic facts",
                case.kind
            );
        }
        Scenario::DryRun => {
            let result = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Preview,
                OperationCancellation::new(),
            );
            assert!(
                matches!(result.lifecycle(), WriterLifecycle::Previewed),
                "{:?}: {result:?}",
                case.kind
            );
            assert_eq!(observe(case.kind, root), before);
        }
        Scenario::Idempotent => {
            let first = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert!(
                matches!(first.lifecycle(), WriterLifecycle::Applied),
                "{:?}: {first:?}",
                case.kind
            );
            let once = observe(case.kind, root);
            assert_eq!(once, expected_after(case.kind));
            let _repeat = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert_eq!(
                observe(case.kind, root),
                once,
                "{:?}: repeat changed semantic facts",
                case.kind
            );
        }
        Scenario::Denied => {
            let blocked = root.join("blocked");
            fs::write(&blocked, b"not a directory").unwrap();
            let result = execute(
                case.command.clone(),
                sources(case.kind, &blocked),
                root,
                MutationMode::Apply,
                OperationCancellation::new(),
            );
            assert!(
                matches!(result.lifecycle(), WriterLifecycle::Rejected(_)),
                "{:?}: {result:?}",
                case.kind
            );
            assert_eq!(observe(case.kind, root), before);
            assert_eq!(fs::read(&blocked).unwrap(), b"not a directory");
        }
        Scenario::Cancelled => {
            let cancellation = OperationCancellation::new();
            cancellation.cancel();
            let result = execute(
                case.command.clone(),
                sources(case.kind, root),
                root,
                MutationMode::Apply,
                cancellation,
            );
            assert!(
                matches!(result.lifecycle(), WriterLifecycle::Cancelled(_)),
                "{:?}: {result:?}",
                case.kind
            );
            assert_eq!(observe(case.kind, root), before);
        }
        Scenario::Concurrent => {
            let root = Arc::new(root.to_path_buf());
            let mut workers = Vec::new();
            for _ in 0..2 {
                let command = case.command.clone();
                let root = Arc::clone(&root);
                let kind = case.kind;
                workers.push(thread::spawn(move || {
                    execute(
                        command,
                        sources(kind, &root),
                        &root,
                        MutationMode::Apply,
                        OperationCancellation::new(),
                    )
                }));
            }
            let results = workers
                .into_iter()
                .map(|worker| worker.join().expect("concurrent writer must not panic"))
                .collect::<Vec<_>>();
            assert!(
                results
                    .iter()
                    .any(|result| matches!(result.lifecycle(), WriterLifecycle::Applied)),
                "{:?}: {results:?}",
                case.kind
            );
            assert_eq!(observe(case.kind, &root), expected_after(case.kind));
        }
    }
}

fn execute(
    command: WriterCommand,
    sources: Vec<(WriterSourceRole, PathBuf)>,
    workspace_root: &Path,
    mode: MutationMode,
    cancellation: OperationCancellation,
) -> WriterResult {
    let session = PlatformXmlAdapterFactory::new()
        .capture_writer_session_with_extension_emitter(
            sources,
            workspace_root,
            workspace_root,
            &workspace_root.join(".cache"),
            0,
            |_plan, existing| {
                let mut body = existing.unwrap_or_default().to_vec();
                if !body.ends_with(b"\n") && !body.is_empty() {
                    body.push(b'\n');
                }
                body.extend_from_slice(b"// MATRIX_PATCH\n");
                Ok(body)
            },
        )
        .unwrap();
    let request = WriterRequest::new(session, command, mode, cancellation);
    PlatformXmlAdapterFactory::new()
        .operational_registration()
        .writer()
        .execute(&request)
        .unwrap()
}

fn observe(kind: WriterCommandKind, root: &Path) -> SemanticFacts {
    let reader_ok = if let Some((operation, tool, command, args)) = inspection(kind, root) {
        let session = PlatformXmlAdapterFactory::new().capture_inspection_session(
            operation,
            tool,
            &args,
            root,
            root,
            &root.join(".reader-cache"),
            0,
        );
        let request = InspectionRequest::new(session, command, OperationCancellation::new());
        match PlatformXmlAdapterFactory::new()
            .inspection_port()
            .inspect(&request)
        {
            Ok(production_projection) => Some(production_projection.ok()),
            Err(_) => Some(false),
        }
    } else {
        None
    };

    SemanticFacts {
        effect: independent_effect(kind, root),
        invariant: independent_invariant(kind, root),
        reader_ok,
    }
}

fn inspection(
    kind: WriterCommandKind,
    root: &Path,
) -> Option<(
    &'static str,
    &'static str,
    InspectionCommand,
    Map<String, Value>,
)> {
    let src = root.join("src");
    let mut args = Map::new();
    let (operation, tool, command, key, target) = match kind {
        WriterCommandKind::ConfigurationInitialize | WriterCommandKind::ConfigurationEdit => (
            "cf-info",
            "unica.cf.info",
            InspectionCommand::Configuration(ConfigurationInspection::Describe),
            "ConfigPath",
            src.join("Configuration.xml"),
        ),
        WriterCommandKind::ExtensionInitialize
        | WriterCommandKind::ExtensionBorrow
        | WriterCommandKind::ExtensionPatchMethod => (
            "cfe-validate",
            "unica.cfe.validate",
            InspectionCommand::Extension(ExtensionInspection::Validate),
            "ExtensionPath",
            root.join("extension/Configuration.xml"),
        ),
        WriterCommandKind::MetadataCreate
        | WriterCommandKind::MetadataEdit
        | WriterCommandKind::MetadataRemove
        | WriterCommandKind::HelpCreate => (
            "meta-info",
            "unica.meta.info",
            InspectionCommand::Metadata(MetadataInspection::Describe),
            "ObjectPath",
            src.join("Catalogs/Items.xml"),
        ),
        WriterCommandKind::FormCreate
        | WriterCommandKind::FormCompile
        | WriterCommandKind::FormEdit
        | WriterCommandKind::FormRemove => (
            "form-info",
            "unica.form.info",
            InspectionCommand::Form(FormInspection::Describe),
            "FormPath",
            form_path(kind, root),
        ),
        WriterCommandKind::TemplateCreate | WriterCommandKind::TemplateRemove => (
            "template-info",
            "unica.template.info",
            InspectionCommand::Template(TemplateInspection::Describe),
            "TemplatePath",
            src.join("Catalogs/Items/Templates/Main.xml"),
        ),
        WriterCommandKind::InterfaceEdit => (
            "interface-validate",
            "unica.interface.validate",
            InspectionCommand::Interface(InterfaceInspection::Validate),
            "CIPath",
            src.join("Subsystems/Sales/Ext/CommandInterface.xml"),
        ),
        WriterCommandKind::RoleCreate => (
            "role-info",
            "unica.role.info",
            InspectionCommand::Role(RoleInspection::Describe),
            "RightsPath",
            src.join("Roles/Reader/Ext/Rights.xml"),
        ),
        WriterCommandKind::SubsystemCreate | WriterCommandKind::SubsystemEdit => (
            "subsystem-info",
            "unica.subsystem.info",
            InspectionCommand::Subsystem(SubsystemInspection::Describe),
            "SubsystemPath",
            src.join("Subsystems/Sales.xml"),
        ),
        WriterCommandKind::SupportEdit => (
            "cf-info",
            "unica.cf.info",
            InspectionCommand::Configuration(ConfigurationInspection::Describe),
            "ConfigPath",
            src.join("Configuration.xml"),
        ),
        WriterCommandKind::ExternalProcessorInitialize
        | WriterCommandKind::ExternalReportInitialize
        | WriterCommandKind::DataCompositionCreate
        | WriterCommandKind::DataCompositionEdit
        | WriterCommandKind::SpreadsheetCreate => return None,
    };
    args.insert(key.to_string(), Value::String(target.display().to_string()));
    Some((operation, tool, command, args))
}

fn expected_before(kind: WriterCommandKind) -> SemanticFacts {
    SemanticFacts {
        effect: matches!(
            kind,
            WriterCommandKind::MetadataRemove
                | WriterCommandKind::FormRemove
                | WriterCommandKind::TemplateRemove
        ),
        invariant: has_configuration_fixture(kind),
        reader_ok: expected_reader_state(kind, false),
    }
}

fn expected_after(kind: WriterCommandKind) -> SemanticFacts {
    SemanticFacts {
        effect: !matches!(
            kind,
            WriterCommandKind::MetadataRemove
                | WriterCommandKind::FormRemove
                | WriterCommandKind::TemplateRemove
        ),
        invariant: has_configuration_fixture(kind),
        reader_ok: expected_reader_state(kind, true),
    }
}

fn expected_reader_state(kind: WriterCommandKind, after: bool) -> Option<bool> {
    Some(match kind {
        WriterCommandKind::ExternalProcessorInitialize
        | WriterCommandKind::ExternalReportInitialize
        | WriterCommandKind::DataCompositionCreate
        | WriterCommandKind::DataCompositionEdit
        | WriterCommandKind::SpreadsheetCreate => return None,
        WriterCommandKind::ConfigurationInitialize
        | WriterCommandKind::ExtensionInitialize
        | WriterCommandKind::MetadataCreate
        | WriterCommandKind::FormCreate
        | WriterCommandKind::FormCompile
        | WriterCommandKind::TemplateCreate
        | WriterCommandKind::RoleCreate
        | WriterCommandKind::SubsystemCreate => after,
        WriterCommandKind::MetadataRemove
        | WriterCommandKind::FormRemove
        | WriterCommandKind::TemplateRemove => !after,
        WriterCommandKind::ConfigurationEdit
        | WriterCommandKind::ExtensionBorrow
        | WriterCommandKind::ExtensionPatchMethod
        | WriterCommandKind::MetadataEdit
        | WriterCommandKind::FormEdit
        | WriterCommandKind::HelpCreate
        | WriterCommandKind::InterfaceEdit
        | WriterCommandKind::SubsystemEdit
        | WriterCommandKind::SupportEdit => true,
    })
}

fn independent_invariant(kind: WriterCommandKind, root: &Path) -> bool {
    if !has_configuration_fixture(kind) {
        return false;
    }
    xml_has_leaf(&root.join("src/Configuration.xml"), "Name", "MatrixBase")
}

fn independent_effect(kind: WriterCommandKind, root: &Path) -> bool {
    let src = root.join("src");
    match kind {
        WriterCommandKind::ConfigurationInitialize => xml_has_leaf(
            &src.join("Configuration.xml"),
            "Name",
            "MatrixConfiguration",
        ),
        WriterCommandKind::ConfigurationEdit => xml_has_leaf(
            &src.join("Configuration.xml"),
            "Comment",
            "matrix configuration comment",
        ),
        WriterCommandKind::ExtensionInitialize => xml_has_leaf(
            &root.join("extension/Configuration.xml"),
            "Name",
            "MatrixExtension",
        ),
        WriterCommandKind::ExtensionBorrow => {
            tree_contains(&root.join("extension"), "Catalog.Items")
                || tree_contains(&root.join("extension"), ">Items<")
        }
        WriterCommandKind::ExtensionPatchMethod => {
            tree_contains(&root.join("extension"), "MATRIX_PATCH")
        }
        WriterCommandKind::ExternalProcessorInitialize => {
            tree_xml_has_leaf(&root.join("external"), "Name", "MatrixProcessor")
        }
        WriterCommandKind::ExternalReportInitialize => {
            tree_xml_has_leaf(&root.join("external"), "Name", "MatrixReport")
        }
        WriterCommandKind::MetadataCreate | WriterCommandKind::MetadataRemove => {
            xml_has_leaf(&src.join("Catalogs/Items.xml"), "Name", "Items")
        }
        WriterCommandKind::MetadataEdit => xml_has_leaf(
            &src.join("Catalogs/Items.xml"),
            "Comment",
            "matrix metadata comment",
        ),
        WriterCommandKind::FormCreate | WriterCommandKind::FormRemove => xml_has_leaf(
            &src.join("Catalogs/Items/Forms/ObjectForm.xml"),
            "Name",
            "ObjectForm",
        ),
        WriterCommandKind::FormCompile => xml_root_is(&form_path(kind, root), "Form"),
        WriterCommandKind::FormEdit => tree_contains(&form_path(kind, root), "MatrixField"),
        WriterCommandKind::TemplateCreate | WriterCommandKind::TemplateRemove => xml_has_leaf(
            &src.join("Catalogs/Items/Templates/Main.xml"),
            "Name",
            "Main",
        ),
        WriterCommandKind::HelpCreate => tree_contains(&src.join("Catalogs/Items"), "Help"),
        WriterCommandKind::InterfaceEdit => tree_contains(
            &src.join("Subsystems/Sales/Ext/CommandInterface.xml"),
            "Catalog.Items.Command.Open",
        ),
        WriterCommandKind::RoleCreate => {
            xml_has_leaf(&src.join("Roles/Reader.xml"), "Name", "Reader")
        }
        WriterCommandKind::SubsystemCreate => {
            xml_has_leaf(&src.join("Subsystems/Sales.xml"), "Name", "Sales")
        }
        WriterCommandKind::SubsystemEdit => {
            tree_contains(&src.join("Subsystems/Sales.xml"), "SalesReports")
        }
        WriterCommandKind::SupportEdit => {
            let Some(object_uuid) =
                xml_attribute(&src.join("Catalogs/Items.xml"), "Catalog", "uuid")
            else {
                return false;
            };
            let Ok(bytes) = fs::read(src.join("Ext/ParentConfigurations.bin")) else {
                return false;
            };
            String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes))
                .ends_with(&format!("{object_uuid},1,0,{object_uuid},{object_uuid}}}"))
        }
        WriterCommandKind::DataCompositionCreate => {
            xml_root_is(&root.join("standalone/dcs.xml"), "DataCompositionSchema")
        }
        WriterCommandKind::DataCompositionEdit => {
            tree_contains(&root.join("standalone/dcs.xml"), "MatrixParameter")
        }
        WriterCommandKind::SpreadsheetCreate => {
            root.join("standalone/mxl.xml").is_file()
                && fs::metadata(root.join("standalone/mxl.xml"))
                    .map(|metadata| metadata.len() > 0)
                    .unwrap_or(false)
        }
    }
}

fn prepare_fixture(kind: WriterCommandKind, root: &Path) {
    if matches!(
        kind,
        WriterCommandKind::FormCompile
            | WriterCommandKind::FormEdit
            | WriterCommandKind::DataCompositionCreate
            | WriterCommandKind::DataCompositionEdit
            | WriterCommandKind::SpreadsheetCreate
    ) {
        fs::create_dir_all(root.join("standalone")).unwrap();
    }

    if has_configuration_fixture(kind) {
        prerequisite(
            WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(text(
                "MatrixBase",
                ConfigurationName::new,
            ))),
            vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
            root,
        );
    }

    if needs_metadata_fixture(kind) {
        prerequisite(
            WriterCommand::MetadataCreate(MetadataCreate::new(metadata_definition())),
            vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
            root,
        );
    }

    match kind {
        WriterCommandKind::ExtensionBorrow | WriterCommandKind::ExtensionPatchMethod => {
            prerequisite(
                WriterCommand::ExtensionInitialize(ExtensionInitialize::new(text(
                    "MatrixExtension",
                    ExtensionName::new,
                ))),
                vec![
                    (
                        WriterSourceRole::DestinationDirectory,
                        root.join("extension"),
                    ),
                    (
                        WriterSourceRole::Configuration,
                        root.join("src/Configuration.xml"),
                    ),
                ],
                root,
            );
            if kind == WriterCommandKind::ExtensionPatchMethod {
                prerequisite(
                    WriterCommand::ExtensionBorrow(ExtensionBorrow::new(text(
                        "Catalog.Items",
                        MetadataObjectReference::new,
                    ))),
                    vec![
                        (WriterSourceRole::Extension, root.join("extension")),
                        (WriterSourceRole::Configuration, root.join("src")),
                    ],
                    root,
                );
                write(
                    &root.join("extension/Catalogs/Items/Ext/ObjectModule.bsl"),
                    "Procedure BeforeWrite()\nEndProcedure\n",
                );
            }
        }
        WriterCommandKind::MetadataEdit | WriterCommandKind::MetadataRemove => {}
        WriterCommandKind::FormEdit => {
            prerequisite(
                WriterCommand::FormCompile(FormCompile::new(ManagedFormDefinition::empty(), false)),
                vec![(
                    WriterSourceRole::DestinationArtifact,
                    root.join("standalone/Form.xml"),
                )],
                root,
            );
        }
        WriterCommandKind::FormRemove => {
            prerequisite(
                WriterCommand::FormCreate(FormCreate::new(
                    text("Catalog.Items", FormOwnerReference::new),
                    text("ObjectForm", FormName::new),
                )),
                vec![(
                    WriterSourceRole::Object,
                    root.join("src/Catalogs/Items.xml"),
                )],
                root,
            );
        }
        WriterCommandKind::TemplateRemove => {
            prerequisite(
                WriterCommand::TemplateCreate(TemplateCreate::new(
                    text("Catalog.Items", TemplateOwnerReference::new),
                    text("Main", TemplateName::new),
                    TemplateKind::Text,
                )),
                vec![(WriterSourceRole::SourceCollection, root.join("src"))],
                root,
            );
        }
        WriterCommandKind::InterfaceEdit => {
            prerequisite(
                WriterCommand::SubsystemCreate(SubsystemCreate::from_definition(
                    SubsystemDefinition::new(text("Sales", SubsystemName::new)),
                )),
                vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
                root,
            );
            let path = root.join("src/Subsystems/Sales/Ext/CommandInterface.xml");
            if !path.exists() {
                write(
                    &path,
                    concat!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                        "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" ",
                        "xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" ",
                        "version=\"2.20\"><CommandsVisibility/><CommandsPlacement/>",
                        "<CommandsOrder/><SubsystemsOrder/><GroupsOrder/>",
                        "</CommandInterface>\n",
                    ),
                );
            }
        }
        WriterCommandKind::SubsystemEdit => {
            prerequisite(
                WriterCommand::SubsystemCreate(SubsystemCreate::from_definition(
                    SubsystemDefinition::new(text("Sales", SubsystemName::new)),
                )),
                vec![(WriterSourceRole::DestinationDirectory, root.join("src"))],
                root,
            );
        }
        WriterCommandKind::SupportEdit => {
            let object_uuid =
                xml_attribute(&root.join("src/Catalogs/Items.xml"), "Catalog", "uuid")
                    .expect("support fixture catalog must have a UUID");
            let payload = format!(
                concat!(
                    "{{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                    "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                    "\"VendorConf\",3,1,0,{0},{0},0,0,{0},{0}}}"
                ),
                object_uuid
            );
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(payload.as_bytes());
            let path = root.join("src/Ext/ParentConfigurations.bin");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        WriterCommandKind::DataCompositionEdit => {
            prerequisite(
                WriterCommand::DataCompositionCreate(DataCompositionCreate::new(
                    data_composition_definition(),
                )),
                vec![(
                    WriterSourceRole::DestinationArtifact,
                    root.join("standalone/dcs.xml"),
                )],
                root,
            );
        }
        _ => {}
    }
}

fn prerequisite(command: WriterCommand, sources: Vec<(WriterSourceRole, PathBuf)>, root: &Path) {
    let kind = command.kind();
    let result = execute(
        command,
        sources,
        root,
        MutationMode::Apply,
        OperationCancellation::new(),
    );
    assert!(
        matches!(result.lifecycle(), WriterLifecycle::Applied),
        "{kind:?} fixture preparation failed: {result:?}"
    );
}

fn sources(kind: WriterCommandKind, root: &Path) -> Vec<(WriterSourceRole, PathBuf)> {
    let src = root.join("src");
    match kind {
        WriterCommandKind::ConfigurationInitialize => {
            vec![(WriterSourceRole::DestinationDirectory, src)]
        }
        WriterCommandKind::ConfigurationEdit => vec![(
            WriterSourceRole::Configuration,
            src.join("Configuration.xml"),
        )],
        WriterCommandKind::ExtensionInitialize => vec![
            (
                WriterSourceRole::DestinationDirectory,
                root.join("extension"),
            ),
            (
                WriterSourceRole::Configuration,
                src.join("Configuration.xml"),
            ),
        ],
        WriterCommandKind::ExtensionBorrow => vec![
            (WriterSourceRole::Extension, root.join("extension")),
            (WriterSourceRole::Configuration, src),
        ],
        WriterCommandKind::ExtensionPatchMethod => {
            vec![(WriterSourceRole::Extension, root.join("extension"))]
        }
        WriterCommandKind::ExternalProcessorInitialize
        | WriterCommandKind::ExternalReportInitialize => vec![(
            WriterSourceRole::DestinationDirectory,
            root.join("external"),
        )],
        WriterCommandKind::MetadataCreate
        | WriterCommandKind::RoleCreate
        | WriterCommandKind::SubsystemCreate => vec![(WriterSourceRole::DestinationDirectory, src)],
        WriterCommandKind::MetadataEdit => {
            vec![(WriterSourceRole::Object, src.join("Catalogs/Items.xml"))]
        }
        WriterCommandKind::MetadataRemove => vec![(WriterSourceRole::ConfigurationDirectory, src)],
        WriterCommandKind::FormCreate => {
            vec![(WriterSourceRole::Object, src.join("Catalogs/Items.xml"))]
        }
        WriterCommandKind::FormCompile => vec![(
            WriterSourceRole::DestinationArtifact,
            root.join("standalone/Form.xml"),
        )],
        WriterCommandKind::FormEdit => {
            vec![(WriterSourceRole::Form, root.join("standalone/Form.xml"))]
        }
        WriterCommandKind::FormRemove => vec![(WriterSourceRole::SourceCollection, src)],
        WriterCommandKind::TemplateCreate
        | WriterCommandKind::TemplateRemove
        | WriterCommandKind::HelpCreate => vec![(WriterSourceRole::SourceCollection, src)],
        WriterCommandKind::InterfaceEdit => vec![(
            WriterSourceRole::Interface,
            src.join("Subsystems/Sales/Ext/CommandInterface.xml"),
        )],
        WriterCommandKind::SubsystemEdit => vec![(
            WriterSourceRole::Subsystem,
            src.join("Subsystems/Sales.xml"),
        )],
        WriterCommandKind::SupportEdit => vec![(
            WriterSourceRole::SupportTarget,
            src.join("Catalogs/Items.xml"),
        )],
        WriterCommandKind::DataCompositionCreate => vec![(
            WriterSourceRole::DestinationArtifact,
            root.join("standalone/dcs.xml"),
        )],
        WriterCommandKind::DataCompositionEdit => {
            vec![(WriterSourceRole::Template, root.join("standalone/dcs.xml"))]
        }
        WriterCommandKind::SpreadsheetCreate => vec![(
            WriterSourceRole::DestinationArtifact,
            root.join("standalone/mxl.xml"),
        )],
    }
}

fn matrix_cases() -> Vec<MatrixCase> {
    let object = || text("Catalog.Items", MetadataObjectReference::new);
    let form_owner = || text("Catalog.Items", FormOwnerReference::new);
    let template_owner = || text("Catalog.Items", TemplateOwnerReference::new);
    let mut cases = vec![
        case(WriterCommand::ConfigurationInitialize(
            ConfigurationInitialize::new(text("MatrixConfiguration", ConfigurationName::new)),
        )),
        case(WriterCommand::ConfigurationEdit(ConfigurationEdit::mutate(
            ConfigurationMutation::SetProperty(
                ConfigurationPropertyPatch::new(
                    ConfigurationProperty::Comment,
                    ConfigurationPropertyValue::Text(text(
                        "matrix configuration comment",
                        ConfigurationTextValue::new,
                    )),
                )
                .unwrap(),
            ),
        ))),
        case(WriterCommand::ExtensionInitialize(
            ExtensionInitialize::new(text("MatrixExtension", ExtensionName::new)),
        )),
        case(WriterCommand::ExtensionBorrow(ExtensionBorrow::new(
            object(),
        ))),
        case(WriterCommand::ExtensionPatchMethod(
            ExtensionPatchMethod::new(
                ExtensionModuleTarget::Object {
                    owner: text("Catalog.Items", MetadataObjectReference::new),
                    role: ExtensionObjectModuleRole::Object,
                },
                text("BeforeWrite", MethodName::new),
                InterceptorKind::Before,
                ExecutionContext::Automatic,
                false,
            ),
        )),
        case(WriterCommand::ExternalProcessorInitialize(
            ExternalArtifactInitialize::new(text("MatrixProcessor", ExternalArtifactName::new)),
        )),
        case(WriterCommand::ExternalReportInitialize(
            ExternalArtifactInitialize::new(text("MatrixReport", ExternalArtifactName::new)),
        )),
        case(WriterCommand::MetadataCreate(MetadataCreate::new(
            metadata_definition(),
        ))),
        case(WriterCommand::MetadataEdit(MetadataEdit::new(
            object(),
            MetadataPatch::SetProperties(MetadataPropertyChanges::one(
                MetadataPropertyPatch::new(
                    MetadataObjectProperty::Comment,
                    MetadataPropertyValue::Comment(text(
                        "matrix metadata comment",
                        CommentText::new,
                    )),
                )
                .unwrap(),
            )),
        ))),
        case(WriterCommand::MetadataRemove(MetadataRemove::new(
            object(),
            false,
        ))),
        case(WriterCommand::FormCreate(FormCreate::new(
            form_owner(),
            text("ObjectForm", FormName::new),
        ))),
        case(WriterCommand::FormCompile(FormCompile::new(
            ManagedFormDefinition::empty(),
            false,
        ))),
        case(WriterCommand::FormEdit(
            FormEdit::new(
                vec![FormPatch::AddElement(FormElementDefinition::new(
                    text("MatrixField", FormElementName::new),
                    FormElementType::Input,
                ))],
                false,
            )
            .unwrap(),
        )),
        case(WriterCommand::FormRemove(FormRemove::new(
            form_owner(),
            text("ObjectForm", FormName::new),
        ))),
        case(WriterCommand::TemplateCreate(TemplateCreate::new(
            template_owner(),
            text("Main", TemplateName::new),
            TemplateKind::Text,
        ))),
        case(WriterCommand::TemplateRemove(TemplateRemove::new(
            template_owner(),
            text("Main", TemplateName::new),
        ))),
        case(WriterCommand::HelpCreate(HelpCreate::new(
            text("Catalog.Items", HelpOwnerReference::new),
            Some(text("en", LanguageCode::new)),
        ))),
        case(WriterCommand::InterfaceEdit(InterfaceEdit::Place(
            InterfacePlacement::new(
                InterfaceItemReference::new(
                    InterfaceItemKind::Command,
                    text("Catalog.Items.Command.Open", InterfaceItemName::new),
                ),
                text("Main", InterfaceGroupName::new),
                1,
            ),
        ))),
        case(WriterCommand::RoleCreate(RoleCreate::from_definition(
            RoleDefinition::new(text("Reader", RoleName::new)),
        ))),
        case(WriterCommand::SubsystemCreate(
            SubsystemCreate::from_definition(SubsystemDefinition::new(text(
                "Sales",
                SubsystemName::new,
            ))),
        )),
        case(WriterCommand::SubsystemEdit(SubsystemEdit::AddChild(text(
            "SalesReports",
            SubsystemName::new,
        )))),
        case(WriterCommand::SupportEdit(SupportEdit::ObjectRule(
            SupportObjectRule::Editable,
        ))),
        case(WriterCommand::DataCompositionCreate(
            DataCompositionCreate::new(data_composition_definition()),
        )),
        case(WriterCommand::DataCompositionEdit(
            DataCompositionEdit::new(DataCompositionMutation::AddParameter(
                DataCompositionParameter::new(text(
                    "MatrixParameter",
                    DataCompositionParameterName::new,
                )),
            )),
        )),
        case(WriterCommand::SpreadsheetCreate(SpreadsheetCreate::new(
            spreadsheet_document(),
        ))),
    ];
    cases.sort_by_key(|case| match case.kind {
        WriterCommandKind::InterfaceEdit => 0,
        WriterCommandKind::FormCompile => 1,
        _ => 2,
    });
    cases
}

fn case(command: WriterCommand) -> MatrixCase {
    MatrixCase {
        kind: command.kind(),
        command,
    }
}

fn metadata_definition() -> MetadataDefinition {
    MetadataDefinition::new(
        MetadataCommonDefinition::new(text("Items", MetadataChildName::new)),
        MetadataKindDefinition::new(MetadataKind::Catalog, Vec::new()),
    )
}

fn data_composition_definition() -> DataCompositionDefinition {
    DataCompositionDefinition::new(vec![DataCompositionDataSet::Query(
        DataCompositionQueryDataSet::new(
            text("MatrixData", DataSetName::new),
            text("SELECT 1 AS MatrixValue", DataCompositionQueryText::new),
        ),
    )])
    .unwrap()
}

fn spreadsheet_document() -> SpreadsheetDocument {
    let cell = SpreadsheetCell::new(
        1,
        SpreadsheetCellValue::Text(text("Matrix value", SpreadsheetCellText::new)),
    )
    .unwrap();
    let area = SpreadsheetArea::new(
        text("MatrixArea", SpreadsheetAreaName::new),
        vec![SpreadsheetRow::new(vec![cell])],
    )
    .unwrap();
    SpreadsheetDocument::new(vec![area]).unwrap()
}

fn text<T>(value: &str, constructor: impl FnOnce(String) -> Result<T, SemanticValueError>) -> T {
    constructor(value.to_string()).unwrap()
}

fn has_configuration_fixture(kind: WriterCommandKind) -> bool {
    !matches!(
        kind,
        WriterCommandKind::ConfigurationInitialize
            | WriterCommandKind::ExternalProcessorInitialize
            | WriterCommandKind::ExternalReportInitialize
            | WriterCommandKind::FormCompile
            | WriterCommandKind::FormEdit
            | WriterCommandKind::DataCompositionCreate
            | WriterCommandKind::DataCompositionEdit
            | WriterCommandKind::SpreadsheetCreate
    )
}

fn needs_metadata_fixture(kind: WriterCommandKind) -> bool {
    matches!(
        kind,
        WriterCommandKind::ExtensionBorrow
            | WriterCommandKind::ExtensionPatchMethod
            | WriterCommandKind::MetadataEdit
            | WriterCommandKind::MetadataRemove
            | WriterCommandKind::FormCreate
            | WriterCommandKind::FormRemove
            | WriterCommandKind::TemplateCreate
            | WriterCommandKind::TemplateRemove
            | WriterCommandKind::HelpCreate
            | WriterCommandKind::InterfaceEdit
            | WriterCommandKind::SupportEdit
    )
}

fn form_path(kind: WriterCommandKind, root: &Path) -> PathBuf {
    if matches!(
        kind,
        WriterCommandKind::FormCompile | WriterCommandKind::FormEdit
    ) {
        root.join("standalone/Form.xml")
    } else {
        root.join("src/Catalogs/Items/Forms/ObjectForm/Ext/Form.xml")
    }
}

fn xml_has_leaf(path: &Path, name: &str, value: &str) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    let Ok(document) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) else {
        return false;
    };
    document.descendants().any(|node| {
        node.is_element()
            && node.tag_name().name() == name
            && node.text().map(str::trim) == Some(value)
    })
}

fn xml_root_is(path: &Path, expected: &str) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map(|document| document.root_element().tag_name().name() == expected)
        .unwrap_or(false)
}

fn xml_attribute(path: &Path, element: &str, attribute: &str) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == element)
        .and_then(|node| node.attribute(attribute))
        .map(str::to_owned)
}

fn tree_xml_has_leaf(root: &Path, name: &str, value: &str) -> bool {
    files(root)
        .into_iter()
        .any(|path| xml_has_leaf(&path, name, value))
}

fn tree_contains(root: &Path, needle: &str) -> bool {
    if root.is_file() {
        return fs::read(root)
            .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
            .unwrap_or(false);
    }
    files(root).into_iter().any(|path| {
        fs::read(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
            .unwrap_or(false)
    })
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_file() {
            files.push(path);
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        pending.extend(entries.into_iter().rev());
    }
    files
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn fixture_root(kind: WriterCommandKind, scenario: Scenario) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-task8-fix2-{kind:?}-{scenario:?}-{}-{nonce}",
        std::process::id()
    ))
}

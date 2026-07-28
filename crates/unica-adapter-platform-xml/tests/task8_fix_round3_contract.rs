use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    commands::*,
    navigation::{
        FacetSelection, NavigationEnvelope, NavigationQuery, NavigationSelection, NavigationTarget,
        PropertySelection,
    },
    ports::{
        CaptureResult, CompatibilityIssueKind, CompatibilityRequest, FormatReadRequest,
        OperationCancellation, OwnerResolutionMode, WriterRequest,
    },
    semantic_ids::SemanticObjectKind,
    source::{SourceContext, SourceFamily, SourceLocation},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn text<T>(value: &str, constructor: impl FnOnce(String) -> Result<T, SemanticValueError>) -> T {
    constructor(value.to_string()).unwrap()
}

fn root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "unica-task8-fix3-{label}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn execute(
    root: &Path,
    command: WriterCommand,
    sources: Vec<(WriterSourceRole, PathBuf)>,
) -> WriterResult {
    let session = PlatformXmlAdapterFactory::new()
        .capture_writer_session(
            sources,
            root,
            root,
            &root.join(".cache"),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        )
        .unwrap();
    PlatformXmlAdapterFactory::new()
        .operational_registration()
        .writer()
        .execute(&WriterRequest::new(
            session,
            command,
            MutationMode::Apply,
            OperationCancellation::new(),
        ))
        .unwrap()
}

fn production_read(root: &Path, source_root: &Path, target: &Path) -> NavigationEnvelope {
    let registration = PlatformXmlAdapterFactory::new().registration();
    let source = SourceContext::new(
        SourceLocation::new(
            root.to_path_buf(),
            source_root.to_path_buf(),
            target.to_path_buf(),
        ),
        Some("task8-fix3".to_string()),
        SourceFamily::PlatformXml,
        None,
    );
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).unwrap() else {
        panic!("written source must be captured")
    };
    registration
        .read
        .read(&FormatReadRequest {
            captured: captured.clone(),
            query: NavigationQuery {
                target: NavigationTarget::CapturedTarget(
                    captured.binding().target_identity.clone(),
                ),
                select: NavigationSelection {
                    properties: PropertySelection::All,
                    facets: FacetSelection::Full,
                    relations: Vec::new(),
                },
            },
        })
        .unwrap()
}

fn leaf(path: &Path, name: &str) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).ok()?;
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == name)
        .and_then(|node| node.text())
        .map(str::to_string)
}

#[test]
fn cf_and_cfe_emit_requested_compatibility_vendor_version_and_role_semantics() {
    let root = root("cf-cfe-semantics");
    fs::create_dir_all(&root).unwrap();
    let base = root.join("base");
    let cf = ConfigurationInitialize::new(text("Base", ConfigurationName::new))
        .with_vendor(Some(text("Base Vendor", VendorName::new)))
        .with_version(Some(text("2.7.0", ArtifactVersion::new)))
        .with_compatibility(CapabilityRequirement::Explicit(
            VersionNumber::new(vec![8, 3, 27]).unwrap(),
        ));
    let result = execute(
        &root,
        WriterCommand::ConfigurationInitialize(cf),
        vec![(WriterSourceRole::DestinationDirectory, base.clone())],
    );
    assert!(matches!(result.lifecycle(), WriterLifecycle::Applied));
    let configuration = base.join("Configuration.xml");
    assert_eq!(
        leaf(&configuration, "CompatibilityMode").as_deref(),
        Some("Version8_3_27")
    );
    assert_eq!(
        leaf(&configuration, "Vendor").as_deref(),
        Some("Base Vendor")
    );
    assert_eq!(leaf(&configuration, "Version").as_deref(), Some("2.7.0"));
    let projection = production_read(&root, &base, &configuration);
    assert!(projection.nodes.iter().any(|node| {
        node.object_ref.kind == SemanticObjectKind::Configuration
            && node.object_ref.display_name == "Base"
    }));

    for (label, requirement, expected) in [
        ("preserve", CapabilityRequirement::Preserve, "Version8_3_27"),
        (
            "default",
            CapabilityRequirement::AdapterDefault,
            "Version8_3_24",
        ),
        (
            "explicit",
            CapabilityRequirement::Explicit(VersionNumber::new(vec![8, 3, 25]).unwrap()),
            "Version8_3_25",
        ),
    ] {
        let destination = root.join(label);
        let command = ExtensionInitialize::new(text("Audit", ExtensionName::new))
            .with_vendor(Some(text("Extension Vendor", VendorName::new)))
            .with_version(Some(text("5.4.3", ArtifactVersion::new)))
            .with_prefix(Some(text("AUD_", NamePrefix::new)))
            .omit_default_role(true)
            .with_compatibility(requirement);
        let result = execute(
            &root,
            WriterCommand::ExtensionInitialize(command),
            vec![
                (WriterSourceRole::DestinationDirectory, destination.clone()),
                (WriterSourceRole::Configuration, configuration.clone()),
            ],
        );
        assert!(
            matches!(result.lifecycle(), WriterLifecycle::Applied),
            "{label}: {result:?}"
        );
        let descriptor = destination.join("Configuration.xml");
        assert_eq!(
            leaf(&descriptor, "ConfigurationExtensionCompatibilityMode").as_deref(),
            Some(expected),
            "{label}"
        );
        assert_eq!(
            leaf(&descriptor, "Vendor").as_deref(),
            Some("Extension Vendor")
        );
        assert_eq!(leaf(&descriptor, "Version").as_deref(), Some("5.4.3"));
        assert!(!destination.join("Roles/AUD_ОсновнаяРоль.xml").exists());
        let projection = production_read(&root, &destination, &descriptor);
        assert!(projection.nodes.iter().any(|node| {
            node.object_ref.kind == SemanticObjectKind::Configuration
                && node.object_ref.display_name == "Audit"
        }));
    }
    fs::remove_dir_all(root).unwrap();
}

fn compatibility_issue(root: &Path, target: &Path) -> Option<CompatibilityIssueKind> {
    let session = PlatformXmlAdapterFactory::new().capture_unscoped_source(
        target,
        root,
        OwnerResolutionMode::Existing,
    );
    let result = PlatformXmlAdapterFactory::new()
        .operational_registration()
        .compatibility()
        .inspect(&CompatibilityRequest::new(vec![session]).unwrap())
        .unwrap();
    result.issue().map(|issue| issue.kind())
}

#[test]
fn compatibility_classification_is_root_and_owner_aware_not_basename_based() {
    let root = root("same-basename");
    let dcs = root.join("dcs/Template.xml");
    let mxl = root.join("mxl/Template.xml");
    let metadata = root.join("metadata/Template.xml");
    for path in [&dcs, &mxl, &metadata] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    fs::write(
        &dcs,
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\"/>"
        ),
    )
    .unwrap();
    fs::write(
        &mxl,
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"/>"
        ),
    )
    .unwrap();
    fs::write(
        &metadata,
        concat!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.19\">",
            "<Template uuid=\"11111111-1111-4111-8111-111111111111\">",
            "<Properties><Name>Template</Name></Properties></Template></MetaDataObject>"
        ),
    )
    .unwrap();

    assert_eq!(compatibility_issue(&root, &dcs), None);
    assert_eq!(compatibility_issue(&root, &mxl), None);
    assert_eq!(
        compatibility_issue(&root, &metadata),
        Some(CompatibilityIssueKind::Older)
    );

    let direct_interface = root.join("direct/Subsystems/Sales/Ext/CommandInterface.xml");
    fs::create_dir_all(direct_interface.parent().unwrap()).unwrap();
    fs::write(
        &direct_interface,
        concat!(
            "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" ",
            "version=\"2.21\"><CommandsVisibility/><CommandsPlacement/>",
            "<CommandsOrder/><SubsystemsOrder/><GroupsOrder/></CommandInterface>"
        ),
    )
    .unwrap();
    assert_eq!(
        compatibility_issue(&root, &direct_interface),
        Some(CompatibilityIssueKind::Newer)
    );

    let source = root.join("owner");
    let subsystem = SubsystemCreate::from_definition(SubsystemDefinition::new(text(
        "Sales",
        SubsystemName::new,
    )));
    let result = execute(
        &root,
        WriterCommand::SubsystemCreate(subsystem),
        vec![(WriterSourceRole::DestinationDirectory, source.clone())],
    );
    assert!(matches!(result.lifecycle(), WriterLifecycle::Applied));
    let owner = source.join("Subsystems/Sales.xml");
    let owner_text = fs::read_to_string(&owner)
        .unwrap()
        .replace("version=\"2.20\"", "version=\"2.21\"");
    fs::write(&owner, owner_text).unwrap();
    let interface = source.join("Subsystems/Sales/Ext/CommandInterface.xml");
    fs::create_dir_all(interface.parent().unwrap()).unwrap();
    fs::write(
        &interface,
        concat!(
            "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" ",
            "version=\"2.19\"><CommandsVisibility/><CommandsPlacement/>",
            "<CommandsOrder/><SubsystemsOrder/><GroupsOrder/></CommandInterface>"
        ),
    )
    .unwrap();
    assert_eq!(
        compatibility_issue(&root, &interface),
        Some(CompatibilityIssueKind::Newer)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn production_legacy_reconstructors_are_test_gated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/versions/v2_20/writers");
    for (file, declaration) in [
        ("cf.rs", "pub(crate) fn cf_edit_operations"),
        ("cf.rs", "pub(crate) fn cf_edit_batch_value"),
        ("cf.rs", "fn cf_legacy_mutation"),
        ("dcs.rs", "fn dcs_compile_input"),
        ("dcs.rs", "pub(crate) fn dcs_compile_xml"),
        ("dcs.rs", "pub(crate) fn dcs_edit_split_values"),
        ("dcs.rs", "fn dcs_legacy_mutation"),
        ("interface.rs", "pub(crate) fn interface_edit_operations"),
        ("interface.rs", "pub(crate) fn interface_value_list"),
        ("interface.rs", "fn interface_legacy_edit"),
        ("role.rs", "fn role_compile_json_bool"),
        ("role.rs", "fn role_compile_model_from_legacy"),
        ("subsystem.rs", "pub(crate) fn subsystem_edit_operations"),
        ("subsystem.rs", "fn subsystem_edit_operations_from_value"),
        ("subsystem.rs", "pub(crate) fn subsystem_edit_remove_child"),
        ("subsystem.rs", "pub(crate) fn subsystem_edit_set_property"),
        ("subsystem.rs", "fn subsystem_compile_model_from_legacy"),
    ] {
        let source = fs::read_to_string(root.join(file)).unwrap();
        let index = source.find(declaration).unwrap();
        let prefix = &source[..index];
        assert!(
            prefix.ends_with("#[cfg(test)]\n"),
            "{file}: {declaration} is in the production compile graph"
        );
    }
}

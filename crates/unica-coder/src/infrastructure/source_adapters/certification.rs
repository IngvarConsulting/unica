//! Reusable read-adapter certification through the public registry contract.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::json;

use crate::{
    domain::{
        navigation::{
            ActionAvailability, Authorability, CoverageState, FormatCompatibility,
            NavigationStatus, NodeKind,
        },
        source_adapters::{AdapterMaturity, SourceAdapterErrorKind},
        workspace::WorkspaceContext,
    },
    infrastructure::{
        native_operations::{typed_result::NativeOperationResult, NativeOperationAdapter},
        source_adapters::{
            platform_xml::PlatformXmlReadAdapter, registry::BuiltInSourceAdapterRegistry,
            SourceInput, SourceReadAdapter,
        },
    },
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct ReadAdapterCertificationCase {
    pub(crate) supported: SourceInput,
    pub(crate) unsupported_version: SourceInput,
    pub(crate) corrupted: SourceInput,
    pub(crate) expected_adapter_id: &'static str,
}

/// Certifies only the registry and typed navigation behavior a consumer sees.
/// Decoder/projector internals have their own focused unit tests.
pub(crate) fn certify_read_adapter(
    registry: &BuiltInSourceAdapterRegistry,
    case: ReadAdapterCertificationCase,
) {
    let ready = registry
        .inspect(case.supported)
        .expect("supported source must inspect");
    assert_eq!(ready.status, NavigationStatus::Available);
    assert_eq!(
        ready
            .snapshot
            .expect("ready navigation must be snapshotted")
            .adapter_id,
        case.expected_adapter_id
    );

    let unsupported = registry
        .inspect(case.unsupported_version)
        .expect("unsupported version is a typed unavailable response");
    assert_eq!(unsupported.status, NavigationStatus::Unavailable);
    assert_eq!(unsupported.diagnostics.len(), 1);
    assert_eq!(unsupported.diagnostics[0].code, "format_unsupported");
    let unavailable = serde_json::to_value(unsupported).expect("unavailable navigation serializes");
    assert_eq!(
        unavailable
            .as_object()
            .expect("navigation is an object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "diagnostics".to_string(),
            "nodes".to_string(),
            "relations".to_string(),
            "root".to_string(),
            "schemaVersion".to_string(),
            "snapshot".to_string(),
            "status".to_string(),
        ])
    );

    let corrupted = registry
        .inspect(case.corrupted)
        .expect_err("corrupted source must fail closed");
    assert_eq!(corrupted.kind, SourceAdapterErrorKind::DecodeCorrupted);
}

#[test]
fn platform_xml_2_20_is_read_compatible_through_the_public_registry() {
    let supported = Fixture::certified_document("2.20");
    let unsupported = Fixture::certified_document("2.19");
    let corrupted = Fixture::corrupted_root();
    let registry = BuiltInSourceAdapterRegistry::new();

    certify_read_adapter(
        &registry,
        ReadAdapterCertificationCase {
            supported: supported.input(),
            unsupported_version: unsupported.input(),
            corrupted: corrupted.input(),
            expected_adapter_id: "platform-xml-2.20",
        },
    );

    let ready = registry
        .inspect(supported.input())
        .expect("supported source must inspect");
    for (kind, name) in [
        (NodeKind::Attribute, "Number"),
        (NodeKind::TabularSection, "Lines"),
        (NodeKind::Command, "Post"),
        (NodeKind::Form, "ShipmentForm"),
    ] {
        assert!(
            ready.node_named(kind, name).is_some(),
            "missing certified node {name}"
        );
    }
    assert!(ready
        .nodes
        .iter()
        .any(|node| node.object_ref.display_name == "Print"));
    let form = ready
        .node_named(NodeKind::Form, "ShipmentForm")
        .expect("form node");
    assert_eq!(form.capability.coverage, CoverageState::Partial);
    assert!(ready.nodes.iter().all(|node| node
        .actions
        .iter()
        .all(|action| action.availability != ActionAvailability::Executable)));
    assert!(ready
        .nodes
        .iter()
        .all(|node| node.capability.format == FormatCompatibility::Compatible));
    let document = ready
        .node_named(NodeKind::Document, "Shipment")
        .expect("document node");
    assert!(ready.owning_relation(&document.object_ref).is_some());

    let adapter = PlatformXmlReadAdapter::new();
    assert_eq!(adapter.manifest().maturity, AdapterMaturity::ReadCompatible);
    assert_eq!(adapter.manifest().adapter_id, "platform-xml-2.20");
}

#[test]
fn platform_xml_certification_fails_closed_at_declared_boundaries() {
    let registry = BuiltInSourceAdapterRegistry::new();

    assert_error(
        &registry,
        Fixture::document(
            "2.20",
            r#"<ChildObjects>
                <TabularSection><Properties><Name>Lines</Name></Properties></TabularSection>
                <TabularSection><Properties><Name>Lines</Name></Properties></TabularSection>
            </ChildObjects>"#,
        ),
        SourceAdapterErrorKind::IdentityCollision,
    );
    assert_error(
        &registry,
        Fixture::document(
            "2.20",
            r#"<ChildObjects>
                <Form>ShipmentForm</Form>
                <Form>ShipmentForm</Form>
            </ChildObjects>"#,
        ),
        SourceAdapterErrorKind::IdentityCollision,
    );

    let conflicting_form = Fixture::document(
        "2.20",
        r#"<ChildObjects><Form uuid="22222222-2222-2222-2222-222222222222">ShipmentForm</Form></ChildObjects>"#,
    );
    conflicting_form.write(
        "src/Documents/Shipment/Forms/ShipmentForm.xml",
        form_descriptor("33333333-3333-3333-3333-333333333333"),
    );
    let error = registry
        .inspect(conflicting_form.input())
        .expect_err("conflicting external Form descriptor must fail closed");
    assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    assert_eq!(error.code(), "projection_ambiguous");

    let noncanonical_mxl = Fixture::document(
        "2.20",
        "<ChildObjects><Template>Print</Template></ChildObjects>",
    );
    noncanonical_mxl.write(
        "src/Documents/Shipment/Templates/Print.xml",
        template_descriptor(),
    );
    noncanonical_mxl.write(
        "src/Documents/Shipment/Templates/Print/Ext/Template.mxl",
        spreadsheet_document(),
    );
    assert_error(
        &registry,
        noncanonical_mxl,
        SourceAdapterErrorKind::DecodeCorrupted,
    );

    let unreadable_support = Fixture::certified_document("2.20");
    unreadable_support.write("src/Documents/ParentConfigurations.bin", "{");
    let navigation = registry
        .inspect(unreadable_support.input())
        .expect("unreadable support remains inspectable but non-authorable");
    let affected = navigation
        .node_named(NodeKind::Document, "Shipment")
        .expect("supported document node");
    assert_eq!(
        affected.capability.authorability,
        Authorability::UnknownSupportState
    );
    assert_eq!(
        affected.capability_state.authorability,
        Authorability::UnknownSupportState
    );
    assert!(navigation.nodes.iter().all(|node| {
        node.actions
            .iter()
            .all(|action| action.availability != ActionAvailability::Executable)
    }));

    let invalid_source_map = Fixture::certified_document("2.20");
    invalid_source_map.write("v8project.yaml", "source-set: [");
    assert_error(
        &registry,
        invalid_source_map,
        SourceAdapterErrorKind::SourceUnavailable,
    );
}

#[test]
fn public_typed_gateway_platform_xml_2_20_serializes_navigation() {
    let fixture = Fixture::certified_document("2.20");
    let result = NativeOperationAdapter::invoke_with_data(
        "meta-info",
        "unica.meta.info",
        &json!({"ObjectPath": fixture.target})
            .as_object()
            .expect("object args")
            .clone(),
        &fixture.context(),
        false,
        false,
    )
    .expect("typed gateway must return navigation");
    assert_public_typed_result(&result);
    let data = result.data.expect("typed gateway data");
    let serialized = json!({"data": data});
    let output = serde_json::to_string(&serialized).expect("typed navigation serializes");
    assert!(!output.contains(&fixture.root.display().to_string()));
    assert_eq!(serialized["data"]["navigation"]["status"], "ready");
    assert!(serialized["data"]["navigation"]["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .any(|node| node["properties"]["name"]["type"].is_string()));
    assert_eq!(
        serialized["data"]["navigation"]["nodes"][0]["properties"]["name"]["valueState"],
        "explicit"
    );
    assert_eq!(
        serialized["data"]["navigation"]["nodes"][0]["properties"]["name"]["capability"],
        "readOnly"
    );
    println!("CERTIFICATION_TYPED_GATEWAY_JSON={output}");
}

fn assert_public_typed_result(result: &NativeOperationResult) {
    assert!(result.adapter.ok);
    assert!(result.adapter.stdout.is_none());
    let data = result.data.as_ref().expect("typed gateway data");
    assert_eq!(
        data.as_object()
            .expect("typed gateway object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["navigation".to_string()])
    );
    assert_eq!(
        data["navigation"]
            .as_object()
            .expect("navigation object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "diagnostics".to_string(),
            "nodes".to_string(),
            "relations".to_string(),
            "root".to_string(),
            "schemaVersion".to_string(),
            "snapshot".to_string(),
            "status".to_string(),
        ])
    );
}

fn assert_error(
    registry: &BuiltInSourceAdapterRegistry,
    fixture: Fixture,
    expected: SourceAdapterErrorKind,
) {
    let error = registry
        .inspect(fixture.input())
        .expect_err("fixture must fail closed");
    assert_eq!(error.kind, expected);
}

struct Fixture {
    root: PathBuf,
    target: PathBuf,
}

impl Fixture {
    fn certified_document(version: &str) -> Self {
        let fixture = Self::document(version, standard_children());
        fixture.write(
            "src/Documents/Shipment/Forms/ShipmentForm.xml",
            form_descriptor("22222222-2222-2222-2222-222222222222"),
        );
        fixture.write(
            "src/Documents/Shipment/Forms/ShipmentForm/Ext/Form.xml",
            managed_form(),
        );
        fixture.write(
            "src/Documents/Shipment/Templates/Print.xml",
            template_descriptor(),
        );
        fixture.write(
            "src/Documents/Shipment/Templates/Print/Ext/Template.xml",
            spreadsheet_document(),
        );
        fixture
    }

    fn document(version: &str, children: &str) -> Self {
        let root = unique_root();
        let target = root.join("src/Documents/Shipment.xml");
        fs::create_dir_all(target.parent().expect("document parent")).expect("create fixture");
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .expect("write project");
        fs::write(
            root.join("src/Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Configuration</Name></Properties></Configuration></MetaDataObject>"#
            ),
        )
        .expect("write configuration");
        fs::write(
            &target,
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Document uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>Shipment</Name></Properties>{children}</Document></MetaDataObject>"#
            ),
        )
        .expect("write document");
        Self { root, target }
    }

    fn corrupted_root() -> Self {
        let fixture = Self::document("2.20", standard_children());
        fixture.write("src/Documents/Shipment.xml", "<MetaDataObject>");
        fixture
    }

    fn write(&self, relative: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
        fs::write(path, contents).expect("write fixture file");
    }

    fn input(&self) -> SourceInput {
        SourceInput {
            workspace_root: self.root.clone(),
            target: self.target.clone(),
            configured_source_set: None,
        }
    }

    fn context(&self) -> WorkspaceContext {
        WorkspaceContext {
            cwd: self.root.clone(),
            workspace_root: self.root.clone(),
            cache_root: self.root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn unique_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "unica-read-adapter-certification-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    ))
}

fn standard_children() -> &'static str {
    r#"<ChildObjects>
        <Attribute><Properties><Name>Number</Name></Properties></Attribute>
        <TabularSection><Properties><Name>Lines</Name></Properties></TabularSection>
        <Command><Properties><Name>Post</Name></Properties></Command>
        <Form uuid="22222222-2222-2222-2222-222222222222">ShipmentForm</Form>
        <Template>Print</Template>
    </ChildObjects>"#
}

fn form_descriptor(uuid: &str) -> String {
    format!(
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Form uuid="{uuid}"><Properties><Name>ShipmentForm</Name></Properties></Form></MetaDataObject>"#
    )
}

fn managed_form() -> &'static str {
    r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#
}

fn template_descriptor() -> &'static str {
    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Template><Properties><Name>Print</Name><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>"#
}

fn spreadsheet_document() -> &'static str {
    r#"<SpreadsheetDocument xmlns="http://v8.1c.ru/spreadsheet/document"/>"#
}

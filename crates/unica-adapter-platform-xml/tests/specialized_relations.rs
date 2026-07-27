use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{
        FacetSelection, NavigationEnvelope, NavigationQuery, NavigationSelection, NavigationStatus,
        NavigationTarget, PropertySelection, RelationKind,
    },
    ports::{CaptureResult, FormatReadRequest},
    semantic_ids::{SemanticObjectKind, SemanticPropertyId, SemanticRelationId},
    source::{SourceContext, SourceFamily, SourceLocation},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

#[test]
fn task6_specialized_children_keep_kind_role_order_and_stable_object_refs() {
    let register = read_inline_twice(
        "register",
        "Metrics.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" version="2.20"><InformationRegister uuid="81000000-0000-0000-0000-000000000001"><Properties><Name>Metrics</Name></Properties><ChildObjects><Dimension uuid="81000000-0000-0000-0000-000000000002"><Properties><Name>SecondDimension</Name><Master>true</Master></Properties></Dimension><Dimension uuid="81000000-0000-0000-0000-000000000003"><Properties><Name>FirstDimension</Name></Properties></Dimension><Resource uuid="81000000-0000-0000-0000-000000000004"><Properties><Name>Amount</Name></Properties></Resource></ChildObjects></InformationRegister></MetaDataObject>"#
        ),
    );
    assert_targets(
        &register.0,
        "Metrics",
        SemanticRelationId::DIMENSIONS,
        RelationKind::Contains,
        &["SecondDimension", "FirstDimension"],
    );
    assert_targets(
        &register.0,
        "Metrics",
        SemanticRelationId::RESOURCES,
        RelationKind::Contains,
        &["Amount"],
    );
    assert_stable_nodes(&register.0, &register.1);

    let enumeration = read_inline_twice(
        "enumeration",
        "Priority.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" version="2.20"><Enum uuid="82000000-0000-0000-0000-000000000001"><Properties><Name>Priority</Name></Properties><ChildObjects><EnumValue uuid="82000000-0000-0000-0000-000000000002"><Properties><Name>High</Name></Properties></EnumValue><EnumValue uuid="82000000-0000-0000-0000-000000000003"><Properties><Name>Low</Name></Properties></EnumValue></ChildObjects></Enum></MetaDataObject>"#
        ),
    );
    assert_targets(
        &enumeration.0,
        "Priority",
        SemanticRelationId::ENUM_VALUES,
        RelationKind::Contains,
        &["High", "Low"],
    );
    assert_stable_nodes(&enumeration.0, &enumeration.1);

    let http = read_inline_twice(
        "http",
        "ExternalApi.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" version="2.20"><HTTPService uuid="83000000-0000-0000-0000-000000000001"><Properties><Name>ExternalApi</Name></Properties><ChildObjects><URLTemplate uuid="83000000-0000-0000-0000-000000000002"><Properties><Name>Items</Name><Template>/items/{{id}}</Template></Properties><ChildObjects><Method uuid="83000000-0000-0000-0000-000000000003"><Properties><Name>Delete</Name><HTTPMethod>DELETE</HTTPMethod><Handler>DeleteItem</Handler></Properties></Method><Method uuid="83000000-0000-0000-0000-000000000004"><Properties><Name>Get</Name><HTTPMethod>GET</HTTPMethod><Handler>GetItem</Handler></Properties></Method></ChildObjects></URLTemplate></ChildObjects></HTTPService></MetaDataObject>"#
        ),
    );
    assert_targets(
        &http.0,
        "ExternalApi",
        SemanticRelationId::URL_TEMPLATES,
        RelationKind::Contains,
        &["Items"],
    );
    assert_targets(
        &http.0,
        "Items",
        SemanticRelationId::METHODS,
        RelationKind::Contains,
        &["Delete", "Get"],
    );
    assert_typed_property(
        &http.0,
        "Delete",
        SemanticPropertyId::HTTP_SERVICE_METHOD,
        "string",
    );
    assert_stable_nodes(&http.0, &http.1);

    let web = read_inline_twice(
        "web",
        "Exchange.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" version="2.20"><WebService uuid="84000000-0000-0000-0000-000000000001"><Properties><Name>Exchange</Name></Properties><ChildObjects><Operation uuid="84000000-0000-0000-0000-000000000002"><Properties><Name>Send</Name><ProcedureName>SendData</ProcedureName></Properties><ChildObjects><Parameter uuid="84000000-0000-0000-0000-000000000003"><Properties><Name>Payload</Name><TransferDirection>In</TransferDirection></Properties></Parameter><Parameter uuid="84000000-0000-0000-0000-000000000004"><Properties><Name>Trace</Name><TransferDirection>Out</TransferDirection></Properties></Parameter></ChildObjects></Operation></ChildObjects></WebService></MetaDataObject>"#
        ),
    );
    assert_targets(
        &web.0,
        "Exchange",
        SemanticRelationId::OPERATIONS,
        RelationKind::Contains,
        &["Send"],
    );
    assert_targets(
        &web.0,
        "Send",
        SemanticRelationId::PARAMETERS,
        RelationKind::Contains,
        &["Payload", "Trace"],
    );
    assert_typed_property(
        &web.0,
        "Payload",
        SemanticPropertyId::WEB_SERVICE_PARAMETER_DIRECTION,
        "enum",
    );
    assert_stable_nodes(&web.0, &web.1);
}

#[test]
fn task6_reference_relations_are_traversable_nodes_with_stable_semantic_targets() {
    let (first, second) = read_inline_twice(
        "document-references",
        "Invoice.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" version="2.20"><Document uuid="85000000-0000-0000-0000-000000000001"><Properties><Name>Invoice</Name><BasedOn><xr:Item>Document.Order</xr:Item><xr:Item>Document.Return</xr:Item></BasedOn><RegisterRecords><xr:Item>InformationRegister.Stock</xr:Item><xr:Item>AccumulationRegister.Balance</xr:Item></RegisterRecords></Properties></Document></MetaDataObject>"#
        ),
    );
    assert_targets(
        &first,
        "Invoice",
        SemanticRelationId::BASED_ON,
        RelationKind::References,
        &["Order", "Return"],
    );
    assert_targets(
        &first,
        "Invoice",
        SemanticRelationId::REGISTER_RECORDS,
        RelationKind::References,
        &["Stock", "Balance"],
    );

    for relation in first.relation_index.iter().filter(|relation| {
        matches!(
            relation.role,
            SemanticRelationId::BASED_ON | SemanticRelationId::REGISTER_RECORDS
        )
    }) {
        let node = first
            .nodes
            .iter()
            .find(|node| node.object_ref == relation.target)
            .unwrap_or_else(|| {
                panic!(
                    "reference target {} has no traversable node",
                    relation.target.display_name
                )
            });
        assert!(
            node.properties
                .contains_key(&SemanticPropertyId::METADATA_NAME),
            "reference target must retain a typed semantic name"
        );
    }
    assert_stable_nodes(&first, &second);
}

#[test]
fn task6_unknown_specialized_children_remain_distinct_readable_partial_facts() {
    let (envelope, _) = read_inline_twice(
        "unknown-specialized",
        "UnknownEndpoint.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" version="2.20"><HTTPService uuid="86000000-0000-0000-0000-000000000001"><Properties><Name>UnknownEndpoint</Name></Properties><ChildObjects><URLTemplate uuid="86000000-0000-0000-0000-000000000002"><Properties><Name>Items</Name><Template>/items</Template></Properties><ChildObjects><FutureMethod><Properties><Name>Repeated</Name><Payload>first-readable-value</Payload></Properties></FutureMethod><FutureMethod><Properties><Name>Repeated</Name><Payload>second-readable-value</Payload></Properties></FutureMethod></ChildObjects></URLTemplate></ChildObjects></HTTPService></MetaDataObject>"#
        ),
    );
    assert_eq!(envelope.status, NavigationStatus::Partial);
    let unknown = envelope
        .nodes
        .iter()
        .filter(|node| {
            node.object_ref.kind == SemanticObjectKind::Unknown
                && node.object_ref.display_name == "Repeated"
        })
        .collect::<Vec<_>>();
    assert_eq!(unknown.len(), 2);
    assert_ne!(
        unknown[0].object_ref.object_key,
        unknown[1].object_ref.object_key
    );
    let readable = unknown
        .iter()
        .map(|node| {
            serde_json::to_string(
                node.properties[&SemanticPropertyId::UNKNOWN_FACTS]
                    .value()
                    .expect("readable unknown fact"),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(readable
        .iter()
        .any(|value| value.contains("first-readable-value")));
    assert!(readable
        .iter()
        .any(|value| value.contains("second-readable-value")));
    assert_targets(
        &envelope,
        "Items",
        SemanticRelationId::UNKNOWN,
        RelationKind::Contains,
        &["Repeated", "Repeated"],
    );
}

#[test]
fn task6_fix1_no_uuid_children_are_unique_across_roles_and_within_one_role() {
    let (first, second) = read_inline_twice(
        "derived-child-identities",
        "Metrics.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" version="2.20"><InformationRegister uuid="87000000-0000-0000-0000-000000000001"><Properties><Name>Metrics</Name></Properties><ChildObjects><Dimension><Properties><Name>Shared</Name></Properties></Dimension><Resource><Properties><Name>Shared</Name></Properties></Resource><Dimension><Properties><Name>Repeated</Name></Properties></Dimension><Dimension><Properties><Name>Repeated</Name></Properties></Dimension></ChildObjects></InformationRegister></MetaDataObject>"#
        ),
    );
    let owner = node_named(&first, "Metrics");
    let dimensions = relation_targets(&first, &owner.object_ref, SemanticRelationId::DIMENSIONS);
    let resources = relation_targets(&first, &owner.object_ref, SemanticRelationId::RESOURCES);

    assert_eq!(
        dimensions
            .iter()
            .map(|target| target.display_name.as_str())
            .collect::<Vec<_>>(),
        ["Shared", "Repeated", "Repeated"]
    );
    assert_eq!(resources[0].display_name, "Shared");
    assert_ne!(dimensions[0].object_key, resources[0].object_key);
    assert_ne!(dimensions[1].object_key, dimensions[2].object_key);
    assert_stable_nodes(&first, &second);
}

#[test]
fn task6_fix1_loaded_forward_reference_resolves_only_to_the_real_node() {
    let (envelope, _) = read_inline_twice(
        "loaded-forward-reference",
        "Configuration.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" version="2.20"><Configuration uuid="88000000-0000-0000-0000-000000000001"><Properties><Name>Configuration</Name></Properties><ChildObjects><Document uuid="88000000-0000-0000-0000-000000000002"><Properties><Name>Invoice</Name><BasedOn><xr:Item>Document.Order</xr:Item></BasedOn></Properties></Document><Document uuid="88000000-0000-0000-0000-000000000003"><Properties><Name>Order</Name></Properties></Document></ChildObjects></Configuration></MetaDataObject>"#
        ),
    );
    let invoice = node_named(&envelope, "Invoice");
    let targets = relation_targets(&envelope, &invoice.object_ref, SemanticRelationId::BASED_ON);
    assert_eq!(targets.len(), 1);
    assert_eq!(
        targets[0].object_key.as_str(),
        "uuid:88000000-0000-0000-0000-000000000003"
    );
    let target = envelope
        .nodes
        .iter()
        .find(|node| node.object_ref == targets[0])
        .expect("forward target must be the loaded node");
    assert_eq!(
        target.capability.resolution,
        unica_format_core::navigation::ResolutionState::Resolved
    );
    assert!(!envelope
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "referenceTargetUnresolved"));
}

#[test]
fn task6_fix1_external_known_reference_is_an_unresolved_partial_stub_without_owner() {
    let (envelope, _) = read_inline_twice(
        "external-known-reference",
        "Invoice.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" version="2.20"><Document uuid="89000000-0000-0000-0000-000000000001"><Properties><Name>Invoice</Name><BasedOn><xr:Item>Document.Order</xr:Item></BasedOn></Properties></Document></MetaDataObject>"#
        ),
    );
    assert_eq!(envelope.status, NavigationStatus::Partial);
    let invoice = node_named(&envelope, "Invoice");
    let target = relation_targets(&envelope, &invoice.object_ref, SemanticRelationId::BASED_ON)
        .pop()
        .expect("external target");
    let stub = envelope
        .nodes
        .iter()
        .find(|node| node.object_ref == target)
        .expect("external target must remain traversable");
    assert_eq!(
        stub.capability.resolution,
        unica_format_core::navigation::ResolutionState::Unresolved
    );
    assert_ne!(
        stub.capability.coverage,
        unica_format_core::navigation::CoverageState::Complete
    );
    assert!(envelope
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "referenceTargetUnresolved"));
    assert!(!envelope.relation_index.iter().any(|relation| {
        relation.kind == RelationKind::Contains && relation.target == stub.object_ref
    }));
    assert!(stub
        .actions
        .iter()
        .all(|action| action.owning_relation.is_none()));
}

#[test]
fn task6_fix1_external_unknown_same_name_targets_do_not_merge_or_leak_native_identity() {
    let (envelope, second) = read_inline_twice(
        "external-unknown-collision",
        "Invoice.xml",
        &format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:xr="{XR}" version="2.20"><Document uuid="8a000000-0000-0000-0000-000000000001"><Properties><Name>Invoice</Name><BasedOn><xr:Item>FutureAlpha.Shared</xr:Item><xr:Item>FutureBeta.Shared</xr:Item></BasedOn></Properties></Document></MetaDataObject>"#
        ),
    );
    let invoice = node_named(&envelope, "Invoice");
    let targets = relation_targets(&envelope, &invoice.object_ref, SemanticRelationId::BASED_ON);
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].display_name, "Shared");
    assert_eq!(targets[1].display_name, "Shared");
    assert_ne!(targets[0].object_key, targets[1].object_key);
    for target in &targets {
        let node = envelope
            .nodes
            .iter()
            .find(|node| node.object_ref == *target)
            .expect("unknown external target must have its own stub");
        assert_eq!(
            node.capability.resolution,
            unica_format_core::navigation::ResolutionState::Unresolved
        );
    }
    let public = serde_json::to_string(&envelope).unwrap();
    assert!(!public.contains("FutureAlpha.Shared"));
    assert!(!public.contains("FutureBeta.Shared"));
    assert_stable_nodes(&envelope, &second);
}

fn assert_targets(
    envelope: &NavigationEnvelope,
    owner_name: &str,
    role: SemanticRelationId,
    kind: RelationKind,
    expected: &[&str],
) {
    let owner = envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.display_name == owner_name)
        .unwrap_or_else(|| panic!("missing owner {owner_name}"));
    let actual = envelope
        .relation_index
        .iter()
        .filter(|relation| {
            relation.source == owner.object_ref && relation.role == role && relation.kind == kind
        })
        .map(|relation| relation.target.display_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "{owner_name}:{}", role.as_str());
}

fn node_named<'a>(
    envelope: &'a NavigationEnvelope,
    name: &str,
) -> &'a unica_format_core::navigation::NavigationNode {
    envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.display_name == name)
        .unwrap_or_else(|| panic!("missing node {name}"))
}

fn relation_targets(
    envelope: &NavigationEnvelope,
    owner: &unica_format_core::navigation::ObjectRef,
    role: SemanticRelationId,
) -> Vec<unica_format_core::navigation::ObjectRef> {
    envelope
        .relation_index
        .iter()
        .filter(|relation| relation.source == *owner && relation.role == role)
        .map(|relation| relation.target.clone())
        .collect()
}

fn assert_stable_nodes(first: &NavigationEnvelope, second: &NavigationEnvelope) {
    let identities = |envelope: &NavigationEnvelope| {
        envelope
            .nodes
            .iter()
            .map(|node| {
                (
                    node.object_ref.kind,
                    node.object_ref.display_name.clone(),
                    node.object_ref.object_key.as_str().to_string(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(identities(first), identities(second));
}

fn assert_typed_property(
    envelope: &NavigationEnvelope,
    node_name: &str,
    property: SemanticPropertyId,
    expected_type: &str,
) {
    let node = envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.display_name == node_name)
        .unwrap_or_else(|| panic!("missing node {node_name}"));
    let value = serde_json::to_value(&node.properties[&property]).unwrap();
    assert_eq!(value["type"], expected_type);
}

fn read_inline_twice(
    label: &str,
    file_name: &str,
    xml: &str,
) -> (NavigationEnvelope, NavigationEnvelope) {
    let root = std::env::temp_dir().join(format!(
        "unica-platform-xml-task6-{label}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let target = root.join(file_name);
    fs::write(&target, xml).unwrap();
    let first = read_path(&root, &target);
    let second = read_path(&root, &target);
    fs::remove_dir_all(root).unwrap();
    (first, second)
}

fn read_path(source_root: &Path, target: &Path) -> NavigationEnvelope {
    let source = SourceContext::new(
        SourceLocation::new(repo_root(), source_root.to_path_buf(), target.to_path_buf()),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    );
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) = registration
        .capture
        .capture(&source)
        .expect("specialized fixture must probe")
    else {
        panic!("specialized fixture must be captured");
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
        .expect("specialized fixture must project")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

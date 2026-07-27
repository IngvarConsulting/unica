use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{
        FacetSelection, NavigationEnvelope, NavigationNode, NavigationQuery, NavigationSelection,
        NavigationStatus, NavigationTarget, PropertySelection,
    },
    ports::{CaptureResult, FormatReadRequest},
    semantic_ids::{
        SemanticEnumValue, SemanticFacetId, SemanticObjectKind, SemanticPropertyId,
        SemanticRelationId,
    },
    source::{SourceContext, SourceFamily, SourceLocation},
    value::PropertyValue,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const V8: &str = "http://v8.1c.ru/8.1/data/core";
const XS: &str = "http://www.w3.org/2001/XMLSchema";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
const CFG: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

#[test]
fn coverage_manifest_uses_only_closed_core_vocabulary() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/versions/v2_20/coverage.json");
    let text = fs::read_to_string(path).expect("the 2.20 adapter must own a coverage manifest");
    let manifest: serde_json::Value =
        serde_json::from_str(&text).expect("coverage manifest must be valid JSON");

    for value in strings(&manifest, "supportedObjectKinds") {
        assert!(
            SemanticObjectKind::parse(value).is_some(),
            "unregistered object kind in coverage manifest: {value}"
        );
    }
    for value in strings(&manifest, "supportedSemanticProperties") {
        assert!(
            SemanticPropertyId::parse(value).is_some(),
            "unregistered property in coverage manifest: {value}"
        );
    }
    for value in strings(&manifest, "supportedRelations") {
        assert!(
            SemanticRelationId::parse(value).is_some(),
            "unregistered relation in coverage manifest: {value}"
        );
    }
    for value in strings(&manifest, "supportedFacets") {
        assert!(
            SemanticFacetId::parse(value).is_some(),
            "unregistered facet in coverage manifest: {value}"
        );
    }
    for area in manifest["knownPartialAreas"]
        .as_array()
        .expect("knownPartialAreas array")
    {
        for kind in area["objectKinds"]
            .as_array()
            .expect("partial-area objectKinds")
        {
            assert!(SemanticObjectKind::parse(kind.as_str().unwrap()).is_some());
        }
        for property in area["semanticProperties"]
            .as_array()
            .expect("partial-area semanticProperties")
        {
            assert!(SemanticPropertyId::parse(property.as_str().unwrap()).is_some());
        }
    }

    for forbidden in ["MetaDataObject", "ChildObjects", "xsi:type", "http://"] {
        assert!(
            !text.contains(forbidden),
            "coverage manifest leaked native vocabulary: {forbidden}"
        );
    }
}

#[test]
fn document_full_and_drill_down_facts_are_typed_and_related() {
    let properties = format!(
        r#"
<Name>Shipment</Name>
<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Отгрузка</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Shipment</v8:content></v8:item></Synonym>
<Comment>Moves goods</Comment>
<ObjectPresentation><v8:item><v8:lang>en</v8:lang><v8:content>Shipment</v8:content></v8:item></ObjectPresentation>
<ExtendedObjectPresentation><v8:item><v8:lang>en</v8:lang><v8:content>Shipment document</v8:content></v8:item></ExtendedObjectPresentation>
<ListPresentation><v8:item><v8:lang>en</v8:lang><v8:content>Shipments</v8:content></v8:item></ListPresentation>
<ExtendedListPresentation><v8:item><v8:lang>en</v8:lang><v8:content>Shipment documents</v8:content></v8:item></ExtendedListPresentation>
<NumberType>String</NumberType>
<NumberLength>11</NumberLength>
<NumberPeriodicity>Year</NumberPeriodicity>
<Autonumbering>true</Autonumbering>
<Posting>Allow</Posting>
<RegisterRecords><xr:Item xsi:type="xr:MDObjectRef">AccumulationRegister.Stock</xr:Item></RegisterRecords>
<BasedOn><xr:Item xsi:type="xr:MDObjectRef">Document.Order</xr:Item></BasedOn>
"#
    );
    let children = r#"
<Attribute uuid="22222222-2222-2222-2222-222222222222">
  <Properties>
    <Name>Customer</Name>
    <Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Customer</v8:content></v8:item></Synonym>
    <Type>
      <v8:Type>xs:string</v8:Type>
      <v8:StringQualifiers><v8:Length>20</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>
      <v8:Type>cfg:CatalogRef.Customers</v8:Type>
    </Type>
    <FillChecking>ShowError</FillChecking>
    <Indexing>Index</Indexing>
    <MultiLine>true</MultiLine>
    <Use>ForItem</Use>
    <FillValue xsi:type="xs:string">Retail</FillValue>
  </Properties>
</Attribute>
<TabularSection uuid="33333333-3333-3333-3333-333333333333">
  <Properties><Name>Lines</Name><LineNumberLength>5</LineNumberLength></Properties>
  <ChildObjects>
    <Attribute uuid="44444444-4444-4444-4444-444444444444">
      <Properties>
        <Name>Item</Name>
        <Type><v8:Type>cfg:CatalogRef.Items</v8:Type></Type>
        <FillChecking>DontCheck</FillChecking>
      </Properties>
    </Attribute>
  </ChildObjects>
</TabularSection>
<Command uuid="55555555-5555-5555-5555-555555555555">
  <Properties><Name>Post</Name><Group>NavigationPanelOrdinary</Group><Representation>Auto</Representation></Properties>
</Command>
"#;
    let envelope = read_fixture("document", "Document", "Shipment", &properties, children);

    assert_eq!(envelope.status, NavigationStatus::Available);
    let document = node(&envelope, SemanticObjectKind::Document, "Shipment");
    assert_value(
        document,
        SemanticPropertyId::METADATA_KIND,
        PropertyValue::String("document".to_string()),
    );
    assert_value(
        document,
        SemanticPropertyId::METADATA_NAME,
        PropertyValue::String("Shipment".to_string()),
    );
    assert_value(
        document,
        SemanticPropertyId::METADATA_UUID,
        PropertyValue::Uuid("11111111-1111-1111-1111-111111111111".parse().unwrap()),
    );
    assert_value(
        document,
        SemanticPropertyId::METADATA_SYNONYM,
        PropertyValue::LocalizedString(BTreeMap::from([
            ("en".to_string(), "Shipment".to_string()),
            ("ru".to_string(), "Отгрузка".to_string()),
        ])),
    );
    assert_value(
        document,
        SemanticPropertyId::METADATA_COMMENT,
        PropertyValue::String("Moves goods".to_string()),
    );
    assert_value(
        document,
        SemanticPropertyId::PRESENTATION_OBJECT,
        localized("en", "Shipment"),
    );
    assert_value(
        document,
        SemanticPropertyId::PRESENTATION_EXTENDED_OBJECT,
        localized("en", "Shipment document"),
    );
    assert_value(
        document,
        SemanticPropertyId::PRESENTATION_LIST,
        localized("en", "Shipments"),
    );
    assert_value(
        document,
        SemanticPropertyId::PRESENTATION_EXTENDED_LIST,
        localized("en", "Shipment documents"),
    );
    assert_value(
        document,
        SemanticPropertyId::DOCUMENT_NUMBER_TYPE,
        PropertyValue::EnumSymbol(SemanticEnumValue::STRING),
    );
    assert_value(
        document,
        SemanticPropertyId::DOCUMENT_NUMBER_LENGTH,
        PropertyValue::Integer(11),
    );
    assert_value(
        document,
        SemanticPropertyId::DOCUMENT_NUMBER_PERIODICITY,
        PropertyValue::EnumSymbol(SemanticEnumValue::YEAR),
    );
    assert_value(
        document,
        SemanticPropertyId::DOCUMENT_NUMBER_AUTO,
        PropertyValue::Boolean(true),
    );
    assert_value(
        document,
        SemanticPropertyId::DOCUMENT_POSTING_MODE,
        PropertyValue::EnumSymbol(SemanticEnumValue::ALLOW),
    );
    for property in [
        SemanticPropertyId::SUPPORT_STATE,
        SemanticPropertyId::SUPPORT_AUTHORABILITY,
        SemanticPropertyId::SUPPORT_EDIT_CAPABILITY,
    ] {
        assert!(document.properties.contains_key(&property));
    }

    let customer = node(&envelope, SemanticObjectKind::Attribute, "Customer");
    assert_value(
        customer,
        SemanticPropertyId::FIELD_REQUIRED,
        PropertyValue::Boolean(true),
    );
    assert_value(
        customer,
        SemanticPropertyId::FIELD_FILL_CHECKING,
        PropertyValue::EnumSymbol(SemanticEnumValue::SHOW_ERROR),
    );
    assert_value(
        customer,
        SemanticPropertyId::FIELD_INDEXING,
        PropertyValue::EnumSymbol(SemanticEnumValue::INDEX),
    );
    assert_value(
        customer,
        SemanticPropertyId::FIELD_MULTI_LINE,
        PropertyValue::Boolean(true),
    );
    assert_value(
        customer,
        SemanticPropertyId::FIELD_FILL_VALUE,
        PropertyValue::String("Retail".to_string()),
    );
    let type_set = match customer.properties[&SemanticPropertyId::FIELD_TYPE]
        .value()
        .unwrap()
    {
        PropertyValue::TypeSet(value) => value,
        value => panic!("expected typeSet, got {value:?}"),
    };
    assert_eq!(type_set.variants().len(), 2);

    assert_relation(
        &envelope,
        SemanticRelationId::BASED_ON,
        SemanticObjectKind::Document,
        "Order",
    );
    assert_relation(
        &envelope,
        SemanticRelationId::REGISTER_RECORDS,
        SemanticObjectKind::AccumulationRegister,
        "Stock",
    );
    assert_relation(
        &envelope,
        SemanticRelationId::ATTRIBUTES,
        SemanticObjectKind::Attribute,
        "Customer",
    );
    assert_relation(
        &envelope,
        SemanticRelationId::TABULAR_SECTIONS,
        SemanticObjectKind::TabularSection,
        "Lines",
    );
    assert_relation(
        &envelope,
        SemanticRelationId::COLUMNS,
        SemanticObjectKind::Attribute,
        "Item",
    );
    assert_relation(
        &envelope,
        SemanticRelationId::COMMANDS,
        SemanticObjectKind::Command,
        "Post",
    );
}

#[test]
fn specialized_legacy_areas_keep_their_properties_and_child_roles() {
    let catalog = read_fixture(
        "catalog",
        "Catalog",
        "Customers",
        "<Name>Customers</Name><HierarchyType>HierarchyFoldersAndItems</HierarchyType><LevelCount>3</LevelCount><CodeLength>9</CodeLength><DescriptionLength>50</DescriptionLength>",
        "",
    );
    let catalog_node = node(&catalog, SemanticObjectKind::Catalog, "Customers");
    assert_value(
        catalog_node,
        SemanticPropertyId::CATALOG_HIERARCHY_TYPE,
        PropertyValue::EnumSymbol(SemanticEnumValue::HIERARCHY_OF_GROUPS_AND_ITEMS),
    );
    assert_value(
        catalog_node,
        SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT,
        PropertyValue::Integer(3),
    );
    assert_value(
        catalog_node,
        SemanticPropertyId::CATALOG_CODE_LENGTH,
        PropertyValue::Integer(9),
    );
    assert_value(
        catalog_node,
        SemanticPropertyId::CATALOG_DESCRIPTION_LENGTH,
        PropertyValue::Integer(50),
    );

    let register = read_fixture(
        "register",
        "InformationRegister",
        "Prices",
        "<Name>Prices</Name><InformationRegisterPeriodicity>Day</InformationRegisterPeriodicity><WriteMode>Independent</WriteMode>",
        r#"
<Dimension uuid="22222222-2222-2222-2222-222222222222"><Properties><Name>Item</Name><Type><v8:Type>cfg:CatalogRef.Items</v8:Type></Type><Master>true</Master><MainFilter>true</MainFilter><FillChecking>ShowError</FillChecking></Properties></Dimension>
<Resource uuid="33333333-3333-3333-3333-333333333333"><Properties><Name>Price</Name><Type><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers><v8:Digits>15</v8:Digits><v8:FractionDigits>2</v8:FractionDigits><v8:AllowedSign>Nonnegative</v8:AllowedSign></v8:NumberQualifiers></Type><FillChecking>DontCheck</FillChecking></Properties></Resource>
"#,
    );
    let register_node = node(
        &register,
        SemanticObjectKind::InformationRegister,
        "Prices",
    );
    assert_value(
        register_node,
        SemanticPropertyId::REGISTER_PERIODICITY,
        PropertyValue::EnumSymbol(SemanticEnumValue::DAY),
    );
    assert_value(
        register_node,
        SemanticPropertyId::REGISTER_WRITE_MODE,
        PropertyValue::EnumSymbol(SemanticEnumValue::INDEPENDENT),
    );
    assert_relation(
        &register,
        SemanticRelationId::DIMENSIONS,
        SemanticObjectKind::Dimension,
        "Item",
    );
    assert_relation(
        &register,
        SemanticRelationId::RESOURCES,
        SemanticObjectKind::Resource,
        "Price",
    );

    let constant = read_fixture(
        "constant",
        "Constant",
        "DefaultWarehouse",
        "<Name>DefaultWarehouse</Name><Type><v8:Type>cfg:CatalogRef.Warehouses</v8:Type></Type>",
        "",
    );
    assert_type_set(
        node(
            &constant,
            SemanticObjectKind::Constant,
            "DefaultWarehouse",
        ),
        SemanticPropertyId::CONSTANT_VALUE_TYPE,
        1,
    );
    let defined_type = read_fixture(
        "defined-type",
        "DefinedType",
        "CustomerOrText",
        "<Name>CustomerOrText</Name><Type><v8:Type>xs:string</v8:Type><v8:Type>cfg:CatalogRef.Customers</v8:Type></Type>",
        "",
    );
    assert_type_set(
        node(
            &defined_type,
            SemanticObjectKind::DefinedType,
            "CustomerOrText",
        ),
        SemanticPropertyId::DEFINED_TYPE,
        2,
    );
    let report = read_fixture(
        "report",
        "Report",
        "Sales",
        "<Name>Sales</Name><MainDataCompositionSchema>Report.Sales.Template.MainSchema</MainDataCompositionSchema>",
        "",
    );
    assert_value(
        node(&report, SemanticObjectKind::Report, "Sales"),
        SemanticPropertyId::REPORT_MAIN_DATA_COMPOSITION_SCHEMA,
        PropertyValue::String("Report.Sales.Template.MainSchema".to_string()),
    );

    let module = read_fixture(
        "module",
        "CommonModule",
        "SalesServer",
        "<Name>SalesServer</Name><Global>true</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>true</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>true</Privileged><ReturnValuesReuse>DuringSession</ReturnValuesReuse>",
        "",
    );
    let module_node = node(&module, SemanticObjectKind::CommonModule, "SalesServer");
    for property in [
        SemanticPropertyId::MODULE_GLOBAL,
        SemanticPropertyId::MODULE_CLIENT_MANAGED_APPLICATION,
        SemanticPropertyId::MODULE_SERVER,
        SemanticPropertyId::MODULE_EXTERNAL_CONNECTION,
        SemanticPropertyId::MODULE_CLIENT_ORDINARY_APPLICATION,
        SemanticPropertyId::MODULE_SERVER_CALL,
        SemanticPropertyId::MODULE_PRIVILEGED,
        SemanticPropertyId::MODULE_RETURN_VALUES_REUSE,
    ] {
        assert!(
            module_node.properties[&property].value().is_some(),
            "missing common-module legacy property {property}"
        );
    }

    let job = read_fixture(
        "job",
        "ScheduledJob",
        "Refresh",
        "<Name>Refresh</Name><MethodName>CommonModule.Jobs.Refresh</MethodName><Use>true</Use><Predefined>true</Predefined><RestartCountOnFailure>3</RestartCountOnFailure><RestartIntervalOnFailure>60</RestartIntervalOnFailure>",
        "",
    );
    let job_node = node(&job, SemanticObjectKind::ScheduledJob, "Refresh");
    assert_value(
        job_node,
        SemanticPropertyId::JOB_METHOD,
        PropertyValue::String("CommonModule.Jobs.Refresh".to_string()),
    );
    assert_value(
        job_node,
        SemanticPropertyId::JOB_RESTART_COUNT,
        PropertyValue::Integer(3),
    );
    assert_value(
        job_node,
        SemanticPropertyId::JOB_RESTART_INTERVAL,
        PropertyValue::Integer(60),
    );

    let subscription = read_fixture(
        "subscription",
        "EventSubscription",
        "OnWrite",
        "<Name>OnWrite</Name><Event>BeforeWrite</Event><Handler>CommonModule.Events.OnWrite</Handler><Source><v8:Type>cfg:DocumentRef.Shipment</v8:Type><v8:Type>cfg:CatalogRef.Customers</v8:Type></Source>",
        "",
    );
    let subscription_node = node(
        &subscription,
        SemanticObjectKind::EventSubscription,
        "OnWrite",
    );
    assert_type_set(
        subscription_node,
        SemanticPropertyId::SUBSCRIPTION_SOURCE_TYPE,
        2,
    );

    let http = read_fixture(
        "http",
        "HTTPService",
        "Api",
        "<Name>Api</Name><RootURL>api/v1</RootURL>",
        r#"
<URLTemplate uuid="22222222-2222-2222-2222-222222222222">
  <Properties><Name>Orders</Name><Template>/orders/{id}</Template></Properties>
  <ChildObjects>
    <Method uuid="33333333-3333-3333-3333-333333333333"><Properties><Name>Get</Name><HTTPMethod>GET</HTTPMethod><Handler>CommonModule.Api.GetOrder</Handler></Properties></Method>
  </ChildObjects>
</URLTemplate>
"#,
    );
    assert_value(
        node(&http, SemanticObjectKind::HttpService, "Api"),
        SemanticPropertyId::HTTP_SERVICE_ROOT_URL,
        PropertyValue::String("api/v1".to_string()),
    );
    assert_relation(
        &http,
        SemanticRelationId::URL_TEMPLATES,
        SemanticObjectKind::HttpServiceUrlTemplate,
        "Orders",
    );
    assert_relation(
        &http,
        SemanticRelationId::METHODS,
        SemanticObjectKind::HttpServiceMethod,
        "Get",
    );

    let web = read_fixture(
        "web",
        "WebService",
        "Gateway",
        "<Name>Gateway</Name><Namespace>urn:gateway</Namespace>",
        r#"
<Operation uuid="22222222-2222-2222-2222-222222222222">
  <Properties><Name>Ping</Name><XDTOReturningValueType>xs:string</XDTOReturningValueType><Nillable>false</Nillable><Transactioned>true</Transactioned><ProcedureName>Ping</ProcedureName></Properties>
  <ChildObjects>
    <Parameter uuid="33333333-3333-3333-3333-333333333333"><Properties><Name>Text</Name><XDTOValueType>xs:string</XDTOValueType><Nillable>false</Nillable><TransferDirection>InOut</TransferDirection></Properties></Parameter>
  </ChildObjects>
</Operation>
"#,
    );
    assert_relation(
        &web,
        SemanticRelationId::OPERATIONS,
        SemanticObjectKind::WebServiceOperation,
        "Ping",
    );
    assert_relation(
        &web,
        SemanticRelationId::PARAMETERS,
        SemanticObjectKind::WebServiceParameter,
        "Text",
    );
    assert_type_set(
        node(
            &web,
            SemanticObjectKind::WebServiceOperation,
            "Ping",
        ),
        SemanticPropertyId::WEB_SERVICE_OPERATION_RETURN_TYPE,
        1,
    );

    let enumeration = read_fixture(
        "enum",
        "Enum",
        "Status",
        "<Name>Status</Name>",
        r#"<EnumValue uuid="22222222-2222-2222-2222-222222222222"><Properties><Name>Ready</Name><Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Ready</v8:content></v8:item></Synonym><Comment>Can be processed</Comment></Properties></EnumValue>"#,
    );
    assert_relation(
        &enumeration,
        SemanticRelationId::ENUM_VALUES,
        SemanticObjectKind::EnumerationValue,
        "Ready",
    );
    let enum_value = node(
        &enumeration,
        SemanticObjectKind::EnumerationValue,
        "Ready",
    );
    assert_value(
        enum_value,
        SemanticPropertyId::METADATA_SYNONYM,
        localized("en", "Ready"),
    );
    assert_value(
        enum_value,
        SemanticPropertyId::METADATA_COMMENT,
        PropertyValue::String("Can be processed".to_string()),
    );

    let children = read_fixture(
        "simple-children",
        "Report",
        "Inventory",
        "<Name>Inventory</Name>",
        r#"
<Form uuid="22222222-2222-2222-2222-222222222222">MainForm</Form>
<Template uuid="33333333-3333-3333-3333-333333333333">MainSchema</Template>
<Command uuid="44444444-4444-4444-4444-444444444444"><Properties><Name>Generate</Name></Properties></Command>
"#,
    );
    assert_eq!(children.status, NavigationStatus::Partial);
    assert_relation(
        &children,
        SemanticRelationId::FORMS,
        SemanticObjectKind::Form,
        "MainForm",
    );
    assert_relation(
        &children,
        SemanticRelationId::TEMPLATES,
        SemanticObjectKind::Template,
        "MainSchema",
    );
    assert_relation(
        &children,
        SemanticRelationId::COMMANDS,
        SemanticObjectKind::Command,
        "Generate",
    );
}

fn strings<'a>(manifest: &'a serde_json::Value, key: &str) -> Vec<&'a str> {
    manifest[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} array"))
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect()
}

fn read_fixture(
    label: &str,
    class: &str,
    name: &str,
    properties: &str,
    children: &str,
) -> NavigationEnvelope {
    let root = std::env::temp_dir().join(format!(
        "unica-platform-xml-task5-{label}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    ));
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let target = source_root.join(format!("{name}.xml"));
    let child_objects = if children.trim().is_empty() {
        String::new()
    } else {
        format!("<ChildObjects>{children}</ChildObjects>")
    };
    let xml = format!(
        r#"<MetaDataObject xmlns="{MD}" xmlns:v8="{V8}" xmlns:xs="{XS}" xmlns:xsi="{XSI}" xmlns:cfg="{CFG}" xmlns:xr="{XR}" version="2.20"><{class} uuid="11111111-1111-1111-1111-111111111111"><Properties>{properties}</Properties>{child_objects}</{class}></MetaDataObject>"#
    );
    fs::write(&target, xml).unwrap();
    let source = SourceContext::new(
        SourceLocation::new(root.clone(), source_root, target),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    );
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).unwrap() else {
        panic!("fixture must be captured");
    };
    let envelope = registration
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
        .unwrap();
    fs::remove_dir_all(root).unwrap();
    envelope
}

fn node<'a>(
    envelope: &'a NavigationEnvelope,
    kind: SemanticObjectKind,
    name: &str,
) -> &'a NavigationNode {
    envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.kind == kind && node.object_ref.display_name == name)
        .unwrap_or_else(|| panic!("missing {kind} node {name}"))
}

fn assert_value(node: &NavigationNode, id: SemanticPropertyId, expected: PropertyValue) {
    assert_eq!(
        node.properties
            .get(&id)
            .unwrap_or_else(|| panic!("missing property {id}"))
            .value(),
        Some(&expected),
        "unexpected value for {id}"
    );
}

fn assert_type_set(node: &NavigationNode, id: SemanticPropertyId, variants: usize) {
    let property = node
        .properties
        .get(&id)
        .unwrap_or_else(|| panic!("missing property {id}"));
    let PropertyValue::TypeSet(value) = property.value().expect("explicit type set") else {
        panic!("{id} is not a type set");
    };
    assert_eq!(value.variants().len(), variants, "{id}");
}

fn assert_relation(
    envelope: &NavigationEnvelope,
    role: SemanticRelationId,
    target_kind: SemanticObjectKind,
    target_name: &str,
) {
    assert!(
        envelope.relation_index.iter().any(|relation| {
            relation.role == role
                && relation.target.kind == target_kind
                && relation.target.display_name == target_name
        }),
        "missing {role} relation to {target_kind} {target_name}"
    );
}

fn localized(locale: &str, value: &str) -> PropertyValue {
    PropertyValue::LocalizedString(BTreeMap::from([(
        locale.to_string(),
        value.to_string(),
    )]))
}

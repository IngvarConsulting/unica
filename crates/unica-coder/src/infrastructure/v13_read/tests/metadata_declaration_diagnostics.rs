use super::*;

const CATALOG: &str = "main:Catalog.Items";
const ACCOUNTS: &str = "main:ChartOfAccounts.Accounts";
const CATALOG_XML: &str = "Catalogs/Items.xml";
const ACCOUNTS_XML: &str = "ChartsOfAccounts/Accounts.xml";
const ROOT_ATTRIBUTES: &str =
    "<StandardAttributes><xr:StandardAttribute name=\"Ref\"/></StandardAttributes>";

const CATALOG_ATTRIBUTE: (&str, &str, &str) =
    ("main:Catalog.Items.StandardAttribute.Ref", "/title", "Ref");
const ACCOUNTS_ATTRIBUTE: (&str, &str, &str) = (
    "main:ChartOfAccounts.Accounts.StandardAttribute.Ref",
    "/title",
    "Ref",
);
const CHARACTERISTIC: (&str, &str, &str) = (
    "main:Catalog.Items.Characteristic",
    "/items/0/values/source",
    "Catalog.Items",
);

struct FailedDeclaration {
    owner: &'static str,
    branch: &'static str,
    field: &'static str,
    cause: &'static str,
    sibling: (&'static str, &'static str, &'static str),
}

fn readable_declarations() -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    for (relative, anchor) in [
        (CATALOG_XML, "<Characteristics>"),
        (ACCOUNTS_XML, "<StandardTabularSections>"),
    ] {
        change_once(
            &fixture,
            relative,
            anchor,
            &format!("{ROOT_ATTRIBUTES}{anchor}"),
        );
    }
    fixture
}

fn change_once(fixture: &RealReaderFixture, relative: &str, from: &str, to: &str) {
    let path = fixture.source.join(relative);
    let xml = fs::read_to_string(&path).unwrap();
    assert_eq!(xml.matches(from).count(), 1, "fixture edit must be unique");
    write(&path, &xml.replacen(from, to, 1));
}

fn duplicate_container(fixture: &RealReaderFixture, relative: &str, name: &str) {
    let path = fixture.source.join(relative);
    let xml = fs::read_to_string(&path).unwrap();
    let opening = format!("<{name}>");
    let closing = format!("</{name}>");
    assert_eq!(xml.matches(&opening).count(), 1);
    let start = xml.find(&opening).unwrap();
    let end = xml[start..].find(&closing).unwrap() + start + closing.len();
    let container = &xml[start..end];
    change_once(
        fixture,
        relative,
        container,
        &format!("{container}{container}"),
    );
}

fn foreign_container(fixture: &RealReaderFixture, relative: &str, name: &str) {
    change_once(
        fixture,
        relative,
        &format!("<{name}>"),
        &format!("<foreign:{name} xmlns:foreign=\"urn:unica:foreign-declaration\">"),
    );
    change_once(
        fixture,
        relative,
        &format!("</{name}>"),
        &format!("</foreign:{name}>"),
    );
}

fn assert_failure(fixture: &RealReaderFixture, case: FailedDeclaration) {
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();

    let owner = service.view(ViewRequest::new(case.owner).unwrap());
    assert!(
        owner.ok,
        "owner must remain readable: {:?}",
        refusal_codes(&owner)
    );
    let node = owner.data.as_ref().unwrap();
    assert_eq!(node["at"], case.owner);
    assert!(
        node["limits"].as_array().is_some_and(|limits| {
            limits.iter().any(|limit| {
                limit.as_str().is_some_and(|limit| {
                    limit.starts_with(&format!("{} не прочитано:", case.field))
                        && limit.contains(case.cause)
                })
            })
        }),
        "owner must name the original field and cause: {}",
        case.field
    );

    let dependent = service.view(ViewRequest::new(case.branch).unwrap());
    assert!(
        !dependent.ok,
        "an unreadable declaration must refuse its branch"
    );
    assert_eq!(dependent.at.as_deref(), Some(case.branch));
    assert_eq!(refusal_codes(&dependent), ["provider_unavailable"]);
    assert_eq!(dependent.diagnostics[0]["detailCode"], "source_unreadable");
    let message = dependent.diagnostics[0]["message"].as_str().unwrap();
    assert!(
        message.contains(&format!(" {}: ", case.field)),
        "dependent refusal lost the field"
    );
    assert!(
        message.contains(case.cause),
        "dependent refusal lost the cause"
    );
    assert!(dependent.data.is_none());
    assert!(dependent.cursor.is_none());

    let wire = serde_json::to_value(&dependent).unwrap();
    assert_eq!(wire["ok"], false);
    assert_eq!(wire["at"], case.branch);
    assert_eq!(wire["diagnostics"][0]["detailCode"], "source_unreadable");

    let (at, pointer, value) = case.sibling;
    let sibling = service.view(ViewRequest::new(at).unwrap());
    assert!(
        sibling.ok,
        "independent sibling must remain readable: {:?}",
        refusal_codes(&sibling)
    );
    assert_eq!(
        sibling.data.as_ref().unwrap().pointer(pointer),
        Some(&json!(value))
    );
    let catalog = service.view(ViewRequest::new(CATALOG).unwrap());
    assert!(catalog.ok, "independent catalog facts must remain readable");
    assert_eq!(catalog.data.as_ref().unwrap()["props"]["CodeLength"], 11);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn malformed_standard_attributes_keep_the_cause_and_the_characteristic_sibling() {
    let fixture = readable_declarations();
    change_once(&fixture, CATALOG_XML, "name=\"Ref\"", "name=\"\"");
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: CATALOG,
            branch: "main:Catalog.Items.StandardAttribute",
            field: "standardAttributes[0].name",
            cause: "no name",
            sibling: CHARACTERISTIC,
        },
    );
}

#[test]
fn malformed_characteristics_keep_the_cause_and_the_standard_attribute_sibling() {
    let fixture = readable_declarations();
    change_once(
        &fixture,
        CATALOG_XML,
        "from=\"Catalog.Items.TabularSection.Lines.Attribute.Quantity\"",
        "from=\"\"",
    );
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: CATALOG,
            branch: "main:Catalog.Items.Characteristic",
            field: "characteristics[0].types.source",
            cause: "source is missing",
            sibling: CATALOG_ATTRIBUTE,
        },
    );
}

#[test]
fn malformed_standard_tabular_sections_keep_the_root_attribute_sibling() {
    let fixture = readable_declarations();
    change_once(
        &fixture,
        ACCOUNTS_XML,
        "name=\"ExtDimensionTypes\"",
        "name=\"\"",
    );
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: ACCOUNTS,
            branch: "main:ChartOfAccounts.Accounts.StandardTabularSection",
            field: "standardTabularSections[0].name",
            cause: "no name",
            sibling: ACCOUNTS_ATTRIBUTE,
        },
    );
}

#[test]
fn nested_standard_attributes_keep_their_standard_tabular_section_context() {
    let fixture = readable_declarations();
    change_once(
        &fixture,
        ACCOUNTS_XML,
        "name=\"ExtDimensionType\"",
        "name=\"\"",
    );
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: ACCOUNTS,
            branch: "main:ChartOfAccounts.Accounts.StandardTabularSection",
            field: "standardTabularSections[0].standardAttributes[0].name",
            cause: "no name",
            sibling: ACCOUNTS_ATTRIBUTE,
        },
    );
    let reader = fixture.read_authority();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "ChartOfAccounts.Accounts")
            .unwrap();
    let local = reader.read.metadata_local(&target, &mut || Ok(())).unwrap();
    let diagnostic = local
        .info
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.field.as_deref()
                == Some("standardTabularSections[0].standardAttributes[0].name")
        })
        .expect("nested producer diagnostic keeps its owner context");
    assert_eq!(
        diagnostic.code,
        unica_metadata::MetaDiagnosticCode::ValidationFailed
    );
    assert_eq!(
        diagnostic.severity,
        unica_metadata::MetaDiagnosticSeverity::Error
    );
    assert_eq!(diagnostic.metadata_path.as_ref(), Some(&target));
    assert_eq!(diagnostic.message, "standard attribute has no name");
}

#[test]
fn duplicate_standard_attributes_refuse_only_their_dependent_branch() {
    let fixture = readable_declarations();
    duplicate_container(&fixture, CATALOG_XML, "StandardAttributes");
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: CATALOG,
            branch: "main:Catalog.Items.StandardAttribute",
            field: "properties.StandardAttributes",
            cause: "duplicate property",
            sibling: CHARACTERISTIC,
        },
    );
}

#[test]
fn duplicate_characteristics_refuse_only_their_dependent_branch() {
    let fixture = readable_declarations();
    duplicate_container(&fixture, CATALOG_XML, "Characteristics");
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: CATALOG,
            branch: "main:Catalog.Items.Characteristic",
            field: "properties.Characteristics",
            cause: "duplicate property",
            sibling: CATALOG_ATTRIBUTE,
        },
    );
}

#[test]
fn duplicate_standard_tabular_sections_refuse_only_their_dependent_branch() {
    let fixture = readable_declarations();
    duplicate_container(&fixture, ACCOUNTS_XML, "StandardTabularSections");
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: ACCOUNTS,
            branch: "main:ChartOfAccounts.Accounts.StandardTabularSection",
            field: "properties.StandardTabularSections",
            cause: "duplicate property",
            sibling: ACCOUNTS_ATTRIBUTE,
        },
    );
}

#[test]
fn foreign_standard_attributes_refuse_only_their_dependent_branch() {
    let fixture = readable_declarations();
    foreign_container(&fixture, CATALOG_XML, "StandardAttributes");
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: CATALOG,
            branch: "main:Catalog.Items.StandardAttribute",
            field: "properties.StandardAttributes",
            cause: "foreign-namespace property",
            sibling: CHARACTERISTIC,
        },
    );
}

#[test]
fn foreign_characteristics_refuse_only_their_dependent_branch() {
    let fixture = readable_declarations();
    foreign_container(&fixture, CATALOG_XML, "Characteristics");
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: CATALOG,
            branch: "main:Catalog.Items.Characteristic",
            field: "properties.Characteristics",
            cause: "foreign-namespace property",
            sibling: CATALOG_ATTRIBUTE,
        },
    );
}

#[test]
fn foreign_standard_tabular_sections_refuse_only_their_dependent_branch() {
    let fixture = readable_declarations();
    foreign_container(&fixture, ACCOUNTS_XML, "StandardTabularSections");
    assert_failure(
        &fixture,
        FailedDeclaration {
            owner: ACCOUNTS,
            branch: "main:ChartOfAccounts.Accounts.StandardTabularSection",
            field: "properties.StandardTabularSections",
            cause: "foreign-namespace property",
            sibling: ACCOUNTS_ATTRIBUTE,
        },
    );
}

#[test]
fn similarly_named_property_diagnostics_do_not_poison_readable_declarations() {
    for (relative, anchor, near_name, owner, branch, sibling) in [
        (
            CATALOG_XML,
            "<StandardAttributes>",
            "StandardAttributesExtra",
            CATALOG,
            "main:Catalog.Items.StandardAttribute",
            CATALOG_ATTRIBUTE,
        ),
        (
            CATALOG_XML,
            "<Characteristics>",
            "CharacteristicsExtra",
            CATALOG,
            "main:Catalog.Items.Characteristic",
            CHARACTERISTIC,
        ),
        (
            ACCOUNTS_XML,
            "<StandardTabularSections>",
            "StandardTabularSectionsExtra",
            ACCOUNTS,
            "main:ChartOfAccounts.Accounts.StandardTabularSection",
            ACCOUNTS_ATTRIBUTE,
        ),
    ] {
        let fixture = readable_declarations();
        change_once(
            &fixture,
            relative,
            anchor,
            &format!("<foreign:{near_name} xmlns:foreign=\"urn:unica:similar-property\"/>{anchor}"),
        );
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        let root = service.view(ViewRequest::new(owner).unwrap());
        assert!(root.ok);
        assert!(root.data.as_ref().unwrap()["limits"]
            .as_array()
            .is_some_and(|limits| {
                limits.iter().any(|limit| {
                    limit
                        .as_str()
                        .is_some_and(|limit| limit.contains(&format!("properties.{near_name}")))
                })
            }));
        let collection = service.view(ViewRequest::new(branch).unwrap());
        assert!(
            collection.ok,
            "a similarly named field must not block this declaration"
        );
        assert_eq!(
            collection.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let (at, pointer, expected) = sibling;
        let detail = service.view(ViewRequest::new(at).unwrap());
        assert!(detail.ok);
        assert_eq!(
            detail.data.as_ref().unwrap().pointer(pointer),
            Some(&json!(expected))
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn valid_declarations_preserve_items_details_and_readonly_sources() {
    let fixture = readable_declarations();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    for (owner, branch) in [
        (CATALOG, "main:Catalog.Items.StandardAttribute"),
        (CATALOG, "main:Catalog.Items.Characteristic"),
        (
            ACCOUNTS,
            "main:ChartOfAccounts.Accounts.StandardTabularSection",
        ),
    ] {
        let root = service.view(ViewRequest::new(owner).unwrap());
        assert!(root.ok);
        assert!(root.data.as_ref().unwrap()["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at": branch, "count": 1})));
        let collection = service.view(ViewRequest::new(branch).unwrap());
        assert!(collection.ok);
        assert_eq!(
            collection.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    for (at, pointer, expected) in [CATALOG_ATTRIBUTE, ACCOUNTS_ATTRIBUTE, CHARACTERISTIC] {
        let detail = service.view(ViewRequest::new(at).unwrap());
        assert!(detail.ok);
        assert_eq!(
            detail.data.as_ref().unwrap().pointer(pointer),
            Some(&json!(expected))
        );
    }
    let nested = service.view(ViewRequest::new(
        "main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes.StandardAttribute.ExtDimensionType"
    ).unwrap());
    assert!(nested.ok);
    assert_eq!(nested.data.as_ref().unwrap()["title"], "ExtDimensionType");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn projection_matches_canonical_segments_and_exact_xml_property_aliases() {
    // Routing alone uses a synthetic payload; the XML cases cover the producer.
    for (branch, canonical, xml_name) in [
        (
            "main:Catalog.Items.StandardAttribute",
            "standardAttributes",
            "StandardAttributes",
        ),
        (
            "main:Catalog.Items.Characteristic",
            "characteristics",
            "Characteristics",
        ),
        (
            "main:ChartOfAccounts.Accounts.StandardTabularSection",
            "standardTabularSections",
            "StandardTabularSections",
        ),
    ] {
        let address = QualifiedAddress::parse(branch).unwrap();
        let route = route_logical_address(&address, PlatformProfile::v8_3_27()).unwrap();
        for (field, should_refuse) in [
            (canonical.to_string(), true),
            (format!("{canonical}.name"), true),
            (format!("{canonical}[0].name"), true),
            (format!("properties.{xml_name}"), true),
            (format!("{canonical}Extra"), false),
            (format!("{canonical}Extra[0].name"), false),
            (format!("properties.{xml_name}Extra"), false),
            (format!("properties.{xml_name}.name"), false),
            (format!("properties.{xml_name}[0]"), false),
        ] {
            let payload = json!({
                "name": "RoutingFixture",
                "collections": {},
                "declarations": {
                    "standardAttributes": null,
                    "characteristics": null,
                    "standardTabularSections": null
                },
                "unreadable": [{"field": field, "message": "routing witness"}]
            });
            let result = project_typed_payload(&route, payload);
            if should_refuse {
                let error = result.expect_err("matching field must refuse this branch");
                assert_eq!(
                    error.detail(),
                    Some(unica_refusal::RefusalDetail::SourceUnreadable)
                );
                assert!(error.to_string().contains(&field));
            } else {
                assert!(
                    result.is_ok(),
                    "a nonmatching field must not refuse this branch: {field}"
                );
            }
        }
    }
}

enum UnexpectedDeclarationContainer {
    Metadata,
    Foreign,
}

fn assert_web_service_declaration_stays_inapplicable(
    xml_name: &str,
    branch_kind: &str,
    container: UnexpectedDeclarationContainer,
) {
    let fixture = RealReaderFixture::new();
    let owner_at = "main:WebService.Exchange";
    let branch_at = format!("{owner_at}.{branch_kind}");
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "WebService.Exchange").unwrap();
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    {
        let service = fixture.view_service();
        let owner = service.view(ViewRequest::new(owner_at).unwrap());
        assert!(owner.ok, "valid WebService fixture must be readable");
        assert_eq!(
            owner.data.as_ref().unwrap()["props"]["Namespace"],
            "urn:example:service"
        );
        let absent = service.view(ViewRequest::new(&branch_at).unwrap());
        assert!(!absent.ok, "this declaration does not apply to WebService");
        assert_eq!(refusal_codes(&absent), ["not_found"]);
        let reader = fixture.read_authority();
        let local = reader.read.metadata_local(&target, &mut || Ok(())).unwrap();
        assert_eq!(
            serde_json::to_value(&local.info.declarations).unwrap(),
            json!({})
        );
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        original
    );

    let (fragment, cause) = match container {
        UnexpectedDeclarationContainer::Metadata => {
            (format!("<{xml_name}/>"), "unclassified property")
        }
        UnexpectedDeclarationContainer::Foreign => (
            format!("<foreign:{xml_name} xmlns:foreign=\"urn:unica:inapplicable-declaration\"/>"),
            "foreign-namespace property",
        ),
    };
    change_once(
        &fixture,
        "WebServices/Exchange.xml",
        "<Name>Exchange</Name>",
        &format!("<Name>Exchange</Name>{fragment}"),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new(owner_at).unwrap());
    assert!(
        owner.ok,
        "an unrelated unsupported property must not erase the owner"
    );
    let node = owner.data.as_ref().unwrap();
    assert_eq!(node["props"]["Namespace"], "urn:example:service");
    assert!(
        node["limits"].as_array().is_some_and(|limits| {
            limits.iter().any(|limit| {
                limit.as_str().is_some_and(|limit| {
                    limit.starts_with(&format!("properties.{xml_name} не прочитано:"))
                        && limit.contains(cause)
                })
            })
        }),
        "the unsupported property must remain visible on its owner"
    );
    assert!(!node["branches"]
        .as_array()
        .is_some_and(|branches| { branches.iter().any(|branch| branch["at"] == branch_at) }));

    let dependent = service.view(ViewRequest::new(&branch_at).unwrap());
    assert!(
        !dependent.ok,
        "an inapplicable declaration must not become a collection"
    );
    assert_eq!(dependent.at.as_deref(), Some(branch_at.as_str()));
    assert_eq!(
        refusal_codes(&dependent),
        ["not_found"],
        "an unrelated property cannot make an inapplicable declaration unreadable"
    );
    assert!(dependent.data.is_none());
    assert!(dependent.cursor.is_none());

    let operation =
        service.view(ViewRequest::new("main:WebService.Exchange.Operation.Send").unwrap());
    assert!(operation.ok, "the independent operation remains readable");
    assert_eq!(
        operation.data.as_ref().unwrap()["props"]["procedure"],
        "SendData"
    );
    let parameter = service.view(
        ViewRequest::new("main:WebService.Exchange.Operation.Send.Parameter.Payload").unwrap(),
    );
    assert!(parameter.ok, "the independent parameter remains readable");
    assert_eq!(parameter.data.as_ref().unwrap()["title"], "Payload");
    let reader = fixture.read_authority();
    let local = reader.read.metadata_local(&target, &mut || Ok(())).unwrap();
    assert_eq!(
        serde_json::to_value(&local.info.declarations).unwrap(),
        json!({})
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn inapplicable_web_service_standard_attributes_ignore_a_metadata_container_alias() {
    assert_web_service_declaration_stays_inapplicable(
        "StandardAttributes",
        "StandardAttribute",
        UnexpectedDeclarationContainer::Metadata,
    );
}

#[test]
fn inapplicable_web_service_characteristics_ignore_a_metadata_container_alias() {
    assert_web_service_declaration_stays_inapplicable(
        "Characteristics",
        "Characteristic",
        UnexpectedDeclarationContainer::Metadata,
    );
}

#[test]
fn inapplicable_web_service_standard_tabular_sections_ignore_a_metadata_container_alias() {
    assert_web_service_declaration_stays_inapplicable(
        "StandardTabularSections",
        "StandardTabularSection",
        UnexpectedDeclarationContainer::Metadata,
    );
}

#[test]
fn inapplicable_web_service_standard_attributes_ignore_a_foreign_container_alias() {
    assert_web_service_declaration_stays_inapplicable(
        "StandardAttributes",
        "StandardAttribute",
        UnexpectedDeclarationContainer::Foreign,
    );
}

#[test]
fn inapplicable_web_service_characteristics_ignore_a_foreign_container_alias() {
    assert_web_service_declaration_stays_inapplicable(
        "Characteristics",
        "Characteristic",
        UnexpectedDeclarationContainer::Foreign,
    );
}

#[test]
fn inapplicable_web_service_standard_tabular_sections_ignore_a_foreign_container_alias() {
    assert_web_service_declaration_stays_inapplicable(
        "StandardTabularSections",
        "StandardTabularSection",
        UnexpectedDeclarationContainer::Foreign,
    );
}

use super::super::{project_typed_payload, project_typed_payload_for_source};
use crate::infrastructure::logical_tree::route_logical_address;
use serde_json::{json, Value};
use unica_address::address::QualifiedAddress;
use unica_bsl::profile::PlatformProfile;
use unica_source_contract::SourceSetKind;

mod catalog_attribute;

fn observed_properties() -> Value {
    json!([
        {"name": "LineNumberLength", "value": {"kind": "unsignedInteger", "value": 5}},
        {"name": "Use", "value": {"kind": "text", "value": "ForItem"}}
    ])
}

fn provider_payload(properties: Value) -> Value {
    json!({
        "kind": "Catalog",
        "name": "Items",
        "memberReadProfile": "catalogTabularSection220",
        "collections": {
            "attributes": [{"name": "Header", "properties": properties.clone()}],
            "tabularSections": [{
                "name": "Lines",
                "required": true,
                "properties": properties.clone(),
                "attributes": [{"name": "Column", "properties": properties}]
            }]
        }
    })
}

fn project(at: &str, payload: Value, source_kind: Option<SourceSetKind>) -> Value {
    let address = QualifiedAddress::parse(at).unwrap();
    let route = route_logical_address(&address, PlatformProfile::v8_3_27()).unwrap();
    let projected = match source_kind {
        Some(kind) => project_typed_payload_for_source(&route, payload, kind),
        None => project_typed_payload(&route, payload),
    }
    .unwrap_or_else(|error| panic!("{at}: {error:?}"));
    serde_json::to_value(projected).unwrap()
}

fn detail(properties: Value) -> Value {
    project(
        "main:Catalog.Items.TabularSection.Lines",
        provider_payload(properties),
        Some(SourceSetKind::Configuration),
    )
}

fn assert_profile_properties_absent(node: &Value) {
    for name in ["LineNumberLength", "Use"] {
        assert!(
            node.get("props")
                .and_then(|props| props.get(name))
                .is_none(),
            "unproved profile property {name} reached public props: {node}"
        );
    }
}

#[test]
fn catalog_tabular_provider_profile_transfers_same_typed_facts_to_collection_and_detail() {
    for usage in ["ForItem", "ForFolder", "ForFolderAndItem"] {
        let properties = json!([
            {"name": "LineNumberLength", "value": {"kind": "unsignedInteger", "value": 5}},
            {"name": "Use", "value": {"kind": "text", "value": usage}}
        ]);
        let payload = provider_payload(properties);
        let collection = project(
            "main:Catalog.Items.TabularSection",
            payload.clone(),
            Some(SourceSetKind::Configuration),
        );
        let item = &collection["items"][0];
        let at = item["at"].as_str().unwrap();
        assert_eq!(at, "main:Catalog.Items.TabularSection.Lines");
        let detail = project(at, payload, Some(SourceSetKind::Configuration));
        for node in [item, &detail] {
            assert_eq!(node["props"]["LineNumberLength"], json!(5));
            assert!(node["props"]["LineNumberLength"].is_number());
            assert_eq!(node["props"]["Use"], json!(usage));
            assert_eq!(node["props"]["required"], json!(true));
        }
        assert_eq!(item["props"], detail["props"]);
    }
}

#[test]
fn catalog_tabular_provider_profile_requires_marker_and_admitted_configuration() {
    let payload = provider_payload(observed_properties());
    for source_kind in [
        None,
        Some(SourceSetKind::Extension),
        Some(SourceSetKind::ExternalProcessor),
        Some(SourceSetKind::ExternalReport),
    ] {
        let node = project(
            "main:Catalog.Items.TabularSection.Lines",
            payload.clone(),
            source_kind,
        );
        assert_profile_properties_absent(&node);
        assert_eq!(node["props"]["required"], json!(true));
    }
    for marker in [
        None,
        Some(json!("futureProfile")),
        Some(json!({"kind": "catalogTabularSection220"})),
    ] {
        let mut payload = payload.clone();
        if let Some(marker) = marker {
            payload["memberReadProfile"] = marker;
        } else {
            payload.as_object_mut().unwrap().remove("memberReadProfile");
        }
        let node = project(
            "main:Catalog.Items.TabularSection.Lines",
            payload,
            Some(SourceSetKind::Configuration),
        );
        assert_profile_properties_absent(&node);
    }
}

#[test]
fn catalog_tabular_provider_profile_does_not_escape_to_other_owners_or_nested_attributes() {
    let mut document = provider_payload(observed_properties());
    document["kind"] = json!("Document");
    let node = project(
        "main:Document.Items.TabularSection.Lines",
        document,
        Some(SourceSetKind::Configuration),
    );
    assert_profile_properties_absent(&node);

    for at in [
        "main:Catalog.Items.Attribute.Header",
        "main:Catalog.Items.TabularSection.Lines.Attribute.Column",
    ] {
        let node = project(
            at,
            provider_payload(observed_properties()),
            Some(SourceSetKind::Configuration),
        );
        assert_profile_properties_absent(&node);
    }
    for at in [
        "main:Catalog.Items.Attribute",
        "main:Catalog.Items.TabularSection.Lines.Attribute",
    ] {
        let collection = project(
            at,
            provider_payload(observed_properties()),
            Some(SourceSetKind::Configuration),
        );
        assert_profile_properties_absent(&collection["items"][0]);
    }
}

#[test]
fn catalog_tabular_provider_profile_rejects_owner_kind_inconsistent_with_route() {
    let mut payload = provider_payload(observed_properties());
    payload["kind"] = json!("Document");
    let node = project(
        "main:Catalog.Items.TabularSection.Lines",
        payload,
        Some(SourceSetKind::Configuration),
    );
    assert_profile_properties_absent(&node);
    assert_eq!(node["props"]["required"], json!(true));
}

#[test]
fn catalog_tabular_provider_profile_rejects_untyped_or_invalid_values_locally() {
    for invalid in [
        json!({"kind": "text", "value": "5"}),
        json!({"kind": "unsignedInteger", "value": "5"}),
        json!({"kind": "unsignedInteger", "value": -1}),
        json!({"kind": "unsignedInteger", "value": 4294967296_u64}),
        json!({"kind": "unsignedInteger", "value": 5.5}),
        json!({"kind": "boolean", "value": true}),
        json!(5),
    ] {
        let node = detail(json!([
            {"name": "LineNumberLength", "value": invalid},
            {"name": "Use", "value": {"kind": "text", "value": "ForItem"}}
        ]));
        assert!(node["props"].get("LineNumberLength").is_none(), "{node}");
        assert_eq!(node["props"]["Use"], json!("ForItem"));
        assert_eq!(node["props"]["required"], json!(true));
        assert!(
            node["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| { limit.as_str().unwrap().contains("LineNumberLength") }),
            "invalid LineNumberLength must remain visible as a limit: {node}"
        );
    }
    for invalid in [
        json!({"kind": "text", "value": "Unknown"}),
        json!({"kind": "text", "value": "ForItem "}),
        json!({"kind": "text", "value": "forItem"}),
        json!({"kind": "unsignedInteger", "value": 5}),
        json!({"kind": "text", "value": "x".repeat(4096)}),
    ] {
        let node = detail(json!([
            {"name": "LineNumberLength", "value": {"kind": "unsignedInteger", "value": 5}},
            {"name": "Use", "value": invalid}
        ]));
        assert!(node["props"].get("Use").is_none(), "{node}");
        assert_eq!(node["props"]["LineNumberLength"], json!(5));
        assert!(
            node["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| { limit.as_str().unwrap().contains("Use") }),
            "invalid Use must remain visible as a limit: {node}"
        );
    }
}

#[test]
fn catalog_tabular_provider_profile_rejects_duplicates_without_hiding_independent_facts() {
    for duplicated in ["LineNumberLength", "Use"] {
        let mut properties = observed_properties();
        let duplicate = properties
            .as_array()
            .unwrap()
            .iter()
            .find(|property| property["name"] == duplicated)
            .unwrap()
            .clone();
        properties.as_array_mut().unwrap().push(duplicate);
        let node = detail(properties);
        assert!(node["props"].get(duplicated).is_none(), "{node}");
        let (independent, expected) = if duplicated == "Use" {
            ("LineNumberLength", json!(5))
        } else {
            ("Use", json!("ForItem"))
        };
        assert_eq!(node["props"][independent], expected);
        assert_eq!(node["props"]["required"], json!(true));
        assert!(
            node["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| { limit.as_str().unwrap().contains(duplicated) }),
            "duplicated fact must remain diagnostic: {node}"
        );
    }
}

#[test]
fn catalog_tabular_provider_profile_does_not_publish_unknown_names_or_raw_item_keys() {
    let mut properties = observed_properties();
    properties.as_array_mut().unwrap().extend([
        json!({"name": "FutureProperty", "value": {"kind": "text", "value": "secret".repeat(1024)}}),
        json!({"name": "LineNumberLengths", "value": {"kind": "unsignedInteger", "value": 42}}),
    ]);
    let node = detail(properties);
    assert_eq!(
        node["props"],
        json!({"required": true, "LineNumberLength": 5, "Use": "ForItem"})
    );
    assert!(!node.to_string().contains("secret"));
    let mut payload = provider_payload(json!([]));
    let item = &mut payload["collections"]["tabularSections"][0];
    item["LineNumberLength"] = json!(5);
    item["Use"] = json!("ForItem");
    let node = project(
        "main:Catalog.Items.TabularSection.Lines",
        payload,
        Some(SourceSetKind::Configuration),
    );
    assert_profile_properties_absent(&node);
    assert_eq!(node["props"]["required"], json!(true));
}

#[test]
fn nested_member_profiles_publish_nine_facts_on_collection_and_detail() {
    for (owner, kind, collection, profile, properties, expected) in [
        (
            "Document",
            "Command",
            "commands",
            "documentCommand220",
            json!([
                {"name":"Group","value":{"kind":"text","value":"NavigationPanelImportant"}},
                {"name":"ParameterUseMode","value":{"kind":"text","value":"Single"}},
                {"name":"ModifiesData","value":{"kind":"boolean","value":false}}
            ]),
            json!({"Group":"NavigationPanelImportant","ParameterUseMode":"Single","ModifiesData":false}),
        ),
        (
            "InformationRegister",
            "Dimension",
            "dimensions",
            "informationRegisterDimension220",
            json!([
                {"name":"Master","value":{"kind":"boolean","value":true}},
                {"name":"MainFilter","value":{"kind":"boolean","value":true}},
                {"name":"DenyIncompleteValues","value":{"kind":"boolean","value":true}},
                {"name":"TypeReductionMode","value":{"kind":"text","value":"TransformValues"}}
            ]),
            json!({"master":true,"mainFilter":true,"denyIncompleteValues":true,"TypeReductionMode":"TransformValues"}),
        ),
        (
            "AccumulationRegister",
            "Dimension",
            "dimensions",
            "accumulationRegisterDimension220",
            json!([
                {"name":"UseInTotals","value":{"kind":"boolean","value":false}}
            ]),
            json!({"UseInTotals":false}),
        ),
        (
            "DocumentJournal",
            "Column",
            "columns",
            "documentJournalColumn220",
            json!([
                {"name":"Indexing","value":{"kind":"text","value":"DontIndex"}}
            ]),
            json!({"Indexing":"DontIndex"}),
        ),
    ] {
        let payload = json!({"kind":owner,"name":"Owner","memberReadProfile":profile,"collections":{collection:[{"name":"Member","synonym":"Member title","required":false,"properties":properties}]}});
        let collection = project(
            &format!("main:{owner}.Owner.{kind}"),
            payload.clone(),
            Some(SourceSetKind::Configuration),
        );
        let item = &collection["items"][0];
        let detail = project(
            item["at"].as_str().unwrap(),
            payload,
            Some(SourceSetKind::Configuration),
        );
        for node in [item, &detail] {
            for (key, value) in expected.as_object().unwrap() {
                assert_eq!(&node["props"][key], value, "{owner}.{kind} {key}: {node}");
            }
            assert_eq!(node["props"]["required"], false);
            assert_eq!(node["props"]["synonym"], "Member title");
        }
        assert_eq!(item["props"], detail["props"]);
    }
}

#[test]
fn nested_dimension_projection_keeps_limits_and_independent_facts_for_invalid_booleans() {
    let payload = |properties: Value| json!({"kind":"InformationRegister","name":"Facts","memberReadProfile":"informationRegisterDimension220","collections":{"dimensions":[{"name":"Dimension","required":false,"properties":properties}]}});
    let at = "main:InformationRegister.Facts.Dimension.Dimension";
    for invalid in [
        json!({"kind":"text","value":"false"}),
        json!({"kind":"boolean","value":"false"}),
        json!({"kind":"boolean","value":0}),
        json!(false),
    ] {
        let node = project(
            at,
            payload(json!([
                {"name":"Master","value":invalid},
                {"name":"MainFilter","value":{"kind":"boolean","value":false}}
            ])),
            Some(SourceSetKind::Configuration),
        );
        assert!(node["props"].get("master").is_none(), "{node}");
        assert_eq!(node["props"]["mainFilter"], false, "{node}");
        assert_eq!(node["props"]["required"], false);
        assert!(
            node["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains("Master")),
            "{node}"
        );
    }
    let duplicate = json!({"name":"Master","value":{"kind":"boolean","value":false}});
    let node = project(
        at,
        payload(json!([duplicate, duplicate])),
        Some(SourceSetKind::Configuration),
    );
    assert!(node["props"].get("master").is_none());
    assert!(node["limits"]
        .as_array()
        .is_some_and(|limits| !limits.is_empty()));
    let node = project(at, payload(json!([])), Some(SourceSetKind::Configuration));
    for key in [
        "master",
        "mainFilter",
        "denyIncompleteValues",
        "TypeReductionMode",
    ] {
        assert!(node["props"].get(key).is_none());
    }
    assert!(node.get("limits").is_none());
}

#[test]
fn nested_member_projection_requires_matching_profile_owner_and_configuration() {
    let payload = json!({"kind":"Document","name":"Orders","memberReadProfile":"documentCommand220","collections":{"commands":[{"name":"Run","synonym":"Run title","properties":[{"name":"ModifiesData","value":{"kind":"boolean","value":false}}]}]}});
    let at = "main:Document.Orders.Command.Run";
    for source in [
        None,
        Some(SourceSetKind::Extension),
        Some(SourceSetKind::ExternalProcessor),
        Some(SourceSetKind::ExternalReport),
    ] {
        let node = project(at, payload.clone(), source);
        assert!(node["props"].get("ModifiesData").is_none(), "{node}");
        assert_eq!(node["props"]["synonym"], "Run title");
    }
    for marker in [
        json!(null),
        json!("catalogTabularSection220"),
        json!("informationRegisterDimension220"),
        json!("documentCommand219"),
    ] {
        let mut payload = payload.clone();
        payload["memberReadProfile"] = marker;
        let node = project(at, payload, Some(SourceSetKind::Configuration));
        assert!(node["props"].get("ModifiesData").is_none(), "{node}");
    }
    let mut wrong_owner = payload.clone();
    wrong_owner["kind"] = json!("Catalog");
    let node = project(at, wrong_owner, Some(SourceSetKind::Configuration));
    assert!(node["props"].get("ModifiesData").is_none());
    let node = project(
        "main:Catalog.Orders.Command.Run",
        payload,
        Some(SourceSetKind::Configuration),
    );
    assert!(node["props"].get("ModifiesData").is_none());
}

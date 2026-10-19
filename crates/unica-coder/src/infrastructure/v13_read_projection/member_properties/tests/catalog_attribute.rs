use super::project;
use crate::domain::project_sources::SourceSetKind;
use serde_json::{json, Value};

const ENUMS: [(&str, &str, &[&str]); 3] = [
    (
        "Indexing",
        "Index",
        &["DontIndex", "Index", "IndexWithAdditionalOrder"],
    ),
    ("FullTextSearch", "Use", &["Use", "DontUse"]),
    ("DataHistory", "Use", &["Use", "DontUse"]),
];
const ATTRIBUTE: &str = "main:Catalog.Items.TabularSection.Lines.Attribute.Column";

fn observations() -> Value {
    json!(ENUMS
        .iter()
        .map(|(name, expected, _)| {
            json!({"name": name, "value": {"kind": "text", "value": expected}})
        })
        .collect::<Vec<_>>())
}

fn payload(properties: Value) -> Value {
    json!({
        "kind": "Catalog", "name": "Items",
        // The native observer emits this assertion after proving the XML
        // version; a provider test does not independently prove XML 2.20.
        "memberReadProfile": "catalogTabularSection220",
        "collections": {
            "attributes": [{"name": "Header", "properties": properties.clone()}],
            "tabularSections": [{
                "name": "Lines", "properties": properties.clone(),
                "standardAttributes": [{"name": "LineNumber", "properties": properties.clone()}],
                "attributes": [{
                    "name": "Column", "synonym": "Language code", "required": false,
                    "type": {"variants": [{"kind": "string", "length": 10, "allowedLength": "variable"}]},
                    "properties": properties
                }]
            }]
        }
    })
}

fn attribute_views(payload: Value) -> [Value; 2] {
    let collection = project(
        "main:Catalog.Items.TabularSection.Lines.Attribute",
        payload.clone(),
        Some(SourceSetKind::Configuration),
    );
    let row = collection["items"][0].clone();
    assert_eq!(row["at"], ATTRIBUTE);
    let detail = project(ATTRIBUTE, payload, Some(SourceSetKind::Configuration));
    [row, detail]
}

fn assert_absent(node: &Value) {
    for name in ["Indexing", "FullTextSearch", "DataHistory"] {
        assert!(node["props"].get(name).is_none(), "unproved {name}: {node}");
    }
}

#[test]
fn catalog_attribute_provider_transfers_each_closed_enum_to_row_and_detail() {
    for (name, _, values) in ENUMS {
        for expected in values {
            let mut properties = observations();
            let property = properties
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|property| property["name"] == name)
                .unwrap();
            property["value"]["value"] = json!(expected);
            for node in attribute_views(payload(properties)) {
                assert_eq!(node["props"][name], json!(expected));
                assert_eq!(node["props"]["required"], false);
                assert_eq!(node["props"]["synonym"], "Language code");
                assert!(node["props"]["type"].is_string());
                for old_section_fact in ["LineNumberLength", "Use"] {
                    assert!(node["props"].get(old_section_fact).is_none());
                }
            }
        }
    }
}

#[test]
fn catalog_attribute_provider_absence_never_invents_values_or_limits() {
    for node in attribute_views(payload(json!([]))) {
        assert_absent(&node);
        assert!(node["limits"].as_array().is_none_or(Vec::is_empty));
        assert_eq!(node["props"]["required"], false);
    }
}

#[test]
fn catalog_attribute_provider_rejects_bad_or_duplicate_facts_locally() {
    for (name, expected, _) in ENUMS {
        for bad in [
            json!({"kind": "unsignedInteger", "value": 1}),
            json!({"kind": "boolean", "value": true}),
            json!({"kind": "text", "value": "Unknown"}),
            json!({"kind": "text", "value": ""}),
            json!({"kind": "text", "value": format!("{expected} ")}),
            json!({"kind": "text", "value": "x".repeat(4096)}),
            json!({"kind": "text", "value": true}),
            json!(expected),
            Value::Null,
        ] {
            let mut properties = observations();
            let property = properties
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|property| property["name"] == name)
                .unwrap();
            property["value"] = bad;
            assert_bad_fact_is_local(payload(properties), name);
        }
        let mut properties = observations();
        properties.as_array_mut().unwrap().push(json!({
            "name": name, "value": {"kind": "text", "value": expected}
        }));
        assert_bad_fact_is_local(payload(properties), name);
    }
}

fn assert_bad_fact_is_local(payload: Value, name: &str) {
    for node in attribute_views(payload) {
        assert!(node["props"].get(name).is_none(), "{node}");
        for (independent, expected, _) in ENUMS {
            if independent != name {
                assert_eq!(node["props"][independent], expected, "{node}");
            }
        }
        assert_eq!(node["props"]["required"], false);
        assert_eq!(node["props"]["synonym"], "Language code");
        assert!(node["props"]["type"].is_string());
        assert!(
            node["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().is_some_and(|limit| limit.contains(name))),
            "{node}"
        );
    }
}

#[test]
fn catalog_attribute_provider_requires_admitted_source_marker_and_owner() {
    for source_kind in [
        None,
        Some(SourceSetKind::Extension),
        Some(SourceSetKind::ExternalProcessor),
        Some(SourceSetKind::ExternalReport),
    ] {
        let node = project(ATTRIBUTE, payload(observations()), source_kind);
        assert_absent(&node);
        assert_eq!(node["props"]["required"], false);
    }
    for marker in [
        Value::Null,
        json!("futureProfile"),
        json!({"kind": "catalogTabularSection220"}),
    ] {
        let mut payload = payload(observations());
        payload["memberReadProfile"] = marker;
        let node = project(ATTRIBUTE, payload, Some(SourceSetKind::Configuration));
        assert_absent(&node);
    }
    let mut payload = payload(observations());
    payload["kind"] = json!("Document");
    let node = project(ATTRIBUTE, payload, Some(SourceSetKind::Configuration));
    assert_absent(&node);
}

#[test]
fn catalog_attribute_provider_keeps_contexts_closed_and_ignores_raw_item_keys() {
    for at in [
        "main:Catalog.Items",
        "main:Catalog.Items.Attribute.Header",
        "main:Catalog.Items.TabularSection.Lines",
        "main:Catalog.Items.TabularSection.Lines.StandardAttribute.LineNumber",
    ] {
        let node = project(
            at,
            payload(observations()),
            Some(SourceSetKind::Configuration),
        );
        assert_absent(&node);
    }
    let mut document = payload(observations());
    document["kind"] = json!("Document");
    let node = project(
        "main:Document.Items.TabularSection.Lines.Attribute.Column",
        document,
        Some(SourceSetKind::Configuration),
    );
    assert_absent(&node);

    let mut properties = observations();
    properties.as_array_mut().unwrap().extend([
        json!({"name": "FutureProperty", "value": {"kind": "text", "value": "secret".repeat(1024)}}),
        json!({"name": "Indexings", "value": {"kind": "text", "value": "Index"}}),
        json!({"name": "LineNumberLength", "value": {"kind": "unsignedInteger", "value": 5}}),
        json!({"name": "Use", "value": {"kind": "text", "value": "ForItem"}}),
    ]);
    for node in attribute_views(payload(properties)) {
        assert!(!node.to_string().contains("secret"));
        for unexpected in ["FutureProperty", "Indexings", "LineNumberLength", "Use"] {
            assert!(node["props"].get(unexpected).is_none(), "{node}");
        }
    }
    let mut payload = payload(json!([]));
    let attribute = &mut payload["collections"]["tabularSections"][0]["attributes"][0];
    for (name, expected, _) in ENUMS {
        attribute[name] = json!(expected);
    }
    for node in attribute_views(payload) {
        assert_absent(&node);
    }
}

fn tooltip_observations(value: Value) -> Value {
    let mut properties = observations();
    properties
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "ToolTip", "value": value}));
    properties
}

#[test]
fn catalog_attribute_provider_tooltip_keeps_the_tagged_localized_value_in_row_and_detail() {
    for value in [
        json!({"kind": "localizedString", "values": []}),
        json!({"kind": "localizedString", "values": [
            {"language": "ru", "content": " Подсказка & текст "},
            {"language": "en", "content": ""},
            {"language": " custom language ", "content": "Line 1\nLine 2"},
            {"language": "", "content": "Empty language is not inferred"}
        ]}),
    ] {
        for node in attribute_views(payload(tooltip_observations(value.clone()))) {
            let compact = node["props"]["ToolTip"].as_str().unwrap();
            assert_eq!(serde_json::from_str::<Value>(compact).unwrap(), value);
            assert!(compact.len() <= 2048);
            assert!(node["limits"].as_array().is_none_or(Vec::is_empty));
            for (name, expected, _) in ENUMS {
                assert_eq!(node["props"][name], expected);
            }
        }
    }
}

#[test]
fn catalog_attribute_provider_tooltip_rejects_wrong_shapes_and_duplicate_languages_locally() {
    for value in [
        json!({"kind": "text", "value": "flattened"}),
        json!({"kind": "empty"}),
        json!({"kind": "nil"}),
        json!({"kind": "localizedString", "values": null}),
        json!({"kind": "localizedString", "values": {"ru": "text"}}),
        json!({"kind": "localizedString", "values": [{"content": "text"}]}),
        json!({"kind": "localizedString", "values": [{"language": "ru", "content": false}]}),
        json!({"kind": "localizedString", "values": [
            {"language": "ru", "content": "first"},
            {"language": "ru", "content": "second"}
        ]}),
        json!("flattened"),
        Value::Null,
    ] {
        assert_bad_fact_is_local(payload(tooltip_observations(value)), "ToolTip");
    }
    let value = json!({"kind": "localizedString", "values": []});
    let mut properties = tooltip_observations(value.clone());
    properties
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "ToolTip", "value": value}));
    assert_bad_fact_is_local(payload(properties), "ToolTip");
}

#[test]
fn catalog_attribute_provider_tooltip_obeys_the_utf8_compact_property_boundary() {
    let empty = json!({"kind": "localizedString", "values": [{"language": "ru", "content": ""}]});
    let available = 2048 - serde_json::to_string(&empty).unwrap().len();
    let content = format!("{}{}", "я".repeat(available / 2), "x".repeat(available % 2));
    let value =
        json!({"kind": "localizedString", "values": [{"language": "ru", "content": content}]});
    for node in attribute_views(payload(tooltip_observations(value.clone()))) {
        let compact = node["props"]["ToolTip"].as_str().unwrap();
        assert_eq!(compact.len(), 2048);
        assert_eq!(serde_json::from_str::<Value>(compact).unwrap(), value);
    }
    let value = json!({"kind": "localizedString", "values": [{"language": "ru", "content": format!("{content}x")}]});
    assert_bad_fact_is_local(payload(tooltip_observations(value)), "ToolTip");
}

#[test]
fn catalog_attribute_provider_tooltip_stays_inside_its_profile_and_exact_context() {
    let value = json!({"kind": "localizedString", "values": []});
    let properties = tooltip_observations(value.clone());
    for source_kind in [
        None,
        Some(SourceSetKind::Extension),
        Some(SourceSetKind::ExternalProcessor),
        Some(SourceSetKind::ExternalReport),
    ] {
        let node = project(ATTRIBUTE, payload(properties.clone()), source_kind);
        assert!(node["props"].get("ToolTip").is_none(), "{node}");
    }
    for marker in [Value::Null, json!("futureProfile")] {
        let mut payload = payload(properties.clone());
        payload["memberReadProfile"] = marker;
        let node = project(ATTRIBUTE, payload, Some(SourceSetKind::Configuration));
        assert!(node["props"].get("ToolTip").is_none(), "{node}");
    }
    for at in [
        "main:Catalog.Items",
        "main:Catalog.Items.Attribute.Header",
        "main:Catalog.Items.TabularSection.Lines",
    ] {
        let node = project(
            at,
            payload(properties.clone()),
            Some(SourceSetKind::Configuration),
        );
        assert!(node["props"].get("ToolTip").is_none(), "{node}");
    }
    let mut document = payload(properties);
    document["kind"] = json!("Document");
    let node = project(
        "main:Document.Items.TabularSection.Lines.Attribute.Column",
        document,
        Some(SourceSetKind::Configuration),
    );
    assert!(node["props"].get("ToolTip").is_none(), "{node}");
    let mut payload = payload(json!([]));
    payload["collections"]["tabularSections"][0]["attributes"][0]["ToolTip"] = value;
    for node in attribute_views(payload) {
        assert!(node["props"].get("ToolTip").is_none(), "{node}");
        assert!(node["limits"].as_array().is_none_or(Vec::is_empty));
    }
}

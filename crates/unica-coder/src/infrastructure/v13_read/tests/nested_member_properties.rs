use super::{write, RealReaderFixture, ViewRequest};
use serde_json::{json, Value};
use std::path::Path;
use unica_workspace::test_support::tree_snapshot;

fn fixture() -> RealReaderFixture {
    fn copy(source: &Path, target: &Path) {
        std::fs::create_dir_all(target).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
            }
        }
    }
    let fixture = RealReaderFixture::new();
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-nested-member-properties/src"),
        &fixture.source,
    );
    fixture
}

fn read(fixture: &RealReaderFixture, at: &str) -> Value {
    let result = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert!(result.ok, "view failed at {at}: {result:?}");
    result.data.unwrap()
}

fn views(fixture: &RealReaderFixture, collection: &str, name: &str) -> [Value; 2] {
    let collection = read(fixture, collection);
    let row = collection["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["at"].as_str().unwrap().ends_with(&format!(".{name}")))
        .unwrap_or_else(|| panic!("missing {name}: {collection}"))
        .clone();
    let detail = read(fixture, row["at"].as_str().unwrap());
    [row, detail]
}

fn cases() -> Vec<(&'static str, &'static str, &'static str, Value)> {
    vec![
        (
            "Documents/Orders.xml",
            "main:Document.Orders.Command",
            "Probe",
            json!({"Group":"NavigationPanelImportant","ParameterUseMode":"Single","ModifiesData":false}),
        ),
        (
            "InformationRegisters/Facts.xml",
            "main:InformationRegister.Facts.Dimension",
            "StringDimension",
            json!({"master":true,"mainFilter":true,"denyIncompleteValues":true,"TypeReductionMode":"TransformValues"}),
        ),
        (
            "AccumulationRegisters/Movements.xml",
            "main:AccumulationRegister.Movements.Dimension",
            "Status",
            json!({"UseInTotals":false}),
        ),
        (
            "DocumentJournals/Entries.xml",
            "main:DocumentJournal.Entries.Column",
            "Status",
            json!({"Indexing":"DontIndex"}),
        ),
    ]
}

fn xml_name(public: &str) -> &str {
    match public {
        "master" => "Master",
        "mainFilter" => "MainFilter",
        "denyIncompleteValues" => "DenyIncompleteValues",
        name => name,
    }
}

fn replace_property(xml: &str, member: &str, property: &str, replacement: &str) -> String {
    let document = roxmltree::Document::parse(xml).unwrap();
    let namespace = "http://v8.1c.ru/8.3/MDClasses";
    let props = document
        .descendants()
        .find(|node| {
            node.has_tag_name((namespace, "Properties"))
                && node.children().any(|child| {
                    child.has_tag_name((namespace, "Name")) && child.text() == Some(member)
                })
        })
        .unwrap();
    let range = props
        .children()
        .find(|node| node.has_tag_name((namespace, property)))
        .unwrap()
        .range();
    format!(
        "{}{}{}",
        &xml[..range.start],
        replacement,
        &xml[range.end..]
    )
}

#[test]
fn nine_nested_facts_reach_public_collection_and_detail_without_writes() {
    let fixture = fixture();
    for (file, _, member, _) in cases() {
        let path = fixture.source.join(file);
        let xml = std::fs::read_to_string(&path).unwrap();
        write(&path, &replace_property(&xml, member, "Synonym", "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Observed member</v8:content></v8:item></Synonym>"));
    }
    let before = tree_snapshot(&fixture.source);
    for (_, collection, name, expected) in cases() {
        for node in views(&fixture, collection, name) {
            assert_eq!(node["props"]["synonym"], "Observed member");
            assert_eq!(node["title"], "Observed member");
            for (key, value) in expected.as_object().unwrap() {
                assert_eq!(&node["props"][key], value, "{collection}.{name}: {node}");
            }
        }
    }
    for (collection, name, expected) in [
        (
            "main:Document.Orders.Command",
            "Other",
            json!({"Group":"NavigationPanelOrdinary","ParameterUseMode":"Multiple","ModifiesData":true}),
        ),
        (
            "main:InformationRegister.Facts.Dimension",
            "RefDimension",
            json!({"master":false,"mainFilter":false,"denyIncompleteValues":false,"TypeReductionMode":"Deny"}),
        ),
        (
            "main:AccumulationRegister.Movements.Dimension",
            "StatusIncluded",
            json!({"UseInTotals":true}),
        ),
        (
            "main:DocumentJournal.Entries.Column",
            "Indexed",
            json!({"Indexing":"Index"}),
        ),
        (
            "main:DocumentJournal.Entries.Column",
            "Ordered",
            json!({"Indexing":"IndexWithAdditionalOrder"}),
        ),
    ] {
        for node in views(&fixture, collection, name) {
            for (key, value) in expected.as_object().unwrap() {
                assert_eq!(&node["props"][key], value, "{node}");
            }
        }
    }
    let dimension = read(
        &fixture,
        "main:InformationRegister.Facts.Dimension.StringDimension",
    );
    assert_eq!(dimension["props"]["required"], false);
    assert_eq!(dimension["props"]["type"]["variants"][0]["kind"], "string");
    assert_eq!(
        dimension["props"]["fillValue"],
        json!({"kind":"string","value":"seed"}),
        "known fill value must survive: {dimension}"
    );
    assert_eq!(dimension["props"]["type"]["variants"][0]["length"], 10);
    let command = read(&fixture, "main:Document.Orders.Command.Probe");
    assert!(
        command["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|branch| branch["at"].as_str().unwrap().contains(".Module")),
        "{command}"
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn absent_nested_facts_are_not_filled_from_defaults_or_siblings() {
    for (file, collection, member, expected) in cases() {
        for public in expected.as_object().unwrap().keys() {
            let fixture = fixture();
            let path = fixture.source.join(file);
            let xml = std::fs::read_to_string(&path).unwrap();
            write(&path, &replace_property(&xml, member, xml_name(public), ""));
            let before = tree_snapshot(&fixture.source);
            for node in views(&fixture, collection, member) {
                assert!(node["props"].get(public).is_none(), "{node}");
                for (key, value) in expected
                    .as_object()
                    .unwrap()
                    .iter()
                    .filter(|(key, _)| *key != public)
                {
                    assert_eq!(&node["props"][key], value);
                }
                assert!(
                    !node["limits"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|limit| limit.to_string().contains(xml_name(public))),
                    "{node}"
                );
            }
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

#[test]
fn damaged_nested_facts_preserve_members_and_independent_facts() {
    for (file, collection, member, expected) in cases() {
        for public in expected.as_object().unwrap().keys() {
            let key = xml_name(public);
            for replacement in [
                format!("<{key}>invalid</{key}>"),
                format!("<{key}/>"),
                format!("<{key}>false</{key}><{key}>true</{key}>"),
                format!("<{key} xmlns=\"urn:foreign\">false</{key}>"),
                format!("<{key}><child>false</child></{key}>"),
            ] {
                let fixture = fixture();
                let path = fixture.source.join(file);
                let xml = std::fs::read_to_string(&path).unwrap();
                write(&path, &replace_property(&xml, member, key, &replacement));
                let before = tree_snapshot(&fixture.source);
                for node in views(&fixture, collection, member) {
                    assert!(node["props"].get(public).is_none(), "{replacement}: {node}");
                    assert!(
                        node["limits"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|limit| limit.to_string().contains(key)),
                        "missing local limit: {node}"
                    );
                    for (other, value) in expected
                        .as_object()
                        .unwrap()
                        .iter()
                        .filter(|(other, _)| *other != public)
                    {
                        assert_eq!(&node["props"][other], value, "{replacement}: {node}");
                    }
                }
                let owner = collection.rsplit_once('.').unwrap().0;
                let owner = read(&fixture, owner);
                assert!(
                    owner["limits"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|limit| limit.to_string().contains(key)),
                    "missing owner limit: {owner}"
                );
                assert_eq!(tree_snapshot(&fixture.source), before);
            }
        }
    }
}

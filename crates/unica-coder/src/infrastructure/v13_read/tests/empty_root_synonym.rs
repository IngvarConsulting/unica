use super::*;
use serde_json::Value;
use unica_workspace::test_support::tree_snapshot;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn synonym_limit(node: &Value) -> bool {
    node.get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|text| text.contains("Synonym"))
}

fn install(fixture: &RealReaderFixture, kind: &str, synonym: &str) -> (String, String) {
    let (relative, name) = match kind {
        "Constant" => ("Constants/MainCurrency.xml", "MainCurrency"),
        "Catalog" => ("Catalogs/Items.xml", "Items"),
        _ => unreachable!(),
    };
    let path = fixture.source.join(relative);
    let original = if kind == "Constant" {
        fixture_text("platform_8_3_27/meta_info_surface/constant/Constants/MainCurrency.xml")
    } else {
        fs::read_to_string(&path).unwrap()
    };
    let text = original.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(text).unwrap();
    let properties = document
        .root_element()
        .children()
        .find(|n| n.has_tag_name((MD, kind)))
        .unwrap()
        .children()
        .find(|n| n.has_tag_name((MD, "Properties")))
        .unwrap();
    let existing = properties
        .children()
        .find(|n| n.has_tag_name((MD, "Synonym")));
    let range = existing.map_or_else(
        || {
            let end = properties
                .children()
                .find(|n| n.has_tag_name((MD, "Name")))
                .unwrap()
                .range()
                .end;
            end..end
        },
        |node| node.range(),
    );
    let mut updated = text.to_string();
    updated.replace_range(range, synonym);
    write(&path, &updated);
    (format!("main:{kind}.{name}"), name.into())
}

#[test]
fn empty_root_synonym_is_omitted_only_in_compact_view_and_remains_a_known_whole_property() {
    for kind in ["Constant", "Catalog"] {
        for raw in [
            "<Synonym/>",
            "<Synonym> \n\t </Synonym>",
            "<Synonym xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><v8:item><v8:lang>ru</v8:lang><v8:content/></v8:item></Synonym>",
        ] {
            let fixture = RealReaderFixture::new();
            let (at, name) = install(&fixture, kind, raw);
            let before = tree_snapshot(&fixture.source);
            let service = fixture.view_service();
            let compact = service.view(ViewRequest::new(&at).unwrap());
            assert!(compact.ok, "{kind} {raw}: {compact:?}");
            let compact = compact.data.unwrap();
            assert_eq!(compact["title"], name);
            assert!(compact["props"].get("Synonym").is_none(), "{kind} {raw}: {compact}");
            assert!(!synonym_limit(&compact), "known empty is not unreadable or oversized: {kind} {raw}: {compact}");
            let whole = crate::infrastructure::v13_read_projection::with_whole_prop("Synonym", || service.view(ViewRequest::new(&at).unwrap()));
            assert!(whole.ok);
            let whole = whole.data.unwrap();
            assert_eq!(whole["props"]["Synonym"], "");
            assert_eq!(compact.get("limits"), whole.get("limits"));
            let another = crate::infrastructure::v13_read_projection::with_whole_prop("Comment", || service.view(ViewRequest::new(&at).unwrap()));
            assert!(another.data.unwrap()["props"].get("Synonym").is_none());
            let mut compact_props = compact["props"].as_object().unwrap().clone();
            let mut whole_props = whole["props"].as_object().unwrap().clone();
            compact_props.remove("Synonym");whole_props.remove("Synonym");
            assert_eq!(compact_props, whole_props, "other observations retain their values");
            if kind == "Constant" {
                assert_eq!(compact["props"]["Comment"], "");
                assert_eq!(compact["props"]["PasswordMode"], false);
                assert!(compact["props"]["Type"].is_object());
            }
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

#[test]
fn root_synonym_absence_damage_and_translations_keep_distinct_observations() {
    for kind in ["Constant", "Catalog"] {
        for (raw, expected, damaged) in [
            ("", None, false),
            ("<Synonym><Unknown/></Synonym>", None, true),
            ("<Synonym xmlns=\"urn:foreign\"/>", None, true),
            ("<Synonym/><Synonym/>", None, true),
            ("<Synonym xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><v8:item><v8:lang>ru</v8:lang><v8:content>Русский &amp; текст</v8:content></v8:item></Synonym>", Some("Русский & текст"), false),
            ("<Synonym xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><v8:item><v8:lang>en</v8:lang><v8:content>English text</v8:content></v8:item></Synonym>", Some("English text"), false),
            ("<Synonym xmlns:v8=\"urn:foreign\"><v8:item><v8:lang>ru</v8:lang><v8:content>decoy</v8:content></v8:item></Synonym>", None, true),
        ] {
            let fixture = RealReaderFixture::new();
            let (at, name) = install(&fixture, kind, raw);
            let before = tree_snapshot(&fixture.source);
            let service = fixture.view_service();
            let compact = service.view(ViewRequest::new(&at).unwrap());
            assert!(compact.ok, "{kind} {raw}: {compact:?}");
            let node = compact.data.unwrap();
            assert_eq!(node["props"].get("Synonym").and_then(Value::as_str), expected, "{kind} {raw}: {node}");
            assert_eq!(node["title"].as_str(), Some(expected.unwrap_or(&name)));
            assert_eq!(node.get("limits").is_some_and(|v|v.as_array().is_some_and(|items|items.iter().any(|item|item.as_str().is_some_and(|text|text.contains("Synonym"))))), damaged, "{kind} {raw}: {node}");
            let whole = crate::infrastructure::v13_read_projection::with_whole_prop("Synonym", || service.view(ViewRequest::new(&at).unwrap()));
            assert!(whole.ok);
            assert_eq!(whole.data.unwrap()["props"].get("Synonym").and_then(Value::as_str), expected);
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

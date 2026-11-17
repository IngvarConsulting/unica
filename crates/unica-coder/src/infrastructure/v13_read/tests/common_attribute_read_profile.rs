use super::*;

const OWNER: &str = "main:CommonAttribute.Probe";

fn descriptor() -> String {
    fixture_text("acceptance/workspace-common-attribute-read/src/CommonAttributes/Probe.xml")
}

fn install(fixture: &RealReaderFixture, xml: &str) -> PathBuf {
    let configuration = fixture.source.join("Configuration.xml");
    write(
        &configuration,
        &fs::read_to_string(&configuration).unwrap().replace(
            "</ChildObjects>",
            "<CommonAttribute>Probe</CommonAttribute></ChildObjects>",
        ),
    );
    let path = fixture.source.join("CommonAttributes/Probe.xml");
    write(&path, xml);
    path
}

/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn common_attribute_view_reads_indexing_history_and_input_flags_from_xml() {
    for alternate in [false, true] {
        let fixture = RealReaderFixture::new();
        let mut xml = descriptor();
        if alternate {
            for (before, after) in [
                ("<Indexing>DontIndex", "<Indexing>IndexWithAdditionalOrder"),
                ("<FullTextSearch>Use", "<FullTextSearch>DontUse"),
                ("<DataHistory>Use", "<DataHistory>DontUse"),
                ("<FillFromFillingValue>false", "<FillFromFillingValue>true"),
                ("<FillChecking>DontCheck", "<FillChecking>ShowError"),
                ("<QuickChoice>Auto", "<QuickChoice>Use"),
                ("<CreateOnInput>Auto", "<CreateOnInput>DontUse"),
                (
                    "<ChoiceHistoryOnInput>Auto",
                    "<ChoiceHistoryOnInput>DontUse",
                ),
            ] {
                assert!(xml.contains(before));
                xml = xml.replace(before, after);
            }
        }
        let path = install(&fixture, &xml);
        let before = fs::read(&path).unwrap();
        let configuration = fs::read(fixture.source.join("Configuration.xml")).unwrap();
        let service = fixture.view_service();
        let collection = service.view(ViewRequest::new("main:CommonAttribute").unwrap());
        assert!(collection.ok, "{collection:?}");
        assert_eq!(collection.data.as_ref().unwrap()["items"][0]["at"], OWNER);
        let result = service.view(ViewRequest::new(OWNER).unwrap());
        assert!(result.ok, "{result:?}");
        let props = &result.data.as_ref().unwrap()["props"];
        assert_eq!(props["kind"], "CommonAttribute");
        let expected = if alternate {
            json!({
                "Indexing": "IndexWithAdditionalOrder", "FullTextSearch": "DontUse",
                "DataHistory": "DontUse", "FillFromFillingValue": true,
                "FillChecking": "ShowError", "QuickChoice": "Use",
                "CreateOnInput": "DontUse", "ChoiceHistoryOnInput": "DontUse"
            })
        } else {
            json!({
                "Indexing": "DontIndex", "FullTextSearch": "Use", "DataHistory": "Use",
                "FillFromFillingValue": false, "FillChecking": "DontCheck",
                "QuickChoice": "Auto", "CreateOnInput": "Auto", "ChoiceHistoryOnInput": "Auto"
            })
        };
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(props.get(key), Some(value), "{key}, alternate={alternate}");
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::read(fixture.source.join("Configuration.xml")).unwrap(),
            configuration
        );
    }
}

/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn common_attribute_view_preserves_absence_and_reports_damaged_input_flags() {
    for (raw, damaged) in [
        ("", false),
        ("<FillFromFillingValue/>", true),
        ("<FillFromFillingValue>yes</FillFromFillingValue>", true),
        ("<FillFromFillingValue><Nested>false</Nested></FillFromFillingValue>", true),
        ("<FillFromFillingValue>false</FillFromFillingValue><FillFromFillingValue>true</FillFromFillingValue>", true),
        ("<FillFromFillingValue xmlns=\"urn:foreign\">false</FillFromFillingValue>", true),
    ] {
        let fixture = RealReaderFixture::new();
        let xml = descriptor().replace("<FillFromFillingValue>false</FillFromFillingValue>", raw);
        install(&fixture, &xml);
        let result = fixture.view_service().view(ViewRequest::new(OWNER).unwrap());
        assert!(result.ok, "{result:?}");
        let node = result.data.as_ref().unwrap();
        assert!(node["props"].get("FillFromFillingValue").is_none());
        assert_eq!(node["props"]["Indexing"], "DontIndex");
        assert_eq!(node["props"]["QuickChoice"], "Auto");
        assert_eq!(node.get("limits").is_some_and(|limits| limits.as_array().unwrap().iter().any(|limit| limit.as_str().is_some_and(|text| text.contains("FillFromFillingValue")))), damaged);
    }
}

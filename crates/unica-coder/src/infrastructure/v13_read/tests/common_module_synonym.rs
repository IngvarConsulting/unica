use super::*;

const MODULE: &str = "main:CommonModule.РеактивныйСервер";

fn replace_synonym(fixture: &RealReaderFixture, replacement: &str) {
    let path = fixture.source.join("CommonModules/РеактивныйСервер.xml");
    let text = fs::read_to_string(&path).unwrap();
    let start = text.find("<Synonym>").unwrap();
    let end = text.find("</Synonym>").unwrap() + "</Synonym>".len();
    write(
        &path,
        &format!("{}{replacement}{}", &text[..start], &text[end..]),
    );
}

#[test]
fn common_module_synonym_reaches_collection_and_detail_without_changing_source() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("CommonModules/РеактивныйСервер.xml");
    let before = fs::read(&descriptor).unwrap();
    let source = fs::read_to_string(&descriptor).unwrap();
    let document = roxmltree::Document::parse(source.trim_start_matches('\u{feff}')).unwrap();
    let expected = document
        .descendants()
        .find(|node| node.has_tag_name(("http://v8.1c.ru/8.1/data/core", "content")))
        .unwrap()
        .text()
        .unwrap();
    let service = fixture.view_service();
    let collection = service.view(ViewRequest::new("main:CommonModule").unwrap());
    assert!(collection.ok, "the collection must be readable");
    let rows = collection.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    let row = rows.iter().find(|row| row["at"] == MODULE).unwrap();
    assert_eq!(row["props"]["synonym"], expected);
    let detail = service.view(ViewRequest::new(MODULE).unwrap());
    assert!(detail.ok, "the module must be readable");
    let data = detail.data.as_ref().unwrap();
    assert_eq!(data["props"]["synonym"], expected);
    assert_eq!(data["at"], MODULE);
    assert_eq!(row["title"], "РеактивныйСервер");
    assert_eq!(data["title"], "Common module РеактивныйСервер");
    assert_eq!(data["props"]["commonModule"].as_object().unwrap().len(), 8);
    let methods = service.view(ViewRequest::new(&format!("{MODULE}.Method")).unwrap());
    assert!(
        methods.ok,
        "the existing method branch must remain readable"
    );
    assert_eq!(fs::read(&descriptor).unwrap(), before);
}

#[test]
fn common_module_synonym_preserves_localization_absence_and_local_damage() {
    for (raw, expected, damaged) in [
        ("", None, false),
        ("<Synonym/>", None, false),
        ("<Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Server</v8:content></v8:item><v8:item><v8:lang>ru</v8:lang><v8:content>Сервер</v8:content></v8:item></Synonym>", Some("Сервер"), false),
        ("<Synonym><v8:item><v8:lang>en</v8:lang><v8:content>Server</v8:content></v8:item></Synonym>", Some("Server"), false),
        ("<Synonym><broken/></Synonym>", None, true),
        ("<Synonym/><Synonym/>", None, true),
    ] {
        let fixture = RealReaderFixture::new();
        replace_synonym(&fixture, raw);
        let service = fixture.view_service();
        for at in ["main:CommonModule", MODULE] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "damage to synonym must not hide the module");
            let data = result.data.as_ref().unwrap();
            let node = if at == MODULE { data } else {
                data["items"].as_array().unwrap().iter().find(|row| row["at"] == MODULE).unwrap()
            };
            assert_eq!(node["props"].get("synonym").and_then(serde_json::Value::as_str), expected);
            assert_eq!(node.get("limits").is_some_and(|v| v.as_array().is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(|s| s.contains("Synonym"))))), damaged);
            if at == MODULE {
                assert!(node["props"]["commonModule"]["server"].is_boolean());
                assert!(node["branches"].is_array());
            }
        }
    }
}

#[test]
fn common_module_synonym_obeys_compact_limit_and_is_readable_whole() {
    let fixture = RealReaderFixture::new();
    let text = "С".repeat(1025);
    replace_synonym(&fixture, &format!("<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>{text}</v8:content></v8:item></Synonym>"));
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new(MODULE).unwrap());
    assert!(result.ok);
    let data = result.data.unwrap();
    assert!(data["props"].get("synonym").is_none());
    assert!(data["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().is_some_and(|s| s.contains(".Props.synonym"))));
    let whole = crate::infrastructure::v13_read_projection::with_whole_prop("synonym", || {
        service.view(ViewRequest::new(MODULE).unwrap())
    });
    assert!(whole.ok, "the whole-property projection must work");
    assert_eq!(whole.data.as_ref().unwrap()["props"]["synonym"], text);
}

#[test]
fn common_module_synonym_of_a_borrowed_module_is_local_and_keeps_unknown_flags() {
    for local in [true, false] {
        let fixture = RealReaderFixture::new();
        let configuration = fixture.source.join("Configuration.xml");
        write(
            &configuration,
            &fs::read_to_string(&configuration).unwrap().replace(
                "</ChildObjects>",
                "<CommonModule>CorpusModule</CommonModule></ChildObjects>",
            ),
        );
        let mut descriptor = fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml");
        let start = descriptor.find("<Synonym>").unwrap();
        let end = descriptor.find("</Synonym>").unwrap() + "</Synonym>".len();
        let replacement = if local {
            "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Локальный</v8:content></v8:item></Synonym>"
        } else {
            ""
        };
        descriptor.replace_range(start..end, replacement);
        // Missing environment flags must remain unknown after adding the fact.
        descriptor = descriptor.replace("<Server>false</Server>", "");
        write(
            &fixture.source.join("CommonModules/CorpusModule.xml"),
            &descriptor,
        );
        write(
            &fixture
                .source
                .join("CommonModules/CorpusModule/Ext/Module.bsl"),
            "Procedure Probe() Export\nEndProcedure\n",
        );
        let result = fixture
            .extension_view_service()
            .view(ViewRequest::new("main:CommonModule.CorpusModule").unwrap());
        assert!(result.ok, "the borrowed module must remain readable");
        let data = result.data.as_ref().unwrap();
        assert_eq!(
            data["props"]
                .get("synonym")
                .and_then(serde_json::Value::as_str),
            local.then_some("Локальный")
        );
        assert!(data["props"]["commonModule"]["privileged"].is_null());
        assert!(data["props"]["commonModule"]["server"].is_null());
        assert!(data["limits"].as_array().unwrap().iter().any(|v| v
            .as_str()
            .is_some_and(|s| s.contains("contexts are unknown"))));
        assert!(data["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["at"] == "main:CommonModule.CorpusModule.Method"));
    }
}

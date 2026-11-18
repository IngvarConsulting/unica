use super::*;

fn fixture() -> RealReaderFixture {
    let f = RealReaderFixture::new();
    let config_path = f.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    write(
        &config_path,
        &config.replacen(
            "<Catalog>Items</Catalog>",
            "<Catalog>Items</Catalog><Task>Approval</Task>",
            1,
        ),
    );
    write(
        &f.source.join("Tasks/Approval.xml"),
        &fixture_text("acceptance/workspace-task-addressing/src/Tasks/Approval.xml"),
    );
    f
}
fn read(f: &RealReaderFixture, at: &str) -> serde_json::Value {
    let r = f.view_service().view(ViewRequest::new(at).unwrap());
    assert!(r.ok, "{at}: {:?}", refusal_codes(&r));
    r.data.unwrap()
}
#[test]
fn task_addressing_has_a_distinct_reachable_branch_and_independent_xml_facts() {
    let f = fixture();
    let before = fs::read(f.source.join("Tasks/Approval.xml")).unwrap();
    let root = read(&f, "main:Task.Approval");
    assert!(root["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at":"main:Task.Approval.AddressingAttribute","count":4})));
    let ordinary = read(&f, "main:Task.Approval.Attribute");
    assert_eq!(ordinary["items"].as_array().unwrap().len(), 14);
    let collection = read(&f, "main:Task.Approval.AddressingAttribute");
    let items = collection["items"].as_array().unwrap();
    let text = std::str::from_utf8(&before).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    let md = "http://v8.1c.ru/8.3/MDClasses";
    let xml = doc
        .descendants()
        .filter(|n| n.has_tag_name((md, "AddressingAttribute")))
        .collect::<Vec<_>>();
    assert_eq!(items.len(), xml.len());
    for (item, node) in items.iter().zip(xml) {
        let props = node
            .children()
            .find(|n| n.has_tag_name((md, "Properties")))
            .unwrap();
        let scalar = |name| {
            props
                .children()
                .find(|n| n.has_tag_name((md, name)))
                .unwrap()
                .text()
                .unwrap()
        };
        let at = format!("main:Task.Approval.AddressingAttribute.{}", scalar("Name"));
        assert_eq!(item["at"], at);
        assert_eq!(item["kind"], "AddressingAttribute");
        let detail = read(&f, &at);
        assert_eq!(item["props"], detail["props"]);
        assert_eq!(
            detail["props"]["addressingDimension"],
            scalar("AddressingDimension")
        );
        assert_eq!(
            detail["props"]["type"]["variants"],
            json!([{"kind":"string","length":50,"allowedLength":"variable"}])
        );
    }
    let usual = read(&f, "main:Task.Approval.Attribute.Performer");
    assert_eq!(usual["kind"], "Attribute");
    assert_eq!(
        usual["props"]["type"]["variants"],
        json!([{"kind":"boolean"}])
    );
    assert!(usual["props"].get("addressingDimension").is_none());
    assert_eq!(
        fs::read(f.source.join("Tasks/Approval.xml")).unwrap(),
        before
    );
}

#[test]
fn task_addressing_unknown_and_damaged_collections_never_become_known_empty() {
    for case in [
        "missing-container",
        "duplicate",
        "foreign",
        "bad-type",
        "bad-dimension",
        "unknown-property",
    ] {
        let f = fixture();
        let path = f.source.join("Tasks/Approval.xml");
        let mut xml = fs::read_to_string(&path).unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let child = doc
            .descendants()
            .find(|n| n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "ChildObjects")))
            .unwrap();
        let first = child
            .children()
            .find(|n| n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "AddressingAttribute")))
            .unwrap();
        let field = xml[first.range()].to_string();
        let range = child.range();
        xml = match case {
            "missing-container" => {
                xml.replace_range(range, "");
                xml
            }
            "duplicate" => xml.replacen(&field, &format!("{field}{field}"), 1),
            "foreign" => xml.replacen(
                &field,
                &field.replacen(
                    "<AddressingAttribute>",
                    "<AddressingAttribute xmlns=\"urn:foreign\">",
                    1,
                ),
                1,
            ),
            "bad-type" => xml.replacen("<v8:Type>xs:string</v8:Type>", "<v8:Type/>", 1),
            "bad-dimension" => xml.replacen(
                "InformationRegister.Performers.Dimension.Performer</AddressingDimension>",
                "Not.A.Dimension</AddressingDimension>",
                1,
            ),
            _ => xml.replacen(
                &field,
                &field.replace("</Properties>", "<UnknownProperty/></Properties>"),
                1,
            ),
        };
        write(&path, &xml);
        let root = read(&f, "main:Task.Approval");
        assert!(
            !root["branches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["at"] == "main:Task.Approval.AddressingAttribute"),
            "{case}"
        );
        assert!(
            root["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l.as_str().unwrap().contains("addressingAttributes")),
            "{case}"
        );
        for at in [
            "main:Task.Approval.AddressingAttribute",
            "main:Task.Approval.AddressingAttribute.Performer",
        ] {
            let r = f.view_service().view(ViewRequest::new(at).unwrap());
            assert!(!r.ok, "{case}/{at}");
            assert_eq!(refusal_codes(&r), ["provider_unavailable"], "{case}");
        }
        if case != "missing-container" {
            assert_eq!(
                read(&f, "main:Task.Approval.Attribute")["items"]
                    .as_array()
                    .unwrap()
                    .len(),
                14
            );
        }
        assert_eq!(read(&f, "main:Catalog.Items")["kind"], "Catalog");
        assert_eq!(fs::read_to_string(&path).unwrap(), xml);
    }
}

#[test]
fn task_addressing_proved_empty_is_reachable_and_other_owners_do_not_acquire_it() {
    let f = fixture();
    let path = f.source.join("Tasks/Approval.xml");
    let mut xml = fs::read_to_string(&path).unwrap();
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let mut ranges = doc
        .descendants()
        .filter(|n| n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "AddressingAttribute")))
        .map(|n| n.range())
        .collect::<Vec<_>>();
    ranges.reverse();
    for range in ranges {
        xml.replace_range(range, "");
    }
    write(&path, &xml);
    let root = read(&f, "main:Task.Approval");
    assert!(root["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at":"main:Task.Approval.AddressingAttribute","count":0})));
    assert_eq!(
        read(&f, "main:Task.Approval.AddressingAttribute")["items"],
        json!([])
    );
    let r = f
        .view_service()
        .view(ViewRequest::new("main:Catalog.Items.AddressingAttribute").unwrap());
    assert!(!r.ok);
    assert_eq!(refusal_codes(&r), ["not_found"]);
    assert!(!read(&f, "main:Catalog.Items")["branches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["at"].as_str().unwrap().ends_with(".AddressingAttribute")));
}

#[test]
fn task_addressing_reads_standard_platform_field_shapes_without_expanding_public_properties() {
    let f = fixture();
    let path = f.source.join("Tasks/Approval.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let fields = r#"<PasswordMode>false</PasswordMode><Format/><EditFormat/><ToolTip/><MarkNegatives>false</MarkNegatives><Mask/><MultiLine>false</MultiLine><ExtendedEdit>false</ExtendedEdit><MinValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:nil="true"/><MaxValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:nil="true"/><FillFromFillingValue>false</FillFromFillingValue><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="xs:string"/><FillChecking>DontCheck</FillChecking><ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems><ChoiceParameterLinks/><ChoiceParameters/><QuickChoice>Auto</QuickChoice><CreateOnInput>Auto</CreateOnInput><ChoiceForm/><LinkByType/><ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput><Indexing>Index</Indexing><FullTextSearch>Use</FullTextSearch><DataHistory>Use</DataHistory>"#;
    let changed = xml.replace(
        "</AddressingDimension>",
        &format!("</AddressingDimension>{fields}"),
    );
    write(&path, &changed);
    let collection = read(&f, "main:Task.Approval.AddressingAttribute");
    assert_eq!(collection["items"].as_array().unwrap().len(), 4);
    let detail = read(&f, "main:Task.Approval.AddressingAttribute.Performer");
    assert_eq!(
        detail["props"]["addressingDimension"],
        "InformationRegister.Performers.Dimension.Performer"
    );
    assert!(detail["props"].get("PasswordMode").is_none());
}

#[test]
fn task_addressing_unmodelled_type_keeps_members_and_reports_local_warning() {
    let f = fixture();
    let path = f.source.join("Tasks/Approval.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let changed = xml.replacen(
        "<v8:Type>xs:string</v8:Type>",
        "<v8:Type>v8:FutureOpaque</v8:Type>",
        1,
    );
    let changed=changed.replace("<v8:StringQualifiers><v8:Length>50</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>", "");
    write(&path, &changed);
    let items = read(&f, "main:Task.Approval.AddressingAttribute");
    assert_eq!(items["items"].as_array().unwrap().len(), 4);
    let detail = read(&f, "main:Task.Approval.AddressingAttribute.Performer");
    assert!(detail["props"].get("type").is_none());
    assert_eq!(detail["props"]["incomplete"], true);
    assert!(detail["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("validation_warning")));
    assert_eq!(
        detail["props"]["addressingDimension"],
        "InformationRegister.Performers.Dimension.Performer"
    );
}

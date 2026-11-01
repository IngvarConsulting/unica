use super::*;

const OWNER: &str = "main:IntegrationService.Bus";
const NAMES: [&str; 4] = [
    "input_from_SM_normal_priority",
    "input_from_SM_high_priority",
    "output_to_SM_high_priority",
    "output_to_SM_normal_priority",
];
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn channels() -> String {
    NAMES.iter().enumerate().map(|(index,name)|format!("<IntegrationServiceChannel uuid=\"15800000-0000-4000-8000-{:012}\"><Properties><Name>{name}</Name><Synonym/><Comment/><ExternalIntegrationServiceChannelName>e1c::FreshBus::Main::{name}</ExternalIntegrationServiceChannelName><MessageDirection>{}</MessageDirection><ReceiveMessageProcessing>{}</ReceiveMessageProcessing><Transactioned>{}</Transactioned></Properties></IntegrationServiceChannel>",index+1,if index<2 {"Receive"}else{"Send"},if index<2 {"ProcessInput"}else{""},if index<2 {"false"}else{"true"})).collect()
}
fn install(fixture: &RealReaderFixture, body: &str) {
    let root = fixture.source.join("Configuration.xml");
    write(
        &root,
        &fs::read_to_string(&root).unwrap().replace(
            "</ChildObjects>",
            "<IntegrationService>Bus</IntegrationService></ChildObjects>",
        ),
    );
    write(&fixture.source.join("IntegrationServices/Bus.xml"),&format!("<MetaDataObject xmlns=\"{MD}\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\"><IntegrationService uuid=\"15800000-0000-4000-8000-000000000001\"><Properties><Name>Bus</Name></Properties>{body}</IntegrationService></MetaDataObject>"));
    write(
        &fixture
            .source
            .join("IntegrationServices/Bus/Ext/Module.bsl"),
        "Procedure ProcessInput() Export\nEndProcedure\n",
    );
}

#[test]
fn integration_channels_expose_four_named_members_and_six_observed_properties_without_writes() {
    let fixture = RealReaderFixture::new();
    install(
        &fixture,
        &format!("<ChildObjects>{}</ChildObjects>", channels()),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let root = service.view(ViewRequest::new(OWNER).unwrap());
    assert!(root.ok, "{:?}", refusal_codes(&root));
    let root = root.data.unwrap();
    let branch = json!({"at":format!("{OWNER}.Channel"),"count":4});
    assert!(
        root["branches"].as_array().unwrap().contains(&branch),
        "{root:#}"
    );
    assert!(
        root["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at":format!("{OWNER}.Module"),"count":1})),
        "{root:#}"
    );
    let mut cursor = None;
    let mut rows = Vec::new();
    loop {
        let mut request = ViewRequest::new(branch["at"].as_str().unwrap())
            .unwrap()
            .with_limit(2)
            .unwrap();
        if let Some(value) = cursor.take() {
            request = request.with_cursor(value);
        }
        let page = service.view(request);
        assert!(page.ok, "{:?}", refusal_codes(&page));
        rows.extend(page.data.unwrap()["items"].as_array().unwrap().clone());
        cursor = page.cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(rows.len(), 4);
    for (index, row) in rows.into_iter().enumerate() {
        assert_eq!(row["at"], format!("{OWNER}.Channel.{}", NAMES[index]));
        let detail = service.view(ViewRequest::new(row["at"].as_str().unwrap()).unwrap());
        assert!(detail.ok, "{:?}", refusal_codes(&detail));
        for node in [row, detail.data.unwrap()] {
            assert_eq!(node["kind"], "Channel");
            assert_eq!(node["title"], NAMES[index]);
            assert_eq!(node["props"]["Comment"], "");
            assert!(node["props"].get("Synonym").is_none());
            assert_eq!(
                node["props"]["ExternalIntegrationServiceChannelName"],
                format!("e1c::FreshBus::Main::{}", NAMES[index])
            );
            assert_eq!(
                node["props"]["MessageDirection"],
                if index < 2 { "Receive" } else { "Send" }
            );
            assert_eq!(
                node["props"]["ReceiveMessageProcessing"],
                if index < 2 { "ProcessInput" } else { "" }
            );
            assert_eq!(node["props"]["Transactioned"], index >= 2);
        }
    }
    let module =
        service.view(ViewRequest::new(&format!("{OWNER}.Module.IntegrationService")).unwrap());
    assert!(module.ok, "{:?}", refusal_codes(&module));
    let denied = service
        .view(ViewRequest::new(&format!("{OWNER}.Channel.{}.Attribute.Fake", NAMES[0])).unwrap());
    assert!(!denied.ok);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn integration_channels_distinguish_unknown_empty_and_partial_composition() {
    for (body, count) in [
        ("".to_string(), None),
        ("<ChildObjects/>".to_string(), Some(0)),
        ("<ChildObjects/><ChildObjects/>".to_string(), None),
        ("<ChildObjects xmlns=\"urn:foreign\"/>".to_string(), None),
        ("<ChildObjects><Unknown/></ChildObjects>".to_string(), None),
        (
            format!("<ChildObjects>{}<Unknown/></ChildObjects>", channels()),
            Some(4),
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, &body);
        let service = fixture.view_service();
        let root = service.view(ViewRequest::new(OWNER).unwrap());
        assert!(root.ok, "{:?}", refusal_codes(&root));
        let root = root.data.unwrap();
        assert!(root["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["at"] == format!("{OWNER}.Module")));
        let list = service.view(ViewRequest::new(&format!("{OWNER}.Channel")).unwrap());
        if let Some(count) = count {
            assert!(list.ok, "{:?}", refusal_codes(&list));
            let data = list.data.unwrap();
            assert_eq!(data["items"].as_array().unwrap().len(), count);
            if count == 0 {
                assert!(data.get("limits").is_none());
            } else {
                assert!(data["limits"].as_array().is_some_and(|l| !l.is_empty()));
            }
        } else {
            assert!(!list.ok);
            assert_eq!(refusal_codes(&list), ["provider_unavailable"]);
            assert!(root["limits"].as_array().is_some_and(|l| !l.is_empty()));
        }
    }
}

#[test]
fn integration_channel_property_damage_preserves_names_other_facts_and_neighbors() {
    for (from, to, key) in [
        (
            "<Transactioned>false</Transactioned>",
            "<Transactioned>False</Transactioned>",
            "Transactioned",
        ),
        (
            "<MessageDirection>Receive</MessageDirection>",
            "<MessageDirection>Unknown</MessageDirection>",
            "MessageDirection",
        ),
        ("<Synonym/>", "<Synonym>bad</Synonym>", "Synonym"),
        ("<Comment/>", "<Comment><Nested/></Comment>", "Comment"),
        (
            "<ReceiveMessageProcessing>ProcessInput</ReceiveMessageProcessing>",
            "<ReceiveMessageProcessing/><ReceiveMessageProcessing/>",
            "ReceiveMessageProcessing",
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(
            &fixture,
            &format!(
                "<ChildObjects>{}</ChildObjects>",
                channels().replacen(from, to, 1)
            ),
        );
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        let root = service.view(ViewRequest::new(OWNER).unwrap());
        assert!(root.ok);
        assert!(root.data.unwrap()["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains(key))));
        let list = service.view(ViewRequest::new(&format!("{OWNER}.Channel")).unwrap());
        assert!(list.ok);
        let data = list.data.unwrap();
        assert_eq!(data["items"].as_array().unwrap().len(), 4);
        let first = &data["items"][0];
        assert!(first["props"].get(key).is_none());
        assert!(first["limits"].as_array().is_some_and(|l| !l.is_empty()));
        let detail = service.view(ViewRequest::new(first["at"].as_str().unwrap()).unwrap());
        assert!(detail.ok);
        let detail = detail.data.unwrap();
        assert_eq!(detail["title"], NAMES[0]);
        assert!(detail["props"].get(key).is_none());
        assert_eq!(data["items"][1]["props"]["Transactioned"], false);
        assert_eq!(data["items"][2]["props"]["Transactioned"], true);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn integration_channel_ambiguous_identity_never_picks_the_first_declaration() {
    let source = channels();
    let first_end =
        source.find("</IntegrationServiceChannel>").unwrap() + "</IntegrationServiceChannel>".len();
    for body in [
        format!("{}{source}", &source[..first_end]),
        source.replacen(
            &format!("<Name>{}</Name>", NAMES[0]),
            &format!("<Name>{}</Name><Name>Alias</Name>", NAMES[0]),
            1,
        ),
        source.replacen(
            "</Properties>",
            "</Properties><Properties><Name>Alias</Name></Properties>",
            1,
        ),
        source.replacen(
            &format!("<Name>{}</Name>", NAMES[0]),
            &format!("<Name xmlns=\"urn:foreign\">{}</Name>", NAMES[0]),
            1,
        ),
        source.replacen(
            &format!("<Name>{}</Name>", NAMES[0]),
            &format!("<Name><Nested>{}</Nested></Name>", NAMES[0]),
            1,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, &format!("<ChildObjects>{body}</ChildObjects>"));
        let service = fixture.view_service();
        let list = service.view(ViewRequest::new(&format!("{OWNER}.Channel")).unwrap());
        assert!(list.ok, "{:?}", refusal_codes(&list));
        let data = list.data.unwrap();
        assert_eq!(data["items"].as_array().unwrap().len(), 3, "{data:#}");
        assert!(data["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["at"] != format!("{OWNER}.Channel.{}", NAMES[0])));
        let ambiguous =
            service.view(ViewRequest::new(&format!("{OWNER}.Channel.{}", NAMES[0])).unwrap());
        assert!(!ambiguous.ok);
        assert_eq!(refusal_codes(&ambiguous), ["provider_unavailable"]);
        let neighbor =
            service.view(ViewRequest::new(data["items"][0]["at"].as_str().unwrap()).unwrap());
        assert!(neighbor.ok);
    }
}

#[test]
fn integration_channel_full_props_and_local_absence_do_not_inherit_values() {
    for size in [0, 2048, 2049] {
        let fixture = RealReaderFixture::new();
        let value = "x".repeat(size);
        let body=channels().replacen(&format!("e1c::FreshBus::Main::{}",NAMES[0]),&value,1).replacen("<Synonym/>","<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Вход</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content/></v8:item></Synonym>",1).replacen("<ReceiveMessageProcessing>ProcessInput</ReceiveMessageProcessing>","",1);
        install(&fixture, &format!("<ChildObjects>{body}</ChildObjects>"));
        let service = fixture.view_service();
        let at = format!("{OWNER}.Channel.{}", NAMES[0]);
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(result.ok);
        let node = result.data.unwrap();
        assert_eq!(
            node["props"]["Synonym"],
            json!([{"language":"ru","content":"Вход"},{"language":"en","content":""}])
        );
        assert!(node["props"].get("ReceiveMessageProcessing").is_none());
        if size <= 2048 {
            assert_eq!(
                node["props"]["ExternalIntegrationServiceChannelName"],
                value
            );
        } else {
            assert!(node["props"]
                .get("ExternalIntegrationServiceChannelName")
                .is_none());
            assert!(node["limits"].as_array().unwrap().iter().any(|l| l
                .as_str()
                .is_some_and(|s| s.contains(".Props.ExternalIntegrationServiceChannelName"))));
        }
        let whole = crate::infrastructure::v13_read_projection::with_whole_prop(
            "ExternalIntegrationServiceChannelName",
            || service.view(ViewRequest::new(&at).unwrap()),
        );
        assert!(whole.ok);
        assert_eq!(
            whole.data.unwrap()["props"]["ExternalIntegrationServiceChannelName"],
            value
        );
    }
}

use super::{write, RealReaderFixture, ViewRequest};
use unica_workspace::test_support::tree_snapshot;

#[test]
fn web_service_handler_collection_and_detail_open_the_bound_procedure() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("WebServices/Exchange/Ext/Module.bsl"),
        "Procedure SendData(Payload) Export\nEndProcedure\n",
    );
    let before = tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let collection = service.view(ViewRequest::new("main:WebService.Exchange.Operation").unwrap());
    assert!(collection.ok, "{collection:?}");
    let item = &collection.data.as_ref().unwrap()["items"][0];
    let detail = service.view(ViewRequest::new("main:WebService.Exchange.Operation.Send").unwrap());
    assert!(detail.ok, "{detail:?}");
    let detail = detail.data.unwrap();
    let target = detail["props"]["implementationAt"]
        .as_str()
        .expect("SOAP operation needs an openable procedure address");
    assert_eq!(
        target,
        "main:WebService.Exchange.Module.WebService.Method.SendData"
    );
    assert_eq!(item["props"]["implementationAt"], target);
    let method = service.view(ViewRequest::new(target).unwrap());
    assert!(method.ok, "{method:?}");
    assert_eq!(method.data.unwrap()["kind"], "Method");
    assert_eq!(detail["props"]["procedure"], "SendData");
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn web_service_handler_does_not_link_missing_or_ambiguous_methods() {
    for text in [
        "",
        "Procedure Other()\nEndProcedure\n",
        "Procedure SendData(\n",
        "Procedure SendData()\nEndProcedure\nProcedure senddata()\nEndProcedure\n",
    ] {
        let fixture = RealReaderFixture::new();
        write(
            &fixture.source.join("WebServices/Exchange/Ext/Module.bsl"),
            text,
        );
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:WebService.Exchange.Operation.Send").unwrap());
        assert!(result.ok, "{result:?}");
        assert!(result.data.unwrap()["props"]["implementationAt"].is_null());
    }
}

#[test]
fn web_service_handler_refuses_ambiguous_raw_operation_names() {
    for second_name in ["Send", "send"] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("WebServices/Exchange.xml");
        let xml = std::fs::read_to_string(&path)
            .unwrap()
            .replace("SendData", "First");
        let start = xml.find("<Operation ").unwrap();
        let end = xml[start..].find("</Operation>").unwrap() + start + "</Operation>".len();
        let second = xml[start..end]
            .replace("First", "Second")
            .replace("<Name>Send</Name>", &format!("<Name>{second_name}</Name>"))
            .replace("666666666667", "666666666669")
            .replace("666666666668", "666666666670");
        write(&path, &format!("{}{}{}", &xml[..end], second, &xml[end..]));
        write(
            &fixture.source.join("WebServices/Exchange/Ext/Module.bsl"),
            "Procedure First() Export\nEndProcedure\nProcedure Second() Export\nEndProcedure\n",
        );
        let service = fixture.view_service();
        for at in [
            "main:WebService.Exchange.Operation",
            "main:WebService.Exchange.Operation.Send",
        ] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "{result:?}");
            let data = result.data.unwrap();
            let rows = data
                .get("items")
                .and_then(|items| items.as_array())
                .cloned()
                .unwrap_or_else(|| vec![data]);
            for row in rows {
                assert!(row["props"]["implementationAt"].is_null(), "{row}");
                assert!(!row["limits"].as_array().unwrap().is_empty());
            }
        }
    }
}

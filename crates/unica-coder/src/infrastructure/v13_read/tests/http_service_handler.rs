use super::{write, RealReaderFixture, ViewRequest};
use unica_workspace::test_support::tree_snapshot;

#[test]
fn http_service_handler_collection_and_detail_open_the_bound_procedure() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("HTTPServices/ExternalAPI/Ext/Module.bsl"),
        "Procedure MetricsGet(Payload) Export\nEndProcedure\n",
    );
    let before = tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let collection = service
        .view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method").unwrap());
    assert!(collection.ok, "{collection:?}");
    let item = &collection.data.as_ref().unwrap()["items"][0];
    let detail = service.view(
        ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method.Get").unwrap(),
    );
    assert!(detail.ok, "{detail:?}");
    let detail = detail.data.unwrap();
    let target = detail["props"]["implementationAt"]
        .as_str()
        .expect("HTTP method needs an openable procedure address");
    assert_eq!(
        target,
        "main:HTTPService.ExternalAPI.Module.HTTPService.Method.MetricsGet"
    );
    assert_eq!(item["props"]["implementationAt"], target);
    let method = service.view(ViewRequest::new(target).unwrap());
    assert!(method.ok, "{method:?}");
    assert_eq!(method.data.unwrap()["kind"], "Method");
    assert_eq!(detail["props"]["handler"], "MetricsGet");
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn http_service_handler_does_not_link_missing_or_ambiguous_methods() {
    for text in [
        "",
        "Procedure Other()\nEndProcedure\n",
        "Procedure MetricsGet(\n",
        "Procedure MetricsGet()\nEndProcedure\nProcedure metricsget()\nEndProcedure\n",
    ] {
        let fixture = RealReaderFixture::new();
        write(
            &fixture
                .source
                .join("HTTPServices/ExternalAPI/Ext/Module.bsl"),
            text,
        );
        let result = fixture.view_service().view(
            ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method.Get")
                .unwrap(),
        );
        assert!(result.ok, "{result:?}");
        assert!(result.data.unwrap()["props"]["implementationAt"].is_null());
    }
}

#[test]
fn http_service_handler_refuses_ambiguous_raw_method_names() {
    for second_name in ["Get", "get"] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("HTTPServices/ExternalAPI.xml");
        let xml = std::fs::read_to_string(&path)
            .unwrap()
            .replace("MetricsGet", "First");
        let start = xml.find("<Method ").unwrap();
        let end = xml[start..].find("</Method>").unwrap() + start + "</Method>".len();
        let second = xml[start..end]
            .replace("First", "Second")
            .replace("<Name>Get</Name>", &format!("<Name>{second_name}</Name>"))
            .replace("555555555557", "555555555559");
        write(&path, &format!("{}{}{}", &xml[..end], second, &xml[end..]));
        write(
            &fixture
                .source
                .join("HTTPServices/ExternalAPI/Ext/Module.bsl"),
            "Procedure First() Export\nEndProcedure\nProcedure Second() Export\nEndProcedure\n",
        );
        let service = fixture.view_service();
        for at in [
            "main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method",
            "main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method.Get",
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

#[test]
fn http_routes_sharing_a_handler_open_the_same_method() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("HTTPServices/ExternalAPI.xml");
    let xml = std::fs::read_to_string(&path).unwrap();
    let start = xml.find("<URLTemplate ").unwrap();
    let end = xml.find("</URLTemplate>").unwrap() + "</URLTemplate>".len();
    let second = xml[start..end]
        .replace("<Name>Metrics</Name>", "<Name>Second</Name>")
        .replace("555555555556", "555555555558")
        .replace("555555555557", "555555555559");
    write(&path, &format!("{}{}{}", &xml[..end], second, &xml[end..]));
    write(
        &fixture
            .source
            .join("HTTPServices/ExternalAPI/Ext/Module.bsl"),
        "Procedure MetricsGet(Request) Export\nEndProcedure\n",
    );
    let service = fixture.view_service();
    let targets = ["Metrics", "Second"].map(|name| {
        let result = service.view(
            ViewRequest::new(&format!(
                "main:HTTPService.ExternalAPI.URLTemplate.{name}.Method.Get"
            ))
            .unwrap(),
        );
        assert!(result.ok, "{result:?}");
        result.data.unwrap()["props"]["implementationAt"].clone()
    });
    assert_eq!(
        targets[0],
        "main:HTTPService.ExternalAPI.Module.HTTPService.Method.MetricsGet"
    );
    assert_eq!(targets[0], targets[1]);
}

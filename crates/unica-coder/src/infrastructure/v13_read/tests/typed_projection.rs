use super::*;

#[test]
fn typed_projection_keeps_summary_compact_and_content_address_selected() {
    let summary =
        QualifiedAddress::parse("main:Report.Продажи.Template.Схема.DataSet.Продажи").unwrap();
    let route = route_logical_address(&summary, PlatformProfile::v8_3_27()).unwrap();
    let payload = json!({
        "support": {"state": "Editable"},
        "dataSets": [{"name": "Продажи", "kind": "Query", "query": "SELECT\n  *"}],
        "sourceSet": "main",
        "metadataPath": "Report.Продажи.Template.Схема"
    });
    let view =
        serde_json::to_value(project_typed_payload(&route, payload.clone()).unwrap()).unwrap();
    assert!(!view.to_string().contains("SELECT"));
    assert!(view.get("sourceSet").is_none());
    assert!(view["branches"].as_array().unwrap().iter().any(|branch| {
        branch["at"] == "main:Report.Продажи.Template.Схема.DataSet.Продажи.Query"
    }));

    let query = QualifiedAddress::parse("main:Report.Продажи.Template.Схема.DataSet.Продажи.Query")
        .unwrap();
    let data = project_known_suffix(
        crate::infrastructure::logical_tree::LogicalReader::Dcs,
        &query,
        &payload,
        &query.segments()[2..],
    )
    .unwrap();
    let view = serde_json::to_value(data).unwrap();
    assert_eq!(view["items"][0], json!({"line": 1, "text": "SELECT"}));
    assert!(view["items"][0].get("at").is_none());
}

/// rule: INV.APP.SUPPORT-STATE
#[test]
fn typed_projection_never_leaks_provider_or_physical_slots() {
    let at = QualifiedAddress::parse("main:Subsystem.Продажи").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let view = serde_json::to_value(
        project_typed_payload(
            &route,
            json!({
                "name": "Продажи",
                "location": {"path": "/secret/Subsystems/Продажи.xml"},
                "fileExists": true,
                "provider": "raw",
                "support": {
                    "state": "locked",
                    "directEditSafe": false,
                    "location": "/secret/ParentConfigurations.bin",
                    "provider": {"raw": "x".repeat(4096)}
                },
                "content": ["Catalog.Товары"],
                "commandInterface": {"visibility": []}
            }),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        view.get("props")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flat_map(|props| props.values())
            .all(|value| !value.is_object() && !value.is_array()),
        "subsystem props must contain only scalar values: {view}"
    );
    assert_eq!(view["props"]["support"], "locked");
    assert_eq!(view["props"]["directEditSafe"], false);
    let text = view.to_string();
    for forbidden in ["/secret", "fileExists", "provider", "sourceState", "layout"] {
        assert!(!text.contains(forbidden), "leaked {forbidden}: {text}");
    }
}

#[test]
fn typed_projection_rejects_unknown_provider_payload_instead_of_dumping_it() {
    for raw in [
        "main:Configuration",
        "main:Catalog.Items",
        "main:CommonForm.Test",
    ] {
        let at = QualifiedAddress::parse(raw).unwrap();
        let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();

        let error = project_typed_payload(
            &route,
            json!({"name": "Main", "mysteryProviderPayload": {"raw": "secret"}}),
        )
        .unwrap_err();

        assert_eq!(error.code().as_str(), "provider_unavailable", "{raw}");
    }
}

#[test]
pub(crate) fn typed_readers_never_publish_their_export_path_in_view_props() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    for at in [
        "main:Report.ParityReport.Form.MainForm",
        "main:Report.ParityReport.Form.MainForm.Item.Goods",
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View",
        "main:Subsystem.Sales.Interface",
        "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        // Assertion messages name the address and the diagnostic code only:
        // a projected payload can carry descriptor identity.
        let codes = result
            .diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic["code"].as_str())
            .collect::<Vec<_>>();
        assert!(result.ok, "{at}: {codes:?}");
        let mut props = result.data.as_ref().unwrap()["props"].clone();
        if at.starts_with("main:XDTOPackage.") {
            let type_info = props["type"]
                .as_object_mut()
                .expect("XDTO type arrives as nested JSON");
            // A namespace URI is a type identity, not an exported file path.
            assert_eq!(type_info["namespace"], "http://v8.1c.ru/8.1/data/core");
            type_info.remove("namespace");
        }
        let props = props.to_string();
        assert!(
            !props.contains(".xml") && !props.contains(".bsl") && !props.contains('/'),
            "{at} published a physical path in view props"
        );
    }
}

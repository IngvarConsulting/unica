use super::*;

#[test]
fn common_template_empty_synonyms_have_no_value_like_other_metadata() {
    for synonym in [
        "",
        "<Synonym/>",
        "<Synonym>   </Synonym>",
        "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content/></v8:item></Synonym>",
    ] {
        let fixture = RealReaderFixture::new();
        install_common_template_fixture(&fixture);
        let descriptor = fixture.source.join("CommonTemplates/Print.xml");
        let original = fs::read_to_string(&descriptor).unwrap();
        let start = original.find("<Synonym>").unwrap();
        let end = original.find("</Synonym>").unwrap() + "</Synonym>".len();
        let mut modified = original;
        modified.replace_range(start..end, synonym);
        write(&descriptor, &modified);
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(result.ok, "{synonym:?}: the call failed");
        let data = result.data.unwrap();
        assert!(
            data["props"]["Synonym"].is_null(),
            "an empty synonym publishes no Synonym value"
        );
        assert_eq!(data["props"]["format"], "SpreadsheetDocument");
    }
}

#[test]
fn common_template_damaged_synonym_keeps_independent_template_facts() {
    for synonym in [
        "<Synonym><Bad/></Synonym>",
        "<Synonym>broken</Synonym>",
        "<Synonym xmlns=\"urn:foreign\"/>",
        "<Synonym/><Synonym/>",
        "<Synonym unexpected=\"yes\"/>",
    ] {
        let fixture = RealReaderFixture::new();
        install_common_template_fixture(&fixture);
        let descriptor = fixture.source.join("CommonTemplates/Print.xml");
        let mut xml = fs::read_to_string(&descriptor).unwrap();
        let start = xml.find("<Synonym>").unwrap();
        let end = xml.find("</Synonym>").unwrap() + "</Synonym>".len();
        xml.replace_range(start..end, synonym);
        write(&descriptor, &xml);
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(result.ok, "{synonym}: the call failed");
        let data = result.data.unwrap();
        assert_eq!(data["title"], "Print");
        assert!(
            data["props"]["Synonym"].is_null(),
            "a damaged synonym publishes no Synonym value"
        );
        assert_eq!(data["props"]["format"], "SpreadsheetDocument");
        assert!(
            data["limits"].to_string().contains("properties.Synonym"),
            "{synonym}: unexpected answer"
        );
        assert!(
            service
                .view(ViewRequest::new("main:CommonTemplate.Print.Area.A.Body").unwrap())
                .ok
        );
        assert_eq!(fs::read_to_string(&descriptor).unwrap(), xml);
    }
}

#[test]
fn common_template_exposes_mxl_areas_body_and_parameters_through_public_view() {
    let fixture = RealReaderFixture::new();
    install_common_template_fixture(&fixture);
    let service = fixture.view_service();
    let at = "main:CommonTemplate.Print";
    let template = service.view(ViewRequest::new(at).unwrap());
    assert!(template.ok, "the call failed");
    let data = template.data.unwrap();
    assert_eq!(data["props"]["format"], "SpreadsheetDocument");
    assert_eq!(data["props"]["Synonym"], "Общий Print");
    assert!(data["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at":format!("{at}.Area"),"count":2})));
    let area = service.view(ViewRequest::new(&format!("{at}.Area.A")).unwrap());
    assert!(area.ok, "the call failed");
    let body = service.view(ViewRequest::new(&format!("{at}.Area.A.Body")).unwrap());
    assert!(body.ok, "the call failed");
    assert_eq!(body.data.as_ref().unwrap()["items"][0]["text"], "Changed");
    let parameter =
        service.view(ViewRequest::new(&format!("{at}.Area.A.Parameter.Probe")).unwrap());
    assert!(parameter.ok, "the call failed");
}

#[test]
fn common_templates_share_mxl_and_dcs_readers_and_validation_inputs() {
    let fixture = RealReaderFixture::new();
    install_common_template_fixture(&fixture);
    for (common, nested) in [("Print", "Print"), ("Schema", "MainSchema")] {
        let bytes = fs::read(
            fixture
                .source
                .join(format!("CommonTemplates/{common}/Ext/Template.xml")),
        )
        .unwrap();
        fs::write(
            fixture.source.join(format!(
                "Reports/ParityReport/Templates/{nested}/Ext/Template.xml"
            )),
            bytes,
        )
        .unwrap();
    }
    let service = fixture.view_service();
    for (common, nested, suffix) in [
        ("Print", "Print", ".Area"),
        ("Print", "Print", ".Area.A"),
        ("Print", "Print", ".Area.A.Body"),
        ("Print", "Print", ".Area.A.Parameter"),
        ("Print", "Print", ".Area.A.Parameter.Probe"),
        ("Schema", "MainSchema", ".DataSet.MainData"),
        ("Schema", "MainSchema", ".DataSet.MainData.Query"),
        ("Schema", "MainSchema", ".Parameter"),
        ("Schema", "MainSchema", ".Parameter.Period"),
        ("Schema", "MainSchema", ".Item"),
        ("Schema", "MainSchema", ".Setting.Основной.Item"),
    ] {
        let a = service
            .view(ViewRequest::new(&format!("main:CommonTemplate.{common}{suffix}")).unwrap());
        let b = service.view(
            ViewRequest::new(&format!(
                "main:Report.ParityReport.Template.{nested}{suffix}"
            ))
            .unwrap(),
        );
        assert!(a.ok && b.ok, "{suffix}: the call failed");
        let normalize = |data: &Option<serde_json::Value>| {
            serde_json::to_string(data)
                .unwrap()
                .replace(&format!("main:CommonTemplate.{common}"), "OWNER")
                .replace(
                    &format!("main:Report.ParityReport.Template.{nested}"),
                    "OWNER",
                )
        };
        assert_eq!(normalize(&a.data), normalize(&b.data), "{suffix}");
    }
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:CommonTemplate.Print").unwrap();
    let input = authority.mxl_validation_input(&at).unwrap();
    assert!(
        crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
            &serde_json::Map::new(),
            &input.text,
            input.artifact
        )
        .ok
    );
    let schema = QualifiedAddress::parse("main:CommonTemplate.Schema").unwrap();
    assert!(authority.dcs_validation_input(&schema).is_ok());
    assert!(authority.dcs_validation_input(&at).is_err());
    assert!(authority.mxl_validation_input(&schema).is_err());
}

#[test]
fn common_template_descriptor_and_registration_boundaries_fail_closed() {
    let fixture = RealReaderFixture::new();
    install_common_template_fixture(&fixture);
    let descriptor = fixture.source.join("CommonTemplates/Print.xml");
    let original = fs::read_to_string(&descriptor).unwrap();
    for variant in [
        "root-qname",
        "artifact-qname",
        "wrong-name",
        "missing-type",
        "unknown-type",
        "duplicate-type",
        "nested-type",
        "missing-properties",
    ] {
        let text = match variant {
            "root-qname" => original
                .replace(
                    "<MetaDataObject ",
                    "<MetaDataObject xmlns:x=\"urn:foreign\" ",
                )
                .replace("<MetaDataObject ", "<x:MetaDataObject ")
                .replace("</MetaDataObject>", "</x:MetaDataObject>"),
            "artifact-qname" => {
                original.replace("<CommonTemplate ", "<CommonTemplate xmlns=\"urn:foreign\" ")
            }
            "wrong-name" => original.replace("<Name>Print</Name>", "<Name>Other</Name>"),
            "missing-type" => {
                original.replace("<TemplateType>SpreadsheetDocument</TemplateType>", "")
            }
            "unknown-type" => original.replace("SpreadsheetDocument", "FutureFormat"),
            "duplicate-type" => original.replace(
                "</TemplateType>",
                "</TemplateType><TemplateType>SpreadsheetDocument</TemplateType>",
            ),
            "nested-type" => {
                original.replace("SpreadsheetDocument", "<Value>SpreadsheetDocument</Value>")
            }
            "missing-properties" => original
                .replace("<Properties>", "<Other>")
                .replace("</Properties>", "</Other>"),
            _ => unreachable!(),
        };
        fs::write(&descriptor, text).unwrap();
        let view = fixture
            .view_service()
            .view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(!view.ok, "{variant}: the call was expected to be refused");
        assert_eq!(
            view.diagnostics[0]["code"], "provider_unavailable",
            "{variant}"
        );
    }
    fs::write(&descriptor, &original).unwrap();
    let config_path = fixture.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        &config_path,
        config.replace("<CommonTemplate>Print</CommonTemplate>", ""),
    )
    .unwrap();
    for at in [
        "main:CommonTemplate.Print",
        "main:CommonTemplate.Print.Area.A",
        "main:CommonTemplate.Print.Area.A.Body",
    ] {
        let view = fixture.view_service().view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: the call was expected to be refused");
        assert_eq!(view.diagnostics[0]["code"], "not_found");
    }
    fs::write(&config_path, config).unwrap();
    fs::remove_file(&descriptor).unwrap();
    for at in ["main:CommonTemplate.Print", "main:CommonTemplate"] {
        let view = fixture.view_service().view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: the call was expected to be refused");
        assert_eq!(view.diagnostics[0]["code"], "provider_unavailable");
    }
}

#[test]
fn common_opaque_and_inherited_templates_do_not_open_a_body() {
    let fixture = RealReaderFixture::new();
    install_common_template_fixture(&fixture);
    let html = fixture
        .view_service()
        .view(ViewRequest::new("main:CommonTemplate.Html").unwrap());
    assert!(html.ok, "the call failed");
    let data = html.data.unwrap();
    assert_eq!(data["props"]["format"], "HTMLDocument");
    assert_eq!(data["props"]["Synonym"], "Общий Html");
    assert!(data.get("branches").is_none());
    for suffix in [".Area", ".DataSet", ".Parameter", ".Item", ".Area.A.Body"] {
        let view = fixture
            .view_service()
            .view(ViewRequest::new(&format!("main:CommonTemplate.Html{suffix}")).unwrap());
        assert!(!view.ok, "{suffix}: the call was expected to be refused");
        assert_eq!(view.diagnostics[0]["code"], "not_found");
    }
    let descriptor = fixture.source.join("CommonTemplates/Print.xml");
    let original = fs::read_to_string(&descriptor).unwrap();
    let payload = fixture
        .source
        .join("CommonTemplates/Print/Ext/Template.xml");
    let original_payload = fs::read(&payload).unwrap();
    for format in [
        "SpreadsheetDocument",
        "HTMLDocument",
        "DataCompositionAppearanceTemplate",
    ] {
        fs::write(&descriptor, original.replace("SpreadsheetDocument", format).replace("</Properties>", "<ExtendedConfigurationObject>99999999-9999-4999-8999-999999999999</ExtendedConfigurationObject></Properties>")).unwrap();
        fs::write(&payload, &original_payload).unwrap();
        let local = fixture
            .extension_view_service()
            .view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(local.ok, "{format}: the call failed");
        let data = local.data.unwrap();
        assert_eq!(data["props"]["format"], format);
        if format != "SpreadsheetDocument" {
            assert!(data.get("branches").is_none(), "opaque {format}");
        }
        fs::remove_file(&payload).unwrap();
        let view = fixture
            .extension_view_service()
            .view(ViewRequest::new("main:CommonTemplate.Print").unwrap());
        assert!(view.ok, "{format}: the call failed");
        let data = view.data.unwrap();
        assert_eq!(
            data["props"]["content"], "inherited",
            "{format}: unexpected answer"
        );
        assert!(
            data["props"]["format"].is_null(),
            "{format}: unexpected answer"
        );
        assert!(data.get("branches").is_none());
    }
}

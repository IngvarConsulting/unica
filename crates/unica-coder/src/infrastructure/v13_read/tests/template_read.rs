use super::*;

/// rule: INV.SOURCE.TEMPLATE-AREA-BODY
#[test]
fn template_area_cell_content_is_a_branch_read_only_when_its_address_is_asked() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let area = "main:Report.ParityReport.Template.Print.Area.Header";

    let node = service.view(ViewRequest::new(area).unwrap());
    assert!(node.ok, "{:?}", refusal_codes(&node));
    let data = node.data.as_ref().unwrap();
    // Структурное чтение области знает длину содержимого, но не несёт текста.
    assert_eq!(data["props"]["contentCount"], 2);
    assert_eq!(data.get("content"), None);
    let branches = data["branches"].as_array().unwrap();
    assert!(
        branches
            .iter()
            .any(|branch| branch["at"] == format!("{area}.Body") && branch["count"] == 2),
        "{branches:?}"
    );

    let body = service.view(ViewRequest::new(&format!("{area}.Body")).unwrap());
    assert!(body.ok, "{:?}", refusal_codes(&body));
    let items = body.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(
        items,
        &vec![
            json!({"index": 1, "template": false, "text": "Sales report"}),
            json!({"index": 2, "template": true, "text": "Period: [Since]"}),
        ],
        "порядок ячеек — это и есть смысл печатной формы"
    );
}

/// rule: INV.SOURCE.TEMPLATE-AREA-BODY
#[test]
fn a_structural_template_read_never_serves_a_cached_payload_to_a_content_read() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let area = "main:Report.ParityReport.Template.Print.Area.Header";

    // Один вызов сначала берёт структуру, потом текст: если признак
    // содержимого не входит в ключ кэша, второе чтение получит первый разбор.
    let structural = service.view(ViewRequest::new(area).unwrap());
    assert!(structural.ok, "{:?}", refusal_codes(&structural));
    let body = service.view(ViewRequest::new(&format!("{area}.Body")).unwrap());

    assert!(body.ok, "{:?}", refusal_codes(&body));
    assert_eq!(
        body.data.as_ref().unwrap()["items"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
}

#[test]
pub(crate) fn template_bodies_are_read_only_when_the_template_node_is_addressed() {
    let fixture = RealReaderFixture::new();
    let body = fixture
        .source
        .join("Reports/ParityReport/Templates/Print/Ext/Template.xml");
    let readable = fs::read(&body).unwrap();
    // An unreadable spreadsheet body must not take the owner or the template
    // collection down with it: only the template node opens the body.
    fs::write(
        &body,
        "<document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"><unclosed>",
    )
    .unwrap();
    let service = fixture.view_service();
    for at in [
        "main:Report.ParityReport",
        "main:Report.ParityReport.Template",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "{at}: {:?}", result.diagnostics);
    }
    let node = service.view(ViewRequest::new("main:Report.ParityReport.Template.Print").unwrap());
    assert!(
        !node.ok,
        "an unreadable template body escaped through its own node"
    );
    assert_eq!(node.diagnostics[0]["code"], "provider_unavailable");

    // A readable body still publishes its interior on the template node.
    fs::write(&body, readable).unwrap();
    let service = fixture.view_service();
    let node = service.view(ViewRequest::new("main:Report.ParityReport.Template.Print").unwrap());
    assert!(node.ok, "{:?}", node.diagnostics);
    let branches = node.data.as_ref().unwrap()["branches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        branches
            .iter()
            .any(|branch| branch["at"] == "main:Report.ParityReport.Template.Print.Area"),
        "{branches:?}"
    );
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
pub(crate) fn add_in_templates_stop_addressing_at_the_template_without_reading_the_payload() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let owner = fs::read_to_string(&descriptor).unwrap();
    let close = owner.rfind("</ChildObjects>").unwrap();
    let owner = format!(
        "{}<Template>Driver</Template>{}",
        &owner[..close],
        &owner[close..]
    );
    fs::write(&descriptor, owner).unwrap();
    write(
        &fixture.source.join("Catalogs/Items/Templates/Driver.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="10000000-0000-4000-8000-000000000081"><Properties><Name>Driver</Name><TemplateType>AddIn</TemplateType></Properties></Template></MetaDataObject>"#,
    );
    // The payload is an opaque archive; make it something no XML reader could parse.
    fs::create_dir_all(fixture.source.join("Catalogs/Items/Templates/Driver/Ext")).unwrap();
    fs::write(
        fixture
            .source
            .join("Catalogs/Items/Templates/Driver/Ext/Template.bin"),
        [0x50, 0x4b, 0x03, 0x04, 0xff, 0x00, 0x80],
    )
    .unwrap();
    let service = fixture.view_service();
    let node = service.view(ViewRequest::new("main:Catalog.Items.Template.Driver").unwrap());
    assert!(node.ok, "{:?}", node.diagnostics);
    let data = node.data.as_ref().unwrap();
    assert_eq!(data["kind"], "Template");
    assert!(
        data.get("branches").is_none(),
        "an add-in template has no addressable interior: {data}"
    );

    let authority = fixture.read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    let expected = "main:Catalog.Items.Template.Driver";
    let found = index.find(FindRequest::new(expected).unwrap());
    assert!(
        !found.is_nearest()
            && found
                .candidates()
                .iter()
                .any(|candidate| candidate.at() == expected),
        "add-in template identity is missing from find: {found:?}"
    );
}

#[test]
fn mxl_check_consumes_captured_body_without_reopening_its_display_path() {
    let fixture = RealReaderFixture::new();
    fs::copy(fixture_path("acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template/Ext/Template.xml"), fixture.source.join("Reports/ParityReport/Templates/Print/Ext/Template.xml")).unwrap();
    let at = QualifiedAddress::parse("main:Report.ParityReport.Template.Print").unwrap();
    let authority = fixture.read_authority();
    let input = authority.mxl_validation_input(&at).unwrap();
    fs::remove_file(&input.artifact).unwrap();
    let result = crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
        &serde_json::Map::new(),
        &input.text,
        input.artifact,
    );
    assert!(result.ok, "{:?}", result.errors);
    assert!(authority.mxl_validation_input(&at).is_err());
    let schema = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    assert!(authority.mxl_validation_input(&schema).is_err());
}

#[test]
fn retained_mxl_check_supports_configuration_extension_epf_and_erf_owners() {
    let body = fixture_text(
        "acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template/Ext/Template.xml",
    );
    let wrapper =
        fixture_text("acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template.xml")
            .replace("F06Template", "Print");
    let fixture = RealReaderFixture::new();
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Templates/Print.xml"),
        &wrapper,
    );
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Templates/Print/Ext/Template.xml"),
        &body,
    );
    for authority in [fixture.read_authority(), fixture.extension_read_authority()] {
        let at = QualifiedAddress::parse("main:Report.ParityReport.Template.Print").unwrap();
        let input = authority.mxl_validation_input(&at).unwrap();
        assert!(matches!(
            input.format_guard,
            crate::application::ports::FormatGuardCheck::Allow
        ));
        assert!(
            crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
                &serde_json::Map::new(),
                &input.text,
                input.artifact
            )
            .ok
        );
    }
    let fixture = RealExternalReaderFixture::new();
    for (root, kind, source_set, owner) in [
        (
            &fixture.processor,
            SourceSetKind::ExternalProcessor,
            "epf",
            "ExternalDataProcessor.Import",
        ),
        (
            &fixture.report,
            SourceSetKind::ExternalReport,
            "erf",
            "ExternalReport.Sales",
        ),
    ] {
        let name = owner.split('.').nth(1).unwrap();
        let descriptor = root.join(format!("{name}.xml"));
        let xml = fs::read_to_string(&descriptor).unwrap().replace(
            "</ChildObjects>",
            "<Template>Print</Template></ChildObjects>",
        );
        write(&descriptor, &xml);
        write(&root.join(format!("{name}/Templates/Print.xml")), &wrapper);
        write(
            &root.join(format!("{name}/Templates/Print/Ext/Template.xml")),
            &body,
        );
        let authority = fixture.read_authority(source_set, kind);
        let at = QualifiedAddress::parse(&format!("{source_set}:{owner}.Template.Print")).unwrap();
        let input = authority.mxl_validation_input(&at).unwrap();
        assert!(matches!(
            input.format_guard,
            crate::application::ports::FormatGuardCheck::Allow
        ));
        assert!(
            crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
                &serde_json::Map::new(),
                &input.text,
                input.artifact
            )
            .ok
        );
        let orphan =
            QualifiedAddress::parse(&format!("{source_set}:{owner}.Template.Orphan")).unwrap();
        assert!(authority.mxl_validation_input(&orphan).is_err());
    }
}

#[test]
fn retained_mxl_check_keeps_unbounded_body_read_and_format_warnings() {
    let fixture = RealReaderFixture::new();
    let body = fixture_text(
        "acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template/Ext/Template.xml",
    );
    let wrapper =
        fixture_text("acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template.xml")
            .replace("F06Template", "Print")
            .replace("version=\"2.20\"", "version=\"2.17\"");
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Templates/Print.xml"),
        &wrapper,
    );
    let padded = body.replacen(
        "</document>",
        &format!("<!--{}--></document>", "x".repeat(8 * 1024 * 1024)),
        1,
    );
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Templates/Print/Ext/Template.xml"),
        &padded,
    );
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:Report.ParityReport.Template.Print").unwrap();
    let input = authority.mxl_validation_input(&at).unwrap();
    assert_eq!(input.text, padded);
    assert!(matches!(
        input.format_guard,
        crate::application::ports::FormatGuardCheck::Warn { .. }
    ));
    assert!(
        crate::infrastructure::native_operations::mxl_composition::validate_mxl_input(
            &serde_json::Map::new(),
            &input.text,
            input.artifact
        )
        .ok
    );
    fixture.cancellation.cancel();
    assert!(authority.mxl_validation_input(&at).is_err());
}

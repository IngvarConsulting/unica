use super::*;

#[test]
fn xdto_anonymous_property_view_returns_recursive_addresses_and_bounds() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "</package>",
        r#"<objectType name="NestedExample">
        <property name="Rows" lowerBound="0" upperBound="-1">
            <typeDef xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="ObjectType">
                <property name="Basis" type="xs:string" lowerBound="0" upperBound="1"/>
                <property name="Nested"><typeDef xsi:type="ObjectType">
                    <property name="Recipient" type="xs:decimal" lowerBound="0"/>
                    <property name="Dotted.Name" type="xs:string"/>
                </typeDef></property>
            </typeDef>
        </property>
    </objectType></package>"#,
    );
    fs::write(&package, &xml).unwrap();
    let service = fixture.view_service();
    let root = "main:XDTOPackage.EnterpriseData_1_17_3.Type.NestedExample.Property.Rows";
    let read = |at: &str| {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "{at}: the call failed");
        result.data.unwrap()
    };
    let rows = read(root);
    assert_eq!(rows["props"]["anonymousType"], "ObjectType");
    let bounds = rows["props"]["bounds"].clone();
    assert_eq!(bounds["lower"], "0");
    assert_eq!(bounds["upper"], "-1");
    let children = read(rows["branches"][0]["at"].as_str().unwrap());
    assert_eq!(children["items"].as_array().unwrap().len(), 2);
    let nested = read(children["items"][1]["at"].as_str().unwrap());
    assert_eq!(nested["props"]["anonymousType"], "ObjectType");
    let grandchildren = read(nested["branches"][0]["at"].as_str().unwrap());
    let recipient = read(grandchildren["items"][0]["at"].as_str().unwrap());
    let type_info = recipient["props"]["type"].clone();
    assert_eq!(type_info["local"], "decimal");
    let dotted_at = format!("{root}.Property.Nested.Property.Dotted\\.Name");
    assert_eq!(grandchildren["items"][1]["at"], dotted_at);
    let dotted = read(&dotted_at);
    assert_eq!(dotted["props"], grandchildren["items"][1]["props"]);
    let type_info = dotted["props"]["type"].clone();
    assert_eq!(type_info["local"], "string");
    assert!(dotted
        .get("limits")
        .is_none_or(|limits| limits.as_array().unwrap().is_empty()));
    assert_eq!(fs::read_to_string(&package).unwrap(), xml);
}

#[test]
fn xdto_object_view_follows_local_base_to_inherited_property_without_flattening() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "</package>",
        r#"<objectType name="Основа"><property name="Код" type="xs:string"/></objectType>
<objectType xmlns:tns="http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3" name="Наследник" base="tns:Основа"/>
<objectType xmlns:tns="http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3" name="НетОсновы" base="tns:НеСуществует"/>
</package>"#,
    );
    fs::write(&package, &xml).unwrap();
    let service = fixture.view_service();
    let result = service
        .view(ViewRequest::new("main:XDTOPackage.EnterpriseData_1_17_3.Type.Наследник").unwrap());
    assert!(result.ok, "the call failed");
    let data = result.data.unwrap();
    let base = data["props"]["baseAt"]
        .as_str()
        .expect("local base must be navigable");
    assert_eq!(base, "main:XDTOPackage.EnterpriseData_1_17_3.Type.Основа");
    let qname = data["props"]["base"].clone();
    assert_eq!(qname["local"], "Основа");
    let inherited = service.view(ViewRequest::new(&format!("{base}.Property.Код")).unwrap());
    assert!(inherited.ok, "the call failed");
    let props = inherited.data.unwrap();
    let type_info = props["props"]["type"].clone();
    assert_eq!(type_info["local"], "string");
    let missing = service
        .view(ViewRequest::new("main:XDTOPackage.EnterpriseData_1_17_3.Type.НетОсновы").unwrap());
    assert!(missing.ok, "the call failed");
    assert!(missing.data.unwrap()["props"].get("baseAt").is_none());
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
}

#[test]
fn xdto_object_view_follows_dotted_local_base_address() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "</package>",
        r#"<objectType name="Base.WithDot"><property name="Код" type="xs:string"/></objectType>
<objectType xmlns:tns="http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3" name="Наследник" base="tns:Base.WithDot"/>
<objectType xmlns:tns="http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3" name="НетОсновы" base="tns:НеСуществует"/>
</package>"#,
    );
    fs::write(&package, &xml).unwrap();
    let service = fixture.view_service();
    let result = service
        .view(ViewRequest::new("main:XDTOPackage.EnterpriseData_1_17_3.Type.Наследник").unwrap());
    assert!(result.ok, "the call failed");
    let data = result.data.unwrap();
    let base = data["props"]["baseAt"]
        .as_str()
        .expect("local base must be navigable");
    assert_eq!(
        base,
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Base\\.WithDot"
    );
    let qname = data["props"]["base"].clone();
    assert_eq!(qname["local"], "Base.WithDot");
    let inherited = service.view(ViewRequest::new(&format!("{base}.Property.Код")).unwrap());
    assert!(inherited.ok, "the call failed");
    let props = inherited.data.unwrap();
    let type_info = props["props"]["type"].clone();
    assert_eq!(type_info["local"], "string");
    let missing = service
        .view(ViewRequest::new("main:XDTOPackage.EnterpriseData_1_17_3.Type.НетОсновы").unwrap());
    assert!(missing.ok, "the call failed");
    assert!(missing.data.unwrap()["props"].get("baseAt").is_none());
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
}

#[test]
fn xdto_property_view_preserves_type_identity_and_explicit_and_effective_bounds() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "</package>",
        r#"<objectType name="ClusterAdministrationInfo">
  <property name="ClusterPort" type="xs:decimal"/>
  <property name="WorkServerUserName" type="xs:string" lowerBound="0" nillable="true"/>
  <property name="Ports" type="xs:decimal" lowerBound="1" upperBound="-1"/>
</objectType></package>"#,
    );
    fs::write(&package, &xml).unwrap();
    let service = fixture.view_service();
    for (name, local, lower, upper, lower_explicit, upper_explicit, unbounded) in [
        ("ClusterPort", "decimal", "1", "1", false, false, false),
        ("WorkServerUserName", "string", "0", "1", true, false, false),
        ("Ports", "decimal", "1", "-1", true, true, true),
    ] {
        let at = format!(
            "main:XDTOPackage.EnterpriseData_1_17_3.Type.ClusterAdministrationInfo.Property.{name}"
        );
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(result.ok, "the call failed");
        let data = result.data.unwrap();
        let props = &data["props"];
        let type_info = props["type"].clone();
        assert!(
            type_info.is_object(),
            "property type arrives as nested JSON"
        );
        assert_eq!(type_info["namespace"], "http://www.w3.org/2001/XMLSchema");
        assert_eq!(type_info["local"], local);
        assert_eq!(type_info["raw"], format!("xs:{local}"));
        let bounds = props["bounds"].clone();
        assert!(bounds.is_object(), "property bounds arrive as nested JSON");
        assert_eq!(
            bounds,
            json!({
                "lower": lower, "upper": upper,
                "lowerExplicit": lower_explicit, "upperExplicit": upper_explicit,
                "unbounded": unbounded
            })
        );
        assert!(props.get("location").is_none());
    }
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
}

#[test]
fn xdto_property_view_names_facts_that_exceed_the_compact_limit() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let xml = original.replace(
        "upperBound=\"3\"",
        &format!("upperBound=\"{}\"", "9".repeat(2100)),
    );
    fs::write(&package, &xml).unwrap();
    let result = fixture.view_service().view(ViewRequest::new(
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы"
    ).unwrap());
    assert!(result.ok, "the call failed");
    let data = result.data.unwrap();
    assert!(data["props"]["type"].is_object());
    assert!(data["props"].get("bounds").is_none());
    assert!(
        data["limits"].as_array().unwrap().iter().any(|limit| limit
            .as_str()
            .is_some_and(|text| text.contains("bounds") && text.contains("2048"))),
        "unexpected answer"
    );
    assert_eq!(fs::read_to_string(package).unwrap(), xml);
}

#[test]
fn xdto_check_preserves_complete_native_findings_and_distinguishes_profile_limits() {
    let fixture = RealReaderFixture::new();
    install_xdto_check_fixture(&fixture);
    for (name, count, error_count) in [
        ("Control", 0, 0),
        ("Unsupported", 7, 0),
        ("Paged", 55, 0),
        ("Mixed", 56, 1),
    ] {
        let at = QualifiedAddress::parse(&format!("main:XDTOPackage.{name}")).unwrap();
        let path = fixture
            .source
            .join(format!("XDTOPackages/{name}/Ext/Package.bin"));
        let before = fs::read(&path).unwrap();
        let authority = fixture.read_authority();
        let target = MetadataAddress::parse_logical(&format!("XDTOPackage.{name}")).unwrap();
        let native = authority.read.xdto_payload(&target, None).unwrap();
        let (_, outcome) = authority.check_xdto(&at).unwrap();
        let checked = crate::application::v13::check::normalize_native_outcome(
            &at,
            "XDTOPackage",
            crate::application::v13::check::CheckValidator::Xdto,
            outcome.unwrap(),
        )
        .unwrap();
        let findings = native["findings"].as_array().unwrap();
        assert_eq!(findings.len(), count, "{name}");
        assert_eq!(checked.diagnostics().len(), count, "{name}");
        assert_eq!(checked.ok(), error_count == 0, "{name}");
        assert_eq!(
            checked
                .diagnostics()
                .iter()
                .filter(|d| d.severity() == "error")
                .count(),
            error_count,
            "{name}"
        );
        for (finding, diagnostic) in findings.iter().zip(checked.diagnostics()) {
            assert_eq!(
                finding["severity"], "error",
                "native/writer classification must remain unchanged"
            );
            assert_eq!(diagnostic.code(), finding["code"].as_str().unwrap());
            let offset = finding["location"]["bodyByteRange"]["start"]
                .as_u64()
                .unwrap();
            assert!(
                diagnostic
                    .message()
                    .contains(&format!("byte offset {offset} (0-based, BOM excluded)")),
                "{diagnostic:?}"
            );
            assert!(
                diagnostic
                    .message()
                    .contains(finding["message"].as_str().unwrap()),
                "{diagnostic:?}"
            );
            if diagnostic.code() == "unsupported_node" {
                assert_eq!(diagnostic.severity(), "warning");
                assert!(diagnostic.message().contains("Unica profile limitation"));
            }
        }
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn xdto_check_rejects_unreadable_sources_and_keeps_inherited_packages_opaque() {
    let fixture = RealReaderFixture::new();
    install_xdto_check_fixture(&fixture);
    let at = QualifiedAddress::parse("main:XDTOPackage.Control").unwrap();
    let path = fixture.source.join("XDTOPackages/Control/Ext/Package.bin");
    let body = fs::read(&path).unwrap();
    for invalid in [
        b"not XML".as_slice(),
        b"\xff",
        b"\xef\xbb\xbf\xef\xbb\xbf<package/>",
    ] {
        fs::write(&path, invalid).unwrap();
        let result = fixture.read_authority().check_xdto(&at);
        assert!(
            result.is_err(),
            "parse failures are not unsupported-node warnings"
        );
    }
    fs::write(&path, &body).unwrap();
    let descriptor = fixture.source.join("XDTOPackages/Control.xml");
    let original = fs::read_to_string(&descriptor).unwrap();
    for invalid in [
        original.replace("urn:sample", "urn:other"),
        original.replace("<Name>Control</Name>", "<Name>Other</Name>"),
        original.replace("http://v8.1c.ru/8.3/MDClasses", "urn:foreign"),
    ] {
        fs::write(&descriptor, invalid).unwrap();
        assert!(fixture.read_authority().check_xdto(&at).is_err());
    }
    fs::write(&descriptor, &original).unwrap();
    let config = fixture.source.join("Configuration.xml");
    let original_config = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        original_config.replace("<XDTOPackage>Control</XDTOPackage>", ""),
    )
    .unwrap();
    assert_eq!(
        fixture.read_authority().check_xdto(&at).unwrap_err().code(),
        RefusalCode::NotFound
    );
    fs::write(&config, original_config).unwrap();
    fs::write(&descriptor,original.replace("</Properties>","<ExtendedConfigurationObject>99999999-9999-4999-8999-999999999999</ExtendedConfigurationObject></Properties>")).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(fixture
        .extension_read_authority()
        .check_xdto(&at)
        .unwrap()
        .1
        .is_none());
    fixture.cancellation.cancel();
    assert_eq!(
        fixture.read_authority().check_xdto(&at).unwrap_err().code(),
        RefusalCode::Cancelled
    );
}

#[test]
fn xdto_check_refuses_linked_descriptor_and_package_resources() {
    use crate::composition::testing::{create_file_link_fixture_for_test, FileLinkFixtureOutcome};
    for relative in [
        "XDTOPackages/Control.xml",
        "XDTOPackages/Control/Ext/Package.bin",
    ] {
        let fixture = RealReaderFixture::new();
        install_xdto_check_fixture(&fixture);
        let path = fixture.source.join(relative);
        let bytes = fs::read(&path).unwrap();
        let foreign = fixture.root.path().join("foreign.xml");
        fs::write(&foreign, bytes).unwrap();
        fs::remove_file(&path).unwrap();
        let outcome = create_file_link_fixture_for_test(&foreign, &path).unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a file link needs the symlink privilege",
            );
            return;
        }
        let at = QualifiedAddress::parse("main:XDTOPackage.Control").unwrap();
        assert_eq!(
            fixture.read_authority().check_xdto(&at).unwrap_err().code(),
            RefusalCode::ProviderUnavailable,
            "{relative}"
        );
    }
}

#[test]
fn xdto_restricted_value_view_returns_named_and_anonymous_facets() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let document = roxmltree::Document::parse(original.trim_start_matches('\u{feff}')).unwrap();
    let namespace = document
        .root_element()
        .attribute("targetNamespace")
        .unwrap();
    let source =
        include_str!("../../../../../../tests/fixtures/xdto/restricted-values/Package.bin")
            .replace("urn:unica:anonymous", namespace);
    fs::write(&package, source.as_bytes()).unwrap();
    let service = fixture.view_service();
    for (suffix, facets, anonymous, base) in [
        ("Type.КодПримера", json!({"maxLength":10}), false, "string"),
        (
            "Type.СуммаПримера",
            json!({"totalDigits":5,"fractionDigits":2,"minInclusive":"0","maxInclusive":"999.99"}),
            false,
            "decimal",
        ),
        (
            "Type.Пример.Property.АнонимныйКод",
            json!({"maxLength":10}),
            true,
            "string",
        ),
    ] {
        let result = service.view(
            ViewRequest::new(&format!("main:XDTOPackage.EnterpriseData_1_17_3.{suffix}")).unwrap(),
        );
        assert!(result.ok, "the call failed");
        let data = result.data.unwrap();
        let props = &data["props"];
        assert_eq!(props["facets"], facets);
        let actual_base = &props["base"];
        assert!(actual_base.is_object(), "base arrives as nested JSON");
        assert_eq!(actual_base["local"], base);
        assert_eq!(actual_base["namespace"], "http://www.w3.org/2001/XMLSchema");
        if anonymous {
            assert_eq!(props["anonymousType"], "ValueType");
        }
    }
    assert_eq!(fs::read(&package).unwrap(), source.as_bytes());
}

#[test]
fn xdto_restricted_value_view_reports_oversized_facets() {
    let fixture = RealReaderFixture::new();
    let package = fixture
        .source
        .join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin");
    let original = fs::read_to_string(&package).unwrap();
    let document = roxmltree::Document::parse(original.trim_start_matches('\u{feff}')).unwrap();
    let namespace = document
        .root_element()
        .attribute("targetNamespace")
        .unwrap();
    let source =
        include_str!("../../../../../../tests/fixtures/xdto/restricted-values/Package.bin")
            .replace("urn:unica:anonymous", namespace)
            .replace("999.99", &"9".repeat(3000))
            .replace(
                "base=\"xs:string\" maxLength=\"10\"",
                &format!("base=\"xs:decimal\" maxInclusive=\"{}\"", "9".repeat(3000)),
            );
    fs::write(&package, source.as_bytes()).unwrap();
    let service = fixture.view_service();
    for suffix in ["Type.СуммаПримера", "Type.Пример.Property.АнонимныйКод"]
    {
        let result = service.view(
            ViewRequest::new(&format!("main:XDTOPackage.EnterpriseData_1_17_3.{suffix}")).unwrap(),
        );
        assert!(result.ok, "the call failed");
        let data = result.data.unwrap();
        assert!(data["props"].get("facets").is_none());
        assert!(data["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("facets"))));
    }
    assert_eq!(fs::read(&package).unwrap(), source.as_bytes());
}

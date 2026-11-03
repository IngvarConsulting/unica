use super::*;

const OWNER: &str = "main:WSReference.Client";
const FULL_WSDL: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-wsreference-definition/src/WSReferences/Client/Ext/WSDefinition.xml"));
const WSDL: &str = r#"<definitions xmlns="http://schemas.xmlsoap.org/wsdl/" xmlns:xs="http://www.w3.org/2001/XMLSchema" name="Client" targetNamespace="urn:client"><message name="Input"><part name="body" type="xs:string"/></message></definitions>"#;

fn install(fixture: &RealReaderFixture, wsdl: Option<&str>) -> PathBuf {
    let configuration = fixture.source.join("Configuration.xml");
    let before = fs::read_to_string(&configuration).unwrap();
    fs::write(
        configuration,
        before.replacen(
            "</ChildObjects>",
            "<WSReference>Client</WSReference></ChildObjects>",
            1,
        ),
    )
    .unwrap();
    let descriptor = fixture.source.join("WSReferences/Client.xml");
    fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
    fs::write(descriptor, r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><WSReference uuid="16230000-0000-4000-8000-000000000001"><Properties><Name>Client</Name></Properties></WSReference></MetaDataObject>"#).unwrap();
    let companion = fixture
        .source
        .join("WSReferences/Client/Ext/WSDefinition.xml");
    if let Some(wsdl) = wsdl {
        fs::create_dir_all(companion.parent().unwrap()).unwrap();
        fs::write(&companion, wsdl).unwrap();
    }
    companion
}

#[test]
fn wsreference_discovers_complete_local_wsdl_without_a_network_call() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture, Some(WSDL));
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new(OWNER).unwrap());
    assert!(owner.ok, "{owner:?}");
    let branch = owner.data.as_ref().unwrap()["branches"]
        .as_array()
        .and_then(|branches| {
            branches
                .iter()
                .find(|branch| branch["at"] == format!("{OWNER}.Definition"))
        })
        .expect("WSReference must expose its local WSDL definition");
    assert_eq!(branch["count"], 3);
    let result = service.view(ViewRequest::new(branch["at"].as_str().unwrap()).unwrap());
    assert!(result.ok, "{result:?}");
    let rows = result.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2]["kind"], "part");
    assert_eq!(
        rows[2]["type"],
        json!({"namespace":"http://www.w3.org/2001/XMLSchema", "localName":"string"})
    );
    assert!(rows.iter().all(|row| row.get("at").is_none()));
    assert_eq!(fs::read(path).unwrap(), before);
}

fn complete_rows<A: ViewReadAuthority>(service: &ViewService<A>) -> Vec<serde_json::Value> {
    let mut cursor = None;
    let mut rows = Vec::new();
    loop {
        let mut request = ViewRequest::new(&format!("{OWNER}.Definition"))
            .unwrap()
            .with_limit(3)
            .unwrap();
        if let Some(value) = cursor.take() {
            request = request.with_cursor(value);
        }
        let result = service.view(request);
        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert!(data.get("limits").is_none(), "{data}");
        rows.extend(data["items"].as_array().unwrap().iter().cloned());
        cursor = result.cursor;
        if cursor.is_none() {
            return rows;
        }
    }
}

#[test]
fn wsreference_all_declarations_keep_namespaces_parentage_and_explicit_values() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture, Some(FULL_WSDL));
    let before = fs::read(&path).unwrap();
    let rows = complete_rows(&fixture.view_service());
    assert_eq!(rows.len(), 41);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["sourceOrder"], index + 1);
        assert!(row.get("at").is_none());
        if index == 0 {
            assert!(row["parentOrder"].is_null());
        } else {
            assert!(row["parentOrder"].as_u64().unwrap() < row["sourceOrder"].as_u64().unwrap());
        }
    }
    let counts = rows
        .iter()
        .fold(std::collections::BTreeMap::new(), |mut counts, row| {
            *counts.entry(row["kind"].as_str().unwrap()).or_insert(0) += 1;
            counts
        });
    assert_eq!(
        counts,
        std::collections::BTreeMap::from([
            ("definitions", 1),
            ("types", 1),
            ("schema", 2),
            ("complexType", 3),
            ("sequence", 3),
            ("element", 6),
            ("attribute", 1),
            ("message", 3),
            ("part", 3),
            ("portType", 1),
            ("operation", 2),
            ("input", 2),
            ("output", 2),
            ("fault", 2),
            ("binding", 1),
            ("soapBinding", 1),
            ("soapOperation", 1),
            ("soapBody", 2),
            ("soapFault", 1),
            ("service", 1),
            ("port", 1),
            ("soapAddress", 1),
        ])
    );
    let row = |kind: &str, name: &str| {
        rows.iter()
            .find(|row| row["kind"] == kind && row["name"] == name)
            .unwrap()
    };
    let values = rows
        .iter()
        .filter(|row| row["kind"] == "element" && row["name"] == "Value")
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 3);
    assert_eq!(values[0]["minOccurs"], "0");
    assert_eq!(values[0]["maxOccurs"], "unbounded");
    assert_eq!(values[0]["nillable"], false);
    assert_eq!(values[0]["form"], "unqualified");
    assert!(values[1]["minOccurs"].is_null());
    assert_ne!(values[0]["parentOrder"], values[1]["parentOrder"]);
    let anonymous = rows
        .iter()
        .find(|row| row["kind"] == "complexType" && row["name"].is_null())
        .unwrap();
    let parent = &rows[anonymous["parentOrder"].as_u64().unwrap() as usize - 1];
    assert_eq!(parent["kind"], "element");
    assert_eq!(parent["name"], "Nested");
    assert_eq!(
        row("element", "Request")["type"],
        json!({"namespace":"urn:payload-a","localName":"Payload"})
    );
    assert_eq!(
        row("element", "Reply")["type"],
        json!({"namespace":"urn:payload-b","localName":"Payload"})
    );
    assert_eq!(row("attribute", "id")["use"], "required");
    assert_eq!(
        row("binding", "ApiSoap")["type"],
        json!({"namespace":"urn:local-api","localName":"Api"})
    );
    assert_eq!(
        row("port", "Http")["binding"],
        json!({"namespace":"urn:local-api","localName":"ApiSoap"})
    );
    assert!(rows
        .iter()
        .find(|row| row["kind"] == "soapOperation")
        .unwrap()["soapAction"]
        .is_null());
    assert_eq!(
        rows.last().unwrap()["location"],
        "https://example.invalid/soap"
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn wsreference_prefix_aliases_do_not_change_any_declaration() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture, Some(FULL_WSDL));
    let expected = complete_rows(&fixture.view_service());
    let aliases = FULL_WSDL
        .replace("xmlns:a=", "xmlns:other=")
        .replace("a:", "other:")
        .replace("xmlns:soap=", "xmlns:wire=")
        .replace("soap:", "wire:");
    fs::write(&path, &aliases).unwrap();
    assert_eq!(complete_rows(&fixture.view_service()), expected);
    assert_eq!(fs::read_to_string(path).unwrap(), aliases);
}

#[test]
fn wsreference_empty_missing_malformed_and_outside_profile_are_distinct() {
    for (content, expected_rows, reason) in [
        (None, 0, "absent"),
        (Some(""), 0, "unreadable"),
        (Some("<definitions"), 0, "unreadable"),
        (Some("<definitions/>"), 0, "root"),
        (
            Some(r#"<definitions xmlns="http://schemas.xmlsoap.org/wsdl/"/>"#),
            1,
            "",
        ),
        (
            Some(
                r#"<definitions xmlns="http://schemas.xmlsoap.org/wsdl/"><unknown><message name="Hidden"/></unknown><message name="Known"/></definitions>"#,
            ),
            2,
            "outside",
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, content);
        let service = fixture.view_service();
        let owner = service.view(ViewRequest::new(OWNER).unwrap());
        assert!(owner.ok, "{owner:?}");
        let result = service.view(ViewRequest::new(&format!("{OWNER}.Definition")).unwrap());
        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(
            data["items"].as_array().unwrap().len(),
            expected_rows,
            "{data}"
        );
        if reason.is_empty() {
            assert!(data.get("limits").is_none(), "{data}");
        } else {
            assert!(data["limits"].to_string().contains(reason), "{data}");
            assert!(owner.data.as_ref().unwrap()["limits"]
                .to_string()
                .contains(reason));
        }
    }
}

#[test]
fn wsreference_local_damage_preserves_independent_declarations_and_dependencies() {
    for (content, reason) in [
        (WSDL.replace("xs:string", "missing:T"),"prefix"),
        (WSDL.replace("xs:string", "xs:UnknownType"),"target"),
        (FULL_WSDL.replace("nillable=\"false\"", "nillable=\"broken\""),"boolean"),
        (FULL_WSDL.replace("minOccurs=\"0\"", "minOccurs=\"-1\""),"bound"),
        (WSDL.replace("</definitions>","<import namespace=\"urn:outside\" location=\"https://example.invalid/must-not-be-loaded\"/></definitions>"),"external dependency"),
        (WSDL.replace("</definitions>","<message name=\"Input\"/></definitions>"),"duplicate"),
    ] {
        let fixture = RealReaderFixture::new(); let path=install(&fixture,Some(&content)); let before=fs::read(&path).unwrap();
        let result=fixture.view_service().view(ViewRequest::new(&format!("{OWNER}.Definition")).unwrap());
        assert!(result.ok,"{result:?}"); let data=result.data.as_ref().unwrap();
        assert!(data["limits"].to_string().contains(reason),"{data}");
        assert!(data["items"].as_array().unwrap().iter().any(|row| row["kind"] == "message"));
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn wsreference_pages_keep_the_original_definition_after_source_changes() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture, Some(FULL_WSDL));
    let service = fixture.view_service();
    let first = service.view(
        ViewRequest::new(&format!("{OWNER}.Definition"))
            .unwrap()
            .with_limit(3)
            .unwrap(),
    );
    assert!(first.ok, "{first:?}");
    let cursor = first.cursor.unwrap();
    fs::write(&path, WSDL).unwrap();
    let request = ViewRequest::new(&format!("{OWNER}.Definition"))
        .unwrap()
        .with_limit(3)
        .unwrap()
        .with_cursor(cursor);
    let old = service.view(request.clone());
    let replay = service.view(request);
    assert!(old.ok, "{old:?}");
    assert_eq!(old.data, replay.data);
    assert_eq!(old.cursor, replay.cursor);
    assert_eq!(old.data.as_ref().unwrap()["items"][0]["sourceOrder"], 4);
    let mut rows = old.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap()
        .clone();
    let mut next = old.cursor;
    while let Some(cursor) = next {
        let page = service.view(
            ViewRequest::new(&format!("{OWNER}.Definition"))
                .unwrap()
                .with_limit(3)
                .unwrap()
                .with_cursor(cursor),
        );
        assert!(page.ok, "{page:?}");
        rows.extend(
            page.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        next = page.cursor;
    }
    assert_eq!(rows.len(), 38);
    assert_eq!(rows.last().unwrap()["kind"], "soapAddress");
    // A read authority belongs to one admitted operation. A new question gets
    // a fresh authority; the issued cursor retains the previous operation.
    assert_eq!(complete_rows(&fixture.view_service()).len(), 3);
}

#[test]
fn wsreference_definition_is_only_a_collection_of_its_registered_owner() {
    let fixture = RealReaderFixture::new();
    install(&fixture, Some(WSDL));
    let service = fixture.view_service();
    for at in [
        "main:Constant.MainCurrency.Definition",
        "main:WSReference.Client.Definition.Property.X",
        "main:WSReference.Orphan.Definition",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "{at}: {result:?}");
        assert_eq!(refusal_codes(&result), vec!["not_found"]);
    }
    assert!(ViewRequest::new("main:WSReference.Client.Definition.Invented").is_err());
}

#[test]
fn wsreference_companion_symlink_cannot_read_another_source() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture, Some(WSDL));
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("private.xml");
    let secret = "outside-companion-must-not-be-observed";
    fs::write(
        &target,
        WSDL.replace("name=\"Client\"", &format!("name=\"{secret}\"")),
    )
    .unwrap();
    fs::remove_file(&path).unwrap();
    let created = unica_workspace::test_support::create_file_symlink_for_test(&target, &path)
        .expect("symlink fixtures exist on supported hosts");
    if created.is_err() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::Symlink,
            "creating a link fixture needs the symlink privilege",
        );
        return;
    }
    let result = fixture
        .view_service()
        .view(ViewRequest::new(OWNER).unwrap());
    assert!(!result.ok, "{result:?}");
    assert_eq!(refusal_codes(&result), vec!["provider_unavailable"]);
    assert!(!serde_json::to_string(&result).unwrap().contains(secret));
}

#[test]
fn wsreference_definition_resolves_to_the_companion_only_while_it_exists() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture, Some(WSDL));
    let authority = fixture.read_authority();
    let owner = QualifiedAddress::parse(OWNER).unwrap();
    let at = QualifiedAddress::parse(&format!("{OWNER}.Definition")).unwrap();
    assert_eq!(
        authority.existing_export_path(&owner, None).unwrap(),
        "WSReferences/Client.xml"
    );
    assert_eq!(
        authority.existing_export_path(&at, None).unwrap(),
        "WSReferences/Client/Ext/WSDefinition.xml"
    );
    fs::remove_file(path).unwrap();
    assert_eq!(
        authority
            .existing_export_path(&at, None)
            .unwrap_err()
            .code(),
        RefusalCode::NotFound
    );
}

#[test]
fn wsreference_ncname_whitespace_is_normalized_before_linking() {
    let fixture = RealReaderFixture::new();
    install(
        &fixture,
        Some(
            &WSDL
                .replace("name=\"Input\"", "name=\" Input \"")
                .replace("name=\"body\"", "name=\" body \""),
        ),
    );
    let rows = complete_rows(&fixture.view_service());
    assert_eq!(rows[1]["name"], "Input");
    assert_eq!(rows[2]["name"], "body");
}

#[test]
fn wsreference_name_and_token_fields_follow_their_xml_types() {
    let canonical = RealReaderFixture::new();
    install(&canonical, Some(FULL_WSDL));
    let expected = complete_rows(&canonical.view_service());
    let padded = FULL_WSDL
        .replace("form=\"unqualified\"", "form=\" unqualified \"")
        .replace("use=\"required\"", "use=\" required \"");
    let fixture = RealReaderFixture::new();
    install(&fixture, Some(&padded));
    assert_eq!(complete_rows(&fixture.view_service()), expected);
    let fixture = RealReaderFixture::new();
    install(
        &fixture,
        Some(&WSDL.replace("name=\"body\"", "name=\" 123:body \"")),
    );
    assert_eq!(
        complete_rows(&fixture.view_service())[2]["name"],
        "123:body"
    );
    for (content, reason) in [
        (
            FULL_WSDL.replace("style=\"document\"", "style=\" document \""),
            "style",
        ),
        (WSDL.replace("name=\"Input\"", "name=\"In put\""), "name"),
        (
            WSDL.replace("name=\"Input\"", "name=\"\u{a0}Input\u{a0}\""),
            "name",
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, Some(&content));
        let result = fixture
            .view_service()
            .view(ViewRequest::new(&format!("{OWNER}.Definition")).unwrap());
        assert!(result.ok, "{result:?}");
        assert!(
            result.data.as_ref().unwrap()["limits"]
                .to_string()
                .contains(reason),
            "{result:?}"
        );
    }
}

#[test]
fn wsreference_incomplete_character_content_and_address_are_named() {
    for (content, reason) in [
        (WSDL.replace("</message>", "&#160;</message>"), "content"),
        (
            FULL_WSDL.replace("location=\"https://example.invalid/soap\"", ""),
            "location",
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, Some(&content));
        let result = fixture
            .view_service()
            .view(ViewRequest::new(&format!("{OWNER}.Definition")).unwrap());
        assert!(result.ok, "{result:?}");
        assert!(
            result.data.as_ref().unwrap()["limits"]
                .to_string()
                .contains(reason),
            "{result:?}"
        );
    }
}

#[test]
fn wsreference_duplicate_part_identity_is_preserved_and_limited() {
    let fixture = RealReaderFixture::new();
    install(
        &fixture,
        Some(&WSDL.replace(
            "</message>",
            "<part name=\"body\" type=\"xs:boolean\"/></message>",
        )),
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new(&format!("{OWNER}.Definition")).unwrap());
    assert!(result.ok, "{result:?}");
    let data = result.data.as_ref().unwrap();
    assert_eq!(data["items"].as_array().unwrap().len(), 4);
    assert!(data["limits"].to_string().contains("duplicate"), "{data}");
}

#[test]
fn wsreference_large_declaration_is_indivisible_on_pages() {
    let fixture = RealReaderFixture::new();
    let name = "N".repeat(70_000);
    install(
        &fixture,
        Some(&WSDL.replace("name=\"body\"", &format!("name=\"{name}\""))),
    );
    let rows = complete_rows(&fixture.view_service());
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2]["name"], name);
}

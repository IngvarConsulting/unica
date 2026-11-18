use super::*;

fn modules(fixture: &RealReaderFixture) {
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    write(&path, &xml.replace("<CommonModule>РеактивныйСервер</CommonModule>", "<CommonModule>Zulu</CommonModule><CommonModule>РеактивныйСервер</CommonModule><CommonModule>Alpha</CommonModule>"));
    let descriptor =
        fs::read_to_string(fixture.source.join("CommonModules/РеактивныйСервер.xml")).unwrap();
    for name in ["Zulu", "Alpha"] {
        write(
            &fixture.source.join(format!("CommonModules/{name}.xml")),
            &descriptor.replace("РеактивныйСервер", name),
        );
    }
}

#[test]
fn collection_selection_uses_registration_names_and_preserves_cursor_snapshots() {
    let fixture = RealReaderFixture::new();
    modules(&fixture);
    let cursors = Arc::new(ViewCursorStore::default());
    let view = |request| {
        ViewService::with_shared_cursors(fixture.read_authority(), Arc::clone(&cursors))
            .view(request)
    };
    let request = || {
        ViewRequest::new("main:CommonModule")
            .unwrap()
            .with_limit(1)
            .unwrap()
    };
    let first = view(request().with_offset(0).unwrap());
    assert!(first.ok, "{first:?}");
    assert_eq!(
        first.data.as_ref().unwrap()["items"][0]["at"],
        "main:CommonModule.Zulu"
    );
    assert_eq!(first.data.as_ref().unwrap()["total"], 3);
    let alpha = view(request().with_from("aL".into()).unwrap());
    assert!(alpha.ok, "{alpha:?}");
    assert_eq!(alpha.data.as_ref().unwrap()["offset"], 2);
    assert_eq!(
        alpha.data.as_ref().unwrap()["items"][0]["at"],
        "main:CommonModule.Alpha"
    );
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    write(&path, &xml.replace("<CommonModule>Zulu</CommonModule>", ""));
    let next = view(request().with_cursor(first.cursor.unwrap()));
    assert!(next.ok, "{next:?}");
    assert_eq!(next.data.as_ref().unwrap()["total"], 3);
    assert_eq!(next.data.as_ref().unwrap()["offset"], 1);
    let fresh = view(request().with_from("aL".into()).unwrap());
    assert!(fresh.ok, "{fresh:?}");
    assert_eq!(fresh.data.as_ref().unwrap()["total"], 2);
    assert_eq!(fresh.data.as_ref().unwrap()["offset"], 1);
    assert!(!view(ViewRequest::new("main:CommonModule.Zulu").unwrap()).ok);
}

#[test]
fn registration_membership_keeps_kind_and_case_while_a_collection_is_read() {
    let fixture = RealReaderFixture::new();
    modules(&fixture);
    let catalog = fs::read_to_string(fixture.source.join("Catalogs/Items.xml")).unwrap();
    write(
        &fixture.source.join("Catalogs/Zulu.xml"),
        &catalog.replace("<Name>Items</Name>", "<Name>Zulu</Name>"),
    );
    let service = fixture.view_service();
    assert!(
        service
            .view(ViewRequest::new("main:CommonModule").unwrap())
            .ok
    );
    for at in ["main:Catalog.Zulu", "main:CommonModule.zulu"] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "{at}: {result:?}");
        assert_eq!(result.diagnostics[0]["code"], "not_found");
    }
}

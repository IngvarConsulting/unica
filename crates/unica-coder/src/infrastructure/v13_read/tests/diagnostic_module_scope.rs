use super::*;

#[test]
fn diagnostic_module_scope_preserves_exact_common_module_and_catalog_roles() {
    let fixture = RealReaderFixture::new();
    for (role, text) in [("Object", "// object\n"), ("Manager", "// manager\n")] {
        write(
            &fixture
                .source
                .join(format!("Catalogs/Items/Ext/{role}Module.bsl")),
            text,
        );
    }
    let authority = fixture.read_authority();
    for (at, relative) in [
        (
            "main:CommonModule.РеактивныйСервер",
            "CommonModules/РеактивныйСервер/Ext/Module.bsl",
        ),
        (
            "main:CommonModule.РеактивныйСервер.Body",
            "CommonModules/РеактивныйСервер/Ext/Module.bsl",
        ),
        (
            "main:Catalog.Items.Module.Object",
            "Catalogs/Items/Ext/ObjectModule.bsl",
        ),
        (
            "main:Catalog.Items.Module.Manager",
            "Catalogs/Items/Ext/ManagerModule.bsl",
        ),
    ] {
        let expected = fixture.source.join(relative);
        let before = fs::read(&expected).unwrap();
        let scope = authority
            .diagnostic_mapping(&QualifiedAddress::parse(at).unwrap(), &fixture.context)
            .map(|mapping| {
                mapping
                    .module_scope()
                    .expect("a module mapping has its scope")
            })
            .unwrap();
        assert_eq!(scope.source_set, "main", "{at}");
        assert_eq!(scope.source_root, fixture.source, "{at}");
        assert_eq!(scope.module_path, expected, "{at}");
        assert_eq!(fs::read(expected).unwrap(), before, "{at}");
    }
}

#[test]
fn diagnostic_module_scope_refuses_foreign_source_object_and_unregistered_module() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("CommonModules/Orphan/Ext/Module.bsl"),
        "// not registered\n",
    );
    let authority = fixture.read_authority();
    for at in [
        "other:CommonModule.РеактивныйСервер",
        "main:Catalog.Items",
        "main:CommonModule.Orphan",
        "main:Catalog.Missing.Module.Manager",
    ] {
        assert!(
            authority
                .diagnostic_mapping(&QualifiedAddress::parse(at).unwrap(), &fixture.context)
                .map(|mapping| mapping
                    .module_scope()
                    .expect("a module mapping has its scope"))
                .is_err(),
            "invented module scope for {at}"
        );
    }
}

/// #1966: объект и корень проверяются как цель целиком — без модульной
/// области и без выдуманного объекта; общий модуль как объект — тоже объект.
#[test]
fn diagnostic_node_mapping_proves_object_common_module_and_root_without_module_scope() {
    use unica_address::source_target::TargetKind;
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    for (at, metadata_path, kind) in [
        (
            "main:Catalog.Items",
            Some("Catalog.Items"),
            TargetKind::MetadataObject,
        ),
        (
            "main:CommonModule.РеактивныйСервер",
            Some("CommonModule.РеактивныйСервер"),
            TargetKind::MetadataObject,
        ),
        ("main:Configuration", None, TargetKind::SourceRoot),
    ] {
        let mapping = authority
            .diagnostic_node_mapping(&QualifiedAddress::parse(at).unwrap(), &fixture.context)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
        assert_eq!(
            mapping
                .target()
                .metadata_path
                .as_ref()
                .map(MetadataAddress::as_str),
            metadata_path,
            "{at}"
        );
        assert_eq!(mapping.target().target_kind, kind, "{at}");
        assert!(mapping.module_scope().is_none(), "{at}");
    }
    for at in [
        "main:Catalog.Missing",
        "other:Catalog.Items",
        "main:CommonModule.РеактивныйСервер.Body",
    ] {
        assert!(
            authority
                .diagnostic_node_mapping(&QualifiedAddress::parse(at).unwrap(), &fixture.context)
                .is_err(),
            "invented node target for {at}"
        );
    }
}

#[test]
fn diagnostic_module_scope_keeps_admitted_source_after_project_remap() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер.Body").unwrap();
    let admitted = authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| {
            mapping
                .module_scope()
                .expect("a module mapping has its scope")
        })
        .unwrap();
    let replacement = fixture.root.path().join("replacement");
    write(
        &replacement.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        "// replacement only\n",
    );
    fs::write(fixture.root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n").unwrap();
    let retained = authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| {
            mapping
                .module_scope()
                .expect("a module mapping has its scope")
        })
        .unwrap();
    assert_eq!(retained.source_root, admitted.source_root);
    assert_eq!(retained.module_path, admitted.module_path);
    assert_ne!(retained.source_root, replacement);
}

#[test]
fn diagnostic_module_scope_refuses_replaced_retained_root() {
    if !supports_retained_root_replacement_test() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
        return;
    }
    let fixture = RealReaderFixture::new();
    let authority = fixture.operation_read_authority();
    let at = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping
            .module_scope()
            .expect("a module mapping has its scope"))
        .is_ok());
    fs::rename(&fixture.source, fixture.root.path().join("saved-source")).unwrap();
    fs::create_dir(&fixture.source).unwrap();
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping
            .module_scope()
            .expect("a module mapping has its scope"))
        .is_err());
}

#[test]
fn diagnostic_module_scope_refuses_module_replaced_by_foreign_symlink() {
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    let at = QualifiedAddress::parse("main:CommonModule.РеактивныйСервер").unwrap();
    let module = authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| {
            mapping
                .module_scope()
                .expect("a module mapping has its scope")
        })
        .unwrap()
        .module_path;
    let foreign = fixture.root.path().join("foreign.bsl");
    fs::write(&foreign, "// foreign module\n").unwrap();
    fs::remove_file(&module).unwrap();
    let outcome = create_file_link_fixture_for_test(&foreign, &module)
        .expect("unexpected file-link creation error must fail the fixture test");
    if outcome != FileLinkFixtureOutcome::Created {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::Symlink,
            "creating a link fixture needs the symlink privilege",
        );
        return;
    }
    assert!(authority
        .diagnostic_mapping(&at, &fixture.context)
        .map(|mapping| mapping
            .module_scope()
            .expect("a module mapping has its scope"))
        .is_err());
    assert_eq!(fs::read_to_string(foreign).unwrap(), "// foreign module\n");
}

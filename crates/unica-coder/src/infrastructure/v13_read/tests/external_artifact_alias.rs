use super::*;

#[test]
fn external_artifact_aliases_preserve_declared_owner_and_all_resource_paths() {
    let fixture = RealExternalReaderFixture::new();
    for (root, source, source_kind, kind, name, stem) in [
        (
            &fixture.processor,
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor",
            "Import",
            "source",
        ),
        (
            &fixture.report,
            "artifact_report",
            SourceSetKind::ExternalReport,
            "ExternalReport",
            "Sales",
            "arbitrary-export",
        ),
    ] {
        fs::rename(
            root.join(format!("{name}.xml")),
            root.join(format!("{stem}.xml")),
        )
        .unwrap();
        fs::rename(root.join(name), root.join(stem)).unwrap();
        let before = fs::read(root.join(format!("{stem}.xml"))).unwrap();
        let service = ViewService::new(
            fixture.read_authority(source, source_kind),
            ViewCursorStore::default(),
        );
        for suffix in [
            "",
            ".Module.Object",
            ".Form.Main",
            ".Form.Main.Metadata",
            ".Form.Main.Module.Form",
            ".Command.Run.Module.Command",
        ] {
            let at = format!("{source}:{kind}.{name}{suffix}");
            let result = service.view(ViewRequest::new(&at).unwrap());
            assert!(result.ok, "{at}: {:?}", result.diagnostics);
        }
        assert_eq!(fs::read(root.join(format!("{stem}.xml"))).unwrap(), before);
    }
}

#[test]
fn external_artifact_alias_map_never_falls_back_to_another_owners_physical_stem() {
    let fixture = RealExternalReaderFixture::new();
    fs::rename(
        fixture.processor.join("Import.xml"),
        fixture.processor.join("export.xml"),
    )
    .unwrap();
    fs::rename(
        fixture.processor.join("Import"),
        fixture.processor.join("export"),
    )
    .unwrap();
    fs::rename(
        fixture.processor.join("Импорт.xml"),
        fixture.processor.join("Import.xml"),
    )
    .unwrap();
    fs::rename(
        fixture.processor.join("Импорт"),
        fixture.processor.join("Import"),
    )
    .unwrap();
    let service = ViewService::new(
        fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor),
        ViewCursorStore::default(),
    );
    for name in ["Import", "Импорт"] {
        let at = format!("artifact_processor:ExternalDataProcessor.{name}");
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(result.ok, "{at}: {:?}", result.diagnostics);
    }
}

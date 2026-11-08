use super::*;

/// `Body` отдаёт исходные строки с их номерами: среза по контексту у `view`
/// нет (#1374), и клиентский код формы остаётся в ответе.
/// rule: INV.SOURCE.MODULE-COMPILATION-AND-BODY
#[test]
fn module_body_returns_the_source_text_byte_for_byte() {
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let at = "main:Report.ParityReport.Form.MainForm.Module.Form.Body";
    let original = fs::read_to_string(&module).unwrap();
    // Начальные U+FEFF снимаются, CRLF, табуляция, кавычки и отсутствие
    // разделителя в конце файла сохраняются как есть.
    let crlf = "\u{feff}// Комментарий \"формы\".\r\nПроцедура А()\r\n\tВозврат;\r\nКонецПроцедуры";
    for source in [original.as_str(), crlf] {
        fs::write(&module, source).unwrap();
        let result = fixture.view_service().view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "module Body must be read");
        assert_eq!(
            result.source_text.as_deref(),
            Some(source.trim_start_matches('\u{feff}'))
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["at"], at);
        assert_eq!(data["kind"], "Body");
        assert!(data.get("items").is_none());
        assert!(result.cursor.is_none());
    }
}

/// rule: INV.WIRE.SOURCE-TEXT-RESPONSE-BUDGET
#[test]
fn view_never_returns_module_text_above_the_largest_host_budget() {
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let at = "main:Report.ParityReport.Form.MainForm.Module.Form.Body";
    // 72 КиБ — наибольший бюджет известного хоста; число записано здесь
    // явно, чтобы смена предела в коде была видна этой проверке.
    let mut source = vec![b'a'; 73_728];
    fs::write(&module, &source).unwrap();
    let whole = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert_eq!(whole.source_text.as_ref().map(String::len), Some(73_728));
    source.push(b'a');
    fs::write(&module, &source).unwrap();
    let refused = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert_eq!(refused.diagnostics[0]["code"], "result_too_large");
    assert!(refused.source_text.is_none());
}

/// rule: INV.SOURCE.MODULE-COMPILATION-AND-BODY
/// rule: INV.WIRE.SOURCE-TEXT-RESPONSE-BUDGET
#[test]
fn module_body_beyond_the_limit_is_still_checked_as_utf8_in_full() {
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let at = "main:Report.ParityReport.Form.MainForm.Module.Form.Body";
    let mut source = vec![b'a'; crate::application::v13::source_text::MAX_SOURCE_TEXT_BYTES + 1];
    let valid = fixture.view_service();
    fs::write(&module, &source).unwrap();
    let oversized = valid.view(ViewRequest::new(at).unwrap());
    assert_eq!(oversized.diagnostics[0]["code"], "result_too_large");
    // Неверный байт за пределом не теряется: отказ называет испорченный
    // исходник, а не размер.
    source.push(0xff);
    fs::write(&module, &source).unwrap();
    let broken = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert!(!broken.ok);
    assert_ne!(
        broken.diagnostics[0]["code"], "result_too_large",
        "a broken byte beyond the limit must not be reported as size"
    );
    assert!(broken.source_text.is_none());
}

/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
#[test]
fn missing_owner_module_branch_is_not_invented_but_registered_owner_without_bsl_is_kept() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let missing = service.view(ViewRequest::new("main:Document.Missing.Module").unwrap());
    assert!(
        !missing.ok,
        "missing owner produced a module branch: {missing:?}"
    );
    assert_eq!(missing.diagnostics[0]["code"], "not_found");

    let registered = service.view(ViewRequest::new("main:Catalog.Items.Module").unwrap());
    assert!(registered.ok, "{:?}", registered.diagnostics);
    let items = registered.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["at"], "main:Catalog.Items.Module.Object");
    assert_eq!(items[1]["at"], "main:Catalog.Items.Module.Manager");
}

/// rule: INV.SOURCE.MODULE-CURRENT-SOURCE
#[test]
pub(crate) fn one_read_parses_each_module_source_once_per_actor_revision() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    assert_reader_reaches(
        &authority,
        &[
            "main:CommonModule.РеактивныйСервер.Method",
            "main:Catalog.Items",
            "main:Catalog.Items.TabularSection.Lines",
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
            "main:Configuration",
        ],
    );
    assert_eq!(
        authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        1,
        "one actor-owned revision must parse each module source once",
    );
    assert_eq!(
        authority.configuration_payload_read_count(),
        1,
        "one actor-owned revision must parse its source-set inventory once",
    );

    let module = fixture
        .source
        .join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
    let mut changed = fs::read_to_string(&module).unwrap();
    changed.push_str("\nProcedure AfterRevisionChange()\nEndProcedure\n");
    fs::write(&module, changed).unwrap();
    assert_reader_reaches(
        &authority,
        &[
            "main:CommonModule.РеактивныйСервер.Method",
            "main:Catalog.Items",
            "main:Catalog.Items.TabularSection.Lines",
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
            "main:Configuration",
        ],
    );
    assert_eq!(
        authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        1,
        "one read keeps its own projection; it is not a snapshot of later edits",
    );
    assert_eq!(authority.configuration_payload_read_count(), 1);
    let changed_methods = ViewService::new(fixture.read_authority(), ViewCursorStore::default())
        .view(ViewRequest::new("main:CommonModule.РеактивныйСервер.Method").unwrap());
    assert!(
        changed_methods.ok,
        "a new read of the changed module must succeed"
    );
    assert!(
        serde_json::to_string(&changed_methods.data)
            .unwrap()
            .contains("AfterRevisionChange"),
        "a new read must parse the changed module source"
    );

    let second_authority = fixture.read_authority();
    assert_reader_reaches(
        &second_authority,
        &[
            "main:CommonModule.РеактивныйСервер.Method",
            "main:Catalog.Items",
            "main:Catalog.Items.TabularSection.Lines",
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
            "main:Configuration",
        ],
    );
    assert_eq!(
        second_authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        1,
        "a second actor must own an independent operation-local projection",
    );
    assert_eq!(second_authority.configuration_payload_read_count(), 1);
}

#[test]
fn an_open_buffer_is_read_in_place_of_its_module_file_and_the_file_is_untouched() {
    use unica_workspace::documents::{DocumentOverlay, DocumentText};

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().canonicalize().unwrap();
    let module = Path::new("CommonModules/Общий/Ext/Module.bsl");
    fs::create_dir_all(workspace.as_path().join(module).parent().unwrap()).unwrap();
    fs::write(workspace.as_path().join(module), "\u{feff}// диск\n").unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonModule.Общий.Module")
            .unwrap();
    let absent =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonModule.Новый.Module")
            .unwrap();
    let overlay = DocumentOverlay::default();
    for relative in [module, Path::new("CommonModules/Новый/Ext/Module.bsl")] {
        overlay
            .put(
                "editor",
                "main",
                relative,
                DocumentText {
                    version: 3,
                    text: Arc::from("\u{feff}// буфер\n// вторая\n"),
                },
            )
            .unwrap();
    }
    let reader = |documents| {
        let root =
            Arc::new(unica_workspace::read::open_source_directory(workspace.as_path()).unwrap());
        let reader =
            ProviderReadAuthority::new("main", "actor-main", SourceSetKind::Configuration, root);
        match documents {
            Some(documents) => reader.with_documents(documents),
            None => reader,
        }
    };

    let editor = reader(Some(overlay.session_documents("editor", "main").unwrap()));
    assert_eq!(
        editor.module_source(&target).unwrap().as_deref(),
        Some("// буфер\n// вторая\n")
    );
    assert!(editor.module_source_exists(&absent).unwrap());
    assert_eq!(
        editor.module_source(&absent).unwrap().as_deref(),
        Some("// буфер\n// вторая\n")
    );

    // Every read of the module in this session sees the same buffer.
    let lines = editor.module_line_count(&target, || Ok(())).unwrap();
    assert_eq!(
        lines,
        reader(None)
            .module_line_count(&target, || Ok(()))
            .unwrap()
            .map(|n| n + 1)
    );

    // Another session and an agent call read the disk.
    for other in [
        Some(overlay.session_documents("agent", "main").unwrap()),
        None,
    ] {
        let other = reader(other);
        assert_eq!(
            other.module_source(&target).unwrap().as_deref(),
            Some("// диск\n")
        );
        assert!(!other.module_source_exists(&absent).unwrap());
    }
    assert_eq!(
        fs::read_to_string(workspace.as_path().join(module)).unwrap(),
        "\u{feff}// диск\n"
    );
    assert!(!workspace
        .as_path()
        .join("CommonModules/Новый")
        .join("Ext/Module.bsl")
        .exists());
}

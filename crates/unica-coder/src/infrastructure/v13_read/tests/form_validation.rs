use super::*;

#[test]
fn form_validation_input_keeps_the_authority_cancellation_and_deadline() {
    let fixture = RealReaderFixture::new();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Form.MainForm").unwrap();
    let mut authority = fixture.read_authority();
    authority.deadline = ProviderDeadline::from_budget(std::time::Duration::ZERO);
    assert_eq!(
        authority
            .form_validation_input(&address)
            .err()
            .unwrap()
            .code(),
        RefusalCode::DeadlineExceeded
    );
    let authority = fixture.read_authority();
    fixture.cancellation.cancel();
    assert_eq!(
        authority
            .form_validation_input(&address)
            .err()
            .unwrap()
            .code(),
        RefusalCode::Cancelled
    );
}

#[test]
fn form_validation_input_does_not_read_a_module_without_checkable_bindings() {
    let fixture = RealExternalReaderFixture::new();
    let address =
        QualifiedAddress::parse("artifact_processor:ExternalDataProcessor.Import.Form.Main")
            .unwrap();
    let body = fixture.processor.join("Import/Forms/Main/Ext/Form.xml");
    let module = fixture
        .processor
        .join("Import/Forms/Main/Ext/Form/Module.bsl");
    if module.exists() {
        fs::remove_file(&module).unwrap();
    }
    // Каталог вместо файла доказывает отсутствие обращения к ненужному модулю.
    fs::create_dir_all(&module).unwrap();
    for text in [
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><AutoCommandBar name="Панель" id="-1"/><ChildItems/></Form>"#,
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><Events><Event name="UnknownEvent">Обработчик</Event></Events></Form>"#,
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><Events><Event name="OnOpen" callType="Before">ПриОткрытии</Event></Events></Form>"#,
        "<Form>",
    ] {
        write(&body, text);
        let input = fixture
            .read_authority("artifact_processor", SourceSetKind::ExternalProcessor)
            .form_validation_input(&address)
            .unwrap();
        assert!(input.module.is_none());
    }
}

#[test]
fn form_validation_input_refuses_linked_form_and_needed_module() {
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    for linked in ["Form.xml", "Form/Module.bsl"] {
        let fixture = RealExternalReaderFixture::new();
        let address =
            QualifiedAddress::parse("artifact_processor:ExternalDataProcessor.Import.Form.Main")
                .unwrap();
        let ext = fixture.processor.join("Import/Forms/Main/Ext");
        write(
            &ext.join("Form.xml"),
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><Events><Event name="OnOpen">ПриОткрытии</Event></Events></Form>"#,
        );
        write(
            &ext.join("Form/Module.bsl"),
            "&НаКлиенте\nПроцедура ПриОткрытии(Отказ)\nКонецПроцедуры\n",
        );
        let path = ext.join(linked);
        let foreign = fixture.context.cwd.join("foreign");
        fs::rename(&path, &foreign).unwrap();
        let before = fs::read(&foreign).unwrap();
        let outcome = create_file_link_fixture_for_test(&foreign, &path)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        assert!(
            fixture
                .read_authority("artifact_processor", SourceSetKind::ExternalProcessor)
                .form_validation_input(&address)
                .is_err(),
            "{linked}"
        );
        assert_eq!(fs::read(&foreign).unwrap(), before);
    }
}

#[test]
fn canonical_form_validation_reports_captured_malformed_xml() {
    let fixture = RealExternalReaderFixture::new();
    let address =
        QualifiedAddress::parse("artifact_processor:ExternalDataProcessor.Import.Form.Main")
            .unwrap();
    write(
        &fixture.processor.join("Import/Forms/Main/Ext/Form.xml"),
        "<Form>",
    );
    let input = fixture
        .read_authority("artifact_processor", SourceSetKind::ExternalProcessor)
        .form_validation_input(&address)
        .unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Form",
        crate::application::v13::check::CheckValidator::Form,
        crate::infrastructure::native_operations::v13_analysis::validate_form_input(input),
    )
    .unwrap();
    assert!(!checked.ok());
    assert!(checked
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.severity() == "error"
            && diagnostic.message().contains("XML parse error")));
}

#[test]
fn retained_form_check_ignores_source_set_remapping() {
    let fixture = RealExternalReaderFixture::new();
    let address =
        QualifiedAddress::parse("artifact_processor:ExternalDataProcessor.Import.Form.Main")
            .unwrap();
    let body = fixture.processor.join("Import/Forms/Main/Ext/Form.xml");
    let original = fs::read_to_string(&body).unwrap();
    let authority = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let admitted = authority.form_validation_input(&address).unwrap();
    assert_eq!(admitted.text, original);

    // Новый корень содержит отличающуюся форму; право чтения остаётся прежним.
    let replacement = fixture.context.cwd.join("replacement");
    write(
        &replacement.join("Import.xml"),
        &fs::read_to_string(fixture.processor.join("Import.xml")).unwrap(),
    );
    write(
        &replacement.join("Import/Forms/Main.xml"),
        &fs::read_to_string(fixture.processor.join("Import/Forms/Main.xml")).unwrap(),
    );
    write(
        &replacement.join("Import/Forms/Main/Ext/Form.xml"),
        "<Form/>",
    );
    fs::write(
        fixture.context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: artifact_processor\n    type: EXTERNAL_DATA_PROCESSORS\n    path: replacement\n",
    )
    .unwrap();
    let retained = authority.form_validation_input(&address).unwrap();
    assert_eq!(retained.text, original);
    assert_eq!(retained.artifact, admitted.artifact);
    assert!(!retained.config_context);
}

#[test]
fn canonical_form_validation_keeps_captured_xml_module_and_owner() {
    let fixture = RealExternalReaderFixture::new();
    for (source_set, kind, root, owner) in [
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            &fixture.processor,
            "ExternalDataProcessor.Import",
        ),
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            &fixture.report,
            "ExternalReport.Sales",
        ),
    ] {
        let name = owner.split('.').nth(1).unwrap();
        let body = root.join(format!("{name}/Forms/Main/Ext/Form.xml"));
        let module = root.join(format!("{name}/Forms/Main/Ext/Form/Module.bsl"));
        write(
            &body,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><AutoCommandBar name="КоманднаяПанель" id="-1"/><Events><Event name="OnOpen">ПриОткрытии</Event></Events><ChildItems/></Form>"#,
        );
        // Предел выдачи текста не ограничивает вход проверки: обработчик
        // после этого предела всё равно должен участвовать в диагностике.
        let padding =
            "// Заполнение\n".repeat(crate::application::v13::source_text::MAX_SOURCE_TEXT_BYTES);
        write(&module, &format!("{padding}&НаКлиенте\nПроцедура ПриОткрытии(Отказ, Лишний, ЕщёОдин)\nКонецПроцедуры\n"));
        let address = QualifiedAddress::parse(&format!("{source_set}:{owner}.Form.Main")).unwrap();
        let input = fixture
            .read_authority(source_set, kind)
            .form_validation_input(&address)
            .unwrap();

        // Валидатор обязан использовать захваченные байты даже после замены всех входов.
        write(&body, "это уже не XML");
        write(
            &module,
            "&НаКлиенте\nПроцедура ПриОткрытии(Отказ)\nКонецПроцедуры\n",
        );
        write(&root.join(format!("{name}.xml")), "описание заменено");
        write(
            &root.join(format!("{name}/Forms/Main.xml")),
            "описание формы заменено",
        );
        let checked = crate::application::v13::check::normalize_native_outcome(
            &address,
            "Form",
            crate::application::v13::check::CheckValidator::Form,
            crate::infrastructure::native_operations::v13_analysis::validate_form_input(input),
        )
        .unwrap();
        assert!(checked.ok());
        assert!(checked
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.severity() == "warning"
                && diagnostic.message().contains("ПриОткрытии")));
        assert_eq!(fs::read_to_string(&body).unwrap(), "это уже не XML");
    }
}

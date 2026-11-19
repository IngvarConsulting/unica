use super::*;
use std::time::Duration;

#[test]
fn notification_bindings_read_registered_candidates_in_the_selected_source_set() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/acceptance/workspace-notification-binding")
        .canonicalize()
        .unwrap();
    let cancellation = CancellationToken::new();
    for (set, directory, kind, expected) in [
        (
            "main",
            "src",
            SourceSetKind::Configuration,
            vec!["main:Catalog.Команды.Form.ФормаСписка.Module.Form.Method.Start"],
        ),
        ("ext", "src-ext", SourceSetKind::Extension, Vec::new()),
    ] {
        let root = Arc::new(
            unica_workspace::read::open_source_directory(&fixture.join(directory)).unwrap(),
        );
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            set,
            set,
            kind,
            root,
            PlatformProfile::v8_3_27(),
        );
        let index = Arc::clone(authority.read.artifacts().names());
        let service = ViewService::new(authority, ViewCursorStore::default());
        let method = format!("{set}:CommonModule.Notifications.Method.Finish");
        let branch = format!("{method}.Binding");

        let data = service
            .view(ViewRequest::new(&method).unwrap())
            .data
            .unwrap();
        assert!(data["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at": branch})));
        assert!(!index.wait_ready_for_test(Duration::from_millis(5)));
        let first = service
            .view(ViewRequest::new(&branch).unwrap())
            .data
            .unwrap();
        assert_eq!(first["props"]["notificationIndex"], "indexing");
        assert!(index.wait_ready_for_test(Duration::from_secs(10)));

        let fresh = service
            .view(ViewRequest::new(&branch).unwrap())
            .data
            .unwrap();
        let items = fresh["items"].as_array().unwrap();
        let origins = items
            .iter()
            .map(|item| item["at"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(origins, expected);
        if let [item] = items.as_slice() {
            assert_eq!(item["range"]["start"], json!({"line": 8, "character": 13}));
        }
    }
}

#[test]
// rule: INV.SOURCE.BSL-SYNTAX-ERROR-READING
fn notification_bindings_refuse_a_form_with_parse_errors() {
    let fixture = RealReaderFixture::new();
    let module = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    fs::write(
        &module,
        concat!(
            "&AtClient\nProcedure Finish() Export\nEndProcedure\n",
            "&AtClient\nProcedure Start()\n  Callback = New NotifyDescription(\"Finish, ThisObject\nEndProcedure\n"
        ),
    )
    .unwrap();
    let service = fixture.view_service();
    let binding = "main:Report.ParityReport.Form.MainForm.Module.Form.Method.Finish.Binding";
    let answer = service.view(ViewRequest::new(binding).unwrap());
    assert!(!answer.ok);
    assert_eq!(answer.diagnostics[0]["code"], "provider_unavailable");
    assert_eq!(answer.diagnostics[0]["detailCode"], "source_unreadable");
    assert!(answer.data.is_none());
    let body = "main:Report.ParityReport.Form.MainForm.Module.Form.Body";
    assert!(answer.next.iter().any(|next| next["args"]["at"] == body));
    let text = fs::read_to_string(&module).unwrap();
    let result = service.view(ViewRequest::new(body).unwrap());
    assert!(result.ok, "{result:?}");
    assert_eq!(result.source_text.as_deref(), Some(text.as_str()));
    assert_eq!(result.diagnostics[0]["code"], "bsl-parse-error");
    assert_eq!(fs::read_to_string(&module).unwrap(), text);
}

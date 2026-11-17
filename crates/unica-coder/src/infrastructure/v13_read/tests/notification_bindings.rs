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

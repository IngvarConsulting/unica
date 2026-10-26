//! Служба LSP демона над рабочей областью на диске: те же актор и наборы,
//! что у MCP, буфер виден только своей сессии и не пишется на диск.
use super::*;
use std::fs;
use std::path::Path;
use std::time::Duration;

const MODULE: &str = "src/CommonModules/Main/Ext/Module.bsl";
const DISK: &str = "Procedure Disk() EndProcedure\n";

fn workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::create_dir_all(root.join(MODULE).parent().unwrap()).unwrap();
    fs::write(
        root.join("src/Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><CommonModule>Main</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(root.join(MODULE), DISK).unwrap();
    workspace
}

fn file_uri(path: &Path) -> String {
    url::Url::from_file_path(path.canonicalize().unwrap())
        .unwrap()
        .into()
}

fn folder(path: &Path, name: &str) -> V5LspWorkspaceFolder {
    V5LspWorkspaceFolder {
        uri: file_uri(path),
        name: name.to_owned(),
    }
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn client() -> LspClientChannel {
    unica_daemon::lsp_lane::LspClientChannel::detached_for_test()
}

fn content(host: &UnicaLspHost, session: &V5LspSessionId, uri: &str) -> V5LspOutcome {
    host.call(
        session,
        V5LspCall::TextDocumentContent {
            uri: uri.to_owned(),
        },
        &LspCallContext::for_test(soon()),
    )
}

fn text(text: &str) -> V5LspOutcome {
    V5LspOutcome::Text {
        text: text.to_owned(),
    }
}

#[test]
fn an_editor_folder_reaches_the_mcp_actor_and_its_buffer_stays_in_its_session() {
    let workspace = workspace();
    let actors = Arc::new(crate::infrastructure::source_workspace::actor_registry());
    let host = UnicaLspHost::new(Arc::clone(&actors));
    let (editor, other) = (V5LspSessionId::new(), V5LspSessionId::new());
    let folders = [folder(workspace.path(), "Work")];

    let opening = host.open(&editor, &folders, client(), soon()).unwrap();
    host.open(&other, &folders, client(), soon()).unwrap();

    assert!(opening.capabilities.text_document_content);
    assert_eq!(
        opening.workspaces,
        vec![V5LspWorkspaceSpace::opened(&folders[0].uri, "work")]
    );
    let module = file_uri(&workspace.path().join(MODULE));
    let uri = {
        let session = host.session(&editor).unwrap();
        let session = lock(&session);
        session.documents.uri_of_file(&module).unwrap()
    };
    assert!(uri.starts_with("unica://work/"), "{uri}");
    assert_eq!(content(&host, &editor, &uri), text(DISK));
    // The editor may name the module by its file as well.
    assert_eq!(content(&host, &editor, &module), text(DISK));

    host.notify(
        &editor,
        V5LspNotification::DidOpen {
            uri: module.clone(),
            language_id: "bsl".into(),
            version: 1,
            text: "Procedure Buffer() EndProcedure\n".into(),
        },
        soon(),
    )
    .unwrap();
    host.notify(
        &editor,
        V5LspNotification::DidChange {
            uri: uri.clone(),
            version: 2,
            changes: vec![V5LspContentChange {
                range: None,
                text: "Procedure Edited() EndProcedure\n".into(),
            }],
        },
        soon(),
    )
    .unwrap();

    assert_eq!(
        content(&host, &editor, &uri),
        text("Procedure Edited() EndProcedure\n")
    );
    assert_eq!(
        content(&host, &other, &uri),
        text(DISK),
        "another session reads the disk"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join(MODULE)).unwrap(),
        DISK
    );
    // MCP admission of the same workspace reaches the actor holding the buffer.
    let mcp = admit(&actors, workspace.path()).unwrap();
    let session = host.session(&editor).unwrap();
    let editor_actor = Arc::clone(&lock(&session).spaces["work"].actor);
    assert!(Arc::ptr_eq(&mcp.actor, &editor_actor));
    assert!(!editor_actor
        .documents()
        .session_documents(editor.as_str(), "main")
        .unwrap()
        .is_empty());

    host.close(&editor);

    assert!(editor_actor
        .documents()
        .session_documents(editor.as_str(), "main")
        .unwrap()
        .is_empty());
    assert_eq!(
        content(&host, &editor, &uri),
        V5LspOutcome::Failed {
            failure: V5LspFailure::SessionUnknown
        }
    );
}

#[test]
fn a_folder_without_sources_or_off_disk_gets_a_failure_and_no_space() {
    let workspace = workspace();
    let empty = tempfile::tempdir().unwrap();
    let host = UnicaLspHost::new(Arc::new(
        crate::infrastructure::source_workspace::actor_registry(),
    ));
    let session = V5LspSessionId::new();
    let folders = [
        folder(empty.path(), "Work"),
        V5LspWorkspaceFolder {
            uri: "vscode-vfs://github/repo".into(),
            name: "Remote".into(),
        },
        folder(workspace.path(), "Work"),
    ];

    let opening = host.open(&session, &folders, client(), soon()).unwrap();

    assert_eq!(
        opening.workspaces,
        vec![
            V5LspWorkspaceSpace::refused(&folders[0].uri, V5LspFailure::WorkspaceUnavailable),
            V5LspWorkspaceSpace::refused(&folders[1].uri, V5LspFailure::WorkspaceUnavailable),
            V5LspWorkspaceSpace::opened(&folders[2].uri, "work"),
        ],
        "a refused folder takes no name"
    );
    assert_eq!(
        content(&host, &session, "unica://other/main/CommonModule.Main"),
        V5LspOutcome::Failed {
            failure: V5LspFailure::DocumentUnknown
        }
    );
}

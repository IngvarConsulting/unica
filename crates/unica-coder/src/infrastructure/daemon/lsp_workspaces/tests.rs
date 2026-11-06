//! Служба LSP демона над рабочей областью на диске: те же актор и наборы,
//! что у MCP, буфер виден только своей сессии и не пишется на диск.
use super::*;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use unica_daemon::lsp_lane::{LspCallContext, LspClientChannel, LspHost};
use unica_daemon::protocol_v5::lsp::{
    V5LspCall, V5LspContentChange, V5LspNotification, V5LspOutcome, V5LspSessionId,
    V5LspWorkspaceFolder, V5LspWorkspaceSpace,
};
use unica_lsp::UnicaLspHost;

const MODULE: &str = "src/CommonModules/Main/Ext/Module.bsl";
const DISK: &str = "Procedure Disk() EndProcedure\n";
const BODY_URI: &str = "unica://work/main/CommonModule.Main.Body";

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

/// The daemon's composition: one actor registry behind MCP and the service.
fn service() -> (UnicaLspHost, Arc<ProductLspWorkspaces>) {
    let workspaces = Arc::new(ProductLspWorkspaces::new(Arc::new(
        crate::infrastructure::source_workspace::actor_registry(),
    )));
    (UnicaLspHost::new(workspaces.clone()), workspaces)
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn client() -> LspClientChannel {
    LspClientChannel::detached_for_test()
}

fn content(host: &UnicaLspHost, session: &V5LspSessionId, uri: &str) -> V5LspOutcome {
    ask(
        host,
        session,
        V5LspCall::TextDocumentContent {
            uri: uri.to_owned(),
        },
    )
}

fn ask(host: &UnicaLspHost, session: &V5LspSessionId, call: V5LspCall) -> V5LspOutcome {
    host.call(session, call, &LspCallContext::for_test(soon()))
}

fn text(text: &str) -> V5LspOutcome {
    V5LspOutcome::Text {
        text: text.to_owned(),
    }
}

#[test]
fn an_editor_folder_reaches_the_mcp_actor_and_its_buffer_stays_in_its_session() {
    let workspace = workspace();
    let (host, workspaces) = service();
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
    let uri = BODY_URI.to_owned();
    assert_eq!(content(&host, &editor, &uri), text(DISK));
    // A file path names no document: only `unica://` does.
    assert_eq!(
        content(&host, &editor, &module),
        V5LspOutcome::Failed {
            failure: V5LspFailure::DocumentUnknown
        }
    );

    host.notify(
        &editor,
        V5LspNotification::DidOpen {
            uri: uri.clone(),
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
    let admitted = workspaces.admit(workspace.path(), soon()).unwrap();
    // The analysis read of the editor's session sees the buffer as well, and
    // the read of another session sees the file.
    let module_target = MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "CommonModule.Main.Module",
    )
    .unwrap();
    let module_of = |session: &V5LspSessionId| {
        admitted
            .reads
            .read(session.as_str(), "main")
            .unwrap()
            .module_source(&module_target)
            .unwrap()
    };
    assert_eq!(
        module_of(&editor).as_deref(),
        Some("Procedure Edited() EndProcedure\n")
    );
    assert_eq!(module_of(&other).as_deref(), Some(DISK));
    let mcp = admitted.actor.clone();
    assert!(!mcp
        .documents()
        .session_documents(editor.as_str(), "main")
        .unwrap()
        .is_empty());

    host.close(&editor);

    assert!(mcp
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
    let (host, _workspaces) = service();
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
        content(&host, &session, "unica://other/main/CommonModule.Main.Body"),
        V5LspOutcome::Failed {
            failure: V5LspFailure::DocumentUnknown
        }
    );
}

fn open_buffer(host: &UnicaLspHost, session: &V5LspSessionId, uri: &str, text: &str) {
    host.notify(
        session,
        V5LspNotification::DidOpen {
            uri: uri.to_owned(),
            language_id: "bsl".into(),
            version: 1,
            text: text.to_owned(),
        },
        soon(),
    )
    .unwrap();
}

/// A replaced source root makes the registry give MCP a new actor. The
/// session follows it there with its buffers instead of holding MCP on the
/// old one.
#[test]
fn after_the_source_root_is_replaced_the_session_moves_its_buffers_to_the_mcp_actor() {
    let workspace = workspace();
    let (host, workspaces) = service();
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let uri = BODY_URI;
    open_buffer(&host, &editor, uri, "Procedure Buffer() EndProcedure\n");
    let before = Arc::downgrade(&workspaces.admit(workspace.path(), soon()).unwrap().actor);

    // A fresh export replaces the source directory with an equal one.
    let source = workspace.path().join("src");
    fs::rename(&source, workspace.path().join("src.old")).unwrap();
    fs::create_dir_all(source.join("CommonModules/Main/Ext")).unwrap();
    fs::copy(
        workspace.path().join("src.old/Configuration.xml"),
        source.join("Configuration.xml"),
    )
    .unwrap();
    fs::write(workspace.path().join(MODULE), DISK).unwrap();
    let mcp = workspaces
        .admit(workspace.path(), soon())
        .expect("MCP admits the new root")
        .actor;

    assert!(
        before.upgrade().is_none_or(|old| !Arc::ptr_eq(&old, &mcp)),
        "MCP is not held on the actor of the replaced root"
    );
    assert_eq!(
        content(&host, &editor, uri),
        text("Procedure Buffer() EndProcedure\n")
    );
    assert!(
        !mcp.documents()
            .session_documents(editor.as_str(), "main")
            .unwrap()
            .is_empty(),
        "the buffer moved into the new actor's overlay"
    );
}

#[test]
fn a_change_the_buffer_cannot_take_is_refused_so_the_editor_drops_it() {
    let workspace = workspace();
    let (host, _workspaces) = service();
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let uri = BODY_URI.to_owned();
    let change = |version, line| V5LspNotification::DidChange {
        uri: uri.clone(),
        version,
        changes: vec![V5LspContentChange {
            range: Some(unica_daemon::protocol_v5::lsp::V5LspRange {
                start: unica_daemon::protocol_v5::lsp::V5LspPosition { line, character: 0 },
                end: unica_daemon::protocol_v5::lsp::V5LspPosition { line, character: 0 },
            }),
            text: "x".into(),
        }],
    };

    assert_eq!(
        host.notify(&editor, change(2, 0), soon()),
        Err(V5LspFailure::DocumentUnknown),
        "a change of a document that is not open"
    );
    open_buffer(&host, &editor, &uri, DISK);
    assert_eq!(
        host.notify(&editor, change(2, 99), soon()),
        Err(V5LspFailure::DocumentUnknown),
        "a range outside the buffer"
    );
    // The refused document is closed: a later change does not land on a
    // buffer that missed one, and answers come from the file.
    assert_eq!(
        host.notify(&editor, change(3, 0), soon()),
        Err(V5LspFailure::DocumentUnknown)
    );
    assert_eq!(content(&host, &editor, &uri), text(DISK));
    open_buffer(&host, &editor, &uri, "Procedure Again() EndProcedure\n");
    assert_eq!(
        content(&host, &editor, &uri),
        text("Procedure Again() EndProcedure\n")
    );
}

/// Every form an editor may send for one module reaches it: the encoded
/// URI the service gives, its decoded text and an authority in any case.
#[test]
fn a_module_is_reached_by_every_spelling_of_its_unica_uri() {
    let workspace = workspace();
    let module = workspace
        .path()
        .join("src/CommonModules/ОбщегоНазначения/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "// УТ\n").unwrap();
    let (host, _workspaces) = service();
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "УТ Проект")],
        client(),
        soon(),
    )
    .unwrap();

    for uri in [
        "unica://ут%20проект/main/CommonModule.ОбщегоНазначения.Body",
        "unica://ут проект/main/CommonModule.ОбщегоНазначения.Body",
        "unica://УТ%20Проект/main/CommonModule.ОбщегоНазначения.Body",
        "unica://%D0%A3%D0%A2%20%D0%9F%D1%80%D0%BE%D0%B5%D0%BA%D1%82/main/CommonModule.ОбщегоНазначения.Body",
    ] {
        assert_eq!(content(&host, &editor, uri), text("// УТ\n"), "{uri}");
    }
}

/// The whole of a large module in one `workspace/textDocumentContent`.
/// Point `UNICA_LSP_UT_SOURCE` at a Designer dump of a large configuration
/// (its `src`, beside a `v8project.yaml`), run with `--ignored --nocapture`.
#[test]
#[ignore = "needs a large local Designer dump in UNICA_LSP_UT_SOURCE"]
fn a_large_common_module_comes_whole_in_one_request() {
    let Some(source) = std::env::var_os("UNICA_LSP_UT_SOURCE").map(PathBuf::from) else {
        return;
    };
    let source = source.canonicalize().unwrap();
    let (host, _workspaces) = service();
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(source.parent().unwrap(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let uri = "unica://work/main/CommonModule.ОбщегоНазначения.Body";
    let started = Instant::now();
    let answer = content(&host, &editor, uri);
    let elapsed = started.elapsed();
    let disk =
        fs::read_to_string(source.join("CommonModules/ОбщегоНазначения/Ext/Module.bsl")).unwrap();
    let disk = disk.trim_start_matches('\u{feff}');
    assert_eq!(answer, text(disk));
    println!(
        "textDocumentContent {uri}: {} lines, {} bytes in {:.1} ms",
        disk.lines().count(),
        disk.len(),
        elapsed.as_secs_f64() * 1000.0
    );
}

/// The module address `unica.resolve` gives a file, followed by `Body`, is
/// the address a document has.
#[test]
fn the_module_address_mcp_gives_a_file_with_body_is_its_document_address() {
    let workspace = workspace();
    let source = workspace.path().join("src");
    let (host, _workspaces) = service();
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    for relative in [
        "CommonModules/Main/Ext/Module.bsl",
        "Catalogs/Items/Ext/ObjectModule.bsl",
        "Catalogs/Items/Ext/ManagerModule.bsl",
        "Catalogs/Items/Forms/Item/Ext/Form/Module.bsl",
        "CommonForms/Dialog/Ext/Form/Module.bsl",
        "Ext/ManagedApplicationModule.bsl",
    ] {
        let path = source.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, relative).unwrap();
        let module = crate::infrastructure::event_sources::module_address_for_relative(
            "main",
            unica_source_contract::SourceSetKind::Configuration,
            unica_bsl::profile::PlatformProfile::v8_3_27(),
            Path::new(relative),
        )
        .unwrap();
        let logical = module.to_string().replacen(':', "/", 1);
        let uri = format!("unica://work/{logical}.Body");
        assert_eq!(content(&host, &editor, &uri), text(relative), "{uri}");
    }
}

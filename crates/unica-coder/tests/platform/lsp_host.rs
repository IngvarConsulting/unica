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
    fs::write(
        root.join("src/CommonModules/Main.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Main</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
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
        let session = read(&session);
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
    let (_, mcp, _) = admit_actor(&actors, workspace.path(), soon()).unwrap();
    let session = host.session(&editor).unwrap();
    let editor_actor = read(&session).spaces["work"].actor.upgrade().unwrap();
    assert!(Arc::ptr_eq(&mcp, &editor_actor));
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

fn session_uri(host: &UnicaLspHost, session: &V5LspSessionId, file: &str) -> String {
    let session = host.session(session).unwrap();
    let session = read(&session);
    session.documents.uri_of_file(file).unwrap()
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
    let actors = Arc::new(crate::infrastructure::source_workspace::actor_registry());
    let host = UnicaLspHost::new(Arc::clone(&actors));
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let uri = session_uri(&host, &editor, &file_uri(&workspace.path().join(MODULE)));
    open_buffer(&host, &editor, &uri, "Procedure Buffer() EndProcedure\n");
    let before = {
        let session = host.session(&editor).unwrap();
        let actor = read(&session).spaces["work"].actor.clone();
        actor
    };

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
    let (_, mcp, _) =
        admit_actor(&actors, workspace.path(), soon()).expect("MCP admits the new root");

    assert!(
        before.upgrade().is_none_or(|old| !Arc::ptr_eq(&old, &mcp)),
        "MCP is not held on the actor of the replaced root"
    );
    assert_eq!(
        content(&host, &editor, &uri),
        text("Procedure Buffer() EndProcedure\n")
    );
    let session = host.session(&editor).unwrap();
    let current = read(&session).spaces["work"].actor.upgrade().unwrap();
    assert!(
        Arc::ptr_eq(&current, &mcp),
        "the session follows MCP's actor"
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
    let host = UnicaLspHost::new(Arc::new(
        crate::infrastructure::source_workspace::actor_registry(),
    ));
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let uri = session_uri(&host, &editor, &file_uri(&workspace.path().join(MODULE)));
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
    let host = UnicaLspHost::new(Arc::new(
        crate::infrastructure::source_workspace::actor_registry(),
    ));
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "УТ Проект")],
        client(),
        soon(),
    )
    .unwrap();
    let given = session_uri(&host, &editor, &file_uri(&module));

    for uri in [
        given.as_str(),
        "unica://ут%20проект/main/CommonModule.ОбщегоНазначения",
        "unica://ут проект/main/CommonModule.ОбщегоНазначения",
        "unica://УТ%20Проект/main/CommonModule.ОбщегоНазначения",
        "unica://%D0%A3%D0%A2%20%D0%9F%D1%80%D0%BE%D0%B5%D0%BA%D1%82/main/CommonModule.ОбщегоНазначения",
    ] {
        assert_eq!(content(&host, &editor, uri), text("// УТ\n"), "{uri}");
    }
}

fn completion_host(platform_path: &str) -> (tempfile::TempDir, UnicaLspHost) {
    let workspace = workspace();
    fs::write(
        workspace.path().join("v8project.yaml"),
        format!(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  platform:\n    path: {platform_path}\n"
        ),
    )
    .unwrap();
    let host = UnicaLspHost::new(Arc::new(
        crate::infrastructure::source_workspace::actor_registry(),
    ));
    (workspace, host)
}

const COMPLETION_TEXT: &str = "Процедура Выполнить()\n    \nКонецПроцедуры\n\n// Помогает выполнить.\nПроцедура Помощник() Экспорт\nКонецПроцедуры\n";

fn complete(host: &UnicaLspHost, workspace: &Path) -> (V5LspSessionId, serde_json::Value) {
    let editor = V5LspSessionId::new();
    host.open(&editor, &[folder(workspace, "Work")], client(), soon())
        .unwrap();
    let uri = session_uri(host, &editor, &file_uri(&workspace.join(MODULE)));
    open_buffer(host, &editor, &uri, COMPLETION_TEXT);
    let V5LspOutcome::Json { value } = ask(
        host,
        &editor,
        V5LspCall::Completion {
            uri,
            position: unica_daemon::protocol_v5::lsp::V5LspPosition {
                line: 1,
                character: 4,
            },
        },
    ) else {
        panic!("completion answers with a list");
    };
    (editor, value)
}

fn labels(list: &serde_json::Value) -> Vec<&str> {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect()
}

/// Completion offers the names of the session's buffer and the platform's
/// global context. While that context is not loaded the list is marked
/// incomplete; once it is, the list is whole and offers its members.
#[test]
fn completion_is_incomplete_until_the_platform_context_is_loaded() {
    let (workspace, host) = completion_host("absent/8.3.27.1");
    let (_, unloaded) = complete(&host, workspace.path());
    assert_eq!(unloaded["isIncomplete"], true, "{unloaded}");
    assert!(labels(&unloaded).contains(&"Помощник"), "{unloaded}");

    let (workspace, host) = completion_host("platform/8.3.27.1");
    host.platform.ready_for_test(
        workspace.path().canonicalize().unwrap().join("platform/8.3.27.1"),
        vec![GlobalContextMember {
            kind: unica_documentation_sources::platform_help::provider::GlobalContextMemberKind::Method,
            name_ru: "ЗначениеЗаполнено".into(),
            name_en: Some("ValueIsFilled".into()),
            signature_ru: Some("ЗначениеЗаполнено(Значение)".into()),
            signature_en: None,
            parameters: vec!["Значение".into()],
        }],
    );
    let (_, loaded) = complete(&host, workspace.path());
    assert_eq!(loaded["isIncomplete"], false, "{loaded}");
    assert!(labels(&loaded).contains(&"ЗначениеЗаполнено"), "{loaded}");
}

/// A project without a platform installation has nothing to wait for: the
/// list is whole, and the editor does not ask again on every key.
#[test]
fn completion_without_an_installation_is_whole() {
    let (workspace, host) = completion_host("no-installation-here");
    let (_, list) = complete(&host, workspace.path());
    assert_eq!(list["isIncomplete"], false, "{list}");
}

#[test]
fn a_completion_item_resolves_from_the_document_it_names() {
    let (workspace, host) = completion_host("no-installation-here");
    let (editor, list) = complete(&host, workspace.path());
    let helper = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "Помощник")
        .unwrap()
        .clone();
    assert!(helper["data"][COMPLETION_DOCUMENT].is_string(), "{helper}");

    let V5LspOutcome::Json { value: resolved } = ask(
        &host,
        &editor,
        V5LspCall::CompletionResolve { item: helper },
    ) else {
        panic!("resolve answers with the item");
    };
    assert!(
        resolved["documentation"]
            .to_string()
            .contains("Помогает выполнить"),
        "{resolved}"
    );

    let foreign = serde_json::json!({"label": "Чужой"});
    assert_eq!(
        ask(
            &host,
            &editor,
            V5LspCall::CompletionResolve {
                item: foreign.clone()
            }
        ),
        V5LspOutcome::Json { value: foreign },
        "an item the service did not give comes back as it was"
    );
}

const SYMBOLS_TEXT: &str = "Процедура Выполнить()\n    Помощник();\n    ЗначениеЗаполнено(1);\nКонецПроцедуры\n\nПроцедура Помощник() Экспорт\nКонецПроцедуры\n";

fn at(line: u32, character: u32) -> unica_daemon::protocol_v5::lsp::V5LspPosition {
    unica_daemon::protocol_v5::lsp::V5LspPosition { line, character }
}

fn json_of(outcome: V5LspOutcome) -> serde_json::Value {
    match outcome {
        V5LspOutcome::Json { value } => value,
        other => panic!("not a JSON answer: {other:?}"),
    }
}

fn symbols_session() -> (tempfile::TempDir, UnicaLspHost, V5LspSessionId, String) {
    let workspace = workspace();
    let host = UnicaLspHost::new(Arc::new(
        crate::infrastructure::source_workspace::actor_registry(),
    ));
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let uri = session_uri(&host, &editor, &file_uri(&workspace.path().join(MODULE)));
    open_buffer(&host, &editor, &uri, SYMBOLS_TEXT);
    (workspace, host, editor, uri)
}

/// The symbol port answers in this session's places: the buffer's own
/// methods by its text, as `unica://` locations of the session's space.
#[test]
fn symbols_answer_from_the_buffer_in_the_sessions_places() {
    let (_workspace, host, editor, uri) = symbols_session();

    let definition = json_of(ask(
        &host,
        &editor,
        V5LspCall::Definition {
            uri: uri.clone(),
            position: at(1, 6),
        },
    ));
    assert_eq!(definition[0]["uri"], uri.as_str(), "{definition}");
    assert_eq!(definition[0]["range"]["start"]["line"], 5, "{definition}");

    let hover = json_of(ask(
        &host,
        &editor,
        V5LspCall::Hover {
            uri: uri.clone(),
            position: at(1, 6),
        },
    ));
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Помощник"),
        "{hover}"
    );

    let references = json_of(ask(
        &host,
        &editor,
        V5LspCall::References {
            uri: uri.clone(),
            position: at(5, 12),
        },
    ));
    let lines = references
        .as_array()
        .unwrap()
        .iter()
        .map(|place| place["range"]["start"]["line"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert!(lines.contains(&1), "{references}");

    let items = json_of(ask(
        &host,
        &editor,
        V5LspCall::PrepareCallHierarchy {
            uri: uri.clone(),
            position: at(5, 12),
        },
    ));
    assert_eq!(items[0]["name"], "Помощник", "{items}");
    let incoming = json_of(ask(
        &host,
        &editor,
        V5LspCall::IncomingCalls {
            item: items[0].clone(),
        },
    ));
    assert!(incoming.is_array(), "{incoming}");
}

/// A provider that is slower than the editor's budget and records the
/// deadline and cancellation it was given.
#[derive(Default)]
struct SlowProvider {
    seen: Mutex<Vec<(Option<Duration>, bool)>>,
}

impl unica_code_intelligence::symbols::SymbolProvider for SlowProvider {
    fn symbol(
        &self,
        _: &unica_code_intelligence::symbols::SymbolQuery,
        context: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
    ) -> Result<
        Option<unica_code_intelligence::symbols::ProviderSymbol>,
        unica_code_intelligence::symbols::ProviderFailure,
    > {
        std::thread::sleep(Duration::from_millis(200));
        self.seen.lock().unwrap().push((
            context.deadline.remaining(),
            context.cancellation.is_cancelled(),
        ));
        Ok(None)
    }

    fn references(
        &self,
        _: &unica_code_intelligence::symbols::SymbolQuery,
        _: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
    ) -> Result<
        unica_code_intelligence::symbols::ProviderPlaces,
        unica_code_intelligence::symbols::ProviderFailure,
    > {
        unreachable!("the editor's references do not call the provider")
    }

    fn calls(
        &self,
        _: &unica_code_intelligence::symbols::SymbolQuery,
        _: unica_code_intelligence::symbols::CallDirection,
        _: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
    ) -> Result<
        unica_code_intelligence::symbols::ProviderPlaces,
        unica_code_intelligence::symbols::ProviderFailure,
    > {
        Ok(unica_code_intelligence::symbols::ProviderPlaces {
            places: Vec::new(),
            complete: true,
        })
    }
}

/// The provider gets the call's full deadline, as MCP does, never the
/// editor's budget: cancelling it would retire the analyzer session MCP
/// shares (#2189). An editor that waits less gets the lane's server
/// cancellation while the provider runs to its own end.
#[test]
fn the_provider_runs_to_its_own_end_past_the_editors_budget() {
    let (_workspace, host, editor, uri) = symbols_session();
    let provider = Arc::new(SlowProvider::default());
    crate::infrastructure::application_ports::SYMBOL_PROVIDER_OVERRIDE
        .with(|slot| *slot.borrow_mut() = Some(provider.clone()));

    let budget = Instant::now() + Duration::from_millis(20);
    let _ = host.call(
        &editor,
        V5LspCall::Hover {
            uri,
            position: at(2, 6),
        },
        &LspCallContext::for_test(budget),
    );
    crate::infrastructure::application_ports::SYMBOL_PROVIDER_OVERRIDE
        .with(|slot| *slot.borrow_mut() = None);

    let seen = provider.seen.lock().unwrap();
    assert!(!seen.is_empty(), "the platform name reaches the provider");
    assert!(
        Instant::now() > budget,
        "the provider outlived the editor's budget"
    );
    for (remaining, cancelled) in seen.iter() {
        assert_eq!(*remaining, None, "the provider has no editor deadline");
        assert!(!cancelled, "nothing cancelled the provider");
    }
}

/// A platform function has its signature once the global context loaded.
#[test]
fn signature_help_names_a_platform_function_from_the_loaded_context() {
    let (workspace, host) = completion_host("platform/8.3.27.1");
    host.platform.ready_for_test(
        workspace.path().canonicalize().unwrap().join("platform/8.3.27.1"),
        vec![GlobalContextMember {
            kind: unica_documentation_sources::platform_help::provider::GlobalContextMemberKind::Method,
            name_ru: "ЗначениеЗаполнено".into(),
            name_en: Some("ValueIsFilled".into()),
            signature_ru: Some("ЗначениеЗаполнено(Значение)".into()),
            signature_en: None,
            parameters: vec!["Значение".into()],
        }],
    );
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let uri = session_uri(&host, &editor, &file_uri(&workspace.path().join(MODULE)));
    open_buffer(
        &host,
        &editor,
        &uri,
        "Процедура А()\n    ЗначениеЗаполнено(\nКонецПроцедуры\n",
    );

    let help = json_of(ask(
        &host,
        &editor,
        V5LspCall::SignatureHelp {
            uri,
            position: unica_daemon::protocol_v5::lsp::V5LspPosition {
                line: 1,
                character: 22,
            },
        },
    ));
    assert!(
        help["signatures"][0]["label"]
            .as_str()
            .is_some_and(|label| label.contains("ЗначениеЗаполнено")),
        "{help}"
    );
}

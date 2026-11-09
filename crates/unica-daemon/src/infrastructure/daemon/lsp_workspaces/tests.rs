//! Служба LSP демона над рабочей областью на диске: те же актор и наборы,
//! что у MCP, буфер виден только своей сессии и не пишется на диск.
//! Что служба делает с сессиями, буферами и акторами сама, проверяет
//! `crates/unica-editor-analysis/src/service_tests.rs`; здесь — допуск и чтение продукта.
use super::*;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use unica_address::source_target::MetadataAddress;
use unica_editor_analysis::UnicaLspHost;
use unica_lsp_contract::host::{LspCallContext, LspClientChannel, LspHost};
use unica_lsp_contract::protocol::{
    V5LspCall, V5LspContentChange, V5LspNotification, V5LspOutcome, V5LspSessionId,
    V5LspWorkspaceFolder, V5LspWorkspaceSpace,
};

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
    let workspaces = Arc::new(ProductLspWorkspaces::new(
        Arc::new(crate::infrastructure::source_workspace::actor_registry()),
        Arc::new(super::super::v13_service::CanonicalV13ReadService::default()),
    ));
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
    let module_target = MetadataAddress::parse_logical("CommonModule.Main.Module").unwrap();
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

/// Product admission after the source set changes on disk: a replaced root and
/// a root moved in `v8project.yaml` each get a new actor bound to the new root,
/// never the old one.
#[test]
fn product_admission_gives_a_new_actor_for_a_replaced_or_moved_root() {
    let workspace = workspace();
    let root = workspace.path();
    let (_host, workspaces) = service();
    let first = Arc::downgrade(&workspaces.admit(root, soon()).unwrap().actor);

    fs::rename(root.join("src"), root.join("src.old")).unwrap();
    fs::create_dir_all(root.join(MODULE).parent().unwrap()).unwrap();
    fs::copy(
        root.join("src.old/Configuration.xml"),
        root.join("src/Configuration.xml"),
    )
    .unwrap();
    fs::write(root.join(MODULE), DISK).unwrap();
    let replaced = workspaces
        .admit(root, soon())
        .expect("the new root is admitted");
    assert!(first
        .upgrade()
        .is_none_or(|old| !Arc::ptr_eq(&old, &replaced.actor)));
    assert!(replaced.source_sets[0].root.ends_with("src"));

    fs::rename(root.join("src"), root.join("moved")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: moved\n",
    )
    .unwrap();
    let moved = workspaces
        .admit(root, soon())
        .expect("the moved root is admitted");
    assert!(!Arc::ptr_eq(&replaced.actor, &moved.actor));
    assert!(moved.source_sets[0].root.ends_with("moved"));
}

/// A provider still indexing: it answers nothing yet.
struct NotReady;

impl unica_code_intelligence::symbols::SymbolProvider for NotReady {
    fn symbol(
        &self,
        _: &unica_code_intelligence::symbols::SymbolQuery,
        _: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
    ) -> Result<
        Option<unica_code_intelligence::symbols::ProviderSymbol>,
        unica_code_intelligence::symbols::ProviderFailure,
    > {
        Err(unica_code_intelligence::symbols::ProviderFailure::NotReady)
    }

    fn references(
        &self,
        _: &unica_code_intelligence::symbols::SymbolQuery,
        _: &unica_code_intelligence::symbols::SymbolRequestContext<'_>,
    ) -> Result<
        unica_code_intelligence::symbols::ProviderPlaces,
        unica_code_intelligence::symbols::ProviderFailure,
    > {
        Err(unica_code_intelligence::symbols::ProviderFailure::NotReady)
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
        Err(unica_code_intelligence::symbols::ProviderFailure::NotReady)
    }
}

/// The product's symbol service answers the editor from its buffer, at the
/// document's own address; a name only the provider knows, while it is not
/// ready, is refused as incomplete rather than answered as absent.
#[test]
fn symbols_answer_from_the_buffer_at_the_documents_address() {
    struct ResetOverride;
    impl Drop for ResetOverride {
        fn drop(&mut self) {
            crate::infrastructure::application_ports::SYMBOL_PROVIDER_OVERRIDE
                .with(|slot| *slot.borrow_mut() = None);
        }
    }
    let _reset = ResetOverride;
    crate::infrastructure::application_ports::SYMBOL_PROVIDER_OVERRIDE
        .with(|slot| *slot.borrow_mut() = Some(Arc::new(NotReady)));
    let workspace = workspace();
    // The symbol service reads modules as MCP does: through their objects.
    let source = workspace.path().join("src");
    fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><CommonModule>Main</CommonModule><CommonModule>Other</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    for (name, uuid) in [
        ("Main", "11111111-1111-4111-8111-111111111111"),
        ("Other", "22222222-2222-4222-8222-222222222222"),
    ] {
        fs::write(
            source.join(format!("CommonModules/{name}.xml")),
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="{uuid}"><Properties><Name>{name}</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#),
        )
        .unwrap();
    }
    fs::create_dir_all(source.join("CommonModules/Other/Ext")).unwrap();
    fs::write(
        source.join("CommonModules/Other/Ext/Module.bsl"),
        "Procedure Shared() Export EndProcedure
",
    )
    .unwrap();
    let (host, _workspaces) = service();
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let buffer =
        "Procedure Callee() EndProcedure\nProcedure Caller() Callee(); Unknown(); Other.Shared(); EndProcedure\n";
    host.notify(
        &editor,
        V5LspNotification::DidOpen {
            uri: BODY_URI.into(),
            language_id: "bsl".into(),
            version: 1,
            text: buffer.into(),
        },
        soon(),
    )
    .unwrap();
    let at = |line, character| unica_lsp_contract::protocol::V5LspPosition { line, character };

    assert_eq!(
        ask(
            &host,
            &editor,
            V5LspCall::Definition {
                uri: BODY_URI.into(),
                position: at(1, 20),
            },
        ),
        V5LspOutcome::Locations {
            locations: vec![unica_lsp_contract::protocol::V5LspLocation {
                uri: BODY_URI.into(),
                range: unica_lsp_contract::protocol::V5LspRange {
                    start: at(0, 10),
                    end: at(0, 16),
                },
            }],
        }
    );
    assert!(matches!(
        ask(
            &host,
            &editor,
            V5LspCall::Hover {
                uri: BODY_URI.into(),
                position: at(1, 20),
            },
        ),
        V5LspOutcome::Hover { contents: Some(text) } if text.contains("Callee")
    ));
    assert_eq!(
        ask(
            &host,
            &editor,
            V5LspCall::Definition {
                uri: BODY_URI.into(),
                position: at(1, 30),
            },
        ),
        V5LspOutcome::Failed {
            failure: V5LspFailure::Incomplete
        }
    );
    // A method of another common module: the document of that module's body.
    assert_eq!(
        ask(
            &host,
            &editor,
            V5LspCall::Definition {
                uri: BODY_URI.into(),
                position: at(1, 47),
            },
        ),
        V5LspOutcome::Locations {
            locations: vec![unica_lsp_contract::protocol::V5LspLocation {
                uri: "unica://work/main/CommonModule.Other.Body".into(),
                range: unica_lsp_contract::protocol::V5LspRange {
                    start: at(0, 10),
                    end: at(0, 16),
                },
            }],
        }
    );
}

/// A workspace whose saved module declares a variable after its method, as
/// the platform does not compile it; its objects have full descriptors.
fn diagnosed_workspace() -> tempfile::TempDir {
    let workspace = workspace();
    let source = workspace.path().join("src");
    fs::write(
        source.join("CommonModules/Main.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Main</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(
        workspace.path().join(MODULE),
        "Procedure Disk() EndProcedure\nVar Late;\n",
    )
    .unwrap();
    workspace
}

fn report(host: &UnicaLspHost, session: &V5LspSessionId) -> Vec<String> {
    match ask(
        host,
        session,
        V5LspCall::Diagnostic {
            uri: BODY_URI.into(),
        },
    ) {
        V5LspOutcome::Diagnostics { items } => items
            .into_iter()
            .map(|item| format!("{}: {}", item.code, item.message))
            .collect(),
        other => panic!("not a diagnostic report: {other:?}"),
    }
}

struct ResetDiagnosticOverride;

impl Drop for ResetDiagnosticOverride {
    fn drop(&mut self) {
        crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
            .with(|slot| *slot.borrow_mut() = None);
    }
}

/// The editor's document gets `check`'s own finding and the analysis of its
/// saved module by `check`'s coordinator, with no deadline put on it; an
/// analysis that fails is named, never an empty complete report.
#[test]
fn diagnostics_run_checks_own_findings_and_its_analysis_of_the_saved_module() {
    let _reset = ResetDiagnosticOverride;
    let (provider, analyzer) =
        unica_coder::infrastructure::daemon_adapters::diagnostics_probe::recording_provider();
    crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE
        .with(|slot| *slot.borrow_mut() = Some(provider));
    let workspace = diagnosed_workspace();
    let (host, _workspaces) = service();
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();

    let found = report(&host, &editor);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].starts_with("VariableDeclarationPlacement: "),
        "{found:?}"
    );
    assert_eq!(analyzer.timeouts(), [None], "no deadline, as `check`");

    crate::infrastructure::application_ports::DIAGNOSTIC_PROVIDER_OVERRIDE.with(|slot| {
        *slot.borrow_mut() = Some(
            unica_coder::infrastructure::daemon_adapters::diagnostics_probe::failing_provider(
                unica_coder::infrastructure::daemon_adapters::diagnostics_probe::AnalyzerRunFailureKind::Exited,
            ),
        )
    });
    let failing = diagnosed_workspace();
    let (host, _workspaces) = service();
    host.open(&editor, &[folder(failing.path(), "Work")], client(), soon())
        .unwrap();
    let found = report(&host, &editor);
    assert!(
        found.last().is_some_and(
            |marker| marker.starts_with("unica.diagnostics_incomplete: ")
                && marker.contains("analysis_unavailable")
        ),
        "{found:?}"
    );
}

/// Completion over the product's sources: an exported method of another
/// common module, its description on resolve, and a member of the project's
/// platform global context; the list is whole once that context is loaded.
#[test]
fn completion_offers_common_module_exports_and_the_platform_context() {
    let workspace = diagnosed_workspace();
    let source = workspace.path().join("src");
    fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><CommonModule>Main</CommonModule><CommonModule>Other</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(
        source.join("CommonModules/Other.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Other</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .unwrap();
    fs::create_dir_all(source.join("CommonModules/Other/Ext")).unwrap();
    fs::write(
        source.join("CommonModules/Other/Ext/Module.bsl"),
        "// Shares a value.\nProcedure Shared() Export EndProcedure\nProcedure Hidden() EndProcedure\n",
    )
    .unwrap();
    let workspaces = Arc::new(
        ProductLspWorkspaces::new(
            Arc::new(crate::infrastructure::source_workspace::actor_registry()),
            Arc::new(super::super::v13_service::CanonicalV13ReadService::default()),
        )
        .with_platform_for_test(
            workspace.path().join("platform"),
            vec![unica_code_intelligence::completion::PlatformMember {
                name: "Сообщить".into(),
                method: true,
                parameters: vec!["ТекстСообщения".into()],
            }],
        ),
    );
    let host = UnicaLspHost::new(workspaces);
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();
    let buffer = "Procedure P()\n\tOther.\n\tСо\nEndProcedure\n";
    host.notify(
        &editor,
        V5LspNotification::DidOpen {
            uri: BODY_URI.into(),
            language_id: "bsl".into(),
            version: 1,
            text: buffer.into(),
        },
        soon(),
    )
    .unwrap();
    let at = |line, character| unica_lsp_contract::protocol::V5LspPosition { line, character };
    let complete = |line, character| match ask(
        &host,
        &editor,
        V5LspCall::Completion {
            uri: BODY_URI.into(),
            position: at(line, character),
        },
    ) {
        V5LspOutcome::CompletionList { incomplete, items } => (incomplete, items),
        other => panic!("not a completion list: {other:?}"),
    };

    let (incomplete, exports) = complete(1, 7);
    assert!(!incomplete);
    assert_eq!(
        exports
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["Shared"]
    );
    match ask(
        &host,
        &editor,
        V5LspCall::CompletionResolve {
            item: Box::new(exports[0].clone()),
        },
    ) {
        V5LspOutcome::CompletionItem { item } => {
            assert_eq!(item.documentation.as_deref(), Some("Shares a value."))
        }
        other => panic!("not an item: {other:?}"),
    }

    let (_, names) = complete(2, 3);
    assert!(
        names.iter().any(|item| item.label == "Сообщить"),
        "{names:?}"
    );
    assert!(names.iter().any(|item| item.label == "Other"), "{names:?}");
}

/// rule: INV.LSP.PARTIAL-ANSWERS
#[test]
fn completion_is_incomplete_while_the_platform_context_loads() {
    let workspace = diagnosed_workspace();
    let workspaces = Arc::new(
        ProductLspWorkspaces::new(
            Arc::new(crate::infrastructure::source_workspace::actor_registry()),
            Arc::new(super::super::v13_service::CanonicalV13ReadService::default()),
        )
        .with_platform_loading_for_test(workspace.path().join("platform")),
    );
    let host = UnicaLspHost::new(workspaces);
    let editor = V5LspSessionId::new();
    host.open(
        &editor,
        &[folder(workspace.path(), "Work")],
        client(),
        soon(),
    )
    .unwrap();

    let outcome = ask(
        &host,
        &editor,
        V5LspCall::Completion {
            uri: BODY_URI.into(),
            position: unica_lsp_contract::protocol::V5LspPosition {
                line: 0,
                character: 0,
            },
        },
    );

    assert!(
        matches!(
            outcome,
            V5LspOutcome::CompletionList {
                incomplete: true,
                ..
            }
        ),
        "{outcome:?}"
    );
}

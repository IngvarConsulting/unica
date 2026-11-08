//! Один допуск для MCP и редактора: на тех же входах это один актор, а
//! расширение без доступного родителя открывает только `view` MCP.
use super::*;
use crate::infrastructure::daemon::lsp_workspaces::ProductLspWorkspaces;
use crate::infrastructure::daemon::protocol::InvocationRequest;
use std::fs;
use std::time::Duration;
use unica_execution::clock::TokioClock;
use unica_execution::invocation_store::ToolIdentity;
use unica_lsp::UnicaLspHost;
use unica_lsp_contract::host::{LspCallContext, LspClientChannel, LspHost};
use unica_lsp_contract::protocol::{
    V5LspCall, V5LspFailure, V5LspOutcome, V5LspSessionId, V5LspWorkspaceFolder,
    V5LspWorkspaceSpace,
};

const DISK: &str = "Procedure FromExtension() EndProcedure\n";

/// The configuration `main` and the extension `ext`, both on disk.
fn configuration_with_extension() -> tempfile::TempDir {
    let workspace = extension_without_parent();
    let source = workspace.path().join("src");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    workspace
}

/// The configuration `main` is declared but its root is missing; the
/// extension `ext` is on disk.
fn extension_without_parent() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
    )
    .unwrap();
    let module = root.join("ext/CommonModules/Main/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(
        root.join("ext/Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Ext</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties><ChildObjects><CommonModule>Main</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(module, DISK).unwrap();
    workspace
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn folder(path: &Path) -> V5LspWorkspaceFolder {
    V5LspWorkspaceFolder {
        uri: url::Url::from_file_path(path.canonicalize().unwrap())
            .unwrap()
            .into(),
        name: "Work".to_owned(),
    }
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

/// rule: CTR.LSP.SEPARATE-DOOR
#[test]
fn an_editor_folder_opens_an_extension_only_with_its_parent_configuration() {
    let host = UnicaLspHost::new(Arc::new(ProductLspWorkspaces::new(
        Arc::new(crate::infrastructure::source_workspace::actor_registry()),
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
    )));
    let session = V5LspSessionId::new();
    let healthy = configuration_with_extension();
    let orphan = extension_without_parent();
    let folders = [folder(healthy.path()), folder(orphan.path())];

    let opening = host
        .open(
            &session,
            &folders,
            LspClientChannel::detached_for_test(),
            soon(),
        )
        .unwrap();

    assert_eq!(
        opening.workspaces,
        vec![
            V5LspWorkspaceSpace::opened(&folders[0].uri, "work"),
            V5LspWorkspaceSpace::refused(&folders[1].uri, V5LspFailure::WorkspaceUnavailable),
        ]
    );
    assert_eq!(
        content(&host, &session, "unica://work/ext/CommonModule.Main.Body"),
        V5LspOutcome::Text { text: DISK.into() }
    );
}

/// rule: INV.APP.ACTOR-AUTHENTICATED-SOURCE-IDENTITY
#[test]
fn a_view_of_the_extension_and_its_editor_folder_reach_one_actor() {
    let workspace = configuration_with_extension();
    let runtime = super::super::super::V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let request = InvocationRequest::new(
        ToolIdentity::View,
        serde_json::json!({"at": "ext:CommonModule.Main"}),
        fs::canonicalize(workspace.path())
            .unwrap()
            .to_string_lossy(),
        7_000,
    )
    .unwrap();

    let invocation = super::super::bind_workspace_invocation(
        &request,
        &runtime.workspace_actors,
        Arc::clone(&runtime.deliveries),
        Arc::clone(&runtime.provider_hosts),
        Arc::clone(&runtime.runtime_resources),
        None,
        runtime.capture_response_deadline_for_test(),
    )
    .unwrap();
    let editor = admit_editor_folder(workspace.path(), &runtime.workspace_actors, soon()).unwrap();

    assert!(Arc::ptr_eq(&invocation.actor, &editor.actor));
    assert_eq!(
        editor
            .bindings
            .iter()
            .map(|binding| binding.source_set_name())
            .collect::<Vec<_>>(),
        vec!["ext", "main"]
    );

    // Without the parent root `view` still reads the extension; the editor
    // folder is refused.
    let orphan = extension_without_parent();
    let request = InvocationRequest::new(
        ToolIdentity::View,
        serde_json::json!({"at": "ext:CommonModule.Main"}),
        fs::canonicalize(orphan.path()).unwrap().to_string_lossy(),
        7_000,
    )
    .unwrap();
    let invocation = super::super::bind_workspace_invocation(
        &request,
        &runtime.workspace_actors,
        Arc::clone(&runtime.deliveries),
        Arc::clone(&runtime.provider_hosts),
        Arc::clone(&runtime.runtime_resources),
        None,
        runtime.capture_response_deadline_for_test(),
    )
    .unwrap();
    assert_eq!(invocation.parent_source_names, vec!["main".to_owned()]);
    assert!(admit_editor_folder(orphan.path(), &runtime.workspace_actors, soon()).is_none());
}

/// A call holding the actor while its root is replaced: other admissions of
/// the workspace get a retryable refusal, not a state error, and the next
/// admission after the call ends gets a new actor.
#[test]
fn a_replaced_root_is_temporarily_unavailable_while_a_call_holds_the_old_actor() {
    let workspace = configuration_with_extension();
    let root = workspace.path();
    let runtime = super::super::super::V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let bind = || {
        let request = InvocationRequest::new(
            ToolIdentity::View,
            serde_json::json!({"at": "ext:CommonModule.Main"}),
            fs::canonicalize(root).unwrap().to_string_lossy(),
            7_000,
        )
        .unwrap();
        super::super::bind_workspace_invocation(
            &request,
            &runtime.workspace_actors,
            Arc::clone(&runtime.deliveries),
            Arc::clone(&runtime.provider_hosts),
            Arc::clone(&runtime.runtime_resources),
            None,
            runtime.capture_response_deadline_for_test(),
        )
    };
    let running = bind().unwrap();
    fs::rename(root.join("ext"), root.join("ext.old")).unwrap();
    fs::create_dir_all(root.join("ext/CommonModules/Main/Ext")).unwrap();
    fs::copy(
        root.join("ext.old/Configuration.xml"),
        root.join("ext/Configuration.xml"),
    )
    .unwrap();
    fs::write(root.join("ext/CommonModules/Main/Ext/Module.bsl"), DISK).unwrap();

    assert!(matches!(
        bind().err(),
        Some(WorkspaceAdmissionError::Unadmitted {
            cause: UnadmittedCause::ActorRootUnavailable,
            ..
        })
    ));
    assert!(admit_editor_folder(root, &runtime.workspace_actors, soon()).is_none());

    let stale = Arc::downgrade(&running.actor);
    drop(running);
    let next = bind().expect("the replaced root is admitted after the call ends");
    assert!(stale.upgrade().is_none());
    assert!(admit_editor_folder(root, &runtime.workspace_actors, soon())
        .is_some_and(|editor| Arc::ptr_eq(&editor.actor, &next.actor)));
}

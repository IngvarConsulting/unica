//! Один допуск для MCP и редактора: расширение без доступного родителя
//! открывается в обоих, и на тех же входах это один актор.
use super::*;
use crate::infrastructure::daemon::lsp_workspaces::ProductLspWorkspaces;
use crate::infrastructure::daemon::protocol::InvocationRequest;
use std::fs;
use std::time::Duration;
use unica_daemon::lsp_lane::{LspCallContext, LspClientChannel, LspHost};
use unica_daemon::protocol_v5::lsp::{
    V5LspCall, V5LspFailure, V5LspOutcome, V5LspSessionId, V5LspWorkspaceFolder,
    V5LspWorkspaceSpace,
};
use unica_execution::clock::TokioClock;
use unica_execution::invocation_store::ToolIdentity;
use unica_lsp::UnicaLspHost;

const DISK: &str = "Procedure FromExtension() EndProcedure\n";

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
fn an_editor_folder_opens_an_extension_whose_parent_configuration_is_unavailable() {
    let workspace = extension_without_parent();
    let host = UnicaLspHost::new(Arc::new(ProductLspWorkspaces::new(Arc::new(
        crate::infrastructure::source_workspace::actor_registry(),
    ))));
    let session = V5LspSessionId::new();
    let folders = [folder(workspace.path())];

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
        vec![V5LspWorkspaceSpace::opened(&folders[0].uri, "work")]
    );
    assert_eq!(
        content(&host, &session, "unica://work/ext/CommonModule.Main.Body"),
        V5LspOutcome::Text { text: DISK.into() }
    );
    // The skipped parent has no document: it is not read from anywhere else.
    assert_eq!(
        content(&host, &session, "unica://work/main/CommonModule.Main.Body"),
        V5LspOutcome::Failed {
            failure: V5LspFailure::DocumentUnknown
        }
    );

    // Without an extension an unavailable configuration still refuses the folder.
    fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    assert!(admit_editor_folder(
        workspace.path(),
        &crate::infrastructure::source_workspace::actor_registry(),
        soon()
    )
    .is_none());
}

/// rule: INV.APP.ACTOR-AUTHENTICATED-SOURCE-IDENTITY
#[test]
fn a_view_of_the_extension_and_its_editor_folder_reach_one_actor() {
    let workspace = extension_without_parent();
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
    assert_eq!(invocation.parent_source_names, vec!["main".to_owned()]);
    assert_eq!(
        editor
            .bindings
            .iter()
            .map(|binding| binding.source_set_name())
            .collect::<Vec<_>>(),
        vec!["ext"]
    );
}

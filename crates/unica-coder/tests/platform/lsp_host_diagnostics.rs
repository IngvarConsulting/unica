//! Диагностики документа через службу LSP (#2176): свои находки Unica по
//! буферу сразу, находки поставщика по сохранённому файлу в фоне, всё —
//! в кодах и словах Unica; неполный ответ помечен, готовый рассылается.
use super::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use unica_address::source_target::TargetKind;
use unica_daemon::protocol_v5::lsp::V5LspDiagnosticsDelivery;
use unica_diagnostics::model::{
    DiagnosticAction, DiagnosticContext, DiagnosticFocusKind, DiagnosticObservation,
    DiagnosticObservationFocus, DiagnosticObservationLocation, DiagnosticProvider,
    DiagnosticProviderDescriptor, DiagnosticProviderId, DiagnosticProviderOutcome,
    DiagnosticProviderRequest, DiagnosticProviderStatus, DiagnosticRange, DiagnosticSeverity,
    EditorFinding,
};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

const MODULE: &str = "src/CommonModules/Main/Ext/Module.bsl";
const SAVED: &str = "Процедура А()\nКонецПроцедуры\nПерем Поздно;\n";
const PROVIDER: &str = "fixture-analyzer";
const PROVIDER_RULE: &str = "LineLength";
const PROVIDER_TEXT: &str = "provider wording";

static DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
    id: DiagnosticProviderId::new_const(PROVIDER),
    actions: &[DiagnosticAction::Analyze],
    findings_target_kinds: &[TargetKind::Module],
    emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
};

/// A provider that reads the saved module and reports one finding on its
/// first line, in its own words, once the test lets it.
struct Provider(PathBuf, Arc<AtomicBool>);

impl DiagnosticProvider for Provider {
    fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
        &DESCRIPTOR
    }

    fn execute(
        &self,
        request: &DiagnosticProviderRequest,
        _: &DiagnosticContext,
        _: ProviderDeadline,
        _: &CancellationToken,
    ) -> DiagnosticProviderOutcome {
        assert_eq!(request.action, DiagnosticAction::Analyze);
        let held = Instant::now() + Duration::from_secs(20);
        while !self.1.load(Ordering::Acquire) && Instant::now() < held {
            std::thread::sleep(Duration::from_millis(5));
        }
        // The analyzer is given the module alone and names it by its path.
        let scope = request
            .module_scope
            .as_ref()
            .expect("one module is analysed");
        assert!(self.0.ends_with(&scope.module_path), "{scope:?}");
        let handle = scope.source_root.join(&scope.module_path);
        let mut outcome = DiagnosticProviderOutcome::empty(DiagnosticProviderStatus::Completed);
        outcome.complete = true;
        outcome.observations = vec![DiagnosticObservation::Diagnostic {
            provider: DESCRIPTOR.id,
            location: DiagnosticObservationLocation::Resource {
                handle: handle.to_string_lossy().into_owned(),
            },
            focus: DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 9,
            }),
            code: PROVIDER_RULE.into(),
            severity: DiagnosticSeverity::Info,
            message: PROVIDER_TEXT.into(),
            tags: Vec::new(),
        }];
        outcome
    }

    fn editor_finding(&self, code: &str, severity: DiagnosticSeverity) -> EditorFinding {
        match code {
            PROVIDER_RULE => EditorFinding {
                code: "bsl.style.line_length",
                message: "Строка длиннее допустимого",
            },
            _ => EditorFinding::generic(severity),
        }
    }
}

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
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><CommonModule uuid="9d3af1f2-e36e-48f7-94bf-2861d90e1314"><Properties><Name>Main</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(root.join(MODULE), SAVED).unwrap();
    workspace
}

fn file_uri(path: &Path) -> String {
    url::Url::from_file_path(path.canonicalize().unwrap())
        .unwrap()
        .into()
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn pull(host: &UnicaLspHost, session: &V5LspSessionId, uri: &str, previous: Option<&str>) -> Value {
    match host.call(
        session,
        V5LspCall::Diagnostic {
            uri: uri.to_owned(),
            previous_result_id: previous.map(str::to_owned),
        },
        &LspCallContext::for_test(soon()),
    ) {
        V5LspOutcome::Json { value } => value,
        other => panic!("diagnostic pull failed: {other:?}"),
    }
}

fn codes(items: &Value) -> Vec<String> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["code"].as_str().unwrap().to_owned())
        .collect()
}

/// The next published diagnostics of `uri`, waiting for the background
/// analysis.
fn published(client: &LspClientChannel, uri: &str, wanted: impl Fn(&Value) -> bool) -> Value {
    published_as(client, uri, |params, _| wanted(params)).0
}

/// The next published diagnostics of `uri` and whether an analysis ended.
fn published_as(
    client: &LspClientChannel,
    uri: &str,
    wanted: impl Fn(&Value, bool) -> bool,
) -> (Value, bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        for notification in client.take_for_test(deadline) {
            if let V5LspServerNotification::PublishDiagnostics {
                uri: published,
                params,
                analysed,
            } = notification
            {
                if published == uri && wanted(&params, analysed) {
                    return (params, analysed);
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "no published diagnostics for {uri}"
        );
    }
}

fn assert_no_provider(answer: &Value) {
    let rendered = answer.to_string();
    for provider in [PROVIDER, PROVIDER_RULE, PROVIDER_TEXT] {
        assert!(
            !rendered.contains(provider),
            "{provider} reached the editor: {rendered}"
        );
    }
}

/// A service over `workspace` with the fixture provider, held until
/// `release` is set, and one session whose editor takes diagnostics by
/// `delivery`.
fn serve(
    workspace: &Path,
    delivery: V5LspDiagnosticsDelivery,
) -> (
    UnicaLspHost,
    V5LspSessionId,
    LspClientChannel,
    Arc<AtomicBool>,
    String,
) {
    let module = workspace.join(MODULE).canonicalize().unwrap();
    let actors = Arc::new(crate::infrastructure::source_workspace::actor_registry());
    let release = Arc::new(AtomicBool::new(false));
    let (analysed, released) = (module.clone(), Arc::clone(&release));
    let host = UnicaLspHost::new(actors).with_diagnostic_providers_for_test(move || {
        vec![Arc::new(Provider(analysed.clone(), Arc::clone(&released)))
            as Arc<dyn DiagnosticProvider>]
    });
    let session = V5LspSessionId::new();
    let client = LspClientChannel::detached_for_test(delivery);
    let opening = host
        .open(
            &session,
            &[V5LspWorkspaceFolder {
                uri: file_uri(workspace),
                name: "Work".into(),
            }],
            client.clone(),
            soon(),
        )
        .unwrap();
    assert!(opening.capabilities.diagnostics);
    (host, session, client, release, file_uri(&module))
}

#[test]
fn diagnostics_answer_at_once_and_publish_the_provider_part_when_ready() {
    let workspace = workspace();
    let module = workspace.path().join(MODULE).canonicalize().unwrap();
    let (host, session, client, release, uri) =
        serve(workspace.path(), V5LspDiagnosticsDelivery::Push);
    host.notify(
        &session,
        V5LspNotification::DidOpen {
            uri: uri.clone(),
            language_id: "bsl".into(),
            version: 1,
            text: SAVED.into(),
        },
        soon(),
    )
    .unwrap();

    // The pull does not wait for the analysis: Unica's own finding is there,
    // the provider's part is named missing.
    let first = pull(&host, &session, &uri, None);
    assert_eq!(first["kind"], "full");
    let first_codes = codes(&first["items"]);
    assert!(
        first_codes.contains(&"VariableDeclarationPlacement".to_string()),
        "{first}"
    );
    assert!(
        first_codes.contains(&"DiagnosticsIncomplete".to_string()),
        "{first}"
    );
    assert_no_provider(&first);
    release.store(true, Ordering::Release);

    // The finished analysis is published in Unica's words.
    let ready = published(&client, &uri, |params| {
        codes(&params["diagnostics"]).contains(&"bsl.style.line_length".to_string())
    });
    assert_no_provider(&ready);
    assert!(!codes(&ready["diagnostics"]).contains(&"DiagnosticsIncomplete".to_string()));

    // The next pull is complete; the editor's copy of it stays unchanged.
    let complete = pull(&host, &session, &uri, None);
    assert_eq!(
        codes(&complete["items"]),
        ["bsl.style.line_length", "VariableDeclarationPlacement"]
    );
    let unchanged = pull(&host, &session, &uri, complete["resultId"].as_str());
    assert_eq!(unchanged["kind"], "unchanged");

    // An edited buffer: Unica's own check follows it at once; the analysis
    // of the saved file no longer stands for it.
    host.notify(
        &session,
        V5LspNotification::DidChange {
            uri: uri.clone(),
            version: 3,
            changes: vec![V5LspContentChange {
                range: None,
                text: "Перем Рано;\nПроцедура А()\nКонецПроцедуры\n".into(),
            }],
        },
        soon(),
    )
    .unwrap();
    let edited = published(&client, &uri, |params| params["version"] == 3);
    let marker = edited["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "DiagnosticsIncomplete")
        .expect("the unsaved buffer is named");
    assert_eq!(marker["data"]["gaps"], serde_json::json!(["unsavedBuffer"]));
    assert!(!codes(&edited["diagnostics"]).contains(&"VariableDeclarationPlacement".to_string()));
    assert_no_provider(&edited);
    assert_eq!(
        fs::read_to_string(&module).unwrap(),
        SAVED,
        "the buffer is not saved"
    );

    // Saved, the buffer is the analysed text again: the provider's part
    // comes back for the new saved text.
    fs::write(&module, "Перем Рано;\nПроцедура А()\nКонецПроцедуры\n").unwrap();
    host.notify(
        &session,
        V5LspNotification::DidSave { uri: uri.clone() },
        soon(),
    )
    .unwrap();
    let saved = published(&client, &uri, |params| {
        codes(&params["diagnostics"]).contains(&"bsl.style.line_length".to_string())
    });
    assert_eq!(saved["version"], 3);
    assert!(!codes(&saved["diagnostics"]).contains(&"DiagnosticsIncomplete".to_string()));

    // Closed, the document is told it has nothing left to show.
    host.notify(
        &session,
        V5LspNotification::DidClose { uri: uri.clone() },
        soon(),
    )
    .unwrap();
    let closed = published(&client, &uri, |params| params.get("version").is_none());
    assert_eq!(closed["diagnostics"], serde_json::json!([]));
}

/// #2176: an editor that asks for diagnostics itself is not sent them on
/// every edit; it is told when an analysis ends, so that it asks again.
#[test]
fn a_pulling_editor_is_told_only_that_an_analysis_ended() {
    let workspace = workspace();
    let (host, session, client, release, uri) =
        serve(workspace.path(), V5LspDiagnosticsDelivery::Pull);
    host.notify(
        &session,
        V5LspNotification::DidOpen {
            uri: uri.clone(),
            language_id: "bsl".into(),
            version: 1,
            text: SAVED.into(),
        },
        soon(),
    )
    .unwrap();
    assert!(
        client
            .take_for_test(Instant::now() + Duration::from_millis(200))
            .is_empty(),
        "an edit is not published to a pulling editor"
    );
    // The editor's own request starts the analysis.
    let first = pull(&host, &session, &uri, None);
    assert!(codes(&first["items"]).contains(&"DiagnosticsIncomplete".to_string()));
    release.store(true, Ordering::Release);
    let (_, analysed) = published_as(&client, &uri, |_, analysed| analysed);
    assert!(analysed);
    let complete = pull(&host, &session, &uri, None);
    assert!(codes(&complete["items"]).contains(&"bsl.style.line_length".to_string()));
}

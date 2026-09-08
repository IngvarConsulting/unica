pub(crate) mod client_v5;
pub(crate) mod identity;
pub(crate) mod protocol;
#[allow(dead_code)]
pub(crate) mod protocol_v5;
pub(crate) mod runtime_v5;
pub(crate) mod server;
pub(crate) mod terminal_codec_v5;
mod v13_infobase_exports;
mod v13_read_modes;
#[allow(dead_code)]
mod v13_service;
mod v13_workspace_bootstrap;
mod v13_workspace_initialize;

use identity::CoreIdentity;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

fn daemon_process_command(
    executable: &Path,
    state_root: &Path,
    core_identity: &CoreIdentity,
    idle_grace: Duration,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--daemon")
        .arg("--state-root")
        .arg(state_root)
        .arg("--core-identity")
        .arg(core_identity.as_str())
        .arg("--idle-grace-ms")
        .arg(idle_grace.as_millis().to_string())
        .current_dir(state_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::identity::{CoreIdentity, DaemonStateDirectory};
    use super::protocol::InvocationRequest;
    use super::server::actor_capacity_tests::{
        ensure_platform_xml_workspace, LiveV5Daemon, V5Submission,
    };
    use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};
    use super::v13_service::CanonicalV13ReadService;
    use crate::application::invocation_store::ToolIdentity;
    use crate::application::invocation_store_v5::V5SafeFailureReason;
    use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::invocation::{DomainResult, InvocationFailure, InvocationStatus};
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, set_unix_mode_for_test, unix_mode_for_test,
        FileLinkFixtureOutcome,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::Duration;

    // These waits only bound native thread coordination in integration tests.
    // They exceed the actor's seven-second operation budget so parallel
    // test-runner scheduling cannot be mistaken for the product deadline.
    const INTEGRATION_COORDINATION_TIMEOUT: Duration = Duration::from_secs(10);
    const INTEGRATION_TASK_WAIT: Duration = Duration::from_secs(15);

    fn physical_root(root: &std::path::Path) -> PathBuf {
        std::fs::canonicalize(root).unwrap()
    }

    #[test]
    fn daemon_process_is_anchored_outside_the_caller_workspace() {
        let state_root = std::env::temp_dir().join("unica-daemon-command-state");
        let command = super::daemon_process_command(
            PathBuf::from("unica").as_path(),
            &state_root,
            &CoreIdentity::production(),
            Duration::from_secs(30),
        );

        assert_eq!(command.get_current_dir(), Some(state_root.as_path()));
    }

    #[test]
    fn world_readable_identity_directory_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let path = DaemonStateDirectory::path_for(&physical, &identity);
        std::fs::create_dir_all(&path).unwrap();
        if !set_unix_mode_for_test(&path, 0o755).unwrap() {
            return;
        }

        let error = DaemonStateDirectory::open(&physical, &identity).unwrap_err();
        assert!(error.contains("owner-only"), "{error}");
        assert_eq!(unix_mode_for_test(&path).unwrap(), Some(0o755));
    }

    #[test]
    fn symlinked_provider_state_root_is_rejected_before_creating_identity_state() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("redirected-parent");
        std::fs::create_dir(&target).unwrap();
        let routed_parent = fixture.path().join("provider-parent-link");
        match create_directory_link_fixture_for_test(&target, &routed_parent).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let routed = routed_parent.join("provider-state-created-through-link");
        let identity = CoreIdentity::production();

        assert!(DaemonStateDirectory::open(&routed, &identity).is_err());
        assert!(
            !target.join("provider-state-created-through-link").exists(),
            "rejected ambient symlink must not receive even the provider-state directory"
        );
    }

    #[test]
    fn missing_provider_state_root_is_created_before_private_identity_child() {
        let fixture = tempfile::tempdir().unwrap();
        let physical_fixture = std::fs::canonicalize(fixture.path()).unwrap();
        let state_root = physical_fixture.join("cold").join("provider-state");
        let identity = CoreIdentity::production();

        let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();

        assert!(state_root.is_dir());
        assert_eq!(
            state.path(),
            DaemonStateDirectory::path_for(&state_root, &identity)
        );
    }

    #[test]
    fn injected_hidden_v13_service_executes_real_view_and_find_through_actor_capabilities() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("src");
        std::fs::create_dir_all(source.join("Catalogs")).unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        let workspace_hint = physical_root(workspace.path())
            .to_string_lossy()
            .into_owned();

        let daemon = LiveV5Daemon::start(Arc::new(CanonicalV13ReadService::default()));
        let owner = daemon.owner();

        let view = daemon.submit(
            &owner,
            &InvocationRequest::new(
                ToolIdentity::View,
                serde_json::json!({"at": "main:Catalog.Items"}),
                workspace_hint.as_str(),
                7_000,
            )
            .unwrap(),
        );
        let V5Submission::Direct(view) = view else {
            panic!("hidden view should complete inline: {view:?}")
        };
        assert!(view.ok, "{} {:?}", view.summary, view.diagnostics);
        assert_eq!(view.at.as_deref(), Some("main:Catalog.Items"));
        assert_eq!(view.data.as_ref().unwrap()["kind"], "Catalog");
        assert!(view.rev.is_some());

        let find = daemon.submit(
            &owner,
            &InvocationRequest::new(
                ToolIdentity::Resolve,
                serde_json::json!({"at": "main:Catalog.Items"}),
                workspace_hint.as_str(),
                7_000,
            )
            .unwrap(),
        );
        let find = match find {
            V5Submission::Direct(find) => *find,
            V5Submission::Task(task_id) => {
                let terminal = daemon.wait_terminal(&owner, task_id, INTEGRATION_TASK_WAIT);
                assert_eq!(terminal.status(), InvocationStatus::Completed);
                terminal
                    .completed_result()
                    .cloned()
                    .expect("the one handed-off hidden find must publish its result")
            }
        };
        assert!(find.ok, "{} {:?}", find.summary, find.diagnostics);
        let bridged = find.data.as_ref().unwrap();
        assert_eq!(bridged["at"], "main:Catalog.Items");
        assert!(
            bridged["path"]
                .as_str()
                .is_some_and(|path| !path.is_empty()),
            "{bridged}"
        );
        // Мост отвечает одним предметом: ранжированных кандидатов у него нет.
        assert_eq!(bridged.get("candidates"), None);
        // A directory of addresses and paths is not a revision snapshot.
        assert!(find.rev.is_none());

        let unknown = daemon.submit(
            &owner,
            &InvocationRequest::new(
                ToolIdentity::View,
                serde_json::json!({
                    "at": "main:Catalog.Items",
                    "raw": true,
                }),
                workspace_hint.as_str(),
                7_000,
            )
            .unwrap(),
        );
        let V5Submission::Direct(unknown) = unknown else {
            panic!("invalid hidden arguments must fail before task materialization: {unknown:?}")
        };
        assert!(!unknown.ok);
        assert!(unknown.summary.contains("unknown argument `raw`"));

        daemon.finish(owner);
    }

    struct BlockingCanonicalService {
        executions: Arc<AtomicUsize>,
        entered: mpsc::Sender<()>,
    }

    impl CanonicalInvocationService for BlockingCanonicalService {
        fn prepare(
            &self,
            _request: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            _request: &ActorBoundExecution,
            cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            self.entered.send(()).unwrap();
            while !cancellation.is_cancelled() {
                std::thread::yield_now();
            }
            Err(InvocationFailure::new("cancelled", "test cancellation"))
        }
    }

    /// One canonical invocation is one execution: polling, waiting and
    /// cancelling the Task never relaunch it, and a restarted daemon reads
    /// the same durable terminal instead of executing again.
    #[test]
    fn daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it() {
        let workspace = tempfile::tempdir().unwrap();
        let executions = Arc::new(AtomicUsize::new(0));
        let (entered, entered_wait) = mpsc::channel();
        let service: Arc<dyn CanonicalInvocationService> = Arc::new(BlockingCanonicalService {
            executions: Arc::clone(&executions),
            entered,
        });
        let mut daemon = LiveV5Daemon::start(Arc::clone(&service));
        let owner = daemon.owner();
        let request = build_request(
            workspace.path(),
            serde_json::json!({"op": "infobase.build", "args": {}}),
        );

        let task_id = daemon.task_id(&owner, &request);
        entered_wait
            .recv_timeout(INTEGRATION_COORDINATION_TIMEOUT)
            .expect("canonical invocation must enter the service within the bounded wait");
        let initial = daemon.get(&owner, task_id);
        assert_eq!(initial.status(), InvocationStatus::Working);
        assert_eq!(
            daemon.get(&owner, task_id).status(),
            InvocationStatus::Working
        );

        daemon.cancel(&owner, task_id);
        let cancelled = daemon.wait_terminal(&owner, task_id, INTEGRATION_TASK_WAIT);
        assert_eq!(cancelled.status(), InvocationStatus::Cancelled);
        assert_eq!(
            cancelled.created_at_epoch_ms(),
            initial.created_at_epoch_ms()
        );
        assert!(cancelled.updated_at_epoch_ms() >= initial.updated_at_epoch_ms());
        assert_eq!(daemon.cancel(&owner, task_id), cancelled);
        assert_eq!(daemon.get(&owner, task_id), cancelled);
        assert_eq!(executions.load(Ordering::SeqCst), 1);

        daemon.stop(owner);
        daemon.restart(service);
        let owner = daemon.owner();
        assert_eq!(
            daemon.get(&owner, task_id),
            cancelled,
            "a restarted daemon reads the durable terminal and executes nothing"
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        daemon.finish(owner);
    }

    struct BoundReadingService {
        observed: mpsc::Sender<(crate::domain::invocation::SafeIdentityHash, Vec<u8>)>,
    }

    impl CanonicalInvocationService for BoundReadingService {
        fn prepare(
            &self,
            invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            assert_eq!(invocation.tool(), ToolIdentity::Run);
            assert_eq!(
                invocation.arguments(),
                &serde_json::json!({"op": "infobase.build", "args": {}})
                    .as_object()
                    .unwrap()
                    .clone()
            );
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            assert_eq!(invocation.tool(), ToolIdentity::Run);
            let bytes = invocation
                .read_relative_file(std::path::Path::new("Module.bsl"), 1_024)
                .map_err(|_| InvocationFailure::new("workspace_changed", "bound read failed"))?;
            self.observed
                .send((invocation.workspace_identity_hash().clone(), bytes))
                .unwrap();
            Ok(DomainResult::success("actor-bound read"))
        }
    }

    struct StagedActorService {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        staged_summary: &'static str,
    }

    impl CanonicalInvocationService for StagedActorService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            let _ = invocation
                .read_relative_file(std::path::Path::new("Module.bsl"), 1_024)
                .map_err(|_| InvocationFailure::new("workspace_changed", "bound read failed"))?;
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(DomainResult::success(self.staged_summary))
        }
    }

    fn build_request(root: &std::path::Path, arguments: serde_json::Value) -> InvocationRequest {
        let root = physical_root(root);
        ensure_platform_xml_workspace(&root.to_string_lossy());
        InvocationRequest::new(ToolIdentity::Run, arguments, root.to_string_lossy(), 7_000).unwrap()
    }

    fn canonical_service_reads_only_actor_bound_roots_and_persists_the_same_identity() {
        let workspace_a = tempfile::tempdir().unwrap();
        let workspace_b = tempfile::tempdir().unwrap();
        std::fs::write(workspace_a.path().join("Module.bsl"), b"workspace A").unwrap();
        std::fs::write(workspace_b.path().join("Module.bsl"), b"workspace B").unwrap();
        let (observed, observed_wait) = mpsc::channel();
        let daemon = LiveV5Daemon::start(Arc::new(BoundReadingService { observed }));
        let owner = daemon.owner();

        let mut actor_hashes = Vec::new();
        for (workspace, expected) in [
            (workspace_a.path(), b"workspace A".as_slice()),
            (workspace_b.path(), b"workspace B".as_slice()),
        ] {
            let request = build_request(
                workspace,
                serde_json::json!({"op": "infobase.build", "args": {}}),
            );
            let task_id = daemon.task_id(&owner, &request);
            let (actor_hash, bytes) = observed_wait
                .recv_timeout(INTEGRATION_COORDINATION_TIMEOUT)
                .unwrap();
            assert_eq!(bytes, expected);
            let terminal = daemon.wait_terminal(&owner, task_id, INTEGRATION_TASK_WAIT);
            assert_eq!(terminal.status(), InvocationStatus::Completed);
            actor_hashes.push(actor_hash);
        }
        assert_ne!(
            actor_hashes[0], actor_hashes[1],
            "two roots are two actor identities"
        );

        daemon.finish(owner);
    }

    fn run_actor_swap_case(replace_root: bool) {
        let workspace_parent = tempfile::tempdir().unwrap();
        let workspace = workspace_parent.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("Module.bsl"), b"initial").unwrap();
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let staged = "STAGED_BYTES_MUST_NOT_ESCAPE_AFTER_SWAP";
        let daemon = LiveV5Daemon::start(Arc::new(StagedActorService {
            entered,
            release: Mutex::new(release_wait),
            staged_summary: staged,
        }));
        let owner = daemon.owner();
        let request = build_request(
            &workspace,
            serde_json::json!({
                "op": "infobase.build",
                "args": {"ambientRoot": "/tmp/foreign"}
            }),
        );
        let task_id = daemon.task_id(&owner, &request);
        entered_wait
            .recv_timeout(INTEGRATION_COORDINATION_TIMEOUT)
            .unwrap();
        if replace_root {
            let retained = workspace_parent.path().join("retained-old-workspace");
            std::fs::rename(&workspace, &retained).unwrap();
            std::fs::create_dir(&workspace).unwrap();
            std::fs::write(workspace.join("Module.bsl"), b"foreign replacement").unwrap();
        } else {
            std::fs::write(workspace.join("Module.bsl"), b"changed revision").unwrap();
        }
        release.send(()).unwrap();
        let terminal = daemon.wait_terminal(&owner, task_id, INTEGRATION_TASK_WAIT);
        assert_eq!(terminal.status(), InvocationStatus::Failed);
        assert!(terminal.completed_result().is_none());
        assert_eq!(
            terminal.failure_reason(),
            Some(V5SafeFailureReason::InvocationFailed)
        );
        assert!(!serde_json::to_string(&terminal).unwrap().contains(staged));

        daemon.finish(owner);
    }

    fn actor_bound_publication_rejects_root_replacement_and_hides_staged_bytes() {
        run_actor_swap_case(true);
    }

    fn actor_bound_publication_rejects_revision_swap_and_hides_staged_bytes() {
        run_actor_swap_case(false);
    }

    #[test]
    fn canonical_invocation_orchestration_is_private_to_server_facade() {
        use quote::ToTokens;
        use syn::visit::Visit;

        fn tokens(node: &impl ToTokens) -> String {
            node.to_token_stream().to_string()
        }

        fn expected_use(source: &str) -> String {
            tokens(&syn::parse_str::<syn::ItemUse>(source).expect("expected import parses"))
        }

        fn expected_visibility(source: &str) -> String {
            tokens(&syn::parse_str::<syn::Visibility>(source).expect("expected visibility parses"))
        }

        #[derive(Default)]
        struct Uses(Vec<String>);

        impl<'ast> Visit<'ast> for Uses {
            fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
                self.0.push(tokens(item));
                syn::visit::visit_item_use(self, item);
            }
        }

        let daemon = syn::parse_file(include_str!("mod.rs")).expect("daemon module parses");
        assert!(
            !daemon.items.iter().any(
                |item| matches!(item, syn::Item::Mod(module) if module.ident == "invocation_service")
            ),
            "canonical invocation orchestration must not be a daemon sibling module"
        );
        let mut daemon_uses = Uses::default();
        daemon_uses.visit_file(&daemon);
        assert!(
            !daemon_uses
                .0
                .iter()
                .any(|item| item.contains("super :: invocation_service")),
            "daemon tests must consume the canonical service seam through server"
        );
        let daemon_test_seam = expected_use(
            "use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};",
        );
        assert!(
            daemon_uses.0.contains(&daemon_test_seam),
            "daemon tests must use the exact server facade seam"
        );

        let server = syn::parse_file(include_str!("server.rs")).expect("server source parses");
        let service_module = server
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Mod(module) if module.ident == "invocation_service" => Some(module),
                _ => None,
            })
            .expect("server owns the canonical invocation implementation module");
        assert!(
            matches!(service_module.vis, syn::Visibility::Inherited),
            "the invocation implementation module must remain private to server"
        );
        let module_path = service_module
            .attrs
            .iter()
            .find_map(|attribute| match &attribute.meta {
                syn::Meta::NameValue(name_value) if name_value.path.is_ident("path") => {
                    match &name_value.value {
                        syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(path),
                            ..
                        }) => Some(path.value()),
                        _ => None,
                    }
                }
                _ => None,
            });
        assert_eq!(
            module_path.as_deref(),
            Some("invocation_service.rs"),
            "server must remain the stable producer/facade while locating the extracted owner"
        );
        let mut server_uses = Uses::default();
        server_uses.visit_file(&server);
        let public_seam = expected_use(
            "pub(crate) use self::invocation_service::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService,};",
        );
        assert!(
            server_uses.0.contains(&public_seam),
            "server must expose only the exact canonical service seam"
        );

        let v13 = syn::parse_file(include_str!("v13_service.rs")).expect("v13 service parses");
        let mut v13_uses = Uses::default();
        v13_uses.visit_file(&v13);
        let v13_seam = expected_use(
            "use super::server::{ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService};",
        );
        assert_eq!(
            v13_uses
                .0
                .iter()
                .filter(|item| item.contains("CanonicalInvocationService"))
                .collect::<Vec<_>>(),
            vec![&v13_seam],
            "the canonical v0.13 consumer must enter through the server facade"
        );

        let owner = syn::parse_file(include_str!("invocation_service.rs"))
            .expect("invocation service owner parses");
        let server_only = expected_visibility("pub(super)");
        let bind_workspace_invocation = owner
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(function) if function.sig.ident == "bind_workspace_invocation" => {
                    Some(function)
                }
                _ => None,
            })
            .expect("missing server-only function bind_workspace_invocation");
        assert_eq!(
            tokens(&bind_workspace_invocation.vis),
            server_only,
            "bind_workspace_invocation escaped server"
        );
        let admission_error = owner
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(item) if item.ident == "WorkspaceAdmissionError" => Some(item),
                _ => None,
            })
            .expect("workspace admission error exists");
        assert_eq!(
            tokens(&admission_error.vis),
            server_only,
            "workspace admission routing escaped server"
        );
        for name in ["response_deadline", "begin_execution", "publish"] {
            let methods = owner
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Impl(item) => Some(item),
                    _ => None,
                })
                .flat_map(|item| item.items.iter())
                .filter_map(|item| match item {
                    syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(methods.len(), 1, "expected one orchestration method {name}");
            assert_eq!(
                tokens(&methods[0].vis),
                server_only,
                "{name} escaped server"
            );
        }

        let daemon_visible = expected_visibility("pub(in crate::infrastructure::daemon)");
        let capability = owner
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == "ActorReadSourceCapability" => Some(item),
                _ => None,
            })
            .expect("actor read capability exists");
        assert_eq!(
            tokens(&capability.vis),
            daemon_visible,
            "the pre-existing daemon capability seam changed visibility"
        );
        for name in ["admitted_source_set_names", "admit_apply", "read_sources"] {
            let method = owner
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Impl(item) => Some(item),
                    _ => None,
                })
                .flat_map(|item| item.items.iter())
                .find_map(|item| match item {
                    syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing daemon-visible method {name}"));
            assert_eq!(
                tokens(&method.vis),
                daemon_visible,
                "the pre-existing daemon method {name} changed visibility"
            );
        }
    }

    #[test]
    fn canonical_service_boundary_exposes_no_raw_request_or_workspace_hint() {
        let source = include_str!("invocation_service.rs");
        let trait_start = source
            .find("pub(crate) trait CanonicalInvocationService")
            .expect("canonical service trait");
        let trait_end = source[trait_start..]
            .find("\n}\npub(super) fn bind_workspace_invocation")
            .expect("canonical service trait end")
            + trait_start;
        let boundary = &source[trait_start..trait_end];
        assert!(boundary.contains("&ActorBoundInvocation"));
        assert!(boundary.contains("&ActorBoundExecution"));
        assert!(!boundary.contains("InvocationRequest"));
        assert!(!boundary.contains("workspace_hint"));
    }

    #[test]
    fn canonical_invocation_authority_is_actor_bound_and_revision_fenced() {
        canonical_service_boundary_exposes_no_raw_request_or_workspace_hint();
        canonical_service_reads_only_actor_bound_roots_and_persists_the_same_identity();
        actor_bound_publication_rejects_root_replacement_and_hides_staged_bytes();
        actor_bound_publication_rejects_revision_swap_and_hides_staged_bytes();
        super::server::actor_capacity_tests::hidden_v13_logical_lease_survives_the_handoff_window_and_confirms_once();
    }

    #[test]
    fn workspace_actor_capacity_has_a_closed_retryable_protocol_code() {
        // Capacity is a closed failure reason of the receipt, retryable by the
        // caller, and its wire name never carries text.
        assert_eq!(
            serde_json::to_value(V5SafeFailureReason::WorkspaceCapacity).unwrap(),
            serde_json::json!("workspace_capacity")
        );
    }
}

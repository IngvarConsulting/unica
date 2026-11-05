use super::*;
use crate::infrastructure::daemon::server::actor_capacity_tests::{
    canonical_v13_service, LiveV5Daemon,
};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use unica_artifact_delivery::{BootstrapError, DownloadObserver, Downloader, HostTarget};

struct RefuseEngineDownload(AtomicUsize);
impl Downloader for RefuseEngineDownload {
    fn download(&self, _: &str, _: &Path, _: &dyn DownloadObserver) -> Result<(), BootstrapError> {
        self.0.fetch_add(1, Ordering::AcqRel);
        Err(std::io::Error::other("controlled offline engine downloader").into())
    }
}

fn install_cold_plugin(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let workspace = root.join("workspace");
    let plugin = root.join("plugin");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::create_dir_all(plugin.join("third-party")).unwrap();
    std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ninfobase:\n  connection: 'File=base'\n").unwrap();
    std::fs::write(workspace.join("src/Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#).unwrap();
    let mut core_targets = serde_json::Map::new();
    let mut runner_targets = serde_json::Map::new();
    let runner_version = "0.15.3";
    let digest = "0".repeat(64);
    for host in HostTarget::ALL {
        let target = host.as_str();
        let suffix = if target == "win-x64" { ".exe" } else { "" };
        let core_asset = format!("unica-runtime-{target}.tar.gz");
        let core_path = format!("bin/{target}/unica{suffix}");
        core_targets.insert(target.to_owned(), json!({
            "asset":{"name":core_asset,"url":format!("https://github.com/IngvarConsulting/unica/releases/download/v{}/{core_asset}",env!("CARGO_PKG_VERSION")),"mediaType":"application/gzip","sha256":digest},
            "files":[{"path":core_path,"sha256":digest,"executable":true}],"entrypoint":core_path
        }));
        let asset = format!("v8-runner-{target}{suffix}");
        runner_targets.insert(target.to_owned(), json!({
            "asset":{"name":asset,"url":format!("https://github.com/IngvarConsulting/v8-runner-rust/releases/download/v8-runner-v{runner_version}-build.1/{asset}"),"mediaType":"application/octet-stream","sha256":digest},
            "files":[{"path":format!("bin/{target}/v8-runner{suffix}"),"sha256":digest,"executable":true}]
        }));
    }
    let release = json!({"schemaVersion":2,"pluginVersion":env!("CARGO_PKG_VERSION"),
        "source":{"repository":"https://github.com/IngvarConsulting/unica","commit":"0".repeat(40)},
        "release":{"repository":"https://github.com/IngvarConsulting/unica","tag":format!("v{}",env!("CARGO_PKG_VERSION"))},
        "artifacts":{"unica":{"version":env!("CARGO_PKG_VERSION"),"role":"core","targets":core_targets},"v8-runner":{"version":runner_version,"role":"engine","targets":runner_targets}}
    });
    let release_path = plugin.join("runtime-manifest.json");
    std::fs::write(&release_path, release.to_string()).unwrap();
    let target = crate::infrastructure::platform::current_target_id().unwrap();
    let relative = format!("bin/{target}/v8-runner{}", std::env::consts::EXE_SUFFIX);
    std::fs::write(plugin.join("third-party/manifest.json"), json!({"schemaVersion":2,"artifactAssets":{"v8-runner":{"sha256":digest}},"tools":[{"name":"v8-runner","version":runner_version,"binaryPath":relative,"deliveredPath":relative,"sha256":digest}]}).to_string()).unwrap();
    std::fs::create_dir_all(root.join("cache")).unwrap();
    std::fs::create_dir_all(root.join("provider-state")).unwrap();
    // Only the exact, isolated test child owns these process-wide overrides.
    std::env::set_var("UNICA_PLUGIN_ROOT", &plugin);
    std::env::set_var("UNICA_RUNTIME_MANIFEST", &release_path);
    std::env::set_var("UNICA_ARTIFACT_CACHE", root.join("cache"));
    std::env::set_var("UNICA_PROVIDER_STATE_DIR", root.join("provider-state"));
    (workspace, plugin)
}

async fn initialize_profile(client: &mut McpClient, native: bool) {
    let capabilities = if native {
        json!({"extensions":{"io.modelcontextprotocol/tasks":{}}})
    } else {
        json!({})
    };
    let protocol = if native { "2026-07-28" } else { "2025-06-18" };
    client.send(json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":protocol,"capabilities":capabilities,"clientInfo":{"name":"engine-free-fixture","version":"1"}}})).await;
    let response = client.receive().await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["protocolVersion"], protocol);
    client
        .send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
}

async fn complete_catalog(client: &mut McpClient, native: bool) {
    let mut cursor = None;
    let mut names = Vec::new();
    let mut id = 1;
    loop {
        let params = cursor
            .as_ref()
            .map_or_else(|| json!({}), |cursor| json!({"cursor":cursor}));
        client
            .send(json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":params}))
            .await;
        let response = client.receive().await;
        assert!(response.get("error").is_none(), "{response}");
        for tool in response["result"]["tools"].as_array().unwrap() {
            names.push(tool["name"].as_str().unwrap().to_owned());
            jsonschema::validator_for(&tool["inputSchema"])
                .expect("input schema remains complete without its engine");
            if let Some(schema) = tool.get("outputSchema") {
                jsonschema::validator_for(schema)
                    .expect("output schema remains complete without its engine");
            }
        }
        cursor = response["result"]["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        id += 1;
        assert!(id < 10, "the real catalog repeated its continuation");
    }
    assert_v13_profile_names(&names, native);
    assert_eq!(
        names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        names.len()
    );
}

/// rule: INV.WIRE.ENGINE-FREE-HANDSHAKE
#[tokio::test]
async fn engine_free_mcp_construction_and_discovery_with_cold_engines() {
    const CHILD_ROOT: &str = "UNICA_ENGINE_FREE_966_CHILD_ROOT";
    if std::env::var_os(CHILD_ROOT).is_none() {
        let root = tempfile::tempdir().unwrap();
        let name = concat!(
            module_path!(),
            "::engine_free_mcp_construction_and_discovery_with_cold_engines"
        );
        let name = name.split_once("::").unwrap().1;
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD_ROOT, root.path())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test") && stdout.contains(name),
            "the exact child must execute the contract test: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap());
    let (workspace, plugin) = install_cold_plugin(&root);
    let before = unica_workspace::actor::test_support::snapshot_tree(&root);
    let downloader = Arc::new(RefuseEngineDownload(AtomicUsize::new(0)));
    let attempts = || {
        [
            unica_adapter_bsl_analyzer::session::preparation_attempts_for_test(),
            unica_adapter_rlm::session::preparation_attempts_for_test(),
            unica_adapter_rlm::index::preparation_attempts_for_test(),
        ]
    };
    assert_eq!(attempts(), [0, 0, 0]);
    for native in [false, true] {
        let daemon = LiveV5Daemon::start_with_delivery_downloader(
            canonical_v13_service(),
            downloader.clone(),
        );
        let owner = daemon.owner();
        let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            daemon.owner(),
            workspace.to_string_lossy().into_owned(),
        ));
        initialize_profile(&mut client, native).await;
        complete_catalog(&mut client, native).await;
        client.shutdown().await;
        daemon.finish(owner);
        assert_eq!(downloader.0.load(Ordering::Acquire), 0);
        assert_eq!(
            attempts(),
            [0, 0, 0],
            "discovery attempted provider/index preparation"
        );
        assert!(
            !plugin.join("bin").exists(),
            "discovery acquired a bundled engine"
        );
        assert!(
            !workspace.join(".build").exists(),
            "discovery started workspace/index preparation"
        );
        assert!(std::fs::read_dir(root.join("provider-state"))
            .unwrap()
            .next()
            .is_none());
        assert_eq!(
            unica_workspace::actor::test_support::snapshot_tree(&root),
            before,
            "constructor, handshake and catalog changed cold engine/index state"
        );
    }
    // Exercise every observer at its real entry point after the zero assertions.
    // Cancellation prevents the controls from acquiring external engines.
    let context = unica_source_contract::workspace::WorkspaceContext {
        cwd: workspace.clone(),
        workspace_root: workspace.clone(),
        cache_root: workspace.join(".build/unica"),
        workspace_epoch: 1,
    };
    let cancellation = unica_work_control::cancellation::CancellationToken::new();
    cancellation.cancel();
    let environment = Arc::new(crate::infrastructure::analysis_composition::AnalysisEnvironment);
    let bsl = unica_adapter_bsl_analyzer::session::BslWorkspaceBackend::new(environment.clone());
    let error = unica_analysis_services::backend::BslBackend::start(
        &bsl,
        &context,
        &workspace.join("src"),
        &cancellation,
    )
    .err()
    .expect("cancelled BSL start must refuse");
    assert!(error.starts_with("cancelled:"), "{error}");
    let error = unica_adapter_rlm::session::start_rlm_transport_pinned(
        &context,
        &workspace.join("src"),
        &root.join("unused-build"),
        &cancellation,
        environment.as_ref(),
    )
    .err()
    .expect("cancelled RLM start must refuse");
    assert!(error.starts_with("cancelled:"), "{error}");
    let report = unica_adapter_rlm::index::WorkspaceIndexService::new(environment.as_ref())
        .start_isolated_build(&context, &workspace.join("src"), &cancellation);
    assert!(
        matches!(report.maintenance,unica_analysis_services::protocol::IndexMaintenanceStatus::StartFailed(ref error) if error.starts_with("cancelled:")),
        "{report:?}"
    );
    assert_eq!(
        attempts(),
        [1, 1, 1],
        "provider preparation observers are disconnected"
    );
    // Positive control: the same production composition must reach this
    // downloader when an actual engine-requiring operation is requested.
    let daemon =
        LiveV5Daemon::start_with_delivery_downloader(canonical_v13_service(), downloader.clone());
    let owner = daemon.owner();
    let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
        daemon.owner(),
        workspace.to_string_lossy().into_owned(),
    ));
    initialize_profile(&mut client, false).await;
    client.send(json!({"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"extensions.list","args":{},"dryRun":true},"_meta":modern_meta()}})).await;
    let response = client.receive().await;
    if let Some(task) = response
        .pointer("/result/structuredContent/data/task/taskId")
        .and_then(Value::as_str)
    {
        daemon.wait_terminal(&owner, task.parse().unwrap(), TEST_STEP);
    }
    timeout(TEST_STEP, async {
        while downloader.0.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("engine-required positive control did not reach the installed downloader");
    client.shutdown().await;
    daemon.finish(owner);
}

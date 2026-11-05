use super::*;
use crate::infrastructure::daemon::server::actor_capacity_tests::{
    canonical_v13_service, LiveV5Daemon,
};
use crate::infrastructure::operational_config::{load_observer, load_operational_config};
use std::sync::atomic::{AtomicUsize, Ordering};

async fn catalog(client: &mut McpClient, native: bool) -> Vec<Value> {
    let version = if native { "2026-07-28" } else { "2025-06-18" };
    let capabilities = if native {
        json!({"extensions":{"io.modelcontextprotocol/tasks":{}}})
    } else {
        json!({})
    };
    client.send(json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":version,"capabilities":capabilities,"clientInfo":{"name":"operational-discovery","version":"1"}}})).await;
    let response = client.receive().await;
    assert!(response.get("error").is_none(), "{response}");
    client
        .send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    let mut all = Vec::new();
    let mut cursor = None;
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
        all.extend(
            response["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        cursor = response["result"]["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
        id += 1;
        assert!(id < 10);
    }
    let names = all
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_v13_profile_names(&names, native);
    all.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    all
}

#[tokio::test]
async fn mcp_construction_and_discovery_never_load_workspace_operational_settings() {
    for native in [false, true] {
        let mut baseline = None;
        for kind in 0..3 {
            let directory = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(directory.path()).unwrap();
            let settings = root.join("unica.toml");
            match kind {
            0=>std::fs::write(&settings,"[operational.code_intelligence]\nprovider_read_timeout_seconds=7111703\n").unwrap(),
            1=>std::fs::write(&settings,"[operational.code_intelligence]\nprovider_read_timeout_seconds='invalid-setting-secret-969'\n").unwrap(),
            _=>std::fs::create_dir(&settings).unwrap(),
        }
            let before = unica_workspace::actor::test_support::snapshot_tree(&root);
            let loads = Arc::new(AtomicUsize::new(0));
            let _observer = load_observer::install(
                &root,
                Arc::new({
                    let loads = loads.clone();
                    move |_| {
                        loads.fetch_add(1, Ordering::AcqRel);
                    }
                }),
            );
            let daemon = LiveV5Daemon::start(canonical_v13_service());
            let owner = daemon.owner();
            let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
                daemon.owner(),
                root.to_string_lossy().into_owned(),
            ));
            let tools = catalog(&mut client, native).await;
            let serialized = serde_json::to_string(&tools).unwrap();
            for secret in [
                "7111703",
                "8111907",
                "invalid-setting-secret-969",
                root.to_str().unwrap(),
            ] {
                assert!(!serialized.contains(secret), "settings leaked into schemas");
            }
            if let Some(expected) = &baseline {
                assert_eq!(&tools, expected);
            } else {
                baseline = Some(tools);
            }
            client.shutdown().await;
            daemon.finish(owner);
            assert_eq!(
                loads.load(Ordering::Acquire),
                0,
                "constructor/discovery read operational settings"
            );
            assert_eq!(
                unica_workspace::actor::test_support::snapshot_tree(&root),
                before
            );
            // Positive control for the cross-thread observer after zero assertions.
            assert_eq!(load_operational_config(&root).is_ok(), kind == 0);
            assert_eq!(loads.load(Ordering::Acquire), 1);
        }
    }
}

//! The packaged bootstrap verifies the runtime it installed before handing it to
//! a host. Its probes speak to the real `unica` binary here, not a scripted
//! fake: a catalog change in the server must break this test, not the release.

#[path = "platform/daemon_teardown.rs"]
mod daemon_teardown;

use std::path::{Path, PathBuf};
use std::time::Duration;

use unica_bootstrap::{verify_mcp_runtime, RuntimeHandoff};

/// rule: CTR.WIRE.CANONICAL-TOOL-PROFILES
/// rule: INV.WIRE.GUARANTEED-VERSIONS
#[test]
fn bootstrap_verification_accepts_the_catalogs_the_real_runtime_publishes() {
    let root = temp_root();
    let state = root.join("provider-state");
    std::fs::create_dir_all(&state).unwrap();
    let plugin_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/unica");
    let plugin_root = std::fs::canonicalize(plugin_root).expect("plugin root");

    let verified = verify_mcp_runtime(
        Path::new(env!("CARGO_BIN_EXE_unica")),
        &plugin_root,
        &RuntimeHandoff {
            provider_state_root: &state,
            artifact_cache: &root.join("artifact-cache"),
            runtime_manifest: &plugin_root.join("runtime-manifest.json"),
            startup_notice: None,
        },
        Duration::from_secs(30),
    );
    daemon_teardown::stop_daemons_under(&state);
    let _ = std::fs::remove_dir_all(&root);

    verified.expect("bootstrap verification of the real runtime");
}

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "unica-bootstrap-runtime-verification-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

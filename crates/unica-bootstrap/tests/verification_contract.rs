#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use unica_bootstrap::verify_mcp_runtime;

#[test]
fn verify_requires_initialize_and_the_three_public_tools() {
    let root = temp_root("valid");
    let runtime = write_fake_runtime(&root, true);

    verify_mcp_runtime(&runtime, &root, Duration::from_secs(2)).unwrap();
}

#[test]
fn verify_rejects_incomplete_tools_list() {
    let root = temp_root("missing-tool");
    let runtime = write_fake_runtime(&root, false);

    let error = verify_mcp_runtime(&runtime, &root, Duration::from_secs(2)).unwrap_err();

    assert!(error.to_string().contains("unica.standards.explain"));
}

fn write_fake_runtime(root: &Path, complete: bool) -> PathBuf {
    let path = root.join("fake-unica");
    let explain = if complete {
        r#",{"name":"unica.standards.explain"}"#
    } else {
        ""
    };
    fs::write(
        &path,
        format!(
            r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2025-06-18","capabilities":{{}},"serverInfo":{{"name":"unica","version":"0.7.0"}}}}}}' ;;
    *'"id":2'*) printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"unica.project.status"}},{{"name":"unica.standards.search"}}{explain}]}}}}' ;;
  esac
done
"#
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-verification-{name}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    root
}

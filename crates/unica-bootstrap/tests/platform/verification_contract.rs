#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use unica_bootstrap::{verify_mcp_runtime, Failure};

#[test]
fn verify_requires_both_lifecycles_and_the_three_public_tools() {
    let root = temp_root("valid");
    let record = root.join("provider-state.txt");
    let runtime = write_fake_runtime(&root, &record, true, true);
    let provider_state = root.join("private-provider-state");

    verify_mcp_runtime(&runtime, &root, &provider_state, Duration::from_secs(2)).unwrap();

    assert_eq!(
        fs::read_to_string(record).unwrap(),
        provider_state.display().to_string()
    );
}

#[test]
fn verify_rejects_incomplete_tools_list() {
    let root = temp_root("missing-tool");
    let record = root.join("provider-state.txt");
    let runtime = write_fake_runtime(&root, &record, false, true);
    let provider_state = root.join("private-provider-state");

    let error =
        verify_mcp_runtime(&runtime, &root, &provider_state, Duration::from_secs(2)).unwrap_err();

    assert!(error.to_string().contains("unica.standards.explain"));
    assert_eq!(
        fs::read_to_string(record).unwrap(),
        provider_state.display().to_string()
    );
}

#[test]
fn verify_rejects_discover_without_the_guaranteed_versions() {
    let root = temp_root("stale-discover");
    let record = root.join("provider-state.txt");
    let runtime = write_fake_runtime(&root, &record, true, false);
    let provider_state = root.join("private-provider-state");

    let error =
        verify_mcp_runtime(&runtime, &root, &provider_state, Duration::from_secs(2)).unwrap_err();

    assert!(error
        .to_string()
        .contains("does not list guaranteed protocol version"));
}

fn write_fake_runtime(
    root: &Path,
    provider_state_record: &Path,
    complete: bool,
    modern: bool,
) -> PathBuf {
    let path = root.join("fake-unica");
    let explain = if complete {
        r#",{"name":"unica.standards.explain"}"#
    } else {
        ""
    };
    let supported = if modern {
        r#"["2025-06-18","2025-11-25","2026-07-28"]"#
    } else {
        r#"["2025-06-18"]"#
    };
    fs::write(
        &path,
        format!(
            r#"#!/bin/sh
printf '%s' "$UNICA_PROVIDER_STATE_DIR" > '{record}'
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2025-06-18","capabilities":{{}},"serverInfo":{{"name":"unica","version":"0.7.0"}}}}}}' ;;
    *'"method":"server/discover"'*) printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","supportedVersions":{supported},"capabilities":{{}},"ttlMs":0,"cacheScope":"private"}}}}' ;;
    *'"method":"tools/list"'*) printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"unica.project.status"}},{{"name":"unica.standards.search"}}{explain}]}}}}' ;;
  esac
done
"#,
            record = provider_state_record.display()
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

#[test]
fn a_runtime_that_never_answers_is_a_timeout_not_a_defect() {
    // Единственный настоящий срок в загрузчике — рукопожатие релизного шлюза:
    // рантайм либо ответил, либо нет. У доставки срока нет, и путать эти два
    // отказа одним кодом выхода значит скрывать, какой из них случился.
    let root = temp_root("silent");
    let runtime = root.join("silent-runtime.sh");
    // Провод открыт, ответа нет: ровно тот случай, ради которого срок и стоит.
    fs::write(&runtime, "#!/bin/sh\nexec sleep 5\n").unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();

    let error = verify_mcp_runtime(
        &runtime,
        &root,
        &root.join("private-provider-state"),
        Duration::from_millis(300),
    )
    .unwrap_err();

    assert!(error.to_string().contains("timed out"), "{error}");
    assert_eq!(error.failure(), Failure::Timeout);
    assert_eq!(error.exit_code(), 75);
}

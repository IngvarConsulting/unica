#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use unica_bootstrap::{verify_mcp_runtime, Failure};

#[test]
fn verify_requires_both_lifecycles_and_the_three_public_tools() {
    let root = temp_root("valid");
    let record = root.join("provider-state.txt");
    let runtime = write_fake_runtime(
        &root,
        &record,
        None,
        None,
        &["2025-06-18", "2025-11-25", "2026-07-28"],
    );
    let provider_state = root.join("private-provider-state");

    verify_mcp_runtime(&runtime, &root, &provider_state, Duration::from_secs(2)).unwrap();

    assert_eq!(
        fs::read_to_string(record).unwrap(),
        provider_state.display().to_string()
    );
    assert_eq!(
        fs::read_to_string(root.join("requests.txt")).unwrap(),
        concat!(
            "initialize\n",
            "notifications/initialized\n",
            "tools/list\n",
            "server/discover\n",
            "tools/list\n",
        )
    );
}

#[test]
fn verify_requires_each_lifecycle_to_expose_each_public_tool() {
    const REQUIRED: [&str; 3] = [
        "unica.project.status",
        "unica.standards.search",
        "unica.standards.explain",
    ];
    for lifecycle in ["legacy", "direct"] {
        for missing in REQUIRED {
            let root = temp_root(&format!(
                "missing-{lifecycle}-{}",
                missing.replace('.', "-")
            ));
            let record = root.join("provider-state.txt");
            let (legacy_missing, direct_missing) = if lifecycle == "legacy" {
                (Some(missing), None)
            } else {
                (None, Some(missing))
            };
            let runtime = write_fake_runtime(
                &root,
                &record,
                legacy_missing,
                direct_missing,
                &["2025-06-18", "2025-11-25", "2026-07-28"],
            );

            let error = verify_mcp_runtime(
                &runtime,
                &root,
                &root.join("private-provider-state"),
                Duration::from_secs(2),
            )
            .unwrap_err();

            assert!(
                error.to_string().contains(missing),
                "{lifecycle} tools/list without {missing} must fail by that name: {error}"
            );
        }
    }
}

#[test]
fn verify_rejects_discover_without_the_guaranteed_versions() {
    const GUARANTEED: [&str; 3] = ["2025-06-18", "2025-11-25", "2026-07-28"];
    for missing in GUARANTEED {
        let root = temp_root(&format!("discover-without-{missing}"));
        let record = root.join("provider-state.txt");
        let supported = GUARANTEED
            .into_iter()
            .filter(|version| *version != missing)
            .collect::<Vec<_>>();
        let runtime = write_fake_runtime(&root, &record, None, None, &supported);
        let provider_state = root.join("private-provider-state");

        let error = verify_mcp_runtime(&runtime, &root, &provider_state, Duration::from_secs(2))
            .unwrap_err();

        assert!(
            error.to_string().contains(missing),
            "omitting {missing} must name that guaranteed version: {error}"
        );
    }
}

fn write_fake_runtime(
    root: &Path,
    provider_state_record: &Path,
    legacy_missing_tool: Option<&str>,
    direct_missing_tool: Option<&str>,
    supported_versions: &[&str],
) -> PathBuf {
    let path = root.join("fake-unica");
    let legacy_tools = tools_list_response(legacy_missing_tool);
    let direct_tools = tools_list_response(direct_missing_tool);
    let supported = serde_json::to_string(supported_versions).unwrap();
    let requests = root.join("requests.txt");
    let tools_list_seen = root.join("tools-list-seen");
    fs::write(
        &path,
        format!(
            r#"#!/bin/sh
printf '%s' "$UNICA_PROVIDER_STATE_DIR" > '{record}'
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' initialize >> '{requests}'; printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2025-06-18","capabilities":{{}},"serverInfo":{{"name":"unica","version":"0.7.0"}}}}}}' ;;
    *'"method":"notifications/initialized"'*) printf '%s\n' notifications/initialized >> '{requests}' ;;
    *'"method":"server/discover"'*) printf '%s\n' server/discover >> '{requests}'; printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"resultType":"complete","supportedVersions":{supported},"capabilities":{{}},"ttlMs":0,"cacheScope":"private"}}}}' ;;
	    *'"method":"tools/list"'*)
	      printf '%s\n' tools/list >> '{requests}'
	      if [ -e '{tools_list_seen}' ]; then
	        printf '%s\n' '{direct_tools}'
	      else
	        : > '{tools_list_seen}'
	        printf '%s\n' '{legacy_tools}'
	      fi
	      ;;
  esac
done
"#,
	            record = provider_state_record.display(),
	            requests = requests.display(),
	            tools_list_seen = tools_list_seen.display(),
	        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn tools_list_response(missing: Option<&str>) -> String {
    let tools = [
        "unica.project.status",
        "unica.standards.search",
        "unica.standards.explain",
    ]
    .into_iter()
    .filter(|name| Some(*name) != missing)
    .map(|name| json!({"name": name}))
    .collect::<Vec<_>>();
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": {"tools": tools},
    }))
    .unwrap()
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

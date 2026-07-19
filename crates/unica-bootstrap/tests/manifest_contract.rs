use std::collections::BTreeSet;

use unica_bootstrap::{HostTarget, RuntimeManifest};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn target(target: &str, entrypoint: &str) -> serde_json::Value {
    serde_json::json!({
        "asset": {
            "name": format!("unica-runtime-{target}.tar.gz"),
            "url": format!(
                "https://github.com/IngvarConsulting/unica/releases/download/v0.7.0/unica-runtime-{target}.tar.gz"
            ),
            "mediaType": "application/gzip",
            "sha256": HASH
        },
        "files": [{"path": entrypoint, "sha256": HASH, "executable": true}],
        "entrypoint": entrypoint
    })
}

fn fixture() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "pluginVersion": "0.7.0",
        "source": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "commit": COMMIT
        },
        "release": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "tag": "v0.7.0"
        },
        "targets": {
            "darwin-arm64": target("darwin-arm64", "bin/darwin-arm64/unica"),
            "linux-x64": target("linux-x64", "bin/linux-x64/unica"),
            "win-x64": target("win-x64", "bin/win-x64/unica.exe")
        }
    })
}

fn parse(value: serde_json::Value) -> RuntimeManifest {
    serde_json::from_value(value).expect("fixture must deserialize")
}

#[test]
fn valid_manifest_selects_the_requested_target() {
    let manifest = parse(fixture());

    manifest.validate("0.7.0").expect("manifest must validate");

    assert_eq!(
        manifest
            .target(HostTarget::LinuxX64)
            .expect("linux target")
            .entrypoint,
        "bin/linux-x64/unica"
    );
}

#[test]
fn manifest_rejects_plugin_version_mismatch_before_target_selection() {
    let manifest = parse(fixture());

    let error = manifest.validate("0.7.1").expect_err("version mismatch");

    assert!(error.to_string().contains("plugin version 0.7.0 != 0.7.1"));
}

#[test]
fn manifest_rejects_non_release_origin() {
    let mut value = fixture();
    value["targets"]["linux-x64"]["asset"]["url"] =
        serde_json::Value::String("https://example.invalid/unica.tar.gz".to_string());
    let manifest = parse(value);

    let error = manifest.validate("0.7.0").expect_err("origin mismatch");

    assert!(error.to_string().contains("release origin"));
}

#[test]
fn manifest_rejects_parent_traversal_and_missing_entrypoint() {
    let mut value = fixture();
    value["targets"]["linux-x64"]["files"][0]["path"] =
        serde_json::Value::String("../unica".to_string());
    let manifest = parse(value);

    let error = manifest.validate("0.7.0").expect_err("path traversal");

    assert!(error.to_string().contains("unsafe runtime file path"));
}

#[test]
fn target_detection_accepts_git_for_windows_uname() {
    assert_eq!(
        HostTarget::detect("MINGW64_NT-10.0", "x86_64").expect("Git for Windows"),
        HostTarget::WinX64
    );
    assert_eq!(
        HostTarget::detect("Darwin", "arm64").expect("Apple Silicon"),
        HostTarget::DarwinArm64
    );
    assert_eq!(
        HostTarget::detect("linux", "amd64").expect("Linux x64"),
        HostTarget::LinuxX64
    );
}

#[test]
fn target_detection_rejects_unsupported_host() {
    let error = HostTarget::detect("Linux", "aarch64").expect_err("unsupported host");

    assert!(error
        .to_string()
        .contains("unsupported Unica host: Linux-aarch64"));
}

#[test]
fn manifest_has_exactly_three_named_targets() {
    let manifest = parse(fixture());
    let keys = manifest
        .targets
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    assert_eq!(
        keys,
        BTreeSet::from(["darwin-arm64", "linux-x64", "win-x64"])
    );
}

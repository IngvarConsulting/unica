use std::collections::BTreeSet;

use unica_bootstrap::{Failure, HostTarget, RuntimeManifest};

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
        "schemaVersion": 2,
        "pluginVersion": "0.7.0",
        "source": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "commit": COMMIT
        },
        "release": {
            "repository": "https://github.com/IngvarConsulting/unica",
            "tag": "v0.7.0"
        },
        "artifacts": {
            "unica": {
                "version": "0.7.0",
                "role": "core",
                "targets": {
                    "darwin-arm64": target("darwin-arm64", "bin/darwin-arm64/unica"),
                    "linux-x64": target("linux-x64", "bin/linux-x64/unica"),
                    "win-x64": target("win-x64", "bin/win-x64/unica.exe")
                }
            }
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
            .entrypoint
            .as_deref(),
        Some("bin/linux-x64/unica")
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
    value["artifacts"]["unica"]["targets"]["linux-x64"]["asset"]["url"] =
        serde_json::Value::String("https://example.invalid/unica.tar.gz".to_string());
    let manifest = parse(value);

    let error = manifest.validate("0.7.0").expect_err("origin mismatch");

    assert!(error.to_string().contains("release origin"));
}

#[test]
fn manifest_rejects_parent_traversal_and_missing_entrypoint() {
    let mut value = fixture();
    value["artifacts"]["unica"]["targets"]["linux-x64"]["files"][0]["path"] =
        serde_json::Value::String("../unica".to_string());
    let manifest = parse(value);

    let error = manifest.validate("0.7.0").expect_err("path traversal");

    assert!(error.to_string().contains("unsafe runtime file path"));
}

#[test]
fn a_core_target_without_an_entrypoint_is_rejected() {
    let mut value = fixture();
    value["artifacts"]["unica"]["targets"]["linux-x64"]
        .as_object_mut()
        .expect("target")
        .remove("entrypoint");

    let error = parse(value)
        .validate("0.7.0")
        .expect_err("missing core entrypoint");

    assert!(error.to_string().contains("core entrypoint"), "{error}");
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
        .core()
        .expect("core artifact")
        .targets
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    assert_eq!(
        keys,
        BTreeSet::from(["darwin-arm64", "linux-x64", "win-x64"])
    );
}

/// Движок в манифесте: приезжает из тулчейна по своему тегу и своему имени.
fn engine_target(target: &str) -> serde_json::Value {
    serde_json::json!({
        "asset": {
            "name": format!("rlm-tools-bsl-{target}.tar.gz"),
            "url": format!(
                "https://github.com/IngvarConsulting/unica-toolchain/releases/download/rlm-tools-bsl-v1.33.0-build.3/rlm-tools-bsl-{target}.tar.gz"
            ),
            "mediaType": "application/gzip",
            "sha256": HASH
        },
        "files": [{"path": "rlm-bsl-index", "sha256": HASH, "executable": true}]
    })
}

fn fixture_with_engine() -> serde_json::Value {
    let mut manifest = fixture();
    manifest["artifacts"]["rlm-tools-bsl"] = serde_json::json!({
        "version": "1.33.0",
        "role": "engine",
        "targets": {
            "darwin-arm64": engine_target("darwin-arm64"),
            "linux-x64": engine_target("linux-x64"),
            "win-x64": engine_target("win-x64")
        }
    });
    manifest
}

#[test]
fn an_engine_is_accepted_from_the_toolchain_release() {
    // Тулчейн уже публикует по архиву на инструмент, с суммами и
    // происхождением. Копия тех же байтов в выпуске плагина стоила 242 МБ на
    // выпуск и не давала ничего.
    let manifest = parse(fixture_with_engine());

    manifest
        .validate("0.7.0")
        .expect("toolchain origin is approved");
}

#[test]
fn an_engine_target_may_not_declare_a_core_entrypoint() {
    let mut value = fixture_with_engine();
    value["artifacts"]["rlm-tools-bsl"]["targets"]["linux-x64"]["entrypoint"] =
        serde_json::json!("rlm-bsl-index");

    let error = parse(value)
        .validate("0.7.0")
        .expect_err("engine entrypoint is not a core launch contract");

    assert!(error.to_string().contains("engine entrypoint"), "{error}");
}

#[test]
fn a_missing_artifact_is_a_configuration_failure_everywhere() {
    let manifest = parse(fixture());

    let direct = manifest
        .artifact("rlm-tools-bsl")
        .expect_err("missing artifact");
    let target = manifest
        .artifact_target("rlm-tools-bsl", HostTarget::LinuxX64)
        .expect_err("missing artifact target");

    assert_eq!(direct.failure(), Failure::Configuration);
    assert_eq!(target.failure(), Failure::Configuration);
    assert_eq!(direct.to_string(), target.to_string());
}

#[test]
fn an_engine_from_an_unapproved_origin_is_refused() {
    let mut value = fixture_with_engine();
    value["artifacts"]["rlm-tools-bsl"]["targets"]["linux-x64"]["asset"]["url"] =
        serde_json::json!("https://example.com/rlm-tools-bsl-linux-x64.tar.gz");

    let error = parse(value).validate("0.7.0").unwrap_err();

    assert!(error.to_string().contains("release origin"), "{error}");
}

#[test]
fn an_engine_may_not_borrow_the_core_origin() {
    // Ядро собирается здесь, движок — нет. Смешать источники значит потерять
    // то, ради чего они названы поимённо.
    let mut value = fixture_with_engine();
    value["artifacts"]["rlm-tools-bsl"]["targets"]["linux-x64"]["asset"]["url"] = serde_json::json!(
        "https://github.com/IngvarConsulting/unica/releases/download/v0.7.0/rlm-tools-bsl-linux-x64.tar.gz"
    );

    let error = parse(value).validate("0.7.0").unwrap_err();

    assert!(error.to_string().contains("release origin"), "{error}");
}

#[test]
fn the_core_may_not_wander_off_to_the_toolchain() {
    let mut value = fixture();
    value["artifacts"]["unica"]["targets"]["linux-x64"]["asset"]["url"] = serde_json::json!(
        "https://github.com/IngvarConsulting/unica-toolchain/releases/download/v0.7.0/unica-runtime-linux-x64.tar.gz"
    );

    let error = parse(value).validate("0.7.0").unwrap_err();

    assert!(error.to_string().contains("release origin"), "{error}");
}

#[test]
fn an_engine_asset_name_is_not_forced_into_the_core_shape() {
    // Имя ассета у движка — то, под которым он опубликован в тулчейне, а не
    // выдуманное нами `<артефакт>-runtime-<цель>`.
    let manifest = parse(fixture_with_engine());
    let asset = &manifest
        .artifact_target("rlm-tools-bsl", HostTarget::LinuxX64)
        .expect("engine target")
        .asset;

    assert_eq!(asset.name, "rlm-tools-bsl-linux-x64.tar.gz");
}

/// Артефакт, изданный одним файлом: `bsl-analyzer` и `v8-runner` в тулчейне
/// лежат голыми бинарями, а расширения и обработки лягут так же.
fn file_target(target: &str) -> serde_json::Value {
    let name = if target == "win-x64" {
        "bsl-analyzer-win-x64.exe".to_string()
    } else {
        format!("bsl-analyzer-{target}")
    };
    serde_json::json!({
        "asset": {
            "name": name,
            "url": format!(
                "https://github.com/IngvarConsulting/unica-toolchain/releases/download/bsl-analyzer-v0.2.67-build.1/{name}"
            ),
            "mediaType": "application/octet-stream",
            "sha256": HASH
        },
        "files": [{"path": name, "sha256": HASH, "executable": true}]
    })
}

fn fixture_with_file_artifact() -> serde_json::Value {
    let mut manifest = fixture();
    manifest["artifacts"]["bsl-analyzer"] = serde_json::json!({
        "version": "0.2.67",
        "role": "engine",
        "targets": {
            "darwin-arm64": file_target("darwin-arm64"),
            "linux-x64": file_target("linux-x64"),
            "win-x64": file_target("win-x64")
        }
    });
    manifest
}

#[test]
fn an_artifact_delivered_as_one_file_is_accepted() {
    parse(fixture_with_file_artifact())
        .validate("0.7.0")
        .expect("a bare asset is a delivery form, not a defect");
}

#[test]
fn a_one_file_artifact_declares_exactly_the_file_it_delivers() {
    // Форма «один файл» ничего не распаковывает, поэтому перечислять больше
    // одного файла ей нечем: лишняя строка описывала бы то, чего не приедет.
    let mut value = fixture_with_file_artifact();
    value["artifacts"]["bsl-analyzer"]["targets"]["linux-x64"]["files"] = serde_json::json!([
        {"path": "bsl-analyzer-linux-x64", "sha256": HASH, "executable": true},
        {"path": "extra", "sha256": HASH, "executable": false}
    ]);

    let error = parse(value).validate("0.7.0").unwrap_err();

    assert!(error.to_string().contains("single file"), "{error}");
}

#[test]
fn the_core_is_still_required_to_arrive_as_an_archive() {
    // Ядро несёт бинарь и его окружение: одним файлом оно не бывает.
    let mut value = fixture();
    value["artifacts"]["unica"]["targets"]["linux-x64"]["asset"]["mediaType"] =
        serde_json::json!("application/octet-stream");

    let error = parse(value).validate("0.7.0").unwrap_err();

    assert!(error.to_string().contains("mediaType"), "{error}");
}

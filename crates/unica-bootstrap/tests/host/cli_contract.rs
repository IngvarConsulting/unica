use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn removed_migration_command_fails_before_any_codex_state_change() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must be after the Unix epoch")
        .as_nanos();
    let codex_home = std::env::temp_dir().join(format!(
        "unica-bootstrap-removed-migration-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&codex_home).expect("isolated CODEX_HOME must be created");

    let plugin_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/unica");
    let output = Command::new(env!("CARGO_BIN_EXE_unica-bootstrap"))
        .arg("migrate")
        .arg("--plugin-root")
        .arg(plugin_root)
        .env("CODEX_HOME", &codex_home)
        .env("PATH", &codex_home)
        .output()
        .expect("bootstrap process must start");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    assert!(
        stderr.contains("unknown bootstrap command: migrate"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(
        fs::read_dir(&codex_home)
            .expect("isolated CODEX_HOME must remain readable")
            .count(),
        0,
        "a removed command must not touch Codex state"
    );

    fs::remove_dir_all(codex_home).expect("isolated CODEX_HOME must be removable");
}

#[test]
fn a_broken_installation_names_its_reason_its_place_and_its_cure() {
    // Каталог плагина без манифеста: ставить нечего. Сказать об этом надо так,
    // чтобы не пришлось читать исходники, — и кодом выхода, а не только текстом.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must be after the Unix epoch")
        .as_nanos();
    let scratch = std::env::temp_dir().join(format!(
        "unica-bootstrap-broken-install-{}-{nonce}",
        std::process::id()
    ));
    let plugin_root = scratch.join("plugin");
    fs::create_dir_all(&plugin_root).expect("empty plugin root must be created");

    let output = Command::new(env!("CARGO_BIN_EXE_unica-bootstrap"))
        .arg("run")
        .arg("--plugin-root")
        .arg(&plugin_root)
        .env("UNICA_RUNTIME_CACHE_DIR", scratch.join("cache"))
        .env("UNICA_PROVIDER_STATE_DIR", scratch.join("state"))
        .output()
        .expect("bootstrap process must start");

    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    assert_eq!(
        output.status.code(),
        Some(78),
        "негодная поставка отличается кодом выхода: {stderr}"
    );
    assert!(
        stderr.contains("runtime-manifest.json"),
        "место названо: {stderr}"
    );
    assert!(
        stderr.contains("reason: configuration"),
        "причина названа: {stderr}"
    );
    assert!(stderr.contains("cure:"), "лечение названо: {stderr}");

    fs::remove_dir_all(scratch).expect("scratch must be removable");
}

#[test]
fn prefetch_names_its_reason_when_there_is_nothing_to_prefetch() {
    // Образ собирают в конвейере, и разбирать там нечего: код выхода решает,
    // упала сборка или нет, а текст говорит человеку, что чинить.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock must be after the Unix epoch")
        .as_nanos();
    let scratch = std::env::temp_dir().join(format!(
        "unica-bootstrap-prefetch-empty-{}-{nonce}",
        std::process::id()
    ));
    let plugin_root = scratch.join("plugin");
    fs::create_dir_all(&plugin_root).expect("empty plugin root must be created");

    let output = Command::new(env!("CARGO_BIN_EXE_unica-bootstrap"))
        .arg("prefetch")
        .arg("--plugin-root")
        .arg(&plugin_root)
        .env("UNICA_RUNTIME_CACHE_DIR", scratch.join("cache"))
        .output()
        .expect("bootstrap process must start");

    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    assert!(
        !output.status.success(),
        "прогрев без манифеста обязан провалить сборку образа: {stderr}"
    );
    assert!(
        stderr.contains("runtime-manifest.json"),
        "отказ обязан назвать место: {stderr}"
    );
    assert_ne!(
        output.status.code(),
        Some(1),
        "код выхода различает причину, а не сваливает всё в единицу: {stderr}"
    );

    fs::remove_dir_all(scratch).ok();
}

#[test]
fn a_development_checkout_has_nothing_to_prefetch_and_says_so() {
    // В дереве разработки инструменты собираются на месте, и манифест это
    // объявляет. Молчаливый успех здесь соврал бы: образ уехал бы пустым.
    let plugin_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/unica");

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let cache = std::env::temp_dir().join(format!(
        "unica-bootstrap-prefetch-development-{}-{nonce}",
        std::process::id()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_unica-bootstrap"))
        .arg("prefetch")
        .arg("--plugin-root")
        .arg(&plugin_root)
        .env("UNICA_RUNTIME_CACHE_DIR", &cache)
        .output()
        .expect("bootstrap process must start");

    let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
    assert!(!output.status.success(), "unexpected success: {stderr}");
    assert!(
        stderr.contains("development"),
        "отказ обязан назвать причину: {stderr}"
    );
    fs::remove_dir_all(cache).ok();
}

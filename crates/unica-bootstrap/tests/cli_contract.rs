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

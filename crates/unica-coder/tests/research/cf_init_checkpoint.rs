//! Исследование: снимок исходного дерева платформы 8.3.27 для ручной сверки `ibcmd`.
//!
//! Не тест: проводится один раз, дерево остаётся уликой вне репозитория.
//! Цель собирается только с фичей `research`, в плане конвейера её нет.
//! Запуск — `scripts/research/cf-init-checkpoint.sh`.

#[allow(dead_code)]
#[path = "../cf_init_platform_contract.rs"]
mod contract;

use std::path::PathBuf;

#[test]
fn public_cf_init_writes_platform_checkpoint_source() {
    let workspace = PathBuf::from(
        std::env::var_os("UNICA_CF_INIT_PLATFORM_EVIDENCE_DIR")
            .expect("UNICA_CF_INIT_PLATFORM_EVIDENCE_DIR"),
    );
    assert!(workspace.is_absolute(), "evidence path must be absolute");
    std::fs::create_dir_all(&workspace).unwrap();
    assert!(
        std::fs::read_dir(&workspace).unwrap().next().is_none(),
        "evidence path must be empty: {}",
        workspace.display()
    );

    let source_root = contract::call_cf_init(&workspace, "source", None);
    println!("CF_INIT_PLATFORM_SOURCE={}", source_root.display());
}

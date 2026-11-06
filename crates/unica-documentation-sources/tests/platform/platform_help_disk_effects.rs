//! rule: INV.PKG.PLATFORM-DOCUMENTATION-CORPUS

use super::tests::{hbk_bytes, request};
use super::*;

const CHILD_ROOT: &str = "UNICA_TEST_HBK_OBSERVED_ROOT";

#[cfg_attr(
    not(windows),
    ignore = "host capability windows-directory-notifications: observes writes using FileSystemWatcher"
)]
#[test]
fn medium_cold_and_warm_platform_help_do_not_write_to_the_observed_directories() {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        search_in_fresh_process(Path::new(&root));
    } else {
        let root = tempfile::tempdir().unwrap();
        for name in ["8.3.27.2074", "workspace", "state", "cache", "temp"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        std::fs::write(
            root.path().join("8.3.27.2074/shcntx_ru.hbk"),
            hbk_bytes(&[(
                "alpha.html",
                "<html><body><h1>AlphaFixture</h1></body></html>",
            )]),
        )
        .unwrap();
        let observer_temp = root.path().join("observer-temp");
        std::fs::create_dir(&observer_temp).unwrap();
        let test = std::thread::current();
        let result = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                include_str!("platform_help_disk_observer.ps1"),
            ])
            .env("TMP", &observer_temp)
            .env("TEMP", &observer_temp)
            .env("UNICA_TEST_HBK_ROOT", root.path())
            .env("UNICA_TEST_HBK_BINARY", std::env::current_exe().unwrap())
            .env(
                "UNICA_TEST_HBK_CASE",
                test.name().expect("full libtest case name"),
            )
            .output()
            .expect("start the Windows filesystem observer");
        assert!(
            result.status.success(),
            "filesystem observer failed: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

fn search_in_fresh_process(root: &Path) {
    let context = DocumentationContext {
        platform_version: Some("8.3.27.2074".to_string()),
        installation_root: Some(root.join("8.3.27.2074")),
    };
    assert!(
        index_slot().lock().unwrap().is_none(),
        "expected a cold provider process"
    );
    let mut first_index = None;
    for _ in 0..2 {
        let sections = PlatformSyntaxHelpProvider::new().search(&request("AlphaFixture"), &context);
        let section = sections
            .iter()
            .find(|section| section.corpus == "syntax-context")
            .unwrap();
        assert!(
            matches!(section.status, DocumentationSectionStatus::Ok),
            "{section:?}"
        );
        assert_eq!(section.hits.len(), 1);
        assert_eq!(section.hits[0].title, "AlphaFixture");
        let index = Arc::clone(&index_slot().lock().unwrap().as_ref().unwrap().index);
        if let Some(first) = &first_index {
            assert!(
                Arc::ptr_eq(first, &index),
                "the repeated search rebuilt its index"
            );
        } else {
            first_index = Some(index);
        }
    }
}

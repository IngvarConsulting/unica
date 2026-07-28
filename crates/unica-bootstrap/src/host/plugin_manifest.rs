use std::path::Path;

use crate::error::{BootstrapError, Result};

use super::descriptor::KNOWN;

/// Plugin name every host manifest has to agree on.
const PLUGIN_NAME: &str = "unica";
/// Pointer a host that does not scan the package expects to find.
const SKILLS_POINTER: &str = "./skills/";

/// A package may carry the manifest of one host, of another, or of several.
/// Every manifest that is present must agree on the plugin identity, and each
/// host keeps its own skill-discovery contract: a host that does not scan the
/// package needs the explicit `skills` pointer, while a host that always scans
/// `skills/` would load the directory twice if the manifest named it again.
pub fn verify_installed_plugin_metadata(plugin_root: &Path, version: &str) -> Result<()> {
    let mut manifests = 0;
    for host in KNOWN {
        let metadata_path = plugin_root.join(host.manifest_dir).join("plugin.json");
        if !metadata_path.is_file() {
            continue;
        }
        manifests += 1;
        let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata_path)?)?;
        // Absence is checked on the raw entry rather than the string projection,
        // so a `skills` key of any type still counts as declared.
        let declared_skills = metadata.get("skills");
        let skills = declared_skills.and_then(serde_json::Value::as_str);
        if metadata.get("name").and_then(serde_json::Value::as_str) != Some(PLUGIN_NAME)
            || metadata.get("version").and_then(serde_json::Value::as_str) != Some(version)
            || (host.expects_skills_pointer && skills != Some(SKILLS_POINTER))
            || (!host.expects_skills_pointer && declared_skills.is_some())
        {
            return Err(BootstrapError::new(format!(
                "installed Unica plugin metadata does not expose version {version} skills: {}",
                metadata_path.display()
            )));
        }
    }
    if manifests == 0 {
        return Err(BootstrapError::new(format!(
            "installed Unica plugin has no host manifest: {}",
            plugin_root.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    /// Fixtures assert the contract, not the release the crate happens to carry.
    const VERSION: &str = "1.2.3";

    struct ManifestFixture {
        root: PathBuf,
    }

    impl ManifestFixture {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("unica-manifest-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, dir: &str, body: serde_json::Value) {
            let manifest_dir = self.root.join(dir);
            std::fs::create_dir_all(&manifest_dir).unwrap();
            std::fs::write(
                manifest_dir.join("plugin.json"),
                serde_json::to_vec(&body).unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for ManifestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn codex_manifest() -> serde_json::Value {
        serde_json::json!({"name": "unica", "version": VERSION, "skills": "./skills/"})
    }

    fn claude_manifest() -> serde_json::Value {
        serde_json::json!({"name": "unica", "version": VERSION})
    }

    #[test]
    fn either_host_manifest_alone_satisfies_the_contract() {
        let codex = ManifestFixture::new("codex-only");
        codex.write(".codex-plugin", codex_manifest());
        verify_installed_plugin_metadata(&codex.root, VERSION).unwrap();

        let claude = ManifestFixture::new("claude-only");
        claude.write(".claude-plugin", claude_manifest());
        verify_installed_plugin_metadata(&claude.root, VERSION).unwrap();
    }

    #[test]
    fn a_package_without_any_host_manifest_is_rejected() {
        let fixture = ManifestFixture::new("no-host");
        verify_installed_plugin_metadata(&fixture.root, VERSION).unwrap_err();
    }

    #[test]
    fn a_manifest_from_another_release_is_rejected() {
        let fixture = ManifestFixture::new("stale-version");
        fixture.write(".codex-plugin", codex_manifest());
        fixture.write(
            ".claude-plugin",
            serde_json::json!({"name": "unica", "version": "0.0.0"}),
        );
        verify_installed_plugin_metadata(&fixture.root, VERSION).unwrap_err();
    }

    #[test]
    fn a_codex_manifest_without_the_skills_pointer_is_rejected() {
        let fixture = ManifestFixture::new("codex-no-pointer");
        fixture.write(
            ".codex-plugin",
            serde_json::json!({"name": "unica", "version": VERSION}),
        );
        verify_installed_plugin_metadata(&fixture.root, VERSION).unwrap_err();
    }

    #[test]
    fn a_claude_manifest_declaring_skills_is_rejected_whatever_its_type() {
        // Claude Code always scans skills/, so any declaration would load the
        // directory twice regardless of the value's JSON type.
        for value in [
            serde_json::json!("./skills/"),
            serde_json::json!(["./skills/"]),
            serde_json::json!({"path": "./skills/"}),
            serde_json::Value::Null,
        ] {
            let fixture = ManifestFixture::new("claude-skills");
            fixture.write(
                ".claude-plugin",
                serde_json::json!({"name": "unica", "version": VERSION, "skills": value}),
            );
            verify_installed_plugin_metadata(&fixture.root, VERSION).unwrap_err();
        }
    }
}

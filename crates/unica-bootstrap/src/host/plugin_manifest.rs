use std::path::Path;

use crate::error::{BootstrapError, Failure, Result};

use super::descriptor::KNOWN;

/// Plugin name every host manifest has to agree on.
const PLUGIN_NAME: &str = "unica";
/// Pointer a host that does not scan the package expects to find.
const SKILLS_POINTER: &str = "./skills/";
/// Server pointer the same host expects, for the same reason.
const SERVERS_POINTER: &str = "./.mcp.json";

/// One plugin directory serves every known host, so a package carries every
/// host manifest (ADR-0012, INV-PKG-VERSION-LOCKSTEP). A package that is
/// missing one is simply unloadable for that host, and accepting it here would
/// let the release gate pass bytes no consumer of that host can install.
///
/// Every manifest agrees on the plugin identity, and each host keeps its own
/// discovery contract: a host that does not scan the package needs the explicit
/// `skills` pointer, while a host that always scans `skills/` and the root
/// `.mcp.json` would load each of them twice if the manifest named them again.
pub fn verify_installed_plugin_metadata(plugin_root: &Path, version: &str) -> Result<()> {
    for host in KNOWN {
        let metadata_path = plugin_root.join(host.manifest_dir).join("plugin.json");
        if !metadata_path.is_file() {
            return Err(BootstrapError::new(format!(
                "installed Unica plugin is missing a host manifest: {}",
                metadata_path.display()
            )));
        }
        let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&metadata_path)?)
            .map_err(|error| {
            BootstrapError::of(
                Failure::Configuration,
                format!(
                    "failed to parse installed host manifest {}: {error}",
                    metadata_path.display()
                ),
            )
        })?;
        // Absence is checked on the raw entry rather than the string projection,
        // so a key of any type still counts as declared.
        let declared_skills = metadata.get("skills");
        let skills = declared_skills.and_then(serde_json::Value::as_str);
        let declared_servers = metadata.get("mcpServers");
        let servers = declared_servers.and_then(serde_json::Value::as_str);
        if metadata.get("name").and_then(serde_json::Value::as_str) != Some(PLUGIN_NAME)
            || metadata.get("version").and_then(serde_json::Value::as_str) != Some(version)
            || (host.expects_skills_pointer && skills != Some(SKILLS_POINTER))
            || (!host.expects_skills_pointer && declared_skills.is_some())
            || (host.expects_manifest_servers && servers != Some(SERVERS_POINTER))
            || (!host.expects_manifest_servers && declared_servers.is_some())
        {
            return Err(BootstrapError::new(format!(
                "installed Unica plugin metadata does not meet the version {version} \
                 host contract (name, version, skills, mcpServers): {}",
                metadata_path.display()
            )));
        }
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

        fn complete(name: &str) -> Self {
            let fixture = Self::new(name);
            fixture.write(".codex-plugin", codex_manifest());
            fixture.write(".claude-plugin", claude_manifest());
            fixture.write(".zcode-plugin", zcode_manifest());
            verify_installed_plugin_metadata(&fixture.root, VERSION).unwrap();
            fixture
        }

        fn assert_rejected(&self, dir: &str, reason: &str) {
            let error = verify_installed_plugin_metadata(&self.root, VERSION).unwrap_err();
            let message = error.to_string();
            let path = self.root.join(dir).join("plugin.json");
            assert!(message.contains(path.to_str().unwrap()), "{error}");
            assert!(message.contains(reason), "{error}");
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

        fn write_raw(&self, dir: &str, body: &[u8]) {
            let manifest_dir = self.root.join(dir);
            std::fs::create_dir_all(&manifest_dir).unwrap();
            std::fs::write(manifest_dir.join("plugin.json"), body).unwrap();
        }
    }

    impl Drop for ManifestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn codex_manifest() -> serde_json::Value {
        serde_json::json!({
            "name": "unica",
            "version": VERSION,
            "skills": "./skills/",
            "mcpServers": "./.mcp.json",
        })
    }

    fn claude_manifest() -> serde_json::Value {
        serde_json::json!({"name": "unica", "version": VERSION})
    }

    fn zcode_manifest() -> serde_json::Value {
        serde_json::json!({
            "name": "unica",
            "version": VERSION,
            "skills": "./skills/",
            "mcpServers": "./.mcp.json",
        })
    }

    #[test]
    fn malformed_host_metadata_is_an_installation_configuration_failure() {
        for dir in [".codex-plugin", ".claude-plugin", ".zcode-plugin"] {
            let fixture = ManifestFixture::complete("malformed-json");
            fixture.write_raw(dir, b"{");

            let error = verify_installed_plugin_metadata(&fixture.root, VERSION)
                .expect_err("malformed plugin metadata");

            assert_eq!(error.failure(), crate::error::Failure::Configuration);
            fixture.assert_rejected(dir, "failed to parse installed host manifest");
        }
    }

    #[test]
    fn a_package_carrying_all_three_host_manifests_is_accepted() {
        let fixture = ManifestFixture::complete("all-hosts");
        verify_installed_plugin_metadata(&fixture.root, VERSION).unwrap();
    }

    #[test]
    fn a_package_missing_any_one_host_manifest_is_rejected() {
        for dir in [".codex-plugin", ".claude-plugin", ".zcode-plugin"] {
            let fixture = ManifestFixture::complete("missing-host");
            std::fs::remove_file(fixture.root.join(dir).join("plugin.json")).unwrap();
            fixture.assert_rejected(dir, "missing a host manifest");
        }
    }

    #[test]
    fn a_package_carrying_only_one_host_manifest_is_rejected() {
        for (dir, body, missing) in [
            (".codex-plugin", codex_manifest(), ".claude-plugin"),
            (".claude-plugin", claude_manifest(), ".codex-plugin"),
            (".zcode-plugin", zcode_manifest(), ".codex-plugin"),
        ] {
            let fixture = ManifestFixture::new("one-host");
            fixture.write(dir, body);
            fixture.assert_rejected(missing, "missing a host manifest");
        }
    }

    #[test]
    fn a_package_without_any_host_manifest_is_rejected() {
        let fixture = ManifestFixture::new("no-host");
        fixture.assert_rejected(".codex-plugin", "missing a host manifest");
    }

    #[test]
    fn a_manifest_from_another_release_is_rejected() {
        for (dir, mut body) in [
            (".codex-plugin", codex_manifest()),
            (".claude-plugin", claude_manifest()),
            (".zcode-plugin", zcode_manifest()),
        ] {
            let fixture = ManifestFixture::complete("stale-version");
            body["version"] = serde_json::json!("0.0.0");
            fixture.write(dir, body);
            fixture.assert_rejected(dir, "does not meet the version");
        }
    }

    #[test]
    fn a_manifest_with_another_plugin_identity_is_rejected() {
        for (dir, mut body) in [
            (".codex-plugin", codex_manifest()),
            (".claude-plugin", claude_manifest()),
            (".zcode-plugin", zcode_manifest()),
        ] {
            let fixture = ManifestFixture::complete("wrong-name");
            body["name"] = serde_json::json!("another-plugin");
            fixture.write(dir, body);
            fixture.assert_rejected(dir, "host contract");
        }
    }

    #[test]
    fn a_codex_manifest_without_the_server_pointer_is_rejected() {
        let fixture = ManifestFixture::complete("codex-no-servers");
        let mut body = codex_manifest();
        body.as_object_mut().unwrap().remove("mcpServers");
        fixture.write(".codex-plugin", body);
        fixture.assert_rejected(".codex-plugin", "host contract");
    }

    #[test]
    fn a_codex_manifest_without_the_skills_pointer_is_rejected() {
        let fixture = ManifestFixture::complete("codex-no-pointer");
        let mut body = codex_manifest();
        body.as_object_mut().unwrap().remove("skills");
        fixture.write(".codex-plugin", body);
        fixture.assert_rejected(".codex-plugin", "host contract");
    }

    #[test]
    fn a_zcode_manifest_requires_exact_skills_and_server_pointers() {
        for key in ["skills", "mcpServers"] {
            for value in [
                None,
                Some(serde_json::json!("./wrong/")),
                Some(serde_json::json!(["./skills/"])),
                Some(serde_json::json!({"path": "./.mcp.json"})),
                Some(serde_json::Value::Null),
            ] {
                let fixture = ManifestFixture::complete("zcode-discovery");
                let mut body = zcode_manifest();
                if let Some(value) = value {
                    body[key] = value;
                } else {
                    body.as_object_mut().unwrap().remove(key);
                }
                fixture.write(".zcode-plugin", body);
                fixture.assert_rejected(".zcode-plugin", "host contract");
            }
        }
    }

    #[test]
    fn a_claude_manifest_declaring_discovery_keys_is_rejected_whatever_their_type() {
        // Claude Code always scans skills/ and always reads the root .mcp.json,
        // so any declaration would load them twice regardless of JSON type.
        for key in ["skills", "mcpServers"] {
            for value in [
                serde_json::json!("./skills/"),
                serde_json::json!(["./skills/"]),
                serde_json::json!({"path": "./skills/"}),
                serde_json::Value::Null,
            ] {
                let fixture = ManifestFixture::complete("claude-discovery");
                fixture.write(
                    ".claude-plugin",
                    serde_json::json!({"name": "unica", "version": VERSION, key: value}),
                );
                fixture.assert_rejected(".claude-plugin", "host contract");
            }
        }
    }
}

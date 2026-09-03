use std::collections::BTreeSet;
use std::path::Path;

use crate::error::{BootstrapError, Result};

use super::plugin_manifest::verify_installed_plugin_metadata;

/// The one prompt-visible skill this check names.
///
/// The expected skill set is whatever the package ships, so adding or removing
/// a skill never touches this module. The anchor is what tells a Unica skill
/// tree from any directory that happens to hold `SKILL.md` files: `code-search`
/// fronts `unica.search`, the entry point of the public surface, and dropping
/// it is a product decision, not a routine surface change, so it is the one
/// name a release is allowed to fail on by design.
const ANCHOR_SKILL: &str = "code-search";

/// The installed package exposes the prompt-visible skills it was built from.
///
/// The expected set is derived from the package itself rather than written
/// here: every directory under `skills/` is a skill the hosts will show, so
/// each has to be complete, and the set has to carry the [`ANCHOR_SKILL`].
pub fn verify_installed_skill_package(plugin_root: &Path, version: &str) -> Result<()> {
    verify_installed_plugin_metadata(plugin_root, version)?;

    let visible = prompt_visible_skills(&plugin_root.join("skills"))?;
    // The anchor would reject an empty package on its own. This branch is here
    // for the diagnosis: a packager that shipped nothing is a different fault
    // from one that shipped a tree missing this skill, and the release log is
    // what someone reads to tell them apart.
    if visible.is_empty() {
        return Err(BootstrapError::new(
            "installed Unica plugin exposes no prompt-visible skills",
        ));
    }
    if !visible.contains(ANCHOR_SKILL) {
        return Err(BootstrapError::new(format!(
            "installed prompt-visible skill is missing: {ANCHOR_SKILL}"
        )));
    }
    Ok(())
}

/// Every skill directory of the package, each proven complete by its `SKILL.md`.
fn prompt_visible_skills(skills_root: &Path) -> Result<BTreeSet<String>> {
    let mut visible = BTreeSet::new();
    for entry in std::fs::read_dir(skills_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let skill_file = entry.path().join("SKILL.md");
        if !skill_file.is_file() {
            return Err(BootstrapError::new(format!(
                "installed prompt-visible skill is incomplete: {}",
                entry.path().display()
            )));
        }
        visible.insert(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    /// Fixtures assert the contract, not the release the crate happens to carry.
    const VERSION: &str = "1.2.3";

    /// A packaged plugin root: both host manifests of one release and the named
    /// prompt-visible skills.
    struct PackageFixture {
        root: PathBuf,
    }

    impl PackageFixture {
        fn new(name: &str, skills: &[&str]) -> Self {
            let root = std::env::temp_dir()
                .join(format!("unica-skill-package-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let manifests = [
                (
                    ".codex-plugin",
                    serde_json::json!({
                        "name": "unica",
                        "version": VERSION,
                        "skills": "./skills/",
                        "mcpServers": "./.mcp.json",
                    }),
                ),
                (
                    ".claude-plugin",
                    serde_json::json!({"name": "unica", "version": VERSION}),
                ),
            ];
            for (dir, body) in manifests {
                let manifest_dir = root.join(dir);
                std::fs::create_dir_all(&manifest_dir).unwrap();
                std::fs::write(
                    manifest_dir.join("plugin.json"),
                    serde_json::to_vec(&body).unwrap(),
                )
                .unwrap();
            }
            std::fs::create_dir_all(root.join("skills")).unwrap();
            let fixture = Self { root };
            for skill in skills {
                fixture.add_skill(skill);
            }
            fixture
        }

        fn add_skill(&self, name: &str) {
            let skill_dir = self.skill_dir(name);
            std::fs::write(skill_dir.join("SKILL.md"), format!("# {name}\n")).unwrap();
        }

        /// A directory under `skills/` that carries no `SKILL.md`.
        fn skill_dir(&self, name: &str) -> PathBuf {
            let skill_dir = self.root.join("skills").join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            skill_dir
        }
    }

    impl Drop for PackageFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn source_plugin_passes_the_installed_skill_package_check() {
        let plugin_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/unica");
        verify_installed_skill_package(&plugin_root, env!("CARGO_PKG_VERSION")).unwrap();
    }

    #[test]
    fn a_package_is_accepted_whatever_skills_it_ships_beside_the_anchor() {
        // Removing a skill changes the package, not this check: the expected
        // set is whatever the package ships, and only the anchor is named.
        let fixture = PackageFixture::new("any-skills", &[ANCHOR_SKILL, "api-design"]);
        verify_installed_skill_package(&fixture.root, VERSION).unwrap();
    }

    #[test]
    fn a_package_without_the_anchor_skill_is_rejected() {
        let fixture = PackageFixture::new("no-anchor", &["api-design", "platform-help"]);
        let error = verify_installed_skill_package(&fixture.root, VERSION).unwrap_err();
        assert!(error.to_string().contains(ANCHOR_SKILL), "{error}");
    }

    #[test]
    fn a_package_without_any_prompt_visible_skill_is_rejected() {
        let fixture = PackageFixture::new("no-skills", &[]);
        let error = verify_installed_skill_package(&fixture.root, VERSION).unwrap_err();
        assert!(
            error.to_string().contains("no prompt-visible skills"),
            "{error}"
        );
    }

    #[test]
    fn a_skill_directory_without_skill_md_is_rejected() {
        let fixture = PackageFixture::new("incomplete-skill", &[ANCHOR_SKILL]);
        fixture.skill_dir("broken");
        let error = verify_installed_skill_package(&fixture.root, VERSION).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("incomplete") && message.contains("broken"),
            "{error}"
        );
    }
}

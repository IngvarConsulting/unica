use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::codex::{
    discover, CodexDiscovery, CommandRunner, CommandSpec, MarketplaceRecord, PluginRecord,
};
use crate::error::{BootstrapError, Result};

pub const CANONICAL_MARKETPLACE: &str = "unica";
pub const CANONICAL_SOURCE: &str = "IngvarConsulting/unica-marketplace";
pub const CANONICAL_REF: &str = "main";
const CANONICAL_GIT_FRAGMENT: &str = "github.com/ingvarconsulting/unica-marketplace";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    pub remove_plugin_ids: Vec<String>,
    pub remove_marketplaces: Vec<String>,
    pub add_canonical_marketplace: bool,
    pub install_canonical_plugin: bool,
    #[serde(skip)]
    legacy_marketplaces: BTreeMap<String, MarketplaceRecord>,
    #[serde(skip)]
    legacy_plugins: BTreeMap<String, PluginRecord>,
}

impl MigrationPlan {
    pub fn is_noop(&self) -> bool {
        self.remove_plugin_ids.is_empty()
            && self.remove_marketplaces.is_empty()
            && !self.add_canonical_marketplace
            && !self.install_canonical_plugin
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub changed: bool,
    pub backup_dir: Option<PathBuf>,
    pub removed_plugins: Vec<String>,
    pub removed_marketplaces: Vec<String>,
    pub installed_plugin: String,
}

pub fn classify_discovery(discovery: CodexDiscovery, _codex_home: &Path) -> Result<MigrationPlan> {
    let mut canonical_marketplace = false;
    let mut legacy_marketplaces = BTreeMap::new();
    for marketplace in discovery.marketplaces.marketplaces {
        match marketplace.name.as_str() {
            "unica" if is_canonical(&marketplace) => canonical_marketplace = true,
            "unica" if marketplace.marketplace_source.source_type == "local" => {
                legacy_marketplaces.insert(marketplace.name.clone(), marketplace);
            }
            "unica" => {
                return Err(BootstrapError::new(format!(
                    "reserved marketplace name unica is owned by unknown source {}",
                    marketplace.marketplace_source.source
                )));
            }
            "unica-local" if marketplace.marketplace_source.source_type == "local" => {
                legacy_marketplaces.insert(marketplace.name.clone(), marketplace);
            }
            "unica-local" => {
                return Err(BootstrapError::new(format!(
                    "legacy marketplace name unica-local is owned by unknown source {}",
                    marketplace.marketplace_source.source
                )));
            }
            _ => {}
        }
    }

    let mut canonical_installed = false;
    let mut legacy_plugins = BTreeMap::new();
    for plugin in discovery.plugins.installed {
        if plugin.name != "unica" {
            continue;
        }
        if plugin.marketplace_name == CANONICAL_MARKETPLACE && canonical_marketplace {
            canonical_installed = true;
        } else if matches!(plugin.marketplace_name.as_str(), "unica" | "unica-local") {
            legacy_plugins.insert(plugin.plugin_id.clone(), plugin);
        }
    }

    let remove_plugin_ids = legacy_plugins.keys().cloned().collect();
    let remove_marketplaces = legacy_marketplaces.keys().cloned().collect();
    Ok(MigrationPlan {
        remove_plugin_ids,
        remove_marketplaces,
        add_canonical_marketplace: !canonical_marketplace,
        install_canonical_plugin: !canonical_installed,
        legacy_marketplaces,
        legacy_plugins,
    })
}

fn is_canonical(marketplace: &MarketplaceRecord) -> bool {
    let source = marketplace
        .marketplace_source
        .source
        .trim_end_matches(".git")
        .to_ascii_lowercase();
    marketplace.marketplace_source.source_type == "git" && source.contains(CANONICAL_GIT_FRAGMENT)
}

pub struct MigrationEngine<R> {
    codex_home: PathBuf,
    runner: R,
}

impl<R: CommandRunner> MigrationEngine<R> {
    pub fn new(codex_home: PathBuf, runner: R) -> Self {
        Self { codex_home, runner }
    }

    pub fn preflight(&self) -> Result<MigrationPlan> {
        self.runner.run(&CommandSpec::git(&[
            "-c",
            "alias.unica-probe=!f() { exit 0; }; f",
            "unica-probe",
        ]))?;
        classify_discovery(discover(&self.runner, &self.codex_home)?, &self.codex_home)
    }

    pub fn apply(&self, plan: MigrationPlan) -> Result<MigrationReport> {
        if plan.is_noop() {
            return Ok(MigrationReport {
                changed: false,
                backup_dir: None,
                removed_plugins: vec![],
                removed_marketplaces: vec![],
                installed_plugin: "unica@unica".to_string(),
            });
        }

        let backup = Backup::capture(&self.codex_home, &plan)?;
        let mut journal = Vec::new();
        let result = self.apply_steps(&plan, &mut journal);
        if let Err(error) = result {
            let rollback = self.rollback(&plan, &journal, &backup);
            return Err(match rollback {
                Ok(()) => BootstrapError::new(format!(
                    "migration failed and was rolled back; backup: {}: {error}",
                    backup.root.display()
                )),
                Err(rollback_error) => BootstrapError::new(format!(
                    "migration failed: {error}; rollback also failed: {rollback_error}; backup: {}",
                    backup.root.display()
                )),
            });
        }

        Ok(MigrationReport {
            changed: true,
            backup_dir: Some(backup.root),
            removed_plugins: plan.remove_plugin_ids,
            removed_marketplaces: plan.remove_marketplaces,
            installed_plugin: "unica@unica".to_string(),
        })
    }

    fn apply_steps(&self, plan: &MigrationPlan, journal: &mut Vec<JournalEntry>) -> Result<()> {
        for plugin_id in &plan.remove_plugin_ids {
            self.run_codex(&["plugin", "remove", plugin_id, "--json"])?;
            journal.push(JournalEntry::RemovedPlugin(plugin_id.clone()));
        }
        for marketplace in &plan.remove_marketplaces {
            self.run_codex(&["plugin", "marketplace", "remove", marketplace, "--json"])?;
            journal.push(JournalEntry::RemovedMarketplace(marketplace.clone()));
        }
        if plan.add_canonical_marketplace {
            self.run_codex(&[
                "plugin",
                "marketplace",
                "add",
                CANONICAL_SOURCE,
                "--ref",
                CANONICAL_REF,
                "--json",
            ])?;
            journal.push(JournalEntry::AddedCanonicalMarketplace);
        }
        if plan.install_canonical_plugin {
            self.run_codex(&["plugin", "add", "unica@unica", "--json"])?;
            journal.push(JournalEntry::AddedCanonicalPlugin);
        }

        let current = discover(&self.runner, &self.codex_home)?;
        let proof = classify_discovery(current, &self.codex_home)?;
        if !proof.is_noop() {
            return Err(BootstrapError::new(
                "Codex discovery did not confirm canonical unica@unica after migration",
            ));
        }
        Ok(())
    }

    fn rollback(
        &self,
        plan: &MigrationPlan,
        journal: &[JournalEntry],
        backup: &Backup,
    ) -> Result<()> {
        let mut errors = Vec::new();
        for entry in journal.iter().rev() {
            let result = match entry {
                JournalEntry::AddedCanonicalPlugin => {
                    self.run_codex(&["plugin", "remove", "unica@unica", "--json"])
                }
                JournalEntry::AddedCanonicalMarketplace => self.run_codex(&[
                    "plugin",
                    "marketplace",
                    "remove",
                    CANONICAL_MARKETPLACE,
                    "--json",
                ]),
                JournalEntry::RemovedMarketplace(name) => {
                    let source = &plan.legacy_marketplaces[name].marketplace_source.source;
                    self.run_codex(&["plugin", "marketplace", "add", source, "--json"])
                }
                JournalEntry::RemovedPlugin(id) => {
                    let selector = &plan.legacy_plugins[id].plugin_id;
                    self.run_codex(&["plugin", "add", selector, "--json"])
                }
            };
            if let Err(error) = result {
                errors.push(error.to_string());
            }
        }
        if let Err(error) = backup.restore_config(&self.codex_home) {
            errors.push(error.to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(BootstrapError::new(errors.join("; ")))
        }
    }

    fn run_codex(&self, args: &[&str]) -> Result<String> {
        self.runner.run(&CommandSpec::codex(&self.codex_home, args))
    }
}

#[derive(Clone, Debug)]
enum JournalEntry {
    RemovedPlugin(String),
    RemovedMarketplace(String),
    AddedCanonicalMarketplace,
    AddedCanonicalPlugin,
}

struct Backup {
    root: PathBuf,
    config: Option<Vec<u8>>,
}

impl Backup {
    fn capture(codex_home: &Path, plan: &MigrationPlan) -> Result<Self> {
        let root = codex_home
            .join("unica")
            .join("migration-backups")
            .join(Uuid::new_v4().to_string());
        fs::create_dir_all(&root)?;
        let config_path = codex_home.join("config.toml");
        let config = if config_path.is_file() {
            let bytes = fs::read(&config_path)?;
            fs::write(root.join("config.toml"), &bytes)?;
            Some(bytes)
        } else {
            None
        };
        let snapshot = serde_json::json!({
            "schemaVersion": 1,
            "removePluginIds": plan.remove_plugin_ids,
            "removeMarketplaces": plan.remove_marketplaces,
            "configExisted": config.is_some(),
        });
        fs::write(
            root.join("snapshot.json"),
            serde_json::to_vec_pretty(&snapshot)?,
        )?;
        Ok(Self { root, config })
    }

    fn restore_config(&self, codex_home: &Path) -> Result<()> {
        let config_path = codex_home.join("config.toml");
        match &self.config {
            Some(bytes) => {
                let temporary = codex_home.join(format!(".config.toml.restore-{}", Uuid::new_v4()));
                fs::write(&temporary, bytes)?;
                fs::rename(temporary, config_path)?;
            }
            None if config_path.exists() => fs::remove_file(config_path)?,
            None => {}
        }
        Ok(())
    }
}

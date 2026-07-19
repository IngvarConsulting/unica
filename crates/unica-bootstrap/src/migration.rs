use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::codex::{discover, CodexDiscovery, CommandRunner, CommandSpec, MarketplaceRecord};
use crate::error::{BootstrapError, Result};

pub const CANONICAL_MARKETPLACE: &str = "unica";
pub const CANONICAL_SOURCE: &str = "IngvarConsulting/unica-marketplace";
pub const CANONICAL_REF: &str = "main";
const CANONICAL_GIT_FRAGMENT: &str = "github.com/ingvarconsulting/unica-marketplace";
const LEGACY_PLUGIN_SELECTOR: &str = "unica@unica-local";
const LEGACY_PLUGIN_CONFIG_TABLE: &str = "[plugins.\"unica@unica-local\"]";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    pub remove_plugin_ids: Vec<String>,
    pub remove_marketplaces: Vec<String>,
    pub add_canonical_marketplace: bool,
    pub install_canonical_plugin: bool,
    pub remove_legacy_paths: Vec<PathBuf>,
    #[serde(skip)]
    legacy_marketplaces: BTreeMap<String, MarketplaceRecord>,
}

impl MigrationPlan {
    pub fn is_noop(&self) -> bool {
        self.remove_plugin_ids.is_empty()
            && self.remove_marketplaces.is_empty()
            && !self.add_canonical_marketplace
            && !self.install_canonical_plugin
            && self.remove_legacy_paths.is_empty()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub changed: bool,
    pub backup_dir: Option<PathBuf>,
    pub removed_plugins: Vec<String>,
    pub removed_marketplaces: Vec<String>,
    pub removed_legacy_paths: Vec<PathBuf>,
    pub installed_plugin: String,
}

pub fn classify_discovery(discovery: CodexDiscovery, codex_home: &Path) -> Result<MigrationPlan> {
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
    let mut remove_plugin_ids = BTreeSet::new();
    for plugin in discovery.plugins.installed {
        if plugin.name != "unica" {
            continue;
        }
        if plugin.marketplace_name == CANONICAL_MARKETPLACE && canonical_marketplace {
            canonical_installed = true;
        } else if matches!(plugin.marketplace_name.as_str(), "unica" | "unica-local") {
            remove_plugin_ids.insert(plugin.plugin_id);
        }
    }
    if config_has_legacy_plugin(codex_home)? {
        remove_plugin_ids.insert(LEGACY_PLUGIN_SELECTOR.to_string());
    }

    let remove_marketplaces = legacy_marketplaces.keys().cloned().collect();
    Ok(MigrationPlan {
        remove_plugin_ids: remove_plugin_ids.into_iter().collect(),
        remove_marketplaces,
        add_canonical_marketplace: !canonical_marketplace,
        install_canonical_plugin: !canonical_installed,
        remove_legacy_paths: existing_legacy_paths(codex_home)?,
        legacy_marketplaces,
    })
}

fn config_has_legacy_plugin(codex_home: &Path) -> Result<bool> {
    let config_path = codex_home.join("config.toml");
    let config = match fs::read_to_string(&config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    Ok(config
        .lines()
        .any(|line| line.trim() == LEGACY_PLUGIN_CONFIG_TABLE))
}

fn existing_legacy_paths(codex_home: &Path) -> Result<Vec<PathBuf>> {
    let candidates = [
        codex_home.join("marketplaces").join("unica-local"),
        codex_home.join("plugins").join("cache").join("unica-local"),
    ];
    candidates
        .into_iter()
        .filter_map(|path| match fs::symlink_metadata(&path) {
            Ok(_) => Some(Ok(path)),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => Some(Err(error.into())),
        })
        .collect()
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

    pub fn apply<F>(&self, plan: MigrationPlan, verify: F) -> Result<MigrationReport>
    where
        F: FnOnce() -> Result<()>,
    {
        if plan.is_noop() {
            verify()?;
            return Ok(MigrationReport {
                changed: false,
                backup_dir: None,
                removed_plugins: vec![],
                removed_marketplaces: vec![],
                removed_legacy_paths: vec![],
                installed_plugin: "unica@unica".to_string(),
            });
        }

        let backup = Backup::capture(&self.codex_home, &plan)?;
        let mut journal = Vec::new();
        let result = self.apply_steps(&plan, &mut journal, verify);
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
            removed_legacy_paths: plan.remove_legacy_paths,
            installed_plugin: "unica@unica".to_string(),
        })
    }

    fn apply_steps<F>(
        &self,
        plan: &MigrationPlan,
        journal: &mut Vec<JournalEntry>,
        verify: F,
    ) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
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
        if !proof.remove_plugin_ids.is_empty()
            || !proof.remove_marketplaces.is_empty()
            || proof.add_canonical_marketplace
            || proof.install_canonical_plugin
        {
            return Err(BootstrapError::new(
                "Codex discovery did not confirm canonical unica@unica after migration",
            ));
        }

        verify()?;

        for path in &plan.remove_legacy_paths {
            journal.push(JournalEntry::RemovedLegacyPath(path.clone()));
            remove_exact_path(path)?;
        }

        let final_state = discover(&self.runner, &self.codex_home)?;
        if !classify_discovery(final_state, &self.codex_home)?.is_noop() {
            return Err(BootstrapError::new(
                "legacy Unica state remains after migration cleanup",
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
                JournalEntry::AddedCanonicalPlugin => self
                    .run_codex(&["plugin", "remove", "unica@unica", "--json"])
                    .map(|_| ()),
                JournalEntry::AddedCanonicalMarketplace => self
                    .run_codex(&[
                        "plugin",
                        "marketplace",
                        "remove",
                        CANONICAL_MARKETPLACE,
                        "--json",
                    ])
                    .map(|_| ()),
                JournalEntry::RemovedMarketplace(name) => {
                    let source = &plan.legacy_marketplaces[name].marketplace_source.source;
                    self.run_codex(&["plugin", "marketplace", "add", source, "--json"])
                        .map(|_| ())
                }
                JournalEntry::RemovedPlugin(id) => {
                    self.run_codex(&["plugin", "add", id, "--json"]).map(|_| ())
                }
                JournalEntry::RemovedLegacyPath(path) => {
                    backup.restore_legacy_path(&self.codex_home, path)
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
    RemovedLegacyPath(PathBuf),
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
            "removeLegacyPaths": plan.remove_legacy_paths,
            "configExisted": config.is_some(),
        });
        fs::write(
            root.join("snapshot.json"),
            serde_json::to_vec_pretty(&snapshot)?,
        )?;
        for path in &plan.remove_legacy_paths {
            let destination = backup_legacy_path(&root, codex_home, path)?;
            copy_path(path, &destination)?;
        }
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

    fn restore_legacy_path(&self, codex_home: &Path, path: &Path) -> Result<()> {
        let source = backup_legacy_path(&self.root, codex_home, path)?;
        if path_exists(path)? {
            remove_exact_path(path)?;
        }
        copy_path(&source, path)
    }
}

fn backup_legacy_path(backup_root: &Path, codex_home: &Path, path: &Path) -> Result<PathBuf> {
    let relative = path.strip_prefix(codex_home).map_err(|_| {
        BootstrapError::new(format!(
            "refusing to back up legacy path outside Codex home: {}",
            path.display()
        ))
    })?;
    Ok(backup_root.join("legacy-paths").join(relative))
}

fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn copy_path(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(BootstrapError::new(format!(
            "legacy backup path contains unsupported symlink: {}",
            source.display()
        )));
    }
    if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(BootstrapError::new(format!(
            "legacy backup path is not a regular file or directory: {}",
            source.display()
        )));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_path(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn remove_exact_path(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::fs::Permissions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::codex::{discover, CodexDiscovery, CommandRunner, CommandSpec, MarketplaceRecord};
use crate::error::{BootstrapError, Result};

pub const CANONICAL_MARKETPLACE: &str = "unica";
pub const CANONICAL_SOURCE: &str = "IngvarConsulting/unica-marketplace";
pub const CANONICAL_REF: &str = "main";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CANONICAL_HTTPS_SOURCE: &str = "https://github.com/ingvarconsulting/unica-marketplace";
const LEGACY_PLUGIN_SELECTOR: &str = "unica@unica-local";
const LEGACY_PLUGIN_CONFIG_TABLE: &str = "[plugins.\"unica@unica-local\"]";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    pub remove_plugin_ids: Vec<String>,
    pub remove_marketplaces: Vec<String>,
    pub add_canonical_marketplace: bool,
    pub upgrade_canonical_marketplace: bool,
    pub install_canonical_plugin: bool,
    pub remove_legacy_paths: Vec<PathBuf>,
    pub preserve_on_rollback_paths: Vec<PathBuf>,
    pub canonical_plugin_root: PathBuf,
    #[serde(skip)]
    legacy_marketplaces: BTreeMap<String, MarketplaceRecord>,
    #[serde(skip)]
    discovered_legacy_plugins: BTreeSet<String>,
    #[serde(skip)]
    original_discovery: CodexDiscovery,
}

impl MigrationPlan {
    pub fn is_noop(&self) -> bool {
        self.remove_plugin_ids.is_empty()
            && self.remove_marketplaces.is_empty()
            && !self.add_canonical_marketplace
            && !self.upgrade_canonical_marketplace
            && !self.install_canonical_plugin
            && self.remove_legacy_paths.is_empty()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub changed: bool,
    pub backup_dir: Option<PathBuf>,
    pub removed_plugins: Vec<String>,
    pub removed_marketplaces: Vec<String>,
    pub removed_legacy_paths: Vec<PathBuf>,
    pub upgraded_canonical_marketplace: bool,
    pub installed_plugin: String,
}

pub fn classify_discovery(discovery: CodexDiscovery, codex_home: &Path) -> Result<MigrationPlan> {
    reject_symlinked_config(codex_home)?;
    let original_discovery = discovery.clone();
    let mut canonical_marketplace = None;
    let mut legacy_marketplaces = BTreeMap::new();
    for marketplace in &discovery.marketplaces.marketplaces {
        match marketplace.name.as_str() {
            "unica" if is_canonical(marketplace) => {
                canonical_marketplace = Some(marketplace.clone())
            }
            "unica" if is_known_legacy_local(marketplace, codex_home)? => {
                legacy_marketplaces.insert(marketplace.name.clone(), marketplace.clone());
            }
            "unica" => {
                return Err(BootstrapError::new(format!(
                    "reserved marketplace name unica is owned by unknown source {}",
                    marketplace.marketplace_source.source
                )));
            }
            "unica-local" if is_known_legacy_local(marketplace, codex_home)? => {
                legacy_marketplaces.insert(marketplace.name.clone(), marketplace.clone());
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
    let mut discovered_legacy_plugins = BTreeSet::new();
    for plugin in &discovery.plugins.installed {
        if plugin.name != "unica" {
            continue;
        }
        if plugin.marketplace_name == CANONICAL_MARKETPLACE && canonical_marketplace.is_some() {
            if is_current_canonical_plugin(plugin) {
                canonical_installed = true;
            } else if is_owned_canonical_plugin(plugin) {
                remove_plugin_ids.insert(plugin.plugin_id.clone());
            } else {
                return Err(BootstrapError::new(format!(
                    "reserved plugin selector {} has unknown source or identity",
                    plugin.plugin_id
                )));
            }
        } else if plugin.marketplace_name == "unica-local"
            && plugin.plugin_id == LEGACY_PLUGIN_SELECTOR
        {
            if legacy_marketplaces.contains_key("unica-local") {
                discovered_legacy_plugins.insert(plugin.plugin_id.clone());
            }
            remove_plugin_ids.insert(plugin.plugin_id.clone());
        } else if plugin.marketplace_name == "unica" && legacy_marketplaces.contains_key("unica") {
            discovered_legacy_plugins.insert(plugin.plugin_id.clone());
            remove_plugin_ids.insert(plugin.plugin_id.clone());
        } else if matches!(plugin.marketplace_name.as_str(), "unica" | "unica-local") {
            return Err(BootstrapError::new(format!(
                "reserved plugin selector {} cannot be attributed to a known Unica source",
                plugin.plugin_id
            )));
        }
    }
    if config_has_legacy_plugin(codex_home)? {
        remove_plugin_ids.insert(LEGACY_PLUGIN_SELECTOR.to_string());
    }

    let add_canonical_marketplace = canonical_marketplace.is_none();
    let install_canonical_plugin = !canonical_installed;
    let upgrade_canonical_marketplace = canonical_marketplace.is_some() && install_canonical_plugin;
    let preserve_on_rollback_paths = existing_rollback_paths(
        codex_home,
        canonical_marketplace.as_ref(),
        upgrade_canonical_marketplace,
    )?;
    let remove_marketplaces = legacy_marketplaces.keys().cloned().collect();
    Ok(MigrationPlan {
        remove_plugin_ids: remove_plugin_ids.into_iter().collect(),
        remove_marketplaces,
        add_canonical_marketplace,
        upgrade_canonical_marketplace,
        install_canonical_plugin,
        remove_legacy_paths: existing_legacy_paths(codex_home)?,
        preserve_on_rollback_paths,
        canonical_plugin_root: canonical_plugin_root(codex_home),
        legacy_marketplaces,
        discovered_legacy_plugins,
        original_discovery,
    })
}

fn reject_symlinked_config(codex_home: &Path) -> Result<()> {
    let config_path = codex_home.join("config.toml");
    match fs::symlink_metadata(&config_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(BootstrapError::new(format!(
            "refusing to migrate with symlinked Codex config: {}",
            config_path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
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
            Ok(_) => Some(validate_legacy_path_tree(&path).map(|()| path)),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => Some(Err(error.into())),
        })
        .collect()
}

fn existing_rollback_paths(
    codex_home: &Path,
    canonical_marketplace: Option<&MarketplaceRecord>,
    upgrade: bool,
) -> Result<Vec<PathBuf>> {
    if !upgrade {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    if let Some(root) = canonical_marketplace.and_then(|marketplace| marketplace.root.as_deref()) {
        candidates.push(PathBuf::from(root));
    }
    candidates.push(
        codex_home
            .join("plugins")
            .join("cache")
            .join("unica")
            .join("unica"),
    );
    let mut existing = Vec::new();
    for path in candidates {
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                ensure_path_within_codex_home(codex_home, &path)?;
                validate_legacy_path_tree(&path)?;
                existing.push(path);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    existing.sort();
    existing.dedup();
    Ok(existing)
}

fn validate_legacy_path_tree(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(BootstrapError::new(format!(
            "legacy path contains unsupported symlink: {}",
            path.display()
        )));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            validate_legacy_path_tree(&entry?.path())?;
        }
    }
    Ok(())
}

fn is_canonical(marketplace: &MarketplaceRecord) -> bool {
    marketplace.marketplace_source.source_type == "git"
        && is_canonical_source(&marketplace.marketplace_source.source)
}

fn is_canonical_source(source: &str) -> bool {
    let normalized = source
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        CANONICAL_HTTPS_SOURCE | "ingvarconsulting/unica-marketplace"
    )
}

fn is_known_legacy_local(marketplace: &MarketplaceRecord, codex_home: &Path) -> Result<bool> {
    if marketplace.marketplace_source.source_type != "local" {
        return Ok(false);
    }
    same_existing_path(
        Path::new(&marketplace.marketplace_source.source),
        &codex_home.join("marketplaces").join("unica-local"),
    )
}

fn same_existing_path(left: &Path, right: &Path) -> Result<bool> {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => Ok(left == right),
        (Err(left_error), Err(right_error))
            if left_error.kind() == ErrorKind::NotFound
                && right_error.kind() == ErrorKind::NotFound =>
        {
            Ok(left == right)
        }
        (Err(left_error), _) if left_error.kind() == ErrorKind::NotFound => Ok(left == right),
        (_, Err(right_error)) if right_error.kind() == ErrorKind::NotFound => Ok(left == right),
        (Err(error), _) | (_, Err(error)) => Err(error.into()),
    }
}

fn is_owned_canonical_plugin(plugin: &crate::codex::PluginRecord) -> bool {
    plugin.plugin_id == "unica@unica"
        && plugin.marketplace_name == CANONICAL_MARKETPLACE
        && plugin.marketplace_source.as_ref().is_some_and(|source| {
            source.source_type == "git" && is_canonical_source(&source.source)
        })
}

fn is_current_canonical_plugin(plugin: &crate::codex::PluginRecord) -> bool {
    is_owned_canonical_plugin(plugin)
        && plugin.version.as_deref() == Some(CURRENT_VERSION)
        && plugin.installed
        && plugin.enabled
        && plugin.source.as_ref().is_some_and(is_current_plugin_source)
}

fn is_current_plugin_source(source: &Value) -> bool {
    let expected_ref = format!("v{CURRENT_VERSION}");
    source.get("source").and_then(Value::as_str) == Some("git-subdir")
        && source
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(is_canonical_source)
        && source
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| matches!(path, "plugins/unica" | "./plugins/unica"))
        && source.get("ref").and_then(Value::as_str) == Some(expected_ref.as_str())
}

fn canonical_plugin_root(codex_home: &Path) -> PathBuf {
    codex_home
        .join("plugins")
        .join("cache")
        .join("unica")
        .join("unica")
        .join(CURRENT_VERSION)
}

fn prove_current_canonical(discovery: CodexDiscovery, codex_home: &Path) -> Result<PathBuf> {
    let current_plugins = discovery
        .plugins
        .installed
        .iter()
        .filter(|plugin| is_current_canonical_plugin(plugin))
        .count();
    let proof = classify_discovery(discovery, codex_home)?;
    if current_plugins != 1
        || !proof.remove_plugin_ids.is_empty()
        || !proof.remove_marketplaces.is_empty()
        || proof.add_canonical_marketplace
        || proof.upgrade_canonical_marketplace
        || proof.install_canonical_plugin
    {
        return Err(BootstrapError::new(
            "Codex discovery did not confirm exactly one current canonical unica@unica",
        ));
    }
    let root = canonical_plugin_root(codex_home);
    if !root.join(".codex-plugin/plugin.json").is_file()
        || !root.join("runtime-manifest.json").is_file()
    {
        return Err(BootstrapError::new(format!(
            "installed canonical Unica package is incomplete: {}",
            root.display()
        )));
    }
    Ok(root)
}

fn verify_restored_discovery(expected: &CodexDiscovery, actual: &CodexDiscovery) -> Result<()> {
    if discovery_signature(expected) == discovery_signature(actual) {
        Ok(())
    } else {
        Err(BootstrapError::new(
            "rollback discovery does not match the preflight installation state",
        ))
    }
}

fn discovery_signature(discovery: &CodexDiscovery) -> (BTreeSet<String>, BTreeSet<String>) {
    let marketplaces = discovery
        .marketplaces
        .marketplaces
        .iter()
        .map(|marketplace| {
            format!(
                "{}\0{}\0{}",
                marketplace.name,
                marketplace.marketplace_source.source_type,
                marketplace.marketplace_source.source
            )
        })
        .collect();
    let plugins = discovery
        .plugins
        .installed
        .iter()
        .map(|plugin| {
            format!(
                "{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
                plugin.plugin_id,
                plugin.name,
                plugin.marketplace_name,
                plugin.version.as_deref().unwrap_or(""),
                plugin.installed,
                plugin.enabled,
                plugin
                    .source
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default(),
                plugin
                    .marketplace_source
                    .as_ref()
                    .map(|source| format!("{}:{}", source.source_type, source.source))
                    .unwrap_or_default()
            )
        })
        .collect();
    (marketplaces, plugins)
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
        F: FnOnce(&Path) -> Result<()>,
    {
        if plan.is_noop() {
            let current = discover(&self.runner, &self.codex_home)?;
            let plugin_root = prove_current_canonical(current, &self.codex_home)?;
            verify(&plugin_root)?;
            return Ok(MigrationReport {
                changed: false,
                backup_dir: None,
                removed_plugins: vec![],
                removed_marketplaces: vec![],
                removed_legacy_paths: vec![],
                upgraded_canonical_marketplace: false,
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
                    "migration failed and was rolled back to the preflight state; backup: {}; resolve the reported cause and rerun the installer: {error}",
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
            upgraded_canonical_marketplace: plan.upgrade_canonical_marketplace,
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
        F: FnOnce(&Path) -> Result<()>,
    {
        for plugin_id in &plan.remove_plugin_ids {
            journal.push(JournalEntry::RemovedPlugin {
                id: plugin_id.clone(),
                restore_via_cli: plan.discovered_legacy_plugins.contains(plugin_id),
            });
            self.run_codex(&["plugin", "remove", plugin_id, "--json"])?;
        }
        for marketplace in &plan.remove_marketplaces {
            journal.push(JournalEntry::RemovedMarketplace(marketplace.clone()));
            self.run_codex(&["plugin", "marketplace", "remove", marketplace, "--json"])?;
        }
        if plan.upgrade_canonical_marketplace {
            journal.push(JournalEntry::UpgradedCanonicalMarketplace);
            self.run_codex(&[
                "plugin",
                "marketplace",
                "upgrade",
                CANONICAL_MARKETPLACE,
                "--json",
            ])?;
        }
        if plan.add_canonical_marketplace {
            journal.push(JournalEntry::AddedCanonicalMarketplace);
            self.run_codex(&[
                "plugin",
                "marketplace",
                "add",
                CANONICAL_SOURCE,
                "--ref",
                CANONICAL_REF,
                "--json",
            ])?;
        }
        if plan.install_canonical_plugin {
            journal.push(JournalEntry::AddedCanonicalPlugin);
            self.run_codex(&["plugin", "add", "unica@unica", "--json"])?;
        }

        let current = discover(&self.runner, &self.codex_home)?;
        let plugin_root = prove_current_canonical(current, &self.codex_home)?;
        verify(&plugin_root)?;

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
        let mut inverse_errors = Vec::new();
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
                JournalEntry::UpgradedCanonicalMarketplace => Ok(()),
                JournalEntry::RemovedMarketplace(name) => {
                    let source = &plan.legacy_marketplaces[name].marketplace_source.source;
                    self.run_codex(&["plugin", "marketplace", "add", source, "--json"])
                        .map(|_| ())
                }
                JournalEntry::RemovedPlugin {
                    id,
                    restore_via_cli: true,
                } => self.run_codex(&["plugin", "add", id, "--json"]).map(|_| ()),
                JournalEntry::RemovedPlugin {
                    restore_via_cli: false,
                    ..
                } => Ok(()),
                JournalEntry::RemovedLegacyPath(path) => {
                    backup.restore_legacy_path(&self.codex_home, path)
                }
            };
            if let Err(error) = result {
                inverse_errors.push(error.to_string());
            }
        }
        if let Err(error) = backup.restore_config(&self.codex_home) {
            errors.push(error.to_string());
        }
        if let Err(error) = backup.restore_preserved_paths(&self.codex_home, plan) {
            errors.push(error.to_string());
        }
        let restoration = discover(&self.runner, &self.codex_home)
            .and_then(|current| verify_restored_discovery(&plan.original_discovery, &current));
        if let Err(error) = restoration {
            errors.push(error.to_string());
            errors.extend(inverse_errors);
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
    RemovedPlugin { id: String, restore_via_cli: bool },
    RemovedMarketplace(String),
    AddedCanonicalMarketplace,
    UpgradedCanonicalMarketplace,
    AddedCanonicalPlugin,
    RemovedLegacyPath(PathBuf),
}

struct Backup {
    root: PathBuf,
    config: Option<Vec<u8>>,
    config_permissions: Option<Permissions>,
}

impl Backup {
    fn capture(codex_home: &Path, plan: &MigrationPlan) -> Result<Self> {
        reject_symlinked_config(codex_home)?;
        let root = codex_home
            .join("unica")
            .join("migration-backups")
            .join(Uuid::new_v4().to_string());
        create_private_dir_all(&root)?;
        let config_path = codex_home.join("config.toml");
        let (config, config_permissions) = if config_path.is_file() {
            let metadata = fs::symlink_metadata(&config_path)?;
            let bytes = fs::read(&config_path)?;
            write_private_file(&root.join("config.toml"), &bytes)?;
            (Some(bytes), Some(metadata.permissions()))
        } else {
            (None, None)
        };
        let snapshot = serde_json::json!({
            "schemaVersion": 2,
            "removePluginIds": plan.remove_plugin_ids,
            "removeMarketplaces": plan.remove_marketplaces,
            "upgradeCanonicalMarketplace": plan.upgrade_canonical_marketplace,
            "removeLegacyPaths": plan.remove_legacy_paths,
            "preserveOnRollbackPaths": plan.preserve_on_rollback_paths,
            "configExisted": config.is_some(),
        });
        write_private_file(
            &root.join("snapshot.json"),
            &serde_json::to_vec_pretty(&snapshot)?,
        )?;
        for path in &plan.remove_legacy_paths {
            let destination = backup_path(&root, "legacy-paths", codex_home, path)?;
            copy_path(path, &destination)?;
        }
        for path in &plan.preserve_on_rollback_paths {
            let destination = backup_path(&root, "preserved-paths", codex_home, path)?;
            copy_path(path, &destination)?;
        }
        Ok(Self {
            root,
            config,
            config_permissions,
        })
    }

    fn restore_config(&self, codex_home: &Path) -> Result<()> {
        let config_path = codex_home.join("config.toml");
        match &self.config {
            Some(bytes) => {
                let temporary = codex_home.join(format!(".config.toml.restore-{}", Uuid::new_v4()));
                write_private_file(&temporary, bytes)?;
                if let Some(permissions) = &self.config_permissions {
                    fs::set_permissions(&temporary, permissions.clone())?;
                }
                replace_config_file(&temporary, &config_path)?;
            }
            None if config_path.exists() => fs::remove_file(config_path)?,
            None => {}
        }
        Ok(())
    }

    fn restore_legacy_path(&self, codex_home: &Path, path: &Path) -> Result<()> {
        let source = backup_path(&self.root, "legacy-paths", codex_home, path)?;
        if path_exists(path)? {
            remove_exact_path(path)?;
        }
        copy_path(&source, path)
    }

    fn restore_preserved_paths(&self, codex_home: &Path, plan: &MigrationPlan) -> Result<()> {
        for path in &plan.preserve_on_rollback_paths {
            let source = backup_path(&self.root, "preserved-paths", codex_home, path)?;
            if path_exists(path)? {
                remove_exact_path(path)?;
            }
            copy_path(&source, path)?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_config_file(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(windows)]
fn replace_config_file(source: &Path, destination: &Path) -> Result<()> {
    match fs::remove_file(destination) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    fs::rename(source, destination)?;
    Ok(())
}

fn backup_path(
    backup_root: &Path,
    category: &str,
    codex_home: &Path,
    path: &Path,
) -> Result<PathBuf> {
    let relative = relative_to_codex_home(codex_home, path).map_err(|_| {
        BootstrapError::new(format!(
            "refusing to back up legacy path outside Codex home: {}",
            path.display()
        ))
    })?;
    Ok(backup_root.join(category).join(relative))
}

fn relative_to_codex_home(codex_home: &Path, path: &Path) -> std::result::Result<PathBuf, ()> {
    if let Ok(relative) = path.strip_prefix(codex_home) {
        return Ok(relative.to_path_buf());
    }
    let canonical_home = fs::canonicalize(codex_home).map_err(|_| ())?;
    let canonical_path = fs::canonicalize(path).map_err(|_| ())?;
    canonical_path
        .strip_prefix(canonical_home)
        .map(Path::to_path_buf)
        .map_err(|_| ())
}

fn ensure_path_within_codex_home(codex_home: &Path, path: &Path) -> Result<()> {
    relative_to_codex_home(codex_home, path)
        .map(|_| ())
        .map_err(|_| {
            BootstrapError::new(format!(
                "refusing to manage path outside Codex home: {}",
                path.display()
            ))
        })
}

fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn create_private_dir_all(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
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
    fs::set_permissions(destination, metadata.permissions())?;
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

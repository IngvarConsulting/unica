use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use unica_bootstrap::{
    classify_discovery, BootstrapError, CodexDiscovery, CommandRunner, CommandSpec,
    MarketplaceList, MarketplaceRecord, MarketplaceSource, MigrationEngine, PluginList,
    PluginRecord, Result,
};

fn marketplace(name: &str, source_type: &str, source: &str) -> MarketplaceRecord {
    MarketplaceRecord {
        name: name.to_string(),
        root: Some(format!("/cache/{name}")),
        marketplace_source: MarketplaceSource {
            source_type: source_type.to_string(),
            source: source.to_string(),
            extra: Default::default(),
        },
        extra: Default::default(),
    }
}

fn plugin(id: &str, marketplace_name: &str, installed: bool) -> PluginRecord {
    PluginRecord {
        plugin_id: id.to_string(),
        name: "unica".to_string(),
        marketplace_name: marketplace_name.to_string(),
        version: Some("0.6.1".to_string()),
        installed,
        enabled: installed,
        source: None,
        marketplace_source: None,
        extra: Default::default(),
    }
}

#[test]
fn parses_current_empty_codex_json_contract() {
    let marketplaces: MarketplaceList =
        serde_json::from_str(include_str!("fixtures/marketplaces-empty.json")).unwrap();
    let plugins: PluginList =
        serde_json::from_str(include_str!("fixtures/plugins-empty.json")).unwrap();

    assert!(marketplaces.marketplaces.is_empty());
    assert!(plugins.installed.is_empty());
    assert!(plugins.available.is_empty());
}

#[test]
fn classifies_local_and_unica_local_duplicates_as_one_legacy_migration() {
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![
                marketplace("unica", "local", "/old/unica"),
                marketplace("unica-local", "local", "/old/unica-local"),
            ],
            ..Default::default()
        },
        plugins: PluginList {
            installed: vec![
                plugin("unica@unica", "unica", true),
                plugin("unica@unica-local", "unica-local", true),
            ],
            available: vec![],
            ..Default::default()
        },
    };

    let plan = classify_discovery(discovery, Path::new("/codex-home")).unwrap();

    assert_eq!(
        plan.remove_plugin_ids,
        vec!["unica@unica", "unica@unica-local"]
    );
    assert_eq!(plan.remove_marketplaces, vec!["unica", "unica-local"]);
    assert!(plan.add_canonical_marketplace);
    assert!(plan.install_canonical_plugin);
}

#[test]
fn canonical_git_marketplace_and_installed_plugin_are_idempotent() {
    let discovery = canonical_discovery();

    let plan = classify_discovery(discovery, Path::new("/codex-home")).unwrap();

    assert!(plan.is_noop());
}

#[test]
fn canonical_discovery_still_classifies_orphaned_legacy_config_and_paths() {
    let codex_home = temp_root("orphaned-preflight");
    fs::write(
        codex_home.join("config.toml"),
        b"[plugins.\"unica@unica-local\"]\nenabled = true\n\n[plugins.\"unica@unica\"]\nenabled = true\n",
    )
    .unwrap();
    let marketplace = codex_home.join("marketplaces/unica-local");
    let cache = codex_home.join("plugins/cache/unica-local");
    fs::create_dir_all(&marketplace).unwrap();
    fs::create_dir_all(&cache).unwrap();

    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();

    assert_eq!(plan.remove_plugin_ids, vec!["unica@unica-local"]);
    assert!(plan.remove_marketplaces.is_empty());
    assert!(!plan.add_canonical_marketplace);
    assert!(!plan.install_canonical_plugin);
    assert_eq!(plan.remove_legacy_paths, vec![marketplace, cache]);
}

#[cfg(unix)]
#[test]
fn preflight_rejects_symlink_inside_exact_legacy_path() {
    use std::os::unix::fs::symlink;

    let codex_home = temp_root("legacy-symlink");
    let marketplace = codex_home.join("marketplaces/unica-local");
    fs::create_dir_all(&marketplace).unwrap();
    symlink(codex_home.join("config.toml"), marketplace.join("escape")).unwrap();

    let error = classify_discovery(canonical_discovery(), &codex_home).unwrap_err();

    assert!(error.to_string().contains("unsupported symlink"), "{error}");
    assert!(marketplace.join("escape").exists() || marketplace.join("escape").is_symlink());
}

#[test]
fn successful_orphan_cleanup_retains_exact_backup_and_becomes_idempotent() {
    let codex_home = orphaned_legacy_home("orphaned-success");
    let runner = OrphanCleanupRunner::new(codex_home.clone(), false);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();

    let verify_home = codex_home.clone();
    let report = engine
        .apply(plan, || {
            assert!(verify_home.join("marketplaces/unica-local").exists());
            assert!(verify_home.join("plugins/cache/unica-local").exists());
            Ok(())
        })
        .unwrap();

    let backup = report.backup_dir.unwrap();
    assert!(!codex_home.join("marketplaces/unica-local").exists());
    assert!(!codex_home.join("plugins/cache/unica-local").exists());
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).unwrap(),
        canonical_config()
    );
    assert_eq!(
        fs::read_to_string(backup.join("legacy-paths/marketplaces/unica-local/marker")).unwrap(),
        "marketplace"
    );
    assert_eq!(
        fs::read_to_string(backup.join("legacy-paths/plugins/cache/unica-local/marker")).unwrap(),
        "cache"
    );
    assert!(engine.preflight().unwrap().is_noop());
}

#[test]
fn failed_post_cleanup_proof_restores_config_and_exact_legacy_paths() {
    let codex_home = orphaned_legacy_home("orphaned-rollback");
    let original_config = fs::read(codex_home.join("config.toml")).unwrap();
    let runner = OrphanCleanupRunner::new(codex_home.clone(), true);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();

    let error = engine.apply(plan, || Ok(())).unwrap_err();

    assert!(error.to_string().contains("rolled back"), "{error}");
    assert_eq!(
        fs::read(codex_home.join("config.toml")).unwrap(),
        original_config
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("marketplaces/unica-local/marker")).unwrap(),
        "marketplace"
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("plugins/cache/unica-local/marker")).unwrap(),
        "cache"
    );
}

#[test]
fn failed_runtime_verification_rolls_back_before_legacy_cleanup() {
    let codex_home = orphaned_legacy_home("verification-rollback");
    let original_config = fs::read(codex_home.join("config.toml")).unwrap();
    let runner = OrphanCleanupRunner::new(codex_home.clone(), false);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();

    let error = engine
        .apply(plan, || {
            Err(BootstrapError::new("injected MCP verification failure"))
        })
        .unwrap_err();

    assert!(error.to_string().contains("rolled back"), "{error}");
    assert!(error.to_string().contains("MCP verification"), "{error}");
    assert_eq!(
        fs::read(codex_home.join("config.toml")).unwrap(),
        original_config
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("marketplaces/unica-local/marker")).unwrap(),
        "marketplace"
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("plugins/cache/unica-local/marker")).unwrap(),
        "cache"
    );
}

#[test]
fn unknown_owner_of_reserved_unica_name_fails_before_mutation() {
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![marketplace(
                "unica",
                "git",
                "https://github.com/example/not-unica.git",
            )],
            ..Default::default()
        },
        plugins: PluginList::default(),
    };

    let error = classify_discovery(discovery, Path::new("/codex-home")).unwrap_err();

    assert!(error
        .to_string()
        .contains("reserved marketplace name unica"));
}

#[test]
fn canonical_marketplace_without_install_only_adds_plugin() {
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![marketplace(
                "unica",
                "git",
                "https://github.com/IngvarConsulting/unica-marketplace.git",
            )],
            ..Default::default()
        },
        plugins: PluginList::default(),
    };

    let plan = classify_discovery(discovery, Path::new("/codex-home")).unwrap();

    assert!(!plan.add_canonical_marketplace);
    assert!(plan.install_canonical_plugin);
}

#[test]
fn each_mutation_failure_restores_exact_config_and_keeps_backup() {
    for fail_on in 0..4 {
        let codex_home = temp_root(&format!("rollback-{fail_on}"));
        let original = b"model = \"gpt-5\"\n# exact legacy config\n";
        fs::write(codex_home.join("config.toml"), original).unwrap();
        let runner = FailingMutationRunner::new(codex_home.clone(), fail_on);
        let engine = MigrationEngine::new(codex_home.clone(), runner);
        let plan = classify_discovery(legacy_discovery(), &codex_home).unwrap();

        let error = engine.apply(plan, || Ok(())).unwrap_err();

        assert!(error.to_string().contains("rolled back"), "{error}");
        assert_eq!(fs::read(codex_home.join("config.toml")).unwrap(), original);
        let backups = fs::read_dir(codex_home.join("unica/migration-backups"))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].path().join("snapshot.json").is_file());
    }
}

fn legacy_discovery() -> CodexDiscovery {
    CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![marketplace("unica-local", "local", "/old/unica-local")],
            ..Default::default()
        },
        plugins: PluginList {
            installed: vec![plugin("unica@unica-local", "unica-local", true)],
            ..Default::default()
        },
    }
}

fn canonical_discovery() -> CodexDiscovery {
    CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![marketplace(
                "unica",
                "git",
                "https://github.com/IngvarConsulting/unica-marketplace.git",
            )],
            ..Default::default()
        },
        plugins: PluginList {
            installed: vec![plugin("unica@unica", "unica", true)],
            available: vec![],
            ..Default::default()
        },
    }
}

fn canonical_config() -> &'static str {
    "[plugins.\"unica@unica\"]\nenabled = true\n"
}

fn orphaned_legacy_home(name: &str) -> PathBuf {
    let codex_home = temp_root(name);
    fs::write(
        codex_home.join("config.toml"),
        format!(
            "[plugins.\"unica@unica-local\"]\nenabled = true\n\n{}",
            canonical_config()
        ),
    )
    .unwrap();
    let marketplace = codex_home.join("marketplaces/unica-local");
    let cache = codex_home.join("plugins/cache/unica-local");
    fs::create_dir_all(&marketplace).unwrap();
    fs::create_dir_all(&cache).unwrap();
    fs::write(marketplace.join("marker"), "marketplace").unwrap();
    fs::write(cache.join("marker"), "cache").unwrap();
    codex_home
}

struct OrphanCleanupRunner {
    codex_home: PathBuf,
    fail_final_plugin_discovery: bool,
    plugin_discoveries: Mutex<usize>,
}

impl OrphanCleanupRunner {
    fn new(codex_home: PathBuf, fail_final_plugin_discovery: bool) -> Self {
        Self {
            codex_home,
            fail_final_plugin_discovery,
            plugin_discoveries: Mutex::new(0),
        }
    }
}

impl CommandRunner for OrphanCleanupRunner {
    fn run(&self, command: &CommandSpec) -> Result<String> {
        if command.program == "git" {
            return Ok(String::new());
        }
        if command
            .args
            .ends_with(&["marketplace".into(), "list".into(), "--json".into()])
        {
            return Ok(serde_json::to_string(&canonical_discovery().marketplaces).unwrap());
        }
        if command
            .args
            .ends_with(&["list".into(), "--available".into(), "--json".into()])
        {
            let mut discoveries = self.plugin_discoveries.lock().unwrap();
            *discoveries += 1;
            if self.fail_final_plugin_discovery && *discoveries == 2 {
                return Ok(serde_json::to_string(&PluginList::default()).unwrap());
            }
            return Ok(serde_json::to_string(&canonical_discovery().plugins).unwrap());
        }
        if command.args
            == [
                "plugin".to_string(),
                "remove".to_string(),
                "unica@unica-local".to_string(),
                "--json".to_string(),
            ]
        {
            fs::write(self.codex_home.join("config.toml"), canonical_config()).unwrap();
        }
        if command.args
            == [
                "plugin".to_string(),
                "add".to_string(),
                "unica@unica-local".to_string(),
                "--json".to_string(),
            ]
        {
            return Err(BootstrapError::new(
                "config-only orphan cannot be reinstalled through Codex CLI",
            ));
        }
        Ok("{}".to_string())
    }
}

struct FailingMutationRunner {
    codex_home: PathBuf,
    state: Mutex<RunnerState>,
}

struct RunnerState {
    fail_on: usize,
    mutation_index: usize,
    failed: bool,
}

impl FailingMutationRunner {
    fn new(codex_home: PathBuf, fail_on: usize) -> Self {
        Self {
            codex_home,
            state: Mutex::new(RunnerState {
                fail_on,
                mutation_index: 0,
                failed: false,
            }),
        }
    }
}

impl CommandRunner for FailingMutationRunner {
    fn run(&self, command: &CommandSpec) -> Result<String> {
        if command.program == "git" {
            return Ok(String::new());
        }
        let is_discovery = command
            .args
            .ends_with(&["list".to_string(), "--json".to_string()])
            || command.args.ends_with(&[
                "list".to_string(),
                "--available".to_string(),
                "--json".to_string(),
            ]);
        if is_discovery {
            if command.args.get(1).map(String::as_str) == Some("marketplace") {
                return Ok(serde_json::json!({
                    "marketplaces": [{
                        "name": "unica",
                        "root": "/cache/unica",
                        "marketplaceSource": {
                            "sourceType": "git",
                            "source": "https://github.com/IngvarConsulting/unica-marketplace.git"
                        }
                    }]
                })
                .to_string());
            }
            return Ok(serde_json::json!({
                "installed": [{
                    "pluginId": "unica@unica",
                    "name": "unica",
                    "marketplaceName": "unica",
                    "installed": true,
                    "enabled": true
                }],
                "available": []
            })
            .to_string());
        }

        fs::write(self.codex_home.join("config.toml"), b"partially mutated\n").unwrap();
        let mut state = self.state.lock().unwrap();
        if !state.failed && state.mutation_index == state.fail_on {
            state.failed = true;
            return Err(BootstrapError::new(
                "injected mutation failure token=secret",
            ));
        }
        state.mutation_index += 1;
        Ok("{}".to_string())
    }
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-migration-{name}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    root
}

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use toml_edit::{DocumentMut, Item, TableLike};

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

fn canonical_plugin(version: &str) -> PluginRecord {
    PluginRecord {
        plugin_id: "unica@unica".to_string(),
        name: "unica".to_string(),
        marketplace_name: "unica".to_string(),
        version: Some(version.to_string()),
        installed: true,
        enabled: true,
        source: Some(serde_json::json!({
            "source": "git-subdir",
            "url": "https://github.com/IngvarConsulting/unica-marketplace.git",
            "path": "plugins/unica",
            "ref": format!("v{version}"),
        })),
        marketplace_source: Some(MarketplaceSource {
            source_type: "git".to_string(),
            source: "https://github.com/IngvarConsulting/unica-marketplace.git".to_string(),
            extra: Default::default(),
        }),
        extra: Default::default(),
    }
}

fn materialize_contract_fixture(contents: &str, codex_home: &str) -> serde_json::Value {
    fn replace_token(value: &mut serde_json::Value, codex_home: &str) {
        match value {
            serde_json::Value::Array(items) => {
                for item in items {
                    replace_token(item, codex_home);
                }
            }
            serde_json::Value::Object(fields) => {
                for item in fields.values_mut() {
                    replace_token(item, codex_home);
                }
            }
            serde_json::Value::String(text) => {
                *text = text.replace("${CODEX_HOME}", codex_home);
            }
            _ => {}
        }
    }

    let mut value: serde_json::Value = serde_json::from_str(contents).unwrap();
    replace_token(&mut value, codex_home);
    value
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
fn parses_codex_0_145_contract_when_unrelated_marketplace_omits_source() {
    let codex_home = temp_root("codex-0-145-contract");
    let home = codex_home.to_string_lossy();
    let metadata: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/codex-0.145.0-alpha.18/metadata.json"
    ))
    .unwrap();
    let marketplaces = materialize_contract_fixture(
        include_str!("fixtures/codex-0.145.0-alpha.18/marketplaces-local.json"),
        &home,
    );
    let plugins = materialize_contract_fixture(
        include_str!("fixtures/codex-0.145.0-alpha.18/plugins-installed.json"),
        &home,
    );

    let discovery = CodexDiscovery {
        marketplaces: serde_json::from_value(marketplaces).unwrap(),
        plugins: serde_json::from_value(plugins).unwrap(),
    };
    let plan = classify_discovery(discovery, &codex_home).unwrap();

    assert_eq!(metadata["codexVersion"], "codex-cli 0.145.0-alpha.18");
    assert_eq!(metadata["captureKind"], "minimized-from-real-output");
    assert_eq!(
        metadata["officialRelease"]["windowsAssetSha256"],
        "f719bcb43de2bcfed3af1055e53a57fa9b7ed00dcbce70c13ec71fd1f41ba86a"
    );
    assert_eq!(plan.remove_plugin_ids, vec!["unica@unica"]);
    assert_eq!(plan.remove_marketplaces, vec!["unica"]);
    assert!(plan.add_canonical_marketplace);
    assert!(plan.install_canonical_plugin);
}

#[test]
fn codex_contract_fixture_materializes_a_windows_root_as_json_data() {
    let marketplaces = materialize_contract_fixture(
        include_str!("fixtures/codex-0.145.0-alpha.18/marketplaces-local.json"),
        r"D:\a\unica\codex-home",
    );

    let parsed: MarketplaceList = serde_json::from_value(marketplaces).unwrap();

    assert_eq!(
        parsed.marketplaces[1].root.as_deref(),
        Some(r"D:\a\unica\codex-home/marketplaces/unica-local")
    );
}

#[test]
fn classifies_local_and_unica_local_duplicates_as_one_legacy_migration() {
    let codex_home = temp_root("legacy-duplicates");
    let legacy_root = codex_home.join("marketplaces/unica-local");
    fs::create_dir_all(&legacy_root).unwrap();
    let source = legacy_root.to_string_lossy();
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![
                marketplace("unica", "local", &source),
                marketplace("unica-local", "local", &source),
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

    let plan = classify_discovery(discovery, &codex_home).unwrap();

    assert_eq!(
        plan.remove_plugin_ids,
        vec!["unica@unica", "unica@unica-local"]
    );
    assert_eq!(plan.remove_marketplaces, vec!["unica", "unica-local"]);
    assert!(plan.add_canonical_marketplace);
    assert!(plan.install_canonical_plugin);
}

#[test]
fn exact_orphaned_local_registration_is_owned_even_when_source_path_is_missing() {
    let codex_home = temp_root("missing-legacy-source");
    let source = codex_home.join("marketplaces/unica-local");
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![marketplace(
                "unica-local",
                "local",
                &source.to_string_lossy(),
            )],
            ..Default::default()
        },
        plugins: PluginList::default(),
    };

    let plan = classify_discovery(discovery, &codex_home).unwrap();

    assert_eq!(plan.remove_marketplaces, vec!["unica-local"]);
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

#[cfg(unix)]
#[test]
fn preflight_rejects_symlinked_codex_config() {
    use std::os::unix::fs::symlink;

    let codex_home = temp_root("config-symlink");
    let target = codex_home.join("real-config.toml");
    fs::write(&target, canonical_config()).unwrap();
    symlink(&target, codex_home.join("config.toml")).unwrap();

    let error = classify_discovery(canonical_discovery(), &codex_home).unwrap_err();

    assert!(
        error.to_string().contains("symlinked Codex config"),
        "{error}"
    );
}

#[test]
fn successful_orphan_cleanup_retains_exact_backup_and_becomes_idempotent() {
    let codex_home = orphaned_legacy_home("orphaned-success");
    let runner = OrphanCleanupRunner::new(codex_home.clone(), false);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();

    let verify_home = codex_home.clone();
    let report = engine
        .apply(plan, |_| {
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(backup.join("config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(backup.join("diagnostics.jsonl"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(engine.preflight().unwrap().is_noop());
}

#[test]
fn failed_post_cleanup_proof_restores_config_and_exact_legacy_paths() {
    let codex_home = orphaned_legacy_home("orphaned-rollback");
    let original_config = fs::read(codex_home.join("config.toml")).unwrap();
    let runner = OrphanCleanupRunner::new(codex_home.clone(), true);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();

    let error = engine.apply(plan, |_| Ok(())).unwrap_err();

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

#[cfg(unix)]
#[test]
fn rollback_restores_original_codex_config_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let codex_home = orphaned_legacy_home("config-permissions");
    fs::set_permissions(
        codex_home.join("config.toml"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    let runner = OrphanCleanupRunner::new(codex_home.clone(), true);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();

    engine.apply(plan, |_| Ok(())).unwrap_err();

    assert_eq!(
        fs::metadata(codex_home.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
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
        .apply(plan, |_| {
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
fn rollback_proof_rejects_a_different_active_marketplace_root() {
    let codex_home = orphaned_legacy_home("rollback-root-mismatch");
    let runner = OrphanCleanupRunner::with_root_mismatch(codex_home.clone());
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();

    let error = engine
        .apply(plan, |_| {
            Err(BootstrapError::new("injected verification failure"))
        })
        .unwrap_err();

    assert!(
        error.to_string().contains("rollback also failed"),
        "{error}"
    );
    assert!(error
        .to_string()
        .contains("rollback discovery does not match"));
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
fn reserved_unica_without_source_is_rejected_as_missing_identity() {
    let marketplaces: MarketplaceList =
        serde_json::from_str(r#"{"marketplaces":[{"name":"unica","root":"/unknown"}]}"#).unwrap();
    let discovery = CodexDiscovery {
        marketplaces,
        plugins: PluginList::default(),
    };

    let error = classify_discovery(discovery, Path::new("/codex-home")).unwrap_err();

    assert!(
        error.to_string().contains("missing source identity"),
        "{error}"
    );
}

#[test]
fn source_less_unica_root_is_owned_only_with_the_official_v061_contract() {
    let codex_home = temp_root("source-less-unica-v061");
    let legacy_root = codex_home.join("marketplaces/unica");
    let manifest_dir = legacy_root.join("plugins/unica/.codex-plugin");
    let legacy_cache = codex_home.join("plugins/cache/unica/unica");
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::create_dir_all(&legacy_cache).unwrap();
    fs::write(
        manifest_dir.join("plugin.json"),
        r#"{
          "name": "unica",
          "version": "0.6.1",
          "repository": "https://github.com/IngvarConsulting/unica"
        }"#,
    )
    .unwrap();
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![MarketplaceRecord {
                name: "unica".to_string(),
                root: Some(legacy_root.to_string_lossy().into_owned()),
                marketplace_source: MarketplaceSource::default(),
                extra: Default::default(),
            }],
            ..Default::default()
        },
        plugins: PluginList::default(),
    };

    let plan = classify_discovery(discovery, &codex_home).unwrap();

    assert_eq!(plan.remove_marketplaces, vec!["unica"]);
    assert!(plan.remove_legacy_paths.contains(&legacy_root));
    assert!(plan.remove_legacy_paths.contains(&legacy_cache));
}

#[test]
fn source_less_unica_root_with_an_unknown_package_contract_is_rejected() {
    let codex_home = temp_root("source-less-unica-unknown");
    let legacy_root = codex_home.join("marketplaces/unica");
    fs::create_dir_all(&legacy_root).unwrap();
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![MarketplaceRecord {
                name: "unica".to_string(),
                root: Some(legacy_root.to_string_lossy().into_owned()),
                marketplace_source: MarketplaceSource::default(),
                extra: Default::default(),
            }],
            ..Default::default()
        },
        plugins: PluginList::default(),
    };

    let error = classify_discovery(discovery, &codex_home).unwrap_err();

    assert!(
        error.to_string().contains("missing source identity"),
        "{error}"
    );
}

#[test]
fn canonical_source_matching_only_by_substring_is_rejected() {
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![marketplace(
                "unica",
                "git",
                "https://github.com/IngvarConsulting/unica-marketplace-evil.git",
            )],
            ..Default::default()
        },
        plugins: PluginList::default(),
    };

    let error = classify_discovery(discovery, Path::new("/codex-home")).unwrap_err();

    assert!(error.to_string().contains("unknown source"), "{error}");
}

#[test]
fn local_reserved_marketplace_outside_exact_legacy_root_is_rejected() {
    let codex_home = temp_root("unknown-local");
    let unknown = codex_home.join("marketplaces/not-unica");
    fs::create_dir_all(&unknown).unwrap();
    let discovery = CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![marketplace("unica", "local", &unknown.to_string_lossy())],
            ..Default::default()
        },
        plugins: PluginList::default(),
    };

    let error = classify_discovery(discovery, &codex_home).unwrap_err();

    assert!(error.to_string().contains("unknown source"), "{error}");
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
    assert!(plan.upgrade_canonical_marketplace);
    assert!(plan.install_canonical_plugin);
}

#[test]
fn previous_canonical_version_is_upgraded_and_preserved_for_rollback() {
    let codex_home = temp_root("previous-stable");
    let marketplace_root = codex_home.join(".tmp/marketplaces/unica");
    let plugin_cache = codex_home.join("plugins/cache/unica/unica");
    fs::create_dir_all(&marketplace_root).unwrap();
    fs::create_dir_all(plugin_cache.join("0.7.2")).unwrap();
    fs::write(marketplace_root.join("marker"), "old marketplace").unwrap();
    fs::write(plugin_cache.join("0.7.2/marker"), "old plugin").unwrap();
    let mut discovery = canonical_discovery();
    discovery.marketplaces.marketplaces[0].root =
        Some(marketplace_root.to_string_lossy().into_owned());
    discovery.plugins.installed = vec![canonical_plugin("0.7.2")];

    let plan = classify_discovery(discovery, &codex_home).unwrap();

    assert_eq!(plan.remove_plugin_ids, vec!["unica@unica"]);
    assert!(plan.upgrade_canonical_marketplace);
    assert!(plan.install_canonical_plugin);
    assert_eq!(
        plan.preserve_on_rollback_paths,
        vec![marketplace_root, plugin_cache]
    );
}

#[test]
fn canonical_marketplace_root_with_parent_traversal_is_rejected_before_backup() {
    let codex_home = temp_root("root-parent-traversal");
    let victim = codex_home.parent().unwrap().join("unica-path-victim");
    fs::create_dir_all(&victim).unwrap();
    fs::write(victim.join("marker"), "outside").unwrap();
    let mut discovery = previous_canonical_discovery(&codex_home);
    discovery.marketplaces.marketplaces[0].root = Some(
        codex_home
            .join("../unica-path-victim")
            .to_string_lossy()
            .into_owned(),
    );

    let error = classify_discovery(discovery, &codex_home).unwrap_err();

    assert!(error.to_string().contains("unsafe components"), "{error}");
    assert_eq!(
        fs::read_to_string(victim.join("marker")).unwrap(),
        "outside"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_managed_path_ancestor_is_rejected_before_backup_or_cleanup() {
    use std::os::unix::fs::symlink;

    let codex_home = temp_root("ancestor-symlink");
    let victim = codex_home.parent().unwrap().join("unica-symlink-victim");
    fs::create_dir_all(victim.join("unica-local")).unwrap();
    fs::write(victim.join("unica-local/marker"), "outside").unwrap();
    symlink(&victim, codex_home.join("marketplaces")).unwrap();

    let error = classify_discovery(canonical_discovery(), &codex_home).unwrap_err();

    assert!(error.to_string().contains("symlinked ancestor"), "{error}");
    assert_eq!(
        fs::read_to_string(victim.join("unica-local/marker")).unwrap(),
        "outside"
    );
}

#[test]
fn previous_canonical_version_updates_and_verifies_installed_plugin_root() {
    let codex_home = previous_canonical_home("update-success");
    let runner = CanonicalUpdateRunner::new(codex_home.clone());
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(previous_canonical_discovery(&codex_home), &codex_home).unwrap();
    let expected_root = current_plugin_root(&codex_home);

    let report = engine
        .apply(plan, |installed_root| {
            assert_eq!(installed_root, expected_root);
            assert!(installed_root.join(".codex-plugin/plugin.json").is_file());
            Ok(())
        })
        .unwrap();

    assert!(report.upgraded_canonical_marketplace);
    assert_eq!(
        fs::read_to_string(codex_home.join(".tmp/marketplaces/unica/marker")).unwrap(),
        "current marketplace"
    );
    assert!(current_plugin_root(&codex_home).is_dir());
    assert!(!codex_home.join("plugins/cache/unica/unica/0.7.2").exists());
}

#[test]
fn canonical_update_preserves_direct_canonical_plugin_setting() {
    let codex_home = previous_canonical_home("update-preserves-settings");
    fs::write(
        codex_home.join("config.toml"),
        canonical_config_with_user_owned_settings(),
    )
    .unwrap();
    let runner = NativeCodexMutationRunner::new(codex_home.clone());
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(previous_canonical_discovery(&codex_home), &codex_home).unwrap();

    engine.apply(plan, |_| Ok(())).unwrap();

    assert_canonical_direct_user_setting(&codex_home);
}

#[test]
fn canonical_update_preserves_nested_canonical_plugin_setting() {
    let codex_home = previous_canonical_home("update-preserves-nested-setting");
    fs::write(
        codex_home.join("config.toml"),
        canonical_config_with_user_owned_settings(),
    )
    .unwrap();
    let runner = NativeCodexMutationRunner::new(codex_home.clone());
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(previous_canonical_discovery(&codex_home), &codex_home).unwrap();

    engine.apply(plan, |_| Ok(())).unwrap();

    assert_canonical_nested_user_owned_server(&codex_home);
}

#[test]
fn orphan_alias_removal_without_install_preserves_direct_canonical_setting() {
    let codex_home = orphaned_legacy_home("orphan-preserves-settings");
    fs::write(
        codex_home.join("config.toml"),
        format!(
            "[plugins.\"unica@unica-local\"]\nenabled = true\n\n{}",
            canonical_config_with_user_owned_settings()
        ),
    )
    .unwrap();
    let runner = OrphanCleanupRunner::new(codex_home.clone(), false);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();
    assert!(!plan.install_canonical_plugin);

    engine.apply(plan, |_| Ok(())).unwrap();

    assert_alias_plugin_table_is_absent(&codex_home);
    assert_canonical_direct_user_setting(&codex_home);
}

#[test]
fn orphan_alias_removal_without_install_preserves_nested_canonical_setting() {
    let codex_home = orphaned_legacy_home("orphan-preserves-nested-setting");
    fs::write(
        codex_home.join("config.toml"),
        format!(
            "[plugins.\"unica@unica-local\"]\nenabled = true\n\n{}",
            canonical_config_with_user_owned_settings()
        ),
    )
    .unwrap();
    let runner = OrphanCleanupRunner::new(codex_home.clone(), false);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();
    assert!(!plan.install_canonical_plugin);

    engine.apply(plan, |_| Ok(())).unwrap();

    assert_alias_plugin_table_is_absent(&codex_home);
    assert_canonical_nested_user_owned_server(&codex_home);
}

#[test]
fn migration_without_plugin_mutation_does_not_rewrite_config() {
    let codex_home = temp_root("path-only-keeps-config-bytes");
    let original =
        b"# keep exact formatting\n[plugins.\"unica@unica\"]\nenabled=true # keep spacing\n";
    fs::write(codex_home.join("config.toml"), original).unwrap();
    fs::create_dir_all(codex_home.join("marketplaces/unica-local")).unwrap();
    fs::create_dir_all(codex_home.join("plugins/cache/unica-local")).unwrap();
    write_current_plugin_package(&codex_home);
    let runner = OrphanCleanupRunner::new(codex_home.clone(), false);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(canonical_discovery(), &codex_home).unwrap();
    assert!(plan.remove_plugin_ids.is_empty());
    assert!(!plan.install_canonical_plugin);
    assert!(!plan.remove_legacy_paths.is_empty());

    engine.apply(plan, |_| Ok(())).unwrap();

    assert_eq!(fs::read(codex_home.join("config.toml")).unwrap(), original);
}

#[test]
fn canonical_update_overlays_user_values_onto_fresh_table_and_preserves_decorations() {
    let codex_home = previous_canonical_home("update-merges-fresh-and-user-settings");
    fs::write(
        codex_home.join("config.toml"),
        "[plugins.\"unica@unica\"]\n\
         enabled = false # stale installer value\n\
         inline_user = { mode = \"fast\", tags = [\"one\", \"two\"] } # keep inline comment\n\
         user_array = [\n\
           \"alpha\", # keep array comment\n\
           \"beta\",\n\
         ]\n",
    )
    .unwrap();
    let runner = NativeCodexMutationRunner::new(codex_home.clone());
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(previous_canonical_discovery(&codex_home), &codex_home).unwrap();

    engine.apply(plan, |_| Ok(())).unwrap();

    let rendered = fs::read_to_string(codex_home.join("config.toml")).unwrap();
    let config: DocumentMut = rendered.parse().unwrap();
    let canonical = canonical_plugin_table(&config);
    assert_eq!(
        canonical.get("fresh_metadata").and_then(Item::as_str),
        Some("generated-by-codex")
    );
    assert_eq!(
        canonical
            .get("inline_user")
            .and_then(Item::as_inline_table)
            .and_then(|table| table.get("mode"))
            .and_then(|value| value.as_str()),
        Some("fast")
    );
    assert_eq!(
        canonical
            .get("user_array")
            .and_then(Item::as_array)
            .map(|array| array.len()),
        Some(2)
    );
    assert!(rendered.contains("# fresh Codex metadata"), "{rendered}");
    assert!(rendered.contains("# keep inline comment"), "{rendered}");
    assert!(rendered.contains("# keep array comment"), "{rendered}");
    assert!(!rendered.contains("# stale installer value"), "{rendered}");
}

#[test]
fn windows_atomic_replace_source_uses_supported_flags_and_partial_failure_states() {
    let source = include_str!("../src/migration.rs");

    assert!(
        !source.contains("REPLACEFILE_WRITE_THROUGH"),
        "ReplaceFileW documents REPLACEFILE_WRITE_THROUGH as unsupported"
    );
    for documented_error in [
        "ERROR_UNABLE_TO_REMOVE_REPLACED",
        "ERROR_UNABLE_TO_MOVE_REPLACEMENT",
        "ERROR_UNABLE_TO_MOVE_REPLACEMENT_2",
    ] {
        assert!(
            source.contains(documented_error),
            "Windows replacement must handle {documented_error} explicitly"
        );
    }
    assert!(
        !source.contains("let _ = fs::remove_file(&backup)"),
        "successful Windows replacement must not hide backup cleanup failures"
    );
    assert!(
        source.contains("retained recovery artifact"),
        "backup cleanup errors must identify the retained recovery artifact"
    );
}

#[test]
fn exact_issue_90_duplicate_migration_removes_alias_and_preserves_direct_canonical_setting() {
    let codex_home = temp_root("issue-90-preserves-settings");
    fs::write(
        codex_home.join("config.toml"),
        format!(
            "{}\n[plugins.\"unica@unica-local\"]\nenabled = true\nalias_user_setting = \"remove me\"\n",
            canonical_config_with_user_owned_settings()
        ),
    )
    .unwrap();
    let legacy_root = codex_home.join("marketplaces/unica-local");
    let alias_cache = codex_home.join("plugins/cache/unica-local");
    fs::create_dir_all(&legacy_root).unwrap();
    fs::create_dir_all(&alias_cache).unwrap();
    fs::write(alias_cache.join("marker"), "legacy alias cache").unwrap();

    let runner = NativeCodexMutationRunner::for_issue_90(codex_home.clone());
    let marketplaces = runner.marketplaces();
    let commands = runner.commands();
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(issue_90_duplicate_discovery(&codex_home), &codex_home).unwrap();

    engine.apply(plan, |_| Ok(())).unwrap();

    assert_canonical_marketplace_removals(&commands, &marketplaces);
    assert!(!alias_cache.exists());
    assert_alias_plugin_table_is_absent(&codex_home);
    assert_canonical_direct_user_setting(&codex_home);
}

#[test]
fn exact_issue_90_duplicate_migration_removes_alias_and_preserves_nested_canonical_setting() {
    let codex_home = temp_root("issue-90-preserves-nested-setting");
    fs::write(
        codex_home.join("config.toml"),
        format!(
            "{}\n[plugins.\"unica@unica-local\"]\nenabled = true\nalias_user_setting = \"remove me\"\n",
            canonical_config_with_user_owned_settings()
        ),
    )
    .unwrap();
    let legacy_root = codex_home.join("marketplaces/unica-local");
    let alias_cache = codex_home.join("plugins/cache/unica-local");
    fs::create_dir_all(&legacy_root).unwrap();
    fs::create_dir_all(&alias_cache).unwrap();
    fs::write(alias_cache.join("marker"), "legacy alias cache").unwrap();

    let runner = NativeCodexMutationRunner::for_issue_90(codex_home.clone());
    let marketplaces = runner.marketplaces();
    let commands = runner.commands();
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(issue_90_duplicate_discovery(&codex_home), &codex_home).unwrap();

    engine.apply(plan, |_| Ok(())).unwrap();

    assert_canonical_marketplace_removals(&commands, &marketplaces);
    assert!(!alias_cache.exists());
    assert_alias_plugin_table_is_absent(&codex_home);
    assert_canonical_nested_user_owned_server(&codex_home);
}

#[test]
#[should_panic(expected = "canonical Codex enable flag must stay true")]
fn setting_assertions_reject_a_disabled_canonical_plugin() {
    let codex_home = temp_root("disabled-canonical-setting");
    fs::write(
        codex_home.join("config.toml"),
        canonical_config_with_user_owned_settings().replace("enabled = true", "enabled = false"),
    )
    .unwrap();

    assert_canonical_direct_user_setting(&codex_home);
}

#[test]
fn previous_canonical_version_is_restored_when_installed_runtime_verification_fails() {
    let codex_home = previous_canonical_home("update-rollback");
    let original_config = fs::read(codex_home.join("config.toml")).unwrap();
    let runner = CanonicalUpdateRunner::new(codex_home.clone());
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(previous_canonical_discovery(&codex_home), &codex_home).unwrap();

    let error = engine
        .apply(plan, |installed_root| {
            assert_eq!(installed_root, current_plugin_root(&codex_home));
            Err(BootstrapError::new("injected installed runtime failure"))
        })
        .unwrap_err();

    assert!(error.to_string().contains("rolled back"), "{error}");
    assert_eq!(
        fs::read(codex_home.join("config.toml")).unwrap(),
        original_config
    );
    assert_eq!(
        fs::read_to_string(codex_home.join(".tmp/marketplaces/unica/marker")).unwrap(),
        "previous marketplace"
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("plugins/cache/unica/unica/0.7.2/marker")).unwrap(),
        "previous plugin"
    );
    assert!(!current_plugin_root(&codex_home).exists());
}

#[test]
fn each_canonical_update_command_failure_restores_previous_version() {
    for fail_on in 0..3 {
        let codex_home = previous_canonical_home(&format!("update-command-{fail_on}"));
        let original_config = fs::read(codex_home.join("config.toml")).unwrap();
        let runner = CanonicalUpdateRunner::with_failure(codex_home.clone(), fail_on);
        let engine = MigrationEngine::new(codex_home.clone(), runner);
        let plan =
            classify_discovery(previous_canonical_discovery(&codex_home), &codex_home).unwrap();

        let error = engine.apply(plan, |_| Ok(())).unwrap_err();

        assert!(
            error.to_string().contains("rolled back"),
            "stage {fail_on}: {error}"
        );
        assert_eq!(
            fs::read(codex_home.join("config.toml")).unwrap(),
            original_config
        );
        assert_eq!(
            fs::read_to_string(codex_home.join(".tmp/marketplaces/unica/marker")).unwrap(),
            "previous marketplace"
        );
        assert_eq!(
            fs::read_to_string(codex_home.join("plugins/cache/unica/unica/0.7.2/marker")).unwrap(),
            "previous plugin"
        );
        assert!(!current_plugin_root(&codex_home).exists());
    }
}

#[test]
fn each_mutation_failure_restores_exact_config_and_keeps_backup() {
    for fail_on in 0..4 {
        let codex_home = temp_root(&format!("rollback-{fail_on}"));
        let original = b"model = \"gpt-5\"\n# exact legacy config\n";
        fs::write(codex_home.join("config.toml"), original).unwrap();
        let runner = FailingMutationRunner::new(codex_home.clone(), fail_on);
        let engine = MigrationEngine::new(codex_home.clone(), runner);
        let legacy_root = codex_home.join("marketplaces/unica-local");
        fs::create_dir_all(&legacy_root).unwrap();
        fs::write(legacy_root.join("marker"), "exact legacy source").unwrap();
        let plan = classify_discovery(legacy_discovery(&codex_home), &codex_home).unwrap();

        let error = engine.apply(plan, |_| Ok(())).unwrap_err();

        assert!(error.to_string().contains("rolled back"), "{error}");
        assert_eq!(fs::read(codex_home.join("config.toml")).unwrap(), original);
        assert_eq!(
            fs::read_to_string(legacy_root.join("marker")).unwrap(),
            "exact legacy source"
        );
        let backups = fs::read_dir(codex_home.join("unica/migration-backups"))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].path().join("snapshot.json").is_file());
        let diagnostics = fs::read_to_string(backups[0].path().join("diagnostics.jsonl")).unwrap();
        assert!(diagnostics.contains("migration-failed"));
        assert!(diagnostics.contains("rollback-succeeded"));
        assert!(!diagnostics.contains("token=secret"));
    }
}

#[test]
fn inverse_cli_failure_is_reported_even_when_discovery_matches_restored_config() {
    let codex_home = temp_root("inverse-failure");
    let original = b"model = \"gpt-5\"\n# exact legacy config\n";
    fs::write(codex_home.join("config.toml"), original).unwrap();
    let legacy_root = codex_home.join("marketplaces/unica-local");
    fs::create_dir_all(&legacy_root).unwrap();
    fs::write(legacy_root.join("marker"), "exact legacy source").unwrap();
    let runner = FailingMutationRunner::with_inverse_failure(codex_home.clone(), 1);
    let engine = MigrationEngine::new(codex_home.clone(), runner);
    let plan = classify_discovery(legacy_discovery(&codex_home), &codex_home).unwrap();

    let error = engine.apply(plan, |_| Ok(())).unwrap_err();

    assert!(
        error.to_string().contains("rollback also failed"),
        "{error}"
    );
    assert!(
        error.to_string().contains("injected inverse failure"),
        "{error}"
    );
    assert_eq!(fs::read(codex_home.join("config.toml")).unwrap(), original);
    assert_eq!(
        fs::read_to_string(legacy_root.join("marker")).unwrap(),
        "exact legacy source"
    );
}

fn legacy_discovery(codex_home: &Path) -> CodexDiscovery {
    CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![marketplace(
                "unica-local",
                "local",
                &codex_home
                    .join("marketplaces/unica-local")
                    .to_string_lossy(),
            )],
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
            installed: vec![canonical_plugin(env!("CARGO_PKG_VERSION"))],
            available: vec![],
            ..Default::default()
        },
    }
}

fn previous_canonical_discovery(codex_home: &Path) -> CodexDiscovery {
    let mut discovery = canonical_discovery();
    discovery.marketplaces.marketplaces[0].root = Some(
        codex_home
            .join(".tmp/marketplaces/unica")
            .to_string_lossy()
            .into_owned(),
    );
    discovery.plugins.installed = vec![canonical_plugin("0.7.2")];
    discovery
}

fn issue_90_duplicate_discovery(codex_home: &Path) -> CodexDiscovery {
    let source = codex_home.join("marketplaces/unica-local");
    CodexDiscovery {
        marketplaces: MarketplaceList {
            marketplaces: vec![
                marketplace("unica", "local", &source.to_string_lossy()),
                marketplace("unica-local", "local", &source.to_string_lossy()),
            ],
            ..Default::default()
        },
        plugins: PluginList {
            installed: vec![
                plugin("unica@unica", "unica", true),
                plugin("unica@unica-local", "unica-local", true),
            ],
            ..Default::default()
        },
    }
}

fn current_plugin_root(codex_home: &Path) -> PathBuf {
    codex_home
        .join("plugins/cache/unica/unica")
        .join(env!("CARGO_PKG_VERSION"))
}

fn previous_canonical_home(name: &str) -> PathBuf {
    let codex_home = temp_root(name);
    let marketplace = codex_home.join(".tmp/marketplaces/unica");
    let plugin = codex_home.join("plugins/cache/unica/unica/0.7.2");
    fs::create_dir_all(&marketplace).unwrap();
    fs::create_dir_all(&plugin).unwrap();
    fs::write(marketplace.join("marker"), "previous marketplace").unwrap();
    fs::write(plugin.join("marker"), "previous plugin").unwrap();
    fs::write(codex_home.join("config.toml"), canonical_config()).unwrap();
    codex_home
}

fn canonical_config() -> &'static str {
    "[plugins.\"unica@unica\"]\nenabled = true\n"
}

fn canonical_config_with_user_owned_settings() -> &'static str {
    "model = \"gpt-5\"\n\
     [plugins.\"unica@unica\"]\n\
     enabled = true\n\
     direct_user_setting = \"keep me\"\n\
     \n\
     [plugins.\"unica@unica\".mcp_servers.user_owned]\n\
     command = \"user-owned-command\"\n\
     args = [\"--persist\"]\n"
}

fn parsed_config(codex_home: &Path) -> DocumentMut {
    fs::read_to_string(codex_home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap()
}

fn plugin_table<'a>(config: &'a DocumentMut, plugin_id: &str) -> &'a dyn TableLike {
    config["plugins"]
        .as_table_like()
        .and_then(|plugins| plugins.get(plugin_id))
        .and_then(Item::as_table_like)
        .unwrap_or_else(|| panic!("missing [plugins.\"{plugin_id}\"] table in {config}"))
}

fn canonical_plugin_table(config: &DocumentMut) -> &dyn TableLike {
    let canonical = plugin_table(config, "unica@unica");
    assert_eq!(
        canonical.get("enabled").and_then(Item::as_bool),
        Some(true),
        "canonical Codex enable flag must stay true at plugins.\"unica@unica\".enabled"
    );
    canonical
}

fn assert_canonical_direct_user_setting(codex_home: &Path) {
    let config = parsed_config(codex_home);
    let canonical = canonical_plugin_table(&config);

    assert_eq!(
        canonical.get("direct_user_setting").and_then(Item::as_str),
        Some("keep me"),
        "direct setting must remain directly owned by [plugins.\"unica@unica\"]"
    );
}

fn assert_canonical_nested_user_owned_server(codex_home: &Path) {
    let config = parsed_config(codex_home);
    let canonical = canonical_plugin_table(&config);
    let mcp_servers = canonical
        .get("mcp_servers")
        .and_then(Item::as_table_like)
        .expect("missing [plugins.\"unica@unica\".mcp_servers] table");
    let user_owned = mcp_servers
        .get("user_owned")
        .and_then(Item::as_table_like)
        .expect("missing [plugins.\"unica@unica\".mcp_servers.user_owned] table");

    assert_eq!(
        user_owned.get("command").and_then(Item::as_str),
        Some("user-owned-command"),
        "nested command must remain owned by [plugins.\"unica@unica\".mcp_servers.user_owned]"
    );
    assert_eq!(
        user_owned
            .get("args")
            .and_then(Item::as_array)
            .and_then(|args| args.get(0))
            .and_then(|arg| arg.as_str()),
        Some("--persist"),
        "nested args must remain owned by [plugins.\"unica@unica\".mcp_servers.user_owned]"
    );
}

fn assert_alias_plugin_table_is_absent(codex_home: &Path) {
    let config = parsed_config(codex_home);
    let plugins = config["plugins"]
        .as_table_like()
        .expect("missing [plugins] table");

    assert!(
        plugins.get("unica@unica-local").is_none(),
        "legacy alias must not remain below [plugins]"
    );
}

fn assert_canonical_marketplace_removals(
    commands: &Arc<Mutex<Vec<CommandSpec>>>,
    marketplaces: &Arc<Mutex<BTreeSet<String>>>,
) {
    let commands = commands.lock().unwrap();
    for marketplace in ["unica", "unica-local"] {
        assert!(
            commands.iter().any(|command| {
                command.args
                    == [
                        "plugin".to_string(),
                        "marketplace".to_string(),
                        "remove".to_string(),
                        marketplace.to_string(),
                        "--json".to_string(),
                    ]
            }),
            "migration did not invoke Codex marketplace removal for {marketplace}"
        );
    }
    drop(commands);
    assert_eq!(
        marketplaces
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["unica".to_string()],
        "the runner's live marketplace registry must reflect those removals"
    );
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
    write_current_plugin_package(&codex_home);
    codex_home
}

fn write_current_plugin_package(codex_home: &Path) {
    let root = codex_home
        .join("plugins/cache/unica/unica")
        .join(env!("CARGO_PKG_VERSION"));
    fs::create_dir_all(root.join(".codex-plugin")).unwrap();
    fs::write(root.join(".codex-plugin/plugin.json"), b"{}").unwrap();
    fs::write(root.join("runtime-manifest.json"), b"{}").unwrap();
}

struct OrphanCleanupRunner {
    codex_home: PathBuf,
    fail_final_plugin_discovery: bool,
    mismatch_rollback_marketplace_root: bool,
    marketplace_discoveries: Mutex<usize>,
    plugin_discoveries: Mutex<usize>,
}

impl OrphanCleanupRunner {
    fn new(codex_home: PathBuf, fail_final_plugin_discovery: bool) -> Self {
        Self {
            codex_home,
            fail_final_plugin_discovery,
            mismatch_rollback_marketplace_root: false,
            marketplace_discoveries: Mutex::new(0),
            plugin_discoveries: Mutex::new(0),
        }
    }

    fn with_root_mismatch(codex_home: PathBuf) -> Self {
        Self {
            codex_home,
            fail_final_plugin_discovery: false,
            mismatch_rollback_marketplace_root: true,
            marketplace_discoveries: Mutex::new(0),
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
            let mut discoveries = self.marketplace_discoveries.lock().unwrap();
            *discoveries += 1;
            let mut marketplaces = canonical_discovery().marketplaces;
            if self.mismatch_rollback_marketplace_root && *discoveries >= 2 {
                marketplaces.marketplaces[0].root = Some("/cache/unica-other".to_string());
            }
            return Ok(serde_json::to_string(&marketplaces).unwrap());
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

struct CanonicalUpdateRunner {
    codex_home: PathBuf,
    failure: Mutex<UpdateFailureState>,
}

struct UpdateFailureState {
    fail_on: usize,
    mutation_index: usize,
    failed: bool,
}

impl CanonicalUpdateRunner {
    fn new(codex_home: PathBuf) -> Self {
        Self::with_failure(codex_home, usize::MAX)
    }

    fn with_failure(codex_home: PathBuf, fail_on: usize) -> Self {
        Self {
            codex_home,
            failure: Mutex::new(UpdateFailureState {
                fail_on,
                mutation_index: 0,
                failed: false,
            }),
        }
    }

    fn finish_mutation(&self) -> Result<String> {
        let mut state = self.failure.lock().unwrap();
        let mutation_index = state.mutation_index;
        state.mutation_index += 1;
        if !state.failed && mutation_index == state.fail_on {
            state.failed = true;
            return Err(BootstrapError::new("injected canonical update failure"));
        }
        Ok("{}".to_string())
    }

    fn discovery(&self) -> CodexDiscovery {
        let mut discovery = previous_canonical_discovery(&self.codex_home);
        if current_plugin_root(&self.codex_home).is_dir() {
            discovery.plugins.installed = vec![canonical_plugin(env!("CARGO_PKG_VERSION"))];
        } else if !self
            .codex_home
            .join("plugins/cache/unica/unica/0.7.2")
            .is_dir()
        {
            discovery.plugins.installed.clear();
        }
        discovery
    }
}

impl CommandRunner for CanonicalUpdateRunner {
    fn run(&self, command: &CommandSpec) -> Result<String> {
        if command.program == "git" {
            return Ok(String::new());
        }
        if command
            .args
            .ends_with(&["marketplace".into(), "list".into(), "--json".into()])
        {
            return Ok(serde_json::to_string(&self.discovery().marketplaces).unwrap());
        }
        if command
            .args
            .ends_with(&["list".into(), "--available".into(), "--json".into()])
        {
            return Ok(serde_json::to_string(&self.discovery().plugins).unwrap());
        }
        if command.args
            == [
                "plugin".to_string(),
                "remove".to_string(),
                "unica@unica".to_string(),
                "--json".to_string(),
            ]
        {
            let cache = self.codex_home.join("plugins/cache/unica/unica");
            if cache.exists() {
                fs::remove_dir_all(cache).unwrap();
            }
            return self.finish_mutation();
        }
        if command.args
            == [
                "plugin".to_string(),
                "marketplace".to_string(),
                "upgrade".to_string(),
                "unica".to_string(),
                "--json".to_string(),
            ]
        {
            let marketplace = self.codex_home.join(".tmp/marketplaces/unica");
            if marketplace.exists() {
                fs::remove_dir_all(&marketplace).unwrap();
            }
            fs::create_dir_all(&marketplace).unwrap();
            fs::write(marketplace.join("marker"), "current marketplace").unwrap();
            return self.finish_mutation();
        }
        if command.args
            == [
                "plugin".to_string(),
                "add".to_string(),
                "unica@unica".to_string(),
                "--json".to_string(),
            ]
        {
            let root = current_plugin_root(&self.codex_home);
            fs::create_dir_all(root.join(".codex-plugin")).unwrap();
            fs::write(root.join(".codex-plugin/plugin.json"), b"{}").unwrap();
            fs::write(root.join("runtime-manifest.json"), b"{}").unwrap();
            return self.finish_mutation();
        }
        Err(BootstrapError::new(format!(
            "unexpected update command: {} {:?}",
            command.program, command.args
        )))
    }
}

struct NativeCodexMutationRunner {
    codex_home: PathBuf,
    marketplaces: Arc<Mutex<BTreeSet<String>>>,
    commands: Arc<Mutex<Vec<CommandSpec>>>,
}

impl NativeCodexMutationRunner {
    fn new(codex_home: PathBuf) -> Self {
        Self::with_marketplaces(codex_home, ["unica"])
    }

    fn for_issue_90(codex_home: PathBuf) -> Self {
        Self::with_marketplaces(codex_home, ["unica", "unica-local"])
    }

    fn with_marketplaces<const N: usize>(codex_home: PathBuf, marketplaces: [&str; N]) -> Self {
        Self {
            codex_home,
            marketplaces: Arc::new(Mutex::new(
                marketplaces.into_iter().map(str::to_string).collect(),
            )),
            commands: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn marketplaces(&self) -> Arc<Mutex<BTreeSet<String>>> {
        Arc::clone(&self.marketplaces)
    }

    fn commands(&self) -> Arc<Mutex<Vec<CommandSpec>>> {
        Arc::clone(&self.commands)
    }

    fn marketplace_discovery(&self) -> MarketplaceList {
        let marketplaces = self.marketplaces.lock().unwrap();
        MarketplaceList {
            marketplaces: marketplaces
                .iter()
                .map(|name| {
                    if name == "unica" {
                        canonical_discovery().marketplaces.marketplaces.remove(0)
                    } else {
                        marketplace(
                            name,
                            "local",
                            &self
                                .codex_home
                                .join("marketplaces")
                                .join(name)
                                .to_string_lossy(),
                        )
                    }
                })
                .collect(),
            ..Default::default()
        }
    }

    fn remove_plugin_config_table(&self, plugin_id: &str) {
        let config_path = self.codex_home.join("config.toml");
        let mut config: DocumentMut = fs::read_to_string(&config_path).unwrap().parse().unwrap();
        let plugins = config["plugins"]
            .as_table_like_mut()
            .expect("native Codex plugin removal requires [plugins]");
        assert!(
            plugins.remove(plugin_id).is_some(),
            "native Codex plugin removal requires [plugins.\"{plugin_id}\"]"
        );

        fs::write(config_path, config.to_string()).unwrap();
    }

    fn add_canonical_plugin_config(&self) {
        let config_path = self.codex_home.join("config.toml");
        let mut config: DocumentMut = fs::read_to_string(&config_path).unwrap().parse().unwrap();
        if !config.as_table().contains_key("plugins") {
            config["plugins"] = Item::Table(toml_edit::Table::new());
        }
        let plugins = config["plugins"]
            .as_table_like_mut()
            .expect("native Codex plugin installation requires [plugins]");
        let mut canonical = toml_edit::Table::new();
        canonical["enabled"] = toml_edit::value(true);
        canonical["fresh_metadata"] = toml_edit::value("generated-by-codex");
        canonical["fresh_metadata"]
            .as_value_mut()
            .unwrap()
            .decor_mut()
            .set_suffix(" # fresh Codex metadata");
        plugins.insert("unica@unica", Item::Table(canonical));

        fs::write(config_path, config.to_string()).unwrap();
    }
}

impl CommandRunner for NativeCodexMutationRunner {
    fn run(&self, command: &CommandSpec) -> Result<String> {
        self.commands.lock().unwrap().push(command.clone());
        if command.program == "git" {
            return Ok(String::new());
        }
        if command
            .args
            .ends_with(&["marketplace".into(), "list".into(), "--json".into()])
        {
            return Ok(serde_json::to_string(&self.marketplace_discovery()).unwrap());
        }
        if command
            .args
            .ends_with(&["list".into(), "--available".into(), "--json".into()])
        {
            return Ok(serde_json::to_string(&canonical_discovery().plugins).unwrap());
        }
        if command.args.len() == 4
            && command.args[0] == "plugin"
            && command.args[1] == "remove"
            && command.args[3] == "--json"
        {
            self.remove_plugin_config_table(&command.args[2]);
            if command.args[2] == "unica@unica" {
                let cache = self.codex_home.join("plugins/cache/unica/unica");
                if cache.exists() {
                    fs::remove_dir_all(cache).unwrap();
                }
            }
            return Ok("{}".to_string());
        }
        if command.args
            == [
                "plugin".to_string(),
                "marketplace".to_string(),
                "upgrade".to_string(),
                "unica".to_string(),
                "--json".to_string(),
            ]
        {
            return Ok("{}".to_string());
        }
        if command.args.len() == 7
            && command.args[0] == "plugin"
            && command.args[1] == "marketplace"
            && command.args[2] == "add"
            && command.args[6] == "--json"
        {
            self.marketplaces
                .lock()
                .unwrap()
                .insert("unica".to_string());
            return Ok("{}".to_string());
        }
        if command.args
            == [
                "plugin".to_string(),
                "add".to_string(),
                "unica@unica".to_string(),
                "--json".to_string(),
            ]
        {
            self.add_canonical_plugin_config();
            write_current_plugin_package(&self.codex_home);
            return Ok("{}".to_string());
        }
        if command.args.len() == 5
            && command.args[0] == "plugin"
            && command.args[1] == "marketplace"
            && command.args[2] == "remove"
            && command.args[4] == "--json"
        {
            let marketplace = &command.args[3];
            if !self.marketplaces.lock().unwrap().remove(marketplace) {
                return Err(BootstrapError::new(format!(
                    "native Codex marketplace {marketplace} was not registered"
                )));
            }
            return Ok("{}".to_string());
        }
        Err(BootstrapError::new(format!(
            "unexpected native Codex mutation command: {} {:?}",
            command.program, command.args
        )))
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
    fail_inverse: bool,
    inverse_failed: bool,
}

impl FailingMutationRunner {
    fn new(codex_home: PathBuf, fail_on: usize) -> Self {
        Self {
            codex_home,
            state: Mutex::new(RunnerState {
                fail_on,
                mutation_index: 0,
                failed: false,
                fail_inverse: false,
                inverse_failed: false,
            }),
        }
    }

    fn with_inverse_failure(codex_home: PathBuf, fail_on: usize) -> Self {
        Self {
            codex_home,
            state: Mutex::new(RunnerState {
                fail_on,
                mutation_index: 0,
                failed: false,
                fail_inverse: true,
                inverse_failed: false,
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
            let state = self.state.lock().unwrap();
            if state.failed {
                let legacy = legacy_discovery(&self.codex_home);
                if command.args.get(1).map(String::as_str) == Some("marketplace") {
                    return Ok(serde_json::to_string(&legacy.marketplaces).unwrap());
                }
                return Ok(serde_json::to_string(&legacy.plugins).unwrap());
            }
            if command.args.get(1).map(String::as_str) == Some("marketplace") {
                return Ok(serde_json::to_string(&canonical_discovery().marketplaces).unwrap());
            }
            return Ok(serde_json::to_string(&canonical_discovery().plugins).unwrap());
        }

        fs::write(self.codex_home.join("config.toml"), b"partially mutated\n").unwrap();
        let mut state = self.state.lock().unwrap();
        let is_marketplace_add = command.args.len() >= 3
            && command.args[0] == "plugin"
            && command.args[1] == "marketplace"
            && command.args[2] == "add";
        if state.failed && state.fail_inverse && !state.inverse_failed && is_marketplace_add {
            state.inverse_failed = true;
            return Err(BootstrapError::new("injected inverse failure"));
        }
        if !state.failed && state.mutation_index == state.fail_on {
            state.failed = true;
            let legacy_root = self.codex_home.join("marketplaces/unica-local");
            if legacy_root.exists() {
                fs::remove_dir_all(legacy_root).unwrap();
            }
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

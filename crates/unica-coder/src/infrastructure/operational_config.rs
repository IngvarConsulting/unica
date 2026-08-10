use crate::domain::operational_config::{
    OperationalConfig, OperationalConfigDiagnostic, OperationalConfigDiagnosticCode,
    OperationalConfigDiagnosticSource, OperationalConfigField, OperationalConfigLayer,
};
use crate::infrastructure::workspace_config::{
    parse_workspace_config_root, WorkspaceConfigRootError, WorkspaceConfigRootErrorKind,
};
use std::fs;
use std::path::Path;
use toml::{Table, Value};

const SHARED_CONFIG_FILENAME: &str = "unica.toml";
const LOCAL_CONFIG_FILENAME: &str = "unica.local.toml";
const OPERATIONAL_SECTIONS: &[&str] = &["code_intelligence", "code_diagnostics"];
const CODE_INTELLIGENCE_FIELDS: &[&str] = &[
    "search_total_timeout_seconds",
    "search_rlm_timeout_seconds",
    "search_git_grep_timeout_seconds",
    "provider_read_timeout_seconds",
];
const CODE_DIAGNOSTICS_FIELDS: &[&str] = &["analyze_timeout_seconds"];

pub(crate) fn load_operational_config(
    workspace_root: &Path,
) -> Result<OperationalConfig, OperationalConfigDiagnostic> {
    let shared = read_layer(
        &workspace_root.join(SHARED_CONFIG_FILENAME),
        OperationalConfigDiagnosticSource::Shared,
    )?;
    let local = read_layer(
        &workspace_root.join(LOCAL_CONFIG_FILENAME),
        OperationalConfigDiagnosticSource::Local,
    )?;

    OperationalConfig::from_layers(shared.as_ref(), local.as_ref())
}

fn read_layer(
    path: &Path,
    source: OperationalConfigDiagnosticSource,
) -> Result<Option<OperationalConfigLayer>, OperationalConfigDiagnostic> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::symlink_metadata(path) {
                Err(metadata_error) if metadata_error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(None);
                }
                Ok(_) | Err(_) => return Err(read_failed_diagnostic(source)),
            }
        }
        Err(_read_error) => return Err(read_failed_diagnostic(source)),
    };

    parse_layer(&contents, source).map(Some)
}

fn read_failed_diagnostic(
    source: OperationalConfigDiagnosticSource,
) -> OperationalConfigDiagnostic {
    OperationalConfigDiagnostic::new(OperationalConfigDiagnosticCode::ReadFailed, source, "$")
}

fn parse_layer(
    contents: &str,
    source: OperationalConfigDiagnosticSource,
) -> Result<OperationalConfigLayer, OperationalConfigDiagnostic> {
    let root = parse_workspace_config_root(contents)
        .map_err(|error| root_error_diagnostic(error, source))?;

    let mut layer = OperationalConfigLayer::default();
    let Some(operational_value) = root.get("operational") else {
        return Ok(layer);
    };
    let operational = require_table(operational_value, "operational", source)?;
    reject_unknown_fields(operational, OPERATIONAL_SECTIONS, "operational", source)?;

    if let Some(code_intelligence_value) = operational.get("code_intelligence") {
        let code_intelligence = require_table(
            code_intelligence_value,
            "operational.code_intelligence",
            source,
        )?;
        reject_unknown_fields(
            code_intelligence,
            CODE_INTELLIGENCE_FIELDS,
            "operational.code_intelligence",
            source,
        )?;
        parse_timeout(
            code_intelligence,
            "search_total_timeout_seconds",
            OperationalConfigField::SearchTotal,
            source,
            &mut layer,
        )?;
        parse_timeout(
            code_intelligence,
            "search_rlm_timeout_seconds",
            OperationalConfigField::SearchRlm,
            source,
            &mut layer,
        )?;
        parse_timeout(
            code_intelligence,
            "search_git_grep_timeout_seconds",
            OperationalConfigField::SearchGitGrep,
            source,
            &mut layer,
        )?;
        parse_timeout(
            code_intelligence,
            "provider_read_timeout_seconds",
            OperationalConfigField::ProviderRead,
            source,
            &mut layer,
        )?;
    }

    if let Some(code_diagnostics_value) = operational.get("code_diagnostics") {
        let code_diagnostics = require_table(
            code_diagnostics_value,
            "operational.code_diagnostics",
            source,
        )?;
        reject_unknown_fields(
            code_diagnostics,
            CODE_DIAGNOSTICS_FIELDS,
            "operational.code_diagnostics",
            source,
        )?;
        parse_timeout(
            code_diagnostics,
            "analyze_timeout_seconds",
            OperationalConfigField::DiagnosticsAnalyze,
            source,
            &mut layer,
        )?;
    }

    Ok(layer)
}

fn root_error_diagnostic(
    error: WorkspaceConfigRootError,
    source: OperationalConfigDiagnosticSource,
) -> OperationalConfigDiagnostic {
    let code = match error.kind() {
        WorkspaceConfigRootErrorKind::InvalidToml => OperationalConfigDiagnosticCode::InvalidToml,
        WorkspaceConfigRootErrorKind::MissingVersion => {
            OperationalConfigDiagnosticCode::MissingField
        }
        WorkspaceConfigRootErrorKind::InvalidVersionType => {
            OperationalConfigDiagnosticCode::InvalidType
        }
        WorkspaceConfigRootErrorKind::UnsupportedVersion => {
            OperationalConfigDiagnosticCode::UnsupportedVersion
        }
        WorkspaceConfigRootErrorKind::UnknownField => OperationalConfigDiagnosticCode::UnknownField,
    };
    OperationalConfigDiagnostic::new(code, source, error.field_path())
}

fn require_table<'a>(
    value: &'a Value,
    field_path: &str,
    source: OperationalConfigDiagnosticSource,
) -> Result<&'a Table, OperationalConfigDiagnostic> {
    value.as_table().ok_or_else(|| {
        OperationalConfigDiagnostic::new(
            OperationalConfigDiagnosticCode::InvalidType,
            source,
            field_path,
        )
    })
}

fn reject_unknown_fields(
    table: &Table,
    allowed: &[&str],
    parent_path: &str,
    source: OperationalConfigDiagnosticSource,
) -> Result<(), OperationalConfigDiagnostic> {
    let unknown = table.keys().find(|key| !allowed.contains(&key.as_str()));
    let Some(unknown) = unknown else {
        return Ok(());
    };
    let field_path = if parent_path.is_empty() {
        unknown.to_string()
    } else {
        format!("{parent_path}.{unknown}")
    };
    Err(OperationalConfigDiagnostic::new(
        OperationalConfigDiagnosticCode::UnknownField,
        source,
        field_path,
    ))
}

fn parse_timeout(
    table: &Table,
    key: &str,
    field: OperationalConfigField,
    source: OperationalConfigDiagnosticSource,
    layer: &mut OperationalConfigLayer,
) -> Result<(), OperationalConfigDiagnostic> {
    let Some(value) = table.get(key) else {
        return Ok(());
    };
    let seconds = value.as_integer().ok_or_else(|| {
        OperationalConfigDiagnostic::new(
            OperationalConfigDiagnosticCode::InvalidType,
            source,
            field.path(),
        )
    })?;
    layer.set_timeout_seconds(field, seconds, source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use std::time::Duration;
    use tempfile::{tempdir, TempDir};

    fn write_config(workspace: &TempDir, filename: &str, contents: &str) {
        fs::write(workspace.path().join(filename), contents).expect("write config fixture");
    }

    fn assert_diagnostic(
        contents: &str,
        expected_code: OperationalConfigDiagnosticCode,
        expected_path: &str,
    ) {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(&workspace, SHARED_CONFIG_FILENAME, contents);

        let diagnostic =
            load_operational_config(workspace.path()).expect_err("invalid shared config must fail");
        assert_eq!(diagnostic.code(), expected_code);
        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::Shared
        );
        assert_eq!(diagnostic.field_path(), expected_path);
    }

    #[test]
    fn missing_files_use_exact_compiled_defaults() {
        let workspace = tempdir().expect("workspace tempdir");

        let config = load_operational_config(workspace.path()).expect("load defaults");
        let code = config.code_intelligence();

        assert_eq!(code.search_total_timeout(), Duration::from_secs(120));
        assert_eq!(code.search_rlm_timeout(), Duration::from_secs(45));
        assert_eq!(code.search_git_grep_timeout(), Duration::from_secs(15));
        assert_eq!(code.provider_read_timeout(), Duration::from_secs(45));
        assert_eq!(
            config.code_diagnostics().analyze_timeout(),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn network_only_layer_without_version_keeps_operational_defaults() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            "[network]\ndefault = \"deny\"\n",
        );

        let config = load_operational_config(workspace.path())
            .expect("network-only legacy layer must not change operational defaults");

        assert_eq!(
            config.code_intelligence().search_total_timeout(),
            Duration::from_secs(120)
        );
        assert_eq!(
            config.code_diagnostics().analyze_timeout(),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn operational_consumer_rejects_unknown_root_field_after_shared_root_validation() {
        assert_diagnostic(
            "version = 1\nunknown_root = true\n",
            OperationalConfigDiagnosticCode::UnknownField,
            "unknown_root",
        );
    }

    #[test]
    fn operational_consumer_keeps_operational_subtree_errors_local() {
        assert_diagnostic(
            "version = 1\n[network]\ndefault = \"deny\"\n\n[operational.unsupported]\ntimeout = 10\n",
            OperationalConfigDiagnosticCode::UnknownField,
            "operational.unsupported",
        );
    }

    #[test]
    fn dangling_config_link_fails_closed_as_an_unreadable_present_layer() {
        let workspace = tempdir().expect("workspace tempdir");
        let outcome = create_file_link_fixture_for_test(
            workspace.path().join("missing-config-target.toml"),
            workspace.path().join(SHARED_CONFIG_FILENAME),
        )
        .expect("create dangling config link fixture");
        if outcome != FileLinkFixtureOutcome::Created {
            return;
        }

        let diagnostic = load_operational_config(workspace.path())
            .expect_err("a present but unreadable config layer must fail closed");

        assert_eq!(
            diagnostic.code(),
            OperationalConfigDiagnosticCode::ReadFailed
        );
        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::Shared
        );
        assert_eq!(diagnostic.field_path(), "$");
    }

    #[test]
    fn shared_config_overrides_every_compiled_deadline() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            r#"version = 1

[operational.code_intelligence]
search_total_timeout_seconds = 100
search_rlm_timeout_seconds = 40
search_git_grep_timeout_seconds = 30
provider_read_timeout_seconds = 35

[operational.code_diagnostics]
analyze_timeout_seconds = 900
"#,
        );

        let config = load_operational_config(workspace.path()).expect("load shared config");
        let code = config.code_intelligence();

        assert_eq!(code.search_total_timeout(), Duration::from_secs(100));
        assert_eq!(code.search_rlm_timeout(), Duration::from_secs(40));
        assert_eq!(code.search_git_grep_timeout(), Duration::from_secs(30));
        assert_eq!(code.provider_read_timeout(), Duration::from_secs(35));
        assert_eq!(
            config.code_diagnostics().analyze_timeout(),
            Duration::from_secs(900)
        );
    }

    #[test]
    fn local_config_overrides_fields_and_inherits_the_rest() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            r#"version = 1
[operational.code_intelligence]
search_total_timeout_seconds = 100
search_rlm_timeout_seconds = 40
provider_read_timeout_seconds = 30
[operational.code_diagnostics]
analyze_timeout_seconds = 600
"#,
        );
        write_config(
            &workspace,
            LOCAL_CONFIG_FILENAME,
            r#"version = 1
[operational.code_intelligence]
search_rlm_timeout_seconds = 35
[operational.code_diagnostics]
analyze_timeout_seconds = 700
"#,
        );

        let config = load_operational_config(workspace.path()).expect("load layered config");
        let code = config.code_intelligence();

        assert_eq!(code.search_total_timeout(), Duration::from_secs(100));
        assert_eq!(code.search_rlm_timeout(), Duration::from_secs(35));
        assert_eq!(code.search_git_grep_timeout(), Duration::from_secs(15));
        assert_eq!(code.provider_read_timeout(), Duration::from_secs(30));
        assert_eq!(
            config.code_diagnostics().analyze_timeout(),
            Duration::from_secs(700)
        );
    }

    #[test]
    fn local_config_is_valid_without_shared_config() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            &workspace,
            LOCAL_CONFIG_FILENAME,
            r#"version = 1
[operational.code_intelligence]
search_total_timeout_seconds = 90
"#,
        );

        let config = load_operational_config(workspace.path()).expect("load local-only config");

        assert_eq!(
            config.code_intelligence().search_total_timeout(),
            Duration::from_secs(90)
        );
        assert_eq!(
            config.code_intelligence().search_rlm_timeout(),
            Duration::from_secs(45)
        );
    }

    #[test]
    fn exact_lower_and_upper_boundaries_are_accepted_without_clamping() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            r#"version = 1
[operational.code_intelligence]
search_total_timeout_seconds = 120
search_rlm_timeout_seconds = 45
search_git_grep_timeout_seconds = 120
provider_read_timeout_seconds = 45
[operational.code_diagnostics]
analyze_timeout_seconds = 3600
"#,
        );

        let maximum = load_operational_config(workspace.path()).expect("load maximum boundaries");
        assert_eq!(
            maximum.code_intelligence().search_total_timeout(),
            Duration::from_secs(120)
        );
        assert_eq!(
            maximum.code_intelligence().search_rlm_timeout(),
            Duration::from_secs(45)
        );
        assert_eq!(
            maximum.code_intelligence().search_git_grep_timeout(),
            Duration::from_secs(120)
        );
        assert_eq!(
            maximum.code_intelligence().provider_read_timeout(),
            Duration::from_secs(45)
        );
        assert_eq!(
            maximum.code_diagnostics().analyze_timeout(),
            Duration::from_secs(3_600)
        );

        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            r#"version = 1
[operational.code_intelligence]
search_total_timeout_seconds = 1
search_rlm_timeout_seconds = 1
search_git_grep_timeout_seconds = 1
provider_read_timeout_seconds = 1
[operational.code_diagnostics]
analyze_timeout_seconds = 30
"#,
        );
        let minimum = load_operational_config(workspace.path()).expect("load minimum boundaries");
        assert_eq!(
            minimum.code_intelligence().search_total_timeout(),
            Duration::from_secs(1)
        );
        assert_eq!(
            minimum.code_diagnostics().analyze_timeout(),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn separate_workspaces_do_not_share_a_process_global_snapshot() {
        let first = tempdir().expect("first workspace tempdir");
        let second = tempdir().expect("second workspace tempdir");
        write_config(
            &first,
            SHARED_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
        );
        write_config(
            &second,
            SHARED_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 80\n",
        );

        let first_config = load_operational_config(first.path()).expect("load first workspace");
        let second_config = load_operational_config(second.path()).expect("load second workspace");

        assert_eq!(
            first_config.code_intelligence().search_total_timeout(),
            Duration::from_secs(90)
        );
        assert_eq!(
            second_config.code_intelligence().search_total_timeout(),
            Duration::from_secs(80)
        );
    }

    #[test]
    fn a_changed_file_is_observed_by_the_next_load_only() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
        );
        let first = load_operational_config(workspace.path()).expect("load first snapshot");

        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 80\n",
        );
        let second = load_operational_config(workspace.path()).expect("load second snapshot");

        assert_eq!(
            first.code_intelligence().search_total_timeout(),
            Duration::from_secs(90)
        );
        assert_eq!(
            second.code_intelligence().search_total_timeout(),
            Duration::from_secs(80)
        );
    }

    #[test]
    fn version_and_structure_errors_are_typed_with_exact_paths() {
        let cases = [
            (
                "[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
                OperationalConfigDiagnosticCode::MissingField,
                "version",
            ),
            (
                "version = \"one\"\n",
                OperationalConfigDiagnosticCode::InvalidType,
                "version",
            ),
            (
                "version = 2\n",
                OperationalConfigDiagnosticCode::UnsupportedVersion,
                "version",
            ),
            (
                "version = 1\nunsupported = true\n",
                OperationalConfigDiagnosticCode::UnknownField,
                "unsupported",
            ),
            (
                "version = 1\noperational = 1\n",
                OperationalConfigDiagnosticCode::InvalidType,
                "operational",
            ),
            (
                "version = 1\n[operational.network]\ntimeout = 10\n",
                OperationalConfigDiagnosticCode::UnknownField,
                "operational.network",
            ),
            (
                "version = 1\n[operational]\ncode_intelligence = 10\n",
                OperationalConfigDiagnosticCode::InvalidType,
                "operational.code_intelligence",
            ),
            (
                "version = 1\n[operational.code_intelligence]\nunknown_timeout_seconds = 10\n",
                OperationalConfigDiagnosticCode::UnknownField,
                "operational.code_intelligence.unknown_timeout_seconds",
            ),
            (
                "version = 1\n[operational.code_diagnostics]\nunknown_timeout_seconds = 10\n",
                OperationalConfigDiagnosticCode::UnknownField,
                "operational.code_diagnostics.unknown_timeout_seconds",
            ),
        ];

        for (contents, code, path) in cases {
            assert_diagnostic(contents, code, path);
        }
    }

    #[test]
    fn all_timeout_fields_reject_invalid_types_and_ranges() {
        let cases = [
            (
                "search_total_timeout_seconds",
                "0",
                "operational.code_intelligence.search_total_timeout_seconds",
                "code_intelligence",
            ),
            (
                "search_total_timeout_seconds",
                "121",
                "operational.code_intelligence.search_total_timeout_seconds",
                "code_intelligence",
            ),
            (
                "search_rlm_timeout_seconds",
                "46",
                "operational.code_intelligence.search_rlm_timeout_seconds",
                "code_intelligence",
            ),
            (
                "search_git_grep_timeout_seconds",
                "-1",
                "operational.code_intelligence.search_git_grep_timeout_seconds",
                "code_intelligence",
            ),
            (
                "search_git_grep_timeout_seconds",
                "121",
                "operational.code_intelligence.search_git_grep_timeout_seconds",
                "code_intelligence",
            ),
            (
                "provider_read_timeout_seconds",
                "46",
                "operational.code_intelligence.provider_read_timeout_seconds",
                "code_intelligence",
            ),
            (
                "analyze_timeout_seconds",
                "29",
                "operational.code_diagnostics.analyze_timeout_seconds",
                "code_diagnostics",
            ),
            (
                "analyze_timeout_seconds",
                "3601",
                "operational.code_diagnostics.analyze_timeout_seconds",
                "code_diagnostics",
            ),
        ];

        for (field, value, path, section) in cases {
            let contents = format!("version = 1\n[operational.{section}]\n{field} = {value}\n");
            assert_diagnostic(&contents, OperationalConfigDiagnosticCode::OutOfRange, path);
        }

        assert_diagnostic(
            "version = 1\n[operational.code_intelligence]\nprovider_read_timeout_seconds = \"45\"\n",
            OperationalConfigDiagnosticCode::InvalidType,
            "operational.code_intelligence.provider_read_timeout_seconds",
        );
    }

    #[test]
    fn syntax_errors_report_only_the_root_path() {
        assert_diagnostic(
            "version = 1\nsecret-value =\n",
            OperationalConfigDiagnosticCode::InvalidToml,
            "$",
        );
    }

    #[test]
    fn invalid_shared_layer_is_not_masked_by_a_local_override() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 121\n",
        );
        write_config(
            &workspace,
            LOCAL_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
        );

        let diagnostic = load_operational_config(workspace.path())
            .expect_err("invalid lower layer must fail before merge");

        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::Shared
        );
        assert_eq!(
            diagnostic.code(),
            OperationalConfigDiagnosticCode::OutOfRange
        );
    }

    #[test]
    fn invalid_local_overlay_is_not_ignored() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(&workspace, SHARED_CONFIG_FILENAME, "version = 1\n");
        write_config(
            &workspace,
            LOCAL_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nprovider_read_timeout_seconds = 46\n",
        );

        let diagnostic =
            load_operational_config(workspace.path()).expect_err("invalid local overlay must fail");

        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::Local
        );
        assert_eq!(
            diagnostic.field_path(),
            "operational.code_intelligence.provider_read_timeout_seconds"
        );
    }

    #[test]
    fn every_present_local_file_requires_version_one() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(&workspace, SHARED_CONFIG_FILENAME, "version = 1\n");
        write_config(
            &workspace,
            LOCAL_CONFIG_FILENAME,
            "[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
        );

        let diagnostic = load_operational_config(workspace.path())
            .expect_err("present local file without a version must fail");

        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::Local
        );
        assert_eq!(
            diagnostic.code(),
            OperationalConfigDiagnosticCode::MissingField
        );
        assert_eq!(diagnostic.field_path(), "version");
    }

    #[test]
    fn final_snapshot_enforces_provider_deadlines_not_exceeding_total() {
        let cases = [
            (
                "search_rlm_timeout_seconds = 30",
                "operational.code_intelligence.search_rlm_timeout_seconds",
            ),
            (
                "search_rlm_timeout_seconds = 20\nsearch_git_grep_timeout_seconds = 30",
                "operational.code_intelligence.search_git_grep_timeout_seconds",
            ),
        ];

        for (provider_line, expected_path) in cases {
            let workspace = tempdir().expect("workspace tempdir");
            write_config(
                &workspace,
                SHARED_CONFIG_FILENAME,
                &format!(
                    "version = 1\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 20\n{provider_line}\n"
                ),
            );

            let diagnostic = load_operational_config(workspace.path())
                .expect_err("provider deadline above total must fail");
            assert_eq!(
                diagnostic.code(),
                OperationalConfigDiagnosticCode::InconsistentValues
            );
            assert_eq!(diagnostic.field_path(), expected_path);
        }
    }

    #[test]
    fn cross_layer_constraint_is_attributed_to_the_later_override() {
        let workspace = tempdir().expect("workspace tempdir");
        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nsearch_rlm_timeout_seconds = 30\n",
        );
        write_config(
            &workspace,
            LOCAL_CONFIG_FILENAME,
            "version = 1\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 20\n",
        );

        let diagnostic = load_operational_config(workspace.path())
            .expect_err("local total creates a cross-layer conflict");

        assert_eq!(
            diagnostic.source(),
            OperationalConfigDiagnosticSource::Local
        );
        assert_eq!(
            diagnostic.field_path(),
            "operational.code_intelligence.search_total_timeout_seconds"
        );
    }

    #[test]
    fn diagnostics_never_expose_absolute_paths_raw_toml_or_values() {
        let workspace = tempdir().expect("workspace tempdir");
        let secret = "classified-timeout-value";
        write_config(
            &workspace,
            SHARED_CONFIG_FILENAME,
            &format!(
                "version = 1\n[operational.code_intelligence]\nprovider_read_timeout_seconds = \"{secret}\"\n"
            ),
        );

        let diagnostic =
            load_operational_config(workspace.path()).expect_err("string timeout must fail safely");
        let serialized = serde_json::to_string(&diagnostic).expect("serialize diagnostic");
        let displayed = diagnostic.to_string();
        let absolute_path = workspace.path().display().to_string();

        for rendered in [&serialized, &displayed] {
            assert!(!rendered.contains(secret));
            assert!(!rendered.contains(&absolute_path));
            assert!(!rendered.contains("provider_read_timeout_seconds ="));
        }
        assert!(serialized.contains("unica.toml"));
        assert!(serialized.contains("operational.code_intelligence.provider_read_timeout_seconds"));
    }

    #[test]
    fn read_errors_are_redacted_to_the_fixed_basename() {
        let workspace = tempdir().expect("workspace tempdir");
        fs::create_dir(workspace.path().join(SHARED_CONFIG_FILENAME))
            .expect("create unreadable-as-file config path");

        let diagnostic = load_operational_config(workspace.path())
            .expect_err("directory cannot be read as a TOML file");
        let rendered = diagnostic.to_string();

        assert_eq!(
            diagnostic.code(),
            OperationalConfigDiagnosticCode::ReadFailed
        );
        assert_eq!(diagnostic.field_path(), "$");
        assert!(rendered.contains(SHARED_CONFIG_FILENAME));
        assert!(!rendered.contains(&workspace.path().display().to_string()));
    }
}

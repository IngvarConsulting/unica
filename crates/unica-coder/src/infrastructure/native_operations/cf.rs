#![allow(dead_code, unused_imports)]
use unica_metadata::configuration::*;
use unica_metadata_designer::configuration::*;
use unica_source_designer::configuration_layout::*;

use crate::infrastructure::form_composition::*;
use crate::infrastructure::interface_composition::*;
use crate::infrastructure::subsystem_composition::*;
use crate::infrastructure::support_state::WorkspaceSupportStateReader;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_forms::facts::*;
use unica_forms_designer::codec::*;
use unica_metadata::interface::*;
use unica_metadata::subsystem::*;
use unica_metadata::support_state::{ConfigurationSupportData, SupportStateReader};
use unica_metadata::MetadataKind;
use unica_metadata_designer::interface::*;
use unica_metadata_designer::subsystem::*;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use unica_source_designer::layout::{
    metadata_kind, metadata_kind_by_directory, metadata_kind_index, METADATA_KIND_TAGS,
};
use unica_source_designer::roots::root_version_literal;

use super::common::*;
use super::{cfe::*, dcs::*, meta::*, role::*};
use unica_workspace::compile_transaction::CompileTransaction;
use unica_workspace::single_file_publisher::{publish, PublishEffect, PublishMode, PublishRequest};

pub(crate) fn validate_cf(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    validate_cf_with_scope(args, context, CfValidationScope::Full)
}

pub(crate) fn validate_cf_owner_path(
    path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let args = Map::from_iter([(
        "ConfigPath".to_string(),
        Value::String(path.display().to_string()),
    )]);
    let outcome = validate_cf_with_scope(&args, context, CfValidationScope::OwnerShape);
    if outcome.ok {
        Ok(())
    } else if outcome.errors.is_empty() {
        Err(outcome.summary)
    } else {
        Err(outcome.errors.join("; "))
    }
}

fn validate_cf_with_scope(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    scope: CfValidationScope,
) -> AdapterOutcome {
    let result = (|| {
        let resolved_path = resolve_cf_read_config_path(args, context)?;
        let reader = unica_workspace::discovery::ProjectDiscoveryReader::new(&context.cwd);
        unica_metadata_designer::configuration::validate_cf_source(
            &reader,
            args,
            resolved_path,
            &context.cwd,
            scope,
        )
    })();

    match result {
        Ok(run) => AdapterOutcome {
            ok: run.ok,
            summary: if run.ok {
                "unica.cf.validate completed with native configuration validator".to_string()
            } else {
                "unica.cf.validate failed in native configuration validator".to_string()
            },
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: run.errors,
            artifacts: vec![run.artifact.display().to_string()],
            stdout: Some(run.stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.cf.validate failed in native configuration validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: Some(format!("{error}\n")),
            stderr: Some(String::new()),
            command: None,
        },
    }
}

pub(crate) fn analyze_cf_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    support_reader: &dyn SupportStateReader,
) -> CfInfoExecution {
    let result = (|| -> Result<(CfInfoData, PathBuf), String> {
        let selection = resolve_cf_info_target(args, context)?;
        let config_path = selection.resource_path;
        let text = fs::read_to_string(&config_path)
            .map_err(|err| format!("failed to read {}: {err}", config_path.display()))?;
        let config_dir = config_path.parent().unwrap_or(context.cwd.as_path());
        let support = support_reader
            .configuration_support(&selection.target)
            .map_err(|error| error.to_string())?;
        let home_page = cf_read_home_page(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
            config_dir,
        )
        .map(|layout| CfHomePageData {
            template: layout.template,
            left: cf_home_page_item_data(&layout.left),
            right: cf_home_page_item_data(&layout.right),
        });
        // Тот же документ, что читает канонический корень: интерфейс
        // конфигурации — её свойство, и здесь он не должен расходиться.
        let interface = fs::read_to_string(config_dir.join("Ext/CommandInterface.xml"))
            .ok()
            .map(|text| parse_cf_command_interface_xml(&text))
            .transpose()?;
        let parsed = parse_cf_info_xml(&text, support, home_page, interface)?;
        Ok((parsed.data, config_path))
    })();

    match result {
        Ok((data, artifact)) => CfInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.cf.info described {} with {} object(s)",
                    data.name, data.total_objects
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![artifact.display().to_string()],
                stdout: None,
                stderr: Some(String::new()),
                command: None,
            },
            data: Some(data),
        },
        Err(error) => CfInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.cf.info failed in native configuration analyzer".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

pub(crate) struct CfInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<CfInfoData>,
}

#[cfg(test)]
mod metadata_kind_consumer_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn cf_edit_home_page_uses_active_format() {
        let root = std::env::temp_dir().join(format!(
            "unica-home-page-format-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let plan = cf_edit_set_home_page(&json!({"template": "OneColumn"}), &root).unwrap();
        let xml = String::from_utf8(plan.bytes).unwrap();
        let document = Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();

        assert_eq!(
            document.root_element().attribute("version"),
            Some(unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE.export_format)
        );
        let columns = document
            .root_element()
            .children()
            .filter(|node| {
                unica_xml_format::names::element_has_name(*node, "Column", Some(CF_HP_NS))
            })
            .collect::<Vec<_>>();
        assert_eq!(columns.len(), 1, "{xml}");
        assert!(
            !document.root_element().children().any(|node| {
                unica_xml_format::names::element_has_name(node, "LeftColumn", Some(CF_HP_NS))
                    || unica_xml_format::names::element_has_name(
                        node,
                        "RightColumn",
                        Some(CF_HP_NS),
                    )
            }),
            "{xml}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_home_page_maps_two_named_columns_for_roundtrip_reading() {
        let root = std::env::temp_dir().join(format!(
            "unica-home-page-columns-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let plan = cf_edit_set_home_page(
            &json!({
                "template": "TwoColumnsEqualWidth",
                "left": ["CommonForm.Left"],
                "right": ["CommonForm.Right"]
            }),
            &root,
        )
        .unwrap();
        let xml = String::from_utf8(plan.bytes.clone()).unwrap();
        let document = Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
        let columns = document
            .root_element()
            .children()
            .filter(|node| {
                unica_xml_format::names::element_has_name(*node, "Column", Some(CF_HP_NS))
            })
            .collect::<Vec<_>>();
        assert!(columns.is_empty(), "{xml}");
        assert!(document.root_element().children().any(|node| {
            unica_xml_format::names::element_has_name(node, "LeftColumn", Some(CF_HP_NS))
        }));
        assert!(document.root_element().children().any(|node| {
            unica_xml_format::names::element_has_name(node, "RightColumn", Some(CF_HP_NS))
        }));

        fs::create_dir_all(plan.path.parent().unwrap()).unwrap();
        fs::write(&plan.path, plan.bytes).unwrap();
        let layout = cf_read_home_page(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
            &root,
        )
        .unwrap();
        assert_eq!(layout.left.len(), 1, "{xml}");
        assert_eq!(layout.left[0].form, "CommonForm.Left");
        assert_eq!(layout.right.len(), 1, "{xml}");
        assert_eq!(layout.right[0].form, "CommonForm.Right");
        fs::remove_dir_all(root).unwrap();
    }
}

struct CfEditRun {
    data: CfEditData,
    config_path: PathBuf,
    artifacts: Vec<PathBuf>,
    config_updated: bool,
    warnings: Vec<String>,
}

pub(crate) struct CfEditExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<CfEditData>,
}

pub(crate) fn edit_cf(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    edit_cf_with_data(args, context).outcome
}

pub(crate) fn edit_cf_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> CfEditExecution {
    let edit_result = (|| -> Result<CfEditRun, String> {
        let definition_file =
            unica_operation_contract::input::path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation =
            unica_operation_contract::input::string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }

        let config_path = resolve_cf_edit_config_path(args, context)?;
        let config_dir = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| context.cwd.clone());
        let source_snapshot = read_utf8_sig_snapshot(&config_path)?;
        let text = unica_xml_format::text::lxml_parser_normalized_text(&source_snapshot.text);
        if !text.contains("<Configuration") {
            return Err("No <Configuration> element found".to_string());
        }
        let obj_name = cf_edit_config_name(&text)?;

        let mut transaction = CompileTransaction::new();
        let operations = cf_edit_operations_guarded(
            args,
            &context.cwd,
            operation,
            definition_file,
            &mut transaction,
        )?;
        cf_edit_validate_child_object_kinds(&operations)?;
        let format_dependencies =
            cf_edit_format_dependency_paths_for_operations(&config_path, &operations)?;
        let CfEditImagePlan {
            text,
            added: add_count,
            removed: remove_count,
            modified: modify_count,
            stdout,
            operations: operation_log,
            artifacts,
            external_files,
            config_changed,
        } = cf_edit_plan_image(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(&context.cwd),
            text,
            &config_path,
            &config_dir,
            operations,
            &obj_name,
            crate::infrastructure::source_ids::fresh_uuid,
        )?;
        let mut config_updated = false;
        for (path, bytes) in external_files {
            transaction.create_or_replace_bytes(path, bytes)?;
        }
        if config_changed {
            let replacement = unica_xml_format::text::utf8_bom_bytes(&cf_edit_serialized_text(
                &text,
                &source_snapshot.text,
            ));
            transaction.replace_bytes(&config_path, &source_snapshot.raw, replacement)?;
        } else {
            guard_exact_preimage_if_unprotected(
                &mut transaction,
                &config_path,
                &source_snapshot.raw,
            )?;
        }
        guard_active_format_dependencies(
            &mut transaction,
            &format_dependencies
                .iter()
                .map(PathBuf::as_path)
                .collect::<Vec<_>>(),
            context,
        )?;
        let show_validation_output =
            !unica_operation_contract::input::bool_arg(args, &["noValidate", "NoValidate"]);
        let mut validation_stdout = None;
        let mut validated = false;
        let validate_args = Map::from_iter([(
            "ConfigPath".to_string(),
            Value::String(config_path.display().to_string()),
        )]);
        let report = transaction
            .commit_with_post_validation(|| {
                let outcome = validate_cf(&validate_args, context);
                validated = outcome.ok;
                if show_validation_output {
                    validation_stdout = outcome.stdout.clone();
                }
                if outcome.ok {
                    Ok(())
                } else {
                    let detail = if outcome.errors.is_empty() {
                        outcome
                            .stdout
                            .unwrap_or_else(|| "validation returned no diagnostics".to_string())
                    } else {
                        outcome.errors.join("; ")
                    };
                    Err(format!("cf validation failed: {detail}"))
                }
            })
            .map_err(cf_edit_transaction_error)?;
        let warnings = report.cleanup_warnings;
        if config_changed && report.updated.contains(&config_path) {
            config_updated = true;
        }
        let _ = (stdout, show_validation_output, validation_stdout);

        // `applied` must mean "something was written". An edit whose every
        // operation was skipped writes nothing, and dcs.edit already follows
        // this rule for an unchanged template.
        let wrote_anything = config_updated || artifacts.iter().any(|path| path != &config_path);
        let mut mutation = unica_execution_contract::mutation::MutationData::new(wrote_anything);
        if config_updated {
            mutation = mutation.updated(&config_path);
        }
        for artifact in &artifacts {
            if artifact != &config_path {
                mutation = mutation.updated(artifact);
            }
        }
        let data = CfEditData {
            configuration: obj_name.to_string(),
            operations: operation_log,
            added: add_count,
            removed: remove_count,
            modified: modify_count,
            config_updated,
            validated,
            mutation,
        };
        Ok(CfEditRun {
            data,
            config_path,
            artifacts,
            config_updated,
            warnings,
        })
    })();

    match edit_result {
        Ok(CfEditRun {
            data,
            config_path,
            artifacts,
            config_updated,
            warnings,
        }) => {
            let mut changes = Vec::new();
            if config_updated {
                changes.push(format!("updated {}", config_path.display()));
            }
            for artifact in &artifacts {
                if artifact != &config_path {
                    changes.push(format!("updated {}", artifact.display()));
                }
            }
            CfEditExecution {
                outcome: AdapterOutcome {
                    ok: true,
                    summary: format!(
                        "unica.cf.edit applied {} of {} operation(s) to {}",
                        data.operations.iter().filter(|item| item.applied).count(),
                        data.operations.len(),
                        data.configuration
                    ),
                    changes,
                    warnings,
                    errors: Vec::new(),
                    artifacts: artifacts
                        .into_iter()
                        .map(|path| path.display().to_string())
                        .collect(),
                    stdout: None,
                    stderr: None,
                    command: None,
                },
                data: Some(data),
            }
        }
        Err(error) => CfEditExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.cf.edit failed in native Configuration.xml editor".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
}

fn cf_edit_transaction_error(error: String) -> String {
    const STALE_REGISTRATION: &str = "registration target changed after planning: ";
    error
        .strip_prefix(STALE_REGISTRATION)
        .map_or(error.clone(), |path| {
            format!("publication target differs from the expected preimage: {path}")
        })
}

#[cfg(test)]
pub(crate) mod cf_edit_transaction_tests {
    use super::*;
    use crate::application::UnicaApplication;
    use unica_workspace::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use unica_workspace::single_file_publisher::with_before_commit_hook;

    const BOOLEAN_PROPERTIES: &[&str] = &[
        "IncludeHelpInContents",
        "UseManagedFormInOrdinaryApplication",
        "UseOrdinaryFormInManagedApplication",
    ];

    struct BooleanEditFixture {
        root: PathBuf,
        context: WorkspaceContext,
        config_path: PathBuf,
    }

    impl BooleanEditFixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "unica-cf-edit-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&root).unwrap();
            let context = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 0,
            };
            crate::infrastructure::test_fixtures::copy_empty_configuration(&context.workspace_root);
            let config_path = root.join("src/Configuration.xml");
            Self {
                root,
                context,
                config_path,
            }
        }

        fn edit_boolean(&self, property: &str, value: &str) -> AdapterOutcome {
            edit_cf(
                &Map::from_iter([
                    ("ConfigPath".to_string(), json!("src")),
                    ("Operation".to_string(), json!("modify-property")),
                    ("Value".to_string(), json!(format!("{property}={value}"))),
                    ("NoValidate".to_string(), json!(true)),
                ]),
                &self.context,
            )
        }
    }

    impl Drop for BooleanEditFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn transaction_test_configuration() -> Vec<u8> {
        unica_xml_format::text::utf8_bom_bytes(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
    <InternalInfo/>
    <Properties>
      <Name>Demo</Name>
      <Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Demo</v8:content></v8:item></Synonym>
      <Version>1.0</Version>
      <DefaultLanguage>Language.Русский</DefaultLanguage>
    </Properties>
    <ChildObjects><Language>Русский</Language></ChildObjects>
  </Configuration>
</MetaDataObject>"#,
        )
    }

    #[test]
    fn cf_edit_post_write_validation_failure_rolls_back_config_and_external_files() {
        let root = std::env::temp_dir().join(format!(
            "unica-cf-edit-transaction-rollback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        fs::create_dir_all(&src).unwrap();
        let config_path = src.join("Configuration.xml");
        let config_before = transaction_test_configuration();
        fs::write(&config_path, &config_before).unwrap();
        let definition_path = workspace.join("combined.json");
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!([
                {"operation": "modify-property", "value": "Version=2.0"},
                {"operation": "set-panels", "value": {"top": ["open"]}},
                {"operation": "set-home-page", "value": {"template": "OneColumn"}}
            ]))
            .unwrap(),
        )
        .unwrap();
        let definition_before = fs::read(&definition_path).unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 0,
        };
        let args = Map::from_iter([
            ("ConfigPath".to_string(), json!("src")),
            (
                "DefinitionFile".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("NoValidate".to_string(), json!(true)),
        ]);

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            edit_cf(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&definition_path).unwrap(), definition_before);
        assert!(!src.join("Ext").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    pub(crate) fn cf_edit_external_only_change_rejects_concurrent_format_owner_change() {
        let fixture = BooleanEditFixture::new("external-owner-guard");
        let config_before = fs::read(&fixture.config_path).unwrap();
        let concurrent_config = String::from_utf8(config_before)
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .into_bytes();
        let interface_path = fixture
            .config_path
            .parent()
            .unwrap()
            .join("Ext/ClientApplicationInterface.xml");
        let interface_before = fs::read(&interface_path).unwrap();
        let owner_for_hook = fixture.config_path.clone();
        let concurrent_for_hook = concurrent_config.clone();
        let args = Map::from_iter([
            ("ConfigPath".to_string(), json!("src")),
            ("Operation".to_string(), json!("set-panels")),
            ("Value".to_string(), json!(r#"{"top":["sections"]}"#)),
            ("NoValidate".to_string(), json!(true)),
        ]);

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, concurrent_for_hook).unwrap(),
            || edit_cf(&args, &fixture.context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&fixture.config_path).unwrap(), concurrent_config);
        assert_eq!(fs::read(&interface_path).unwrap(), interface_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn cf_edit_home_page_prioritizes_newer_target_over_older_configuration() {
        let fixture = BooleanEditFixture::new("home-page-format-priority");
        let older_config = fs::read_to_string(&fixture.config_path).unwrap().replacen(
            r#"version="2.20""#,
            r#"version="2.19""#,
            1,
        );
        fs::write(&fixture.config_path, &older_config).unwrap();
        let config_dir = fixture.config_path.parent().unwrap();
        let home_page_path = config_dir.join("Ext/HomePageWorkArea.xml");
        let newer_home_page = String::from_utf8(
            cf_edit_set_home_page(&json!({"template": "OneColumn"}), config_dir)
                .unwrap()
                .bytes,
        )
        .unwrap()
        .replacen(r#"version="2.20""#, r#"version="2.21""#, 1);
        fs::write(&home_page_path, &newer_home_page).unwrap();
        let config_before = fs::read(&fixture.config_path).unwrap();
        let home_page_before = fs::read(&home_page_path).unwrap();
        let args = Map::from_iter([
            ("ConfigPath".to_string(), json!("src")),
            ("Operation".to_string(), json!("set-home-page")),
            (
                "Value".to_string(),
                json!(r#"{"template":"TwoColumnsEqualWidth"}"#),
            ),
            ("NoValidate".to_string(), json!(true)),
        ]);

        let outcome = edit_cf(&args, &fixture.context);

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostics = outcome.errors.join("\n");
        assert!(diagnostics.contains("2.21"), "{diagnostics}");
        assert!(diagnostics.contains("1C 8.5"), "{diagnostics}");
        assert!(
            !diagnostics.contains("older than supported"),
            "{diagnostics}"
        );
        assert_eq!(fs::read(&fixture.config_path).unwrap(), config_before);
        assert_eq!(fs::read(&home_page_path).unwrap(), home_page_before);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    }

    #[test]
    fn cf_edit_real_validation_failure_rolls_back_config_and_external_files() {
        let root = std::env::temp_dir().join(format!(
            "unica-cf-edit-real-validation-rollback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 0,
        };
        crate::infrastructure::test_fixtures::copy_empty_configuration(&context.workspace_root);

        let config_path = root.join("src/Configuration.xml");
        let interface_path = root.join("src/Ext/ClientApplicationInterface.xml");
        let config_before = fs::read(&config_path).unwrap();
        let interface_before = fs::read(&interface_path).unwrap();
        let definition_path = root.join("invalid-combined.json");
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!([
                {
                    "operation": "modify-property",
                    "value": "DefaultLanguage="
                },
                {"operation": "set-panels", "value": {"top": ["sections"]}}
            ]))
            .unwrap(),
        )
        .unwrap();
        let args = Map::from_iter([
            ("ConfigPath".to_string(), json!("src")),
            (
                "DefinitionFile".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("NoValidate".to_string(), json!(false)),
        ]);

        let outcome = edit_cf(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("DefaultLanguage is missing or empty"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&interface_path).unwrap(), interface_before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_rejects_noncanonical_boolean_scalars_without_mutating_configuration() {
        let fixture = BooleanEditFixture::new("boolean-preflight");
        let original = fs::read(&fixture.config_path).unwrap();

        for property in BOOLEAN_PROPERTIES {
            let malicious = format!("</{property}><Injected>true</Injected>");
            for invalid in ["", "banana", "TRUE", "1", malicious.as_str()] {
                let outcome = fixture.edit_boolean(property, invalid);

                assert!(!outcome.ok, "{property}={invalid:?}: {outcome:?}");
                let errors = outcome.errors.join("\n");
                assert!(errors.contains(property), "{outcome:?}");
                assert!(errors.contains("8.3.27"), "{outcome:?}");
                assert!(errors.contains("true"), "{outcome:?}");
                assert!(errors.contains("false"), "{outcome:?}");
                assert_eq!(
                    fs::read(&fixture.config_path).unwrap(),
                    original,
                    "{property}={invalid:?}"
                );
                assert!(outcome.changes.is_empty(), "{outcome:?}");
                assert!(outcome.artifacts.is_empty(), "{outcome:?}");
            }
        }
    }

    #[test]
    fn cf_edit_accepts_canonical_true_and_false_for_every_boolean_scalar() {
        let fixture = BooleanEditFixture::new("boolean-canonical");

        for property in BOOLEAN_PROPERTIES {
            for value in ["true", "false"] {
                let outcome = fixture.edit_boolean(property, value);

                assert!(outcome.ok, "{property}={value}: {outcome:?}");
                let xml = fs::read_to_string(&fixture.config_path).unwrap();
                assert!(
                    xml.contains(&format!("<{property}>{value}</{property}>")),
                    "{property}={value}: {xml}"
                );
            }
        }
    }

    #[test]
    fn cf_validate_and_unrelated_edit_reject_existing_noncanonical_boolean_without_byte_changes() {
        let fixture = BooleanEditFixture::new("boolean-existing-invalid");
        let canonical_bytes = fs::read(&fixture.config_path).unwrap();
        let canonical_text = String::from_utf8(canonical_bytes.clone()).unwrap();

        for property in BOOLEAN_PROPERTIES {
            let invalid_text = canonical_text.replacen(
                &format!("<{property}>false</{property}>"),
                &format!("<{property}>banana</{property}>"),
                1,
            );
            assert_ne!(invalid_text, canonical_text, "missing fixture {property}");
            let invalid_bytes = invalid_text.into_bytes();
            fs::write(&fixture.config_path, &invalid_bytes).unwrap();

            let validate = validate_cf(
                &Map::from_iter([("ConfigPath".to_string(), json!("src"))]),
                &fixture.context,
            );
            assert!(!validate.ok, "{property}: {validate:?}");
            let validate_errors = validate.errors.join("\n");
            assert!(validate_errors.contains(property), "{validate:?}");
            assert!(validate_errors.contains("banana"), "{validate:?}");
            assert!(validate_errors.contains("true"), "{validate:?}");
            assert!(validate_errors.contains("false"), "{validate:?}");

            let edit = edit_cf(
                &Map::from_iter([
                    ("ConfigPath".to_string(), json!("src")),
                    ("Operation".to_string(), json!("modify-property")),
                    ("Value".to_string(), json!("Version=2.0")),
                    ("NoValidate".to_string(), json!(false)),
                ]),
                &fixture.context,
            );
            assert!(!edit.ok, "{property}: {edit:?}");
            assert!(edit.errors.join("\n").contains(property), "{edit:?}");
            assert_eq!(
                fs::read(&fixture.config_path).unwrap(),
                invalid_bytes,
                "{property}"
            );

            fs::write(&fixture.config_path, &canonical_bytes).unwrap();
        }
    }

    #[test]
    fn cf_edit_no_validate_suppresses_output_but_not_source_contract_validation() {
        for (label, canonical, invalid, property) in [
            (
                "boolean",
                "<IncludeHelpInContents>false</IncludeHelpInContents>",
                "<IncludeHelpInContents>banana</IncludeHelpInContents>",
                "IncludeHelpInContents",
            ),
            (
                "enum",
                "<DefaultRunMode>ManagedApplication</DefaultRunMode>",
                "<DefaultRunMode>DefinitelyInvalid</DefaultRunMode>",
                "DefaultRunMode",
            ),
        ] {
            let fixture = BooleanEditFixture::new(&format!("no-validate-{label}"));
            let canonical_bytes = fs::read(&fixture.config_path).unwrap();
            let canonical_text = String::from_utf8(canonical_bytes).unwrap();
            let invalid_text = canonical_text.replacen(canonical, invalid, 1);
            assert_ne!(invalid_text, canonical_text, "missing fixture {property}");
            let invalid_bytes = invalid_text.into_bytes();
            fs::write(&fixture.config_path, &invalid_bytes).unwrap();

            let outcome = edit_cf(
                &Map::from_iter([
                    ("ConfigPath".to_string(), json!("src")),
                    ("Operation".to_string(), json!("modify-property")),
                    ("Value".to_string(), json!("Version=2.0")),
                    ("NoValidate".to_string(), json!(true)),
                ]),
                &fixture.context,
            );

            assert!(!outcome.ok, "{property}: {outcome:?}");
            assert!(outcome.errors.join("\n").contains(property), "{outcome:?}");
            assert_eq!(fs::read(&fixture.config_path).unwrap(), invalid_bytes);
            assert!(outcome.changes.is_empty(), "{outcome:?}");
            assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        }

        let fixture = BooleanEditFixture::new("no-validate-valid");
        let outcome = edit_cf(
            &Map::from_iter([
                ("ConfigPath".to_string(), json!("src")),
                ("Operation".to_string(), json!("modify-property")),
                ("Value".to_string(), json!("Version=2.0")),
                ("NoValidate".to_string(), json!(true)),
            ]),
            &fixture.context,
        );

        assert!(outcome.ok, "{outcome:?}");
        assert!(
            !outcome
                .stdout
                .as_deref()
                .unwrap_or_default()
                .contains("--- Running cf-validate ---"),
            "{outcome:?}"
        );
    }

    #[test]
    fn public_cf_edit_set_home_page_rejects_noncanonical_nested_scalars_without_writes() {
        let cases = [
            (
                "visibility-string",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "visibility": "false"}),
                "visibility",
            ),
            (
                "visibility-number",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "visibility": 1}),
                "visibility",
            ),
            (
                "visibility-array",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "visibility": [false]}),
                "visibility",
            ),
            (
                "visibility-object",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "visibility": {"value": false}}),
                "visibility",
            ),
            (
                "role-string",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "roles": {"Role.Reader": "false"}}),
                "roles.Role.Reader",
            ),
            (
                "role-number",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "roles": {"Role.Reader": 0}}),
                "roles.Role.Reader",
            ),
            (
                "role-array",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "roles": {"Role.Reader": [false]}}),
                "roles.Role.Reader",
            ),
            (
                "role-object",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "roles": {"Role.Reader": {"value": false}}}),
                "roles.Role.Reader",
            ),
            (
                "roles-container-array",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "roles": []}),
                "roles",
            ),
            (
                "height-string",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "height": "25"}),
                "height",
            ),
            (
                "height-fraction",
                json!({"form": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "height": 2.5}),
                "height",
            ),
        ];

        for (label, item, expected_error) in cases {
            let fixture = BooleanEditFixture::new(&format!("home-page-{label}"));
            let definition_path = fixture.root.join("invalid-home-page.json");
            fs::write(
                &definition_path,
                serde_json::to_vec(&json!([{
                    "operation": "set-home-page",
                    "value": {
                        "template": "OneColumn",
                        "left": [item]
                    }
                }]))
                .unwrap(),
            )
            .unwrap();
            let config_before = fs::read(&fixture.config_path).unwrap();
            let interface_path = fixture.root.join("src/Ext/ClientApplicationInterface.xml");
            let interface_before = fs::read(&interface_path).unwrap();
            let definition_before = fs::read(&definition_path).unwrap();

            let args = Map::from_iter([
                ("cwd".to_string(), json!(fixture.root.display().to_string())),
                ("dryRun".to_string(), json!(false)),
                ("ConfigPath".to_string(), json!("src")),
                (
                    "DefinitionFile".to_string(),
                    json!(definition_path.display().to_string()),
                ),
                ("NoValidate".to_string(), json!(true)),
            ]);

            let result = UnicaApplication::new()
                .call_tool("unica.cf.edit", &args)
                .unwrap();

            assert!(!result.ok, "{label}: {result:?}");
            assert!(
                result.errors.join("\n").contains(expected_error),
                "{label}: {result:?}"
            );
            assert_eq!(
                fs::read(&fixture.config_path).unwrap(),
                config_before,
                "{label}"
            );
            assert_eq!(
                fs::read(&interface_path).unwrap(),
                interface_before,
                "{label}"
            );
            assert_eq!(
                fs::read(&definition_path).unwrap(),
                definition_before,
                "{label}"
            );
            assert!(
                !fixture.root.join("src/Ext/HomePageWorkArea.xml").exists(),
                "{label}: {result:?}"
            );
            assert!(result.changes.is_empty(), "{label}: {result:?}");
            assert!(result.artifacts.is_empty(), "{label}: {result:?}");
        }
    }

    #[test]
    fn cf_edit_rejects_invalid_core_enum_even_when_optional_validation_is_disabled() {
        let root = std::env::temp_dir().join(format!(
            "unica-cf-edit-core-enum-preflight-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 0,
        };
        crate::infrastructure::test_fixtures::copy_empty_configuration(&context.workspace_root);
        let config_path = root.join("src/Configuration.xml");
        let before = fs::read(&config_path).unwrap();
        for (property, value) in [
            ("DefaultRunMode", "DefinitelyInvalid"),
            ("CompatibilityMode", "Version8_3_28"),
            ("CompatibilityMode", "Version8_5_1"),
            ("ConfigurationExtensionCompatibilityMode", "Version8_3_28"),
            ("InterfaceCompatibilityMode", "TaxiEnableVersion8_5"),
            ("InterfaceCompatibilityMode", "Version8_5EnableTaxi"),
            ("InterfaceCompatibilityMode", "Version8_5"),
            ("InterfaceCompatibilityMode", "Version8_3_24"),
        ] {
            let args = Map::from_iter([
                ("ConfigPath".to_string(), json!("src")),
                ("Operation".to_string(), json!("modify-property")),
                ("Value".to_string(), json!(format!("{property}={value}"))),
                ("NoValidate".to_string(), json!(true)),
            ]);

            let outcome = edit_cf(&args, &context);

            assert!(!outcome.ok, "{property}={value}: {outcome:?}");
            assert!(
                outcome.errors.join("\n").contains(property),
                "{property}={value}: {outcome:?}"
            );
            assert_eq!(
                fs::read(&config_path).unwrap(),
                before,
                "{property}={value}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_missing_child_routes_to_current_creation_contracts() {
        let fixture = BooleanEditFixture::new("missing-child-routing");
        let cases = [
            (
                "Catalog",
                "MissingCatalog",
                &[
                    "unica.meta.add",
                    "/unica:meta-add",
                    "\"sourceSet\":\"<sourceSet>\"",
                    "\"kind\":\"Catalog\"",
                    "\"name\":\"MissingCatalog\"",
                    "\"dryRun\":true",
                ][..],
            ),
            (
                "Subsystem",
                "MissingSubsystem",
                &[
                    "unica.apply",
                    "/unica:subsystem-edit",
                    "\"op\":\"subsystem.create\"",
                    "\"at\":\"<sourceSet>:Configuration\"",
                    "\"values\":{\"name\":\"MissingSubsystem\"}",
                ][..],
            ),
            (
                "Role",
                "MissingRole",
                &[
                    "unica.apply",
                    "/unica:role-edit",
                    "\"op\":\"role.create\"",
                    "\"at\":\"<sourceSet>:Configuration\"",
                    "\"values\":{\"name\":\"MissingRole\"}",
                ][..],
            ),
            ("Bot", "MissingBot", &["platform tooling"][..]),
            ("Language", "MissingLanguage", &["platform tooling"][..]),
        ];

        for (kind, name, expected) in cases {
            let outcome = edit_cf(
                &Map::from_iter([
                    ("ConfigPath".to_string(), json!("src")),
                    ("Operation".to_string(), json!("add-childObject")),
                    ("Value".to_string(), json!(format!("{kind}.{name}"))),
                    ("NoValidate".to_string(), json!(true)),
                ]),
                &fixture.context,
            );
            assert!(!outcome.ok, "{kind}: {outcome:?}");
            let error = outcome.errors.join("\n");
            for token in expected {
                assert!(error.contains(token), "{kind}: missing {token:?}: {error}");
            }
            assert!(!error.contains("compile"), "{kind}: {error}");
            assert!(!error.contains("\"type\":"), "{kind}: {error}");
        }
    }
}

fn cf_edit_operations_guarded(
    args: &Map<String, Value>,
    cwd: &Path,
    operation: Option<&str>,
    definition_file: Option<PathBuf>,
    transaction: &mut CompileTransaction,
) -> Result<Vec<(String, Value)>, String> {
    if let Some(definition_file) = definition_file {
        let definition_file = unica_operation_contract::input::absolutize(definition_file, cwd);
        let parsed = FileBackedJson::read(&definition_file, |err| {
            format!("failed to parse {}: {err}", definition_file.display())
        })?
        .bind_to(transaction)?;
        Ok(cf_edit_operations_from_value(parsed, operation))
    } else {
        cf_edit_operations(
            &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
            args,
            cwd,
            operation,
            None,
        )
    }
}

pub(crate) fn cf_edit_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let definition_file =
        unica_operation_contract::input::path_arg(args, &["definitionFile", "DefinitionFile"]);
    let operation = unica_operation_contract::input::string_arg(args, &["operation", "Operation"]);
    if definition_file.is_some() && operation.is_some() {
        return Err("Cannot use both -DefinitionFile and -Operation".to_string());
    }
    if definition_file.is_none() && operation.is_none() {
        return Err("Either -DefinitionFile or -Operation is required".to_string());
    }
    let config_path = resolve_cf_edit_config_path(args, context)?;
    let operations = cf_edit_operations(
        &unica_workspace::discovery::ProjectDiscoveryReader::new(Path::new("")),
        args,
        &context.cwd,
        operation,
        definition_file,
    )?;
    cf_edit_validate_child_object_kinds(&operations)?;
    cf_edit_format_dependency_paths_for_operations(&config_path, &operations)
}

pub(crate) fn cf_read_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    operation: &str,
) -> Result<Vec<PathBuf>, String> {
    let config_path = resolve_cf_read_config_path(args, context)?;
    let config_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut paths = vec![config_path];
    match operation {
        "cf-validate" => {
            paths.push(config_dir.join("Ext/HomePageWorkArea.xml"));
        }
        "cf-info" => {
            let mode = unica_operation_contract::input::string_arg(args, &["mode", "Mode"])
                .unwrap_or("overview");
            let section = unica_operation_contract::input::string_arg(
                args,
                &["section", "Section", "name", "Name"],
            )
            .unwrap_or("");
            if section == "home-page" || mode == "full" {
                paths.push(config_dir.join("Ext/HomePageWorkArea.xml"));
            }
            if section != "home-page" && mode == "full" {
                paths.push(config_dir.join("Ext/ClientApplicationInterface.xml"));
            }
        }
        _ => {}
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        // `cf-info` answers with typed data; the registry keeps only the
        // prose-shaped path, so the typed route reaches it in typed_result.rs.
        "cf-info" => Some(Ok(analyze_cf_info(
            args,
            context,
            &WorkspaceSupportStateReader::new(context),
        )
        .outcome)),
        "cf-validate" => Some(Ok(validate_cf(args, context))),
        _ => None,
    }
}

pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    match operation {
        "cf-edit" => Some(edit_cf(args, context)),
        _ => None,
    }
}

#[cfg(test)]
pub(super) mod cf_read_selector_bridge_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NONCE: AtomicU64 = AtomicU64::new(0);

    /// A configuration root reachable both ways: by the `src` path and by the
    /// name of the source set that declares it.
    fn workspace(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!(
            "unica-cf-read-bridge-{name}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Demo</Name><Vendor>Unica</Vendor><Version>1.0.0.1</Version></Properties><ChildObjects></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    #[test]
    pub(crate) fn cf_info_answers_identically_for_a_source_set_and_a_config_path() {
        let context = workspace("info");

        let physical = analyze_cf_info(
            &Map::from_iter([("ConfigPath".to_string(), json!("src"))]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );
        let logical = analyze_cf_info(
            &Map::from_iter([("sourceSet".to_string(), json!("main"))]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        );

        assert!(logical.outcome.ok, "{:?}", logical.outcome);
        assert!(physical.outcome.ok, "{:?}", physical.outcome);
        // Compare what the caller actually receives: the serialized
        // answer, not a struct the wire never carries.
        assert_eq!(
            serde_json::to_value(&logical.data).unwrap(),
            serde_json::to_value(&physical.data).unwrap()
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn cf_info_reports_an_unknown_source_set_as_such() {
        let context = workspace("unknown-set");

        let outcome = analyze_cf_info(
            &Map::from_iter([("sourceSet".to_string(), json!("nope"))]),
            &context,
            &WorkspaceSupportStateReader::new(&context),
        )
        .outcome;

        assert!(!outcome.ok);
        assert!(
            outcome
                .errors
                .iter()
                .chain(std::iter::once(&outcome.summary))
                .any(|message| message.contains("source_set_unknown")),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    pub(crate) fn cf_validate_answers_identically_for_a_source_set_and_a_config_path() {
        let context = workspace("validate");

        let physical = validate_cf(
            &Map::from_iter([("ConfigPath".to_string(), json!("src"))]),
            &context,
        );
        let logical = validate_cf(
            &Map::from_iter([("sourceSet".to_string(), json!("main"))]),
            &context,
        );

        assert_eq!(logical.ok, physical.ok, "{logical:?} vs {physical:?}");
        assert_eq!(logical.errors, physical.errors);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}

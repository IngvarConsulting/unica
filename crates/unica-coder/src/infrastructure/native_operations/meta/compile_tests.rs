#![allow(dead_code, unused_imports)]

use crate::application::AdapterOutcome;
use crate::domain::metadata::MetadataKind;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::metadata_kind;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::cf::create_configuration_scaffold;
use super::super::subsystem::compile_subsystem;
use super::edit::meta_edit_fill_value_xml;
use super::legacy_dsl::{compile_meta, meta_compile_object_xml, meta_compile_type_plural};
use super::publisher::{fresh_meta_compile_uuid, register_compiled_meta_in_configuration};
use super::remove::{meta_remove_supported_types, meta_remove_type_plural, remove_metadata_object};
use super::template_catalog::normalize_meta_enum_value;
use super::validation::is_guid;
use super::{
    with_meta_compile_after_format_plan_hook, with_meta_compile_after_owner_validation_hook,
};

#[cfg(test)]
mod uuid_tests {
    use super::{fresh_meta_compile_uuid, is_guid};

    #[test]
    fn fresh_meta_compile_uuid_generates_uuid_v4() {
        let value = fresh_meta_compile_uuid();

        assert!(is_guid(&value), "{value}");
        assert!(!value.starts_with("00000000-0000-0000-"), "{value}");
        assert_eq!(value.as_bytes()[14], b'4', "{value}");
        assert!(
            matches!(value.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "{value}"
        );
    }
}

#[cfg(test)]
mod enum_contract_tests {
    use super::{json, meta_compile_object_xml, normalize_meta_enum_value, Value};

    #[test]
    fn legacy_hierarchy_items_only_normalizes_to_platform_value() {
        assert_eq!(
            normalize_meta_enum_value("HierarchyItemsOnly"),
            "HierarchyOfItems"
        );
    }

    #[test]
    fn meta_compile_rejects_values_outside_exact_8_3_27_enum_contracts() {
        let cases = [
            (
                "Catalog",
                json!({"subordinationUse": "Sideways"}),
                "SubordinationUse",
            ),
            (
                "Catalog",
                json!({"codeSeries": "WholeChartOfAccounts"}),
                "CatalogCodeSeries",
            ),
            (
                "ChartOfAccounts",
                json!({"codeSeries": "WholeCatalog"}),
                "ChartOfAccountsCodeSeries",
            ),
            (
                "ChartOfCharacteristicTypes",
                json!({"codeSeries": "WholeCatalog"}),
                "CharacteristicTypeCodeSeries",
            ),
            (
                "ExchangePlan",
                json!({"choiceMode": "DialogOnly"}),
                "ChoiceMode",
            ),
            (
                "Document",
                json!({"numberPeriodicity": "Second"}),
                "DocumentNumberPeriodicity",
            ),
            (
                "BusinessProcess",
                json!({"numberPeriodicity": "Second"}),
                "BusinessProcessNumberPeriodicity",
            ),
            (
                "CalculationRegister",
                json!({"periodicity": "Nonperiodical"}),
                "CalculationRegisterPeriodicity",
            ),
            (
                "ChartOfCharacteristicTypes",
                json!({"predefinedDataUpdate": "Manual"}),
                "PredefinedDataUpdate",
            ),
            (
                "HTTPService",
                json!({
                    "urlTemplates": {
                        "Items": {"methods": {"Fetch": "FETCH"}}
                    }
                }),
                "HTTPMethod",
            ),
            (
                "WebService",
                json!({
                    "operations": {
                        "Ping": {
                            "parameters": {
                                "Text": {"type": "xs:string", "direction": "Sideways"}
                            }
                        }
                    }
                }),
                "TransferDirection",
            ),
            (
                "Catalog",
                json!({
                    "attributes": [{
                        "name": "Value",
                        "type": "String(10)",
                        "fillChecking": "ShowWarning"
                    }]
                }),
                "FillChecking",
            ),
        ];

        for (object_type, definition, expected_property) in cases {
            let error = meta_compile_object_xml(
                definition.as_object().unwrap(),
                object_type,
                "ContractProbe",
                "2.20",
            )
            .unwrap_err();

            assert!(error.contains(expected_property), "{object_type}: {error}");
            assert!(error.contains("8.3.27"), "{object_type}: {error}");
        }
    }

    #[test]
    fn meta_compile_accepts_exact_8_3_27_context_specific_values_and_aliases() {
        let cases: [(&str, Value, &[&str]); 8] = [
            (
                "Catalog",
                json!({"subordinationUse": "ToFoldersAndItems", "codeSeries": "WithinOwnerSubordination", "choiceMode": "FromForm"}),
                &[
                    "<SubordinationUse>ToFoldersAndItems</SubordinationUse>",
                    "<CodeSeries>WithinOwnerSubordination</CodeSeries>",
                    "<ChoiceMode>FromForm</ChoiceMode>",
                ],
            ),
            (
                "ChartOfAccounts",
                json!({"codeSeries": "WithinSubordination"}),
                &["<CodeSeries>WithinSubordination</CodeSeries>"],
            ),
            (
                "ChartOfCharacteristicTypes",
                json!({"codeSeries": "WholeCharacteristicKind", "choiceMode": "QuickChoice", "predefinedDataUpdate": "DontAutoUpdate"}),
                &[
                    "<CodeSeries>WholeCharacteristicKind</CodeSeries>",
                    "<ChoiceMode>QuickChoice</ChoiceMode>",
                    "<PredefinedDataUpdate>DontAutoUpdate</PredefinedDataUpdate>",
                ],
            ),
            (
                "Document",
                json!({"numberPeriodicity": "Daily"}),
                &["<NumberPeriodicity>Day</NumberPeriodicity>"],
            ),
            (
                "BusinessProcess",
                json!({"numberPeriodicity": "Quarterly"}),
                &["<NumberPeriodicity>Quarter</NumberPeriodicity>"],
            ),
            (
                "CalculationRegister",
                json!({"periodicity": "Yearly"}),
                &["<Periodicity>Year</Periodicity>"],
            ),
            (
                "HTTPService",
                json!({"urlTemplates": {"Items": {"methods": {"Lock": "LOCK"}}}}),
                &["<HTTPMethod>LOCK</HTTPMethod>"],
            ),
            (
                "WebService",
                json!({"operations": {"Ping": {"parameters": {"Text": {"type": "xs:string", "direction": "InOut"}}}}}),
                &["<TransferDirection>InOut</TransferDirection>"],
            ),
        ];

        for (object_type, definition, expected_fragments) in cases {
            let xml = meta_compile_object_xml(
                definition.as_object().unwrap(),
                object_type,
                "ContractProbe",
                "2.20",
            )
            .unwrap_or_else(|error| panic!("{object_type}: {error}"))
            .0;
            for expected in expected_fragments {
                assert!(xml.contains(expected), "{object_type}: missing {expected}");
            }
        }
    }
}

#[cfg(test)]
mod fill_value_contract_tests {
    use super::meta_edit_fill_value_xml;

    #[test]
    fn fill_value_literals_use_documented_xsi_types() {
        let cases = [
            ("nil", "<FillValue xsi:nil=\"true\"/>"),
            (
                "Catalog.Items.EmptyRef",
                "<FillValue xsi:type=\"xr:DesignTimeRef\">Catalog.Items.EmptyRef</FillValue>",
            ),
            (
                "TRUE",
                "<FillValue xsi:type=\"xs:boolean\">true</FillValue>",
            ),
            (
                "-12.50",
                "<FillValue xsi:type=\"xs:decimal\">-12.50</FillValue>",
            ),
            (
                "2026-07-19T10:20:30",
                "<FillValue xsi:type=\"xs:dateTime\">2026-07-19T10:20:30</FillValue>",
            ),
            (
                "2026-99-99T99:99:99",
                "<FillValue xsi:type=\"xs:string\">2026-99-99T99:99:99</FillValue>",
            ),
            (
                "2025-02-29T10:20:30",
                "<FillValue xsi:type=\"xs:string\">2025-02-29T10:20:30</FillValue>",
            ),
            (
                "A&B",
                "<FillValue xsi:type=\"xs:string\">A&amp;B</FillValue>",
            ),
        ];

        for (value, expected) in cases {
            assert_eq!(meta_edit_fill_value_xml("", value), expected, "{value}");
        }
    }
}

#[cfg(test)]
mod registration_tests {
    use super::{
        compile_meta, fs, json, meta_compile_object_xml, meta_compile_type_plural,
        meta_remove_supported_types, meta_remove_type_plural, metadata_kind,
        register_compiled_meta_in_configuration, Map, MetadataKind, PathBuf, Value,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_output_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let output_dir = std::env::temp_dir().join(format!("unica-register-{name}-{nanos}"));
        fs::create_dir_all(&output_dir).unwrap();
        output_dir
    }

    #[test]
    fn root_registration_uses_canonical_order_and_is_idempotent() {
        let output_dir = temp_output_dir("canonical");
        let config_path = output_dir.join("Configuration.xml");
        fs::write(
            &config_path,
            concat!(
                "<MetaDataObject><Configuration><ChildObjects>\n",
                "\t<CommonModule>Core</CommonModule>\n",
                "\t<CommonAttribute>Shared</CommonAttribute>\n",
                "</ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();

        let status = register_compiled_meta_in_configuration(&output_dir, "Bot", "Assistant")
            .expect("Bot registration must succeed");
        assert_eq!(status.as_deref(), Some("added"));
        let after_add = fs::read_to_string(&config_path).unwrap();
        assert!(
            after_add.find("<CommonModule>Core</CommonModule>").unwrap()
                < after_add.find("<Bot>Assistant</Bot>").unwrap()
        );
        assert!(
            after_add.find("<Bot>Assistant</Bot>").unwrap()
                < after_add
                    .find("<CommonAttribute>Shared</CommonAttribute>")
                    .unwrap()
        );

        let duplicate = register_compiled_meta_in_configuration(&output_dir, "Bot", "Assistant")
            .expect("duplicate registration must be a no-op");
        assert_eq!(duplicate.as_deref(), Some("already"));
        assert_eq!(fs::read_to_string(&config_path).unwrap(), after_add);

        fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn root_registration_expands_self_closing_child_objects() {
        let output_dir = temp_output_dir("self-closing");
        let config_path = output_dir.join("Configuration.xml");
        fs::write(
            &config_path,
            "<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>",
        )
        .unwrap();

        let status = register_compiled_meta_in_configuration(&output_dir, "Bot", "Assistant")
            .expect("Bot registration must succeed");

        assert_eq!(status.as_deref(), Some("added"));
        assert!(fs::read_to_string(&config_path)
            .unwrap()
            .contains("<ChildObjects>\n\t<Bot>Assistant</Bot>\n</ChildObjects>"));
        fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn root_registration_rejects_unknown_metadata_kind_without_mutation() {
        let output_dir = temp_output_dir("unknown");
        let config_path = output_dir.join("Configuration.xml");
        let before =
            "<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>";
        fs::write(&config_path, before).unwrap();

        let error =
            register_compiled_meta_in_configuration(&output_dir, "SyntheticMetadata", "Unknown")
                .expect_err("unknown metadata kinds must be rejected");

        assert!(
            error.contains("Unknown type 'SyntheticMetadata'"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&config_path).unwrap(), before);
        fs::remove_dir_all(output_dir).unwrap();
    }

    #[test]
    fn narrower_metadata_capability_sets_use_registry_directories_without_expansion() {
        assert_eq!(meta_remove_supported_types().len(), 39);
        assert!(!meta_remove_supported_types().contains(&"Bot"));
        for object_type in meta_remove_supported_types() {
            assert_eq!(
                meta_remove_type_plural(object_type),
                metadata_kind(object_type).map(|kind| kind.directory)
            );
        }

        assert_eq!(MetadataKind::ALL.len(), 23);
        for kind in MetadataKind::ALL {
            let object_type = kind.as_str();
            assert_eq!(
                meta_compile_type_plural(object_type),
                metadata_kind(object_type).map(|kind| kind.directory)
            );
        }
        assert_eq!(meta_compile_type_plural("Bot"), None);
        assert_eq!(meta_remove_type_plural("Bot"), None);
    }
}

#[cfg(test)]
mod owner_contract_tests {
    use super::{
        compile_meta, compile_subsystem, create_configuration_scaffold, fs, json,
        meta_compile_object_xml, meta_compile_type_plural, remove_metadata_object,
        with_meta_compile_after_format_plan_hook, with_meta_compile_after_owner_validation_hook,
        AdapterOutcome, Map, Path, PathBuf, Value, WorkspaceContext,
    };
    use crate::application::UnicaApplication;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-meta-owner-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn create_valid_configuration(context: &WorkspaceContext) -> PathBuf {
        let args = Map::from_iter([
            ("Name".to_string(), json!("OwnerContract")),
            ("OutputDir".to_string(), json!("src")),
        ]);
        let outcome = create_configuration_scaffold(&args, context);
        assert!(outcome.ok, "{outcome:?}");
        context.cwd.join("src/Configuration.xml")
    }

    fn compile_catalog(context: &WorkspaceContext, name: &str) -> AdapterOutcome {
        let definition_path = context.cwd.join(format!("{name}.json"));
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({"type": "Catalog", "name": name})).unwrap(),
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("OutputDir".to_string(), json!("src")),
        ]);
        compile_meta(&args, context)
    }

    fn seed_event_handlers(context: &WorkspaceContext) {
        let definition_path = context.cwd.join("event-handlers.json");
        fs::write(
            &definition_path,
            serde_json::to_vec_pretty(&json!({
                "type": "CommonModule",
                "name": "EventHandlers",
                "context": "server"
            }))
            .unwrap(),
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("OutputDir".to_string(), json!("src")),
        ]);
        let outcome = compile_meta(&args, context);
        assert!(outcome.ok, "{outcome:?}");
        fs::write(
            context
                .cwd
                .join("src/CommonModules/EventHandlers/Ext/Module.bsl"),
            "Procedure OnBeforeWrite(Source, Cancel) Export\nEndProcedure\n",
        )
        .unwrap();
    }

    fn compile_subsystem_for_catalog(
        context: &WorkspaceContext,
        subsystem_name: &str,
        catalog_name: &str,
    ) -> AdapterOutcome {
        let definition = json!({
            "name": subsystem_name,
            "content": [format!("Catalog.{catalog_name}")]
        });
        let args = Map::from_iter([
            ("OutputDir".to_string(), json!("src")),
            ("Value".to_string(), json!(definition.to_string())),
        ]);
        compile_subsystem(&args, context)
    }

    fn make_configuration_enum_invalid(path: &Path) -> Vec<u8> {
        let original = fs::read(path).unwrap();
        let text = String::from_utf8(original).unwrap();
        let invalid = text.replacen(
            "<ConfigurationExtensionCompatibilityMode>Version8_3_27</ConfigurationExtensionCompatibilityMode>",
            "<ConfigurationExtensionCompatibilityMode>Bogus</ConfigurationExtensionCompatibilityMode>",
            1,
        );
        assert_ne!(invalid, text);
        fs::write(path, invalid.as_bytes()).unwrap();
        invalid.into_bytes()
    }

    #[test]
    fn meta_compile_rejects_invalid_configuration_owner_without_creating_object() {
        let context = temp_context("compile-invalid-configuration");
        let config_path = create_valid_configuration(&context);
        let invalid_owner = make_configuration_enum_invalid(&config_path);

        let outcome = compile_catalog(&context, "RejectedCatalog");

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("ConfigurationExtensionCompatibilityMode"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), invalid_owner);
        assert!(!context
            .cwd
            .join("src/Catalogs/RejectedCatalog.xml")
            .exists());
        assert!(!context.cwd.join("src/Catalogs/RejectedCatalog").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn public_meta_compile_rejects_event_subscription_with_missing_source_object() {
        let context = temp_context("compile-event-subscription-missing-source");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = create_valid_configuration(&context);
        seed_event_handlers(&context);
        let configuration_before = fs::read(&config_path).unwrap();
        let definition_path = context.cwd.join("event-subscription.json");
        fs::write(
            &definition_path,
            serde_json::to_vec_pretty(&json!({
                "type": "EventSubscription",
                "name": "MissingCatalogSubscription",
                "source": ["CatalogObject.MissingCatalog"],
                "event": "BeforeWrite",
                "handler": "EventHandlers.OnBeforeWrite"
            }))
            .unwrap(),
        )
        .unwrap();
        let args = Map::from_iter([
            ("cwd".to_string(), json!(context.cwd.display().to_string())),
            ("dryRun".to_string(), json!(false)),
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("OutputDir".to_string(), json!("src")),
        ]);

        let outcome =
            crate::application::call_legacy_metadata_tool_for_tests("unica.meta.compile", &args)
                .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostics = outcome.errors.join("\n").replace('\\', "/");
        assert!(
            diagnostics.contains("EventSubscription")
                && diagnostics.contains("CatalogObject.MissingCatalog")
                && diagnostics.contains("Catalogs/MissingCatalog.xml"),
            "{diagnostics}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), configuration_before);
        assert!(!context
            .cwd
            .join("src/EventSubscriptions/MissingCatalogSubscription.xml")
            .exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_compile_accepts_event_subscription_source_created_later_in_same_batch() {
        let context = temp_context("compile-event-subscription-forward-batch-source");
        create_valid_configuration(&context);
        seed_event_handlers(&context);
        let definition_path = context.cwd.join("event-subscription-batch.json");
        fs::write(
            &definition_path,
            serde_json::to_vec_pretty(&json!([
                {
                    "type": "EventSubscription",
                    "name": "BatchCatalogSubscription",
                    "source": ["CatalogObject.BatchCatalog"],
                    "event": "BeforeWrite",
                    "handler": "EventHandlers.OnBeforeWrite"
                },
                {
                    "type": "Catalog",
                    "name": "BatchCatalog"
                }
            ]))
            .unwrap(),
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("OutputDir".to_string(), json!("src")),
        ]);

        let outcome = compile_meta(&args, &context);

        assert!(outcome.ok, "{outcome:?}");
        assert!(context
            .cwd
            .join("src/EventSubscriptions/BatchCatalogSubscription.xml")
            .is_file());
        assert!(context.cwd.join("src/Catalogs/BatchCatalog.xml").is_file());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn public_meta_compile_prioritizes_newer_existing_target_over_older_configuration() {
        let context = temp_context("public-compile-existing-newer-target");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = create_valid_configuration(&context);
        let older_configuration = fs::read_to_string(&config_path)
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.19""#, 1)
            .into_bytes();
        fs::write(&config_path, &older_configuration).unwrap();

        let target_path = context.cwd.join("src/Catalogs/ExistingCatalog.xml");
        fs::create_dir_all(target_path.parent().unwrap()).unwrap();
        let newer_target = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog/></MetaDataObject>"#.to_vec();
        fs::write(&target_path, &newer_target).unwrap();
        let definition_path = context.cwd.join("catalog.json");
        let definition =
            serde_json::to_vec_pretty(&json!({"type": "Catalog", "name": "ExistingCatalog"}))
                .unwrap();
        fs::write(&definition_path, &definition).unwrap();
        let args = Map::from_iter([
            ("cwd".to_string(), json!(context.cwd.display().to_string())),
            ("dryRun".to_string(), json!(false)),
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("OutputDir".to_string(), json!("src")),
        ]);

        let outcome =
            crate::application::call_legacy_metadata_tool_for_tests("unica.meta.compile", &args)
                .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostic = &outcome.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        let warning = outcome.warnings.join("\n");
        assert!(warning.contains("1С 8.5"), "{warning}");
        assert!(!warning.contains("миграц"), "{warning}");
        assert!(!warning.contains("повторно выгруз"), "{warning}");
        assert!(!warning.contains("re-export"), "{warning}");
        assert_eq!(fs::read(&config_path).unwrap(), older_configuration);
        assert_eq!(fs::read(&target_path).unwrap(), newer_target);
        assert_eq!(fs::read(&definition_path).unwrap(), definition);
        assert!(!context.cwd.join("src/Catalogs/ExistingCatalog").exists());
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn public_meta_compile_rejects_newer_partial_exchange_plan_target_without_mutation() {
        let context = temp_context("public-compile-partial-exchange-plan-target");
        fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = create_valid_configuration(&context);
        let configuration = fs::read(&config_path).unwrap();

        let object_dir = context.cwd.join("src/ExchangePlans/PartialExchangePlan");
        let descriptor_path = object_dir.with_extension("xml");
        let object_module_path = object_dir.join("Ext/ObjectModule.bsl");
        let content_path = object_dir.join("Ext/Content.xml");
        fs::create_dir_all(content_path.parent().unwrap()).unwrap();
        let newer_content = br#"<?xml version="1.0" encoding="UTF-8"?>
<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.21"/>
"#
        .to_vec();
        fs::write(&content_path, &newer_content).unwrap();

        let definition_path = context.cwd.join("exchange-plan.json");
        let definition = serde_json::to_vec_pretty(
            &json!({"type": "ExchangePlan", "name": "PartialExchangePlan"}),
        )
        .unwrap();
        fs::write(&definition_path, &definition).unwrap();
        let args = Map::from_iter([
            ("cwd".to_string(), json!(context.cwd.display().to_string())),
            ("dryRun".to_string(), json!(false)),
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("OutputDir".to_string(), json!("src")),
        ]);

        assert!(!descriptor_path.exists());
        assert!(!object_module_path.exists());
        let outcome =
            crate::application::call_legacy_metadata_tool_for_tests("unica.meta.compile", &args)
                .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostic = &outcome.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(fs::read(&config_path).unwrap(), configuration);
        assert_eq!(fs::read(&content_path).unwrap(), newer_content);
        assert_eq!(fs::read(&definition_path).unwrap(), definition);
        assert!(!descriptor_path.exists());
        assert!(!object_module_path.exists());
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_compile_rejects_configuration_replaced_after_owner_validation() {
        let context = temp_context("compile-detached-owner-race");
        let config_path = create_valid_configuration(&context);
        let original = fs::read(&config_path).unwrap();
        let concurrent = String::from_utf8(original.clone())
            .unwrap()
            .replacen(
                "<Name>OwnerContract</Name>",
                "<Name>ConcurrentOwner</Name>",
                1,
            )
            .into_bytes();
        assert_ne!(concurrent, original);
        let config_for_hook = config_path.clone();
        let concurrent_for_hook = concurrent.clone();

        let outcome = with_meta_compile_after_owner_validation_hook(
            move |_| fs::write(&config_for_hook, &concurrent_for_hook).unwrap(),
            || compile_catalog(&context, "RaceCatalog"),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("changed while planning"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), concurrent);
        assert!(!context.cwd.join("src/Catalogs/RaceCatalog.xml").exists());
        assert!(!context.cwd.join("src/Catalogs/RaceCatalog").exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_compile_rejects_newer_partial_extra_created_after_format_plan() {
        let context = temp_context("compile-partial-extra-race");
        let config_path = create_valid_configuration(&context);
        let config_before = fs::read(&config_path).unwrap();
        let definition_path = context.cwd.join("exchange-plan-race.json");
        fs::write(
            &definition_path,
            serde_json::to_vec(&json!({
                "type": "ExchangePlan",
                "name": "RacePlan"
            }))
            .unwrap(),
        )
        .unwrap();
        let descriptor = context.cwd.join("src/ExchangePlans/RacePlan.xml");
        let content = context
            .cwd
            .join("src/ExchangePlans/RacePlan/Ext/Content.xml");
        let newer =
            br#"<ExchangePlanContent xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.21"/>"#
                .to_vec();
        let content_for_hook = content.clone();
        let newer_for_hook = newer.clone();
        let args = Map::from_iter([
            (
                "JsonPath".to_string(),
                json!(definition_path.display().to_string()),
            ),
            ("OutputDir".to_string(), json!("src")),
        ]);

        let outcome = with_meta_compile_after_format_plan_hook(
            move || {
                fs::create_dir_all(content_for_hook.parent().unwrap()).unwrap();
                fs::write(&content_for_hook, &newer_for_hook).unwrap();
            },
            || compile_meta(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.errors.join("\n").contains("2.21"), "{outcome:?}");
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&content).unwrap(), newer);
        assert!(!descriptor.exists());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rejects_invalid_configuration_owner_without_deleting_object() {
        let context = temp_context("remove-invalid-configuration");
        let config_path = create_valid_configuration(&context);
        let compiled = compile_catalog(&context, "ProtectedCatalog");
        assert!(compiled.ok, "{compiled:?}");
        let object_path = context.cwd.join("src/Catalogs/ProtectedCatalog.xml");
        let object_before = fs::read(&object_path).unwrap();
        let invalid_owner = make_configuration_enum_invalid(&config_path);
        let args = Map::from_iter([
            ("ConfigDir".to_string(), json!("src")),
            ("Object".to_string(), json!("Catalog.ProtectedCatalog")),
            ("Force".to_string(), json!(true)),
        ]);

        let outcome = remove_metadata_object(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostics = format!(
            "{}\n{}",
            outcome.errors.join("\n"),
            outcome.stdout.unwrap_or_default()
        );
        assert!(
            diagnostics.contains("ConfigurationExtensionCompatibilityMode"),
            "{diagnostics}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), invalid_owner);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn public_meta_remove_rejects_newer_reference_scan_xml_without_mutation() {
        let context = temp_context("remove-newer-reference-scan");
        let config_path = create_valid_configuration(&context);
        let compiled = compile_catalog(&context, "ProtectedCatalog");
        assert!(compiled.ok, "{compiled:?}");
        let object_path = context.cwd.join("src/Catalogs/ProtectedCatalog.xml");
        let reference_path = context.cwd.join("src/Documents/NewerReader.xml");
        fs::create_dir_all(reference_path.parent().unwrap()).unwrap();
        let newer = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Document/></MetaDataObject>"#.to_vec();
        fs::write(&reference_path, &newer).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_before = fs::read(&object_path).unwrap();
        let args = Map::from_iter([
            ("cwd".to_string(), json!(context.cwd.display().to_string())),
            ("ConfigDir".to_string(), json!("src")),
            ("Object".to_string(), json!("Catalog.ProtectedCatalog")),
            ("Force".to_string(), json!(true)),
            ("dryRun".to_string(), json!(false)),
        ]);

        let outcome =
            crate::application::call_legacy_metadata_tool_for_tests("unica.meta.remove", &args)
                .unwrap();

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostic = &outcome
            .diagnostics
            .as_ref()
            .unwrap_or_else(|| panic!("{outcome:?}"))["formatCompatibility"];
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&reference_path).unwrap(), newer);
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{outcome:?}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rejects_invalid_subsystem_owner_without_mutating_any_owner() {
        let context = temp_context("remove-invalid-subsystem");
        let config_path = create_valid_configuration(&context);
        let compiled = compile_catalog(&context, "ProtectedBySubsystem");
        assert!(compiled.ok, "{compiled:?}");
        let subsystem =
            compile_subsystem_for_catalog(&context, "RemovalScope", "ProtectedBySubsystem");
        assert!(subsystem.ok, "{subsystem:?}");
        let object_path = context.cwd.join("src/Catalogs/ProtectedBySubsystem.xml");
        let subsystem_path = context.cwd.join("src/Subsystems/RemovalScope.xml");
        let source = fs::read_to_string(&subsystem_path).unwrap();
        let invalid_subsystem = source.replacen(
            "<IncludeHelpInContents>true</IncludeHelpInContents>",
            "<IncludeHelpInContents>banana</IncludeHelpInContents>",
            1,
        );
        assert_ne!(invalid_subsystem, source);
        fs::write(&subsystem_path, invalid_subsystem.as_bytes()).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_before = fs::read(&object_path).unwrap();
        let subsystem_before = fs::read(&subsystem_path).unwrap();
        let args = Map::from_iter([
            ("ConfigDir".to_string(), json!("src")),
            ("Object".to_string(), json!("Catalog.ProtectedBySubsystem")),
            ("Force".to_string(), json!(true)),
        ]);

        let outcome = remove_metadata_object(&args, &context);

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostics = format!(
            "{}\n{}",
            outcome.errors.join("\n"),
            outcome.stdout.unwrap_or_default()
        );
        assert!(
            diagnostics.contains("IncludeHelpInContents"),
            "{diagnostics}"
        );
        assert!(diagnostics.contains("banana"), "{diagnostics}");
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&subsystem_path).unwrap(), subsystem_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

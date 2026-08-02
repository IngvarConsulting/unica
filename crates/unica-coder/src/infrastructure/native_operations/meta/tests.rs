#![allow(dead_code, unused_imports)]

use super::*;

#[cfg(test)]
mod uuid_tests {
    use super::*;

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
    use super::*;

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
    use super::*;

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
    use super::*;
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

        assert_eq!(META_COMPILE_SUPPORTED_TYPES.len(), 23);
        assert!(!META_COMPILE_SUPPORTED_TYPES.contains(&"Bot"));
        for object_type in META_COMPILE_SUPPORTED_TYPES {
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
    use super::*;
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

        let outcome = UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
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

        let outcome = UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
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
        let outcome = UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
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

        let outcome = UnicaApplication::new()
            .call_tool("unica.meta.remove", &args)
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

#[cfg(test)]
mod remove_tests {
    use super::super::super::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use super::super::super::single_file_publisher::with_before_commit_hook;
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-meta-remove-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    fn remove_args(config_dir: &Path, object: &str, force: bool) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "ConfigDir".to_string(),
            Value::String(config_dir.display().to_string()),
        );
        args.insert("Object".to_string(), Value::String(object.to_string()));
        args.insert("Force".to_string(), Value::Bool(force));
        args
    }

    fn configuration_bytes(object_name: &str) -> Vec<u8> {
        utf8_bom_bytes(&format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration><ChildObjects><Catalog>{object_name}</Catalog></ChildObjects></Configuration></MetaDataObject>\r\n"
        ))
    }

    fn initialized_config_with_catalog(
        context: &WorkspaceContext,
        object_name: &str,
    ) -> (PathBuf, PathBuf) {
        let config_dir = context.cwd.join("src");
        let init = create_configuration_scaffold(
            &Map::from_iter([
                ("Name".to_string(), json!("RemoveReferenceGuard")),
                (
                    "OutputDir".to_string(),
                    json!(config_dir.display().to_string()),
                ),
            ]),
            context,
        );
        assert!(init.ok, "{init:?}");
        let config_path = config_dir.join("Configuration.xml");
        let mut registration = CompileTransaction::new();
        assert_eq!(
            registration
                .register_canonical_child(&config_path, "Catalog", object_name)
                .unwrap(),
            RegistrationStatus::Added
        );
        registration.commit().unwrap();
        (config_dir, config_path)
    }

    #[test]
    fn meta_remove_rejects_unsafe_name_before_inspecting_config_directory() {
        let context = temp_context("unsafe-before-config");
        let missing_config = context.cwd.join("missing-config");

        for object in ["Catalog.../Victim", "Catalog.Bad&Name"] {
            let outcome =
                remove_metadata_object(&remove_args(&missing_config, object, false), &context);

            assert!(!outcome.ok, "{object}: {outcome:?}");
            let error = outcome.errors.join("\n");
            assert!(error.contains("Unicode XML NCName"), "{object}: {error}");
            assert!(error.contains("single path component"), "{object}: {error}");
            assert!(
                !error.contains("Config directory not found"),
                "{object}: {error}"
            );
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rejects_unsafe_names_without_mutating_workspace() {
        for (case_name, object, candidate) in [
            ("traversal", "Catalog.../Victim", "Victim.xml"),
            ("xml-name", "Catalog.Bad&Name", "Catalogs/Bad&Name.xml"),
        ] {
            let context = temp_context(case_name);
            let config_dir = context.cwd.join("src");
            fs::create_dir_all(config_dir.join("Catalogs")).unwrap();
            let config_path = config_dir.join("Configuration.xml");
            let config_before = configuration_bytes("SafeObject");
            fs::write(&config_path, &config_before).unwrap();
            let candidate_path = config_dir.join(candidate);
            if let Some(parent) = candidate_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let candidate_before = b"candidate-before".to_vec();
            fs::write(&candidate_path, &candidate_before).unwrap();

            let outcome = remove_metadata_object(&remove_args(&config_dir, object, true), &context);

            assert!(!outcome.ok, "{object}: {outcome:?}");
            assert_eq!(fs::read(&config_path).unwrap(), config_before, "{object}");
            assert_eq!(
                fs::read(&candidate_path).unwrap(),
                candidate_before,
                "{object}"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn meta_remove_removes_the_last_empty_type_collection_directory() {
        let context = temp_context("remove-last-type-collection");
        let config_dir = context.cwd.join("src");
        let init = create_configuration_scaffold(
            &Map::from_iter([
                ("Name".to_string(), json!("RemoveLastTypeCollection")),
                (
                    "OutputDir".to_string(),
                    json!(config_dir.display().to_string()),
                ),
            ]),
            &context,
        );
        assert!(init.ok, "{init:?}");
        let config_path = config_dir.join("Configuration.xml");
        let mut registration = CompileTransaction::new();
        registration
            .register_canonical_child(&config_path, "Catalog", "Victim")
            .unwrap();
        registration.commit().unwrap();
        let catalogs = config_dir.join("Catalogs");
        fs::create_dir_all(&catalogs).unwrap();
        fs::write(
            catalogs.join("Victim.xml"),
            utf8_bom_bytes(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>\n",
            ),
        )
        .unwrap();

        let outcome =
            remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", true), &context);

        assert!(outcome.ok, "{outcome:?}");
        assert!(
            !catalogs.exists(),
            "the platform removes an empty metadata type collection"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_post_write_failure_restores_all_owners_and_payloads() {
        let context = temp_context("atomic-rollback");
        let config_dir = context.cwd.join("src");
        let init = create_configuration_scaffold(
            &Map::from_iter([
                ("Name".to_string(), json!("AtomicRollback")),
                (
                    "OutputDir".to_string(),
                    json!(config_dir.display().to_string()),
                ),
            ]),
            &context,
        );
        assert!(init.ok, "{init:?}");
        let catalogs = config_dir.join("Catalogs");
        let object_xml = catalogs.join("Victim.xml");
        let object_dir = catalogs.join("Victim");
        let module = object_dir.join("Ext/ObjectModule.bsl");
        let subsystem = config_dir.join("Subsystems/Main.xml");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::create_dir_all(subsystem.parent().unwrap()).unwrap();

        let config_path = config_dir.join("Configuration.xml");
        let mut registration = CompileTransaction::new();
        assert_eq!(
            registration
                .register_canonical_child(&config_path, "Catalog", "Victim")
                .unwrap(),
            RegistrationStatus::Added
        );
        registration.commit().unwrap();
        let subsystem_outcome = compile_subsystem(
            &Map::from_iter([
                (
                    "OutputDir".to_string(),
                    json!(config_dir.display().to_string()),
                ),
                (
                    "Value".to_string(),
                    json!(json!({
                        "name": "Main",
                        "content": ["Catalog.Victim"]
                    })
                    .to_string()),
                ),
            ]),
            &context,
        );
        assert!(subsystem_outcome.ok, "{subsystem_outcome:?}");
        let config_before = fs::read(&config_path).unwrap();
        let subsystem_before = fs::read(&subsystem).unwrap();
        let object_before = utf8_bom_bytes(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>\n",
        );
        let module_before = b"// object module before\r\n".to_vec();
        fs::write(&object_xml, &object_before).unwrap();
        fs::write(&module, &module_before).unwrap();

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", true), &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("post-write validation"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&subsystem).unwrap(), subsystem_before);
        assert_eq!(fs::read(&object_xml).unwrap(), object_before);
        assert_eq!(fs::read(&module).unwrap(), module_before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rejects_newer_xml_anywhere_in_removed_tree_without_mutation() {
        let context = temp_context("newer-removed-tree");
        let config_dir = context.cwd.join("src");
        let init = create_configuration_scaffold(
            &Map::from_iter([
                ("Name".to_string(), json!("NewerRemovedTree")),
                (
                    "OutputDir".to_string(),
                    json!(config_dir.display().to_string()),
                ),
            ]),
            &context,
        );
        assert!(init.ok, "{init:?}");
        let config_path = config_dir.join("Configuration.xml");
        let mut registration = CompileTransaction::new();
        registration
            .register_canonical_child(&config_path, "Catalog", "Victim")
            .unwrap();
        registration.commit().unwrap();

        let object_path = config_dir.join("Catalogs/Victim.xml");
        let nested_form = config_dir.join("Catalogs/Victim/Forms/Main/Ext/Form.xml");
        fs::create_dir_all(nested_form.parent().unwrap()).unwrap();
        fs::write(
            &object_path,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Catalog/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            &nested_form,
            br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#,
        )
        .unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_before = fs::read(&object_path).unwrap();
        let nested_before = fs::read(&nested_form).unwrap();

        let outcome =
            remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", true), &context);

        assert!(!outcome.ok, "{outcome:?}");
        let diagnostics = outcome.errors.join("\n");
        assert!(diagnostics.contains("2.21"), "{diagnostics}");
        assert!(diagnostics.contains("1C 8.5"), "{diagnostics}");
        assert!(
            !diagnostics.contains("older than supported"),
            "{diagnostics}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&nested_form).unwrap(), nested_before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rolls_back_if_scanned_xml_changes_during_publication() {
        let context = temp_context("reference-xml-race");
        let config_dir = context.cwd.join("src");
        let init = create_configuration_scaffold(
            &Map::from_iter([
                ("Name".to_string(), json!("ReferenceRace")),
                (
                    "OutputDir".to_string(),
                    json!(config_dir.display().to_string()),
                ),
            ]),
            &context,
        );
        assert!(init.ok, "{init:?}");
        fs::create_dir_all(config_dir.join("Catalogs")).unwrap();
        fs::create_dir_all(config_dir.join("Documents")).unwrap();
        let config_path = config_dir.join("Configuration.xml");
        let object_path = config_dir.join("Catalogs/Victim.xml");
        let reference_path = config_dir.join("Documents/Reader.xml");
        let mut registration = CompileTransaction::new();
        assert_eq!(
            registration
                .register_canonical_child(&config_path, "Catalog", "Victim")
                .unwrap(),
            RegistrationStatus::Added
        );
        registration.commit().unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_before = utf8_bom_bytes(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>
"#,
        );
        let reference_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document><Properties><Name>Reader</Name><Comment>before</Comment></Properties><ChildObjects/></Document></MetaDataObject>"#.to_vec();
        let reference_concurrent = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document><Properties><Name>Reader</Name><Comment>concurrent</Comment></Properties><ChildObjects/></Document></MetaDataObject>"#.to_vec();
        fs::write(&object_path, &object_before).unwrap();
        fs::write(&reference_path, &reference_before).unwrap();
        let reference_for_hook = reference_path.clone();
        let concurrent_for_hook = reference_concurrent.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&reference_for_hook, &concurrent_for_hook).unwrap(),
            || remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", true), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&reference_path).unwrap(), reference_concurrent);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rejects_payload_directory_that_appears_after_absent_probe() {
        let context = temp_context("late-payload-directory");
        let (config_dir, config_path) = initialized_config_with_catalog(&context, "Victim");
        fs::create_dir_all(config_dir.join("Catalogs")).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_path = config_dir.join("Catalogs/Victim.xml");
        let object_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        let sibling_path = config_dir.join("Catalogs/Sibling.xml");
        let sibling_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Sibling</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        fs::write(&object_path, &object_before).unwrap();
        fs::write(&sibling_path, &sibling_before).unwrap();
        let late_module = config_dir.join("Catalogs/Victim/Ext/ObjectModule.bsl");
        let late_module_for_hook = late_module.clone();

        let outcome = with_before_commit_hook(
            move |_| {
                fs::create_dir_all(late_module_for_hook.parent().unwrap()).unwrap();
                fs::write(&late_module_for_hook, b"// late payload\r\n").unwrap();
            },
            || remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", false), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("pair member"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&sibling_path).unwrap(), sibling_before);
        assert_eq!(fs::read(&late_module).unwrap(), b"// late payload\r\n");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rejects_descriptor_that_appears_after_absent_probe() {
        let context = temp_context("late-descriptor");
        let (config_dir, config_path) = initialized_config_with_catalog(&context, "Victim");
        let object_dir = config_dir.join("Catalogs/Victim");
        fs::create_dir_all(object_dir.join("Ext")).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let module_path = object_dir.join("Ext/ObjectModule.bsl");
        let module_before = b"// victim payload\r\n".to_vec();
        let sibling_path = config_dir.join("Catalogs/Sibling.xml");
        let sibling_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Sibling</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        fs::write(&module_path, &module_before).unwrap();
        fs::write(&sibling_path, &sibling_before).unwrap();
        let late_descriptor = config_dir.join("Catalogs/Victim.xml");
        let late_descriptor_bytes = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        let descriptor_for_hook = late_descriptor.clone();
        let descriptor_bytes_for_hook = late_descriptor_bytes.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&descriptor_for_hook, &descriptor_bytes_for_hook).unwrap(),
            || remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", false), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("pair member"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&module_path).unwrap(), module_before);
        assert_eq!(fs::read(&sibling_path).unwrap(), sibling_before);
        assert_eq!(fs::read(&late_descriptor).unwrap(), late_descriptor_bytes);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rolls_back_if_scanned_bsl_changes_during_publication() {
        let context = temp_context("reference-bsl-race");
        let (config_dir, config_path) = initialized_config_with_catalog(&context, "Victim");
        fs::create_dir_all(config_dir.join("Catalogs")).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_path = config_dir.join("Catalogs/Victim.xml");
        let object_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        let reader_module = config_dir.join("CommonModules/Reader/Ext/Module.bsl");
        let reader_before = b"// no references\r\n".to_vec();
        let reader_concurrent = b"Value = Catalogs.Victim.FindByCode(Code);\r\n".to_vec();
        fs::create_dir_all(reader_module.parent().unwrap()).unwrap();
        fs::write(&object_path, &object_before).unwrap();
        fs::write(&reader_module, &reader_before).unwrap();
        let reader_for_hook = reader_module.clone();
        let concurrent_for_hook = reader_concurrent.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&reader_for_hook, &concurrent_for_hook).unwrap(),
            || remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", false), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&reader_module).unwrap(), reader_concurrent);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rolls_back_if_reference_scan_topology_changes_during_publication() {
        let context = temp_context("reference-topology-race");
        let (config_dir, config_path) = initialized_config_with_catalog(&context, "Victim");
        fs::create_dir_all(config_dir.join("Catalogs")).unwrap();
        fs::create_dir_all(config_dir.join("CommonModules")).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_path = config_dir.join("Catalogs/Victim.xml");
        let object_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        fs::write(&object_path, &object_before).unwrap();
        let late_module = config_dir.join("CommonModules/LateReader/Ext/Module.bsl");
        let late_module_for_hook = late_module.clone();

        let outcome = with_before_commit_hook(
            move |_| {
                fs::create_dir_all(late_module_for_hook.parent().unwrap()).unwrap();
                fs::write(
                    &late_module_for_hook,
                    b"Value = Catalogs.Victim.FindByCode(Code);\r\n",
                )
                .unwrap();
            },
            || remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", false), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("directory membership guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert!(late_module.is_file());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rolls_back_if_reference_scan_entry_changes_from_file_to_directory() {
        let context = temp_context("reference-entry-kind-race");
        let (config_dir, config_path) = initialized_config_with_catalog(&context, "Victim");
        fs::create_dir_all(config_dir.join("Catalogs")).unwrap();
        let common_modules = config_dir.join("CommonModules");
        fs::create_dir_all(&common_modules).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_path = config_dir.join("Catalogs/Victim.xml");
        let object_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        fs::write(&object_path, &object_before).unwrap();
        let topology_entry = common_modules.join("LateReader");
        fs::write(&topology_entry, b"irrelevant regular file").unwrap();
        let late_module = topology_entry.join("Ext/Module.bsl");
        let topology_entry_for_hook = topology_entry.clone();
        let late_module_for_hook = late_module.clone();

        let outcome = with_before_commit_hook(
            move |_| {
                fs::remove_file(&topology_entry_for_hook).unwrap();
                fs::create_dir_all(late_module_for_hook.parent().unwrap()).unwrap();
                fs::write(
                    &late_module_for_hook,
                    b"Value = Catalogs.Victim.FindByCode(Code);\r\n",
                )
                .unwrap();
            },
            || remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", false), &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("directory membership guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert!(late_module.is_file());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_fails_closed_when_reference_bsl_is_not_utf8() {
        let context = temp_context("invalid-reference-bsl");
        let (config_dir, config_path) = initialized_config_with_catalog(&context, "Victim");
        fs::create_dir_all(config_dir.join("Catalogs")).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_path = config_dir.join("Catalogs/Victim.xml");
        let object_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        let invalid_module = config_dir.join("CommonModules/Unreadable/Ext/Module.bsl");
        let invalid_bytes = vec![0xff, 0xfe, 0xfd];
        fs::create_dir_all(invalid_module.parent().unwrap()).unwrap();
        fs::write(&object_path, &object_before).unwrap();
        fs::write(&invalid_module, &invalid_bytes).unwrap();

        let outcome =
            remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", false), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("not valid UTF-8"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&invalid_module).unwrap(), invalid_bytes);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_fails_closed_on_reference_scan_symlink() {
        let context = temp_context("reference-symlink");
        let (config_dir, config_path) = initialized_config_with_catalog(&context, "Victim");
        fs::create_dir_all(config_dir.join("Catalogs")).unwrap();
        let config_before = fs::read(&config_path).unwrap();
        let object_path = config_dir.join("Catalogs/Victim.xml");
        let object_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        fs::write(&object_path, &object_before).unwrap();
        let dangling_link = config_dir.join("CommonModules");
        crate::infrastructure::platform::filesystem::create_test_directory_link(
            &config_dir.join("missing-reference-tree"),
            &dangling_link,
        )
        .unwrap();

        let outcome =
            remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", false), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("symbolic link"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert!(fs::symlink_metadata(&dangling_link)
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_remove_rejects_payload_directory_symlink_before_traversal() {
        let context = temp_context("payload-directory-symlink");
        let (config_dir, config_path) = initialized_config_with_catalog(&context, "Victim");
        let catalogs = config_dir.join("Catalogs");
        fs::create_dir_all(&catalogs).unwrap();
        let object_path = catalogs.join("Victim.xml");
        let object_before = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Victim</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#.to_vec();
        fs::write(&object_path, &object_before).unwrap();
        let external = context.cwd.join("external-payload");
        let external_marker = external.join("must-not-be-traversed.txt");
        fs::create_dir_all(&external).unwrap();
        fs::write(&external_marker, b"external").unwrap();
        let payload_link = catalogs.join("Victim");
        crate::infrastructure::platform::filesystem::create_test_directory_link(
            &external,
            &payload_link,
        )
        .unwrap();
        let config_before = fs::read(&config_path).unwrap();

        let outcome =
            remove_metadata_object(&remove_args(&config_dir, "Catalog.Victim", false), &context);

        assert!(!outcome.ok, "{outcome:?}");
        let error = outcome.errors.join("\n");
        assert!(
            error.contains(
                "metadata payload directory must not be a symbolic link or reparse point"
            ),
            "{error}"
        );
        assert!(
            !error.contains("reference scan entry"),
            "payload link must be rejected by preflight, before the reference scanner: {error}"
        );
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&external_marker).unwrap(), b"external");
        assert!(fs::symlink_metadata(&payload_link)
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn reference_scan_entry_budget_stops_incrementally_at_a_test_limit() {
        let context = temp_context("reference-entry-budget");
        let root = context.cwd.join("scan");
        fs::create_dir(&root).unwrap();
        for name in ["A.xml", "B.xml", "C.xml"] {
            fs::write(root.join(name), b"<Root/>").unwrap();
        }
        let mut visited_directories = std::collections::HashSet::new();
        let mut visited_entries = 0usize;

        let result = metadata_files_recursive_bounded(
            &root,
            0,
            MetaRemoveTraversalLimits {
                max_depth: 4,
                max_entries: 1,
            },
            &mut visited_directories,
            &mut visited_entries,
        );
        let error = match result {
            Ok(_) => panic!("the injected one-entry budget must stop the scan"),
            Err(error) => error,
        };

        assert!(error.contains("maximum of 1 entries"), "{error}");
        assert_eq!(
            visited_entries, 1,
            "the scanner must stop before retaining or inspecting entries beyond the budget"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn reference_scan_depth_budget_stops_before_recursive_descent() {
        let context = temp_context("reference-depth-budget");
        let root = context.cwd.join("scan");
        fs::create_dir_all(root.join("Level1/Level2")).unwrap();
        fs::write(root.join("Level1/Level2/deep.xml"), b"<Root/>").unwrap();

        let result = metadata_files_recursive_with_limits(
            &root,
            MetaRemoveTraversalLimits {
                max_depth: 1,
                max_entries: 8,
            },
        );
        let error = match result {
            Ok(_) => panic!("the injected depth budget must reject the deeper directory"),
            Err(error) => error,
        };

        assert!(error.contains("maximum depth of 1"), "{error}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn reference_scan_rejects_a_direct_symlink_before_any_recursive_descent() {
        let context = temp_context("reference-symlink-before-recursion");
        let root = context.cwd.join("scan");
        fs::create_dir_all(root.join("A-directory")).unwrap();
        crate::infrastructure::platform::filesystem::create_test_directory_link(
            &context.cwd.join("external"),
            &root.join("Z-symlink-directory"),
        )
        .unwrap();

        let result = metadata_files_recursive_with_limits(
            &root,
            MetaRemoveTraversalLimits {
                max_depth: 0,
                max_entries: 8,
            },
        );
        let error = match result {
            Ok(_) => {
                panic!("a direct symlink must fail before descending into the regular directory")
            }
            Err(error) => error,
        };

        assert!(error.contains("symbolic link or reparse point"), "{error}");
        assert!(
            !error.contains("maximum depth"),
            "direct symlink rejection must retain precedence over recursive depth failure: {error}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    fn subsystem_descriptor_bytes(name: &str) -> Vec<u8> {
        format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Subsystem><Properties><Name>{name}</Name></Properties><ChildObjects><Content/></ChildObjects></Subsystem></MetaDataObject>"
        )
        .into_bytes()
    }

    fn plan_subsystem_replacements_for_test(root: &Path) -> Result<(), String> {
        let mut replacements = Vec::new();
        let mut descriptor_reads = Vec::new();
        plan_meta_remove_subsystem_replacements(
            root,
            "Catalog.Victim",
            &mut replacements,
            &mut descriptor_reads,
        )
    }

    fn plan_subsystem_replacements_with_limits_for_test(
        root: &Path,
        limits: MetaRemoveTraversalLimits,
    ) -> (Result<(), String>, usize) {
        let mut replacements = Vec::new();
        let mut descriptor_reads = Vec::new();
        let mut visited_directories = HashSet::new();
        let mut visited_entries = 0usize;
        let result = plan_meta_remove_subsystem_replacements_bounded(
            root,
            "Catalog.Victim",
            &mut replacements,
            &mut descriptor_reads,
            0,
            limits,
            &mut visited_directories,
            &mut visited_entries,
        );
        (result, visited_entries)
    }

    #[test]
    fn subsystem_planner_rejects_forced_reparse_at_every_inspection_point() {
        for point in ["root", "entry", "child"] {
            let context = temp_context(&format!("subsystem-reparse-{point}"));
            let root = context.cwd.join("Subsystems");
            let descriptor = root.join("Parent.xml");
            let child = root.join("Parent/Subsystems");
            fs::create_dir_all(&child).unwrap();
            fs::write(&descriptor, subsystem_descriptor_bytes("Parent")).unwrap();
            let forced = match point {
                "root" => root.clone(),
                "entry" => descriptor.clone(),
                "child" => child.clone(),
                _ => unreachable!(),
            };

            let error = with_meta_remove_forced_reparse_paths([forced], || {
                plan_subsystem_replacements_for_test(&root)
            })
            .expect_err("every subsystem planner inspection point must fail closed on reparse");

            assert!(
                error.contains("symbolic link or reparse point"),
                "{point}: {error}"
            );
            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn subsystem_planner_rejects_reparse_injected_at_child_inspection_window() {
        use std::cell::Cell;
        use std::rc::Rc;

        let context = temp_context("subsystem-reparse-race");
        let root = context.cwd.join("Subsystems");
        let descriptor = root.join("Parent.xml");
        let child = root.join("Parent/Subsystems");
        fs::create_dir_all(&child).unwrap();
        fs::write(&descriptor, subsystem_descriptor_bytes("Parent")).unwrap();
        let expected_child = child.clone();
        let injected = Rc::new(Cell::new(false));
        let injected_for_hook = Rc::clone(&injected);

        let error = with_meta_remove_forced_reparse_paths(Vec::new(), || {
            with_before_meta_remove_subsystem_child_inspection_hook(
                move |inspected| {
                    assert_eq!(inspected, expected_child);
                    injected_for_hook.set(true);
                    force_meta_remove_reparse_path(inspected.to_path_buf());
                },
                || plan_subsystem_replacements_for_test(&root),
            )
        })
        .expect_err("a reparse injected at the child inspection window must fail closed");

        assert!(
            injected.get(),
            "test hook must cover the child inspection window"
        );
        assert!(error.contains("symbolic link or reparse point"), "{error}");
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_planner_rejects_descent_beyond_meta_remove_depth_budget() {
        let context = temp_context("subsystem-depth-budget");
        let mut directory = context.cwd.join("Subsystems");
        fs::create_dir_all(&directory).unwrap();
        for depth in 0..=1 {
            let name = format!("Nested{depth}");
            fs::write(
                directory.join(format!("{name}.xml")),
                subsystem_descriptor_bytes(&name),
            )
            .unwrap();
            directory = directory.join(name).join("Subsystems");
            fs::create_dir_all(&directory).unwrap();
        }

        let (result, _) = plan_subsystem_replacements_with_limits_for_test(
            &context.cwd.join("Subsystems"),
            MetaRemoveTraversalLimits {
                max_depth: 1,
                max_entries: 8,
            },
        );
        let error = result
            .expect_err("subsystem recursion beyond the meta.remove depth budget must fail closed");

        assert!(
            error.contains("subsystem traversal exceeded the maximum depth"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn subsystem_planner_stops_before_retaining_entries_beyond_meta_remove_budget() {
        let context = temp_context("subsystem-entry-budget");
        let root = context.cwd.join("Subsystems");
        fs::create_dir(&root).unwrap();
        for name in ["A.txt", "B.txt"] {
            fs::write(root.join(name), b"not a subsystem descriptor").unwrap();
        }

        let (result, visited_entries) = plan_subsystem_replacements_with_limits_for_test(
            &root,
            MetaRemoveTraversalLimits {
                max_depth: 4,
                max_entries: 1,
            },
        );
        let error = result
            .expect_err("subsystem traversal must stop before retaining entries beyond the budget");

        assert!(
            error.contains("subsystem traversal exceeded the maximum of 1 entries"),
            "{error}"
        );
        assert_eq!(
            visited_entries, 1,
            "the subsystem planner must stop before retaining or inspecting entries beyond the budget"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use crate::application::UnicaApplication;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::compile_transaction::{
        with_commit_failpoint, CommitFailpoint,
    };
    use crate::infrastructure::native_operations::single_file_publisher::with_before_commit_hook;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-meta-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn canonical_path(path: &Path) -> PathBuf {
        crate::infrastructure::source_roots::normalize_path_identity(path).unwrap()
    }

    const TEST_MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    const TEST_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";
    const TEST_XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";

    fn write_owner(
        source_dir: &Path,
        object_type: &str,
        object_name: &str,
        languages: &[&str],
    ) -> PathBuf {
        fs::create_dir_all(source_dir.join("Languages")).unwrap();
        let language_nodes = languages
            .iter()
            .map(|name| format!("<Language>{name}</Language>"))
            .collect::<String>();
        let configuration = format!(
            r#"<MetaDataObject xmlns="{TEST_MD_NS}" version="2.20">
<Configuration uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Owner</Name></Properties>
<ChildObjects>{language_nodes}<{object_type}>{object_name}</{object_type}></ChildObjects>
</Configuration></MetaDataObject>"#
        );
        fs::write(source_dir.join("Configuration.xml"), configuration).unwrap();
        source_dir.to_path_buf()
    }

    fn meta_validate_args(path: &Path) -> Map<String, Value> {
        Map::from_iter([
            (
                "ObjectPath".to_string(),
                Value::String(path.display().to_string()),
            ),
            ("Detailed".to_string(), Value::Bool(true)),
        ])
    }

    fn sample_meta_named(object_type: &str, object_name: &str) -> String {
        sample_meta_object_xml(object_type, object_name, "", "\t\t<ChildObjects/>")
    }

    fn sample_document_xml(register_records: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Document uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>SampleShipment</Name>
			<Synonym/>
			<Comment/>
			{register_records}
			<PostInPrivilegedMode>true</PostInPrivilegedMode>
			<UnpostInPrivilegedMode>true</UnpostInPrivilegedMode>
		</Properties>
		<ChildObjects/>
	</Document>
</MetaDataObject>
"#
        )
    }

    fn sample_meta_object_xml(
        object_type: &str,
        object_name: &str,
        extra_properties: &str,
        child_objects: &str,
    ) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<{object_type} uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>{object_name}</Name>
			<Synonym/>
			<Comment/>
{extra_properties}
		</Properties>
{child_objects}
	</{object_type}>
</MetaDataObject>
"#
        )
    }

    fn sample_register_xml(object_type: &str) -> String {
        sample_meta_object_xml(object_type, "SampleStock", "", "\t\t<ChildObjects/>")
    }

    fn sample_enum_xml() -> String {
        sample_meta_object_xml("Enum", "SampleStatuses", "", "\t\t<ChildObjects/>")
    }

    fn sample_catalog_xml() -> String {
        sample_meta_object_xml(
            "Catalog",
            "SampleContracts",
            "\t\t\t<Owners/>\n\t\t\t<InputByString/>\n\t\t\t<BasedOn/>",
            "\t\t<ChildObjects/>",
        )
    }

    fn sample_document_journal_xml() -> String {
        sample_meta_object_xml(
            "DocumentJournal",
            "SampleJournal",
            "",
            "\t\t<ChildObjects/>",
        )
    }

    fn sample_document_with_child_objects(child_objects: &str) -> String {
        sample_document_xml("<RegisterRecords/>").replace(
            "\t\t<ChildObjects/>",
            &format!("\t\t<ChildObjects>\n{child_objects}\n\t\t</ChildObjects>"),
        )
    }

    fn sample_attribute(name: &str, type_xml: &str, fill_value_xml: &str) -> String {
        format!(
            "\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>{name}</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
{type_xml}
\t\t\t\t\t<PasswordMode>false</PasswordMode>
\t\t\t\t\t<Format/>
\t\t\t\t\t<EditFormat/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<MarkNegatives>false</MarkNegatives>
\t\t\t\t\t<Mask/>
\t\t\t\t\t<MultiLine>false</MultiLine>
\t\t\t\t\t<ExtendedEdit>false</ExtendedEdit>
\t\t\t\t\t<MinValue xsi:nil=\"true\"/>
\t\t\t\t\t<MaxValue xsi:nil=\"true\"/>
\t\t\t\t\t<FillFromFillingValue>false</FillFromFillingValue>
{fill_value_xml}
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t\t<Indexing>DontIndex</Indexing>
\t\t\t\t</Properties>
\t\t\t</Attribute>"
        )
    }

    fn sample_object_with_tabular_fill_value(object_type: &str) -> String {
        sample_meta_object_xml(
            object_type,
            "SampleObject",
            "",
            "\t\t<ChildObjects>
\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects>
\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>Status</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type>
\t\t\t\t\t\t\t\t<v8:Type>cfg:EnumRef.SampleStatus</v8:Type>
\t\t\t\t\t\t\t</Type>
\t\t\t\t\t\t\t<FillValue xsi:type=\"xr:DesignTimeRef\">Enum.SampleStatus.EnumValue.Default</FillValue>
\t\t\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t</ChildObjects>
\t\t\t</TabularSection>
\t\t</ChildObjects>",
        )
    }

    fn sample_object_with_line_number_length(
        object_type: &str,
        line_number_length: &str,
    ) -> String {
        sample_meta_object_xml(
            object_type,
            "SampleObject",
            "",
            &format!(
                "\t\t<ChildObjects>
\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t\t<LineNumberLength>{line_number_length}</LineNumberLength>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects/>
\t\t\t</TabularSection>
\t\t</ChildObjects>"
            ),
        )
    }

    fn write_owner_with_compatibility(
        source_dir: &Path,
        object_type: &str,
        object_name: &str,
        compatibility_mode: &str,
    ) {
        fs::create_dir_all(source_dir).unwrap();
        write_file(
            &source_dir.join("Configuration.xml"),
            &format!(
                r#"<MetaDataObject xmlns="{TEST_MD_NS}" version="2.20">
<Configuration uuid="11111111-1111-4111-8111-111111111111">
<Properties>
<Name>Owner</Name>
<CompatibilityMode>{compatibility_mode}</CompatibilityMode>
</Properties>
<ChildObjects><{object_type}>{object_name}</{object_type}></ChildObjects>
</Configuration>
</MetaDataObject>"#
            ),
        );
    }

    fn register_record_args(object_path: &Path) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!(object_path.display().to_string()),
        );
        args.insert("Operation".to_string(), json!("add-registerRecord"));
        args.insert(
            "Value".to_string(),
            json!("AccumulationRegister.SampleUnshippedGoods"),
        );
        args
    }

    fn meta_edit_args(object_path: &Path, operation: &str, value: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!(object_path.display().to_string()),
        );
        args.insert("Operation".to_string(), json!(operation));
        args.insert("Value".to_string(), json!(value));
        args
    }

    fn meta_edit_definition_args(object_path: &Path, definition_path: &Path) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!(object_path.display().to_string()),
        );
        args.insert(
            "DefinitionFile".to_string(),
            json!(definition_path.display().to_string()),
        );
        args
    }

    fn sample_catalog_with_autonumbering(value: &str) -> String {
        sample_catalog_xml().replace(
            "\t\t\t<Owners/>",
            &format!("\t\t\t<Autonumbering>{value}</Autonumbering>\n\t\t\t<Owners/>"),
        )
    }

    fn boolean_contract_cases() -> &'static [(&'static str, &'static [&'static str])] {
        &[
            (
                "AccountingFlag",
                &[
                    "PasswordMode",
                    "MarkNegatives",
                    "MultiLine",
                    "ExtendedEdit",
                    "FillFromFillingValue",
                ],
            ),
            (
                "AccountingRegister",
                &[
                    "UseStandardCommands",
                    "IncludeHelpInContents",
                    "Correspondence",
                    "EnableTotalsSplitting",
                ],
            ),
            (
                "AccumulationRegister",
                &[
                    "UseStandardCommands",
                    "IncludeHelpInContents",
                    "EnableTotalsSplitting",
                ],
            ),
            (
                "AddressingAttribute",
                &[
                    "PasswordMode",
                    "MarkNegatives",
                    "MultiLine",
                    "ExtendedEdit",
                    "FillFromFillingValue",
                ],
            ),
            (
                "Attribute",
                &[
                    "PasswordMode",
                    "MarkNegatives",
                    "MultiLine",
                    "ExtendedEdit",
                    "FillFromFillingValue",
                ],
            ),
            (
                "BusinessProcess",
                &[
                    "UseStandardCommands",
                    "CheckUnique",
                    "Autonumbering",
                    "CreateTaskInPrivilegedMode",
                    "IncludeHelpInContents",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            (
                "CalculationRegister",
                &[
                    "UseStandardCommands",
                    "ActionPeriod",
                    "BasePeriod",
                    "IncludeHelpInContents",
                ],
            ),
            (
                "Catalog",
                &[
                    "Hierarchical",
                    "LimitLevelCount",
                    "FoldersOnTop",
                    "UseStandardCommands",
                    "CheckUnique",
                    "Autonumbering",
                    "QuickChoice",
                    "IncludeHelpInContents",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            (
                "ChartOfAccounts",
                &[
                    "UseStandardCommands",
                    "IncludeHelpInContents",
                    "CheckUnique",
                    "QuickChoice",
                    "AutoOrderByCode",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            (
                "ChartOfCalculationTypes",
                &[
                    "UseStandardCommands",
                    "QuickChoice",
                    "ActionPeriodUse",
                    "IncludeHelpInContents",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            (
                "ChartOfCharacteristicTypes",
                &[
                    "UseStandardCommands",
                    "IncludeHelpInContents",
                    "Hierarchical",
                    "FoldersOnTop",
                    "CheckUnique",
                    "Autonumbering",
                    "QuickChoice",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            ("Command", &["ModifiesData"]),
            (
                "CommonModule",
                &[
                    "Global",
                    "ClientManagedApplication",
                    "Server",
                    "ExternalConnection",
                    "ClientOrdinaryApplication",
                    "Client",
                    "ServerCall",
                    "Privileged",
                ],
            ),
            (
                "Constant",
                &[
                    "UseStandardCommands",
                    "PasswordMode",
                    "MarkNegatives",
                    "MultiLine",
                    "ExtendedEdit",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            (
                "DataProcessor",
                &["UseStandardCommands", "IncludeHelpInContents"],
            ),
            (
                "Dimension",
                &[
                    "PasswordMode",
                    "MarkNegatives",
                    "MultiLine",
                    "ExtendedEdit",
                    "DenyIncompleteValues",
                    "BaseDimension",
                    "UseInTotals",
                    "FillFromFillingValue",
                    "Master",
                    "MainFilter",
                    "Balance",
                ],
            ),
            (
                "DocumentJournal",
                &["UseStandardCommands", "IncludeHelpInContents"],
            ),
            (
                "Document",
                &[
                    "UseStandardCommands",
                    "CheckUnique",
                    "Autonumbering",
                    "PostInPrivilegedMode",
                    "UnpostInPrivilegedMode",
                    "IncludeHelpInContents",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            ("Enum", &["UseStandardCommands", "QuickChoice"]),
            (
                "ExchangePlan",
                &[
                    "UseStandardCommands",
                    "QuickChoice",
                    "DistributedInfoBase",
                    "IncludeConfigurationExtensions",
                    "IncludeHelpInContents",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            (
                "ExtDimensionAccountingFlag",
                &[
                    "PasswordMode",
                    "MarkNegatives",
                    "MultiLine",
                    "ExtendedEdit",
                    "FillFromFillingValue",
                ],
            ),
            (
                "InformationRegister",
                &[
                    "UseStandardCommands",
                    "MainFilterOnPeriod",
                    "IncludeHelpInContents",
                    "EnableTotalsSliceFirst",
                    "EnableTotalsSliceLast",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
            ("Operation", &["Nillable", "Transactioned"]),
            ("Parameter", &["Nillable"]),
            ("Report", &["UseStandardCommands", "IncludeHelpInContents"]),
            (
                "Resource",
                &[
                    "PasswordMode",
                    "MarkNegatives",
                    "MultiLine",
                    "ExtendedEdit",
                    "Balance",
                    "FillFromFillingValue",
                ],
            ),
            ("ScheduledJob", &["Use", "Predefined"]),
            (
                "Task",
                &[
                    "UseStandardCommands",
                    "CheckUnique",
                    "Autonumbering",
                    "IncludeHelpInContents",
                    "UpdateDataHistoryImmediatelyAfterWrite",
                    "ExecuteAfterWriteDataHistoryVersionProcessing",
                ],
            ),
        ]
    }

    fn sample_with_invalid_boolean_node(node_type: &str, property: &str) -> String {
        if META_COMPILE_SUPPORTED_TYPES.contains(&node_type) {
            return sample_meta_object_xml(
                node_type,
                "BooleanProbe",
                &format!("\t\t\t<{property}>banana</{property}>"),
                "\t\t<ChildObjects/>",
            );
        }

        let (root_type, child) = match node_type {
            "AccountingFlag" | "ExtDimensionAccountingFlag" => ("ChartOfAccounts", node_type),
            "AddressingAttribute" => ("Task", node_type),
            "Dimension" | "Resource" => ("InformationRegister", node_type),
            "Operation" | "Parameter" => ("WebService", node_type),
            _ => ("Catalog", node_type),
        };
        let invalid_child = format!(
            "\t\t\t<{child}>\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>BooleanChild</Name>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<{property}>banana</{property}>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects/>\n\t\t\t</{child}>"
        );
        sample_meta_object_xml(
            root_type,
            "BooleanProbe",
            "",
            &format!("\t\t<ChildObjects>\n{invalid_child}\n\t\t</ChildObjects>"),
        )
    }

    #[test]
    fn edit_meta_rejects_invalid_boolean_inline_without_writing() {
        let context = temp_context("invalid-boolean-inline");
        let object_path = context.cwd.join("Catalogs/BooleanProbe.xml");
        let original = sample_catalog_with_autonumbering("true");

        for invalid in ["banana", "1", "0", "yes"] {
            write_file(&object_path, &original);
            let before = fs::read(&object_path).unwrap();
            let outcome = edit_meta(
                &meta_edit_args(
                    &object_path,
                    "modify-property",
                    &format!("Autonumbering={invalid}"),
                ),
                &context,
            );

            assert!(!outcome.ok, "{invalid}: {outcome:?}");
            assert!(
                outcome.errors.iter().any(|error| {
                    error.contains("Autonumbering")
                        && error.contains("xs:boolean")
                        && error.contains("8.3.27")
                }),
                "{invalid}: {:?}",
                outcome.errors
            );
            assert_eq!(fs::read(&object_path).unwrap(), before, "{invalid}");
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Verifies inline previews expose the exact projected XML without writing it.
    #[test]
    fn preview_meta_edit_reports_projected_inline_diff_without_writing() {
        let context = temp_context("preview-inline-change");
        let object_path = context.cwd.join("Catalogs/Preview.xml");
        write_file(&object_path, &sample_catalog_xml());
        let before = fs::read(&object_path).unwrap();

        let execution = preview_meta_edit_with_data(
            &meta_edit_args(&object_path, "modify-property", "Comment=Previewed"),
            &context,
        );
        let outcome = &execution.outcome;

        assert!(outcome.ok, "{outcome:?}");
        assert!(outcome.summary.contains("planned native metadata edit"));
        assert_eq!(
            outcome.changes,
            vec![format!("would update {}", object_path.display())]
        );
        let data = execution.data.as_ref().unwrap();
        assert!(data.changed);
        assert_eq!(data.counts.modified, 1);
        let diff = data.diff.as_deref().unwrap_or_default();
        assert!(diff.contains("--- a/"), "{diff}");
        assert!(diff.contains("+++ b/"), "{diff}");
        assert!(diff.contains("-\t\t\t<Comment/>"), "{diff}");
        assert!(
            diff.contains("+\t\t\t<Comment>Previewed</Comment>"),
            "{diff}"
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Verifies a missing preview target remains a useful dry-run failure.
    #[test]
    fn preview_meta_edit_rejects_missing_object() {
        let context = temp_context("preview-missing-object");
        let object_path = context.cwd.join("Catalogs/Missing.xml");

        let outcome = preview_meta_edit(
            &meta_edit_args(&object_path, "modify-property", "Comment=Previewed"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(outcome.summary.starts_with("dry run:"), "{outcome:?}");
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("Object file not found:")));
        assert!(outcome.changes.is_empty());
        assert!(outcome.artifacts.is_empty());

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Verifies DefinitionFile previews use the same exact diff contract.
    #[test]
    fn preview_meta_edit_reports_definition_file_diff_without_writing() {
        let context = temp_context("preview-definition-file");
        let object_path = context.cwd.join("Catalogs/Preview.xml");
        let definition_path = context.cwd.join("meta-edit.json");
        write_file(&object_path, &sample_catalog_xml());
        write_file(
            &definition_path,
            &json!({"modify": {"properties": {"Comment": "Defined"}}}).to_string(),
        );
        let before = fs::read(&object_path).unwrap();

        let execution = preview_meta_edit_with_data(
            &meta_edit_definition_args(&object_path, &definition_path),
            &context,
        );
        let outcome = &execution.outcome;

        assert!(outcome.ok, "{outcome:?}");
        let diff = execution
            .data
            .as_ref()
            .unwrap()
            .diff
            .as_deref()
            .unwrap_or_default();
        assert!(diff.contains("--- a/"), "{diff}");
        assert!(diff.contains("-\t\t\t<Comment/>"), "{diff}");
        assert!(diff.contains("+\t\t\t<Comment>Defined</Comment>"), "{diff}");
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Verifies byte-identical previews report a no-op and omit the diff.
    #[test]
    fn preview_meta_edit_reports_no_file_changes_without_diff() {
        let context = temp_context("preview-no-op");
        let object_path = context.cwd.join("Catalogs/Preview.xml");
        let xml = sample_catalog_xml().replace("<Comment/>", "<Comment>Unchanged</Comment>");
        write_file(&object_path, &xml);
        let before = fs::read(&object_path).unwrap();

        let execution = preview_meta_edit_with_data(
            &meta_edit_args(&object_path, "modify-property", "Comment=Unchanged"),
            &context,
        );
        let outcome = &execution.outcome;

        assert!(outcome.ok, "{outcome:?}");
        assert!(
            outcome.summary.contains("found no metadata changes"),
            "{outcome:?}"
        );
        assert!(outcome.changes.is_empty());
        // ADR-0023: "nothing changed" is a value, and no diff means no diff.
        let data = execution.data.as_ref().unwrap();
        assert!(!data.changed);
        assert!(data.diff.is_none());
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    /// Verifies a renderer fault does not turn a valid metadata edit into a failure.
    #[test]
    fn projected_diff_render_failure_becomes_warning() {
        let mut warnings = Vec::new();

        let diff =
            meta_edit_projected_diff(&mut warnings, Err("synthetic renderer failure".to_string()));

        assert!(diff.is_none());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("synthetic renderer failure"));
    }

    #[test]
    fn edit_meta_rejects_invalid_boolean_definition_map_without_writing() {
        let context = temp_context("invalid-boolean-definition");
        let object_path = context.cwd.join("Catalogs/BooleanProbe.xml");
        let definition_path = context.cwd.join("edit.json");
        write_file(&object_path, &sample_catalog_with_autonumbering("true"));
        write_file(
            &definition_path,
            &json!({"modify": {"properties": {"Autonumbering": "banana"}}}).to_string(),
        );
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_definition_args(&object_path, &definition_path),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("Autonumbering")
                    && error.contains("xs:boolean")
                    && error.contains("8.3.27")
            }),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_normalizes_valid_boolean_values_to_canonical_xml() {
        let context = temp_context("valid-boolean-canonical");
        let object_path = context.cwd.join("Catalogs/BooleanProbe.xml");
        let definition_path = context.cwd.join("edit.json");

        for (raw, expected) in [("TRUE", "true"), ("false", "false")] {
            write_file(&object_path, &sample_catalog_with_autonumbering("true"));
            let outcome = edit_meta(
                &meta_edit_args(
                    &object_path,
                    "modify-property",
                    &format!("Autonumbering={raw}"),
                ),
                &context,
            );
            let updated = fs::read_to_string(&object_path).unwrap();
            assert!(outcome.ok, "{raw}: {outcome:?}");
            assert!(
                updated.contains(&format!("<Autonumbering>{expected}</Autonumbering>")),
                "{raw}: {updated}"
            );
        }

        for (raw, expected) in [(true, "true"), (false, "false")] {
            write_file(&object_path, &sample_catalog_with_autonumbering("false"));
            write_file(
                &definition_path,
                &json!({"modify": {"properties": {"Autonumbering": raw}}}).to_string(),
            );
            let outcome = edit_meta(
                &meta_edit_definition_args(&object_path, &definition_path),
                &context,
            );
            let updated = fs::read_to_string(&object_path).unwrap();
            assert!(outcome.ok, "{raw}: {outcome:?}");
            assert!(
                updated.contains(&format!("<Autonumbering>{expected}</Autonumbering>")),
                "{raw}: {updated}"
            );
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_unrelated_edit_when_existing_boolean_is_invalid() {
        let context = temp_context("existing-invalid-boolean");
        let object_path = context.cwd.join("Catalogs/BooleanProbe.xml");
        write_file(&object_path, &sample_catalog_with_autonumbering("banana"));
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Comment=changed"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("Autonumbering")
                    && error.contains("xs:boolean")
                    && error.contains("8.3.27")
            }),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_unrelated_edit_when_existing_enum_is_invalid() {
        let context = temp_context("existing-invalid-enum");
        let object_path = context.cwd.join("Catalogs/EnumProbe.xml");
        let original = sample_catalog_xml().replace(
            "\t\t\t<Owners/>",
            "\t\t\t<HierarchyType>Bogus</HierarchyType>\n\t\t\t<Owners/>",
        );
        write_file(&object_path, &original);
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Comment=changed"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("HierarchyType")
                    && error.contains("Bogus")
                    && error.contains("8.3.27")
            }),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_checks_existing_child_enum_before_unrelated_edit() {
        let context = temp_context("existing-invalid-child-enum");
        let object_path = context.cwd.join("Documents/EnumProbe.xml");
        let attribute = sample_attribute(
            "ProbeAttribute",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
        )
        .replace(
            "<FillChecking>DontCheck</FillChecking>",
            "<FillChecking>ShowWarning</FillChecking>",
        );
        write_file(
            &object_path,
            &sample_document_with_child_objects(&attribute),
        );
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Comment=changed"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("Attribute.FillChecking")
                    && error.contains("ShowWarning")
                    && error.contains("8.3.27")
            }),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_checks_every_relevant_boolean_object_and_child_property() {
        let context = temp_context("boolean-contract-table");
        let object_path = context.cwd.join("BooleanProbe.xml");

        for (node_type, properties) in boolean_contract_cases() {
            for property in *properties {
                write_file(
                    &object_path,
                    &sample_with_invalid_boolean_node(node_type, property),
                );
                let before = fs::read(&object_path).unwrap();
                let outcome = edit_meta(
                    &meta_edit_args(&object_path, "modify-property", "Comment=changed"),
                    &context,
                );

                assert!(!outcome.ok, "{node_type}.{property}: {outcome:?}");
                assert!(
                    outcome.errors.iter().any(|error| {
                        error.contains(node_type)
                            && error.contains(property)
                            && error.contains("xs:boolean")
                    }),
                    "{node_type}.{property}: {:?}",
                    outcome.errors
                );
                assert_eq!(
                    fs::read(&object_path).unwrap(),
                    before,
                    "{node_type}.{property}"
                );
            }
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_does_not_classify_text_or_attribute_use_as_boolean() {
        let context = temp_context("boolean-name-collisions");
        let catalog_path = context.cwd.join("Catalogs/BooleanProbe.xml");
        write_file(&catalog_path, &sample_catalog_with_autonumbering("true"));

        let text_outcome = edit_meta(
            &meta_edit_args(&catalog_path, "modify-property", "Comment=TRUE"),
            &context,
        );
        assert!(text_outcome.ok, "{text_outcome:?}");
        assert!(fs::read_to_string(&catalog_path)
            .unwrap()
            .contains("<Comment>TRUE</Comment>"));

        let document_path = context.cwd.join("Documents/BooleanProbe.xml");
        let document = sample_document_with_child_objects(&sample_attribute(
            "ProbeAttribute",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
        ));
        write_file(&document_path, &document);
        let use_outcome = edit_meta(
            &meta_edit_args(
                &document_path,
                "modify-attribute",
                "ProbeAttribute: use=ForItem",
            ),
            &context,
        );
        assert!(use_outcome.ok, "{use_outcome:?}");
        assert!(fs::read_to_string(&document_path)
            .unwrap()
            .contains("<Use>ForItem</Use>"));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_modify_property_comment_replaces_self_closing_object_comment() {
        let context = temp_context("modify-property-comment-self-closing");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        write_file(&object_path, &sample_catalog_xml());

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
            &context,
        );

        assert!(outcome.ok, "{:?}", outcome.errors);
        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Comment>TEST-COMMENT</Comment>"));
        assert_eq!(updated.matches("<Comment").count(), 1, "{updated}");
        assert!(!updated.contains("<Comment/>"), "{updated}");
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_edit_post_write_failure_restores_the_exact_source_bytes() {
        let context = temp_context("post-write-rollback");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        write_file(&object_path, &sample_catalog_xml());
        let original = fs::read(&object_path).unwrap();
        let args = meta_edit_args(&object_path, "modify-property", "Comment=UPDATED-COMMENT");

        let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
            edit_meta(&args, &context)
        });

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("post-write validation")),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&object_path).unwrap(), original);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_edit_rolls_back_if_format_owner_changes_during_publication() {
        let context = temp_context("format-owner-race");
        let source = context.cwd.join("src");
        let object_path = source.join("Catalogs/SampleContracts.xml");
        let owner_path = source.join("Configuration.xml");
        write_file(
            &context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        write_file(
            &owner_path,
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration/></MetaDataObject>",
        );
        write_file(&object_path, &sample_catalog_xml());
        let object_before = fs::read(&object_path).unwrap();
        let concurrent_owner = b"<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.21\"><Configuration/></MetaDataObject>".to_vec();
        let owner_for_hook = owner_path.clone();
        let owner_bytes_for_hook = concurrent_owner.clone();
        let args = meta_edit_args(&object_path, "modify-property", "Comment=UPDATED-COMMENT");

        let outcome = with_before_commit_hook(
            move |_| fs::write(&owner_for_hook, &owner_bytes_for_hook).unwrap(),
            || edit_meta(&args, &context),
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("read guard"),
            "{outcome:?}"
        );
        assert_eq!(fs::read(&object_path).unwrap(), object_before);
        assert_eq!(fs::read(&owner_path).unwrap(), concurrent_owner);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_modify_property_comment_replaces_existing_object_comment() {
        let context = temp_context("modify-property-comment-existing");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        let xml = sample_catalog_xml().replace("<Comment/>", "<Comment>OLD</Comment>");
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
            &context,
        );

        assert!(outcome.ok, "{:?}", outcome.errors);
        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Comment>TEST-COMMENT</Comment>"));
        assert!(!updated.contains("<Comment>OLD</Comment>"));
        assert_eq!(updated.matches("<Comment").count(), 1, "{updated}");
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_modify_property_comment_rejects_duplicate_without_mutation() {
        let context = temp_context("modify-property-comment-duplicate");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        let xml = sample_catalog_xml().replace(
            "<Comment/>",
            "<Comment>FIRST</Comment>\n\t\t\t<Comment>SECOND</Comment>",
        );
        write_file(&object_path, &xml);
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("2 direct <Comment>")),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_modify_property_same_comment_is_byte_identical_noop() {
        let context = temp_context("modify-property-comment-noop");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        let xml = sample_catalog_xml().replace("<Comment/>", "<Comment>TEST-COMMENT</Comment>");
        fs::create_dir_all(object_path.parent().unwrap()).unwrap();
        write_utf8_bom(&object_path, &xml).unwrap();
        let before = fs::read(&object_path).unwrap();

        let execution = edit_meta_with_data(
            &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
            &context,
        );
        let outcome = &execution.outcome;

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert!(outcome.changes.is_empty(), "{:?}", outcome.changes);
        assert_eq!(fs::read(&object_path).unwrap(), before);
        assert!(!execution.data.as_ref().unwrap().changed);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_unknown_scalar_property_without_writing() {
        let context = temp_context("modify-property-unknown-scalar");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        write_file(&object_path, &sample_catalog_xml());
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Bogus=x"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("Bogus") && error.contains("does not exist")),
            "{outcome:?}"
        );
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_invalid_known_enum_without_writing() {
        let context = temp_context("modify-property-invalid-enum");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        let xml = sample_catalog_xml().replace(
            "\t\t\t<Owners/>",
            "\t\t\t<HierarchyType>HierarchyFoldersAndItems</HierarchyType>\n\t\t\t<Owners/>",
        );
        write_file(&object_path, &xml);
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "HierarchyType=Bogus"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("HierarchyType")
                    && error.contains("Bogus")
                    && error.contains("8.3.27")
            }),
            "{outcome:?}"
        );
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_register_record_to_document() {
        let context = temp_context("add-register-record");
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

        let outcome = edit_meta(&register_record_args(&object_path), &context);
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<RegisterRecords>"));
        assert!(updated.contains(
            "<xr:Item xsi:type=\"xr:MDObjectRef\">AccumulationRegister.SampleUnshippedGoods</xr:Item>"
        ));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_duplicate_register_record() {
        let context = temp_context("duplicate-register-record");
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        let original = sample_document_xml(
            r#"<RegisterRecords>
				<xr:Item xsi:type="xr:MDObjectRef">AccumulationRegister.SampleUnshippedGoods</xr:Item>
			</RegisterRecords>"#,
        );
        write_file(&object_path, &original);

        let outcome = edit_meta(&register_record_args(&object_path), &context);
        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("already exists")));
        assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_register_record_dry_run_does_not_write_file() {
        let context = temp_context("dry-run-register-record");
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        let original = sample_document_xml("<RegisterRecords/>");
        write_file(&object_path, &original);

        let result = UnicaApplication::new()
            .call_tool("unica.meta.edit", &register_record_args(&object_path))
            .unwrap();

        assert!(result.ok);
        assert!(result.summary.contains("dry run"));
        assert_eq!(result.cache.mode, "dry-run");
        assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_attribute_to_document() {
        let context = temp_context("add-attribute");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-attribute",
                "SampleCargoPlaceCode: String(50)",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Attribute uuid=\""));
        assert!(updated.contains("<Name>SampleCargoPlaceCode</Name>"));
        assert!(updated.contains("<v8:Length>50</v8:Length>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_tabular_section_to_document() {
        let context = temp_context("add-tabular-section");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "add-ts", "SampleItems"),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<TabularSection uuid=\""));
        assert!(updated.contains("<Name>SampleItems</Name>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_tabular_section_with_inline_columns() {
        let context = temp_context("add-tabular-section-with-inline-columns");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-ts",
                "SampleItems: SampleSourceDocument: DocumentRef.SampleSale, SampleQuantity: Number(15,3)",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<TabularSection uuid=\""));
        assert!(updated.contains("<Name>SampleItems</Name>"));
        assert!(!updated.contains("<Name>SampleItems: SampleSourceDocument"));
        assert!(updated.contains("<Name>SampleSourceDocument</Name>"));
        assert!(updated.contains("<v8:Type>cfg:DocumentRef.SampleSale</v8:Type>"));
        assert!(updated.contains("<Name>SampleQuantity</Name>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_omits_line_number_length_for_external_tabular_sections() {
        for object_type in ["ExternalReport", "ExternalDataProcessor"] {
            let context = temp_context(&format!(
                "add-unbounded-tabular-section-{}",
                object_type.to_ascii_lowercase()
            ));
            let object_path = context.cwd.join(format!("{object_type}.xml"));
            write_file(
                &object_path,
                &sample_meta_named(object_type, "SampleExternalObject"),
            );

            let outcome = edit_meta(
                &meta_edit_args(
                    &object_path,
                    "add-ts",
                    "SampleItems: SampleValue: String(100)",
                ),
                &context,
            );

            assert!(outcome.ok, "{object_type}: {outcome:?}");
            let updated = fs::read_to_string(&object_path).unwrap();
            let document = Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();
            let section = document
                .descendants()
                .find(|node| {
                    node.is_element()
                        && node.tag_name().name() == "TabularSection"
                        && meta_info_child(*node, "Properties")
                            .and_then(|properties| meta_info_child_text(properties, "Name"))
                            .as_deref()
                            == Some("SampleItems")
                })
                .expect("SampleItems tabular section");
            let properties = meta_info_child(section, "Properties").unwrap();
            assert!(
                meta_info_child(properties, "LineNumberLength").is_none(),
                "{object_type}: {updated}"
            );

            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn edit_meta_adds_attribute_to_tabular_section() {
        let context = temp_context("add-tabular-section-attribute");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let mut xml = sample_document_xml("<RegisterRecords/>");
        xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects/>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-ts-attribute",
                "SampleItems.SampleSourceDocument: DocumentRef.SampleSale",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Name>SampleSourceDocument</Name>"));
        assert!(updated.contains("<v8:Type>cfg:DocumentRef.SampleSale</v8:Type>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_attribute_to_non_empty_tabular_section() {
        let context = temp_context("add-non-empty-tabular-section-attribute");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let mut xml = sample_document_xml("<RegisterRecords/>");
        xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>ExistingItem</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type/>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t</ChildObjects>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-ts-attribute",
                "SampleItems.SampleSourceDocument: DocumentRef.SampleSale",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Name>ExistingItem</Name>"));
        assert!(updated.contains("<Name>SampleSourceDocument</Name>"));
        assert!(updated.contains("<v8:Type>cfg:DocumentRef.SampleSale</v8:Type>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_tabular_attribute_to_bom_xml_with_cyrillic_section() {
        let context = temp_context("add-bom-cyrillic-tabular-section-attribute");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let mut xml = sample_document_xml("<RegisterRecords/>");
        xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>Товары</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>Номенклатура</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type/>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t</ChildObjects>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
        write_file(&object_path, &format!("\u{feff}{xml}"));

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-ts-attribute",
                "Товары.кшРеализация: DocumentRef.РеализацияТоваровУслуг",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.starts_with('\u{feff}'));
        assert!(updated.contains("<Name>Номенклатура</Name>"));
        assert!(updated.contains("<Name>кшРеализация</Name>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_add_tabular_attribute_reports_missing_target() {
        let context = temp_context("missing-tabular-section");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-ts-attribute",
                "SampleItems.SampleSourceDocument: DocumentRef.SampleSale",
            ),
            &context,
        );

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("TabularSection 'SampleItems' not found")));

        let _ = fs::remove_dir_all(&context.cwd);
    }
    #[test]
    fn edit_meta_removes_attribute_from_tabular_section() {
        let context = temp_context("remove-tabular-section-attribute");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let mut xml = sample_document_xml("<RegisterRecords/>");
        xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>ExistingItem</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type/>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t\t<Attribute uuid=\"44444444-4444-4444-8444-444444444444\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>ObsoleteItem</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type/>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t</ChildObjects>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "remove-ts-attribute",
                "SampleItems.ObsoleteItem",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Name>ExistingItem</Name>"));
        assert!(!updated.contains("<Name>ObsoleteItem</Name>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_remove_tabular_attribute_reports_missing_attribute() {
        let context = temp_context("missing-tabular-section-attribute");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let mut xml = sample_document_xml("<RegisterRecords/>");
        xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects/>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "remove-ts-attribute",
                "SampleItems.MissingItem",
            ),
            &context,
        );

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("Attribute 'SampleItems.MissingItem' not found")));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_modifies_attribute_synonym_and_comment() {
        let context = temp_context("modify-attribute");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let mut xml = sample_document_xml("<RegisterRecords/>");
        xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleCargoPlaceCode</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t\t<v8:StringQualifiers>\n\t\t\t\t\t\t\t<v8:Length>50</v8:Length>\n\t\t\t\t\t\t</v8:StringQualifiers>\n\t\t\t\t\t</Type>\n\t\t\t\t</Properties>\n\t\t\t</Attribute>\n\t\t</ChildObjects>",
        );
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-attribute",
                "SampleCargoPlaceCode: synonym=Код грузового места, comment=TZ-SAMPLE",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Name>SampleCargoPlaceCode</Name>"));
        assert!(updated.contains("<v8:content>Код грузового места</v8:content>"));
        assert!(updated.contains("<Comment>TZ-SAMPLE</Comment>"));
        assert!(updated.contains("<v8:Length>50</v8:Length>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_modifies_tabular_attribute_synonym_comment_and_allowed_sign() {
        let context = temp_context("modify-tabular-attribute");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let mut xml = sample_document_xml("<RegisterRecords/>");
        xml = xml.replace(
            "\t\t<ChildObjects/>",
            "\t\t<ChildObjects>\n\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleItems</Name>\n\t\t\t\t\t<Synonym/>\n\t\t\t\t\t<Comment/>\n\t\t\t\t\t<ToolTip/>\n\t\t\t\t\t<FillChecking>DontCheck</FillChecking>\n\t\t\t\t</Properties>\n\t\t\t\t<ChildObjects>\n\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">\n\t\t\t\t\t\t<Properties>\n\t\t\t\t\t\t\t<Name>SampleQuantity</Name>\n\t\t\t\t\t\t\t<Synonym/>\n\t\t\t\t\t\t\t<Comment/>\n\t\t\t\t\t\t\t<Type>\n\t\t\t\t\t\t\t\t<v8:Type>xs:decimal</v8:Type>\n\t\t\t\t\t\t\t\t<v8:NumberQualifiers>\n\t\t\t\t\t\t\t\t\t<v8:Digits>15</v8:Digits>\n\t\t\t\t\t\t\t\t\t<v8:FractionDigits>3</v8:FractionDigits>\n\t\t\t\t\t\t\t\t</v8:NumberQualifiers>\n\t\t\t\t\t\t\t</Type>\n\t\t\t\t\t\t</Properties>\n\t\t\t\t\t</Attribute>\n\t\t\t\t</ChildObjects>\n\t\t\t</TabularSection>\n\t\t</ChildObjects>",
        );
        write_file(&object_path, &format!("\u{feff}{xml}"));

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-ts-attribute",
                "SampleItems.SampleQuantity: synonym=Количество, comment=TZ-SAMPLE, v8:AllowedSign=Nonnegative",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.starts_with('\u{feff}'));
        assert!(updated.contains("<Name>SampleQuantity</Name>"));
        assert!(updated.contains("<v8:content>Количество</v8:content>"));
        assert!(updated.contains("<Comment>TZ-SAMPLE</Comment>"));
        assert!(updated.contains("<v8:AllowedSign>Nonnegative</v8:AllowedSign>"));
        assert!(updated.contains("<v8:FractionDigits>3</v8:FractionDigits>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_modifies_tabular_section_properties() {
        let context = temp_context("modify-tabular-section");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let xml = sample_document_with_child_objects(
            "\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects/>
\t\t\t</TabularSection>",
        );
        write_file(&object_path, &format!("\u{feff}{xml}"));

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-ts",
                "SampleItems: synonym=Товарный состав, fillChecking=ShowError",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.starts_with('\u{feff}'));
        assert!(updated.contains("<Name>SampleItems</Name>"));
        assert!(updated.contains("<v8:content>Товарный состав</v8:content>"));
        assert!(updated.contains("<FillChecking>ShowError</FillChecking>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_modifies_line_number_length_without_reformatting_xml() {
        let context = temp_context("modify-line-number-length");
        let source_dir = context.cwd.join("src");
        write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
        let object_path = source_dir.join("Documents").join("SampleObject.xml");
        let original = format!(
            "\u{feff}{}",
            sample_object_with_line_number_length("Document", "5").replace('\n', "\r\n")
        );
        write_file(&object_path, &original);

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
            &context,
        );

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&object_path).unwrap();
        assert_eq!(
            updated,
            original.replace(
                "<LineNumberLength>5</LineNumberLength>",
                "<LineNumberLength>9</LineNumberLength>"
            )
        );
        assert!(updated.starts_with('\u{feff}'));
        assert!(updated.contains("\r\n"));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_definition_file_modifies_line_number_length() {
        let context = temp_context("definition-line-number-length");
        let source_dir = context.cwd.join("src");
        write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
        let object_path = source_dir.join("Documents").join("SampleObject.xml");
        let definition_path = context.cwd.join("meta-edit.json");
        write_file(
            &object_path,
            &sample_object_with_line_number_length("Document", "5"),
        );
        write_file(
            &definition_path,
            r#"{
  "modify": {
    "tabularSections": {
      "SampleItems": {
        "lineNumberLength": 9
      }
    }
  }
}"#,
        );

        let outcome = edit_meta(
            &meta_edit_definition_args(&object_path, &definition_path),
            &context,
        );

        assert!(outcome.ok, "{outcome:?}");
        assert!(fs::read_to_string(&object_path)
            .unwrap()
            .contains("<LineNumberLength>9</LineNumberLength>"));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_accepts_line_number_length_key_aliases() {
        let context = temp_context("line-number-length-aliases");
        let source_dir = context.cwd.join("src");
        write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
        let object_path = source_dir.join("Documents").join("SampleObject.xml");
        let original = sample_object_with_line_number_length("Document", "5");

        for key in [
            "lineNumberLength",
            "line_number_length",
            "line-number-length",
        ] {
            write_file(&object_path, &original);
            let outcome = edit_meta(
                &meta_edit_args(&object_path, "modify-ts", &format!("SampleItems: {key}=9")),
                &context,
            );

            assert!(outcome.ok, "{key}: {outcome:?}");
            assert!(
                fs::read_to_string(&object_path)
                    .unwrap()
                    .contains("<LineNumberLength>9</LineNumberLength>"),
                "{key}"
            );
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_invalid_line_number_length_without_writing() {
        let context = temp_context("invalid-line-number-length");
        let source_dir = context.cwd.join("src");
        write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
        let object_path = source_dir.join("Documents").join("SampleObject.xml");
        let original = sample_object_with_line_number_length("Document", "5");

        for value in ["", "4", "10", "5.5", "text", "-1"] {
            write_file(&object_path, &original);
            let before = fs::read(&object_path).unwrap();
            let outcome = edit_meta(
                &meta_edit_args(
                    &object_path,
                    "modify-ts",
                    &format!("SampleItems: lineNumberLength={value}"),
                ),
                &context,
            );

            assert!(!outcome.ok, "{value}: {outcome:?}");
            assert!(
                outcome.errors.iter().any(|error| {
                    error.contains("LineNumberLength")
                        && error.contains("integer")
                        && error.contains("5..=9")
                }),
                "{value}: {:?}",
                outcome.errors
            );
            assert_eq!(fs::read(&object_path).unwrap(), before, "{value}");
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_line_number_length_for_unbounded_tabular_sections() {
        for object_type in [
            "Report",
            "DataProcessor",
            "ExternalReport",
            "ExternalDataProcessor",
        ] {
            let context = temp_context(&format!(
                "line-number-length-{}",
                object_type.to_ascii_lowercase()
            ));
            let object_path = if let Some(directory) = meta_compile_type_plural(object_type) {
                let source_dir = context.cwd.join("src");
                write_owner_with_compatibility(
                    &source_dir,
                    object_type,
                    "SampleObject",
                    "Version8_3_27",
                );
                source_dir.join(directory).join("SampleObject.xml")
            } else {
                context.cwd.join(format!("{object_type}.xml"))
            };
            let original = sample_object_with_line_number_length(object_type, "5");
            write_file(&object_path, &original);
            let before = fs::read(&object_path).unwrap();

            let outcome = edit_meta(
                &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
                &context,
            );

            assert!(!outcome.ok, "{object_type}: {outcome:?}");
            assert!(
                outcome.errors.iter().any(|error| {
                    error.contains("LineNumberLength") && error.contains("not applicable")
                }),
                "{object_type}: {:?}",
                outcome.errors
            );
            assert_eq!(fs::read(&object_path).unwrap(), before, "{object_type}");

            let _ = fs::remove_dir_all(&context.cwd);
        }
    }

    #[test]
    fn edit_meta_rejects_line_number_length_fixed_by_compatibility_mode() {
        let context = temp_context("fixed-line-number-length");
        let source_dir = context.cwd.join("src");
        write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_26");
        let object_path = source_dir.join("Documents").join("SampleObject.xml");
        let original = sample_object_with_line_number_length("Document", "5");
        write_file(&object_path, &original);
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("LineNumberLength")
                    && error.contains("fixed at 5")
                    && error.contains("Version8_3_26")
            }),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn line_number_length_policy_covers_supported_compatibility_generations() {
        for fixed in [
            "Version8_1",
            "Version8_2_13",
            "Version8_3_1",
            "Version8_3_25",
            "Version8_3_26",
        ] {
            assert!(
                matches!(
                    meta_edit_line_number_length_policy_from_mode(fixed),
                    MetaEditLineNumberLengthPolicy::FixedFive
                ),
                "{fixed}"
            );
        }
        for editable in ["DontUse", "Version8_3_27"] {
            assert!(
                matches!(
                    meta_edit_line_number_length_policy_from_mode(editable),
                    MetaEditLineNumberLengthPolicy::Editable
                ),
                "{editable}"
            );
        }
        assert!(matches!(
            meta_edit_line_number_length_policy_from_mode("Bogus"),
            MetaEditLineNumberLengthPolicy::UnknownCompatibility
        ));
    }

    #[test]
    fn line_number_length_policy_uses_effective_platform_version() {
        for (mode, platform_line) in [
            ("DontUse", "8.3.27"),
            ("DontUse", "8.5.4"),
            ("Version8_3_27", "8.5.4"),
        ] {
            assert!(
                matches!(
                    meta_edit_line_number_length_policy_for_platform(mode, platform_line),
                    MetaEditLineNumberLengthPolicy::Editable
                ),
                "{mode} on {platform_line}"
            );
        }

        for (mode, platform_line) in [("DontUse", "8.3.26"), ("Version8_3_24", "8.5.4")] {
            assert!(
                matches!(
                    meta_edit_line_number_length_policy_for_platform(mode, platform_line),
                    MetaEditLineNumberLengthPolicy::FixedFive
                ),
                "{mode} on {platform_line}"
            );
        }

        for platform_line in ["8.3.27.2074", "invalid"] {
            assert!(
                matches!(
                    meta_edit_line_number_length_policy_for_platform("DontUse", platform_line),
                    MetaEditLineNumberLengthPolicy::UnknownCompatibility
                ),
                "{platform_line}"
            );
        }
    }

    #[test]
    fn line_number_length_policy_rejects_unsupported_future_mode() {
        for unsupported in ["Version8_5_1", "Version999_0_0"] {
            assert!(
                matches!(
                    meta_edit_line_number_length_policy_from_mode(unsupported),
                    MetaEditLineNumberLengthPolicy::UnknownCompatibility
                ),
                "{unsupported}"
            );
        }
    }

    #[test]
    fn edit_meta_rejects_source_map_remap_after_line_number_length_authorization() {
        let context = temp_context("line-number-length-source-map-race");
        let project_map = context.cwd.join("v8project.yaml");
        let source_dir = context.cwd.join("src");
        write_file(
            &project_map,
            "format: DESIGNER\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        );
        write_owner_with_compatibility(&source_dir, "Document", "SampleObject", "Version8_3_27");
        write_owner_with_compatibility(
            &source_dir.join("Documents"),
            "Document",
            "SampleObject",
            "Version8_3_26",
        );
        let object_path = source_dir.join("Documents").join("SampleObject.xml");
        let original = sample_object_with_line_number_length("Document", "5");
        write_file(&object_path, &original);
        let before = fs::read(&object_path).unwrap();
        let project_map_for_hook = project_map.clone();

        let outcome = with_meta_edit_after_line_number_length_policy_hook(
            move || {
                fs::write(
                    project_map_for_hook,
                    "format: DESIGNER\nsource-set:\n  - name: remapped\n    type: CONFIGURATION\n    path: src/Documents\n",
                )
                .unwrap();
            },
            || {
                edit_meta(
                    &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
                    &context,
                )
            },
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("v8project.yaml") && error.contains("changed")),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn line_number_length_owner_policy_is_requested_only_for_matching_changes() {
        assert!(meta_edit_inline_requests_line_number_length(
            "modify-ts",
            "Items: line-number-length=9"
        ));
        assert!(!meta_edit_inline_requests_line_number_length(
            "modify-ts",
            "Items: synonym=lineNumberLength"
        ));
        assert!(!meta_edit_inline_requests_line_number_length(
            "modify-attribute",
            "LineNumberLength: synonym=Length"
        ));

        assert!(meta_edit_definition_requests_line_number_length(&json!({
            "modify": {
                "tabularSections": {
                    "Items": {"line_number_length": 9}
                }
            }
        })));
        assert!(!meta_edit_definition_requests_line_number_length(&json!({
            "modify": {
                "tabularSections": {
                    "Items": {
                        "modify": {
                            "LineNumberLength": {"synonym": "Length"}
                        }
                    }
                }
            }
        })));
    }

    #[test]
    fn edit_meta_rejects_line_number_length_without_compatibility_context() {
        let context = temp_context("unknown-line-number-length-compatibility");
        let object_path = context.cwd.join("Documents").join("SampleObject.xml");
        let original = sample_object_with_line_number_length("Document", "5");
        write_file(&object_path, &original);
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-ts", "SampleItems: lineNumberLength=9"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("LineNumberLength")
                    && error.contains("CompatibilityMode")
                    && error.contains("cannot be determined")
            }),
            "{:?}",
            outcome.errors
        );
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_register_dimensions_and_resources() {
        let context = temp_context("add-register-fields");
        let object_path = context
            .cwd
            .join("InformationRegisters")
            .join("SampleStock.xml");
        write_file(&object_path, &sample_register_xml("InformationRegister"));

        let dimension = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-dimension",
                "SampleWarehouse: CatalogRef.Warehouses | master, mainFilter",
            ),
            &context,
        );
        assert!(dimension.ok, "{:?}", dimension.errors);

        let resource = edit_meta(
            &meta_edit_args(&object_path, "add-resource", "SampleQty: Number(15,3)"),
            &context,
        );
        assert!(resource.ok, "{:?}", resource.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Dimension uuid="));
        assert!(updated.contains("<Name>SampleWarehouse</Name>"));
        assert!(updated.contains("<Master>true</Master>"));
        assert!(updated.contains("<MainFilter>true</MainFilter>"));
        assert!(updated.contains("<Resource uuid="));
        assert!(updated.contains("<Name>SampleQty</Name>"));
        assert!(updated.contains("<v8:FractionDigits>3</v8:FractionDigits>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_and_removes_enum_values_and_simple_children() {
        let context = temp_context("enum-and-simple-children");
        let object_path = context.cwd.join("Enums").join("SampleStatuses.xml");
        write_file(&object_path, &sample_enum_xml());

        for (operation, value) in [
            ("add-enumValue", "Pending ;; Obsolete"),
            ("add-form", "FormItem"),
            ("add-template", "PrintTemplate"),
            ("add-command", "OpenCommand"),
            ("modify-enumValue", "Pending: synonym=Ожидает"),
            ("remove-enumValue", "Obsolete"),
        ] {
            let outcome = edit_meta(&meta_edit_args(&object_path, operation, value), &context);
            assert!(outcome.ok, "{operation}: {:?}", outcome.errors);
        }

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<EnumValue uuid="));
        assert!(updated.contains("<Name>Pending</Name>"));
        assert!(updated.contains("<v8:content>Ожидает</v8:content>"));
        assert!(!updated.contains("<Name>Obsolete</Name>"));
        assert!(updated.contains("<Form uuid="));
        assert!(updated.contains("<FormType>Ordinary</FormType>"));
        assert!(updated.contains("<Template uuid="));
        assert!(updated.contains("<TemplateType>SpreadsheetDocument</TemplateType>"));
        assert!(updated.contains("<Command uuid="));
        assert!(updated.contains("<Representation>Auto</Representation>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_invalid_enum_column_and_simple_child_names_without_writing() {
        let context = temp_context("invalid-added-child-names");
        let enum_path = context.cwd.join("Enums").join("SampleStatuses.xml");
        let journal_path = context
            .cwd
            .join("DocumentJournals")
            .join("SampleJournal.xml");
        let document_path = context.cwd.join("Documents").join("SampleDocument.xml");

        for (path, original, operation, value) in [
            (&enum_path, sample_enum_xml(), "add-enumValue", "Bad Name"),
            (&enum_path, sample_enum_xml(), "add-form", "Bad Name"),
            (&enum_path, sample_enum_xml(), "add-template", "Bad Name"),
            (&enum_path, sample_enum_xml(), "add-command", "Bad Name"),
            (
                &document_path,
                sample_document_xml("<RegisterRecords/>"),
                "add-ts",
                "Bad Name",
            ),
            (
                &journal_path,
                sample_document_journal_xml(),
                "add-column",
                "Bad Name: DocumentRef.SampleDocument",
            ),
        ] {
            write_file(path, &original);
            let before = fs::read(path).unwrap();

            let outcome = edit_meta(&meta_edit_args(path, operation, value), &context);

            assert!(!outcome.ok, "{operation}: {outcome:?}");
            assert!(
                outcome.errors.iter().any(|error| {
                    error.contains("Bad Name") && error.contains("valid 1C identifier")
                }),
                "{operation}: {:?}",
                outcome.errors
            );
            assert!(outcome.changes.is_empty(), "{operation}: {outcome:?}");
            assert_eq!(fs::read(path).unwrap(), before, "{operation}");
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_every_invalid_name_rename_before_mutating_xml() {
        let targets = [
            MetaEditModifyTarget::Attribute {
                fill_value_allowed: true,
            },
            MetaEditModifyTarget::RegisterField,
            MetaEditModifyTarget::EnumValue,
            MetaEditModifyTarget::Column,
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::Editable,
            },
        ];

        for target in targets {
            let mut xml = "<Properties><Name>ValidName</Name></Properties>".to_string();
            let before = xml.clone();
            let length = xml.len();

            let error =
                meta_edit_modify_properties_range(&mut xml, 0..length, "name=Bad Name", target)
                    .unwrap_err();

            assert!(error.contains("Bad Name"), "{error}");
            assert!(error.contains("valid 1C identifier"), "{error}");
            assert_eq!(xml, before);
        }
    }

    #[test]
    fn edit_meta_rejects_invalid_object_name_rename_without_writing() {
        let context = temp_context("invalid-object-rename");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        write_file(&object_path, &sample_catalog_xml());
        let before = fs::read(&object_path).unwrap();

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Name=Bad Name"),
            &context,
        );

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.iter().any(|error| {
                error.contains("Bad Name") && error.contains("valid 1C identifier")
            }),
            "{:?}",
            outcome.errors
        );
        assert!(outcome.changes.is_empty(), "{outcome:?}");
        assert_eq!(fs::read(&object_path).unwrap(), before);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_adds_document_journal_columns() {
        let context = temp_context("add-document-journal-column");
        let object_path = context
            .cwd
            .join("DocumentJournals")
            .join("SampleJournal.xml");
        write_file(&object_path, &sample_document_journal_xml());

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-column",
                "SampleKind: EnumRef.SampleKinds",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Column uuid="));
        assert!(updated.contains("<Name>SampleKind</Name>"));
        assert!(updated.contains("<References>"));
        assert!(updated.contains(">EnumRef.SampleKinds</xr:Item>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_sets_adds_and_removes_complex_properties() {
        let context = temp_context("complex-properties");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        write_file(&object_path, &sample_catalog_xml());

        for (operation, value) in [
            (
                "set-owners",
                "Catalog.SampleCounterparties ;; Catalog.SampleOrganizations",
            ),
            ("remove-owner", "Catalog.SampleOrganizations"),
            ("add-inputByString", "StandardAttribute.Description"),
            ("add-basedOn", "Document.SampleOrder"),
        ] {
            let outcome = edit_meta(&meta_edit_args(&object_path, operation, value), &context);
            assert!(outcome.ok, "{operation}: {:?}", outcome.errors);
        }

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Owners>"));
        assert!(updated.contains(">Catalog.SampleCounterparties</xr:Item>"));
        assert!(!updated.contains(">Catalog.SampleOrganizations</xr:Item>"));
        assert!(updated.contains("<InputByString>"));
        assert!(updated.contains(
            "<xr:Field>Catalog.SampleContracts.StandardAttribute.Description</xr:Field>"
        ));
        assert!(updated.contains(">Document.SampleOrder</xr:Item>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_definition_file_processes_json_dsl() {
        let context = temp_context("definition-file-json-dsl");
        let object_path = context.cwd.join("Catalogs").join("SampleContracts.xml");
        let definition_path = context.cwd.join("meta-edit.json");
        write_file(&object_path, &sample_catalog_xml());
        write_file(
            &definition_path,
            r#"{
  "add": {
    "attributes": [
      { "name": "SampleNote", "type": ["String", "Number(10,2)"], "indexing": "Index" }
    ],
    "tabularSections": [
      { "name": "Items", "attrs": ["Item: CatalogRef.Items", "Qty: Number(15,3)"] }
    ],
    "forms": ["FormItem"]
  },
  "modify": {
    "properties": {
      "Owners": ["Catalog.SampleCounterparties"],
      "InputByString": ["StandardAttribute.Description"]
    },
    "tabularSections": {
      "Items": {
        "add": ["Discount: Number(5,2)"]
      }
    }
  }
}"#,
        );

        let outcome = edit_meta(
            &meta_edit_definition_args(&object_path, &definition_path),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Name>SampleNote</Name>"));
        assert!(updated.contains("<v8:Type>xs:string</v8:Type>"));
        assert!(updated.contains("<v8:Type>xs:decimal</v8:Type>"));
        assert!(updated.contains("<Name>Items</Name>"));
        assert!(updated.contains("<Name>Discount</Name>"));
        assert!(updated.contains("<Name>FormItem</Name>"));
        assert!(updated.contains(">Catalog.SampleCounterparties</xr:Item>"));
        assert!(updated.contains(
            "<xr:Field>Catalog.SampleContracts.StandardAttribute.Description</xr:Field>"
        ));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_tabular_section_type_change() {
        let context = temp_context("modify-tabular-section-type");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let xml = sample_document_with_child_objects(
            "\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects/>
\t\t\t</TabularSection>",
        );
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-ts", "SampleItems: type=String(50)"),
            &context,
        );

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("Unsupported modify property key 'type'")));
        assert!(!fs::read_to_string(&object_path).unwrap().contains("<Type>"));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_add_attribute_supports_batch_values() {
        let context = temp_context("add-attribute-batch");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-attribute",
                "SampleCargoPlaceCode: String(50) ;; SampleWeight: Number(10,2)",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<Name>SampleCargoPlaceCode</Name>"));
        assert!(updated.contains("<v8:Length>50</v8:Length>"));
        assert!(updated.contains("<Name>SampleWeight</Name>"));
        assert!(updated.contains("<v8:Digits>10</v8:Digits>"));
        assert!(!updated.contains(";; SampleWeight"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_invalid_type_expressions_without_writing() {
        let context = temp_context("invalid-type-expression");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let original = sample_document_with_child_objects(&sample_attribute(
            "Value",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
        ));

        for (operation, value) in [
            ("add-attribute", "Broken: String(foo)"),
            ("modify-attribute", "Value: type=Number(x,2)"),
        ] {
            write_file(&object_path, &original);
            let outcome = edit_meta(&meta_edit_args(&object_path, operation, value), &context);

            assert!(!outcome.ok, "{operation}: {:?}", outcome.stdout);
            assert!(
                outcome.errors.iter().any(|error| error.contains("8.3.27")),
                "{operation}: {:?}",
                outcome.errors
            );
            assert_eq!(fs::read_to_string(&object_path).unwrap(), original);
        }

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_add_attribute_supports_inline_position() {
        let context = temp_context("add-attribute-position");
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        let child_objects = format!(
            "{}\n{}",
            sample_attribute(
                "Organization",
                "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>cfg:CatalogRef.Organizations</v8:Type>\n\t\t\t\t\t</Type>",
                "\t\t\t\t\t<FillValue xsi:nil=\"true\"/>",
            ),
            sample_attribute(
                "Comment",
                "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
                "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
            )
        );
        write_file(
            &object_path,
            &sample_document_with_child_objects(&child_objects),
        );

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "add-attribute",
                "Warehouse: CatalogRef.Warehouses >> after Organization",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        let organization = updated.find("<Name>Organization</Name>").unwrap();
        let warehouse = updated.find("<Name>Warehouse</Name>").unwrap();
        let comment = updated.find("<Name>Comment</Name>").unwrap();
        assert!(organization < warehouse && warehouse < comment, "{updated}");
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_add_tabular_attribute_supports_json_position() {
        let context = temp_context("add-ts-attribute-json-position");
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        let definition_path = context.cwd.join("meta-edit.json");
        let xml = sample_document_with_child_objects(
            "\t\t\t<TabularSection uuid=\"33333333-3333-4333-8333-333333333333\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>Items</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects>
\t\t\t\t\t<Attribute uuid=\"44444444-4444-4444-8444-444444444444\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>Price</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type>
\t\t\t\t\t\t\t\t<v8:Type>xs:decimal</v8:Type>
\t\t\t\t\t\t\t</Type>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t\t<Attribute uuid=\"55555555-5555-4555-8555-555555555555\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>Amount</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type>
\t\t\t\t\t\t\t\t<v8:Type>xs:decimal</v8:Type>
\t\t\t\t\t\t\t</Type>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t</ChildObjects>
\t\t\t</TabularSection>",
        );
        write_file(&object_path, &xml);
        write_file(
            &definition_path,
            r#"{
  "modify": {
    "tabularSections": {
      "Items": {
        "add": [
          { "name": "Discount", "type": "Number(5,2)", "before": "Amount" }
        ]
      }
    }
  }
}"#,
        );

        let outcome = edit_meta(
            &meta_edit_definition_args(&object_path, &definition_path),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        let price = updated.find("<Name>Price</Name>").unwrap();
        let discount = updated.find("<Name>Discount</Name>").unwrap();
        let amount = updated.find("<Name>Amount</Name>").unwrap();
        assert!(price < discount && discount < amount, "{updated}");
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_attribute_rename_to_existing_name() {
        let context = temp_context("rename-attribute-duplicate");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let child_objects = format!(
            "{}\n{}",
            sample_attribute(
                "ExistingA",
                "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
                "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>"
            ),
            sample_attribute(
                "ExistingB",
                "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
                "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>"
            )
        );
        write_file(
            &object_path,
            &sample_document_with_child_objects(&child_objects),
        );

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-attribute",
                "ExistingA: name=ExistingB",
            ),
            &context,
        );

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("Attribute 'ExistingB' already exists")));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_tabular_attribute_rename_to_existing_name() {
        let context = temp_context("rename-tabular-attribute-duplicate");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let xml = sample_document_with_child_objects(
            "\t\t\t<TabularSection uuid=\"22222222-2222-4222-8222-222222222222\">
\t\t\t\t<Properties>
\t\t\t\t\t<Name>SampleItems</Name>
\t\t\t\t\t<Synonym/>
\t\t\t\t\t<Comment/>
\t\t\t\t\t<ToolTip/>
\t\t\t\t\t<FillChecking>DontCheck</FillChecking>
\t\t\t\t</Properties>
\t\t\t\t<ChildObjects>
\t\t\t\t\t<Attribute uuid=\"33333333-3333-4333-8333-333333333333\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>ExistingA</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type/>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t\t<Attribute uuid=\"44444444-4444-4444-8444-444444444444\">
\t\t\t\t\t\t<Properties>
\t\t\t\t\t\t\t<Name>ExistingB</Name>
\t\t\t\t\t\t\t<Synonym/>
\t\t\t\t\t\t\t<Comment/>
\t\t\t\t\t\t\t<Type/>
\t\t\t\t\t\t</Properties>
\t\t\t\t\t</Attribute>
\t\t\t\t</ChildObjects>
\t\t\t</TabularSection>",
        );
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-ts-attribute",
                "SampleItems.ExistingA: name=ExistingB",
            ),
            &context,
        );

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("Attribute 'SampleItems.ExistingB' already exists")));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_type_change_updates_existing_fill_value() {
        let context = temp_context("modify-attribute-type-fill-value");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let xml = sample_document_with_child_objects(&sample_attribute(
            "SampleCargoPlaceCode",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t\t<v8:StringQualifiers>\n\t\t\t\t\t\t\t<v8:Length>50</v8:Length>\n\t\t\t\t\t\t</v8:StringQualifiers>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
        ));
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-attribute",
                "SampleCargoPlaceCode: type=Number(15,2)",
            ),
            &context,
        );
        assert!(outcome.ok, "{:?}", outcome.errors);

        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(updated.contains("<v8:Type>xs:decimal</v8:Type>"));
        assert!(updated.contains("<FillValue xsi:type=\"xs:decimal\">0</FillValue>"));
        assert!(!updated.contains("<FillValue xsi:type=\"xs:string\"/>"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_sets_enum_fill_value() {
        let context = temp_context("modify-attribute-enum-fill-value");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let xml = format!(
            "\u{feff}{}",
            sample_document_with_child_objects(&sample_attribute(
                "Status",
                "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>cfg:EnumRef.SampleStatus</v8:Type>\n\t\t\t\t\t</Type>",
                "\t\t\t\t\t<FillValue xsi:nil=\"true\"/>",
            ))
            .replace("<Synonym/>", "<Synonym />")
            .trim_end_matches('\n')
            .replace('\n', "\r\n")
        );
        let mut patched = xml.clone();
        let modified = meta_edit_modify_top_attribute_properties(
            &mut patched,
            "Status",
            "fillValue=Enum.SampleStatus.EnumValue.Default",
        )
        .unwrap();
        let expected = xml.replace(
            "<FillValue xsi:nil=\"true\"/>",
            "<FillValue xsi:type=\"xr:DesignTimeRef\">Enum.SampleStatus.EnumValue.Default</FillValue>",
        );
        assert_eq!(modified, 1);
        assert_eq!(patched, expected);
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-attribute",
                "Status: fillValue=Enum.SampleStatus.EnumValue.Default",
            ),
            &context,
        );

        assert!(outcome.ok, "{:?}", outcome.errors);
        let updated = fs::read_to_string(&object_path).unwrap();
        assert_eq!(updated, expected);
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_resets_tabular_attribute_fill_value() {
        let context = temp_context("modify-tabular-attribute-reset-fill-value");
        let object_path = context
            .cwd
            .join("DataProcessors")
            .join("SampleProcessor.xml");
        let xml = sample_object_with_tabular_fill_value("DataProcessor");
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-ts-attribute",
                "SampleItems.Status: fillValue=nil",
            ),
            &context,
        );

        assert!(outcome.ok, "{:?}", outcome.errors);
        let updated = fs::read_to_string(&object_path).unwrap();
        assert!(!updated.starts_with('\u{feff}'));
        assert!(updated.contains("<FillValue xsi:nil=\"true\"/>"));
        assert!(updated.contains("<FillChecking>DontCheck</FillChecking>"));
        assert!(!updated.contains("Enum.SampleStatus.EnumValue.Default"));
        Document::parse(updated.trim_start_matches('\u{feff}')).unwrap();

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_fill_value_for_stored_object_tabular_attribute() {
        let context = temp_context("reject-stored-tabular-fill-value");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let original = sample_object_with_tabular_fill_value("Document");
        write_file(&object_path, &original);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-ts-attribute",
                "SampleItems.Status: fillValue=nil",
            ),
            &context,
        );

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("Unsupported modify property key 'fillValue'")));
        assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_fill_value_when_property_is_absent() {
        let context = temp_context("modify-attribute-missing-fill-value-property");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let original = sample_document_with_child_objects(&sample_attribute(
            "Status",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>cfg:EnumRef.SampleStatus</v8:Type>\n\t\t\t\t\t</Type>",
            "",
        ));
        write_file(&object_path, &original);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-attribute",
                "Status: fillValue=Enum.SampleStatus.EnumValue.Default",
            ),
            &context,
        );

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome.errors.iter().any(
            |error| error.contains("Property 'FillValue' is not available for this attribute")
        ));
        assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_fill_value_dry_run_does_not_write_file() {
        let context = temp_context("modify-attribute-fill-value-dry-run");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let original = sample_document_with_child_objects(&sample_attribute(
            "Status",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>cfg:EnumRef.SampleStatus</v8:Type>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:nil=\"true\"/>",
        ));
        write_file(&object_path, &original);
        let mut args = meta_edit_args(
            &object_path,
            "modify-attribute",
            "Status: fillValue=Enum.SampleStatus.EnumValue.Default",
        );
        args.insert("dryRun".to_string(), json!(true));

        let result = UnicaApplication::new()
            .call_tool("unica.meta.edit", &args)
            .unwrap();

        assert!(result.ok, "{:?}", result.errors);
        assert!(result.summary.contains("dry run"));
        assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    fn validate_stdout_with_synonym(test_name: &str, synonym_xml: &str) -> String {
        let context = temp_context(test_name);
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        let xml = sample_document_xml("<RegisterRecords/>").replace("<Synonym/>", synonym_xml);
        write_owner(&context.cwd, "Document", "SampleShipment", &["Русский"]);
        write_language_fixture(&context, "Русский", "ru");
        write_file(&object_path, &xml);

        let mut args = serde_json::Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!(object_path.to_string_lossy().to_string()),
        );
        let outcome = validate_meta(&args, &context);
        let stdout = outcome.stdout.clone().unwrap_or_default();

        let _ = fs::remove_dir_all(&context.cwd);
        stdout
    }

    fn localized_text(items: &[(&str, &str)]) -> String {
        items
            .iter()
            .map(|(language, content)| {
                format!(
                    "<v8:item><v8:lang>{language}</v8:lang><v8:content>{content}</v8:content></v8:item>"
                )
            })
            .collect::<String>()
    }

    fn write_language_fixture(context: &WorkspaceContext, name: &str, code: &str) {
        write_file(
            &context.cwd.join("Languages").join(format!("{name}.xml")),
            &sample_language_named(name, code),
        );
    }

    fn sample_language_named(name: &str, code: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Language uuid="22222222-2222-4222-8222-222222222222">
    <Properties>
      <Name>{name}</Name>
      <Synonym/>
      <Comment/>
      <LanguageCode>{code}</LanguageCode>
    </Properties>
  </Language>
</MetaDataObject>
"#
        )
    }

    fn validate_registered_object(
        object_type: &str,
        object_name: &str,
        object_xml: &str,
        languages: &[(&str, &str)],
    ) -> AdapterOutcome {
        let context = temp_context(&format!("registered-{object_type}-{object_name}"));
        let language_names = languages.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        let src = write_owner(
            &context.cwd.join("src"),
            object_type,
            object_name,
            &language_names,
        );
        for (name, code) in languages {
            write_file(
                &src.join("Languages").join(format!("{name}.xml")),
                &sample_language_named(name, code),
            );
        }
        let object = src
            .join(format!("{object_type}s"))
            .join(format!("{object_name}.xml"));
        write_file(&object, object_xml);
        let outcome = validate_meta(&meta_validate_args(&object), &context);
        let _ = fs::remove_dir_all(&context.cwd);
        outcome
    }

    fn outcome_text(outcome: &AdapterOutcome) -> String {
        format!(
            "{}\n{}\n{}",
            outcome.stdout.clone().unwrap_or_default(),
            outcome.warnings.join("\n"),
            outcome.errors.join("\n")
        )
    }

    fn localized_property(name: &str, values: &[(&str, &str)]) -> String {
        let items = values
            .iter()
            .map(|(language, content)| {
                format!(
                    "<v8:item><v8:lang>{language}</v8:lang>\
                     <v8:content>{content}</v8:content></v8:item>"
                )
            })
            .collect::<String>();
        format!("<{name}>{items}</{name}>")
    }

    fn sample_common_module_named(name: &str, synonyms: &[(&str, &str)]) -> String {
        let synonym = localized_property("Synonym", synonyms);
        format!(
            r#"<MetaDataObject xmlns="{TEST_MD_NS}" xmlns:v8="{TEST_V8_NS}" version="2.20">
<CommonModule uuid="33333333-3333-4333-8333-333333333333">
<Properties><Name>{name}</Name>{synonym}<Comment/></Properties>
<ChildObjects/>
</CommonModule></MetaDataObject>"#
        )
    }

    fn sample_enum_with_presentations(
        name: &str,
        synonyms: &[(&str, &str)],
        list_presentations: &[(&str, &str)],
    ) -> String {
        let synonym = localized_property("Synonym", synonyms);
        let list_presentation = localized_property("ListPresentation", list_presentations);
        format!(
            r#"<MetaDataObject xmlns="{TEST_MD_NS}" xmlns:v8="{TEST_V8_NS}"
 xmlns:xr="{TEST_XR_NS}" version="2.20">
<Enum uuid="44444444-4444-4444-8444-444444444444">
<InternalInfo>
<xr:GeneratedType name="EnumRef.{name}" category="Ref">
<xr:TypeId>55555555-5555-4555-8555-555555555551</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555552</xr:ValueId>
</xr:GeneratedType>
<xr:GeneratedType name="EnumManager.{name}" category="Manager">
<xr:TypeId>55555555-5555-4555-8555-555555555553</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555554</xr:ValueId>
</xr:GeneratedType>
<xr:GeneratedType name="EnumList.{name}" category="List">
<xr:TypeId>55555555-5555-4555-8555-555555555555</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555556</xr:ValueId>
</xr:GeneratedType>
</InternalInfo>
<Properties><Name>{name}</Name>{synonym}<Comment/>
{list_presentation}</Properties>
<ChildObjects/>
</Enum></MetaDataObject>"#
        )
    }

    fn validate_stdout_with_presentations(
        test_name: &str,
        synonym_items: &[(&str, &str)],
        list_presentation_items: &[(&str, &str)],
        configured_languages: &[(&str, &str)],
    ) -> String {
        assert!(
            !configured_languages.is_empty(),
            "presentation validation requires an explicit language profile"
        );
        let context = temp_context(test_name);
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        let synonym = format!("<Synonym>{}</Synonym>", localized_text(synonym_items));
        let list_presentation = format!(
            "<ListPresentation>{}</ListPresentation>",
            localized_text(list_presentation_items)
        );
        let xml = sample_document_xml("<RegisterRecords/>")
            .replace("<Synonym/>", &synonym)
            .replace("<Comment/>", &format!("<Comment/>{list_presentation}"));
        let language_names = configured_languages
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        write_owner(&context.cwd, "Document", "SampleShipment", &language_names);
        write_file(&object_path, &xml);

        for (name, code) in configured_languages {
            write_language_fixture(&context, name, code);
        }

        let mut args = serde_json::Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!(object_path.to_string_lossy().to_string()),
        );
        let stdout = validate_meta(&args, &context).stdout.unwrap_or_default();

        let _ = fs::remove_dir_all(&context.cwd);
        stdout
    }

    #[test]
    fn validate_meta_allows_self_closing_synonym() {
        let stdout = validate_stdout_with_synonym("validate-empty-synonym", "<Synonym/>");
        assert!(!stdout.contains("Synonym is empty"), "{stdout}");
    }

    #[test]
    fn validate_meta_rejects_non_external_object_without_owner() {
        let context = temp_context("missing-owner");
        let object = context.cwd.join("Enums/Detached.xml");
        write_file(&object, &sample_meta_named("Enum", "Detached"));

        let outcome = validate_meta(&meta_validate_args(&object), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("Configuration.xml"),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_meta_rejects_object_missing_from_owner_registration() {
        let context = temp_context("missing-registration");
        let src = write_owner(&context.cwd.join("src"), "Enum", "Other", &["Русский"]);
        let object = src.join("Enums/Detached.xml");
        write_file(&object, &sample_meta_named("Enum", "Detached"));

        let outcome = validate_meta(&meta_validate_args(&object), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome.errors.join("\n").contains("not registered"),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_meta_external_descriptor_ignores_neighbor_configuration() {
        let context = temp_context("external-owner");
        write_file(
            &context.cwd.join("Configuration.xml"),
            r#"<broken-neighbor version="2.21">"#,
        );
        let object = context.cwd.join("tools/Standalone.xml");
        write_file(
            &object,
            &sample_meta_named("ExternalDataProcessor", "Standalone"),
        );

        let inspection = inspect_meta_validation_reads(&object, &context);
        let owner = inspection.context.expect("external descriptor owns itself");

        assert_eq!(owner.owner_kind, MetaValidationOwnerKind::External);
        assert_eq!(inspection.paths, vec![canonical_path(&object)]);
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_validation_context_classifies_registered_extension_owner() {
        let context = temp_context("extension-owner");
        write_file(
            &context.cwd.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: extension\n    type: EXTENSION\n    path: extension\n",
        );
        let source_dir = write_owner(
            &context.cwd.join("extension"),
            "CommonModule",
            "ExtensionModule",
            &[],
        );
        let object = source_dir.join("CommonModules/ExtensionModule.xml");
        write_file(
            &object,
            &sample_meta_named("CommonModule", "ExtensionModule"),
        );

        let inspection = inspect_meta_validation_reads(&object, &context);
        let owner = inspection.context.expect("registered extension owner");

        assert_eq!(owner.owner_kind, MetaValidationOwnerKind::Extension);
        assert_eq!(
            inspection.paths,
            vec![
                canonical_path(&object),
                canonical_path(&source_dir.join("Configuration.xml"))
            ]
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_meta_rejects_list_type_without_registered_languages() {
        let context = temp_context("missing-language-profile");
        let src = write_owner(&context.cwd.join("src"), "Enum", "Statuses", &[]);
        let object = src.join("Enums/Statuses.xml");
        write_file(&object, &sample_meta_named("Enum", "Statuses"));

        let outcome = validate_meta(&meta_validate_args(&object), &context);

        assert!(!outcome.ok, "{outcome:?}");
        assert!(
            outcome
                .errors
                .join("\n")
                .contains("has no registered language profile"),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_validation_reads_missing_registered_language_before_reporting_error() {
        let context = temp_context("missing-language-file");
        let src = write_owner(&context.cwd.join("src"), "Enum", "Statuses", &["Russian"]);
        let object = src.join("Enums/Statuses.xml");
        write_file(&object, &sample_meta_named("Enum", "Statuses"));

        let inspection = inspect_meta_validation_reads(&object, &context);

        assert_eq!(
            inspection.paths,
            vec![
                canonical_path(&object),
                canonical_path(&src.join("Configuration.xml")),
                canonical_path(&src).join("Languages/Russian.xml")
            ]
        );
        let error = inspection
            .context
            .expect_err("missing registered language must fail");
        assert!(
            error.starts_with("registered language file not found: "),
            "{error}"
        );
        assert!(
            error.ends_with(&format!(
                "Languages{}Russian.xml",
                std::path::MAIN_SEPARATOR
            )),
            "{error}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_meta_rejects_malformed_registered_language() {
        let context = temp_context("malformed-language");
        let src = write_owner(&context.cwd.join("src"), "Enum", "Statuses", &["Russian"]);
        let object = src.join("Enums/Statuses.xml");
        let language = src.join("Languages/Russian.xml");
        write_file(&object, &sample_meta_named("Enum", "Statuses"));
        write_file(&language, "<broken-language");

        let outcome = validate_meta(&meta_validate_args(&object), &context);
        let errors = outcome.errors.join("\n");

        assert!(!outcome.ok, "{outcome:?}");
        assert!(errors.contains("failed to parse"), "{outcome:?}");
        assert!(
            errors.contains(&canonical_path(&language).display().to_string()),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_meta_rejects_empty_registered_language_code() {
        let context = temp_context("empty-language-code");
        let src = write_owner(&context.cwd.join("src"), "Enum", "Statuses", &["Russian"]);
        let object = src.join("Enums/Statuses.xml");
        let language = src.join("Languages/Russian.xml");
        write_file(&object, &sample_meta_named("Enum", "Statuses"));
        write_file(&language, &sample_language_named("Russian", ""));

        let outcome = validate_meta(&meta_validate_args(&object), &context);
        let errors = outcome.errors.join("\n");

        assert!(!outcome.ok, "{outcome:?}");
        assert!(errors.contains("empty LanguageCode"), "{outcome:?}");
        assert!(
            errors.contains(&canonical_path(&language).display().to_string()),
            "{outcome:?}"
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_validation_deduplicates_language_codes_in_registration_order() {
        let context = temp_context("language-code-order");
        let src = write_owner(
            &context.cwd.join("src"),
            "Enum",
            "Statuses",
            &["RussianOne", "English", "RussianTwo"],
        );
        let object = src.join("Enums/Statuses.xml");
        write_file(&object, &sample_meta_named("Enum", "Statuses"));
        for (name, code) in [
            ("RussianOne", "ru"),
            ("English", "en"),
            ("RussianTwo", "ru"),
        ] {
            write_file(
                &src.join("Languages").join(format!("{name}.xml")),
                &sample_language_named(name, code),
            );
        }

        let inspection = inspect_meta_validation_reads(&object, &context);
        let owner = inspection.context.expect("complete language profile");

        assert_eq!(owner.language_codes, vec!["ru", "en"]);
        assert_eq!(
            inspection.paths,
            vec![
                canonical_path(&object),
                canonical_path(&src.join("Configuration.xml")),
                canonical_path(&src.join("Languages/RussianOne.xml")),
                canonical_path(&src.join("Languages/English.xml")),
                canonical_path(&src.join("Languages/RussianTwo.xml")),
            ]
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn meta_validate_batch_read_set_stably_deduplicates_shared_owner() {
        let context = temp_context("batch-read-set");
        let src = context.cwd.join("src");
        let configuration = src.join("Configuration.xml");
        let language = src.join("Languages/Russian.xml");
        let first = src.join("Enums/First.xml");
        let second = src.join("Enums/Second.xml");
        write_file(
            &configuration,
            &format!(
                r#"<MetaDataObject xmlns="{TEST_MD_NS}" version="2.20">
<Configuration uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Owner</Name></Properties>
<ChildObjects><Language>Russian</Language><Enum>First</Enum><Enum>Second</Enum></ChildObjects>
</Configuration></MetaDataObject>"#
            ),
        );
        write_file(&language, &sample_language_named("Russian", "ru"));
        write_file(&first, &sample_meta_named("Enum", "First"));
        write_file(&second, &sample_meta_named("Enum", "Second"));
        let args = Map::from_iter([(
            "ObjectPath".to_string(),
            Value::String(format!("{}|{}", first.display(), second.display())),
        )]);

        let dependencies = meta_validate_format_dependency_paths(&args, &context).unwrap();

        assert_eq!(
            dependencies,
            vec![
                canonical_path(&first),
                canonical_path(&configuration),
                canonical_path(&language),
                canonical_path(&second)
            ]
        );
        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn post_write_metadata_owner_shape_does_not_require_workspace_owner() {
        let context = temp_context("post-write-local");
        let object = context.cwd.join("CommonModules/Local.xml");
        write_file(
            &object,
            &sample_meta_object_xml("CommonModule", "Local", "", "\t\t<ChildObjects/>"),
        );
        write_file(
            &context.cwd.join("Configuration.xml"),
            "<malformed-neighbor",
        );

        validate_metadata_owner_shape_8_3_27(&object, &context, "test")
            .expect("post-write validation must not read a neighboring owner");

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn validate_meta_accepts_filled_short_synonym() {
        let synonym = "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Отгрузка</v8:content></v8:item></Synonym>";
        let stdout = validate_stdout_with_synonym("validate-filled-synonym", synonym);
        assert!(!stdout.contains("Synonym is empty"), "{stdout}");
        assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
    }

    #[test]
    fn validate_meta_warns_on_long_synonym() {
        let synonym = "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Очень длинное наименование для командного интерфейса</v8:content></v8:item></Synonym>";
        let stdout = validate_stdout_with_synonym("validate-long-synonym", synonym);
        assert!(stdout.contains("longer than 38 characters"), "{stdout}");
    }

    #[test]
    fn validate_meta_allows_empty_synonym() {
        let stdout = validate_stdout_with_synonym(
            "validate-empty-synonym-is-allowed",
            "<Synonym><v8:item><v8:lang>en</v8:lang><v8:content/></v8:item></Synonym>",
        );
        assert!(!stdout.contains("Synonym is empty"), "{stdout}");
    }

    #[test]
    fn validate_meta_does_not_apply_list_command_limit_to_common_module() {
        let outcome = validate_registered_object(
            "CommonModule",
            "LongModule",
            &sample_common_module_named(
                "LongModule",
                &[(
                    "ru",
                    "Очень длинный синоним общего модуля для проверки ограничения",
                )],
            ),
            &[],
        );
        let stdout = outcome_text(&outcome);

        assert!(outcome.ok, "{outcome:?}");
        assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
    }

    #[test]
    fn validate_meta_prefers_list_presentation_per_registered_language() {
        let outcome = validate_registered_object(
            "Enum",
            "Status",
            &sample_enum_with_presentations(
                "Status",
                &[
                    (
                        "ru",
                        "Очень длинный синоним для командного интерфейса перечисления",
                    ),
                    ("en", "Status"),
                ],
                &[("ru", "Статусы")],
            ),
            &[("Русский", "ru"), ("English", "en")],
        );
        let stdout = outcome_text(&outcome);

        assert!(outcome.ok, "{outcome:?}");
        assert!(!stdout.contains("language 'ru'"), "{stdout}");
    }

    #[test]
    fn validate_meta_uses_synonym_when_registered_language_has_no_list_presentation() {
        let outcome = validate_registered_object(
            "Enum",
            "Status",
            &sample_enum_with_presentations(
                "Status",
                &[(
                    "en",
                    "A very long status title intended for the command interface",
                )],
                &[("ru", "Статусы")],
            ),
            &[("Русский", "ru"), ("English", "en")],
        );
        let stdout = outcome_text(&outcome);

        assert!(outcome.ok, "{outcome:?}");
        assert!(stdout.contains("Synonym"), "{stdout}");
        assert!(stdout.contains("language 'en'"), "{stdout}");
    }

    #[test]
    fn validate_meta_skips_missing_or_empty_text_for_registered_language() {
        let outcome = validate_registered_object(
            "Enum",
            "Status",
            &sample_enum_with_presentations(
                "Status",
                &[("ru", "Статус"), ("en", "")],
                &[("ru", "Статусы")],
            ),
            &[("Русский", "ru"), ("English", "en")],
        );
        let stdout = outcome_text(&outcome);

        assert!(outcome.ok, "{outcome:?}");
        assert!(!stdout.contains("language 'en'"), "{stdout}");
    }

    #[test]
    fn validate_meta_checks_every_configured_language() {
        let stdout = validate_stdout_with_presentations(
            "validate-every-configured-language",
            &[
                ("ru", "Отгрузка"),
                (
                    "en",
                    "A very long shipment title intended for the command interface",
                ),
            ],
            &[],
            &[("Русский", "ru"), ("English", "en")],
        );
        assert!(stdout.contains("language 'en'"), "{stdout}");
        assert!(stdout.contains("longer than 38 characters"), "{stdout}");
    }

    #[test]
    fn validate_meta_prefers_list_presentation_per_language() {
        let stdout = validate_stdout_with_presentations(
            "validate-list-presentation-per-language",
            &[
                ("ru", "Очень длинное наименование для командного интерфейса"),
                ("en", "Shipment"),
            ],
            &[("ru", "Отгрузки")],
            &[("Русский", "ru"), ("English", "en")],
        );
        assert!(!stdout.contains("language 'ru'"), "{stdout}");
        assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
    }

    #[test]
    fn validate_meta_ignores_unconfigured_translation() {
        let stdout = validate_stdout_with_presentations(
            "validate-ignore-unconfigured-language",
            &[
                (
                    "en",
                    "A very long shipment title intended for the command interface",
                ),
                ("ru", "Отгрузка"),
            ],
            &[],
            &[("Русский", "ru")],
        );
        assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
    }

    #[test]
    fn validate_meta_ignores_non_v8_language_elements() {
        let xml = sample_document_xml("<RegisterRecords/>")
            .replace(
                "xmlns:xsi=",
                "xmlns:foo=\"urn:unrelated\" xmlns:xsi=",
            )
            .replace(
                "<Synonym/>",
                "<Synonym><v8:item><foo:lang>en</foo:lang><v8:content>Shipment</v8:content></v8:item></Synonym>",
            );
        let document = Document::parse(&xml).unwrap();
        let type_node = document
            .root_element()
            .children()
            .find(|node| node.is_element())
            .unwrap();
        let properties = meta_info_child(type_node, "Properties").unwrap();
        let synonym = meta_info_child(properties, "Synonym");

        assert_eq!(
            meta_validate_localized_values(synonym),
            vec![(None, "Shipment".to_string())]
        );
    }

    fn validate_stdout_for_information_register(
        write_mode: &str,
        use_standard_commands: &str,
    ) -> String {
        // The validator requires an InternalInfo block with seven GeneratedType
        // entries and a Type block on every dimension; without them validation
        // fails with unrelated errors before the target warning is checked.
        let internal_info = concat!(
            "\t\t<InternalInfo>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterRecord.SampleRegister\" category=\"Record\">\n",
            "\t\t\t\t<xr:TypeId>11111111-1111-4111-8111-111111111111</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>21111111-1111-4111-8111-111111111111</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterManager.SampleRegister\" category=\"Manager\">\n",
            "\t\t\t\t<xr:TypeId>12222222-2222-4222-8222-222222222222</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>22222222-2222-4222-8222-222222222222</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterSelection.SampleRegister\" category=\"Selection\">\n",
            "\t\t\t\t<xr:TypeId>13333333-3333-4333-8333-333333333333</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>23333333-3333-4333-8333-333333333333</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterList.SampleRegister\" category=\"List\">\n",
            "\t\t\t\t<xr:TypeId>14444444-4444-4444-8444-444444444444</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>24444444-4444-4444-8444-444444444444</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterRecordSet.SampleRegister\" category=\"RecordSet\">\n",
            "\t\t\t\t<xr:TypeId>15555555-5555-4555-8555-555555555555</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>25555555-5555-4555-8555-555555555555</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterRecordKey.SampleRegister\" category=\"RecordKey\">\n",
            "\t\t\t\t<xr:TypeId>16666666-6666-4666-8666-666666666666</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>26666666-6666-4666-8666-666666666666</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t\t<xr:GeneratedType name=\"InformationRegisterRecordManager.SampleRegister\" category=\"RecordManager\">\n",
            "\t\t\t\t<xr:TypeId>17777777-7777-4777-8777-777777777777</xr:TypeId>\n",
            "\t\t\t\t<xr:ValueId>27777777-7777-4777-8777-777777777777</xr:ValueId>\n",
            "\t\t\t</xr:GeneratedType>\n",
            "\t\t</InternalInfo>",
        );
        let xml = sample_meta_object_xml(
            "InformationRegister",
            "SampleRegister",
            &format!(
                "\t\t\t<UseStandardCommands>{use_standard_commands}</UseStandardCommands>\n\t\t\t<WriteMode>{write_mode}</WriteMode>"
            ),
            "\t\t<ChildObjects>\n\t\t\t<Dimension uuid=\"66666666-6666-4666-8666-666666666666\">\n\t\t\t\t<Properties>\n\t\t\t\t\t<Name>SampleDimension</Name>\n\t\t\t\t\t<Type><v8:Type>xs:string</v8:Type></Type>\n\t\t\t\t</Properties>\n\t\t\t</Dimension>\n\t\t</ChildObjects>",
        )
        // Anchor on the register name so the dimension's more deeply indented
        // <Properties> (SampleDimension) is not matched by the shared substring.
        .replace(
            "\t\t<Properties>\n\t\t\t<Name>SampleRegister",
            &format!("{internal_info}\n\t\t<Properties>\n\t\t\t<Name>SampleRegister"),
        );
        let outcome = validate_registered_object(
            "InformationRegister",
            "SampleRegister",
            &xml,
            &[("Русский", "ru")],
        );
        assert!(outcome.ok, "{outcome:?}");
        outcome_text(&outcome)
    }

    const SUBORDINATE_REGISTER_WARNING: &str =
        "subordinate registers are not shown in the command interface";

    #[test]
    fn validate_meta_warns_on_subordinate_register_in_command_interface() {
        let stdout = validate_stdout_for_information_register("RecorderSubordinate", "true");
        assert!(stdout.contains(SUBORDINATE_REGISTER_WARNING), "{stdout}");
    }

    #[test]
    fn validate_meta_allows_independent_register_in_command_interface() {
        let stdout = validate_stdout_for_information_register("Independent", "true");
        assert!(!stdout.contains(SUBORDINATE_REGISTER_WARNING), "{stdout}");
    }

    #[test]
    fn validate_meta_allows_subordinate_register_without_standard_commands() {
        let stdout = validate_stdout_for_information_register("RecorderSubordinate", "false");
        assert!(!stdout.contains(SUBORDINATE_REGISTER_WARNING), "{stdout}");
    }

    #[test]
    fn edit_meta_rejects_unknown_modify_attribute_key() {
        let context = temp_context("modify-attribute-unknown-key");
        let object_path = context.cwd.join("Documents").join("SamplePackingList.xml");
        let xml = sample_document_with_child_objects(&sample_attribute(
            "SampleCargoPlaceCode",
            "\t\t\t\t\t<Type>\n\t\t\t\t\t\t<v8:Type>xs:string</v8:Type>\n\t\t\t\t\t</Type>",
            "\t\t\t\t\t<FillValue xsi:type=\"xs:string\"/>",
        ));
        write_file(&object_path, &xml);

        let outcome = edit_meta(
            &meta_edit_args(
                &object_path,
                "modify-attribute",
                "SampleCargoPlaceCode: typo=1",
            ),
            &context,
        );

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("Unsupported modify property key 'typo'")));
        assert!(!fs::read_to_string(&object_path).unwrap().contains("<typo>"));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_rejects_register_record_duplicate_with_formatted_text() {
        let context = temp_context("duplicate-register-record-formatted");
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        let original = sample_document_xml(
            r#"<RegisterRecords>
				<xr:Item xsi:type="xr:MDObjectRef">
					AccumulationRegister.SampleUnshippedGoods
				</xr:Item>
			</RegisterRecords>"#,
        );
        write_file(&object_path, &original);

        let outcome = edit_meta(&register_record_args(&object_path), &context);

        assert!(!outcome.ok, "{:?}", outcome.stdout);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("already exists")));
        assert_eq!(fs::read_to_string(&object_path).unwrap(), original);

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_dry_run_rejects_unsupported_operation() {
        let context = temp_context("dry-run-unsupported-operation");
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));
        let mut args = meta_edit_args(&object_path, "definitely-unsupported", "Value");
        args.insert("dryRun".to_string(), json!(true));

        let error = UnicaApplication::new()
            .call_tool("unica.meta.edit", &args)
            .unwrap_err();

        assert!(error.contains("unsupported Operation"));

        let _ = fs::remove_dir_all(&context.cwd);
    }

    #[test]
    fn edit_meta_dry_run_accepts_definition_file_mode() {
        let context = temp_context("dry-run-definition-file");
        let object_path = context.cwd.join("Documents").join("SampleShipment.xml");
        let definition_path = context.cwd.join("edit.json");
        write_file(&object_path, &sample_document_xml("<RegisterRecords/>"));
        write_file(&definition_path, "{}");
        let mut args = Map::new();
        args.insert(
            "ObjectPath".to_string(),
            json!(object_path.display().to_string()),
        );
        args.insert(
            "DefinitionFile".to_string(),
            json!(definition_path.display().to_string()),
        );
        args.insert("dryRun".to_string(), json!(true));

        let result = UnicaApplication::new()
            .call_tool("unica.meta.edit", &args)
            .unwrap();

        assert!(result.ok);

        let _ = fs::remove_dir_all(&context.cwd);
    }
}

#[cfg(test)]
mod meta_info_logical_target_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn workspace(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-meta-info-logical-{name}-{nanos}"));
        fs::create_dir_all(root.join("src/Catalogs/Items/Ext")).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name><Synonym><v8:item xmlns:v8="http://v8.1c.ru/8.1/data/core"><v8:lang>ru</v8:lang><v8:content>Номенклатура</v8:content></v8:item></Synonym></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Catalogs/Items/Ext/ObjectModule.bsl"),
            "Procedure BeforeWrite()\nEndProcedure\n",
        )
        .unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn info_args(address: &str) -> Map<String, Value> {
        Map::from_iter([
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!(address)),
        ])
    }

    #[test]
    fn meta_info_reads_the_descriptor_named_by_a_logical_address() {
        let context = workspace("reads");

        let execution = analyze_meta_info_with_data(&info_args("Catalog.Items"), &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let data = execution.data.expect("a resolved target is reported");
        assert_eq!(data.kind, "Catalog");
        assert_eq!(data.name, "Items");
        assert_eq!(data.target.source_set, "main");
        assert_eq!(
            data.target.metadata_path.as_ref().map(|path| path.as_str()),
            Some("Catalog.Items")
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// The profile accepts a Russian kind alias and answers with the canonical
    /// English address, so the answer can be fed back to any logical tool.
    #[test]
    fn meta_info_accepts_a_russian_kind_alias_and_answers_with_the_canonical_address() {
        let context = workspace("alias");

        let execution = analyze_meta_info_with_data(&info_args("Справочник.Items"), &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        assert_eq!(
            execution
                .data
                .and_then(|data| data.target.metadata_path)
                .map(|path| path.as_str().to_string()),
            Some("Catalog.Items".to_string())
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// Subordination is the structural fact that separates one catalog from
    /// another; reading it required opening the raw XML before.
    #[test]
    fn meta_info_reports_owners_and_their_absence() {
        let context = workspace("owners");
        let subordinate = context.workspace_root.join("src/Catalogs/Series.xml");
        fs::write(
            &subordinate,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Catalog><Properties><Name>Series</Name><Owners><xr:Item>Catalog.Items</xr:Item><xr:Item>Catalog.Kinds</xr:Item></Owners></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            context.workspace_root.join("src/Catalogs/Plain.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Plain</Name><Owners/></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();

        let subordinate = analyze_meta_info_with_data(&info_args("Catalog.Series"), &context);
        let plain = analyze_meta_info_with_data(&info_args("Catalog.Plain"), &context);

        assert!(subordinate.outcome.ok, "{:?}", subordinate.outcome);
        assert_eq!(
            subordinate
                .data
                .expect("meta.info answers with data")
                .owners,
            vec!["Catalog.Items".to_string(), "Catalog.Kinds".to_string()]
        );
        assert!(plain.outcome.ok, "{:?}", plain.outcome);
        // An empty list is the answer: the catalog is not subordinate.
        assert!(plain
            .data
            .expect("meta.info answers with data")
            .owners
            .is_empty());
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// A silent property cannot be told apart from an unreported one, which is
    /// what forced a reader to the XML to learn a catalog is flat.
    #[test]
    fn meta_info_states_catalog_properties_including_their_negatives() {
        let context = workspace("catalog-properties");
        fs::write(
            context.workspace_root.join("src/Catalogs/Flat.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Flat</Name><Hierarchical>false</Hierarchical><CodeLength>0</CodeLength><DescriptionLength>150</DescriptionLength><DefaultPresentation>AsCode</DefaultPresentation></Properties><ChildObjects><Form>ФормаЭлемента</Form></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();

        let execution = analyze_meta_info_with_data(&info_args("Catalog.Flat"), &context);

        assert!(execution.outcome.ok, "{:?}", execution.outcome);
        let data = execution.data.expect("meta.info answers with data");
        // Properties keep the platform's own names and values, so a negative is
        // stated rather than left out.
        let property = |name: &str| {
            data.properties
                .iter()
                .find(|property| property.name == name)
                .map(|property| property.value.as_str())
        };
        assert_eq!(property("Hierarchical"), Some("false"), "{data:?}");
        assert_eq!(property("CodeLength"), Some("0"), "{data:?}");
        assert_eq!(property("DescriptionLength"), Some("150"), "{data:?}");
        assert_eq!(property("DefaultPresentation"), Some("AsCode"), "{data:?}");
        // Forms used to appear in overview only for reports and data processors.
        assert_eq!(data.forms, vec!["ФормаЭлемента".to_string()]);
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    /// Reading a module is `unica.code.*` work. Quietly reading the owner
    /// instead would answer a question the caller did not ask.
    #[test]
    fn meta_info_refuses_a_module_terminal_by_name() {
        let context = workspace("module");

        let outcome = analyze_meta_info(&info_args("Catalog.Items.ObjectModule"), &context);

        assert!(!outcome.ok);
        assert!(
            outcome.errors[0].contains("names a module terminal"),
            "{:?}",
            outcome.errors
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn meta_info_reports_an_unknown_address_without_naming_a_path() {
        let context = workspace("unknown");

        let outcome = analyze_meta_info(&info_args("Catalog.Missing"), &context);

        assert!(!outcome.ok);
        assert!(
            outcome.errors[0].contains("Catalog.Missing"),
            "{:?}",
            outcome.errors
        );
        assert!(
            !outcome.errors[0].contains("Catalogs/"),
            "{:?}",
            outcome.errors
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }

    #[test]
    fn meta_info_requires_a_source_set() {
        let context = workspace("no-source-set");

        let outcome = analyze_meta_info(
            &Map::from_iter([("metadataPath".to_string(), json!("Catalog.Items"))]),
            &context,
        );

        assert!(!outcome.ok);
        assert!(
            outcome.errors[0].contains("sourceSet"),
            "{:?}",
            outcome.errors
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_info_directory_fallback_selects_first_xml_by_file_name() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-info-sorted-fallback-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let later = root.join("z-later.xml");
        let first = root.join("a-first.xml");
        fs::write(&later, "<later/>").unwrap();
        fs::write(&first, "<first/>").unwrap();

        assert_eq!(resolve_meta_info_path(root.clone()).unwrap(), first);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn exchange_plan_content_matches_8_3_27() {
        let files = meta_compile_extra_ext_files("ExchangePlan", "2.20");
        let (name, content) = &files[0];
        let expected = include_str!(
            "../../../../../../tests/fixtures/platform_8_3_27/exchange_plan/Content.xml"
        );

        assert_eq!(*name, "Content.xml");
        assert_eq!(
            content.replace("\r\n", "\n"),
            expected.replace("\r\n", "\n")
        );
    }

    #[test]
    fn standard_attributes_emit_platform_type_reduction_modes_in_order() {
        let (xml, _) = meta_compile_catalog_xml(&Map::new(), "CorpusCatalog", "2.20").unwrap();
        let document = Document::parse(&xml).unwrap();
        let standard_attributes = document
            .descendants()
            .find(|node| node.tag_name().name() == "StandardAttributes")
            .unwrap();
        let expected = [
            ("PredefinedDataName", "TransformValues"),
            ("Predefined", "TransformValues"),
            ("Ref", "TransformValues"),
            ("DeletionMark", "TransformValues"),
            ("IsFolder", "TransformValues"),
            ("Owner", "Deny"),
            ("Parent", "TransformValues"),
            ("Description", "TransformValues"),
            ("Code", "TransformValues"),
        ];
        let attributes = standard_attributes
            .children()
            .filter(|node| node.is_element())
            .collect::<Vec<_>>();
        assert_eq!(attributes.len(), expected.len());

        for (attribute, (expected_name, expected_mode)) in attributes.iter().zip(expected) {
            assert_eq!(attribute.attribute("name"), Some(expected_name));
            let children = attribute
                .children()
                .filter(|node| node.is_element())
                .collect::<Vec<_>>();
            let child_names = children
                .iter()
                .map(|node| node.tag_name().name())
                .collect::<Vec<_>>();
            let create_on_input = child_names
                .iter()
                .position(|name| *name == "CreateOnInput")
                .unwrap();
            let type_reduction_mode = child_names
                .iter()
                .position(|name| *name == "TypeReductionMode")
                .unwrap();
            let max_value = child_names
                .iter()
                .position(|name| *name == "MaxValue")
                .unwrap();
            assert_eq!(type_reduction_mode, create_on_input + 1, "{expected_name}");
            assert_eq!(max_value, type_reduction_mode + 1, "{expected_name}");
            assert_eq!(children[type_reduction_mode].text(), Some(expected_mode));
        }

        for object_type in [
            "Document",
            "Enum",
            "InformationRegister",
            "AccumulationRegister",
            "AccountingRegister",
            "CalculationRegister",
            "ChartOfAccounts",
            "ChartOfCharacteristicTypes",
            "ChartOfCalculationTypes",
            "BusinessProcess",
            "Task",
            "ExchangePlan",
            "DocumentJournal",
            "TabularSection",
        ] {
            let mut lines = Vec::new();
            emit_meta_standard_attributes(&mut lines, "\t", object_type);
            let xml = format!(
                "<Properties xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">{}</Properties>",
                lines.join("")
            );
            let document = Document::parse(&xml).unwrap();
            let attributes = document
                .descendants()
                .filter(|node| node.tag_name().name() == "StandardAttribute")
                .collect::<Vec<_>>();
            assert!(!attributes.is_empty(), "{object_type}");
            for attribute in attributes {
                let children = attribute
                    .children()
                    .filter(|node| node.is_element())
                    .collect::<Vec<_>>();
                let child_names = children
                    .iter()
                    .map(|node| node.tag_name().name())
                    .collect::<Vec<_>>();
                let create_on_input = child_names
                    .iter()
                    .position(|name| *name == "CreateOnInput")
                    .unwrap();
                let type_reduction_mode = child_names
                    .iter()
                    .position(|name| *name == "TypeReductionMode")
                    .unwrap();
                let max_value = child_names
                    .iter()
                    .position(|name| *name == "MaxValue")
                    .unwrap();
                assert_eq!(
                    type_reduction_mode,
                    create_on_input + 1,
                    "{object_type}.{}",
                    attribute.attribute("name").unwrap_or("")
                );
                assert_eq!(max_value, type_reduction_mode + 1, "{object_type}");
                assert_eq!(
                    children[type_reduction_mode].text(),
                    Some("TransformValues"),
                    "{object_type}.{}",
                    attribute.attribute("name").unwrap_or("")
                );
            }
        }

        let mut ext_dimension_lines = Vec::new();
        emit_meta_standard_attribute(
            &mut ext_dimension_lines,
            "",
            "ChartOfAccounts.ExtDimensionTypes",
            "ExtDimensionType",
        );
        assert!(
            ext_dimension_lines.iter().any(|line| line
                .contains("<xr:TypeReductionMode>TransformValues</xr:TypeReductionMode>")),
            "{}",
            ext_dimension_lines.join("\n")
        );
    }

    #[test]
    fn document_tabular_section_emits_platform_line_number_length() {
        let definition = json!({
            "tabularSections": {
                "Lines": ["Quantity:Number(10,2)"]
            }
        });
        let (xml, _) = meta_compile_object_xml(
            definition.as_object().unwrap(),
            "Document",
            "CorpusDocument",
            "2.20",
        )
        .unwrap();
        let document = Document::parse(&xml).unwrap();
        let properties = document
            .descendants()
            .find(|node| {
                node.tag_name().name() == "TabularSection" && node.attribute("uuid").is_some()
            })
            .and_then(|section| {
                section
                    .children()
                    .find(|node| node.is_element() && node.tag_name().name() == "Properties")
            })
            .unwrap();
        let children = properties
            .children()
            .filter(|node| node.is_element())
            .collect::<Vec<_>>();
        let standard_attributes = children
            .iter()
            .position(|node| node.tag_name().name() == "StandardAttributes")
            .unwrap();
        let line_number_length = children
            .iter()
            .position(|node| node.tag_name().name() == "LineNumberLength")
            .unwrap();

        assert_eq!(line_number_length, standard_attributes + 1, "{xml}");
        assert_eq!(children[line_number_length].text(), Some("9"), "{xml}");
    }

    #[test]
    fn business_process_flowchart_matches_platform_8_3_27_schema() {
        let files = meta_compile_extra_ext_files("BusinessProcess", "2.20");
        let (name, content) = files
            .iter()
            .find(|(name, _)| *name == "Flowchart.xml")
            .unwrap();
        let document = Document::parse(content.trim_start_matches('\u{feff}')).unwrap();
        let root = document.root_element();

        assert_eq!(*name, "Flowchart.xml");
        assert_eq!(root.tag_name().name(), "GraphicalSchema");
        assert_eq!(
            root.tag_name().namespace(),
            Some("http://v8.1c.ru/8.3/xcf/scheme")
        );
        assert_eq!(root.attribute("version"), Some("2.20"));
        assert_eq!(
            test_meta_direct_child_names(root),
            [
                "BackColor",
                "GridEnabled",
                "DrawGridMode",
                "GridHorizontalStep",
                "GridVerticalStep",
                "PrintParameters",
                "Items",
            ]
        );
        let print_parameters = meta_info_child(root, "PrintParameters").unwrap();
        assert_eq!(
            test_meta_direct_child_names(print_parameters),
            [
                "TopMargin",
                "LeftMargin",
                "BottomMargin",
                "RightMargin",
                "BlackAndWhite",
                "FitPageMode",
            ]
        );
    }

    #[test]
    fn chart_characteristic_generated_type_uses_platform_prefix() {
        let xml = test_compile_meta_xml(
            "ChartOfCharacteristicTypes",
            "CorpusCharacteristics",
            json!({}),
        );
        let document = Document::parse(&xml).unwrap();
        let characteristic = document
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().name() == "GeneratedType"
                    && node.attribute("category") == Some("Characteristic")
            })
            .unwrap();

        assert_eq!(
            characteristic.attribute("name"),
            Some("Characteristic.CorpusCharacteristics")
        );
    }

    #[test]
    fn multi_value_types_follow_platform_type_description_order() {
        for (object_type, object_name) in [
            ("ChartOfCharacteristicTypes", "CorpusCharacteristics"),
            ("DefinedType", "CorpusDefinedType"),
        ] {
            let xml = test_compile_meta_xml(
                object_type,
                object_name,
                json!({"valueTypes": ["String(100)", "Number(15,2)"]}),
            );
            let document = Document::parse(&xml).unwrap();
            let properties = test_meta_root_properties(&document);
            let type_node = meta_info_child(properties, "Type").unwrap();

            assert_eq!(
                test_meta_direct_child_names(type_node),
                ["Type", "Type", "NumberQualifiers", "StringQualifiers"],
                "{object_type}: {xml}"
            );
        }
    }

    #[test]
    fn object_specific_standard_attributes_match_platform_order() {
        for (object_type, expected) in [
            (
                "BusinessProcess",
                vec![
                    "Started",
                    "HeadTask",
                    "Completed",
                    "Ref",
                    "DeletionMark",
                    "Date",
                    "Number",
                ],
            ),
            (
                "Task",
                vec![
                    "Executed",
                    "Description",
                    "RoutePoint",
                    "BusinessProcess",
                    "Ref",
                    "DeletionMark",
                    "Date",
                    "Number",
                ],
            ),
            (
                "ChartOfCharacteristicTypes",
                vec![
                    "PredefinedDataName",
                    "ValueType",
                    "Description",
                    "Code",
                    "IsFolder",
                    "Parent",
                    "Predefined",
                    "DeletionMark",
                    "Ref",
                ],
            ),
            (
                "ExchangePlan",
                vec![
                    "ExchangeDate",
                    "ThisNode",
                    "ReceivedNo",
                    "SentNo",
                    "Ref",
                    "DeletionMark",
                    "Description",
                    "Code",
                ],
            ),
        ] {
            let mut lines = Vec::new();
            emit_meta_standard_attributes(&mut lines, "", object_type);
            let xml = format!(
                "<Properties xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">{}</Properties>",
                lines.join("")
            );
            let document = Document::parse(&xml).unwrap();
            let attributes = document
                .descendants()
                .filter(|node| node.is_element() && node.tag_name().name() == "StandardAttribute")
                .map(|node| node.attribute("name").unwrap_or_default())
                .collect::<Vec<_>>();

            assert_eq!(attributes, expected, "{object_type}");
        }
    }

    #[test]
    fn business_process_properties_match_platform_order_and_defaults() {
        let xml = test_compile_meta_xml(
            "BusinessProcess",
            "CorpusBusinessProcess",
            json!({"task": "Task.CorpusTask"}),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_direct_child_names(properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "EditType",
                "InputByString",
                "CreateOnInput",
                "SearchStringModeOnInputByString",
                "ChoiceDataGetModeOnInputByString",
                "FullTextSearchOnInputByString",
                "DefaultObjectForm",
                "DefaultListForm",
                "DefaultChoiceForm",
                "AuxiliaryObjectForm",
                "AuxiliaryListForm",
                "AuxiliaryChoiceForm",
                "ChoiceHistoryOnInput",
                "NumberType",
                "NumberLength",
                "NumberAllowedLength",
                "CheckUnique",
                "StandardAttributes",
                "Characteristics",
                "Autonumbering",
                "BasedOn",
                "NumberPeriodicity",
                "Task",
                "CreateTaskInPrivilegedMode",
                "DataLockFields",
                "DataLockControlMode",
                "IncludeHelpInContents",
                "FullTextSearch",
                "ObjectPresentation",
                "ExtendedObjectPresentation",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
                "DataHistory",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ]
        );
        assert_eq!(
            meta_info_child_text(properties, "NumberPeriodicity").as_deref(),
            Some("Nonperiodical")
        );
        assert_eq!(
            meta_info_child_text(properties, "CreateTaskInPrivilegedMode").as_deref(),
            Some("true")
        );
    }

    #[test]
    fn task_properties_match_platform_order_and_defaults() {
        let xml = test_compile_meta_xml("Task", "CorpusTask", json!({}));
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_direct_child_names(properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "NumberType",
                "NumberLength",
                "NumberAllowedLength",
                "CheckUnique",
                "Autonumbering",
                "TaskNumberAutoPrefix",
                "DescriptionLength",
                "Addressing",
                "MainAddressingAttribute",
                "CurrentPerformer",
                "BasedOn",
                "StandardAttributes",
                "Characteristics",
                "DefaultPresentation",
                "EditType",
                "InputByString",
                "SearchStringModeOnInputByString",
                "FullTextSearchOnInputByString",
                "ChoiceDataGetModeOnInputByString",
                "CreateOnInput",
                "DefaultObjectForm",
                "DefaultListForm",
                "DefaultChoiceForm",
                "AuxiliaryObjectForm",
                "AuxiliaryListForm",
                "AuxiliaryChoiceForm",
                "ChoiceHistoryOnInput",
                "IncludeHelpInContents",
                "DataLockFields",
                "DataLockControlMode",
                "FullTextSearch",
                "ObjectPresentation",
                "ExtendedObjectPresentation",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
                "DataHistory",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ]
        );
    }

    #[test]
    fn chart_characteristic_properties_match_platform_order_and_defaults() {
        let xml = test_compile_meta_xml(
            "ChartOfCharacteristicTypes",
            "CorpusCharacteristics",
            json!({"valueTypes": ["String(50)", "Number(15,2)"]}),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_direct_child_names(properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "IncludeHelpInContents",
                "CharacteristicExtValues",
                "Type",
                "Hierarchical",
                "FoldersOnTop",
                "CodeLength",
                "CodeAllowedLength",
                "DescriptionLength",
                "CodeSeries",
                "CheckUnique",
                "Autonumbering",
                "DefaultPresentation",
                "StandardAttributes",
                "Characteristics",
                "PredefinedDataUpdate",
                "EditType",
                "QuickChoice",
                "ChoiceMode",
                "InputByString",
                "CreateOnInput",
                "SearchStringModeOnInputByString",
                "ChoiceDataGetModeOnInputByString",
                "FullTextSearchOnInputByString",
                "ChoiceHistoryOnInput",
                "DefaultObjectForm",
                "DefaultFolderForm",
                "DefaultListForm",
                "DefaultChoiceForm",
                "DefaultFolderChoiceForm",
                "AuxiliaryObjectForm",
                "AuxiliaryFolderForm",
                "AuxiliaryListForm",
                "AuxiliaryChoiceForm",
                "AuxiliaryFolderChoiceForm",
                "BasedOn",
                "DataLockFields",
                "DataLockControlMode",
                "FullTextSearch",
                "ObjectPresentation",
                "ExtendedObjectPresentation",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
                "DataHistory",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ]
        );
        assert_eq!(
            meta_info_child_text(properties, "CodeSeries").as_deref(),
            Some("WholeCharacteristicKind")
        );
    }

    #[test]
    fn accumulation_register_standard_attributes_match_platform_order() {
        let xml = test_compile_meta_xml(
            "AccumulationRegister",
            "CorpusAccumulationRegister",
            json!({
                "registerType": "Balances",
                "dimensions": ["Warehouse:String(50)|index"],
                "resources": ["Quantity:Number(15,3)"]
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_standard_attribute_names(properties),
            ["RecordType", "Active", "LineNumber", "Recorder", "Period"],
            "{xml}"
        );
    }

    #[test]
    fn accounting_register_matches_platform_order_and_defaults() {
        let xml = test_compile_meta_xml(
            "AccountingRegister",
            "CorpusAccountingRegister",
            json!({
                "chartOfAccounts": "ChartOfAccounts.CorpusAccounts",
                "dimensions": ["Department:String(50)"],
                "resources": ["Amount:Number(15,2)"]
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_direct_child_names(properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "IncludeHelpInContents",
                "ChartOfAccounts",
                "Correspondence",
                "PeriodAdjustmentLength",
                "DefaultListForm",
                "AuxiliaryListForm",
                "StandardAttributes",
                "DataLockControlMode",
                "EnableTotalsSplitting",
                "FullTextSearch",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
            ],
            "{xml}"
        );
        assert_eq!(
            test_meta_standard_attribute_names(properties),
            [
                "Account",
                "RecordType",
                "Active",
                "LineNumber",
                "Recorder",
                "Period"
            ],
            "{xml}"
        );
        assert_eq!(
            meta_info_child_text(properties, "EnableTotalsSplitting").as_deref(),
            Some("false"),
            "{xml}"
        );

        let child_objects = test_meta_root_child_objects(&document);
        assert_eq!(
            test_meta_direct_child_names(child_objects),
            ["Dimension", "Resource"],
            "{xml}"
        );
        let dimension = test_meta_named_object(&document, "Dimension", "Department");
        assert_eq!(
            test_meta_direct_child_names(meta_info_child(dimension, "Properties").unwrap()),
            [
                "Name",
                "Synonym",
                "Comment",
                "Type",
                "PasswordMode",
                "Format",
                "EditFormat",
                "ToolTip",
                "MarkNegatives",
                "Mask",
                "MultiLine",
                "ExtendedEdit",
                "MinValue",
                "MaxValue",
                "FillChecking",
                "ChoiceFoldersAndItems",
                "ChoiceParameterLinks",
                "ChoiceParameters",
                "QuickChoice",
                "CreateOnInput",
                "ChoiceForm",
                "LinkByType",
                "ChoiceHistoryOnInput",
                "Balance",
                "AccountingFlag",
                "DenyIncompleteValues",
                "Indexing",
                "FullTextSearch",
            ],
            "{xml}"
        );
        let resource = test_meta_named_object(&document, "Resource", "Amount");
        assert_eq!(
            test_meta_direct_child_names(meta_info_child(resource, "Properties").unwrap()),
            [
                "Name",
                "Synonym",
                "Comment",
                "Type",
                "PasswordMode",
                "Format",
                "EditFormat",
                "ToolTip",
                "MarkNegatives",
                "Mask",
                "MultiLine",
                "ExtendedEdit",
                "MinValue",
                "MaxValue",
                "FillChecking",
                "ChoiceFoldersAndItems",
                "ChoiceParameterLinks",
                "ChoiceParameters",
                "QuickChoice",
                "CreateOnInput",
                "ChoiceForm",
                "LinkByType",
                "ChoiceHistoryOnInput",
                "Balance",
                "AccountingFlag",
                "ExtDimensionAccountingFlag",
                "FullTextSearch",
            ],
            "{xml}"
        );
    }

    #[test]
    fn calculation_register_matches_platform_order_and_defaults() {
        let xml = test_compile_meta_xml(
            "CalculationRegister",
            "CorpusCalculationRegister",
            json!({
                "chartOfCalculationTypes": "ChartOfCalculationTypes.CorpusCalculationTypes",
                "periodicity": "Month",
                "dimensions": ["Employee:String(50)"],
                "resources": ["Result:Number(15,2)"]
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_direct_child_names(properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "DefaultListForm",
                "AuxiliaryListForm",
                "Periodicity",
                "ActionPeriod",
                "BasePeriod",
                "Schedule",
                "ScheduleValue",
                "ScheduleDate",
                "ChartOfCalculationTypes",
                "IncludeHelpInContents",
                "StandardAttributes",
                "DataLockControlMode",
                "FullTextSearch",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
            ],
            "{xml}"
        );
        assert_eq!(
            test_meta_standard_attribute_names(properties),
            [
                "RegistrationPeriod",
                "ReversingEntry",
                "Active",
                "EndOfBasePeriod",
                "BegOfBasePeriod",
                "EndOfActionPeriod",
                "BegOfActionPeriod",
                "ActionPeriod",
                "CalculationType",
                "LineNumber",
                "Recorder",
            ],
            "{xml}"
        );
        let dimension = test_meta_named_object(&document, "Dimension", "Employee");
        let dimension_properties = meta_info_child(dimension, "Properties").unwrap();
        assert_eq!(
            test_meta_direct_child_names(dimension_properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "Type",
                "PasswordMode",
                "Format",
                "EditFormat",
                "ToolTip",
                "MarkNegatives",
                "Mask",
                "MultiLine",
                "ExtendedEdit",
                "MinValue",
                "MaxValue",
                "FillChecking",
                "ChoiceFoldersAndItems",
                "ChoiceParameterLinks",
                "ChoiceParameters",
                "QuickChoice",
                "CreateOnInput",
                "ChoiceForm",
                "LinkByType",
                "ChoiceHistoryOnInput",
                "DenyIncompleteValues",
                "BaseDimension",
                "ScheduleLink",
                "Indexing",
                "FullTextSearch",
            ],
            "{xml}"
        );
    }

    #[test]
    fn chart_of_accounts_matches_platform_order_and_defaults() {
        let xml = test_compile_meta_xml(
            "ChartOfAccounts",
            "CorpusAccounts",
            json!({"accountingFlags": ["Tax"]}),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_direct_child_names(properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "IncludeHelpInContents",
                "BasedOn",
                "ExtDimensionTypes",
                "MaxExtDimensionCount",
                "CodeMask",
                "CodeLength",
                "DescriptionLength",
                "CodeSeries",
                "CheckUnique",
                "DefaultPresentation",
                "StandardAttributes",
                "Characteristics",
                "StandardTabularSections",
                "PredefinedDataUpdate",
                "EditType",
                "QuickChoice",
                "ChoiceMode",
                "InputByString",
                "SearchStringModeOnInputByString",
                "FullTextSearchOnInputByString",
                "ChoiceDataGetModeOnInputByString",
                "CreateOnInput",
                "ChoiceHistoryOnInput",
                "DefaultObjectForm",
                "DefaultListForm",
                "DefaultChoiceForm",
                "AuxiliaryObjectForm",
                "AuxiliaryListForm",
                "AuxiliaryChoiceForm",
                "AutoOrderByCode",
                "OrderLength",
                "DataLockFields",
                "DataLockControlMode",
                "FullTextSearch",
                "DataHistory",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
                "ObjectPresentation",
                "ExtendedObjectPresentation",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
            ],
            "{xml}"
        );
        assert_eq!(
            meta_info_child_text(properties, "MaxExtDimensionCount").as_deref(),
            Some("0"),
            "{xml}"
        );
        assert_eq!(
            meta_info_child_text(properties, "CodeSeries").as_deref(),
            Some("WholeChartOfAccounts"),
            "{xml}"
        );
        assert_eq!(
            test_meta_standard_attribute_names(properties),
            [
                "PredefinedDataName",
                "Order",
                "OffBalance",
                "Type",
                "Description",
                "Code",
                "Parent",
                "Predefined",
                "DeletionMark",
                "Ref",
            ],
            "{xml}"
        );
        let section = meta_info_child(properties, "StandardTabularSections")
            .unwrap()
            .children()
            .find(roxmltree::Node::is_element)
            .unwrap();
        assert_eq!(
            test_meta_direct_child_names(section),
            [
                "Synonym",
                "Comment",
                "ToolTip",
                "FillChecking",
                "StandardAttributes"
            ],
            "{xml}"
        );
        assert_eq!(
            section
                .descendants()
                .find(|node| node.tag_name().name() == "content")
                .and_then(|node| node.text()),
            Some("Extra dimension types"),
            "{xml}"
        );
        let flag = test_meta_named_object(&document, "AccountingFlag", "Tax");
        assert_eq!(
            test_meta_direct_child_names(meta_info_child(flag, "Properties").unwrap()),
            [
                "Name",
                "Synonym",
                "Comment",
                "Type",
                "PasswordMode",
                "Format",
                "EditFormat",
                "ToolTip",
                "MarkNegatives",
                "Mask",
                "MultiLine",
                "ExtendedEdit",
                "MinValue",
                "MaxValue",
                "FillFromFillingValue",
                "FillValue",
                "FillChecking",
                "ChoiceFoldersAndItems",
                "ChoiceParameterLinks",
                "ChoiceParameters",
                "QuickChoice",
                "CreateOnInput",
                "ChoiceForm",
                "LinkByType",
                "ChoiceHistoryOnInput",
                "DataHistory",
            ],
            "{xml}"
        );
    }

    #[test]
    fn chart_of_calculation_types_matches_platform_order_and_defaults() {
        let xml = test_compile_meta_xml(
            "ChartOfCalculationTypes",
            "CorpusCalculationTypes",
            json!({}),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_direct_child_names(properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "CodeLength",
                "DescriptionLength",
                "CodeType",
                "CodeAllowedLength",
                "DefaultPresentation",
                "EditType",
                "QuickChoice",
                "ChoiceMode",
                "InputByString",
                "SearchStringModeOnInputByString",
                "FullTextSearchOnInputByString",
                "ChoiceDataGetModeOnInputByString",
                "CreateOnInput",
                "ChoiceHistoryOnInput",
                "DefaultObjectForm",
                "DefaultListForm",
                "DefaultChoiceForm",
                "AuxiliaryObjectForm",
                "AuxiliaryListForm",
                "AuxiliaryChoiceForm",
                "BasedOn",
                "DependenceOnCalculationTypes",
                "BaseCalculationTypes",
                "ActionPeriodUse",
                "StandardAttributes",
                "Characteristics",
                "PredefinedDataUpdate",
                "IncludeHelpInContents",
                "DataLockFields",
                "DataLockControlMode",
                "FullTextSearch",
                "ObjectPresentation",
                "ExtendedObjectPresentation",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
                "DataHistory",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ],
            "{xml}"
        );
        assert_eq!(
            test_meta_standard_attribute_names(properties),
            [
                "PredefinedDataName",
                "Predefined",
                "Ref",
                "DeletionMark",
                "ActionPeriodIsBasic",
                "Description",
                "Code",
            ],
            "{xml}"
        );
    }

    #[test]
    fn exchange_plan_properties_match_platform_order_and_defaults() {
        let xml = test_compile_meta_xml(
            "ExchangePlan",
            "CorpusExchangePlan",
            json!({
                "distributedInfoBase": true,
                "includeConfigurationExtensions": true
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);

        assert_eq!(
            test_meta_direct_child_names(properties),
            [
                "Name",
                "Synonym",
                "Comment",
                "UseStandardCommands",
                "CodeLength",
                "CodeAllowedLength",
                "DescriptionLength",
                "DefaultPresentation",
                "EditType",
                "QuickChoice",
                "ChoiceMode",
                "InputByString",
                "SearchStringModeOnInputByString",
                "FullTextSearchOnInputByString",
                "ChoiceDataGetModeOnInputByString",
                "DefaultObjectForm",
                "DefaultListForm",
                "DefaultChoiceForm",
                "AuxiliaryObjectForm",
                "AuxiliaryListForm",
                "AuxiliaryChoiceForm",
                "StandardAttributes",
                "Characteristics",
                "BasedOn",
                "DistributedInfoBase",
                "IncludeConfigurationExtensions",
                "CreateOnInput",
                "ChoiceHistoryOnInput",
                "IncludeHelpInContents",
                "DataLockFields",
                "DataLockControlMode",
                "FullTextSearch",
                "ObjectPresentation",
                "ExtendedObjectPresentation",
                "ListPresentation",
                "ExtendedListPresentation",
                "Explanation",
                "DataHistory",
                "UpdateDataHistoryImmediatelyAfterWrite",
                "ExecuteAfterWriteDataHistoryVersionProcessing",
            ]
        );
    }

    #[test]
    fn constant_data_lock_mode_precedes_data_history() {
        let xml = test_compile_meta_xml(
            "Constant",
            "CorpusConstant",
            json!({"valueType": "Boolean"}),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);
        let names = test_meta_direct_child_names(properties);
        let lock_mode = names
            .iter()
            .position(|name| name == "DataLockControlMode")
            .unwrap();
        let data_history = names.iter().position(|name| name == "DataHistory").unwrap();

        assert_eq!(lock_mode + 1, data_history, "{xml}");
    }

    #[test]
    fn unbounded_tabular_sections_omit_line_number_length() {
        for object_type in ["DataProcessor", "Report"] {
            let xml = test_compile_meta_xml(
                object_type,
                &format!("Corpus{object_type}"),
                json!({"tabularSections": {"Rows": ["Value:String(100)"]}}),
            );
            let document = Document::parse(&xml).unwrap();
            let section = test_meta_named_object(&document, "TabularSection", "Rows");
            let properties = meta_info_child(section, "Properties").unwrap();

            assert!(
                meta_info_child(properties, "LineNumberLength").is_none(),
                "{object_type}: {xml}"
            );
        }
    }

    #[test]
    fn document_journal_includes_help_before_standard_attributes() {
        let xml = test_compile_meta_xml(
            "DocumentJournal",
            "CorpusDocumentJournal",
            json!({"registeredDocuments": ["Document.CorpusDocument"]}),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);
        let names = test_meta_direct_child_names(properties);
        let registered = names
            .iter()
            .position(|name| name == "RegisteredDocuments")
            .unwrap();
        let include_help = names
            .iter()
            .position(|name| name == "IncludeHelpInContents")
            .unwrap();
        let standard = names
            .iter()
            .position(|name| name == "StandardAttributes")
            .unwrap();

        assert_eq!(include_help, registered + 1, "{xml}");
        assert_eq!(standard, include_help + 1, "{xml}");
    }

    #[test]
    fn http_service_child_properties_include_comments() {
        let xml = test_compile_meta_xml(
            "HTTPService",
            "CorpusHTTPService",
            json!({
                "urlTemplates": {
                    "Items": {"template": "/items/{id}", "methods": {"Get": "GET"}}
                }
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let template = test_meta_named_object(&document, "URLTemplate", "Items");
        let method = test_meta_named_object(&document, "Method", "Get");

        assert_eq!(
            test_meta_direct_child_names(meta_info_child(template, "Properties").unwrap()),
            ["Name", "Synonym", "Comment", "Template"]
        );
        assert_eq!(
            test_meta_direct_child_names(meta_info_child(method, "Properties").unwrap()),
            ["Name", "Synonym", "Comment", "HTTPMethod", "Handler"]
        );
    }

    #[test]
    fn information_register_dimension_emits_type_reduction_mode() {
        let xml = test_compile_meta_xml(
            "InformationRegister",
            "CorpusInformationRegister",
            json!({"dimensions": ["Item:String(50)|master,index"]}),
        );
        let document = Document::parse(&xml).unwrap();
        let dimension = test_meta_named_object(&document, "Dimension", "Item");
        let properties = meta_info_child(dimension, "Properties").unwrap();
        let children = properties
            .children()
            .filter(roxmltree::Node::is_element)
            .collect::<Vec<_>>();
        let last = children.last().unwrap();

        assert_eq!(last.tag_name().name(), "TypeReductionMode", "{xml}");
        assert_eq!(last.text(), Some("TransformValues"), "{xml}");
    }

    #[test]
    fn web_service_emits_platform_defaults_and_parameter_comment() {
        let xml = test_compile_meta_xml(
            "WebService",
            "CorpusWebService",
            json!({
                "namespace": "urn:corpus",
                "operations": {
                    "Ping": {"returnType": "xs:string", "parameters": {"Text": "xs:string"}}
                }
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let properties = test_meta_root_properties(&document);
        let operation = test_meta_named_object(&document, "Operation", "Ping");
        let parameter = test_meta_named_object(&document, "Parameter", "Text");

        assert_eq!(
            meta_info_child_text(properties, "DescriptorFileName").as_deref(),
            Some("ws1.1cws")
        );
        assert_eq!(
            meta_info_child_text(
                meta_info_child(operation, "Properties").unwrap(),
                "DataLockControlMode"
            )
            .as_deref(),
            Some("Managed")
        );
        assert_eq!(
            test_meta_direct_child_names(meta_info_child(parameter, "Properties").unwrap()),
            [
                "Name",
                "Synonym",
                "Comment",
                "XDTOValueType",
                "Nillable",
                "TransferDirection",
            ]
        );
    }

    #[test]
    fn value_type_unions_reject_duplicate_wire_types() {
        for (object_type, value_types) in [
            (
                "DefinedType",
                json!(["String(50)", "String(20)", "Number(15,2)"]),
            ),
            ("ChartOfCharacteristicTypes", json!(["Date", "DateTime"])),
        ] {
            let definition = json!({"valueTypes": value_types});
            let error = meta_compile_object_xml(
                definition.as_object().unwrap(),
                object_type,
                "CorpusType",
                "2.20",
            )
            .unwrap_err();

            assert!(error.contains("duplicate platform type"), "{error}");
        }
    }

    #[test]
    fn nested_value_type_unions_reject_duplicate_wire_types() {
        for definition in [
            json!({"attributes": ["Value: String(50) + String(20)"]}),
            json!({"resources": ["Value: Date + DateTime"]}),
            json!({
                "tabularSections": {
                    "Lines": ["Value: Number(15,2) + Number(10,0)"]
                }
            }),
        ] {
            let error = meta_compile_object_xml(
                definition.as_object().unwrap(),
                if definition.get("resources").is_some() {
                    "InformationRegister"
                } else {
                    "Catalog"
                },
                "CorpusObject",
                "2.20",
            )
            .unwrap_err();

            assert!(error.contains("duplicate platform type"), "{error}");
        }
    }

    #[test]
    fn value_type_parameters_must_follow_the_8_3_27_contract() {
        for value_type in [
            "String(foo)",
            "String(10,20)",
            "String(-1)",
            "String(1025)",
            "String(1.5)",
            "Number(x,2)",
            "Number(15,y)",
            "Number(15,2,wrong)",
            "Number(15,2,nonneg,extra)",
            "Number(-1,0)",
            "Number(39,0)",
            "Number(10,11)",
            "Number(10,-1)",
            "Number(10,1.5)",
            "Garbage",
            "xs:string",
            "v8:UUID",
            "v8:ValueStorage",
            "CatalogRef.Bad Name",
            "CatalogRef.Bad:Name",
            "CatalogRef.Bad.Name",
        ] {
            let definition = json!({"attributes": [format!("Value: {value_type}")]});
            let error = meta_compile_object_xml(
                definition.as_object().unwrap(),
                "Catalog",
                "CorpusCatalog",
                "2.20",
            )
            .unwrap_err();

            assert!(error.contains(value_type), "{value_type}: {error}");
            assert!(error.contains("8.3.27"), "{value_type}: {error}");
        }
    }

    #[test]
    fn value_type_parameter_boundaries_match_8_3_27() {
        for value_type in [
            "String(0)",
            "String(1024)",
            "Number(0,0)",
            "Number(38,0)",
            "Number(38,38)",
            "Number(38,38,nonneg)",
        ] {
            let definition = json!({"attributes": [format!("Value: {value_type}")]});
            meta_compile_object_xml(
                definition.as_object().unwrap(),
                "Catalog",
                "CorpusCatalog",
                "2.20",
            )
            .unwrap_or_else(|error| panic!("{value_type}: {error}"));
        }
    }

    #[test]
    fn configuration_type_names_accept_unicode_xml_ncnames() {
        validate_meta_type_union(["CatalogRef.Контрагенты_1"]).unwrap();
        let mut lines = Vec::new();
        emit_meta_type_content(&mut lines, "", "CatalogRef.Контрагенты_1");
        assert_eq!(lines, ["<v8:Type>cfg:CatalogRef.Контрагенты_1</v8:Type>"]);
    }

    #[test]
    fn value_type_tags_follow_the_8_3_27_canonical_order() {
        let mut lines = Vec::new();

        emit_meta_type_contents(
            &mut lines,
            "",
            [
                "Number(15,2)",
                "DateTime",
                "String(50)",
                "Boolean",
                "DefinedType.CorpusDefinedType",
                "CatalogRef.CorpusCatalog",
            ],
        );

        assert_eq!(
            &lines[..6],
            [
                "<v8:Type>cfg:CatalogRef.CorpusCatalog</v8:Type>",
                "<v8:Type>xs:boolean</v8:Type>",
                "<v8:Type>xs:string</v8:Type>",
                "<v8:Type>xs:dateTime</v8:Type>",
                "<v8:Type>xs:decimal</v8:Type>",
                "<v8:TypeSet>cfg:DefinedType.CorpusDefinedType</v8:TypeSet>",
            ]
        );
        assert!(lines[6].starts_with("<v8:NumberQualifiers>"), "{lines:?}");
        assert!(lines[11].starts_with("<v8:StringQualifiers>"), "{lines:?}");
        assert!(lines[15].starts_with("<v8:DateQualifiers>"), "{lines:?}");
    }

    #[test]
    fn event_subscription_sources_form_one_8_3_27_type_description() {
        let xml = test_compile_meta_xml(
            "EventSubscription",
            "CorpusSubscription",
            json!({
                "source": ["String(10)", "DocumentObject.CorpusDocument"],
                "event": "BeforeWrite",
                "handler": "CorpusModule.Handle"
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let source = meta_info_child(test_meta_root_properties(&document), "Source").unwrap();

        assert_eq!(
            test_meta_direct_child_names(source),
            ["Type", "Type", "StringQualifiers"],
            "{xml}"
        );
    }

    #[test]
    fn event_subscription_source_string_is_unbounded_in_8_3_27() {
        let xml = test_compile_meta_xml(
            "EventSubscription",
            "CorpusSubscription",
            json!({
                "source": [
                    "DocumentObject.CorpusDocument",
                    "String(37)",
                    "CatalogObject.CorpusCatalog"
                ],
                "event": "BeforeWrite",
                "handler": "CorpusModule.Handle"
            }),
        );
        let document = Document::parse(&xml).unwrap();
        let source = meta_info_child(test_meta_root_properties(&document), "Source").unwrap();
        let mut source_types = meta_info_children(source, "Type")
            .into_iter()
            .map(meta_info_inner_text)
            .collect::<Vec<_>>();
        source_types.sort();

        assert_eq!(
            source_types,
            [
                "cfg:CatalogObject.CorpusCatalog",
                "cfg:DocumentObject.CorpusDocument",
                "xs:string",
            ],
            "{xml}"
        );
        let qualifiers = meta_info_child(source, "StringQualifiers").unwrap();
        assert_eq!(
            meta_info_child_text(qualifiers, "Length").as_deref(),
            Some("0"),
            "{xml}"
        );
        assert_eq!(
            meta_info_child_text(qualifiers, "AllowedLength").as_deref(),
            Some("Variable"),
            "{xml}"
        );
    }

    #[test]
    fn event_subscription_sources_validate_as_one_union() {
        for (sources, expected) in [
            (vec!["String(10)", "String(20)"], "duplicate platform type"),
            (
                vec!["ValueStorage", "DocumentObject.CorpusDocument"],
                "only platform type",
            ),
        ] {
            let definition = json!({
                "source": sources,
                "event": "BeforeWrite",
                "handler": "CorpusModule.Handle"
            });
            let error = meta_compile_object_xml(
                definition.as_object().unwrap(),
                "EventSubscription",
                "CorpusSubscription",
                "2.20",
            )
            .unwrap_err();

            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn configuration_type_order_is_not_faked_without_workspace_type_ids() {
        let mut lines = Vec::new();

        emit_meta_type_contents(
            &mut lines,
            "",
            [
                "DocumentRef.CorpusDocument",
                "CatalogRef.CorpusCatalog",
                "Boolean",
            ],
        );

        assert_eq!(
            &lines[..3],
            [
                "<v8:Type>cfg:DocumentRef.CorpusDocument</v8:Type>",
                "<v8:Type>cfg:CatalogRef.CorpusCatalog</v8:Type>",
                "<v8:Type>xs:boolean</v8:Type>",
            ]
        );
    }

    #[test]
    fn value_storage_uses_the_8_3_27_core_type() {
        for alias in ["ValueStorage", "valuestorage", "ХранилищеЗначения"] {
            validate_meta_type_union([alias]).unwrap();
            let mut lines = Vec::new();
            emit_meta_type_content(&mut lines, "", alias);
            assert_eq!(lines, ["<v8:Type>v8:ValueStorage</v8:Type>"]);
        }

        let error = validate_meta_type_union(["Boolean", "ValueStorage"]).unwrap_err();
        assert!(error.contains("only platform type"), "{error}");
    }

    #[test]
    fn defined_type_references_use_type_set_after_concrete_types() {
        let mut lines = Vec::new();

        emit_meta_type_contents(
            &mut lines,
            "",
            ["DefinedType.CorpusDefinedType", "String(50)"],
        );

        assert_eq!(lines[0], "<v8:Type>xs:string</v8:Type>");
        assert_eq!(
            lines[1],
            "<v8:TypeSet>cfg:DefinedType.CorpusDefinedType</v8:TypeSet>"
        );
        assert!(lines[2].starts_with("<v8:StringQualifiers>"), "{lines:?}");
    }

    #[test]
    fn chart_of_accounts_rejects_positive_extra_dimension_count_without_type() {
        for ext_dimension_types in [None, Some(""), Some("   ")] {
            let mut definition = json!({"maxExtDimensionCount": 3});
            if let Some(value) = ext_dimension_types {
                definition["extDimensionTypes"] = json!(value);
            }
            let error = meta_compile_object_xml(
                definition.as_object().unwrap(),
                "ChartOfAccounts",
                "CorpusAccounts",
                "2.20",
            )
            .unwrap_err();

            assert!(error.contains("extDimensionTypes"), "{error}");
            assert!(error.contains("maxExtDimensionCount"), "{error}");
        }
    }

    #[test]
    fn meta_compile_rejects_invalid_object_and_child_names_before_emission() {
        let cases = [
            ("metadata object", "Catalog", "../EscapedName", json!({})),
            (
                "attributes",
                "Catalog",
                "ValidCatalog",
                json!({"attributes": [{"name": "Bad Name", "type": "String"}]}),
            ),
            (
                "tabularSections",
                "Document",
                "ValidDocument",
                json!({"tabularSections": [{"name": "Bad/Section", "attributes": []}]}),
            ),
            (
                "enum value",
                "Enum",
                "ValidEnum",
                json!({"values": ["Bad Value"]}),
            ),
            (
                "URL template",
                "HTTPService",
                "ValidService",
                json!({"urlTemplates": {"Bad/Template": "/probe"}}),
            ),
            (
                "operation parameter",
                "WebService",
                "ValidWebService",
                json!({
                    "operations": {
                        "ValidOperation": {"parameters": {"Bad Parameter": "xs:string"}}
                    }
                }),
            ),
        ];

        for (context, object_type, object_name, definition) in cases {
            let error = meta_compile_object_xml(
                definition.as_object().unwrap(),
                object_type,
                object_name,
                "2.20",
            )
            .unwrap_err();

            assert!(error.contains(context), "{context}: {error}");
            assert!(error.contains("valid 1C identifier"), "{context}: {error}");
        }
    }

    #[test]
    fn meta_compile_rejects_invalid_8_3_27_enum_before_emission() {
        let definition = json!({"hierarchyType": "Bogus"});

        let error = meta_compile_object_xml(
            definition.as_object().unwrap(),
            "Catalog",
            "ValidCatalog",
            "2.20",
        )
        .unwrap_err();

        assert!(error.contains("HierarchyType"), "{error}");
        assert!(error.contains("Bogus"), "{error}");
        assert!(error.contains("8.3.27"), "{error}");
    }

    fn test_compile_meta_xml(object_type: &str, object_name: &str, definition: Value) -> String {
        meta_compile_object_xml(
            definition.as_object().unwrap(),
            object_type,
            object_name,
            "2.20",
        )
        .unwrap()
        .0
    }

    fn test_meta_root_properties<'a, 'input>(
        document: &'a Document<'input>,
    ) -> roxmltree::Node<'a, 'input> {
        let object = document
            .root_element()
            .children()
            .find(roxmltree::Node::is_element)
            .unwrap();
        meta_info_child(object, "Properties").unwrap()
    }

    fn test_meta_direct_child_names(node: roxmltree::Node<'_, '_>) -> Vec<String> {
        node.children()
            .filter(roxmltree::Node::is_element)
            .map(|child| child.tag_name().name().to_string())
            .collect()
    }

    fn test_meta_standard_attribute_names(properties: roxmltree::Node<'_, '_>) -> Vec<String> {
        meta_info_child(properties, "StandardAttributes")
            .unwrap()
            .children()
            .filter(roxmltree::Node::is_element)
            .map(|child| child.attribute("name").unwrap_or_default().to_string())
            .collect()
    }

    fn test_meta_root_child_objects<'a, 'input>(
        document: &'a Document<'input>,
    ) -> roxmltree::Node<'a, 'input> {
        let object = document
            .root_element()
            .children()
            .find(roxmltree::Node::is_element)
            .unwrap();
        meta_info_child(object, "ChildObjects").unwrap()
    }

    fn test_meta_named_object<'a, 'input>(
        document: &'a Document<'input>,
        object_type: &str,
        name: &str,
    ) -> roxmltree::Node<'a, 'input> {
        document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == object_type)
            .find(|node| {
                meta_info_child(*node, "Properties")
                    .and_then(|properties| meta_info_child_text(properties, "Name"))
                    .as_deref()
                    == Some(name)
            })
            .unwrap_or_else(|| panic!("{object_type} {name} not found"))
    }
}

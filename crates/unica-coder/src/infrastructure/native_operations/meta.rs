#![allow(dead_code, unused_imports)]

use crate::application::operation_descriptors::OBJECT_PATH;
use crate::application::AdapterOutcome;
use crate::domain::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::metadata_kinds::metadata_kind;
use crate::infrastructure::platform_xml_owner::{
    resolve_platform_xml_owners_with_provenance, root_version_literal, PlatformXmlOwnerKind,
    PlatformXmlOwnerProvenance,
};
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use super::common::*;
use super::compile_transaction::{
    CompileTransaction, DirectoryTopologyEntry, DirectoryTopologyEntryKind, RegistrationStatus,
};
use super::meta_validation_context::{
    inspect_meta_validation_reads, meta_validate_registrar_document_scan,
    meta_validate_types_with_list_presentation, MetaValidationOwnerKind,
};
use super::{
    cf::*, cfe::*, dcs::*, form::*, interface::*, mxl::*, role::*, subsystem::*, template::*,
};

#[cfg(test)]
type MetaCompileAfterOwnerValidationHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
type MetaCompileAfterFormatPlanHook = Box<dyn FnOnce()>;

#[cfg(test)]
type MetaEditAfterLineNumberLengthPolicyHook = Box<dyn FnOnce()>;

#[cfg(test)]
type MetaRemoveSubsystemChildInspectionHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static META_COMPILE_AFTER_OWNER_VALIDATION_HOOK:
        std::cell::RefCell<Option<MetaCompileAfterOwnerValidationHook>> =
        const { std::cell::RefCell::new(None) };
    static META_COMPILE_AFTER_FORMAT_PLAN_HOOK:
        std::cell::RefCell<Option<MetaCompileAfterFormatPlanHook>> =
        const { std::cell::RefCell::new(None) };
    static META_EDIT_AFTER_LINE_NUMBER_LENGTH_POLICY_HOOK:
        std::cell::RefCell<Option<MetaEditAfterLineNumberLengthPolicyHook>> =
        const { std::cell::RefCell::new(None) };
    static META_REMOVE_FORCED_REPARSE_PATHS:
        std::cell::RefCell<HashSet<PathBuf>> =
        std::cell::RefCell::new(HashSet::new());
    static META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK:
        std::cell::RefCell<Option<MetaRemoveSubsystemChildInspectionHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn with_meta_compile_after_owner_validation_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaCompileAfterOwnerValidationHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_COMPILE_AFTER_OWNER_VALIDATION_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        META_COMPILE_AFTER_OWNER_VALIDATION_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_meta_compile_after_owner_validation_hook(path: &Path) {
    if let Some(hook) =
        META_COMPILE_AFTER_OWNER_VALIDATION_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(path);
    }
}

#[cfg(test)]
fn with_meta_compile_after_format_plan_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaCompileAfterFormatPlanHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_COMPILE_AFTER_FORMAT_PLAN_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        META_COMPILE_AFTER_FORMAT_PLAN_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_meta_compile_after_format_plan_hook() {
    if let Some(hook) = META_COMPILE_AFTER_FORMAT_PLAN_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
fn with_meta_edit_after_line_number_length_policy_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaEditAfterLineNumberLengthPolicyHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_EDIT_AFTER_LINE_NUMBER_LENGTH_POLICY_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = META_EDIT_AFTER_LINE_NUMBER_LENGTH_POLICY_HOOK
        .with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_meta_edit_after_line_number_length_policy_hook() {
    if let Some(hook) =
        META_EDIT_AFTER_LINE_NUMBER_LENGTH_POLICY_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[cfg(test)]
fn with_meta_remove_forced_reparse_paths<T>(
    paths: impl IntoIterator<Item = PathBuf>,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(HashSet<PathBuf>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| {
                slot.replace(std::mem::take(&mut self.0));
            });
        }
    }

    let paths = paths.into_iter().collect();
    let previous = META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| slot.replace(paths));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn force_meta_remove_reparse_path(path: impl Into<PathBuf>) {
    META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| {
        slot.borrow_mut().insert(path.into());
    });
}

#[cfg(test)]
fn with_before_meta_remove_subsystem_child_inspection_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaRemoveSubsystemChildInspectionHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn run_before_meta_remove_subsystem_child_inspection_hook(path: &Path) {
    if let Some(hook) =
        META_REMOVE_SUBSYSTEM_CHILD_INSPECTION_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(path);
    }
}

pub(crate) fn fresh_meta_compile_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

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
    use super::super::compile_transaction::{with_commit_failpoint, CommitFailpoint};
    use super::super::single_file_publisher::with_before_commit_hook;
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

        let outcome = edit_meta(
            &meta_edit_args(&object_path, "modify-property", "Comment=TEST-COMMENT"),
            &context,
        );

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert!(outcome.changes.is_empty(), "{:?}", outcome.changes);
        assert_eq!(fs::read(&object_path).unwrap(), before);
        assert!(outcome
            .stdout
            .as_deref()
            .unwrap_or_default()
            .contains("No changes"));

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);
        assert!(stdout.contains("Added:    1"), "{stdout}");

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);
        assert!(stdout.contains("Added:    1"), "{stdout}");

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);
        assert!(stdout.contains("Removed:  1"), "{stdout}");

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);
        assert!(stdout.contains("Modified: 2"), "{stdout}");

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);
        assert!(stdout.contains("Modified: 3"), "{stdout}");

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);
        assert!(stdout.contains("Modified: 2"), "{stdout}");

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);
        assert!(stdout.contains("Added:    2"), "{stdout}");

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
        let stdout = outcome.stdout.as_deref().unwrap_or("");
        assert!(outcome.ok, "{stdout}\n{:?}", outcome.errors);

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

#[derive(Clone)]
pub(crate) struct MetaInfoAttr<'a, 'input> {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) flags: String,
    pub(crate) _marker: std::marker::PhantomData<roxmltree::Node<'a, 'input>>,
}

pub(crate) struct MetaInfoTabularSection<'a, 'input> {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaInfoAttr<'a, 'input>>,
}

pub(crate) struct MetaInfoHttpMethod {
    pub(crate) http_method: String,
    pub(crate) handler: String,
}

pub(crate) struct MetaInfoHttpEndpoint {
    pub(crate) name: String,
    pub(crate) template: String,
    pub(crate) methods: Vec<MetaInfoHttpMethod>,
}

pub(crate) struct MetaInfoWsOperation {
    pub(crate) name: String,
    pub(crate) params: String,
    pub(crate) return_type: String,
    pub(crate) proc_name: String,
}

pub(crate) struct MetaValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
    pub(crate) md_type: String,
    pub(crate) obj_name: String,
}

pub(crate) struct MetaValidationRun {
    pub(crate) ok: bool,
    pub(crate) stdout: String,
    pub(crate) out_files: Vec<PathBuf>,
    pub(crate) artifacts: Vec<PathBuf>,
    pub(crate) errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct MetaValidationOptions {
    pub(crate) detailed: bool,
    pub(crate) max_errors: usize,
    pub(crate) out_file_label: Option<String>,
    pub(crate) out_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaValidationScope {
    PublicOwnerAware,
    PostWriteLocal,
}

struct MetaValidationReferenceInputs {
    config_dir: Option<PathBuf>,
    language_codes: Vec<String>,
}

impl MetaValidationReporter {
    pub(crate) fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: vec![String::new()],
            md_type: "(unknown)".to_string(),
            obj_name: "(unknown)".to_string(),
        }
    }

    pub(crate) fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub(crate) fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub(crate) fn finalize(mut self) -> (bool, String, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!(
                    "=== Validation OK: {}.{} ({checks} checks) ===",
                    self.md_type, self.obj_name
                ),
                Vec::new(),
            );
        }
        self.lines.insert(
            0,
            format!("=== Validation: {}.{} ===", self.md_type, self.obj_name),
        );
        self.lines.push(String::new());
        self.lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            self.errors, self.warnings
        ));
        let errors = self
            .lines
            .iter()
            .filter(|line| line.starts_with("[ERROR]"))
            .cloned()
            .collect::<Vec<_>>();
        (ok, self.lines.join("\n"), errors)
    }
}

pub(crate) fn validate_meta(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<MetaValidationRun, String> {
        let raw_path = required_path(
            args,
            &["objectPath", "ObjectPath", "path", "Path"],
            "ObjectPath",
        )?;
        let raw_path_text = raw_path.to_string_lossy();
        let paths = raw_path_text
            .split('|')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return Err("[ERROR] No ObjectPath values were provided".to_string());
        }

        let options = meta_validation_options(args, context);
        if paths.len() > 1 {
            meta_validate_batch(paths, &options, context)
        } else {
            meta_validate_one(paths[0].clone(), &options, context)
        }
    })();

    match result {
        Ok(run) => {
            let mut artifacts = run
                .artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
            artifacts.extend(run.out_files.iter().map(|path| path.display().to_string()));
            AdapterOutcome {
                ok: run.ok,
                summary: if run.ok {
                    "unica.meta.validate completed with native metadata validator".to_string()
                } else {
                    "unica.meta.validate failed in native metadata validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: run.errors,
                artifacts,
                stdout: Some(run.stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.validate failed in native metadata validator".to_string(),
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

pub(crate) fn meta_validation_options(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> MetaValidationOptions {
    let out_file_label = string_arg(args, &["outFile", "OutFile"]).map(ToOwned::to_owned);
    MetaValidationOptions {
        detailed: bool_arg(args, &["detailed", "Detailed"]),
        max_errors: int_arg(args, &["maxErrors", "MaxErrors"])
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(30),
        out_file: out_file_label
            .as_ref()
            .map(|path| absolutize(PathBuf::from(path), &context.cwd)),
        out_file_label,
    }
}

/// Return the platform XML documents whose contents `meta.validate` reads,
/// including each member of a batch and the registrar documents inspected for
/// register cross-reference diagnostics.
pub(crate) fn meta_validate_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let raw_path = required_path(
        args,
        &["objectPath", "ObjectPath", "path", "Path"],
        "ObjectPath",
    )?;
    let raw_path_text = raw_path.to_string_lossy();
    let mut dependencies = Vec::new();
    for raw in raw_path_text
        .split('|')
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let candidate = absolutize(PathBuf::from(raw), &context.cwd);
        let object_path = resolve_meta_info_path(candidate.clone()).unwrap_or(candidate);
        let inspection = inspect_meta_validation_reads(&object_path, context);
        for path in inspection.paths {
            if !dependencies.contains(&path) {
                dependencies.push(path);
            }
        }
    }
    Ok(dependencies)
}

pub(crate) fn meta_validate_batch(
    paths: Vec<PathBuf>,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
) -> Result<MetaValidationRun, String> {
    let total = paths.len();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut stdout_blocks = Vec::<String>::new();
    let mut errors = Vec::<String>::new();
    let mut artifacts = Vec::<PathBuf>::new();
    let mut out_files = Vec::<PathBuf>::new();

    for path in paths {
        let item_options = meta_validate_batch_options(options, &path, context);
        match meta_validate_one(path.clone(), &item_options, context) {
            Ok(run) => {
                if run.ok {
                    passed += 1;
                } else {
                    failed += 1;
                }
                errors.extend(run.errors);
                artifacts.extend(run.artifacts);
                out_files.extend(run.out_files);
                stdout_blocks.push(format!("--- {} ---", path.display()));
                stdout_blocks.push(run.stdout.trim_end().to_string());
            }
            Err(error) => {
                failed += 1;
                let message = format!("[ERROR] {}: {error}", path.display());
                errors.push(message.clone());
                stdout_blocks.push(message);
            }
        }
    }

    stdout_blocks.push(String::new());
    stdout_blocks.push("=== meta-validate batch summary ===".to_string());
    stdout_blocks.push(format!("Validated: {total}"));
    stdout_blocks.push(format!("Passed:    {passed}"));
    stdout_blocks.push(format!("Failed:    {failed}"));

    Ok(MetaValidationRun {
        ok: failed == 0,
        stdout: format!("{}\n", stdout_blocks.join("\n")),
        out_files,
        artifacts,
        errors,
    })
}

pub(crate) fn meta_validate_batch_options(
    options: &MetaValidationOptions,
    path: &Path,
    context: &WorkspaceContext,
) -> MetaValidationOptions {
    let Some(label) = &options.out_file_label else {
        return options.clone();
    };
    let label_path = PathBuf::from(label);
    let stem = label_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("meta-validate");
    let extension = label_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let object_leaf = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("object");
    let file_name = format!("{stem}_{object_leaf}{extension}");
    let item_label = label_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.join(&file_name))
        .unwrap_or_else(|| PathBuf::from(&file_name));
    MetaValidationOptions {
        out_file: Some(absolutize(item_label.clone(), &context.cwd)),
        out_file_label: Some(item_label.display().to_string()),
        ..options.clone()
    }
}

pub(crate) fn meta_validate_one(
    raw_path: PathBuf,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
) -> Result<MetaValidationRun, String> {
    meta_validate_one_with_scope(
        raw_path,
        options,
        context,
        MetaValidationScope::PublicOwnerAware,
    )
}

fn meta_validate_one_with_scope(
    raw_path: PathBuf,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
    scope: MetaValidationScope,
) -> Result<MetaValidationRun, String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let object_path = resolve_meta_info_path(absolutize(raw_path, &context.cwd))?;
    let resolved_path = object_path
        .canonicalize()
        .unwrap_or_else(|_| object_path.clone());
    let owner_inspection = match scope {
        MetaValidationScope::PublicOwnerAware => {
            Some(inspect_meta_validation_reads(&resolved_path, context))
        }
        MetaValidationScope::PostWriteLocal => None,
    };

    let text = read_utf8_sig(&resolved_path)?;
    let source = text.trim_start_matches('\u{feff}');
    let doc = match Document::parse(source) {
        Ok(doc) => doc,
        Err(err) => {
            let mut report = MetaValidationReporter::new(options.max_errors, options.detailed);
            report.md_type = "(parse failed)".to_string();
            report.obj_name.clear();
            report.error(format!("1. XML parse failed: {err}"));
            return meta_validate_finish(
                report,
                options.out_file.clone(),
                options.out_file_label.clone(),
                resolved_path,
            );
        }
    };

    let root = doc.root_element();
    let mut report = MetaValidationReporter::new(options.max_errors, options.detailed);
    let mut check1_ok = true;

    if root.tag_name().name() != "MetaDataObject" {
        report.error(format!(
            "1. Root element is '{}', expected 'MetaDataObject'",
            root.tag_name().name()
        ));
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }

    let root_ns = root.tag_name().namespace().unwrap_or("");
    if root_ns != MD_NS {
        report.error(format!(
            "1. Root namespace is '{root_ns}', expected '{MD_NS}'"
        ));
        check1_ok = false;
    }

    let version_literal = root_version_literal(source, root);
    match classify_root_version(version_literal.as_deref()) {
        Ok(FormatCompatibility::Supported { .. }) => report.ok("Export format: 2.20"),
        Ok(compatibility) => report.warn(format_compatibility_warning(&compatibility)),
        Err(error) => report.error(error.to_string()),
    }
    let version = version_literal.as_deref().unwrap_or("");

    let child_elements = root
        .children()
        .filter(|child| child.is_element() && child.tag_name().namespace() == Some(MD_NS))
        .collect::<Vec<_>>();
    if child_elements.is_empty() {
        report.error("1. No metadata type element found inside MetaDataObject");
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    if child_elements.len() > 1 {
        let names = child_elements
            .iter()
            .map(|child| format!("'{}'", child.tag_name().name()))
            .collect::<Vec<_>>();
        report.error(format!(
            "1. Multiple type elements found: [{}]",
            names.join(", ")
        ));
        check1_ok = false;
    }

    let type_node = child_elements[0];
    let md_type = type_node.tag_name().name();
    report.md_type = md_type.to_string();
    if !meta_validate_valid_types().contains(&md_type) {
        report.error(format!("1. Unrecognized metadata type: {md_type}"));
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }

    let type_uuid = type_node.attribute("uuid").unwrap_or("");
    if type_uuid.is_empty() {
        report.error(format!("1. Missing uuid on <{md_type}> element"));
        check1_ok = false;
    } else if !is_guid(type_uuid) {
        report.error(format!("1. Invalid uuid '{type_uuid}' on <{md_type}>"));
        check1_ok = false;
    }

    let props_node = meta_info_child(type_node, "Properties");
    let name_node = props_node.and_then(|props| meta_info_child(props, "Name"));
    let obj_name = name_node
        .map(meta_info_inner_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(unknown)".to_string());
    report.obj_name = obj_name.clone();

    let reference_inputs = match scope {
        MetaValidationScope::PublicOwnerAware => {
            let owner_context = match owner_inspection
                .expect("public validation always creates owner inspection")
                .context
            {
                Ok(owner_context) => owner_context,
                Err(error) => {
                    report.error(format!("1. Owner context: {error}"));
                    return meta_validate_finish(
                        report,
                        options.out_file.clone(),
                        options.out_file_label.clone(),
                        resolved_path,
                    );
                }
            };
            let config_dir = match owner_context.owner_kind {
                MetaValidationOwnerKind::Configuration | MetaValidationOwnerKind::Extension => {
                    owner_context.owner_path.parent().map(Path::to_path_buf)
                }
                MetaValidationOwnerKind::External => None,
            };
            MetaValidationReferenceInputs {
                config_dir,
                language_codes: owner_context.language_codes,
            }
        }
        MetaValidationScope::PostWriteLocal => MetaValidationReferenceInputs {
            config_dir: None,
            language_codes: Vec::new(),
        },
    };

    if check1_ok {
        report.ok(format!(
            "1. Root structure: MetaDataObject/{md_type}, version {version}"
        ));
    }
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }

    meta_validate_check_internal_info(&mut report, md_type, type_node, &obj_name);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_properties(
        &mut report,
        md_type,
        props_node,
        name_node,
        &obj_name,
        &reference_inputs.language_codes,
    );
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_property_values(&mut report, props_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_standard_attributes(&mut report, md_type, props_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }

    let child_obj_node = meta_info_child(type_node, "ChildObjects");
    meta_validate_check_child_objects(&mut report, md_type, child_obj_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_child_elements(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_reserved_attr_names(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_uniqueness(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_tabular_sections(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_cross_properties(
        &mut report,
        md_type,
        props_node,
        child_obj_node,
        reference_inputs.config_dir.as_deref(),
        &obj_name,
    );
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_services(&mut report, md_type, child_obj_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_forbidden_properties(&mut report, md_type, props_node);
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_method_reference(
        &mut report,
        md_type,
        props_node,
        reference_inputs.config_dir.as_deref(),
    );
    if report.stopped {
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
    meta_validate_check_document_journal_columns(&mut report, md_type, child_obj_node);

    meta_validate_finish(
        report,
        options.out_file.clone(),
        options.out_file_label.clone(),
        resolved_path,
    )
}

pub(crate) fn meta_validate_finish(
    report: MetaValidationReporter,
    out_file: Option<PathBuf>,
    out_file_label: Option<String>,
    artifact: PathBuf,
) -> Result<MetaValidationRun, String> {
    let (ok, result_text, errors) = report.finalize();
    let stdout = if let Some(out_file) = &out_file {
        write_utf8_bom(out_file, &result_text)?;
        let label = out_file_label
            .as_deref()
            .unwrap_or_else(|| out_file.to_str().unwrap_or(""));
        format!("{result_text}\nWritten to: {label}\n")
    } else {
        format!("{result_text}\n")
    };
    Ok(MetaValidationRun {
        ok,
        stdout,
        out_files: out_file.into_iter().collect(),
        artifacts: vec![artifact],
        errors,
    })
}

pub(crate) fn meta_validate_localized_values(
    node: Option<roxmltree::Node<'_, '_>>,
) -> Vec<(Option<String>, String)> {
    const V8_CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";

    let Some(node) = node else {
        return Vec::new();
    };
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().name() == "item"
                && child.tag_name().namespace() == Some(V8_CORE_NS)
        })
        .filter_map(|item| {
            let child_text = |name| {
                item.children()
                    .find(|child| {
                        child.is_element()
                            && child.tag_name().name() == name
                            && child.tag_name().namespace() == Some(V8_CORE_NS)
                    })
                    .map(meta_info_inner_text)
            };
            let language = child_text("lang")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let text = child_text("content").unwrap_or_default();
            (!text.trim().is_empty()).then_some((language, text))
        })
        .collect()
}

pub(crate) fn meta_validate_check_internal_info(
    report: &mut MetaValidationReporter,
    md_type: &str,
    type_node: roxmltree::Node<'_, '_>,
    obj_name: &str,
) {
    let internal_info = meta_info_child(type_node, "InternalInfo");
    if meta_validate_types_without_internal_info().contains(&md_type) {
        if let Some(internal_info) = internal_info {
            let gen_types = meta_info_children(internal_info, "GeneratedType");
            if gen_types.is_empty() {
                report.ok(format!(
                    "2. InternalInfo: absent or empty (correct for {md_type})"
                ));
            } else {
                report.warn(format!(
                    "2. InternalInfo: {md_type} should not have GeneratedType entries, found {}",
                    gen_types.len()
                ));
            }
        } else {
            report.ok(format!("2. InternalInfo: absent (correct for {md_type})"));
        }
        return;
    }

    let Some(expected_categories) = meta_validate_generated_categories(md_type) else {
        return;
    };
    let Some(internal_info) = internal_info else {
        report.error(format!(
            "2. InternalInfo: missing (expected {} GeneratedType)",
            expected_categories.len()
        ));
        return;
    };
    let gen_types = meta_info_children(internal_info, "GeneratedType");
    let mut check_ok = true;
    let mut found_categories = Vec::<String>::new();
    for generated_type in &gen_types {
        let name = generated_type.attribute("name").unwrap_or("");
        let category = generated_type.attribute("category").unwrap_or("");
        found_categories.push(category.to_string());
        if !name.is_empty() && obj_name != "(unknown)" && !name.ends_with(&format!(".{obj_name}")) {
            report.error(format!(
                "2. GeneratedType name '{name}' does not end with '.{obj_name}'"
            ));
            check_ok = false;
        }
        if !expected_categories.contains(&category) {
            report.warn(format!(
                "2. Unexpected GeneratedType category '{category}' for {md_type}"
            ));
        }
        if let Some(type_id) = meta_info_child(*generated_type, "TypeId") {
            if !is_guid(&meta_info_inner_text(type_id)) {
                report.error(format!(
                    "2. Invalid TypeId UUID in GeneratedType '{category}'"
                ));
                check_ok = false;
            }
        }
        if let Some(value_id) = meta_info_child(*generated_type, "ValueId") {
            if !is_guid(&meta_info_inner_text(value_id)) {
                report.error(format!(
                    "2. Invalid ValueId UUID in GeneratedType '{category}'"
                ));
                check_ok = false;
            }
        }
    }

    if md_type == "ExchangePlan" {
        if let Some(this_node) = meta_info_child(internal_info, "ThisNode") {
            if !is_guid(&meta_info_inner_text(this_node)) {
                report.error("2. ExchangePlan xr:ThisNode has invalid UUID");
                check_ok = false;
            }
        } else {
            report.warn("2. ExchangePlan missing xr:ThisNode in InternalInfo");
        }
    }

    let missing_categories = expected_categories
        .iter()
        .filter(|category| !found_categories.iter().any(|found| found == **category))
        .copied()
        .collect::<Vec<_>>();
    if !missing_categories.is_empty() {
        report.warn(format!(
            "2. Missing GeneratedType categories: {}",
            missing_categories.join(", ")
        ));
    }
    if check_ok {
        found_categories.sort();
        report.ok(format!(
            "2. InternalInfo: {} GeneratedType ({})",
            gen_types.len(),
            found_categories.join(", ")
        ));
    }
}

pub(crate) fn meta_validate_check_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    name_node: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
    configured_language_codes: &[String],
) {
    let Some(props_node) = props_node else {
        report.error("3. Properties block missing");
        return;
    };
    let mut check_ok = true;
    if name_node.is_none() || obj_name.is_empty() {
        report.error("3. Properties: Name is missing or empty");
        check_ok = false;
    } else {
        if !is_1c_identifier(obj_name) {
            report.error(format!(
                "3. Properties: Name '{obj_name}' is not a valid 1C identifier"
            ));
            check_ok = false;
        }
        if obj_name.chars().count() > 80 {
            report.warn(format!(
                "3. Properties: Name '{obj_name}' is longer than 80 characters ({})",
                obj_name.chars().count()
            ));
        }
    }
    let synonym_values = meta_validate_localized_values(meta_info_child(props_node, "Synonym"));
    let syn_present = !synonym_values.is_empty();

    if meta_validate_types_with_list_presentation().contains(&md_type) {
        meta_validate_check_command_texts(report, props_node, configured_language_codes);
    }
    if check_ok {
        let syn_info = if syn_present {
            "Synonym present"
        } else {
            "no Synonym"
        };
        report.ok(format!("3. Properties: Name=\"{obj_name}\", {syn_info}"));
    }
}

fn meta_validate_check_command_texts(
    report: &mut MetaValidationReporter,
    props_node: roxmltree::Node<'_, '_>,
    language_codes: &[String],
) {
    let synonyms = meta_validate_localized_values(meta_info_child(props_node, "Synonym"));
    let lists = meta_validate_localized_values(meta_info_child(props_node, "ListPresentation"));

    for language_code in language_codes {
        let list_values = lists
            .iter()
            .filter(|(language, text)| {
                language.as_deref() == Some(language_code.as_str()) && !text.trim().is_empty()
            })
            .collect::<Vec<_>>();
        let selected = if list_values.is_empty() {
            synonyms
                .iter()
                .filter(|(language, text)| {
                    language.as_deref() == Some(language_code.as_str()) && !text.trim().is_empty()
                })
                .map(|(_, text)| ("Synonym", text))
                .collect::<Vec<_>>()
        } else {
            list_values
                .into_iter()
                .map(|(_, text)| ("ListPresentation", text))
                .collect::<Vec<_>>()
        };
        for (source, text) in selected {
            meta_validate_warn_long_command_text(report, source, text, Some(language_code));
        }
    }
}

fn meta_validate_warn_long_command_text(
    report: &mut MetaValidationReporter,
    source: &str,
    text: &str,
    language: Option<&String>,
) {
    let length = text.chars().count();
    if length <= 38 {
        return;
    }
    let language_suffix = language
        .map(|language| format!(", language '{language}'"))
        .unwrap_or_default();
    report.warn(format!(
        "3. Properties: {source} '{text}' is longer than 38 characters ({length}) for the command interface{language_suffix}"
    ));
}

pub(crate) fn meta_validate_check_property_values(
    report: &mut MetaValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        report.warn("4. No Properties block to check");
        return;
    };
    let mut enum_checked = 0usize;
    let mut check_ok = true;
    for (prop_name, allowed) in meta_validate_property_values() {
        if let Some(value) =
            meta_info_child_text(props_node, prop_name).filter(|value| !value.is_empty())
        {
            if !allowed.contains(&value.as_str()) {
                report.error(format!(
                    "4. Property '{prop_name}' has invalid value '{value}' (allowed: {})",
                    allowed.join(", ")
                ));
                check_ok = false;
            }
            enum_checked += 1;
        }
    }
    if check_ok {
        report.ok(format!(
            "4. Property values: {enum_checked} enum properties checked"
        ));
    }
}

pub(crate) fn meta_validate_check_standard_attributes(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    if !meta_validate_types_with_std_attrs().contains(&md_type) {
        return;
    }
    let Some(props_node) = props_node else {
        return;
    };
    let Some(std_attr_node) = meta_info_child(props_node, "StandardAttributes") else {
        report.ok(format!(
            "5. StandardAttributes: absent (optional for {md_type})"
        ));
        return;
    };
    let std_attrs = meta_info_children(std_attr_node, "StandardAttribute");
    let expected_std_attrs = meta_validate_standard_attributes(md_type).unwrap_or_default();
    let mut check_ok = true;
    let mut found_names = Vec::<String>::new();
    for standard_attr in &std_attrs {
        let name = standard_attr.attribute("name").unwrap_or("");
        if name.is_empty() {
            report.error("5. StandardAttribute without 'name' attribute");
            check_ok = false;
            continue;
        }
        found_names.push(name.to_string());
        if !expected_std_attrs.contains(&name)
            && !meta_validate_dynamic_standard_attr(md_type, name)
        {
            report.warn(format!(
                "5. Unexpected StandardAttribute '{name}' for {md_type}"
            ));
        }
    }
    let missing_attrs = expected_std_attrs
        .iter()
        .filter(|attr| !found_names.iter().any(|found| found == **attr))
        .copied()
        .collect::<Vec<_>>();
    if !missing_attrs.is_empty() {
        report.warn(format!(
            "5. Missing StandardAttributes: {}",
            missing_attrs.join(", ")
        ));
    }
    if check_ok {
        report.ok(format!(
            "5. StandardAttributes: {} entries",
            std_attrs.len()
        ));
    }
}

pub(crate) fn meta_validate_check_child_objects(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let allowed_children = meta_validate_child_rules(md_type).unwrap_or_default();
    if let Some(child_obj_node) = child_obj_node {
        let mut check_ok = true;
        let mut child_counts = BTreeMap::<String, usize>::new();
        for child in child_obj_node.children().filter(|child| child.is_element()) {
            let child_tag = child.tag_name().name();
            if !allowed_children.contains(&child_tag) {
                report.error(format!(
                    "6. ChildObjects: disallowed element '{child_tag}' for {md_type}"
                ));
                check_ok = false;
            }
            *child_counts.entry(child_tag.to_string()).or_default() += 1;
        }
        if check_ok {
            if child_counts.is_empty() {
                report.ok(format!("6. ChildObjects: empty (valid for {md_type})"));
            } else {
                let summary = child_counts
                    .iter()
                    .map(|(name, count)| format!("{name}({count})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                report.ok(format!("6. ChildObjects types: {summary}"));
            }
        }
    } else if allowed_children.is_empty() {
        report.ok(format!("6. ChildObjects: absent (correct for {md_type})"));
    } else {
        report.ok("6. ChildObjects: absent");
    }
}

pub(crate) fn meta_validate_check_child_elements(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    let mut count = 0usize;
    for kind in ["Attribute", "Dimension", "Resource", "EnumValue", "Column"] {
        let require_type = !matches!(kind, "EnumValue" | "Column");
        for element in meta_info_children(child_obj_node, kind) {
            if !meta_validate_check_child_element(report, element, kind, require_type) {
                check_ok = false;
            }
            count += 1;
            if report.stopped {
                break;
            }
        }
    }
    if check_ok && count > 0 {
        report.ok(format!(
            "7. Child elements: {count} items checked (UUID, Name, Type)"
        ));
    } else if count == 0 {
        report.ok("7. Child elements: none to check");
    }
}

pub(crate) fn meta_validate_check_child_element(
    report: &mut MetaValidationReporter,
    node: roxmltree::Node<'_, '_>,
    kind: &str,
    require_type: bool,
) -> bool {
    let uuid = node.attribute("uuid").unwrap_or("");
    if uuid.is_empty() {
        report.error(format!("7. {kind} missing uuid"));
        return false;
    }
    if !is_guid(uuid) {
        report.error(format!("7. {kind} has invalid uuid '{uuid}'"));
        return false;
    }
    let Some(props) = meta_info_child(node, "Properties") else {
        report.error(format!("7. {kind} (uuid={uuid}) missing Properties"));
        return false;
    };
    let name = meta_info_child_text(props, "Name").unwrap_or_default();
    if name.is_empty() {
        report.error(format!("7. {kind} (uuid={uuid}) missing or empty Name"));
        return false;
    }
    if !is_1c_identifier(&name) {
        report.error(format!("7. {kind} '{name}' has invalid identifier"));
        return false;
    }
    if require_type {
        let Some(type_el) = meta_info_child(props, "Type") else {
            report.error(format!("7. {kind} '{name}' missing Type block"));
            return false;
        };
        if meta_info_children(type_el, "Type").is_empty()
            && meta_info_children(type_el, "TypeSet").is_empty()
        {
            report.error(format!(
                "7. {kind} '{name}' Type block has no v8:Type or v8:TypeSet"
            ));
            return false;
        }
    }
    true
}

pub(crate) fn meta_validate_check_reserved_attr_names(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    for attr_node in meta_info_children(child_obj_node, "Attribute") {
        if let Some(name) = meta_info_child(attr_node, "Properties")
            .and_then(|props| meta_info_child_text(props, "Name"))
            .filter(|value| meta_validate_reserved_attr_names().contains(&value.as_str()))
        {
            report.warn(format!(
                "7b. Attribute '{name}' conflicts with a standard attribute name"
            ));
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("7b. Reserved attribute names: no conflicts");
    }
}

pub(crate) fn meta_validate_check_uniqueness(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    for kind in [
        "Attribute",
        "TabularSection",
        "Dimension",
        "Resource",
        "EnumValue",
        "Column",
        "URLTemplate",
        "Operation",
    ] {
        if !meta_validate_names_unique(report, meta_info_children(child_obj_node, kind), kind) {
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("8. Name uniqueness: all names unique");
    }
}

pub(crate) fn meta_validate_names_unique(
    report: &mut MetaValidationReporter,
    nodes: Vec<roxmltree::Node<'_, '_>>,
    kind: &str,
) -> bool {
    let mut names = HashSet::<String>::new();
    let mut ok = true;
    for node in nodes {
        let Some(name) = meta_info_child(node, "Properties")
            .and_then(|props| meta_info_child_text(props, "Name"))
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !names.insert(name.clone()) {
            report.error(format!("8. Duplicate {kind} name: '{name}'"));
            ok = false;
        }
    }
    ok
}

pub(crate) fn meta_validate_check_tabular_sections(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let sections = meta_info_children(child_obj_node, "TabularSection");
    if sections.is_empty() {
        report.ok("9. TabularSections: none present");
        return;
    }
    let mut check_ok = true;
    for (index, section) in sections.iter().enumerate() {
        let count = index + 1;
        let uuid = section.attribute("uuid").unwrap_or("");
        if uuid.is_empty() || !is_guid(uuid) {
            report.error(format!(
                "9. TabularSection #{count}: invalid or missing uuid"
            ));
            check_ok = false;
        }
        let props = meta_info_child(*section, "Properties");
        let section_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(unnamed)".to_string());
        if section_name == "(unnamed)" {
            report.error(format!("9. TabularSection #{count}: missing or empty Name"));
            check_ok = false;
        }
        if let Some(internal_info) = meta_info_child(*section, "InternalInfo") {
            let generated = meta_info_children(internal_info, "GeneratedType");
            if generated.len() < 2 {
                report.warn(format!(
                    "9. TabularSection '{section_name}': expected 2 GeneratedType, found {}",
                    generated.len()
                ));
            }
        }
        if let Some(ts_child_obj) = meta_info_child(*section, "ChildObjects") {
            let mut names = HashSet::<String>::new();
            for attr in meta_info_children(ts_child_obj, "Attribute") {
                if !meta_validate_check_child_element(
                    report,
                    attr,
                    &format!("TabularSection '{section_name}'.Attribute"),
                    true,
                ) {
                    check_ok = false;
                }
                if let Some(name) = meta_info_child(attr, "Properties")
                    .and_then(|node| meta_info_child_text(node, "Name"))
                    .filter(|value| !value.is_empty())
                {
                    if !names.insert(name.clone()) {
                        report.error(format!(
                            "9. Duplicate attribute '{name}' in TabularSection '{section_name}'"
                        ));
                        check_ok = false;
                    }
                }
            }
            if let Some(props) = props {
                if let Some(std_attr) = meta_info_child(props, "StandardAttributes") {
                    let has_line_number = meta_info_children(std_attr, "StandardAttribute")
                        .iter()
                        .any(|attr| attr.attribute("name") == Some("LineNumber"));
                    if !has_line_number {
                        report.warn(format!(
                            "9. TabularSection '{section_name}': missing LineNumber StandardAttribute"
                        ));
                    }
                }
            }
        }
    }
    if check_ok {
        report.ok(format!(
            "9. TabularSections: {} sections, structure valid",
            sections.len()
        ));
    }
}

pub(crate) fn meta_validate_check_cross_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: Option<&Path>,
    obj_name: &str,
) {
    let Some(props_node) = props_node else {
        return;
    };
    let mut check_ok = true;
    let mut issues = 0usize;
    if meta_info_child_text(props_node, "Hierarchical").as_deref() == Some("false") {
        if let Some(hierarchy_type) =
            meta_info_child_text(props_node, "HierarchyType").filter(|value| !value.is_empty())
        {
            report.warn(format!(
                "10. HierarchyType='{hierarchy_type}' but Hierarchical=false"
            ));
            issues += 1;
        }
    }
    if md_type == "CommonModule" {
        let any_enabled = [
            "Server",
            "ClientManagedApplication",
            "ClientOrdinaryApplication",
            "ExternalConnection",
            "ServerCall",
            "Global",
        ]
        .iter()
        .any(|name| meta_info_child_text(props_node, name).as_deref() == Some("true"));
        if !any_enabled {
            report.warn("10. CommonModule: no execution context enabled");
            issues += 1;
        }
    }
    if md_type == "EventSubscription" {
        if meta_info_child_text(props_node, "Handler").is_none_or(|value| value.trim().is_empty()) {
            report.error("10. EventSubscription: empty Handler");
            check_ok = false;
            issues += 1;
        }
        let has_source = meta_info_child(props_node, "Source")
            .map(|node| !meta_info_children(node, "Type").is_empty())
            .unwrap_or(false);
        if !has_source {
            report.warn("10. EventSubscription: no Source types specified");
            issues += 1;
        }
    }
    if md_type == "ScheduledJob"
        && meta_info_child_text(props_node, "MethodName")
            .is_none_or(|value| value.trim().is_empty())
    {
        report.error("10. ScheduledJob: empty MethodName");
        check_ok = false;
        issues += 1;
    }
    for (type_name, property, message) in [
        (
            "AccountingRegister",
            "ChartOfAccounts",
            "10. AccountingRegister: empty ChartOfAccounts",
        ),
        (
            "CalculationRegister",
            "ChartOfCalculationTypes",
            "10. CalculationRegister: empty ChartOfCalculationTypes",
        ),
    ] {
        if md_type == type_name
            && meta_info_child_text(props_node, property)
                .is_none_or(|value| value.trim().is_empty())
        {
            report.error(message);
            check_ok = false;
            issues += 1;
        }
    }
    if md_type == "BusinessProcess"
        && meta_info_child_text(props_node, "Task").is_none_or(|value| value.trim().is_empty())
    {
        report.warn("10. BusinessProcess: empty Task reference");
        issues += 1;
    }
    if md_type == "CalculationRegister"
        && meta_info_child_text(props_node, "ActionPeriod").as_deref() == Some("true")
        && meta_info_child_text(props_node, "Schedule").is_none_or(|value| value.trim().is_empty())
    {
        report.warn(
            "10. CalculationRegister: ActionPeriod=true but Schedule is empty — platform requires a schedule register",
        );
        issues += 1;
    }
    if md_type == "DocumentJournal" {
        let has_registered = meta_info_child(props_node, "RegisteredDocuments")
            .map(|node| !meta_info_children(node, "Type").is_empty())
            .unwrap_or(false);
        if !has_registered {
            report.warn("10. DocumentJournal: no RegisteredDocuments specified");
            issues += 1;
        }
    }
    if md_type == "ChartOfAccounts" {
        let max_ext_dim = meta_info_child_text(props_node, "MaxExtDimensionCount")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        if max_ext_dim > 0
            && meta_info_child_text(props_node, "ExtDimensionTypes")
                .is_none_or(|value| value.trim().is_empty())
        {
            report
                .warn("10. ChartOfAccounts: MaxExtDimensionCount>0 but ExtDimensionTypes is empty");
            issues += 1;
        }
    }
    if matches!(
        md_type,
        "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "InformationRegister"
    ) {
        if let Some(child_obj_node) = child_obj_node {
            let count = meta_info_children(child_obj_node, "Dimension").len()
                + meta_info_children(child_obj_node, "Resource").len()
                + meta_info_children(child_obj_node, "Attribute").len();
            if count == 0 {
                report.warn(format!(
                    "10. {md_type}: no Dimensions, Resources, or Attributes — platform will reject"
                ));
                issues += 1;
            }
        }
    }
    meta_validate_check_document_register_records(
        report,
        md_type,
        props_node,
        config_dir,
        &mut issues,
    );
    meta_validate_check_register_registrar(
        report,
        md_type,
        props_node,
        config_dir,
        obj_name,
        &mut issues,
    );
    if check_ok && issues == 0 {
        report.ok("10. Cross-property consistency");
    }
}

pub(crate) fn meta_validate_check_document_register_records(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    config_dir: Option<&Path>,
    issues: &mut usize,
) {
    if md_type != "Document" {
        return;
    }
    let Some(config_dir) = config_dir else {
        return;
    };
    let Some(register_records) = meta_info_child(props_node, "RegisterRecords") else {
        return;
    };
    for item in meta_info_children(register_records, "Item") {
        let ref_value = meta_info_inner_text(item).trim().to_string();
        let Some((ref_type, ref_name)) = ref_value.split_once('.') else {
            continue;
        };
        let ref_dir = match ref_type {
            "AccumulationRegister" => "AccumulationRegisters",
            "InformationRegister" => "InformationRegisters",
            "AccountingRegister" => "AccountingRegisters",
            "CalculationRegister" => "CalculationRegisters",
            _ => continue,
        };
        let ref_path = config_dir.join(ref_dir).join(ref_name);
        let ref_xml = config_dir.join(ref_dir).join(format!("{ref_name}.xml"));
        if !ref_path.exists() && !ref_xml.exists() {
            report.warn(format!(
                "10. Document.RegisterRecords references '{ref_value}' but object not found in config"
            ));
            *issues += 1;
        }
    }
}

pub(crate) fn meta_validate_check_register_registrar(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    config_dir: Option<&Path>,
    obj_name: &str,
    issues: &mut usize,
) {
    if !matches!(
        md_type,
        "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "InformationRegister"
    ) || obj_name == "(unknown)"
    {
        return;
    }
    if md_type == "InformationRegister"
        && meta_info_child_text(props_node, "WriteMode").as_deref() != Some("RecorderSubordinate")
    {
        return;
    }
    let Some(config_dir) = config_dir else {
        return;
    };
    let docs_dir = config_dir.join("Documents");
    let reg_ref = format!("{md_type}.{obj_name}");
    let has_registrar = docs_dir.is_dir()
        && meta_validate_registrar_document_scan(&docs_dir, &reg_ref)
            .map(|(_, found)| found)
            .unwrap_or(false);
    if !has_registrar {
        report.warn(format!(
            "10. {md_type}: no registrar document found (none references '{reg_ref}' in RegisterRecords)"
        ));
        *issues += 1;
    }
}

pub(crate) fn meta_validate_check_services(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    if md_type == "HTTPService" {
        let templates = meta_info_children(child_obj_node, "URLTemplate");
        let mut check_ok = true;
        let mut method_count = 0usize;
        for template in &templates {
            let props = meta_info_child(*template, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_else(|| "(unnamed)".to_string());
            if props
                .and_then(|node| meta_info_child_text(node, "Template"))
                .is_none_or(|value| value.trim().is_empty())
            {
                report.error(format!(
                    "11. HTTPService URLTemplate '{name}': empty Template"
                ));
                check_ok = false;
            }
            if let Some(child_objects) = meta_info_child(*template, "ChildObjects") {
                for method in meta_info_children(child_objects, "Method") {
                    method_count += 1;
                    let props = meta_info_child(method, "Properties");
                    let http_method =
                        props.and_then(|node| meta_info_child_text(node, "HTTPMethod"));
                    if let Some(http_method) = http_method.filter(|value| !value.is_empty()) {
                        if !meta_validate_valid_http_methods().contains(&http_method.as_str()) {
                            report.error(format!(
                                "11. HTTPService URLTemplate '{name}': invalid HTTPMethod '{http_method}'"
                            ));
                            check_ok = false;
                        }
                    } else {
                        report.error(format!(
                            "11. HTTPService URLTemplate '{name}': Method missing HTTPMethod"
                        ));
                        check_ok = false;
                    }
                }
            }
        }
        if check_ok {
            report.ok(format!(
                "11. HTTPService: {} URLTemplate(s), {method_count} method(s)",
                templates.len()
            ));
        }
    } else if md_type == "WebService" {
        let operations = meta_info_children(child_obj_node, "Operation");
        let mut check_ok = true;
        let mut param_count = 0usize;
        for operation in &operations {
            let props = meta_info_child(*operation, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_else(|| "(unnamed)".to_string());
            if props
                .and_then(|node| meta_info_child_text(node, "XDTOReturningValueType"))
                .is_none_or(|value| value.trim().is_empty())
            {
                report.warn(format!(
                    "11. WebService Operation '{name}': no XDTOReturningValueType"
                ));
            }
            if let Some(child_objects) = meta_info_child(*operation, "ChildObjects") {
                for param in meta_info_children(child_objects, "Parameter") {
                    param_count += 1;
                    let direction = meta_info_child(param, "Properties")
                        .and_then(|node| meta_info_child_text(node, "TransferDirection"));
                    if let Some(direction) = direction.filter(|value| !value.is_empty()) {
                        if !["In", "Out", "InOut"].contains(&direction.as_str()) {
                            report.error(format!(
                                "11. WebService Operation '{name}': Parameter has invalid TransferDirection '{direction}'"
                            ));
                            check_ok = false;
                        }
                    }
                }
            }
        }
        if check_ok {
            report.ok(format!(
                "11. WebService: {} operation(s), {param_count} parameter(s)",
                operations.len()
            ));
        }
    }
}

pub(crate) fn meta_validate_check_forbidden_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        return;
    };
    let Some(forbidden) = meta_validate_forbidden_properties(md_type) else {
        return;
    };
    let mut check_ok = true;
    for property in forbidden {
        if meta_info_child(props_node, property).is_some() {
            report.error(format!(
                "12. Forbidden property '{property}' present in {md_type} (will fail on LoadConfigFromFiles)"
            ));
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("12. Forbidden properties: none found");
    }
}

pub(crate) fn meta_validate_check_method_reference(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: Option<&Path>,
) {
    if !matches!(md_type, "EventSubscription" | "ScheduledJob") {
        return;
    }
    let (Some(props_node), Some(config_dir)) = (props_node, config_dir) else {
        return;
    };
    let (property, method_ref) = if md_type == "EventSubscription" {
        ("Handler", meta_info_child_text(props_node, "Handler"))
    } else {
        ("MethodName", meta_info_child_text(props_node, "MethodName"))
    };
    let Some(method_ref) = method_ref.filter(|value| !value.is_empty()) else {
        return;
    };
    let parts = method_ref.split('.').collect::<Vec<_>>();
    let parsed = if parts.len() == 3 && parts[0] == "CommonModule" {
        Some((parts[1], parts[2]))
    } else if parts.len() == 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    };
    let Some((module_name, proc_name)) = parsed else {
        report.error(format!(
            "13. {md_type}.{property} = '{method_ref}': expected format 'CommonModule.ModuleName.ProcedureName'"
        ));
        return;
    };
    let module_xml = config_dir
        .join("CommonModules")
        .join(format!("{module_name}.xml"));
    if !module_xml.exists() {
        report.error(format!(
            "13. {md_type}.{property}: CommonModule '{module_name}' not found (expected {})",
            module_xml.display()
        ));
        return;
    }
    let bsl_path = config_dir
        .join("CommonModules")
        .join(module_name)
        .join("Ext")
        .join("Module.bsl");
    if bsl_path.exists() {
        if let Ok(content) = read_utf8_sig(&bsl_path) {
            if !meta_validate_bsl_has_export(&content, proc_name) {
                report.warn(format!(
                    "13. {md_type}.{property}: procedure '{proc_name}' not found as exported in CommonModule '{module_name}'"
                ));
                return;
            }
        }
    } else {
        report.warn(format!(
            "13. {md_type}.{property}: BSL file not found ({}), cannot verify procedure",
            bsl_path.display()
        ));
        return;
    }
    report.ok(format!("13. Method reference: {property} = '{method_ref}'"));
}

pub(crate) fn meta_validate_check_document_journal_columns(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    if md_type != "DocumentJournal" {
        return;
    }
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let columns = meta_info_children(child_obj_node, "Column");
    let mut check_ok = true;
    let mut empty_ref_count = 0usize;
    for column in &columns {
        let props = meta_info_child(*column, "Properties");
        let name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_else(|| "(unnamed)".to_string());
        let has_items = props
            .and_then(|node| meta_info_child(node, "References"))
            .map(|node| !meta_info_children(node, "Item").is_empty())
            .unwrap_or(false);
        if !has_items {
            report.error(format!(
                "14. DocumentJournal Column '{name}': empty References (will fail on LoadConfigFromFiles)"
            ));
            check_ok = false;
            empty_ref_count += 1;
        }
    }
    if check_ok && !columns.is_empty() {
        report.ok(format!(
            "14. DocumentJournal Columns: {} column(s), all have References",
            columns.len()
        ));
    } else if columns.is_empty() && empty_ref_count == 0 {
        report.ok("14. DocumentJournal Columns: none");
    }
}

pub(crate) fn meta_validate_bsl_has_export(content: &str, proc_name: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        let starts = ["Procedure", "Function", "Процедура", "Функция"]
            .iter()
            .any(|prefix| trimmed.starts_with(prefix));
        starts
            && trimmed.contains(proc_name)
            && (trimmed.contains(" Export") || trimmed.contains(" Экспорт"))
    })
}

pub(crate) fn is_guid(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && value
            .chars()
            .enumerate()
            .all(|(index, ch)| [8, 13, 18, 23].contains(&index) || ch.is_ascii_hexdigit())
}

pub(crate) fn meta_validate_valid_types() -> &'static [&'static str] {
    &[
        "Catalog",
        "Document",
        "Enum",
        "Constant",
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
        "Report",
        "DataProcessor",
        "ExternalReport",
        "ExternalDataProcessor",
        "CommonModule",
        "ScheduledJob",
        "EventSubscription",
        "HTTPService",
        "WebService",
        "DefinedType",
    ]
}

pub(crate) fn meta_validate_generated_categories(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "Catalog" | "Document" => Some(&["Object", "Ref", "Selection", "List", "Manager"]),
        "Enum" => Some(&["Ref", "Manager", "List"]),
        "Constant" => Some(&["Manager", "ValueManager", "ValueKey"]),
        "InformationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "RecordManager",
        ]),
        "AccumulationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
        ]),
        "AccountingRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "ExtDimensions",
        ]),
        "CalculationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "Recalcs",
        ]),
        "ChartOfAccounts" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "ExtDimensionTypes",
            "ExtDimensionTypesRow",
        ]),
        "ChartOfCharacteristicTypes" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "Characteristic",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "DisplacingCalculationTypes",
            "DisplacingCalculationTypesRow",
            "BaseCalculationTypes",
            "BaseCalculationTypesRow",
            "LeadingCalculationTypes",
            "LeadingCalculationTypesRow",
        ]),
        "BusinessProcess" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "RoutePointRef",
        ]),
        "Task" | "ExchangePlan" => Some(&["Object", "Ref", "Selection", "List", "Manager"]),
        "DocumentJournal" => Some(&["Selection", "List", "Manager"]),
        "Report" | "DataProcessor" => Some(&["Object", "Manager"]),
        "ExternalReport" | "ExternalDataProcessor" => Some(&["Object"]),
        "DefinedType" => Some(&["DefinedType"]),
        _ => None,
    }
}

pub(crate) fn meta_validate_types_without_internal_info() -> &'static [&'static str] {
    &["CommonModule", "ScheduledJob", "EventSubscription"]
}

pub(crate) fn meta_validate_types_with_std_attrs() -> &'static [&'static str] {
    &[
        "Catalog",
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
    ]
}

pub(crate) fn meta_validate_standard_attributes(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "Catalog" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "IsFolder",
            "Owner",
            "Parent",
            "Description",
            "Code",
        ]),
        "Document" => Some(&["Posted", "Ref", "DeletionMark", "Date", "Number"]),
        "Enum" => Some(&["Order", "Ref"]),
        "InformationRegister" => Some(&["Active", "LineNumber", "Recorder", "Period"]),
        "AccumulationRegister" => {
            Some(&["RecordType", "Active", "LineNumber", "Recorder", "Period"])
        }
        "AccountingRegister" => Some(&[
            "Account",
            "RecordType",
            "Active",
            "LineNumber",
            "Recorder",
            "Period",
        ]),
        "CalculationRegister" => Some(&[
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
        ]),
        "ChartOfAccounts" => Some(&[
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
        ]),
        "ChartOfCharacteristicTypes" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "Description",
            "Code",
            "Parent",
            "IsFolder",
            "ValueType",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "ActionPeriodIsBasic",
            "Description",
            "Code",
        ]),
        "BusinessProcess" => Some(&[
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
            "Started",
            "Completed",
            "HeadTask",
        ]),
        "Task" => Some(&[
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
            "Executed",
            "Description",
            "RoutePoint",
            "BusinessProcess",
        ]),
        "ExchangePlan" => Some(&[
            "Ref",
            "DeletionMark",
            "Code",
            "Description",
            "ThisNode",
            "SentNo",
            "ReceivedNo",
        ]),
        "DocumentJournal" => Some(&["Type", "Ref", "Date", "Posted", "DeletionMark", "Number"]),
        _ => None,
    }
}

pub(crate) fn meta_validate_dynamic_standard_attr(md_type: &str, name: &str) -> bool {
    (md_type == "AccountingRegister"
        && (name == "PeriodAdjustment"
            || name
                .strip_prefix("ExtDimension")
                .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))
            || name
                .strip_prefix("ExtDimensionType")
                .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))))
        || (md_type == "CalculationRegister"
            && matches!(
                name,
                "ActionPeriod"
                    | "BegOfActionPeriod"
                    | "EndOfActionPeriod"
                    | "BegOfBasePeriod"
                    | "EndOfBasePeriod"
            ))
}

pub(crate) fn meta_validate_child_rules(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "Catalog"
        | "Document"
        | "ExchangePlan"
        | "ChartOfCharacteristicTypes"
        | "ChartOfCalculationTypes"
        | "BusinessProcess"
        | "Report"
        | "DataProcessor"
        | "ExternalReport"
        | "ExternalDataProcessor" => {
            Some(&["Attribute", "TabularSection", "Form", "Template", "Command"])
        }
        "ChartOfAccounts" => Some(&[
            "Attribute",
            "TabularSection",
            "Form",
            "Template",
            "Command",
            "AccountingFlag",
            "ExtDimensionAccountingFlag",
        ]),
        "Task" => Some(&[
            "Attribute",
            "TabularSection",
            "Form",
            "Template",
            "Command",
            "AddressingAttribute",
        ]),
        "Enum" => Some(&["EnumValue", "Form", "Template", "Command"]),
        "InformationRegister" | "AccumulationRegister" | "AccountingRegister" => Some(&[
            "Dimension",
            "Resource",
            "Attribute",
            "Form",
            "Template",
            "Command",
        ]),
        "CalculationRegister" => Some(&[
            "Dimension",
            "Resource",
            "Attribute",
            "Form",
            "Template",
            "Command",
            "Recalculation",
        ]),
        "DocumentJournal" => Some(&["Column", "Form", "Template", "Command"]),
        "HTTPService" => Some(&["URLTemplate"]),
        "WebService" => Some(&["Operation"]),
        "Constant" => Some(&["Form"]),
        "DefinedType" | "CommonModule" | "ScheduledJob" | "EventSubscription" => Some(&[]),
        _ => None,
    }
}

pub(crate) fn meta_validate_property_values() -> &'static [(&'static str, &'static [&'static str])]
{
    &[
        ("CodeType", &["String", "Number"]),
        ("CodeAllowedLength", &["Variable", "Fixed"]),
        ("NumberType", &["String", "Number"]),
        ("NumberAllowedLength", &["Variable", "Fixed"]),
        ("Posting", &["Allow", "Deny"]),
        ("RealTimePosting", &["Allow", "Deny"]),
        (
            "RegisterRecordsDeletion",
            &["AutoDelete", "AutoDeleteOnUnpost", "AutoDeleteOff"],
        ),
        (
            "RegisterRecordsWritingOnPost",
            &["WriteModified", "WriteSelected", "WriteAll"],
        ),
        ("DataLockControlMode", &["Automatic", "Managed"]),
        ("FullTextSearch", &["Use", "DontUse"]),
        ("DefaultPresentation", &["AsDescription", "AsCode"]),
        (
            "HierarchyType",
            &["HierarchyFoldersAndItems", "HierarchyOfItems"],
        ),
        ("EditType", &["InDialog", "InList", "BothWays"]),
        ("WriteMode", &["Independent", "RecorderSubordinate"]),
        (
            "InformationRegisterPeriodicity",
            &[
                "Nonperiodical",
                "Second",
                "Day",
                "Month",
                "Quarter",
                "Year",
                "RecorderPosition",
            ],
        ),
        ("RegisterType", &["Balance", "Turnovers"]),
        (
            "ReturnValuesReuse",
            &["DontUse", "DuringRequest", "DuringSession"],
        ),
        ("ReuseSessions", &["DontUse", "AutoUse"]),
        ("FillChecking", &["DontCheck", "ShowError"]),
        (
            "Indexing",
            &["DontIndex", "Index", "IndexWithAdditionalOrder"],
        ),
        ("DataHistory", &["Use", "DontUse"]),
        (
            "DependenceOnCalculationTypes",
            &["DontUse", "OnActionPeriod"],
        ),
        (
            "SubordinationUse",
            &["ToFolders", "ToFoldersAndItems", "ToItems"],
        ),
        (
            "CatalogCodeSeries",
            &[
                "WholeCatalog",
                "WithinOwnerSubordination",
                "WithinSubordination",
            ],
        ),
        (
            "ChartOfAccountsCodeSeries",
            &["WholeChartOfAccounts", "WithinSubordination"],
        ),
        (
            "CharacteristicTypeCodeSeries",
            &["WholeCharacteristicKind", "WithinSubordination"],
        ),
        ("ChoiceMode", &["BothWays", "FromForm", "QuickChoice"]),
        (
            "DocumentNumberPeriodicity",
            &["Day", "Month", "Nonperiodical", "Quarter", "Year"],
        ),
        (
            "BusinessProcessNumberPeriodicity",
            &["Day", "Month", "Nonperiodical", "Quarter", "Year"],
        ),
        (
            "CalculationRegisterPeriodicity",
            &["Day", "Month", "Quarter", "Year"],
        ),
        (
            "PredefinedDataUpdate",
            &["Auto", "AutoUpdate", "DontAutoUpdate"],
        ),
        (
            "HTTPMethod",
            &[
                "Any",
                "CONNECT",
                "COPY",
                "DELETE",
                "GET",
                "HEAD",
                "LOCK",
                "MERGE",
                "MKCOL",
                "MOVE",
                "OPTIONS",
                "PATCH",
                "POST",
                "PROPFIND",
                "PROPPATCH",
                "PUT",
                "TRACE",
                "UNLOCK",
            ],
        ),
        ("TransferDirection", &["In", "InOut", "Out"]),
    ]
}

pub(crate) fn meta_validate_reserved_attr_names() -> &'static [&'static str] {
    &[
        "Ref",
        "DeletionMark",
        "Code",
        "Description",
        "Date",
        "Number",
        "Posted",
        "Parent",
        "Owner",
        "IsFolder",
        "Predefined",
        "PredefinedDataName",
        "Recorder",
        "Period",
        "LineNumber",
        "Active",
        "Order",
        "Type",
        "OffBalance",
        "Started",
        "Completed",
        "HeadTask",
        "Executed",
        "RoutePoint",
        "BusinessProcess",
        "ThisNode",
        "SentNo",
        "ReceivedNo",
        "CalculationType",
        "RegistrationPeriod",
        "ReversingEntry",
        "Account",
        "ValueType",
        "ActionPeriodIsBasic",
    ]
}

pub(crate) fn meta_validate_valid_http_methods() -> &'static [&'static str] {
    &[
        "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "MERGE", "CONNECT",
    ]
}

pub(crate) fn meta_validate_forbidden_properties(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "ChartOfCharacteristicTypes" => Some(&["CodeType"]),
        "ChartOfAccounts" => Some(&["Autonumbering", "Hierarchical"]),
        "ChartOfCalculationTypes" => Some(&["CheckUnique", "Autonumbering"]),
        "ExchangePlan" => Some(&["CodeType", "CheckUnique", "Autonumbering"]),
        _ => None,
    }
}

pub(crate) fn analyze_meta_info(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<(String, Option<PathBuf>, PathBuf), String> {
        let raw_path = required_path(
            args,
            &["objectPath", "ObjectPath", "path", "Path"],
            "ObjectPath",
        )?;
        let object_path = resolve_meta_info_path(absolutize(raw_path, &context.cwd))?;
        let text = read_utf8_sig(&object_path)?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", object_path.display()))?;
        let root = doc.root_element();
        if root.tag_name().name() != "MetaDataObject" {
            return Err("[ERROR] Not a valid 1C metadata XML file".to_string());
        }

        let Some(type_node) = root
            .children()
            .find(|child| child.is_element() && child.tag_name().namespace() == Some(MD_NS))
        else {
            return Err("[ERROR] Cannot detect metadata type".to_string());
        };
        let md_type = type_node.tag_name().name();
        let props = meta_info_child(type_node, "Properties");
        let child_objs = meta_info_child(type_node, "ChildObjects");
        let obj_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        let synonym = props
            .and_then(|node| meta_info_child(node, "Synonym"))
            .map(meta_info_ml_text)
            .unwrap_or_default();
        let mode = string_arg(args, &["mode", "Mode"]).unwrap_or("overview");
        let drill_name = string_arg(args, &["name", "Name"]).unwrap_or("");
        let out_file =
            path_arg(args, &["outFile", "OutFile"]).map(|path| absolutize(path, &context.cwd));

        let mut lines = if drill_name.is_empty() {
            meta_info_main_lines(md_type, props, child_objs, &obj_name, &synonym, mode)?
        } else {
            meta_info_drill_lines(md_type, child_objs, drill_name, &obj_name)?
        };
        if drill_name.is_empty() {
            lines.insert(
                1,
                format!("Поддержка: {}", support_status_for_path(&object_path)),
            );
        }
        let output_text = meta_info_paginate(lines, args);
        let stdout = if let Some(out_file) = &out_file {
            write_utf8_bom(out_file, &output_text)?;
            format!("Output written to {}\n", out_file.display())
        } else {
            format!("{output_text}\n")
        };

        Ok((stdout, out_file, object_path))
    })();

    match result {
        Ok((stdout, out_file, artifact)) => {
            let mut artifacts = vec![artifact.display().to_string()];
            if let Some(out_file) = out_file {
                artifacts.push(out_file.display().to_string());
            }
            AdapterOutcome {
                ok: true,
                summary: "unica.meta.info completed with native metadata analyzer".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts,
                stdout: Some(stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.info failed in native metadata analyzer".to_string(),
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

pub(crate) fn resolve_meta_info_path(mut object_path: PathBuf) -> Result<PathBuf, String> {
    if object_path.is_dir() {
        let dir_name = object_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let candidate = object_path.join(format!("{dir_name}.xml"));
        let sibling = object_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(format!("{dir_name}.xml"));
        if candidate.is_file() {
            object_path = candidate;
        } else if sibling.is_file() {
            object_path = sibling;
        } else {
            let mut xml_files = fs::read_dir(&object_path)
                .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
                })
                .collect::<Vec<_>>();
            xml_files.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
            if let Some(xml_file) = xml_files.into_iter().next() {
                object_path = xml_file;
            } else {
                return Err(format!(
                    "[ERROR] No XML file found in directory: {}",
                    object_path.display()
                ));
            }
        }
    }

    if !object_path.exists() {
        let file_name = object_path.file_stem().and_then(|name| name.to_str());
        let parent_dir = object_path.parent();
        let parent_dir_name = parent_dir
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str());
        if file_name == parent_dir_name {
            if let (Some(parent_dir), Some(file_name)) = (parent_dir, file_name) {
                let candidate = parent_dir
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .join(format!("{file_name}.xml"));
                if candidate.exists() {
                    object_path = candidate;
                }
            }
        }
    }

    if !object_path.exists() {
        return Err(format!("[ERROR] File not found: {}", object_path.display()));
    }
    Ok(object_path)
}

pub(crate) fn meta_info_main_lines(
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
    synonym: &str,
    mode: &str,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::<String>::new();
    let ru_type_name = meta_info_type_ru(md_type);
    let mut header = format!("=== {ru_type_name}: {obj_name}");
    if !synonym.is_empty() && synonym != obj_name {
        header.push_str(&format!(" — \"{synonym}\""));
    }
    header.push_str(" ===");
    lines.push(header);

    if meta_info_is_reference_metadata_type(md_type) {
        let object_presentation = meta_info_ml_child_text(props, "ObjectPresentation");
        let extended_object_presentation =
            meta_info_ml_child_text(props, "ExtendedObjectPresentation");
        let list_presentation = meta_info_ml_child_text(props, "ListPresentation");
        let extended_list_presentation = meta_info_ml_child_text(props, "ExtendedListPresentation");
        let type_presentation = object_presentation
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| (!synonym.is_empty()).then_some(synonym))
            .unwrap_or(obj_name);
        lines.push(format!("Представление типа: {type_presentation}"));
        if mode == "full" {
            if let Some(value) = object_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Представление объекта: {value}"));
            }
            if let Some(value) = extended_object_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Расширенное представление объекта: {value}"));
            }
            if let Some(value) = list_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Представление списка: {value}"));
            }
            if let Some(value) = extended_list_presentation.filter(|value| !value.is_empty()) {
                lines.push(format!("Расширенное представление списка: {value}"));
            }
        }
    }

    if mode == "brief" {
        meta_info_append_brief(&mut lines, md_type, props, child_objs);
    } else if mode == "overview" || mode == "full" {
        meta_info_append_overview_or_full(&mut lines, md_type, props, child_objs, mode);
    } else {
        return Err(format!(
            "argument -Mode: invalid choice: '{mode}' (choose from 'overview', 'brief', 'full')"
        ));
    }
    Ok(lines)
}

pub(crate) fn meta_info_append_brief(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let attrs = meta_info_attributes(child_objs, "Attribute", false);
    if !attrs.is_empty() {
        lines.push(format!(
            "Реквизиты ({}): {}",
            attrs.len(),
            attrs
                .iter()
                .map(|attr| attr.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if md_type.ends_with("Register") {
        let dims = meta_info_attributes(child_objs, "Dimension", true);
        if !dims.is_empty() {
            lines.push(format!(
                "Измерения ({}): {}",
                dims.len(),
                dims.iter()
                    .map(|attr| attr.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let resources = meta_info_attributes(child_objs, "Resource", false);
        if !resources.is_empty() {
            lines.push(format!(
                "Ресурсы ({}): {}",
                resources.len(),
                resources
                    .iter()
                    .map(|attr| attr.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    let tabular_sections = meta_info_tabular_sections(child_objs);
    if !tabular_sections.is_empty() {
        let parts = tabular_sections
            .iter()
            .map(|section| format!("{}({})", section.name, section.columns.len()))
            .collect::<Vec<_>>();
        lines.push(format!(
            "ТЧ ({}): {}",
            tabular_sections.len(),
            parts.join(", ")
        ));
    }

    if md_type == "Enum" {
        let values = meta_info_enum_values(child_objs);
        if !values.is_empty() {
            lines.push(format!(
                "Значения ({}): {}",
                values.len(),
                values.join(", ")
            ));
        }
    }

    if md_type == "DefinedType" {
        if let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) {
            let types = meta_info_children(type_node, "Type")
                .into_iter()
                .map(|node| meta_info_format_single_type(meta_info_inner_text(node), type_node))
                .collect::<Vec<_>>();
            if !types.is_empty() {
                lines.push(format!("Типы ({}): {}", types.len(), types.join(", ")));
            }
        }
    }

    if md_type == "CommonModule" {
        let flags = meta_info_common_module_flags(props);
        if !flags.is_empty() {
            lines.push(flags.join(" | "));
        }
    }

    if md_type == "ScheduledJob" {
        meta_info_append_scheduled_job(lines, props);
    }

    if md_type == "EventSubscription" {
        meta_info_append_event_subscription_brief(lines, props);
    }

    if md_type == "HTTPService" {
        if let Some(root_url) = props.and_then(|node| meta_info_child_text(node, "RootURL")) {
            if !root_url.is_empty() {
                lines.push(format!("Корневой URL: /{root_url}"));
            }
        }
        let endpoints = meta_info_http_endpoints(child_objs);
        if !endpoints.is_empty() {
            let total_methods = endpoints
                .iter()
                .map(|endpoint| endpoint.methods.len())
                .sum::<usize>();
            lines.push(format!(
                "Шаблоны: {} | Методы: {total_methods}",
                endpoints.len()
            ));
        }
    }

    if md_type == "WebService" {
        if let Some(namespace) = props.and_then(|node| meta_info_child_text(node, "Namespace")) {
            if !namespace.is_empty() {
                lines.push(format!("Пространство имён: {namespace}"));
            }
        }
        let operations = meta_info_ws_operations(child_objs);
        if !operations.is_empty() {
            lines.push(format!("Операции: {}", operations.len()));
        }
    }
}

pub(crate) fn meta_info_append_overview_or_full(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    if md_type == "Document" {
        meta_info_append_document_header(lines, props);
    }
    if md_type == "Catalog" {
        meta_info_append_catalog_header(lines, props);
    }
    if md_type.ends_with("Register") {
        meta_info_append_register_header(lines, md_type, props);
    }
    if md_type == "Constant" {
        if let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) {
            let type_name = meta_info_format_type(type_node);
            if !type_name.is_empty() {
                lines.push(format!("Тип: {type_name}"));
            }
        }
    }
    if md_type == "Report" {
        if let Some(main_dcs) =
            props.and_then(|node| meta_info_child_text(node, "MainDataCompositionSchema"))
        {
            if !main_dcs.is_empty() {
                let dcs_name = main_dcs
                    .rsplit_once(".Template.")
                    .map(|(_, name)| name)
                    .unwrap_or(&main_dcs);
                lines.push(format!("Основная СКД: {dcs_name}"));
            }
        }
    }
    if md_type == "DefinedType" {
        meta_info_append_defined_type(lines, props);
    }
    if md_type == "CommonModule" {
        let flags = meta_info_common_module_flags(props);
        if !flags.is_empty() {
            lines.push(flags.join(" | "));
        }
    }
    if md_type == "ScheduledJob" {
        meta_info_append_scheduled_job(lines, props);
    }
    if md_type == "EventSubscription" {
        meta_info_append_event_subscription(lines, props, mode);
    }
    if md_type == "HTTPService" {
        meta_info_append_http_service(lines, props, child_objs);
    }
    if md_type == "WebService" {
        meta_info_append_web_service(lines, props, child_objs);
    }
    if md_type == "Enum" {
        meta_info_append_enum_values(lines, child_objs);
    }
    if md_type.ends_with("Register") {
        meta_info_append_attribute_section(lines, "Измерения", child_objs, "Dimension", true);
        meta_info_append_attribute_section(lines, "Ресурсы", child_objs, "Resource", false);
    }
    if md_type != "Enum" {
        meta_info_append_attribute_section(lines, "Реквизиты", child_objs, "Attribute", false);
        meta_info_append_tabular_sections(lines, child_objs, mode);
    }
    if mode == "overview" && matches!(md_type, "Report" | "DataProcessor") {
        meta_info_append_simple_children(lines, child_objs);
    }
    if mode == "full" {
        meta_info_append_full_tail(lines, md_type, props, child_objs);
    }
}

pub(crate) fn meta_info_drill_lines(
    md_type: &str,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    drill_name: &str,
    obj_name: &str,
) -> Result<Vec<String>, String> {
    let Some(child_objs) = child_objs else {
        return Err(format!("[ERROR] '{drill_name}' not found in {obj_name}"));
    };
    for (tag, label, is_dimension) in [
        ("Attribute", "Реквизит", false),
        ("Dimension", "Измерение", true),
        ("Resource", "Ресурс", false),
    ] {
        for attr in meta_info_children(child_objs, tag) {
            let Some(props) = meta_info_child(attr, "Properties") else {
                continue;
            };
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            if name == drill_name {
                return Ok(meta_info_drill_attr_lines(
                    label,
                    &name,
                    props,
                    is_dimension,
                ));
            }
        }
    }

    for section in meta_info_children(child_objs, "TabularSection") {
        let props = meta_info_child(section, "Properties");
        let section_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        if section_name == drill_name {
            let section_child_objs = meta_info_child(section, "ChildObjects");
            let columns = meta_info_attributes(section_child_objs, "Attribute", false);
            let mut lines = vec![format!(
                "ТЧ: {section_name} ({} {}):",
                columns.len(),
                meta_info_decline_cols(columns.len())
            )];
            if !columns.is_empty() {
                let width = meta_info_max_name_len(&columns);
                for column in columns {
                    lines.push(meta_info_format_attr_line(&column, width));
                }
            }
            return Ok(lines);
        }
    }

    for value in meta_info_children(child_objs, "EnumValue") {
        let props = meta_info_child(value, "Properties");
        let value_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        if value_name == drill_name {
            let mut lines = vec![format!("Значение перечисления: {value_name}")];
            if let Some(synonym) = props
                .and_then(|node| meta_info_child(node, "Synonym"))
                .map(meta_info_ml_text)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("  Синоним: \"{synonym}\""));
            }
            if let Some(comment) = props
                .and_then(|node| meta_info_child_text(node, "Comment"))
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("  Комментарий: {comment}"));
            }
            return Ok(lines);
        }
    }

    if md_type == "HTTPService" {
        for endpoint in meta_info_http_endpoints(Some(child_objs)) {
            if endpoint.name == drill_name {
                let mut lines = vec![
                    format!("Шаблон URL: {drill_name}"),
                    format!("  Путь: {}", endpoint.template),
                ];
                for method in endpoint.methods {
                    lines.push(format!("  {} → {}", method.http_method, method.handler));
                }
                return Ok(lines);
            }
        }
    }

    if md_type == "WebService" {
        for operation in meta_info_ws_operations(Some(child_objs)) {
            if operation.name == drill_name {
                let mut lines = vec![
                    format!("Операция: {drill_name}"),
                    format!("  Возвращает: {}", operation.return_type),
                ];
                if !operation.proc_name.is_empty() {
                    lines.push(format!("  Процедура: {}", operation.proc_name));
                }
                return Ok(lines);
            }
        }
    }

    Err(format!("[ERROR] '{drill_name}' not found in {obj_name}"))
}

pub(crate) fn meta_info_drill_attr_lines(
    label: &str,
    name: &str,
    props: roxmltree::Node<'_, '_>,
    is_dimension: bool,
) -> Vec<String> {
    let type_name = meta_info_child(props, "Type")
        .map(meta_info_format_type)
        .unwrap_or_default();
    let fill_checking = meta_info_child_text(props, "FillChecking").unwrap_or_default();
    let indexing = meta_info_child_text(props, "Indexing").unwrap_or_default();
    let indexing_ru = match indexing.as_str() {
        "" | "DontIndex" => "нет".to_string(),
        "Index" => "Индекс".to_string(),
        "IndexWithAdditionalOrder" => "Индекс с доп. упорядочиванием".to_string(),
        other => other.to_string(),
    };
    let mut lines = vec![
        format!("{label}: {name}"),
        format!("  Тип: {type_name}"),
        format!(
            "  Обязательный: {}",
            if fill_checking == "ShowError" {
                "да"
            } else {
                "нет"
            }
        ),
        format!("  Индексирование: {indexing_ru}"),
    ];
    if meta_info_child_text(props, "MultiLine").as_deref() == Some("true") {
        lines.push("  Многострочный: да".to_string());
    }
    if let Some(use_value) = meta_info_child_text(props, "Use") {
        if use_value != "ForItem" {
            let use_ru = match use_value.as_str() {
                "ForFolder" => "для папок",
                "ForFolderAndItem" => "для папок и элементов",
                _ => &use_value,
            };
            lines.push(format!("  Использование: {use_ru}"));
        }
    }
    if let Some(fill_value) = meta_info_child(props, "FillValue") {
        let value = meta_info_inner_text(fill_value);
        if meta_info_attr_by_local(fill_value, "nil") != Some("true") && !value.is_empty() {
            let value = match value.as_str() {
                "false" => "Ложь".to_string(),
                "true" => "Истина".to_string(),
                other if other.ends_with(".EmptyRef") => "Пустая ссылка".to_string(),
                other => other.to_string(),
            };
            lines.push(format!("  Значение заполнения: {value}"));
        } else {
            lines.push("  Значение заполнения: —".to_string());
        }
    } else {
        lines.push("  Значение заполнения: —".to_string());
    }
    if is_dimension {
        lines.push(format!(
            "  Ведущее: {}",
            if meta_info_child_text(props, "Master").as_deref() == Some("true") {
                "да"
            } else {
                "нет"
            }
        ));
        lines.push(format!(
            "  Основной отбор: {}",
            if meta_info_child_text(props, "MainFilter").as_deref() == Some("true") {
                "да"
            } else {
                "нет"
            }
        ));
    }
    if let Some(synonym) = meta_info_child(props, "Synonym")
        .map(meta_info_ml_text)
        .filter(|value| !value.is_empty() && value != name)
    {
        lines.push(format!("  Синоним: {synonym}"));
    }
    lines
}

pub(crate) fn meta_info_append_document_header(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    let number_type = meta_info_child_text(props, "NumberType");
    let number_length = meta_info_child_text(props, "NumberLength");
    if let (Some(number_type), Some(number_length)) = (number_type, number_length) {
        let type_name = if number_type == "String" {
            "Строка"
        } else {
            "Число"
        };
        let mut piece = format!("Номер: {type_name}({number_length})");
        if let Some(periodicity) = meta_info_child_text(props, "NumberPeriodicity") {
            piece.push_str(&format!(", {}", meta_info_number_period_ru(&periodicity)));
        }
        if meta_info_child_text(props, "Autonumbering").as_deref() == Some("true") {
            piece.push_str(", авто");
        }
        parts.push(piece);
    }
    if let Some(posting) = meta_info_child_text(props, "Posting") {
        parts.push(format!(
            "Проведение: {}",
            if posting == "Allow" { "да" } else { "нет" }
        ));
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_catalog_header(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if meta_info_child_text(props, "Hierarchical").as_deref() == Some("true") {
        let mut hierarchy_type = if meta_info_child_text(props, "HierarchyType").as_deref()
            == Some("HierarchyFoldersAndItems")
        {
            "группы и элементы".to_string()
        } else {
            "элементы".to_string()
        };
        if meta_info_child_text(props, "LimitLevelCount").as_deref() == Some("true") {
            if let Some(level_count) = meta_info_child_text(props, "LevelCount") {
                hierarchy_type.push_str(&format!(", уровней: {level_count}"));
            }
        } else {
            hierarchy_type.push_str(", без ограничения уровней");
        }
        parts.push(format!("Иерархический: {hierarchy_type}"));
    }
    if let Some(code_length) = meta_info_child_text(props, "CodeLength") {
        if code_length.parse::<i64>().unwrap_or(0) > 0 {
            parts.push(format!("Код({code_length})"));
        }
    }
    if let Some(description_length) = meta_info_child_text(props, "DescriptionLength") {
        if description_length.parse::<i64>().unwrap_or(0) > 0 {
            parts.push(format!("Наименование({description_length})"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_register_header(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if md_type == "InformationRegister" {
        if let Some(periodicity) = meta_info_child_text(props, "InformationRegisterPeriodicity") {
            parts.push(format!(
                "Периодичность: {}",
                meta_info_period_ru(&periodicity)
            ));
        }
        if let Some(write_mode) = meta_info_child_text(props, "WriteMode") {
            parts.push(format!("Запись: {}", meta_info_write_mode_ru(&write_mode)));
        }
    }
    if md_type == "AccumulationRegister" {
        if let Some(register_type) = meta_info_child_text(props, "RegisterType") {
            let register_type = match register_type.as_str() {
                "Balances" => "остатки",
                "Turnovers" => "обороты",
                _ => &register_type,
            };
            parts.push(format!("Вид: {register_type}"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_defined_type(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(type_node) = props.and_then(|node| meta_info_child(node, "Type")) else {
        return;
    };
    let types = meta_info_children(type_node, "Type")
        .into_iter()
        .map(|node| meta_info_format_single_type(meta_info_inner_text(node), type_node))
        .collect::<Vec<_>>();
    if types.is_empty() {
        return;
    }
    lines.push(format!("Типы ({}):", types.len()));
    for type_name in types {
        lines.push(format!("  {type_name}"));
    }
}

pub(crate) fn meta_info_append_scheduled_job(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    if let Some(method) =
        meta_info_child_text(props, "MethodName").filter(|value| !value.is_empty())
    {
        lines.push(format!(
            "Метод: {}",
            method.strip_prefix("CommonModule.").unwrap_or(&method)
        ));
    }
    let mut parts = Vec::new();
    parts.push(format!(
        "Использование: {}",
        if meta_info_child_text(props, "Use").as_deref() == Some("true") {
            "да"
        } else {
            "нет"
        }
    ));
    parts.push(format!(
        "Предопределённое: {}",
        if meta_info_child_text(props, "Predefined").as_deref() == Some("true") {
            "да"
        } else {
            "нет"
        }
    ));
    let restart_count = meta_info_child_text(props, "RestartCountOnFailure")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if restart_count > 0 {
        let interval = meta_info_child_text(props, "RestartIntervalOnFailure").unwrap_or_default();
        parts.push(format!(
            "Перезапуск: {restart_count} (через {interval} сек)"
        ));
    }
    lines.push(parts.join(" | "));
}

pub(crate) fn meta_info_append_event_subscription_brief(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props) = props else {
        return;
    };
    let mut parts = Vec::new();
    if let Some(event) = meta_info_child_text(props, "Event").filter(|value| !value.is_empty()) {
        parts.push(format!("Событие: {}", meta_info_event_ru(&event)));
    }
    if let Some(handler) = meta_info_child_text(props, "Handler").filter(|value| !value.is_empty())
    {
        parts.push(format!(
            "Обработчик: {}",
            handler.strip_prefix("CommonModule.").unwrap_or(&handler)
        ));
    }
    if let Some(source) = meta_info_child(props, "Source") {
        let source_count = meta_info_children(source, "Type").len();
        if source_count > 0 {
            parts.push(format!("Источники: {source_count}"));
        }
    }
    if !parts.is_empty() {
        lines.push(parts.join(" | "));
    }
}

pub(crate) fn meta_info_append_event_subscription(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    let Some(props) = props else {
        return;
    };
    if let Some(event) = meta_info_child_text(props, "Event").filter(|value| !value.is_empty()) {
        lines.push(format!("Событие: {}", meta_info_event_ru(&event)));
    }
    if let Some(handler) = meta_info_child_text(props, "Handler").filter(|value| !value.is_empty())
    {
        lines.push(format!(
            "Обработчик: {}",
            handler.strip_prefix("CommonModule.").unwrap_or(&handler)
        ));
    }
    if let Some(source) = meta_info_child(props, "Source") {
        let source_types = meta_info_children(source, "Type")
            .into_iter()
            .map(|node| meta_info_format_source_type(&meta_info_inner_text(node)))
            .collect::<Vec<_>>();
        if !source_types.is_empty() {
            if mode == "full" {
                lines.push(format!("Источники ({}):", source_types.len()));
                for source_type in source_types {
                    lines.push(format!("  {source_type}"));
                }
            } else {
                lines.push(format!("Источники ({})", source_types.len()));
            }
        }
    }
}

pub(crate) fn meta_info_append_http_service(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if let Some(root_url) = props.and_then(|node| meta_info_child_text(node, "RootURL")) {
        if !root_url.is_empty() {
            lines.push(format!("Корневой URL: /{root_url}"));
        }
    }
    let endpoints = meta_info_http_endpoints(child_objs);
    if endpoints.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Шаблоны URL ({}):", endpoints.len()));
    for endpoint in endpoints {
        lines.push(format!("  {}", endpoint.template));
        for method in endpoint.methods {
            lines.push(format!(
                "    {:<6} → {}",
                method.http_method, method.handler
            ));
        }
    }
}

pub(crate) fn meta_info_append_web_service(
    lines: &mut Vec<String>,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if let Some(namespace) = props.and_then(|node| meta_info_child_text(node, "Namespace")) {
        if !namespace.is_empty() {
            lines.push(format!("Пространство имён: {namespace}"));
        }
    }
    let operations = meta_info_ws_operations(child_objs);
    if operations.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Операции ({}):", operations.len()));
    for operation in operations {
        lines.push(format!(
            "  {}({}) → {}",
            operation.name, operation.params, operation.return_type
        ));
    }
}

pub(crate) fn meta_info_append_enum_values(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_objs) = child_objs else {
        return;
    };
    let values = meta_info_children(child_objs, "EnumValue")
        .into_iter()
        .filter_map(|value| {
            let props = meta_info_child(value, "Properties")?;
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            let synonym = meta_info_child(props, "Synonym")
                .map(meta_info_ml_text)
                .unwrap_or_default();
            Some((name, synonym))
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("Значения ({}):", values.len()));
    let max_len = values
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(10)
        .max(10)
        + 2;
    for (name, synonym) in values {
        let synonym_text = if !synonym.is_empty() && synonym != name {
            format!("\"{synonym}\"")
        } else {
            String::new()
        };
        lines.push(format!("  {name:<max_len$} {synonym_text}"));
    }
}

pub(crate) fn meta_info_append_attribute_section(
    lines: &mut Vec<String>,
    header: &str,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    child_tag: &str,
    is_dimension: bool,
) {
    let attrs = meta_info_attributes(child_objs, child_tag, is_dimension);
    if attrs.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("{header} ({}):", attrs.len()));
    let sorted_attrs = meta_info_sort_attrs_ref_first(attrs);
    let width = meta_info_max_name_len(&sorted_attrs);
    for attr in sorted_attrs {
        lines.push(meta_info_format_attr_line(&attr, width));
    }
}

pub(crate) fn meta_info_append_tabular_sections(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
    mode: &str,
) {
    let tabular_sections = meta_info_tabular_sections(child_objs);
    if tabular_sections.is_empty() {
        return;
    }
    if mode == "full" {
        for section in tabular_sections {
            lines.push(String::new());
            lines.push(format!(
                "ТЧ {} ({} {}):",
                section.name,
                section.columns.len(),
                meta_info_decline_cols(section.columns.len())
            ));
            if !section.columns.is_empty() {
                let sorted_cols = meta_info_sort_attrs_ref_first(section.columns);
                let width = meta_info_max_name_len(&sorted_cols);
                for column in sorted_cols {
                    lines.push(meta_info_format_attr_line(&column, width));
                }
            }
        }
    } else {
        lines.push(String::new());
        let parts = tabular_sections
            .iter()
            .map(|section| format!("{}({})", section.name, section.columns.len()))
            .collect::<Vec<_>>();
        lines.push(format!(
            "ТЧ ({}): {}",
            tabular_sections.len(),
            parts.join(", ")
        ));
    }
}

pub(crate) fn meta_info_append_simple_children(
    lines: &mut Vec<String>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    let forms = meta_info_simple_children(child_objs, "Form");
    if !forms.is_empty() {
        lines.push(format!("Формы: {}", forms.join(", ")));
    }
    let templates = meta_info_simple_children(child_objs, "Template");
    if !templates.is_empty() {
        lines.push(format!("Макеты: {}", templates.join(", ")));
    }
    let commands = meta_info_simple_children(child_objs, "Command");
    if !commands.is_empty() {
        lines.push(format!("Команды: {}", commands.join(", ")));
    }
}

pub(crate) fn meta_info_append_full_tail(
    lines: &mut Vec<String>,
    md_type: &str,
    props: Option<roxmltree::Node<'_, '_>>,
    child_objs: Option<roxmltree::Node<'_, '_>>,
) {
    if md_type == "Document" {
        let Some(props) = props else {
            return;
        };
        let register_records = meta_info_child(props, "RegisterRecords")
            .map(|node| {
                meta_info_children(node, "Item")
                    .into_iter()
                    .map(|item| {
                        let raw = meta_info_inner_text(item);
                        if let Some((prefix, name)) = raw.split_once('.') {
                            format!("{}.{}", meta_info_register_short(prefix), name)
                        } else {
                            raw
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !register_records.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Движения ({}): {}",
                register_records.len(),
                register_records.join(", ")
            ));
        }
        let based_on = meta_info_child(props, "BasedOn")
            .map(|node| {
                meta_info_children(node, "Item")
                    .into_iter()
                    .map(|item| {
                        let raw = meta_info_inner_text(item);
                        raw.split_once('.')
                            .map(|(_, name)| name.to_string())
                            .unwrap_or(raw)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !based_on.is_empty() {
            lines.push(format!("Ввод на основании: {}", based_on.join(", ")));
        }
    }
    meta_info_append_simple_children(lines, child_objs);
}

pub(crate) fn meta_info_attributes<'a, 'input>(
    parent_node: Option<roxmltree::Node<'a, 'input>>,
    child_tag: &str,
    is_dimension: bool,
) -> Vec<MetaInfoAttr<'a, 'input>> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, child_tag)
        .into_iter()
        .filter_map(|attr| {
            let props = meta_info_child(attr, "Properties")?;
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            let type_name = meta_info_child(props, "Type")
                .map(meta_info_format_type)
                .unwrap_or_default();
            let flags = meta_info_format_flags(props, is_dimension);
            Some(MetaInfoAttr {
                name,
                type_name,
                flags,
                _marker: std::marker::PhantomData,
            })
        })
        .collect()
}

pub(crate) fn meta_info_tabular_sections<'a, 'input>(
    parent_node: Option<roxmltree::Node<'a, 'input>>,
) -> Vec<MetaInfoTabularSection<'a, 'input>> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, "TabularSection")
        .into_iter()
        .map(|section| {
            let props = meta_info_child(section, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_default();
            let columns =
                meta_info_attributes(meta_info_child(section, "ChildObjects"), "Attribute", false);
            MetaInfoTabularSection { name, columns }
        })
        .collect()
}

pub(crate) fn meta_info_http_endpoints(
    child_objs: Option<roxmltree::Node<'_, '_>>,
) -> Vec<MetaInfoHttpEndpoint> {
    let Some(child_objs) = child_objs else {
        return Vec::new();
    };
    meta_info_children(child_objs, "URLTemplate")
        .into_iter()
        .map(|template| {
            let props = meta_info_child(template, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_default();
            let template_path = props
                .and_then(|node| meta_info_child_text(node, "Template"))
                .unwrap_or_default();
            let methods = meta_info_child(template, "ChildObjects")
                .map(|node| {
                    meta_info_children(node, "Method")
                        .into_iter()
                        .map(|method| {
                            let props = meta_info_child(method, "Properties");
                            MetaInfoHttpMethod {
                                http_method: props
                                    .and_then(|node| meta_info_child_text(node, "HTTPMethod"))
                                    .unwrap_or_default(),
                                handler: props
                                    .and_then(|node| meta_info_child_text(node, "Handler"))
                                    .unwrap_or_default(),
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            MetaInfoHttpEndpoint {
                name,
                template: template_path,
                methods,
            }
        })
        .collect()
}

pub(crate) fn meta_info_ws_operations(
    child_objs: Option<roxmltree::Node<'_, '_>>,
) -> Vec<MetaInfoWsOperation> {
    let Some(child_objs) = child_objs else {
        return Vec::new();
    };
    meta_info_children(child_objs, "Operation")
        .into_iter()
        .map(|operation| {
            let props = meta_info_child(operation, "Properties");
            let params = meta_info_child(operation, "ChildObjects")
                .map(|node| {
                    meta_info_children(node, "Parameter")
                        .into_iter()
                        .map(|param| {
                            let props = meta_info_child(param, "Properties");
                            let name = props
                                .and_then(|node| meta_info_child_text(node, "Name"))
                                .unwrap_or_default();
                            let type_name = props
                                .and_then(|node| meta_info_child_text(node, "XDTOValueType"))
                                .filter(|value| !value.is_empty())
                                .unwrap_or_else(|| "?".to_string());
                            let direction = props
                                .and_then(|node| meta_info_child_text(node, "TransferDirection"))
                                .filter(|value| value != "In")
                                .map(|value| format!(" [{}]", value.to_lowercase()))
                                .unwrap_or_default();
                            format!("{name}: {type_name}{direction}")
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
                .join(", ");
            let return_type = props
                .and_then(|node| meta_info_child_text(node, "XDTOReturningValueType"))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "void".to_string());
            MetaInfoWsOperation {
                name: props
                    .and_then(|node| meta_info_child_text(node, "Name"))
                    .unwrap_or_default(),
                params,
                return_type,
                proc_name: props
                    .and_then(|node| meta_info_child_text(node, "ProcedureName"))
                    .unwrap_or_default(),
            }
        })
        .collect()
}

pub(crate) fn meta_info_common_module_flags(props: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(props) = props else {
        return Vec::new();
    };
    let mut flags = Vec::new();
    for (flag_name, flag_label) in [
        ("Global", "Глобальный"),
        ("Server", "Сервер"),
        ("ServerCall", "Вызов сервера"),
        ("ClientManagedApplication", "Клиент управляемое"),
        ("ClientOrdinaryApplication", "Обычный клиент"),
        ("ExternalConnection", "Внешнее соединение"),
        ("Privileged", "Привилегированный"),
    ] {
        if meta_info_child_text(props, flag_name).as_deref() == Some("true") {
            flags.push(flag_label.to_string());
        }
    }
    if let Some(reuse) =
        meta_info_child_text(props, "ReturnValuesReuse").filter(|value| value != "DontUse")
    {
        flags.push(format!(
            "Повторное использование: {}",
            meta_info_reuse_ru(&reuse)
        ));
    }
    flags
}

pub(crate) fn meta_info_format_type(type_node: roxmltree::Node<'_, '_>) -> String {
    let mut types = Vec::new();
    for type_item in meta_info_children(type_node, "Type") {
        types.push(meta_info_format_single_type(
            meta_info_inner_text(type_item),
            type_node,
        ));
    }
    for type_set in meta_info_children(type_node, "TypeSet") {
        let raw = meta_info_inner_text(type_set);
        if let Some(name) = raw.strip_prefix("cfg:DefinedType.") {
            types.push(format!("ОпределяемыйТип.{name}"));
        } else if let Some(name) = raw.strip_prefix("cfg:Characteristic.") {
            types.push(format!("Характеристика.{name}"));
        } else {
            types.push(raw);
        }
    }
    types.join(" | ")
}

pub(crate) fn meta_info_format_single_type(
    raw: String,
    parent_node: roxmltree::Node<'_, '_>,
) -> String {
    match raw.as_str() {
        "xs:string" => {
            let length = meta_info_child(parent_node, "StringQualifiers")
                .and_then(|node| meta_info_child_text(node, "Length"))
                .unwrap_or_default();
            if length.is_empty() {
                "Строка".to_string()
            } else {
                format!("Строка({length})")
            }
        }
        "xs:decimal" => {
            let qualifiers = meta_info_child(parent_node, "NumberQualifiers");
            let digits = qualifiers
                .and_then(|node| meta_info_child_text(node, "Digits"))
                .unwrap_or_default();
            let fraction = qualifiers
                .and_then(|node| meta_info_child_text(node, "FractionDigits"))
                .unwrap_or_else(|| "0".to_string());
            if digits.is_empty() {
                "Число".to_string()
            } else {
                format!("Число({digits},{fraction})")
            }
        }
        "xs:boolean" => "Булево".to_string(),
        "xs:dateTime" => {
            let date_fraction = meta_info_child(parent_node, "DateQualifiers")
                .and_then(|node| meta_info_child_text(node, "DateFractions"));
            match date_fraction.as_deref() {
                Some("Date") => "Дата".to_string(),
                Some("Time") => "Время".to_string(),
                Some("DateTime") => "ДатаВремя".to_string(),
                Some(_) => "Дата".to_string(),
                None => "ДатаВремя".to_string(),
            }
        }
        "v8:ValueStorage" => "ХранилищеЗначения".to_string(),
        "v8:UUID" => "УникальныйИдентификатор".to_string(),
        "v8:Null" => "Null".to_string(),
        _ => meta_info_format_cfg_type(&raw),
    }
}

pub(crate) fn meta_info_format_cfg_type(raw: &str) -> String {
    let normalized = meta_info_normalize_cfg_prefix(raw);
    if let Some(rest) = normalized.strip_prefix("cfg:") {
        if let Some((prefix, name)) = rest.split_once('.') {
            if let Some(ref_type) = meta_info_ref_type_ru(prefix) {
                return format!("{ref_type}.{name}");
            }
            if prefix == "Characteristic" {
                return format!("Характеристика.{name}");
            }
            if prefix == "DefinedType" {
                return format!("ОпределяемыйТип.{name}");
            }
        }
        return rest.to_string();
    }
    normalized
}

pub(crate) fn meta_info_format_flags(props: roxmltree::Node<'_, '_>, is_dimension: bool) -> String {
    let mut flags = Vec::new();
    if meta_info_child_text(props, "FillChecking").as_deref() == Some("ShowError") {
        flags.push("обязательный");
    }
    if let Some(indexing) = meta_info_child_text(props, "Indexing") {
        match indexing.as_str() {
            "Index" => flags.push("индекс"),
            "IndexWithAdditionalOrder" => flags.push("индекс+доп"),
            _ => {}
        }
    }
    if is_dimension && meta_info_child_text(props, "Master").as_deref() == Some("true") {
        flags.push("ведущее");
    }
    if meta_info_child_text(props, "MultiLine").as_deref() == Some("true") {
        flags.push("многострочный");
    }
    if let Some(use_value) = meta_info_child_text(props, "Use") {
        match use_value.as_str() {
            "ForFolder" => flags.push("для папок"),
            "ForFolderAndItem" => flags.push("для папок и элементов"),
            _ => {}
        }
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!("  [{}]", flags.join(", "))
    }
}

pub(crate) fn meta_info_sort_attrs_ref_first<'a, 'input>(
    attrs: Vec<MetaInfoAttr<'a, 'input>>,
) -> Vec<MetaInfoAttr<'a, 'input>> {
    let mut refs = Vec::new();
    let mut prims = Vec::new();
    for attr in attrs {
        if meta_info_type_is_reference(&attr.type_name) {
            refs.push(attr);
        } else {
            prims.push(attr);
        }
    }
    refs.extend(prims);
    refs
}

pub(crate) fn meta_info_type_is_reference(type_name: &str) -> bool {
    type_name.contains("Ссылка.")
        || type_name.contains("Характеристика.")
        || type_name.contains("ОпределяемыйТип.")
        || type_name.contains("ПланСчетовСсылка")
        || type_name.contains("ПВХСсылка")
        || type_name.contains("ПВРСсылка")
}

pub(crate) fn meta_info_format_attr_line(attr: &MetaInfoAttr<'_, '_>, width: usize) -> String {
    format!("  {:<width$} {}{}", attr.name, attr.type_name, attr.flags)
}

pub(crate) fn meta_info_max_name_len(attrs: &[MetaInfoAttr<'_, '_>]) -> usize {
    let max_len = attrs
        .iter()
        .map(|attr| attr.name.chars().count())
        .max()
        .unwrap_or(10)
        .max(10);
    (max_len + 2).min(40)
}

pub(crate) fn meta_info_simple_children(
    parent_node: Option<roxmltree::Node<'_, '_>>,
    tag: &str,
) -> Vec<String> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, tag)
        .into_iter()
        .map(meta_info_inner_text)
        .collect()
}

pub(crate) fn meta_info_enum_values(parent_node: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, "EnumValue")
        .into_iter()
        .filter_map(|value| {
            meta_info_child(value, "Properties")
                .and_then(|props| meta_info_child_text(props, "Name"))
        })
        .collect()
}

pub(crate) fn meta_info_paginate(lines: Vec<String>, args: &Map<String, Value>) -> String {
    let total_lines = lines.len();
    let offset = int_arg(args, &["offset", "Offset"]).unwrap_or(0).max(0) as usize;
    let limit = int_arg(args, &["limit", "Limit"]).unwrap_or(150).max(0) as usize;
    if offset >= total_lines && offset > 0 {
        return format!(
            "[INFO] Offset {offset} exceeds total lines ({total_lines}). Nothing to show."
        );
    }
    let mut out_lines = if offset > 0 {
        lines.into_iter().skip(offset).collect::<Vec<_>>()
    } else {
        lines
    };
    if limit > 0 && out_lines.len() > limit {
        let mut shown = out_lines.drain(..limit).collect::<Vec<_>>();
        shown.push(String::new());
        shown.push(format!(
            "[ОБРЕЗАНО] Показано {limit} из {total_lines} строк. Используйте -Offset {} для продолжения.",
            offset + limit
        ));
        out_lines = shown;
    }
    out_lines.join("\n")
}

pub(crate) fn meta_info_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| role_info_element(*child, local_name, None))
}

pub(crate) fn meta_info_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| role_info_element(*child, local_name, None))
        .collect()
}

pub(crate) fn meta_info_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
) -> Option<String> {
    meta_info_child(node, local_name).map(meta_info_inner_text)
}

pub(crate) fn meta_info_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").to_string()
}

pub(crate) fn meta_info_ml_text(node: roxmltree::Node<'_, '_>) -> String {
    let value = multilang_text(node);
    if value.is_empty() {
        node.text().unwrap_or("").trim().to_string()
    } else {
        value
    }
}

pub(crate) fn meta_info_ml_child_text(
    node: Option<roxmltree::Node<'_, '_>>,
    local_name: &str,
) -> Option<String> {
    node.and_then(|node| meta_info_child(node, local_name))
        .map(meta_info_ml_text)
}

pub(crate) fn meta_info_attr_by_local<'a>(
    node: roxmltree::Node<'a, '_>,
    local_name: &str,
) -> Option<&'a str> {
    node.attributes()
        .find(|attr| attr.name() == local_name)
        .map(|attr| attr.value())
}

pub(crate) fn meta_info_normalize_cfg_prefix(raw: &str) -> String {
    let Some((prefix, rest)) = raw.split_once(':') else {
        return raw.to_string();
    };
    if prefix.starts_with('d')
        && prefix[1..]
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == 'p')
    {
        format!("cfg:{rest}")
    } else {
        raw.to_string()
    }
}

pub(crate) fn meta_info_format_source_type(raw: &str) -> String {
    let normalized = meta_info_normalize_cfg_prefix(raw);
    let Some(rest) = normalized.strip_prefix("cfg:") else {
        return normalized;
    };
    let Some((prefix, name)) = rest.split_once('.') else {
        return rest.to_string();
    };
    if let Some(object_type) = meta_info_object_type_ru(prefix) {
        format!("{object_type}.{name}")
    } else {
        rest.to_string()
    }
}

pub(crate) fn meta_info_type_ru(md_type: &str) -> String {
    match md_type {
        "Catalog" => "Справочник",
        "Document" => "Документ",
        "Enum" => "Перечисление",
        "Constant" => "Константа",
        "InformationRegister" => "Регистр сведений",
        "AccumulationRegister" => "Регистр накопления",
        "AccountingRegister" => "Регистр бухгалтерии",
        "CalculationRegister" => "Регистр расчёта",
        "ChartOfAccounts" => "План счетов",
        "ChartOfCharacteristicTypes" => "План видов характеристик",
        "ChartOfCalculationTypes" => "План видов расчёта",
        "BusinessProcess" => "Бизнес-процесс",
        "Task" => "Задача",
        "ExchangePlan" => "План обмена",
        "DocumentJournal" => "Журнал документов",
        "Report" => "Отчёт",
        "DataProcessor" => "Обработка",
        "DefinedType" => "Определяемый тип",
        "CommonModule" => "Общий модуль",
        "ScheduledJob" => "Регламентное задание",
        "EventSubscription" => "Подписка на событие",
        "HTTPService" => "HTTP-сервис",
        "WebService" => "Веб-сервис",
        _ => md_type,
    }
    .to_string()
}

pub(crate) fn meta_info_is_reference_metadata_type(md_type: &str) -> bool {
    matches!(
        md_type,
        "Catalog"
            | "Document"
            | "Enum"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
    )
}

pub(crate) fn meta_info_ref_type_ru(prefix: &str) -> Option<&'static str> {
    match prefix {
        "CatalogRef" => Some("СправочникСсылка"),
        "DocumentRef" => Some("ДокументСсылка"),
        "EnumRef" => Some("ПеречислениеСсылка"),
        "ChartOfAccountsRef" => Some("ПланСчетовСсылка"),
        "ChartOfCharacteristicTypesRef" => Some("ПВХСсылка"),
        "ChartOfCalculationTypesRef" => Some("ПВРСсылка"),
        "ExchangePlanRef" => Some("ПланОбменаСсылка"),
        "BusinessProcessRef" => Some("БизнесПроцессСсылка"),
        "TaskRef" => Some("ЗадачаСсылка"),
        _ => None,
    }
}

pub(crate) fn meta_info_object_type_ru(prefix: &str) -> Option<&'static str> {
    match prefix {
        "CatalogObject" => Some("СправочникОбъект"),
        "DocumentObject" => Some("ДокументОбъект"),
        "ChartOfAccountsObject" => Some("ПланСчетовОбъект"),
        "ChartOfCharacteristicTypesObject" => Some("ПВХОбъект"),
        "BusinessProcessObject" => Some("БизнесПроцессОбъект"),
        "TaskObject" => Some("ЗадачаОбъект"),
        "ExchangePlanObject" => Some("ПланОбменаОбъект"),
        "InformationRegisterRecordSet" => Some("НаборЗаписейРС"),
        "AccumulationRegisterRecordSet" => Some("НаборЗаписейРН"),
        "AccountingRegisterRecordSet" => Some("НаборЗаписейРБ"),
        _ => None,
    }
}

pub(crate) fn meta_info_period_ru(value: &str) -> &str {
    match value {
        "Nonperiodical" => "Непериодический",
        "Day" => "День",
        "Month" => "Месяц",
        "Quarter" => "Квартал",
        "Year" => "Год",
        "Second" => "Секунда",
        _ => value,
    }
}

pub(crate) fn meta_info_write_mode_ru(value: &str) -> &str {
    match value {
        "Independent" => "независимая",
        "RecorderSubordinate" => "подчинение регистратору",
        _ => value,
    }
}

pub(crate) fn meta_info_reuse_ru(value: &str) -> &str {
    match value {
        "DontUse" => "нет",
        "DuringRequest" => "на время вызова",
        "DuringSession" => "на время сеанса",
        _ => value,
    }
}

pub(crate) fn meta_info_event_ru(value: &str) -> &str {
    match value {
        "BeforeWrite" => "ПередЗаписью",
        "OnWrite" => "ПриЗаписи",
        "AfterWrite" => "ПослеЗаписи",
        "BeforeDelete" => "ПередУдалением",
        "Posting" => "ОбработкаПроведения",
        "UndoPosting" => "ОбработкаУдаленияПроведения",
        "OnReadAtServer" => "ПриЧтенииНаСервере",
        "FillCheckProcessing" => "ОбработкаПроверкиЗаполнения",
        _ => value,
    }
}

pub(crate) fn meta_info_number_period_ru(value: &str) -> &str {
    match value {
        "Year" => "по году",
        "Quarter" => "по кварталу",
        "Month" => "по месяцу",
        "Day" => "по дню",
        "WholeCatalog" => "сквозная",
        _ => value,
    }
}

pub(crate) fn meta_info_register_short(value: &str) -> &str {
    match value {
        "AccumulationRegister" => "РН",
        "AccountingRegister" => "РБ",
        "CalculationRegister" => "РР",
        "InformationRegister" => "РС",
        _ => value,
    }
}

pub(crate) fn meta_info_decline_cols(n: usize) -> &'static str {
    let m = n % 10;
    let h = n % 100;
    if (11..=19).contains(&h) {
        "колонок"
    } else if m == 1 {
        "колонка"
    } else if (2..=4).contains(&m) {
        "колонки"
    } else {
        "колонок"
    }
}

pub(crate) struct MetaRemoveError {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) message: String,
}

struct MetaRemoveSuccess {
    stdout: String,
    changes: Vec<String>,
    artifacts: Vec<String>,
    warnings: Vec<String>,
}

pub(crate) fn meta_remove_stdout_error(message: String) -> MetaRemoveError {
    MetaRemoveError {
        stdout: format!("{message}\n"),
        stderr: String::new(),
        message,
    }
}

fn validate_meta_remove_object_name(name: &str) -> Result<(), String> {
    let mut components = Path::new(name).components();
    let is_single_path_component = matches!(
        components.next(),
        Some(std::path::Component::Normal(component))
            if component == std::ffi::OsStr::new(name)
    ) && components.next().is_none();

    if form_is_xml_ncname(name) && is_single_path_component {
        Ok(())
    } else {
        Err(format!(
            "Object name must be a valid Unicode XML NCName and a single path component: {name:?}"
        ))
    }
}

struct MetaRemoveSubsystemReplacement {
    path: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
    subsystem_name: String,
    removed_references: usize,
}

struct MetaRemoveTextRead {
    path: PathBuf,
    raw: Vec<u8>,
    text: String,
}

struct MetaRemoveReferenceScan {
    references: Vec<MetaRemoveReference>,
    reads: Vec<MetaRemoveTextRead>,
    directory_reads: Vec<MetaRemoveDirectoryRead>,
}

struct MetaRemoveDirectoryRead {
    path: PathBuf,
    direct_entries: Vec<DirectoryTopologyEntry>,
}

struct MetaRemoveTraversal {
    files: Vec<PathBuf>,
    directories: Vec<MetaRemoveDirectoryRead>,
}

const META_REMOVE_MAX_TRAVERSAL_DEPTH: usize = 256;
const META_REMOVE_MAX_TRAVERSAL_ENTRIES: usize = 1_000_000;

#[derive(Clone, Copy)]
struct MetaRemoveTraversalLimits {
    max_depth: usize,
    max_entries: usize,
}

fn remove_subsystem_content_items(
    xml_text: &str,
    qualified_object_name: &str,
) -> Result<(String, usize), String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let content = document
        .descendants()
        .find(|node| role_info_element(*node, "Content", Some(MD_NS)));
    let Some(content) = content else {
        return Ok((xml_text.to_string(), 0));
    };
    let mut ranges = content
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Item")
        .filter(|node| {
            node.text()
                .is_some_and(|text| text.trim() == qualified_object_name)
        })
        .map(|node| node.range())
        .collect::<Vec<_>>();
    if ranges.is_empty() {
        return Ok((xml_text.to_string(), 0));
    }
    ranges.sort_by_key(|range| range.start);
    let removed = ranges.len();
    let mut updated = xml_text.to_string();
    for range in ranges.into_iter().rev() {
        let line_start = updated[..range.start]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let leading_is_indent = updated[line_start..range.start]
            .chars()
            .all(|character| matches!(character, ' ' | '\t' | '\r'));
        let removal = if leading_is_indent && updated[range.end..].starts_with("\r\n") {
            line_start..range.end + 2
        } else if leading_is_indent && updated[range.end..].starts_with('\n') {
            line_start..range.end + 1
        } else {
            range
        };
        updated.replace_range(removal, "");
    }
    Ok((updated, removed))
}

fn meta_remove_path_is_link_or_reparse_point(_path: &Path, metadata: &fs::Metadata) -> bool {
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(metadata) {
        return true;
    }
    #[cfg(test)]
    {
        META_REMOVE_FORCED_REPARSE_PATHS.with(|slot| slot.borrow().contains(_path))
    }
    #[cfg(not(test))]
    false
}

fn require_meta_remove_real_path(
    path: &Path,
    metadata: &fs::Metadata,
    role: &str,
) -> Result<(), String> {
    if meta_remove_path_is_link_or_reparse_point(path, metadata) {
        Err(format!(
            "{role} must not be a symbolic link or reparse point: {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

fn plan_meta_remove_subsystem_replacements(
    dir: &Path,
    qualified_object_name: &str,
    replacements: &mut Vec<MetaRemoveSubsystemReplacement>,
    descriptor_reads: &mut Vec<MetaRemoveTextRead>,
) -> Result<(), String> {
    let mut visited_directories = HashSet::new();
    let mut visited_entries = 0usize;
    plan_meta_remove_subsystem_replacements_bounded(
        dir,
        qualified_object_name,
        replacements,
        descriptor_reads,
        0,
        MetaRemoveTraversalLimits {
            max_depth: META_REMOVE_MAX_TRAVERSAL_DEPTH,
            max_entries: META_REMOVE_MAX_TRAVERSAL_ENTRIES,
        },
        &mut visited_directories,
        &mut visited_entries,
    )
}

#[allow(clippy::too_many_arguments)]
fn plan_meta_remove_subsystem_replacements_bounded(
    dir: &Path,
    qualified_object_name: &str,
    replacements: &mut Vec<MetaRemoveSubsystemReplacement>,
    descriptor_reads: &mut Vec<MetaRemoveTextRead>,
    depth: usize,
    limits: MetaRemoveTraversalLimits,
    visited_directories: &mut HashSet<PathBuf>,
    visited_entries: &mut usize,
) -> Result<(), String> {
    if depth > limits.max_depth {
        return Err(format!(
            "subsystem traversal exceeded the maximum depth of {}: {}",
            limits.max_depth,
            dir.display()
        ));
    }
    let dir_metadata = fs::symlink_metadata(dir)
        .map_err(|err| format!("failed to inspect {}: {err}", dir.display()))?;
    require_meta_remove_real_path(dir, &dir_metadata, "subsystem directory")?;
    if !dir_metadata.is_dir() {
        return Err(format!(
            "subsystem path is not a directory: {}",
            dir.display()
        ));
    }

    let directory_identity = fs::canonicalize(dir).map_err(|err| {
        format!(
            "failed to resolve subsystem directory identity {}: {err}",
            dir.display()
        )
    })?;
    if !visited_directories.insert(directory_identity) {
        return Err(format!(
            "subsystem traversal directory cycle or duplicate identity detected before traversal: {}",
            dir.display()
        ));
    }

    let directory_entries =
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?;
    let mut entries = Vec::new();
    for entry in directory_entries {
        let entry =
            entry.map_err(|err| format!("failed to read an entry in {}: {err}", dir.display()))?;
        if *visited_entries >= limits.max_entries {
            return Err(format!(
                "subsystem traversal exceeded the maximum of {} entries: {}",
                limits.max_entries,
                dir.display()
            ));
        }
        *visited_entries += 1;
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());

    let mut descriptors = Vec::new();
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| format!("failed to inspect {}: {err}", path.display()))?;
        require_meta_remove_real_path(&path, &metadata, "subsystem entry")?;
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(format!(
                "subsystem entry has an unsupported filesystem type: {}",
                path.display()
            ));
        }
        let is_xml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"));
        if is_xml && metadata.is_file() {
            descriptors.push(path);
        }
    }

    for path in descriptors {
        let snapshot = read_utf8_sig_snapshot(&path)?;
        let subsystem_name = first_tag_text_in_xml(&snapshot.text, "Name")
            .unwrap_or_else(|| file_stem_string(&path));
        let (updated, removed_references) =
            remove_subsystem_content_items(&snapshot.text, qualified_object_name)?;
        descriptor_reads.push(MetaRemoveTextRead {
            path: path.clone(),
            raw: snapshot.raw.clone(),
            text: snapshot.text.clone(),
        });

        let child_dir = path
            .parent()
            .unwrap_or(dir)
            .join(file_stem_string(&path))
            .join("Subsystems");
        if removed_references > 0 {
            replacements.push(MetaRemoveSubsystemReplacement {
                path: path.clone(),
                original: snapshot.raw,
                replacement: utf8_bom_bytes(&ensure_trailing_newline(updated)),
                subsystem_name,
                removed_references,
            });
        }

        #[cfg(test)]
        run_before_meta_remove_subsystem_child_inspection_hook(&child_dir);
        match fs::symlink_metadata(&child_dir) {
            Ok(metadata) => {
                require_meta_remove_real_path(&child_dir, &metadata, "subsystem directory")?;
                if metadata.is_dir() {
                    if depth >= limits.max_depth {
                        return Err(format!(
                            "subsystem traversal exceeded the maximum depth of {}: {}",
                            limits.max_depth,
                            child_dir.display()
                        ));
                    }
                    plan_meta_remove_subsystem_replacements_bounded(
                        &child_dir,
                        qualified_object_name,
                        replacements,
                        descriptor_reads,
                        depth + 1,
                        limits,
                        visited_directories,
                        visited_entries,
                    )?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!("failed to inspect {}: {err}", child_dir.display()));
            }
        }
    }

    Ok(())
}

pub(crate) fn meta_remove_subsystem_dependency_paths(
    dir: &Path,
    qualified_object_name: &str,
) -> Result<Vec<PathBuf>, String> {
    let mut replacements = Vec::new();
    let mut descriptor_reads = Vec::new();
    plan_meta_remove_subsystem_replacements(
        dir,
        qualified_object_name,
        &mut replacements,
        &mut descriptor_reads,
    )?;
    let descriptors = replacements
        .into_iter()
        .map(|replacement| replacement.path)
        .collect::<Vec<_>>();
    let descriptor_refs = descriptors.iter().map(PathBuf::as_path).collect::<Vec<_>>();
    let mut dependencies = descriptor_reads
        .into_iter()
        .map(|read| read.path)
        .collect::<Vec<_>>();
    dependencies.extend(subsystem_validation_format_dependency_paths(
        &descriptor_refs,
    ));
    dependencies.sort();
    dependencies.dedup();
    Ok(dependencies)
}

fn validate_meta_remove_post_state(
    config_xml: &Path,
    obj_type: &str,
    obj_name: &str,
    subsystem_paths: &[PathBuf],
    removed_paths: &[PathBuf],
    paired_paths: Option<(&Path, &Path)>,
) -> Result<(), String> {
    let config_text = read_utf8_sig(config_xml)?;
    if remove_metadata_child_text_with_flag(&config_text, obj_type, obj_name).1 {
        return Err(format!(
            "post-write validation found <{obj_type}>{obj_name}</{obj_type}> in {}",
            config_xml.display()
        ));
    }

    let qualified_object_name = format!("{obj_type}.{obj_name}");
    for path in subsystem_paths {
        let text = read_utf8_sig(path)?;
        if remove_subsystem_content_items(&text, &qualified_object_name)?.1 > 0 {
            return Err(format!(
                "post-write validation found {qualified_object_name} in {}",
                path.display()
            ));
        }
    }

    for path in removed_paths {
        match fs::symlink_metadata(path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(format!(
                    "post-write validation found removal target still present: {}",
                    path.display()
                ));
            }
            Err(err) => {
                return Err(format!(
                    "post-write validation failed to inspect {}: {err}",
                    path.display()
                ));
            }
        }
    }

    if let Some((descriptor, payload)) = paired_paths {
        for path in [descriptor, payload] {
            match fs::symlink_metadata(path) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(format!(
                        "post-write validation found removed metadata pair member still present: {}",
                        path.display()
                    ));
                }
                Err(err) => {
                    return Err(format!(
                        "post-write validation failed to inspect metadata pair member {}: {err}",
                        path.display()
                    ));
                }
            }
        }
    }

    Ok(())
}

fn meta_remove_descriptor_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "failed to inspect metadata descriptor {}: {error}",
            path.display()
        )),
        Ok(metadata)
            if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(
                &metadata,
            ) =>
        {
            Err(format!(
                "metadata descriptor must not be a symbolic link or reparse point: {}",
                path.display()
            ))
        }
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(format!(
            "metadata descriptor is not a regular file: {}",
            path.display()
        )),
    }
}

fn meta_remove_payload_file_count(path: &Path) -> Result<Option<usize>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to inspect metadata payload directory {}: {error}",
            path.display()
        )),
        Ok(metadata)
            if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(
                &metadata,
            ) =>
        {
            Err(format!(
                "metadata payload directory must not be a symbolic link or reparse point: {}",
                path.display()
            ))
        }
        Ok(metadata) if metadata.is_dir() => Ok(Some(metadata_files_recursive(path)?.files.len())),
        Ok(_) => Err(format!(
            "metadata payload path is not a directory: {}",
            path.display()
        )),
    }
}

pub(crate) fn remove_metadata_object(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<MetaRemoveSuccess, MetaRemoveError> {
        let config_dir_raw = required_string(args, &["configDir", "ConfigDir"], "ConfigDir")
            .map_err(|err| meta_remove_stdout_error(format!("[ERROR] {err}")))?;
        let object = required_string(args, &["object", "Object"], "Object")
            .map_err(|err| meta_remove_stdout_error(format!("[ERROR] {err}")))?;

        let Some((obj_type, obj_name)) = object.split_once('.') else {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Invalid object format '{object}'. Expected: Type.Name (e.g. Catalog.Товары)"
            )));
        };
        if obj_type.is_empty() || obj_name.is_empty() {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Invalid object format '{object}'. Expected: Type.Name (e.g. Catalog.Товары)"
            )));
        }
        validate_meta_remove_object_name(obj_name)
            .map_err(|err| meta_remove_stdout_error(format!("[ERROR] {err}")))?;
        let Some(type_plural) = meta_remove_type_plural(obj_type) else {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Unknown type '{obj_type}'. Supported: {}",
                meta_remove_supported_types().join(", ")
            )));
        };

        let config_dir_display = PathBuf::from(config_dir_raw);
        let config_dir = absolutize(config_dir_display.clone(), &context.cwd);
        if !config_dir.is_dir() {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Config directory not found: {}",
                config_dir.display()
            )));
        }

        let config_xml = config_dir.join("Configuration.xml");
        if !config_xml.is_file() {
            return Err(meta_remove_stdout_error(format!(
                "[ERROR] Configuration.xml not found in: {}",
                config_dir.display()
            )));
        }
        require_meta_configuration_owner_validation(&config_xml, context, "meta.remove")
            .map_err(meta_remove_stdout_error)?;

        let dry_run = bool_arg(args, &["DryRun"]);
        let keep_files = bool_arg(args, &["KeepFiles", "keepFiles"]);
        let force = bool_arg(args, &["Force", "force"]);

        let type_dir = config_dir.join(type_plural);
        let obj_xml = type_dir.join(format!("{obj_name}.xml"));
        let obj_dir = type_dir.join(obj_name);
        let has_xml = meta_remove_descriptor_exists(&obj_xml).map_err(meta_remove_stdout_error)?;
        let payload_file_count =
            meta_remove_payload_file_count(&obj_dir).map_err(meta_remove_stdout_error)?;
        let has_dir = payload_file_count.is_some();

        let mut stdout = String::new();
        stdout.push_str(&format!("=== meta-remove: {obj_type}.{obj_name} ===\n\n"));
        if dry_run {
            stdout.push_str("[DRY-RUN] No changes will be made\n\n");
        }

        let mut changes = Vec::new();
        let mut artifacts = vec![config_xml.display().to_string()];
        let mut actions = 0usize;
        let config_snapshot =
            read_utf8_sig_snapshot(&config_xml).map_err(meta_remove_stdout_error)?;

        if !has_xml && !has_dir {
            if !remove_metadata_child_text_with_flag(&config_snapshot.text, obj_type, obj_name).1 {
                stdout.push_str(&format!(
                    "[ERROR] Object not found: {type_plural}/{obj_name}.xml and not registered in Configuration.xml\n"
                ));
                return Err(MetaRemoveError {
                    message: stdout.trim().to_string(),
                    stdout,
                    stderr: String::new(),
                });
            }
            stdout.push_str(&format!(
                "[WARN]  Object files not found: {type_plural}/{obj_name}.xml\n"
            ));
            stdout.push_str("        Proceeding with deregistration only...\n");
        } else {
            if has_xml {
                stdout.push_str(&format!("[FOUND] {type_plural}/{obj_name}.xml\n"));
                artifacts.push(obj_xml.display().to_string());
            }
            if has_dir {
                let file_count = payload_file_count
                    .expect("existing metadata payload directory must have a safe file count");
                stdout.push_str(&format!(
                    "[FOUND] {type_plural}/{obj_name}/ ({file_count} files)\n"
                ));
                artifacts.push(obj_dir.display().to_string());
            }
        }

        stdout.push('\n');
        stdout.push_str("--- Reference check ---\n");
        let reference_scan = meta_remove_reference_scan(
            &config_dir,
            obj_type,
            obj_name,
            type_plural,
            &obj_xml,
            &obj_dir,
            has_xml,
            has_dir,
        )
        .map_err(meta_remove_stdout_error)?;
        let references = &reference_scan.references;
        if references.is_empty() {
            stdout.push_str("[OK]    No references found\n");
        } else {
            stdout.push_str(&format!(
                "[WARN]  Found {} reference(s) to {obj_type}.{obj_name}:\n\n",
                references.len()
            ));
            for (index, reference) in references.iter().take(20).enumerate() {
                stdout.push_str(&format!("        {}\n", reference.file));
                stdout.push_str(&format!("          pattern: {}\n", reference.pattern));
                if index == 19 && references.len() > 20 {
                    stdout.push_str(&format!("        ... and {} more\n", references.len() - 20));
                }
            }
            stdout.push('\n');
            if !force {
                stdout.push_str(&format!(
                    "[ERROR] Cannot remove: object has {} reference(s).\n",
                    references.len()
                ));
                stdout.push_str("        Use -Force to remove anyway, or fix references first.\n");
                return Err(MetaRemoveError {
                    message: stdout.trim().to_string(),
                    stdout,
                    stderr: String::new(),
                });
            }
            stdout.push_str("[WARN]  -Force specified, proceeding despite references\n");
        }

        let mut transaction = CompileTransaction::new();

        stdout.push('\n');
        stdout.push_str("--- Configuration.xml ---\n");
        let (next_config_text, removed_from_config) =
            remove_metadata_child_text_with_flag(&config_snapshot.text, obj_type, obj_name);
        let config_replacement = if removed_from_config {
            utf8_bom_bytes(&ensure_trailing_newline(next_config_text))
        } else {
            config_snapshot.raw.clone()
        };
        transaction
            .replace_bytes(config_xml.clone(), &config_snapshot.raw, config_replacement)
            .map_err(meta_remove_stdout_error)?;
        if removed_from_config {
            stdout.push_str(&format!(
                "[OK]    Removed <{obj_type}>{obj_name}</{obj_type}> from ChildObjects\n"
            ));
            actions += 1;
            if !dry_run {
                stdout.push_str("[OK]    Configuration.xml saved\n");
                changes.push(format!("updated {}", config_xml.display()));
            }
        } else {
            stdout.push_str(&format!(
                "[WARN]  <{obj_type}>{obj_name}</{obj_type}> not found in ChildObjects\n"
            ));
        }

        stdout.push('\n');
        stdout.push_str("--- Subsystems ---\n");
        let subsystems_dir = config_dir.join("Subsystems");
        let mut subsystems_cleaned = 0usize;
        let mut subsystem_replacements = Vec::new();
        let mut subsystem_descriptor_reads = Vec::new();
        if subsystems_dir.is_dir() {
            plan_meta_remove_subsystem_replacements(
                &subsystems_dir,
                &format!("{obj_type}.{obj_name}"),
                &mut subsystem_replacements,
                &mut subsystem_descriptor_reads,
            )
            .map_err(meta_remove_stdout_error)?;
            for replacement in &subsystem_replacements {
                validate_metadata_owner_shape_8_3_27(&replacement.path, context, "meta.remove")
                    .map_err(meta_remove_stdout_error)?;
                for _ in 0..replacement.removed_references {
                    stdout.push_str(&format!(
                        "[OK]    Removed from subsystem '{}'\n",
                        replacement.subsystem_name
                    ));
                }
                subsystems_cleaned += replacement.removed_references;
                transaction
                    .replace_bytes(
                        replacement.path.clone(),
                        &replacement.original,
                        replacement.replacement.clone(),
                    )
                    .map_err(meta_remove_stdout_error)?;
                if !dry_run {
                    changes.push(format!("updated {}", replacement.path.display()));
                }
            }
            if subsystems_cleaned == 0 {
                stdout.push_str("[OK]    Not referenced in any subsystem\n");
            }
        } else {
            stdout.push_str("[OK]    No Subsystems directory\n");
        }

        stdout.push('\n');
        stdout.push_str("--- Files ---\n");
        let mut removed_paths = Vec::new();
        if !keep_files {
            let mut type_collection_targets = Vec::new();
            if has_xml {
                type_collection_targets.push(obj_xml.as_path());
            }
            if has_dir {
                type_collection_targets.push(obj_dir.as_path());
            }
            let remove_type_collection = !type_collection_targets.is_empty()
                && transaction
                    .remove_directory_if_only_direct_entries(
                        &type_dir,
                        type_collection_targets
                            .iter()
                            .map(|path| {
                                path.file_name()
                                    .expect("metadata collection target must have a file name")
                                    .to_os_string()
                            })
                            .collect(),
                    )
                    .map_err(meta_remove_stdout_error)?;

            if remove_type_collection {
                removed_paths.push(type_dir.clone());
                if !dry_run {
                    changes.push(format!(
                        "removed empty collection directory {}",
                        type_dir.display()
                    ));
                }
            } else {
                if has_dir {
                    transaction
                        .remove_path(obj_dir.clone())
                        .map_err(meta_remove_stdout_error)?;
                    removed_paths.push(obj_dir.clone());
                } else {
                    transaction
                        .guard_path_absent(obj_dir.clone())
                        .map_err(meta_remove_stdout_error)?;
                }
                if has_xml {
                    transaction
                        .remove_path(obj_xml.clone())
                        .map_err(meta_remove_stdout_error)?;
                    removed_paths.push(obj_xml.clone());
                } else {
                    transaction
                        .guard_path_absent(obj_xml.clone())
                        .map_err(meta_remove_stdout_error)?;
                }
            }

            if has_dir {
                if dry_run {
                    stdout.push_str(&format!(
                        "[DRY]   Would delete directory: {type_plural}/{obj_name}/\n"
                    ));
                } else {
                    stdout.push_str(&format!(
                        "[OK]    Deleted directory: {type_plural}/{obj_name}/\n"
                    ));
                    changes.push(format!("removed directory {}", obj_dir.display()));
                }
                actions += 1;
            }

            if has_xml {
                if dry_run {
                    stdout.push_str(&format!(
                        "[DRY]   Would delete file: {type_plural}/{obj_name}.xml\n"
                    ));
                } else {
                    stdout.push_str(&format!(
                        "[OK]    Deleted file: {type_plural}/{obj_name}.xml\n"
                    ));
                    changes.push(format!("removed file {}", obj_xml.display()));
                }
                actions += 1;
            }

            if !has_xml && !has_dir {
                stdout.push_str("[OK]    No files to delete\n");
            }
        } else {
            stdout.push_str("[SKIP]  File deletion skipped (-KeepFiles)\n");
        }

        let warnings = if dry_run {
            Vec::new()
        } else {
            for read in reference_scan
                .reads
                .iter()
                .chain(subsystem_descriptor_reads.iter())
            {
                transaction
                    .guard_or_verify_exact_preimage(&read.path, &read.raw)
                    .map_err(meta_remove_stdout_error)?;
            }
            let mut dependency_paths = vec![config_xml.clone()];
            dependency_paths.extend(
                reference_scan
                    .reads
                    .iter()
                    .filter(|read| {
                        read.path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
                    })
                    .map(|read| read.path.clone()),
            );
            dependency_paths.extend(
                subsystem_descriptor_reads
                    .iter()
                    .map(|read| read.path.clone()),
            );
            let subsystem_descriptors = subsystem_replacements
                .iter()
                .map(|replacement| replacement.path.as_path())
                .collect::<Vec<_>>();
            let subsystem_format_dependencies =
                subsystem_validation_format_dependency_paths(&subsystem_descriptors);
            dependency_paths.extend(subsystem_format_dependencies);
            dependency_paths.sort();
            dependency_paths.dedup();
            let dependencies = dependency_paths
                .iter()
                .map(PathBuf::as_path)
                .collect::<Vec<_>>();
            let mut trees = Vec::new();
            if has_xml {
                trees.push(obj_xml.as_path());
            }
            if has_dir {
                trees.push(obj_dir.as_path());
            }
            guard_active_format_dependencies_and_xml_trees(
                &mut transaction,
                &dependencies,
                &trees,
                context,
            )
            .map_err(meta_remove_stdout_error)?;
            for directory_read in &reference_scan.directory_reads {
                if removed_paths
                    .iter()
                    .any(|removed| directory_read.path.starts_with(removed))
                {
                    continue;
                }
                transaction
                    .guard_or_verify_directory_topology(
                        directory_read.path.clone(),
                        directory_read.direct_entries.clone(),
                    )
                    .map_err(meta_remove_stdout_error)?;
            }
            let validation_config_xml = config_xml.clone();
            let validation_obj_type = obj_type.to_string();
            let validation_obj_name = obj_name.to_string();
            let validation_subsystem_paths = subsystem_replacements
                .iter()
                .map(|replacement| replacement.path.clone())
                .collect::<Vec<_>>();
            let validation_removed_paths = removed_paths.clone();
            let validation_obj_xml = obj_xml.clone();
            let validation_obj_dir = obj_dir.clone();
            let validate_pair_absent = !keep_files;
            transaction
                .commit_with_post_validation(move || {
                    require_meta_configuration_owner_validation(
                        &validation_config_xml,
                        context,
                        "meta.remove",
                    )?;
                    for path in &validation_subsystem_paths {
                        validate_metadata_owner_shape_8_3_27(path, context, "meta.remove")?;
                    }
                    validate_meta_remove_post_state(
                        &validation_config_xml,
                        &validation_obj_type,
                        &validation_obj_name,
                        &validation_subsystem_paths,
                        &validation_removed_paths,
                        validate_pair_absent.then_some((
                            validation_obj_xml.as_path(),
                            validation_obj_dir.as_path(),
                        )),
                    )
                })
                .map_err(meta_remove_stdout_error)?
                .cleanup_warnings
        };

        stdout.push('\n');
        let total_actions = actions + subsystems_cleaned;
        if dry_run {
            stdout.push_str(&format!(
                "=== Dry run complete: {total_actions} actions would be performed ===\n"
            ));
        } else {
            stdout.push_str(&format!(
                "=== Done: {total_actions} actions performed ({subsystems_cleaned} subsystem references removed) ===\n"
            ));
        }

        Ok(MetaRemoveSuccess {
            stdout,
            changes,
            artifacts,
            warnings,
        })
    })();

    match result {
        Ok(success) => AdapterOutcome {
            ok: true,
            summary: "unica.meta.remove completed with native metadata remover".to_string(),
            changes: success.changes,
            warnings: success.warnings,
            errors: Vec::new(),
            artifacts: success.artifacts,
            stdout: Some(success.stdout),
            stderr: Some(String::new()),
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.remove failed in native metadata remover".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: if error.message.is_empty() {
                Vec::new()
            } else {
                vec![error.message]
            },
            artifacts: Vec::new(),
            stdout: Some(error.stdout),
            stderr: Some(error.stderr),
            command: None,
        },
    }
}

pub(crate) fn remove_metadata_child_text_lxml(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> String {
    let plain = format!("<{local_name}>{item_name}</{local_name}>");
    let prefixed = format!("<md:{local_name}>{item_name}</md:{local_name}>");
    for (open, target) in [
        ("<ChildObjects>", plain.as_str()),
        ("<md:ChildObjects>", prefixed.as_str()),
    ] {
        let Some(open_idx) = xml_text.find(open) else {
            continue;
        };
        let after_open = open_idx + open.len();
        if !xml_text[after_open..].starts_with('\n') {
            continue;
        }
        let child_indent_start = after_open + 1;
        let child_start = child_indent_start
            + xml_text[child_indent_start..]
                .chars()
                .take_while(|ch| *ch == '\t' || *ch == ' ')
                .map(char::len_utf8)
                .sum::<usize>();
        if !xml_text[child_start..].starts_with(target) {
            continue;
        }
        let after_child = child_start + target.len();
        if !xml_text[after_child..].starts_with('\n') {
            continue;
        }
        let next_line_start = after_child + 1;
        let next_content_start = next_line_start
            + xml_text[next_line_start..]
                .chars()
                .take_while(|ch| *ch == '\t' || *ch == ' ')
                .map(char::len_utf8)
                .sum::<usize>();
        let mut result = String::with_capacity(xml_text.len());
        result.push_str(&xml_text[..after_open]);
        result.push_str(&xml_text[next_content_start..]);
        return result;
    }
    remove_metadata_child_text(xml_text, local_name, item_name)
}

pub(crate) fn remove_metadata_child_text(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> String {
    remove_metadata_child_text_with_flag(xml_text, local_name, item_name).0
}

pub(crate) fn remove_metadata_child_text_with_flag(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> (String, bool) {
    let plain = format!("<{local_name}>{item_name}</{local_name}>");
    let prefixed = format!("<md:{local_name}>{item_name}</md:{local_name}>");
    let mut removed = false;
    let mut result = String::with_capacity(xml_text.len());
    for line in xml_text.split_inclusive('\n') {
        let trimmed = line.trim();
        if !removed && (trimmed == plain || trimmed == prefixed) {
            removed = true;
            continue;
        }
        result.push_str(line);
    }
    if removed {
        (result, true)
    } else if let Some(index) = xml_text.find(&plain) {
        let mut result = xml_text.to_string();
        result.replace_range(index..index + plain.len(), "");
        (result, true)
    } else if let Some(index) = xml_text.find(&prefixed) {
        let mut result = xml_text.to_string();
        result.replace_range(index..index + prefixed.len(), "");
        (result, true)
    } else {
        (xml_text.to_string(), false)
    }
}

pub(crate) struct MetaRemoveReference {
    pub(crate) file: String,
    pub(crate) pattern: String,
}

pub(crate) fn metadata_object_registered(
    config_xml: &Path,
    obj_type: &str,
    obj_name: &str,
) -> bool {
    let Ok(text) = read_utf8_sig(config_xml) else {
        return false;
    };
    text.contains(&format!("<{obj_type}>{obj_name}</{obj_type}>"))
        || text.contains(&format!("<md:{obj_type}>{obj_name}</md:{obj_type}>"))
}

#[allow(clippy::too_many_arguments)]
fn meta_remove_reference_scan(
    config_dir: &Path,
    obj_type: &str,
    obj_name: &str,
    type_plural: &str,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> Result<MetaRemoveReferenceScan, String> {
    let patterns = meta_remove_search_patterns(obj_type, obj_name, type_plural);
    let mut references = Vec::new();
    let mut already_found = HashSet::new();
    let mut reads = Vec::new();
    let traversal = metadata_files_recursive(config_dir)?;

    for file in traversal.files.iter().filter(|file| {
        matches!(
            file.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase),
            Some(ext) if ext == "xml" || ext == "bsl"
        )
    }) {
        if meta_remove_should_skip_file(file, config_dir, obj_xml, obj_dir, has_xml, has_dir) {
            continue;
        }
        let snapshot = read_utf8_sig_snapshot(file)?;
        let content = snapshot.text.clone();
        reads.push(MetaRemoveTextRead {
            path: file.clone(),
            raw: snapshot.raw,
            text: snapshot.text,
        });
        let rel = relative_display(file, config_dir);
        for pattern in &patterns {
            if content.contains(pattern) {
                already_found.insert(rel.clone());
                references.push(MetaRemoveReference {
                    file: rel,
                    pattern: pattern.clone(),
                });
                break;
            }
        }
    }

    let type_name_ref = format!("{obj_type}.{obj_name}");
    for read in reads.iter().filter(|read| {
        read.path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
    }) {
        let rel = relative_display(&read.path, config_dir);
        if already_found.contains(&rel) {
            continue;
        }
        if read.text.contains(&type_name_ref) {
            references.push(MetaRemoveReference {
                file: rel,
                pattern: type_name_ref.clone(),
            });
        }
    }

    Ok(MetaRemoveReferenceScan {
        references,
        reads,
        directory_reads: traversal.directories,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn meta_remove_reference_xml_dependency_paths(
    config_dir: &Path,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> Result<Vec<PathBuf>, String> {
    metadata_files_recursive(config_dir)?
        .files
        .into_iter()
        .filter(|file| {
            file.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
        })
        .filter(|file| {
            !meta_remove_should_skip_file(file, config_dir, obj_xml, obj_dir, has_xml, has_dir)
        })
        .map(|file| {
            read_utf8_sig(&file)?;
            Ok(file)
        })
        .collect()
}

fn metadata_files_recursive(root: &Path) -> Result<MetaRemoveTraversal, String> {
    metadata_files_recursive_with_limits(
        root,
        MetaRemoveTraversalLimits {
            max_depth: META_REMOVE_MAX_TRAVERSAL_DEPTH,
            max_entries: META_REMOVE_MAX_TRAVERSAL_ENTRIES,
        },
    )
}

fn metadata_files_recursive_with_limits(
    root: &Path,
    limits: MetaRemoveTraversalLimits,
) -> Result<MetaRemoveTraversal, String> {
    let mut visited_directories = HashSet::new();
    let mut visited_entries = 0usize;
    metadata_files_recursive_bounded(
        root,
        0,
        limits,
        &mut visited_directories,
        &mut visited_entries,
    )
}

fn metadata_files_recursive_bounded(
    root: &Path,
    depth: usize,
    limits: MetaRemoveTraversalLimits,
    visited_directories: &mut HashSet<PathBuf>,
    visited_entries: &mut usize,
) -> Result<MetaRemoveTraversal, String> {
    if depth > limits.max_depth {
        return Err(format!(
            "reference scan traversal exceeded the maximum depth of {}: {}",
            limits.max_depth,
            root.display()
        ));
    }
    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        // The format guard calls this helper before the operation can render
        // its normal "Config directory not found" outcome. Preserve that
        // public error path for an initially absent root, while retaining
        // fail-closed traversal once a root or child has been observed.
        Err(error) if depth == 0 && error.kind() == ErrorKind::NotFound => {
            return Ok(MetaRemoveTraversal {
                files: Vec::new(),
                directories: Vec::new(),
            });
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect reference scan root {}: {error}",
                root.display()
            ));
        }
    };
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(
        &root_metadata,
    ) {
        return Err(format!(
            "reference scan directory must not be a symbolic link or reparse point: {}",
            root.display()
        ));
    }
    if !root_metadata.is_dir() {
        return Err(format!(
            "reference scan root is not a directory: {}",
            root.display()
        ));
    }
    let directory_identity = fs::canonicalize(root).map_err(|error| {
        format!(
            "failed to resolve reference scan directory identity {}: {error}",
            root.display()
        )
    })?;
    if !visited_directories.insert(directory_identity) {
        return Err(format!(
            "reference scan directory cycle or duplicate identity detected before traversal: {}",
            root.display()
        ));
    }

    let directory_entries = fs::read_dir(root).map_err(|error| {
        format!(
            "failed to read reference scan directory {}: {error}",
            root.display()
        )
    })?;
    let mut entries = Vec::new();
    for entry in directory_entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read an entry in reference scan directory {}: {error}",
                root.display()
            )
        })?;
        if *visited_entries >= limits.max_entries {
            return Err(format!(
                "reference scan traversal exceeded the maximum of {} entries: {}",
                limits.max_entries,
                root.display()
            ));
        }
        *visited_entries += 1;
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());
    let mut inspected_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "failed to inspect reference scan entry {}: {error}",
                path.display()
            )
        })?;
        if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(&metadata)
        {
            return Err(format!(
                "reference scan entry must not be a symbolic link or reparse point: {}",
                path.display()
            ));
        }
        let kind = if metadata.is_dir() {
            DirectoryTopologyEntryKind::Directory
        } else if metadata.is_file() {
            DirectoryTopologyEntryKind::File
        } else {
            return Err(format!(
                "reference scan entry has an unsupported filesystem type: {}",
                path.display()
            ));
        };
        inspected_entries.push((path, entry.file_name(), kind));
    }
    let direct_entries = inspected_entries
        .iter()
        .map(|(_, name, kind)| DirectoryTopologyEntry {
            name: name.clone(),
            kind: *kind,
        })
        .collect();
    let mut result = MetaRemoveTraversal {
        files: Vec::new(),
        directories: vec![MetaRemoveDirectoryRead {
            path: root.to_path_buf(),
            direct_entries,
        }],
    };

    for (path, _, kind) in inspected_entries {
        if kind == DirectoryTopologyEntryKind::Directory {
            if depth >= limits.max_depth {
                return Err(format!(
                    "reference scan traversal exceeded the maximum depth of {}: {}",
                    limits.max_depth,
                    path.display()
                ));
            }
            let nested = metadata_files_recursive_bounded(
                &path,
                depth + 1,
                limits,
                visited_directories,
                visited_entries,
            )?;
            result.files.extend(nested.files);
            result.directories.extend(nested.directories);
        } else {
            result.files.push(path);
        }
    }
    Ok(result)
}

pub(crate) fn meta_remove_should_skip_file(
    file: &Path,
    config_dir: &Path,
    obj_xml: &Path,
    obj_dir: &Path,
    has_xml: bool,
    has_dir: bool,
) -> bool {
    if has_xml && file == obj_xml {
        return true;
    }
    if has_dir && (file == obj_dir || file.starts_with(obj_dir)) {
        return true;
    }
    let rel = relative_display(file, config_dir);
    rel == "Configuration.xml" || rel == "ConfigDumpInfo.xml" || rel.starts_with("Subsystems")
}

pub(crate) fn meta_remove_search_patterns(
    obj_type: &str,
    obj_name: &str,
    type_plural: &str,
) -> Vec<String> {
    let mut patterns = Vec::new();
    if let Some(ref_names) = meta_remove_type_ref_names(obj_type) {
        patterns.extend(ref_names.iter().map(|name| format!("{name}.{obj_name}")));
    }
    if let Some(manager) = meta_remove_ru_manager(obj_type) {
        patterns.push(format!("{manager}.{obj_name}"));
    }
    patterns.push(format!("{type_plural}.{obj_name}"));
    if obj_type == "CommonModule" {
        patterns.push(format!("{obj_name}."));
        patterns.push(format!("<Handler>{obj_name}."));
        patterns.push(format!("<MethodName>{obj_name}."));
    }
    patterns
}

pub(crate) fn meta_remove_supported_types() -> &'static [&'static str] {
    &[
        "Catalog",
        "Document",
        "Enum",
        "Constant",
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
        "Report",
        "DataProcessor",
        "CommonModule",
        "ScheduledJob",
        "EventSubscription",
        "HTTPService",
        "WebService",
        "DefinedType",
        "Role",
        "Subsystem",
        "CommonForm",
        "CommonTemplate",
        "CommonPicture",
        "CommonAttribute",
        "SessionParameter",
        "FunctionalOption",
        "FunctionalOptionsParameter",
        "Sequence",
        "FilterCriterion",
        "SettingsStorage",
        "XDTOPackage",
        "WSReference",
        "StyleItem",
        "Language",
    ]
}

pub(crate) fn meta_remove_type_plural(obj_type: &str) -> Option<&'static str> {
    if !meta_remove_supported_types().contains(&obj_type) {
        return None;
    }
    metadata_kind(obj_type).map(|kind| kind.directory)
}

pub(crate) fn meta_remove_type_ref_names(obj_type: &str) -> Option<&'static [&'static str]> {
    match obj_type {
        "Catalog" => Some(&["CatalogRef", "CatalogObject"]),
        "Document" => Some(&["DocumentRef", "DocumentObject"]),
        "Enum" => Some(&["EnumRef"]),
        "ExchangePlan" => Some(&["ExchangePlanRef", "ExchangePlanObject"]),
        "ChartOfAccounts" => Some(&["ChartOfAccountsRef", "ChartOfAccountsObject"]),
        "ChartOfCharacteristicTypes" => Some(&[
            "ChartOfCharacteristicTypesRef",
            "ChartOfCharacteristicTypesObject",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "ChartOfCalculationTypesRef",
            "ChartOfCalculationTypesObject",
        ]),
        "BusinessProcess" => Some(&["BusinessProcessRef", "BusinessProcessObject"]),
        "Task" => Some(&["TaskRef", "TaskObject"]),
        _ => None,
    }
}

pub(crate) fn meta_remove_ru_manager(obj_type: &str) -> Option<&'static str> {
    match obj_type {
        "Catalog" => Some("Справочники"),
        "Document" => Some("Документы"),
        "Enum" => Some("Перечисления"),
        "Constant" => Some("Константы"),
        "InformationRegister" => Some("РегистрыСведений"),
        "AccumulationRegister" => Some("РегистрыНакопления"),
        "AccountingRegister" => Some("РегистрыБухгалтерии"),
        "CalculationRegister" => Some("РегистрыРасчета"),
        "ChartOfAccounts" => Some("ПланыСчетов"),
        "ChartOfCharacteristicTypes" => Some("ПланыВидовХарактеристик"),
        "ChartOfCalculationTypes" => Some("ПланыВидовРасчета"),
        "BusinessProcess" => Some("БизнесПроцессы"),
        "Task" => Some("Задачи"),
        "ExchangePlan" => Some("ПланыОбмена"),
        "Report" => Some("Отчеты"),
        "DataProcessor" => Some("Обработки"),
        "DocumentJournal" => Some("ЖурналыДокументов"),
        _ => None,
    }
}

pub(crate) const META_COMPILE_SUPPORTED_TYPES: &[&str] = &[
    "Catalog",
    "Document",
    "Enum",
    "Constant",
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
    "Report",
    "DataProcessor",
    "CommonModule",
    "ScheduledJob",
    "EventSubscription",
    "HTTPService",
    "WebService",
    "DefinedType",
];

pub(crate) const META_COMPILE_PENDING_TYPES: &[&str] = &[];

pub(crate) fn meta_compile_type_plural(obj_type: &str) -> Option<&'static str> {
    if !META_COMPILE_SUPPORTED_TYPES.contains(&obj_type) {
        return None;
    }
    metadata_kind(obj_type).map(|kind| kind.directory)
}

pub(crate) fn meta_compile_uses_object_subdir(obj_type: &str) -> bool {
    !matches!(
        obj_type,
        "DefinedType" | "ScheduledJob" | "EventSubscription"
    )
}

pub(crate) fn meta_compile_module_files(obj_type: &str) -> &'static [&'static str] {
    match obj_type {
        "Catalog"
        | "Document"
        | "ChartOfAccounts"
        | "ChartOfCharacteristicTypes"
        | "ChartOfCalculationTypes"
        | "BusinessProcess"
        | "Task"
        | "ExchangePlan" => &["ObjectModule.bsl"],
        "Enum" => &["ManagerModule.bsl"],
        "Constant" => &["ManagerModule.bsl", "ValueManagerModule.bsl"],
        "InformationRegister"
        | "AccumulationRegister"
        | "AccountingRegister"
        | "CalculationRegister" => &["RecordSetModule.bsl"],
        "Report" | "DataProcessor" => &["ObjectModule.bsl", "ManagerModule.bsl"],
        "CommonModule" | "HTTPService" | "WebService" => &["Module.bsl"],
        _ => &[],
    }
}

pub(crate) fn meta_compile_extra_ext_files(
    obj_type: &str,
    format_version: &str,
) -> Vec<(&'static str, String)> {
    match obj_type {
        "ExchangePlan" => vec![(
            "Content.xml",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<ExchangePlanContent xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\"/>\r\n"
            ),
        )],
        "BusinessProcess" => vec![(
            "Flowchart.xml",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<GraphicalSchema xmlns=\"http://v8.1c.ru/8.3/xcf/scheme\" xmlns:sch=\"http://v8.1c.ru/8.2/data/graphscheme\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"{format_version}\">\r\n\
\t<BackColor>style:FieldBackColor</BackColor>\r\n\
\t<GridEnabled>true</GridEnabled>\r\n\
\t<DrawGridMode>Lines</DrawGridMode>\r\n\
\t<GridHorizontalStep>20</GridHorizontalStep>\r\n\
\t<GridVerticalStep>20</GridVerticalStep>\r\n\
\t<PrintParameters>\r\n\
\t\t<TopMargin>10</TopMargin>\r\n\
\t\t<LeftMargin>10</LeftMargin>\r\n\
\t\t<BottomMargin>10</BottomMargin>\r\n\
\t\t<RightMargin>10</RightMargin>\r\n\
\t\t<BlackAndWhite>false</BlackAndWhite>\r\n\
\t\t<FitPageMode>Auto</FitPageMode>\r\n\
\t</PrintParameters>\r\n\
\t<Items/>\r\n\
</GraphicalSchema>\r\n"
            ),
        )],
        _ => Vec::new(),
    }
}

pub(crate) fn meta_compile_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let output_dir_label = string_arg(args, &["outputDir", "OutputDir"])
        .ok_or_else(|| "missing required OutputDir argument".to_string())?;
    let output_dir = absolutize(PathBuf::from(output_dir_label), &context.cwd);
    let definition = read_meta_compile_definition(args, context)?;
    Ok(meta_compile_definition_format_dependency_paths(
        &definition,
        &output_dir,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MetaCompileEventSubscriptionDependency {
    subscription_name: String,
    subscription_descriptor_path: PathBuf,
    source_type: String,
    source_descriptor_path: PathBuf,
}

fn meta_compile_event_subscription_dependencies(
    definition: &Value,
    output_dir: &Path,
) -> Vec<MetaCompileEventSubscriptionDependency> {
    let definitions = definition
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(definition));
    let mut dependencies = Vec::new();
    for definition in definitions {
        let Some(object) = definition.as_object() else {
            continue;
        };
        let Some(raw_type) = object
            .get("type")
            .or_else(|| object.get("objectType"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if normalize_meta_object_type(raw_type) != "EventSubscription" {
            continue;
        }
        let Some(subscription_name) = object.get("name").and_then(Value::as_str) else {
            continue;
        };
        if validate_meta_compile_name("metadata object", subscription_name).is_err() {
            continue;
        }
        let subscription_descriptor_path = output_dir
            .join("EventSubscriptions")
            .join(format!("{subscription_name}.xml"));
        for raw_source in meta_compile_string_list(object.get("source")) {
            for source_type in raw_source
                .split('+')
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let resolved = resolve_meta_type(source_type);
                if validate_meta_resolved_type(source_type, &resolved).is_err() {
                    continue;
                }
                let Some((prefix, source_name)) = resolved.split_once('.') else {
                    continue;
                };
                let source_name = source_name.to_string();
                let source_object_type = match prefix {
                    "CatalogRef" | "CatalogObject" => "Catalog",
                    "DocumentRef" | "DocumentObject" => "Document",
                    "EnumRef" => "Enum",
                    "ChartOfAccountsRef" | "ChartOfAccountsObject" => "ChartOfAccounts",
                    "ChartOfCharacteristicTypesRef" | "ChartOfCharacteristicTypesObject" => {
                        "ChartOfCharacteristicTypes"
                    }
                    "ChartOfCalculationTypesRef" | "ChartOfCalculationTypesObject" => {
                        "ChartOfCalculationTypes"
                    }
                    "ExchangePlanRef" | "ExchangePlanObject" => "ExchangePlan",
                    "BusinessProcessRef" | "BusinessProcessObject" => "BusinessProcess",
                    "TaskRef" | "TaskObject" => "Task",
                    "ReportObject" => "Report",
                    "DataProcessorObject" => "DataProcessor",
                    "DefinedType" => "DefinedType",
                    _ => continue,
                };
                let Some(source_directory) =
                    metadata_kind(source_object_type).map(|kind| kind.directory)
                else {
                    continue;
                };
                dependencies.push(MetaCompileEventSubscriptionDependency {
                    subscription_name: subscription_name.to_string(),
                    subscription_descriptor_path: subscription_descriptor_path.clone(),
                    source_type: resolved,
                    source_descriptor_path: output_dir
                        .join(source_directory)
                        .join(format!("{source_name}.xml")),
                });
            }
        }
    }
    dependencies
}

fn validate_meta_compile_event_subscription_dependencies(
    dependencies: &[MetaCompileEventSubscriptionDependency],
    transaction: &CompileTransaction,
) -> Result<(), String> {
    let planned_creates = transaction
        .planned_created_paths()
        .into_iter()
        .collect::<HashSet<_>>();
    for dependency in dependencies {
        if !planned_creates.contains(&dependency.subscription_descriptor_path)
            || planned_creates.contains(&dependency.source_descriptor_path)
        {
            continue;
        }
        match fs::symlink_metadata(&dependency.source_descriptor_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(format!(
                    "EventSubscription '{}' source type '{}' requires a regular metadata descriptor at {}",
                    dependency.subscription_name,
                    dependency.source_type,
                    dependency.source_descriptor_path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "EventSubscription '{}' source type '{}' requires an existing or same-batch metadata descriptor at {}; 1C 8.3.27 rejects unknown source types",
                    dependency.subscription_name,
                    dependency.source_type,
                    dependency.source_descriptor_path.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect EventSubscription '{}' source type '{}' descriptor {}: {error}",
                    dependency.subscription_name,
                    dependency.source_type,
                    dependency.source_descriptor_path.display()
                ));
            }
        }
    }
    Ok(())
}

fn meta_compile_definition_format_dependency_paths(
    definition: &Value,
    output_dir: &Path,
) -> Vec<PathBuf> {
    let definitions = definition
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(definition));
    let mut paths = vec![output_dir.join("Configuration.xml")];
    for definition in definitions {
        let Some(object) = definition.as_object() else {
            continue;
        };
        let Some(raw_type) = object
            .get("type")
            .or_else(|| object.get("objectType"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(name) = object.get("name").and_then(Value::as_str) else {
            continue;
        };
        if validate_meta_compile_name("metadata object", name).is_err() {
            continue;
        }
        let object_type = normalize_meta_object_type(raw_type);
        let Some(type_dir) = meta_compile_type_plural(&object_type) else {
            continue;
        };
        let target = output_dir.join(type_dir).join(name);
        let descriptor = target.with_extension("xml");
        let descriptor_exists = descriptor.is_file();
        paths.push(descriptor);
        if descriptor_exists {
            continue;
        }
        let ext_dir = target.join("Ext");
        for (file_name, _) in meta_compile_extra_ext_files(&object_type, "") {
            let path = ext_dir.join(file_name);
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    paths.extend(
        meta_compile_event_subscription_dependencies(definition, output_dir)
            .into_iter()
            .map(|dependency| dependency.source_descriptor_path),
    );
    paths.sort();
    paths.dedup();
    paths
}

pub(crate) fn compile_meta(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let write_result = plan_meta_compile(args, context).and_then(
        |(stdout, mut transaction, validation_paths, config_owner, format_dependencies)| {
            let format_dependencies = format_dependencies
                .iter()
                .map(PathBuf::as_path)
                .collect::<Vec<_>>();
            guard_active_format_dependencies(&mut transaction, &format_dependencies, context)?;
            let report = transaction.commit_with_post_validation(|| {
                if let Some(config_owner) = config_owner.as_deref() {
                    require_meta_configuration_owner_validation(
                        config_owner,
                        context,
                        "meta.compile",
                    )?;
                }
                validate_meta_compile_post_state(&validation_paths, context)
            })?;
            let mut changes = report
                .created
                .iter()
                .map(|path| format!("created {}", path.display()))
                .collect::<Vec<_>>();
            changes.extend(
                report
                    .updated
                    .iter()
                    .map(|path| format!("updated {}", path.display())),
            );
            Ok((stdout, report.created, changes, report.cleanup_warnings))
        },
    );

    match write_result {
        Ok((stdout, artifacts, changes, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.meta.compile completed with native metadata compiler".to_string(),
            changes,
            warnings,
            errors: Vec::new(),
            artifacts: artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.compile failed in native metadata compiler".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

pub(crate) fn preview_meta_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<AdapterOutcome, String> {
    let (_stdout, transaction, _validation_paths, _config_owner, _format_dependencies) =
        plan_meta_compile(args, context)?;
    Ok(AdapterOutcome {
        ok: true,
        summary: "dry run: unica.meta.compile planned native metadata compilation".to_string(),
        changes: transaction.dry_run_changes(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: Vec::new(),
        stdout: Some(transaction.dry_run_stdout()),
        stderr: None,
        command: None,
    })
}

type MetaCompilePlan = (
    String,
    CompileTransaction,
    Vec<PathBuf>,
    Option<PathBuf>,
    Vec<PathBuf>,
);

fn plan_meta_compile(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<MetaCompilePlan, String> {
    let output_dir_label = string_arg(args, &["outputDir", "OutputDir"])
        .ok_or_else(|| "missing required OutputDir argument".to_string())?
        .to_string();
    let output_dir = absolutize(PathBuf::from(&output_dir_label), &context.cwd);
    let config_path = output_dir.join("Configuration.xml");
    let config_owner = if config_path.is_file() {
        let snapshot = read_utf8_sig_snapshot(&config_path)?;
        require_meta_configuration_owner_validation(&config_path, context, "meta.compile")?;
        if fs::read(&config_path)
            .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?
            != snapshot.raw
        {
            return Err(format!(
                "Configuration owner changed while planning: {}",
                config_path.display()
            ));
        }
        #[cfg(test)]
        run_meta_compile_after_owner_validation_hook(&config_path);
        Some((config_path, snapshot.raw))
    } else {
        None
    };
    let mut transaction = CompileTransaction::new();
    let defn = read_meta_compile_definition_guarded(args, context, &mut transaction)?;
    let event_subscription_dependencies =
        meta_compile_event_subscription_dependencies(&defn, &output_dir);
    let mut format_dependencies =
        meta_compile_definition_format_dependency_paths(&defn, &output_dir);
    #[cfg(test)]
    run_meta_compile_after_format_plan_hook();
    let (stdout, planned_artifacts) = compile_meta_value(
        defn,
        &output_dir_label,
        &output_dir,
        context,
        &mut transaction,
        &mut format_dependencies,
    )?;
    validate_meta_compile_event_subscription_dependencies(
        &event_subscription_dependencies,
        &transaction,
    )?;
    if let Some((config_owner, expected_preimage)) = &config_owner {
        transaction.guard_or_verify_exact_preimage(config_owner, expected_preimage)?;
    }
    Ok((
        stdout,
        transaction,
        planned_artifacts,
        config_owner.map(|(path, _)| path),
        format_dependencies,
    ))
}

fn read_meta_compile_definition(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Value, String> {
    let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
    let json_path = absolutize(json_path_raw.clone(), &context.cwd);
    if !json_path.is_file() {
        return Err(format!("File not found: {}", json_path_raw.display()));
    }

    let json_text = fs::read_to_string(&json_path)
        .map_err(|err| format!("failed to read {}: {err}", json_path.display()))?;
    serde_json::from_str(json_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("failed to parse metadata JSON: {err}"))
}

fn read_meta_compile_definition_guarded(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
) -> Result<Value, String> {
    let json_path_raw = required_path(args, &["jsonPath", "JsonPath"], "JsonPath")?;
    let json_path = absolutize(json_path_raw.clone(), &context.cwd);
    if !json_path.is_file() {
        return Err(format!("File not found: {}", json_path_raw.display()));
    }
    FileBackedJson::read(&json_path, |err| {
        format!("failed to parse metadata JSON: {err}")
    })?
    .bind_to(transaction)
}

fn require_meta_configuration_owner_validation(
    config_path: &Path,
    context: &WorkspaceContext,
    operation: &str,
) -> Result<(), String> {
    validate_cf_owner_path(config_path, context).map_err(|detail| {
        format!(
            "{operation} Configuration owner validation failed for {}: {}",
            config_path.display(),
            detail.trim()
        )
    })
}

fn validate_meta_compile_post_state(
    validation_paths: &[PathBuf],
    context: &WorkspaceContext,
) -> Result<(), String> {
    for path in validation_paths {
        if path.extension().and_then(|value| value.to_str()) != Some("xml") {
            continue;
        }
        let xml = read_utf8_sig(path)?;
        let document = Document::parse(xml.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("XML parse error in {}: {error}", path.display()))?;
        if document.root_element().tag_name().name() == "MetaDataObject" {
            validate_metadata_owner_shape_8_3_27(path, context, "meta.compile")?;
        }
    }
    Ok(())
}

fn compile_meta_value(
    defn: Value,
    output_dir_label: &str,
    output_dir: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
    format_dependencies: &mut Vec<PathBuf>,
) -> Result<(String, Vec<PathBuf>), String> {
    match defn {
        Value::Array(items) => compile_meta_batch(
            items,
            output_dir_label,
            output_dir,
            context,
            transaction,
            format_dependencies,
        ),
        single => compile_meta_object(
            single,
            output_dir_label,
            output_dir,
            context,
            transaction,
            format_dependencies,
        ),
    }
}

fn compile_meta_batch(
    items: Vec<Value>,
    output_dir_label: &str,
    output_dir: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
    format_dependencies: &mut Vec<PathBuf>,
) -> Result<(String, Vec<PathBuf>), String> {
    let total = items.len();
    let mut stdout = String::new();
    let mut artifacts = Vec::<PathBuf>::new();
    let mut failed = Vec::<String>::new();

    for (index, item) in items.into_iter().enumerate() {
        match compile_meta_object(
            item,
            output_dir_label,
            output_dir,
            context,
            transaction,
            format_dependencies,
        ) {
            Ok((item_stdout, mut item_artifacts)) => {
                stdout.push_str(&item_stdout);
                artifacts.append(&mut item_artifacts);
            }
            Err(error) => {
                failed.push(format!("#{}: {error}", index + 1));
                stdout.push_str(&format!("[FAIL] #{}: {error}\n", index + 1));
            }
        }
    }

    let compiled = total.saturating_sub(failed.len());
    stdout.push_str(&format!(
        "\n=== Batch: {total} objects, {compiled} compiled, {} failed ===\n",
        failed.len()
    ));

    if failed.is_empty() {
        Ok((stdout, artifacts))
    } else {
        Err(failed.join("\n"))
    }
}

fn compile_meta_object(
    mut defn: Value,
    output_dir_label: &str,
    output_dir: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
    format_dependencies: &mut Vec<PathBuf>,
) -> Result<(String, Vec<PathBuf>), String> {
    if defn.get("type").is_none() {
        if let Some(object_type) = defn.get("objectType").cloned() {
            defn.as_object_mut()
                .ok_or_else(|| "metadata JSON must be an object".to_string())?
                .insert("type".to_string(), object_type);
        }
    }
    let object = defn
        .as_object()
        .ok_or_else(|| "metadata JSON must be an object".to_string())?;
    let raw_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "JSON must have 'type' field".to_string())?;
    let obj_type = normalize_meta_object_type(raw_type);
    let type_plural = meta_compile_type_plural(&obj_type).ok_or_else(|| {
        format!(
            "Unsupported type: {obj_type}. Supported: {}. Documented pending: {}",
            META_COMPILE_SUPPORTED_TYPES.join(", "),
            META_COMPILE_PENDING_TYPES.join(", ")
        )
    })?;
    let obj_name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "JSON must have 'name' field".to_string())?;
    validate_meta_compile_name("metadata object", obj_name)?;
    let type_dir = output_dir.join(type_plural);
    let main_xml_path = type_dir.join(format!("{obj_name}.xml"));
    let obj_sub_dir = type_dir.join(obj_name);
    let ext_dir = obj_sub_dir.join("Ext");

    match fs::symlink_metadata(&main_xml_path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            return Ok((
                format!(
                    "[SKIP] {obj_type} '{obj_name}' already exists at {}; no files changed\n",
                    main_xml_path.display()
                ),
                Vec::new(),
            ));
        }
        Ok(_) => {
            return Err(format!(
                "existing metadata target is not a regular file: {}",
                main_xml_path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to inspect metadata target {}: {error}",
                main_xml_path.display()
            ));
        }
    }
    let format_version = detect_format_version(output_dir, context)?.to_string();
    let (metadata_xml, uid) =
        meta_compile_object_xml(object, &obj_type, obj_name, &format_version)?;
    transaction.create_utf8_bom_text(&main_xml_path, &metadata_xml)?;

    let mut artifacts = vec![main_xml_path.clone()];
    let mut modules_created = Vec::<PathBuf>::new();
    for module_name in meta_compile_module_files(&obj_type) {
        let module_path = ext_dir.join(module_name);
        if !module_path.is_file() {
            transaction.create_utf8_bom_text(&module_path, "")?;
            modules_created.push(module_path.clone());
            artifacts.push(module_path.clone());
        }
    }
    for (file_name, content) in meta_compile_extra_ext_files(&obj_type, &format_version) {
        let file_path = ext_dir.join(file_name);
        match fs::symlink_metadata(&file_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                transaction.create_utf8_bom_text(&file_path, &content)?;
                modules_created.push(file_path.clone());
                artifacts.push(file_path.clone());
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect metadata extra target {}: {error}",
                    file_path.display()
                ));
            }
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                let snapshot = fs::read(&file_path).map_err(|error| {
                    format!(
                        "failed to read metadata extra target {}: {error}",
                        file_path.display()
                    )
                })?;
                transaction.guard_or_verify_exact_preimage(&file_path, &snapshot)?;
                format_dependencies.push(file_path);
            }
            Ok(_) => {
                return Err(format!(
                    "existing metadata extra target is not a regular file: {}",
                    file_path.display()
                ));
            }
        }
    }

    let reg_result = transaction.register_canonical_child(
        output_dir.join("Configuration.xml"),
        &obj_type,
        obj_name,
    )?;

    let attr_count = object
        .get("attributes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let ts_count = object
        .get("tabularSections")
        .map(meta_compile_collection_count)
        .unwrap_or(0);
    let enum_value_count = object
        .get("values")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let dim_count = object
        .get("dimensions")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let res_count = object
        .get("resources")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let column_count = object
        .get("columns")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut stdout = format!(
        "[OK] {obj_type} '{obj_name}' compiled\n     UUID: {uid}\n     File: {}/{type_plural}/{obj_name}.xml\n",
        output_dir_label.trim_end_matches(['/', '\\'])
    );
    let mut details = Vec::new();
    if attr_count > 0 {
        details.push(format!("Attributes: {attr_count}"));
    }
    if ts_count > 0 {
        details.push(format!("TabularSections: {ts_count}"));
    }
    if enum_value_count > 0 {
        details.push(format!("Values: {enum_value_count}"));
    }
    if dim_count > 0 {
        details.push(format!("Dimensions: {dim_count}"));
    }
    if res_count > 0 {
        details.push(format!("Resources: {res_count}"));
    }
    if column_count > 0 {
        details.push(format!("Columns: {column_count}"));
    }
    if !details.is_empty() {
        stdout.push_str(&format!("     {}\n", details.join(", ")));
    }
    for module in modules_created {
        stdout.push_str(&format!(
            "     Module: {}/{type_plural}/{obj_name}/Ext/{}\n",
            output_dir_label.trim_end_matches(['/', '\\']),
            module
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("ObjectModule.bsl")
        ));
    }
    match reg_result {
        RegistrationStatus::Added => stdout.push_str(&format!(
            "     Configuration.xml: <{obj_type}>{obj_name}</{obj_type}> added to ChildObjects\n"
        )),
        RegistrationStatus::AlreadyPresent => stdout.push_str(&format!(
            "     Configuration.xml: <{obj_type}>{obj_name}</{obj_type}> already registered\n"
        )),
        RegistrationStatus::MissingTarget => stdout.push_str(&format!(
            "     Configuration.xml: not found at {}/Configuration.xml (register manually)\n",
            output_dir_label.trim_end_matches(['/', '\\'])
        )),
    }

    Ok((stdout, artifacts))
}

pub(crate) fn meta_compile_collection_count(value: &Value) -> usize {
    value
        .as_array()
        .map(Vec::len)
        .or_else(|| value.as_object().map(Map::len))
        .unwrap_or(0)
}

pub(crate) fn normalize_meta_object_type(raw: &str) -> String {
    match raw {
        "Справочник" | "Каталог" => "Catalog",
        "Документ" => "Document",
        "Перечисление" => "Enum",
        "Константа" => "Constant",
        "РегистрСведений" => "InformationRegister",
        "РегистрНакопления" => "AccumulationRegister",
        "РегистрБухгалтерии" => "AccountingRegister",
        "РегистрРасчёта" | "РегистрРасчета" => "CalculationRegister",
        "ПланСчетов" => "ChartOfAccounts",
        "ПланВидовХарактеристик" => "ChartOfCharacteristicTypes",
        "ПланВидовРасчёта" | "ПланВидовРасчета" => {
            "ChartOfCalculationTypes"
        }
        "БизнесПроцесс" => "BusinessProcess",
        "Задача" => "Task",
        "ПланОбмена" => "ExchangePlan",
        "ЖурналДокументов" => "DocumentJournal",
        "Отчёт" | "Отчет" => "Report",
        "Обработка" => "DataProcessor",
        "ОбщийМодуль" => "CommonModule",
        "РегламентноеЗадание" => "ScheduledJob",
        "ПодпискаНаСобытие" => "EventSubscription",
        "HTTPСервис" => "HTTPService",
        "ВебСервис" => "WebService",
        "ОпределяемыйТип" => "DefinedType",
        other => other,
    }
    .to_string()
}

pub(crate) fn meta_compile_object_xml(
    defn: &Map<String, Value>,
    obj_type: &str,
    obj_name: &str,
    format_version: &str,
) -> Result<(String, String), String> {
    validate_meta_compile_name("metadata object", obj_name)?;
    validate_meta_compile_type_contract(defn, obj_type)?;
    if obj_type == "Catalog" {
        return meta_compile_catalog_xml(defn, obj_name, format_version);
    }

    let mut next_uuid = fresh_meta_compile_uuid;
    let obj_uuid = next_uuid();
    let synonym = meta_compile_synonym(defn, obj_name);

    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        meta_xmlns_decl(),
        escape_xml(format_version)
    ));
    lines.push(format!("\t<{obj_type} uuid=\"{obj_uuid}\">"));
    emit_meta_internal_info(&mut lines, "\t\t", obj_type, obj_name, &mut next_uuid);
    lines.push("\t\t<Properties>".to_string());
    match obj_type {
        "Document" => emit_meta_document_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym),
        "Enum" => emit_meta_enum_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym),
        "Constant" => emit_meta_constant_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym),
        "InformationRegister" => emit_meta_information_register_properties(
            &mut lines, "\t\t\t", defn, obj_name, &synonym,
        ),
        "AccumulationRegister" => emit_meta_accumulation_register_properties(
            &mut lines, "\t\t\t", defn, obj_name, &synonym,
        ),
        "AccountingRegister" => {
            emit_meta_accounting_register_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "CalculationRegister" => emit_meta_calculation_register_properties(
            &mut lines, "\t\t\t", defn, obj_name, &synonym,
        ),
        "ChartOfAccounts" => {
            emit_meta_chart_of_accounts_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "ChartOfCharacteristicTypes" => emit_meta_chart_of_characteristic_types_properties(
            &mut lines, "\t\t\t", defn, obj_name, &synonym,
        ),
        "ChartOfCalculationTypes" => emit_meta_chart_of_calculation_types_properties(
            &mut lines, "\t\t\t", defn, obj_name, &synonym,
        ),
        "BusinessProcess" => {
            emit_meta_business_process_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "Task" => emit_meta_task_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym),
        "ExchangePlan" => {
            emit_meta_exchange_plan_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "DocumentJournal" => {
            emit_meta_document_journal_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "Report" => emit_meta_report_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym),
        "DataProcessor" => {
            emit_meta_data_processor_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "CommonModule" => {
            emit_meta_common_module_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "ScheduledJob" => {
            emit_meta_scheduled_job_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "EventSubscription" => {
            emit_meta_event_subscription_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "HTTPService" => {
            emit_meta_http_service_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "WebService" => {
            emit_meta_web_service_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        "DefinedType" => {
            emit_meta_defined_type_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym)
        }
        _ => {
            return Err(format!(
                "Unsupported type: {obj_type}. Supported: {}. Documented pending: {}",
                META_COMPILE_SUPPORTED_TYPES.join(", "),
                META_COMPILE_PENDING_TYPES.join(", ")
            ));
        }
    }
    lines.push("\t\t</Properties>".to_string());

    emit_meta_child_objects(&mut lines, "\t\t", defn, obj_type, obj_name, &mut next_uuid)?;

    lines.push(format!("\t</{obj_type}>"));
    lines.push("</MetaDataObject>".to_string());
    Ok((format!("{}\n", lines.join("\n")), obj_uuid))
}

fn validate_meta_compile_type_contract(
    defn: &Map<String, Value>,
    obj_type: &str,
) -> Result<(), String> {
    validate_meta_compile_enum_properties(defn, obj_type)?;
    for field_name in ["attributes", "dimensions", "resources"] {
        for attr in meta_compile_attributes(defn.get(field_name)) {
            validate_meta_compile_attr_type(&attr, field_name)?;
        }
    }
    for section in meta_compile_tabular_sections(defn.get("tabularSections"))? {
        validate_meta_compile_name("tabularSections", &section.name)?;
        for attr in section.columns {
            validate_meta_compile_attr_type(&attr, "tabularSections")?;
        }
    }
    if obj_type == "Task" {
        for value in meta_compile_value_items(defn.get("addressingAttributes")) {
            let attr = meta_compile_parse_attr(&value);
            validate_meta_compile_attr_type(&attr, "addressingAttributes")?;
        }
    }
    if obj_type == "Enum" {
        for value in meta_compile_enum_values(defn.get("values"))? {
            validate_meta_compile_name("enum value", &value.name)?;
        }
    }
    if obj_type == "ChartOfAccounts" {
        for name in meta_compile_named_items(defn.get("accountingFlags")) {
            validate_meta_compile_name("accounting flag", &name)?;
        }
        for name in meta_compile_named_items(defn.get("extDimensionAccountingFlags")) {
            validate_meta_compile_name("ext-dimension accounting flag", &name)?;
        }
    }
    if obj_type == "DocumentJournal" {
        for value in meta_compile_value_items(defn.get("columns")) {
            let name = meta_edit_value_name(&value).unwrap_or_default();
            validate_meta_compile_name("document journal column", &name)?;
            if let Some(indexing) = value
                .as_object()
                .and_then(|object| object.get("indexing"))
                .and_then(Value::as_str)
            {
                validate_meta_8_3_27_property_value(
                    "document journal column",
                    "Indexing",
                    indexing,
                )?;
            }
        }
    }
    if obj_type == "HTTPService" {
        if let Some(templates) = defn.get("urlTemplates").and_then(Value::as_object) {
            for (template_name, template_value) in templates {
                validate_meta_compile_name("URL template", template_name)?;
                if let Some(methods) = template_value
                    .as_object()
                    .and_then(|object| object.get("methods"))
                    .and_then(Value::as_object)
                {
                    for method_name in methods.keys() {
                        validate_meta_compile_name("HTTP method", method_name)?;
                    }
                }
            }
        }
    }
    if obj_type == "WebService" {
        if let Some(operations) = defn.get("operations").and_then(Value::as_object) {
            for (operation_name, operation_value) in operations {
                validate_meta_compile_name("web service operation", operation_name)?;
                if let Some(parameters) = operation_value
                    .as_object()
                    .and_then(|object| object.get("parameters"))
                    .and_then(Value::as_object)
                {
                    for parameter_name in parameters.keys() {
                        validate_meta_compile_name("operation parameter", parameter_name)?;
                    }
                }
            }
        }
    }
    if obj_type == "Constant" {
        let value_type = meta_compile_root_value_type(defn);
        validate_meta_type_union(std::iter::once(value_type.as_str()))?;
    }
    if obj_type == "EventSubscription" {
        let sources = meta_compile_string_list(defn.get("source"));
        validate_meta_type_union(sources.iter().map(String::as_str))?;
    }
    if matches!(obj_type, "ChartOfCharacteristicTypes" | "DefinedType") {
        let value_types = meta_compile_value_types(defn);
        validate_meta_type_union(value_types.iter().map(String::as_str))?;
    }
    if obj_type == "ChartOfAccounts" {
        let max_count = defn
            .get("maxExtDimensionCount")
            .and_then(json_i64_value)
            .unwrap_or(0);
        let has_type = defn
            .get("extDimensionTypes")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        if max_count > 0 && !has_type {
            return Err(
                "ChartOfAccounts maxExtDimensionCount > 0 requires non-empty extDimensionTypes on 8.3.27"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn validate_meta_compile_name(context: &str, name: &str) -> Result<(), String> {
    if is_1c_identifier(name) {
        Ok(())
    } else {
        Err(format!(
            "{context} name '{name}' is not a valid 1C identifier"
        ))
    }
}

fn validate_meta_compile_enum_properties(
    defn: &Map<String, Value>,
    obj_type: &str,
) -> Result<(), String> {
    for (field_name, property_name) in [
        ("codeType", "CodeType"),
        ("codeAllowedLength", "CodeAllowedLength"),
        ("numberType", "NumberType"),
        ("numberAllowedLength", "NumberAllowedLength"),
        ("posting", "Posting"),
        ("realTimePosting", "RealTimePosting"),
        ("registerRecordsDeletion", "RegisterRecordsDeletion"),
        (
            "registerRecordsWritingOnPost",
            "RegisterRecordsWritingOnPost",
        ),
        ("dataLockControlMode", "DataLockControlMode"),
        ("fullTextSearch", "FullTextSearch"),
        ("defaultPresentation", "DefaultPresentation"),
        ("hierarchyType", "HierarchyType"),
        ("editType", "EditType"),
        ("writeMode", "WriteMode"),
        ("registerType", "RegisterType"),
        ("returnValuesReuse", "ReturnValuesReuse"),
        ("reuseSessions", "ReuseSessions"),
        (
            "dependenceOnCalculationTypes",
            "DependenceOnCalculationTypes",
        ),
    ] {
        validate_meta_compile_enum_field(defn, field_name, property_name)?;
    }
    if obj_type == "InformationRegister" {
        validate_meta_compile_enum_field(defn, "periodicity", "InformationRegisterPeriodicity")?;
    }
    match obj_type {
        "Catalog" => {
            validate_meta_compile_enum_field(defn, "subordinationUse", "SubordinationUse")?;
            validate_meta_compile_enum_field(defn, "codeSeries", "CatalogCodeSeries")?;
            validate_meta_compile_enum_field(defn, "choiceMode", "ChoiceMode")?;
        }
        "ChartOfAccounts" => {
            validate_meta_compile_enum_field(defn, "codeSeries", "ChartOfAccountsCodeSeries")?;
        }
        "ChartOfCharacteristicTypes" => {
            validate_meta_compile_enum_field(defn, "codeSeries", "CharacteristicTypeCodeSeries")?;
            validate_meta_compile_enum_field(defn, "predefinedDataUpdate", "PredefinedDataUpdate")?;
            validate_meta_compile_enum_field(defn, "choiceMode", "ChoiceMode")?;
        }
        "Document" => {
            validate_meta_compile_enum_field(
                defn,
                "numberPeriodicity",
                "DocumentNumberPeriodicity",
            )?;
        }
        "BusinessProcess" => {
            validate_meta_compile_enum_field(
                defn,
                "numberPeriodicity",
                "BusinessProcessNumberPeriodicity",
            )?;
        }
        "CalculationRegister" => {
            validate_meta_compile_enum_field(
                defn,
                "periodicity",
                "CalculationRegisterPeriodicity",
            )?;
        }
        "ExchangePlan" => {
            validate_meta_compile_enum_field(defn, "choiceMode", "ChoiceMode")?;
        }
        "HTTPService" => validate_meta_compile_http_methods(defn)?,
        "WebService" => validate_meta_compile_transfer_directions(defn)?,
        _ => {}
    }
    Ok(())
}

fn validate_meta_compile_http_methods(defn: &Map<String, Value>) -> Result<(), String> {
    let Some(templates) = defn.get("urlTemplates").and_then(Value::as_object) else {
        return Ok(());
    };
    for template in templates.values() {
        let Some(methods) = template
            .as_object()
            .and_then(|object| object.get("methods"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for method in methods.values() {
            let value = method.as_str().ok_or_else(|| {
                "meta.compile property HTTPMethod must be a string for the fixed 8.3.27 contract"
                    .to_string()
            })?;
            validate_meta_8_3_27_property_value("meta.compile", "HTTPMethod", value)?;
        }
    }
    Ok(())
}

fn validate_meta_compile_transfer_directions(defn: &Map<String, Value>) -> Result<(), String> {
    let Some(operations) = defn.get("operations").and_then(Value::as_object) else {
        return Ok(());
    };
    for operation in operations.values() {
        let Some(parameters) = operation
            .as_object()
            .and_then(|object| object.get("parameters"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        for parameter in parameters.values() {
            let Some(direction) = parameter
                .as_object()
                .and_then(|object| object.get("direction"))
            else {
                continue;
            };
            let value = direction.as_str().ok_or_else(|| {
                "meta.compile property TransferDirection must be a string for the fixed 8.3.27 contract"
                    .to_string()
            })?;
            validate_meta_8_3_27_property_value("meta.compile", "TransferDirection", value)?;
        }
    }
    Ok(())
}

fn validate_meta_compile_enum_field(
    defn: &Map<String, Value>,
    field_name: &str,
    property_name: &str,
) -> Result<(), String> {
    let Some(value) = defn.get(field_name) else {
        return Ok(());
    };
    let raw_value = value.as_str().ok_or_else(|| {
        format!(
            "meta.compile property {property_name} must be a string for the fixed 8.3.27 contract"
        )
    })?;
    validate_meta_8_3_27_property_value("meta.compile", property_name, raw_value)
}

fn validate_meta_8_3_27_property_value(
    context: &str,
    property_name: &str,
    raw_value: &str,
) -> Result<(), String> {
    let Some((_, allowed_values)) = meta_validate_property_values()
        .iter()
        .find(|(known_property, _)| *known_property == property_name)
    else {
        return Ok(());
    };
    let normalized = normalize_meta_enum_value(raw_value);
    if allowed_values.contains(&normalized.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "{context} property {property_name} value '{normalized}' is not valid for 8.3.27; expected one of: {}",
            allowed_values.join(", ")
        ))
    }
}

fn meta_8_3_27_boolean_properties(object_type: &str) -> &'static [&'static str] {
    match object_type {
        "AccountingFlag" | "AddressingAttribute" | "Attribute" | "ExtDimensionAccountingFlag" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "FillFromFillingValue",
        ],
        "AccountingRegister" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "Correspondence",
            "EnableTotalsSplitting",
        ],
        "AccumulationRegister" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "EnableTotalsSplitting",
        ],
        "BusinessProcess" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "CreateTaskInPrivilegedMode",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "CalculationRegister" => &[
            "UseStandardCommands",
            "ActionPeriod",
            "BasePeriod",
            "IncludeHelpInContents",
        ],
        "Catalog" => &[
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
        "ChartOfAccounts" => &[
            "UseStandardCommands",
            "IncludeHelpInContents",
            "CheckUnique",
            "QuickChoice",
            "AutoOrderByCode",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfCalculationTypes" => &[
            "UseStandardCommands",
            "QuickChoice",
            "ActionPeriodUse",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "ChartOfCharacteristicTypes" => &[
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
        "Command" => &["ModifiesData"],
        "CommonModule" => &[
            "Global",
            "ClientManagedApplication",
            "Server",
            "ExternalConnection",
            "ClientOrdinaryApplication",
            "Client",
            "ServerCall",
            "Privileged",
        ],
        "Constant" => &[
            "UseStandardCommands",
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "DataProcessor" | "DocumentJournal" | "Report" => {
            &["UseStandardCommands", "IncludeHelpInContents"]
        }
        "Dimension" => &[
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
        "Document" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "PostInPrivilegedMode",
            "UnpostInPrivilegedMode",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Enum" => &["UseStandardCommands", "QuickChoice"],
        "ExchangePlan" => &[
            "UseStandardCommands",
            "QuickChoice",
            "DistributedInfoBase",
            "IncludeConfigurationExtensions",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "InformationRegister" => &[
            "UseStandardCommands",
            "MainFilterOnPeriod",
            "IncludeHelpInContents",
            "EnableTotalsSliceFirst",
            "EnableTotalsSliceLast",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        "Operation" => &["Nillable", "Transactioned"],
        "Parameter" => &["Nillable"],
        "Resource" => &[
            "PasswordMode",
            "MarkNegatives",
            "MultiLine",
            "ExtendedEdit",
            "Balance",
            "FillFromFillingValue",
        ],
        "ScheduledJob" => &["Use", "Predefined"],
        "Task" => &[
            "UseStandardCommands",
            "CheckUnique",
            "Autonumbering",
            "IncludeHelpInContents",
            "UpdateDataHistoryImmediatelyAfterWrite",
            "ExecuteAfterWriteDataHistoryVersionProcessing",
        ],
        _ => &[],
    }
}

fn validate_meta_8_3_27_boolean_property_value(
    context: &str,
    object_type: &str,
    property_name: &str,
    value: &str,
) -> Result<(), String> {
    if !meta_8_3_27_boolean_properties(object_type).contains(&property_name) {
        return Ok(());
    }
    if matches!(value, "true" | "false") {
        Ok(())
    } else {
        Err(format!(
            "{context} property {object_type}.{property_name} value '{value}' is not a canonical xs:boolean for the fixed 8.3.27 contract; expected true or false"
        ))
    }
}

pub(crate) fn validate_metadata_8_3_27_boolean_contract(
    xml_text: &str,
    context: &str,
) -> Result<(), String> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = meta_edit_object_node(&document)?;
    if !meta_validate_valid_types().contains(&root_object.tag_name().name()) {
        return Ok(());
    }

    for object in root_object
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let object_type = object.tag_name().name();
        let boolean_properties = meta_8_3_27_boolean_properties(object_type);
        if boolean_properties.is_empty() {
            continue;
        }
        let Some(properties) = meta_info_child(object, "Properties") else {
            continue;
        };
        for property in properties.children().filter(roxmltree::Node::is_element) {
            let property_name = property.tag_name().name();
            if boolean_properties.contains(&property_name) {
                validate_meta_8_3_27_boolean_property_value(
                    context,
                    object_type,
                    property_name,
                    property.text().unwrap_or(""),
                )?;
            }
        }
    }

    Ok(())
}

pub(crate) fn validate_metadata_8_3_27_enum_contract(
    xml_text: &str,
    context: &str,
) -> Result<(), String> {
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = meta_edit_object_node(&document)?;
    if !meta_validate_valid_types().contains(&root_object.tag_name().name()) {
        return Ok(());
    }

    for object in root_object
        .descendants()
        .filter(roxmltree::Node::is_element)
    {
        let Some(properties) = meta_info_child(object, "Properties") else {
            continue;
        };
        for (property_name, allowed) in meta_validate_property_values() {
            let Some(value) =
                meta_info_child_text(properties, property_name).filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !allowed.contains(&value.as_str()) {
                return Err(format!(
                    "{context} property {}.{property_name} value '{value}' is not valid for the fixed 8.3.27 contract; expected one of: {}",
                    object.tag_name().name(),
                    allowed.join(", ")
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn validate_metadata_owner_shape_8_3_27(
    object_path: &Path,
    workspace: &WorkspaceContext,
    operation: &str,
) -> Result<(), String> {
    let xml_text = read_utf8_sig(object_path)?;
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = meta_edit_object_node(&document)?;
    match root_object.tag_name().name() {
        "Configuration" => return validate_cf_owner_path(object_path, workspace),
        "Subsystem" => return validate_subsystem_owner_path(object_path, workspace),
        _ => {}
    }
    validate_metadata_8_3_27_boolean_contract(&xml_text, operation)?;
    validate_metadata_8_3_27_enum_contract(&xml_text, operation)?;

    let options = MetaValidationOptions {
        detailed: true,
        max_errors: 30,
        out_file_label: None,
        out_file: None,
    };
    let run = meta_validate_one_with_scope(
        object_path.to_path_buf(),
        &options,
        workspace,
        MetaValidationScope::PostWriteLocal,
    )?;
    if run.ok {
        Ok(())
    } else {
        Err(format!(
            "{operation} owner metadata validation failed for {}: {}",
            object_path.display(),
            run.errors.join("; ")
        ))
    }
}

fn validate_meta_compile_attr_type(attr: &MetaCompileAttr, context: &str) -> Result<(), String> {
    validate_meta_compile_name(context, &attr.name)?;
    if !attr.fill_checking.is_empty() {
        validate_meta_8_3_27_property_value(context, "FillChecking", &attr.fill_checking)?;
    }
    if !attr.indexing.is_empty() {
        validate_meta_8_3_27_property_value(context, "Indexing", &attr.indexing)?;
    }
    if attr.type_name.trim().is_empty() {
        return Ok(());
    }
    validate_meta_type_union(std::iter::once(attr.type_name.as_str())).map_err(|error| {
        format!(
            "invalid 8.3.27 type for {context} attribute {}: {error}",
            attr.name
        )
    })
}

fn validate_meta_compile_tabular_section_types(
    section: &MetaCompileTabularSection,
    context: &str,
) -> Result<(), String> {
    validate_meta_compile_name(context, &section.name)?;
    for attr in &section.columns {
        validate_meta_compile_attr_type(attr, context)?;
    }
    Ok(())
}

pub(crate) fn meta_compile_catalog_xml(
    defn: &Map<String, Value>,
    obj_name: &str,
    format_version: &str,
) -> Result<(String, String), String> {
    let mut next_uuid = fresh_meta_compile_uuid;
    let obj_uuid = next_uuid();
    let synonym = defn
        .get("synonym")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(obj_name));

    let mut lines = Vec::<String>::new();
    lines.push("<?xml version=\"1.0\" encoding=\"UTF-8\"?>".to_string());
    lines.push(format!(
        "<MetaDataObject {} version=\"{}\">",
        meta_xmlns_decl(),
        escape_xml(format_version)
    ));
    lines.push(format!("\t<Catalog uuid=\"{obj_uuid}\">"));
    emit_meta_internal_info(&mut lines, "\t\t", "Catalog", obj_name, &mut next_uuid);
    lines.push("\t\t<Properties>".to_string());
    emit_meta_catalog_properties(&mut lines, "\t\t\t", defn, obj_name, &synonym);
    lines.push("\t\t</Properties>".to_string());

    let attrs = meta_compile_attributes(defn.get("attributes"));
    let tabular_sections = meta_compile_tabular_sections(defn.get("tabularSections"))?;
    if attrs.is_empty() && tabular_sections.is_empty() {
        lines.push("\t\t<ChildObjects/>".to_string());
    } else {
        lines.push("\t\t<ChildObjects>".to_string());
        for attr in &attrs {
            emit_meta_attribute(&mut lines, "\t\t\t", attr, "catalog", &mut next_uuid);
        }
        for section in &tabular_sections {
            emit_meta_tabular_section(
                &mut lines,
                "\t\t\t",
                section,
                "Catalog",
                obj_name,
                &mut next_uuid,
            );
        }
        lines.push("\t\t</ChildObjects>".to_string());
    }

    lines.push("\t</Catalog>".to_string());
    lines.push("</MetaDataObject>".to_string());
    Ok((format!("{}\n", lines.join("\n")), obj_uuid))
}

pub(crate) fn meta_xmlns_decl() -> &'static str {
    "xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:cmi=\"http://v8.1c.ru/8.2/managed-application/cmi\" xmlns:ent=\"http://v8.1c.ru/8.1/data/enterprise\" xmlns:lf=\"http://v8.1c.ru/8.2/managed-application/logform\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:xen=\"http://v8.1c.ru/8.3/xcf/enums\" xmlns:xpr=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\""
}

pub(crate) fn metadata_generated_types_8_3_27(
    object_type: &str,
) -> Option<&'static [(&'static str, &'static str)]> {
    match object_type {
        "Catalog" => Some(&[
            ("CatalogObject", "Object"),
            ("CatalogRef", "Ref"),
            ("CatalogSelection", "Selection"),
            ("CatalogList", "List"),
            ("CatalogManager", "Manager"),
        ]),
        "Document" => Some(&[
            ("DocumentObject", "Object"),
            ("DocumentRef", "Ref"),
            ("DocumentSelection", "Selection"),
            ("DocumentList", "List"),
            ("DocumentManager", "Manager"),
        ]),
        "BusinessProcess" => Some(&[
            ("BusinessProcessObject", "Object"),
            ("BusinessProcessRef", "Ref"),
            ("BusinessProcessSelection", "Selection"),
            ("BusinessProcessList", "List"),
            ("BusinessProcessManager", "Manager"),
            ("BusinessProcessRoutePointRef", "RoutePointRef"),
        ]),
        "Enum" => Some(&[
            ("EnumRef", "Ref"),
            ("EnumManager", "Manager"),
            ("EnumList", "List"),
        ]),
        "Constant" => Some(&[
            ("ConstantManager", "Manager"),
            ("ConstantValueManager", "ValueManager"),
            ("ConstantValueKey", "ValueKey"),
        ]),
        "InformationRegister" => Some(&[
            ("InformationRegisterRecord", "Record"),
            ("InformationRegisterManager", "Manager"),
            ("InformationRegisterSelection", "Selection"),
            ("InformationRegisterList", "List"),
            ("InformationRegisterRecordSet", "RecordSet"),
            ("InformationRegisterRecordKey", "RecordKey"),
            ("InformationRegisterRecordManager", "RecordManager"),
        ]),
        "AccumulationRegister" => Some(&[
            ("AccumulationRegisterRecord", "Record"),
            ("AccumulationRegisterManager", "Manager"),
            ("AccumulationRegisterSelection", "Selection"),
            ("AccumulationRegisterList", "List"),
            ("AccumulationRegisterRecordSet", "RecordSet"),
            ("AccumulationRegisterRecordKey", "RecordKey"),
        ]),
        "AccountingRegister" => Some(&[
            ("AccountingRegisterRecord", "Record"),
            ("AccountingRegisterExtDimensions", "ExtDimensions"),
            ("AccountingRegisterRecordSet", "RecordSet"),
            ("AccountingRegisterRecordKey", "RecordKey"),
            ("AccountingRegisterSelection", "Selection"),
            ("AccountingRegisterList", "List"),
            ("AccountingRegisterManager", "Manager"),
        ]),
        "CalculationRegister" => Some(&[
            ("CalculationRegisterRecord", "Record"),
            ("CalculationRegisterManager", "Manager"),
            ("CalculationRegisterSelection", "Selection"),
            ("CalculationRegisterList", "List"),
            ("CalculationRegisterRecordSet", "RecordSet"),
            ("CalculationRegisterRecordKey", "RecordKey"),
            ("RecalculationsManager", "Recalcs"),
        ]),
        "ChartOfAccounts" => Some(&[
            ("ChartOfAccountsObject", "Object"),
            ("ChartOfAccountsRef", "Ref"),
            ("ChartOfAccountsSelection", "Selection"),
            ("ChartOfAccountsList", "List"),
            ("ChartOfAccountsManager", "Manager"),
            ("ChartOfAccountsExtDimensionTypes", "ExtDimensionTypes"),
            (
                "ChartOfAccountsExtDimensionTypesRow",
                "ExtDimensionTypesRow",
            ),
        ]),
        "ChartOfCharacteristicTypes" => Some(&[
            ("ChartOfCharacteristicTypesObject", "Object"),
            ("ChartOfCharacteristicTypesRef", "Ref"),
            ("ChartOfCharacteristicTypesSelection", "Selection"),
            ("ChartOfCharacteristicTypesList", "List"),
            ("Characteristic", "Characteristic"),
            ("ChartOfCharacteristicTypesManager", "Manager"),
        ]),
        "ChartOfCalculationTypes" => Some(&[
            ("ChartOfCalculationTypesObject", "Object"),
            ("ChartOfCalculationTypesRef", "Ref"),
            ("ChartOfCalculationTypesSelection", "Selection"),
            ("ChartOfCalculationTypesList", "List"),
            ("ChartOfCalculationTypesManager", "Manager"),
            ("DisplacingCalculationTypes", "DisplacingCalculationTypes"),
            (
                "DisplacingCalculationTypesRow",
                "DisplacingCalculationTypesRow",
            ),
            ("BaseCalculationTypes", "BaseCalculationTypes"),
            ("BaseCalculationTypesRow", "BaseCalculationTypesRow"),
            ("LeadingCalculationTypes", "LeadingCalculationTypes"),
            ("LeadingCalculationTypesRow", "LeadingCalculationTypesRow"),
        ]),
        "Report" => Some(&[("ReportObject", "Object"), ("ReportManager", "Manager")]),
        "DataProcessor" => Some(&[
            ("DataProcessorObject", "Object"),
            ("DataProcessorManager", "Manager"),
        ]),
        "Task" => Some(&[
            ("TaskObject", "Object"),
            ("TaskRef", "Ref"),
            ("TaskSelection", "Selection"),
            ("TaskList", "List"),
            ("TaskManager", "Manager"),
        ]),
        "ExchangePlan" => Some(&[
            ("ExchangePlanObject", "Object"),
            ("ExchangePlanRef", "Ref"),
            ("ExchangePlanSelection", "Selection"),
            ("ExchangePlanList", "List"),
            ("ExchangePlanManager", "Manager"),
        ]),
        "DocumentJournal" => Some(&[
            ("DocumentJournalSelection", "Selection"),
            ("DocumentJournalList", "List"),
            ("DocumentJournalManager", "Manager"),
        ]),
        "FilterCriterion" => Some(&[
            ("FilterCriterionManager", "Manager"),
            ("FilterCriterionList", "List"),
        ]),
        "SettingsStorage" => Some(&[("SettingsStorageManager", "Manager")]),
        "Sequence" => Some(&[("SequenceRecordSet", "RecordSet")]),
        "IntegrationService" => Some(&[("IntegrationServiceManager", "Manager")]),
        "DefinedType" => Some(&[("DefinedType", "DefinedType")]),
        "Language"
        | "Subsystem"
        | "StyleItem"
        | "Style"
        | "CommonPicture"
        | "SessionParameter"
        | "Role"
        | "CommonTemplate"
        | "CommonModule"
        | "Bot"
        | "CommonAttribute"
        | "XDTOPackage"
        | "WebService"
        | "HTTPService"
        | "WSReference"
        | "EventSubscription"
        | "ScheduledJob"
        | "FunctionalOption"
        | "FunctionalOptionsParameter"
        | "CommonCommand"
        | "CommandGroup"
        | "CommonForm"
        | "DocumentNumerator" => Some(&[]),
        _ => None,
    }
}

pub(crate) fn emit_meta_internal_info<F>(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    object_name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let Some(generated) = metadata_generated_types_8_3_27(object_type) else {
        return;
    };
    if generated.is_empty() {
        return;
    }
    lines.push(format!("{indent}<InternalInfo>"));
    if object_type == "ExchangePlan" {
        lines.push(format!(
            "{indent}\t<xr:ThisNode>{}</xr:ThisNode>",
            next_uuid()
        ));
    }
    for (prefix, category) in generated {
        let generated_name = escape_xml(&format!("{prefix}.{object_name}"));
        lines.push(format!(
            "{indent}\t<xr:GeneratedType name=\"{generated_name}\" category=\"{}\">",
            escape_xml(category)
        ));
        lines.push(format!(
            "{indent}\t\t<xr:TypeId>{}</xr:TypeId>",
            next_uuid()
        ));
        lines.push(format!(
            "{indent}\t\t<xr:ValueId>{}</xr:ValueId>",
            next_uuid()
        ));
        lines.push(format!("{indent}\t</xr:GeneratedType>"));
    }
    lines.push(format!("{indent}</InternalInfo>"));
}

pub(crate) fn emit_meta_catalog_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let hierarchical = defn.get("hierarchical").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<Hierarchical>{hierarchical}</Hierarchical>"
    ));
    lines.push(format!(
        "{indent}<HierarchyType>{}</HierarchyType>",
        meta_enum_prop(defn, "hierarchyType", "HierarchyFoldersAndItems")
    ));
    let limit_level_count = defn.get("limitLevelCount").and_then(Value::as_bool) == Some(true);
    let level_count = defn.get("levelCount").and_then(json_i64_value).unwrap_or(2);
    let folders_on_top = defn.get("foldersOnTop").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<LimitLevelCount>{limit_level_count}</LimitLevelCount>"
    ));
    lines.push(format!("{indent}<LevelCount>{level_count}</LevelCount>"));
    lines.push(format!(
        "{indent}<FoldersOnTop>{folders_on_top}</FoldersOnTop>"
    ));
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<Owners/>"));
    lines.push(format!(
        "{indent}<SubordinationUse>{}</SubordinationUse>",
        meta_enum_prop(defn, "subordinationUse", "ToItems")
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(25);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeType>{}</CodeType>",
        meta_enum_prop(defn, "codeType", "String")
    ));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        meta_enum_prop(defn, "codeSeries", "WholeCatalog")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "Catalog");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    let quick_choice = defn.get("quickChoice").and_then(Value::as_bool) == Some(true);
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        meta_enum_prop(defn, "choiceMode", "BothWays")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("Catalog.{obj_name}.StandardAttribute.Description"))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("Catalog.{obj_name}.StandardAttribute.Code"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    lines.push(format!(
        "{indent}<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>"
    ));
    lines.push(format!(
        "{indent}<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>"
    ));
    for tag in [
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
    ] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    for line in [
        "<BasedOn/>",
        "<DataLockFields/>",
        "<DataLockControlMode>Automatic</DataLockControlMode>",
        "<FullTextSearch>Use</FullTextSearch>",
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn meta_compile_synonym(defn: &Map<String, Value>, obj_name: &str) -> String {
    defn.get("synonym")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(obj_name))
}

pub(crate) fn emit_meta_base_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    lines.push(format!("{indent}<Name>{}</Name>", escape_xml(obj_name)));
    emit_meta_mltext(lines, indent, "Synonym", synonym);
    match defn.get("comment").and_then(Value::as_str) {
        Some(comment) if !comment.is_empty() => {
            lines.push(format!(
                "{indent}<Comment>{}</Comment>",
                escape_xml(comment)
            ));
        }
        _ => lines.push(format!("{indent}<Comment/>")),
    }
}

pub(crate) fn emit_meta_enum_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>false</UseStandardCommands>"
    ));
    emit_meta_standard_attributes(lines, indent, "Enum");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!("{indent}<QuickChoice>false</QuickChoice>"));
    lines.push(format!("{indent}<ChoiceMode>BothWays</ChoiceMode>"));
    for tag in [
        "DefaultListForm",
        "DefaultChoiceForm",
        "AuxiliaryListForm",
        "AuxiliaryChoiceForm",
    ] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
    lines.push(format!(
        "{indent}<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>"
    ));
}

pub(crate) fn emit_meta_constant_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let value_type = meta_compile_root_value_type(defn);
    emit_meta_value_type(lines, indent, &value_type);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    for tag in ["DefaultForm", "ExtendedPresentation", "Explanation"] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    for line in [
        "<PasswordMode>false</PasswordMode>",
        "<Format/>",
        "<EditFormat/>",
        "<ToolTip/>",
        "<MarkNegatives>false</MarkNegatives>",
        "<Mask/>",
        "<MultiLine>false</MultiLine>",
        "<ExtendedEdit>false</ExtendedEdit>",
        "<MinValue xsi:nil=\"true\"/>",
        "<MaxValue xsi:nil=\"true\"/>",
        "<FillChecking>DontCheck</FillChecking>",
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<ChoiceForm/>",
        "<LinkByType/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    for line in [
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_document_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<Numerator/>"));
    lines.push(format!(
        "{indent}<NumberType>{}</NumberType>",
        meta_enum_prop(defn, "numberType", "String")
    ));
    let number_length = defn
        .get("numberLength")
        .and_then(json_i64_value)
        .unwrap_or(11);
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        meta_enum_prop(defn, "numberAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<NumberPeriodicity>{}</NumberPeriodicity>",
        meta_enum_prop(defn, "numberPeriodicity", "Year")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) != Some(false);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    emit_meta_standard_attributes(lines, indent, "Document");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!("{indent}<BasedOn/>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("Document.{obj_name}.StandardAttribute.Number"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<Posting>{}</Posting>",
        meta_enum_prop(defn, "posting", "Allow")
    ));
    lines.push(format!(
        "{indent}<RealTimePosting>{}</RealTimePosting>",
        meta_enum_prop(defn, "realTimePosting", "Deny")
    ));
    lines.push(format!(
        "{indent}<RegisterRecordsDeletion>{}</RegisterRecordsDeletion>",
        meta_enum_prop(defn, "registerRecordsDeletion", "AutoDelete")
    ));
    lines.push(format!(
        "{indent}<RegisterRecordsWritingOnPost>{}</RegisterRecordsWritingOnPost>",
        meta_enum_prop(defn, "registerRecordsWritingOnPost", "WriteModified")
    ));
    lines.push(format!(
        "{indent}<SequenceFilling>{}</SequenceFilling>",
        escape_xml(
            defn.get("sequenceFilling")
                .and_then(Value::as_str)
                .unwrap_or("AutoFill")
        )
    ));
    emit_meta_md_object_refs(
        lines,
        indent,
        "RegisterRecords",
        &meta_compile_string_list(defn.get("registerRecords")),
    );
    let post_in_privileged =
        defn.get("postInPrivilegedMode").and_then(Value::as_bool) != Some(false);
    let unpost_in_privileged =
        defn.get("unpostInPrivilegedMode").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<PostInPrivilegedMode>{post_in_privileged}</PostInPrivilegedMode>"
    ));
    lines.push(format!(
        "{indent}<UnpostInPrivilegedMode>{unpost_in_privileged}</UnpostInPrivilegedMode>"
    ));
    emit_meta_lock_search_presentation_tail(lines, indent, "Use");
}

pub(crate) fn emit_meta_information_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    let _ = obj_name;
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    for tag in [
        "DefaultRecordForm",
        "DefaultListForm",
        "AuxiliaryRecordForm",
        "AuxiliaryListForm",
    ] {
        lines.push(format!("{indent}<{tag}/>"));
    }
    emit_meta_standard_attributes(lines, indent, "InformationRegister");
    let periodicity = meta_enum_prop(defn, "periodicity", "Nonperiodical");
    let write_mode = meta_enum_prop(defn, "writeMode", "Independent");
    let main_filter_on_period = defn
        .get("mainFilterOnPeriod")
        .and_then(Value::as_bool)
        .unwrap_or(periodicity != "Nonperiodical");
    lines.push(format!(
        "{indent}<InformationRegisterPeriodicity>{periodicity}</InformationRegisterPeriodicity>"
    ));
    lines.push(format!("{indent}<WriteMode>{write_mode}</WriteMode>"));
    lines.push(format!(
        "{indent}<MainFilterOnPeriod>{main_filter_on_period}</MainFilterOnPeriod>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<EnableTotalsSliceFirst>false</EnableTotalsSliceFirst>",
        "<EnableTotalsSliceLast>false</EnableTotalsSliceLast>",
        "<RecordPresentation/>",
        "<ExtendedRecordPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_accumulation_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<DefaultListForm/>"));
    lines.push(format!("{indent}<AuxiliaryListForm/>"));
    lines.push(format!(
        "{indent}<RegisterType>{}</RegisterType>",
        meta_enum_prop(defn, "registerType", "Balance")
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_standard_attributes(lines, indent, "AccumulationRegister");
    emit_meta_register_tail(lines, indent, defn);
    let enable_totals_splitting =
        defn.get("enableTotalsSplitting").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<EnableTotalsSplitting>{enable_totals_splitting}</EnableTotalsSplitting>"
    ));
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(crate) fn emit_meta_accounting_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "ChartOfAccounts",
        defn.get("chartOfAccounts").and_then(Value::as_str),
    );
    let correspondence = defn.get("correspondence").and_then(Value::as_bool) == Some(true);
    let period_adjustment_length = defn
        .get("periodAdjustmentLength")
        .and_then(json_i64_value)
        .unwrap_or(0);
    lines.push(format!(
        "{indent}<Correspondence>{correspondence}</Correspondence>"
    ));
    lines.push(format!(
        "{indent}<PeriodAdjustmentLength>{period_adjustment_length}</PeriodAdjustmentLength>"
    ));
    lines.push(format!("{indent}<DefaultListForm/>"));
    lines.push(format!("{indent}<AuxiliaryListForm/>"));
    emit_meta_standard_attributes(lines, indent, "AccountingRegister");
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<EnableTotalsSplitting>false</EnableTotalsSplitting>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(crate) fn emit_meta_calculation_register_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!("{indent}<DefaultListForm/>"));
    lines.push(format!("{indent}<AuxiliaryListForm/>"));
    lines.push(format!(
        "{indent}<Periodicity>{}</Periodicity>",
        meta_enum_prop(defn, "periodicity", "Month")
    ));
    let action_period = defn.get("actionPeriod").and_then(Value::as_bool) == Some(true);
    let base_period = defn.get("basePeriod").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<ActionPeriod>{action_period}</ActionPeriod>"
    ));
    lines.push(format!("{indent}<BasePeriod>{base_period}</BasePeriod>"));
    emit_meta_optional_text(
        lines,
        indent,
        "Schedule",
        defn.get("schedule").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "ScheduleValue",
        defn.get("scheduleValue").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "ScheduleDate",
        defn.get("scheduleDate").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "ChartOfCalculationTypes",
        defn.get("chartOfCalculationTypes").and_then(Value::as_str),
    );
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_standard_attributes(lines, indent, "CalculationRegister");
    emit_meta_register_tail(lines, indent, defn);
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(crate) fn emit_meta_chart_of_accounts_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<BasedOn/>"));
    let ext_dimension_types = defn
        .get("extDimensionTypes")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    emit_meta_optional_text(lines, indent, "ExtDimensionTypes", ext_dimension_types);
    let max_ext_dimension_count = defn
        .get("maxExtDimensionCount")
        .and_then(json_i64_value)
        .unwrap_or_else(|| if ext_dimension_types.is_some() { 3 } else { 0 });
    lines.push(format!(
        "{indent}<MaxExtDimensionCount>{max_ext_dimension_count}</MaxExtDimensionCount>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "CodeMask",
        defn.get("codeMask").and_then(Value::as_str),
    );
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(8);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(120);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        meta_enum_prop(defn, "codeSeries", "WholeChartOfAccounts")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "ChartOfAccounts");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!("{indent}<StandardTabularSections>"));
    lines.push(format!(
        "{indent}\t<xr:StandardTabularSection name=\"ExtDimensionTypes\">"
    ));
    lines.push(format!("{indent}\t\t<xr:Synonym>"));
    lines.push(format!("{indent}\t\t\t<v8:item>"));
    lines.push(format!("{indent}\t\t\t\t<v8:lang/>"));
    lines.push(format!(
        "{indent}\t\t\t\t<v8:content>Extra dimension types</v8:content>"
    ));
    lines.push(format!("{indent}\t\t\t</v8:item>"));
    lines.push(format!("{indent}\t\t</xr:Synonym>"));
    lines.push(format!("{indent}\t\t<xr:Comment/>"));
    lines.push(format!("{indent}\t\t<xr:ToolTip/>"));
    lines.push(format!(
        "{indent}\t\t<xr:FillChecking>DontCheck</xr:FillChecking>"
    ));
    lines.push(format!("{indent}\t\t<xr:StandardAttributes>"));
    for attr in [
        "TurnoversOnly",
        "Predefined",
        "ExtDimensionType",
        "LineNumber",
    ] {
        emit_meta_standard_attribute(
            lines,
            &format!("{indent}\t\t\t"),
            "ChartOfAccounts.ExtDimensionTypes",
            attr,
        );
    }
    lines.push(format!("{indent}\t\t</xr:StandardAttributes>"));
    lines.push(format!("{indent}\t</xr:StandardTabularSection>"));
    lines.push(format!("{indent}</StandardTabularSections>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    lines.push(format!("{indent}<QuickChoice>false</QuickChoice>"));
    lines.push(format!("{indent}<ChoiceMode>BothWays</ChoiceMode>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfAccounts.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfAccounts.{obj_name}.StandardAttribute.Code"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    let auto_order_by_code = defn.get("autoOrderByCode").and_then(Value::as_bool) != Some(false);
    let order_length = defn
        .get("orderLength")
        .and_then(json_i64_value)
        .unwrap_or(5);
    lines.push(format!(
        "{indent}<AutoOrderByCode>{auto_order_by_code}</AutoOrderByCode>"
    ));
    lines.push(format!("{indent}<OrderLength>{order_length}</OrderLength>"));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_chart_of_characteristic_types_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "CharacteristicExtValues",
        defn.get("characteristicExtValues").and_then(Value::as_str),
    );
    let value_types = meta_compile_value_types(defn);
    if value_types.is_empty() {
        lines.push(format!("{indent}<Type>"));
        emit_meta_type_contents(
            lines,
            &format!("{indent}\t"),
            ["Boolean", "String(100)", "Number(15,2)", "DateTime"],
        );
        lines.push(format!("{indent}</Type>"));
    } else {
        lines.push(format!("{indent}<Type>"));
        emit_meta_type_contents(
            lines,
            &format!("{indent}\t"),
            value_types.iter().map(String::as_str),
        );
        lines.push(format!("{indent}</Type>"));
    }
    let hierarchical = defn.get("hierarchical").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<Hierarchical>{hierarchical}</Hierarchical>"
    ));
    let folders_on_top = defn.get("foldersOnTop").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<FoldersOnTop>{folders_on_top}</FoldersOnTop>"
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(25);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeSeries>{}</CodeSeries>",
        meta_enum_prop(defn, "codeSeries", "WholeCharacteristicKind")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    emit_meta_standard_attributes(lines, indent, "ChartOfCharacteristicTypes");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>{}</PredefinedDataUpdate>",
        meta_enum_prop(defn, "predefinedDataUpdate", "Auto")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        meta_enum_prop(defn, "editType", "InDialog")
    ));
    let quick_choice = defn.get("quickChoice").and_then(Value::as_bool) == Some(true);
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        meta_enum_prop(defn, "choiceMode", "BothWays")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfCharacteristicTypes.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfCharacteristicTypes.{obj_name}.StandardAttribute.Code"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DefaultObjectForm/>",
        "<DefaultFolderForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<DefaultFolderChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryFolderForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<AuxiliaryFolderChoiceForm/>",
        "<BasedOn/>",
        "<DataLockFields/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_chart_of_calculation_types_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(25);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<CodeType>{}</CodeType>",
        meta_enum_prop(defn, "codeType", "String")
    ));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    lines.push(format!("{indent}<QuickChoice>false</QuickChoice>"));
    lines.push(format!("{indent}<ChoiceMode>BothWays</ChoiceMode>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfCalculationTypes.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ChartOfCalculationTypes.{obj_name}.StandardAttribute.Code"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<BasedOn/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DependenceOnCalculationTypes>{}</DependenceOnCalculationTypes>",
        meta_enum_prop(defn, "dependenceOnCalculationTypes", "DontUse")
    ));
    emit_meta_md_object_refs(
        lines,
        indent,
        "BaseCalculationTypes",
        &meta_compile_string_list(defn.get("baseCalculationTypes")),
    );
    let action_period_use = defn.get("actionPeriodUse").and_then(Value::as_bool) == Some(true);
    lines.push(format!(
        "{indent}<ActionPeriodUse>{action_period_use}</ActionPeriodUse>"
    ));
    emit_meta_standard_attributes(lines, indent, "ChartOfCalculationTypes");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_business_process_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        meta_enum_prop(defn, "editType", "InDialog")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "BusinessProcess.{obj_name}.StandardAttribute.Number"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<NumberType>{}</NumberType>",
        meta_enum_prop(defn, "numberType", "String")
    ));
    let number_length = defn
        .get("numberLength")
        .and_then(json_i64_value)
        .unwrap_or(11);
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        meta_enum_prop(defn, "numberAllowedLength", "Variable")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    emit_meta_standard_attributes(lines, indent, "BusinessProcess");
    lines.push(format!("{indent}<Characteristics/>"));
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
    lines.push(format!("{indent}<BasedOn/>"));
    lines.push(format!(
        "{indent}<NumberPeriodicity>{}</NumberPeriodicity>",
        meta_enum_prop(defn, "numberPeriodicity", "Nonperiodical")
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "Task",
        defn.get("task").and_then(Value::as_str),
    );
    let privileged = defn
        .get("createTaskInPrivilegedMode")
        .and_then(Value::as_bool)
        != Some(false);
    lines.push(format!(
        "{indent}<CreateTaskInPrivilegedMode>{privileged}</CreateTaskInPrivilegedMode>"
    ));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_task_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    emit_meta_number_properties(lines, indent, defn, 14);
    lines.push(format!(
        "{indent}<TaskNumberAutoPrefix>{}</TaskNumberAutoPrefix>",
        escape_xml(
            defn.get("taskNumberAutoPrefix")
                .and_then(Value::as_str)
                .unwrap_or("BusinessProcessNumber")
        )
    ));
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(150);
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "Addressing",
        defn.get("addressing").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "MainAddressingAttribute",
        defn.get("mainAddressingAttribute").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "CurrentPerformer",
        defn.get("currentPerformer").and_then(Value::as_str),
    );
    lines.push(format!("{indent}<BasedOn/>"));
    emit_meta_standard_attributes(lines, indent, "Task");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        meta_enum_prop(defn, "editType", "InDialog")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("Task.{obj_name}.StandardAttribute.Number"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<DataLockFields/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_exchange_plan_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    let code_length = defn.get("codeLength").and_then(json_i64_value).unwrap_or(9);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(100);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
    lines.push(format!(
        "{indent}<EditType>{}</EditType>",
        meta_enum_prop(defn, "editType", "InDialog")
    ));
    let quick_choice = defn.get("quickChoice").and_then(Value::as_bool) == Some(true);
    lines.push(format!("{indent}<QuickChoice>{quick_choice}</QuickChoice>"));
    lines.push(format!(
        "{indent}<ChoiceMode>{}</ChoiceMode>",
        meta_enum_prop(defn, "choiceMode", "BothWays")
    ));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "ExchangePlan.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("ExchangePlan.{obj_name}.StandardAttribute.Code"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    emit_meta_standard_attributes(lines, indent, "ExchangePlan");
    lines.push(format!("{indent}<Characteristics/>"));
    lines.push(format!("{indent}<BasedOn/>"));
    let distributed = defn.get("distributedInfoBase").and_then(Value::as_bool) == Some(true);
    let include_ext = defn
        .get("includeConfigurationExtensions")
        .and_then(Value::as_bool)
        == Some(true);
    lines.push(format!(
        "{indent}<DistributedInfoBase>{distributed}</DistributedInfoBase>"
    ));
    lines.push(format!(
        "{indent}<IncludeConfigurationExtensions>{include_ext}</IncludeConfigurationExtensions>"
    ));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<DataLockFields/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_document_journal_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    for tag in ["DefaultForm", "AuxiliaryForm"] {
        let field = if tag == "DefaultForm" {
            "defaultForm"
        } else {
            "auxiliaryForm"
        };
        emit_meta_optional_text(lines, indent, tag, defn.get(field).and_then(Value::as_str));
    }
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    emit_meta_md_object_refs(
        lines,
        indent,
        "RegisteredDocuments",
        &meta_compile_string_list(defn.get("registeredDocuments")),
    );
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    emit_meta_standard_attributes(lines, indent, "DocumentJournal");
    lines.push(format!("{indent}<ListPresentation/>"));
    lines.push(format!("{indent}<ExtendedListPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(crate) fn emit_meta_report_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>true</UseStandardCommands>"
    ));
    for (tag, field) in [
        ("DefaultForm", "defaultForm"),
        ("AuxiliaryForm", "auxiliaryForm"),
        ("MainDataCompositionSchema", "mainDataCompositionSchema"),
        ("DefaultSettingsForm", "defaultSettingsForm"),
        ("AuxiliarySettingsForm", "auxiliarySettingsForm"),
        ("DefaultVariantForm", "defaultVariantForm"),
    ] {
        emit_meta_optional_text(lines, indent, tag, defn.get(field).and_then(Value::as_str));
    }
    for line in [
        "<VariantsStorage/>",
        "<SettingsStorage/>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<ExtendedPresentation/>",
        "<Explanation/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_data_processor_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    lines.push(format!(
        "{indent}<UseStandardCommands>false</UseStandardCommands>"
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "DefaultForm",
        defn.get("defaultForm").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "AuxiliaryForm",
        defn.get("auxiliaryForm").and_then(Value::as_str),
    );
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<ExtendedPresentation/>"));
    lines.push(format!("{indent}<Explanation/>"));
}

pub(crate) fn emit_meta_scheduled_job_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let method_name = meta_compile_common_module_method(
        defn.get("methodName")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    lines.push(format!(
        "{indent}<MethodName>{}</MethodName>",
        escape_xml(&method_name)
    ));
    let description = defn
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or(synonym);
    lines.push(format!(
        "{indent}<Description>{}</Description>",
        escape_xml(description)
    ));
    emit_meta_optional_text(
        lines,
        indent,
        "Key",
        defn.get("key").and_then(Value::as_str),
    );
    let use_job = defn.get("use").and_then(Value::as_bool) == Some(true);
    let predefined = defn.get("predefined").and_then(Value::as_bool) == Some(true);
    let restart_count = defn
        .get("restartCountOnFailure")
        .and_then(json_i64_value)
        .unwrap_or(3);
    let restart_interval = defn
        .get("restartIntervalOnFailure")
        .and_then(json_i64_value)
        .unwrap_or(10);
    lines.push(format!("{indent}<Use>{use_job}</Use>"));
    lines.push(format!("{indent}<Predefined>{predefined}</Predefined>"));
    lines.push(format!(
        "{indent}<RestartCountOnFailure>{restart_count}</RestartCountOnFailure>"
    ));
    lines.push(format!(
        "{indent}<RestartIntervalOnFailure>{restart_interval}</RestartIntervalOnFailure>"
    ));
}

pub(crate) fn emit_meta_event_subscription_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let sources = meta_compile_string_list(defn.get("source"));
    if sources.is_empty() {
        lines.push(format!("{indent}<Source/>"));
    } else {
        lines.push(format!("{indent}<Source>"));
        emit_meta_event_subscription_source_type_contents(
            lines,
            &format!("{indent}\t"),
            sources.iter().map(String::as_str),
        );
        lines.push(format!("{indent}</Source>"));
    }
    lines.push(format!(
        "{indent}<Event>{}</Event>",
        escape_xml(
            defn.get("event")
                .and_then(Value::as_str)
                .unwrap_or("BeforeWrite")
        )
    ));
    let handler = meta_compile_common_module_method(
        defn.get("handler")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    lines.push(format!(
        "{indent}<Handler>{}</Handler>",
        escape_xml(&handler)
    ));
}

pub(crate) fn emit_meta_http_service_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let root_url = defn
        .get("rootURL")
        .or_else(|| defn.get("rootUrl"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| obj_name.to_lowercase());
    lines.push(format!(
        "{indent}<RootURL>{}</RootURL>",
        escape_xml(&root_url)
    ));
    lines.push(format!(
        "{indent}<ReuseSessions>{}</ReuseSessions>",
        meta_enum_prop(defn, "reuseSessions", "DontUse")
    ));
    let session_max_age = defn
        .get("sessionMaxAge")
        .and_then(json_i64_value)
        .unwrap_or(20);
    lines.push(format!(
        "{indent}<SessionMaxAge>{session_max_age}</SessionMaxAge>"
    ));
}

pub(crate) fn emit_meta_web_service_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    emit_meta_optional_text(
        lines,
        indent,
        "Namespace",
        defn.get("namespace").and_then(Value::as_str),
    );
    emit_meta_optional_text(
        lines,
        indent,
        "XDTOPackages",
        defn.get("xdtoPackages").and_then(Value::as_str),
    );
    lines.push(format!(
        "{indent}<DescriptorFileName>{}</DescriptorFileName>",
        escape_xml(
            defn.get("descriptorFileName")
                .and_then(Value::as_str)
                .unwrap_or("ws1.1cws")
        )
    ));
    lines.push(format!(
        "{indent}<ReuseSessions>{}</ReuseSessions>",
        meta_enum_prop(defn, "reuseSessions", "DontUse")
    ));
    let session_max_age = defn
        .get("sessionMaxAge")
        .and_then(json_i64_value)
        .unwrap_or(20);
    lines.push(format!(
        "{indent}<SessionMaxAge>{session_max_age}</SessionMaxAge>"
    ));
}

pub(crate) fn meta_compile_root_value_type(defn: &Map<String, Value>) -> String {
    let mut type_name = defn
        .get("valueType")
        .and_then(Value::as_str)
        .unwrap_or("String")
        .to_string();
    if !type_name.is_empty() && !type_name.contains('(') {
        if type_name == "String" {
            if let Some(length) = defn.get("length").and_then(json_i64_value) {
                type_name = format!("String({length})");
            }
        } else if type_name == "Number" {
            if let Some(length) = defn.get("length").and_then(json_i64_value) {
                let precision = defn.get("precision").and_then(json_i64_value).unwrap_or(0);
                let nn = if defn.get("nonneg").and_then(Value::as_bool) == Some(true)
                    || defn.get("nonnegative").and_then(Value::as_bool) == Some(true)
                {
                    ",nonneg"
                } else {
                    ""
                };
                type_name = format!("Number({length},{precision}{nn})");
            }
        }
    }
    type_name
}

pub(crate) fn emit_meta_common_module_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let context = defn
        .get("context")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut server = bool_arg_from_json(defn, "server");
    let mut server_call = bool_arg_from_json(defn, "serverCall");
    let mut client_managed = bool_arg_from_json(defn, "clientManagedApplication");
    match context {
        "server" | "serverCall" => {
            server = true;
            server_call = true;
        }
        "client" => client_managed = true,
        "serverClient" => {
            server = true;
            client_managed = true;
        }
        _ => {}
    }
    lines.push(format!(
        "{indent}<Global>{}</Global>",
        bool_arg_from_json(defn, "global")
    ));
    lines.push(format!(
        "{indent}<ClientManagedApplication>{client_managed}</ClientManagedApplication>"
    ));
    lines.push(format!("{indent}<Server>{server}</Server>"));
    lines.push(format!(
        "{indent}<ExternalConnection>{}</ExternalConnection>",
        bool_arg_from_json(defn, "externalConnection")
    ));
    lines.push(format!(
        "{indent}<ClientOrdinaryApplication>{}</ClientOrdinaryApplication>",
        bool_arg_from_json(defn, "clientOrdinaryApplication")
    ));
    lines.push(format!("{indent}<ServerCall>{server_call}</ServerCall>"));
    lines.push(format!(
        "{indent}<Privileged>{}</Privileged>",
        bool_arg_from_json(defn, "privileged")
    ));
    lines.push(format!(
        "{indent}<ReturnValuesReuse>{}</ReturnValuesReuse>",
        meta_enum_prop(defn, "returnValuesReuse", "DontUse")
    ));
}

pub(crate) fn emit_meta_defined_type_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_name: &str,
    synonym: &str,
) {
    emit_meta_base_properties(lines, indent, defn, obj_name, synonym);
    let value_types = meta_compile_value_types(defn);
    if value_types.is_empty() {
        lines.push(format!("{indent}<Type/>"));
        return;
    }
    lines.push(format!("{indent}<Type>"));
    emit_meta_type_contents(
        lines,
        &format!("{indent}\t"),
        value_types.iter().map(String::as_str),
    );
    lines.push(format!("{indent}</Type>"));
}

pub(crate) fn bool_arg_from_json(defn: &Map<String, Value>, field_name: &str) -> bool {
    defn.get(field_name).and_then(Value::as_bool) == Some(true)
}

pub(crate) fn meta_compile_value_types(defn: &Map<String, Value>) -> Vec<String> {
    let value = defn.get("valueTypes").or_else(|| defn.get("valueType"));
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect(),
        Some(Value::String(value)) if !value.is_empty() => vec![value.to_string()],
        _ => Vec::new(),
    }
}

pub(crate) fn emit_meta_optional_text(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    value: Option<&str>,
) {
    match value.filter(|value| !value.is_empty()) {
        Some(value) => lines.push(format!("{indent}<{tag}>{}</{tag}>", escape_xml(value))),
        None => lines.push(format!("{indent}<{tag}/>")),
    }
}

pub(crate) fn meta_compile_string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                if let Some(text) = item.as_str() {
                    Some(text.to_string())
                } else {
                    item.as_object()
                        .and_then(|object| object.get("name"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }
            })
            .collect(),
        Some(Value::String(value)) if !value.is_empty() => vec![value.to_string()],
        Some(Value::Object(object)) => object.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn meta_compile_named_items(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.as_str().map(ToOwned::to_owned).or_else(|| {
                    item.as_object()
                        .and_then(|object| object.get("name"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
            })
            .collect(),
        Some(Value::Object(object)) => object.keys().cloned().collect(),
        Some(Value::String(value)) if !value.is_empty() => vec![value.to_string()],
        _ => Vec::new(),
    }
}

pub(crate) fn normalize_meta_object_ref(value: &str) -> String {
    let Some((prefix, suffix)) = value.split_once('.') else {
        return value.to_string();
    };
    let normalized = normalize_meta_object_type(prefix);
    format!("{normalized}.{suffix}")
}

pub(crate) fn emit_meta_md_object_refs(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    refs: &[String],
) {
    if refs.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>"));
    for item in refs {
        lines.push(format!(
            "{indent}\t<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
            escape_xml(&normalize_meta_object_ref(item))
        ));
    }
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn meta_compile_common_module_method(value: &str) -> String {
    if value.is_empty() || value.starts_with("CommonModule.") {
        value.to_string()
    } else {
        format!("CommonModule.{value}")
    }
}

pub(crate) fn emit_meta_lock_search_presentation_tail(
    lines: &mut Vec<String>,
    indent: &str,
    full_text_search_default: &str,
) {
    lines.push(format!(
        "{indent}<IncludeHelpInContents>false</IncludeHelpInContents>"
    ));
    lines.push(format!("{indent}<DataLockFields/>"));
    lines.push(format!(
        "{indent}<DataLockControlMode>Automatic</DataLockControlMode>"
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        escape_xml(full_text_search_default)
    ));
    for line in [
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_register_tail(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
) {
    lines.push(format!(
        "{indent}<DataLockControlMode>{}</DataLockControlMode>",
        meta_enum_prop(defn, "dataLockControlMode", "Automatic")
    ));
    lines.push(format!(
        "{indent}<FullTextSearch>{}</FullTextSearch>",
        meta_enum_prop(defn, "fullTextSearch", "Use")
    ));
}

pub(crate) fn emit_meta_code_description_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    default_code_length: i64,
    default_description_length: i64,
    include_check_unique: bool,
    include_autonumbering: bool,
) {
    let code_length = defn
        .get("codeLength")
        .and_then(json_i64_value)
        .unwrap_or(default_code_length);
    let description_length = defn
        .get("descriptionLength")
        .and_then(json_i64_value)
        .unwrap_or(default_description_length);
    lines.push(format!("{indent}<CodeLength>{code_length}</CodeLength>"));
    lines.push(format!(
        "{indent}<CodeAllowedLength>{}</CodeAllowedLength>",
        meta_enum_prop(defn, "codeAllowedLength", "Variable")
    ));
    lines.push(format!(
        "{indent}<DescriptionLength>{description_length}</DescriptionLength>"
    ));
    if include_check_unique {
        let check_unique = defn.get("checkUnique").and_then(Value::as_bool) == Some(true);
        lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    } else {
        lines.push(format!("{indent}<CheckUnique>false</CheckUnique>"));
    }
    if include_autonumbering {
        let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
        lines.push(format!(
            "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
        ));
    }
    lines.push(format!(
        "{indent}<DefaultPresentation>{}</DefaultPresentation>",
        meta_enum_prop(defn, "defaultPresentation", "AsDescription")
    ));
}

pub(crate) fn emit_meta_choice_object_tail(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    obj_name: &str,
    include_characteristics: bool,
) {
    if include_characteristics {
        lines.push(format!("{indent}<Characteristics/>"));
        lines.push(format!(
            "{indent}<PredefinedDataUpdate>Auto</PredefinedDataUpdate>"
        ));
    }
    lines.push(format!("{indent}<EditType>InDialog</EditType>"));
    lines.push(format!("{indent}<QuickChoice>false</QuickChoice>"));
    lines.push(format!("{indent}<ChoiceMode>BothWays</ChoiceMode>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "{object_type}.{obj_name}.StandardAttribute.Description"
        ))
    ));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!("{object_type}.{obj_name}.StandardAttribute.Code"))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
        "<IncludeHelpInContents>false</IncludeHelpInContents>",
        "<BasedOn/>",
        "<DataLockFields/>",
        "<DataLockControlMode>Automatic</DataLockControlMode>",
        "<FullTextSearch>Use</FullTextSearch>",
        "<ObjectPresentation/>",
        "<ExtendedObjectPresentation/>",
        "<ListPresentation/>",
        "<ExtendedListPresentation/>",
        "<Explanation/>",
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>DontUse</DataHistory>",
        "<UpdateDataHistoryImmediatelyAfterWrite>false</UpdateDataHistoryImmediatelyAfterWrite>",
        "<ExecuteAfterWriteDataHistoryVersionProcessing>false</ExecuteAfterWriteDataHistoryVersionProcessing>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
}

pub(crate) fn emit_meta_number_properties(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    default_number_length: i64,
) {
    lines.push(format!(
        "{indent}<NumberType>{}</NumberType>",
        meta_enum_prop(defn, "numberType", "String")
    ));
    let number_length = defn
        .get("numberLength")
        .and_then(json_i64_value)
        .unwrap_or(default_number_length);
    lines.push(format!(
        "{indent}<NumberLength>{number_length}</NumberLength>"
    ));
    lines.push(format!(
        "{indent}<NumberAllowedLength>{}</NumberAllowedLength>",
        meta_enum_prop(defn, "numberAllowedLength", "Variable")
    ));
    let check_unique = defn.get("checkUnique").and_then(Value::as_bool) != Some(false);
    let autonumbering = defn.get("autonumbering").and_then(Value::as_bool) != Some(false);
    lines.push(format!("{indent}<CheckUnique>{check_unique}</CheckUnique>"));
    lines.push(format!(
        "{indent}<Autonumbering>{autonumbering}</Autonumbering>"
    ));
}

pub(crate) fn emit_meta_numbered_object_tail(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    obj_name: &str,
) {
    lines.push(format!("{indent}<BasedOn/>"));
    lines.push(format!("{indent}<InputByString>"));
    lines.push(format!(
        "{indent}\t<xr:Field>{}</xr:Field>",
        escape_xml(&format!(
            "{object_type}.{obj_name}.StandardAttribute.Number"
        ))
    ));
    lines.push(format!("{indent}</InputByString>"));
    for line in [
        "<CreateOnInput>DontUse</CreateOnInput>",
        "<SearchStringModeOnInputByString>Begin</SearchStringModeOnInputByString>",
        "<FullTextSearchOnInputByString>DontUse</FullTextSearchOnInputByString>",
        "<ChoiceDataGetModeOnInputByString>Directly</ChoiceDataGetModeOnInputByString>",
        "<DefaultObjectForm/>",
        "<DefaultListForm/>",
        "<DefaultChoiceForm/>",
        "<AuxiliaryObjectForm/>",
        "<AuxiliaryListForm/>",
        "<AuxiliaryChoiceForm/>",
    ] {
        lines.push(format!("{indent}{line}"));
    }
    emit_meta_lock_search_presentation_tail(lines, indent, "Use");
}

pub(crate) struct MetaCompileEnumValue {
    pub(crate) name: String,
    pub(crate) synonym: String,
    pub(crate) comment: String,
}

pub(crate) fn meta_compile_enum_values(
    value: Option<&Value>,
) -> Result<Vec<MetaCompileEnumValue>, String> {
    let Some(Value::Array(items)) = value else {
        return Ok(Vec::new());
    };
    let mut values = Vec::new();
    for item in items {
        if let Some(name) = item.as_str() {
            values.push(MetaCompileEnumValue {
                name: name.to_string(),
                synonym: split_meta_camel_case(name),
                comment: String::new(),
            });
            continue;
        }
        let object = item
            .as_object()
            .ok_or_else(|| "enum value must be a string or object".to_string())?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "enum value is missing name".to_string())?;
        values.push(MetaCompileEnumValue {
            name: name.to_string(),
            synonym: object
                .get("synonym")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| split_meta_camel_case(name)),
            comment: object
                .get("comment")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(values)
}

pub(crate) fn emit_meta_enum_value<F>(
    lines: &mut Vec<String>,
    indent: &str,
    value: &MetaCompileEnumValue,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<EnumValue uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&value.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &value.synonym);
    if value.comment.is_empty() {
        lines.push(format!("{indent}\t\t<Comment/>"));
    } else {
        lines.push(format!(
            "{indent}\t\t<Comment>{}</Comment>",
            escape_xml(&value.comment)
        ));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</EnumValue>"));
}

pub(crate) fn emit_meta_child_objects<F>(
    lines: &mut Vec<String>,
    indent: &str,
    defn: &Map<String, Value>,
    obj_type: &str,
    obj_name: &str,
    next_uuid: &mut F,
) -> Result<(), String>
where
    F: FnMut() -> String,
{
    match obj_type {
        "Enum" => {
            let values = meta_compile_enum_values(defn.get("values"))?;
            if values.is_empty() {
                lines.push(format!("{indent}<ChildObjects/>"));
            } else {
                lines.push(format!("{indent}<ChildObjects>"));
                for value in &values {
                    emit_meta_enum_value(lines, &format!("{indent}\t"), value, next_uuid);
                }
                lines.push(format!("{indent}</ChildObjects>"));
            }
        }
        "Document"
        | "Report"
        | "DataProcessor"
        | "ExchangePlan"
        | "ChartOfCharacteristicTypes"
        | "ChartOfAccounts"
        | "ChartOfCalculationTypes"
        | "BusinessProcess"
        | "Task" => {
            let attrs = meta_compile_attributes(defn.get("attributes"));
            let tabular_sections = meta_compile_tabular_sections(defn.get("tabularSections"))?;
            let accounting_flags = if obj_type == "ChartOfAccounts" {
                meta_compile_named_items(defn.get("accountingFlags"))
            } else {
                Vec::new()
            };
            let ext_dimension_flags = if obj_type == "ChartOfAccounts" {
                meta_compile_named_items(defn.get("extDimensionAccountingFlags"))
            } else {
                Vec::new()
            };
            let addressing_attrs = if obj_type == "Task" {
                meta_compile_value_items(defn.get("addressingAttributes"))
            } else {
                Vec::new()
            };
            if attrs.is_empty()
                && tabular_sections.is_empty()
                && accounting_flags.is_empty()
                && ext_dimension_flags.is_empty()
                && addressing_attrs.is_empty()
            {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            let attr_context = match obj_type {
                "Document" => "document",
                "Report" | "DataProcessor" => "processor",
                "ChartOfAccounts" | "ChartOfCharacteristicTypes" | "ChartOfCalculationTypes" => {
                    "chart"
                }
                _ => "object",
            };
            for attr in &attrs {
                emit_meta_attribute(lines, &format!("{indent}\t"), attr, attr_context, next_uuid);
            }
            for section in &tabular_sections {
                emit_meta_tabular_section(
                    lines,
                    &format!("{indent}\t"),
                    section,
                    obj_type,
                    obj_name,
                    next_uuid,
                );
            }
            for name in accounting_flags {
                emit_meta_boolean_child(
                    lines,
                    &format!("{indent}\t"),
                    "AccountingFlag",
                    &name,
                    next_uuid,
                );
            }
            for name in ext_dimension_flags {
                emit_meta_boolean_child(
                    lines,
                    &format!("{indent}\t"),
                    "ExtDimensionAccountingFlag",
                    &name,
                    next_uuid,
                );
            }
            for item in addressing_attrs {
                emit_meta_addressing_attribute(lines, &format!("{indent}\t"), &item, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        "InformationRegister"
        | "AccumulationRegister"
        | "AccountingRegister"
        | "CalculationRegister" => {
            let dimensions = meta_compile_attributes(defn.get("dimensions"));
            let resources = meta_compile_attributes(defn.get("resources"));
            let attrs = meta_compile_attributes(defn.get("attributes"));
            if dimensions.is_empty() && resources.is_empty() && attrs.is_empty() {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            if obj_type == "AccountingRegister" {
                for dimension in &dimensions {
                    emit_meta_register_field(
                        lines,
                        &format!("{indent}\t"),
                        "Dimension",
                        dimension,
                        obj_type,
                        next_uuid,
                    );
                }
                for resource in &resources {
                    emit_meta_register_field(
                        lines,
                        &format!("{indent}\t"),
                        "Resource",
                        resource,
                        obj_type,
                        next_uuid,
                    );
                }
            } else {
                for resource in &resources {
                    emit_meta_register_field(
                        lines,
                        &format!("{indent}\t"),
                        "Resource",
                        resource,
                        obj_type,
                        next_uuid,
                    );
                }
                for dimension in &dimensions {
                    emit_meta_register_field(
                        lines,
                        &format!("{indent}\t"),
                        "Dimension",
                        dimension,
                        obj_type,
                        next_uuid,
                    );
                }
            }
            let attr_context = if obj_type == "InformationRegister" {
                "register-info"
            } else {
                "register-other"
            };
            for attr in &attrs {
                emit_meta_attribute(lines, &format!("{indent}\t"), attr, attr_context, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        "DocumentJournal" => {
            let columns = meta_compile_value_items(defn.get("columns"));
            if columns.is_empty() {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            for column in columns {
                emit_meta_column(lines, &format!("{indent}\t"), &column, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        "HTTPService" => {
            let templates = defn.get("urlTemplates").and_then(Value::as_object);
            if templates.is_none_or(Map::is_empty) {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            let mut ordered = templates.unwrap().iter().collect::<Vec<_>>();
            ordered.sort_by(|left, right| left.0.cmp(right.0));
            for (name, value) in ordered {
                emit_meta_url_template(lines, &format!("{indent}\t"), name, value, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        "WebService" => {
            let operations = defn.get("operations").and_then(Value::as_object);
            if operations.is_none_or(Map::is_empty) {
                lines.push(format!("{indent}<ChildObjects/>"));
                return Ok(());
            }
            lines.push(format!("{indent}<ChildObjects>"));
            let mut ordered = operations.unwrap().iter().collect::<Vec<_>>();
            ordered.sort_by(|left, right| left.0.cmp(right.0));
            for (name, value) in ordered {
                emit_meta_operation(lines, &format!("{indent}\t"), name, value, next_uuid);
            }
            lines.push(format!("{indent}</ChildObjects>"));
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn meta_compile_value_items(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Object(object)) => object
            .iter()
            .map(|(name, value)| {
                if let Some(mut cloned) = value.as_object().cloned() {
                    cloned
                        .entry("name".to_string())
                        .or_insert_with(|| Value::String(name.to_string()));
                    Value::Object(cloned)
                } else {
                    Value::String(name.to_string())
                }
            })
            .collect(),
        Some(Value::String(value)) => vec![Value::String(value.to_string())],
        _ => Vec::new(),
    }
}

pub(crate) fn emit_meta_register_field<F>(
    lines: &mut Vec<String>,
    indent: &str,
    field_tag: &str,
    attr: &MetaCompileAttr,
    register_type: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<{field_tag} uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&attr.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &attr.synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    if attr.type_name.is_empty() {
        if field_tag == "Resource" {
            emit_meta_value_type(lines, &format!("{indent}\t\t"), "Number(15,2)");
        } else {
            emit_meta_value_type(lines, &format!("{indent}\t\t"), "String");
        }
    } else {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    for line in [
        "<PasswordMode>false</PasswordMode>",
        "<Format/>",
        "<EditFormat/>",
        "<ToolTip/>",
        "<MarkNegatives>false</MarkNegatives>",
        "<Mask/>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    let multi_line = attr.multi_line || attr.flags.iter().any(|flag| flag == "multiline");
    lines.push(format!("{indent}\t\t<MultiLine>{multi_line}</MultiLine>"));
    lines.push(format!("{indent}\t\t<ExtendedEdit>false</ExtendedEdit>"));
    lines.push(format!("{indent}\t\t<MinValue xsi:nil=\"true\"/>"));
    lines.push(format!("{indent}\t\t<MaxValue xsi:nil=\"true\"/>"));
    if register_type == "InformationRegister" {
        let fill_from = field_tag == "Dimension" && attr.flags.iter().any(|flag| flag == "master");
        lines.push(format!(
            "{indent}\t\t<FillFromFillingValue>{fill_from}</FillFromFillingValue>"
        ));
        lines.push(format!("{indent}\t\t<FillValue xsi:nil=\"true\"/>"));
    }
    let fill_checking = if !attr.fill_checking.is_empty() {
        attr.fill_checking.as_str()
    } else if attr.flags.iter().any(|flag| flag == "req") {
        "ShowError"
    } else {
        "DontCheck"
    };
    lines.push(format!(
        "{indent}\t\t<FillChecking>{}</FillChecking>",
        escape_xml(fill_checking)
    ));
    for line in [
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<CreateOnInput>Auto</CreateOnInput>",
        "<ChoiceForm/>",
        "<LinkByType/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    if register_type == "AccountingRegister" {
        lines.push(format!("{indent}\t\t<Balance>true</Balance>"));
        lines.push(format!("{indent}\t\t<AccountingFlag/>"));
        if field_tag == "Resource" {
            lines.push(format!("{indent}\t\t<ExtDimensionAccountingFlag/>"));
        }
    }
    if field_tag == "Dimension" {
        if register_type == "InformationRegister" {
            let master = attr.flags.iter().any(|flag| flag == "master");
            let main_filter = attr.flags.iter().any(|flag| flag == "mainfilter");
            let deny_incomplete = attr.flags.iter().any(|flag| flag == "denyincomplete");
            lines.push(format!("{indent}\t\t<Master>{master}</Master>"));
            lines.push(format!(
                "{indent}\t\t<MainFilter>{main_filter}</MainFilter>"
            ));
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>{deny_incomplete}</DenyIncompleteValues>"
            ));
        } else if register_type == "AccumulationRegister" {
            let deny_incomplete = attr.flags.iter().any(|flag| flag == "denyincomplete");
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>{deny_incomplete}</DenyIncompleteValues>"
            ));
        } else if register_type == "AccountingRegister" {
            let deny_incomplete = attr.flags.iter().any(|flag| flag == "denyincomplete");
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>{deny_incomplete}</DenyIncompleteValues>"
            ));
        } else if register_type == "CalculationRegister" {
            let deny_incomplete = attr.flags.iter().any(|flag| flag == "denyincomplete");
            lines.push(format!(
                "{indent}\t\t<DenyIncompleteValues>{deny_incomplete}</DenyIncompleteValues>"
            ));
            lines.push(format!("{indent}\t\t<BaseDimension>false</BaseDimension>"));
            lines.push(format!("{indent}\t\t<ScheduleLink/>"));
        }
    }
    let indexing = if !attr.indexing.is_empty() {
        attr.indexing.as_str()
    } else if attr.flags.iter().any(|flag| flag == "index") {
        "Index"
    } else {
        "DontIndex"
    };
    if field_tag == "Dimension" || register_type == "InformationRegister" {
        lines.push(format!(
            "{indent}\t\t<Indexing>{}</Indexing>",
            escape_xml(indexing)
        ));
    }
    lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
    if field_tag == "Dimension" && register_type == "AccumulationRegister" {
        let use_in_totals = !attr.flags.iter().any(|flag| flag == "nouseintotals");
        lines.push(format!(
            "{indent}\t\t<UseInTotals>{use_in_totals}</UseInTotals>"
        ));
    }
    if register_type == "InformationRegister" {
        lines.push(format!("{indent}\t\t<DataHistory>Use</DataHistory>"));
        if field_tag == "Dimension" {
            lines.push(format!(
                "{indent}\t\t<TypeReductionMode>TransformValues</TypeReductionMode>"
            ));
        }
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</{field_tag}>"));
}

pub(crate) fn emit_meta_boolean_child<F>(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<{tag} uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    emit_meta_value_type(lines, &format!("{indent}\t\t"), "Boolean");
    for line in [
        "<PasswordMode>false</PasswordMode>",
        "<Format/>",
        "<EditFormat/>",
        "<ToolTip/>",
        "<MarkNegatives>false</MarkNegatives>",
        "<Mask/>",
        "<MultiLine>false</MultiLine>",
        "<ExtendedEdit>false</ExtendedEdit>",
        "<MinValue xsi:nil=\"true\"/>",
        "<MaxValue xsi:nil=\"true\"/>",
        "<FillFromFillingValue>false</FillFromFillingValue>",
        "<FillValue xsi:nil=\"true\"/>",
        "<FillChecking>DontCheck</FillChecking>",
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<CreateOnInput>Auto</CreateOnInput>",
        "<ChoiceForm/>",
        "<LinkByType/>",
        "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
        "<DataHistory>Use</DataHistory>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn emit_meta_addressing_attribute<F>(
    lines: &mut Vec<String>,
    indent: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let attr = meta_compile_parse_attr(value);
    let object = value.as_object();
    lines.push(format!(
        "{indent}<AddressingAttribute uuid=\"{}\">",
        next_uuid()
    ));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&attr.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &attr.synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    if attr.type_name.is_empty() {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), "String");
    } else {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    emit_meta_optional_text(
        lines,
        &format!("{indent}\t\t"),
        "AddressingDimension",
        object
            .and_then(|object| object.get("addressingDimension"))
            .and_then(Value::as_str),
    );
    let indexing = object
        .and_then(|object| object.get("indexing"))
        .and_then(Value::as_str)
        .unwrap_or("Index");
    lines.push(format!(
        "{indent}\t\t<Indexing>{}</Indexing>",
        escape_xml(indexing)
    ));
    lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
    lines.push(format!("{indent}\t\t<DataHistory>Use</DataHistory>"));
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</AddressingAttribute>"));
}

pub(crate) fn emit_meta_column<F>(
    lines: &mut Vec<String>,
    indent: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let object = value.as_object();
    let name = value
        .as_str()
        .or_else(|| {
            object
                .and_then(|object| object.get("name"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let synonym = object
        .and_then(|object| object.get("synonym"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(name));
    let indexing = object
        .and_then(|object| object.get("indexing"))
        .and_then(Value::as_str)
        .unwrap_or("DontIndex");
    let references = object
        .and_then(|object| object.get("references"))
        .map(|value| meta_compile_string_list(Some(value)))
        .unwrap_or_default();
    lines.push(format!("{indent}<Column uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<Indexing>{}</Indexing>",
        escape_xml(indexing)
    ));
    emit_meta_md_object_refs(lines, &format!("{indent}\t\t"), "References", &references);
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Column>"));
}

pub(crate) fn emit_meta_url_template<F>(
    lines: &mut Vec<String>,
    indent: &str,
    name: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let object = value.as_object();
    let template = value
        .as_str()
        .or_else(|| {
            object
                .and_then(|object| object.get("template"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("/{}", name.to_lowercase()));
    lines.push(format!("{indent}<URLTemplate uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<Template>{}</Template>",
        escape_xml(&template)
    ));
    lines.push(format!("{indent}\t</Properties>"));
    let methods = object
        .and_then(|object| object.get("methods"))
        .and_then(Value::as_object);
    if methods.is_none_or(Map::is_empty) {
        lines.push(format!("{indent}\t<ChildObjects/>"));
    } else {
        lines.push(format!("{indent}\t<ChildObjects>"));
        let mut ordered = methods.unwrap().iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.0.cmp(right.0));
        for (method_name, http_method_value) in ordered {
            let http_method = http_method_value.as_str().unwrap_or("GET");
            emit_meta_http_method(
                lines,
                &format!("{indent}\t\t"),
                name,
                method_name,
                http_method,
                next_uuid,
            );
        }
        lines.push(format!("{indent}\t</ChildObjects>"));
    }
    lines.push(format!("{indent}</URLTemplate>"));
}

pub(crate) fn emit_meta_http_method<F>(
    lines: &mut Vec<String>,
    indent: &str,
    template_name: &str,
    method_name: &str,
    http_method: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<Method uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(method_name)
    ));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(method_name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<HTTPMethod>{}</HTTPMethod>",
        escape_xml(http_method)
    ));
    lines.push(format!(
        "{indent}\t\t<Handler>{}{}</Handler>",
        escape_xml(template_name),
        escape_xml(method_name)
    ));
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Method>"));
}

pub(crate) fn emit_meta_operation<F>(
    lines: &mut Vec<String>,
    indent: &str,
    name: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let object = value.as_object();
    let return_type = value
        .as_str()
        .or_else(|| {
            object
                .and_then(|object| object.get("returnType"))
                .and_then(Value::as_str)
        })
        .unwrap_or("xs:string");
    let nillable = object
        .and_then(|object| object.get("nillable"))
        .and_then(Value::as_bool)
        == Some(true);
    let transactioned = object
        .and_then(|object| object.get("transactioned"))
        .and_then(Value::as_bool)
        == Some(true);
    let handler = object
        .and_then(|object| object.get("handler"))
        .and_then(Value::as_str)
        .unwrap_or(name);
    lines.push(format!("{indent}<Operation uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<XDTOReturningValueType>{}</XDTOReturningValueType>",
        escape_xml(return_type)
    ));
    lines.push(format!("{indent}\t\t<Nillable>{nillable}</Nillable>"));
    lines.push(format!(
        "{indent}\t\t<Transactioned>{transactioned}</Transactioned>"
    ));
    lines.push(format!(
        "{indent}\t\t<ProcedureName>{}</ProcedureName>",
        escape_xml(handler)
    ));
    lines.push(format!(
        "{indent}\t\t<DataLockControlMode>Managed</DataLockControlMode>"
    ));
    lines.push(format!("{indent}\t</Properties>"));
    let parameters = object
        .and_then(|object| object.get("parameters"))
        .and_then(Value::as_object);
    if parameters.is_none_or(Map::is_empty) {
        lines.push(format!("{indent}\t<ChildObjects/>"));
    } else {
        lines.push(format!("{indent}\t<ChildObjects>"));
        let mut ordered = parameters.unwrap().iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.0.cmp(right.0));
        for (param_name, param_value) in ordered {
            emit_meta_operation_parameter(
                lines,
                &format!("{indent}\t\t"),
                param_name,
                param_value,
                next_uuid,
            );
        }
        lines.push(format!("{indent}\t</ChildObjects>"));
    }
    lines.push(format!("{indent}</Operation>"));
}

pub(crate) fn emit_meta_operation_parameter<F>(
    lines: &mut Vec<String>,
    indent: &str,
    name: &str,
    value: &Value,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    let object = value.as_object();
    let value_type = value
        .as_str()
        .or_else(|| {
            object
                .and_then(|object| object.get("type"))
                .and_then(Value::as_str)
        })
        .unwrap_or("xs:string");
    let nillable = object
        .and_then(|object| object.get("nillable"))
        .and_then(Value::as_bool)
        != Some(false);
    let direction = object
        .and_then(|object| object.get("direction"))
        .and_then(Value::as_str)
        .unwrap_or("In");
    lines.push(format!("{indent}<Parameter uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!(
        "{indent}\t\t<XDTOValueType>{}</XDTOValueType>",
        escape_xml(value_type)
    ));
    lines.push(format!("{indent}\t\t<Nillable>{nillable}</Nillable>"));
    lines.push(format!(
        "{indent}\t\t<TransferDirection>{}</TransferDirection>",
        escape_xml(direction)
    ));
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Parameter>"));
}

pub(crate) fn meta_enum_prop(defn: &Map<String, Value>, field_name: &str, default: &str) -> String {
    defn.get(field_name)
        .and_then(Value::as_str)
        .map(normalize_meta_enum_value)
        .map(|value| escape_xml(&value))
        .unwrap_or_else(|| escape_xml(default))
}

pub(crate) fn normalize_meta_enum_value(value: &str) -> String {
    match value {
        // Keep old DSL requests readable while emitting only the platform enum value.
        "HierarchyItemsOnly" => "HierarchyOfItems",
        "Balances" => "Balance",
        "Остатки" => "Balance",
        "Обороты" => "Turnovers",
        "None" => "Nonperiodical",
        "Daily" => "Day",
        "Monthly" => "Month",
        "Quarterly" => "Quarter",
        "Yearly" => "Year",
        "Непериодический" => "Nonperiodical",
        "Секунда" => "Second",
        "День" => "Day",
        "Месяц" => "Month",
        "Квартал" => "Quarter",
        "Год" => "Year",
        "ПозицияРегистратора" => "RecorderPosition",
        "RecordSubordinate" | "Subordinate" | "ПодчинениеРегистратору" => {
            "RecorderSubordinate"
        }
        "Независимый" => "Independent",
        "NotDependOnCalculationTypes" | "NoDependence" | "NotUsed" => "DontUse",
        "Depend" | "ПоПериодуДействия" => "OnActionPeriod",
        "Автоматический" => "Automatic",
        "Управляемый" => "Managed",
        "Использовать" => "Use",
        "НеИспользовать" => "DontUse",
        "Разрешить" => "Allow",
        "Запретить" => "Deny",
        "ВВидеНаименования" => "AsDescription",
        "ВВидеКода" => "AsCode",
        "ВДиалоге" => "InDialog",
        "ВСписке" => "InList",
        "ОбаСпособа" => "BothWays",
        "НеПроверять" => "DontCheck",
        "Ошибка" => "ShowError",
        "НеИндексировать" => "DontIndex",
        "Индексировать" => "Index",
        "ИндексироватьСДопУпорядочиванием" => {
            "IndexWithAdditionalOrder"
        }
        other => other,
    }
    .to_string()
}

pub(crate) fn emit_meta_standard_attributes(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
) {
    let attrs = match object_type {
        "Catalog" => vec![
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "IsFolder",
            "Owner",
            "Parent",
            "Description",
            "Code",
        ],
        "Document" => vec!["Posted", "Ref", "DeletionMark", "Date", "Number"],
        "Enum" => vec!["Order", "Ref"],
        "InformationRegister" => vec!["Active", "LineNumber", "Recorder", "Period"],
        "AccumulationRegister" => vec!["RecordType", "Active", "LineNumber", "Recorder", "Period"],
        "AccountingRegister" => vec![
            "Account",
            "RecordType",
            "Active",
            "LineNumber",
            "Recorder",
            "Period",
        ],
        "CalculationRegister" => vec![
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
        "ChartOfAccounts" => vec![
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
        "ChartOfCharacteristicTypes" => vec![
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
        "ChartOfCalculationTypes" => vec![
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "ActionPeriodIsBasic",
            "Description",
            "Code",
        ],
        "BusinessProcess" => vec![
            "Started",
            "HeadTask",
            "Completed",
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
        ],
        "Task" => vec![
            "Executed",
            "Description",
            "RoutePoint",
            "BusinessProcess",
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
        ],
        "ExchangePlan" => vec![
            "ExchangeDate",
            "ThisNode",
            "ReceivedNo",
            "SentNo",
            "Ref",
            "DeletionMark",
            "Description",
            "Code",
        ],
        "DocumentJournal" => vec!["Type", "Ref", "Date", "Posted", "DeletionMark", "Number"],
        "TabularSection" => vec!["LineNumber"],
        _ => Vec::new(),
    };
    if attrs.is_empty() {
        return;
    }
    lines.push(format!("{indent}<StandardAttributes>"));
    for attr in attrs {
        emit_meta_standard_attribute(lines, &format!("{indent}\t"), object_type, attr);
    }
    lines.push(format!("{indent}</StandardAttributes>"));
}

pub(crate) fn meta_standard_attribute_type_reduction_mode(
    object_type: &str,
    attr_name: &str,
) -> Option<&'static str> {
    if object_type == "Catalog" && attr_name == "Owner" {
        Some("Deny")
    } else {
        Some("TransformValues")
    }
}

pub(crate) fn emit_meta_standard_attribute(
    lines: &mut Vec<String>,
    indent: &str,
    object_type: &str,
    attr_name: &str,
) {
    lines.push(format!(
        "{indent}<xr:StandardAttribute name=\"{}\">",
        escape_xml(attr_name)
    ));
    for line in [
        "<xr:LinkByType/>",
        "<xr:FillChecking>DontCheck</xr:FillChecking>",
        "<xr:MultiLine>false</xr:MultiLine>",
        "<xr:FillFromFillingValue>false</xr:FillFromFillingValue>",
        "<xr:CreateOnInput>Auto</xr:CreateOnInput>",
    ] {
        lines.push(format!("{indent}\t{line}"));
    }
    if let Some(mode) = meta_standard_attribute_type_reduction_mode(object_type, attr_name) {
        lines.push(format!(
            "{indent}\t<xr:TypeReductionMode>{}</xr:TypeReductionMode>",
            escape_xml(mode)
        ));
    }
    for line in [
        "<xr:MaxValue xsi:nil=\"true\"/>",
        "<xr:ToolTip/>",
        "<xr:ExtendedEdit>false</xr:ExtendedEdit>",
        "<xr:Format/>",
        "<xr:ChoiceForm/>",
        "<xr:QuickChoice>Auto</xr:QuickChoice>",
        "<xr:ChoiceHistoryOnInput>Auto</xr:ChoiceHistoryOnInput>",
        "<xr:EditFormat/>",
        "<xr:PasswordMode>false</xr:PasswordMode>",
        "<xr:DataHistory>Use</xr:DataHistory>",
        "<xr:MarkNegatives>false</xr:MarkNegatives>",
        "<xr:MinValue xsi:nil=\"true\"/>",
        "<xr:Synonym/>",
        "<xr:Comment/>",
        "<xr:FullTextSearch>Use</xr:FullTextSearch>",
        "<xr:ChoiceParameterLinks/>",
        "<xr:FillValue xsi:nil=\"true\"/>",
        "<xr:Mask/>",
        "<xr:ChoiceParameters/>",
    ] {
        lines.push(format!("{indent}\t{line}"));
    }
    lines.push(format!("{indent}</xr:StandardAttribute>"));
}

#[derive(Clone)]
pub(crate) struct MetaCompileAttr {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) synonym: String,
    pub(crate) flags: Vec<String>,
    pub(crate) fill_checking: String,
    pub(crate) indexing: String,
    pub(crate) multi_line: bool,
    pub(crate) choice_history_on_input: String,
}

pub(crate) struct MetaCompileTabularSection {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaCompileAttr>,
}

pub(crate) fn meta_compile_attributes(value: Option<&Value>) -> Vec<MetaCompileAttr> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(object) = value.as_object() {
        return object
            .iter()
            .map(|(key, value)| {
                meta_compile_parse_attr(&Value::String(format!(
                    "{key}:{}",
                    json_value_to_python_string(value)
                )))
            })
            .collect();
    }
    value
        .as_array()
        .map(|items| items.iter().map(meta_compile_parse_attr).collect())
        .unwrap_or_default()
}

pub(crate) fn meta_compile_tabular_sections(
    value: Option<&Value>,
) -> Result<Vec<MetaCompileTabularSection>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    if let Some(items) = value.as_array() {
        for item in items {
            let object = item
                .as_object()
                .ok_or_else(|| "tabular section must be an object".to_string())?;
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "tabular section is missing name".to_string())?
                .to_string();
            result.push(MetaCompileTabularSection {
                name,
                columns: meta_compile_attributes(object.get("attributes")),
            });
        }
    } else if let Some(object) = value.as_object() {
        for (name, columns) in object {
            result.push(MetaCompileTabularSection {
                name: name.to_string(),
                columns: meta_compile_attributes(Some(columns)),
            });
        }
    }
    Ok(result)
}

pub(crate) fn meta_compile_parse_attr(value: &Value) -> MetaCompileAttr {
    if let Some(text) = value.as_str() {
        let mut pieces = text.splitn(2, '|');
        let main = pieces.next().unwrap_or_default().trim();
        let flags = pieces
            .next()
            .map(|part| {
                part.split(',')
                    .map(str::trim)
                    .filter(|flag| !flag.is_empty())
                    .map(|flag| flag.to_lowercase())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut colon = main.splitn(2, ':');
        let name = colon.next().unwrap_or_default().trim().to_string();
        let type_name = colon.next().unwrap_or_default().trim().to_string();
        let synonym = split_meta_camel_case(&name);
        return MetaCompileAttr {
            name,
            type_name,
            synonym,
            flags,
            fill_checking: String::new(),
            indexing: String::new(),
            multi_line: false,
            choice_history_on_input: String::new(),
        };
    }
    let object = value.as_object();
    let name = object
        .and_then(|object| object.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let type_name = object.map(meta_compile_build_type).unwrap_or_default();
    let synonym = object
        .and_then(|object| object.get("synonym"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| split_meta_camel_case(&name));
    let flags = object
        .and_then(|object| object.get("flags"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    MetaCompileAttr {
        name,
        type_name,
        synonym,
        flags,
        fill_checking: object
            .and_then(|object| object.get("fillChecking"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        indexing: object
            .and_then(|object| object.get("indexing"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        multi_line: object
            .and_then(|object| object.get("multiLine"))
            .and_then(Value::as_bool)
            == Some(true),
        choice_history_on_input: object
            .and_then(|object| object.get("choiceHistoryOnInput"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

pub(crate) fn meta_compile_build_type(object: &Map<String, Value>) -> String {
    let mut type_name = object
        .get("valueType")
        .or_else(|| object.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !type_name.is_empty() && !type_name.contains('(') {
        if type_name == "String" {
            if let Some(length) = object.get("length").and_then(json_i64_value) {
                type_name = format!("String({length})");
            }
        } else if type_name == "Number" {
            if let Some(length) = object.get("length").and_then(json_i64_value) {
                let precision = object
                    .get("precision")
                    .and_then(json_i64_value)
                    .unwrap_or(0);
                let nn = if object.get("nonneg").and_then(Value::as_bool) == Some(true)
                    || object.get("nonnegative").and_then(Value::as_bool) == Some(true)
                {
                    ",nonneg"
                } else {
                    ""
                };
                type_name = format!("Number({length},{precision}{nn})");
            }
        }
    }
    type_name
}

pub(crate) fn emit_meta_attribute<F>(
    lines: &mut Vec<String>,
    indent: &str,
    attr: &MetaCompileAttr,
    context: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<Attribute uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&attr.name)
    ));
    emit_meta_mltext(lines, &format!("{indent}\t\t"), "Synonym", &attr.synonym);
    lines.push(format!("{indent}\t\t<Comment/>"));
    if attr.type_name.is_empty() {
        lines.push(format!("{indent}\t\t<Type>"));
        lines.push(format!("{indent}\t\t\t<v8:Type>xs:string</v8:Type>"));
        lines.push(format!("{indent}\t\t</Type>"));
    } else {
        emit_meta_value_type(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    lines.push(format!("{indent}\t\t<PasswordMode>false</PasswordMode>"));
    lines.push(format!("{indent}\t\t<Format/>"));
    lines.push(format!("{indent}\t\t<EditFormat/>"));
    lines.push(format!("{indent}\t\t<ToolTip/>"));
    lines.push(format!("{indent}\t\t<MarkNegatives>false</MarkNegatives>"));
    lines.push(format!("{indent}\t\t<Mask/>"));
    let multi_line = attr.multi_line || attr.flags.iter().any(|flag| flag == "multiline");
    lines.push(format!("{indent}\t\t<MultiLine>{multi_line}</MultiLine>"));
    lines.push(format!("{indent}\t\t<ExtendedEdit>false</ExtendedEdit>"));
    lines.push(format!("{indent}\t\t<MinValue xsi:nil=\"true\"/>"));
    lines.push(format!("{indent}\t\t<MaxValue xsi:nil=\"true\"/>"));
    if !matches!(
        context,
        "tabular" | "processor" | "chart" | "register-other"
    ) {
        lines.push(format!(
            "{indent}\t\t<FillFromFillingValue>false</FillFromFillingValue>"
        ));
    }
    if !matches!(
        context,
        "tabular" | "processor" | "chart" | "register-other"
    ) {
        emit_meta_fill_value(lines, &format!("{indent}\t\t"), &attr.type_name);
    }
    let fill_checking = if !attr.fill_checking.is_empty() {
        attr.fill_checking.as_str()
    } else if attr.flags.iter().any(|flag| flag == "req") {
        "ShowError"
    } else {
        "DontCheck"
    };
    lines.push(format!(
        "{indent}\t\t<FillChecking>{}</FillChecking>",
        escape_xml(fill_checking)
    ));
    for line in [
        "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
        "<ChoiceParameterLinks/>",
        "<ChoiceParameters/>",
        "<QuickChoice>Auto</QuickChoice>",
        "<CreateOnInput>Auto</CreateOnInput>",
        "<ChoiceForm/>",
        "<LinkByType/>",
    ] {
        lines.push(format!("{indent}\t\t{line}"));
    }
    let choice_history_on_input = if attr.choice_history_on_input.is_empty() {
        "Auto"
    } else {
        attr.choice_history_on_input.as_str()
    };
    lines.push(format!(
        "{indent}\t\t<ChoiceHistoryOnInput>{}</ChoiceHistoryOnInput>",
        escape_xml(choice_history_on_input)
    ));
    if context == "catalog" {
        lines.push(format!("{indent}\t\t<Use>ForItem</Use>"));
    }
    if !matches!(context, "processor" | "processor-tabular") {
        let indexing = if !attr.indexing.is_empty() {
            attr.indexing.as_str()
        } else if attr.flags.iter().any(|flag| flag == "indexadditional") {
            "IndexWithAdditionalOrder"
        } else if attr.flags.iter().any(|flag| flag == "index") {
            "Index"
        } else {
            "DontIndex"
        };
        lines.push(format!(
            "{indent}\t\t<Indexing>{}</Indexing>",
            escape_xml(indexing)
        ));
        lines.push(format!("{indent}\t\t<FullTextSearch>Use</FullTextSearch>"));
        if !matches!(context, "chart" | "register-other") {
            lines.push(format!("{indent}\t\t<DataHistory>Use</DataHistory>"));
        }
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</Attribute>"));
}

pub(crate) fn emit_meta_tabular_section<F>(
    lines: &mut Vec<String>,
    indent: &str,
    section: &MetaCompileTabularSection,
    object_type: &str,
    object_name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<TabularSection uuid=\"{}\">", next_uuid()));
    let type_prefix = format!("{object_type}TabularSection");
    let row_prefix = format!("{object_type}TabularSectionRow");
    let generated_type_name = escape_xml(&format!("{type_prefix}.{object_name}.{}", section.name));
    let generated_row_name = escape_xml(&format!("{row_prefix}.{object_name}.{}", section.name));
    lines.push(format!("{indent}\t<InternalInfo>"));
    lines.push(format!(
        "{indent}\t\t<xr:GeneratedType name=\"{generated_type_name}\" category=\"TabularSection\">"
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:TypeId>{}</xr:TypeId>",
        next_uuid()
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:ValueId>{}</xr:ValueId>",
        next_uuid()
    ));
    lines.push(format!("{indent}\t\t</xr:GeneratedType>"));
    lines.push(format!(
        "{indent}\t\t<xr:GeneratedType name=\"{generated_row_name}\" category=\"TabularSectionRow\">"
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:TypeId>{}</xr:TypeId>",
        next_uuid()
    ));
    lines.push(format!(
        "{indent}\t\t\t<xr:ValueId>{}</xr:ValueId>",
        next_uuid()
    ));
    lines.push(format!("{indent}\t\t</xr:GeneratedType>"));
    lines.push(format!("{indent}\t</InternalInfo>"));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!(
        "{indent}\t\t<Name>{}</Name>",
        escape_xml(&section.name)
    ));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(&section.name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    lines.push(format!("{indent}\t\t<ToolTip/>"));
    lines.push(format!(
        "{indent}\t\t<FillChecking>DontCheck</FillChecking>"
    ));
    emit_meta_standard_attributes(lines, &format!("{indent}\t\t"), "TabularSection");
    if meta_line_number_length_is_applicable(object_type) {
        lines.push(format!(
            "{indent}\t\t<LineNumberLength>9</LineNumberLength>"
        ));
    }
    if object_type == "Catalog" {
        lines.push(format!("{indent}\t\t<Use>ForItem</Use>"));
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}\t<ChildObjects>"));
    let column_context = if matches!(object_type, "DataProcessor" | "Report") {
        "processor-tabular"
    } else {
        "tabular"
    };
    for column in &section.columns {
        emit_meta_attribute(
            lines,
            &format!("{indent}\t\t"),
            column,
            column_context,
            next_uuid,
        );
    }
    lines.push(format!("{indent}\t</ChildObjects>"));
    lines.push(format!("{indent}</TabularSection>"));
}

fn meta_line_number_length_is_applicable(object_type: &str) -> bool {
    !matches!(
        object_type,
        "Report" | "DataProcessor" | "ExternalReport" | "ExternalDataProcessor"
    )
}

pub(crate) fn emit_meta_mltext(lines: &mut Vec<String>, indent: &str, tag: &str, text: &str) {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>"));
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn emit_meta_value_type(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    lines.push(format!("{indent}<Type>"));
    emit_meta_type_content(lines, &format!("{indent}\t"), type_name);
    lines.push(format!("{indent}</Type>"));
}

pub(crate) fn emit_meta_type_content(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    emit_meta_type_contents(lines, indent, std::iter::once(type_name));
}

pub(crate) fn emit_meta_type_contents<'a>(
    lines: &mut Vec<String>,
    indent: &str,
    type_names: impl IntoIterator<Item = &'a str>,
) {
    emit_meta_type_contents_with_string_length(lines, indent, type_names, None);
}

fn emit_meta_event_subscription_source_type_contents<'a>(
    lines: &mut Vec<String>,
    indent: &str,
    type_names: impl IntoIterator<Item = &'a str>,
) {
    // Event sources are type identities, not constrained values. 8.3.27
    // canonicalizes every string source to the unbounded Length=0 form.
    emit_meta_type_contents_with_string_length(lines, indent, type_names, Some(0));
}

fn emit_meta_type_contents_with_string_length<'a>(
    lines: &mut Vec<String>,
    indent: &str,
    type_names: impl IntoIterator<Item = &'a str>,
    string_length_override: Option<u32>,
) {
    let mut resolved_types = type_names
        .into_iter()
        .flat_map(|type_name| type_name.split('+'))
        .map(str::trim)
        .filter(|type_name| !type_name.is_empty())
        .map(resolve_meta_type)
        .collect::<Vec<_>>();
    // 8.3.27 groups concrete configuration types before primitive types, but
    // orders configuration types by their xr:TypeId from the surrounding
    // configuration. This pure serializer has no workspace TypeId index, so
    // the stable sort deliberately preserves DSL order inside that group.
    resolved_types.sort_by_key(|resolved| meta_type_platform_group_rank(resolved));

    for resolved in resolved_types
        .iter()
        .filter(|resolved| !resolved.starts_with("DefinedType."))
    {
        emit_meta_type_tag(lines, indent, resolved);
    }
    for resolved in resolved_types
        .iter()
        .filter(|resolved| resolved.starts_with("DefinedType."))
    {
        emit_meta_type_tag(lines, indent, resolved);
    }
    for resolved in &resolved_types {
        emit_meta_number_qualifiers(lines, indent, resolved);
    }
    for resolved in &resolved_types {
        emit_meta_string_qualifiers_with_length(lines, indent, resolved, string_length_override);
    }
    for resolved in &resolved_types {
        emit_meta_date_qualifiers(lines, indent, resolved);
    }
}

fn meta_type_platform_group_rank(resolved: &str) -> u8 {
    let (tag, wire_name) = meta_type_wire_contract(resolved);
    match (tag, wire_name.as_str()) {
        ("TypeSet", _) => 6,
        (_, "xs:boolean") => 1,
        (_, "xs:string") => 2,
        (_, "xs:dateTime") => 3,
        (_, "xs:decimal") => 4,
        (_, "v8:ValueStorage") => 5,
        _ => 0,
    }
}

fn meta_type_wire_contract(resolved: &str) -> (&'static str, String) {
    if resolved.starts_with("DefinedType.") {
        ("TypeSet", format!("cfg:{resolved}"))
    } else if resolved == "Boolean" {
        ("Type", "xs:boolean".to_string())
    } else if matches!(resolved, "Date" | "DateTime") {
        ("Type", "xs:dateTime".to_string())
    } else if resolved == "ValueStorage" {
        ("Type", "v8:ValueStorage".to_string())
    } else if resolved == "String" || resolved.starts_with("String(") {
        ("Type", "xs:string".to_string())
    } else if resolved == "Number" || parse_meta_number_type(resolved).is_some() {
        ("Type", "xs:decimal".to_string())
    } else if meta_compile_is_config_type(resolved) {
        ("Type", format!("cfg:{resolved}"))
    } else {
        ("Type", resolved.to_string())
    }
}

fn validate_meta_type_union<'a>(
    type_names: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let mut seen = BTreeMap::<(String, String), String>::new();
    let mut type_count = 0usize;
    let mut has_value_storage = false;
    for raw in type_names {
        for type_name in raw
            .split('+')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let resolved = resolve_meta_type(type_name);
            validate_meta_resolved_type(type_name, &resolved)?;
            type_count += 1;
            has_value_storage |= resolved == "ValueStorage";
            let (tag, wire_name) = meta_type_wire_contract(&resolved);
            let key = (tag.to_string(), wire_name.clone());
            if let Some(previous) = seen.insert(key, type_name.to_string()) {
                return Err(format!(
                    "duplicate platform type in valueTypes: {previous} and {type_name} both map to v8:{tag} {wire_name}"
                ));
            }
        }
    }
    if has_value_storage && type_count > 1 {
        return Err(
            "ValueStorage must be the only platform type in an 8.3.27 type description".to_string(),
        );
    }
    Ok(())
}

fn validate_meta_resolved_type(raw: &str, resolved: &str) -> Result<(), String> {
    if resolved == "String" {
        return Ok(());
    }
    if resolved.starts_with("String") {
        if parse_meta_string_type(resolved).is_none() {
            return Err(format!(
                "type {raw} is not valid for 8.3.27; expected String or String(integer length 0..=1024)"
            ));
        }
        return Ok(());
    }
    if resolved == "Number" {
        return Ok(());
    }
    if resolved.starts_with("Number") {
        if parse_meta_number_type(resolved).is_none() {
            return Err(format!(
                "type {raw} is not valid for 8.3.27; expected Number(integer digits 0..=38, integer fraction 0..=digits[,nonneg])"
            ));
        }
        return Ok(());
    }
    if resolved.contains(['(', ')']) {
        return Err(format!(
            "type {raw} is not valid for 8.3.27; parameters are supported only for String and Number"
        ));
    }
    if meta_compile_is_config_type(resolved) {
        let invalid_name = resolved
            .split_once('.')
            .is_none_or(|(_, name)| name.trim().is_empty() || name.contains('.'));
        if invalid_name || !form_is_xml_ncname(resolved) {
            return Err(format!(
                "type {raw} is not valid for 8.3.27; configuration type name is not an XML NCName"
            ));
        }
        return Ok(());
    }
    if matches!(resolved, "Boolean" | "Date" | "DateTime" | "ValueStorage") {
        return Ok(());
    }
    Err(format!(
        "type {raw} is not supported by the fixed 8.3.27 metadata DSL"
    ))
}

pub(crate) fn emit_meta_type_tag(lines: &mut Vec<String>, indent: &str, resolved: &str) {
    let (tag, wire_name) = meta_type_wire_contract(resolved);
    lines.push(format!(
        "{indent}<v8:{tag}>{}</v8:{tag}>",
        escape_xml(&wire_name)
    ));
}

pub(crate) fn emit_meta_number_qualifiers(lines: &mut Vec<String>, indent: &str, resolved: &str) {
    let number = if resolved == "Number" {
        Some((10, 0, false))
    } else {
        parse_meta_number_type(resolved)
    };
    if let Some((digits, fraction, nonnegative)) = number {
        lines.push(format!("{indent}<v8:NumberQualifiers>"));
        lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
        lines.push(format!(
            "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
        ));
        lines.push(format!(
            "{indent}\t<v8:AllowedSign>{}</v8:AllowedSign>",
            if nonnegative { "Nonnegative" } else { "Any" }
        ));
        lines.push(format!("{indent}</v8:NumberQualifiers>"));
    }
}

pub(crate) fn emit_meta_string_qualifiers(lines: &mut Vec<String>, indent: &str, resolved: &str) {
    emit_meta_string_qualifiers_with_length(lines, indent, resolved, None);
}

fn emit_meta_string_qualifiers_with_length(
    lines: &mut Vec<String>,
    indent: &str,
    resolved: &str,
    length_override: Option<u32>,
) {
    let length = if resolved == "String" {
        Some(length_override.unwrap_or(10))
    } else {
        parse_meta_string_type(resolved).map(|length| length_override.unwrap_or(length))
    };
    if let Some(length) = length {
        lines.push(format!("{indent}<v8:StringQualifiers>"));
        lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
        lines.push(format!(
            "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
        ));
        lines.push(format!("{indent}</v8:StringQualifiers>"));
    }
}

pub(crate) fn emit_meta_date_qualifiers(lines: &mut Vec<String>, indent: &str, resolved: &str) {
    if matches!(resolved, "Date" | "DateTime") {
        lines.push(format!("{indent}<v8:DateQualifiers>"));
        lines.push(format!(
            "{indent}\t<v8:DateFractions>{resolved}</v8:DateFractions>"
        ));
        lines.push(format!("{indent}</v8:DateQualifiers>"));
    }
}

pub(crate) fn meta_compile_is_config_type(type_name: &str) -> bool {
    [
        "CatalogRef.",
        "CatalogObject.",
        "DocumentRef.",
        "DocumentObject.",
        "EnumRef.",
        "ChartOfAccountsRef.",
        "ChartOfAccountsObject.",
        "ChartOfCharacteristicTypesRef.",
        "ChartOfCharacteristicTypesObject.",
        "ChartOfCalculationTypesRef.",
        "ChartOfCalculationTypesObject.",
        "ExchangePlanRef.",
        "ExchangePlanObject.",
        "BusinessProcessRef.",
        "BusinessProcessObject.",
        "TaskRef.",
        "TaskObject.",
        "ReportObject.",
        "DataProcessorObject.",
        "DefinedType.",
    ]
    .iter()
    .any(|prefix| type_name.starts_with(prefix))
}

pub(crate) fn emit_meta_fill_value(lines: &mut Vec<String>, indent: &str, type_name: &str) {
    if type_name.is_empty() {
        lines.push(format!("{indent}<FillValue xsi:nil=\"true\"/>"));
        return;
    }
    let resolved = resolve_meta_type(type_name);
    if resolved == "Boolean" {
        lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:boolean\">false</FillValue>"
        ));
    } else if resolved.starts_with("String") {
        lines.push(format!("{indent}<FillValue xsi:type=\"xs:string\"/>"));
    } else if resolved.starts_with("Number") {
        lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:decimal\">0</FillValue>"
        ));
    } else {
        lines.push(format!("{indent}<FillValue xsi:nil=\"true\"/>"));
    }
}

pub(crate) fn resolve_meta_type(type_name: &str) -> String {
    if let Some(open) = type_name.find('(') {
        if type_name.ends_with(')') {
            let base = type_name[..open].trim();
            let params = &type_name[open + 1..type_name.len() - 1];
            if let Some(resolved) = meta_type_synonym(base) {
                return format!("{resolved}({params})");
            }
        }
    }
    if let Some(dot) = type_name.find('.') {
        let prefix = &type_name[..dot];
        let suffix = &type_name[dot..];
        if let Some(resolved) = meta_type_synonym(prefix) {
            return format!("{resolved}{suffix}");
        }
    }
    meta_type_synonym(type_name)
        .unwrap_or(type_name)
        .to_string()
}

pub(crate) fn meta_type_synonym(value: &str) -> Option<&'static str> {
    match value.to_lowercase().as_str() {
        "число" | "number" => Some("Number"),
        "строка" | "string" => Some("String"),
        "булево" | "boolean" | "bool" => Some("Boolean"),
        "дата" | "date" => Some("Date"),
        "датавремя" | "datetime" => Some("DateTime"),
        "хранилищезначения" | "valuestorage" => Some("ValueStorage"),
        "справочникссылка" | "catalogref" => Some("CatalogRef"),
        "документссылка" | "documentref" => Some("DocumentRef"),
        "перечислениессылка" | "enumref" => Some("EnumRef"),
        "плансчетовссылка" | "chartofaccountsref" => Some("ChartOfAccountsRef"),
        "планвидовхарактеристикссылка" | "chartofcharacteristictypesref" => {
            Some("ChartOfCharacteristicTypesRef")
        }
        "планвидоврасчётассылка" | "планвидоврасчетассылка" | "chartofcalculationtypesref" => {
            Some("ChartOfCalculationTypesRef")
        }
        "планобменассылка" | "exchangeplanref" => Some("ExchangePlanRef"),
        "бизнеспроцессссылка" | "businessprocessref" => {
            Some("BusinessProcessRef")
        }
        "задачассылка" | "taskref" => Some("TaskRef"),
        "определяемыйтип" | "definedtype" => Some("DefinedType"),
        _ => None,
    }
}

pub(crate) fn parse_meta_string_type(value: &str) -> Option<u32> {
    let rest = value.strip_prefix("String(")?.strip_suffix(')')?.trim();
    if rest.is_empty() || rest.contains(',') {
        return None;
    }
    rest.parse().ok().filter(|length| *length <= 1024)
}

pub(crate) fn parse_meta_number_type(value: &str) -> Option<(u32, u32, bool)> {
    let rest = value.strip_prefix("Number(")?.strip_suffix(')')?;
    let parts = rest.split(',').map(str::trim).collect::<Vec<_>>();
    if !matches!(parts.len(), 2 | 3)
        || parts.iter().any(|part| part.is_empty())
        || (parts.len() == 3 && parts[2] != "nonneg")
    {
        return None;
    }
    let digits = parts[0].parse().ok()?;
    let fraction = parts[1].parse().ok()?;
    if digits > 38 || fraction > digits {
        return None;
    }
    Some((digits, fraction, parts.len() == 3))
}

pub(crate) fn split_meta_camel_case(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let mut result = String::new();
    let mut previous_lower = false;
    for ch in name.chars() {
        if previous_lower && ch.is_uppercase() {
            result.push(' ');
        }
        result.push(ch);
        previous_lower = ch.is_lowercase();
    }
    let mut chars = result.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first, chars.as_str().to_lowercase()),
        None => result,
    }
}

pub(crate) fn register_compiled_meta_in_configuration(
    output_dir: &Path,
    child_tag: &str,
    obj_name: &str,
) -> Result<Option<String>, String> {
    metadata_kind(child_tag).ok_or_else(|| format!("Unknown type '{child_tag}'"))?;
    let config_xml_path = output_dir.join("Configuration.xml");
    let mut transaction = CompileTransaction::new();
    let status = transaction.register_canonical_child(&config_xml_path, child_tag, obj_name)?;
    if status == RegistrationStatus::Added {
        transaction.commit()?;
    }
    Ok(Some(
        match status {
            RegistrationStatus::Added => "added",
            RegistrationStatus::AlreadyPresent => "already",
            RegistrationStatus::MissingTarget => "no-config",
        }
        .to_string(),
    ))
}

#[derive(Default)]
pub(crate) struct MetaEditCounts {
    pub(crate) added: usize,
    pub(crate) modified: usize,
    pub(crate) removed: usize,
}

#[derive(Clone, Copy)]
enum MetaEditEol {
    Lf,
    CrLf,
    Cr,
}

#[derive(Clone, Copy)]
struct MetaEditSourceFormat {
    has_bom: bool,
    eol: MetaEditEol,
}

#[derive(Clone, Copy)]
pub(crate) enum MetaEditLineNumberLengthPolicy {
    Editable,
    FixedFive,
    NotApplicable,
    UnknownCompatibility,
}

struct MetaEditLineNumberLengthAuthorization {
    policy: MetaEditLineNumberLengthPolicy,
    provenance: Option<PlatformXmlOwnerProvenance>,
}

pub(crate) fn edit_meta(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    let edit_result = (|| -> Result<(String, PathBuf, bool, Vec<String>), String> {
        let definition_file = path_arg(args, &["definitionFile", "DefinitionFile"]);
        let operation = string_arg(args, &["operation", "Operation"]);
        if definition_file.is_some() && operation.is_some() {
            return Err("Cannot use both -DefinitionFile and -Operation".to_string());
        }
        if definition_file.is_none() && operation.is_none() {
            return Err("Either -DefinitionFile or -Operation is required".to_string());
        }
        let object_path_raw = required_path(args, OBJECT_PATH, "ObjectPath")?;
        let object_path = resolve_meta_edit_object_path(&object_path_raw, &context.cwd)?;
        let value = string_arg(args, &["value", "Value"]).unwrap_or_default();

        let original_bytes = fs::read(&object_path)
            .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?;
        let mut xml_text = String::from_utf8(original_bytes.clone())
            .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?;
        let source_format = MetaEditSourceFormat {
            has_bom: original_bytes.starts_with(b"\xef\xbb\xbf"),
            eol: meta_edit_source_eol(&xml_text),
        };
        if xml_text.starts_with('\u{feff}') {
            xml_text = xml_text.trim_start_matches('\u{feff}').to_string();
        }
        let (object_type, object_name) = meta_edit_object_identity(&xml_text)?;
        validate_metadata_8_3_27_enum_contract(&xml_text, "meta.edit")?;

        let mut counts = MetaEditCounts::default();
        let mut info_lines = vec![format!("[INFO] Object: {object_type}.{object_name}")];
        let mut transaction = CompileTransaction::new();
        let line_number_length_provenance = if let Some(definition_file) = definition_file {
            let definition_path = absolutize(definition_file.clone(), &context.cwd);
            if !definition_path.exists() {
                return Err(format!(
                    "Definition file not found: {}",
                    definition_file.display()
                ));
            }
            let definition = FileBackedJson::read(&definition_path, |err| {
                format!("DefinitionFile JSON parse error: {err}")
            })?
            .bind_to(&mut transaction)?;
            let authorization = if meta_edit_definition_requests_line_number_length(&definition) {
                meta_edit_line_number_length_policy(
                    &object_type,
                    &object_path,
                    context,
                    &mut transaction,
                )?
            } else {
                MetaEditLineNumberLengthAuthorization {
                    policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
                    provenance: None,
                }
            };
            meta_edit_apply_definition(
                &mut xml_text,
                &object_type,
                &object_name,
                &definition,
                authorization.policy,
                &mut counts,
            )?;
            info_lines.extend(meta_edit_definition_info_lines(&definition));
            authorization.provenance
        } else {
            let operation = operation.expect("checked above");
            let authorization = if meta_edit_inline_requests_line_number_length(operation, value) {
                meta_edit_line_number_length_policy(
                    &object_type,
                    &object_path,
                    context,
                    &mut transaction,
                )?
            } else {
                MetaEditLineNumberLengthAuthorization {
                    policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
                    provenance: None,
                }
            };
            meta_edit_apply_inline_operation(
                &mut xml_text,
                &object_type,
                &object_name,
                operation,
                value,
                authorization.policy,
                &mut counts,
            )?;
            authorization.provenance
        };

        #[cfg(test)]
        run_meta_edit_after_line_number_length_policy_hook();

        Document::parse(xml_text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error after meta-edit: {err}"))?;
        validate_metadata_8_3_27_boolean_contract(&xml_text, "meta.edit")?;
        let serialized_bytes = meta_edit_preserve_source_format(&xml_text, source_format);
        let changed = serialized_bytes != original_bytes;
        let mut warnings = Vec::new();
        if changed {
            transaction.replace_bytes(&object_path, &original_bytes, serialized_bytes)?;
            if let Some(provenance) = line_number_length_provenance {
                provenance.bind_to(&mut transaction)?;
            }
            guard_active_format_owner(&mut transaction, &object_path, context)?;
            let validation_path = object_path.clone();
            warnings = transaction
                .commit_with_post_validation(move || {
                    let published = read_utf8_sig(&validation_path)?;
                    validate_metadata_8_3_27_boolean_contract(&published, "meta.edit")?;
                    validate_metadata_8_3_27_enum_contract(&published, "meta.edit")
                })?
                .cleanup_warnings;
            info_lines.push(format!("[INFO] Saved: {}", object_path.display()));
        } else {
            counts = MetaEditCounts::default();
            info_lines.push("[INFO] No changes".to_string());
        }
        let stdout = format!(
            "{}\n\n=== meta-edit summary ===\n  Object:   {object_type}.{object_name}\n  Added:    {}\n  Removed:  {}\n  Modified: {}\n",
            info_lines.join("\n"),
            counts.added, counts.removed, counts.modified
        );
        Ok((stdout, object_path, changed, warnings))
    })();

    match edit_result {
        Ok((stdout, object_path, changed, warnings)) => AdapterOutcome {
            ok: true,
            summary: "unica.meta.edit completed with native metadata editor".to_string(),
            changes: if changed {
                vec![format!("updated {}", object_path.display())]
            } else {
                Vec::new()
            },
            warnings,
            errors: Vec::new(),
            artifacts: vec![object_path.display().to_string()],
            stdout: Some(stdout),
            stderr: None,
            command: None,
        },
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.edit failed in native metadata editor".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: Some(format!("{error}\n")),
            command: None,
        },
    }
}

fn meta_edit_source_eol(text: &str) -> MetaEditEol {
    let bytes = text.as_bytes();
    if let Some(index) = bytes.iter().position(|byte| *byte == b'\n') {
        return if index > 0 && bytes[index - 1] == b'\r' {
            MetaEditEol::CrLf
        } else {
            MetaEditEol::Lf
        };
    }
    if bytes.contains(&b'\r') {
        MetaEditEol::Cr
    } else {
        MetaEditEol::Lf
    }
}

fn meta_edit_preserve_source_format(text: &str, format: MetaEditSourceFormat) -> Vec<u8> {
    let normalized = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let serialized = match format.eol {
        MetaEditEol::Lf => normalized,
        MetaEditEol::CrLf => normalized.replace('\n', "\r\n"),
        MetaEditEol::Cr => normalized.replace('\n', "\r"),
    };
    let mut bytes = Vec::with_capacity(serialized.len() + usize::from(format.has_bom) * 3);
    if format.has_bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(serialized.as_bytes());
    bytes
}

pub(crate) fn resolve_meta_edit_object_path(raw: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let mut path = absolutize(raw.to_path_buf(), cwd);
    if path.is_dir() {
        let dir_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let candidate = path.join(format!("{dir_name}.xml"));
        let sibling = path
            .parent()
            .map(|parent| parent.join(format!("{dir_name}.xml")));
        if candidate.exists() {
            path = candidate;
        } else if let Some(sibling) = sibling.filter(|candidate| candidate.exists()) {
            path = sibling;
        } else {
            return Err(format!(
                "Directory given but no {dir_name}.xml found inside or as sibling"
            ));
        }
    }

    if !path.exists() {
        let file_name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let parent_dir = path.parent();
        let parent_dir_name = parent_dir
            .and_then(|parent| parent.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if file_name == parent_dir_name {
            if let Some(grandparent) = parent_dir.and_then(Path::parent) {
                let candidate = grandparent.join(format!("{file_name}.xml"));
                if candidate.exists() {
                    path = candidate;
                }
            }
        }
    }

    if !path.exists() {
        return Err(format!("Object file not found: {}", raw.display()));
    }
    Ok(path)
}

pub(crate) fn meta_edit_object_identity(xml_text: &str) -> Result<(String, String), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "Root element must be MetaDataObject, got: {}",
            root.tag_name().name()
        ));
    }
    let object = root
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "No object element found under MetaDataObject".to_string())?;
    let object_type = object.tag_name().name().to_string();
    let object_name = object
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Name")
        .and_then(|node| node.text())
        .unwrap_or("")
        .to_string();
    Ok((object_type, object_name))
}

fn meta_edit_line_number_length_policy(
    object_type: &str,
    object_path: &Path,
    context: &WorkspaceContext,
    transaction: &mut CompileTransaction,
) -> Result<MetaEditLineNumberLengthAuthorization, String> {
    if !meta_line_number_length_is_applicable(object_type) {
        return Ok(MetaEditLineNumberLengthAuthorization {
            policy: MetaEditLineNumberLengthPolicy::NotApplicable,
            provenance: None,
        });
    }

    let resolution = match resolve_platform_xml_owners_with_provenance(object_path, context) {
        Ok(resolution) => resolution,
        Err(_) => {
            return Ok(MetaEditLineNumberLengthAuthorization {
                policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
                provenance: None,
            })
        }
    };
    let Some(owner) = resolution.owners.iter().find(|owner| {
        matches!(
            owner.kind,
            PlatformXmlOwnerKind::Configuration | PlatformXmlOwnerKind::Extension
        )
    }) else {
        return Ok(MetaEditLineNumberLengthAuthorization {
            policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
            provenance: None,
        });
    };
    if owner.path != object_path {
        transaction.guard_or_verify_exact_preimage(&owner.path, &owner.raw)?;
    }
    let property_name = match owner.kind {
        PlatformXmlOwnerKind::Configuration => "CompatibilityMode",
        PlatformXmlOwnerKind::Extension => "ConfigurationExtensionCompatibilityMode",
        _ => unreachable!("configuration-like owners were filtered above"),
    };
    let owner_text = std::str::from_utf8(&owner.raw)
        .map_err(|error| format!("failed to read {}: {error}", owner.path.display()))?;
    let document = Document::parse(owner_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("failed to parse {}: {error}", owner.path.display()))?;
    let Some(mode) = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == property_name)
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
    else {
        return Ok(MetaEditLineNumberLengthAuthorization {
            policy: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
            provenance: None,
        });
    };

    Ok(MetaEditLineNumberLengthAuthorization {
        policy: meta_edit_line_number_length_policy_from_mode(mode),
        provenance: Some(resolution.provenance),
    })
}

pub(crate) fn meta_edit_line_number_length_policy_from_mode(
    mode: &str,
) -> MetaEditLineNumberLengthPolicy {
    meta_edit_line_number_length_policy_for_platform(mode, ACTIVE_FORMAT_PROFILE.platform_line)
}

pub(crate) fn meta_edit_line_number_length_policy_for_platform(
    mode: &str,
    platform_line: &str,
) -> MetaEditLineNumberLengthPolicy {
    if !cf_validate_enum_allowed("CompatibilityMode").contains(&mode) {
        return MetaEditLineNumberLengthPolicy::UnknownCompatibility;
    }
    let version = if mode == "DontUse" {
        meta_edit_parse_platform_line(platform_line)
    } else {
        mode.strip_prefix("Version")
            .and_then(meta_edit_parse_compatibility_version)
    };
    match version {
        Some(version) if version > (8, 3, 26) => MetaEditLineNumberLengthPolicy::Editable,
        Some(_) => MetaEditLineNumberLengthPolicy::FixedFive,
        None => MetaEditLineNumberLengthPolicy::UnknownCompatibility,
    }
}

pub(crate) fn meta_edit_parse_platform_line(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(crate) fn meta_edit_parse_compatibility_version(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.split('_');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().map(str::parse).transpose().ok()?.unwrap_or(0);
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(crate) fn meta_edit_is_line_number_length_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "linenumberlength" | "line_number_length" | "line-number-length"
    )
}

pub(crate) fn meta_edit_changes_request_line_number_length(raw_changes: &str) -> bool {
    split_meta_edit_commas_outside_parens(raw_changes)
        .into_iter()
        .filter_map(|change| change.split_once('='))
        .any(|(key, _)| meta_edit_is_line_number_length_key(key))
}

pub(crate) fn meta_edit_inline_requests_line_number_length(operation: &str, value: &str) -> bool {
    if !operation.eq_ignore_ascii_case("modify-ts") {
        return false;
    }
    split_meta_edit_batch_items(value, operation)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.split_once(':'))
        .any(|(_, changes)| meta_edit_changes_request_line_number_length(changes))
}

pub(crate) fn meta_edit_definition_requests_line_number_length(definition: &Value) -> bool {
    let Some(definition) = definition.as_object() else {
        return false;
    };
    definition.iter().any(|(operation, operation_value)| {
        if meta_edit_operation_key(operation).as_deref() != Some("modify") {
            return false;
        }
        let Some(modify) = operation_value.as_object() else {
            return false;
        };
        modify.iter().any(|(child_type, child_value)| {
            if meta_edit_child_type_key(child_type) != Some("tabularSections") {
                return false;
            }
            child_value
                .as_object()
                .into_iter()
                .flat_map(|sections| sections.values())
                .filter_map(Value::as_object)
                .flat_map(|changes| changes.keys())
                .any(|key| meta_edit_is_line_number_length_key(key))
        })
    })
}

pub(crate) fn split_meta_edit_batch_items<'a>(
    raw_value: &'a str,
    operation: &str,
) -> Result<Vec<&'a str>, String> {
    let items = raw_value
        .split(";;")
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Err(format!("{operation} requires non-empty Value"));
    }
    Ok(items)
}

pub(crate) fn meta_edit_apply_inline_operation(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    operation: &str,
    value: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let (action, target) = operation
        .split_once('-')
        .ok_or_else(|| format!("Invalid meta-edit Operation: {operation}"))?;

    if let Some(property) = meta_edit_complex_property_from_inline_target(target) {
        meta_edit_apply_complex_property_action(
            xml_text,
            object_type,
            object_name,
            action,
            property,
            meta_edit_split_values(value),
            counts,
        )?;
        return Ok(());
    }

    if target == "ts-attribute" {
        match action {
            "add" => {
                for item in split_meta_edit_batch_items(value, operation)? {
                    meta_edit_add_tabular_section_attribute(xml_text, item)?;
                    counts.added += 1;
                }
            }
            "remove" => {
                counts.removed += meta_edit_remove_tabular_section_attribute(xml_text, value)?
            }
            "modify" => {
                counts.modified += meta_edit_modify_tabular_section_attribute(xml_text, value)?
            }
            _ => return Err(format!("Unsupported meta-edit Operation: {operation}")),
        }
        return Ok(());
    }

    if target == "property" {
        if action != "modify" {
            return Err(format!("Unsupported meta-edit Operation: {operation}"));
        }
        counts.modified += meta_edit_modify_object_properties_from_pairs(xml_text, value)?;
        return Ok(());
    }

    let Some(child_type) = meta_edit_child_type_from_inline_target(target) else {
        return Err(format!("Unsupported meta-edit Operation: {operation}"));
    };

    match action {
        "add" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                let item_value = Value::String(item.to_string());
                meta_edit_add_child_value(
                    xml_text,
                    object_type,
                    object_name,
                    child_type,
                    &item_value,
                )?;
                counts.added += 1;
            }
        }
        "remove" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                meta_edit_remove_child_value(
                    xml_text,
                    child_type,
                    &Value::String(item.to_string()),
                )?;
                counts.removed += 1;
            }
        }
        "modify" => {
            for item in split_meta_edit_batch_items(value, operation)? {
                let (name, raw_changes) = item
                    .split_once(':')
                    .ok_or_else(|| format!("{operation} requires Value like Name: key=value"))?;
                counts.modified += meta_edit_modify_top_child(
                    xml_text,
                    child_type,
                    name.trim(),
                    raw_changes.trim(),
                    line_number_length_policy,
                )?;
            }
        }
        _ => return Err(format!("Unsupported meta-edit Operation: {operation}")),
    }

    Ok(())
}

pub(crate) fn meta_edit_apply_definition(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    definition: &Value,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let definition = definition
        .as_object()
        .ok_or_else(|| "DefinitionFile root must be a JSON object".to_string())?;

    if let Some(Value::Array(items)) = definition.get("_complex") {
        for item in items {
            let object = item
                .as_object()
                .ok_or_else(|| "_complex item must be an object".to_string())?;
            let action = object
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| "_complex item is missing action".to_string())?;
            let property = object
                .get("property")
                .and_then(Value::as_str)
                .ok_or_else(|| "_complex item is missing property".to_string())?;
            let values = meta_edit_values_from_json(object.get("values"));
            meta_edit_apply_complex_property_action(
                xml_text,
                object_type,
                object_name,
                action,
                property,
                values,
                counts,
            )?;
        }
    }

    for (raw_key, value) in definition {
        if raw_key == "_complex" {
            continue;
        }
        match meta_edit_operation_key(raw_key).as_deref() {
            Some("add") => {
                meta_edit_apply_definition_add(xml_text, object_type, object_name, value, counts)?
            }
            Some("remove") => meta_edit_apply_definition_remove(xml_text, value, counts)?,
            Some("modify") => meta_edit_apply_definition_modify(
                xml_text,
                object_type,
                object_name,
                value,
                line_number_length_policy,
                counts,
            )?,
            Some(other) => return Err(format!("Unsupported definition operation: {other}")),
            None => return Err(format!("Unknown definition operation: {raw_key}")),
        }
    }

    Ok(())
}

pub(crate) fn meta_edit_apply_definition_add(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    value: &Value,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition add must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown add child type: {raw_child_type}"))?;
        for item in meta_edit_definition_items(items) {
            meta_edit_add_child_value(xml_text, object_type, object_name, child_type, &item)?;
            counts.added += 1;
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_apply_definition_remove(
    xml_text: &mut String,
    value: &Value,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition remove must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown remove child type: {raw_child_type}"))?;
        for item in meta_edit_definition_items(items) {
            meta_edit_remove_child_value(xml_text, child_type, &item)?;
            counts.removed += 1;
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_apply_definition_modify(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    value: &Value,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "definition modify must be an object".to_string())?;
    for (raw_child_type, items) in object {
        let child_type = meta_edit_child_type_key(raw_child_type)
            .ok_or_else(|| format!("Unknown modify child type: {raw_child_type}"))?;
        if child_type == "properties" {
            meta_edit_modify_object_properties_from_map(
                xml_text,
                object_type,
                object_name,
                items,
                counts,
            )?;
        } else if child_type == "tabularSections" {
            meta_edit_modify_tabular_sections_from_definition(
                xml_text,
                items,
                line_number_length_policy,
                counts,
            )?;
        } else {
            let item_object = items
                .as_object()
                .ok_or_else(|| format!("modify {child_type} must be an object"))?;
            for (name, changes) in item_object {
                let raw_changes = meta_edit_changes_to_inline(changes)?;
                counts.modified += meta_edit_modify_top_child(
                    xml_text,
                    child_type,
                    name,
                    &raw_changes,
                    line_number_length_policy,
                )?;
            }
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_definition_info_lines(definition: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = definition.as_object() else {
        return lines;
    };

    for (raw_key, value) in object {
        if raw_key == "_complex" {
            continue;
        }
        match meta_edit_operation_key(raw_key).as_deref() {
            Some("add") => lines.extend(meta_edit_definition_add_info_lines(value)),
            Some("remove") => lines.extend(meta_edit_definition_remove_info_lines(value)),
            Some("modify") => lines.extend(meta_edit_definition_modify_info_lines(value)),
            _ => {}
        }
    }

    lines
}

pub(crate) fn meta_edit_definition_add_info_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = value.as_object() else {
        return lines;
    };

    for (raw_child_type, items) in object {
        let Some(child_type) = meta_edit_child_type_key(raw_child_type) else {
            continue;
        };
        for item in meta_edit_definition_items(items) {
            if let Some(name) = meta_edit_log_child_name(child_type, &item) {
                lines.push(format!(
                    "[INFO] Added {}: {name}",
                    meta_edit_added_child_log_label(child_type)
                ));
            }
        }
    }

    lines
}

pub(crate) fn meta_edit_definition_remove_info_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = value.as_object() else {
        return lines;
    };

    for (raw_child_type, items) in object {
        let Some(child_type) = meta_edit_child_type_key(raw_child_type) else {
            continue;
        };
        let label = meta_edit_child_xml_tag(child_type)
            .map(|tag| tag.to_ascii_lowercase())
            .unwrap_or_else(|| child_type.to_string());
        for item in meta_edit_definition_items(items) {
            if let Some(name) = meta_edit_value_name(&item) {
                lines.push(format!("[INFO] Removed {label}: {name}"));
            }
        }
    }

    lines
}

pub(crate) fn meta_edit_definition_modify_info_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = value.as_object() else {
        return lines;
    };

    for (raw_child_type, items) in object {
        let Some(child_type) = meta_edit_child_type_key(raw_child_type) else {
            continue;
        };
        if child_type == "properties" {
            if let Some(properties) = items.as_object() {
                for (key, value) in properties {
                    if meta_edit_complex_property_kind(key).is_none() {
                        lines.push(format!(
                            "[INFO] Modified property: {key} = {}",
                            json_value_to_python_string(value)
                        ));
                    }
                }
            }
        } else if child_type == "tabularSections" {
            lines.extend(meta_edit_tabular_section_definition_info_lines(items));
        } else if let Some(item_object) = items.as_object() {
            if let Some(tag) = meta_edit_child_xml_tag(child_type) {
                for (name, changes) in item_object {
                    lines.extend(meta_edit_modify_child_info_lines(tag, name, changes));
                }
            }
        }
    }

    lines
}

pub(crate) fn meta_edit_tabular_section_definition_info_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(object) = value.as_object() else {
        return lines;
    };

    for (section_name, changes) in object {
        let Some(changes) = changes.as_object() else {
            continue;
        };
        for (raw_key, change_value) in changes {
            match meta_edit_operation_key(raw_key).as_deref() {
                Some("add") => {
                    for item in meta_edit_definition_items(change_value) {
                        let attr = meta_compile_parse_attr(&item);
                        if !attr.name.is_empty() {
                            lines.push(format!(
                                "[INFO] Added attribute to TS '{section_name}': {}",
                                attr.name
                            ));
                        }
                    }
                }
                Some("remove") => {
                    for item in meta_edit_definition_items(change_value) {
                        if let Some(attr_name) = meta_edit_value_name(&item) {
                            lines.push(format!(
                                "[INFO] Removed attribute from TS '{section_name}': {attr_name}"
                            ));
                        }
                    }
                }
                Some("modify") => {
                    if let Some(attrs) = change_value.as_object() {
                        for (attr_name, attr_changes) in attrs {
                            lines.extend(meta_edit_modify_child_info_lines(
                                "Attribute",
                                attr_name,
                                attr_changes,
                            ));
                        }
                    }
                }
                _ => {
                    let mut scalar_change = Map::new();
                    scalar_change.insert(raw_key.to_string(), change_value.clone());
                    lines.extend(meta_edit_modify_child_info_lines(
                        "TabularSection",
                        section_name,
                        &Value::Object(scalar_change),
                    ));
                }
            }
        }
    }

    lines
}

pub(crate) fn meta_edit_modify_child_info_lines(
    xml_tag: &str,
    child_name: &str,
    changes: &Value,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (key, value) in meta_edit_log_change_items(changes) {
        match key.as_str() {
            "name" => lines.push(format!("[INFO] Renamed {xml_tag}: {child_name} -> {value}")),
            "type" => lines.push(format!(
                "[INFO] Changed type of {xml_tag} '{child_name}': {value}"
            )),
            "synonym" => lines.push(format!(
                "[INFO] Changed synonym of {xml_tag} '{child_name}': {value}"
            )),
            _ => lines.push(format!(
                "[INFO] Modified {xml_tag} '{child_name}'.{key} = {value}"
            )),
        }
    }
    lines
}

pub(crate) fn meta_edit_log_change_items(value: &Value) -> Vec<(String, String)> {
    if let Some(text) = value.as_str() {
        return split_meta_edit_commas_outside_parens(text)
            .into_iter()
            .filter_map(|change| {
                let (key, value) = change.split_once('=')?;
                Some((key.trim().to_string(), value.trim().to_string()))
            })
            .collect();
    }
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .map(|(key, value)| (key.to_string(), json_value_to_python_string(value)))
        .collect()
}

pub(crate) fn meta_edit_added_child_log_label(child_type: &str) -> &'static str {
    match child_type {
        "attributes" => "attribute",
        "tabularSections" => "tabular section",
        "dimensions" => "dimension",
        "resources" => "resource",
        "enumValues" => "enum value",
        "columns" => "column",
        "forms" => "form",
        "templates" => "template",
        "commands" => "command",
        _ => "item",
    }
}

pub(crate) fn meta_edit_log_child_name(child_type: &str, value: &Value) -> Option<String> {
    let name = match child_type {
        "attributes" | "dimensions" | "resources" => meta_compile_parse_attr(value).name,
        "tabularSections" => meta_edit_tabular_section_from_value(value).ok()?.name,
        "enumValues" => meta_edit_enum_value_from_value(value).ok()?.name,
        "columns" => meta_edit_value_name(&meta_edit_column_value(value))?,
        "forms" | "templates" | "commands" => meta_edit_value_name(value)?,
        _ => return None,
    };
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

pub(crate) fn meta_edit_modify_object_properties_from_pairs(
    xml_text: &mut String,
    value: &str,
) -> Result<usize, String> {
    let mut modified = 0usize;
    for pair in value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Some((key, raw_value)) = pair.split_once('=') else {
            return Err(format!("modify-property requires Key=Value, got: {pair}"));
        };
        meta_edit_set_scalar_property(xml_text, key.trim(), raw_value.trim())?;
        modified += 1;
    }
    if modified == 0 {
        return Err("modify-property requires non-empty Value".to_string());
    }
    Ok(modified)
}

pub(crate) fn meta_edit_modify_object_properties_from_map(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    value: &Value,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "modify.properties must be an object".to_string())?;
    for (key, value) in object {
        if meta_edit_complex_property_kind(key).is_some() {
            meta_edit_apply_complex_property_action(
                xml_text,
                object_type,
                object_name,
                "set",
                key,
                meta_edit_values_from_json(Some(value)),
                counts,
            )?;
        } else {
            meta_edit_set_scalar_property(xml_text, key, &json_value_to_python_string(value))?;
            counts.modified += 1;
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_set_scalar_property(
    xml_text: &mut String,
    key: &str,
    raw_value: &str,
) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("modify-property requires non-empty key".to_string());
    }
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let object_type = object.tag_name().name();
    let normalized = normalize_meta_edit_scalar_property_value(object_type, key, raw_value);
    if key == "Name" {
        validate_meta_compile_name("meta.edit object", &normalized)?;
    }
    let properties = meta_info_child(object, "Properties")
        .ok_or_else(|| "Object has no Properties".to_string())?;
    let matching_properties = meta_info_children(properties, key).len();
    if matching_properties == 0 {
        return Err(format!(
            "direct scalar property <{key}> does not exist in object Properties"
        ));
    }
    if matching_properties > 1 {
        return Err(format!(
            "Properties contains {matching_properties} direct <{key}> elements; expected at most one"
        ));
    }
    validate_meta_8_3_27_property_value("meta.edit", key, &normalized)?;
    validate_meta_8_3_27_boolean_property_value("meta.edit", object_type, key, &normalized)?;
    let range = properties.range();
    drop(doc);

    let mut properties_text = xml_text[range.clone()].to_string();
    let child_indent = meta_edit_property_child_indent(&properties_text);
    let replacement = format!("{child_indent}<{key}>{}</{key}>", escape_xml(&normalized));
    meta_edit_replace_or_insert_property(&mut properties_text, key, &replacement, &child_indent)?;
    xml_text.replace_range(range, &properties_text);
    Ok(())
}

pub(crate) fn meta_edit_add_child_value(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    child_type: &str,
    value: &Value,
) -> Result<(), String> {
    let (value, position) = meta_edit_extract_insert_position(value)?;
    match child_type {
        "attributes" => {
            let attr = meta_compile_parse_attr(&value);
            if attr.name.is_empty() {
                return Err("add-attribute requires Value like Name: Type".to_string());
            }
            validate_meta_compile_attr_type(&attr, "meta.edit add attribute")?;
            meta_edit_ensure_top_child_name_free(xml_text, "Attribute", &attr.name)?;
            let context = meta_edit_attribute_context(object_type);
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_attribute(&mut lines, "\t\t\t", &attr, context, &mut next_uuid);
            meta_edit_insert_top_child_object_with_position(
                xml_text,
                "Attribute",
                &position,
                &lines,
            )
        }
        "tabularSections" => {
            let section = meta_edit_tabular_section_from_value(&value)?;
            meta_edit_ensure_top_child_name_free(xml_text, "TabularSection", &section.name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_tabular_section(
                &mut lines,
                "\t\t\t",
                &section,
                object_type,
                object_name,
                &mut next_uuid,
            );
            meta_edit_insert_top_child_object_with_position(
                xml_text,
                "TabularSection",
                &position,
                &lines,
            )
        }
        "dimensions" | "resources" => {
            let attr = meta_compile_parse_attr(&value);
            if attr.name.is_empty() {
                return Err(format!("add-{child_type} requires Value like Name: Type"));
            }
            validate_meta_compile_attr_type(&attr, "meta.edit add register field")?;
            let tag = if child_type == "dimensions" {
                "Dimension"
            } else {
                "Resource"
            };
            meta_edit_ensure_top_child_name_free(xml_text, tag, &attr.name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_register_field(
                &mut lines,
                "\t\t\t",
                tag,
                &attr,
                object_type,
                &mut next_uuid,
            );
            meta_edit_insert_top_child_object_with_position(xml_text, tag, &position, &lines)
        }
        "enumValues" => {
            let enum_value = meta_edit_enum_value_from_value(&value)?;
            validate_meta_compile_name("meta.edit enum value", &enum_value.name)?;
            meta_edit_ensure_top_child_name_free(xml_text, "EnumValue", &enum_value.name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_enum_value(&mut lines, "\t\t\t", &enum_value, &mut next_uuid);
            meta_edit_insert_top_child_object_with_position(
                xml_text,
                "EnumValue",
                &position,
                &lines,
            )
        }
        "columns" => {
            let column_value = meta_edit_column_value(&value);
            let column_name = meta_edit_value_name(&column_value)
                .ok_or_else(|| "add-column requires non-empty name".to_string())?;
            validate_meta_compile_name("meta.edit column", &column_name)?;
            meta_edit_ensure_top_child_name_free(xml_text, "Column", &column_name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_column(&mut lines, "\t\t\t", &column_value, &mut next_uuid);
            meta_edit_insert_top_child_object_with_position(xml_text, "Column", &position, &lines)
        }
        "forms" | "templates" | "commands" => {
            let tag = match child_type {
                "forms" => "Form",
                "templates" => "Template",
                _ => "Command",
            };
            let name = meta_edit_value_name(&value)
                .ok_or_else(|| format!("add-{child_type} requires non-empty name"))?;
            validate_meta_compile_name(&format!("meta.edit {child_type}"), &name)?;
            meta_edit_ensure_top_child_name_free(xml_text, tag, &name)?;
            let mut lines = Vec::new();
            let mut next_uuid = fresh_meta_compile_uuid;
            emit_meta_simple_child(&mut lines, "\t\t\t", tag, &name, &mut next_uuid);
            meta_edit_insert_top_child_object_with_position(xml_text, tag, &position, &lines)
        }
        other => Err(format!("Unsupported add child type: {other}")),
    }
}

pub(crate) fn meta_edit_remove_child_value(
    xml_text: &mut String,
    child_type: &str,
    value: &Value,
) -> Result<(), String> {
    let tag = meta_edit_child_xml_tag(child_type)
        .ok_or_else(|| format!("Unsupported remove child type: {child_type}"))?;
    let name = meta_edit_value_name(value)
        .ok_or_else(|| format!("remove {child_type} requires non-empty name"))?;
    meta_edit_remove_top_child_by_name(xml_text, tag, &name)
}

pub(crate) fn meta_edit_modify_top_child(
    xml_text: &mut String,
    child_type: &str,
    name: &str,
    raw_changes: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
) -> Result<usize, String> {
    let tag = meta_edit_child_xml_tag(child_type)
        .ok_or_else(|| format!("Unsupported modify child type: {child_type}"))?;
    let target = match child_type {
        "attributes" => {
            let (object_type, _) = meta_edit_object_identity(xml_text)?;
            MetaEditModifyTarget::Attribute {
                fill_value_allowed: !matches!(object_type.as_str(), "DataProcessor" | "Report"),
            }
        }
        "dimensions" | "resources" => MetaEditModifyTarget::RegisterField,
        "enumValues" => MetaEditModifyTarget::EnumValue,
        "columns" => MetaEditModifyTarget::Column,
        "tabularSections" => MetaEditModifyTarget::TabularSection {
            line_number_length: line_number_length_policy,
        },
        _ => return Err(format!("Unsupported modify child type: {child_type}")),
    };
    meta_edit_modify_top_child_properties(xml_text, tag, name, raw_changes, target)
}

pub(crate) fn meta_edit_modify_tabular_sections_from_definition(
    xml_text: &mut String,
    value: &Value,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "modify.tabularSections must be an object".to_string())?;
    for (section_name, changes) in object {
        let changes = changes
            .as_object()
            .ok_or_else(|| "tabular section modify entry must be an object".to_string())?;
        let mut section_property_changes = Map::new();
        for (raw_key, change_value) in changes {
            match meta_edit_operation_key(raw_key).as_deref() {
                Some("add") => {
                    for item in meta_edit_definition_items(change_value) {
                        meta_edit_add_tabular_section_attribute_value(
                            xml_text,
                            section_name,
                            &item,
                        )?;
                        counts.added += 1;
                    }
                }
                Some("remove") => {
                    for item in meta_edit_definition_items(change_value) {
                        let attr_name = meta_edit_value_name(&item).ok_or_else(|| {
                            format!("remove attribute from TS '{section_name}' requires name")
                        })?;
                        meta_edit_remove_tabular_child_by_name(
                            xml_text,
                            section_name,
                            "Attribute",
                            &attr_name,
                        )?;
                        counts.removed += 1;
                    }
                }
                Some("modify") => {
                    let attrs = change_value.as_object().ok_or_else(|| {
                        format!("modify attributes in TS '{section_name}' must be an object")
                    })?;
                    for (attr_name, attr_changes) in attrs {
                        let raw_changes = meta_edit_changes_to_inline(attr_changes)?;
                        let modified = meta_edit_modify_tabular_attribute_properties(
                            xml_text,
                            section_name,
                            attr_name,
                            &raw_changes,
                        )?;
                        counts.modified += modified;
                    }
                }
                _ => {
                    section_property_changes.insert(raw_key.to_string(), change_value.clone());
                }
            }
        }
        if !section_property_changes.is_empty() {
            let raw_changes =
                meta_edit_changes_to_inline(&Value::Object(section_property_changes))?;
            meta_edit_modify_tabular_section_properties(
                xml_text,
                section_name,
                &raw_changes,
                line_number_length_policy,
            )?;
            counts.modified += 1;
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_apply_complex_property_action(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    action: &str,
    property: &str,
    raw_values: Vec<String>,
    counts: &mut MetaEditCounts,
) -> Result<(), String> {
    let property = meta_edit_complex_property_kind(property)
        .ok_or_else(|| format!("Unsupported complex property: {property}"))?;
    if property == "RegisterRecords" && object_type != "Document" {
        return Err(format!(
            "RegisterRecords is supported for Document only, got: {object_type}"
        ));
    }
    let values = raw_values
        .into_iter()
        .map(|value| {
            meta_edit_normalize_complex_property_value(property, object_type, object_name, &value)
        })
        .collect::<Vec<_>>();
    if property == "RegisterRecords" {
        for value in &values {
            if !matches!(
                value.split('.').next().unwrap_or_default(),
                "AccumulationRegister"
                    | "InformationRegister"
                    | "AccountingRegister"
                    | "CalculationRegister"
            ) {
                return Err(format!(
                    "RegisterRecords value must be a register reference, got: {value}"
                ));
            }
        }
    }
    let existing = meta_edit_complex_property_values(xml_text, property)?;
    match action {
        "add" => {
            let mut next = existing;
            for value in values {
                if next.iter().any(|existing| existing == &value) {
                    return Err(format!("{property} item '{value}' already exists"));
                }
                next.push(value);
                counts.added += 1;
            }
            meta_edit_replace_complex_property(xml_text, property, &next)
        }
        "remove" => {
            let mut next = existing;
            for value in values {
                let Some(index) = next.iter().position(|existing| existing == &value) else {
                    return Err(format!("{property} item '{value}' not found"));
                };
                next.remove(index);
                counts.removed += 1;
            }
            meta_edit_replace_complex_property(xml_text, property, &next)
        }
        "set" => {
            meta_edit_replace_complex_property(xml_text, property, &values)?;
            counts.modified += 1;
            Ok(())
        }
        other => Err(format!("Unsupported complex property action: {other}")),
    }
}

pub(crate) fn meta_edit_complex_property_values(
    xml_text: &str,
    property: &str,
) -> Result<Vec<String>, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let Some(properties) = meta_info_child(object, "Properties") else {
        return Ok(Vec::new());
    };
    let Some(property_node) = meta_info_child(properties, property) else {
        return Ok(Vec::new());
    };
    Ok(property_node
        .children()
        .filter(|node| node.is_element())
        .filter_map(|node| node.text().map(str::trim).map(ToOwned::to_owned))
        .filter(|value| !value.is_empty())
        .collect())
}

pub(crate) fn meta_edit_replace_complex_property(
    xml_text: &mut String,
    property: &str,
    values: &[String],
) -> Result<(), String> {
    let replacement = meta_edit_complex_property_xml(xml_text, property, values)?;
    if let Some(range) = meta_edit_xml_element_range(xml_text, property)? {
        xml_text.replace_range(range, &replacement);
        return Ok(());
    }
    let Some(close_pos) = xml_text.find("</Properties>") else {
        return Err("No closing </Properties> found".to_string());
    };
    xml_text.insert_str(close_pos, &format!("{replacement}\n\t\t\t"));
    Ok(())
}

pub(crate) fn meta_edit_complex_property_xml(
    xml_text: &str,
    property: &str,
    values: &[String],
) -> Result<String, String> {
    let indent = if let Some(range) = meta_edit_xml_element_range(xml_text, property)? {
        meta_edit_line_indent(xml_text, range.start)
    } else {
        "\t\t\t".to_string()
    };
    if values.is_empty() {
        return Ok(format!("{indent}<{property}/>"));
    }
    let child_indent = format!("{indent}\t");
    let mut lines = vec![format!("{indent}<{property}>")];
    for value in values {
        if property == "InputByString" {
            lines.push(format!(
                "{child_indent}<xr:Field>{}</xr:Field>",
                escape_xml(value)
            ));
        } else {
            lines.push(format!(
                "{child_indent}<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
                escape_xml(value)
            ));
        }
    }
    lines.push(format!("{indent}</{property}>"));
    Ok(lines.join("\n"))
}

pub(crate) fn meta_edit_line_indent(text: &str, pos: usize) -> String {
    let line_start = text[..pos].rfind('\n').map_or(0, |index| index + 1);
    text[line_start..pos]
        .chars()
        .take_while(|ch| *ch == '\t' || *ch == ' ')
        .collect()
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MetaEditInsertPosition {
    pub(crate) before: Option<String>,
    pub(crate) after: Option<String>,
}

impl MetaEditInsertPosition {
    pub(crate) fn is_empty(&self) -> bool {
        self.before.is_none() && self.after.is_none()
    }

    pub(crate) fn target(&self) -> Option<(&str, bool)> {
        if let Some(after) = self.after.as_deref() {
            Some((after, true))
        } else {
            self.before.as_deref().map(|before| (before, false))
        }
    }
}

pub(crate) fn meta_edit_extract_insert_position(
    value: &Value,
) -> Result<(Value, MetaEditInsertPosition), String> {
    if let Some(text) = value.as_str() {
        let (cleaned, position) = meta_edit_extract_insert_position_from_text(text)?;
        return Ok((Value::String(cleaned), position));
    }
    if let Some(object) = value.as_object() {
        let mut object = object.clone();
        let before = object
            .remove("before")
            .and_then(|value| value.as_str().map(str::trim).map(ToOwned::to_owned))
            .filter(|value| !value.is_empty());
        let after = object
            .remove("after")
            .and_then(|value| value.as_str().map(str::trim).map(ToOwned::to_owned))
            .filter(|value| !value.is_empty());
        if before.is_some() && after.is_some() {
            return Err("Use either before or after, not both".to_string());
        }
        return Ok((
            Value::Object(object),
            MetaEditInsertPosition { before, after },
        ));
    }
    Ok((value.clone(), MetaEditInsertPosition::default()))
}

pub(crate) fn meta_edit_extract_insert_position_from_text(
    text: &str,
) -> Result<(String, MetaEditInsertPosition), String> {
    let after_marker = ">> after ";
    let before_marker = "<< before ";
    let after_pos = text.rfind(after_marker);
    let before_pos = text.rfind(before_marker);
    let Some((marker_pos, marker, is_after)) = (match (after_pos, before_pos) {
        (Some(_), Some(_)) => {
            return Err("Use either >> after or << before, not both".to_string());
        }
        (Some(pos), None) => Some((pos, after_marker, true)),
        (None, Some(pos)) => Some((pos, before_marker, false)),
        (None, None) => None,
    }) else {
        return Ok((text.trim().to_string(), MetaEditInsertPosition::default()));
    };

    let cleaned = text[..marker_pos].trim().to_string();
    let target = text[marker_pos + marker.len()..].trim();
    if target.is_empty() {
        return Err("Position target must be non-empty".to_string());
    }
    let position = if is_after {
        MetaEditInsertPosition {
            before: None,
            after: Some(target.to_string()),
        }
    } else {
        MetaEditInsertPosition {
            before: Some(target.to_string()),
            after: None,
        }
    };
    Ok((cleaned, position))
}

pub(crate) fn meta_edit_normalize_complex_property_value(
    property: &str,
    object_type: &str,
    object_name: &str,
    value: &str,
) -> String {
    let value = value.trim();
    if property != "InputByString" {
        return normalize_meta_object_ref(value);
    }
    let first = value.split('.').next().unwrap_or_default();
    let is_prefixed = matches!(
        first,
        "Catalog"
            | "Document"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "ChartOfAccounts"
            | "ExchangePlan"
            | "BusinessProcess"
            | "Task"
            | "Enum"
            | "Report"
            | "DataProcessor"
    );
    if is_prefixed {
        value.to_string()
    } else {
        format!("{object_type}.{object_name}.{value}")
    }
}

pub(crate) fn meta_edit_complex_property_from_inline_target(target: &str) -> Option<&'static str> {
    match target {
        "owner" | "owners" => Some("Owners"),
        "registerRecord" | "registerRecords" => Some("RegisterRecords"),
        "basedOn" => Some("BasedOn"),
        "inputByString" => Some("InputByString"),
        _ => None,
    }
}

pub(crate) fn meta_edit_complex_property_kind(property: &str) -> Option<&'static str> {
    match property {
        "Owners" | "owners" => Some("Owners"),
        "RegisterRecords" | "registerRecords" => Some("RegisterRecords"),
        "BasedOn" | "basedOn" => Some("BasedOn"),
        "InputByString" | "inputByString" => Some("InputByString"),
        _ => None,
    }
}

pub(crate) fn meta_edit_operation_key(key: &str) -> Option<String> {
    match key.to_lowercase().as_str() {
        "add" | "добавить" => Some("add".to_string()),
        "remove" | "удалить" => Some("remove".to_string()),
        "modify" | "изменить" => Some("modify".to_string()),
        _ => None,
    }
}

pub(crate) fn meta_edit_child_type_from_inline_target(target: &str) -> Option<&'static str> {
    match target {
        "attribute" => Some("attributes"),
        "ts" => Some("tabularSections"),
        "dimension" => Some("dimensions"),
        "resource" => Some("resources"),
        "enumValue" => Some("enumValues"),
        "column" => Some("columns"),
        "form" => Some("forms"),
        "template" => Some("templates"),
        "command" => Some("commands"),
        _ => None,
    }
}

pub(crate) fn meta_edit_child_type_key(key: &str) -> Option<&'static str> {
    match key.to_lowercase().as_str() {
        "attributes" | "реквизиты" | "attrs" => Some("attributes"),
        "tabularsections" | "табличныечасти" | "тч" | "ts" => {
            Some("tabularSections")
        }
        "dimensions" | "измерения" | "dims" => Some("dimensions"),
        "resources" | "ресурсы" | "res" => Some("resources"),
        "enumvalues" | "значения" | "values" => Some("enumValues"),
        "columns" | "графы" | "колонки" => Some("columns"),
        "forms" | "формы" => Some("forms"),
        "templates" | "макеты" => Some("templates"),
        "commands" | "команды" => Some("commands"),
        "properties" | "свойства" => Some("properties"),
        _ => None,
    }
}

pub(crate) fn meta_edit_child_xml_tag(child_type: &str) -> Option<&'static str> {
    match child_type {
        "attributes" => Some("Attribute"),
        "tabularSections" => Some("TabularSection"),
        "dimensions" => Some("Dimension"),
        "resources" => Some("Resource"),
        "enumValues" => Some("EnumValue"),
        "columns" => Some("Column"),
        "forms" => Some("Form"),
        "templates" => Some("Template"),
        "commands" => Some("Command"),
        _ => None,
    }
}

pub(crate) fn meta_edit_split_values(value: &str) -> Vec<String> {
    value
        .split(";;")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn meta_edit_values_from_json(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .map(json_value_to_python_string)
            .filter(|value| !value.trim().is_empty())
            .collect(),
        Some(Value::String(text)) => meta_edit_split_values(text),
        Some(value) => vec![json_value_to_python_string(value)],
        None => Vec::new(),
    }
}

pub(crate) fn meta_edit_definition_items(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.clone(),
        Value::String(_) => vec![value.clone()],
        Value::Object(object) if object.contains_key("name") => vec![value.clone()],
        Value::Object(object) => object
            .iter()
            .map(|(name, item)| {
                if let Some(mut item_object) = item.as_object().cloned() {
                    item_object
                        .entry("name".to_string())
                        .or_insert_with(|| Value::String(name.clone()));
                    Value::Object(item_object)
                } else if let Some(type_text) = item.as_str() {
                    Value::String(format!("{name}: {type_text}"))
                } else {
                    Value::String(name.clone())
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn meta_edit_value_name(value: &Value) -> Option<String> {
    value.as_str().map(ToOwned::to_owned).or_else(|| {
        value
            .as_object()
            .and_then(|object| object.get("name"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

pub(crate) fn meta_edit_changes_to_inline(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    let object = value
        .as_object()
        .ok_or_else(|| "modify changes must be an object or string".to_string())?;
    Ok(object
        .iter()
        .map(|(key, value)| format!("{key}={}", json_value_to_python_string(value)))
        .collect::<Vec<_>>()
        .join(", "))
}

pub(crate) fn meta_edit_tabular_section_from_value(
    value: &Value,
) -> Result<MetaCompileTabularSection, String> {
    if let Some(text) = value.as_str() {
        return meta_edit_parse_tabular_section(text);
    }
    let object = value
        .as_object()
        .ok_or_else(|| "tabular section must be a string or object".to_string())?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "tabular section is missing name".to_string())?;
    let columns_value = object
        .get("attrs")
        .or_else(|| object.get("attributes"))
        .or_else(|| object.get("реквизиты"));
    let section = MetaCompileTabularSection {
        name: name.to_string(),
        columns: meta_compile_attributes(columns_value),
    };
    validate_meta_compile_tabular_section_types(&section, "meta.edit tabular section")?;
    Ok(section)
}

pub(crate) fn meta_edit_enum_value_from_value(
    value: &Value,
) -> Result<MetaCompileEnumValue, String> {
    let mut values = meta_compile_enum_values(Some(&Value::Array(vec![value.clone()])))?;
    values
        .pop()
        .ok_or_else(|| "enum value is missing name".to_string())
}

pub(crate) fn meta_edit_column_value(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        if let Some((name, reference)) = text.split_once(':') {
            let mut object = Map::new();
            object.insert("name".to_string(), Value::String(name.trim().to_string()));
            object.insert(
                "references".to_string(),
                Value::Array(vec![Value::String(reference.trim().to_string())]),
            );
            return Value::Object(object);
        }
    }
    value.clone()
}

pub(crate) fn emit_meta_simple_child<F>(
    lines: &mut Vec<String>,
    indent: &str,
    tag: &str,
    name: &str,
    next_uuid: &mut F,
) where
    F: FnMut() -> String,
{
    lines.push(format!("{indent}<{tag} uuid=\"{}\">", next_uuid()));
    lines.push(format!("{indent}\t<Properties>"));
    lines.push(format!("{indent}\t\t<Name>{}</Name>", escape_xml(name)));
    emit_meta_mltext(
        lines,
        &format!("{indent}\t\t"),
        "Synonym",
        &split_meta_camel_case(name),
    );
    lines.push(format!("{indent}\t\t<Comment/>"));
    match tag {
        "Form" => {
            lines.push(format!("{indent}\t\t<FormType>Ordinary</FormType>"));
            lines.push(format!(
                "{indent}\t\t<IncludeHelpInContents>false</IncludeHelpInContents>"
            ));
            lines.push(format!("{indent}\t\t<UsePurposes/>"));
        }
        "Template" => {
            lines.push(format!(
                "{indent}\t\t<TemplateType>SpreadsheetDocument</TemplateType>"
            ));
        }
        "Command" => {
            lines.push(format!(
                "{indent}\t\t<Group>FormNavigationPanelGoTo</Group>"
            ));
            lines.push(format!("{indent}\t\t<Representation>Auto</Representation>"));
            lines.push(format!("{indent}\t\t<ToolTip/>"));
            lines.push(format!("{indent}\t\t<Picture/>"));
            lines.push(format!("{indent}\t\t<Shortcut/>"));
        }
        _ => {}
    }
    lines.push(format!("{indent}\t</Properties>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(crate) fn meta_edit_add_register_record(
    xml_text: &mut String,
    object_type: &str,
    raw_value: &str,
) -> Result<(), String> {
    if object_type != "Document" {
        return Err(format!(
            "add-registerRecord is supported for Document only, got: {object_type}"
        ));
    }
    let value = normalize_meta_object_ref(raw_value.trim());
    if value.is_empty() {
        return Err("add-registerRecord requires non-empty Value".to_string());
    }
    if !value.starts_with("AccumulationRegister.")
        && !value.starts_with("InformationRegister.")
        && !value.starts_with("AccountingRegister.")
        && !value.starts_with("CalculationRegister.")
    {
        return Err(format!(
            "add-registerRecord Value must be a register reference, got: {value}"
        ));
    }
    if meta_edit_register_record_exists(xml_text, &value)? {
        return Err(format!("Register record '{value}' already exists"));
    }
    let item = format!(
        "<xr:Item xsi:type=\"xr:MDObjectRef\">{}</xr:Item>",
        escape_xml(&value)
    );
    if xml_text.contains(&item) {
        return Err(format!("Register record '{value}' already exists"));
    }

    if xml_text.contains("<RegisterRecords/>") {
        *xml_text = xml_text.replacen(
            "<RegisterRecords/>",
            &format!("<RegisterRecords>\n\t\t\t{item}\n\t\t</RegisterRecords>"),
            1,
        );
        return Ok(());
    }
    if let Some(close_pos) = xml_text.find("</RegisterRecords>") {
        xml_text.insert_str(close_pos, &format!("\t\t\t{item}\n\t\t"));
        return Ok(());
    }
    if let Some(pos) = xml_text.find("<PostInPrivilegedMode>") {
        xml_text.insert_str(
            pos,
            &format!("<RegisterRecords>\n\t\t\t{item}\n\t\t</RegisterRecords>\n\t\t"),
        );
        return Ok(());
    }
    let Some(pos) = xml_text.find("</Properties>") else {
        return Err("No <Properties> section found in metadata object".to_string());
    };
    xml_text.insert_str(
        pos,
        &format!("\t\t<RegisterRecords>\n\t\t\t{item}\n\t\t</RegisterRecords>\n"),
    );
    Ok(())
}

pub(crate) fn meta_edit_register_record_exists(
    xml_text: &str,
    value: &str,
) -> Result<bool, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let Some(properties) = meta_info_child(object, "Properties") else {
        return Ok(false);
    };
    let Some(register_records) = meta_info_child(properties, "RegisterRecords") else {
        return Ok(false);
    };
    Ok(meta_info_children(register_records, "Item")
        .into_iter()
        .any(|item| item.text().unwrap_or("").trim() == value))
}

pub(crate) fn meta_edit_add_attribute(
    xml_text: &mut String,
    object_type: &str,
    raw_value: &str,
) -> Result<(), String> {
    let attr = meta_compile_parse_attr(&Value::String(raw_value.trim().to_string()));
    if attr.name.is_empty() {
        return Err("add-attribute requires Value like Name: Type".to_string());
    }
    validate_meta_compile_attr_type(&attr, "meta.edit add-attribute")?;
    meta_edit_ensure_top_child_name_free(xml_text, "Attribute", &attr.name)?;
    let context = meta_edit_attribute_context(object_type);
    let mut lines = Vec::new();
    let mut next_uuid = fresh_meta_compile_uuid;
    emit_meta_attribute(&mut lines, "\t\t\t", &attr, context, &mut next_uuid);
    meta_edit_insert_top_child_object(xml_text, &lines)
}

pub(crate) fn meta_edit_add_tabular_section(
    xml_text: &mut String,
    object_type: &str,
    object_name: &str,
    raw_value: &str,
) -> Result<(), String> {
    let section = meta_edit_parse_tabular_section(raw_value)?;
    meta_edit_ensure_top_child_name_free(xml_text, "TabularSection", &section.name)?;
    let mut lines = Vec::new();
    let mut next_uuid = fresh_meta_compile_uuid;
    emit_meta_tabular_section(
        &mut lines,
        "\t\t\t",
        &section,
        object_type,
        object_name,
        &mut next_uuid,
    );
    meta_edit_insert_top_child_object(xml_text, &lines)
}

pub(crate) fn meta_edit_parse_tabular_section(
    raw_value: &str,
) -> Result<MetaCompileTabularSection, String> {
    let value = raw_value.trim();
    if value.is_empty() {
        return Err("add-ts requires non-empty Value".to_string());
    }

    let Some((name, raw_columns)) = value.split_once(':') else {
        let section = MetaCompileTabularSection {
            name: value.to_string(),
            columns: Vec::new(),
        };
        validate_meta_compile_tabular_section_types(&section, "meta.edit add-ts")?;
        return Ok(section);
    };

    let name = name.trim();
    if name.is_empty() {
        return Err("add-ts requires non-empty tabular section name".to_string());
    }

    let columns = meta_edit_parse_tabular_section_columns(raw_columns)?;
    let section = MetaCompileTabularSection {
        name: name.to_string(),
        columns,
    };
    validate_meta_compile_tabular_section_types(&section, "meta.edit add-ts")?;
    Ok(section)
}

pub(crate) fn meta_edit_parse_tabular_section_columns(
    raw_columns: &str,
) -> Result<Vec<MetaCompileAttr>, String> {
    let mut column_defs = Vec::new();
    let mut current = String::new();

    for part in split_meta_edit_commas_outside_parens(raw_columns) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !current.is_empty() && meta_edit_looks_like_attr_definition(part) {
            column_defs.push(current);
            current = part.to_string();
        } else if current.is_empty() {
            current = part.to_string();
        } else {
            current.push_str(", ");
            current.push_str(part);
        }
    }
    if !current.is_empty() {
        column_defs.push(current);
    }

    column_defs
        .into_iter()
        .map(|column| {
            let attr = meta_compile_parse_attr(&Value::String(column.clone()));
            if attr.name.is_empty() || attr.type_name.is_empty() {
                return Err(format!(
                    "add-ts column requires Value like Name: Type, got: {column}"
                ));
            }
            validate_meta_compile_attr_type(&attr, "meta.edit add-ts column")?;
            Ok(attr)
        })
        .collect()
}

pub(crate) fn split_meta_edit_commas_outside_parens(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;

    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&value[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

pub(crate) fn meta_edit_looks_like_attr_definition(value: &str) -> bool {
    value
        .split_once(':')
        .map(|(name, _)| !name.trim().is_empty())
        .unwrap_or(false)
}

pub(crate) fn meta_edit_add_tabular_section_attribute(
    xml_text: &mut String,
    raw_value: &str,
) -> Result<(), String> {
    let (section_name, attr_text) = raw_value.trim().split_once('.').ok_or_else(|| {
        "add-ts-attribute requires Value like Section.Attribute: Type".to_string()
    })?;
    let section_name = section_name.trim();
    meta_edit_add_tabular_section_attribute_value(
        xml_text,
        section_name,
        &Value::String(attr_text.trim().to_string()),
    )
}

pub(crate) fn meta_edit_add_tabular_section_attribute_value(
    xml_text: &mut String,
    section_name: &str,
    value: &Value,
) -> Result<(), String> {
    let (value, position) = meta_edit_extract_insert_position(value)?;
    let attr = meta_compile_parse_attr(&value);
    if section_name.is_empty() || attr.name.is_empty() {
        return Err("add-ts-attribute requires Value like Section.Attribute: Type".to_string());
    }
    validate_meta_compile_attr_type(&attr, "meta.edit add-ts-attribute")?;
    meta_edit_ensure_tabular_child_name_free(xml_text, section_name, "Attribute", &attr.name)?;
    let mut lines = Vec::new();
    let mut next_uuid = fresh_meta_compile_uuid;
    emit_meta_attribute(&mut lines, "\t\t\t\t\t", &attr, "tabular", &mut next_uuid);
    meta_edit_insert_tabular_child_object_with_position(
        xml_text,
        section_name,
        "Attribute",
        &position,
        &lines,
    )
}

pub(crate) fn meta_edit_remove_tabular_section_attribute(
    xml_text: &mut String,
    raw_value: &str,
) -> Result<usize, String> {
    let mut removed = 0usize;
    for item in raw_value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (section_name, attr_name) = item.split_once('.').ok_or_else(|| {
            "remove-ts-attribute requires Value like Section.Attribute".to_string()
        })?;
        let section_name = section_name.trim();
        let attr_name = attr_name.trim();
        if section_name.is_empty() || attr_name.is_empty() {
            return Err("remove-ts-attribute requires Value like Section.Attribute".to_string());
        }
        meta_edit_remove_tabular_child_by_name(xml_text, section_name, "Attribute", attr_name)?;
        removed += 1;
    }

    if removed == 0 {
        return Err("remove-ts-attribute requires non-empty Value".to_string());
    }
    Ok(removed)
}

pub(crate) fn meta_edit_modify_attribute(
    xml_text: &mut String,
    raw_value: &str,
) -> Result<usize, String> {
    let mut modified = 0usize;
    for item in raw_value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (attr_name, raw_changes) = item.split_once(':').ok_or_else(|| {
            "modify-attribute requires Value like Attribute: key=value".to_string()
        })?;
        let attr_name = attr_name.trim();
        if attr_name.is_empty() || raw_changes.trim().is_empty() {
            return Err("modify-attribute requires Value like Attribute: key=value".to_string());
        }
        modified += meta_edit_modify_top_attribute_properties(xml_text, attr_name, raw_changes)?;
    }
    if modified == 0 {
        return Err("modify-attribute requires non-empty Value".to_string());
    }
    Ok(modified)
}

pub(crate) fn meta_edit_modify_tabular_section(
    xml_text: &mut String,
    raw_value: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
) -> Result<usize, String> {
    let mut modified = 0usize;
    for item in raw_value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (section_name, raw_changes) = item
            .split_once(':')
            .ok_or_else(|| "modify-ts requires Value like TabularSection: key=value".to_string())?;
        let section_name = section_name.trim();
        if section_name.is_empty() || raw_changes.trim().is_empty() {
            return Err("modify-ts requires Value like TabularSection: key=value".to_string());
        }
        modified += meta_edit_modify_tabular_section_properties(
            xml_text,
            section_name,
            raw_changes,
            line_number_length_policy,
        )?;
    }
    if modified == 0 {
        return Err("modify-ts requires non-empty Value".to_string());
    }
    Ok(modified)
}

pub(crate) fn meta_edit_modify_tabular_section_attribute(
    xml_text: &mut String,
    raw_value: &str,
) -> Result<usize, String> {
    let mut modified = 0usize;
    for item in raw_value
        .split(";;")
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (target, raw_changes) = item.split_once(':').ok_or_else(|| {
            "modify-ts-attribute requires Value like Section.Attribute: key=value".to_string()
        })?;
        let (section_name, attr_name) = target.trim().split_once('.').ok_or_else(|| {
            "modify-ts-attribute requires Value like Section.Attribute: key=value".to_string()
        })?;
        let section_name = section_name.trim();
        let attr_name = attr_name.trim();
        if section_name.is_empty() || attr_name.is_empty() || raw_changes.trim().is_empty() {
            return Err(
                "modify-ts-attribute requires Value like Section.Attribute: key=value".to_string(),
            );
        }
        modified += meta_edit_modify_tabular_attribute_properties(
            xml_text,
            section_name,
            attr_name,
            raw_changes,
        )?;
    }
    if modified == 0 {
        return Err("modify-ts-attribute requires non-empty Value".to_string());
    }
    Ok(modified)
}

pub(crate) fn meta_edit_modify_top_attribute_properties(
    xml_text: &mut String,
    attr_name: &str,
    raw_changes: &str,
) -> Result<usize, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let target_kind = MetaEditModifyTarget::Attribute {
        fill_value_allowed: !matches!(object.tag_name().name(), "DataProcessor" | "Report"),
    };
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| format!("Attribute '{attr_name}' not found"))?;
    let target = meta_info_children(child_objects, "Attribute")
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(attr_name))
        .ok_or_else(|| format!("Attribute '{attr_name}' not found"))?;
    if let Some(new_name) = meta_edit_requested_name(raw_changes, target_kind)? {
        meta_edit_ensure_sibling_name_free(
            child_objects,
            "Attribute",
            target.range(),
            &new_name,
            None,
        )?;
    }
    let props = meta_info_child(target, "Properties")
        .ok_or_else(|| format!("Attribute '{attr_name}' has no Properties"))?;
    let range = props.range();
    drop(doc);
    meta_edit_modify_properties_range(xml_text, range, raw_changes, target_kind)
}

pub(crate) fn meta_edit_modify_top_child_properties(
    xml_text: &mut String,
    tag: &str,
    child_name: &str,
    raw_changes: &str,
    target_kind: MetaEditModifyTarget,
) -> Result<usize, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| format!("{tag} '{child_name}' not found"))?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(child_name))
        .ok_or_else(|| format!("{tag} '{child_name}' not found"))?;
    if let Some(new_name) = meta_edit_requested_name(raw_changes, target_kind)? {
        meta_edit_ensure_sibling_name_free(child_objects, tag, target.range(), &new_name, None)?;
    }
    let props = meta_info_child(target, "Properties")
        .ok_or_else(|| format!("{tag} '{child_name}' has no Properties"))?;
    let range = props.range();
    drop(doc);
    meta_edit_modify_properties_range(xml_text, range, raw_changes, target_kind)
}

pub(crate) fn meta_edit_modify_tabular_section_properties(
    xml_text: &mut String,
    section_name: &str,
    raw_changes: &str,
    line_number_length_policy: MetaEditLineNumberLengthPolicy,
) -> Result<usize, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let section = meta_info_children(child_objects, "TabularSection")
        .into_iter()
        .find(|section| meta_edit_child_object_name(*section).as_deref() == Some(section_name))
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let target = MetaEditModifyTarget::TabularSection {
        line_number_length: line_number_length_policy,
    };
    if let Some(new_name) = meta_edit_requested_name(raw_changes, target)? {
        meta_edit_ensure_sibling_name_free(
            child_objects,
            "TabularSection",
            section.range(),
            &new_name,
            None,
        )?;
    }
    let props = meta_info_child(section, "Properties")
        .ok_or_else(|| format!("TabularSection '{section_name}' has no Properties"))?;
    let range = props.range();
    drop(doc);
    meta_edit_modify_properties_range(xml_text, range, raw_changes, target)
}

pub(crate) fn meta_edit_modify_tabular_attribute_properties(
    xml_text: &mut String,
    section_name: &str,
    attr_name: &str,
    raw_changes: &str,
) -> Result<usize, String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let target_kind = MetaEditModifyTarget::Attribute {
        fill_value_allowed: matches!(object.tag_name().name(), "DataProcessor" | "Report"),
    };
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let child_objects = meta_info_child(section, "ChildObjects")
        .ok_or_else(|| format!("Attribute '{section_name}.{attr_name}' not found"))?;
    let target = meta_info_children(child_objects, "Attribute")
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(attr_name))
        .ok_or_else(|| format!("Attribute '{section_name}.{attr_name}' not found"))?;
    if let Some(new_name) = meta_edit_requested_name(raw_changes, target_kind)? {
        meta_edit_ensure_sibling_name_free(
            child_objects,
            "Attribute",
            target.range(),
            &new_name,
            Some(section_name),
        )?;
    }
    let props = meta_info_child(target, "Properties")
        .ok_or_else(|| format!("Attribute '{section_name}.{attr_name}' has no Properties"))?;
    let range = props.range();
    drop(doc);
    meta_edit_modify_properties_range(xml_text, range, raw_changes, target_kind)
}

#[derive(Clone, Copy)]
pub(crate) enum MetaEditModifyTarget {
    Attribute {
        fill_value_allowed: bool,
    },
    RegisterField,
    EnumValue,
    Column,
    TabularSection {
        line_number_length: MetaEditLineNumberLengthPolicy,
    },
}

pub(crate) fn meta_edit_modify_properties_range(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    raw_changes: &str,
    target: MetaEditModifyTarget,
) -> Result<usize, String> {
    let mut properties = xml_text[range.clone()].to_string();
    let child_indent = meta_edit_property_child_indent(&properties);
    let mut modified = 0usize;

    for change in split_meta_edit_commas_outside_parens(raw_changes) {
        let change = change.trim();
        if change.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = change
            .split_once('=')
            .ok_or_else(|| format!("modify attribute change requires key=value, got: {change}"))?;
        let key = raw_key.trim();
        let value = raw_value.trim();
        let canonical = meta_edit_canonical_attribute_property(key, target)?;
        match canonical.as_str() {
            "Name" => {
                validate_meta_compile_name("meta.edit rename", value)?;
                let replacement = format!("{child_indent}<Name>{}</Name>", escape_xml(value));
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "Name",
                    &replacement,
                    &child_indent,
                )?;
            }
            "Synonym" => {
                let mut lines = Vec::new();
                emit_meta_mltext(&mut lines, &child_indent, "Synonym", value);
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "Synonym",
                    &lines.join("\n"),
                    &child_indent,
                )?;
            }
            "Comment" => {
                let replacement = if value.is_empty() {
                    format!("{child_indent}<Comment/>")
                } else {
                    format!("{child_indent}<Comment>{}</Comment>", escape_xml(value))
                };
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "Comment",
                    &replacement,
                    &child_indent,
                )?;
            }
            "Type" => {
                validate_meta_type_union(std::iter::once(value)).map_err(|error| {
                    format!("invalid 8.3.27 type for meta.edit modify: {error}")
                })?;
                let mut lines = Vec::new();
                emit_meta_value_type(&mut lines, &child_indent, value);
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "Type",
                    &lines.join("\n"),
                    &child_indent,
                )?;
                if meta_edit_property_exists(&properties, "FillValue")? {
                    let mut fill_lines = Vec::new();
                    emit_meta_fill_value(&mut fill_lines, &child_indent, value);
                    meta_edit_replace_or_insert_property(
                        &mut properties,
                        "FillValue",
                        &fill_lines.join("\n"),
                        &child_indent,
                    )?;
                }
            }
            "FillValue" => {
                if !meta_edit_property_exists(&properties, "FillValue")? {
                    return Err(
                        "Property 'FillValue' is not available for this attribute".to_string()
                    );
                }
                let replacement = meta_edit_fill_value_xml(&child_indent, value);
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "FillValue",
                    &replacement,
                    &child_indent,
                )?;
            }
            "v8:AllowedSign" => {
                meta_edit_replace_or_insert_nested_v8_property(
                    &mut properties,
                    "NumberQualifiers",
                    "AllowedSign",
                    value,
                    &child_indent,
                )?;
            }
            "LineNumberLength" => {
                if !meta_edit_property_exists(&properties, "LineNumberLength")? {
                    return Err(
                        "Property 'LineNumberLength' is not available in this tabular section"
                            .to_string(),
                    );
                }
                let value = meta_edit_line_number_length_value(value)?;
                let replacement =
                    format!("{child_indent}<LineNumberLength>{value}</LineNumberLength>");
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    "LineNumberLength",
                    &replacement,
                    &child_indent,
                )?;
            }
            _ => {
                let replacement = format!(
                    "{child_indent}<{canonical}>{}</{canonical}>",
                    escape_xml(value)
                );
                meta_edit_replace_or_insert_property(
                    &mut properties,
                    &canonical,
                    &replacement,
                    &child_indent,
                )?;
            }
        }
        modified += 1;
    }

    xml_text.replace_range(range, &properties);
    Ok(modified)
}

pub(crate) fn meta_edit_canonical_attribute_property(
    key: &str,
    target: MetaEditModifyTarget,
) -> Result<String, String> {
    let trimmed = key.trim();
    let normalized = trimmed.to_ascii_lowercase();
    if meta_edit_is_line_number_length_key(trimmed) {
        return match target {
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::Editable,
            } => Ok("LineNumberLength".to_string()),
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::FixedFive,
            } => Err(
                "LineNumberLength is fixed at 5 when CompatibilityMode is Version8_3_26 or earlier"
                    .to_string(),
            ),
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::NotApplicable,
            } => Err(
                "LineNumberLength is not applicable to Report, DataProcessor, ExternalReport, or ExternalDataProcessor tabular sections".to_string(),
            ),
            MetaEditModifyTarget::TabularSection {
                line_number_length: MetaEditLineNumberLengthPolicy::UnknownCompatibility,
            } => Err(
                "LineNumberLength cannot be changed because CompatibilityMode cannot be determined"
                    .to_string(),
            ),
            _ => Err(format!("Unsupported modify property key '{trimmed}'")),
        };
    }
    let canonical = match normalized.as_str() {
        "name" | "имя" => Ok("Name".to_string()),
        "synonym" | "синоним" => Ok("Synonym".to_string()),
        "comment" | "комментарий" => Ok("Comment".to_string()),
        "fillchecking" | "fill_checking" | "fill-checking"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. }
                    | MetaEditModifyTarget::RegisterField
                    | MetaEditModifyTarget::TabularSection { .. }
            ) =>
        {
            Ok("FillChecking".to_string())
        }
        "use" | "использование"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. }
                    | MetaEditModifyTarget::RegisterField
                    | MetaEditModifyTarget::TabularSection { .. }
            ) =>
        {
            Ok("Use".to_string())
        }
        "type" | "тип"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. } | MetaEditModifyTarget::RegisterField
            ) =>
        {
            Ok("Type".to_string())
        }
        "fillvalue" | "fill_value" | "fill-value" | "значениезаполнения"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute {
                    fill_value_allowed: true
                }
            ) =>
        {
            Ok("FillValue".to_string())
        }
        "indexing" | "индексирование"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. }
                    | MetaEditModifyTarget::RegisterField
                    | MetaEditModifyTarget::Column
            ) =>
        {
            Ok("Indexing".to_string())
        }
        "allowedsign" | "allowed_sign" | "allowed-sign" | "v8:allowedsign"
            if matches!(
                target,
                MetaEditModifyTarget::Attribute { .. } | MetaEditModifyTarget::RegisterField
            ) =>
        {
            Ok("v8:AllowedSign".to_string())
        }
        _ => Err(format!("Unsupported modify property key '{trimmed}'")),
    }?;
    Ok(canonical)
}

pub(crate) fn meta_edit_line_number_length_value(raw_value: &str) -> Result<String, String> {
    let parsed = raw_value.parse::<u8>().map_err(|_| {
        format!(
            "LineNumberLength must be an integer in 5..=9, got '{}'",
            raw_value
        )
    })?;
    if !(5..=9).contains(&parsed) {
        return Err(format!(
            "LineNumberLength must be an integer in 5..=9, got '{raw_value}'"
        ));
    }
    Ok(parsed.to_string())
}

fn meta_edit_fill_value_xml(indent: &str, raw_value: &str) -> String {
    let value = raw_value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("nil") {
        return format!("{indent}<FillValue xsi:nil=\"true\"/>");
    }

    let value_type = if meta_edit_is_design_time_ref(value) {
        "xr:DesignTimeRef"
    } else if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
        "xs:boolean"
    } else if meta_edit_is_decimal_literal(value) {
        "xs:decimal"
    } else if meta_edit_is_date_time_literal(value) {
        "xs:dateTime"
    } else {
        "xs:string"
    };
    let normalized_value = if value_type == "xs:boolean" {
        value.to_ascii_lowercase()
    } else {
        value.to_string()
    };
    format!(
        "{indent}<FillValue xsi:type=\"{value_type}\">{}</FillValue>",
        escape_xml(&normalized_value)
    )
}

fn meta_edit_is_design_time_ref(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    matches!(parts.as_slice(), ["Enum", name, "EnumValue", item] if !name.is_empty() && !item.is_empty())
        || matches!(
            parts.as_slice(),
            [kind, name, "EmptyRef"]
                if !name.is_empty()
                    && matches!(
                        *kind,
                        "Catalog"
                            | "Document"
                            | "ExchangePlan"
                            | "ChartOfAccounts"
                            | "ChartOfCharacteristicTypes"
                            | "ChartOfCalculationTypes"
                            | "BusinessProcess"
                            | "Task"
                    )
        )
}

fn meta_edit_is_decimal_literal(value: &str) -> bool {
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let mut parts = unsigned.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    let has_digit = !integer.is_empty() || fraction.is_some_and(|part| !part.is_empty());
    has_digit
        && integer.chars().all(|ch| ch.is_ascii_digit())
        && fraction.is_none_or(|part| part.chars().all(|ch| ch.is_ascii_digit()))
        && parts.next().is_none()
}

fn meta_edit_is_date_time_literal(value: &str) -> bool {
    if value.len() != 19 || !value.is_ascii() {
        return false;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| !matches!(index, 4 | 7 | 10 | 13 | 16) && !byte.is_ascii_digit())
    {
        return false;
    }

    let parse = |start: usize, end: usize| value[start..end].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        parse(0, 4),
        parse(5, 7),
        parse(8, 10),
        parse(11, 13),
        parse(14, 16),
        parse(17, 19),
    ) else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=max_day).contains(&day)
}

pub(crate) fn meta_edit_requested_name(
    raw_changes: &str,
    target: MetaEditModifyTarget,
) -> Result<Option<String>, String> {
    for change in split_meta_edit_commas_outside_parens(raw_changes) {
        let change = change.trim();
        if change.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = change
            .split_once('=')
            .ok_or_else(|| format!("modify attribute change requires key=value, got: {change}"))?;
        if meta_edit_canonical_attribute_property(raw_key, target)?.as_str() == "Name" {
            let name = raw_value.trim();
            if name.is_empty() {
                return Err("modify name requires non-empty value".to_string());
            }
            validate_meta_compile_name("meta.edit rename", name)?;
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

pub(crate) fn meta_edit_ensure_sibling_name_free(
    child_objects: roxmltree::Node<'_, '_>,
    tag: &str,
    current_range: std::ops::Range<usize>,
    new_name: &str,
    parent_name: Option<&str>,
) -> Result<(), String> {
    for child in meta_info_children(child_objects, tag) {
        if child.range() == current_range {
            continue;
        }
        if meta_edit_child_object_name(child).as_deref() == Some(new_name) {
            return Err(match parent_name {
                Some(parent_name) => format!("{tag} '{parent_name}.{new_name}' already exists"),
                None => format!("{tag} '{new_name}' already exists"),
            });
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_property_child_indent(properties: &str) -> String {
    for tag in ["Name", "Synonym", "Comment", "Type"] {
        let needle = format!("<{tag}");
        if let Some(pos) = properties.find(&needle) {
            let line_start = properties[..pos]
                .rfind('\n')
                .map(|idx| idx + 1)
                .unwrap_or(0);
            let indent = &properties[line_start..pos];
            if indent.chars().all(|ch| ch == '\t' || ch == ' ') {
                return indent.to_string();
            }
        }
    }
    "\t\t\t\t\t".to_string()
}

pub(crate) fn meta_edit_replace_or_insert_property(
    properties: &mut String,
    tag: &str,
    replacement: &str,
    child_indent: &str,
) -> Result<(), String> {
    if let Some(range) = meta_edit_xml_element_range(properties, tag)? {
        properties.replace_range(range, replacement.trim_start());
        return Ok(());
    }

    let Some(close_pos) = properties.rfind("</Properties>") else {
        return Err("No closing </Properties> found".to_string());
    };
    properties.insert_str(close_pos, &format!("{replacement}\n{child_indent}"));
    Ok(())
}

pub(crate) fn meta_edit_property_exists(properties: &str, tag: &str) -> Result<bool, String> {
    meta_edit_xml_element_range(properties, tag).map(|range| range.is_some())
}

pub(crate) fn meta_edit_xml_element_range(
    text: &str,
    tag: &str,
) -> Result<Option<std::ops::Range<usize>>, String> {
    let needle = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut search_start = 0usize;

    while let Some(relative_start) = text[search_start..].find(&needle) {
        let start = search_start + relative_start;
        let after_tag = text[start + needle.len()..].chars().next();
        if after_tag.is_some_and(|ch| ch != '>' && ch != '/' && !ch.is_whitespace()) {
            search_start = start + needle.len();
            continue;
        }
        let Some(relative_open_end) = text[start..].find('>') else {
            return Err(format!("No closing > found for <{tag}>"));
        };
        let open_end = start + relative_open_end;
        let opening = &text[start..=open_end];
        if opening.trim_end().ends_with("/>") {
            return Ok(Some(start..open_end + 1));
        }
        let content_start = open_end + 1;
        let Some(relative_end) = text[content_start..].find(&close) else {
            return Err(format!("No closing </{tag}> found"));
        };
        let end = content_start + relative_end + close.len();
        return Ok(Some(start..end));
    }

    Ok(None)
}

pub(crate) fn meta_edit_replace_or_insert_nested_v8_property(
    properties: &mut String,
    parent_tag: &str,
    child_tag: &str,
    value: &str,
    child_indent: &str,
) -> Result<(), String> {
    let parent_open = format!("<v8:{parent_tag}>");
    let parent_close = format!("</v8:{parent_tag}>");
    let Some(parent_start) = properties.find(&parent_open) else {
        return Err(format!("No <v8:{parent_tag}> found"));
    };
    let parent_content_start = parent_start + parent_open.len();
    let Some(relative_parent_end) = properties[parent_content_start..].find(&parent_close) else {
        return Err(format!("No </v8:{parent_tag}> found"));
    };
    let parent_end = parent_content_start + relative_parent_end;
    let parent_range = parent_start..parent_end + parent_close.len();
    let mut parent = properties[parent_range.clone()].to_string();
    let nested_indent = format!("{child_indent}\t\t");
    let replacement = format!(
        "{nested_indent}<v8:{child_tag}>{}</v8:{child_tag}>",
        escape_xml(value)
    );

    let self_closing = format!("<v8:{child_tag}/>");
    if let Some(pos) = parent.find(&self_closing) {
        parent.replace_range(pos..pos + self_closing.len(), replacement.trim_start());
    } else {
        let open = format!("<v8:{child_tag}>");
        let close = format!("</v8:{child_tag}>");
        if let Some(start) = parent.find(&open) {
            let Some(relative_end) = parent[start + open.len()..].find(&close) else {
                return Err(format!("No </v8:{child_tag}> found"));
            };
            let end = start + open.len() + relative_end + close.len();
            parent.replace_range(start..end, replacement.trim_start());
        } else {
            let Some(close_pos) = parent.rfind(&parent_close) else {
                return Err(format!("No </v8:{parent_tag}> found"));
            };
            parent.insert_str(close_pos, &format!("{replacement}\n{child_indent}\t"));
        }
    }
    properties.replace_range(parent_range, &parent);
    Ok(())
}

pub(crate) fn meta_edit_remove_tabular_child_by_name(
    xml_text: &mut String,
    section_name: &str,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let child_objects = meta_info_child(section, "ChildObjects")
        .ok_or_else(|| format!("{tag} '{section_name}.{name}' not found"))?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(name))
        .ok_or_else(|| format!("{tag} '{section_name}.{name}' not found"))?;
    let range = target.range();
    drop(doc);
    meta_edit_remove_xml_node_range(xml_text, range);
    Ok(())
}

pub(crate) fn meta_edit_remove_top_child_by_name(
    xml_text: &mut String,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| format!("{tag} '{name}' not found"))?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(name))
        .ok_or_else(|| format!("{tag} '{name}' not found"))?;
    let range = target.range();
    drop(doc);
    meta_edit_remove_xml_node_range(xml_text, range);
    Ok(())
}

pub(crate) fn meta_edit_attribute_context(object_type: &str) -> &str {
    match object_type {
        "Catalog" => "catalog",
        "DataProcessor" | "Report" => "processor",
        "InformationRegister"
        | "AccumulationRegister"
        | "AccountingRegister"
        | "CalculationRegister" => "register-other",
        _ => "object",
    }
}

pub(crate) fn meta_edit_object_node<'a, 'input>(
    doc: &'a Document<'input>,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(format!(
            "Root element must be MetaDataObject, got: {}",
            root.tag_name().name()
        ));
    }
    root.children()
        .find(|node| node.is_element())
        .ok_or_else(|| "No object element found under MetaDataObject".to_string())
}

pub(crate) fn meta_edit_child_object_name(node: roxmltree::Node<'_, '_>) -> Option<String> {
    meta_info_child(node, "Properties").and_then(|props| meta_info_child_text(props, "Name"))
}

pub(crate) fn meta_edit_ensure_top_child_name_free(
    xml_text: &str,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    if let Some(child_objects) = meta_info_child(object, "ChildObjects") {
        for child in meta_info_children(child_objects, tag) {
            if meta_edit_child_object_name(child).as_deref() == Some(name) {
                return Err(format!("{tag} '{name}' already exists"));
            }
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_find_tabular_section<'a, 'input>(
    object: roxmltree::Node<'a, 'input>,
    section_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    let child_objects = meta_info_child(object, "ChildObjects")?;
    meta_info_children(child_objects, "TabularSection")
        .into_iter()
        .find(|section| meta_edit_child_object_name(*section).as_deref() == Some(section_name))
}

pub(crate) fn meta_edit_ensure_tabular_child_name_free(
    xml_text: &str,
    section_name: &str,
    tag: &str,
    name: &str,
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    if let Some(child_objects) = meta_info_child(section, "ChildObjects") {
        for child in meta_info_children(child_objects, tag) {
            if meta_edit_child_object_name(child).as_deref() == Some(name) {
                return Err(format!("{tag} '{section_name}.{name}' already exists"));
            }
        }
    }
    Ok(())
}

pub(crate) fn meta_edit_insert_top_child_object(
    xml_text: &mut String,
    lines: &[String],
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    if let Some(child_objects) = meta_info_child(object, "ChildObjects") {
        let range = child_objects.range();
        drop(doc);
        return meta_edit_insert_lines_into_child_objects(xml_text, range, "\t\t", lines);
    }
    let range = object.range();
    let tag = object.tag_name().name().to_string();
    drop(doc);
    meta_edit_insert_child_objects_into_node(xml_text, range, &tag, "\t\t", lines)
}

pub(crate) fn meta_edit_insert_top_child_object_with_position(
    xml_text: &mut String,
    tag: &str,
    position: &MetaEditInsertPosition,
    lines: &[String],
) -> Result<(), String> {
    if position.is_empty() {
        return meta_edit_insert_top_child_object(xml_text, lines);
    }
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let child_objects = meta_info_child(object, "ChildObjects")
        .ok_or_else(|| "ChildObjects not found for positional insert".to_string())?;
    let (target_name, after) = position
        .target()
        .ok_or_else(|| "Position target must be non-empty".to_string())?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(target_name))
        .ok_or_else(|| format!("{tag} '{target_name}' not found for positional insert"))?;
    let range = target.range();
    drop(doc);
    meta_edit_insert_lines_near_node(xml_text, range, after, lines);
    Ok(())
}

pub(crate) fn meta_edit_insert_tabular_child_object(
    xml_text: &mut String,
    section_name: &str,
    lines: &[String],
) -> Result<(), String> {
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let range = section.range();
    drop(doc);
    meta_edit_insert_lines_into_node_child_objects(
        xml_text,
        range,
        "TabularSection",
        "\t\t\t\t",
        lines,
    )
}

pub(crate) fn meta_edit_insert_tabular_child_object_with_position(
    xml_text: &mut String,
    section_name: &str,
    tag: &str,
    position: &MetaEditInsertPosition,
    lines: &[String],
) -> Result<(), String> {
    if position.is_empty() {
        return meta_edit_insert_tabular_child_object(xml_text, section_name, lines);
    }
    let doc = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("XML parse error: {err}"))?;
    let object = meta_edit_object_node(&doc)?;
    let section = meta_edit_find_tabular_section(object, section_name)
        .ok_or_else(|| format!("TabularSection '{section_name}' not found"))?;
    let child_objects = meta_info_child(section, "ChildObjects")
        .ok_or_else(|| format!("TabularSection '{section_name}' has no ChildObjects"))?;
    let (target_name, after) = position
        .target()
        .ok_or_else(|| "Position target must be non-empty".to_string())?;
    let target = meta_info_children(child_objects, tag)
        .into_iter()
        .find(|child| meta_edit_child_object_name(*child).as_deref() == Some(target_name))
        .ok_or_else(|| {
            format!("{tag} '{section_name}.{target_name}' not found for positional insert")
        })?;
    let range = target.range();
    drop(doc);
    meta_edit_insert_lines_near_node(xml_text, range, after, lines);
    Ok(())
}

pub(crate) fn meta_edit_insert_lines_into_child_objects(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    close_indent: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let section_text = &xml_text[range.clone()];
    if section_text.trim_end().ends_with("/>") {
        xml_text.replace_range(
            range,
            &format!("<ChildObjects>\n{content}\n{close_indent}</ChildObjects>"),
        );
        return Ok(());
    }
    let Some(relative_pos) = section_text.rfind("</ChildObjects>") else {
        if section_text.trim_end().ends_with('>') {
            xml_text.insert_str(range.end, &format!("\n{content}\n{close_indent}"));
            return Ok(());
        }
        return Err("No closing </ChildObjects> found".to_string());
    };
    let close_pos = range.start + relative_pos;
    let line_start = xml_text[..close_pos]
        .rfind('\n')
        .map_or(close_pos, |index| index + 1);
    let insert_at_closing_indent = xml_text[line_start..close_pos]
        .chars()
        .all(|ch| ch == '\t' || ch == ' ');
    if insert_at_closing_indent {
        let insert_pos = meta_edit_mark_lxml_append_tail(xml_text, line_start);
        xml_text.insert_str(insert_pos, &format!("{content}\n"));
    } else {
        xml_text.insert_str(close_pos, &format!("{content}\n{close_indent}"));
    }
    Ok(())
}

pub(crate) fn meta_edit_mark_lxml_append_tail(xml_text: &mut String, insert_pos: usize) -> usize {
    if insert_pos == 0 || xml_text[..insert_pos].ends_with("&#13;\n") {
        return insert_pos;
    }
    if insert_pos >= 2 && &xml_text[insert_pos - 2..insert_pos] == "\r\n" {
        xml_text.replace_range(insert_pos - 2..insert_pos, "&#13;\n");
        return insert_pos + 4;
    }
    if insert_pos >= 1 && &xml_text[insert_pos - 1..insert_pos] == "\n" {
        xml_text.replace_range(insert_pos - 1..insert_pos, "&#13;\n");
        return insert_pos + 5;
    }
    insert_pos
}

pub(crate) fn meta_edit_insert_lines_near_node(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    after: bool,
    lines: &[String],
) {
    let content = lines.join("\n");
    if after {
        if let Some(relative_newline) = xml_text[range.end..].find('\n') {
            let insert_pos = range.end + relative_newline + 1;
            xml_text.insert_str(insert_pos, &format!("{content}&#13;\n"));
        } else {
            xml_text.insert_str(range.end, &format!("\n{content}"));
        }
        return;
    }

    let line_start = xml_text[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let insert_pos = if xml_text[line_start..range.start]
        .chars()
        .all(|ch| ch == '\t' || ch == ' ')
    {
        line_start
    } else {
        range.start
    };
    xml_text.insert_str(insert_pos, &format!("{content}\n"));
}

pub(crate) fn meta_edit_insert_child_objects_into_node(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    tag: &str,
    close_indent: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let node_text = &xml_text[range.clone()];
    if let Some(relative_pos) = node_text.rfind("/>") {
        let pos = range.start + relative_pos;
        xml_text.replace_range(
            pos..pos + 2,
            &format!(">\n{close_indent}<ChildObjects>\n{content}\n{close_indent}</ChildObjects>\n\t</{tag}>"),
        );
        return Ok(());
    }
    let close = format!("</{tag}>");
    let Some(relative_pos) = node_text.rfind(&close) else {
        return Err(format!("No closing </{tag}> found"));
    };
    xml_text.insert_str(
        range.start + relative_pos,
        &format!("{close_indent}<ChildObjects>\n{content}\n{close_indent}</ChildObjects>\n"),
    );
    Ok(())
}

pub(crate) fn meta_edit_insert_lines_into_node_child_objects(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
    tag: &str,
    close_indent: &str,
    lines: &[String],
) -> Result<(), String> {
    let content = lines.join("\n");
    let node_text = &xml_text[range.clone()];
    if let Some(relative_pos) = node_text.find("<ChildObjects/>") {
        let pos = range.start + relative_pos;
        xml_text.replace_range(
            pos..pos + "<ChildObjects/>".len(),
            &format!("<ChildObjects>\n{content}\n{close_indent}</ChildObjects>"),
        );
        return Ok(());
    }
    if let Some(relative_pos) = node_text.find("<ChildObjects>") {
        let pos = range.start + relative_pos + "<ChildObjects>".len();
        xml_text.insert_str(pos, &format!("\n{content}"));
        return Ok(());
    }
    meta_edit_insert_child_objects_into_node(xml_text, range, tag, close_indent, lines)
}

pub(crate) fn meta_edit_remove_xml_node_range(
    xml_text: &mut String,
    range: std::ops::Range<usize>,
) {
    let mut start = range.start;
    let mut end = range.end;

    if let Some(line_start) = xml_text[..start].rfind('\n') {
        let prefix = &xml_text[line_start + 1..start];
        if prefix.trim().is_empty() {
            start = line_start + 1;
        }
    }

    if end < xml_text.len() {
        if let Some(line_end) = xml_text[end..].find('\n') {
            let suffix_end = end + line_end;
            let suffix = &xml_text[end..suffix_end];
            if suffix.trim().is_empty() {
                end = suffix_end + 1;
            }
        }
    }

    xml_text.replace_range(start..end, "");
}

pub(crate) fn normalize_meta_edit_property_value(key: &str, value: &str) -> String {
    match key {
        "HierarchyType" => normalize_meta_enum_value(value),
        "DefaultPresentation" => normalize_meta_enum_value(value),
        "DataLockControlMode" => normalize_meta_enum_value(value),
        "FullTextSearch" => normalize_meta_enum_value(value),
        "Posting" => normalize_meta_enum_value(value),
        "EditType" => normalize_meta_enum_value(value),
        _ => value.to_string(),
    }
}

fn normalize_meta_edit_scalar_property_value(object_type: &str, key: &str, value: &str) -> String {
    let normalized = normalize_meta_edit_property_value(key, value);
    if meta_8_3_27_boolean_properties(object_type).contains(&key) {
        if normalized.eq_ignore_ascii_case("true") {
            return "true".to_string();
        }
        if normalized.eq_ignore_ascii_case("false") {
            return "false".to_string();
        }
    }
    normalized
}

pub(crate) fn invoke_read(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<Result<AdapterOutcome, String>> {
    match operation {
        "meta-info" => Some(Ok(analyze_meta_info(args, context))),
        "meta-validate" => Some(Ok(validate_meta(args, context))),
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
        "meta-compile" => Some(compile_meta(args, context)),
        "meta-edit" => Some(edit_meta(args, context)),
        "meta-remove" => Some(remove_metadata_object(args, context)),
        _ => None,
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
        let expected =
            include_str!("../../../../../tests/fixtures/platform_8_3_27/exchange_plan/Content.xml");

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

#![allow(dead_code, unused_imports)]

use super::super::cf::create_configuration_scaffold;
use super::super::common::utf8_bom_bytes;
use super::super::compile_transaction::{with_commit_failpoint, CommitFailpoint};
use super::super::compile_transaction::{CompileTransaction, RegistrationStatus};
use super::super::single_file_publisher::with_before_commit_hook;
use super::super::subsystem::compile_subsystem;
use super::remove::{
    metadata_files_recursive_bounded, metadata_files_recursive_with_limits,
    plan_meta_remove_subsystem_replacements, plan_meta_remove_subsystem_replacements_bounded,
    remove_metadata_object_with_data, MetaRemoveTraversalLimits,
};
use super::{
    force_meta_remove_reparse_path, with_before_meta_remove_subsystem_child_inspection_hook,
    with_meta_remove_forced_reparse_paths,
};
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn remove_metadata_object(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> crate::application::AdapterOutcome {
    remove_metadata_object_with_data(args, context).outcome
}

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
        error.contains("metadata payload directory must not be a symbolic link or reparse point"),
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

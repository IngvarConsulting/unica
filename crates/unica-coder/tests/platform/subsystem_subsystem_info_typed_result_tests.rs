use super::*;
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_workspace::read::{with_secure_tree_test_hook, SecureTreePhase};
use unica_workspace::test_support::{create_dir_symlink_for_test, create_file_symlink_for_test};

fn workspace(name: &str, with_command_interface: bool) -> (WorkspaceContext, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "unica-subsystem-info-typed-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("src/Subsystems/Продажи/Ext")).unwrap();
    fs::write(
            root.join("src/Subsystems/Продажи.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Продажи</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><xr:Item>Catalog.Goods</xr:Item></Content></Properties></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    if with_command_interface {
        fs::write(
                root.join("src/Subsystems/Продажи/Ext/CommandInterface.xml"),
                r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><CommandsVisibility><Command name="Catalog.Goods.Create"><Visibility xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><xr:Common>false</xr:Common></Visibility></Command></CommandsVisibility></CommandInterface>"#,
            )
            .unwrap();
    }
    let physical_root = root.canonicalize().unwrap();
    let context = WorkspaceContext {
        cwd: physical_root.clone(),
        workspace_root: physical_root.clone(),
        cache_root: physical_root.join(".build/unica"),
        workspace_epoch: 1,
    };
    (context, root)
}

fn info_args() -> Map<String, Value> {
    Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Продажи.xml"),
    )])
}

fn write_configuration(root: &Path, registrations: &[&str]) {
    let registrations = registrations
        .iter()
        .map(|name| format!("<Subsystem>{name}</Subsystem>"))
        .collect::<String>();
    fs::write(
            root.join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects>{registrations}</ChildObjects></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
}

fn create_directory_symlink_or_skip(target: &Path, link: &Path) -> bool {
    match create_dir_symlink_for_test(target, link) {
        Some(Ok(())) => true,
        Some(Err(error)) if error.raw_os_error() == Some(1314) => false,
        Some(Err(error)) => panic!("failed to create directory symlink fixture: {error}"),
        None => false,
    }
}

fn create_file_symlink_or_skip(target: &Path, link: &Path) -> bool {
    match create_file_symlink_for_test(target, link) {
        Some(Ok(())) => true,
        Some(Err(error)) if error.raw_os_error() == Some(1314) => false,
        Some(Err(error)) => panic!("failed to create file symlink fixture: {error}"),
        None => false,
    }
}

/// `Mode=tree` answered the hierarchy question; typing must not drop the
/// capability, only change how it is selected.
#[test]
fn pointing_at_the_subsystems_folder_answers_only_with_tree() {
    let (context, root) = workspace("tree", false);
    fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>СтандартныеПодсистемы</Subsystem><Subsystem>Служебные</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/Subsystems/СтандартныеПодсистемы.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>СтандартныеПодсистемы</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Обсуждения</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::create_dir_all(root.join("src/Subsystems/СтандартныеПодсистемы/Subsystems")).unwrap();
    fs::write(
            root.join("src/Subsystems/СтандартныеПодсистемы/Subsystems/Обсуждения.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Subsystem><Properties><Name>Обсуждения</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content><xr:Item xsi:type="xr:MDObjectRef">CommonForm.Обсуждение</xr:Item></Content></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/Subsystems/Служебные.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Служебные</Name><IncludeInCommandInterface>false</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>ФоновыеЗадания</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::create_dir_all(root.join("src/Subsystems/Служебные/Subsystems")).unwrap();
    fs::write(
            root.join("src/Subsystems/Служебные/Subsystems/ФоновыеЗадания.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>ФоновыеЗадания</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems"))]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    assert_eq!(
        serde_json::to_value(execution.data.expect("the folder answers with a tree")).unwrap(),
        serde_json::json!({
            "tree": [
                {
                    "name": "СтандартныеПодсистемы",
                    "content": 0,
                    "children": [
                        {"name": "Обсуждения", "content": 1, "children": []}
                    ]
                },
                {
                    "name": "Служебные",
                    "content": 0,
                    "children": [
                        {"name": "ФоновыеЗадания", "content": 0, "children": []}
                    ]
                }
            ]
        })
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn nested_subsystems_folder_is_not_a_tree_target() {
    let (context, root) = workspace("nested-tree", false);
    fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>СтандартныеПодсистемы</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/Subsystems/СтандартныеПодсистемы.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>СтандартныеПодсистемы</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Обсуждения</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::create_dir_all(root.join("src/Subsystems/СтандартныеПодсистемы/Subsystems")).unwrap();
    fs::write(
            root.join("src/Subsystems/СтандартныеПодсистемы/Subsystems/Обсуждения.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Обсуждения</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/СтандартныеПодсистемы/Subsystems"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    assert!(
        execution
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("root Subsystems directory")),
        "{:?}",
        execution.outcome
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn nested_configuration_cannot_lend_registered_topology_to_a_local_xml() {
    let (context, root) = workspace("nested-configuration-decoy", false);
    let source_root = root.join("src");
    write_configuration(&source_root, &["Parent"]);
    fs::write(
        source_root.join("Subsystems/Parent.xml"),
        child_subsystem_stub_xml(
            "Parent",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let nested_root = source_root.join("Subsystems/Parent");
    fs::create_dir_all(nested_root.join("Subsystems")).unwrap();
    write_configuration(&nested_root, &["Child"]);
    fs::write(
        nested_root.join("Subsystems/Child.xml"),
        child_subsystem_stub_xml(
            "Child",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Parent/Subsystems/Child.xml"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    let SubsystemInfoAnswer::Subsystem(data) = execution
        .data
        .expect("the unregistered XML remains local data")
    else {
        panic!("a concrete XML must answer with subsystem data");
    };
    assert!(
        data.tree.is_none(),
        "a nested Configuration.xml must not redefine the registered source root"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unconfigured_configuration_source_cannot_lend_registered_topology() {
    let (context, root) = workspace("unconfigured-configuration-decoy", false);
    fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: configured\n",
        )
        .unwrap();
    let configured = root.join("configured");
    fs::create_dir_all(configured.join("Subsystems")).unwrap();
    write_configuration(&configured, &[]);

    let decoy = root.join("src");
    write_configuration(&decoy, &["Продажи"]);
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Продажи.xml"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    let SubsystemInfoAnswer::Subsystem(data) =
        execution.data.expect("the exact XML remains local data")
    else {
        panic!("a concrete XML must answer with subsystem data");
    };
    assert!(
        data.tree.is_none(),
        "an undeclared source root must not lend registered topology"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn single_subsystem_directory_alias_is_not_an_info_target() {
    let (context, root) = workspace("single-directory-alias", false);
    let source_root = root.join("src");
    write_configuration(&source_root, &["Parent"]);
    fs::write(
        source_root.join("Subsystems/Parent.xml"),
        child_subsystem_stub_xml(
            "Parent",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    fs::create_dir_all(source_root.join("Subsystems/Parent")).unwrap();
    let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems/Parent"))]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    assert!(
        execution
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("XML file or the root Subsystems directory")),
        "{:?}",
        execution.outcome
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn concrete_subsystem_contains_its_root_chain_and_complete_descendant_tree() {
    let (context, root) = workspace("focused-tree", false);
    fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>Корень</Subsystem><Subsystem>ДругойКорень</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/Subsystems/Корень.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Корень</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Родитель</Subsystem><Subsystem>Соседняя</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/Subsystems/ДругойКорень.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>ДругойКорень</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::create_dir_all(root.join("src/Subsystems/Корень/Subsystems/Родитель/Subsystems")).unwrap();
    fs::write(
            root.join("src/Subsystems/Корень/Subsystems/Родитель.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Родитель</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Выбранная</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/Subsystems/Корень/Subsystems/Соседняя.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Соседняя</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/Subsystems/Корень/Subsystems/Родитель/Subsystems/Выбранная.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Выбранная</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects><Subsystem>Потомок</Subsystem></ChildObjects></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    fs::create_dir_all(
        root.join("src/Subsystems/Корень/Subsystems/Родитель/Subsystems/Выбранная/Subsystems"),
    )
    .unwrap();
    fs::write(
            root.join("src/Subsystems/Корень/Subsystems/Родитель/Subsystems/Выбранная/Subsystems/Потомок.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Subsystem><Properties><Name>Потомок</Name><IncludeInCommandInterface>true</IncludeInCommandInterface><Content/></Properties><ChildObjects/></Subsystem></MetaDataObject>"#,
        )
        .unwrap();
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Корень/Subsystems/Родитель/Subsystems/Выбранная.xml"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    let SubsystemInfoAnswer::Subsystem(data) =
        execution.data.expect("a file answers with subsystem data")
    else {
        panic!("a file must not answer as a directory tree");
    };
    assert_eq!(
        serde_json::to_value(&data.tree).unwrap(),
        serde_json::json!([{
            "name": "Корень",
            "content": 0,
            "children": [{
                "name": "Родитель",
                "content": 0,
                "children": [{
                    "name": "Выбранная",
                    "content": 0,
                    "children": [{"name": "Потомок", "content": 0, "children": []}]
                }]
            }]
        }])
    );
    let serialized = serde_json::to_value(data).unwrap();
    assert!(serialized.get("functionalSubsystems").is_none());
    assert!(serialized.get("interfaceSubsystems").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unregistered_alias_keeps_local_data_without_borrowing_a_registered_tree() {
    let (context, root) = workspace("unregistered-alias", false);
    let source_root = root.join("src");
    write_configuration(&source_root, &["Sales"]);
    fs::write(
        source_root.join("Subsystems/Sales.xml"),
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    fs::write(
        source_root.join("Subsystems/Copy.xml"),
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Copy.xml"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    let SubsystemInfoAnswer::Subsystem(data) = execution
        .data
        .expect("the alias remains readable local data")
    else {
        panic!("a concrete XML must answer with local subsystem data");
    };
    assert_eq!(data.name, "Sales");
    assert!(
        data.tree.is_none(),
        "an unregistered file stem cannot borrow the registered Sales identity"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parent_component_before_subsystems_is_rejected_before_any_registered_read() {
    let (context, root) = workspace("parent-component", false);
    let source_root = root.join("src");
    write_configuration(&source_root, &["Sales"]);
    fs::write(
        source_root.join("Subsystems/Sales.xml"),
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();

    let outside = root.join("outside");
    fs::create_dir_all(outside.join("nested")).unwrap();
    fs::create_dir_all(outside.join("Subsystems")).unwrap();
    write_configuration(&outside, &["Sales"]);
    fs::write(
        outside.join("Subsystems/Sales.xml"),
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    if !create_directory_symlink_or_skip(&outside.join("nested"), &source_root.join("tmp")) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/tmp/../Subsystems/Sales.xml"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    assert!(
        execution
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("parent path component")),
        "{:?}",
        execution.outcome
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bak_symlink_alias_is_not_a_subsystem_info_xml_target() {
    let (context, root) = workspace("bak-alias", false);
    let source_root = root.join("src");
    write_configuration(&source_root, &["Sales"]);
    let registered = source_root.join("Subsystems/Sales.xml");
    fs::write(
        &registered,
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let alias = source_root.join("Subsystems/Sales.bak");
    if !create_file_symlink_or_skip(&registered, &alias) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Sales.bak"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    assert!(
        execution
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("exact lowercase .xml")),
        "{:?}",
        execution.outcome
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn xml_symlink_alias_is_not_a_subsystem_info_target() {
    let (context, root) = workspace("xml-symlink-alias", false);
    let registered = root.join("src/Subsystems/Продажи.xml");
    let alias = root.join("src/Subsystems/Alias.xml");
    if !create_file_symlink_or_skip(&registered, &alias) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Alias.xml"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    assert!(
        execution
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("regular lowercase .xml")),
        "{:?}",
        execution.outcome
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn xml_beneath_an_intermediate_directory_symlink_is_not_a_local_target() {
    let (context, root) = workspace("xml-intermediate-directory-symlink", false);
    let external = root.join("external");
    fs::create_dir_all(&external).unwrap();
    fs::write(
        external.join("Local.xml"),
        child_subsystem_stub_xml(
            "Local",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let alias = root.join("src/Alias");
    if !create_directory_symlink_or_skip(&external, &alias) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Alias/Local.xml"))]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn xml_replaced_by_a_symlink_between_check_and_open_is_rejected() {
    let (context, root) = workspace("xml-check-open-swap", false);
    let target = root.join("src/Subsystems/Local.xml");
    fs::write(
        &target,
        child_subsystem_stub_xml(
            "Local",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let external = root.join("external.xml");
    fs::write(
        &external,
        child_subsystem_stub_xml(
            "External",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let target_for_hook = target.clone();
    let external_for_hook = external.clone();
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Local.xml"),
    )]);

    let execution = with_secure_tree_test_hook(
        move |phase| {
            if phase == &SecureTreePhase::BeforeOpenEntry(PathBuf::from("Local.xml")) {
                fs::remove_file(&target_for_hook).unwrap();
                let _ = create_file_symlink_for_test(&external_for_hook, &target_for_hook);
            }
        },
        || analyze_subsystem_info(&args, &context),
    );

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn extensionless_alias_is_not_a_subsystem_info_xml_target() {
    let (context, root) = workspace("extensionless-alias", false);
    let source_root = root.join("src");
    write_configuration(&source_root, &["Sales"]);
    fs::write(
        source_root.join("Subsystems/Sales.xml"),
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    fs::write(
        source_root.join("Subsystems/Sales"),
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems/Sales"))]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    assert!(
        execution
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("exact lowercase .xml")),
        "{:?}",
        execution.outcome
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn uppercase_xml_alias_is_not_a_subsystem_info_xml_target() {
    let (context, root) = workspace("uppercase-xml-alias", false);
    let source_root = root.join("src");
    fs::write(
        source_root.join("Subsystems/Sales.XML"),
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Sales.XML"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    assert!(
        execution
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("exact lowercase .xml")),
        "{:?}",
        execution.outcome
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn root_subsystems_symlink_is_not_followed_for_a_tree_answer() {
    let (context, root) = workspace("root-symlink", false);
    let source_root = root.join("src");
    fs::remove_dir_all(source_root.join("Subsystems")).unwrap();
    write_configuration(&source_root, &["Sales"]);

    let external = root.join("external-root");
    fs::create_dir_all(external.join("Subsystems")).unwrap();
    write_configuration(&external, &["Sales"]);
    fs::write(
        external.join("Subsystems/Sales.xml"),
        child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    if !create_directory_symlink_or_skip(
        &external.join("Subsystems"),
        &source_root.join("Subsystems"),
    ) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    let args = Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems"))]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn nested_subsystems_symlink_is_not_followed_for_a_tree_answer() {
    let (context, root) = workspace("nested-symlink", false);
    let source_root = root.join("src");
    write_configuration(&source_root, &["Root"]);
    fs::write(
        source_root.join("Subsystems/Root.xml"),
        child_subsystem_stub_xml(
            "Root",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        )
        .replacen(
            "<ChildObjects/>",
            "<ChildObjects><Subsystem>Child</Subsystem></ChildObjects>",
            1,
        ),
    )
    .unwrap();

    let external = root.join("external-nested");
    fs::create_dir_all(external.join("Subsystems")).unwrap();
    write_configuration(&external, &["Child"]);
    fs::write(
        external.join("Subsystems/Child.xml"),
        child_subsystem_stub_xml(
            "Child",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
        ),
    )
    .unwrap();
    let nested = source_root.join("Subsystems/Root/Subsystems");
    fs::create_dir_all(nested.parent().unwrap()).unwrap();
    if !create_directory_symlink_or_skip(&external.join("Subsystems"), &nested) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    let args = Map::from_iter([(
        "SubsystemPath".to_string(),
        json!("src/Subsystems/Root/Subsystems"),
    )]);

    let execution = analyze_subsystem_info(&args, &context);

    assert!(!execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.data.is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn subsystem_info_answers_content_and_command_interface_at_once() {
    let (context, root) = workspace("full", true);

    let execution = analyze_subsystem_info(&info_args(), &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    assert!(execution.outcome.stdout.is_none());
    let SubsystemInfoAnswer::Subsystem(data) =
        execution.data.expect("subsystem info answers with data")
    else {
        panic!("a file must answer as one subsystem");
    };
    assert_eq!(data.name, "Продажи");
    assert_eq!(data.content, vec!["Catalog.Goods".to_string()]);
    assert!(data.tree.is_none());
    let interface = data
        .command_interface
        .expect("the subsystem ships a command interface");
    assert_eq!(interface.visibility.len(), 1);
    assert!(!interface.visibility[0].visible);
    let _ = fs::remove_dir_all(root);
}

/// An absent `CommandInterface.xml` and an interface that hides nothing are
/// different facts, so one is `null` and the other is an empty list.
#[test]
fn a_missing_command_interface_is_null_not_an_empty_interface() {
    let (context, root) = workspace("bare", false);

    let execution = analyze_subsystem_info(&info_args(), &context);

    assert!(execution.outcome.ok, "{:?}", execution.outcome);
    let SubsystemInfoAnswer::Subsystem(data) =
        execution.data.expect("subsystem info answers with data")
    else {
        panic!("a file must answer as one subsystem");
    };
    assert!(data.tree.is_none());
    assert!(data.command_interface.is_none());
    let _ = fs::remove_dir_all(root);
}

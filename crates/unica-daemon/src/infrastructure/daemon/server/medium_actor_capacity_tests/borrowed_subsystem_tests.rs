use super::*;

/// rule: INV.SOURCE.BORROWED-OBJECT-PARENT-ADDRESS
/// rule: INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH
#[test]
fn borrowed_subsystem_topology_rechecks_parent_flag_and_registration() {
    let workspace = tempfile::tempdir().unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/acceptance/workspace-borrowed-subsystems");
    copy_tree_for_test(&fixture, workspace.path());
    let runtime = v13_runtime_for_borrowing_test();
    let read = |at| {
        let result = submit_canonical(
            &runtime,
            workspace.path(),
            ToolIdentity::View,
            serde_json::json!({ "at": at }),
        );
        assert!(result.ok, "{result:?}");
        result.data.unwrap()
    };
    let address = "cfe:Subsystem.Отчеты";
    let parent = workspace.path().join("src/cf/Subsystems/Отчеты.xml");
    let original = std::fs::read_to_string(&parent).unwrap();
    let first = read(address);
    assert!(first.get("limits").is_none(), "{first}");
    assert!(first["props"].get("libraryRoot").is_none());

    std::fs::write(&parent, original.replace(">true<", ">false<")).unwrap();
    let changed = read(address);
    assert_eq!(
        changed["props"]["libraryRoot"]["module"],
        "cfe:CommonModule.ОбновлениеИнформационнойБазыТест"
    );

    let registration = workspace.path().join("src/cf/Configuration.xml");
    let registered = std::fs::read_to_string(&registration).unwrap();
    std::fs::write(
        registration,
        registered.replace("<Subsystem>Отчеты</Subsystem>", ""),
    )
    .unwrap();
    let missing = read(address);
    assert_eq!(missing["props"]["parentStatus"], "not_found");
    assert!(missing["props"].get("libraryRoot").is_none());
    assert!(
        missing["limits"][0]
            .as_str()
            .unwrap()
            .contains("registered parent subsystem"),
        "{missing}"
    );

    let module = read("cfe:CommonModule.ОбновлениеИнформационнойБазыТест.Relation");
    assert!(
        module["limits"][0]
            .as_str()
            .unwrap()
            .contains("registered parent subsystem"),
        "{module}"
    );
}

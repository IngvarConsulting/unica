#[test]
fn internal_meta_add_remains_absent_from_the_production_registry() {
    let names = unica_coder::application::tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert!(!names.contains(&"unica.meta.add"));
}

#[test]
fn meta_edit_keeps_the_current_public_registry_entry_until_the_atomic_switch() {
    let names = unica_coder::application::tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert!(names.contains(&"unica.meta.edit"));
    assert!(!names.contains(&"unica.meta.add"));
}

#[test]
fn meta_remove_keeps_the_legacy_public_entry_until_the_atomic_switch() {
    let tools = unica_coder::application::tools();
    let names = tools.iter().map(|tool| tool.name).collect::<Vec<_>>();

    assert!(names.contains(&"unica.meta.remove"));
    assert_eq!(
        names
            .iter()
            .filter(|name| name.starts_with("unica.meta."))
            .count(),
        6
    );
    assert!(!names.contains(&"unica.meta.add"));
}

#[test]
fn meta_info_keeps_the_legacy_public_entry_until_the_atomic_switch() {
    let tools = unica_coder::application::tools();
    let info = tools
        .iter()
        .find(|tool| tool.name == "unica.meta.info")
        .expect("the current six-tool registry must retain meta.info");

    assert!(matches!(
        info.handler,
        unica_coder::application::ToolHandler::NativeOperation {
            operation: "meta-info",
            event: None
        }
    ));
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool.name.starts_with("unica.meta."))
            .count(),
        6
    );
}

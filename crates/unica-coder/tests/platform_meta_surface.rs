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

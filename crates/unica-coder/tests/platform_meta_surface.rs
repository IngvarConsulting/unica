#[test]
fn internal_meta_add_remains_absent_from_the_production_registry() {
    let names = unica_coder::application::tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    assert!(!names.contains(&"unica.meta.add"));
}

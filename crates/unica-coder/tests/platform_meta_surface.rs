#[test]
fn public_meta_surface_is_the_exact_four_typed_operations() {
    let tools = unica_coder::application::tools();
    let meta = tools
        .iter()
        .filter(|tool| tool.name.starts_with("unica.meta."))
        .collect::<Vec<_>>();

    assert_eq!(
        meta.iter().map(|tool| tool.name).collect::<Vec<_>>(),
        vec![
            "unica.meta.info",
            "unica.meta.add",
            "unica.meta.edit",
            "unica.meta.remove",
        ]
    );
    assert!(!meta[0].execution.is_mutating());
    assert!(meta[1..].iter().all(|tool| tool.execution.is_mutating()));
    assert!(meta.iter().all(|tool| matches!(
        tool.handler,
        unica_coder::application::ToolHandler::Metadata { .. }
    )));
}

#[test]
fn retired_meta_routes_are_not_registered() {
    let names = unica_coder::application::tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();

    for retired in [
        "unica.meta.compile",
        "unica.meta.profile",
        "unica.meta.validate",
    ] {
        assert!(!names.contains(&retired), "retired {retired} is public");
    }
}

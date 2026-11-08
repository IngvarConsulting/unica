#[test]
fn public_meta_surface_is_the_exact_four_typed_operations() {
    let tools = unica_daemon::application::tools();
    let meta = tools
        .iter()
        .filter(|tool| tool.name.starts_with("unica.meta."))
        .collect::<Vec<_>>();

    assert_eq!(
        meta.iter().map(|tool| tool.name).collect::<Vec<_>>(),
        vec!["unica.meta.info", "unica.meta.add", "unica.meta.edit",]
    );
    assert!(!meta[0].execution.is_mutating());
    assert!(meta[1..].iter().all(|tool| tool.execution.is_mutating()));
    assert!(meta.iter().all(|tool| matches!(
        tool.handler,
        unica_daemon::application::ToolHandler::Metadata { .. }
    )));
}

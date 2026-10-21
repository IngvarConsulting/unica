use serde_json::Value;
#[test]
fn parser_library_commit_matches_the_bundled_analyzer_contract() {
    let tools: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/unica/third-party/tools.lock.json"
    )))
    .unwrap();
    let expected = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bsl-analyzer")
        .and_then(|tool| tool["sourceCommit"].as_str())
        .unwrap();
    let cargo_lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"));

    for package in ["parser", "syntax"] {
        let block = cargo_lock
            .split("[[package]]")
            .find(|block| block.contains(&format!("name = \"{package}\"")))
            .unwrap();
        assert!(
            block.contains(expected),
            "{package} must use the bundled bsl-analyzer sourceCommit {expected}"
        );
    }
}

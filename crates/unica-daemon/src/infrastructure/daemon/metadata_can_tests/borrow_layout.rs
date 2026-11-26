use super::*;

#[test]
fn overlap_refusal_preserves_both_locations_and_guide_after_redaction() {
    let fixture = Fixture::new("workspace-borrow-overlap");
    let workspace = &fixture.workspace;
    let extension = "src/cf/token=fixture-secret-root";
    std::fs::rename(
        workspace.join("src/cf/extension-project"),
        workspace.join(extension),
    )
    .unwrap();
    std::fs::rename(
        workspace.join("src/cf/Configuration.xml"),
        workspace.join("Configuration.xml"),
    )
    .unwrap();
    std::fs::rename(
        workspace.join("src/cf/Catalogs"),
        workspace.join("Catalogs"),
    )
    .unwrap();
    let config = workspace.join("v8project.yaml");
    let yaml = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        config,
        yaml.replace("path: src/cf\n", "path: .\n")
            .replace("src/cf/extension-project", extension),
    )
    .unwrap();

    let before = unica_workspace::test_support::tree_snapshot(workspace);
    let response = call(
        &runtime(),
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"ext:Configuration","ops":[{"op":"object.borrow","args":{"from":"main:Catalog.Items"}}]}),
    );

    assert!(!response.ok && response.data.is_none(), "{response:?}");
    assert_eq!(response.diagnostics[0]["code"], "provider_unavailable");
    assert_eq!(response.diagnostics[0]["outcome"], "needsHuman");
    assert_eq!(response.diagnostics[0]["message"], response.summary);
    let message = response.summary;
    assert!(
        message.contains(r#""ext" ("src/cf/token=<redacted>")"#),
        "{message}"
    );
    assert!(message.contains(r#""main" (".")"#), "{message}");
    assert!(
        message.contains("Move the extension") && message.contains("v8project.yaml"),
        "{message}"
    );
    assert!(!message.contains("fixture-secret-root"), "{message}");
    assert!(
        !message.contains(workspace.to_string_lossy().as_ref()),
        "{message}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(workspace),
        before
    );
}

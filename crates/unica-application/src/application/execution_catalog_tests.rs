use crate::application::tool_contracts::SurfaceRelease;
use crate::application::v13::tool_catalog::catalog_for;
use unica_execution::invocation_store::ToolIdentity;
#[test]
fn persisted_tool_identity_matches_the_eight_invocation_catalog_entries() {
    let catalog = catalog_for(SurfaceRelease::V13).expect("hidden v0.13 catalog");
    let expected = catalog
        .tools
        .iter()
        .map(|tool| format!("unica.{}", tool.name))
        .collect::<Vec<_>>();
    let actual = ToolIdentity::ALL
        .iter()
        .map(|tool| {
            serde_json::to_value(tool)
                .expect("tool serializes")
                .as_str()
                .expect("tool identity is a string")
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
}

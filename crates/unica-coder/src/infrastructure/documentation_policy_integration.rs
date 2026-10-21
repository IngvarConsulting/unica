//! Cross-consumer configuration and platform policy integration.

use crate::infrastructure::operational_config::load_operational_config;
use std::time::Duration;
use unica_documentation_sources::documentation_policy::{DocumentationPolicy, NetworkAccess};
use unica_workspace::test_support::{create_file_link_fixture_for_test, FileLinkFixtureOutcome};
const KNOWN: &[&str] = &["v8std", "configuration-help"];
fn workspace() -> tempfile::TempDir {
    tempfile::tempdir().expect("временный каталог")
}

#[test]
fn mixed_unversioned_container_serves_network_and_operational_consumers() {
    let dir = workspace();
    std::fs::write(
            dir.path().join("unica.toml"),
            "[network]\ndefault = \"deny\"\n\n[operational.code_intelligence]\nsearch_total_timeout_seconds = 90\n",
        )
        .expect("write mixed workspace config");

    let policy = DocumentationPolicy::load(dir.path(), KNOWN)
        .expect("network consumer must accept a valid operational sibling");
    let operational = load_operational_config(dir.path())
        .expect("operational consumer must accept a valid network sibling");

    assert_eq!(policy.network("v8std"), NetworkAccess::Deny);
    assert_eq!(
        operational.code_intelligence().search_total_timeout(),
        Duration::from_secs(90)
    );
}

#[test]
fn characterization_operational_loader_ignores_invalid_policy_subtrees() {
    let cases = [
        ("[network]\nunknown = \"deny\"\n", "unknown"),
        ("[providers.v8std]\nnetwork = \"maybe\"\n", "maybe"),
    ];

    for (contents, policy_error_fragment) in cases {
        let dir = workspace();
        std::fs::write(dir.path().join("unica.toml"), contents)
            .expect("write policy-owned invalid subtree");

        load_operational_config(dir.path())
            .expect("policy subtree errors must not change operational defaults");
        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("policy consumer must refuse its invalid subtree");

        assert!(
            error.contains(policy_error_fragment),
            "unexpected error: {error}"
        );
    }
}

#[test]
fn dangling_policy_config_links_fail_closed_as_present_files() {
    for file_name in ["unica.toml", "unica.local.toml"] {
        let dir = workspace();
        let outcome = create_file_link_fixture_for_test(
            dir.path().join("missing-policy-target.toml"),
            dir.path().join(file_name),
        )
        .expect("create dangling policy config link fixture");
        if outcome != FileLinkFixtureOutcome::Created {
            return;
        }

        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("a present but unreadable policy layer must fail closed");

        assert!(error.contains(file_name), "unexpected error: {error}");
        assert!(error.contains("не читается"), "unexpected error: {error}");
    }
}

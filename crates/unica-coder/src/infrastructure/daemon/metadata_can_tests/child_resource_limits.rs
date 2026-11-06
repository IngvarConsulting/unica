use super::*;
use unica_workspace::test_support::retained_read_probe::{RetainedRead, RetainedReadProbe};

const OWNER: &str = "main:Catalog.Products";

fn resource_fixture(runtime: &V5CanonicalInvocationRuntime) -> Fixture {
    let fixture = catalog_and_constant(runtime);
    for operation in [
        json!({"op":"form.add","args":{"items":[{"name":"Main","type":"ObjectForm"}]}}),
        json!({"op":"template.add","args":{"items":[{"name":"Sheet","templateType":"SpreadsheetDocument"}]}}),
        json!({"op":"command.add","args":{"items":[{"name":"Run"}]}}),
    ] {
        publish(runtime, &fixture, json!({"at":OWNER,"ops":[operation]}));
    }
    fixture
}

fn preserves_owner(data: &Value) {
    assert_eq!(data["at"], OWNER);
    assert_eq!(data["title"], "Products");
    assert_eq!(data["props"]["Synonym"], "Products");
    for kind in ["Form", "Template", "Command"] {
        assert!(
            data["branches"]
                .as_array()
                .unwrap()
                .contains(&json!({"at":format!("{OWNER}.{kind}"),"count":1})),
            "{data}"
        );
    }
}

fn reports_resource(data: &Value, logical: &str) {
    preserves_owner(data);
    assert!(
        data["limits"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|limit| limit.as_str().is_some_and(
                |text| text.contains("provider_unavailable") && text.contains(logical)
            )),
        "{data}"
    );
}

#[test]
fn canonical_child_resource_limits_are_fresh_and_cover_form_template_and_command() {
    let runtime = runtime();
    let fixture = resource_fixture(&runtime);
    let owner_bytes = std::fs::read(fixture.source.join("Catalogs/Products.xml")).unwrap();
    for (kind, name, directory) in [
        ("Form", "Main", "Forms"),
        ("Template", "Sheet", "Templates"),
        ("Command", "Run", "Commands"),
    ] {
        let root = fixture
            .source
            .join("Catalogs/Products")
            .join(directory)
            .join(name);
        let existed = root.exists();
        std::fs::create_dir_all(&root).unwrap();
        let unexpected = root.join("unexpected.bin");
        std::fs::write(&unexpected, b"unmodelled").unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let data = view(&runtime, &fixture, OWNER);
        reports_resource(&data, &format!("{OWNER}.{kind}.{name}"));
        assert!(!data
            .to_string()
            .contains(&fixture.workspace.display().to_string()));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        let checked = call(&runtime, &fixture, ToolIdentity::Check, json!({"at":OWNER}));
        assert_eq!(check_status(&checked), "failed", "{checked:?}");
        assert!(diagnostics(&checked)
            .iter()
            .any(|diagnostic| diagnostic["code"] == "provider_unavailable"
                && diagnostic["metadataPath"] == format!("Catalog.Products.{kind}.{name}")));
        std::fs::remove_file(unexpected).unwrap();
        if !existed {
            std::fs::remove_dir(&root).unwrap();
        }
        let clean = view(&runtime, &fixture, OWNER);
        preserves_owner(&clean);
        assert!(
            clean["limits"].as_array().is_none_or(Vec::is_empty),
            "{clean}"
        );
        assert_eq!(
            std::fs::read(fixture.source.join("Catalogs/Products.xml")).unwrap(),
            owner_bytes,
            "only a resource changed; the descriptor cache cannot hide it"
        );
    }
}

#[test]
fn canonical_missing_form_payload_limits_the_resource_without_hiding_owner() {
    let runtime = runtime();
    let fixture = resource_fixture(&runtime);
    std::fs::remove_file(
        fixture
            .source
            .join("Catalogs/Products/Forms/Main/Ext/Form.xml"),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let data = view(&runtime, &fixture, OWNER);
    reports_resource(&data, &format!("{OWNER}.Form.Main"));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn canonical_child_resource_observation_never_reads_an_unrelated_owner() {
    let runtime = runtime();
    let fixture = resource_fixture(&runtime);
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Catalog","name":"Unrelated"}}}]}),
    );
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Unrelated","ops":[{"op":"form.add","args":{"items":[{"name":"Other","type":"ObjectForm"}]}}]}),
    );
    std::fs::write(
        fixture
            .source
            .join("Catalogs/Unrelated/Forms/Other/unexpected.bin"),
        b"not selected",
    )
    .unwrap();
    let reads = RetainedReadProbe::watch(&fixture.source);
    let data = view(&runtime, &fixture, OWNER);
    preserves_owner(&data);
    assert!(
        data["limits"].as_array().is_none_or(Vec::is_empty),
        "{data}"
    );
    let observed = reads.take();
    assert!(observed.iter().any(|event| matches!(event,RetainedRead::Opened(path) if path==Path::new("Catalogs/Products/Forms/Main/Ext/Form.xml"))),"{observed:?}");
    assert!(
        observed.iter().all(|event| match event {
            RetainedRead::Opened(path) | RetainedRead::Listed(path) =>
                !path.starts_with("Catalogs/Unrelated") && !path.starts_with("Constants"),
        }),
        "{observed:?}"
    );
}

#[test]
fn canonical_child_resource_links_are_limits_without_reading_the_link_target() {
    use unica_workspace::test_support::{
        create_directory_link_fixture_for_test, create_file_link_fixture_for_test,
        remove_dir_symlink_for_test, FileLinkFixtureOutcome,
    };
    let probe = tempfile::tempdir().unwrap();
    std::fs::create_dir(probe.path().join("directory")).unwrap();
    std::fs::write(probe.path().join("file"), b"probe").unwrap();
    for outcome in [
        create_directory_link_fixture_for_test(
            probe.path().join("directory"),
            probe.path().join("dir-link"),
        )
        .unwrap(),
        create_file_link_fixture_for_test(
            probe.path().join("file"),
            probe.path().join("file-link"),
        )
        .unwrap(),
    ] {
        if !matches!(outcome, FileLinkFixtureOutcome::Created) {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "the host cannot create both file and directory symlinks",
            );
            return;
        }
    }
    for relative in ["", "Ext", "Ext/Form.xml"] {
        let runtime = runtime();
        let fixture = resource_fixture(&runtime);
        let outside = tempfile::tempdir().unwrap();
        let base = fixture.source.join("Catalogs/Products/Forms/Main");
        let link = if relative.is_empty() {
            base
        } else {
            base.join(relative)
        };
        let target = outside.path().join("payload");
        std::fs::rename(&link, &target).unwrap();
        let outcome = if relative == "Ext/Form.xml" {
            create_file_link_fixture_for_test(&target, &link)
        } else {
            create_directory_link_fixture_for_test(&target, &link)
        }
        .unwrap();
        assert!(matches!(outcome, FileLinkFixtureOutcome::Created));
        let reads = RetainedReadProbe::watch(outside.path());
        let owner_before = std::fs::read(fixture.source.join("Catalogs/Products.xml")).unwrap();
        let data = view(&runtime, &fixture, OWNER);
        reports_resource(&data, &format!("{OWNER}.Form.Main"));
        assert!(
            reads.take().is_empty(),
            "the child reader must not follow {relative}"
        );
        assert_eq!(
            std::fs::read(fixture.source.join("Catalogs/Products.xml")).unwrap(),
            owner_before
        );
        if relative == "Ext/Form.xml" {
            std::fs::remove_file(link).unwrap();
        } else {
            remove_dir_symlink_for_test(link).unwrap();
        }
    }
}

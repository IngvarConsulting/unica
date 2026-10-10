use super::*;
use crate::domain::apply::ApplyRequest;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply;
use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
use crate::infrastructure::platform::testing::file_identity_for_test;
use crate::infrastructure::workspace_actor::{
    ApplyPublicationErrorKind, PreparedApplyBatch, WorkspaceActor, WorkspaceIdentity,
    WorkspaceSourceSetInput,
};
use crate::test_support::tree_snapshot;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const TEMPLATE: &str = "main:Report.Versions.Template.Schema";
const DATASET: &str = "main:Report.Versions.Template.Schema.DataSet.Data";
const SETTING: &str = "main:Report.Versions.Template.Schema.Setting.Main";
const RELATIVE: &str = "Reports/Versions/Templates/Schema/Ext/Template.xml";
const XML: &str = "<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:t=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\r\n<dataSource><name>Local</name><dataSourceType>Local</dataSourceType></dataSource>\n<dataSet xsi:type=\"DataSetQuery\"><name>Data</name><field><dataPath>Amount</dataPath><field>Amount</field></field><dataSource>Local</dataSource><query>ВЫБРАТЬ 1 КАК Amount<!-- query note -->, <?keep original?>2 КАК Added</query></dataSet>\r\n<settingsVariant><t:name>Main</t:name><t:settings><t:selection/><t:filter/></t:settings></settingsVariant>\n<!-- untouched --><?outside exact?></DataCompositionSchema>";

fn exact_bytes() -> Vec<u8> {
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend_from_slice(XML.as_bytes());
    bytes
}

fn fixture(bytes: &[u8]) -> ApplySeamFixture {
    let fixture = ApplySeamFixture::new();
    let source = fixture.source_dir();
    let configuration = source.join("Configuration.xml");
    let original = std::fs::read_to_string(&configuration).unwrap();
    std::fs::write(
        configuration,
        original.replace("<ChildObjects>", "<ChildObjects><Report>Versions</Report>"),
    )
    .unwrap();
    std::fs::create_dir_all(source.join("Reports/Versions/Templates/Schema/Ext")).unwrap();
    std::fs::write(source.join("Reports/Versions.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Versions</Name></Properties><ChildObjects><Template>Schema</Template></ChildObjects></Report></MetaDataObject>"#).unwrap();
    std::fs::write(source.join("Reports/Versions/Templates/Schema.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Schema</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#).unwrap();
    std::fs::write(source.join(RELATIVE), bytes).unwrap();
    fixture
}

fn preview_request(at: &str, ops: Value) -> ApplyRequest {
    let args = json!({"at":at,"ops":ops,"dryRun":true});
    ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap()
}

fn query_request(at: &str) -> ApplyRequest {
    preview_request(
        at,
        json!([{"op":"query.patch","args":{"values":{"find":"1 КАК Amount","replace":"3 КАК Amount","once":true}}}]),
    )
}

fn prepare_preview(fixture: &ApplySeamFixture, request: &ApplyRequest) -> PreparedApplyBatch {
    assert!(request.dry_run());
    let mut admission = fixture.admission();
    admission.bind_request(request);
    let (staged, effects) = plan_hidden_v13_apply(request, &fixture.binding, &admission).unwrap();
    admission.prepare_with_effects(staged, effects).unwrap()
}

fn publication_input(
    fixture: &ApplySeamFixture,
    preview: &ApplyRequest,
) -> (
    ApplyRequest,
    crate::infrastructure::workspace_actor::ApplyAdmission,
) {
    let projected = fixture
        .actor()
        .publish_prepared_apply(prepare_preview(fixture, preview))
        .unwrap();
    assert_eq!(projected.commit_count_for_test(), 0);
    let operations = preview
        .ops()
        .iter()
        .map(|operation| json!({"op":operation.name(),"args":operation.args()}))
        .collect::<Vec<_>>();
    let args = json!({"at":preview.at().to_string(),"ops":operations,"dryRun":false,"ifRev":projected.rev()});
    let request = ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
    let mut admission = fixture
        .actor()
        .admit_apply(
            &fixture.binding,
            request.if_rev(),
            false,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    admission.bind_request(&request);
    (request, admission)
}

fn prepare_publication(fixture: &ApplySeamFixture, preview: &ApplyRequest) -> PreparedApplyBatch {
    let (request, admission) = publication_input(fixture, preview);
    let (staged, effects) = plan_hidden_v13_apply(&request, &fixture.binding, &admission).unwrap();
    admission.prepare_with_effects(staged, effects).unwrap()
}

fn snapshot(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    if path.exists() {
        tree_snapshot(path)
    } else {
        BTreeMap::new()
    }
}

fn cache_root(fixture: &ApplySeamFixture) -> PathBuf {
    fixture.source_dir().parent().unwrap().join(".build/unica")
}

#[test]
fn canonical_dcs_preview_and_publication_preserve_bom_mixed_newlines_and_unowned_bytes() {
    let original = exact_bytes();
    let fixture = fixture(&original);
    let path = fixture.source_dir().join(RELATIVE);
    let source_before = snapshot(&fixture.source_dir());
    let cache_before = snapshot(&cache_root(&fixture));
    let identity = file_identity_for_test(&path).unwrap();
    let preview = prepare_preview(&fixture, &query_request(DATASET));
    let projected = fixture.actor().publish_prepared_apply(preview).unwrap();
    assert_eq!(projected.commit_count_for_test(), 0);
    assert_eq!(snapshot(&fixture.source_dir()), source_before);
    assert_eq!(snapshot(&cache_root(&fixture)), cache_before);
    assert_eq!(file_identity_for_test(&path).unwrap(), identity);

    let prepared = prepare_publication(&fixture, &query_request(DATASET));
    let published = fixture.actor().publish_prepared_apply(prepared).unwrap();
    assert_eq!(
        published.commit_count_for_test(),
        1,
        "must execute a real actor commit"
    );
    assert!(!published.effects().events().is_empty());
    let expected = String::from_utf8(original)
        .unwrap()
        .replace("1 КАК Amount", "3 КАК Amount")
        .into_bytes();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        expected,
        "only the selected query text may change"
    );
    assert!(!expected.ends_with(b"\n"));
    let mut expected_source = source_before;
    expected_source.insert(PathBuf::from(RELATIVE), expected);
    assert_eq!(snapshot(&fixture.source_dir()), expected_source);
}

#[test]
fn canonical_dcs_noop_has_no_commit_events_cache_write_or_file_replacement() {
    let original = exact_bytes();
    let fixture = fixture(&original);
    let path = fixture.source_dir().join(RELATIVE);
    let source_before = snapshot(&fixture.source_dir());
    let cache_before = snapshot(&cache_root(&fixture));
    let identity = file_identity_for_test(&path).unwrap();
    let revision_service = fixture
        .actor()
        .source_revision_service(&fixture.binding)
        .unwrap();
    let machine_before = revision_service.machine_state_for_test();
    let preview = preview_request(SETTING, json!([{"op":"filter.clear","args":{}}]));
    let (noop, admission) = publication_input(&fixture, &preview);
    let (staged, effects) = plan_hidden_v13_apply(&noop, &fixture.binding, &admission).unwrap();
    assert!(staged.planned_changes().is_empty());
    let published = fixture
        .actor()
        .publish_prepared_apply(admission.prepare_with_effects(staged, effects).unwrap())
        .unwrap();
    assert_eq!(published.commit_count_for_test(), 0);
    assert!(published.effects().events().is_empty());
    assert_eq!(snapshot(&fixture.source_dir()), source_before);
    assert_eq!(snapshot(&cache_root(&fixture)), cache_before);
    assert_eq!(file_identity_for_test(&path).unwrap(), identity);
    assert_eq!(revision_service.machine_state_for_test(), machine_before);
}

#[test]
fn canonical_dcs_refuses_wrong_root_namespace_and_version_attribute_before_staging() {
    for body in [
        b"<garbage/>".to_vec(),
        XML.replace(
            "http://v8.1c.ru/8.1/data-composition-system/schema",
            "urn:foreign",
        )
        .into_bytes(),
        XML.replacen(
            "<DataCompositionSchema ",
            "<DataCompositionSchema version=\"2.20\" ",
            1,
        )
        .into_bytes(),
    ] {
        let fixture = fixture(&body);
        let path = fixture.source_dir().join(RELATIVE);
        let source_before = snapshot(&fixture.source_dir());
        let identity = file_identity_for_test(&path).unwrap();
        let admission = fixture.admission();
        let result = plan_hidden_v13_apply(&query_request(DATASET), &fixture.binding, &admission);
        assert!(result.is_err(), "wrong declared DCS document was planned");
        assert_eq!(snapshot(&fixture.source_dir()), source_before);
        assert_eq!(file_identity_for_test(&path).unwrap(), identity);
        assert!(snapshot(&cache_root(&fixture)).is_empty());
    }
}

#[test]
fn canonical_dcs_captured_body_and_each_owner_descriptor_reject_stale_publication() {
    for relative in [
        RELATIVE,
        "Reports/Versions/Templates/Schema.xml",
        "Reports/Versions.xml",
        "Configuration.xml",
    ] {
        let original = exact_bytes();
        let fixture = fixture(&original);
        let target = fixture.source_dir().join(RELATIVE);
        let prepared = prepare_publication(&fixture, &query_request(DATASET));
        let changed = fixture.source_dir().join(relative);
        let mut concurrent = std::fs::read(&changed).unwrap();
        concurrent.extend_from_slice(b"<!-- concurrent owner -->");
        std::fs::write(&changed, &concurrent).unwrap();
        let concurrent_source = snapshot(&fixture.source_dir());
        let cache_before = snapshot(&cache_root(&fixture));
        let error = fixture
            .actor()
            .publish_prepared_apply(prepared)
            .expect_err("stale plan must not publish");
        assert_eq!(
            error.kind(),
            ApplyPublicationErrorKind::ConcurrentRevision,
            "{relative}: {error}"
        );
        assert_eq!(
            snapshot(&fixture.source_dir()),
            concurrent_source,
            "{relative}: overwrite or rollback of foreign bytes"
        );
        assert_eq!(snapshot(&cache_root(&fixture)), cache_before);
        if relative != RELATIVE {
            assert_eq!(std::fs::read(target).unwrap(), original);
        }
    }
}

#[test]
fn canonical_dcs_supported_source_still_requires_each_exact_owner_version() {
    for relative in [
        "Configuration.xml",
        "Reports/Versions.xml",
        "Reports/Versions/Templates/Schema.xml",
    ] {
        for version in ["2.19", "2.20.0", "2.&#50;0"] {
            let fixture = fixture(&exact_bytes());
            let owner = fixture.source_dir().join(relative);
            let original = std::fs::read_to_string(&owner).unwrap();
            std::fs::write(
                &owner,
                original.replace("version=\"2.20\"", &format!("version=\"{version}\"")),
            )
            .unwrap();
            let before = snapshot(&fixture.source_dir());
            let target = fixture.source_dir().join(RELATIVE);
            let identity = file_identity_for_test(&target).unwrap();
            let admission = fixture.admission();
            let error =
                plan_hidden_v13_apply(&query_request(DATASET), &fixture.binding, &admission)
                    .expect_err("an owner version was silently migrated");
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::InvalidSource,
                "{relative} version={version}: {error}"
            );
            assert_eq!(snapshot(&fixture.source_dir()), before);
            assert_eq!(file_identity_for_test(&target).unwrap(), identity);
            assert!(snapshot(&cache_root(&fixture)).is_empty());
        }
    }
}

#[test]
fn canonical_dcs_late_planning_failure_discards_prior_successful_operations() {
    let original = exact_bytes();
    let fixture = fixture(&original);
    let source_before = snapshot(&fixture.source_dir());
    let path = fixture.source_dir().join(RELATIVE);
    let identity = file_identity_for_test(&path).unwrap();
    let batch = preview_request(
        TEMPLATE,
        json!([
            {"op":"field.add","args":{"at":DATASET,"items":[{"dataPath":"Added","title":"Added"}]}},
            {"op":"query.patch","args":{"at":DATASET,"values":{"find":"Missing query text","replace":"cannot publish","once":true}}}
        ]),
    );
    let admission = fixture.admission();
    let error = plan_hidden_v13_apply(&batch, &fixture.binding, &admission).unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert!(
        error.path().is_some_and(|path| path.starts_with("ops[1]")),
        "first operation did not reach planning: {error}"
    );
    assert_eq!(snapshot(&fixture.source_dir()), source_before);
    assert_eq!(file_identity_for_test(&path).unwrap(), identity);
    assert!(snapshot(&cache_root(&fixture)).is_empty());
}

#[test]
fn canonical_dcs_late_publication_failure_rolls_back_both_schemas_and_cache() {
    use crate::infrastructure::native_operations::compile_transaction::{
        set_retained_apply_failpoint, RetainedApplyFailpoint,
    };
    for failpoint in [
        RetainedApplyFailpoint::StateMarker,
        RetainedApplyFailpoint::AfterAllPostimages,
    ] {
        let fixture = fixture(&exact_bytes());
        let source = fixture.source_dir();
        let second = "Reports/Versions/Templates/Second/Ext/Template.xml";
        std::fs::create_dir_all(source.join(second).parent().unwrap()).unwrap();
        std::fs::write(source.join(second), exact_bytes()).unwrap();
        std::fs::write(source.join("Reports/Versions/Templates/Second.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><Properties><Name>Second</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#).unwrap();
        let owner = source.join("Reports/Versions.xml");
        std::fs::write(
            &owner,
            std::fs::read_to_string(&owner).unwrap().replace(
                "<Template>Schema</Template>",
                "<Template>Schema</Template><Template>Second</Template>",
            ),
        )
        .unwrap();
        let batch = preview_request(
            "main:Report.Versions",
            json!([
                {"op":"query.patch","args":{"at":DATASET,"values":{"find":"1 КАК Amount","replace":"3 КАК Amount","once":true}}},
                {"op":"query.patch","args":{"at":"main:Report.Versions.Template.Second.DataSet.Data","values":{"find":"1 КАК Amount","replace":"4 КАК Amount","once":true}}}
            ]),
        );
        let prepared = prepare_publication(&fixture, &batch);
        let source_before = snapshot(&source);
        let cache_before = snapshot(&cache_root(&fixture));
        let identity = file_identity_for_test(&source.join(RELATIVE)).unwrap();
        let second_identity = file_identity_for_test(&source.join(second)).unwrap();
        let service = fixture
            .actor()
            .source_revision_service(&fixture.binding)
            .unwrap();
        let machine_before = service.machine_state_for_test();
        set_retained_apply_failpoint(failpoint);
        let error = fixture
            .actor()
            .publish_prepared_apply(prepared)
            .expect_err("injected late publication must fail");
        assert!(
            error
                .to_string()
                .contains("injected retained apply failure"),
            "late checkpoint was not reached: {error}"
        );
        assert_eq!(snapshot(&source), source_before);
        assert_eq!(snapshot(&cache_root(&fixture)), cache_before);
        assert_eq!(
            file_identity_for_test(&source.join(RELATIVE)).unwrap(),
            identity
        );
        assert_eq!(
            file_identity_for_test(&source.join(second)).unwrap(),
            second_identity
        );
        assert_eq!(service.machine_state_for_test(), machine_before);
    }
}

#[test]
fn canonical_dcs_old_external_owner_is_refused_before_any_source_write() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("epf");
    std::fs::create_dir_all(source.join("Old/Templates/Schema/Ext")).unwrap();
    std::fs::write(root.path().join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: epf\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n").unwrap();
    std::fs::write(source.join("Old.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><ExternalDataProcessor uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Old</Name></Properties><ChildObjects><Template>Schema</Template></ChildObjects></ExternalDataProcessor></MetaDataObject>"#).unwrap();
    std::fs::write(source.join("Old/Templates/Schema.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Schema</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#).unwrap();
    std::fs::write(
        source.join("Old/Templates/Schema/Ext/Template.xml"),
        exact_bytes(),
    )
    .unwrap();
    let workspace_root = std::fs::canonicalize(root.path()).unwrap();
    let source = std::fs::canonicalize(source).unwrap();
    let context = WorkspaceContext {
        cwd: workspace_root.clone(),
        workspace_root: workspace_root.clone(),
        cache_root: workspace_root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [WorkspaceSourceSetInput::new(
            "epf",
            &source,
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )],
        "dcs-external-refusal",
    )
    .unwrap();
    let actor = WorkspaceActor::new(identity, context).unwrap();
    let binding: ProviderRootBinding = actor.bind_provider_root("epf", &source).unwrap();
    let admission = actor
        .admit_apply(
            &binding,
            None,
            true,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        )
        .unwrap();
    let args = json!({"at":"epf:ExternalDataProcessor.Old.Template.Schema.DataSet.Data","ops":[{"op":"query.set","args":{"values":{"query":"ВЫБРАТЬ 3 КАК Amount"}}}],"dryRun":true});
    let request = ApplyRequest::parse(args.as_object().unwrap(), &["epf"]).unwrap();
    let before = snapshot(&source);
    let error = plan_hidden_v13_apply(&request, &binding, &admission)
        .expect_err("external DCS must not gain writer authority");
    assert!(
        matches!(
            error.kind(),
            ApplyPlanErrorKind::ProviderUnavailable | ApplyPlanErrorKind::InvalidSource
        ),
        "{error}"
    );
    assert_eq!(snapshot(&source), before);
    assert!(snapshot(&workspace_root.join(".build/unica")).is_empty());
}

use super::{plan_hidden_v13_apply, tests::ApplySeamFixture};
use crate::domain::apply::ApplyRequest;
use serde_json::{json, Value};
use unica_source_contract::proposal::ApplyPlanError;
use unica_workspace::apply::ApplyStagedState;
use unica_workspace::test_support::tree_snapshot;

const OWNER: &str = "main:IntegrationService.Шина";
const DESCRIPTOR: &str = "IntegrationServices/Шина.xml";
struct Fixture(ApplySeamFixture);
impl Fixture {
    fn new() -> Self {
        let seam = ApplySeamFixture::new();
        let native = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-integration-service-address/src");
        for (relative, bytes) in tree_snapshot(&native) {
            let path = seam.source_dir().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        Self(seam)
    }
    fn request(ops: Vec<Value>) -> ApplyRequest {
        let input = json!({"at":OWNER,"ops":ops,"dryRun":true});
        ApplyRequest::parse(input.as_object().unwrap(), &["main"]).unwrap()
    }
    fn plan(&self, ops: Vec<Value>) -> Result<ApplyStagedState, ApplyPlanError> {
        let request = Self::request(ops);
        plan_hidden_v13_apply(&request, &self.0.binding, &self.0.admission())
            .map(|(staged, _)| staged)
    }
    fn bytes(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.0.source_dir().join(relative)).unwrap()
    }
    fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) {
        std::fs::write(self.0.source_dir().join(relative), bytes).unwrap();
    }
    fn postimage(&self, staged: &ApplyStagedState, relative: &str) -> Vec<u8> {
        staged
            .planned_changes()
            .into_iter()
            .find(|change| change.relative_path == std::path::Path::new(relative))
            .map_or_else(
                || self.bytes(relative),
                |change| {
                    change
                        .current
                        .as_option()
                        .expect("expected a retained file")
                },
            )
    }
    fn save(&self, ops: Vec<Value>) -> String {
        use unica_execution_contract::invocation::DomainResult;
        let request = Self::request(ops);
        let mut admission = self.0.admission();
        admission.bind_request_identity(
            &request.at().to_string(),
            request
                .ops()
                .iter()
                .map(|op| (op.name(), op.at().to_string(), op.args())),
        );
        let before = tree_snapshot(&self.0.source_dir());
        let (staged, effects) =
            plan_hidden_v13_apply(&request, &self.0.binding, &admission).unwrap();
        let batch = admission.prepare_with_effects(staged, effects).unwrap();
        let projected = self.0.actor().preview_prepared_apply(&batch).unwrap();
        assert_eq!(projected.commit_count_for_test(), 0);
        assert_eq!(tree_snapshot(&self.0.source_dir()), before);
        let mut result = DomainResult::success("preview");
        result.rev = Some(projected.rev().to_owned());
        self.0.actor().save_prepared_apply(batch, result).unwrap()
    }
    fn execute(
        &self,
        token: &str,
    ) -> Result<
        unica_execution_contract::invocation::DomainResult,
        unica_workspace::actor::ApplyPublicationError,
    > {
        use std::time::Duration;
        use unica_work_control::{
            cancellation::CancellationToken, provider_deadline::ProviderDeadline,
        };
        let mut refusal = None;
        let result = self
            .0
            .actor()
            .execute_saved_apply(
                token,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
                |mut result, publication| {
                    match publication {
                        Ok(publication) => result.rev = Some(publication.rev().to_owned()),
                        Err(error) => refusal = Some(error),
                    }
                    result
                },
            )
            .map_err(|error| match error {
                unica_workspace::actor::SavedApplyExecutionError::Publication(error) => error,
                other => panic!("unexpected saved-plan error: {other:?}"),
            })?;
        refusal.map_or(Ok(result), Err)
    }
}

fn set(value: Value) -> Vec<Value> {
    vec![
        json!({"op":"props.set","args":{"at":OWNER,"values":{"ExternalIntegrationServiceAddress":value}}}),
    ]
}

#[test]
fn integration_address_preview_token_roundtrip_preserves_channels_modules_and_other_bytes() {
    let fixture = Fixture::new();
    let original = tree_snapshot(&fixture.0.source_dir());
    for value in [
        "https://example.invalid/Первый?a=1&b=2",
        "https://example.invalid/Второй?quote=\"x\"",
        "",
    ] {
        let before = tree_snapshot(&fixture.0.source_dir());
        let staged = fixture.plan(set(json!(value))).unwrap();
        let expected = fixture.postimage(&staged, DESCRIPTOR);
        let token = fixture.save(set(json!(value)));
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
        assert!(fixture.execute(&token).unwrap().ok);
        let mut after = tree_snapshot(&fixture.0.source_dir());
        assert_eq!(
            after.remove(std::path::Path::new(DESCRIPTOR)).unwrap(),
            expected
        );
        let mut unmentioned = before;
        unmentioned.remove(std::path::Path::new(DESCRIPTOR));
        assert_eq!(after, unmentioned, "no unmentioned file is changed");
        assert_eq!(
            unica_metadata_designer::integration_service_address::read(&fixture.bytes(DESCRIPTOR))
                .unwrap()
                .as_deref(),
            Some(value)
        );
        let before_noop = tree_snapshot(&fixture.0.source_dir());
        let staged = fixture.plan(set(json!(value))).unwrap();
        assert!(staged.planned_changes().is_empty());
        let token = fixture.save(set(json!(value)));
        assert!(fixture.execute(&token).unwrap().ok);
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before_noop);
    }
    assert_eq!(tree_snapshot(&fixture.0.source_dir()), original);
}

#[test]
fn integration_address_preview_refuses_bad_arguments_and_unproven_source_without_writes() {
    for value in [
        json!(false),
        json!(7),
        json!(null),
        json!([]),
        json!("\u{0}"),
    ] {
        let fixture = Fixture::new();
        let before = tree_snapshot(&fixture.0.source_dir());
        assert!(fixture.plan(set(value)).is_err());
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
    }
    for (relative, from, to) in [
        (DESCRIPTOR, "<ExternalIntegrationServiceAddress/>", ""),
        (
            DESCRIPTOR,
            "<ExternalIntegrationServiceAddress/>",
            "<ExternalIntegrationServiceAddress/><ExternalIntegrationServiceAddress/>",
        ),
        (
            DESCRIPTOR,
            "<ExternalIntegrationServiceAddress/>",
            "<ExternalIntegrationServiceAddress xmlns=\"urn:foreign\"/>",
        ),
        (
            DESCRIPTOR,
            "<ExternalIntegrationServiceAddress/>",
            "<ExternalIntegrationServiceAddress><Nested/></ExternalIntegrationServiceAddress>",
        ),
        (DESCRIPTOR, "<Name>Шина</Name>", "<Name>Чужая</Name>"),
        (DESCRIPTOR, "version=\"2.20\"", "version=\"2.&#50;0\""),
        (
            "Configuration.xml",
            "<IntegrationService>Шина</IntegrationService>",
            "",
        ),
        ("Configuration.xml", "version=\"2.20\"", "version=\"2.21\""),
    ] {
        let fixture = Fixture::new();
        let source = String::from_utf8(fixture.bytes(relative)).unwrap();
        assert!(source.contains(from));
        fixture.write(relative, source.replace(from, to));
        let before = tree_snapshot(&fixture.0.source_dir());
        assert!(
            fixture
                .plan(set(json!("https://example.invalid/")))
                .is_err(),
            "{relative} {to}"
        );
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
    }
}

#[test]
fn integration_address_saved_plan_fences_descriptor_and_configuration_registration() {
    for relative in [DESCRIPTOR, "Configuration.xml"] {
        let fixture = Fixture::new();
        let token = fixture.save(set(json!("https://example.invalid/")));
        let mut bytes = fixture.bytes(relative);
        bytes.extend_from_slice(b"<!-- external edit -->");
        fixture.write(relative, bytes);
        let before = tree_snapshot(&fixture.0.source_dir());
        assert!(fixture.execute(&token).is_err(), "{relative}");
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
    }
}

#[test]
fn integration_address_publication_keeps_property_local_namespace_declarations() {
    let fixture = Fixture::new();
    let original = String::from_utf8(fixture.bytes(DESCRIPTOR)).unwrap().replace(
        "<ExternalIntegrationServiceAddress/>",
        "<m:ExternalIntegrationServiceAddress xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:opaque=\"urn:keep>quoted\"/>",
    );
    fixture.write(DESCRIPTOR, &original);
    for value in ["", "https://example.invalid/", ""] {
        let token = fixture.save(set(json!(value)));
        assert!(fixture.execute(&token).unwrap().ok);
        let bytes = fixture.bytes(DESCRIPTOR);
        assert_eq!(
            unica_metadata_designer::integration_service_address::read(&bytes)
                .unwrap()
                .as_deref(),
            Some(value)
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\""));
        assert!(text.contains("xmlns:opaque=\"urn:keep>quoted\""));
    }
    assert_eq!(fixture.bytes(DESCRIPTOR), original.as_bytes());
}

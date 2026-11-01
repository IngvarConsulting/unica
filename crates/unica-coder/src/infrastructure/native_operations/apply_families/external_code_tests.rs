use super::plan_hidden_v13_apply;
use crate::domain::apply::ApplyRequest;
use crate::infrastructure::source_workspace::WorkspaceActor;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;
use unica_execution_contract::invocation::DomainResult;
use unica_source_contract::proposal::{ApplyPlanError, SupportPolicyMode};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::{
    ApplyAdmission, ApplyPublicationError, ApplyPublicationErrorKind, ProviderRootBinding,
    WorkspaceIdentity, WorkspaceSourceSetInput,
};
use unica_workspace::test_support::{
    attempt_retained_directory_replacement_for_test, tree_snapshot,
    RetainedDirectoryReplacementOutcome,
};

const INSERT: &str =
    "Процедура CorpusProbe() Экспорт\n    Сообщить(\"external code\");\nКонецПроцедуры\n";

struct Fixture {
    _root: tempfile::TempDir,
    source: PathBuf,
    at: String,
    name: &'static str,
    actor: WorkspaceActor,
    binding: ProviderRootBinding,
}
fn copy_tree(from: &Path, to: &Path) {
    for (relative, bytes) in tree_snapshot(from) {
        let path = to.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}
impl Fixture {
    fn new(kind: SourceSetKind) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(root.path()).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        copy_tree(
            &repo.join("tests/fixtures/acceptance/workspace-external"),
            &workspace,
        );
        // Workspace editing policy remains applicable; a configuration marker
        // at the artifact collection root does not describe its support state.
        std::fs::write(
            workspace.join(".v8-project.json"),
            r#"{"editingAllowedCheck":"on"}"#,
        )
        .unwrap();
        let (set, owner, name) = match kind {
            SourceSetKind::ExternalReport => ("erf", "ExternalReport", "Sales"),
            SourceSetKind::ExternalProcessor => ("epf", "ExternalDataProcessor", "Import"),
            _ => unreachable!(),
        };
        let source = workspace.join(set);
        if kind == SourceSetKind::ExternalReport {
            copy_tree(&source.join("Sales"), &source.join("Other"));
            let descriptor = std::fs::read_to_string(source.join("Sales.xml"))
                .unwrap()
                .replace("<Name>Sales</Name>", "<Name>Other</Name>")
                .replace(
                    "b405f719-5b3d-5912-8bbb-cf9adfc25f8c",
                    "33333333-3333-4333-8333-333333333333",
                );
            std::fs::write(source.join("Other.xml"), descriptor).unwrap();
        }
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/code-test"),
            workspace_epoch: 1,
        };
        let identity = WorkspaceIdentity::new(
            &context,
            [
                WorkspaceSourceSetInput::new(
                    "erf",
                    workspace.join("erf"),
                    SourceSetKind::ExternalReport,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                ),
                WorkspaceSourceSetInput::new(
                    "epf",
                    workspace.join("epf"),
                    SourceSetKind::ExternalProcessor,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                ),
            ],
            "external-code-test",
        )
        .unwrap();
        let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
        let binding = actor.bind_provider_root(set, &source).unwrap();
        Self {
            _root: root,
            source,
            at: format!("{set}:{owner}.{name}.Module.Object"),
            name,
            actor,
            binding,
        }
    }
    fn request(&self, at: &str) -> ApplyRequest {
        ApplyRequest::parse(
            json!({"at":at,"dryRun":true,"ops":[{"op":"code.insert","args":{"text":INSERT}}]})
                .as_object()
                .unwrap(),
            &["epf", "erf"],
        )
        .unwrap()
    }
    fn admission(&self, request: &ApplyRequest) -> ApplyAdmission {
        let mut admission = self
            .actor
            .admit_apply(
                &self.binding,
                request.if_rev(),
                request.dry_run(),
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(admission.support_policy_mode(), SupportPolicyMode::Deny);
        admission.bind_request_identity(
            &request.at().to_string(),
            request
                .ops()
                .iter()
                .map(|op| (op.name(), op.at().to_string(), op.args())),
        );
        admission
    }
    fn save(&self) -> String {
        let before = tree_snapshot(&self.source);
        let request = self.request(&self.at);
        let admission = self.admission(&request);
        let (staged, effects) = plan_hidden_v13_apply(&request, &self.binding, &admission).unwrap();
        let batch = admission.prepare_with_effects(staged, effects).unwrap();
        let projected = self.actor.preview_prepared_apply(&batch).unwrap();
        assert_eq!(projected.commit_count_for_test(), 0);
        assert_eq!(tree_snapshot(&self.source), before);
        let mut result = DomainResult::success("preview");
        result.rev = Some(projected.rev().to_owned());
        self.actor.save_prepared_apply(batch, result).unwrap()
    }
    fn execute(&self, token: &str) -> Result<DomainResult, ApplyPublicationError> {
        let mut refusal = None;
        let result = self
            .actor
            .execute_saved_apply(
                token,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
                |mut result, publication| {
                    match publication {
                        Ok(publication) => {
                            result.rev = Some(publication.rev().to_owned());
                            result.data =
                                Some(json!({"commits":publication.commit_count_for_test()}));
                        }
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
    fn refused(&self, at: &str) -> ApplyPlanError {
        let before = tree_snapshot(&self.source);
        let request = self.request(at);
        let error = match plan_hidden_v13_apply(&request, &self.binding, &self.admission(&request))
        {
            Err(error) => error,
            Ok(_) => panic!("invalid external owner must refuse"),
        };
        assert_eq!(tree_snapshot(&self.source), before);
        error
    }
    fn module(&self) -> PathBuf {
        self.source.join(self.name).join("Ext/ObjectModule.bsl")
    }
}

#[test]
fn external_object_code_publishes_for_erf_and_epf_without_touching_neighbors() {
    for kind in [
        SourceSetKind::ExternalReport,
        SourceSetKind::ExternalProcessor,
    ] {
        let fixture = Fixture::new(kind);
        let owner = fixture.source.join(format!("{}.xml", fixture.name));
        let owner_before = std::fs::read(&owner).unwrap();
        let other_before = tree_snapshot(&fixture.source.join("Other"));
        let other_xml = std::fs::read(fixture.source.join("Other.xml")).unwrap();
        let token = fixture.save();
        let published = fixture.execute(&token).unwrap();
        assert!(published.ok);
        assert_eq!(published.data.unwrap()["commits"], 1);
        assert!(std::fs::read_to_string(fixture.module())
            .unwrap()
            .contains("Процедура CorpusProbe() Экспорт"));
        assert_eq!(std::fs::read(owner).unwrap(), owner_before);
        assert_eq!(tree_snapshot(&fixture.source.join("Other")), other_before);
        assert_eq!(
            std::fs::read(fixture.source.join("Other.xml")).unwrap(),
            other_xml
        );
    }
}

#[test]
fn external_code_can_create_an_absent_object_module_only_under_a_proven_owner() {
    for kind in [
        SourceSetKind::ExternalReport,
        SourceSetKind::ExternalProcessor,
    ] {
        let fixture = Fixture::new(kind);
        std::fs::remove_file(fixture.module()).unwrap();
        let token = fixture.save();
        assert!(!fixture.module().exists());
        assert!(fixture.execute(&token).unwrap().ok);
        assert!(std::fs::read_to_string(fixture.module())
            .unwrap()
            .contains("CorpusProbe"));
    }
}

#[test]
fn external_code_token_retains_its_bsl_and_specific_owner_descriptor() {
    for descriptor in [false, true] {
        let fixture = Fixture::new(SourceSetKind::ExternalReport);
        let token = fixture.save();
        let path = if descriptor {
            fixture.source.join("Sales.xml")
        } else {
            fixture.module()
        };
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(if descriptor {
            b"<!-- concurrent -->"
        } else {
            b"\n// concurrent\n"
        });
        std::fs::write(path, bytes).unwrap();
        let concurrent = tree_snapshot(&fixture.source);
        let error = fixture.execute(&token).unwrap_err();
        assert_eq!(error.kind(), ApplyPublicationErrorKind::ConcurrentRevision);
        assert_eq!(tree_snapshot(&fixture.source), concurrent);
    }
}

#[test]
fn external_code_ignores_a_configuration_support_marker_at_the_collection_root() {
    let fixture = Fixture::new(SourceSetKind::ExternalReport);
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    std::fs::create_dir_all(&marker).unwrap();
    let token = fixture.save();
    std::fs::write(marker.join("unrelated.txt"), "changed after preview").unwrap();
    assert!(fixture.execute(&token).unwrap().ok);
}

#[test]
fn external_code_never_substitutes_a_neighbor_for_its_own_missing_or_invalid_owner() {
    for replacement in [None, Some("name"), Some("kind"), Some("version")] {
        let fixture = Fixture::new(SourceSetKind::ExternalReport);
        let owner = fixture.source.join("Sales.xml");
        let text = std::fs::read_to_string(&owner).unwrap();
        match replacement {
            None => std::fs::remove_file(owner).unwrap(),
            Some("name") => std::fs::write(
                owner,
                text.replace("<Name>Sales</Name>", "<Name>Other</Name>"),
            )
            .unwrap(),
            Some("kind") => std::fs::write(
                owner,
                text.replace("ExternalReport", "ExternalDataProcessor"),
            )
            .unwrap(),
            Some("version") => {
                std::fs::write(owner, text.replace("version=\"2.20\"", "version=\"2.19\"")).unwrap()
            }
            _ => unreachable!(),
        }
        let error = fixture.refused(&fixture.at);
        if replacement == Some("version") {
            assert!(error.to_string().contains("2.19"), "{error:?}");
        }
    }
}

#[test]
fn external_code_refuses_nested_modules_and_the_wrong_artifact_kind() {
    let fixture = Fixture::new(SourceSetKind::ExternalReport);
    for at in [
        "erf:ExternalReport.Sales.Form.Main.Module.Form",
        "erf:ExternalReport.Sales.Command.Run.Module.Command",
        "erf:ExternalDataProcessor.Sales.Module.Object",
    ] {
        fixture.refused(at);
    }
}

#[test]
fn external_code_refuses_ambiguous_and_non_scalar_owner_names() {
    let mut accepted = Vec::new();
    for name in [
        "<Name>Sales</Name><Name>Other</Name>",
        "<Name>Sales</Name><Name xmlns=\"urn:foreign\">Other</Name>",
        "<Name>Sales<Nested/></Name>",
        "<Name> Sales </Name>",
        "<Name xmlns=\"urn:foreign\">Sales</Name>",
        "<Name role=\"owner\">Sales</Name>",
    ] {
        let fixture = Fixture::new(SourceSetKind::ExternalReport);
        let owner = fixture.source.join("Sales.xml");
        let original = std::fs::read_to_string(&owner).unwrap();
        std::fs::write(&owner, original.replace("<Name>Sales</Name>", name)).unwrap();
        let before = tree_snapshot(&fixture.source);
        let request = fixture.request(&fixture.at);
        let admission = fixture.admission(&request);
        let result = plan_hidden_v13_apply(&request, &fixture.binding, &admission);
        if result.is_ok() {
            accepted.push(name);
        }
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
    assert!(
        accepted.is_empty(),
        "ambiguous owners authorized code previews: {accepted:?}"
    );
}

#[test]
fn external_code_refuses_duplicate_owner_properties() {
    let fixture = Fixture::new(SourceSetKind::ExternalReport);
    let owner = fixture.source.join("Sales.xml");
    let original = std::fs::read_to_string(&owner).unwrap();
    std::fs::write(
        &owner,
        original.replace(
            "</Properties>",
            "</Properties><Properties><Name>Other</Name></Properties>",
        ),
    )
    .unwrap();
    let before = tree_snapshot(&fixture.source);
    let request = fixture.request(&fixture.at);
    let admission = fixture.admission(&request);
    assert!(plan_hidden_v13_apply(&request, &fixture.binding, &admission).is_err());
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn external_code_accepts_comments_within_the_exact_owner_name() {
    for kind in [
        SourceSetKind::ExternalReport,
        SourceSetKind::ExternalProcessor,
    ] {
        let fixture = Fixture::new(kind);
        let owner = fixture.source.join(format!("{}.xml", fixture.name));
        let original = std::fs::read_to_string(&owner).unwrap();
        let (first, last) = fixture.name.split_at(2);
        std::fs::write(
            &owner,
            format!("\u{feff}{}", original.trim_start_matches('\u{feff}')).replace(
                &format!("<Name>{}</Name>", fixture.name),
                &format!("<Name>{first}<!-- retained -->{last}</Name>"),
            ),
        )
        .unwrap();
        let request = fixture.request(&fixture.at);
        let admission = fixture.admission(&request);
        assert!(plan_hidden_v13_apply(&request, &fixture.binding, &admission).is_ok());
    }
}

#[test]
fn external_code_cannot_publish_through_a_replaced_collection_root() {
    let fixture = Fixture::new(SourceSetKind::ExternalReport);
    let token = fixture.save();
    let old_module = std::fs::read(fixture.module()).unwrap();
    let held = fixture.source.with_file_name("erf-held");
    let original = tree_snapshot(&fixture.source);
    match attempt_retained_directory_replacement_for_test(&fixture.source, &held).unwrap() {
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
            assert_eq!(tree_snapshot(&fixture.source), original);
            assert!(!held.exists());
        }
        RetainedDirectoryReplacementOutcome::Replaced => {
            copy_tree(&held, &fixture.source);
            let replacement = tree_snapshot(&fixture.source);
            assert!(fixture.execute(&token).is_err());
            assert_eq!(tree_snapshot(&fixture.source), replacement);
            assert_eq!(
                std::fs::read(held.join("Sales/Ext/ObjectModule.bsl")).unwrap(),
                old_module
            );
        }
    }
}

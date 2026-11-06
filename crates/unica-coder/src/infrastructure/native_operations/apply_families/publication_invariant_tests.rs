//! Publication invariants through the ordinary native planner and workspace actor.
use super::tests::ApplySeamFixture;
use super::{plan_hidden_v13_apply, ApplyStagedState, PlannedApplyEffects};
use crate::domain::apply::ApplyRequest;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::{ApplyAdmission, ApplyPublicationErrorKind, PreparedApplyBatch};
use unica_workspace::test_support::{path_identity_for_test, tree_snapshot};

pub(super) fn request(at: &str, ops: Value) -> ApplyRequest {
    let input = json!({"at":at,"ops":ops,"dryRun":true});
    ApplyRequest::parse(input.as_object().unwrap(), &["main"]).unwrap()
}

pub(super) fn plan(
    fixture: &ApplySeamFixture,
    request: &ApplyRequest,
    admission: &mut ApplyAdmission,
) -> (ApplyStagedState, PlannedApplyEffects) {
    admission.bind_request_identity(
        &request.at().to_string(),
        request.ops().iter().map(|operation| {
            (
                operation.name(),
                operation.at().to_string(),
                operation.args(),
            )
        }),
    );
    plan_hidden_v13_apply(request, &fixture.binding, admission).unwrap()
}

pub(super) fn prepare(
    fixture: &ApplySeamFixture,
    request: &ApplyRequest,
    mut admission: ApplyAdmission,
) -> PreparedApplyBatch {
    let (staged, effects) = plan(fixture, request, &mut admission);
    admission.prepare_with_effects(staged, effects).unwrap()
}

pub(super) fn publication_input(
    fixture: &ApplySeamFixture,
    preview: &ApplyRequest,
) -> (ApplyRequest, ApplyAdmission) {
    assert!(preview.dry_run());
    let result = fixture
        .actor()
        .publish_prepared_apply(prepare(fixture, preview, fixture.admission()))
        .unwrap();
    assert_eq!(result.commit_count_for_test(), 0);
    let operations = preview
        .ops()
        .iter()
        .map(|operation| json!({"op":operation.name(),"args":operation.args()}))
        .collect::<Vec<_>>();
    let input =
        json!({"at":preview.at().to_string(),"ops":operations,"dryRun":false,"ifRev":result.rev()});
    let request = ApplyRequest::parse(input.as_object().unwrap(), &["main"]).unwrap();
    assert!(!request.dry_run());
    let admission = fixture
        .actor()
        .admit_apply(
            &fixture.binding,
            request.if_rev(),
            false,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
        .unwrap();
    (request, admission)
}

pub(super) fn prepare_publication(
    fixture: &ApplySeamFixture,
    preview: &ApplyRequest,
) -> PreparedApplyBatch {
    let (request, admission) = publication_input(fixture, preview);
    prepare(fixture, &request, admission)
}

#[derive(Debug, PartialEq, Eq)]
struct SnapshotEntry {
    bytes: Option<Vec<u8>>,
    identity: Option<String>,
}

#[derive(Debug)]
pub(super) struct Snapshot {
    root: PathBuf,
    entries: BTreeMap<PathBuf, SnapshotEntry>,
}

impl Snapshot {
    pub(super) fn capture(fixture: &ApplySeamFixture) -> Self {
        Self::capture_root(fixture.source_dir().parent().unwrap())
    }

    pub(super) fn capture_root(root: &Path) -> Self {
        let mut entries = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            assert!(
                !metadata.file_type().is_symlink(),
                "owned fixture has an unexpected symlink: {path:?}"
            );
            let bytes = if metadata.is_dir() {
                pending.extend(
                    std::fs::read_dir(&path)
                        .unwrap()
                        .map(|entry| entry.unwrap().path()),
                );
                None
            } else {
                Some(std::fs::read(&path).unwrap())
            };
            entries.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                SnapshotEntry {
                    bytes,
                    identity: path_identity_for_test(&path).unwrap(),
                },
            );
        }
        Self {
            root: root.to_path_buf(),
            entries,
        }
    }

    pub(super) fn assert_unchanged(&self) {
        assert_eq!(
            Self::capture_root(&self.root).entries,
            self.entries,
            "source, cache and state bytes/identities must remain exactly unchanged"
        );
    }
}

pub(super) fn assert_noop(fixture: &ApplySeamFixture, preview: &ApplyRequest) {
    let before = Snapshot::capture(fixture);
    // The preview issues a fresh revision; publication itself is explicitly
    // non-preview, so this cannot pass through the dry-run-only actor branch.
    let (request, mut admission) = publication_input(fixture, preview);
    assert!(!request.dry_run());
    let (staged, effects) = plan(fixture, &request, &mut admission);
    assert!(staged.planned_changes().is_empty());
    assert!(effects.events().is_empty());
    let prepared = admission.prepare_with_effects(staged, effects).unwrap();
    unica_workspace::compile_transaction::take_retained_apply_observed_events();
    let result = fixture.actor().publish_prepared_apply(prepared).unwrap();
    assert!(
        unica_workspace::compile_transaction::take_retained_apply_observed_events().is_empty(),
        "an empty publication must not enter physical source/cache/state publication"
    );
    assert_eq!(result.commit_count_for_test(), 0);
    assert!(result.effects().events().is_empty());
    before.assert_unchanged();
}

fn publish_change(fixture: &ApplySeamFixture, preview: &ApplyRequest) {
    let before = tree_snapshot(&fixture.source_dir());
    let result = fixture
        .actor()
        .publish_prepared_apply(prepare_publication(fixture, preview))
        .unwrap();
    assert_eq!(result.commit_count_for_test(), 1);
    assert!(!result.effects().events().is_empty());
    assert_ne!(
        tree_snapshot(&fixture.source_dir()),
        before,
        "the first actor publication must actually change source bytes"
    );
}

#[derive(Debug, Clone, Copy)]
enum Family {
    Configuration,
    Metadata,
    Form,
    Interface,
    Role,
    Subsystem,
}
const FAMILIES: [Family; 6] = [
    Family::Configuration,
    Family::Metadata,
    Family::Form,
    Family::Interface,
    Family::Role,
    Family::Subsystem,
];

fn write(fixture: &ApplySeamFixture, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = fixture.source_dir().join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn register(fixture: &ApplySeamFixture, kind: &str, name: &str) {
    let path = fixture.source_dir().join("Configuration.xml");
    let before = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        path,
        before.replace(
            "<ChildObjects>",
            &format!("<ChildObjects><{kind}>{name}</{kind}>"),
        ),
    )
    .unwrap();
}

fn seed_subsystem(fixture: &ApplySeamFixture) {
    register(fixture, "Subsystem", "Main");
    write(
        fixture,
        "Subsystems/Main.xml",
        unica_metadata_designer::resource_planner::test_support::subsystem_stub_xml(
            "Main",
            "2.20",
            "55555555-5555-4555-8555-555555555555",
        ),
    );
}

fn seed_form(fixture: &ApplySeamFixture) {
    // Use the current operation to seed a registered form and all of its real
    // companion resources, rather than hand-authoring an approximate layout.
    let creation = request(
        "main:Document.First",
        json!([
            {"op":"form.add","args":{"items":[{"name":"Card","type":"ObjectForm"}]}}
        ]),
    );
    publish_change(fixture, &creation);
}

fn seed_role(fixture: &ApplySeamFixture) {
    let creation = request(
        "main:Configuration",
        json!([
            {"op":"role.create","args":{"values":{"name":"Reader"}}}
        ]),
    );
    publish_change(fixture, &creation);
    // Keep a real, meaningful rights entry without granting Read yet.
    // right.set may then add Read to the existing object entry; its second
    // identical application must be an actual publication with no effects.
    let rights = request(
        "main:Role.Reader",
        json!([
            {"op":"right.add","args":{"items":[{"object":"Document.First","rights":{"Update":true}}]}}
        ]),
    );
    publish_change(fixture, &rights);
}

fn case(family: Family) -> (ApplySeamFixture, ApplyRequest, Vec<&'static str>) {
    let fixture = ApplySeamFixture::new();
    if matches!(family, Family::Configuration) {
        let path = fixture.source_dir().join("Configuration.xml");
        let source = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            path,
            source.replace(
                "<Name>Main</Name>",
                "<Name>Main</Name><Comment>original</Comment>",
            ),
        )
        .unwrap();
    }
    let (at, operation, args, inputs) = match family {
        Family::Configuration => (
            "main:Configuration",
            "props.set",
            json!({"values":{"Comment":"published"}}),
            vec!["Configuration.xml"],
        ),
        Family::Metadata => (
            "main:Document.First",
            "props.set",
            json!({"values":{"Comment":"published"}}),
            vec!["Configuration.xml", "Documents/First.xml"],
        ),
        Family::Form => {
            seed_form(&fixture);
            (
                "main:Document.First.Form.Card",
                "props.set",
                json!({"values":{"AutoTitle":false}}),
                vec![
                    "Configuration.xml",
                    "Documents/First.xml",
                    "Documents/First/Forms/Card.xml",
                    "Documents/First/Forms/Card/Ext/Form.xml",
                ],
            )
        }
        Family::Interface => {
            seed_subsystem(&fixture);
            write(&fixture, "Subsystems/Main/Ext/CommandInterface.xml", concat!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
                "<CommandInterface xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" version=\"2.20\">\n",
                "\t<GroupsOrder>\n\t\t<Group>NavigationPanelImportant</Group>\n\t\t<Group>NavigationPanelSeeAlso</Group>\n\t</GroupsOrder>\n</CommandInterface>\n",
            ));
            (
                "main:Subsystem.Main.Interface",
                "groupOrder.set",
                json!({"values":{"groups":["NavigationPanelSeeAlso","NavigationPanelImportant"]}}),
                vec![
                    "Configuration.xml",
                    "Subsystems/Main.xml",
                    "Subsystems/Main/Ext/CommandInterface.xml",
                ],
            )
        }
        Family::Role => {
            seed_role(&fixture);
            (
                "main:Role.Reader.Right.Document_First",
                "right.set",
                json!({"values":{"right":"Read","value":true}}),
                vec![
                    "Configuration.xml",
                    "Roles/Reader.xml",
                    "Roles/Reader/Ext/Rights.xml",
                ],
            )
        }
        Family::Subsystem => {
            seed_subsystem(&fixture);
            (
                "main:Subsystem.Main",
                "content.add",
                json!({"items":[{"object":"Document.First"}]}),
                vec!["Configuration.xml", "Subsystems/Main.xml"],
            )
        }
    };
    (
        fixture,
        request(at, json!([{"op":operation,"args":args}])),
        inputs,
    )
}

/// rule: INV.SOURCE.EMPTY-APPLY-PUBLICATION
#[test]
fn six_native_families_publish_the_mutation_then_publish_the_equivalent_request_without_writes() {
    for family in FAMILIES {
        let (fixture, preview, _) = case(family);
        publish_change(&fixture, &preview);
        assert_noop(&fixture, &preview);
    }
}

fn foreign_change(fixture: &ApplySeamFixture, relative: &str) {
    let path = fixture.source_dir().join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = std::fs::read(&path).unwrap_or_default();
    bytes.extend_from_slice(b"<!-- independently written after planning -->");
    std::fs::write(path, bytes).unwrap();
}

fn assert_stale_input_refused(
    fixture: &ApplySeamFixture,
    preview: &ApplyRequest,
    relative: &str,
    after_prepare: bool,
    creation: bool,
) {
    use unica_source_contract::proposal::ApplyStagingErrorKind;
    let was_absent = !fixture.source_dir().join(relative).exists();
    let publication_kind = if was_absent {
        ApplyPublicationErrorKind::ContainmentIdentity
    } else {
        ApplyPublicationErrorKind::ConcurrentRevision
    };
    let (request, mut admission) = if creation {
        // The domain request is a validated preview operation list. The
        // actor's admission is explicitly non-preview: creation is planned
        // once, without fabricating a token from an unrelated random image.
        (preview.clone(), fixture.admission_with_dry_run(false))
    } else {
        publication_input(fixture, preview)
    };
    let (staged, effects) = plan(fixture, &request, &mut admission);
    assert!(
        !staged.planned_changes().is_empty(),
        "must test a real pending publication"
    );
    if after_prepare {
        let prepared = admission.prepare_with_effects(staged, effects).unwrap();
        foreign_change(fixture, relative);
        let foreign = Snapshot::capture(fixture);
        let error = fixture
            .actor()
            .publish_prepared_apply(prepared)
            .expect_err("an actually consumed input changed after preparation");
        assert_eq!(error.kind(), publication_kind, "{relative}: {error}");
        foreign.assert_unchanged();
    } else {
        foreign_change(fixture, relative);
        let foreign = Snapshot::capture(fixture);
        match admission.prepare_with_effects(staged, effects) {
            Err(error) => assert!(
                error.kind() == ApplyStagingErrorKind::ConcurrentRevision
                    || (was_absent && error.kind() == ApplyStagingErrorKind::AbsentChainOccupied),
                "{relative}: {error}",
            ),
            Ok(prepared) => {
                // Preparation may retain the admitted inputs; the invariant
                // requires refusal before the actual publication, not at a
                // particular earlier guard implementation.
                let error = fixture
                    .actor()
                    .publish_prepared_apply(prepared)
                    .expect_err("a stale preimage prepared successfully and then published");
                assert_eq!(error.kind(), publication_kind, "{relative}: {error}");
            }
        }
        foreign.assert_unchanged();
    }
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
#[test]
fn medium_six_native_families_fence_each_read_owner_and_dependency_at_prepare_and_publication() {
    for family in FAMILIES {
        let (_, _, inputs) = case(family);
        for relative in inputs {
            for after_prepare in [false, true] {
                let (fixture, preview, _) = case(family);
                assert_stale_input_refused(&fixture, &preview, relative, after_prepare, false);
            }
        }
    }
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
#[test]
fn native_creations_fence_the_actual_absent_outputs_without_overwriting_foreign_files() {
    for after_prepare in [false, true] {
        for (op, at, args, outputs) in [
            (
                "object.create",
                "main:Configuration",
                json!({"values":{"kind":"Catalog","name":"Created"}}),
                vec!["Catalogs/Created.xml"],
            ),
            (
                "form.add",
                "main:Document.First",
                json!({"items":[{"name":"Created","type":"ObjectForm"}]}),
                vec![
                    "Documents/First/Forms/Created.xml",
                    "Documents/First/Forms/Created/Ext/Form.xml",
                ],
            ),
            (
                "role.create",
                "main:Configuration",
                json!({"values":{"name":"Created"}}),
                vec!["Roles/Created.xml", "Roles/Created/Ext/Rights.xml"],
            ),
            (
                "subsystem.create",
                "main:Configuration",
                json!({"values":{"name":"Created"}}),
                vec!["Subsystems/Created.xml"],
            ),
        ] {
            for relative in outputs {
                let fixture = ApplySeamFixture::new();
                assert!(!fixture.source_dir().join(relative).exists());
                let preview = request(at, json!([{"op":op,"args":args}]));
                assert_stale_input_refused(&fixture, &preview, relative, after_prepare, true);
            }
        }
    }
}

/// rule: INV.SOURCE.EMPTY-APPLY-PUBLICATION
/// rule: INV.APP.REQUEST-LEVEL-APPLY-EFFECT-RECONCILIATION
#[test]
fn reversing_a_real_configuration_edit_within_one_request_has_no_publication_effects() {
    let (fixture, first, _) = case(Family::Configuration);
    publish_change(&fixture, &first);
    let cancellation = request(
        "main:Configuration",
        json!([
            {"op":"props.set","args":{"values":{"Comment":"temporary"}}},
            {"op":"props.set","args":{"values":{"Comment":"published"}}},
        ]),
    );
    assert_noop(&fixture, &cancellation);
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
#[test]
fn native_creations_revalidate_the_consumed_owner_and_language_descriptors() {
    for after_prepare in [false, true] {
        for (op, at, args, dependencies) in [
            (
                "object.create",
                "main:Configuration",
                json!({"values":{"kind":"Catalog","name":"Created"}}),
                vec!["Configuration.xml", "Languages/Русский.xml"],
            ),
            (
                "form.add",
                "main:Document.First",
                json!({"items":[{"name":"Created","type":"ObjectForm"}]}),
                vec![
                    "Configuration.xml",
                    "Languages/Русский.xml",
                    "Documents/First.xml",
                ],
            ),
            (
                "role.create",
                "main:Configuration",
                json!({"values":{"name":"Created"}}),
                vec!["Configuration.xml", "Languages/Русский.xml"],
            ),
            (
                "subsystem.create",
                "main:Configuration",
                json!({"values":{"name":"Created"}}),
                vec!["Configuration.xml"],
            ),
        ] {
            for relative in dependencies {
                let fixture = ApplySeamFixture::new();
                assert!(fixture.source_dir().join(relative).is_file());
                let preview = request(at, json!([{"op":op,"args":args}]));
                assert_stale_input_refused(&fixture, &preview, relative, after_prepare, true);
            }
        }
    }
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
/// rule: INV.SOURCE.EMPTY-APPLY-PUBLICATION
#[test]
fn an_equivalent_empty_batch_still_revalidates_its_read_inputs_at_actual_publication() {
    for family in FAMILIES {
        let (fixture, preview, inputs) = case(family);
        publish_change(&fixture, &preview);
        let (request, mut admission) = publication_input(&fixture, &preview);
        let (staged, effects) = plan(&fixture, &request, &mut admission);
        assert!(staged.planned_changes().is_empty());
        assert!(effects.events().is_empty());
        let prepared = admission.prepare_with_effects(staged, effects).unwrap();
        let relative = inputs.last().unwrap();
        foreign_change(&fixture, relative);
        let foreign = Snapshot::capture(&fixture);
        let error = fixture
            .actor()
            .publish_prepared_apply(prepared)
            .expect_err("the empty-publication fast path must still validate consumed preimages");
        assert_eq!(
            error.kind(),
            ApplyPublicationErrorKind::ConcurrentRevision,
            "{family:?}: {error}"
        );
        foreign.assert_unchanged();
    }
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
#[test]
fn adding_a_physically_existing_child_subsystem_guards_its_read_only_descriptor() {
    const CHILD: &str = "Subsystems/Main/Subsystems/ExistingChild.xml";
    for after_prepare in [false, true] {
        let fixture = ApplySeamFixture::new();
        seed_subsystem(&fixture);
        write(
            &fixture,
            CHILD,
            unica_metadata_designer::resource_planner::test_support::subsystem_stub_xml(
                "ExistingChild",
                "2.20",
                "66666666-6666-4666-8666-666666666666",
            ),
        );
        // The physical child exists, but the parent's ChildObjects does not
        // register it yet. Only the parent is a planned output; the existing
        // child's descriptor is a consumed, read-only dependency.
        let preview = request(
            "main:Subsystem.Main",
            json!([
                {"op":"childSubsystem.add","args":{"items":[{"name":"ExistingChild"}]}}
            ]),
        );
        let (request, mut admission) = publication_input(&fixture, &preview);
        let (staged, _) = plan(&fixture, &request, &mut admission);
        assert!(staged
            .planned_changes()
            .iter()
            .any(|change| change.relative_path == Path::new("Subsystems/Main.xml")));
        assert!(!staged
            .planned_changes()
            .iter()
            .any(|change| change.relative_path == Path::new(CHILD)));
        drop(staged);
        drop(admission);
        assert_stale_input_refused(&fixture, &preview, CHILD, after_prepare, false);
    }
}

fn panels_request(changed: bool) -> ApplyRequest {
    // The same supported layouts used by the current public client-interface
    // tests: the native generator owns the CAI header and section structure.
    let values = if changed {
        json!({"top":[["tools","open"]],"left":["sections"],"bottom":["favorites"]})
    } else {
        json!({"top":["open"],"left":["sections"]})
    };
    request(
        "main:Configuration",
        json!([{"op":"panels.set","args":{"values":values}}]),
    )
}

/// rule: INV.SOURCE.EMPTY-APPLY-PUBLICATION
#[test]
fn configuration_panels_publish_a_real_change_then_an_equivalent_nonpreview_noop() {
    let fixture = ApplySeamFixture::new();
    publish_change(&fixture, &panels_request(false));
    publish_change(&fixture, &panels_request(true));
    assert_noop(&fixture, &panels_request(true));
}

/// rule: INV.SOURCE.BOUND-PREIMAGES
#[test]
fn configuration_panels_guard_the_read_only_owner_existing_payload_and_absent_output() {
    const INTERFACE: &str = "Ext/ClientApplicationInterface.xml";
    for after_prepare in [false, true] {
        // The CAI edit reads Configuration.xml but never plans to replace it.
        // Both the read-only owner and the existing generated payload matter.
        for relative in ["Configuration.xml", INTERFACE] {
            let fixture = ApplySeamFixture::new();
            publish_change(&fixture, &panels_request(false));
            assert_stale_input_refused(
                &fixture,
                &panels_request(true),
                relative,
                after_prepare,
                false,
            );
        }
        // The same route creates its payload when none exists. An independent
        // file occupying that exact absence must survive with its identity.
        let fixture = ApplySeamFixture::new();
        assert!(!fixture.source_dir().join(INTERFACE).exists());
        assert_stale_input_refused(
            &fixture,
            &panels_request(true),
            INTERFACE,
            after_prepare,
            false,
        );
    }
}

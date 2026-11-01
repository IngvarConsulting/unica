use super::{plan_hidden_v13_apply, tests::ApplySeamFixture};
use crate::domain::apply::ApplyRequest;
use serde_json::{json, Value};
use unica_source_contract::proposal::ApplyPlanError;
use unica_workspace::apply::ApplyStagedState;
use unica_workspace::test_support::tree_snapshot;

const OWNER: &str = "main:ChartOfCalculationTypes.ВидыРасчета";

struct Fixture(ApplySeamFixture);
impl Fixture {
    fn new() -> Self {
        Self::with_kind(unica_source_contract::SourceSetKind::Configuration)
    }
    fn with_kind(kind: unica_source_contract::SourceSetKind) -> Self {
        let seam = ApplySeamFixture::with_kind(kind);
        let native = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-calculation-code-type/src");
        for (relative, bytes) in tree_snapshot(&native) {
            let path = seam.source_dir().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        Self(seam)
    }
    fn request(ops: Vec<Value>) -> ApplyRequest {
        let first = ops
            .first()
            .and_then(|op| op["args"]["at"].as_str())
            .unwrap_or("main:Configuration");
        let at = if first.starts_with(EMPTY_OWNER) {
            EMPTY_OWNER
        } else if first.starts_with(OWNER) {
            OWNER
        } else {
            "main:Configuration"
        };
        let input = json!({"at":at,"ops":ops,"dryRun":true});
        ApplyRequest::parse(input.as_object().unwrap(), &["main"]).unwrap()
    }
    fn plan(&self, ops: Vec<Value>) -> Result<ApplyStagedState, ApplyPlanError> {
        let request = Self::request(ops);
        plan_hidden_v13_apply(&request, &self.0.binding, &self.0.admission())
            .map(|(staged, _)| staged)
    }
}
fn code_type(value: &str) -> Value {
    json!({"op":"props.set","args":{"at":OWNER,"values":{"CodeType":value}}})
}

#[test]
fn calculation_chart_refuses_implicit_code_type_migration() {
    let fixture = Fixture::new();
    let before = tree_snapshot(&fixture.0.source_dir());
    let result = fixture.plan(vec![code_type("Number")]);
    let error =
        result.expect_err("a CodeType-only edit must not reinterpret string predefined codes");
    assert!(error.to_string().contains("Code"), "{error:?}");
    assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
}

const DESCRIPTOR: &str = "ChartsOfCalculationTypes/ВидыРасчета.xml";
const PREDEFINED: &str = "ChartsOfCalculationTypes/ВидыРасчета/Ext/Predefined.xml";
const EMPTY_OWNER: &str = "main:ChartOfCalculationTypes.ПустойПлан";
const EMPTY_DESCRIPTOR: &str = "ChartsOfCalculationTypes/ПустойПлан.xml";
const EMPTY_PREDEFINED: &str = "ChartsOfCalculationTypes/ПустойПлан/Ext/Predefined.xml";

fn set_code(name: &str, code: &str) -> Value {
    json!({"op":"predefinedItem.set","args":{"at":format!("{OWNER}.PredefinedItem.{name}"),"values":{"code":code}}})
}
fn codes_for(kind: &str) -> Vec<Value> {
    let (first, second) = if kind == "Number" {
        ("1", "2")
    } else {
        ("00001", "00002")
    };
    vec![set_code("ОкладПоДням", first), set_code("Запасной", second)]
}
impl Fixture {
    fn bytes(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.0.source_dir().join(relative)).unwrap()
    }
    fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) {
        let path = self.0.source_dir().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    fn start_with_numbers(&self) {
        let descriptor = String::from_utf8(self.bytes(DESCRIPTOR))
            .unwrap()
            .replace("<CodeType>String</CodeType>", "<CodeType>Number</CodeType>");
        self.write(DESCRIPTOR, descriptor);
        let predefined = String::from_utf8(self.bytes(PREDEFINED))
            .unwrap()
            .replace(
                "<Code>00001</Code>",
                "<Code xsi:type=\"xs:decimal\">1</Code>",
            )
            .replace(
                "<Code>00002</Code>",
                "<Code xsi:type=\"xs:decimal\">2</Code>",
            );
        self.write(PREDEFINED, predefined);
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

#[test]
fn calculation_chart_code_type_and_explicit_codes_use_the_final_state_in_either_order() {
    for from in ["String", "Number"] {
        let to = if from == "String" { "Number" } else { "String" };
        for properties_first in [false, true] {
            for split_families in [false, true] {
                let fixture = Fixture::new();
                if from == "Number" {
                    fixture.start_with_numbers();
                }
                let original_descriptor = fixture.bytes(DESCRIPTOR);
                let original_predefined = fixture.bytes(PREDEFINED);
                let before = tree_snapshot(&fixture.0.source_dir());
                let mut codes = codes_for(to);
                let mut ops = if properties_first {
                    let mut ops = vec![code_type(to)];
                    ops.append(&mut codes);
                    ops
                } else {
                    codes.push(code_type(to));
                    codes
                };
                if split_families {
                    ops.insert(1, json!({"op":"code.insert","args":{"at":format!("{OWNER}.Module.Object.Body"),"text":"// split metadata runs\n"}}));
                }
                let staged = fixture.plan(ops).unwrap_or_else(|error| panic!("{from}->{to}, props first {properties_first}, split {split_families}: {error:?}"));
                let expected_descriptor = String::from_utf8(original_descriptor).unwrap().replace(
                    &format!("<CodeType>{from}</CodeType>"),
                    &format!("<CodeType>{to}</CodeType>"),
                );
                assert_eq!(
                    fixture.postimage(&staged, DESCRIPTOR),
                    expected_descriptor.as_bytes()
                );
                let expected_predefined = if to == "Number" {
                    String::from_utf8(original_predefined)
                        .unwrap()
                        .replace(
                            "<Code>00001</Code>",
                            "<Code xsi:type=\"xs:decimal\">1</Code>",
                        )
                        .replace(
                            "<Code>00002</Code>",
                            "<Code xsi:type=\"xs:decimal\">2</Code>",
                        )
                } else {
                    String::from_utf8(original_predefined)
                        .unwrap()
                        .replace(
                            "<Code xsi:type=\"xs:decimal\">1</Code>",
                            "<Code>00001</Code>",
                        )
                        .replace(
                            "<Code xsi:type=\"xs:decimal\">2</Code>",
                            "<Code>00002</Code>",
                        )
                };
                assert_eq!(
                    fixture.postimage(&staged, PREDEFINED),
                    expected_predefined.as_bytes()
                );
                assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
            }
        }
    }
}

#[test]
fn calculation_chart_partial_code_migration_and_invalid_explicit_values_never_publish() {
    for from in ["String", "Number"] {
        let to = if from == "String" { "Number" } else { "String" };
        let fixture = Fixture::new();
        if from == "Number" {
            fixture.start_with_numbers();
        }
        let before = tree_snapshot(&fixture.0.source_dir());
        let error = fixture
            .plan(vec![codes_for(to).remove(0), code_type(to)])
            .expect_err("one unchanged incompatible code must refuse");
        assert!(error.to_string().contains("Predefined.xml"), "{error:?}");
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
    }
    let fixture = Fixture::new();
    let before = tree_snapshot(&fixture.0.source_dir());
    let error = fixture
        .plan(vec![
            set_code("ОкладПоДням", "not-a-number"),
            code_type("Number"),
        ])
        .expect_err("an explicit nondecimal code must refuse");
    assert!(error.to_string().contains("decimal"), "{error:?}");
    assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
}

#[test]
fn calculation_chart_empty_companion_and_unchanged_broken_companion_keep_noop_semantics() {
    let fixture = Fixture::new();
    let mut change = code_type("Number");
    change["args"]["at"] = json!(EMPTY_OWNER);
    let staged = fixture.plan(vec![change]).unwrap();
    assert!(staged
        .planned_changes()
        .iter()
        .all(|change| change.relative_path != std::path::Path::new(EMPTY_PREDEFINED)));
    assert!(!fixture.0.source_dir().join(EMPTY_PREDEFINED).exists());
    fixture.write(PREDEFINED, b"not XML: keep bytes on a no-op");
    let before = tree_snapshot(&fixture.0.source_dir());
    let staged = fixture.plan(vec![code_type("String")]).unwrap();
    assert!(staged.planned_changes().is_empty());
    assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
}

#[test]
fn calculation_chart_code_type_does_not_repair_missing_or_damaged_source_values() {
    for replacement in [
        "",
        "<CodeType/>",
        "<CodeType>Unknown</CodeType>",
        "<CodeType>String</CodeType><CodeType>Number</CodeType>",
        "<CodeType>String<Nested/></CodeType>",
    ] {
        let fixture = Fixture::new();
        let descriptor = String::from_utf8(fixture.bytes(EMPTY_DESCRIPTOR))
            .unwrap()
            .replace("<CodeType>String</CodeType>", replacement);
        fixture.write(EMPTY_DESCRIPTOR, descriptor);
        let before = tree_snapshot(&fixture.0.source_dir());
        let mut op = code_type("Number");
        op["args"]["at"] = json!(EMPTY_OWNER);
        assert!(
            fixture.plan(vec![op]).is_err(),
            "damaged preimage accepted: {replacement}"
        );
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
    }
}

#[test]
fn calculation_chart_saved_type_change_retains_descriptor_resource_and_absence_fences() {
    for changed in ["descriptor", "predefined", "absence"] {
        let fixture = Fixture::new();
        let ops = if changed == "absence" {
            let mut op = code_type("Number");
            op["args"]["at"] = json!(EMPTY_OWNER);
            vec![op]
        } else {
            let mut ops = codes_for("Number");
            ops.push(code_type("Number"));
            ops
        };
        let token = fixture.save(ops);
        match changed {
            "descriptor" => fixture.write(
                DESCRIPTOR,
                String::from_utf8(fixture.bytes(DESCRIPTOR))
                    .unwrap()
                    .replace("<Comment/>", "<Comment>concurrent edit</Comment>"),
            ),
            "predefined" => fixture.write(
                PREDEFINED,
                String::from_utf8(fixture.bytes(PREDEFINED))
                    .unwrap()
                    .replace("Оклад по дням", "concurrent edit"),
            ),
            "absence" => fixture.write(EMPTY_PREDEFINED, fixture.bytes(PREDEFINED)),
            _ => unreachable!(),
        }
        let before = tree_snapshot(&fixture.0.source_dir());
        assert!(
            fixture.execute(&token).is_err(),
            "{changed} fence must refuse"
        );
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
    }
}

#[test]
fn calculation_chart_type_change_before_removal_keeps_the_existing_dirty_state_barrier() {
    let fixture = Fixture::new();
    let owner = "main:ChartOfCalculationTypes.Transient";
    let before = tree_snapshot(&fixture.0.source_dir());
    let error = fixture.plan(vec![
        json!({"op":"object.create","args":{"values":{"kind":"ChartOfCalculationTypes","name":"Transient"}}}),
        json!({"op":"props.set","args":{"at":owner,"values":{"CodeType":"Number"}}}),
        json!({"op":"object.remove","args":{"at":owner}}),
    ]).expect_err("removal cannot inspect a source root after staged changes");
    assert_eq!(
        error.kind(),
        unica_source_contract::proposal::ApplyPlanErrorKind::InvalidState
    );
    assert!(error.to_string().contains("own apply call"));
    assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
}

#[test]
fn calculation_chart_new_object_predefined_uses_its_final_code_type() {
    let owner = "main:ChartOfCalculationTypes.Transient";
    let create = || json!({"op":"object.create","args":{"values":{"kind":"ChartOfCalculationTypes","name":"Transient"}}});
    let change = || json!({"op":"props.set","args":{"at":owner,"values":{"CodeType":"Number"}}});
    let fixture = Fixture::new();
    let staged = fixture.plan(vec![create(),
        json!({"op":"predefinedItem.add","args":{"at":owner,"items":[{"id":"29a82698-a123-4c75-a117-48831510cba9","name":"New","code":"7"}]}}),
        change(),
    ]).unwrap();
    let predefined = staged
        .planned_changes()
        .into_iter()
        .find(|change| {
            change.relative_path
                == std::path::Path::new("ChartsOfCalculationTypes/Transient/Ext/Predefined.xml")
        })
        .unwrap()
        .current
        .as_option()
        .unwrap();
    assert!(String::from_utf8(predefined)
        .unwrap()
        .contains("<Code xsi:type=\"xs:decimal\">7</Code>"));
}

#[test]
fn metadata_owned_code_type_context_resets_on_recreation_after_an_unchanged_intent() {
    use super::metadata::{parse_metadata_plan_operation, plan_metadata_batch_owned_for_test};
    use unica_source_contract::proposal::IndexedPlanOperation;
    let fixture = Fixture::new();
    fixture.write(
        EMPTY_DESCRIPTOR,
        String::from_utf8(fixture.bytes(EMPTY_DESCRIPTOR))
            .unwrap()
            .replace("<CodeType>String</CodeType>", "<CodeType>Number</CodeType>"),
    );
    let ops = [
        json!({"op":"props.set","args":{"at":EMPTY_OWNER,"values":{"CodeType":"Number"}}}),
        json!({"op":"object.remove","args":{"at":EMPTY_OWNER}}),
        json!({"op":"object.create","args":{"at":"main:Configuration","values":{"kind":"ChartOfCalculationTypes","name":"ПустойПлан"}}}),
        json!({"op":"predefinedItem.add","args":{"at":EMPTY_OWNER,"items":[{"id":"29a82698-a123-4c75-a117-48831510cba9","name":"New","code":"ABC"}]}}),
    ];
    let operations = ops
        .iter()
        .enumerate()
        .map(|(index, op)| {
            IndexedPlanOperation::new(
                index,
                parse_metadata_plan_operation(
                    op["op"].as_str().unwrap(),
                    &op["args"],
                    index,
                    &fixture.0.binding,
                )
                .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let before = tree_snapshot(&fixture.0.source_dir());
    let admission = fixture.0.admission();
    let authority = admission
        .metadata_planning_authority(&fixture.0.binding)
        .unwrap();
    let (staged, _) = plan_metadata_batch_owned_for_test(
        admission.staged_state().unwrap(),
        authority,
        &operations,
    )
    .unwrap();
    assert!(
        String::from_utf8(fixture.postimage(&staged, EMPTY_DESCRIPTOR))
            .unwrap()
            .contains("<CodeType>String</CodeType>")
    );
    let predefined = fixture.postimage(&staged, EMPTY_PREDEFINED);
    assert!(String::from_utf8(predefined)
        .unwrap()
        .contains("<Code>ABC</Code>"));
    assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
}

#[test]
fn calculation_chart_explicit_code_migration_preserves_unknown_extensions() {
    let fixture = Fixture::new();
    let payload =
        "<foreign:Payload xmlns:foreign=\"urn:keep\" marker=\"opaque\">untouched</foreign:Payload>";
    let predefined = String::from_utf8(fixture.bytes(PREDEFINED))
        .unwrap()
        .replace(
            "<Code>00001</Code>",
            "<Code xmlns:foreign=\"urn:keep\" foreign:marker=\"opaque\">00001</Code>",
        )
        .replacen("</Description>", &format!("</Description>{payload}"), 1);
    fixture.write(PREDEFINED, &predefined);
    let mut ops = codes_for("Number");
    ops.push(code_type("Number"));
    let staged = fixture.plan(ops).unwrap();
    let postimage = String::from_utf8(fixture.postimage(&staged, PREDEFINED)).unwrap();
    assert!(postimage.contains(payload));
    let document = roxmltree::Document::parse(postimage.trim_start_matches('\u{feff}')).unwrap();
    let code = document
        .descendants()
        .find(|node| {
            node.is_element() && node.tag_name().name() == "Code" && node.text() == Some("1")
        })
        .unwrap();
    assert_eq!(code.attribute(("urn:keep", "marker")), Some("opaque"));
    assert_eq!(
        code.attribute(("http://www.w3.org/2001/XMLSchema-instance", "type")),
        Some("xs:decimal")
    );
    assert_eq!(fixture.bytes(PREDEFINED), predefined.as_bytes());
}

#[test]
fn calculation_chart_deferred_validation_rejects_an_untouched_wrong_code_xsi_type() {
    let fixture = Fixture::new();
    fixture.start_with_numbers();
    fixture.write(
        PREDEFINED,
        String::from_utf8(fixture.bytes(PREDEFINED))
            .unwrap()
            .replace("xsi:type=\"xs:decimal\"", "xsi:type=\"xs:string\""),
    );
    let before = tree_snapshot(&fixture.0.source_dir());
    let error = fixture.plan(vec![code_type("Number"), json!({"op":"predefinedItem.set","args":{"at":format!("{OWNER}.PredefinedItem.Запасной"),"values":{"description":"changed"}}})]).expect_err("changing another field must not hide the wrong xsi:type");
    assert!(error.to_string().contains("xsi:type"), "{error:?}");
    assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
}

#[test]
fn calculation_chart_extension_code_type_requires_complete_local_composition() {
    use unica_source_contract::SourceSetKind;
    for local_predefined in [false, true] {
        let fixture = Fixture::with_kind(SourceSetKind::Extension);
        let descriptor = String::from_utf8(fixture.bytes(EMPTY_DESCRIPTOR)).unwrap()
            .replace("<Name>ПустойПлан</Name>", "<ObjectBelonging>Adopted</ObjectBelonging><Name>ПустойПлан</Name><ExtendedConfigurationObject>415c4609-54df-430a-b914-e33b6dc8ae6b</ExtendedConfigurationObject>");
        fixture.write(EMPTY_DESCRIPTOR, descriptor);
        if local_predefined {
            // A valid local numeric companion still says nothing about inherited codes.
            fixture.start_with_numbers();
            fixture.write(EMPTY_PREDEFINED, fixture.bytes(PREDEFINED));
        }
        let before = tree_snapshot(&fixture.0.source_dir());
        let operation =
            |value| json!({"op":"props.set","args":{"at":EMPTY_OWNER,"values":{"CodeType":value}}});
        let error = fixture
            .plan(vec![operation("Number")])
            .expect_err("inherited composition is not locally proven");
        assert_eq!(
            error.kind(),
            unica_source_contract::proposal::ApplyPlanErrorKind::InvalidSource
        );
        assert!(
            error.to_string().contains("inherited predefined codes"),
            "{error:?}"
        );
        let no_op = fixture.plan(vec![operation("String")]).unwrap();
        assert!(no_op.planned_changes().is_empty());
        assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
    }
    let fixture = Fixture::with_kind(SourceSetKind::Extension);
    let staged = fixture
        .plan(vec![
            json!({"op":"props.set","args":{"at":EMPTY_OWNER,"values":{"CodeType":"Number"}}}),
        ])
        .unwrap();
    assert!(
        String::from_utf8(fixture.postimage(&staged, EMPTY_DESCRIPTOR))
            .unwrap()
            .contains("<CodeType>Number</CodeType>")
    );
}

#[test]
fn calculation_chart_equivalent_code_type_preserves_lexical_bytes_without_reading_companion() {
    let fixture = Fixture::new();
    let descriptor = String::from_utf8(fixture.bytes(DESCRIPTOR))
        .unwrap()
        .replace(
            "<CodeType>String</CodeType>",
            "<CodeType>Str&#105;ng<!--keep--></CodeType>",
        );
    fixture.write(DESCRIPTOR, descriptor);
    fixture.write(PREDEFINED, b"unchanged unsupported companion");
    let before = tree_snapshot(&fixture.0.source_dir());
    let staged = fixture
        .plan(vec![code_type("String")])
        .expect("the current semantic CodeType must be a no-op");
    assert!(staged.planned_changes().is_empty());
    assert_eq!(tree_snapshot(&fixture.0.source_dir()), before);
}

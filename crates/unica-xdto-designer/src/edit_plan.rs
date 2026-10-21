//! Existing ordered XDTO image planning, with no source selection or publisher.
use crate::info::{addressed_location, classified_finding, model_error, writer_finding};
use crate::{
    encode_like,
    model::PackageModel,
    validation::{self, validate, ValidationDiff},
    writer,
};
use serde_json::{Map, Value};
use unica_xdto::{operations::parse_operations, results::*};

pub struct PlannedXdtoImage {
    pub after: Vec<u8>,
    pub data: XdtoEditData,
    pub no_op: bool,
}

pub struct PlanningFailure {
    pub error: String,
    pub data: Option<Box<XdtoEditData>>,
}
impl PlanningFailure {
    pub fn plain(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            data: None,
        }
    }
}
impl From<String> for PlanningFailure {
    fn from(error: String) -> Self {
        Self::plain(error)
    }
}

pub fn plan_xdto_edit_operations<I: XdtoIdentity>(
    package: &I,
    before: &[u8],
    text: String,
    before_model: &PackageModel,
    args: &Map<String, Value>,
) -> Result<PlannedXdtoImage, PlanningFailure> {
    let operations = parse_operations(args)?;
    // Operations apply in order against the accumulated text, so
    // element `i` sees what element `i - 1` produced; the file is written
    // once after the whole batch plans cleanly.
    let mut effects: Vec<XdtoOperationEffect> = Vec::new();
    let mut current_text = text;
    let mut step_baseline = validate(before_model);
    let partial_data = |effects: &[XdtoOperationEffect], package: &I, no_op: bool| XdtoEditData {
        source_set: package.source_set().to_string(),
        metadata_path: package.metadata_path().to_string(),
        location: addressed_location(package),
        no_op,
        effects: effects.to_vec(),
    };
    for (index, (operation, operation_args)) in operations.iter().enumerate() {
        let writer_plan = writer::plan(&current_text, operation_args, operation.writer_key())
            .map_err(|error| PlanningFailure {
                error: format!("operations[{index}]: {error}"),
                data: Some(Box::new(partial_data(&effects, package, false))),
            })?;
        let after_model =
            PackageModel::parse(&writer_plan.after).map_err(|error| PlanningFailure {
                error: format!("operations[{index}]: {}", model_error(error)),
                data: Some(Box::new(partial_data(&effects, package, false))),
            })?;
        let validation = ValidationDiff::between(&step_baseline, validate(&after_model));
        let blocked = writer_plan.blocks() || validation.blocks();
        let step_no_op = current_text == writer_plan.after && !blocked;
        debug_assert!(writer_plan.edits.len() <= 1);
        let change = writer_plan.edits.first().map(|edit| XdtoProjectedChange {
            kind: XdtoChangeKind::PackageTextEdit,
            location: addressed_location(package),
            body_byte_range: XdtoByteRange {
                start: edit.range.start,
                end: edit.range.end,
            },
            removed_byte_count: edit.range.end - edit.range.start,
            replacement_byte_count: edit.replacement.len(),
        });
        let mut findings = validation
            .findings
            .iter()
            .map(|finding| classified_finding(package, finding))
            .collect::<Vec<_>>();
        if let Some(finding) = &writer_plan.finding {
            findings.push(writer_finding(package, finding));
        }
        effects.push(XdtoOperationEffect {
            operation_index: index,
            op: *operation,
            no_op: step_no_op,
            change,
            findings,
        });
        if blocked {
            let mut codes = validation
                .findings
                .iter()
                .filter(|finding| finding.state == validation::FindingState::Introduced)
                .map(|finding| finding.code.as_str())
                .collect::<Vec<_>>();
            if let Some(finding) = writer_plan
                .finding
                .as_ref()
                .filter(|_| writer_plan.blocks())
            {
                codes.push(finding.code);
            }
            return Err(PlanningFailure {
                error: format!(
                    "operations[{index}]: xdto_validation_failed: introduced findings: {}",
                    codes.join(", ")
                ),
                data: Some(Box::new(partial_data(&effects, package, false))),
            });
        }
        step_baseline = validate(&after_model);
        current_text = writer_plan.after;
    }
    let after = encode_like(before, &current_text);
    let no_op = before == after;
    let data = partial_data(&effects, package, no_op);
    Ok(PlannedXdtoImage { after, data, no_op })
}

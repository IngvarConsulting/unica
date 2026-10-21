use crate::operations::MxlOperation;
use serde_json::{json, Value};
use unica_operation_contract::FormatValidator;
use unica_operation_contract::{object, text, OperationContract};

pub const MXL_MAX_ROW: i64 = 10_000;
pub const MXL_MAX_COLUMN: i64 = 1_000;

/// Preserve the existing integer spelling accepted at the apply boundary,
/// including leading zeros and a leading plus; hints and planner share it.
pub fn parse_mxl_cell_address(key: &str) -> Option<(i64, i64)> {
    let rest = key.strip_prefix('R')?;
    let (row, column) = rest.split_once('C')?;
    let row = row.parse::<i64>().ok()?;
    let column = column.parse::<i64>().ok()?;
    ((1..=MXL_MAX_ROW).contains(&row) && (1..=MXL_MAX_COLUMN).contains(&column))
        .then_some((row, column))
}

fn validate_mxl_cell(value: &Value) -> Result<(), String> {
    if value
        .as_str()
        .is_some_and(|key| parse_mxl_cell_address(key).is_some())
    {
        Ok(())
    } else {
        Err(format!(
            "expected area-local R<row>C<column>, up to R{MXL_MAX_ROW}C{MXL_MAX_COLUMN}"
        ))
    }
}

fn validate_exact_json_i64(_value: &Value) -> Result<(), String> {
    // The generic integer type check runs first and retains the exact JSON spelling rule.
    Ok(())
}

const MXL_FORMATS: &[FormatValidator] = &[
    FormatValidator {
        name: "unica-mxl-cell-address",
        validate: validate_mxl_cell,
    },
    FormatValidator {
        name: "unica-json-i64",
        validate: validate_exact_json_i64,
    },
];

pub fn mxl(operation: MxlOperation) -> OperationContract {
    let scalar =
        json!({"anyOf":[{"type":"string"},{"type":"number"},{"type":"boolean"},{"type":"null"}]});
    let cells = json!({"type":"object","minProperties":1,
        "propertyNames":{"type":"string","format":"unica-mxl-cell-address",
            "description":format!("R<row>C<column>, 1-based inside the area; row <= {MXL_MAX_ROW}, column <= {MXL_MAX_COLUMN}; leading zeros and plus are accepted")},
        "additionalProperties":scalar});
    let columns = json!({"type":"integer","format":"unica-json-i64","minimum":1,"maximum":i64::MAX,
        "description":"Positive JSON integer serialized without a decimal point or exponent, in the signed 64-bit range; 1 is accepted, 1.0 and 1e0 are refused."});
    let notes = "Read Template -> Area -> Body for ordered nonempty text cells; Body index is a reading ordinal, not RnCn. Parameter lists parameter names. The writer changes only the addressed cells, rows and areas; other content and bytes stay. A new cell has no own format. Refused before writing: a non-Rows or rowless area, a cell inside a merge it does not start, a cell holding a value or control, a cell that takes a parameter or template fill from its row, column, column-set or document format. Strings remain text; numbers/booleans are serialized as text; null is empty text and writes no new cell. Fonts, styles, merges and page properties are not public inputs. Create and execute template.add before addressing a new template.";
    let (schema, target, effect, example_args) = match operation {
        MxlOperation::Set => (
            object(&[("at",text(true)),("values",object(&[("cells",cells),("columns",columns)], &["cells"]))], &["values"]),
            "An existing named area of a SpreadsheetDocument Template: <source-set>:<Owner>.<name>.Template.<name>.Area.<area>. The address names the area; a missing area is refused, create it with area.add on the template.",
            "Update cells of the existing named Rows area. Addressed parameter/template cells become ordinary text. Unaddressed cells and areas remain. A row below the area grows the area only when nothing follows it. Width grows to max(existing width, written column, columns); it never shrinks.",
            json!({"values":{"columns":4,"cells":{"R1C1":"Item","R1C2":"Amount","R2C1":true,"R2C2":null}}}),
        ),
        MxlOperation::AreaAdd => (
            object(&[("at",text(true)),("items",json!({"type":"array","minItems":1,"items":object(&[("name",text(true)),("cells",cells),("columns",columns)], &["name","cells"])}))], &["items"]),
            "The existing SpreadsheetDocument Template itself: <source-set>:<Owner>.<name>.Template.<name>. Read its Area collection before choosing a new area name.",
            "Append new named Rows areas after every used row with their initial cells. An existing area name is refused; change it with mxl.set at its Area address. Width grows to max(existing width, written column, columns); it never shrinks.",
            json!({"items":[{"name":"Header","columns":4,"cells":{"R1C1":"Item","R1C2":"Amount"}}]}),
        ),
    };
    OperationContract {
        schema,
        target,
        effect,
        notes,
        example_args,
        help: REPORTS_HELP,
        format_validators: MXL_FORMATS,
    }
}

const REPORTS_HELP: &str = "references/use-cases/reports-printing.md";

use super::{object, text, OperationContract, MXL_MAX_COLUMN, MXL_MAX_ROW, REPORTS_HELP};
use crate::domain::mxl_operation::MxlOperation;
use serde_json::json;

impl OperationContract {
    pub(crate) fn mxl(operation: MxlOperation) -> Self {
        let scalar = json!({"anyOf":[{"type":"string"},{"type":"number"},{"type":"boolean"},{"type":"null"}]});
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
        Self {
            schema,
            target,
            effect,
            notes,
            example_args,
            help: REPORTS_HELP,
        }
    }
}

//! Contextual BSL argument shapes.
use crate::operations::CodeOperation;
use serde_json::{json, Value};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_operation_contract::{object, text, OperationContract};

fn contract(
    schema: Value,
    target: &'static str,
    effect: &'static str,
    notes: &'static str,
    example_args: Value,
    help: &'static str,
) -> OperationContract {
    OperationContract {
        schema,
        target,
        effect,
        notes,
        example_args,
        help,
        format_validators: &[],
    }
}

impl CodeOperation {
    pub fn contract(self, at: Option<&QualifiedAddress>) -> OperationContract {
        let terminal = at.and_then(|at| at.segments().last()).map(|s| s.kind());
        let method = terminal == Some(NodeKind::Method);
        let addressed = method || terminal == Some(NodeKind::Body);
        // The code parser accepts whitespace as source text and as an exact anchor.
        let source = json!({"type":"string","pattern":r"[\s\S]"});
        let mut fields = vec![("at", text(true)), ("text", source.clone())];
        let mut required = vec!["text"];
        if !addressed {
            fields.push(("selector", object(&[("anchor", source)], &["anchor"])));
        }
        if self == Self::Insert && (method || !addressed) {
            fields.push((
                "position",
                json!({"type":"string","enum":["before","after"]}),
            ));
        }
        if self == Self::Insert && method {
            required.push("position");
        }
        if self == Self::Replace && !addressed {
            required.push("selector");
        }
        let mut schema = object(&fields, &required);
        if self == Self::Insert && !addressed {
            schema["anyOf"] = json!([
                {"required":["selector","position"]},
                {"not":{"anyOf":[{"required":["selector"]},{"required":["position"]}]}}
            ]);
        }
        let example = match (self, method, addressed) {
            (Self::Insert, true, _) => json!({"text":"// inserted\n","position":"after"}),
            (Self::Replace, _, false) => {
                json!({"text":"// replacement\n","selector":{"anchor":"// existing anchor"}})
            }
            _ => json!({"text":"// sample\n"}),
        };
        contract(schema, self.applicability().shape(), "Insert or replace BSL source at the addressed module or projection.", "Module insertion without selector appends at the tail; an exact unique anchor inside one existing method requires before/after. Method insertion requires before/after. Addressed Body/Method replacement has no selector. Method.Body does not support insertion. Example anchors are templates: read the existing text before use. An args.at override changes the target; read Can at that target to obtain its contextual schema.", example, "skills/code-patch/SKILL.md")
    }
}

use crate::event::EventOperation;
use crate::event::FormCallType;
use serde_json::json;
use unica_operation_contract::{object, text, OperationContract};

pub fn event(operation: EventOperation) -> OperationContract {
    match operation {
        EventOperation::Implement => OperationContract {
            schema: object(
                &[
                    ("at", text(true)),
                    (
                        "callType",
                        json!({"type":"string","enum":FormCallType::ALL.map(FormCallType::as_str),
                            "description":"Only in an extension form."}),
                    ),
                ],
                &[],
            ),
            target: "An event node: a form or item event ….Form.<Form>[.Item.<Element>…].Event.<Event>, or an object module event <source-set>:<Kind>.<Name>.Event.<Event>.",
            effect: "Write the handler procedure into the module and, for a form event, its binding.",
            notes: "The event is named by the address; the handler name follows the platform convention.",
            example_args: json!({}),
            help: "skills/form-events/SKILL.md",
            format_validators: &[],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_contract_accepts_its_example_and_keeps_call_type_vocabulary() {
        let contract = EventOperation::Implement.contract();
        assert!(contract.normalize(&contract.example_args, "args").is_ok());
        assert_eq!(contract.schema["required"], json!([]));
        assert_eq!(
            contract.schema["properties"]["callType"]["enum"],
            json!(["Before", "After", "Override"])
        );
        assert_eq!(contract.schema["additionalProperties"], false);
        for call_type in FormCallType::ALL {
            assert!(contract
                .normalize(&json!({"callType":call_type.as_str()}), "args")
                .is_ok());
        }
        for call_type in ["Instead", "before", " Before", "After "] {
            let error = contract
                .normalize(&json!({"callType":call_type}), "ops[4].args")
                .unwrap_err();
            assert_eq!(error.path, "ops[4].args.callType");
        }
    }
}

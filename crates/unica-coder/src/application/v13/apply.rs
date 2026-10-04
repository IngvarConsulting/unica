use crate::domain::address::NodeKind;
use crate::domain::apply::{
    ApplyRequest, ApplyValidationError, OperationFamily, OperationRegistry,
};
use crate::domain::node_view::OperationRef;
use serde_json::{Map, Value};

pub(crate) fn parse_request(
    input: &Map<String, Value>,
    available_source_sets: &[&str],
) -> Result<ApplyRequest, ApplyValidationError> {
    ApplyRequest::parse(input, available_source_sets)
}

pub(crate) fn dispatch_family(operation: &str) -> Option<OperationFamily> {
    OperationRegistry::closed()
        .lookup(operation)
        .map(|descriptor| descriptor.family())
}

pub(crate) fn copyable_can(kind: NodeKind) -> Vec<OperationRef> {
    OperationRegistry::closed().copyable_skeletons(kind)
}

#[cfg(test)]
mod tests {
    use super::{copyable_can, dispatch_family, parse_request};
    use crate::application::tool_contracts::SurfaceRelease;
    use crate::application::v13::tool_catalog::catalog_for;
    use crate::domain::address::NodeKind;
    use crate::domain::apply::OperationFamily;
    use serde_json::{json, Value};

    #[test]
    fn planning_and_saved_plan_execution_have_disjoint_public_shapes() {
        let catalog = catalog_for(SurfaceRelease::V13).expect("v0.13 catalog");
        let schema = &catalog
            .tools
            .iter()
            .find(|tool| tool.name == "apply")
            .unwrap()
            .input_schema;
        let validator = jsonschema::validator_for(schema).unwrap();
        let plan = json!({"at": "main:Document.Order", "ops": [{"op": "props.set", "args": {"synonym": "Order"}}]});
        assert!(validator.is_valid(&plan));
        assert!(validator.is_valid(&json!({"executionToken": "saved-plan"})));
        for invalid in [
            json!({}),
            json!({"executionToken": ""}),
            json!({"executionToken": null}),
            json!({"executionToken": 7}),
            json!({"executionToken": "saved-plan", "unknown": true}),
            json!({"executionToken": "saved-plan", "at": "main:Document.Order"}),
            json!({"executionToken": "saved-plan", "ops": [{"op": "props.set"}]}),
            json!({"at": "main:Document.Order"}),
            json!({"ops": [{"op": "props.set"}]}),
            json!({"at": "main:Document.Order", "ops": []}),
        ] {
            assert!(
                !validator.is_valid(&invalid),
                "invalid public request: {invalid}"
            );
        }
        for (key, value) in [
            ("dryRun", json!(true)),
            ("dryRun", json!(false)),
            ("ifRev", json!("revision")),
            ("executionToken", json!("saved-plan")),
            ("executionToken", Value::Null),
            ("executionToken", json!("")),
            ("executionToken", json!(7)),
            ("unknown", json!(true)),
        ] {
            let mut mixed = plan.clone();
            mixed[key] = value;
            assert!(
                !validator.is_valid(&mixed),
                "invalid mixed request: {mixed}"
            );
        }
        let alternatives = [&schema["else"], &schema["then"]];
        assert_eq!(alternatives[0]["required"], json!(["at", "ops"]));
        assert_eq!(alternatives[1]["required"], json!(["executionToken"]));
        assert_eq!(
            alternatives[1]["properties"]["executionToken"]["minLength"],
            1
        );
        for alternative in &alternatives {
            assert_eq!(alternative["additionalProperties"], false);
            assert!(alternative["properties"].get("dryRun").is_none());
            assert!(alternative["properties"].get("ifRev").is_none());
        }
        assert!(alternatives[0]["properties"]
            .get("executionToken")
            .is_none());
        assert!(alternatives[1]["properties"].get("at").is_none());
        assert!(alternatives[1]["properties"].get("ops").is_none());
    }

    #[test]
    fn v13_apply_registry_projection_and_dispatch_share_the_closed_descriptor_source() {
        let request = parse_request(
            json!({
                "at": "main:Document.Order",
                "ops": [{"op": "props.set", "args": {"synonym": "Order"}}],
                "dryRun": true
            })
            .as_object()
            .unwrap(),
            &["main"],
        )
        .unwrap();
        assert_eq!(request.ops()[0].name(), "props.set");
        assert_eq!(
            dispatch_family("props.set"),
            Some(OperationFamily::Properties)
        );
        assert_eq!(dispatch_family("module.create"), None);
        assert_eq!(
            serde_json::to_value(copyable_can(NodeKind::Event))
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .find(|value| value["op"] == "event.implement")
                .cloned(),
            Some(json!({"op": "event.implement", "args": {"at": ""}}))
        );
    }
}

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
    use crate::domain::address::NodeKind;
    use crate::domain::apply::OperationFamily;
    use serde_json::json;

    #[test]
    fn v13_apply_registry_projection_and_dispatch_share_the_closed_descriptor_source() {
        let request = parse_request(
            json!({
                "at": "main:Document.Order",
                "ops": [{"op": "props.set", "args": {"synonym": "Order"}}]
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
            Some(json!({"op": "event.implement"}))
        );
    }
}

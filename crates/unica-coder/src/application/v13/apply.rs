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

    /// Три входа режима: опущенный, `false` и `true`.
    ///
    /// Опущенный равен `false`, поэтому забора требуют два входа из трёх, и
    /// опубликованная схема объявляет это условием `if`/`then`, а не только
    /// словами в описании поля.
    #[test]
    fn the_fence_is_required_by_the_mode_and_the_schema_says_so() {
        let request = |mode: Option<bool>, fence: bool| {
            let mut arguments = json!({
                "at": "main:Document.Order",
                "ops": [{"op": "props.set", "args": {"values": {"Comment": "x"}}}],
            });
            if let Some(mode) = mode {
                arguments["dryRun"] = Value::Bool(mode);
            }
            if fence {
                arguments["ifRev"] = Value::String(
                    "unica-source-sha256-v1:1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                );
            }
            parse_request(arguments.as_object().unwrap(), &["main"])
        };

        assert!(
            request(Some(true), false).is_ok(),
            "предпросмотр без забора"
        );
        assert!(request(Some(true), true).is_ok(), "предпросмотр с забором");
        assert!(request(Some(false), true).is_ok(), "применение с забором");
        for mode in [None, Some(false)] {
            let error = request(mode, false).expect_err("применение без забора");
            assert_eq!(error.location(), "ifRev", "{mode:?}");
        }

        let catalog = catalog_for(SurfaceRelease::V13).expect("v0.13 catalog");
        let fence = &catalog
            .tools
            .iter()
            .find(|tool| tool.name == "apply")
            .expect("apply is published")
            .input_schema;
        assert_eq!(fence["if"]["properties"]["dryRun"]["const"], json!(false));
        assert_eq!(fence["then"]["required"], json!(["ifRev"]));
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

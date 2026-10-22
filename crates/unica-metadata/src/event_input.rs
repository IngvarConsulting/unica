//! Existing event aliases and strict typed operation input.
use crate::event::{EventOperation, FormCallType};
use unica_operation_contract::ArgumentShape;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventImplementArgs {
    pub at: unica_address::address::QualifiedAddress,
    pub call_type: Option<FormCallType>,
}
/// Object and manager events by their Russian names, as developers write them.
const RUSSIAN_EVENT_NAMES: &[(&str, &str)] = &[
    ("ПередЗаписью", "BeforeWrite"),
    ("ПриЗаписи", "OnWrite"),
    ("ПередУдалением", "BeforeDelete"),
    ("ОбработкаЗаполнения", "Filling"),
    ("ОбработкаПроверкиЗаполнения", "FillCheckProcessing"),
    ("ПриКопировании", "OnCopy"),
    ("ПриУстановкеНовогоНомера", "OnSetNewNumber"),
    ("ПриУстановкеНовогоКода", "OnSetNewCode"),
    ("ОбработкаПроведения", "Posting"),
    ("ОбработкаУдаленияПроведения", "UndoPosting"),
    ("ОбработкаПолученияДанныхВыбора", "ChoiceDataGetProcessing"),
    ("ОбработкаПолученияФормы", "FormGetProcessing"),
    (
        "ОбработкаПолученияПолейПредставления",
        "PresentationFieldsGetProcessing",
    ),
    (
        "ОбработкаПолученияПредставления",
        "PresentationGetProcessing",
    ),
    (
        "ОбработкаФормированияИзВерсииИсторииДанных",
        "GenerateFromDataHistoryVersionProcessing",
    ),
    (
        "ОбработкаПослеЗаписиВерсийИсторииДанных",
        "AfterWriteDataHistoryVersionsProcessing",
    ),
];

const MANAGER_EVENTS: &[&str] = &[
    "ChoiceDataGetProcessing",
    "FormGetProcessing",
    "PresentationFieldsGetProcessing",
    "PresentationGetProcessing",
    "AfterWriteDataHistoryVersionsProcessing",
];

/// `Owner.Name.Event.ПриЗаписи` is how a developer names an object event; the
/// event planner addresses the module that hosts it
/// (`Owner.Name.Module.Object.Event.OnWrite`). Both spellings are accepted:
/// the module role follows the owner kind and the event, and Russian event
/// names map onto the registry. Anything else passes through unchanged.
fn canonical_event_args(args: serde_json::Value) -> serde_json::Value {
    use unica_address::address::{NodeKind, QualifiedAddress};
    let Some(raw) = args.get("at").and_then(serde_json::Value::as_str) else {
        return args;
    };
    let Ok(address) = QualifiedAddress::parse(raw) else {
        return args;
    };
    let segments = address.segments();
    let english = |name: &str| -> String {
        RUSSIAN_EVENT_NAMES
            .iter()
            .find(|(russian, _)| *russian == name)
            .map_or_else(|| name.to_string(), |(_, english)| (*english).to_string())
    };
    let rewritten = match segments {
        [owner, event] if event.kind() == NodeKind::Event => {
            let (Some(owner_name), Some(event_name)) = (owner.name(), event.name()) else {
                return args;
            };
            let event_name = english(event_name);
            let role = if MANAGER_EVENTS.contains(&event_name.as_str()) {
                "Manager"
            } else {
                match owner.kind() {
                    NodeKind::InformationRegister
                    | NodeKind::AccumulationRegister
                    | NodeKind::AccountingRegister
                    | NodeKind::CalculationRegister => "RecordSet",
                    NodeKind::Constant => "ValueManager",
                    _ => "Object",
                }
            };
            format!(
                "{}:{}.{owner_name}.Module.{role}.Event.{event_name}",
                address.source_set(),
                owner.kind().as_str()
            )
        }
        [owner, module, role, event]
            if module.kind() == NodeKind::Module && event.kind() == NodeKind::Event =>
        {
            let (Some(owner_name), Some(role_name), Some(event_name)) = (
                owner.name(),
                role.name().or(Some(role.kind().as_str())),
                event.name(),
            ) else {
                return args;
            };
            format!(
                "{}:{}.{owner_name}.Module.{role_name}.Event.{}",
                address.source_set(),
                owner.kind().as_str(),
                english(event_name)
            )
        }
        _ => return args,
    };
    let mut args = args;
    if let Some(object) = args.as_object_mut() {
        object.insert("at".to_string(), serde_json::Value::String(rewritten));
    }
    args
}

pub fn parse_event_operation(
    operation: EventOperation,
    args: serde_json::Value,
    path: &str,
) -> Result<EventImplementArgs, ApplyPlanError> {
    match operation {
        EventOperation::Implement => {
            let contract = operation.contract();
            parse_event_implement_args_in(
                &canonical_event_args(args),
                path,
                ArgumentShape::new(&contract.schema),
            )
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn parse_event_implement_args(
    value: &serde_json::Value,
    path: &str,
) -> Result<EventImplementArgs, ApplyPlanError> {
    parse_event_operation(EventOperation::Implement, value.clone(), path)
}

fn parse_event_implement_args_in(
    value: &serde_json::Value,
    path: &str,
    shape: ArgumentShape<'_>,
) -> Result<EventImplementArgs, ApplyPlanError> {
    let object = value.as_object().ok_or_else(|| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "event args must be an object")
            .at_path(path)
    })?;
    if let Some(unknown) = object.keys().find(|key| shape.property(key).is_none()) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("unknown event argument `{unknown}`"),
        )
        .at_path(format!("{path}.{unknown}")));
    }
    let at_path = format!("{path}.at");
    let at = object
        .get("at")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "at must be a string")
                .at_path(&at_path)
        })?;
    let at = unica_address::address::QualifiedAddress::parse(at).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.to_string()).at_path(&at_path)
    })?;
    check_event_input_field(shape, "at", &object["at"], &at_path)?;
    let call_type = match object.get("callType") {
        None => None,
        Some(value) => {
            let call_path = format!("{path}.callType");
            let raw = value.as_str().ok_or_else(|| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "callType must be a string")
                    .at_path(&call_path)
            })?;
            let call_type = FormCallType::from_xml(raw).ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    "callType must be Before, After, or Override",
                )
                .at_path(&call_path)
            })?;
            check_event_input_field(shape, "callType", value, &call_path)?;
            Some(call_type)
        }
    };
    Ok(EventImplementArgs { at, call_type })
}

fn check_event_input_field(
    shape: ArgumentShape<'_>,
    name: &str,
    value: &serde_json::Value,
    path: &str,
) -> Result<(), ApplyPlanError> {
    shape
        .property(name)
        .expect("known Event input field")
        .validate_field(value, path)
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(error.path)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_event_decoder_keeps_compound_refusal_order_and_operation_index() {
        let at = "main:Catalog.Products.Form.Main.Event.OnOpen";
        for (args, path, message) in [
            (
                serde_json::json!({"extra":true,"at":false,"callType":false}),
                "ops[4].args.extra",
                "unknown event argument `extra`",
            ),
            (
                serde_json::json!({"at":false,"callType":false}),
                "ops[4].args.at",
                "at must be a string",
            ),
            (
                serde_json::json!({"at":at,"callType":false}),
                "ops[4].args.callType",
                "callType must be a string",
            ),
            (
                serde_json::json!({"at":at,"callType":"Instead"}),
                "ops[4].args.callType",
                "callType must be Before, After, or Override",
            ),
        ] {
            let error = parse_event_operation(EventOperation::Implement, args, "ops[4].args")
                .expect_err("the earlier conversion must retain its diagnostic");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
            assert_eq!(error.path(), Some(path));
            assert_eq!(error.to_string(), message);
        }
        let error = parse_event_operation(
            EventOperation::Implement,
            serde_json::json!({"at":"not an address","callType":false}),
            "ops[6].args",
        )
        .unwrap_err();
        assert_eq!(error.path(), Some("ops[6].args.at"));
    }

    #[test]
    fn typed_event_entry_keeps_platform_aliases_and_consumes_shared_field_shape() {
        for (at, expected) in [
            (
                "main:Catalog.Products.Event.ПриЗаписи",
                "main:Catalog.Products.Module.Object.Event.OnWrite",
            ),
            (
                "main:Catalog.Products.Event.ОбработкаПолученияФормы",
                "main:Catalog.Products.Module.Manager.Event.FormGetProcessing",
            ),
            (
                "main:InformationRegister.Prices.Event.ПриЗаписи",
                "main:InformationRegister.Prices.Module.RecordSet.Event.OnWrite",
            ),
            (
                // The existing adapter leaves this explicit module spelling unchanged.
                "main:Catalog.Products.Module.Object.Event.ПередЗаписью",
                "main:Catalog.Products.Module.Object.Event.ПередЗаписью",
            ),
        ] {
            let parsed = parse_event_operation(
                EventOperation::Implement,
                serde_json::json!({"at":at}),
                "ops[2].args",
            )
            .unwrap();
            assert_eq!(parsed.at.to_string(), expected);
        }

        let args = serde_json::json!({
            "at":"ext:Catalog.Products.Form.Main.Event.OnOpen",
            "callType":"After",
        });
        let parsed =
            parse_event_operation(EventOperation::Implement, args.clone(), "ops[3].args").unwrap();
        assert_eq!(parsed.call_type, Some(FormCallType::After));
        let mut contract = EventOperation::Implement.contract();
        contract.schema["properties"]["callType"]["type"] = serde_json::json!("boolean");
        let error = parse_event_implement_args_in(
            &args,
            "ops[3].args",
            ArgumentShape::new(&contract.schema),
        )
        .expect_err("a changed shared field type must reach the productive decoder");
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[3].args.callType"));
    }
}

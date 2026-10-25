//! Subsystem and interface argument shapes shared by Can and staged decoding.
use crate::{interface_operations::InterfaceOperation, subsystem_operations::SubsystemOperation};
use serde_json::{json, Value};
use unica_operation_contract::{object, text, OperationContract};

fn array(items: Value) -> Value {
    json!({"type":"array","minItems":1,"items":items})
}

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

impl SubsystemOperation {
    pub fn contract(self) -> OperationContract {
        let (schema, example) = match self {
            Self::Create => (
                object(
                    &[
                        ("at", text(true)),
                        (
                            "values",
                            object(
                                &[(
                                    "name",
                                    json!({"type":"string","pattern":r"^\s*[_A-Za-zА-Яа-яЁё][_A-Za-zА-Яа-яЁё0-9]*\s*$"}),
                                )],
                                &["name"],
                            ),
                        ),
                    ],
                    &["values"],
                ),
                json!({"values":{"name":"Sample"}}),
            ),
            Self::ChildRemove => (object(&[("at", text(true))], &[]), json!({})),
            Self::ContentAdd | Self::ContentRemove | Self::ChildAdd => {
                let key = if self == Self::ChildAdd {
                    "name"
                } else {
                    "object"
                };
                let entry = object(&[(key, text(true))], &[key]);
                let mut schema = object(
                    &[
                        ("at", text(true)),
                        ("items", array(json!({"anyOf":[text(true), entry.clone()]}))),
                        ("values", entry),
                    ],
                    &[],
                );
                schema["anyOf"] = json!([{"required":["items"]},{"required":["values"]}]);
                // Both lists at once are refused rather than silently resolved.
                schema["not"] = json!({"required":["items","values"]});
                (
                    schema,
                    if self == Self::ChildAdd {
                        json!({"items":[{"name":"Sample"}]})
                    } else {
                        json!({"items":[{"object":"Catalog.Sample"}]})
                    },
                )
            }
        };
        contract(schema, self.applicability().shape(), "Create a subsystem, edit its content or add/remove an addressed child.", "The address selects the owner (or the child for removal). items accepts strings or keyed objects; values accepts one entry. Pass one of them: both together are refused. Choose existing metadata references and unused child names through view.", example, "skills/subsystem-edit/SKILL.md")
    }
}

impl InterfaceOperation {
    pub fn contract(self) -> OperationContract {
        let (schema, example) = match self {
            Self::VisibilitySet => (
                object(
                    &[
                        ("at", text(true)),
                        (
                            "items",
                            array(object(
                                &[
                                    ("command", text(true)),
                                    ("visible", json!({"type":"boolean"})),
                                ],
                                &["command", "visible"],
                            )),
                        ),
                    ],
                    &["items"],
                ),
                json!({"items":[{"command":"Catalog.Sample.StandardCommand.OpenList","visible":true}]}),
            ),
            Self::PlacementSet => {
                let mut entry = object(
                    &[
                        ("command", text(true)),
                        ("group", text(false)),
                        ("placement", text(false)),
                    ],
                    &["command"],
                );
                entry["anyOf"] = json!([{"required":["group"]},{"required":["placement"]}]);
                (
                    object(&[("at", text(true)), ("items", array(entry))], &["items"]),
                    json!({"items":[{"command":"Catalog.Sample.StandardCommand.OpenList","group":"NavigationPanelImportant"}]}),
                )
            }
            Self::CommandOrderSet | Self::GroupOrderSet | Self::SubsystemOrderSet => {
                let (key, name) = match self {
                    Self::CommandOrderSet => {
                        ("commands", "Catalog.Sample.StandardCommand.OpenList")
                    }
                    Self::GroupOrderSet => ("groups", "NavigationPanelImportant"),
                    _ => ("subsystems", "Subsystem.Sample"),
                };
                (
                    object(
                        &[
                            ("at", text(true)),
                            ("values", object(&[(key, array(text(true)))], &[key])),
                        ],
                        &["values"],
                    ),
                    json!({"values":{key:[name]}}),
                )
            }
        };
        contract(schema, self.applicability().shape(), "Edit command visibility, placement or the full ordering of the addressed interface section.", "Read actual command/group/subsystem names before preview. Orders are complete nonempty sequences, not partial additions. Placement supplies group and/or placement.", example, "skills/interface-edit/SKILL.md")
    }
}

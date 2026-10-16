use super::apply::{OperationApplicability, OperationSkeleton};
use super::operation_contract::{object, text, OperationContract};
use serde_json::json;

const ROLE_HELP: &str = "skills/role-edit/SKILL.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoleOperation {
    Create,
    RightAdd,
    RightSet,
}

impl RoleOperation {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Create => "role.create",
            Self::RightAdd => "right.add",
            Self::RightSet => "right.set",
        }
    }

    pub(super) const fn applicability(self) -> OperationApplicability {
        match self {
            Self::Create => OperationApplicability::Configuration,
            Self::RightAdd => OperationApplicability::Role,
            Self::RightSet => OperationApplicability::RoleRight,
        }
    }

    pub(super) const fn skeleton(self) -> OperationSkeleton {
        match self {
            Self::Create => OperationSkeleton::Values,
            Self::RightAdd => OperationSkeleton::Items,
            Self::RightSet => OperationSkeleton::Values,
        }
    }

    pub(crate) const fn content_resource(self) -> Option<&'static str> {
        match self {
            Self::Create => None,
            Self::RightAdd | Self::RightSet => Some("Rights.xml"),
        }
    }

    pub(crate) fn contract(self) -> OperationContract {
        let (schema, target, effect, notes, example_args) = match self {
            Self::Create => (
                object(
                    &[
                        ("at", text(true)),
                        (
                            "values",
                            object(
                                &[(
                                    "name",
                                    json!({
                                        "type":"string",
                                        "pattern":r"^\s*[_A-Za-zА-Яа-яЁё][_A-Za-zА-Яа-яЁё0-9]*\s*$",
                                        "description":"A new 1C role identifier; surrounding whitespace is trimmed."
                                    }),
                                )],
                                &["name"],
                            ),
                        ),
                    ],
                    &["values"],
                ),
                "The source-set configuration root: <source-set>:Configuration.",
                "Create a role descriptor, its empty Rights.xml and its registration in Configuration.xml.",
                "The name must be unused. Add initial rights with right.add at Role.<Name> in the same batch. Preview returns an execution token; execute it separately to write the source.",
                json!({"values":{"name":"ContractRole"}}),
            ),
            Self::RightAdd => (
                object(
                    &[
                        ("at", text(true)),
                        (
                            "items",
                            json!({"type":"array","minItems":1,"items":object(&[
                                ("object",json!({"type":"string","pattern":r"^[^.\s]+\.[^.\s]+(\.[^.\s]+\.[^.\s]+)*$",
                                    "description":"A metadata object `Kind.Name` (or `Configuration.Name`), or a nested one such as `Kind.Name.Command.C`, without a rights entry in the role yet."})),
                                ("rights",json!({"type":"object","minProperties":1,"additionalProperties":{"type":"boolean"},
                                    "description":"Initial rights by platform name, e.g. Read, View, Use; values different from the role default for new objects are stored."})),
                            ], &["object","rights"])}),
                        ),
                    ],
                    &["items"],
                ),
                "The existing role itself: <source-set>:Role.<Name>. Read its Right branch before choosing objects without an entry.",
                "Create rights entries for objects the role does not list yet, each with its initial rights. An object that already has an entry is refused; change it with right.set at Role.<Name>.Right.<Kind>_<Name>.",
                "An entry whose rights all equal the role default would stay empty and is refused. Restrict records with right.set on the new entry in the same batch. A nested object (command, attribute) is one entry whose further dots are escaped: Role.<Name>.Right.<Kind>_<Name>\\.Command\\.<C>.",
                json!({"items":[{"object":"Catalog.Sample","rights":{"Read":true,"View":true}}]}),
            ),
            Self::RightSet => {
                let mut values = object(
                    &[
                        ("right", text(true)),
                        ("value", json!({"type":"boolean"})),
                        ("rls", text(true)),
                    ],
                    &["right"],
                );
                values["anyOf"] = json!([{"required":["value"]},{"required":["rls"]}]);
                (
                    object(&[("at", text(true)), ("values", values)], &["values"]),
                    "An existing rights entry by its canonical name: <source-set>:Role.<Name>.Right.<Kind>_<Name>. The address names the object; a missing entry is refused, create it with right.add on the role.",
                    "Set one right of the addressed entry and/or its record restriction. A value equal to the role default removes the right with its restriction; an emptied entry is removed.",
                    "rls alone also grants the right. Restriction templates are preserved but not created.",
                    json!({"values":{"right":"Read","value":true}}),
                )
            }
        };
        OperationContract {
            schema,
            target,
            effect,
            notes,
            example_args,
            help: ROLE_HELP,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RoleOperation;
    use serde_json::json;

    #[test]
    fn normalization_preserves_creation_names_and_explicit_right_payloads() {
        for (operation, args) in [
            (
                RoleOperation::Create,
                json!({"values":{"name":"\tЧитатель\r\n"}}),
            ),
            (
                RoleOperation::RightAdd,
                json!({"items":[{"object":"DataProcessor.A.Command.B","rights":{"Use":true,"Read":false}}]}),
            ),
            (
                RoleOperation::RightSet,
                json!({"values":{"right":"Read","rls":"ГДЕ ИСТИНА"}}),
            ),
        ] {
            let contract = operation.contract();
            let validator = jsonschema::validator_for(&contract.schema).unwrap();
            assert!(validator.is_valid(&args), "{}: {args}", operation.name());
            assert_eq!(
                contract.normalize(&args, "args").unwrap(),
                args,
                "{} must preserve the accepted payload",
                operation.name()
            );
        }
    }
}

//! Cross-family contract integration coverage; implementations live with families.
use serde_json::{json, Value};
use unica_dcs::contracts::DcsContractExt;
use unica_forms::contracts::form_move;
use unica_metadata::contracts::*;

use unica_operation_contract::OperationContract;

use unica_templates::contracts::parse_mxl_cell_address;
#[cfg(test)]
mod tests {
    use super::*;

    use unica_operations::OperationFamily;
    use unica_roles::operations::RoleOperation;

    fn dcs_contract(name: &str) -> Option<OperationContract> {
        unica_dcs::operations::Primitive::named(name)
            .map(unica_dcs::operations::Primitive::contract)
    }

    #[test]
    fn every_dictionary_publishes_each_property_alias_group_once() {
        use unica_metadata::descriptor_operations::{PropertiesOperation, PropertiesSite};
        fn compare(internal: &Value, published: &Value, groups: &mut usize) {
            if let Some(aliases) = internal["x-unica-alias-groups"].as_array() {
                for group in aliases {
                    let names = group.as_array().unwrap();
                    let canonical = names[0].as_str().unwrap();
                    assert_eq!(
                        published["properties"][canonical],
                        internal["properties"][canonical]
                    );
                    for alias in &names[1..] {
                        assert!(
                            published["properties"]
                                .get(alias.as_str().unwrap())
                                .is_none(),
                            "duplicate published alias {alias}: {published}"
                        );
                    }
                    *groups += 1;
                }
                assert!(published.get("x-unica-alias-groups").is_none());
            }
            for key in [
                "properties",
                "patternProperties",
                "$defs",
                "definitions",
                "dependentSchemas",
            ] {
                if let Some(children) = internal[key].as_object() {
                    for (name, child) in children {
                        compare(child, &published[key][name], groups);
                    }
                }
            }
            for key in [
                "items",
                "additionalProperties",
                "contains",
                "not",
                "if",
                "then",
                "else",
                "propertyNames",
            ] {
                if internal[key].is_object() {
                    compare(&internal[key], &published[key], groups);
                }
            }
            for key in ["oneOf", "anyOf", "allOf", "prefixItems"] {
                if let Some(children) = internal[key].as_array() {
                    for (index, child) in children.iter().enumerate() {
                        compare(child, &published[key][index], groups);
                    }
                }
            }
        }
        let mut groups = 0;
        let registry = crate::domain::apply::operation_registry();
        for descriptor in registry.descriptors() {
            if let Some(contract) = descriptor.argument_contract() {
                compare(
                    &contract.schema,
                    &contract.details()["argsSchema"],
                    &mut groups,
                );
            }
        }
        let sites = [
            PropertiesSite::Configuration { extension: false },
            PropertiesSite::Configuration { extension: true },
            PropertiesSite::Role,
            PropertiesSite::Subsystem,
        ]
        .into_iter()
        .chain(
            unica_metadata::MetadataKind::ALL
                .iter()
                .copied()
                .map(PropertiesSite::Metadata),
        );
        for site in sites {
            let contract = PropertiesOperation::Set.contract(site);
            let published = contract.details()["argsSchema"].clone();
            compare(&contract.schema, &published, &mut groups);
            assert!(
                jsonschema::validator_for(&published)
                    .unwrap()
                    .is_valid(&contract.example_args),
                "{site:?}"
            );
            let validator = jsonschema::validator_for(&published).unwrap();
            for field in site.descriptor_fields() {
                let value = match field.kind() {
                    unica_metadata::descriptor_operations::DescriptorPropertyKind::String => {
                        json!("")
                    }
                    unica_metadata::descriptor_operations::DescriptorPropertyKind::Boolean => {
                        json!(false)
                    }
                    unica_metadata::descriptor_operations::DescriptorPropertyKind::Enum => {
                        json!(field.allowed_values()[0])
                    }
                };
                let names = field.names();
                let canonical = json!({"values":{names[0]:value}});
                let alias = json!({"values":{names[1]:value}});
                assert!(validator.is_valid(&canonical));
                assert!(
                    !validator.is_valid(&alias),
                    "aliases are retained only for compatibility"
                );
                let decode = |args: &Value| {
                    unica_metadata::property_input::decode_properties(
                        PropertiesOperation::Set,
                        site,
                        args,
                        "args",
                    )
                };
                assert_eq!(decode(&canonical).unwrap(), decode(&alias).unwrap());
            }
        }
        assert!(
            groups > 20,
            "the root and descriptor profiles must be visited"
        );
    }

    #[test]
    fn typed_metadata_contracts_cover_each_registered_identity_and_site() {
        let sites = std::iter::once(None).chain(unica_metadata::MetadataKind::ALL.iter().flat_map(
            |owner| {
                [false, true].map(|in_section| {
                    Some(MetadataSite {
                        owner: *owner,
                        in_section,
                    })
                })
            },
        ));
        for site in sites {
            for descriptor in crate::domain::apply::operation_registry()
                .descriptors()
                .iter()
                .filter(|descriptor| descriptor.family() == OperationFamily::Metadata)
            {
                let operation = descriptor
                    .operation::<unica_metadata::apply_operations::MetadataOperation>()
                    .expect("every Metadata registration is typed");
                let contract = operation.contract_at(site);
                assert_eq!(contract.schema["type"], "object", "{}", operation.name());
                assert_eq!(contract.schema["additionalProperties"], false);
                assert_eq!(contract.schema["properties"]["at"]["type"], "string");
                assert!(descriptor.argument_contract().is_some());
            }
        }
    }

    #[test]
    fn mxl_and_template_contracts_cover_examples_and_keep_legacy_value_grammar() {
        let registry = crate::domain::apply::operation_registry();
        for name in ["mxl.set", "area.add", "template.add"] {
            let descriptor = registry
                .descriptors()
                .iter()
                .find(|d| d.name() == name)
                .unwrap();
            let contract = descriptor.argument_contract().expect("shared contract");
            assert!(contract.normalize(&contract.example_args, "args").is_ok());
        }
    }

    /// Контракт прав публикует ту же форму, что принимает планировщик
    /// (#1354): объект — только `Kind.Name`, право — значение или
    /// ограничение, объект записи прав не аргумент `right.set`.
    #[test]
    fn role_contracts_publish_the_entry_shape() {
        let registry = crate::domain::apply::operation_registry();
        for descriptor in registry
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.family() == OperationFamily::Role)
        {
            let op = descriptor.name();
            let contract = descriptor
                .argument_contract()
                .expect("every implemented Role operation publishes its argument contract");
            assert!(
                contract.normalize(&contract.example_args, "args").is_ok(),
                "{op}"
            );
        }
        let create = RoleOperation::Create.contract();
        for name in [
            "Reader",
            "МенеджерПродаж",
            "_Reader2",
            " Reader ",
            "\tЧитатель\r\n",
            "\u{a0}Ёж\u{a0}",
        ] {
            assert!(
                create
                    .normalize(&json!({"values":{"name":name}}), "args")
                    .is_ok(),
                "{name:?}"
            );
        }
        for values in [
            json!({}),
            json!({"name":""}),
            json!({"name":"  "}),
            json!({"name":1}),
            json!({"name":null}),
            json!({"name":"1Reader"}),
            json!({"name":"Role.Reader"}),
            json!({"name":"Reader/Other"}),
            json!({"name":"Reader", "synonym":"Ignored"}),
        ] {
            assert!(
                create.normalize(&json!({"values":values}), "args").is_err(),
                "{values}"
            );
        }
        assert!(create.example_args.get("at").is_none());
        let add = RoleOperation::RightAdd.contract();
        for (args, path) in [
            (json!({"items":[]}), "args.items"),
            (
                json!({"items":[{"object":"Catalog.A","rights":{}}]}),
                "args.items[0].rights",
            ),
            (
                json!({"items":[{"object":"Catalog.A","rights":{"Read":"yes"}}]}),
                "args.items[0].rights.Read",
            ),
            (
                json!({"items":[{"object":"DataProcessor.A.Command","rights":{"Use":true}}]}),
                "args.items[0].object",
            ),
            (
                json!({"items":[{"object":"Catalog.A","rights":{"Read":true},"rls":"x"}]}),
                "args.items[0].rls",
            ),
        ] {
            let error = add.normalize(&args, "args").unwrap_err();
            assert_eq!(error.path, path, "{args}");
        }
        // Вложенный объект — одна запись роли (#1356).
        assert!(add
            .normalize(
                &json!({"items":[{"object":"DataProcessor.A.Command.B","rights":{"Use":true}}]}),
                "args"
            )
            .is_ok());
        let set = RoleOperation::RightSet.contract();
        assert!(set
            .normalize(
                &json!({"values":{"right":"Read","rls":"ГДЕ ИСТИНА"}}),
                "args"
            )
            .is_ok());
        for args in [
            json!({"values":{"right":"Read"}}),
            json!({"values":{"object":"Catalog.A","right":"Read","value":true}}),
            json!({"values":{"right":"Read","value":"true"}}),
        ] {
            assert!(set.normalize(&args, "args").is_err(), "{args}");
        }
    }

    #[test]
    fn mxl_contract_checks_dynamic_keys_scalars_and_exact_integer_values() {
        let contract = unica_templates::operations::MxlOperation::Set.contract();
        for key in ["R1C1", "R01C01", "R+1C+1", "R10000C1000"] {
            assert!(
                contract
                    .normalize(
                        &json!({"values":{"cells":{key:null},"columns":1001}}),
                        "args"
                    )
                    .is_ok(),
                "{key}"
            );
        }
        for values in [
            json!({"cells":[]}),
            json!({"cells":{}}),
            json!({"cells":{"R0C1":"x"}}),
            json!({"cells":{"R10001C1":"x"}}),
            json!({"cells":{"R1C1001":"x"}}),
            json!({"cells":{"R1C1":{}}}),
            json!({"cells":{"R1C1":"x"},"columns":0}),
            json!({"cells":{"R1C1":"x"},"columns":1.0}),
            serde_json::from_str(r#"{"cells":{"R1C1":"x"},"columns":1e0}"#).unwrap(),
            json!({"cells":{"R1C1":"x"},"columns":u64::MAX}),
            json!({"cells":{"R1C1":"x"},"style":"bold"}),
        ] {
            assert!(
                contract
                    .normalize(&json!({"values":values}), "args")
                    .is_err(),
                "{values}"
            );
        }
        assert!(contract
            .normalize(
                &json!({"values":{"cells":{"R1C1":"x","R1C2":42,"R1C3":true,"R1C4":null}}}),
                "args"
            )
            .is_ok());
        assert!(contract
            .normalize(
                &json!({"values":{"cells":{"R1C1":"x"},"columns":i64::MAX}}),
                "args"
            )
            .is_ok());
        // Область называет адрес (#1354): ключ, который её называл, снят.
        let error = contract
            .normalize(&json!({"values":{"area":"A","cells":{"R1C1":"x"}}}), "args")
            .unwrap_err();
        assert_eq!(error.path, "args.values.area");
        // `area.add` называет новую область и её начальные ячейки.
        let area = unica_templates::operations::MxlOperation::AreaAdd.contract();
        assert!(area
            .normalize(
                &json!({"items":[{"name":"A","cells":{"R1C1":"x"},"columns":2}]}),
                "args"
            )
            .is_ok());
        for args in [
            json!({"items":[]}),
            json!({"items":[{"name":"A"}]}),
            json!({"items":[{"cells":{"R1C1":"x"}}]}),
            json!({"items":[{"name":"A","cells":{"R0C1":"x"}}]}),
        ] {
            assert!(area.normalize(&args, "args").is_err(), "{args}");
        }
        let creation = metadata_template_add();
        for kind in [
            Value::Null,
            json!(7),
            json!(true),
            json!({}),
            json!("SpreadsheetDocument"),
        ] {
            assert!(creation
                .normalize(
                    &json!({"items":[{"name":"Layout","templateType":kind,"synonym":7}]}),
                    "args"
                )
                .is_ok());
        }
        assert_eq!(
            creation
                .normalize(&json!({"items":[{"name":"Layout"}]}), "args")
                .unwrap()["items"][0]["templateType"],
            "SpreadsheetDocument"
        );
        assert!(creation
            .normalize(
                &json!({"items":[{"name":"Layout","templateType":"Unknown"}]}),
                "args"
            )
            .is_err());
        assert!(creation.normalize(&json!({"items":[]}), "args").is_err());
        assert_eq!(parse_mxl_cell_address("R01C+01"), Some((1, 1)));
    }

    #[test]
    fn form_move_contract_selects_one_source_and_preserves_exact_names() {
        for (args, accepted) in [
            (
                json!({"values":{"into":" Group ","after":" Previous "}}),
                true,
            ),
            (json!({"values":{"into":null}}), true),
            (json!({"values":{}}), false),
            (json!({"values":{"into":false}}), false),
            (json!({"values":{"into":""}}), false),
            (json!({"values":{"into":"G","name":"A"}}), false),
            (json!({"items":[{"name":"A","into":"G"}]}), false),
            (json!({"values":{"into":"G","after":null}}), false),
            (json!({"values":{"into":"G","before":"A"}}), false),
        ] {
            let contract = form_move();
            let validator = jsonschema::validator_for(&contract.schema).unwrap();
            assert_eq!(validator.is_valid(&args), accepted, "{args}");
            let normalized = contract.normalize(&args, "ops[3].args");
            assert_eq!(normalized.is_ok(), accepted, "{args}: {normalized:?}");
            if accepted {
                assert_eq!(normalized.unwrap(), args);
            }
        }
        let error = form_move()
            .normalize(&json!({"values":{"into":false}}), "ops[3].args")
            .unwrap_err();
        assert_eq!(error.path, "ops[3].args.values.into");
    }

    #[test]
    fn every_implemented_dcs_operation_has_a_valid_example_and_small_contract() {
        let registry = crate::domain::apply::operation_registry();
        let mut count = 0;
        for descriptor in registry
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.family() == OperationFamily::Dcs)
        {
            let contract = descriptor
                .argument_contract()
                .unwrap_or_else(|| panic!("{} lacks its DCS contract", descriptor.name()));
            contract
                .normalize(&contract.example_args, "args")
                .unwrap_or_else(|error| panic!("{}: {error:?}", descriptor.name()));
            assert!(serde_json::to_vec(&contract.details()).unwrap().len() < 16 * 1024);
            count += 1;
        }
        assert_eq!(count, 53);
    }

    #[test]
    fn dcs_contract_defaults_aliases_and_roles_match_the_typed_boundary() {
        let query = dcs_contract("query.patch").unwrap();
        assert_eq!(
            query
                .normalize(&json!({"values":{"find":"old"}}), "args")
                .unwrap(),
            json!({"values":{"find":"old","replace":"","once":false}})
        );
        let structure = dcs_contract("structure.set").unwrap();
        assert_eq!(
            structure.normalize(&json!({"values":{}}), "args").unwrap(),
            json!({"values":{"groupBy":[],"details":true}})
        );
        // Набор и вариант называет адрес: ключи, которые его дублировали,
        // контракт не принимает (П2, #1353).
        for (op, values, key) in [
            (
                "query.set",
                json!({"dataSet":"D","query":"ВЫБРАТЬ 1"}),
                "dataSet",
            ),
            ("query.patch", json!({"dataSet":"D","find":"1"}), "dataSet"),
            ("structure.set", json!({"variant":"V"}), "variant"),
            (
                "structure.patch",
                json!({"variant":"V","name":"G","groupBy":["A"]}),
                "variant",
            ),
            // Подузел называет адрес (П3, #1354).
            ("variant.set", json!({"name":"V","title":"T"}), "name"),
            ("parameter.set", json!({"name":"P","title":"T"}), "name"),
            (
                "parameter.rename",
                json!({"name":"P","newName":"Q"}),
                "name",
            ),
            (
                "field.set",
                json!({"sourceField":"F","title":"T"}),
                "sourceField",
            ),
            ("field.set", json!({"dataPath":"F","title":"T"}), "dataPath"),
            (
                "fieldRole.set",
                json!({"field":"F","role":{"dimension":true}}),
                "field",
            ),
        ] {
            let error = dcs_contract(op)
                .unwrap()
                .normalize(&json!({ "values": values }), "args")
                .unwrap_err();
            assert_eq!(error.path, format!("args.values.{key}"), "{op}");
            assert_eq!(error.message, "unknown argument", "{op}");
        }
        let field = dcs_contract("field.add").unwrap();
        for args in [
            json!({"items":[{"name":"Alias"}]}),
            json!({"items":[{"dataPath":"Path","name":"Alias","title":""}]}),
            json!({"items":[{"dataPath":"Path","name":"","title":""}]}),
        ] {
            assert!(field.normalize(&args, "args").is_ok());
        }
        for args in [
            json!({"items":[{}]}),
            json!({"items":[{"name":"  \n"}]}),
            json!({"items":[{"name":7}]}),
            json!({"items":[{"dataPath":"","name":"Fallback"}]}),
        ] {
            assert!(field.normalize(&args, "args").is_err());
        }
        let role = dcs_contract("fieldRole.set").unwrap();
        for value in [
            json!({"dimension":true}),
            json!({"periodType":"Main"}),
            json!({"balanceType":"OpeningBalance"}),
            json!({"parentDimension":"Parent","accountField":"Account"}),
        ] {
            assert!(role
                .normalize(&json!({"values":{"role":value}}), "args")
                .is_ok());
        }
        assert!(role
            .normalize(&json!({"values":{"role":"dimension"}}), "args")
            .is_err());
        // Правка без свойств ничего не меняет и отвергается.
        for op in ["field.set", "parameter.set"] {
            assert!(
                dcs_contract(op)
                    .unwrap()
                    .normalize(&json!({"values":{}}), "args")
                    .is_err(),
                "{op}"
            );
        }
        assert!(dcs_contract("variant.remove")
            .unwrap()
            .normalize(&json!({"values":{"name":"V"}}), "args")
            .is_err());
    }

    #[test]
    fn dcs_link_removal_accepts_its_selector_and_refuses_patch_properties() {
        let remove = dcs_contract("dataSetLink.remove").unwrap();
        let set = dcs_contract("dataSetLink.set").unwrap();
        let selector = set.example_args["values"]["selector"].clone();
        let selected = json!({"values":{"selector":selector}});
        assert!(remove.normalize_dcs(&selected, "ops[4].args").is_ok());
        for (key, value) in [
            ("parameter", json!("Period")),
            ("parameterListAllowed", json!(true)),
            ("required", json!(false)),
            ("condition", json!("Amount > 0")),
            ("startExpression", json!("Amount")),
        ] {
            let mut args = selected.clone();
            args["values"][key] = value;
            assert!(set.normalize_dcs(&args, "ops[4].args").is_ok(), "{key}");
            assert!(
                remove.normalize_dcs(&args, "ops[4].args").is_err(),
                "link removal silently accepted patch property {key}"
            );
        }
    }

    #[test]
    fn dcs_structure_add_refuses_grouping_on_tables_and_charts() {
        let contract = dcs_contract("structure.add").unwrap();
        assert!(contract
            .normalize_dcs(&contract.example_args, "args")
            .is_ok());
        let mut default_group = contract.example_args.clone();
        default_group["items"][0]
            .as_object_mut()
            .unwrap()
            .remove("kind");
        assert_eq!(
            contract.normalize_dcs(&default_group, "args").unwrap()["items"][0]["kind"],
            "group"
        );
        for kind in ["table", "chart"] {
            let mut args = contract.example_args.clone();
            args["items"][0]["kind"] = json!(kind);
            args["items"][0].as_object_mut().unwrap().remove("groupBy");
            assert!(contract.normalize_dcs(&args, "args").is_ok(), "{kind}");
            for group_by in [json!(["Amount"]), json!([])] {
                args["items"][0]["groupBy"] = group_by;
                assert!(
                    contract.normalize_dcs(&args, "args").is_err(),
                    "{kind} silently accepted groupBy"
                );
            }
        }
    }

    #[test]
    fn dcs_auto_order_refuses_explicit_direction_and_field_order_keeps_defaults() {
        let contract = dcs_contract("order.add").unwrap();
        let default_field = contract
            .normalize_dcs(&contract.example_args, "args")
            .unwrap();
        assert_eq!(default_field["items"][0]["kind"], "Field");
        assert_eq!(default_field["items"][0]["direction"], "Asc");
        let mut auto = json!({"items":[{"kind":"Auto"}]});
        let normalized_auto = contract.normalize_dcs(&auto, "args").unwrap();
        assert!(normalized_auto["items"][0].get("direction").is_none());
        for direction in ["Asc", "Desc"] {
            let mut field = contract.example_args.clone();
            field["items"][0]["direction"] = json!(direction);
            assert!(
                contract.normalize_dcs(&field, "args").is_ok(),
                "{direction}"
            );
            auto["items"][0]["direction"] = json!(direction);
            assert!(
                contract.normalize_dcs(&auto, "args").is_err(),
                "Auto order silently accepted {direction} direction"
            );
        }
    }

    #[test]
    fn dcs_value_type_requires_a_corresponding_value_or_list() {
        for (operation, args) in [
            (
                "parameter.add",
                json!({"items":[{"name":"Period","valueType":"xs:string"}]}),
            ),
            ("parameter.set", json!({"values":{"valueType":"xs:string"}})),
            (
                "filter.add",
                json!({"items":[{"field":"Amount","valueType":"xs:string"}]}),
            ),
            (
                "filter.set",
                json!({"values":{"field":"Amount","valueType":"xs:string"}}),
            ),
            (
                "dataParameter.set",
                json!({"values":{"name":"Period","valueType":"xs:string"}}),
            ),
            (
                "outputParameter.set",
                json!({"values":{"name":"Period","valueType":"xs:string"}}),
            ),
        ] {
            let contract = dcs_contract(operation).unwrap();
            assert!(contract
                .normalize_dcs(&contract.example_args, "args")
                .is_ok());
            let mut with_value = args.clone();
            let values = if with_value.get("items").is_some() {
                &mut with_value["items"][0]
            } else {
                &mut with_value["values"]
            };
            values["value"] = json!("literal");
            assert!(
                contract.normalize_dcs(&with_value, "args").is_ok(),
                "{operation}"
            );
            if operation.starts_with("parameter.") {
                for (key, list) in [
                    ("values", json!(["literal"])),
                    ("availableValues", json!([{"value":"literal"}])),
                    ("values", json!([])),
                    ("availableValues", json!([])),
                ] {
                    let mut with_list = args.clone();
                    let values = if with_list.get("items").is_some() {
                        &mut with_list["items"][0]
                    } else {
                        &mut with_list["values"]
                    };
                    values[key] = list;
                    assert!(
                        contract.normalize_dcs(&with_list, "args").is_ok(),
                        "{operation}.{key}"
                    );
                }
            }
            assert!(
                contract.normalize_dcs(&args, "args").is_err(),
                "{operation} silently accepted valueType without its value"
            );
        }
        let appearance = dcs_contract("conditionalAppearance.add").unwrap();
        let mut args = appearance.example_args.clone();
        args["items"][0]["filter"] =
            json!([{"field":"Amount","valueType":"xs:string","value":"literal"}]);
        assert!(appearance.normalize_dcs(&args, "args").is_ok());
        args["items"][0]["filter"][0]
            .as_object_mut()
            .unwrap()
            .remove("value");
        assert!(
            appearance.normalize_dcs(&args, "args").is_err(),
            "appearance filter silently accepted valueType without its value"
        );
    }
}

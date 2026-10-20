use super::*;
use unica_bsl::profile::PlatformProfile;
use unica_workspace::test_support::tree_snapshot;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use unica_address::address::QualifiedAddress;
use unica_operations::OperationFamily;
use unica_source_contract::proposal::ApplyPlanError;

const OP_INDEX: usize = 7;

fn compare_schema_and_parser(
    fixture: &MetadataFixture,
    operation: &str,
    target: &str,
    mut args: Value,
    expected: bool,
) {
    args["at"] = json!(target);
    let address = QualifiedAddress::parse(target).unwrap();
    let traits = crate::domain::apply::source_set_traits_for_read(
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
    );
    let site = crate::domain::apply::operation_site(&address, traits).unwrap();
    let descriptor = crate::domain::apply::operation_registry()
        .lookup_at(operation, &site)
        .unwrap();
    assert_eq!(descriptor.family(), OperationFamily::Metadata);
    assert!(descriptor.applies_at(&site), "{operation} at {target}");
    let contract = descriptor.argument_contract_at(&site).unwrap();
    // A complete, independent schema engine is the oracle. The production
    // staged reader intentionally does not implement all JSON Schema keywords.
    let oracle = jsonschema::validator_for(&contract.schema).unwrap();
    let schema = oracle.is_valid(&args);
    let parsed = parse_metadata_plan_operation(operation, &args, OP_INDEX, &fixture.binding);
    assert_eq!(schema, expected, "schema: {operation} {args}");
    assert_eq!(
        parsed.is_ok(),
        expected,
        "productive parser: {operation} {args}: {parsed:?}"
    );
    if let Err(error) = parsed {
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert!(
            error
                .path()
                .is_some_and(|path| path.starts_with("ops[7].args")),
            "{operation} {args}: {error:?}"
        );
    }
}

fn field_args(verb: &str, field: &str, value: Value) -> Value {
    let mut payload = Map::new();
    if verb == "add" {
        payload.insert("name".into(), json!("Member"));
    }
    payload.insert(field.into(), value);
    if verb == "add" {
        json!({"items": [payload]})
    } else {
        json!({"values": payload})
    }
}

fn expect_error(
    fixture: &MetadataFixture,
    operation: &str,
    args: Value,
    suffix: &str,
    message: &str,
) -> ApplyPlanError {
    let error =
        parse_metadata_plan_operation(operation, &args, OP_INDEX, &fixture.binding).unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{args}");
    assert_eq!(
        error.path(),
        Some(format!("ops[{OP_INDEX}].args.{suffix}").as_str()),
        "{args}: {error:?}"
    );
    assert_eq!(error.to_string(), message, "{args}");
    error
}

#[test]
fn all_twenty_one_member_shapes_match_the_productive_parser_without_source_writes() {
    let fixture = MetadataFixture::new();
    let source = fixture._root.path().join("src");
    let before = tree_snapshot(&source);
    let mut checked = BTreeSet::new();
    for (family, owner, leaf) in [
        ("attribute", "Document.Order", "Attribute"),
        ("tabularSection", "Document.Order", "TabularSection"),
        ("dimension", "InformationRegister.Prices", "Dimension"),
        ("resource", "InformationRegister.Prices", "Resource"),
        ("enumValue", "Enum.Kinds", "EnumValue"),
        ("column", "DocumentJournal.All", "Column"),
        ("command", "Document.Order", "Command"),
    ] {
        for verb in ["add", "set", "remove"] {
            let operation = format!("{family}.{verb}");
            checked.insert(operation.clone());
            let target = if verb == "add" {
                format!("main:{owner}")
            } else {
                format!("main:{owner}.{leaf}.Member")
            };
            let samples = match verb {
                "add" => vec![
                    (
                        json!({"items":[{"name":"Member","comment":"probe","synonym":""}]}),
                        true,
                    ),
                    (json!({"items":[{"name":"Member","ignored":true}]}), false),
                    (json!({"items":[{"comment":"probe"}]}), false),
                    (json!({"items":[{"name":false}]}), false),
                    (json!({"items":[{"name":"Member","comment":false}]}), false),
                    (json!({"items":[{"name":"Member","synonym":42}]}), false),
                    (json!({"items":[]}), false),
                    (json!({"items":false}), false),
                    (json!({}), false),
                ],
                "set" => vec![
                    (
                        json!({"values":{"newName":"Renamed","comment":"probe","synonym":""}}),
                        true,
                    ),
                    (json!({"values":{"ignored":true}}), false),
                    (json!({"values":{"name":"Member"}}), false),
                    (json!({"values":{"newName":false}}), false),
                    (json!({"values":{"comment":false}}), false),
                    (json!({"values":{"synonym":42}}), false),
                    (json!({"values":{}}), false),
                    (json!({"values":false}), false),
                    (json!({}), false),
                ],
                "remove" => vec![
                    (json!({}), true),
                    (json!({"values":{}}), false),
                    (json!({"items":[]}), false),
                ],
                _ => unreachable!(),
            };
            for (args, expected) in samples {
                compare_schema_and_parser(&fixture, &operation, &target, args, expected);
            }
            if verb != "remove" {
                for (field, value, expected) in [
                    (
                        "required",
                        json!(true),
                        matches!(family, "attribute" | "dimension" | "resource"),
                    ),
                    ("required", json!("true"), false),
                    (
                        "indexing",
                        json!("Index"),
                        matches!(family, "attribute" | "dimension" | "resource" | "column"),
                    ),
                    ("indexing", json!(false), false),
                ] {
                    compare_schema_and_parser(
                        &fixture,
                        &operation,
                        &target,
                        field_args(verb, field, value),
                        expected,
                    );
                }
            }
            let mut unknown_envelope = if verb == "add" {
                json!({"items":[{"name":"Member"}]})
            } else if verb == "set" {
                json!({"values":{"comment":"probe"}})
            } else {
                json!({})
            };
            unknown_envelope["ignored"] = json!(true);
            compare_schema_and_parser(&fixture, &operation, &target, unknown_envelope, false);
        }
    }
    assert_eq!(checked.len(), 21);
    assert_eq!(tree_snapshot(&source), before);
}

#[test]
fn nested_type_and_position_shapes_reach_each_typed_member_reader() {
    let fixture = MetadataFixture::new();
    let float_type = |raw: &str, field: &str| {
        let value: Value = serde_json::from_str(raw).unwrap();
        assert!(
            value["variants"][0][field].is_f64(),
            "the case must reach the parser as a JSON floating-point number: {raw}"
        );
        value
    };
    let type_samples = [
        (
            json!({"variants":[{"kind":"string","length":12,"allowedLength":"variable"}]}),
            true,
        ),
        (
            json!({"variants":[{"kind":"number","digits":15,"fraction":2,"sign":"any"}]}),
            true,
        ),
        (json!({"variants":[{"kind":"boolean"}]}), true),
        (
            float_type(
                r#"{"variants":[{"kind":"string","length":1.0,"allowedLength":"variable"}]}"#,
                "length",
            ),
            true,
        ),
        (
            float_type(
                r#"{"variants":[{"kind":"string","length":1e0,"allowedLength":"variable"}]}"#,
                "length",
            ),
            true,
        ),
        (
            float_type(
                r#"{"variants":[{"kind":"number","digits":1.0,"fraction":0.0,"sign":"any"}]}"#,
                "digits",
            ),
            true,
        ),
        (
            float_type(
                r#"{"variants":[{"kind":"number","digits":1e0,"fraction":0e0,"sign":"any"}]}"#,
                "fraction",
            ),
            true,
        ),
        (
            float_type(
                r#"{"variants":[{"kind":"string","length":1.5,"allowedLength":"variable"}]}"#,
                "length",
            ),
            false,
        ),
        (json!(false), false),
        (json!({}), false),
        (json!({"variants":false}), false),
        (json!({"variants":[]}), false),
        (
            json!({"variants":[{"kind":"boolean","ignored":true}]}),
            false,
        ),
        (json!({"variants":[{"kind":"unknown"}]}), false),
        (
            json!({"variants":[{"kind":"string","length":false,"allowedLength":"variable"}]}),
            false,
        ),
        (
            json!({"variants":[{"kind":"string","length":1025,"allowedLength":"variable"}]}),
            false,
        ),
        (
            json!({"variants":[{"kind":"string","length":0,"allowedLength":"fixed"}]}),
            false,
        ),
        (
            json!({"variants":[{"kind":"number","digits":2,"fraction":3,"sign":"any"}]}),
            false,
        ),
        (
            json!({"variants":[{"kind":"boolean"},{"kind":"boolean"}]}),
            false,
        ),
        (
            json!({"variants":[{"kind":"valueStorage"},{"kind":"boolean"}]}),
            false,
        ),
    ];
    for (family, owner, leaf) in [
        ("attribute", "Document.Order", "Attribute"),
        ("dimension", "InformationRegister.Prices", "Dimension"),
        ("resource", "InformationRegister.Prices", "Resource"),
        ("column", "DocumentJournal.All", "Column"),
    ] {
        for verb in ["add", "set"] {
            let target = if verb == "add" {
                format!("main:{owner}")
            } else {
                format!("main:{owner}.{leaf}.Member")
            };
            for (value, expected) in &type_samples {
                compare_schema_and_parser(
                    &fixture,
                    &format!("{family}.{verb}"),
                    &target,
                    field_args(verb, "type", value.clone()),
                    *expected,
                );
            }
        }
    }
    for (family, owner, leaf) in [
        ("attribute", "Document.Order", "Attribute"),
        ("tabularSection", "Document.Order", "TabularSection"),
        ("dimension", "InformationRegister.Prices", "Dimension"),
        ("resource", "InformationRegister.Prices", "Resource"),
        ("enumValue", "Enum.Kinds", "EnumValue"),
        ("column", "DocumentJournal.All", "Column"),
        ("command", "Document.Order", "Command"),
    ] {
        for verb in ["add", "set"] {
            let target = if verb == "add" {
                format!("main:{owner}")
            } else {
                format!("main:{owner}.{leaf}.Member")
            };
            for (value, expected) in [
                (json!({"before":"Anchor"}), true),
                (json!({"after":"Anchor"}), true),
                (json!({}), false),
                (json!({"before":"Anchor","after":"Other"}), false),
                (json!({"before":false}), false),
                (json!({"before":""}), false),
                (json!({"before":"Anchor","ignored":true}), false),
            ] {
                compare_schema_and_parser(
                    &fixture,
                    &format!("{family}.{verb}"),
                    &target,
                    field_args(verb, "position", value),
                    expected,
                );
            }
        }
    }
}

#[test]
fn fill_shape_uses_the_same_owner_and_tabular_context_as_the_productive_reader() {
    let fixture = MetadataFixture::new();
    for (family, owner, leaf, allowed) in [
        ("attribute", "Document.Order", "Attribute", true),
        (
            "attribute",
            "Document.Order.TabularSection.Lines",
            "Attribute",
            false,
        ),
        ("attribute", "Report.Sales", "Attribute", false),
        (
            "attribute",
            "Report.Sales.TabularSection.Lines",
            "Attribute",
            true,
        ),
        ("resource", "InformationRegister.Prices", "Resource", true),
        ("resource", "AccumulationRegister.Stock", "Resource", false),
    ] {
        for verb in ["add", "set"] {
            let target = if verb == "add" {
                format!("main:{owner}")
            } else {
                format!("main:{owner}.{leaf}.Member")
            };
            for (fill, structurally_valid) in [
                (json!({"kind":"boolean","value":true}), true),
                (json!({"kind":"boolean","value":"true"}), false),
                (json!({"kind":"boolean","value":true,"ignored":1}), false),
                (json!({"kind":"boolean"}), false),
                (json!({"kind":"unknown"}), false),
                (json!(false), false),
            ] {
                let mut payload =
                    json!({"type":{"variants":[{"kind":"boolean"}]},"fillValue":fill});
                let args = if verb == "add" {
                    payload["name"] = json!("Member");
                    json!({"items":[payload]})
                } else {
                    json!({"values":payload})
                };
                compare_schema_and_parser(
                    &fixture,
                    &format!("{family}.{verb}"),
                    &target,
                    args,
                    allowed && structurally_valid,
                );
            }
        }
    }
    for (owner, allows_fill) in [("Document.Order", false), ("Report.Sales", true)] {
        for (attributes, expected) in [
            (json!(false), false),
            (json!({}), false),
            (json!([]), false),
            (
                json!([{"name":"Value","type":{"variants":[{"kind":"boolean"}]}}]),
                true,
            ),
        ] {
            compare_schema_and_parser(
                &fixture,
                "tabularSection.add",
                &format!("main:{owner}"),
                json!({"items":[{"name":"Lines","attributes":attributes}]}),
                expected,
            );
        }
        for (attribute, structurally_valid) in [
            (
                json!({"name":"Value","type":{"variants":[{"kind":"boolean"}]},"fillValue":{"kind":"boolean","value":true}}),
                true,
            ),
            (json!({"name":"Value","ignored":1}), false),
            (
                json!({"name":"Value","type":{"variants":[{"kind":"boolean","ignored":1}]}}),
                false,
            ),
        ] {
            let expected =
                structurally_valid && (allows_fill || attribute.get("fillValue").is_none());
            compare_schema_and_parser(
                &fixture,
                "tabularSection.add",
                &format!("main:{owner}"),
                json!({"items":[{"name":"Lines","attributes":[attribute]}]}),
                expected,
            );
        }
    }
}

#[test]
fn hidden_host_envelope_consumes_the_same_member_field_shapes() {
    let schema = unica_metadata::request::metadata_input_schema(unica_metadata::request::MetadataOperation::Edit);
    let oracle = jsonschema::validator_for(&schema).unwrap();
    for verb in ["add", "update"] {
        for (field, value, expected) in [
            ("comment", json!("probe"), true),
            ("comment", json!(false), false),
            ("synonym", json!(""), true),
            ("synonym", json!(42), false),
            ("required", json!(true), true),
            ("required", json!("true"), false),
            ("type", json!({"variants":[{"kind":"boolean"}]}), true),
            (
                "type",
                json!({"variants":[{"kind":"boolean","ignored":true}]}),
                false,
            ),
        ] {
            let mut element = json!({"name":"Member"});
            element[field] = value;
            let args = json!({
                "sourceSet":"main",
                "metadataPath":"InformationRegister.Prices",
                "operations":[{"op":verb,"collection":"resources","elements":[element]}],
                "dryRun":true,
            });
            let parsed = unica_metadata::request::parse_metadata_request(unica_metadata::request::MetadataOperation::Edit, args.as_object().unwrap());
            assert_eq!(oracle.is_valid(&args), expected, "host schema: {args}");
            assert_eq!(parsed.is_ok(), expected, "host parser: {args}: {parsed:?}");
        }

        // The legacy host union deliberately publishes an open element and
        // omits indexing. Its oracle therefore accepts unknown properties;
        // the productive decoder still checks their real meaning. A2 keeps
        // this existing envelope rather than narrowing its schema language.
        for (field, value) in [("indexing", json!(false)), ("ignored", json!(true))] {
            let mut element = json!({"name":"Member"});
            element[field] = value;
            let args = json!({
                "sourceSet":"main",
                "metadataPath":"InformationRegister.Prices",
                "operations":[{"op":verb,"collection":"resources","elements":[element]}],
                "dryRun":true,
            });
            assert!(
                oracle.is_valid(&args),
                "the host envelope stays open: {args}"
            );
            assert!(
                unica_metadata::request::parse_metadata_request(unica_metadata::request::MetadataOperation::Edit, args.as_object().unwrap()).is_err(),
                "the decoder refuses malformed or unknown fields: {args}"
            );
        }
    }
    let host_add = schema["properties"]["operations"]["items"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| variant["properties"]["op"]["enum"] == json!(["add"]))
        .unwrap();
    assert!(host_add["properties"]["elements"]["items"]["properties"]
        .get("indexing")
        .is_none());
}

#[test]
fn raw_union_read_order_precedes_contextual_legality_at_the_original_index() {
    let fixture = MetadataFixture::new();
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":null,"unknown":true,"values":false}),
        "unknown",
        "operation does not accept argument `unknown`; `resource.set` expects `values`",
    );
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":"main:AccumulationRegister.Stock.Resource.Member","values":{"newName":false,"fillValue":{"kind":"invalid"}}}),
        "values.newName",
        "`newName` must be a string",
    );
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":"main:AccumulationRegister.Stock.Resource.Member","values":{"newName":"Renamed","fillValue":{"kind":"invalid"}}}),
        "values.fillValue.kind",
        "unsupported fill value kind `invalid`",
    );
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":"main:InformationRegister.Prices.Resource.Member","values":{"name":false,"newName":false}}),
        "values.name",
        "`resource.set` names the member by its address; rename it with `newName`",
    );
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":"main:InformationRegister.Prices.Resource.Member","values":{"newName":false,"unknown":true}}),
        "values.unknown",
        "unknown field `unknown`",
    );
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":"main:InformationRegister.Prices.Resource.Member","values":{"indexing":false,"newName":false}}),
        "values.indexing",
        "`indexing` must be a string",
    );
    expect_error(
        &fixture,
        "command.set",
        json!({"at":"main:Document.Order.Command.Member","values":{"type":false}}),
        "values.type",
        "type must be a structured object",
    );
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":"main:InformationRegister.Prices.Resource.Member","values":{"position":{"before":false,"unknown":true}}}),
        "values.position.unknown",
        "unknown field `unknown`",
    );
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":"main:InformationRegister.Prices.Resource.Member","values":{"fillValue":{"kind":"boolean","value":"wrong","unknown":true}}}),
        "values.fillValue.value",
        "boolean fill value must be a boolean",
    );
}

#[test]
fn nested_payloads_finish_decoding_before_element_conversion() {
    let fixture = MetadataFixture::new();
    expect_error(
        &fixture,
        "resource.add",
        json!({"at":"main:InformationRegister.Prices","items":[{"name":"bad identifier"},{"name":"Second","type":false}]}),
        "items[1].type",
        "type must be a structured object",
    );
    expect_error(
        &fixture,
        "tabularSection.add",
        json!({"at":"main:Document.Order","items":[{"name":false,"attributes":[{"name":false}]}]}),
        "items[0].attributes[0].name",
        "`elements[0].attributes[0].name` must be a string",
    );
    expect_error(
        &fixture,
        "resource.set",
        json!({"at":"main:InformationRegister.Prices.Resource.Member","values":{"type":{"variants":[{"kind":"unknown","ignored":true}]}}}),
        "values.type.variants[0].kind",
        "unsupported type kind `unknown`",
    );
}

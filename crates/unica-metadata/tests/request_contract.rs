//! Контракт запроса метаданных: опубликованная JSON-схема и парсер
//! `unica_metadata::request` держат одну и ту же замкнутую область значений.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::contracts::metadata_predefined_element_input_schema;
use unica_metadata::request::*;
use unica_metadata::*;

fn object(value: Value) -> Map<String, Value> {
    value
        .as_object()
        .expect("test input must be an object")
        .clone()
}

#[test]
fn metadata_kind_rejects_one_segment_root_module_address() {
    let address = MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "ManagedApplicationModule",
    )
    .unwrap();

    let diagnostic = metadata_kind_for_address(&address).unwrap_err();

    assert_eq!(diagnostic.code, MetaDiagnosticCode::InvalidArguments);
    assert_eq!(diagnostic.field.as_deref(), Some("metadataPath"));
}

fn resolve_definition<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(reference) => {
            let definition = reference
                .strip_prefix("#/$defs/")
                .expect("test schema reference must target the root definitions");
            &root["$defs"][definition]
        }
        None => schema,
    }
}

fn resolve_operation_variant<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    let resolved = resolve_definition(root, schema);
    resolved
        .get("allOf")
        .and_then(Value::as_array)
        .and_then(|schemas| schemas.first())
        .map(|schema| resolve_definition(root, schema))
        .unwrap_or(resolved)
}

/// The published, host-visible operation union.
///
/// It is kind-agnostic by design: per-kind legality is enforced by the
/// parser and the writer, which name the exact offending field, so schema
/// assertions target the union and kind assertions target the parser.
fn published_operation_union(operation: unica_metadata::request::MetadataOperation) -> Value {
    unica_metadata::request::metadata_input_schema(operation)["properties"]["operations"]["items"]
        .clone()
}

#[test]
fn constant_type_property_is_accepted_by_existing_add_and_edit_operations() {
    for (operation, mut call) in [
        (
            unica_metadata::request::MetadataOperation::Add,
            json!({"sourceSet":"main", "kind":"Constant", "name":"Enabled"}),
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({"sourceSet":"main", "metadataPath":"Constant.Enabled"}),
        ),
    ] {
        call["operations"] =
            json!([{"op":"setProperties", "values":{"Type":{"variants":[{"kind":"boolean"}]}}}]);
        let parsed =
            unica_metadata::request::parse_metadata_request(operation, call.as_object().unwrap())
                .unwrap_or_else(|error| {
                    panic!("{operation:?}: structured Constant.Type rejected: {error:?}")
                });
        let operations = match parsed {
            unica_metadata::request::MetadataRequest::Add(request) => request.operations,
            unica_metadata::request::MetadataRequest::Edit(request) => request.operations,
            unica_metadata::request::MetadataRequest::Info(_) => {
                panic!("mutation parsed as info")
            }
        };
        assert!(
            matches!(&operations[..], [MetaEditOperation::SetProperties { values }] if values.entries().len() == 1)
        );
        let schema = unica_metadata::request::metadata_input_schema(operation);
        assert!(
            jsonschema::validator_for(&schema).unwrap().is_valid(&call),
            "{operation:?}: published schema rejected structured Constant.Type"
        );
    }
}

#[test]
fn constant_type_property_preserves_reference_type_through_add_and_edit_schema() {
    for (operation, mut call) in [
        (
            unica_metadata::request::MetadataOperation::Add,
            json!({"sourceSet":"main", "kind":"Constant", "name":"DefaultItem"}),
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({"sourceSet":"main", "metadataPath":"Constant.DefaultItem"}),
        ),
    ] {
        call["operations"] = json!([{"op":"setProperties", "values":{"Type":{"variants":[{"kind":"reference", "metadataPath":"Catalog.Items"}]}}}]);
        let schema = unica_metadata::request::metadata_input_schema(operation);
        assert!(
            jsonschema::validator_for(&schema).unwrap().is_valid(&call),
            "{operation:?}"
        );
        let parsed =
            unica_metadata::request::parse_metadata_request(operation, call.as_object().unwrap())
                .unwrap();
        let operations = match parsed {
            unica_metadata::request::MetadataRequest::Add(request) => request.operations,
            unica_metadata::request::MetadataRequest::Edit(request) => request.operations,
            unica_metadata::request::MetadataRequest::Info(_) => {
                panic!("mutation parsed as info")
            }
        };
        let [MetaEditOperation::SetProperties { values }] = &operations[..] else {
            panic!("Type must stay in existing SetProperties operation");
        };
        let expected = MetadataType::new(vec![MetadataTypeVariant::Reference {
            metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items")
                .unwrap(),
        }])
        .unwrap();
        assert!(
            matches!(values.entries(), [(_, MetaPropertyValue::Type(actual))] if actual == &expected)
        );
    }
}

#[test]
fn constant_type_property_schema_and_parser_reject_non_closed_type_values() {
    let invalid_types = [
        json!("xs:boolean"),
        json!({"variants":[]}),
        json!({"variants":[{"kind":"boolean"}], "unknown":true}),
        json!({"variants":[{"kind":"boolean", "unknown":true}]}),
        json!({"variants":[{"kind":"notAType"}]}),
        json!({"variants":[{"kind":"reference", "metadataPath":"Catalog.Items.Form.Main"}]}),
    ];
    for (operation, base) in [
        (
            unica_metadata::request::MetadataOperation::Add,
            json!({"sourceSet":"main", "kind":"Constant", "name":"Enabled"}),
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({"sourceSet":"main", "metadataPath":"Constant.Enabled"}),
        ),
    ] {
        let schema = unica_metadata::request::metadata_input_schema(operation);
        let validator = jsonschema::validator_for(&schema).unwrap();
        for value in &invalid_types {
            let mut call = base.clone();
            call["operations"] = json!([{"op":"setProperties", "values":{"Type":value}}]);
            assert!(
                !validator.is_valid(&call),
                "{operation:?}: schema accepted {value}"
            );
            let error = unica_metadata::request::parse_metadata_request(
                operation,
                call.as_object().unwrap(),
            )
            .expect_err("malformed Constant.Type must be refused");
            assert!(
                error.diagnostics.iter().any(|item| item
                    .field
                    .as_deref()
                    .is_some_and(|field| field.starts_with("values.Type"))),
                "{operation:?}/{value}: {:?}",
                error.diagnostics
            );
        }
    }
}

#[test]
fn constant_type_property_does_not_become_a_property_of_other_owners() {
    for kind in ["Catalog", "Document", "CommonModule"] {
        for (operation, mut call) in [
            (
                unica_metadata::request::MetadataOperation::Add,
                json!({"sourceSet":"main", "kind":kind, "name":"Sample"}),
            ),
            (
                unica_metadata::request::MetadataOperation::Edit,
                json!({"sourceSet":"main", "metadataPath":format!("{kind}.Sample")}),
            ),
        ] {
            call["operations"] = json!([{"op":"setProperties", "values":{"Type":{"variants":[{"kind":"boolean"}]}}}]);
            // The published operation schema is intentionally owner-agnostic.
            let error = unica_metadata::request::parse_metadata_request(
                operation,
                call.as_object().unwrap(),
            )
            .expect_err("only Constant owns the root Type property");
            assert!(
                error
                    .diagnostics
                    .iter()
                    .any(|item| item.field.as_deref() == Some("values.Type")),
                "{operation:?}/{kind}: {:?}",
                error.diagnostics
            );
        }
    }
}

/// The published `values` object of the `setProperties` union branch.
fn published_root_property_values(operation: unica_metadata::request::MetadataOperation) -> Value {
    published_operation_union(operation)["oneOf"]
        .as_array()
        .expect("operation items publish a oneOf union")
        .iter()
        .find(|variant| variant["properties"]["op"]["enum"] == json!(["setProperties"]))
        .expect("the union publishes a setProperties branch")["properties"]["values"]
        .clone()
}

fn diagnostic(
    operation: unica_metadata::request::MetadataOperation,
    value: Value,
) -> unica_metadata::MetaDiagnostic {
    unica_metadata::request::parse_metadata_request(operation, &object(value))
        .expect_err("input must fail")
        .diagnostics
        .into_iter()
        .next()
        .expect("failure must contain a diagnostic")
}

fn edit(operation: Value) -> Value {
    json!({
        "sourceSet": "main",
        "metadataPath": "Document.Order",
        "operations": [operation]
    })
}

/// Parse a call and hold the published schema to its half of the contract.
///
/// The published union is kind-agnostic, so it deliberately admits calls the
/// writer refuses for a concrete kind: the writer is the arbiter and answers
/// with `unsupported_kind` naming the exact field. The one direction that
/// must never break is the other one — the schema may not reject a call the
/// writer accepts, because that advertises a narrower contract than the tool
/// actually honours and leaves a legal call unreachable.
fn validate_schema_and_parse(
    operation: unica_metadata::request::MetadataOperation,
    call: &Value,
) -> bool {
    let schema = unica_metadata::request::metadata_input_schema(operation);
    let validator = jsonschema::validator_for(&schema).unwrap();
    let schema_accepts = validator.is_valid(call);
    let parser_accepts =
        unica_metadata::request::parse_metadata_request(operation, call.as_object().unwrap())
            .is_ok();
    assert!(
        schema_accepts || !parser_accepts,
        "published schema rejects a call the writer accepts: {call}"
    );
    parser_accepts
}

#[test]
fn parse_info_and_add_apply_defaults_without_accepting_aliases() {
    let unica_metadata::request::MetadataRequest::Info(info) =
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Info,
            &object(json!({"sourceSet": "main", "metadataPath": "Документ.Заказ"})),
        )
        .unwrap()
    else {
        panic!("expected info request")
    };
    assert_eq!(info.source_set, "main");
    assert_eq!(info.metadata_path.as_str(), "Document.Заказ");
    assert!(info.sections.is_empty());
    assert_eq!(info.limit, 20);

    for kind in MetadataKind::ALL {
        let unica_metadata::request::MetadataRequest::Add(add) =
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Add,
                &object(json!({"sourceSet": "main", "kind": kind.as_str(), "name": "Created"})),
            )
            .unwrap()
        else {
            panic!("expected add request")
        };
        assert_eq!(add.kind, *kind);
        assert!(add.dry_run);
    }

    let error = diagnostic(
        unica_metadata::request::MetadataOperation::Info,
        json!({"sourceSet": "main", "metadataPath": "Document.Order", "ObjectPath": "x"}),
    );
    assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
    assert_eq!(error.field.as_deref(), Some("ObjectPath"));
}

#[test]
fn info_limit_is_bounded() {
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Info,
    );
    assert_eq!(schema["properties"]["sections"]["default"], json!([]));
    assert_eq!(schema["properties"]["limit"]["maximum"], json!(50));

    let unica_metadata::request::MetadataRequest::Info(info) =
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Info,
            &object(json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "limit": 50,
            })),
        )
        .unwrap()
    else {
        panic!("expected info request")
    };
    assert_eq!(info.limit, 50);

    let error = diagnostic(
        unica_metadata::request::MetadataOperation::Info,
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "limit": 51,
        }),
    );
    assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
    assert_eq!(error.field.as_deref(), Some("limit"));
}

#[test]
fn edit_schema() {
    let root = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let schema = published_operation_union(unica_metadata::request::MetadataOperation::Edit);
    let variants = schema["oneOf"]
        .as_array()
        .expect("metadata edit operations must publish a oneOf union");
    let expected = [
        ("setProperties", &["op", "values"][..]),
        ("add", &["op", "collection", "elements"][..]),
        ("update", &["op", "collection", "elements"][..]),
        ("remove", &["op", "collection", "names"][..]),
        ("add", &["op", "collection", "elements"][..]),
        ("update", &["op", "collection", "elements"][..]),
        ("remove", &["op", "collection", "ids"][..]),
        ("addHelp", &["op"][..]),
        ("editRelations", &["op", "relation", "mode", "targets"][..]),
    ];

    assert_eq!(variants.len(), expected.len());
    for (variant, (op, required)) in variants.iter().zip(expected) {
        let variant = resolve_operation_variant(&root, variant);
        assert_eq!(variant["type"], json!("object"));
        assert_eq!(variant["additionalProperties"], json!(false));
        assert_eq!(variant["properties"]["op"]["enum"], json!([op]));
        assert_eq!(variant["required"], json!(required));
    }
}

#[test]
fn relation_schema_correlates_source_mode_and_target_algebra() {
    let add = published_operation_union(unica_metadata::request::MetadataOperation::Add);
    let edit = published_operation_union(unica_metadata::request::MetadataOperation::Edit);
    assert_eq!(
        add, edit,
        "meta.add and meta.edit share one operation algebra"
    );
    let operations = edit["oneOf"].as_array().expect("closed operation union");
    // Пять тегов, но `add`/`update`/`remove` дополнительно публикуют
    // predefined-ветки со своим составом полей.
    assert_eq!(operations.len(), MetaEditOperationTag::ALL.len() + 3);
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation["properties"]["op"]["enum"][0]
                .as_str()
                .expect("operation tag"))
            .collect::<HashSet<_>>(),
        HashSet::from([
            "setProperties",
            "add",
            "update",
            "remove",
            "editRelations",
            "addHelp",
        ]),
    );

    let relation_operation = operations
        .iter()
        .find(|operation| operation["properties"]["op"]["enum"] == json!(["editRelations"]))
        .expect("editRelations operation");
    let relations = relation_operation["oneOf"]
        .as_array()
        .expect("relation-correlated operation union");
    assert_eq!(relations.len(), MetaRelation::ALL.len());
    let relation = |name: &str| {
        relations
            .iter()
            .find(|branch| branch["properties"]["relation"]["const"] == name)
            .unwrap_or_else(|| panic!("missing {name} relation branch"))
    };

    let source = relation("source");
    assert_eq!(
        source["properties"]["mode"]["enum"],
        json!(["add", "remove", "replace"])
    );
    let source_targets = &source["properties"]["targets"];
    assert_eq!(source_targets["type"], "array");
    assert_eq!(source_targets["minItems"], 1);
    let source_variants = source_targets["items"]["oneOf"]
        .as_array()
        .expect("closed logical event source target union");
    assert_eq!(source_variants.len(), 6);
    assert_eq!(
        source_variants
            .iter()
            .map(|variant| variant["properties"]["kind"]["const"]
                .as_str()
                .expect("event source kind"))
            .collect::<HashSet<_>>(),
        HashSet::from(["object", "manager", "recordSet", "definedType", "family",]),
    );
    assert!(source_variants.iter().all(|variant| {
        variant["type"] == "object" && variant["additionalProperties"] == false
    }));
    let manager_variants = source_variants
        .iter()
        .filter(|variant| variant["properties"]["kind"]["const"] == "manager")
        .collect::<Vec<_>>();
    assert_eq!(manager_variants.len(), 2);
    assert!(manager_variants.iter().any(|variant| {
        variant["required"] == json!(["kind", "metadataPath", "sourceClass"])
            && variant["properties"]["sourceClass"]["enum"]
                == json!(["constantManager", "constantValueManager"])
    }));

    for name in ["owners", "registerRecords", "basedOn", "inputByString"] {
        let ordinary = relation(name);
        assert_eq!(
            ordinary["properties"]["mode"]["enum"],
            json!(["add", "remove", "replace"]),
            "{name} modes"
        );
        assert_eq!(ordinary["properties"]["targets"]["minItems"], 1);
    }
    for name in ["owners", "registerRecords", "basedOn"] {
        assert_eq!(
            relation(name)["properties"]["targets"]["items"]["required"],
            json!(["metadataPath"]),
            "{name} target shape"
        );
    }
    assert_eq!(
        relation("inputByString")["properties"]["targets"]["items"]["required"],
        json!(["fieldPath"]),
    );

    let validator = jsonschema::validator_for(&unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    ))
    .expect("published edit schema");
    let call = |relation: &str, mode: &str, targets: Value| {
        json!({
            "sourceSet": "main",
            "metadataPath": "EventSubscription.Notify",
            "operations": [{
                "op": "editRelations",
                "relation": relation,
                "mode": mode,
                "targets": targets,
            }],
        })
    };
    assert!(!validator.is_valid(&call("source", "replace", json!([]))));
    for invalid in [
        call("source", "add", json!([{"kind": "boolean"}])),
        call(
            "source",
            "replace",
            json!([{"metadataPath": "Catalog.Items"}]),
        ),
        call("owners", "replace", json!([])),
        call("owners", "replace", json!([{"kind": "boolean"}])),
        call(
            "inputByString",
            "replace",
            json!([{"metadataPath": "Catalog.Items"}]),
        ),
    ] {
        assert!(!validator.is_valid(&invalid), "schema accepted {invalid}");
    }
}

#[test]
fn property_schema_publishes_closed_enum_domains_for_retired_scalars() {
    let values = published_root_property_values(unica_metadata::request::MetadataOperation::Edit);
    let properties = values["properties"]
        .as_object()
        .expect("published values object carries a closed property registry")
        .clone();

    assert_eq!(
        properties["HierarchyType"]["enum"],
        json!(["HierarchyFoldersAndItems", "HierarchyOfItems"]),
    );
    assert_eq!(
        properties["RegisterRecordsDeletion"]["enum"],
        json!(["AutoDelete", "AutoDeleteOnUnpost", "AutoDeleteOff"]),
    );
    assert_eq!(values["additionalProperties"], json!(false));

    // One public name whose legal values differ by kind publishes the union
    // of every per-kind domain, so the schema stays a superset the writer
    // then narrows. `Periodicity` is the case that proves it: the register
    // domains differ and neither may be silently dropped.
    let periodicity = properties["Periodicity"]["enum"]
        .as_array()
        .expect("Periodicity publishes a closed domain")
        .iter()
        .map(|value| value.as_str().expect("domain members are strings"))
        .collect::<Vec<_>>();
    for member in [
        "Nonperiodical",
        "Second",
        "Day",
        "Month",
        "Quarter",
        "Year",
        "RecorderPosition",
    ] {
        assert!(
            periodicity.contains(&member),
            "published Periodicity domain lost {member}"
        );
    }
    assert_eq!(
        periodicity.len(),
        periodicity.iter().collect::<HashSet<_>>().len(),
        "published domain repeats a member"
    );
}

#[test]
fn property_schema_publishes_the_platform_register_type_domain() {
    let values = published_root_property_values(unica_metadata::request::MetadataOperation::Edit);
    let properties = values["properties"].as_object().unwrap();

    assert_eq!(
        properties["RegisterType"],
        json!({"type": "string", "enum": ["Balance", "Turnovers"]})
    );
    assert_eq!(
        properties["IncludeHelpInContents"],
        json!({"type": "boolean"})
    );
}

#[test]
fn common_module_ordinary_client_property_is_shared_by_add_and_edit() {
    let calls = [
        (
            unica_metadata::request::MetadataOperation::Add,
            json!({
                "sourceSet": "main",
                "kind": "CommonModule",
                "name": "OrdinaryClient",
                "operations": [{
                    "op": "setProperties",
                    "values": {"ClientOrdinaryApplication": true},
                }],
            }),
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "CommonModule.OrdinaryClient",
                "operations": [{
                    "op": "setProperties",
                    "values": {"ClientOrdinaryApplication": true},
                }],
            }),
        ),
    ];

    for (operation, call) in calls {
        let values = published_root_property_values(operation);
        assert_eq!(
            values["properties"]["ClientOrdinaryApplication"],
            json!({"type": "boolean"}),
            "{operation:?}"
        );
        assert_eq!(
            values["additionalProperties"],
            json!(false),
            "{operation:?}"
        );
        unica_metadata::request::parse_metadata_request(operation, &object(call))
            .unwrap_or_else(|failure| panic!("{operation:?}: {failure:?}"));
    }
}

#[test]
fn add_and_edit_report_the_same_supported_common_module_properties() {
    let calls = [
        (
            unica_metadata::request::MetadataOperation::Add,
            json!({
                "sourceSet": "main",
                "kind": "CommonModule",
                "name": "OrdinaryClient",
                "operations": [{
                    "op": "setProperties",
                    "values": {"ClientOrdinaryApplications": true},
                }],
            }),
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "CommonModule.OrdinaryClient",
                "operations": [{
                    "op": "setProperties",
                    "values": {"ClientOrdinaryApplications": true},
                }],
            }),
        ),
    ];
    let expected = "unknown metadata property `ClientOrdinaryApplications`; supported properties for CommonModule: Synonym, Comment, ClientManagedApplication, ClientOrdinaryApplication, ExternalConnection, Global, Privileged, ReturnValuesReuse, Server, ServerCall";

    for (operation, call) in calls {
        let error = diagnostic(operation, call);
        assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(error.message, expected, "{operation:?}");
        assert_eq!(
            error.field.as_deref(),
            Some("values.ClientOrdinaryApplications"),
            "{operation:?}"
        );
    }
}

#[test]
fn property_parser_rejects_an_enum_outside_the_exact_kind_domain() {
    let error = diagnostic(
        unica_metadata::request::MetadataOperation::Edit,
        json!({
            "sourceSet": "main",
            "metadataPath": "CalculationRegister.Payroll",
            "operations": [{
                "op": "setProperties",
                "values": {"Periodicity": "Nonperiodical"},
            }],
        }),
    );

    assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
    assert_eq!(error.field.as_deref(), Some("values.Periodicity"));
    assert!(error.message.contains("expected one of"), "{error:?}");
}

#[test]
fn schema_rejects_operations_the_closed_domain_rejects() {
    let cases = [
        json!({
            "sourceSet": "main",
            "metadataPath": "CalculationRegister.Payroll",
            "operations": [{
                "op": "setProperties",
                "values": {"Periodicity": "Nonperiodical"},
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Enum.Statuses",
            "operations": [{
                "op": "update",
                "collection": "enumValues",
                "elements": [{
                    "name": "Open",
                    "type": {"variants": [{"kind": "boolean"}]},
                }],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "remove",
                "collection": "forms",
                "scope": {"tabularSection": "Lines"},
                "names": ["MainForm"],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "update",
                "collection": "attributes",
                "elements": [{"name": "Customer"}],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Enum.Statuses",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{"name": "Code"}],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Value",
                    "type": {"variants": [
                        {"kind": "string", "length": 10, "allowedLength": "variable"},
                        {"kind": "string", "length": 20, "allowedLength": "variable"},
                    ]},
                }],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Value",
                    "type": {"variants": [
                        {"kind": "valueStorage"},
                        {"kind": "boolean"},
                    ]},
                }],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Value",
                    "type": {"variants": [{
                        "kind": "string",
                        "length": 0,
                        "allowedLength": "fixed",
                    }]},
                }],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Value",
                    "type": {"variants": [{
                        "kind": "binaryData",
                        "length": 0,
                        "allowedLength": "fixed",
                    }]},
                }],
            }],
        }),
    ];
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();

    for call in cases {
        assert!(
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Edit,
                call.as_object().unwrap()
            )
            .is_err(),
            "closed conversion unexpectedly accepted {call}"
        );
        // The union may admit what the closed conversion refuses; it may
        // never refuse what the conversion admits.
        let _ = &validator;
    }
}

#[test]
fn binary_data_length_schema_and_parser_follow_the_closed_domain_bounds() {
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();
    let call = |length| {
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Payload",
                    "type": {"variants": [{
                        "kind": "binaryData",
                        "length": length,
                        "allowedLength": "fixed",
                    }]},
                }],
            }],
        })
    };

    for length in [2_000_u64, u64::from(u32::MAX)] {
        let valid = call(length);
        assert!(
            validator.is_valid(&valid),
            "schema rejected runtime-valid binaryData length: {valid}"
        );
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            valid.as_object().unwrap(),
        )
        .unwrap_or_else(|error| panic!("parser rejected {valid}: {error:?}"));
    }

    let too_large = call(u64::from(u32::MAX) + 1);
    assert!(
        !validator.is_valid(&too_large),
        "schema accepted binaryData length above u32: {too_large}"
    );
    assert!(
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            too_large.as_object().unwrap()
        )
        .is_err(),
        "parser accepted binaryData length above u32: {too_large}"
    );

    let invalid = call(0);
    assert!(
        !validator.is_valid(&invalid),
        "schema accepted zero-length fixed binaryData: {invalid}"
    );
    assert!(
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            invalid.as_object().unwrap()
        )
        .is_err(),
        "parser accepted zero-length fixed binaryData: {invalid}"
    );
}

#[test]
fn number_fraction_schema_and_parser_enforce_every_digits_boundary() {
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();
    let call = |digits: u64, fraction: u64| {
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Amount",
                    "type": {"variants": [{
                        "kind": "number",
                        "digits": digits,
                        "fraction": fraction,
                        "sign": "any",
                    }]},
                }],
            }],
        })
    };
    let assert_case = |digits, fraction, expected| {
        let call = call(digits, fraction);
        assert_eq!(
            validator.is_valid(&call),
            expected,
            "published schema disagrees for digits={digits}, fraction={fraction}: {call}"
        );
        assert_eq!(
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Edit,
                call.as_object().unwrap()
            )
            .is_ok(),
            expected,
            "parser disagrees for digits={digits}, fraction={fraction}: {call}"
        );
    };

    for digits in 0_u64..=38 {
        assert_case(digits, digits, true);
        if digits < 38 {
            assert_case(digits, digits + 1, false);
        }
    }
    for (digits, fraction, expected) in [
        (10, 2, true),
        (38, 0, true),
        (38, 39, false),
        (39, 0, false),
    ] {
        assert_case(digits, fraction, expected);
    }
}

#[test]
fn parser_accepts_every_json_lexeme_that_schema_integer_accepts() {
    let edit_schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let edit_validator = jsonschema::validator_for(&edit_schema).unwrap();
    for raw in [
        r#"{
            "sourceSet":"main",
            "metadataPath":"Document.Order",
            "operations":[{
                "op":"add",
                "collection":"attributes",
                "elements":[{
                    "name":"Text",
                    "type":{"variants":[{
                        "kind":"string",
                        "length":1.0,
                        "allowedLength":"fixed"
                    }]}
                }]
            }]
        }"#,
        r#"{
            "sourceSet":"main",
            "metadataPath":"Document.Order",
            "operations":[{
                "op":"add",
                "collection":"attributes",
                "elements":[{
                    "name":"Payload",
                    "type":{"variants":[{
                        "kind":"binaryData",
                        "length":1e0,
                        "allowedLength":"fixed"
                    }]}
                }]
            }]
        }"#,
        r#"{
            "sourceSet":"main",
            "metadataPath":"Document.Order",
            "operations":[{
                "op":"add",
                "collection":"attributes",
                "elements":[{
                    "name":"Amount",
                    "type":{"variants":[{
                        "kind":"number",
                        "digits":1.0,
                        "fraction":0.0,
                        "sign":"any"
                    }]}
                }]
            }]
        }"#,
        r#"{
            "sourceSet":"main",
            "metadataPath":"Document.Order",
            "operations":[{
                "op":"setProperties",
                "values":{"NumberLength":1e0}
            }]
        }"#,
    ] {
        let call: Value = serde_json::from_str(raw).unwrap();
        assert!(edit_validator.is_valid(&call), "schema rejected {call}");
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            call.as_object().unwrap(),
        )
        .unwrap_or_else(|error| panic!("parser rejected {call}: {error:?}"));
    }

    let info: Value =
        serde_json::from_str(r#"{"sourceSet":"main","metadataPath":"Document.Order","limit":1e0}"#)
            .unwrap();
    let info_schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Info,
    );
    assert!(jsonschema::validator_for(&info_schema)
        .unwrap()
        .is_valid(&info));
    unica_metadata::request::parse_metadata_request(
        unica_metadata::request::MetadataOperation::Info,
        info.as_object().unwrap(),
    )
    .unwrap();
}

#[test]
fn schema_and_parser_reject_non_u32_json_numbers() {
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();
    for number in ["1.5", "-1.0", "4294967296.0"] {
        let raw = format!(
            r#"{{
                "sourceSet":"main",
                "metadataPath":"Document.Order",
                "operations":[{{
                    "op":"add",
                    "collection":"attributes",
                    "elements":[{{
                        "name":"Payload",
                        "type":{{"variants":[{{
                            "kind":"binaryData",
                            "length":{number},
                            "allowedLength":"fixed"
                        }}]}}
                    }}]
                }}]
            }}"#,
        );
        let call: Value = serde_json::from_str(&raw).unwrap();
        assert!(!validator.is_valid(&call), "schema accepted {call}");
        assert!(
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Edit,
                call.as_object().unwrap()
            )
            .is_err(),
            "parser accepted {call}"
        );
    }
}

#[test]
fn parser_rejects_a_collection_outside_the_owner_kind_registry() {
    let call = json!({
        "sourceSet": "main",
        "metadataPath": "Enum.Statuses",
        "operations": [{
            "op": "add",
            "collection": "attributes",
            "elements": [{"name": "Code"}],
        }],
    });

    let error = diagnostic(unica_metadata::request::MetadataOperation::Edit, call);
    assert_eq!(error.code, MetaDiagnosticCode::UnsupportedKind);
    assert_eq!(error.field.as_deref(), Some("collection"));
}

#[test]
fn edit_schema_and_parser_share_the_top_level_address_alias_contract() {
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();
    let call = |metadata_path: &str| {
        json!({
            "sourceSet": "main",
            "metadataPath": metadata_path,
            "operations": [{
                "op": "setProperties",
                "values": {"Comment": "typed"},
            }],
        })
    };

    for metadata_path in ["Catalog.Items", "Справочник.Товары"] {
        let call = call(metadata_path);
        assert!(validator.is_valid(&call), "schema rejected {call}");
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            call.as_object().unwrap(),
        )
        .unwrap_or_else(|error| panic!("parser rejected {call}: {error:?}"));
    }

    for metadata_path in [
        "Catalogs.Items",
        "Справочники.Товары",
        "Catalog.Items.Form.List",
    ] {
        let call = call(metadata_path);
        assert!(!validator.is_valid(&call), "schema accepted {call}");
        assert!(
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Edit,
                call.as_object().unwrap()
            )
            .is_err(),
            "parser accepted {call}"
        );
    }
}

#[test]
fn schema_and_parser_accept_every_registered_kind_collection_and_operation_tag() {
    fn add_element(kind: MetadataKind, collection: MetaCollection) -> Value {
        let mut element = json!({
            "name": "Element",
            "synonym": "Element synonym",
            "comment": "Element comment",
            "position": {"after": "Previous"},
        })
        .as_object()
        .unwrap()
        .clone();
        match collection {
            MetaCollection::Attributes | MetaCollection::Dimensions | MetaCollection::Resources => {
                element.insert("type".into(), json!({"variants": [{"kind": "boolean"}]}));
                element.insert("required".into(), json!(true));
                if metadata_fill_value_is_allowed(kind, collection, MetaElementScope::TopLevel) {
                    element.insert(
                        "fillValue".into(),
                        json!({"kind": "boolean", "value": false}),
                    );
                }
            }
            MetaCollection::TabularSections => {
                let mut nested = json!({
                    "name": "Nested",
                    "type": {"variants": [{"kind": "boolean"}]},
                    "required": true,
                })
                .as_object()
                .unwrap()
                .clone();
                if metadata_fill_value_is_allowed(
                    kind,
                    MetaCollection::Attributes,
                    MetaElementScope::TabularSection,
                ) {
                    nested.insert(
                        "fillValue".into(),
                        json!({"kind": "boolean", "value": false}),
                    );
                }
                element.insert(
                    "attributes".into(),
                    Value::Array(vec![Value::Object(nested)]),
                );
            }
            MetaCollection::Columns => {
                element.insert("type".into(), json!({"variants": [{"kind": "boolean"}]}));
            }
            MetaCollection::EnumValues
            | MetaCollection::Forms
            | MetaCollection::Templates
            | MetaCollection::Commands => {}
            MetaCollection::PredefinedItems => {
                unreachable!("predefined items use their own typed element schema")
            }
        }
        Value::Object(element)
    }

    fn update_element(kind: MetadataKind, collection: MetaCollection) -> Value {
        let mut element = json!({
            "name": "Element",
            "newName": "Renamed",
            "synonym": "Replacement synonym",
            "comment": "Replacement comment",
            "position": {"before": "Next"},
        })
        .as_object()
        .unwrap()
        .clone();
        match collection {
            MetaCollection::Attributes | MetaCollection::Dimensions | MetaCollection::Resources => {
                element.insert("type".into(), json!({"variants": [{"kind": "boolean"}]}));
                element.insert("required".into(), json!(false));
                if metadata_fill_value_is_allowed(kind, collection, MetaElementScope::TopLevel) {
                    element.insert(
                        "fillValue".into(),
                        json!({"kind": "boolean", "value": true}),
                    );
                }
            }
            MetaCollection::Columns => {
                element.insert("type".into(), json!({"variants": [{"kind": "boolean"}]}));
            }
            MetaCollection::TabularSections
            | MetaCollection::EnumValues
            | MetaCollection::Forms
            | MetaCollection::Templates
            | MetaCollection::Commands => {}
            MetaCollection::PredefinedItems => {
                unreachable!("predefined items use their own typed element schema")
            }
        }
        Value::Object(element)
    }

    let add_schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Add,
    );
    let add_validator = jsonschema::validator_for(&add_schema).unwrap();
    let edit_schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let edit_validator = jsonschema::validator_for(&edit_schema).unwrap();
    for kind in MetadataKind::ALL {
        let mut common_operations =
            vec![json!({"op": "setProperties", "values": {"Comment": "typed"}})];
        if let Some(spec) = metadata_relation_specs(*kind).first() {
            let target = match spec.target_policy {
                MetaRelationTargetPolicy::MetadataKinds(kinds) => {
                    json!({"metadataPath": format!("{}.Target", kinds[0].as_str())})
                }
                MetaRelationTargetPolicy::SameOwnerField => json!({
                    "fieldPath": format!("{}.Object.Attribute.Code", kind.as_str()),
                }),
                MetaRelationTargetPolicy::EventSources => {
                    json!({"kind": "family", "sourceClass": "catalogObject"})
                }
            };
            common_operations.push(json!({
                "op": "editRelations",
                "relation": spec.relation.as_str(),
                "mode": if spec.target_policy == MetaRelationTargetPolicy::EventSources {
                    "replace"
                } else {
                    "add"
                },
                "targets": [target],
            }));
        }
        let collection_operations = unica_metadata::metadata_kind_collections(*kind)
            .iter()
            .flat_map(|collection| {
                let collection_name = collection.as_str();
                if *collection == MetaCollection::PredefinedItems {
                    let id = "c7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
                    return vec![
                        json!({
                            "op": "add",
                            "collection": collection_name,
                            "elements": [{"id": id, "name": "Element"}],
                        }),
                        json!({
                            "op": "update",
                            "collection": collection_name,
                            "elements": [{"id": id, "description": "Updated"}],
                        }),
                        json!({
                            "op": "remove",
                            "collection": collection_name,
                            "ids": [id],
                        }),
                    ];
                }
                vec![
                    json!({
                        "op": "add",
                        "collection": collection_name,
                        "elements": [add_element(*kind, *collection)],
                    }),
                    json!({
                        "op": "update",
                        "collection": collection_name,
                        "elements": [update_element(*kind, *collection)],
                    }),
                    json!({
                        "op": "remove",
                        "collection": collection_name,
                        "names": ["Element"],
                    }),
                ]
            });

        for operation in common_operations.into_iter().chain(collection_operations) {
            let add_call = json!({
                "sourceSet": "main",
                "kind": kind.as_str(),
                "name": "Object",
                "operations": [operation.clone()],
            });
            assert!(
                add_validator.is_valid(&add_call),
                "published add schema rejected {add_call}"
            );
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Add,
                add_call.as_object().unwrap(),
            )
            .unwrap_or_else(|error| panic!("conversion rejected {add_call}: {error:?}"));

            let edit_call = json!({
                "sourceSet": "main",
                "metadataPath": format!("{}.Object", kind.as_str()),
                "operations": [operation],
            });
            assert!(
                edit_validator.is_valid(&edit_call),
                "published edit schema rejected {edit_call}"
            );
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Edit,
                edit_call.as_object().unwrap(),
            )
            .unwrap_or_else(|error| panic!("conversion rejected {edit_call}: {error:?}"));
        }

        if unica_metadata::metadata_kind_collections(*kind).contains(&MetaCollection::Attributes) {
            for operation in [
                json!({
                    "op": "add",
                    "collection": "attributes",
                    "scope": {"tabularSection": "Lines"},
                    "elements": [{"name": "Nested"}],
                }),
                json!({
                    "op": "update",
                    "collection": "attributes",
                    "scope": {"tabularSection": "Lines"},
                    "elements": [{"name": "Nested", "comment": "changed"}],
                }),
                json!({
                    "op": "remove",
                    "collection": "attributes",
                    "scope": {"tabularSection": "Lines"},
                    "names": ["Nested"],
                }),
            ] {
                let call = json!({
                    "sourceSet": "main",
                    "metadataPath": format!("{}.Object", kind.as_str()),
                    "operations": [operation],
                });
                assert!(
                    edit_validator.is_valid(&call),
                    "published edit schema rejected scoped operation {call}"
                );
                unica_metadata::request::parse_metadata_request(
                    unica_metadata::request::MetadataOperation::Edit,
                    call.as_object().unwrap(),
                )
                .unwrap_or_else(|error| panic!("conversion rejected {call}: {error:?}"));
            }
        }
    }
}

#[test]
fn edit_rejects_relation_target_cross_products() {
    let invalid_owners_target = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Items",
        "operations": [{
            "op": "editRelations",
            "relation": "owners",
            "mode": "add",
            "targets": [{"fieldPath": "Catalog.Items.StandardAttribute.Code"}],
        }],
    });
    let invalid_input_by_string_target = json!({
        "sourceSet": "main",
        "metadataPath": "Document.Order",
        "operations": [{
            "op": "editRelations",
            "relation": "inputByString",
            "mode": "add",
            "targets": [{"metadataPath": "Catalog.Items"}],
        }],
    });

    // The published union types `targets` as objects because the legal
    // target shape follows the relation, which the kind-agnostic schema
    // does not resolve. The writer is the arbiter and names the exact
    // offending target.
    for (call, field) in [
        (invalid_owners_target, "targets[0].fieldPath"),
        (invalid_input_by_string_target, "targets[0].metadataPath"),
    ] {
        assert!(
            !validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &call),
            "relation target cross-product was accepted: {call}"
        );
        let error = diagnostic(unica_metadata::request::MetadataOperation::Edit, call);
        assert_eq!(error.operation_index, Some(0), "{error:?}");
        assert_eq!(error.field.as_deref(), Some(field), "{error:?}");
    }
}

#[test]
fn schema_and_parser_follow_every_owner_relation_profile() {
    use unica_metadata::{metadata_relation_specs, MetaRelationTargetPolicy};

    let add_schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Add,
    );
    let add_validator = jsonschema::validator_for(&add_schema).unwrap();
    let edit_schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let edit_validator = jsonschema::validator_for(&edit_schema).unwrap();

    for kind in MetadataKind::ALL.iter().copied() {
        let owner = format!("{}.Object", kind.as_str());
        for spec in metadata_relation_specs(kind) {
            let target = match spec.target_policy {
                MetaRelationTargetPolicy::MetadataKinds(kinds) => {
                    json!({"metadataPath": format!("{}.Target", kinds[0].as_str())})
                }
                MetaRelationTargetPolicy::SameOwnerField => json!({
                    "fieldPath": format!("{}.Attribute.Code", owner),
                }),
                MetaRelationTargetPolicy::EventSources => {
                    json!({"kind": "family", "sourceClass": "catalogObject"})
                }
            };
            let operation = json!({
                "op": "editRelations",
                "relation": spec.relation.as_str(),
                "mode": "replace",
                "targets": [target],
            });
            let add_call = json!({
                "sourceSet": "main",
                "kind": kind.as_str(),
                "name": "Object",
                "operations": [operation.clone()],
            });
            assert!(
                add_validator.is_valid(&add_call),
                "schema rejected {add_call}"
            );
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Add,
                add_call.as_object().unwrap(),
            )
            .unwrap_or_else(|error| panic!("parser rejected {add_call}: {error:?}"));

            let edit_call = json!({
                "sourceSet": "main",
                "metadataPath": owner,
                "operations": [operation],
            });
            assert!(
                edit_validator.is_valid(&edit_call),
                "schema rejected {edit_call}"
            );
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Edit,
                edit_call.as_object().unwrap(),
            )
            .unwrap_or_else(|error| panic!("parser rejected {edit_call}: {error:?}"));
        }
    }

    let cross_kind_based_on = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Items",
        "operations": [{
            "op": "editRelations",
            "relation": "basedOn",
            "mode": "add",
            "targets": [{"metadataPath": "Document.Order"}],
        }],
    });
    assert!(
        validate_schema_and_parse(
            unica_metadata::request::MetadataOperation::Edit,
            &cross_kind_based_on
        ),
        "the platform BasedOn generation matrix permits cross-kind targets"
    );
}

#[test]
fn relation_readers_preserve_conversion_and_unknown_field_diagnostic_order() {
    let call = |relation: Value, mode: Value, targets: Value| {
        json!({
            "op": "editRelations", "relation": relation, "mode": mode, "targets": targets,
        })
    };
    for (operation, field, message) in [
        (
            call(json!("unknown"), json!(false), json!(false)),
            "relation",
            "unsupported metadata relation `unknown`",
        ),
        (
            call(json!("owners"), json!("unknown"), json!(false)),
            "mode",
            "unsupported relation mode `unknown`",
        ),
        (
            call(
                json!("owners"),
                json!("add"),
                json!([{"metadataPath": false, "extra": true}]),
            ),
            "targets[0].extra",
            "unknown field `extra`",
        ),
        (
            call(
                json!("inputByString"),
                json!("add"),
                json!([{"fieldPath": false, "extra": true}]),
            ),
            "targets[0].extra",
            "unknown field `extra`",
        ),
        (
            call(
                json!("source"),
                json!("add"),
                json!([{"kind": "unknown", "metadataPath": false, "extra": true}]),
            ),
            "targets[0].kind",
            "unsupported event source kind `unknown`",
        ),
        (
            call(
                json!("source"),
                json!("add"),
                json!([{"kind": "manager", "metadataPath": false, "sourceClass": false, "extra": true}]),
            ),
            "targets[0].metadataPath",
            "`targets[0].metadataPath` must be a string",
        ),
        (
            call(
                json!("source"),
                json!("add"),
                json!([{"kind": "manager", "metadataPath": "Catalog.Items", "sourceClass": false, "extra": true}]),
            ),
            "targets[0].sourceClass",
            "sourceClass must be a string",
        ),
        (
            call(
                json!("source"),
                json!("add"),
                json!([{"kind": "family", "sourceClass": false, "extra": true}]),
            ),
            "targets[0].sourceClass",
            "`targets[0].sourceClass` must be a string",
        ),
        (
            call(
                json!("source"),
                json!("add"),
                json!([{"kind": "object", "metadataPath": "Catalog.Items", "extra": true}]),
            ),
            "targets[0].extra",
            "unknown field `extra`",
        ),
    ] {
        let error = parse_edit_operation(&operation, MetadataKind::Catalog).unwrap_err();
        assert_eq!(error.field.as_deref(), Some(field), "{operation}");
        assert_eq!(error.message, message, "{operation}");
    }

    let error = diagnostic(
        unica_metadata::request::MetadataOperation::Edit,
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Items",
            "operations": [
                {"op": "editRelations", "relation": "owners", "mode": "add", "targets": [{"metadataPath": "Catalog.Parent"}]},
                {"op": "editRelations", "relation": "owners", "mode": "unknown", "targets": false},
            ],
        }),
    );
    assert_eq!(error.operation_index, Some(1));
    assert_eq!(error.field.as_deref(), Some("mode"));
    assert_eq!(error.message, "unsupported relation mode `unknown`");
}

#[test]
fn raw_event_source_manager_keeps_optional_class_before_domain_correlation() {
    for (target, class) in [
        (
            json!({"kind": "manager", "metadataPath": "Constant.Flag"}),
            None,
        ),
        (
            json!({"kind": "manager", "metadataPath": "Catalog.Items", "sourceClass": "constantManager"}),
            Some(EventSourceClass::ConstantManager),
        ),
    ] {
        let parsed = parse_event_source(&target, "targets[1]").unwrap();
        assert!(
            matches!(parsed, MetaEventSource::Manager { source_class, .. } if source_class == class)
        );
    }
    let error = parse_event_source(&json!({
        "kind": "manager", "metadataPath": "Catalog.Items", "sourceClass": "unknown", "extra": true,
    }), "targets[1]").unwrap_err();
    assert_eq!(error.field.as_deref(), Some("targets[1].sourceClass"));
    assert_eq!(error.message, "unsupported event source class `unknown`");
}

#[test]
fn source_parser_accepts_the_closed_nonempty_logical_event_source_algebra() {
    let call = |targets: Value| {
        json!({
            "sourceSet": "main",
            "metadataPath": "EventSubscription.Notify",
            "operations": [{
                "op": "editRelations",
                "relation": "source",
                "mode": "replace",
                "targets": targets,
            }],
        })
    };
    let all_logical_sources = call(json!([
        {"kind": "object", "metadataPath": "Document.Order"},
        {"kind": "manager", "metadataPath": "Catalog.Items"},
        {"kind": "recordSet", "metadataPath": "InformationRegister.Facts"},
        {"kind": "definedType", "metadataPath": "DefinedType.Identifier"},
        {"kind": "family", "sourceClass": "catalogObject"}
    ]));
    assert!(validate_schema_and_parse(
        unica_metadata::request::MetadataOperation::Edit,
        &all_logical_sources
    ));
    let unica_metadata::request::MetadataRequest::Edit(request) =
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            all_logical_sources.as_object().unwrap(),
        )
        .unwrap()
    else {
        panic!("expected edit request")
    };
    let MetaEditOperation::EditRelations {
        relation,
        mode,
        targets,
    } = &request.operations[0]
    else {
        panic!("expected relation operation")
    };
    assert_eq!(*relation, MetaRelation::Source);
    assert_eq!(*mode, RelationEditMode::Replace);
    assert_eq!(targets.len(), 5);
    assert!(targets
        .iter()
        .all(|target| matches!(target, MetaRelationTarget::EventSource(_))));

    for target in [
        json!({"kind": "manager", "metadataPath": "Constant.Mode", "sourceClass": "constantManager"}),
        json!({"kind": "manager", "metadataPath": "Constant.Mode", "sourceClass": "constantValueManager"}),
        json!({"kind": "manager", "metadataPath": "FilterCriterion.Active"}),
        json!({"kind": "manager", "metadataPath": "SettingsStorage.User"}),
        json!({"kind": "recordSet", "metadataPath": "Sequence.Documents"}),
        json!({"kind": "recordSet", "metadataPath": "CalculationRegister.Payroll.Recalculation.Main"}),
        json!({"kind": "recordSet", "metadataPath": "CalculationRegister.Payroll.Recalculations.Main"}),
        json!({"kind": "object", "metadataPath": "Справочник.Номенклатура"}),
        json!({"kind": "manager", "metadataPath": "ХранилищеНастроек.Пользователь"}),
        json!({"kind": "recordSet", "metadataPath": "РегистрРасчёта.Зарплата.Перерасчёт.Основной"}),
        json!({"kind": "recordSet", "metadataPath": "РегистрРасчёта.Зарплата.Перерасчёты.Основной"}),
    ] {
        let request = call(json!([target]));
        assert!(
            validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &request),
            "logical platform source was rejected: {request}"
        );
    }

    for target in [
        json!({"kind": "manager", "metadataPath": "Constant.Mode"}),
        json!({"kind": "manager", "metadataPath": "Catalog.Items", "sourceClass": "catalogManager"}),
        json!({"kind": "object", "metadataPath": "ExternalDataSource.Remote.Table.Items"}),
    ] {
        let request = call(json!([target]));
        assert!(
            !validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &request),
            "ambiguous or unaddressable source was accepted: {request}"
        );
    }

    let empty = call(json!([]));
    assert!(
        !validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &empty),
        "a published binding cannot have an empty source: {empty}"
    );

    let unicode_name = call(json!([
        {"kind": "object", "metadataPath": "Catalog.Основной"}
    ]));
    assert!(
        validate_schema_and_parse(
            unica_metadata::request::MetadataOperation::Edit,
            &unicode_name
        ),
        "source metadata names retain the Unicode XML NCName range"
    );
}

#[test]
fn source_parser_rejects_modes_composites_duplicates_and_incompatible_paths() {
    let call = |mode: &str, targets: Value| {
        json!({
            "sourceSet": "main",
            "metadataPath": "EventSubscription.Notify",
            "operations": [{
                "op": "editRelations",
                "relation": "source",
                "mode": mode,
                "targets": targets,
            }],
        })
    };
    for mode in ["add", "remove", "replace"] {
        assert!(validate_schema_and_parse(
            unica_metadata::request::MetadataOperation::Edit,
            &call(
                mode,
                json!([{"kind":"family", "sourceClass":"catalogObject"}])
            )
        ));
    }
    for invalid_call in [
        call(
            "append",
            json!([{"kind": "family", "sourceClass": "catalogObject"}]),
        ),
        call(
            "replace",
            json!([
                {"kind": "object", "metadataPath": "Catalog.Items"},
                {"kind": "object", "metadataPath": "Catalog.items"}
            ]),
        ),
        call(
            "replace",
            json!([{"kind": "recordSet", "metadataPath": "Catalog.Items"}]),
        ),
        call(
            "replace",
            json!([{"kind": "object", "metadataPath": "Catalog.Bad Name"}]),
        ),
        call(
            "replace",
            json!([{"kind": "object", "metadataPath": "Catalog.1Bad"}]),
        ),
        call(
            "replace",
            json!([{"kind": "object", "metadataPath": "Catalog.Bad:Name"}]),
        ),
        call(
            "replace",
            json!([{"kind": "object", "metadataPath": "Catalog.Bad-Name"}]),
        ),
        call(
            "replace",
            json!([{"kind": "family", "sourceClass": "unknownClass"}]),
        ),
    ] {
        assert!(
            !validate_schema_and_parse(
                unica_metadata::request::MetadataOperation::Edit,
                &invalid_call
            ),
            "invalid source operation was accepted: {invalid_call}"
        );
    }

    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();
    for invalid_call in [
        call("replace", json!([])),
        call("replace", json!([{"kind": "boolean"}])),
        call(
            "replace",
            json!([{"kind": "reference", "metadataPath": "Catalog.Items"}]),
        ),
    ] {
        assert!(
            !validator.is_valid(&invalid_call),
            "published schema accepted an impossible source state: {invalid_call}"
        );
    }

    let ordinary_clear = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Items",
        "operations": [{
            "op": "editRelations",
            "relation": "owners",
            "mode": "replace",
            "targets": [],
        }],
    });
    assert!(!validate_schema_and_parse(
        unica_metadata::request::MetadataOperation::Edit,
        &ordinary_clear
    ));

    let wrong_owner = json!({
        "sourceSet": "main",
        "metadataPath": "Document.Order",
        "operations": [{
            "op": "editRelations",
            "relation": "source",
            "mode": "replace",
            "targets": [{"kind": "boolean"}],
        }],
    });
    assert!(!validate_schema_and_parse(
        unica_metadata::request::MetadataOperation::Edit,
        &wrong_owner
    ));
}

#[test]
fn source_target_schema_rejects_open_or_retired_shapes() {
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();
    let call = |target: Value| {
        json!({
            "sourceSet": "main",
            "metadataPath": "EventSubscription.Notify",
            "operations": [{
                "op": "editRelations",
                "relation": "source",
                "mode": "replace",
                "targets": [target],
            }],
        })
    };
    for target in [
        json!({"kind": "boolean"}),
        json!({"kind": "reference", "metadataPath": "Catalog.Items"}),
        json!({"kind": "family", "sourceClass": "catalogObject", "extra": true}),
        json!({"kind": "family", "sourceClass": "unknownClass"}),
        json!({"kind": "recordSet"}),
        json!({"generatedType": "InformationRegisterRecordSet.Facts"}),
        json!("InformationRegisterRecordSet.Facts"),
    ] {
        let call = call(target);
        assert!(!validator.is_valid(&call), "schema accepted {call}");
        assert!(
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Edit,
                call.as_object().unwrap()
            )
            .is_err(),
            "parser accepted {call}"
        );
    }
}

#[test]
fn schema_and_parser_reject_owner_relation_cross_products() {
    let calls = [
        json!({
            "sourceSet": "main",
            "metadataPath": "CommonModule.Utility",
            "operations": [{
                "op": "editRelations",
                "relation": "basedOn",
                "mode": "add",
                "targets": [{"metadataPath": "CommonModule.Base"}],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Items",
            "operations": [{
                "op": "editRelations",
                "relation": "owners",
                "mode": "add",
                "targets": [{"metadataPath": "Document.Order"}],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "editRelations",
                "relation": "registerRecords",
                "mode": "add",
                "targets": [{"metadataPath": "Catalog.Items"}],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Items",
            "operations": [{
                "op": "editRelations",
                "relation": "basedOn",
                "mode": "add",
                "targets": [{"metadataPath": "InformationRegister.Facts"}],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "editRelations",
                "relation": "inputByString",
                "mode": "add",
                "targets": [{"fieldPath": "Catalog.Items.Attribute.Code"}],
            }],
        }),
    ];

    for call in calls {
        assert!(
            !validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &call),
            "owner relation cross-product was accepted: {call}"
        );
    }
}

#[test]
fn input_by_string_exact_owner_name_is_a_parser_constraint() {
    let call = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Items",
        "operations": [{
            "op": "editRelations",
            "relation": "inputByString",
            "mode": "add",
            "targets": [{"fieldPath": "Catalog.Other.Attribute.Code"}],
        }],
    });
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();

    assert!(
        validator.is_valid(&call),
        "standard JSON Schema cannot correlate the root owner name into fieldPath"
    );
    let error = diagnostic(unica_metadata::request::MetadataOperation::Edit, call);
    assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(error.field.as_deref(), Some("targets[0]"));
}

#[test]
fn parser_reports_owner_capability_failures_before_provider_dispatch() {
    let cases = [
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "CommonModule.Utility",
                "operations": [
                    {"op": "setProperties", "values": {"Comment": "ok"}},
                    {
                        "op": "editRelations",
                        "relation": "basedOn",
                        "mode": "add",
                        "targets": [{"metadataPath": "CommonModule.Base"}],
                    },
                ],
            }),
            MetaDiagnosticCode::UnsupportedKind,
            "relation",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "AccountingRegister.Ledger",
                "operations": [
                    {"op": "setProperties", "values": {"Comment": "ok"}},
                    {
                        "op": "add",
                        "collection": "dimensions",
                        "elements": [{
                            "name": "Account",
                            "fillValue": {"kind": "boolean", "value": false},
                        }],
                    },
                ],
            }),
            MetaDiagnosticCode::InvalidArguments,
            "elements[0].fillValue",
        ),
        (
            unica_metadata::request::MetadataOperation::Add,
            json!({
                "sourceSet": "main",
                "kind": "Catalog",
                "name": "Items",
                "operations": [
                    {"op": "setProperties", "values": {"Comment": "ok"}},
                    {
                        "op": "editRelations",
                        "relation": "inputByString",
                        "mode": "add",
                        "targets": [{"fieldPath": "Catalog.Other.Attribute.Code"}],
                    },
                ],
            }),
            MetaDiagnosticCode::InvalidArguments,
            "targets[0]",
        ),
    ];

    for (operation, call, code, field) in cases {
        let error = diagnostic(operation, call);
        assert_eq!(error.code, code, "{error:?}");
        assert_eq!(error.operation_index, Some(1), "{error:?}");
        assert_eq!(error.field.as_deref(), Some(field), "{error:?}");
    }
}

#[test]
fn schema_and_parser_follow_every_fill_value_context() {
    use unica_metadata::{metadata_fill_value_contexts, MetaElementScope};

    for kind in MetadataKind::ALL.iter().copied() {
        for context in metadata_fill_value_contexts(kind) {
            for tag in ["add", "update"] {
                let mut operation = json!({
                    "op": tag,
                    "collection": context.collection.as_str(),
                    "elements": [{
                        "name": "Value",
                        "type": {"variants": [{"kind": "boolean"}]},
                        "fillValue": {"kind": "boolean", "value": false},
                    }],
                });
                if context.scope == MetaElementScope::TabularSection {
                    operation["scope"] = json!({"tabularSection": "Lines"});
                }
                let call = json!({
                    "sourceSet": "main",
                    "metadataPath": format!("{}.Object", kind.as_str()),
                    "operations": [operation],
                });
                assert!(
                    validate_schema_and_parse(
                        unica_metadata::request::MetadataOperation::Edit,
                        &call
                    ),
                    "registered fillValue context was rejected: {call}"
                );
            }
        }
    }

    for kind in [MetadataKind::Report, MetadataKind::DataProcessor] {
        let call = json!({
            "sourceSet": "main",
            "metadataPath": format!("{}.Object", kind.as_str()),
            "operations": [{
                "op": "add",
                "collection": "tabularSections",
                "elements": [{
                    "name": "Lines",
                    "attributes": [{
                        "name": "Value",
                        "type": {"variants": [{"kind": "boolean"}]},
                        "fillValue": {"kind": "boolean", "value": false},
                    }],
                }],
            }],
        });
        assert!(
            validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &call),
            "nested registered fillValue context was rejected: {call}"
        );
    }
}

#[test]
fn schema_and_parser_reject_fill_value_outside_owner_context() {
    let calls = [
        json!({
            "sourceSet": "main",
            "metadataPath": "AccountingRegister.Ledger",
            "operations": [{
                "op": "add",
                "collection": "dimensions",
                "elements": [{
                    "name": "Account",
                    "fillValue": {"kind": "boolean", "value": false},
                }],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Items",
            "operations": [{
                "op": "update",
                "collection": "attributes",
                "scope": {"tabularSection": "Lines"},
                "elements": [{
                    "name": "Value",
                    "fillValue": {"kind": "boolean", "value": false},
                }],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Report.Sales",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Value",
                    "fillValue": {"kind": "boolean", "value": false},
                }],
            }],
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Items",
            "operations": [{
                "op": "add",
                "collection": "tabularSections",
                "elements": [{
                    "name": "Lines",
                    "attributes": [{
                        "name": "Value",
                        "fillValue": {"kind": "boolean", "value": false},
                    }],
                }],
            }],
        }),
    ];

    for call in calls {
        assert!(
            !validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &call),
            "out-of-context fillValue was accepted: {call}"
        );
    }

    // A kind without collections or relations reaches the same published
    // union; the writer is what answers `unsupported_kind` for it.
    let error = diagnostic(
        unica_metadata::request::MetadataOperation::Edit,
        json!({
            "sourceSet": "main",
            "metadataPath": "CommonModule.Shared",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{"name": "Extra"}],
            }],
        }),
    );
    assert_eq!(error.code, MetaDiagnosticCode::UnsupportedKind);
    assert_eq!(error.field.as_deref(), Some("collection"));
}

#[test]
fn schema_and_parser_reject_writer_static_type_fill_profiles() {
    let cases = [
        (
            unica_metadata::request::MetadataOperation::Add,
            json!({
                "sourceSet": "main",
                "kind": "Catalog",
                "name": "Items",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Enabled",
                        "fillValue": {"kind": "boolean", "value": true}
                    }]
                }]
            }),
            "elements[0].fillValue",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Enabled",
                        "type": {"variants": [{
                            "kind": "string",
                            "length": 10,
                            "allowedLength": "variable"
                        }]},
                        "fillValue": {"kind": "boolean", "value": true}
                    }]
                }]
            }),
            "elements[0].fillValue",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "update",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Amount",
                        "fillValue": {"kind": "number", "value": "1e3"}
                    }]
                }]
            }),
            "elements[0].fillValue",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "update",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Enabled",
                        "type": {"variants": [{
                            "kind": "string",
                            "length": 10,
                            "allowedLength": "variable"
                        }]},
                        "fillValue": {"kind": "boolean", "value": true}
                    }]
                }]
            }),
            "elements[0].fillValue",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "UnsupportedRef",
                        "type": {"variants": [{
                            "kind": "reference",
                            "metadataPath": "Report.Sales"
                        }]}
                    }]
                }]
            }),
            "elements[0].type.variants[0].metadataPath",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "update",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Value",
                        "type": {"variants": [{
                            "kind": "definedType",
                            "metadataPath": "Catalog.Items"
                        }]}
                    }]
                }]
            }),
            "elements[0].type.variants[0].metadataPath",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Document.Order",
                "operations": [{
                    "op": "add",
                    "collection": "attributes",
                    "elements": [{
                        "name": "Amount",
                        "type": {"variants": [{
                            "kind": "number",
                            "digits": 10,
                            "fraction": 2,
                            "sign": "any"
                        }]},
                        "fillValue": {"kind": "number", "value": "1e3"}
                    }]
                }]
            }),
            "elements[0].fillValue",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({
                "sourceSet": "main",
                "metadataPath": "Report.Sales",
                "operations": [{
                    "op": "add",
                    "collection": "tabularSections",
                    "elements": [{
                        "name": "Lines",
                        "attributes": [{
                            "name": "When",
                            "fillValue": {
                                "kind": "dateTime",
                                "value": "2026-02-30T25:61:00"
                            }
                        }]
                    }]
                }]
            }),
            "elements[0].attributes[0].fillValue",
        ),
    ];

    for (operation, call, expected_field) in cases {
        assert!(
            !validate_schema_and_parse(operation, &call),
            "writer accepted a static type/fill profile it must refuse: {call}"
        );
        let error = diagnostic(operation, call);
        assert_eq!(error.operation_index, Some(0), "{error:?}");
        assert_eq!(error.field.as_deref(), Some(expected_field), "{error:?}");
    }
}

#[test]
fn schema_and_parser_accept_the_complete_reference_type_registry() {
    let variants = [
        "Catalog",
        "Document",
        "Enum",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ChartOfCalculationTypes",
        "BusinessProcess",
        "Task",
        "ExchangePlan",
    ]
    .into_iter()
    .map(|kind| json!({"kind": "reference", "metadataPath": format!("{kind}.Target")}))
    .chain(std::iter::once(json!({
        "kind": "definedType",
        "metadataPath": "DefinedType.Value"
    })))
    .collect::<Vec<_>>();
    let call = json!({
        "sourceSet": "main",
        "metadataPath": "Document.Order",
        "operations": [{
            "op": "add",
            "collection": "attributes",
            "elements": [{
                "name": "Target",
                "type": {"variants": variants}
            }]
        }]
    });

    assert!(
        validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &call),
        "registered reference type kind was rejected: {call}"
    );
}

#[test]
fn parser_rejects_cross_value_constraints_schema_cannot_correlate() {
    let calls = [
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Code",
                    "type": {"variants": [{
                        "kind": "string",
                        "length": 3,
                        "allowedLength": "variable"
                    }]},
                    "fillValue": {"kind": "string", "value": "LONG"}
                }]
            }]
        }),
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "update",
                "collection": "attributes",
                "elements": [{
                    "name": "Customer",
                    "type": {"variants": [{
                        "kind": "reference",
                        "metadataPath": "Catalog.Customers"
                    }]},
                    "fillValue": {
                        "kind": "reference",
                        "metadataPath": "Catalog.Counterparties"
                    }
                }]
            }]
        }),
    ];
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();

    for call in calls {
        assert!(
            validator.is_valid(&call),
            "standard JSON Schema cannot compare sibling qualifier/path values: {call}"
        );
        let error = diagnostic(unica_metadata::request::MetadataOperation::Edit, call);
        assert_eq!(error.operation_index, Some(0), "{error:?}");
        assert_eq!(error.field.as_deref(), Some("elements[0].fillValue"));
    }
}

#[test]
fn update_keeps_post_image_dependent_value_profiles_open() {
    for element in [
        json!({
            "name": "Value",
            "fillValue": {"kind": "boolean", "value": true}
        }),
        json!({
            "name": "Value",
            "type": {"variants": [{
                "kind": "string",
                "length": 20,
                "allowedLength": "variable"
            }]}
        }),
    ] {
        let call = json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "update",
                "collection": "attributes",
                "elements": [element]
            }]
        });
        assert!(
            validate_schema_and_parse(unica_metadata::request::MetadataOperation::Edit, &call),
            "post-image-dependent update was rejected: {call}"
        );
    }
}

#[test]
fn schema_and_parser_share_the_timezone_optional_datetime_lexicon() {
    let call = |value: &str| {
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [{
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "When",
                    "type": {"variants": [{
                        "kind": "date",
                        "fractions": "dateTime"
                    }]},
                    "fillValue": {"kind": "dateTime", "value": value}
                }]
            }]
        })
    };
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();

    for value in [
        "2026-01-01T12:00:00",
        "2026-01-01T12:00:00Z",
        "2026-01-01T12:00:00.125+07:00",
        "2026-01-01T12:00:00+13:59",
        "2026-01-01T12:00:00-13:59",
        "2026-01-01T12:00:00+14:00",
        "2026-01-01T12:00:00-14:00",
    ] {
        let call = call(value);
        assert!(validator.is_valid(&call), "schema rejected {value}");
        assert!(
            unica_metadata::request::parse_metadata_request(
                unica_metadata::request::MetadataOperation::Edit,
                call.as_object().unwrap()
            )
            .is_ok(),
            "parser rejected {value}"
        );
    }
    for value in [
        "2026-01-01 12:00:00",
        "2026-01-01T25:00:00",
        "2026-01-01T12:00:00+07",
        "2026-01-01T12:00:00+14:01",
        "2026-01-01T12:00:00-14:01",
        "2026-01-01T12:00:00+14:59",
        "2026-01-01T12:00:00-14:59",
    ] {
        let call = call(value);
        assert!(!validator.is_valid(&call), "schema accepted {value}");
        let error = diagnostic(unica_metadata::request::MetadataOperation::Edit, call);
        assert_eq!(error.field.as_deref(), Some("elements[0].fillValue"));
    }

    let invalid_calendar = call("2026-02-30T12:00:00");
    assert!(
        validator.is_valid(&invalid_calendar),
        "calendar correlation intentionally remains a domain check"
    );
    let error = diagnostic(
        unica_metadata::request::MetadataOperation::Edit,
        invalid_calendar,
    );
    assert_eq!(error.field.as_deref(), Some("elements[0].fillValue"));
}

#[test]
fn parse_all_five_edit_operations_and_every_collection_into_closed_domain_variants() {
    let unica_metadata::request::MetadataRequest::Edit(request) =
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            &object(edit(json!({
                "op": "setProperties",
                "values": {"NumberLength": 12, "CheckUnique": true}
            }))),
        )
        .unwrap()
    else {
        panic!("expected edit request")
    };
    assert!(matches!(
        request.operations[0],
        MetaEditOperation::SetProperties { .. }
    ));
    assert!(request.dry_run);

    for collection in MetaCollection::ALL {
        if *collection == MetaCollection::PredefinedItems {
            continue;
        }
        let collection_name = collection.as_str();
        let metadata_path = match collection {
            MetaCollection::Dimensions | MetaCollection::Resources => "InformationRegister.Entries",
            MetaCollection::EnumValues => "Enum.Statuses",
            MetaCollection::Columns => "DocumentJournal.Documents",
            MetaCollection::Attributes
            | MetaCollection::TabularSections
            | MetaCollection::Forms
            | MetaCollection::Templates
            | MetaCollection::Commands => "Document.Order",
            MetaCollection::PredefinedItems => unreachable!("filtered above"),
        };
        for (op, payload) in [
            ("add", json!({"elements": [{"name": "Element"}]})),
            (
                "update",
                json!({"elements": [{"name": "Element", "comment": "changed"}]}),
            ),
            ("remove", json!({"names": ["Element"]})),
        ] {
            let mut operation = payload.as_object().unwrap().clone();
            operation.insert("op".into(), json!(op));
            operation.insert("collection".into(), json!(collection_name));
            let unica_metadata::request::MetadataRequest::Edit(request) =
                unica_metadata::request::parse_metadata_request(
                    unica_metadata::request::MetadataOperation::Edit,
                    &object(json!({
                        "sourceSet": "main",
                        "metadataPath": metadata_path,
                        "operations": [Value::Object(operation)],
                    })),
                )
                .unwrap()
            else {
                panic!("expected edit request")
            };
            match (&request.operations[0], op) {
                (
                    MetaEditOperation::Add {
                        collection: actual, ..
                    },
                    "add",
                )
                | (
                    MetaEditOperation::Update {
                        collection: actual, ..
                    },
                    "update",
                )
                | (
                    MetaEditOperation::Remove {
                        collection: actual, ..
                    },
                    "remove",
                ) => {
                    assert_eq!(*actual, *collection)
                }
                (actual, _) => {
                    panic!("wrong conversion for {op}/{collection_name}: {actual:?}")
                }
            }
        }
    }

    for relation in MetaRelation::ALL {
        let relation_name = relation.as_str();
        let (metadata_path, target) = match relation {
            MetaRelation::Owners => ("Catalog.Items", json!({"metadataPath": "Catalog.Owner"})),
            MetaRelation::RegisterRecords => (
                "Document.Order",
                json!({"metadataPath": "InformationRegister.Entries"}),
            ),
            MetaRelation::BasedOn => ("Document.Order", json!({"metadataPath": "Catalog.Items"})),
            MetaRelation::InputByString => (
                "Catalog.Items",
                json!({"fieldPath": "Catalog.Items.StandardAttribute.Code"}),
            ),
            MetaRelation::Source => (
                "EventSubscription.Notify",
                json!({"kind": "family", "sourceClass": "catalogObject"}),
            ),
        };
        let modes = RelationEditMode::ALL;
        for mode in modes {
            let mode_name = mode.as_str();
            let unica_metadata::request::MetadataRequest::Edit(request) =
                unica_metadata::request::parse_metadata_request(
                    unica_metadata::request::MetadataOperation::Edit,
                    &object(json!({
                        "sourceSet": "main",
                        "metadataPath": metadata_path,
                        "operations": [{
                            "op": "editRelations",
                            "relation": relation_name,
                            "mode": mode_name,
                            "targets": [target]
                        }]
                    })),
                )
                .unwrap()
            else {
                panic!("expected edit request")
            };
            assert!(matches!(
                &request.operations[0],
                MetaEditOperation::EditRelations { relation: actual_relation, mode: actual_mode, .. }
                    if *actual_relation == *relation && *actual_mode == *mode
            ));
        }
    }
}

#[test]
fn predefined_readers_keep_legacy_compound_error_order() {
    for (changes, field, message) in [
        (
            json!({"id": false, "extra": true}),
            "elements[3].extra",
            "unknown field `extra`",
        ),
        (
            json!({"id": "not-a-uuid", "name": false}),
            "elements[3].id",
            "predefined item id must be a UUID",
        ),
        (
            json!({"name": false, "code": false}),
            "elements[3].name",
            "`elements[3].name` must be a string",
        ),
        (
            json!({"code": false, "description": false}),
            "elements[3].code",
            "field must be a string",
        ),
        (
            json!({"isFolder": "bad", "accountType": false}),
            "elements[3].isFolder",
            "field must be a boolean",
        ),
        (
            json!({"accountType": false, "offBalance": "bad"}),
            "elements[3].accountType",
            "accountType must be a string",
        ),
        (
            json!({"accountType": "bad", "offBalance": "bad"}),
            "elements[3].accountType",
            "accountType must be Active, Passive, or ActivePassive",
        ),
        (
            json!({"order": false, "accountingFlags": false}),
            "elements[3].order",
            "field must be a string",
        ),
        (
            json!({"accountingFlags": {"": false, "Bad": "bad"}, "extDimensionTypes": false}),
            "elements[3].accountingFlags",
            "accounting flag name must not be empty",
        ),
        (
            json!({"accountingFlags": {"Bad": "bad"}, "extDimensionTypes": false}),
            "elements[3].accountingFlags.Bad",
            "accounting flag must be boolean",
        ),
        (
            json!({"extDimensionTypes": [{"name": false, "extra": true, "turnover": "bad"}]}),
            "elements[3].extDimensionTypes[0].extra",
            "unknown field `extra`",
        ),
        (
            json!({"extDimensionTypes": [{"name": false, "turnover": "bad"}]}),
            "elements[3].extDimensionTypes[0].name",
            "`elements[3].extDimensionTypes[0].name` must be a string",
        ),
        (
            json!({"extDimensionTypes": [{"name": "Dim"}, {"name": "dim", "turnover": "bad"}]}),
            "elements[3].extDimensionTypes[1].name",
            "ext-dimension type name is duplicated",
        ),
        (
            json!({"extDimensionTypes": [{"name": "Dim", "turnover": "bad", "accountingFlags": false}]}),
            "elements[3].extDimensionTypes[0].turnover",
            "turnover must be boolean",
        ),
        (
            json!({"extDimensionTypes": [{"name": "Dim", "accountingFlags": {"": "bad"}}]}),
            "elements[3].extDimensionTypes[0].accountingFlags",
            "accounting flag name must not be empty",
        ),
    ] {
        let mut item = json!({"id":"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e","name":"Main"});
        item.as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        let error = parse_predefined_add(&item, 3).unwrap_err();
        assert_eq!(error.field.as_deref(), Some(field), "{item}: {error:?}");
        assert_eq!(error.message, message, "{item}: {error:?}");
    }
    let item = json!({"id":"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e","code":""});
    assert!(parse_predefined_update(&item, 3).unwrap().name.is_none());
    let error = parse_predefined_update(&json!({"id":false,"newName":"Renamed"}), 3).unwrap_err();
    assert_eq!(error.field.as_deref(), Some("elements[3].newName"));
    assert_eq!(error.message, "unknown field `newName`");
}

#[test]
fn predefined_items_schema_and_parser_use_uuid_typed_collection_contract() {
    let name_shape = json!({"type":"string","minLength":1});
    for update in [false, true] {
        let raw = metadata_predefined_element_input_schema(update);
        assert_eq!(raw["properties"]["id"], name_shape);
        assert_eq!(raw["properties"]["name"], name_shape);
        assert_eq!(
            raw["properties"]["extDimensionTypes"]["items"]["properties"]["name"],
            name_shape
        );
    }
    let host = host_visible_operation_schema();
    let predefined_operations = host["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|operation| {
            operation["properties"]["collection"]["enum"][0] == "predefinedItems"
                && operation["properties"].get("elements").is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(predefined_operations.len(), 2);
    for operation in predefined_operations {
        let fields = &operation["properties"]["elements"]["items"]["properties"];
        assert_eq!(fields["name"], name_shape);
        assert_eq!(
            fields["extDimensionTypes"]["items"]["properties"]["name"],
            name_shape
        );
        assert_eq!(
            fields["id"],
            json!({"type":"string","format":"uuid","pattern":"^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$"})
        );
        assert!(fields.get("newName").is_none());
    }
    let can =
        unica_metadata::contracts::metadata_predefined_add(Some(MetadataKind::ChartOfAccounts));
    assert_eq!(
        can.schema["properties"]["items"]["items"]["properties"]["extDimensionTypes"]["items"]
            ["properties"]["name"],
        name_shape
    );
    let schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let validator = jsonschema::validator_for(&schema).unwrap();
    let valid = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Items",
        "operations": [{
            "op": "add",
            "collection": "predefinedItems",
            "elements": [{
                "id": "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
                "name": "Main",
                "isFolder": false
            }]
        }]
    });
    assert!(validator.is_valid(&valid));
    let unica_metadata::request::MetadataRequest::Edit(request) =
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            valid.as_object().unwrap(),
        )
        .unwrap()
    else {
        panic!("expected edit request")
    };
    assert!(matches!(
        &request.operations[0],
        MetaEditOperation::AddPredefinedItems { elements }
            if elements[0].id == "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"
    ));

    let clear_structural_fields = json!({
        "sourceSet": "main",
        "metadataPath": "ChartOfAccounts.Items",
        "operations": [{
            "op": "update",
            "collection": "predefinedItems",
            "elements": [{
                "id": "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
                "accountingFlags": {},
                "extDimensionTypes": []
            }]
        }]
    });
    assert!(validator.is_valid(&clear_structural_fields));
    let unica_metadata::request::MetadataRequest::Edit(clear_request) =
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            clear_structural_fields.as_object().unwrap(),
        )
        .unwrap()
    else {
        panic!("expected edit request")
    };
    assert!(matches!(
        &clear_request.operations[0],
        MetaEditOperation::UpdatePredefinedItems { elements }
            if elements[0]
                .fields
                .accounting_flags
                .as_ref()
                .is_some_and(BTreeMap::is_empty)
                && elements[0]
                    .fields
                    .ext_dimension_types
                    .as_ref()
                    .is_some_and(Vec::is_empty)
    ));

    for invalid_operation in [
        json!({
            "op": "remove",
            "collection": "predefinedItems",
            "names": ["Main"]
        }),
        json!({
            "op": "remove",
            "collection": "predefinedItems",
            "ids": ["not-a-uuid"]
        }),
        json!({
            "op": "remove",
            "collection": "predefinedItems",
            "ids": ["a7d2e6fc38244b56b4beae6be4944c0e"]
        }),
        json!({
            "op": "add",
            "collection": "predefinedItems",
            "scope": {"tabularSection": "Rows"},
            "elements": [{
                "id": "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
                "name": "Main"
            }]
        }),
    ] {
        let call = json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Items",
            "operations": [invalid_operation]
        });
        assert!(!validator.is_valid(&call));
        assert!(unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            call.as_object().unwrap(),
        )
        .is_err());
    }
    let wrong_owner_field = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Items",
        "operations": [{
            "op": "add",
            "collection": "predefinedItems",
            "elements": [{
                "id": "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e",
                "name": "Main",
                "accountType": "Active"
            }]
        }]
    });
    assert!(validator.is_valid(&wrong_owner_field));
    assert!(unica_metadata::request::parse_metadata_request(
        unica_metadata::request::MetadataOperation::Edit,
        wrong_owner_field.as_object().unwrap(),
    )
    .is_err());
}

#[test]
fn predefined_info_section_rejects_an_unsupported_owner_kind() {
    let error = unica_metadata::request::parse_metadata_request(
        unica_metadata::request::MetadataOperation::Info,
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "sections": ["predefinedItems"]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap_err();
    assert_eq!(
        error.diagnostics[0].code,
        MetaDiagnosticCode::UnsupportedKind
    );
    assert_eq!(error.diagnostics[0].field.as_deref(), Some("sections"));
}

#[test]
fn parse_rejects_conditional_shapes_with_indexed_field_diagnostics() {
    let cases = [
        (json!({"op": "setProperties"}), "values"),
        (
            json!({"op": "setProperties", "values": {"Comment": "x"}, "collection": "attributes"}),
            "collection",
        ),
        (json!({"op": "add", "collection": "attributes"}), "elements"),
        (
            json!({"op": "remove", "collection": "attributes", "names": []}),
            "names",
        ),
        (
            json!({"op": "editRelations", "relation": "owners", "mode": "add", "targets": []}),
            "targets",
        ),
        (
            json!({"op": "add", "collection": "forms", "scope": {"tabularSection": "Lines"}, "elements": [{"name": "F"}]}),
            "scope",
        ),
        (
            json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "position": {"before": "B", "after": "C"}}]}),
            "elements[0].position",
        ),
        (
            json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": "String(10) | req"}]}),
            "elements[0].type",
        ),
        (
            json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": {"variants": [{"kind": "number", "digits": 10, "fraction": 2}]}}]}),
            "elements[0].type.variants[0].sign",
        ),
        (
            json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "type": {"variants": "number"}}]}),
            "elements[0].type.variants",
        ),
        (
            json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "fillValue": {"kind": "reference"}}]}),
            "elements[0].fillValue.metadataPath",
        ),
        (
            json!({"op": "add", "collection": "attributes", "elements": [{"name": "A", "unknown": true}]}),
            "elements[0].unknown",
        ),
        (
            json!({"op": "add", "collection": "tabularSections", "elements": [{"name": "Lines", "attributes": []}]}),
            "elements[0].attributes",
        ),
        (
            json!({"op": "add", "collection": "tabularSections", "elements": [{"name": "Lines", "attributes": "Quantity"}]}),
            "elements[0].attributes",
        ),
        (
            json!({"op": "setProperties", "values": {"UnknownProperty": true}}),
            "values.UnknownProperty",
        ),
    ];

    for (operation, expected_field) in cases {
        let error = diagnostic(
            unica_metadata::request::MetadataOperation::Edit,
            edit(operation),
        );
        assert_eq!(
            error.code,
            MetaDiagnosticCode::InvalidArguments,
            "{error:?}"
        );
        assert_eq!(error.operation_index, Some(0), "{error:?}");
        assert_eq!(error.field.as_deref(), Some(expected_field), "{error:?}");
    }

    let error = diagnostic(
        unica_metadata::request::MetadataOperation::Edit,
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": "add attribute A;;add attribute B"
        }),
    );
    assert_eq!(error.field.as_deref(), Some("operations"));
}

#[test]
fn parse_accepts_binary_data_type_qualifiers() {
    let unica_metadata::request::MetadataRequest::Edit(request) =
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            &object(edit(json!({
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Payload",
                    "type": {"variants": [{
                        "kind": "binaryData",
                        "length": 512,
                        "allowedLength": "fixed"
                    }]}
                }]
            }))),
        )
        .unwrap()
    else {
        panic!("expected edit request")
    };
    let MetaEditOperation::Add { elements, .. } = &request.operations[0] else {
        panic!("expected add operation")
    };
    assert!(matches!(
        elements[0].r#type.as_ref().unwrap().variants[0],
        MetadataTypeVariant::BinaryData {
            length: 512,
            allowed_length: StringLengthMode::Fixed
        }
    ));
}

#[test]
fn schema_and_parser_accept_the_uuid_writer_variant() {
    let call = edit(json!({
        "op": "add",
        "collection": "attributes",
        "elements": [{
            "name": "ExternalId",
            "type": {"variants": [{"kind": "uuid"}]}
        }]
    }));
    assert!(validate_schema_and_parse(
        unica_metadata::request::MetadataOperation::Edit,
        &call
    ));

    let unica_metadata::request::MetadataRequest::Edit(request) =
        unica_metadata::request::parse_metadata_request(
            unica_metadata::request::MetadataOperation::Edit,
            &object(call),
        )
        .unwrap()
    else {
        panic!("expected edit request")
    };
    let MetaEditOperation::Add { elements, .. } = &request.operations[0] else {
        panic!("expected add operation")
    };
    assert_eq!(
        elements[0].r#type.as_ref().unwrap().variants,
        vec![MetadataTypeVariant::Uuid]
    );
}

#[test]
fn parse_reports_exact_paths_for_later_and_nested_array_items() {
    let cases = [
        (
            json!({
                "op": "add",
                "collection": "attributes",
                "elements": [{"name": "First"}, {"comment": "missing name"}]
            }),
            "elements[1].name",
        ),
        (
            json!({
                "op": "add",
                "collection": "forms",
                "elements": [
                    {"name": "First"},
                    {"name": "Second", "type": {"variants": [{"kind": "boolean"}]}}
                ]
            }),
            "elements[1].type",
        ),
        (
            json!({
                "op": "add",
                "collection": "attributes",
                "elements": [{"name": "Duplicate"}, {"name": "Duplicate"}]
            }),
            "elements[1].name",
        ),
        (
            json!({
                "op": "update",
                "collection": "attributes",
                "elements": [
                    {"name": "First", "comment": "changed"},
                    {"name": "Second", "position": {"before": "A", "after": "B"}}
                ]
            }),
            "elements[1].position",
        ),
        (
            json!({
                "op": "add",
                "collection": "tabularSections",
                "elements": [{
                    "name": "Lines",
                    "attributes": [{"name": "First"}, {"type": {"variants": []}}]
                }]
            }),
            "elements[0].attributes[1].name",
        ),
        (
            json!({
                "op": "add",
                "collection": "tabularSections",
                "elements": [{
                    "name": "Lines",
                    "attributes": [{"name": "Duplicate"}, {"name": "Duplicate"}]
                }]
            }),
            "elements[0].attributes[1].name",
        ),
        (
            json!({
                "op": "remove",
                "collection": "attributes",
                "names": ["First", ""]
            }),
            "names[1]",
        ),
        (
            json!({
                "op": "remove",
                "collection": "attributes",
                "names": ["Duplicate", "Duplicate"]
            }),
            "names[1]",
        ),
        (
            json!({
                "op": "editRelations",
                "relation": "owners",
                "mode": "add",
                "targets": [
                    {"metadataPath": "Catalog.Items"},
                    {"metadataPath": ""}
                ]
            }),
            "targets[1].metadataPath",
        ),
        (
            json!({
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "Typed",
                    "type": {"variants": [
                        {"kind": "boolean"},
                        {"kind": "number", "digits": 10, "fraction": 2}
                    ]}
                }]
            }),
            "elements[0].type.variants[1].sign",
        ),
    ];

    for (operation, expected_field) in cases {
        let error = diagnostic(
            unica_metadata::request::MetadataOperation::Edit,
            edit(operation),
        );
        assert_eq!(error.operation_index, Some(0), "{error:?}");
        assert_eq!(error.field.as_deref(), Some(expected_field), "{error:?}");
    }

    let error = diagnostic(
        unica_metadata::request::MetadataOperation::Edit,
        json!({
            "sourceSet": "main",
            "metadataPath": "Document.Order",
            "operations": [
                {"op": "setProperties", "values": {"Comment": "valid"}},
                {"op": "remove", "collection": "attributes", "names": ["First", ""]}
            ]
        }),
    );
    assert_eq!(error.operation_index, Some(1));
    assert_eq!(error.field.as_deref(), Some("names[1]"));
}

#[test]
fn parse_rejects_unknown_top_level_fields_missing_required_values_and_wrong_scalars() {
    let cases = [
        (
            unica_metadata::request::MetadataOperation::Info,
            json!({"sourceSet": "main", "metadataPath": "Document.Order", "path": "legacy"}),
            "path",
        ),
        (
            unica_metadata::request::MetadataOperation::Add,
            json!({"sourceSet": "main", "name": "A"}),
            "kind",
        ),
        (
            unica_metadata::request::MetadataOperation::Edit,
            json!({"sourceSet": "main", "metadataPath": "Document.Order", "operations": []}),
            "operations",
        ),
    ];
    for (operation, input, field) in cases {
        let error = diagnostic(operation, input);
        assert_eq!(error.code, MetaDiagnosticCode::InvalidArguments);
        assert_eq!(error.field.as_deref(), Some(field), "{error:?}");
    }
}

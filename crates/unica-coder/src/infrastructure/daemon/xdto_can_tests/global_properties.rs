use super::*;

const NS: &str = "http://v8.1c.ru/edi/edi_stnd/EnterpriseData/1.17.3";
const FOREIGN: &str = "urn:unica:global-foreign";
const HOST: &str = "main:XDTOPackage.EnterpriseData_1_17_3.Type.Host";
const RESOURCE: &str = "src/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin";
const EXTERNAL: &str = "src/XDTOPackages/Foreign/Ext/Package.bin";

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    let descriptor = fixture
        .workspace
        .join("src/XDTOPackages/EnterpriseData_1_17_3.xml");
    let descriptor = std::fs::read_to_string(descriptor).unwrap();
    let old_uuid = descriptor
        .split("<XDTOPackage uuid=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let foreign = descriptor
        .replace("EnterpriseData_1_17_3", "Foreign")
        .replace(NS, FOREIGN)
        .replace(old_uuid, "00000000-0000-4000-8000-000000001661");
    std::fs::write(
        fixture.workspace.join("src/XDTOPackages/Foreign.xml"),
        foreign,
    )
    .unwrap();
    let config = fixture.workspace.join("src/Configuration.xml");
    let text = std::fs::read_to_string(&config).unwrap();
    assert_eq!(text.matches("</ChildObjects>").count(), 1);
    std::fs::write(
        config,
        text.replace(
            "</ChildObjects>",
            "<XDTOPackage>Foreign</XDTOPackage></ChildObjects>",
        ),
    )
    .unwrap();
    std::fs::write(fixture.workspace.join(RESOURCE), package(NS, &format!(
        "\t<import namespace=\"{FOREIGN}\"/>\r\n\t<property name=\"Shared\" type=\"xs:string\"/>\r\n\t<valueType name=\"TypeOnly\" base=\"xs:string\"/>\r\n\t<objectType name=\"Host\">\r\n\t\t<property name=\"Kept\" type=\"xs:string\"/>\r\n\t\t<property name=\"Outer\">\r\n\t\t\t<typeDef xsi:type=\"ObjectType\">\r\n\t\t\t\t<property name=\"Child\" type=\"xs:string\"/>\r\n\t\t\t</typeDef>\r\n\t\t</property>\r\n\t</objectType>"
    ))).unwrap();
    std::fs::create_dir_all(fixture.workspace.join(EXTERNAL).parent().unwrap()).unwrap();
    std::fs::write(fixture.workspace.join(EXTERNAL), package(FOREIGN,
        "\t<property name=\"Shared\" type=\"xs:string\"/>\r\n\t<valueType name=\"TypeOnly\" base=\"xs:string\"/>"
    )).unwrap();
    fixture
}

fn package(namespace: &str, body: &str) -> String {
    format!("\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<package xmlns=\"http://v8.1c.ru/8.1/xdto\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:tns=\"{namespace}\" xmlns:ext=\"{FOREIGN}\" targetNamespace=\"{namespace}\">\r\n{body}\r\n</package>\r\n")
}

fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

fn call(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    tool: ToolIdentity,
    args: Value,
) -> DomainResult {
    direct_v5(
        runtime,
        InvocationRequest::new(tool, args, fixture.workspace.to_string_lossy(), 7_000).unwrap(),
    )
    .unwrap()
}

fn ref_op(at: &str, reference: &str) -> Value {
    json!({"op":"property.add","args":{"at":at,"values":{"property":{"ref":reference}}}})
}

fn preview(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, ops: Value) -> DomainResult {
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let result = call(
        runtime,
        fixture,
        ToolIdentity::Apply,
        json!({"at":PACKAGE,"ops":ops}),
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before,
        "preview/refusal never writes"
    );
    result
}

/// rule: INV.SOURCE.XDTO-PROPERTY-PATH
#[test]
fn xdto_global_references_write_read_repeat_and_preserve_other_bytes() {
    let fixture = fixture();
    let runtime = runtime();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let ops = json!([
        {"op":"property.add","args":{"values":{"property":{"name":"Added","type":"xs:string","lowerBound":0}}}},
        ref_op(HOST, "tns:Added"), ref_op(HOST, "ext:Shared")
    ]);
    let planned = preview(&runtime, &fixture, ops.clone());
    assert!(planned.ok, "{planned:?}");
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned.data.unwrap()["executionToken"]}),
    );
    assert!(result.ok, "{result:?}");
    let after = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let changed = before
        .iter()
        .filter(|(path, bytes)| after.get(*path) != Some(*bytes))
        .map(|(path, _)| path.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(changed, [RESOURCE]);
    let before_text = String::from_utf8(before[Path::new(RESOURCE)].clone()).unwrap();
    let after_text = String::from_utf8(after[Path::new(RESOURCE)].clone()).unwrap();
    let expected = before_text
        .replace("\t<valueType", "\t<property name=\"Added\" type=\"xs:string\" lowerBound=\"0\"/>\r\n\t<valueType")
        .replace("\t</objectType>", "\t\t<property ref=\"tns:Added\"/>\r\n\t\t<property ref=\"ext:Shared\"/>\r\n\t</objectType>");
    assert_eq!(after_text, expected);
    let read = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at":format!("{PACKAGE}.Property.Added")}),
    );
    assert!(read.ok, "{read:?}");
    assert_eq!(read.data.unwrap()["props"]["type"]["raw"], "xs:string");
    let repeated = preview(&runtime, &fixture, ops);
    assert!(repeated.ok, "{repeated:?}");
    assert_eq!(repeated.data.unwrap()["effects"], 0);
}

#[test]
fn xdto_global_reference_final_batch_handles_deleted_and_renamed_owners() {
    for (reference, tail) in [
        ("tns:Shared", json!({"op":"type.remove","args":{"at":HOST}})),
        (
            "ext:Missing",
            json!({"op":"type.remove","args":{"at":HOST}}),
        ),
    ] {
        let fixture = fixture();
        let result = preview(&runtime(), &fixture, json!([ref_op(HOST, reference), tail]));
        assert!(
            result.ok,
            "removed reference is absent from final package: {result:?}"
        );
    }
    let fixture = fixture();
    let result = preview(
        &runtime(),
        &fixture,
        json!([
            ref_op(&format!("{HOST}.Property.Outer"), "ext:Shared"),
            {"op":"property.rename","args":{"at":format!("{HOST}.Property.Outer"),"values":{"newName":"Renamed"}}}
        ]),
    );
    assert!(
        result.ok,
        "surviving reference follows its renamed parent: {result:?}"
    );
}

#[test]
fn xdto_global_reference_forward_declaration_and_bad_late_operation_are_atomic() {
    let fixture = fixture();
    let valid = json!([ref_op(HOST,"tns:Future"), {"op":"property.add","args":{"values":{"property":{"name":"Future","type":"xs:string"}}}}]);
    let result = preview(&runtime(), &fixture, valid);
    assert!(result.ok, "forward global declaration: {result:?}");
    let result = preview(
        &runtime(),
        &fixture,
        json!([
            {"op":"property.add","args":{"values":{"property":{"name":"ValidFirst","type":"xs:string"}}}},
            ref_op(HOST,"ext:TypeOnly")
        ]),
    );
    assert!(!result.ok, "a type is not a global declaration");
    assert_eq!(
        result.at.as_deref(),
        Some("ops[1].args.values.property.ref")
    );
}

#[test]
fn xdto_global_reference_refusals_keep_sources_unchanged() {
    for reference in [
        "missing:Shared",
        "tns:Missing",
        "tns:TypeOnly",
        "ext:Missing",
        "ext:TypeOnly",
    ] {
        let fixture = fixture();
        assert!(
            !preview(&runtime(), &fixture, json!([ref_op(HOST, reference)])).ok,
            "{reference}"
        );
    }
    for property in [
        json!({"ref":"tns:Shared","name":"Bad"}),
        json!({"ref":"tns:Shared","type":"xs:string"}),
        json!({"ref":"tns:Shared","objectType":{"properties":[]}}),
        json!({"ref":"tns:Shared","valueType":{"base":"xs:string"}}),
        json!({"ref":"tns:Shared","typeDef":"raw"}),
        json!({"ref":"tns:Shared","lowerBound":true}),
    ] {
        let fixture = fixture();
        assert!(
            !preview(
                &runtime(),
                &fixture,
                json!([{"op":"property.add","args":{"at":HOST,"values":{"property":property}}}])
            )
            .ok
        );
    }
    for property in [
        json!({"ref":"tns:Shared"}),
        json!({"name":"Bad","objectType":{"properties":[]}}),
        json!({"name":"Bad","valueType":{"base":"xs:string"}}),
    ] {
        let fixture = fixture();
        assert!(
            !preview(
                &runtime(),
                &fixture,
                json!([{"op":"property.add","args":{"values":{"property":property}}}])
            )
            .ok
        );
    }
}

#[test]
fn xdto_global_reference_repeated_bad_baseline_is_still_refused() {
    for reference in ["tns:TypeOnly", "ext:TypeOnly"] {
        let fixture = fixture();
        let path = fixture.workspace.join(RESOURCE);
        let text = std::fs::read_to_string(&path).unwrap().replace(
            "\t</objectType>",
            &format!("\t\t<property ref=\"{reference}\"/>\r\n\t</objectType>"),
        );
        std::fs::write(path, text).unwrap();
        assert!(!preview(&runtime(), &fixture, json!([ref_op(HOST, reference)])).ok);
    }
}

#[test]
fn xdto_global_reference_import_dependencies_invalidate_saved_token() {
    for relative in [
        "src/Configuration.xml",
        "src/XDTOPackages/Foreign.xml",
        EXTERNAL,
    ] {
        let fixture = fixture();
        let runtime = runtime();
        let planned = preview(&runtime, &fixture, json!([ref_op(HOST, "ext:Shared")]));
        assert!(planned.ok, "{planned:?}");
        let path = fixture.workspace.join(relative);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(b' ');
        std::fs::write(path, bytes).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":planned.data.unwrap()["executionToken"]}),
        );
        assert!(!result.ok, "stale {relative}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
}

#[test]
fn xdto_global_reference_reports_earliest_invalid_imported_operand() {
    let fixture = fixture();
    let runtime = runtime();
    for _ in 0..12 {
        let result = preview(
            &runtime,
            &fixture,
            json!([
                ref_op(HOST, "ext:FirstMissing"),
                ref_op(HOST, "ext:SecondMissing")
            ]),
        );
        assert!(!result.ok);
        assert_eq!(
            result.at.as_deref(),
            Some("ops[0].args.values.property.ref")
        );
    }
}

#[test]
fn xdto_global_reference_requires_registered_unique_consistent_imported_package() {
    for case in [
        "unregistered",
        "missing-package",
        "wrong-body-namespace",
        "wrong-descriptor-name",
        "duplicate-namespace",
        "damaged-body",
        "missing-import",
    ] {
        let fixture = fixture();
        let edit = |relative: &str, from: &str, to: &str| {
            let path = fixture.workspace.join(relative);
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains(from));
            std::fs::write(path, text.replace(from, to)).unwrap();
        };
        match case {
            "unregistered" => edit(
                "src/Configuration.xml",
                "<XDTOPackage>Foreign</XDTOPackage>",
                "",
            ),
            "missing-package" => std::fs::remove_file(fixture.workspace.join(EXTERNAL)).unwrap(),
            "wrong-body-namespace" => edit(EXTERNAL, FOREIGN, "urn:wrong"),
            "wrong-descriptor-name" => edit(
                "src/XDTOPackages/Foreign.xml",
                "<Name>Foreign</Name>",
                "<Name>Other</Name>",
            ),
            "duplicate-namespace" => {
                let descriptor =
                    std::fs::read_to_string(fixture.workspace.join("src/XDTOPackages/Foreign.xml"))
                        .unwrap()
                        .replace("<Name>Foreign</Name>", "<Name>Duplicate</Name>")
                        .replace(
                            "00000000-0000-4000-8000-000000001661",
                            "00000000-0000-4000-8000-000000001662",
                        );
                std::fs::write(
                    fixture.workspace.join("src/XDTOPackages/Duplicate.xml"),
                    descriptor,
                )
                .unwrap();
                edit(
                    "src/Configuration.xml",
                    "</ChildObjects>",
                    "<XDTOPackage>Duplicate</XDTOPackage></ChildObjects>",
                );
            }
            "damaged-body" => std::fs::write(fixture.workspace.join(EXTERNAL), b"<broken").unwrap(),
            "missing-import" => edit(
                RESOURCE,
                &format!("\t<import namespace=\"{FOREIGN}\"/>\r\n"),
                "",
            ),
            _ => unreachable!(),
        }
        let result = preview(&runtime(), &fixture, json!([ref_op(HOST, "ext:Shared")]));
        assert!(!result.ok, "{case}: {result:?}");
    }
}

#[test]
fn xdto_global_reference_alias_noop_keeps_actual_lexical_key_and_dependency() {
    let fixture = fixture();
    let path = fixture.workspace.join(RESOURCE);
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "\t</objectType>",
        &format!(
            "\t\t<property xmlns:alias=\"{FOREIGN}\" ref=\"alias:Shared\"/>\r\n\t</objectType>"
        ),
    );
    std::fs::write(path, text).unwrap();
    let runtime = runtime();
    let result = preview(&runtime, &fixture, json!([ref_op(HOST, "ext:Shared")]));
    assert!(
        result.ok,
        "equivalent raw QName remains unchanged: {result:?}"
    );
    assert_eq!(result.data.unwrap()["effects"], 0);
    let result = preview(
        &runtime,
        &fixture,
        json!([ref_op(&format!("{PACKAGE}.Type.TypeOnly"), "ext:Shared")]),
    );
    assert!(
        !result.ok,
        "a value type is not an ObjectType property container"
    );
}

#[test]
fn xdto_global_reference_rejects_platform_property_name_collisions() {
    for own in [true, false] {
        let fixture = fixture();
        if own {
            let path = fixture.workspace.join(RESOURCE);
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .replace("name=\"Kept\"", "name=\"Shared\"");
            std::fs::write(path, text).unwrap();
        }
        let ops = if own {
            json!([ref_op(HOST, "ext:Shared")])
        } else {
            json!([ref_op(HOST, "tns:Shared"), ref_op(HOST, "ext:Shared")])
        };
        let result = preview(&runtime(), &fixture, ops);
        assert!(
            !result.ok,
            "the platform does not permit two properties with the same local name: {result:?}"
        );
    }
}

#[test]
fn xdto_global_reference_cannot_be_followed_by_a_conflicting_own_property() {
    let fixture = fixture();
    let result = preview(
        &runtime(),
        &fixture,
        json!([
            ref_op(HOST, "ext:Shared"),
            {"op":"property.add","args":{"at":HOST,"values":{"property":{"name":"Shared","type":"xs:string"}}}}
        ]),
    );
    assert!(
        !result.ok,
        "a later own property must not bypass the reference collision guard: {result:?}"
    );
    assert_eq!(
        result.at.as_deref(),
        Some("ops[1].args.values.property.name")
    );
}

#[test]
fn xdto_global_reference_collision_guard_preserves_existing_own_noop() {
    let fixture = fixture();
    let path = fixture.workspace.join(RESOURCE);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("name=\"Kept\"", "name=\"Shared\"")
        .replace(
            "\t</objectType>",
            "\t\t<property ref=\"ext:Shared\"/>\r\n\t</objectType>",
        );
    std::fs::write(path, text).unwrap();
    let result = preview(
        &runtime(),
        &fixture,
        json!([
            {"op":"property.add","args":{"at":HOST,"values":{"property":{"name":"Shared","type":"xs:string"}}}}
        ]),
    );
    assert!(
        result.ok,
        "an exact own no-op does not introduce the pre-existing collision: {result:?}"
    );
    assert_eq!(result.data.unwrap()["effects"], 0);
}

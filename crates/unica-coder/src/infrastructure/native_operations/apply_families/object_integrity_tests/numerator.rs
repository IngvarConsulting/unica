use super::*;

fn numerator_fixture(number_type: &str, length: u32) -> Fixture {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    fixture.put("DocumentNumerator", "DocumentNumerators", &format!(
        "<NumberType>{number_type}</NumberType><NumberLength>{length}</NumberLength><NumberAllowedLength>Fixed</NumberAllowedLength><NumberPeriodicity>Year</NumberPeriodicity><CheckUnique>true</CheckUnique>"
    ), "");
    fixture
}

#[test]
fn canonical_numerator_integrity_uses_the_final_number_type_and_length() {
    let fixture = numerator_fixture("String", 51);
    fixture.preview_and_publish("main:DocumentNumerator.Target", json!([
        {"op":"props.set","args":{"values":{"NumberType":"Number"}}},
        {"op":"props.set","args":{"values":{"NumberLength":38}}}
    ]));
    let xml = std::fs::read_to_string(fixture.source.join("DocumentNumerators/Target.xml")).unwrap();
    assert!(xml.contains("<NumberType>Number</NumberType>"));
    assert!(xml.contains("<NumberLength>38</NumberLength>"));
}

#[test]
fn canonical_numerator_invalid_type_length_and_invalid_noop_refuse_without_writes() {
    for (number_type, length, values) in [
        ("String", 39, json!({"NumberType":"Number"})),
        ("Number", 38, json!({"NumberLength":39})),
        ("Number", 39, json!({"NumberLength":39})),
    ] {
        let fixture = numerator_fixture(number_type, length);
        let mut operations = Vec::new();
        if number_type != "Number" || length != 39 {
            operations.push(json!({"op":"props.set","args":{"values":{"CheckUnique":false}}}));
        }
        let index = operations.len();
        operations.push(json!({"op":"props.set","args":{"values":values}}));
        fixture.refuse_preview("main:DocumentNumerator.Target", json!(operations),
            &format!("ops[{index}].args.values"), "38 digits");
    }
}

#[test]
fn canonical_numerator_saved_plan_refuses_a_changed_descriptor_and_preserves_neighbors() {
    let fixture = numerator_fixture("String", 11);
    let request = request("main:DocumentNumerator.Target", json!([
        {"op":"props.set","args":{"values":{"CheckUnique":false}}},
        {"op":"props.set","args":{"values":{"NumberPeriodicity":"Month"}}}
    ]));
    let batch = fixture.prepare(&request);
    let previewed = fixture.actor.preview_prepared_apply(&batch).unwrap();
    let mut preview = DomainResult::success("preview");
    preview.rev = Some(previewed.rev().to_owned());
    let token = fixture.actor.save_prepared_apply(batch, preview).unwrap();
    let path = fixture.source.join("DocumentNumerators/Target.xml");
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, old.replace("<Comment/>", "<Comment>external edit</Comment>")).unwrap();
    let before = tree_snapshot(&fixture.source);
    fixture.actor.execute_saved_apply(&token,
        ProviderDeadline::no_deadline(), &CancellationToken::new(),
        |result, publication| {
            let error = publication.map(|_| ()).expect_err("stale descriptor must refuse publication");
            assert_eq!(error.kind(), ApplyPublicationErrorKind::ConcurrentRevision);
            result
        }
    ).unwrap();
    assert_eq!(tree_snapshot(&fixture.source), before);
}

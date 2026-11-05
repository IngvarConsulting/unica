use super::*;

const SEQUENCE: &str = include_str!("../../../../../../../tests/fixtures/acceptance/workspace-sequence-boundary/src/Sequences/Sales.xml");

fn fixture() -> Fixture {
    let fixture = Fixture::new(SourceSetKind::Configuration);
    fixture.put("Sequence", "Sequences", "", "");
    std::fs::write(
        fixture.source.join("Sequences/Target.xml"),
        SEQUENCE
            .replace("Sales", "Target")
            .replace("Document.Linked", "Document.Records"),
    )
    .unwrap();
    let module = fixture
        .source
        .join("Sequences/Target/Ext/RecordSetModule.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(module, "// retained companion\r\n").unwrap();
    fixture
}

#[test]
fn sequence_boundary_mixed_comment_edit_preserves_intervening_line_endings() {
    let fixture = fixture();
    let path = fixture.source.join("Sequences/Target.xml");
    let native = std::fs::read_to_string(&path).unwrap();
    let source = native.replace("</Comment>\r\n", "</Comment>\n<!-- retain bare LF -->\n");
    assert_ne!(source, native);
    std::fs::write(&path, &source).unwrap();
    let mut expected = tree_snapshot(&fixture.source);
    let postimage = source
        .replace("boundary fixture", "mixed &amp; value")
        .replace(">DontMove<", ">Move<");
    expected.insert(
        PathBuf::from("Sequences/Target.xml"),
        postimage.as_bytes().to_vec(),
    );
    fixture.preview_and_publish("main:Sequence.Target", json!([
        {"op":"props.set","args":{"values":{"MoveBoundaryOnPosting":"Move","comment":"mixed & value"}}}
    ]));
    assert_eq!(tree_snapshot(&fixture.source), expected);
}

#[test]
fn sequence_boundary_comment_only_ignores_missing_or_damaged_mode_and_mixed_changes_refuse() {
    for kind in [SourceSetKind::Configuration, SourceSetKind::Extension] {
        for mode in ["", "<MoveBoundaryOnPosting>Unknown</MoveBoundaryOnPosting>"] {
            let fixture = Fixture::new(kind);
            fixture.put("Sequence", "Sequences", mode, "");
            let path = fixture.source.join("Sequences/Target.xml");
            let before = std::fs::read_to_string(&path).unwrap();
            fixture.preview_and_publish(
                "main:Sequence.Target",
                json!([
                    {"op":"props.set","args":{"values":{"Comment":"independent"}}}
                ]),
            );
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                before.replace("<Comment/>", "<Comment>independent</Comment>")
            );
            let before = tree_snapshot(&fixture.source);
            let request = request(
                "main:Sequence.Target",
                json!([
                    {"op":"props.set","args":{"values":{"MoveBoundaryOnPosting":"Move","Comment":"must not publish"}}}
                ]),
            );
            assert!(plan_hidden_v13_apply(
                &request,
                &fixture.binding,
                &fixture.admission(&request)
            )
            .is_err());
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

#[test]
fn sequence_boundary_saved_apply_roundtrip_changes_only_the_scalar() {
    let fixture = fixture();
    let original = tree_snapshot(&fixture.source);
    let path = fixture.source.join("Sequences/Target.xml");
    let before = std::fs::read_to_string(&path).unwrap();
    for (value, expected) in [
        ("Move", before.replace(">DontMove<", ">Move<")),
        ("DontMove", before.clone()),
    ] {
        fixture.preview_and_publish(
            "main:Sequence.Target",
            json!([
                {"op":"props.set","args":{"values":{"MoveBoundaryOnPosting":value}}}
            ]),
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }
    assert_eq!(tree_snapshot(&fixture.source), original);
}

#[test]
fn sequence_boundary_saved_plan_rejects_changed_descriptor_or_registration_even_for_noop() {
    for (relative, old, new) in [
        ("Sequences/Target.xml", "boundary fixture", "external edit"),
        ("Configuration.xml", "<Sequence>Target</Sequence>", ""),
    ] {
        for value in ["Move", "DontMove"] {
            let fixture = fixture();
            let request = request(
                "main:Sequence.Target",
                json!([
                    {"op":"props.set","args":{"values":{"MoveBoundaryOnPosting":value}}}
                ]),
            );
            let batch = fixture.prepare(&request);
            let previewed = fixture.actor.preview_prepared_apply(&batch).unwrap();
            let mut preview = DomainResult::success("preview");
            preview.rev = Some(previewed.rev().to_owned());
            let token = fixture.actor.save_prepared_apply(batch, preview).unwrap();
            let path = fixture.source.join(relative);
            let bytes = std::fs::read_to_string(&path).unwrap();
            let changed = bytes.replace(old, new);
            assert_ne!(changed, bytes);
            std::fs::write(path, changed).unwrap();
            let before = tree_snapshot(&fixture.source);
            fixture
                .actor
                .execute_saved_apply(
                    &token,
                    ProviderDeadline::no_deadline(),
                    &CancellationToken::new(),
                    |result, publication| {
                        assert_eq!(
                            publication
                                .map(|_| ())
                                .expect_err("stale input must refuse")
                                .kind(),
                            ApplyPublicationErrorKind::ConcurrentRevision
                        );
                        result
                    },
                )
                .unwrap();
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

#[test]
fn sequence_boundary_invalid_arguments_and_other_owners_refuse_without_writes() {
    let fixture = fixture();
    for (at, values) in [
        (
            "main:Sequence.Target",
            json!({"MoveBoundaryOnPosting":"Unknown"}),
        ),
        (
            "main:Sequence.Target",
            json!({"MoveBoundaryOnPosting":true}),
        ),
        ("main:Sequence.Target", json!({"Documents":[]})),
        (
            "main:Document.Records",
            json!({"MoveBoundaryOnPosting":"Move"}),
        ),
    ] {
        let before = tree_snapshot(&fixture.source);
        let first = if at.starts_with("main:Sequence.") {
            json!({"MoveBoundaryOnPosting":"Move"})
        } else {
            json!({"Comment":"must not publish"})
        };
        let request = request(
            at,
            json!([
                {"op":"props.set","args":{"values":first}},
                {"op":"props.set","args":{"values":values}}
            ]),
        );
        let result =
            plan_hidden_v13_apply(&request, &fixture.binding, &fixture.admission(&request));
        assert!(result.is_err(), "{at}");
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

#[test]
fn sequence_boundary_refuses_extension_or_orphan_targets_even_for_noop() {
    for kind in [SourceSetKind::Configuration, SourceSetKind::Extension] {
        let fixture = Fixture::new(kind);
        fixture.put(
            "Sequence",
            "Sequences",
            "<MoveBoundaryOnPosting>DontMove</MoveBoundaryOnPosting>",
            "",
        );
        if kind == SourceSetKind::Configuration {
            let path = fixture.source.join("Configuration.xml");
            let bytes = std::fs::read_to_string(&path).unwrap();
            std::fs::write(path, bytes.replace("<Sequence>Target</Sequence>", "")).unwrap();
        }
        for value in ["Move", "DontMove"] {
            let before = tree_snapshot(&fixture.source);
            let request = request(
                "main:Sequence.Target",
                json!([
                    {"op":"props.set","args":{"values":{"MoveBoundaryOnPosting":value}}}
                ]),
            );
            assert!(plan_hidden_v13_apply(
                &request,
                &fixture.binding,
                &fixture.admission(&request)
            )
            .is_err());
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

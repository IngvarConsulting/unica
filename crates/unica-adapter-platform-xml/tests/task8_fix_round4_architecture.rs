use std::{fs, path::PathBuf};

#[test]
fn preservation_oracle_has_no_implementation_captured_hashes_or_markers() {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/task8_fix_round1_preservation_matrix.rs"),
    )
    .unwrap();
    for forbidden in [
        "Sha256",
        "fact_set_digest",
        "expected_fact_digests",
        "before_fact_digest",
        "after_fact_digest",
        "ExpectedFact",
        "expected_fact_matches",
    ] {
        assert!(
            !source.contains(forbidden),
            "implementation-captured preservation oracle remains: {forbidden}"
        );
    }
}

#[test]
fn every_writer_variant_has_a_readable_provenance_bearing_fact_fixture() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/task8-writer-semantic-facts");
    let mut fixtures = fs::read_dir(&root)
        .unwrap_or_else(|error| {
            panic!(
                "missing readable fact fixtures at {}: {error}",
                root.display()
            )
        })
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    fixtures.sort();
    assert_eq!(fixtures.len(), 25, "{fixtures:?}");

    for fixture in fixtures {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&fixture).unwrap()).unwrap();
        assert!(
            value
                .get("provenance")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|provenance| {
                    provenance
                        .get("source")
                        .and_then(serde_json::Value::as_str)
                        .is_some()
                        && provenance
                            .get("reviewedEvidence")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|items| !items.is_empty())
                }),
            "{} lacks independent provenance",
            fixture.display()
        );
        for field in ["added", "removed"] {
            assert!(
                value
                    .get(field)
                    .and_then(serde_json::Value::as_array)
                    .is_some(),
                "{} lacks exact {field} fact multiset",
                fixture.display()
            );
        }
    }
}

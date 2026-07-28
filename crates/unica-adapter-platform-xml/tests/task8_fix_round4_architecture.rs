use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

const APPROVED_PROVENANCE_SOURCES: &[&str] = &[
    "spec/designs/2026-07-26-versioned-source-adapter-architecture.md",
    "crates/unica-adapter-platform-xml/tests/legacy_parity.rs",
    "crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/legacy-semantic-oracle.json",
    "plugins/unica/references/specs/1c-dcs-spec.md",
    "plugins/unica/references/specs/1c-spreadsheet-spec.md",
];

const FORBIDDEN_ORACLE_MARKERS: &[&str] = &[
    ".superpowers/",
    "workspace:",
    "session:",
    "sourceId",
    "objectKey",
    "relationKey",
    "groupKey",
    "ordinal_path",
    "ordinalPath",
    "<semantic-hash>",
    "<semantic-id>",
    "\"document\":",
];

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("adapter crate is two levels below the repository root")
        .to_path_buf()
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/task8-writer-semantic-facts")
}

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn validate_fixture(value: &serde_json::Value, repo: &Path) -> Result<(), String> {
    if value["schemaVersion"] != 2 {
        return Err("semantic oracle schemaVersion must be 2".to_string());
    }
    let provenance = value["provenance"]
        .as_object()
        .ok_or_else(|| "semantic oracle lacks provenance".to_string())?;
    if provenance
        .get("verification")
        .and_then(serde_json::Value::as_str)
        != Some("reviewed-against-tracked-sources")
    {
        return Err("semantic oracle provenance is unverified".to_string());
    }
    let normalization = provenance
        .get("normalization")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "semantic oracle lacks normalization policy".to_string())?;
    if !normalization.contains("identities")
        || !normalization.contains("properties")
        || !normalization.contains("relations")
        || !normalization.contains("coverage")
    {
        return Err("semantic normalization omits required semantic domains".to_string());
    }
    let sources = provenance
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "semantic oracle lacks reviewed sources".to_string())?;
    if sources.len() < 2 {
        return Err("semantic oracle requires at least two reviewed sources".to_string());
    }
    for source in sources {
        let path = source["path"]
            .as_str()
            .ok_or_else(|| "provenance source lacks path".to_string())?;
        if !APPROVED_PROVENANCE_SOURCES.contains(&path) {
            return Err(format!("unapproved provenance source: {path}"));
        }
        let absolute = repo.join(path);
        if !absolute.is_file() {
            return Err(format!("nonexistent provenance source: {path}"));
        }
        let expected_hash = source["sha256"]
            .as_str()
            .ok_or_else(|| format!("provenance source lacks hash: {path}"))?;
        if expected_hash.len() != 64
            || !expected_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            || sha256(&absolute) != expected_hash
        {
            return Err(format!("provenance hash mismatch: {path}"));
        }
        let line_start = source["lineStart"]
            .as_u64()
            .ok_or_else(|| format!("provenance source lacks lineStart: {path}"))?;
        let line_end = source["lineEnd"]
            .as_u64()
            .ok_or_else(|| format!("provenance source lacks lineEnd: {path}"))?;
        let line_count = fs::read_to_string(&absolute)
            .map_err(|error| format!("provenance source is not text: {path}: {error}"))?
            .lines()
            .count() as u64;
        if line_start == 0 || line_end < line_start || line_end > line_count {
            return Err(format!(
                "invalid provenance line range {line_start}-{line_end} for {path}"
            ));
        }
        if source["claim"]
            .as_str()
            .is_none_or(|claim| claim.trim().is_empty())
        {
            return Err(format!("provenance source lacks a verified claim: {path}"));
        }
    }
    let serialized = serde_json::to_string(value).unwrap();
    for marker in FORBIDDEN_ORACLE_MARKERS {
        if serialized.contains(marker) {
            return Err(format!("reader-internal oracle marker remains: {marker}"));
        }
    }
    for field in ["added", "removed"] {
        if !value[field].is_array() {
            return Err(format!("semantic oracle lacks exact {field} fact multiset"));
        }
    }
    let added = value["added"]
        .as_array()
        .expect("validated added fact multiset must be an array");
    let removed = value["removed"]
        .as_array()
        .expect("validated removed fact multiset must be an array");
    if removed.iter().any(|removed_entry| {
        added
            .iter()
            .any(|added_entry| removed_entry["fact"] == added_entry["fact"])
    }) {
        return Err("unchanged semantic fact appears in both delta directions".to_string());
    }
    Ok(())
}

#[test]
fn preservation_oracle_has_no_implementation_captured_hashes_or_markers() {
    let source = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/task8_fix_round1_preservation_matrix.rs"),
    )
    .unwrap();
    for forbidden in [
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
fn every_writer_variant_has_verified_readable_semantic_fact_provenance() {
    let root = fixture_root();
    let repo = repository_root();
    let mut fixtures = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    fixtures.sort();
    assert_eq!(fixtures.len(), 25, "{fixtures:?}");

    for fixture in fixtures {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&fixture).unwrap()).unwrap();
        validate_fixture(&value, &repo)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.display()));
    }
}

#[test]
fn provenance_guard_rejects_missing_sources_hash_drift_internals_and_unverified_claims() {
    let repo = repository_root();
    let path = fixture_root().join("ConfigurationInitialize.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    validate_fixture(&original, &repo).unwrap();

    let mut missing = original.clone();
    missing["provenance"]["sources"][0]["path"] = "spec/designs/not-present.md".into();
    assert!(validate_fixture(&missing, &repo).is_err());

    let mut drift = original.clone();
    drift["provenance"]["sources"][0]["sha256"] = "0".repeat(64).into();
    assert!(validate_fixture(&drift, &repo).is_err());

    let mut internal = original.clone();
    internal["added"][0]["fact"]["sourceId"] = "workspace:captured".into();
    assert!(validate_fixture(&internal, &repo).is_err());

    let mut unverified = original;
    unverified["provenance"]["verification"] = "unverified".into();
    assert!(validate_fixture(&unverified, &repo).is_err());
}

#[test]
fn dcs_mxl_and_form_boundaries_encode_the_correct_ownership_direction() {
    let repo = repository_root();
    let operations = fs::read_to_string(
        repo.join("crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs"),
    )
    .unwrap();
    assert!(operations.contains("family_owned_artifact_candidate"));
    let family_classifier = operations
        .split("fn classify_family_owned_revision")
        .nth(1)
        .and_then(|source| source.split("fn version_is_inherited_when_missing").next())
        .expect("family-owned schema classifier must remain a private closed function");
    assert!(!family_classifier.contains("attribute(\"version\")"));

    let dcs = fs::read_to_string(
        repo.join("crates/unica-adapter-platform-xml/src/versions/v2_20/writers/dcs.rs"),
    )
    .unwrap();
    let mxl = fs::read_to_string(
        repo.join("crates/unica-adapter-platform-xml/src/versions/v2_20/writers/mxl.rs"),
    )
    .unwrap();
    let matrix =
        fs::read_to_string(repo.join(
            "crates/unica-adapter-platform-xml/tests/task8_fix_round1_preservation_matrix.rs",
        ))
        .unwrap();
    assert!(matrix.contains("is_transport_generated_identifier"));
    assert!(matrix.contains("if value_type == \"uuid\""));
    assert!(!dcs.contains("DataCompositionSchema xmlns=\\\"http://v8.1c.ru/8.1/data-composition-system/schema\\\" xmlns:dcscom=\\\"http://v8.1c.ru/8.1/data-composition-system/common\\\" xmlns:dcscor=\\\"http://v8.1c.ru/8.1/data-composition-system/core\\\" xmlns:dcsset=\\\"http://v8.1c.ru/8.1/data-composition-system/settings\\\" xmlns:v8=\\\"http://v8.1c.ru/8.1/data/core\\\" xmlns:v8ui=\\\"http://v8.1c.ru/8.1/data/ui\\\" xmlns:xs=\\\"http://www.w3.org/2001/XMLSchema\\\" xmlns:xsi=\\\"http://www.w3.org/2001/XMLSchema-instance\\\" version="));
    assert!(!mxl.contains("xmlns:xsi=\\\"http://www.w3.org/2001/XMLSchema-instance\\\" version="));

    let core =
        fs::read_to_string(repo.join("crates/unica-format-core/src/commands/writer_payloads.rs"))
            .unwrap();
    let host = fs::read_to_string(
        repo.join("crates/unica-coder/src/infrastructure/native_operations/registry.rs"),
    )
    .unwrap();
    let adapter = fs::read_to_string(
        repo.join("crates/unica-adapter-platform-xml/src/versions/v2_20/writers/form.rs"),
    )
    .unwrap();
    assert!(core.contains("enum FormOwnerSelection"));
    assert!(core.contains("CapturedObject"));
    assert!(host.contains("FormCreate::selected_owner"));
    assert!(adapter.contains("resolve_form_create_owner"));
    assert!(adapter.contains("detect_form_add_object"));
}

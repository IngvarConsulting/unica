#[allow(dead_code)]
pub(crate) mod artifacts;
#[allow(dead_code)]
pub(crate) mod registry;
#[allow(dead_code)]
pub(crate) mod requests;
#[allow(dead_code)]
pub(crate) mod scalars;
#[allow(dead_code)]
pub(crate) mod schema;
#[allow(dead_code)]
pub(crate) mod selectors;

#[cfg(test)]
mod tests {
    use super::scalars::{
        BoundedVec, Diagnostic, DisplayPath, LocalProfileName, Name, Narrative,
        NormalizedUtcInstant, OriginalProjectCwd, PositiveGeneration, PropertyPath, Reason,
        RepositoryVersion, Summary,
    };
    use super::schema::{
        audit_json_schema, is_i_json_lf_text, is_i_json_single_line_text,
        is_normalized_utc_instant, I_JSON_LF_TEXT_FORMAT, I_JSON_SINGLE_LINE_TEXT_FORMAT,
        NORMALIZED_UTC_INSTANT_FORMAT,
    };
    use crate::domain::branched_development::{
        BranchedLifecycleToolName, CapabilityRowId, DurableExecutionPolicy, ExecutionPolicy,
        MetadataObjectId, OperationId, ProfileArtifactRefId, ProjectId, Sha256Digest,
        SupportLayerId, TaskId, TaskPhase, UnicaId,
    };
    use regex::Regex;
    use schemars::{schema_for, JsonSchema};
    use serde_json::{json, Value};
    use std::str::FromStr;

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).unwrap()
    }

    fn assert_string_schema<T: JsonSchema>(min: u64, max: u64, pattern: Option<&str>) {
        let actual = schema::<T>();
        assert_eq!(actual["type"], "string");
        assert_eq!(actual["minLength"], min);
        assert_eq!(actual["maxLength"], max);
        assert_eq!(actual.get("pattern").and_then(Value::as_str), pattern);
    }

    fn contract_validator<T: JsonSchema>() -> jsonschema::Validator {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .with_format(I_JSON_SINGLE_LINE_TEXT_FORMAT, is_i_json_single_line_text)
            .with_format(I_JSON_LF_TEXT_FORMAT, is_i_json_lf_text)
            .with_format(NORMALIZED_UTC_INSTANT_FORMAT, is_normalized_utc_instant)
            .should_validate_formats(true)
            .should_ignore_unknown_formats(false)
            .build(&schema::<T>())
            .expect("generated contract schema must compile")
    }

    #[test]
    fn identifier_schemas_and_deserializers_enforce_the_exact_contracts() {
        assert_string_schema::<TaskId>(1, 64, Some("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"));
        assert_string_schema::<OperationId>(
            36,
            36,
            Some("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"),
        );
        assert_string_schema::<Sha256Digest>(64, 64, Some("^[0-9a-f]{64}$"));
        for id in [
            schema::<UnicaId>(),
            schema::<ProjectId>(),
            schema::<MetadataObjectId>(),
        ] {
            assert_eq!(id["type"], "string");
            assert_eq!(id["format"], "uuid");
            assert_eq!(
                id["pattern"],
                "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
            );
        }
        assert_string_schema::<ProfileArtifactRefId>(
            1,
            128,
            Some("^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$"),
        );
        assert_string_schema::<CapabilityRowId>(1, 128, Some("^[a-z0-9][a-z0-9._-]{0,127}$"));

        for invalid in ["\"bad/path\"", "\"\u{fdd0}\""] {
            assert!(serde_json::from_str::<TaskId>(invalid).is_err());
        }
        assert!(
            serde_json::from_str::<UnicaId>("\"123E4567-E89B-12D3-A456-426614174000\"").is_err()
        );
        assert!(serde_json::from_str::<CapabilityRowId>("\"Row-1\"").is_err());
        assert!(serde_json::from_str::<ProfileArtifactRefId>("\"ref/path\"").is_err());
        assert!(serde_json::from_str::<SupportLayerId>("\"layer\\nname\"").is_err());
    }

    #[test]
    fn bounded_text_counts_unicode_scalars_and_enforces_i_json_controls() {
        assert_string_schema::<Name>(1, 256, None);
        assert_string_schema::<Summary>(1, 2048, None);
        assert_string_schema::<Narrative>(1, 4096, None);
        assert_string_schema::<Reason>(1, 4096, None);
        assert_string_schema::<DisplayPath>(1, 4096, None);
        assert_string_schema::<PropertyPath>(1, 2048, None);
        assert_string_schema::<Diagnostic>(0, 8192, None);
        assert_string_schema::<SupportLayerId>(1, 256, None);
        assert_string_schema::<RepositoryVersion>(1, 128, None);
        assert_string_schema::<OriginalProjectCwd>(1, 4096, None);
        assert_string_schema::<LocalProfileName>(1, 256, None);

        assert!(Name::from_str(&"界".repeat(256)).is_ok());
        assert!(Name::from_str(&"界".repeat(257)).is_err());
        assert!(Narrative::from_str("first\nsecond").is_ok());
        for value in ["first\r\nsecond", "tab\t", "\u{fdd0}"] {
            assert!(Narrative::from_str(value).is_err(), "accepted {value:?}");
        }
        assert!(Diagnostic::from_str("").is_ok());
        assert!(Diagnostic::from_str(&"x".repeat(8193)).is_err());
        assert!(SupportLayerId::from_str("layer\nname").is_err());
        assert!(RepositoryVersion::from_str("v\u{ffff}").is_err());
        assert!(OriginalProjectCwd::from_str("/workspace/project").is_ok());
        assert!(OriginalProjectCwd::from_str("/workspace\n/project").is_err());
        assert!(LocalProfileName::from_str("profile\tname").is_err());
        assert!(serde_json::from_str::<Name>("\"\\u0000\"").is_err());
        assert!(serde_json::from_str::<Summary>("\"summary\\nline\"").is_err());
        assert!(serde_json::from_str::<Narrative>("\"line\\r\\nline\"").is_err());
        assert!(serde_json::from_str::<Reason>("\"reason\\ttext\"").is_err());
        assert!(serde_json::from_str::<DisplayPath>("\"display\\tpath\"").is_err());
        assert!(serde_json::from_str::<PropertyPath>("\"property\\u0001\"").is_err());
        assert!(serde_json::from_str::<Diagnostic>("\"\u{fdd0}\"").is_err());
        assert!(serde_json::from_str::<RepositoryVersion>("\"version\\n1\"").is_err());
        assert!(serde_json::from_str::<OriginalProjectCwd>("\"/project\\tname\"").is_err());
        assert!(serde_json::from_str::<LocalProfileName>("\"profile\\rname\"").is_err());

        let single_line = contract_validator::<Name>();
        assert!(single_line.is_valid(&json!("界".repeat(256))));
        assert!(!single_line.is_valid(&json!("界".repeat(257))));
        for invalid in ["\u{0}", "line\nline", "\u{fdd0}", "\u{1fffe}"] {
            assert!(
                !single_line.is_valid(&json!(invalid)),
                "single-line schema accepted {invalid:?}"
            );
        }
        let narrative = contract_validator::<Narrative>();
        assert!(narrative.is_valid(&json!("first\nsecond")));
        for invalid in ["first\rsecond", "tab\t", "\u{ffff}", "\u{10ffff}"] {
            assert!(
                !narrative.is_valid(&json!(invalid)),
                "LF-text schema accepted {invalid:?}"
            );
        }
        let support_layer = contract_validator::<SupportLayerId>();
        assert!(!support_layer.is_valid(&json!("layer\nname")));
    }

    #[test]
    fn normalized_utc_instants_are_validated_and_digest_stable() {
        for value in [
            "2026-07-22T01:02:03Z",
            "2024-02-29T23:59:59.123456789Z",
            "2026-07-22T01:02:03.1Z",
        ] {
            let instant = NormalizedUtcInstant::from_str(value).unwrap();
            assert_eq!(
                serde_json::to_string(&instant).unwrap(),
                format!("\"{value}\"")
            );
        }
        for invalid in [
            "2026-02-29T01:02:03Z",
            "2026-07-22t01:02:03z",
            "2026-07-22T01:02:03+00:00",
            "2026-07-22T01:02:03.0Z",
            "2026-07-22T01:02:03.120Z",
            "2026-07-22T01:02:60Z",
        ] {
            assert!(
                NormalizedUtcInstant::from_str(invalid).is_err(),
                "accepted {invalid}"
            );
        }
        assert!(
            serde_json::from_str::<NormalizedUtcInstant>("\"2026-07-22T01:02:03.0Z\"").is_err()
        );
        assert_string_schema::<NormalizedUtcInstant>(
            20,
            30,
            Some(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.(?:\d{0,8}[1-9]))?Z$"),
        );
        assert_eq!(
            schema::<NormalizedUtcInstant>()["format"],
            NORMALIZED_UTC_INSTANT_FORMAT
        );
        let instant_schema = schema::<NormalizedUtcInstant>();
        let pattern = instant_schema["pattern"].as_str().unwrap();
        let expression = Regex::new(pattern).unwrap();
        for valid in ["2026-07-22T01:02:03.1Z", "2026-07-22T01:02:03.123456789Z"] {
            assert!(expression.is_match(valid), "schema rejected {valid}");
        }
        for invalid in [
            "2026-07-22T01:02:03.0Z",
            "2026-07-22T01:02:03.120Z",
            "2026-07-22T01:02:03.1234567890Z",
        ] {
            assert!(!expression.is_match(invalid), "schema accepted {invalid}");
        }

        let validator = contract_validator::<NormalizedUtcInstant>();
        assert!(validator.is_valid(&json!("2024-02-29T23:59:59.123456789Z")));
        for invalid in [
            "2026-02-29T01:02:03Z",
            "2026-13-01T01:02:03Z",
            "2026-07-22T24:00:00Z",
            "2026-07-22T01:02:60Z",
        ] {
            assert!(
                !validator.is_valid(&json!(invalid)),
                "timestamp schema accepted {invalid}"
            );
        }
    }

    #[test]
    fn positive_generations_and_bounded_vectors_enforce_i_json_limits() {
        let maximum_i_json_integer = (1_u64 << 53) - 1;
        assert_eq!(PositiveGeneration::new(1).unwrap().get(), 1);
        assert_eq!(
            PositiveGeneration::new(maximum_i_json_integer)
                .unwrap()
                .get(),
            maximum_i_json_integer
        );
        assert!(PositiveGeneration::new(0).is_err());
        assert!(PositiveGeneration::new(maximum_i_json_integer + 1).is_err());
        assert!(serde_json::from_str::<PositiveGeneration>("0").is_err());
        assert!(serde_json::from_str::<PositiveGeneration>("9007199254740992").is_err());
        let generation_schema = schema::<PositiveGeneration>();
        assert_eq!(generation_schema["type"], "integer");
        assert_eq!(generation_schema["minimum"], 1);
        assert_eq!(generation_schema["maximum"], maximum_i_json_integer);

        let values = BoundedVec::<TaskId, 2>::new(vec![TaskId::from_str("one").unwrap()]).unwrap();
        assert_eq!(values.as_slice().len(), 1);
        assert!(BoundedVec::<TaskId, 2>::new(vec![
            TaskId::from_str("a").unwrap(),
            TaskId::from_str("b").unwrap(),
            TaskId::from_str("c").unwrap()
        ])
        .is_err());
        assert!(serde_json::from_str::<BoundedVec<TaskId, 2>>(r#"["a","b","c"]"#).is_err());
        let vector_schema = schema::<BoundedVec<TaskId, 2>>();
        assert_eq!(vector_schema["type"], "array");
        assert_eq!(vector_schema["maxItems"], 2);
        assert!(vector_schema.get("items").is_some());
    }

    #[test]
    fn vocabulary_schemas_are_exact_and_durable_policy_excludes_read_only() {
        for (actual, expected) in [
            (
                schema::<TaskPhase>(),
                TaskPhase::ALL
                    .iter()
                    .map(TaskPhase::as_str)
                    .collect::<Vec<_>>(),
            ),
            (
                schema::<ExecutionPolicy>(),
                ExecutionPolicy::ALL
                    .iter()
                    .map(ExecutionPolicy::as_str)
                    .collect::<Vec<_>>(),
            ),
            (
                schema::<DurableExecutionPolicy>(),
                DurableExecutionPolicy::ALL
                    .iter()
                    .map(DurableExecutionPolicy::as_str)
                    .collect::<Vec<_>>(),
            ),
            (
                schema::<BranchedLifecycleToolName>(),
                BranchedLifecycleToolName::ALL
                    .iter()
                    .map(BranchedLifecycleToolName::as_str)
                    .collect::<Vec<_>>(),
            ),
        ] {
            assert_eq!(actual["type"], "string");
            assert_eq!(actual["enum"], json!(expected));
        }
        assert!(!schema::<DurableExecutionPolicy>()["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("readOnly")));
    }

    #[test]
    fn schema_audit_rejects_open_or_untyped_object_and_array_leaves() {
        assert!(audit_json_schema(&json!({
            "type": "object",
            "properties": { "ids": { "type": "array", "items": { "type": "string" }, "maxItems": 2 } },
            "required": ["ids"],
            "additionalProperties": false,
        }))
        .is_ok());
        assert!(audit_json_schema(&json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        }))
        .is_ok());
        for schema in [
            json!({"type": "object", "properties": {}}),
            json!({"type": "object", "additionalProperties": true}),
            json!({"type": "array"}),
            json!({"type": "array", "items": {}}),
        ] {
            assert!(audit_json_schema(&schema).is_err(), "accepted {schema}");
        }
    }

    #[test]
    fn schema_audit_resolves_local_refs_and_rejects_hidden_open_shapes() {
        assert!(audit_json_schema(&json!({
            "$defs": {
                "Closed": {
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"],
                    "additionalProperties": false
                }
            },
            "$ref": "#/$defs/Closed"
        }))
        .is_ok());

        for schema in [
            json!({"not": {"type": "object"}}),
            json!({"$defs": {}, "$ref": "#/$defs/Missing"}),
            json!({
                "$defs": {"Open": {"type": "object", "properties": {}}},
                "$ref": "#/$defs/Open"
            }),
            json!({"$ref": "https://example.invalid/schema.json"}),
            json!({"type": ["object", "null"], "properties": {}}),
            json!({"type": ["array", "null"]}),
            json!({
                "type": "object",
                "properties": {},
                "patternProperties": {".*": {}},
                "additionalProperties": false
            }),
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
                "unevaluatedProperties": false
            }),
            json!({"$defs": {"Loop": {"$ref": "#/$defs/Loop"}}, "$ref": "#/$defs/Loop"}),
            json!({"type": "string", "unknownEscapeHatch": true}),
        ] {
            assert!(audit_json_schema(&schema).is_err(), "accepted {schema}");
        }

        assert!(audit_json_schema(&json!({
            "type": ["object", "null"],
            "properties": {},
            "additionalProperties": false
        }))
        .is_ok());
    }

    #[test]
    fn generated_foundation_schemas_pass_the_fail_closed_audit() {
        for generated in [
            schema::<Name>(),
            schema::<Narrative>(),
            schema::<NormalizedUtcInstant>(),
            schema::<BoundedVec<TaskId, 2>>(),
            schema::<TaskPhase>(),
            schema::<ExecutionPolicy>(),
            schema::<DurableExecutionPolicy>(),
            schema::<BranchedLifecycleToolName>(),
        ] {
            audit_json_schema(&generated).unwrap_or_else(|error| {
                panic!("generated schema failed audit: {error}: {generated}")
            });
        }
    }
}

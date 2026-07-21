use super::{BranchedLifecycleToolName, DurableExecutionPolicy, Sha256Digest};
use serde::Serialize;
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use std::fmt;

const MAX_I_JSON_INTEROPERABLE_INTEGER: u64 = (1 << 53) - 1;
const OPERATION_INPUT_DIGEST_KIND: &str = "branchedOperationInputV1";

#[derive(Debug)]
pub(crate) enum CanonicalJsonError {
    RequestMustBeObject,
    NonInteroperableInteger { value: String },
    Canonicalization(serde_json::Error),
}

impl fmt::Display for CanonicalJsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestMustBeObject => {
                formatter.write_str("operation request must be a JSON object")
            }
            Self::NonInteroperableInteger { value } => write!(
                formatter,
                "integer {value} is outside the I-JSON interoperability range"
            ),
            Self::Canonicalization(error) => {
                write!(formatter, "JSON canonicalization failed: {error}")
            }
        }
    }
}

impl std::error::Error for CanonicalJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Canonicalization(error) => Some(error),
            Self::RequestMustBeObject | Self::NonInteroperableInteger { .. } => None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationInputDigestRecord<'a> {
    digest_kind: &'static str,
    tool_name: BranchedLifecycleToolName,
    execution_policy: DurableExecutionPolicy,
    request: &'a Map<String, Value>,
}

/// Returns RFC 8785 canonical UTF-8 JSON bytes for an already duplicate-free I-JSON value.
fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, CanonicalJsonError> {
    validate_i_json_numbers(value)?;
    canonical_json_bytes_for(value)
}

/// Returns the SHA-256 digest of RFC 8785 canonical JSON for a duplicate-free I-JSON value.
fn canonical_json_digest(value: &Value) -> Result<Sha256Digest, CanonicalJsonError> {
    canonical_json_bytes(value).map(|bytes| sha256_digest(&bytes))
}

/// Returns the domain-separated digest for one durable branched operation request.
pub(crate) fn operation_input_digest(
    tool_name: BranchedLifecycleToolName,
    execution_policy: DurableExecutionPolicy,
    request: &Value,
) -> Result<Sha256Digest, CanonicalJsonError> {
    let request = request
        .as_object()
        .ok_or(CanonicalJsonError::RequestMustBeObject)?;
    for value in request.values() {
        validate_i_json_numbers(value)?;
    }
    let mut request = request.clone();
    request.remove("operationId");

    let record = OperationInputDigestRecord {
        digest_kind: OPERATION_INPUT_DIGEST_KIND,
        tool_name,
        execution_policy,
        request: &request,
    };
    canonical_json_bytes_for(&record).map(|bytes| sha256_digest(&bytes))
}

fn canonical_json_bytes_for<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalJsonError> {
    serde_json_canonicalizer::to_vec(value).map_err(CanonicalJsonError::Canonicalization)
}

fn sha256_digest(bytes: &[u8]) -> Sha256Digest {
    let hex = format!("{:x}", Sha256::digest(bytes));
    Sha256Digest::parse(&hex).expect("SHA-256 lower-hex output always satisfies Sha256Digest")
}

fn validate_i_json_numbers(value: &Value) -> Result<(), CanonicalJsonError> {
    match value {
        Value::Array(values) => {
            for value in values {
                validate_i_json_numbers(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_i_json_numbers(value)?;
            }
        }
        Value::Number(number) => validate_i_json_number(number)?,
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
    Ok(())
}

fn validate_i_json_number(number: &Number) -> Result<(), CanonicalJsonError> {
    let interoperable = number
        .as_i64()
        .map(|value| value.unsigned_abs() <= MAX_I_JSON_INTEROPERABLE_INTEGER)
        .or_else(|| {
            number
                .as_u64()
                .map(|value| value <= MAX_I_JSON_INTEROPERABLE_INTEGER)
        })
        .or_else(|| {
            number.as_f64().map(|value| {
                value.is_finite()
                    && (value.fract() != 0.0
                        || value.abs() <= MAX_I_JSON_INTEROPERABLE_INTEGER as f64)
            })
        })
        .unwrap_or(false);

    interoperable
        .then_some(())
        .ok_or_else(|| CanonicalJsonError::NonInteroperableInteger {
            value: number.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_json_bytes, canonical_json_digest, operation_input_digest, CanonicalJsonError,
    };
    use crate::domain::branched_development::{BranchedLifecycleToolName, DurableExecutionPolicy};
    use serde_json::json;

    #[test]
    fn canonical_json_matches_the_contract_golden_vectors() {
        let vectors = [
            (
                json!([]),
                b"[]".as_slice(),
                "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945",
            ),
            (
                json!({"b": 2, "a": 1}),
                br#"{"a":1,"b":2}"#.as_slice(),
                "43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777",
            ),
            (
                json!({"z": null, "a": "€"}),
                "{\"a\":\"€\",\"z\":null}".as_bytes(),
                "31eb5e4b861ebb6f087b97caf4f3009898b494ee322dd037ac9fa57a330b5315",
            ),
        ];

        for (value, expected_bytes, expected_digest) in vectors {
            assert_eq!(canonical_json_bytes(&value).unwrap(), expected_bytes);
            assert_eq!(
                canonical_json_digest(&value).unwrap().as_str(),
                expected_digest
            );
        }
    }

    #[test]
    fn operation_input_digest_is_independent_of_request_key_order() {
        let left = json!({"taskId": "TASK-137", "operationId": "123e4567-e89b-12d3-a456-426614174000", "approval": {"digest": "a", "decision": "apply"}});
        let right = json!({"approval": {"decision": "apply", "digest": "a"}, "operationId": "123e4567-e89b-12d3-a456-426614174000", "taskId": "TASK-137"});

        assert_eq!(
            operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &left,
            )
            .unwrap(),
            operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &right,
            )
            .unwrap(),
        );
    }

    #[test]
    fn operation_input_digest_binds_the_exact_tool_and_durable_policy() {
        let request =
            json!({"taskId": "TASK-137", "operationId": "123e4567-e89b-12d3-a456-426614174000"});
        let baseline = operation_input_digest(
            BranchedLifecycleToolName::MergeApply,
            DurableExecutionPolicy::JournaledEffect,
            &request,
        )
        .unwrap();

        assert_ne!(
            baseline,
            operation_input_digest(
                BranchedLifecycleToolName::MergeVerify,
                DurableExecutionPolicy::JournaledEffect,
                &request,
            )
            .unwrap(),
        );
        assert_ne!(
            baseline,
            operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::Contained,
                &request,
            )
            .unwrap(),
        );
    }

    #[test]
    fn operation_input_digest_removes_only_the_top_level_operation_id() {
        let request = json!({
            "operationId": "123e4567-e89b-12d3-a456-426614174000",
            "taskId": "TASK-137",
            "nested": {"operationId": "nested-a", "evidenceDigest": null},
            "array": [false, 0, null]
        });
        let different_top_level_id = json!({
            "operationId": "123e4567-e89b-12d3-a456-426614174001",
            "taskId": "TASK-137",
            "nested": {"operationId": "nested-a", "evidenceDigest": null},
            "array": [false, 0, null]
        });
        let different_nested_id = json!({
            "operationId": "123e4567-e89b-12d3-a456-426614174000",
            "taskId": "TASK-137",
            "nested": {"operationId": "nested-b", "evidenceDigest": null},
            "array": [false, 0, null]
        });
        let digest = operation_input_digest(
            BranchedLifecycleToolName::MergeApply,
            DurableExecutionPolicy::JournaledEffect,
            &request,
        )
        .unwrap();

        assert_eq!(
            digest,
            operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &different_top_level_id,
            )
            .unwrap(),
        );
        assert_ne!(
            digest,
            operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &different_nested_id,
            )
            .unwrap(),
        );
    }

    #[test]
    fn operation_input_digest_requires_a_json_object_request() {
        assert!(matches!(
            operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &json!([]),
            ),
            Err(CanonicalJsonError::RequestMustBeObject)
        ));
    }

    #[test]
    fn canonical_json_rejects_integers_outside_the_i_json_interoperability_range() {
        for value in [
            json!(9_007_199_254_740_992_u64),
            json!(-9_007_199_254_740_992_i64),
        ] {
            assert!(matches!(
                canonical_json_bytes(&value),
                Err(CanonicalJsonError::NonInteroperableInteger { .. })
            ));
            assert!(matches!(
                canonical_json_digest(&value),
                Err(CanonicalJsonError::NonInteroperableInteger { .. })
            ));
        }
    }

    #[test]
    fn canonical_json_checks_safe_integer_range_for_decimal_and_exponent_forms() {
        for value in [
            json!(9_007_199_254_740_991.0),
            json!(-9_007_199_254_740_991.0),
            json!(9.007_199_254_740_991e15),
            json!(-9.007_199_254_740_991e15),
        ] {
            assert!(canonical_json_digest(&value).is_ok(), "accepted {value}");
        }

        for value in [
            json!(9_007_199_254_740_992.0),
            json!(9_007_199_254_740_993.0),
            json!(-9_007_199_254_740_992.0),
            json!(-9_007_199_254_740_993.0),
            json!(9.007_199_254_740_992e15),
            json!(-9.007_199_254_740_992e15),
        ] {
            assert!(matches!(
                canonical_json_digest(&value),
                Err(CanonicalJsonError::NonInteroperableInteger { .. })
            ));
        }
    }

    #[test]
    fn operation_input_digest_checks_safe_integer_range_for_decimal_and_exponent_forms() {
        let request = |number| {
            json!({
                "operationId": "123e4567-e89b-12d3-a456-426614174000",
                "taskId": "TASK-137",
                "number": number,
            })
        };

        for number in [9_007_199_254_740_991.0, 9.007_199_254_740_991e15] {
            assert!(operation_input_digest(
                BranchedLifecycleToolName::MergeApply,
                DurableExecutionPolicy::JournaledEffect,
                &request(number),
            )
            .is_ok());
        }

        for number in [
            9_007_199_254_740_992.0,
            9_007_199_254_740_993.0,
            9.007_199_254_740_992e15,
        ] {
            assert!(matches!(
                operation_input_digest(
                    BranchedLifecycleToolName::MergeApply,
                    DurableExecutionPolicy::JournaledEffect,
                    &request(number),
                ),
                Err(CanonicalJsonError::NonInteroperableInteger { .. })
            ));
        }
    }
}

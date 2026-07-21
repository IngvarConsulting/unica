use super::{BranchedLifecycleToolName, DurableExecutionPolicy, Sha256Digest};
#[cfg(test)]
use crate::domain::i_json::validate_i_json_value;
use crate::domain::i_json::{validate_i_json_object, IJsonValidationError};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;

pub(super) mod contract_digest_record_sealed {
    pub trait Sealed {}
}

/// Marker for an exact closed contract digest preimage.
///
/// Only sibling modules in the branched-development domain can implement the
/// sealed marker, so production callers cannot hash arbitrary JSON values.
pub(super) trait ContractDigestRecord:
    Serialize + contract_digest_record_sealed::Sealed
{
}

const OPERATION_INPUT_DIGEST_KIND: &str = "branchedOperationInputV1";

#[derive(Debug)]
pub(super) enum CanonicalJsonError {
    RequestMustBeObject,
    NonInteroperableInteger,
    NonInteroperableString,
    RetainedEncodingMismatch,
    TypedRoundTripMismatch,
    Canonicalization(serde_json::Error),
}

impl fmt::Display for CanonicalJsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestMustBeObject => {
                formatter.write_str("operation request must be a JSON object")
            }
            Self::NonInteroperableInteger => {
                formatter.write_str("integer is outside the I-JSON interoperability range")
            }
            Self::NonInteroperableString => {
                formatter.write_str("string contains an I-JSON forbidden Unicode scalar")
            }
            Self::RetainedEncodingMismatch => formatter.write_str(
                "retained contract bytes are not the exact canonical encoding of the typed record",
            ),
            Self::TypedRoundTripMismatch => formatter
                .write_str("typed JSON serialization disagrees with its strict I-JSON round trip"),
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
            Self::RequestMustBeObject
            | Self::NonInteroperableInteger
            | Self::NonInteroperableString
            | Self::RetainedEncodingMismatch
            | Self::TypedRoundTripMismatch => None,
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
#[cfg(test)]
fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, CanonicalJsonError> {
    validate_i_json_value(value).map_err(canonical_json_validation_error)?;
    canonical_json_bytes_for(value)
}

/// Returns the SHA-256 digest of RFC 8785 canonical JSON for a duplicate-free I-JSON value.
#[cfg(test)]
fn canonical_json_digest(value: &Value) -> Result<Sha256Digest, CanonicalJsonError> {
    canonical_json_bytes(value).map(|bytes| sha256_digest(&bytes))
}

/// Returns the domain-separated digest for one durable branched operation request.
pub(super) fn operation_input_digest(
    tool_name: BranchedLifecycleToolName,
    execution_policy: DurableExecutionPolicy,
    request: &Value,
) -> Result<Sha256Digest, CanonicalJsonError> {
    let request = request
        .as_object()
        .ok_or(CanonicalJsonError::RequestMustBeObject)?;
    validate_i_json_object(request).map_err(canonical_json_validation_error)?;
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

/// Hashes a schema-valid typed contract record using the one production JCS path.
///
/// The first serialization is validation-only: reparsing it through the strict
/// I-JSON parser preserves and rejects duplicate object members before the JCS
/// serializer can order members. Only the subsequently canonicalized bytes are
/// hashed.
pub(super) fn canonical_contract_digest<T: ContractDigestRecord>(
    value: &T,
    retained_canonical_encoding: Option<&[u8]>,
) -> Result<Sha256Digest, CanonicalJsonError> {
    // Running JCS on the original typed serializer first is significant:
    // serde_json's ordinary serializer maps non-finite floats to `null`, while
    // the RFC 8785 serializer rejects them.
    let typed_canonical_bytes = canonical_json_bytes_for(value)?;
    let validation_bytes =
        serde_json::to_vec(value).map_err(CanonicalJsonError::Canonicalization)?;
    let strict_value = crate::domain::i_json::from_slice(&validation_bytes)
        .map_err(CanonicalJsonError::Canonicalization)?;
    let strict_canonical_bytes = canonical_json_bytes_for(&strict_value)?;
    if typed_canonical_bytes != strict_canonical_bytes {
        return Err(CanonicalJsonError::TypedRoundTripMismatch);
    }
    if let Some(retained) = retained_canonical_encoding {
        let retained_value = crate::domain::i_json::from_slice(retained)
            .map_err(CanonicalJsonError::Canonicalization)?;
        if retained_value != strict_value || retained != typed_canonical_bytes {
            return Err(CanonicalJsonError::RetainedEncodingMismatch);
        }
    }
    Ok(sha256_digest(&typed_canonical_bytes))
}

fn sha256_digest(bytes: &[u8]) -> Sha256Digest {
    let hex = format!("{:x}", Sha256::digest(bytes));
    Sha256Digest::parse(&hex).expect("SHA-256 lower-hex output always satisfies Sha256Digest")
}

fn canonical_json_validation_error(error: IJsonValidationError) -> CanonicalJsonError {
    match error {
        IJsonValidationError::NonInteroperableNumber => CanonicalJsonError::NonInteroperableInteger,
        IJsonValidationError::NonInteroperableString => CanonicalJsonError::NonInteroperableString,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_contract_digest, canonical_json_bytes, canonical_json_bytes_for,
        canonical_json_digest, contract_digest_record_sealed, operation_input_digest,
        CanonicalJsonError, ContractDigestRecord,
    };
    use crate::domain::branched_development::{BranchedLifecycleToolName, DurableExecutionPolicy};
    use serde::ser::SerializeMap;
    use serde::{Serialize, Serializer};
    use serde_json::{json, Map, Value};

    impl contract_digest_record_sealed::Sealed for Value {}
    impl ContractDigestRecord for Value {}

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
    fn raw_jcs_matches_the_rfc_8785_serialization_vector() {
        let value = json!({
            "numbers": [333_333_333.333_333_3_f64, 1E30_f64, 4.50_f64, 2e-3_f64, 1e-27_f64],
            "string": "€$\u{000f}\nA'B\"\\\\\"/",
            "literals": [null, true, false]
        });
        let expected = concat!(
            r#"{"literals":[null,true,false],"numbers":[333333333.3333333,1e+30,4.5,0.002,1e-27],"#,
            r#""string":"€$\u000f\nA'B\"\\\\\"/"}"#
        );
        assert_eq!(
            canonical_json_bytes_for(&value).unwrap(),
            expected.as_bytes()
        );

        // The contract layer deliberately applies a stricter I-JSON safe-
        // integer rule than the raw RFC serializer.
        assert!(canonical_contract_digest(&json!(1E30_f64), None).is_err());
    }

    #[test]
    fn raw_jcs_orders_object_properties_by_utf16_code_units() {
        let value = json!({
            "€": "Euro Sign",
            "\r": "Carriage Return",
            "דּ": "Hebrew Letter Dalet With Dagesh",
            "1": "One",
            "😀": "Emoji: Grinning Face",
            "\u{0080}": "Control",
            "ö": "Latin Small Letter O With Diaeresis"
        });
        let expected = concat!(
            r#"{"\r":"Carriage Return","1":"One","":"Control","ö":"Latin Small Letter O With Diaeresis","#,
            r#""€":"Euro Sign","😀":"Emoji: Grinning Face","דּ":"Hebrew Letter Dalet With Dagesh"}"#
        );
        assert_eq!(
            canonical_json_bytes_for(&value).unwrap(),
            expected.as_bytes()
        );
    }

    #[test]
    fn typed_contract_digest_uses_the_same_jcs_vectors() {
        assert_eq!(
            canonical_contract_digest(&json!([]), None)
                .unwrap()
                .as_str(),
            "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
        );
        assert_eq!(
            canonical_contract_digest(&json!({"b": 2, "a": 1}), None)
                .unwrap()
                .as_str(),
            "43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777"
        );
    }

    struct DuplicateMemberRecord;

    impl contract_digest_record_sealed::Sealed for DuplicateMemberRecord {}
    impl ContractDigestRecord for DuplicateMemberRecord {}

    impl Serialize for DuplicateMemberRecord {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("a", &1_u8)?;
            map.serialize_entry("a", &2_u8)?;
            map.end()
        }
    }

    #[test]
    fn typed_contract_digest_rejects_duplicate_serialized_members() {
        assert!(canonical_contract_digest(&DuplicateMemberRecord, None).is_err());
    }

    struct FloatingPointRecord(f64);

    impl contract_digest_record_sealed::Sealed for FloatingPointRecord {}
    impl ContractDigestRecord for FloatingPointRecord {}

    impl Serialize for FloatingPointRecord {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_f64(self.0)
        }
    }

    #[test]
    fn typed_contract_digest_rejects_non_finite_and_unsafe_numbers() {
        for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(canonical_contract_digest(&FloatingPointRecord(number), None).is_err());
        }
        assert!(canonical_contract_digest(&json!(9_007_199_254_740_992_u64), None).is_err());
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
                Err(CanonicalJsonError::NonInteroperableInteger)
            ));
            assert!(matches!(
                canonical_json_digest(&value),
                Err(CanonicalJsonError::NonInteroperableInteger)
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
                Err(CanonicalJsonError::NonInteroperableInteger)
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
                Err(CanonicalJsonError::NonInteroperableInteger)
            ));
        }
    }

    #[test]
    fn canonical_json_and_operation_input_reject_i_json_noncharacters_in_keys_and_values() {
        for character in [
            '\u{fdd0}',
            '\u{fdef}',
            '\u{fffe}',
            '\u{ffff}',
            '\u{1fffe}',
            '\u{1ffff}',
        ] {
            let value = Value::String(character.to_string());
            let mut key = Map::new();
            key.insert(character.to_string(), Value::Bool(true));

            for value in [value, Value::Object(key)] {
                assert!(matches!(
                    canonical_json_bytes(&value),
                    Err(CanonicalJsonError::NonInteroperableString)
                ));
                assert!(matches!(
                    canonical_json_digest(&value),
                    Err(CanonicalJsonError::NonInteroperableString)
                ));
                assert!(matches!(
                    operation_input_digest(
                        BranchedLifecycleToolName::MergeApply,
                        DurableExecutionPolicy::JournaledEffect,
                        &json!({ "request": value }),
                    ),
                    Err(CanonicalJsonError::NonInteroperableString)
                ));
            }
        }
    }
}

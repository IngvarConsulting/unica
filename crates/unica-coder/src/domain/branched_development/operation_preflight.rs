use crate::domain::branched_development::Sha256Digest;
use crate::domain::i_json;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug)]
pub(super) enum OperationPreflight {
    StrictJsonFailure {
        source_bytes: Arc<[u8]>,
        observed_digest: Sha256Digest,
    },
    TopLevelNotObject {
        source_bytes: Arc<[u8]>,
        observed_digest: Sha256Digest,
    },
    ForbiddenReadOnlyPolicy {
        source_bytes: Arc<[u8]>,
        observed_digest: Sha256Digest,
    },
    OpaqueCandidate {
        source_bytes: Arc<[u8]>,
        observed_digest: Sha256Digest,
    },
}

impl OperationPreflight {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Task 5B retains exact source bytes for state-corrupt evidence"
        )
    )]
    pub(super) fn source_bytes(&self) -> &Arc<[u8]> {
        match self {
            Self::StrictJsonFailure { source_bytes, .. }
            | Self::TopLevelNotObject { source_bytes, .. }
            | Self::ForbiddenReadOnlyPolicy { source_bytes, .. }
            | Self::OpaqueCandidate { source_bytes, .. } => source_bytes,
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Task 5B reports the observed exact-byte digest for corrupt state"
        )
    )]
    pub(super) fn observed_digest(&self) -> &Sha256Digest {
        match self {
            Self::StrictJsonFailure {
                observed_digest, ..
            }
            | Self::TopLevelNotObject {
                observed_digest, ..
            }
            | Self::ForbiddenReadOnlyPolicy {
                observed_digest, ..
            }
            | Self::OpaqueCandidate {
                observed_digest, ..
            } => observed_digest,
        }
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Task 5B preflights durable operation source bytes before loading fields"
    )
)]
pub(super) fn preflight(source_bytes: Arc<[u8]>) -> OperationPreflight {
    let observed_digest = Sha256Digest::parse(&format!("{:x}", Sha256::digest(&source_bytes)))
        .expect("SHA-256 lower-hex output always satisfies Sha256Digest");

    match i_json::from_slice(&source_bytes) {
        Err(_) => OperationPreflight::StrictJsonFailure {
            source_bytes,
            observed_digest,
        },
        Ok(Value::Object(object)) => {
            if matches!(object.get("policy"), Some(Value::String(policy)) if policy == "readOnly") {
                OperationPreflight::ForbiddenReadOnlyPolicy {
                    source_bytes,
                    observed_digest,
                }
            } else {
                OperationPreflight::OpaqueCandidate {
                    source_bytes,
                    observed_digest,
                }
            }
        }
        Ok(_) => OperationPreflight::TopLevelNotObject {
            source_bytes,
            observed_digest,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{preflight, OperationPreflight};
    use crate::domain::branched_development::Sha256Digest;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;

    fn source(bytes: &[u8]) -> Arc<[u8]> {
        Arc::from(bytes)
    }

    fn invalid_utf8_source() -> Arc<[u8]> {
        let mut bytes = b"{\"policy\":\"".to_vec();
        bytes.push(0xff);
        bytes.extend_from_slice(b"\"}");
        Arc::from(bytes)
    }

    fn digest(bytes: &[u8]) -> Sha256Digest {
        Sha256Digest::parse(&format!("{:x}", Sha256::digest(bytes))).unwrap()
    }

    fn assert_retains_exact_bytes(result: &OperationPreflight, expected: &Arc<[u8]>) {
        assert!(Arc::ptr_eq(result.source_bytes(), expected));
        assert_eq!(result.source_bytes().as_ref(), expected.as_ref());
        assert_eq!(result.observed_digest(), &digest(expected));
    }

    #[test]
    fn strict_failure_retains_and_hashes_invalid_source_bytes_before_parsing() {
        let bytes = invalid_utf8_source();
        let result = preflight(Arc::clone(&bytes));

        assert!(matches!(
            result,
            OperationPreflight::StrictJsonFailure { .. }
        ));
        assert_retains_exact_bytes(&result, &bytes);
    }

    #[test]
    fn exact_byte_digest_changes_for_whitespace_and_key_order() {
        let compact = source(br#"{"a":1,"b":2}"#);
        let reordered = source(br#"{ "b": 2, "a": 1 }"#);
        let compact_result = preflight(Arc::clone(&compact));
        let reordered_result = preflight(Arc::clone(&reordered));

        assert!(matches!(
            compact_result,
            OperationPreflight::OpaqueCandidate { .. }
        ));
        assert!(matches!(
            reordered_result,
            OperationPreflight::OpaqueCandidate { .. }
        ));
        assert_retains_exact_bytes(&compact_result, &compact);
        assert_retains_exact_bytes(&reordered_result, &reordered);
        assert_ne!(
            compact_result.observed_digest(),
            reordered_result.observed_digest()
        );
    }

    #[test]
    fn distinguishes_non_object_from_strict_json_failure() {
        let bytes = source(br#"["readOnly"]"#);
        let result = preflight(Arc::clone(&bytes));

        assert!(matches!(
            result,
            OperationPreflight::TopLevelNotObject { .. }
        ));
        assert_retains_exact_bytes(&result, &bytes);
    }

    #[test]
    fn rejects_literal_and_escaped_top_level_read_only_policy() {
        for bytes in [
            br#"{"policy":"readOnly"}"#.as_slice(),
            br#"{"pol\u0069cy":"read\u004fnly"}"#.as_slice(),
        ] {
            let bytes = source(bytes);
            let result = preflight(Arc::clone(&bytes));

            assert!(matches!(
                result,
                OperationPreflight::ForbiddenReadOnlyPolicy { .. }
            ));
            assert_retains_exact_bytes(&result, &bytes);
        }
    }

    #[test]
    fn accepts_nested_read_only_as_an_opaque_candidate() {
        let bytes = source(br#"{"nested":{"policy":"readOnly"},"policy":"contained"}"#);
        let result = preflight(Arc::clone(&bytes));

        assert!(matches!(result, OperationPreflight::OpaqueCandidate { .. }));
        assert_retains_exact_bytes(&result, &bytes);
    }

    #[test]
    fn duplicate_or_escape_equivalent_policy_names_fail_strictly() {
        for bytes in [
            br#"{"policy":"contained","policy":"readOnly"}"#.as_slice(),
            br#"{"policy":"contained","pol\u0069cy":"readOnly"}"#.as_slice(),
        ] {
            let bytes = source(bytes);
            let result = preflight(Arc::clone(&bytes));

            assert!(matches!(
                result,
                OperationPreflight::StrictJsonFailure { .. }
            ));
            assert_retains_exact_bytes(&result, &bytes);
        }
    }

    #[test]
    fn strict_failures_cover_invalid_utf8_trailing_data_noncharacters_and_unsafe_numbers() {
        for bytes in [
            invalid_utf8_source(),
            source(br#"{"policy":"contained"} {"unexpected":true}"#),
            source(br#"{"policy":"\uFDD0"}"#),
            source(br#"{"policy":"contained","value":9007199254740992}"#),
        ] {
            let result = preflight(Arc::clone(&bytes));

            assert!(matches!(
                result,
                OperationPreflight::StrictJsonFailure { .. }
            ));
            assert_retains_exact_bytes(&result, &bytes);
        }
    }

    #[test]
    fn every_durable_policy_yields_only_an_opaque_candidate() {
        for policy in [
            "localJournaled",
            "contained",
            "preparedJournaledEffect",
            "journaledEffect",
            "previewedJournaledEffect",
        ] {
            let bytes = source(format!(r#"{{"policy":"{policy}"}}"#).as_bytes());
            let result = preflight(Arc::clone(&bytes));

            assert!(matches!(result, OperationPreflight::OpaqueCandidate { .. }));
            assert_retains_exact_bytes(&result, &bytes);
        }
    }
}

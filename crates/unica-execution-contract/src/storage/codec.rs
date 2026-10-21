//! Closed raw sections. Metadata alone is never restored as durable evidence.
use super::*;
use std::io::{self, Write};

// JSON width of the closed metadata shape, including impossible combinations
// as a conservative upper bound. No field contains caller text or body bytes.
const HASH_BYTES: usize = 2 + 32 * 3 + 31;
const U64_BYTES: usize = 20;
const UUID_BYTES: usize = 2 + 36;
const BOOL_BYTES: usize = 5;
const KIND_BYTES: usize = object(&[("Failed", 2 + "ResumeUnsupported".len())]);
const TOOL_BYTES: usize = 2 + "Resolve".len();
const fn object(fields: &[(&str, usize)]) -> usize {
    let mut total = 2;
    let mut index = 0;
    while index < fields.len() {
        total += fields[index].0.len() + 3 + fields[index].1;
        if index != 0 {
            total += 1;
        }
        index += 1;
    }
    total
}
const DESCRIPTOR_BYTES: usize = object(&[
    ("kind", KIND_BYTES),
    ("digest", HASH_BYTES),
    ("length", U64_BYTES),
]);
pub const MAX_METADATA_BYTES: usize = object(&[
    (
        "key",
        object(&[
            ("invocation", UUID_BYTES),
            ("task", UUID_BYTES),
            ("core", HASH_BYTES),
            ("tool", TOOL_BYTES),
            ("arguments", HASH_BYTES),
            ("scope", HASH_BYTES),
        ]),
    ),
    ("revision", U64_BYTES),
    (
        "accepted",
        object(&[
            ("owner_hash", HASH_BYTES),
            (
                "window",
                object(&[("received_epoch_ms", U64_BYTES), ("budget_ms", U64_BYTES)]),
            ),
        ]),
    ),
    (
        "begun",
        object(&[
            (
                "run",
                object(&[("attempt", HASH_BYTES), ("generation", U64_BYTES)]),
            ),
            ("started_epoch_ms", U64_BYTES),
            (
                "read_authority",
                object(&[
                    ("source", HASH_BYTES),
                    ("actor_generation", U64_BYTES),
                    ("mutation_generation", U64_BYTES),
                ]),
            ),
        ]),
    ),
    ("cancel_requested", BOOL_BYTES),
    ("producer", DESCRIPTOR_BYTES),
    ("producer_digest", HASH_BYTES),
    ("primary_override", DESCRIPTOR_BYTES),
    ("primary_digest", HASH_BYTES),
    (
        "acknowledgement",
        object(&[
            ("digest", HASH_BYTES),
            ("first_acknowledged_epoch_ms", U64_BYTES),
        ]),
    ),
    ("public_task", BOOL_BYTES),
    ("compacted", BOOL_BYTES),
]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    InvalidRecord,
    TooLarge,
}

struct Counter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self.bytes.checked_add(bytes.len());
        if next.is_none_or(|n| n > self.limit) {
            self.exceeded = true;
            return Err(io::Error::new(io::ErrorKind::FileTooLarge, "result size"));
        }
        self.bytes = next.expect("checked above");
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn outcome(bytes: Arc<[u8]>, result_limit: usize) -> Result<CanonicalOutcome, DecodeError> {
    let value: OutcomeValue =
        serde_json::from_slice(&bytes).map_err(|_| DecodeError::InvalidRecord)?;
    if let OutcomeValue::Completed { result } = &value {
        let mut counter = Counter {
            bytes: 0,
            limit: result_limit,
            exceeded: false,
        };
        if serde_json::to_writer(&mut counter, result).is_err() {
            return Err(if counter.exceeded {
                DecodeError::TooLarge
            } else {
                DecodeError::InvalidRecord
            });
        }
    }
    // Canonical validation still has a transient parse/re-encode cost. Keep
    // only the caller's shared bytes after comparing the exact encoding.
    let canonical = CanonicalOutcome::of(value);
    if canonical.bytes() != bytes.as_ref() {
        return Err(DecodeError::InvalidRecord);
    }
    Ok(CanonicalOutcome {
        bytes,
        digest: canonical.digest,
        kind: canonical.kind,
    })
}

fn validate_descriptor_bounds(record: &Record, result_limit: usize) -> Result<(), DecodeError> {
    let result = DomainResult::success("");
    let semantic_length = serde_json::to_vec(&result).expect("typed result").len();
    let wrapped_length = CanonicalOutcome::completed(result).bytes().len();
    let bound = result_limit
        .checked_add(wrapped_length - semantic_length)
        .ok_or(DecodeError::TooLarge)?;
    for descriptor in [record.producer(), record.primary()].into_iter().flatten() {
        if matches!(descriptor.kind(), OutcomeKind::Completed { .. })
            && descriptor.length() > bound as u64
        {
            return Err(DecodeError::TooLarge);
        }
    }
    Ok(())
}

pub fn decode_parts(
    metadata: &[u8],
    producer: Option<Arc<[u8]>>,
    primary_override: Option<Arc<[u8]>>,
    result_limit: usize,
) -> Result<StoredRecord, DecodeError> {
    if metadata.len() > MAX_METADATA_BYTES {
        return Err(DecodeError::TooLarge);
    }
    let encoded: EncodedRecord =
        serde_json::from_slice(metadata).map_err(|_| DecodeError::InvalidRecord)?;
    let record = encoded.validate().map_err(|_| DecodeError::InvalidRecord)?;
    // Compacted Completed bodies are absent: only their closed size bound and
    // metadata links can be proved, not the authenticity of the removed body.
    validate_descriptor_bounds(&record, result_limit)?;
    if record.encode() != metadata {
        return Err(DecodeError::InvalidRecord);
    }
    StoredRecord::new(
        record,
        producer.map(|b| outcome(b, result_limit)).transpose()?,
        primary_override
            .map(|b| outcome(b, result_limit))
            .transpose()?,
    )
    .map_err(|_| DecodeError::InvalidRecord)
}

pub fn validate_result_sizes(
    stored: &StoredRecord,
    result_limit: usize,
) -> Result<(), DecodeError> {
    validate_descriptor_bounds(stored.record(), result_limit)?;
    for body in [stored.producer(), stored.primary_override()]
        .into_iter()
        .flatten()
    {
        outcome(body.bytes.clone(), result_limit)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_metadata_upper_bound_covers_every_field_and_enum_width() {
        // This deliberately invalid all-Some superset tests serialized width,
        // not restoration or execution. Each field uses its maximal JSON width.
        let descriptor = OutcomeDescriptor {
            kind: OutcomeKind::Failed(FailureReason::ResumeUnsupported),
            digest: [255; 32],
            length: u64::MAX,
        };
        let metadata = Record {
            key: Key::new(
                InvocationId::new(),
                TaskId::new(),
                [255; 32],
                Tool::Resolve,
                [255; 32],
                [255; 32],
            ),
            revision: u64::MAX,
            accepted: Some(Accepted {
                owner_hash: [255; 32],
                window: ObservationWindow {
                    received_epoch_ms: u64::MAX,
                    budget_ms: u64::MAX,
                },
            }),
            begun: Some(Begun {
                run: RunIdentity {
                    attempt: AttemptId([255; 32]),
                    generation: u64::MAX,
                },
                started_epoch_ms: u64::MAX,
                read_authority: Some(ReadAuthority::new([255; 32], u64::MAX, u64::MAX)),
            }),
            cancel_requested: false,
            producer: Some(descriptor),
            producer_digest: Some([255; 32]),
            primary_override: Some(descriptor),
            primary_digest: Some([255; 32]),
            acknowledgement: Some(AckRecord {
                digest: [255; 32],
                first_acknowledged_epoch_ms: u64::MAX,
            }),
            public_task: false,
            compacted: false,
        };
        assert!(metadata.validate().is_err());
        assert_eq!(metadata.encode().len(), MAX_METADATA_BYTES);
        assert_eq!(MAX_METADATA_BYTES, 2303);
        for tool in [
            Tool::View,
            Tool::Apply,
            Tool::Resolve,
            Tool::Search,
            Tool::Check,
            Tool::Diff,
            Tool::Run,
            Tool::Docs,
        ] {
            match tool {
                Tool::View
                | Tool::Apply
                | Tool::Resolve
                | Tool::Search
                | Tool::Check
                | Tool::Diff
                | Tool::Run
                | Tool::Docs => (),
            }
            assert!(serde_json::to_vec(&tool).unwrap().len() <= TOOL_BYTES);
        }
        let reasons = [
            FailureReason::InvocationFailed,
            FailureReason::ResultTooLarge,
            FailureReason::Interrupted,
            FailureReason::ResumeUnsupported,
            FailureReason::PersistenceFailed,
            FailureReason::OutcomeUncertain,
            FailureReason::WorkspaceChanged,
        ];
        for reason in reasons {
            match reason {
                FailureReason::InvocationFailed
                | FailureReason::ResultTooLarge
                | FailureReason::Interrupted
                | FailureReason::ResumeUnsupported
                | FailureReason::PersistenceFailed
                | FailureReason::OutcomeUncertain
                | FailureReason::WorkspaceChanged => (),
            }
            assert!(
                serde_json::to_vec(&OutcomeKind::Failed(reason))
                    .unwrap()
                    .len()
                    <= KIND_BYTES
            );
        }
        for kind in [
            OutcomeKind::Completed { ok: false },
            OutcomeKind::Completed { ok: true },
            OutcomeKind::Cancelled,
        ] {
            match kind {
                OutcomeKind::Completed { .. } | OutcomeKind::Failed(_) | OutcomeKind::Cancelled => {
                }
            }
            assert!(serde_json::to_vec(&kind).unwrap().len() <= KIND_BYTES);
        }
    }

    #[test]
    fn oversized_metadata_is_refused_before_deserializing() {
        let invalid = vec![b'!'; MAX_METADATA_BYTES + 1];
        assert_eq!(
            decode_parts(&invalid, None, None, 1),
            Err(DecodeError::TooLarge)
        );
        assert_eq!(
            decode_parts(&invalid[..MAX_METADATA_BYTES], None, None, 1),
            Err(DecodeError::InvalidRecord)
        );
    }

    #[test]
    fn semantic_result_boundary_does_not_count_the_outcome_wrapper() {
        let result = DomainResult::success("exact semantic result");
        let limit = serde_json::to_vec(&result).unwrap().len();
        let canonical = CanonicalOutcome::completed(result);
        assert!(canonical.bytes().len() > limit);
        assert_eq!(outcome(canonical.bytes.clone(), limit).unwrap(), canonical);
        assert_eq!(
            outcome(canonical.bytes.clone(), limit - 1),
            Err(DecodeError::TooLarge)
        );
        let mut noncanonical = canonical.bytes().to_vec();
        noncanonical.push(b' ');
        assert_eq!(
            outcome(noncanonical.into(), limit),
            Err(DecodeError::InvalidRecord)
        );
    }
}

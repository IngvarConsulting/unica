use crate::application::receipt_ledger::ReceiptKeyDigest;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

const RECEIPT_ABSENCE_EVIDENCE_DOMAIN: &[u8] = b"unica.receipt-ledger-test-evidence.v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProductionBoundary {
    V5ReceiptRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProductionProtocolIdentity {
    V5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProductionPostAttemptEvent {
    V5ReceiptRuntimeEntered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProductionMissingTransitionCode {
    ReceiptRowAbsent,
}

/// Proof that execution passed a production-owned boundary.
///
/// Private fields keep the feature facade read-only. Constructor visibility is
/// deliberately confined to infrastructure; the static ownership guard added
/// with the facade must restrict this runtime-entry evidence to runtime-v5.
/// Protocol-v5 may add a distinct boundary constructor when its evidence is
/// wired, but ReceiptLedger itself only returns raw store observations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReachedProductionBoundary {
    boundary: ProductionBoundary,
    current_protocol: ProductionProtocolIdentity,
    event: Option<ProductionPostAttemptEvent>,
    generation_before: u64,
    generation_after: u64,
}

impl ReachedProductionBoundary {
    pub(crate) const fn boundary(&self) -> ProductionBoundary {
        self.boundary
    }

    pub(crate) const fn current_protocol(&self) -> ProductionProtocolIdentity {
        self.current_protocol
    }

    pub(crate) const fn event(&self) -> Option<ProductionPostAttemptEvent> {
        self.event
    }

    pub(crate) const fn generation_before(&self) -> u64 {
        self.generation_before
    }

    pub(crate) const fn generation_after(&self) -> u64 {
        self.generation_after
    }
}

/// Opaque evidence that a production operation reached a real boundary but
/// could not yet perform the next W0a transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProductionMissingTransitionEvidence {
    reached: ReachedProductionBoundary,
    code: ProductionMissingTransitionCode,
    fingerprint: String,
}

impl ProductionMissingTransitionEvidence {
    /// Called by runtime-v5 only after it has entered the production runtime
    /// and received this raw missing-row observation from ReceiptLedger.
    pub(in crate::infrastructure) fn receipt_row_absent(
        receipt_key_digest: &ReceiptKeyDigest,
        generation: u64,
    ) -> Self {
        let fingerprint = receipt_absence_fingerprint(receipt_key_digest, generation);
        Self {
            reached: ReachedProductionBoundary {
                boundary: ProductionBoundary::V5ReceiptRuntime,
                current_protocol: ProductionProtocolIdentity::V5,
                event: Some(ProductionPostAttemptEvent::V5ReceiptRuntimeEntered),
                generation_before: generation,
                generation_after: generation,
            },
            code: ProductionMissingTransitionCode::ReceiptRowAbsent,
            fingerprint,
        }
    }

    pub(crate) const fn reached(&self) -> &ReachedProductionBoundary {
        &self.reached
    }

    pub(crate) const fn code(&self) -> ProductionMissingTransitionCode {
        self.code
    }

    pub(crate) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

fn receipt_absence_fingerprint(receipt_key_digest: &ReceiptKeyDigest, generation: u64) -> String {
    let mut digest = Sha256::new();
    digest.update(RECEIPT_ABSENCE_EVIDENCE_DOMAIN);
    update_framed(&mut digest, b"v5_receipt_runtime");
    update_framed(&mut digest, b"v5");
    update_framed(&mut digest, b"v5_receipt_runtime_entered");
    update_framed(&mut digest, b"receipt_row_absent");
    update_framed(&mut digest, receipt_key_digest.as_str().as_bytes());
    digest.update(generation.to_be_bytes());
    let bytes: [u8; 32] = digest.finalize().into();
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn update_framed(digest: &mut Sha256, bytes: &[u8]) {
    let length = u32::try_from(bytes.len()).expect("bounded evidence component fits in u32");
    digest.update(length.to_be_bytes());
    digest.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn digest(byte: char) -> ReceiptKeyDigest {
        ReceiptKeyDigest::from_str(&byte.to_string().repeat(64)).expect("checked digest")
    }

    #[test]
    fn receipt_absence_evidence_is_opaque_bounded_and_production_derived() {
        let evidence = ProductionMissingTransitionEvidence::receipt_row_absent(&digest('a'), 7);

        assert_eq!(
            evidence.reached().boundary(),
            ProductionBoundary::V5ReceiptRuntime
        );
        assert_eq!(
            evidence.reached().current_protocol(),
            ProductionProtocolIdentity::V5
        );
        assert_eq!(
            evidence.reached().event(),
            Some(ProductionPostAttemptEvent::V5ReceiptRuntimeEntered)
        );
        assert_eq!(evidence.reached().generation_before(), 7);
        assert_eq!(evidence.reached().generation_after(), 7);
        assert_eq!(
            evidence.code(),
            ProductionMissingTransitionCode::ReceiptRowAbsent
        );
        assert_eq!(evidence.fingerprint().len(), 64);
        assert!(evidence
            .fingerprint()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }

    #[test]
    fn receipt_absence_fingerprint_changes_with_production_observation() {
        let baseline = ProductionMissingTransitionEvidence::receipt_row_absent(&digest('a'), 7);
        let other_key = ProductionMissingTransitionEvidence::receipt_row_absent(&digest('b'), 7);
        let other_generation =
            ProductionMissingTransitionEvidence::receipt_row_absent(&digest('a'), 8);

        assert_ne!(baseline.fingerprint(), other_key.fingerprint());
        assert_ne!(baseline.fingerprint(), other_generation.fingerprint());
    }
}

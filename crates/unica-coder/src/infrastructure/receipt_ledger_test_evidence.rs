use crate::infrastructure::daemon::protocol_v5::{DecodedV5Request, StrictV5EnvelopeRejection};
use crate::infrastructure::daemon::runtime_v5::{
    V5ExecutorReachability, V5TaskProjectionReachability,
};
use crate::infrastructure::receipt_ledger::{
    MissingReceiptObservation, StableReceiptLedgerObservation,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

const RECEIPT_ABSENCE_EVIDENCE_DOMAIN: &[u8] = b"unica.receipt-ledger-test-evidence.v1\0";
const PROTOCOL_FRAME_EVIDENCE_DOMAIN: &[u8] = b"unica.protocol-v5-test-evidence.v1\0";
const RECEIPT_OWNER_EVIDENCE_DOMAIN: &[u8] = b"unica.receipt-ledger-owner-test-evidence.v1\0";

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProductionBoundary {
    StrictEnvelopeValidation,
    V5ReceiptRuntime,
    V5Executor,
    TaskProjection,
    ProtocolNegotiation,
    ReceiptTransition,
    CapacityCoordination,
    ReceiptIdentity,
    CrossStoreReconciliation,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProductionProtocolIdentity {
    V5,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProductionPostAttemptEvent {
    V5ReceiptRuntimeEntered,
    V5ExecutorEntered,
    ProtocolFrameRead,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProductionMissingTransitionCode {
    StrictEnvelopeObservationUnavailable,
    ReceiptRowAbsent,
    WriterPathUnavailable,
    TaskProjectionUnavailable,
    ProtocolBehaviorUnavailable,
    ReceiptTransitionUnavailable,
    CapacityLatchUnavailable,
    ReceiptIdentityUnavailable,
    CrossStoreIntentUnavailable,
}

/// Proof that execution passed a production-owned boundary.
///
/// Private fields keep the feature facade read-only. Constructor visibility is
/// deliberately confined to infrastructure; the static ownership guard added
/// with the facade must restrict both constructors to their named production
/// owners. ReceiptLedger itself only returns raw store observations.
struct ReachedProductionBoundary {
    boundary: ProductionBoundary,
    current_protocol: ProductionProtocolIdentity,
    event: Option<ProductionPostAttemptEvent>,
    generation_before: u64,
    generation_after: u64,
}

/// Opaque evidence that a production operation reached a real boundary but
/// could not yet perform the next W0a transition.
pub(crate) struct ProductionMissingTransitionEvidence {
    reached: ReachedProductionBoundary,
    code: ProductionMissingTransitionCode,
    correlation: ActionCorrelation,
    fingerprint: String,
}

enum ActionCorrelation {
    Submit,
    Exact(&'static str),
}

impl ProductionMissingTransitionEvidence {
    /// Called only by the strict protocol-v5 decoder after a malformed frame
    /// has been rejected by either its bounded reader or its closed schema.
    pub(in crate::infrastructure) fn strict_envelope_observation_unavailable(
        token: StrictV5EnvelopeRejection,
    ) -> Self {
        Self {
            reached: ReachedProductionBoundary {
                boundary: ProductionBoundary::StrictEnvelopeValidation,
                current_protocol: ProductionProtocolIdentity::V5,
                event: None,
                generation_before: 0,
                generation_after: 0,
            },
            code: ProductionMissingTransitionCode::StrictEnvelopeObservationUnavailable,
            correlation: ActionCorrelation::Exact("send_outer_envelope"),
            fingerprint: strict_envelope_fingerprint(&token),
        }
    }

    /// Called only by the dedicated protocol-v5 executor after it has retained
    /// the canonical service and observed a stable production receipt store.
    pub(in crate::infrastructure) fn writer_path_unavailable(
        token: V5ExecutorReachability,
    ) -> Self {
        let action_kind = token.action().wire_name();
        let generation_before = token.observation().generation_before();
        let generation_after = token.observation().generation_after();
        Self {
            reached: ReachedProductionBoundary {
                boundary: ProductionBoundary::V5Executor,
                current_protocol: ProductionProtocolIdentity::V5,
                event: Some(ProductionPostAttemptEvent::V5ExecutorEntered),
                generation_before,
                generation_after,
            },
            code: ProductionMissingTransitionCode::WriterPathUnavailable,
            correlation: match token.action() {
                crate::infrastructure::daemon::runtime_v5::V5ExecutorReachabilityAction::SubmitInvocation => {
                    ActionCorrelation::Submit
                }
                _ => ActionCorrelation::Exact(action_kind),
            },
            fingerprint: executor_fingerprint(&token),
        }
    }

    /// Called only by runtime-v5 after the retained v5 TaskStore namespace has
    /// been validated and the not-yet-implemented v5 seed writer is reached.
    pub(in crate::infrastructure) fn task_projection_unavailable(
        token: V5TaskProjectionReachability,
    ) -> Self {
        let generation_before = token.observation().generation_before();
        let generation_after = token.observation().generation_after();
        Self {
            reached: ReachedProductionBoundary {
                boundary: ProductionBoundary::TaskProjection,
                current_protocol: ProductionProtocolIdentity::V5,
                event: None,
                generation_before,
                generation_after,
            },
            code: ProductionMissingTransitionCode::TaskProjectionUnavailable,
            correlation: ActionCorrelation::Exact("seed_task"),
            fingerprint: task_projection_fingerprint(&token),
        }
    }

    /// Called by runtime-v5 only after it has entered the production runtime
    /// and received this raw missing-row observation from ReceiptLedger.
    pub(in crate::infrastructure) fn receipt_row_absent(
        observation: MissingReceiptObservation,
    ) -> Self {
        let fingerprint = receipt_absence_fingerprint(
            observation.receipt_key_digest(),
            observation.generation_before(),
            observation.generation_after(),
        );
        Self {
            reached: ReachedProductionBoundary {
                boundary: ProductionBoundary::V5ReceiptRuntime,
                current_protocol: ProductionProtocolIdentity::V5,
                event: Some(ProductionPostAttemptEvent::V5ReceiptRuntimeEntered),
                generation_before: observation.generation_before(),
                generation_after: observation.generation_after(),
            },
            code: ProductionMissingTransitionCode::ReceiptRowAbsent,
            correlation: ActionCorrelation::Submit,
            fingerprint,
        }
    }

    /// Accepts only a frame token returned by the bounded strict protocol-v5
    /// reader. Runtime-v5 calls this after its real listener has handled that
    /// frame and the next scenario-observation transition is still absent.
    pub(in crate::infrastructure) fn protocol_behavior_unavailable(
        decoded_frame: &DecodedV5Request,
    ) -> Self {
        Self {
            reached: ReachedProductionBoundary {
                boundary: ProductionBoundary::ProtocolNegotiation,
                current_protocol: ProductionProtocolIdentity::V5,
                event: Some(ProductionPostAttemptEvent::ProtocolFrameRead),
                generation_before: 0,
                generation_after: 0,
            },
            code: ProductionMissingTransitionCode::ProtocolBehaviorUnavailable,
            correlation: ActionCorrelation::Exact("probe_protocol"),
            fingerprint: protocol_frame_fingerprint(decoded_frame.raw_frame()),
        }
    }

    /// The crate-root test facade can project evidence only through this closed
    /// envelope. It supplies the scenario index and the same snake-case action
    /// name used by the scenario wire format; the evidence owner validates the
    /// correlation and owns every boundary, protocol, event and code value.
    pub(crate) fn encode_facade_envelope(
        &self,
        action_index: u32,
        action_kind: &str,
    ) -> Result<String, String> {
        self.validate_action_correlation(action_kind)?;
        serde_json::to_string(&FacadeEnvelope::ProductionMissingTransition(
            ProductionMissingTransition {
                action_index,
                action_kind,
                reached_boundary: self.reached.boundary,
                current_protocol: self.reached.current_protocol,
                evidence: MissingEvidence {
                    code: self.code,
                    event: self.reached.event,
                    generation_before: self.reached.generation_before,
                    generation_after: self.reached.generation_after,
                    fingerprint: &self.fingerprint,
                },
            },
        ))
        .map_err(|_| "encode production missing-transition evidence".to_string())
    }

    fn validate_action_correlation(&self, action_kind: &str) -> Result<(), String> {
        let correlated = match self.correlation {
            ActionCorrelation::Submit => matches!(action_kind, "submit" | "spawn_submit"),
            ActionCorrelation::Exact(expected) => action_kind == expected,
        };
        if correlated {
            Ok(())
        } else {
            Err(format!(
                "{} evidence does not correlate with action kind {action_kind}",
                self.code.wire_name()
            ))
        }
    }
}

impl ProductionMissingTransitionCode {
    const fn wire_name(self) -> &'static str {
        match self {
            Self::StrictEnvelopeObservationUnavailable => "strict_envelope_observation_unavailable",
            Self::ReceiptRowAbsent => "receipt_row_absent",
            Self::WriterPathUnavailable => "writer_path_unavailable",
            Self::TaskProjectionUnavailable => "task_projection_unavailable",
            Self::ProtocolBehaviorUnavailable => "protocol_behavior_unavailable",
            Self::ReceiptTransitionUnavailable => "receipt_transition_unavailable",
            Self::CapacityLatchUnavailable => "capacity_latch_unavailable",
            Self::ReceiptIdentityUnavailable => "receipt_identity_unavailable",
            Self::CrossStoreIntentUnavailable => "cross_store_intent_unavailable",
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum FacadeEnvelope<'a> {
    ProductionMissingTransition(ProductionMissingTransition<'a>),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProductionMissingTransition<'a> {
    action_index: u32,
    action_kind: &'a str,
    reached_boundary: ProductionBoundary,
    current_protocol: ProductionProtocolIdentity,
    evidence: MissingEvidence<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MissingEvidence<'a> {
    code: ProductionMissingTransitionCode,
    event: Option<ProductionPostAttemptEvent>,
    generation_before: u64,
    generation_after: u64,
    fingerprint: &'a str,
}

fn receipt_absence_fingerprint(
    receipt_key_digest: &crate::application::receipt_ledger::ReceiptKeyDigest,
    generation_before: u64,
    generation_after: u64,
) -> String {
    let mut digest = Sha256::new();
    digest.update(RECEIPT_ABSENCE_EVIDENCE_DOMAIN);
    update_framed(&mut digest, b"v5_receipt_runtime");
    update_framed(&mut digest, b"v5");
    update_framed(&mut digest, b"v5_receipt_runtime_entered");
    update_framed(&mut digest, b"receipt_row_absent");
    update_framed(&mut digest, receipt_key_digest.as_str().as_bytes());
    digest.update(generation_before.to_be_bytes());
    digest.update(generation_after.to_be_bytes());
    let bytes: [u8; 32] = digest.finalize().into();
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn protocol_frame_fingerprint(raw_frame: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(PROTOCOL_FRAME_EVIDENCE_DOMAIN);
    update_framed(&mut digest, b"protocol_negotiation");
    update_framed(&mut digest, b"v5");
    update_framed(&mut digest, b"protocol_frame_read");
    update_framed(&mut digest, b"protocol_behavior_unavailable");
    update_framed(&mut digest, raw_frame);
    encode_digest(digest.finalize().into())
}

fn strict_envelope_fingerprint(token: &StrictV5EnvelopeRejection) -> String {
    let mut digest = Sha256::new();
    digest.update(PROTOCOL_FRAME_EVIDENCE_DOMAIN);
    update_framed(&mut digest, b"strict_envelope_validation");
    update_framed(&mut digest, b"v5");
    update_framed(&mut digest, b"strict_envelope_observation_unavailable");
    update_framed(&mut digest, token.rejection().wire_name().as_bytes());
    update_framed(&mut digest, token.raw_frame());
    encode_digest(digest.finalize().into())
}

fn executor_fingerprint(token: &V5ExecutorReachability) -> String {
    owner_fingerprint(
        b"v5_executor",
        b"writer_path_unavailable",
        token.action().wire_name(),
        token.observation(),
        &[],
    )
}

fn task_projection_fingerprint(token: &V5TaskProjectionReachability) -> String {
    owner_fingerprint(
        b"task_projection",
        b"task_projection_unavailable",
        "seed_task",
        token.observation(),
        &[],
    )
}

fn owner_fingerprint(
    boundary: &[u8],
    code: &[u8],
    action_kind: &str,
    observation: &StableReceiptLedgerObservation,
    additional_components: &[&[u8]],
) -> String {
    let mut digest = Sha256::new();
    digest.update(RECEIPT_OWNER_EVIDENCE_DOMAIN);
    update_framed(&mut digest, boundary);
    update_framed(&mut digest, b"v5");
    update_framed(&mut digest, code);
    update_framed(&mut digest, action_kind.as_bytes());
    digest.update(observation.generation_before().to_be_bytes());
    digest.update(observation.generation_after().to_be_bytes());
    for component in additional_components {
        update_framed(&mut digest, component);
    }
    encode_digest(digest.finalize().into())
}

fn encode_digest(bytes: [u8; 32]) -> String {
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
    use crate::application::receipt_ledger::ReceiptKeyDigest;
    use proc_macro2::{Spacing, TokenStream, TokenTree};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};
    use std::str::FromStr;
    use syn::visit::Visit;

    fn digest(byte: char) -> ReceiptKeyDigest {
        ReceiptKeyDigest::from_str(&byte.to_string().repeat(64)).expect("checked digest")
    }

    #[test]
    fn receipt_absence_fingerprint_changes_with_production_observation() {
        let baseline = receipt_absence_fingerprint(&digest('a'), 7, 7);
        let other_key = receipt_absence_fingerprint(&digest('b'), 7, 7);
        let other_generation = receipt_absence_fingerprint(&digest('a'), 8, 8);
        let unequal_generation = receipt_absence_fingerprint(&digest('a'), 7, 8);

        assert_ne!(baseline, other_key);
        assert_ne!(baseline, other_generation);
        assert_ne!(baseline, unequal_generation);
    }

    #[test]
    fn protocol_fingerprint_is_bounded_and_uses_the_exact_strict_frame_bytes() {
        let canonical = protocol_frame_fingerprint(b"{\"kind\":\"ping\"}");
        let differently_spaced = protocol_frame_fingerprint(b"{ \"kind\":\"ping\"}");

        assert_eq!(canonical.len(), 64);
        assert!(canonical
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        assert_ne!(canonical, differently_spaced);
    }

    #[test]
    fn constructor_reference_finder_detects_direct_and_aliased_calls() {
        let constructors = ["receipt_row_absent"];
        let direct = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            fn mint(observation: MissingReceiptObservation) {
                ProductionMissingTransitionEvidence::receipt_row_absent(observation);
            }
        "#;
        let aliased = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence as Evidence;

            fn mint(observation: MissingReceiptObservation) {
                Evidence::receipt_row_absent(observation);
            }
        "#;
        let expected = BTreeSet::from(["receipt_row_absent".to_string()]);

        assert_eq!(
            evidence_constructor_references(direct, &constructors).expect("parse direct call"),
            expected
        );
        assert_eq!(
            evidence_constructor_references(aliased, &constructors).expect("parse aliased call"),
            expected
        );
    }

    #[test]
    fn constructor_reference_finder_detects_grouped_use_and_module_qualified_receiver() {
        let constructors = ["receipt_row_absent"];
        let grouped = r#"
            use crate::{
                infrastructure::{
                    receipt_ledger_test_evidence::{
                        ProductionMissingTransitionEvidence as GroupedEvidence,
                    },
                },
            };

            fn mint(observation: MissingReceiptObservation) {
                GroupedEvidence::receipt_row_absent(observation);
            }
        "#;
        let module_qualified = r#"
            fn mint(observation: MissingReceiptObservation) {
                crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence::receipt_row_absent(observation);
            }
        "#;
        let expected = BTreeSet::from(["receipt_row_absent".to_string()]);

        assert_eq!(
            evidence_constructor_references(grouped, &constructors).expect("parse grouped import"),
            expected
        );
        assert_eq!(
            evidence_constructor_references(module_qualified, &constructors)
                .expect("parse module-qualified receiver"),
            expected
        );
    }

    #[test]
    fn constructor_reference_finder_detects_self_receiver() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            impl ProductionMissingTransitionEvidence {
                fn mint(observation: MissingReceiptObservation) {
                    Self::receipt_row_absent(observation);
                }
            }
        "#;

        assert_eq!(
            evidence_constructor_references(source, &["receipt_row_absent"])
                .expect("parse Self receiver"),
            BTreeSet::from(["receipt_row_absent".to_string()])
        );
    }

    #[test]
    fn constructor_reference_finder_detects_local_type_alias_receiver() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            type Evidence = ProductionMissingTransitionEvidence;
            type ChainedEvidence = Evidence;

            fn mint(observation: MissingReceiptObservation) {
                ChainedEvidence::receipt_row_absent(observation);
            }
        "#;

        assert_eq!(
            evidence_constructor_references(source, &["receipt_row_absent"])
                .expect("parse local type alias"),
            BTreeSet::from(["receipt_row_absent".to_string()])
        );
    }

    #[test]
    fn constructor_reference_finder_fails_closed_on_local_canonical_name_shadow() {
        let source = r#"
            fn mint(observation: MissingReceiptObservation) {
                struct ProductionMissingTransitionEvidence;
                impl ProductionMissingTransitionEvidence {
                    fn receipt_row_absent(_: MissingReceiptObservation) {}
                }

                ProductionMissingTransitionEvidence::receipt_row_absent(observation);
            }
        "#;

        let error = evidence_constructor_references(source, &["receipt_row_absent"])
            .expect_err("an unrelated local type cannot inherit sealed evidence authority");
        assert!(error.contains("unsupported sealed evidence constructor receiver"));
        assert!(error.contains("ProductionMissingTransitionEvidence::receipt_row_absent"));
    }

    #[test]
    fn constructor_reference_finder_does_not_inherit_self_authority_into_nested_impl() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            impl ProductionMissingTransitionEvidence {
                fn outer(observation: MissingReceiptObservation) {
                    struct Unrelated;
                    impl Unrelated {
                        fn mint(observation: MissingReceiptObservation) {
                            Self::receipt_row_absent(observation);
                        }
                    }
                    let _ = observation;
                }
            }
        "#;

        let error = evidence_constructor_references(source, &["receipt_row_absent"])
            .expect_err("Self authority belongs only to the current evidence impl receiver");
        assert!(error.contains("unsupported sealed evidence constructor receiver"));
        assert!(error.contains("Self::receipt_row_absent"));
    }

    #[test]
    fn constructor_reference_finder_fails_closed_on_unknown_receiver() {
        let source = r#"
            fn mint(observation: MissingReceiptObservation) {
                HiddenReexport::receipt_row_absent(observation);
            }
        "#;

        let error = evidence_constructor_references(source, &["receipt_row_absent"])
            .expect_err("unknown sealed-constructor receiver must fail closed");
        assert!(error.contains("unsupported sealed evidence constructor receiver"));
        assert!(error.contains("HiddenReexport::receipt_row_absent"));
    }

    #[test]
    fn constructor_reference_finder_detects_macro_rules_body() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            macro_rules! mint {
                ($observation:expr) => {
                    ProductionMissingTransitionEvidence::receipt_row_absent($observation)
                };
            }
        "#;

        assert_eq!(
            evidence_constructor_references(source, &["receipt_row_absent"])
                .expect("parse macro_rules body"),
            BTreeSet::from(["receipt_row_absent".to_string()])
        );
    }

    #[test]
    fn constructor_reference_finder_detects_associated_path_in_macro_invocation() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            fn mint(observation: MissingReceiptObservation) {
                helper!(ProductionMissingTransitionEvidence::receipt_row_absent(observation));
            }
        "#;

        assert_eq!(
            evidence_constructor_references(source, &["receipt_row_absent"])
                .expect("parse macro invocation"),
            BTreeSet::from(["receipt_row_absent".to_string()])
        );
    }

    #[test]
    fn constructor_reference_finder_ignores_macro_metavariable_named_like_constructor() {
        let source = r#"
            macro_rules! bind {
                ($receipt_row_absent:ident) => {};
            }
        "#;

        assert!(
            evidence_constructor_references(source, &["receipt_row_absent"])
                .expect("parse macro metavariable")
                .is_empty()
        );
    }

    #[test]
    fn constructor_reference_finder_fails_closed_on_unknown_macro_receiver() {
        let source = r#"
            fn mint(observation: MissingReceiptObservation) {
                helper!(HiddenReexport::receipt_row_absent(observation));
            }
        "#;

        let error = evidence_constructor_references(source, &["receipt_row_absent"])
            .expect_err("unknown macro receiver must fail closed");
        assert!(error.contains("unsupported sealed evidence constructor receiver"));
        assert!(error.contains("HiddenReexport::receipt_row_absent"));
    }

    #[test]
    fn constructor_reference_finder_fails_closed_on_macro_constructor_metavariable() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            macro_rules! mint {
                ($constructor:ident, $observation:expr) => {
                    ProductionMissingTransitionEvidence::$constructor($observation)
                };
            }

            fn call(observation: MissingReceiptObservation) {
                mint!(receipt_row_absent, observation);
            }
        "#;

        let error = evidence_constructor_references(source, &["receipt_row_absent"])
            .expect_err("a dynamic sealed constructor must fail closed");
        assert!(error.contains("unsupported macro metavariable constructor"));
    }

    #[test]
    fn constructor_reference_finder_fails_closed_on_top_level_self_macro_constructor() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            macro_rules! mint {
                ($constructor:ident, $observation:expr) => {
                    Self::$constructor($observation)
                };
            }

            impl ProductionMissingTransitionEvidence {
                fn call(observation: MissingReceiptObservation) {
                    mint!(receipt_row_absent, observation);
                }
            }
        "#;

        let error = evidence_constructor_references(source, &["receipt_row_absent"])
            .expect_err("top-level Self macro can expand inside the evidence impl");
        assert!(error.contains("unsupported macro metavariable constructor"));
    }

    #[test]
    fn constructor_reference_finder_fails_closed_on_top_level_self_with_evidence_alias() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence as Evidence;

            macro_rules! mint {
                ($constructor:ident, $observation:expr) => {
                    Self::$constructor($observation)
                };
            }

            impl Evidence {
                fn call(observation: MissingReceiptObservation) {
                    mint!(receipt_row_absent, observation);
                }
            }
        "#;

        let error = evidence_constructor_references(source, &["receipt_row_absent"])
            .expect_err("top-level Self macro sees every canonical evidence alias");
        assert!(error.contains("unsupported macro metavariable constructor"));
    }

    #[test]
    fn constructor_reference_finder_rejects_outer_trait_generic_shadow() {
        let source = r#"
            use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

            trait Shadow<ProductionMissingTransitionEvidence> {
                fn mint(observation: MissingReceiptObservation) {
                    ProductionMissingTransitionEvidence::receipt_row_absent(observation);
                }
            }
        "#;

        let error = evidence_constructor_references(source, &["receipt_row_absent"])
            .expect_err("outer trait generics shadow imported evidence bindings");
        assert!(error.contains("unsupported sealed evidence constructor receiver"));
    }

    #[test]
    fn constructor_reference_finder_rejects_fake_canonical_module_suffix() {
        for source in [
            r#"
                mod receipt_ledger_test_evidence {
                    pub struct ProductionMissingTransitionEvidence;
                    impl ProductionMissingTransitionEvidence {
                        pub fn receipt_row_absent(_: MissingReceiptObservation) {}
                    }
                }

                fn mint(observation: MissingReceiptObservation) {
                    receipt_ledger_test_evidence::ProductionMissingTransitionEvidence::receipt_row_absent(observation);
                }
            "#,
            r#"
                mod receipt_ledger_test_evidence {
                    pub struct ProductionMissingTransitionEvidence;
                    impl ProductionMissingTransitionEvidence {
                        pub fn receipt_row_absent(_: MissingReceiptObservation) {}
                    }
                }
                use receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;

                fn mint(observation: MissingReceiptObservation) {
                    ProductionMissingTransitionEvidence::receipt_row_absent(observation);
                }
            "#,
        ] {
            let error = evidence_constructor_references(source, &["receipt_row_absent"])
                .expect_err("a local suffix cannot impersonate the canonical evidence module");
            assert!(error.contains("unsupported sealed evidence constructor receiver"));
        }
    }

    #[test]
    fn constructor_reference_finder_ignores_constructor_names_inside_macro_literals() {
        for source in [
            r#"fn check() { helper!("receipt_row_absent"); }"#,
            r#"fn check() { helper!(b"receipt_row_absent"); }"#,
            r#"fn check() { helper!(c"receipt_row_absent"); }"#,
            r##"fn check() { helper!(r#"receipt_row_absent"#); }"##,
            r##"fn check() { helper!(br#"receipt_row_absent"#); }"##,
            r##"fn check() { helper!(cr#"receipt_row_absent"#); }"##,
        ] {
            assert!(
                evidence_constructor_references(source, &["receipt_row_absent"])
                    .expect("parse macro literal")
                    .is_empty(),
                "literal text cannot claim evidence ownership: {source}"
            );
        }
    }

    #[test]
    fn repository_source_scan_reports_normalized_relative_parse_error() {
        let manifest = PathBuf::from("/workspace/crates/unica-coder");
        let source_path = manifest.join("src").join("broken.rs");
        let error = evidence_constructor_references_for_source(
            &manifest,
            &source_path,
            "fn broken( {",
            &["receipt_row_absent"],
        )
        .expect_err("malformed Rust must fail the repository guard");

        assert!(error.starts_with("parse src/broken.rs:"), "{error}");
        assert!(!error.contains("/workspace/crates/unica-coder"), "{error}");
    }

    #[test]
    fn sealed_missing_evidence_constructors_stay_with_their_production_owners() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut sources = Vec::new();
        collect_rust_sources(&manifest.join("src"), &mut sources);
        let ownership = [
            (
                "strict_envelope_observation_unavailable",
                Some("src/infrastructure/daemon/protocol_v5.rs"),
            ),
            ("receipt_row_absent", None),
            (
                "protocol_behavior_unavailable",
                Some("src/infrastructure/daemon/runtime_v5.rs"),
            ),
            (
                "writer_path_unavailable",
                Some("src/infrastructure/daemon/runtime_v5.rs"),
            ),
            (
                "task_projection_unavailable",
                Some("src/infrastructure/daemon/runtime_v5.rs"),
            ),
            ("receipt_transition_unavailable", None),
            ("capacity_latch_unavailable", None),
            ("receipt_identity_unavailable", None),
            ("cross_store_intent_unavailable", None),
        ];

        let constructors = ownership
            .iter()
            .map(|(constructor, _)| *constructor)
            .collect::<Vec<_>>();
        let mut owners_by_constructor = BTreeMap::<String, BTreeSet<String>>::new();
        for path in &sources {
            let source = std::fs::read_to_string(path).expect("read Rust source");
            let (owner, references) =
                evidence_constructor_references_for_source(&manifest, path, &source, &constructors)
                    .unwrap_or_else(|error| panic!("{error}"));
            for constructor in references {
                owners_by_constructor
                    .entry(constructor)
                    .or_default()
                    .insert(owner.clone());
            }
        }

        for (constructor, expected_owner) in ownership {
            let owners = owners_by_constructor
                .remove(constructor)
                .unwrap_or_default();
            let expected = expected_owner
                .map(|owner| BTreeSet::from([owner.to_string()]))
                .unwrap_or_default();
            assert_eq!(owners, expected, "{constructor} has no honest owner yet");
        }
    }

    fn collect_rust_sources(root: &Path, sources: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(root).expect("read source directory") {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                collect_rust_sources(&path, sources);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }

    fn evidence_constructor_references(
        source: &str,
        constructors: &[&str],
    ) -> Result<BTreeSet<String>, String> {
        let syntax = syn::parse_file(source).map_err(|error| error.to_string())?;
        let mut finder = EvidenceConstructorReferenceFinder {
            constructors: constructors.iter().copied().collect(),
            references: BTreeSet::new(),
            unsupported_references: BTreeSet::new(),
            type_scopes: Vec::new(),
            impl_receivers: Vec::new(),
        };
        finder.visit_file(&syntax);
        if finder.unsupported_references.is_empty() {
            Ok(finder.references)
        } else {
            Err(finder
                .unsupported_references
                .into_iter()
                .collect::<Vec<_>>()
                .join("; "))
        }
    }

    fn evidence_constructor_references_for_source(
        manifest: &Path,
        path: &Path,
        source: &str,
        constructors: &[&str],
    ) -> Result<(String, BTreeSet<String>), String> {
        let owner = path
            .strip_prefix(manifest)
            .map_err(|_| "receipt evidence source escapes the crate root".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let references = evidence_constructor_references(source, constructors)
            .map_err(|error| format!("parse {owner}: {error}"))?;
        Ok((owner, references))
    }

    struct EvidenceConstructorReferenceFinder<'a> {
        constructors: BTreeSet<&'a str>,
        references: BTreeSet<String>,
        unsupported_references: BTreeSet<String>,
        type_scopes: Vec<BTreeMap<String, EvidenceTypeBinding>>,
        impl_receivers: Vec<Option<EvidenceTypeBinding>>,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum EvidenceTypeBinding {
        Evidence,
        NonEvidence,
        Ambiguous,
    }

    enum MacroLexeme {
        Identifier { name: String, metavariable: bool },
        PathSeparator,
        Barrier,
    }

    impl EvidenceConstructorReferenceFinder<'_> {
        const EVIDENCE_TYPE: &'static str = "ProductionMissingTransitionEvidence";
        const EVIDENCE_MODULE: &'static str = "receipt_ledger_test_evidence";

        fn segments_are_canonical_evidence(segments: &[String]) -> bool {
            segments
                == [
                    "crate",
                    "infrastructure",
                    Self::EVIDENCE_MODULE,
                    Self::EVIDENCE_TYPE,
                ]
        }

        fn merge_binding(
            bindings: &mut BTreeMap<String, EvidenceTypeBinding>,
            name: String,
            binding: EvidenceTypeBinding,
        ) {
            bindings
                .entry(name)
                .and_modify(|existing| {
                    if *existing != binding {
                        *existing = EvidenceTypeBinding::Ambiguous;
                    }
                })
                .or_insert(binding);
        }

        fn path_is_canonical_evidence(path: &syn::Path) -> bool {
            let segments = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            Self::segments_are_canonical_evidence(&segments)
        }

        fn resolve_name(&self, name: &str) -> Option<EvidenceTypeBinding> {
            self.type_scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(name).copied())
        }

        fn has_visible_evidence_binding(&self) -> bool {
            self.type_scopes.iter().rev().any(|scope| {
                scope
                    .values()
                    .any(|binding| *binding == EvidenceTypeBinding::Evidence)
            })
        }

        fn resolve_name_with_local(
            &self,
            name: &str,
            local: &BTreeMap<String, EvidenceTypeBinding>,
        ) -> Option<EvidenceTypeBinding> {
            local.get(name).copied().or_else(|| self.resolve_name(name))
        }

        fn binding_for_path_with_local(
            &self,
            path: &syn::Path,
            local: &BTreeMap<String, EvidenceTypeBinding>,
        ) -> Option<EvidenceTypeBinding> {
            if Self::path_is_canonical_evidence(path) {
                return Some(EvidenceTypeBinding::Evidence);
            }
            if path.segments.len() == 1 {
                return path.segments.first().and_then(|segment| {
                    self.resolve_name_with_local(segment.ident.to_string().as_str(), local)
                });
            }
            None
        }

        fn binding_for_type_with_local(
            &self,
            ty: &syn::Type,
            local: &BTreeMap<String, EvidenceTypeBinding>,
        ) -> Option<EvidenceTypeBinding> {
            match ty {
                syn::Type::Path(path) if path.qself.is_none() => {
                    self.binding_for_path_with_local(&path.path, local)
                }
                syn::Type::Group(group) => self.binding_for_type_with_local(&group.elem, local),
                syn::Type::Paren(paren) => self.binding_for_type_with_local(&paren.elem, local),
                _ => Some(EvidenceTypeBinding::NonEvidence),
            }
        }

        fn binding_for_type(&self, ty: &syn::Type) -> Option<EvidenceTypeBinding> {
            self.binding_for_type_with_local(ty, &BTreeMap::new())
        }

        fn collect_use_bindings(
            tree: &syn::UseTree,
            prefix: &mut Vec<String>,
            bindings: &mut BTreeMap<String, EvidenceTypeBinding>,
        ) {
            match tree {
                syn::UseTree::Path(path) => {
                    prefix.push(path.ident.to_string());
                    Self::collect_use_bindings(&path.tree, prefix, bindings);
                    prefix.pop();
                }
                syn::UseTree::Name(name) => {
                    let mut imported = prefix.clone();
                    imported.push(name.ident.to_string());
                    let binding = if Self::segments_are_canonical_evidence(&imported) {
                        EvidenceTypeBinding::Evidence
                    } else {
                        EvidenceTypeBinding::NonEvidence
                    };
                    Self::merge_binding(bindings, name.ident.to_string(), binding);
                }
                syn::UseTree::Rename(rename) => {
                    let mut imported = prefix.clone();
                    imported.push(rename.ident.to_string());
                    let binding = if Self::segments_are_canonical_evidence(&imported) {
                        EvidenceTypeBinding::Evidence
                    } else {
                        EvidenceTypeBinding::NonEvidence
                    };
                    Self::merge_binding(bindings, rename.rename.to_string(), binding);
                }
                syn::UseTree::Group(group) => {
                    for tree in &group.items {
                        Self::collect_use_bindings(tree, prefix, bindings);
                    }
                }
                syn::UseTree::Glob(_) => {}
            }
        }

        fn bindings_for_items<'ast>(
            &self,
            items: impl IntoIterator<Item = &'ast syn::Item>,
        ) -> BTreeMap<String, EvidenceTypeBinding> {
            let items = items.into_iter().collect::<Vec<_>>();
            let mut bindings = BTreeMap::new();
            let mut aliases = Vec::new();

            for item in &items {
                let non_evidence_name = match item {
                    syn::Item::Enum(item) => Some(item.ident.to_string()),
                    syn::Item::ExternCrate(item) => Some(
                        item.rename
                            .as_ref()
                            .map_or_else(|| item.ident.to_string(), |(_, name)| name.to_string()),
                    ),
                    syn::Item::Mod(item) => Some(item.ident.to_string()),
                    syn::Item::Struct(item) => Some(item.ident.to_string()),
                    syn::Item::Trait(item) => Some(item.ident.to_string()),
                    syn::Item::TraitAlias(item) => Some(item.ident.to_string()),
                    syn::Item::Union(item) => Some(item.ident.to_string()),
                    _ => None,
                };
                if let Some(name) = non_evidence_name {
                    Self::merge_binding(&mut bindings, name, EvidenceTypeBinding::NonEvidence);
                }
                match item {
                    syn::Item::Use(item) => {
                        Self::collect_use_bindings(&item.tree, &mut Vec::new(), &mut bindings);
                    }
                    syn::Item::Type(item) => {
                        aliases.push((item.ident.to_string(), item.ty.as_ref()));
                    }
                    _ => {}
                }
            }

            let mut unresolved = aliases;
            loop {
                let mut progress = false;
                unresolved.retain(|(alias, target)| {
                    let Some(binding) = self.binding_for_type_with_local(target, &bindings) else {
                        return true;
                    };
                    Self::merge_binding(&mut bindings, alias.clone(), binding);
                    progress = true;
                    false
                });
                if !progress {
                    break;
                }
            }
            for (alias, _) in unresolved {
                Self::merge_binding(&mut bindings, alias, EvidenceTypeBinding::NonEvidence);
            }
            bindings
        }

        fn bindings_for_generics(
            generics: &syn::Generics,
        ) -> BTreeMap<String, EvidenceTypeBinding> {
            generics
                .type_params()
                .map(|parameter| {
                    (
                        parameter.ident.to_string(),
                        EvidenceTypeBinding::NonEvidence,
                    )
                })
                .collect()
        }

        fn record_path(&mut self, path: &syn::Path) {
            let Some(constructor) = path.segments.last() else {
                return;
            };
            let constructor = constructor.ident.to_string();
            if !self.constructors.contains(constructor.as_str()) {
                return;
            }

            let receiver_segments = path
                .segments
                .iter()
                .take(path.segments.len().saturating_sub(1))
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            let receiver = receiver_segments.last().map(String::as_str);
            let binding = if Self::segments_are_canonical_evidence(&receiver_segments) {
                Some(EvidenceTypeBinding::Evidence)
            } else if receiver_segments.len() == 1 {
                self.binding_for_receiver(receiver)
            } else {
                None
            };
            let rendered = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            self.record_binding(binding, false, constructor.as_str(), &rendered);
        }

        fn binding_for_receiver(&self, receiver: Option<&str>) -> Option<EvidenceTypeBinding> {
            match receiver {
                Some("Self") => self.impl_receivers.last().copied().flatten(),
                Some(receiver) => self.resolve_name(receiver),
                None => None,
            }
        }

        fn record_binding(
            &mut self,
            binding: Option<EvidenceTypeBinding>,
            receiver_is_metavariable: bool,
            constructor: &str,
            rendered: &str,
        ) {
            let supported =
                !receiver_is_metavariable && binding == Some(EvidenceTypeBinding::Evidence);
            if supported {
                self.references.insert(constructor.to_string());
            } else {
                self.unsupported_references.insert(format!(
                    "unsupported sealed evidence constructor receiver in `{rendered}`"
                ));
            }
        }

        fn scan_macro_token_stream(&mut self, tokens: TokenStream) {
            let tokens = tokens.into_iter().collect::<Vec<_>>();
            let mut lexemes = Vec::new();
            let mut next_identifier_is_metavariable = false;
            let mut index = 0;
            while index < tokens.len() {
                match &tokens[index] {
                    TokenTree::Group(group) => {
                        self.scan_macro_token_stream(group.stream());
                        lexemes.push(MacroLexeme::Barrier);
                        next_identifier_is_metavariable = false;
                        index += 1;
                    }
                    TokenTree::Ident(identifier) => {
                        lexemes.push(MacroLexeme::Identifier {
                            name: identifier.to_string(),
                            metavariable: next_identifier_is_metavariable,
                        });
                        next_identifier_is_metavariable = false;
                        index += 1;
                    }
                    TokenTree::Punct(punctuation)
                        if punctuation.as_char() == ':'
                            && punctuation.spacing() == Spacing::Joint
                            && tokens.get(index + 1).is_some_and(|next| {
                                matches!(next, TokenTree::Punct(next) if next.as_char() == ':')
                            }) =>
                    {
                        lexemes.push(MacroLexeme::PathSeparator);
                        next_identifier_is_metavariable = false;
                        index += 2;
                    }
                    TokenTree::Punct(punctuation) if punctuation.as_char() == '$' => {
                        if let (
                            Some(MacroLexeme::Identifier {
                                name: receiver,
                                metavariable: receiver_is_metavariable,
                            }),
                            Some(MacroLexeme::PathSeparator),
                            Some(TokenTree::Ident(constructor)),
                        ) = (
                            lexemes.iter().rev().nth(1),
                            lexemes.last(),
                            tokens.get(index + 1),
                        ) {
                            let receiver_binding = self.binding_for_receiver(Some(receiver));
                            let top_level_self = receiver == "Self"
                                && self.impl_receivers.last().copied().flatten().is_none()
                                && !self.has_visible_evidence_binding();
                            if !top_level_self
                                && (*receiver_is_metavariable
                                    || receiver_binding
                                        != Some(EvidenceTypeBinding::NonEvidence))
                            {
                                let rendered_receiver = if *receiver_is_metavariable {
                                    format!("${receiver}")
                                } else {
                                    receiver.clone()
                                };
                                self.unsupported_references.insert(format!(
                                    "unsupported macro metavariable constructor in `{rendered_receiver}::${constructor}`"
                                ));
                            }
                        }
                        lexemes.push(MacroLexeme::Barrier);
                        next_identifier_is_metavariable = true;
                        index += 1;
                    }
                    TokenTree::Punct(_) | TokenTree::Literal(_) => {
                        lexemes.push(MacroLexeme::Barrier);
                        next_identifier_is_metavariable = false;
                        index += 1;
                    }
                }
            }

            for window in lexemes.windows(3) {
                let [MacroLexeme::Identifier {
                    name: receiver,
                    metavariable: receiver_is_metavariable,
                }, MacroLexeme::PathSeparator, MacroLexeme::Identifier {
                    name: constructor,
                    metavariable: constructor_is_metavariable,
                }] = window
                else {
                    continue;
                };

                if *constructor_is_metavariable {
                    let rendered_receiver = if *receiver_is_metavariable {
                        format!("${receiver}")
                    } else {
                        receiver.clone()
                    };
                    self.unsupported_references.insert(format!(
                        "unsupported macro metavariable constructor in `{rendered_receiver}::${constructor}`"
                    ));
                    continue;
                }
                if !self.constructors.contains(constructor.as_str()) {
                    continue;
                }
                let rendered_receiver = if *receiver_is_metavariable {
                    format!("${receiver}")
                } else {
                    receiver.clone()
                };
                let rendered = format!("{rendered_receiver}::{constructor}");
                let binding = self.binding_for_receiver(Some(receiver));
                self.record_binding(binding, *receiver_is_metavariable, constructor, &rendered);
            }
        }
    }

    impl<'ast> Visit<'ast> for EvidenceConstructorReferenceFinder<'_> {
        fn visit_file(&mut self, file: &'ast syn::File) {
            let bindings = self.bindings_for_items(&file.items);
            self.type_scopes.push(bindings);
            for item in &file.items {
                self.visit_item(item);
            }
            self.type_scopes.pop();
        }

        fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
            let Some((_, items)) = &item.content else {
                syn::visit::visit_item_mod(self, item);
                return;
            };
            let bindings = self.bindings_for_items(items);
            self.type_scopes.push(bindings);
            for item in items {
                self.visit_item(item);
            }
            self.type_scopes.pop();
        }

        fn visit_block(&mut self, block: &'ast syn::Block) {
            let bindings = self.bindings_for_items(block.stmts.iter().filter_map(|statement| {
                if let syn::Stmt::Item(item) = statement {
                    Some(item)
                } else {
                    None
                }
            }));
            self.type_scopes.push(bindings);
            syn::visit::visit_block(self, block);
            self.type_scopes.pop();
        }

        fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
            self.record_path(&expression.path);
            syn::visit::visit_expr_path(self, expression);
        }

        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            self.type_scopes
                .push(Self::bindings_for_generics(&item.generics));
            let receiver = self.binding_for_type(&item.self_ty);
            self.impl_receivers.push(receiver);
            syn::visit::visit_item_impl(self, item);
            self.impl_receivers.pop();
            self.type_scopes.pop();
        }

        fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
            self.type_scopes
                .push(Self::bindings_for_generics(&item.generics));
            syn::visit::visit_item_trait(self, item);
            self.type_scopes.pop();
        }

        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            self.type_scopes
                .push(Self::bindings_for_generics(&item.sig.generics));
            self.impl_receivers.push(None);
            syn::visit::visit_item_fn(self, item);
            self.impl_receivers.pop();
            self.type_scopes.pop();
        }

        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            self.type_scopes
                .push(Self::bindings_for_generics(&item.sig.generics));
            syn::visit::visit_impl_item_fn(self, item);
            self.type_scopes.pop();
        }

        fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
            self.type_scopes
                .push(Self::bindings_for_generics(&item.sig.generics));
            self.impl_receivers.push(None);
            syn::visit::visit_trait_item_fn(self, item);
            self.impl_receivers.pop();
            self.type_scopes.pop();
        }

        fn visit_macro(&mut self, macro_: &'ast syn::Macro) {
            self.scan_macro_token_stream(macro_.tokens.clone());
            syn::visit::visit_macro(self, macro_);
        }
    }
}

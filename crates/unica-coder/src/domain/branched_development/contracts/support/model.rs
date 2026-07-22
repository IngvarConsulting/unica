#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::{MetadataObjectId, SupportLayerId};
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const OBJECT_A: &str = "00000000-0000-0000-0000-000000000001";
    const OBJECT_B: &str = "00000000-0000-0000-0000-000000000002";

    fn accepts<T: DeserializeOwned>(value: Value) -> T {
        serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("contract rejected {value}: {error}"))
    }

    fn rejects<T: DeserializeOwned>(value: Value) {
        assert!(
            serde_json::from_value::<T>(value.clone()).is_err(),
            "contract accepted {value}"
        );
    }

    fn assert_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).expect("support model schema must be recursively closed");
    }

    fn transition_values() -> Vec<Value> {
        vec![
            json!({
                "transitionKind": "enableConfigurationChanges",
                "configurationDisplay": "Configuration",
                "layerId": "layer-a",
                "fromEnabled": false,
                "toEnabled": true
            }),
            json!({
                "transitionKind": "restoreConfigurationChangesDisabled",
                "configurationDisplay": "Configuration",
                "layerId": "layer-b",
                "fromEnabled": true,
                "toEnabled": false
            }),
            json!({
                "transitionKind": "makeObjectEditable",
                "objectId": OBJECT_A,
                "objectDisplay": "Catalog.A",
                "layerId": "layer-c",
                "fromState": "locked",
                "toState": "editable"
            }),
            json!({
                "transitionKind": "restoreObjectLocked",
                "objectId": OBJECT_B,
                "objectDisplay": "Catalog.B",
                "layerId": "layer-d",
                "fromState": "editable",
                "toState": "locked"
            }),
        ]
    }

    fn candidate_evidence(
        object_id: &str,
        layer_id: Option<&str>,
        reasons: &[SupportCandidateReason],
    ) -> SupportCandidateEvidenceAuthority {
        let has = |reason| reasons.contains(&reason).then(digest);
        SupportCandidateEvidenceAuthority::from_capability_adapter(
            MetadataObjectId::parse(object_id).unwrap(),
            layer_id.map(|value| SupportLayerId::parse(value).unwrap()),
            has(SupportCandidateReason::PlatformComparison),
            has(SupportCandidateReason::CanonicalDelta),
            has(SupportCandidateReason::Ownership),
            has(SupportCandidateReason::AddDelete),
            has(SupportCandidateReason::ReferenceClosure),
        )
        .unwrap()
    }

    fn digest() -> Sha256Digest {
        Sha256Digest::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap()
    }

    fn promote_candidate(
        value: Value,
        evidence: &SupportCandidateEvidenceAuthority,
    ) -> Result<SupportCandidate, SupportContractError> {
        let wire = serde_json::from_value::<UnvalidatedSupportCandidate>(value)
            .map_err(|_| SupportContractError("candidate wire decode failed"))?;
        SupportCandidate::from_wire(wire, evidence)
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: ?Sized + DeserializeOwned>
                    AmbiguousIfDeserialize<ImplementsDeserialize> for T
                {
                }
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    assert_not_deserialize_owned!(SupportCandidate);
    assert_not_deserialize_owned!(SupportCandidates);
    assert_not_deserialize_owned!(SupportCandidateSet);
    assert_not_deserialize_owned!(ManualWorkingInfobaseBaseline);

    #[test]
    fn support_transition_has_exact_four_discriminated_leaves() {
        for value in transition_values() {
            accepts::<SupportTransition>(value);
        }

        let mut missing_discriminator = transition_values()[0].clone();
        missing_discriminator
            .as_object_mut()
            .unwrap()
            .remove("transitionKind");
        rejects::<SupportTransition>(missing_discriminator);

        let mut wrong_literal = transition_values()[0].clone();
        wrong_literal["fromEnabled"] = json!(true);
        rejects::<SupportTransition>(wrong_literal);

        let mut cross_leaf = transition_values()[2].clone();
        cross_leaf["configurationDisplay"] = json!("Configuration");
        rejects::<SupportTransition>(cross_leaf);

        let schema = serde_json::to_value(schema_for!(SupportTransition)).unwrap();
        assert_eq!(schema["oneOf"].as_array().map(Vec::len), Some(4));
        assert_closed::<SupportTransition>();
    }

    #[test]
    fn transition_collection_rejects_reordering_duplicates_and_inverse_pairs() {
        accepts::<SupportTransitions>(json!(transition_values()));

        let mut reversed = transition_values();
        reversed.swap(0, 1);
        rejects::<SupportTransitions>(json!(reversed));

        let duplicate = vec![
            transition_values()[0].clone(),
            transition_values()[0].clone(),
        ];
        rejects::<SupportTransitions>(json!(duplicate));

        let inverse = vec![
            json!({
                "transitionKind": "enableConfigurationChanges",
                "configurationDisplay": "Configuration A",
                "layerId": "layer-a",
                "fromEnabled": false,
                "toEnabled": true
            }),
            json!({
                "transitionKind": "restoreConfigurationChangesDisabled",
                "configurationDisplay": "Different presentation",
                "layerId": "layer-a",
                "fromEnabled": true,
                "toEnabled": false
            }),
        ];
        rejects::<SupportTransitions>(json!(inverse));
    }

    #[test]
    fn support_candidate_vocabularies_and_mode_rules_are_fail_closed() {
        for reason in [
            "platformComparison",
            "canonicalDelta",
            "ownership",
            "addDelete",
            "referenceClosure",
        ] {
            accepts::<SupportCandidateReason>(json!(reason));
        }
        rejects::<SupportCandidateReason>(json!("diagnostic"));

        let ordinary = json!({
            "objectId": OBJECT_A,
            "objectDisplay": "Catalog.A",
            "repositoryAction": "modify",
            "currentState": "notApplicable",
            "vendorRestriction": "notApplicable",
            "requiredState": "notApplicable",
            "reasons": ["platformComparison", "canonicalDelta"]
        });
        let ordinary_evidence = candidate_evidence(
            OBJECT_A,
            None,
            &[
                SupportCandidateReason::PlatformComparison,
                SupportCandidateReason::CanonicalDelta,
            ],
        );
        let ordinary_candidate = promote_candidate(ordinary.clone(), &ordinary_evidence).unwrap();

        let incomplete_evidence = candidate_evidence(
            OBJECT_A,
            None,
            &[SupportCandidateReason::PlatformComparison],
        );
        assert!(promote_candidate(ordinary.clone(), &incomplete_evidence).is_err());

        let mut illegal_layer = ordinary.clone();
        illegal_layer["layerId"] = json!("layer-a");
        assert!(promote_candidate(illegal_layer, &ordinary_evidence).is_err());

        let mut missing_layer = ordinary.clone();
        missing_layer["currentState"] = json!("locked");
        assert!(promote_candidate(missing_layer, &ordinary_evidence).is_err());

        let mut wrong_preserve = ordinary.clone();
        wrong_preserve["currentState"] = json!("editable");
        wrong_preserve["requiredState"] = json!("preserveOffSupport");
        wrong_preserve["vendorRestriction"] = json!("changesAllowed");
        wrong_preserve["layerId"] = json!("layer-a");
        assert!(promote_candidate(wrong_preserve, &ordinary_evidence).is_err());

        let mut reordered_reasons = ordinary.clone();
        reordered_reasons["reasons"] = json!(["canonicalDelta", "platformComparison"]);
        assert!(promote_candidate(reordered_reasons, &ordinary_evidence).is_err());

        let second = json!({
            "objectId": OBJECT_B,
            "objectDisplay": "Catalog.B",
            "repositoryAction": "add",
            "currentState": "notApplicable",
            "vendorRestriction": "notApplicable",
            "requiredState": "notApplicable",
            "reasons": ["addDelete"]
        });
        let second_evidence =
            candidate_evidence(OBJECT_B, None, &[SupportCandidateReason::AddDelete]);
        let second_candidate = promote_candidate(second.clone(), &second_evidence).unwrap();
        SupportCandidates::new(vec![ordinary_candidate.clone(), second_candidate.clone()]).unwrap();

        assert!(
            SupportCandidates::new(vec![second_candidate, ordinary_candidate.clone()]).is_err()
        );
        assert!(
            SupportCandidates::new(vec![ordinary_candidate.clone(), ordinary_candidate]).is_err()
        );
        assert_closed::<SupportCandidate>();
    }

    #[test]
    fn blocker_and_missing_kind_collections_use_semantic_declaration_order() {
        let blockers = json!([
            {
                "objectId": OBJECT_A,
                "objectDisplay": "Catalog.A",
                "reason": "configurationChangesDisabled",
                "diagnostic": "redacted"
            },
            {
                "objectId": OBJECT_A,
                "objectDisplay": "Catalog.A",
                "layerId": "layer-a",
                "reason": "objectLocked",
                "diagnostic": "redacted"
            }
        ]);
        accepts::<SupportBlockers>(blockers.clone());

        let mut reversed = blockers.as_array().unwrap().clone();
        reversed.reverse();
        rejects::<SupportBlockers>(json!(reversed));

        let duplicate = json!([
            blockers[0].clone(),
            {
                "objectId": OBJECT_A,
                "objectDisplay": "Different presentation",
                "reason": "configurationChangesDisabled",
                "diagnostic": "other"
            }
        ]);
        rejects::<SupportBlockers>(duplicate);

        accepts::<SupportMissingEvidenceKinds>(json!([
            "candidateClassificationUnavailable",
            "supportGraphIncomplete",
            "manualLeaseBusy"
        ]));
        rejects::<SupportMissingEvidenceKinds>(json!([
            "manualLeaseBusy",
            "supportGraphIncomplete"
        ]));
        rejects::<SupportMissingEvidenceKinds>(json!([
            "supportGraphIncomplete",
            "supportGraphIncomplete"
        ]));
    }

    #[test]
    fn evidence_gap_leaves_preserve_null_presence_and_conditional_fields() {
        accepts::<SupportEvidenceGap>(json!({
            "gapKind": "candidateEvidence",
            "objectId": OBJECT_A,
            "objectDisplay": "Catalog.A",
            "missingEvidenceKind": "candidateClassificationUnavailable",
            "diagnostic": "redacted"
        }));
        accepts::<SupportEvidenceGap>(json!({
            "gapKind": "prerequisiteVersionEvidence",
            "supportActionId": "00000000-0000-0000-0000-000000000010",
            "repositoryVersion": null,
            "missingEvidenceKind": "repositoryActorUnavailable",
            "diagnostic": "redacted"
        }));
        rejects::<SupportEvidenceGap>(json!({
            "gapKind": "prerequisiteVersionEvidence",
            "supportActionId": "00000000-0000-0000-0000-000000000010",
            "missingEvidenceKind": "repositoryActorUnavailable",
            "diagnostic": "redacted"
        }));
        rejects::<SupportEvidenceGap>(json!({
            "gapKind": "candidateEvidence",
            "objectId": OBJECT_A,
            "objectDisplay": "Catalog.A",
            "missingEvidenceKind": "manualLeaseBusy",
            "diagnostic": "redacted"
        }));
        rejects::<SupportEvidenceGap>(json!({
            "gapKind": "repositoryHistoryEvidence",
            "fromCursor": {
                "throughVersion": "v1",
                "historyPrefixDigest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "firstObservedVersion": "v2",
            "missingEvidenceKind": "repositoryHistoryCoverageIncomplete",
            "diagnostic": "redacted"
        }));
        assert_closed::<SupportEvidenceGap>();
    }

    #[test]
    fn manual_target_identity_and_baseline_reject_digest_and_mode_splices() {
        let identity = ManualWorkingInfobaseIdentity::new(
            RepositoryIdentityComponent::parse("HOST").unwrap(),
            RepositoryIdentityComponent::parse("Working IB").unwrap(),
        )
        .unwrap();
        let identity_value = serde_json::to_value(&identity).unwrap();
        accepts::<ManualWorkingInfobaseIdentity>(identity_value.clone());

        let mut wrong_digest = identity_value.clone();
        wrong_digest["digest"] =
            json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        rejects::<ManualWorkingInfobaseIdentity>(wrong_digest);

        for mode in ["reservedOriginal", "separateWorkingInfobase"] {
            accepts::<ManualSupportTargetMode>(json!(mode));
        }
        rejects::<ManualSupportTargetMode>(json!("original"));

        assert_closed::<ManualWorkingInfobaseIdentity>();
        assert_closed::<ManualWorkingInfobaseBaseline>();
    }

    #[test]
    fn factories_emit_canonical_transition_order() {
        let layer_a = SupportLayerId::parse("layer-a").unwrap();
        let layer_b = SupportLayerId::parse("layer-b").unwrap();
        let object = MetadataObjectId::parse(OBJECT_A).unwrap();
        let transitions = SupportTransitions::new(vec![
            SupportTransition::enable_configuration_changes(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                layer_a,
            ),
            SupportTransition::make_object_editable(
                object,
                RepositoryTargetDisplay::parse("Catalog.A").unwrap(),
                layer_b,
            ),
        ])
        .unwrap();
        assert_eq!(transitions.as_slice().len(), 2);
    }
}
use super::super::repository::{RepositoryActorIdentity, RepositoryHistoryCursor};
use super::super::scalars::{
    Diagnostic, RepositoryIdentityComponent, RepositoryTargetDisplay, RepositoryVersion,
    RequiredNullable,
};
use super::SupportMissingEvidenceKind;
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, MetadataObjectId, Sha256Digest, SupportLayerId, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;

const MAX_GENERAL_ITEMS: usize = 1_024;
const MAX_METADATA_ITEMS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportContractError(pub(super) &'static str);

impl fmt::Display for SupportContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for SupportContractError {}

fn contract_digest<T: ContractDigestRecord>(
    record: &T,
    message: &'static str,
) -> Result<Sha256Digest, SupportContractError> {
    canonical_contract_digest(record, None).map_err(|_| SupportContractError(message))
}

macro_rules! literal_bool {
    ($name:ident, $value:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(super) struct $name;

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_bool($value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let observed = bool::deserialize(deserializer)?;
                (observed == $value)
                    .then_some(Self)
                    .ok_or_else(|| D::Error::custom(concat!("expected literal ", stringify!($value))))
            }
        }

        impl JsonSchema for $name {
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "boolean", "const": $value })
            }
        }
    };
}

literal_bool!(TrueLiteral, true);
literal_bool!(FalseLiteral, false);

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        pub(super) enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(LockedState, "locked");
wire_literal!(EditableState, "editable");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[allow(private_interfaces)]
#[serde(
    tag = "transitionKind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum SupportTransition {
    EnableConfigurationChanges {
        configuration_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
        from_enabled: FalseLiteral,
        to_enabled: TrueLiteral,
    },
    RestoreConfigurationChangesDisabled {
        configuration_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
        from_enabled: TrueLiteral,
        to_enabled: FalseLiteral,
    },
    MakeObjectEditable {
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
        from_state: LockedState,
        to_state: EditableState,
    },
    RestoreObjectLocked {
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
        from_state: EditableState,
        to_state: LockedState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SupportTransitionTarget {
    Configuration(SupportLayerId),
    Object(MetadataObjectId, SupportLayerId),
}

impl SupportTransition {
    pub(crate) const fn enable_configuration_changes(
        configuration_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
    ) -> Self {
        Self::EnableConfigurationChanges {
            configuration_display,
            layer_id,
            from_enabled: FalseLiteral,
            to_enabled: TrueLiteral,
        }
    }

    pub(crate) const fn restore_configuration_changes_disabled(
        configuration_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
    ) -> Self {
        Self::RestoreConfigurationChangesDisabled {
            configuration_display,
            layer_id,
            from_enabled: TrueLiteral,
            to_enabled: FalseLiteral,
        }
    }

    pub(crate) const fn make_object_editable(
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
    ) -> Self {
        Self::MakeObjectEditable {
            object_id,
            object_display,
            layer_id,
            from_state: LockedState::Value,
            to_state: EditableState::Value,
        }
    }

    pub(crate) const fn restore_object_locked(
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
    ) -> Self {
        Self::RestoreObjectLocked {
            object_id,
            object_display,
            layer_id,
            from_state: EditableState::Value,
            to_state: LockedState::Value,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Self::EnableConfigurationChanges { .. } => 0,
            Self::RestoreConfigurationChangesDisabled { .. } => 1,
            Self::MakeObjectEditable { .. } => 2,
            Self::RestoreObjectLocked { .. } => 3,
        }
    }

    fn target(&self) -> SupportTransitionTarget {
        match self {
            Self::EnableConfigurationChanges { layer_id, .. }
            | Self::RestoreConfigurationChangesDisabled { layer_id, .. } => {
                SupportTransitionTarget::Configuration(layer_id.clone())
            }
            Self::MakeObjectEditable {
                object_id,
                layer_id,
                ..
            }
            | Self::RestoreObjectLocked {
                object_id,
                layer_id,
                ..
            } => SupportTransitionTarget::Object(object_id.clone(), layer_id.clone()),
        }
    }

    fn canonical_cmp(&self, other: &Self) -> Ordering {
        self.rank()
            .cmp(&other.rank())
            .then_with(|| match (self, other) {
                (
                    Self::EnableConfigurationChanges { layer_id: left, .. },
                    Self::EnableConfigurationChanges {
                        layer_id: right, ..
                    },
                )
                | (
                    Self::RestoreConfigurationChangesDisabled { layer_id: left, .. },
                    Self::RestoreConfigurationChangesDisabled {
                        layer_id: right, ..
                    },
                ) => left.cmp(right),
                (
                    Self::MakeObjectEditable {
                        object_id: left_object,
                        layer_id: left_layer,
                        ..
                    },
                    Self::MakeObjectEditable {
                        object_id: right_object,
                        layer_id: right_layer,
                        ..
                    },
                )
                | (
                    Self::RestoreObjectLocked {
                        object_id: left_object,
                        layer_id: left_layer,
                        ..
                    },
                    Self::RestoreObjectLocked {
                        object_id: right_object,
                        layer_id: right_layer,
                        ..
                    },
                ) => left_object
                    .cmp(right_object)
                    .then(left_layer.cmp(right_layer)),
                _ => Ordering::Equal,
            })
    }

    pub(crate) fn layer_id(&self) -> &SupportLayerId {
        match self {
            Self::EnableConfigurationChanges { layer_id, .. }
            | Self::RestoreConfigurationChangesDisabled { layer_id, .. }
            | Self::MakeObjectEditable { layer_id, .. }
            | Self::RestoreObjectLocked { layer_id, .. } => layer_id,
        }
    }

    pub(crate) fn object_id(&self) -> Option<&MetadataObjectId> {
        match self {
            Self::MakeObjectEditable { object_id, .. }
            | Self::RestoreObjectLocked { object_id, .. } => Some(object_id),
            Self::EnableConfigurationChanges { .. }
            | Self::RestoreConfigurationChangesDisabled { .. } => None,
        }
    }

    pub(crate) fn target_display(&self) -> &RepositoryTargetDisplay {
        match self {
            Self::EnableConfigurationChanges {
                configuration_display,
                ..
            }
            | Self::RestoreConfigurationChangesDisabled {
                configuration_display,
                ..
            } => configuration_display,
            Self::MakeObjectEditable { object_display, .. }
            | Self::RestoreObjectLocked { object_display, .. } => object_display,
        }
    }

    pub(crate) const fn is_restore(&self) -> bool {
        matches!(
            self,
            Self::RestoreConfigurationChangesDisabled { .. } | Self::RestoreObjectLocked { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportTransitions(Vec<SupportTransition>);

impl SupportTransitions {
    pub(crate) fn new(values: Vec<SupportTransition>) -> Result<Self, SupportContractError> {
        if values.len() > MAX_METADATA_ITEMS {
            return Err(SupportContractError("support transition list is too large"));
        }
        if values
            .windows(2)
            .any(|pair| pair[0].canonical_cmp(&pair[1]) != Ordering::Less)
        {
            return Err(SupportContractError(
                "support transitions must be unique and canonically ordered",
            ));
        }
        let mut targets = HashSet::with_capacity(values.len());
        if values.iter().any(|value| !targets.insert(value.target())) {
            return Err(SupportContractError(
                "support transition list contains duplicate or inverse targets",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportTransition] {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn contains(&self, transition: &SupportTransition) -> bool {
        self.0.contains(transition)
    }
}

impl<'de> Deserialize<'de> for SupportTransitions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<SupportTransition>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for SupportTransitions {
    fn schema_name() -> Cow<'static, str> {
        "SupportTransitions".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_METADATA_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportTransition>(),
        })
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportTransitionOverlapKind {
    SameTarget,
    LayerDependency,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportTransitionConflict {
    repository_version: RepositoryVersion,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    repository_actor: RequiredNullable<RepositoryActorIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    object_id: Option<MetadataObjectId>,
    object_display: RepositoryTargetDisplay,
    layer_id: SupportLayerId,
    authorized_transition: SupportTransition,
    external_transition_digest: Sha256Digest,
    overlap_kind: SupportTransitionOverlapKind,
    diagnostic: Diagnostic,
}

impl SupportTransitionConflict {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_capability_adapter(
        repository_version: RepositoryVersion,
        repository_actor: RequiredNullable<RepositoryActorIdentity>,
        object_id: Option<MetadataObjectId>,
        object_display: RepositoryTargetDisplay,
        layer_id: SupportLayerId,
        authorized_transition: SupportTransition,
        external_transition_digest: Sha256Digest,
        overlap_kind: SupportTransitionOverlapKind,
        diagnostic: Diagnostic,
    ) -> Result<Self, SupportContractError> {
        if object_id.as_ref() != authorized_transition.object_id()
            || &object_display != authorized_transition.target_display()
            || &layer_id != authorized_transition.layer_id()
        {
            return Err(SupportContractError(
                "support transition conflict target disagrees with its authorized transition",
            ));
        }
        Ok(Self {
            repository_version,
            repository_actor,
            object_id,
            object_display,
            layer_id,
            authorized_transition,
            external_transition_digest,
            overlap_kind,
            diagnostic,
        })
    }

    fn compare_after_version(&self, other: &Self) -> Ordering {
        self.authorized_transition
            .canonical_cmp(&other.authorized_transition)
            .then(
                self.external_transition_digest
                    .cmp(&other.external_transition_digest),
            )
            .then(self.overlap_kind.cmp(&other.overlap_kind))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportTransitionConflicts(Vec<SupportTransitionConflict>);

impl SupportTransitionConflicts {
    pub(crate) fn new(
        values: Vec<SupportTransitionConflict>,
        history_order: &dyn SupportHistoryOrderAuthority,
    ) -> Result<Self, SupportContractError> {
        if values.is_empty() || values.len() > MAX_GENERAL_ITEMS {
            return Err(SupportContractError(
                "support transition conflicts must be non-empty and bounded",
            ));
        }
        for pair in values.windows(2) {
            let ordering = history_order
                .compare_versions(&pair[0].repository_version, &pair[1].repository_version)?;
            let ordering = if ordering == Ordering::Equal {
                pair[0].compare_after_version(&pair[1])
            } else {
                ordering
            };
            if ordering != Ordering::Less {
                return Err(SupportContractError(
                    "support transition conflicts must be unique and in proven history order",
                ));
            }
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportTransitionConflict] {
        &self.0
    }
}

impl JsonSchema for SupportTransitionConflicts {
    fn schema_name() -> Cow<'static, str> {
        "SupportTransitionConflicts".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_GENERAL_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportTransitionConflict>(),
        })
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportCandidateReason {
    PlatformComparison,
    CanonicalDelta,
    Ownership,
    AddDelete,
    ReferenceClosure,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RepositoryAction {
    Add,
    Modify,
    Delete,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportCurrentState {
    NotApplicable,
    Locked,
    Editable,
    OffSupport,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VendorChangeRestriction {
    ChangesAllowed,
    ChangesNotRecommended,
    ChangesForbidden,
    Unknown,
    NotApplicable,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportRequiredState {
    NotApplicable,
    Editable,
    PreserveOffSupport,
    OffSupportRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportCandidateReasons(Vec<SupportCandidateReason>);

impl SupportCandidateReasons {
    pub(crate) fn new(values: Vec<SupportCandidateReason>) -> Result<Self, SupportContractError> {
        if values.is_empty() || values.len() > 5 {
            return Err(SupportContractError(
                "support candidate reasons must be non-empty and bounded",
            ));
        }
        if values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(SupportContractError(
                "support candidate reasons must be unique and in declaration order",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportCandidateReason] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SupportCandidateReasons {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<SupportCandidateReason>::deserialize(deserializer)?)
            .map_err(D::Error::custom)
    }
}

impl JsonSchema for SupportCandidateReasons {
    fn schema_name() -> Cow<'static, str> {
        "SupportCandidateReasons".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": 5,
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportCandidateReason>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportCandidate {
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    #[serde(skip_serializing_if = "Option::is_none")]
    layer_id: Option<SupportLayerId>,
    repository_action: RepositoryAction,
    current_state: SupportCurrentState,
    vendor_restriction: VendorChangeRestriction,
    required_state: SupportRequiredState,
    reasons: SupportCandidateReasons,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedSupportCandidate {
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    layer_id: Option<SupportLayerId>,
    repository_action: RepositoryAction,
    current_state: SupportCurrentState,
    vendor_restriction: VendorChangeRestriction,
    required_state: SupportRequiredState,
    reasons: SupportCandidateReasons,
}

/// Producer-backed evidence for the exact candidate-reason projection.
///
/// This authority is deliberately neither serialized nor deserialized.  Each
/// reason is represented by its own named producer-digest slot, so a caller
/// cannot promote a wire candidate merely by supplying a raw enum list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportCandidateEvidenceAuthority {
    object_id: MetadataObjectId,
    layer_id: Option<SupportLayerId>,
    platform_comparison_evidence_digest: Option<Sha256Digest>,
    canonical_delta_evidence_digest: Option<Sha256Digest>,
    ownership_evidence_digest: Option<Sha256Digest>,
    add_delete_evidence_digest: Option<Sha256Digest>,
    reference_closure_evidence_digest: Option<Sha256Digest>,
}

impl SupportCandidateEvidenceAuthority {
    /// Fixture mint only. Production promotion must be added by the task that
    /// owns the concrete typed comparison/delta/ownership/closure producers.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_capability_adapter(
        object_id: MetadataObjectId,
        layer_id: Option<SupportLayerId>,
        platform_comparison_evidence_digest: Option<Sha256Digest>,
        canonical_delta_evidence_digest: Option<Sha256Digest>,
        ownership_evidence_digest: Option<Sha256Digest>,
        add_delete_evidence_digest: Option<Sha256Digest>,
        reference_closure_evidence_digest: Option<Sha256Digest>,
    ) -> Result<Self, SupportContractError> {
        if platform_comparison_evidence_digest.is_none()
            && canonical_delta_evidence_digest.is_none()
            && ownership_evidence_digest.is_none()
            && add_delete_evidence_digest.is_none()
            && reference_closure_evidence_digest.is_none()
        {
            return Err(SupportContractError(
                "candidate evidence authority must contain at least one typed producer digest",
            ));
        }
        Ok(Self {
            object_id,
            layer_id,
            platform_comparison_evidence_digest,
            canonical_delta_evidence_digest,
            ownership_evidence_digest,
            add_delete_evidence_digest,
            reference_closure_evidence_digest,
        })
    }

    fn reasons(&self) -> SupportCandidateReasons {
        let mut reasons = Vec::with_capacity(5);
        if self.platform_comparison_evidence_digest.is_some() {
            reasons.push(SupportCandidateReason::PlatformComparison);
        }
        if self.canonical_delta_evidence_digest.is_some() {
            reasons.push(SupportCandidateReason::CanonicalDelta);
        }
        if self.ownership_evidence_digest.is_some() {
            reasons.push(SupportCandidateReason::Ownership);
        }
        if self.add_delete_evidence_digest.is_some() {
            reasons.push(SupportCandidateReason::AddDelete);
        }
        if self.reference_closure_evidence_digest.is_some() {
            reasons.push(SupportCandidateReason::ReferenceClosure);
        }
        SupportCandidateReasons::new(reasons)
            .expect("typed producer slots always project canonical non-empty reasons")
    }
}

pub(crate) trait SupportCandidateEvidenceResolver {
    fn resolve_candidate_evidence(
        &self,
        candidate: &UnvalidatedSupportCandidate,
    ) -> Result<SupportCandidateEvidenceAuthority, SupportContractError>;
}

impl SupportCandidate {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_evidence_authority(
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        layer_id: Option<SupportLayerId>,
        repository_action: RepositoryAction,
        current_state: SupportCurrentState,
        vendor_restriction: VendorChangeRestriction,
        required_state: SupportRequiredState,
        evidence: &SupportCandidateEvidenceAuthority,
    ) -> Result<Self, SupportContractError> {
        if object_id != evidence.object_id || layer_id != evidence.layer_id {
            return Err(SupportContractError(
                "candidate identity disagrees with its typed producer evidence",
            ));
        }
        let layer_required = current_state != SupportCurrentState::NotApplicable
            || vendor_restriction != VendorChangeRestriction::NotApplicable
            || required_state != SupportRequiredState::NotApplicable;
        if layer_required != layer_id.is_some() {
            return Err(SupportContractError(
                "support candidate layer presence disagrees with support state",
            ));
        }
        if required_state == SupportRequiredState::PreserveOffSupport
            && current_state != SupportCurrentState::OffSupport
        {
            return Err(SupportContractError(
                "preserve-off-support requires an observed off-support state",
            ));
        }
        Ok(Self {
            object_id,
            object_display,
            layer_id,
            repository_action,
            current_state,
            vendor_restriction,
            required_state,
            reasons: evidence.reasons(),
        })
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedSupportCandidate,
        evidence: &SupportCandidateEvidenceAuthority,
    ) -> Result<Self, SupportContractError> {
        if wire.reasons != evidence.reasons() {
            return Err(SupportContractError(
                "candidate reasons disagree with typed producer evidence",
            ));
        }
        Self::from_evidence_authority(
            wire.object_id,
            wire.object_display,
            wire.layer_id,
            wire.repository_action,
            wire.current_state,
            wire.vendor_restriction,
            wire.required_state,
            evidence,
        )
    }

    fn semantic_key(&self) -> (&MetadataObjectId, &Option<SupportLayerId>) {
        (&self.object_id, &self.layer_id)
    }

    pub(crate) const fn object_id(&self) -> &MetadataObjectId {
        &self.object_id
    }

    pub(crate) const fn object_display(&self) -> &RepositoryTargetDisplay {
        &self.object_display
    }

    pub(crate) const fn layer_id(&self) -> Option<&SupportLayerId> {
        self.layer_id.as_ref()
    }

    pub(crate) const fn vendor_restriction(&self) -> VendorChangeRestriction {
        self.vendor_restriction
    }

    pub(crate) const fn current_state(&self) -> SupportCurrentState {
        self.current_state
    }

    pub(crate) const fn required_state(&self) -> SupportRequiredState {
        self.required_state
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportCandidates(Vec<SupportCandidate>);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(transparent)]
pub(crate) struct UnvalidatedSupportCandidates(Vec<UnvalidatedSupportCandidate>);

impl SupportCandidates {
    pub(crate) fn new(values: Vec<SupportCandidate>) -> Result<Self, SupportContractError> {
        if values.len() > MAX_METADATA_ITEMS {
            return Err(SupportContractError("support candidate list is too large"));
        }
        if values
            .windows(2)
            .any(|pair| pair[0].semantic_key() >= pair[1].semantic_key())
        {
            return Err(SupportContractError(
                "support candidates must be unique and canonically ordered",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportCandidate] {
        &self.0
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedSupportCandidates,
        resolver: &dyn SupportCandidateEvidenceResolver,
    ) -> Result<Self, SupportContractError> {
        let mut values = Vec::with_capacity(wire.0.len());
        for candidate in wire.0 {
            let evidence = resolver.resolve_candidate_evidence(&candidate)?;
            values.push(SupportCandidate::from_wire(candidate, &evidence)?);
        }
        Self::new(values)
    }
}

impl JsonSchema for SupportCandidates {
    fn schema_name() -> Cow<'static, str> {
        "SupportCandidates".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_METADATA_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportCandidate>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportCandidateSetDigestRecord {
    candidate_set_id: UnicaId,
    candidates: SupportCandidates,
}

impl contract_digest_record_sealed::Sealed for SupportCandidateSetDigestRecord {}
impl ContractDigestRecord for SupportCandidateSetDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportCandidateSet {
    candidate_set_id: UnicaId,
    candidates: SupportCandidates,
    candidate_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedSupportCandidateSet {
    candidate_set_id: UnicaId,
    candidates: UnvalidatedSupportCandidates,
    candidate_set_digest: Sha256Digest,
}

impl SupportCandidateSet {
    pub(crate) fn new(
        candidate_set_id: UnicaId,
        candidates: SupportCandidates,
    ) -> Result<Self, SupportContractError> {
        let candidate_set_digest = contract_digest(
            &SupportCandidateSetDigestRecord {
                candidate_set_id: candidate_set_id.clone(),
                candidates: candidates.clone(),
            },
            "support candidate-set digest failed",
        )?;
        Ok(Self {
            candidate_set_id,
            candidates,
            candidate_set_digest,
        })
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedSupportCandidateSet,
        resolver: &dyn SupportCandidateEvidenceResolver,
    ) -> Result<Self, SupportContractError> {
        let candidates = SupportCandidates::from_wire(wire.candidates, resolver)?;
        let value = Self::new(wire.candidate_set_id, candidates)?;
        (value.candidate_set_digest == wire.candidate_set_digest)
            .then_some(value)
            .ok_or(SupportContractError(
                "support candidate-set digest mismatch",
            ))
    }

    pub(crate) const fn candidate_set_id(&self) -> &UnicaId {
        &self.candidate_set_id
    }

    pub(crate) const fn candidates(&self) -> &SupportCandidates {
        &self.candidates
    }

    pub(crate) const fn digest(&self) -> &Sha256Digest {
        &self.candidate_set_digest
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportBlockerReason {
    ConfigurationChangesDisabled,
    ObjectLocked,
    VendorRestriction,
    OffSupportRequired,
    ClassificationIncomplete,
    DiagnosticCoverageIncomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportBlocker {
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    #[serde(skip_serializing_if = "Option::is_none")]
    layer_id: Option<SupportLayerId>,
    reason: SupportBlockerReason,
    diagnostic: Diagnostic,
}

impl SupportBlocker {
    fn semantic_key(
        &self,
    ) -> (
        &MetadataObjectId,
        &Option<SupportLayerId>,
        SupportBlockerReason,
    ) {
        (&self.object_id, &self.layer_id, self.reason)
    }

    pub(crate) const fn object_id(&self) -> &MetadataObjectId {
        &self.object_id
    }

    pub(crate) const fn object_display(&self) -> &RepositoryTargetDisplay {
        &self.object_display
    }

    pub(crate) const fn layer_id(&self) -> Option<&SupportLayerId> {
        self.layer_id.as_ref()
    }

    pub(crate) const fn reason(&self) -> SupportBlockerReason {
        self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportBlockers(Vec<SupportBlocker>);

impl SupportBlockers {
    pub(crate) fn new(values: Vec<SupportBlocker>) -> Result<Self, SupportContractError> {
        if values.len() > MAX_METADATA_ITEMS {
            return Err(SupportContractError("support blocker list is too large"));
        }
        if values
            .windows(2)
            .any(|pair| pair[0].semantic_key() >= pair[1].semantic_key())
        {
            return Err(SupportContractError(
                "support blockers must be unique and canonically ordered",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportBlocker] {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for SupportBlockers {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<SupportBlocker>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for SupportBlockers {
    fn schema_name() -> Cow<'static, str> {
        "SupportBlockers".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_METADATA_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportBlocker>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportMissingEvidenceKinds(Vec<SupportMissingEvidenceKind>);

impl SupportMissingEvidenceKinds {
    pub(crate) fn new(
        values: Vec<SupportMissingEvidenceKind>,
    ) -> Result<Self, SupportContractError> {
        if values.len() > SupportMissingEvidenceKind::ALL.len() {
            return Err(SupportContractError(
                "support missing-evidence projection is too large",
            ));
        }
        if values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(SupportContractError(
                "support missing-evidence kinds must be unique and in declaration order",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportMissingEvidenceKind] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SupportMissingEvidenceKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<SupportMissingEvidenceKind>::deserialize(
            deserializer,
        )?)
        .map_err(D::Error::custom)
    }
}

impl JsonSchema for SupportMissingEvidenceKinds {
    fn schema_name() -> Cow<'static, str> {
        "SupportMissingEvidenceKinds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": SupportMissingEvidenceKind::ALL.len(),
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportMissingEvidenceKind>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[allow(clippy::enum_variant_names)]
#[serde(
    tag = "gapKind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum SupportEvidenceGap {
    CandidateEvidence {
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        #[serde(skip_serializing_if = "Option::is_none")]
        layer_id: Option<SupportLayerId>,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    SupportLayerRecoveryEvidence {
        layer_id: SupportLayerId,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    UnidentifiedSupportLayerEvidence {
        layer_observation_digest: Sha256Digest,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    ManualWorkingInfobaseEvidence {
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    PrerequisiteVersionEvidence {
        support_action_id: UnicaId,
        #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
        repository_version: RequiredNullable<RepositoryVersion>,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    RepositoryHistoryEvidence {
        from_cursor: RepositoryHistoryCursor,
        #[serde(skip_serializing_if = "Option::is_none")]
        first_observed_version: Option<RepositoryVersion>,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    GlobalSupportEvidence {
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[allow(clippy::enum_variant_names)]
#[serde(
    tag = "gapKind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum UnvalidatedSupportEvidenceGap {
    CandidateEvidence {
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        layer_id: Option<SupportLayerId>,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    SupportLayerRecoveryEvidence {
        layer_id: SupportLayerId,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    UnidentifiedSupportLayerEvidence {
        layer_observation_digest: Sha256Digest,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    ManualWorkingInfobaseEvidence {
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    PrerequisiteVersionEvidence {
        support_action_id: UnicaId,
        #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
        repository_version: RequiredNullable<RepositoryVersion>,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    RepositoryHistoryEvidence {
        from_cursor: RepositoryHistoryCursor,
        first_observed_version: Option<RepositoryVersion>,
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
    GlobalSupportEvidence {
        missing_evidence_kind: SupportMissingEvidenceKind,
        diagnostic: Diagnostic,
    },
}

impl SupportEvidenceGap {
    fn validate_kind(self) -> Result<Self, SupportContractError> {
        let kind = self.missing_evidence_kind();
        let allowed = match &self {
            Self::CandidateEvidence { .. } => matches!(
                kind,
                SupportMissingEvidenceKind::CandidateClassificationUnavailable
                    | SupportMissingEvidenceKind::DiagnosticCoverageIncomplete
            ),
            Self::SupportLayerRecoveryEvidence { .. } => matches!(
                kind,
                SupportMissingEvidenceKind::RecoverySourceMissing
                    | SupportMissingEvidenceKind::RecoveryArtifactMissing
                    | SupportMissingEvidenceKind::RecoveryArtifactStale
                    | SupportMissingEvidenceKind::RecoveryArtifactKindMismatch
                    | SupportMissingEvidenceKind::ConfigurationUpdateRejected
                    | SupportMissingEvidenceKind::RecoveryCapabilityUnproven
                    | SupportMissingEvidenceKind::RecoveryLayerIdentityMismatch
                    | SupportMissingEvidenceKind::RecoveryHandoffUnavailable
                    | SupportMissingEvidenceKind::RecoveryHandoffUnreadable
                    | SupportMissingEvidenceKind::RecoveryRetentionLeaseBroken
            ),
            Self::UnidentifiedSupportLayerEvidence { .. } => {
                kind == SupportMissingEvidenceKind::SupportLayerIdentityUnavailable
            }
            Self::ManualWorkingInfobaseEvidence { .. } => matches!(
                kind,
                SupportMissingEvidenceKind::ManualLeaseBusy
                    | SupportMissingEvidenceKind::ManualLeaseEffectUnknown
                    | SupportMissingEvidenceKind::ManualBaselineDirty
                    | SupportMissingEvidenceKind::ManualBaselineInspectionUnproven
                    | SupportMissingEvidenceKind::ManualCapabilityUnproven
            ),
            Self::PrerequisiteVersionEvidence { .. } => matches!(
                kind,
                SupportMissingEvidenceKind::RepositoryActorUnavailable
                    | SupportMissingEvidenceKind::ManualTargetModeUnavailable
                    | SupportMissingEvidenceKind::WorkingInfobaseIdentityUnavailable
                    | SupportMissingEvidenceKind::RootDeltaUnavailable
                    | SupportMissingEvidenceKind::ContentDeltaUnavailable
                    | SupportMissingEvidenceKind::OwnershipEvidenceUnavailable
                    | SupportMissingEvidenceKind::SupportLayerIdentityUnavailable
                    | SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete
            ),
            Self::RepositoryHistoryEvidence {
                first_observed_version,
                ..
            } => {
                let kind_allowed = matches!(
                    kind,
                    SupportMissingEvidenceKind::RepositoryActorUnavailable
                        | SupportMissingEvidenceKind::RootDeltaUnavailable
                        | SupportMissingEvidenceKind::ContentDeltaUnavailable
                        | SupportMissingEvidenceKind::OwnershipEvidenceUnavailable
                        | SupportMissingEvidenceKind::SupportLayerIdentityUnavailable
                        | SupportMissingEvidenceKind::SupportGraphIncomplete
                        | SupportMissingEvidenceKind::DiagnosticCoverageIncomplete
                        | SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete
                );
                let successor_presence = first_observed_version.is_none()
                    == (kind == SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete);
                kind_allowed && successor_presence
            }
            Self::GlobalSupportEvidence { .. } => matches!(
                kind,
                SupportMissingEvidenceKind::SupportGraphIncomplete
                    | SupportMissingEvidenceKind::DiagnosticCoverageIncomplete
            ),
        };
        allowed.then_some(self).ok_or(SupportContractError(
            "evidence-gap kind is not legal for its leaf",
        ))
    }

    pub(crate) const fn missing_evidence_kind(&self) -> SupportMissingEvidenceKind {
        match self {
            Self::CandidateEvidence {
                missing_evidence_kind,
                ..
            }
            | Self::SupportLayerRecoveryEvidence {
                missing_evidence_kind,
                ..
            }
            | Self::UnidentifiedSupportLayerEvidence {
                missing_evidence_kind,
                ..
            }
            | Self::ManualWorkingInfobaseEvidence {
                missing_evidence_kind,
                ..
            }
            | Self::PrerequisiteVersionEvidence {
                missing_evidence_kind,
                ..
            }
            | Self::RepositoryHistoryEvidence {
                missing_evidence_kind,
                ..
            }
            | Self::GlobalSupportEvidence {
                missing_evidence_kind,
                ..
            } => *missing_evidence_kind,
        }
    }

    pub(crate) const fn is_recovery_gap(&self) -> bool {
        matches!(
            self,
            Self::SupportLayerRecoveryEvidence { .. }
                | Self::UnidentifiedSupportLayerEvidence { .. }
        )
    }

    pub(crate) const fn recovery_layer_id(&self) -> Option<&SupportLayerId> {
        match self {
            Self::SupportLayerRecoveryEvidence { layer_id, .. } => Some(layer_id),
            Self::CandidateEvidence { .. }
            | Self::UnidentifiedSupportLayerEvidence { .. }
            | Self::ManualWorkingInfobaseEvidence { .. }
            | Self::PrerequisiteVersionEvidence { .. }
            | Self::RepositoryHistoryEvidence { .. }
            | Self::GlobalSupportEvidence { .. } => None,
        }
    }

    pub(crate) const fn is_manual_working_infobase_gap(&self) -> bool {
        matches!(self, Self::ManualWorkingInfobaseEvidence { .. })
    }

    pub(crate) const fn is_preflight_classification_gap(&self) -> bool {
        matches!(
            self,
            Self::CandidateEvidence { .. }
                | Self::UnidentifiedSupportLayerEvidence { .. }
                | Self::RepositoryHistoryEvidence { .. }
                | Self::GlobalSupportEvidence { .. }
        )
    }

    fn rank(&self) -> u8 {
        match self {
            Self::CandidateEvidence { .. } => 0,
            Self::SupportLayerRecoveryEvidence { .. } => 1,
            Self::UnidentifiedSupportLayerEvidence { .. } => 2,
            Self::ManualWorkingInfobaseEvidence { .. } => 3,
            Self::PrerequisiteVersionEvidence { .. } => 4,
            Self::RepositoryHistoryEvidence { .. } => 5,
            Self::GlobalSupportEvidence { .. } => 6,
        }
    }
}

impl From<UnvalidatedSupportEvidenceGap> for SupportEvidenceGap {
    fn from(value: UnvalidatedSupportEvidenceGap) -> Self {
        match value {
            UnvalidatedSupportEvidenceGap::CandidateEvidence {
                object_id,
                object_display,
                layer_id,
                missing_evidence_kind,
                diagnostic,
            } => Self::CandidateEvidence {
                object_id,
                object_display,
                layer_id,
                missing_evidence_kind,
                diagnostic,
            },
            UnvalidatedSupportEvidenceGap::SupportLayerRecoveryEvidence {
                layer_id,
                missing_evidence_kind,
                diagnostic,
            } => Self::SupportLayerRecoveryEvidence {
                layer_id,
                missing_evidence_kind,
                diagnostic,
            },
            UnvalidatedSupportEvidenceGap::UnidentifiedSupportLayerEvidence {
                layer_observation_digest,
                missing_evidence_kind,
                diagnostic,
            } => Self::UnidentifiedSupportLayerEvidence {
                layer_observation_digest,
                missing_evidence_kind,
                diagnostic,
            },
            UnvalidatedSupportEvidenceGap::ManualWorkingInfobaseEvidence {
                working_infobase_identity,
                missing_evidence_kind,
                diagnostic,
            } => Self::ManualWorkingInfobaseEvidence {
                working_infobase_identity,
                missing_evidence_kind,
                diagnostic,
            },
            UnvalidatedSupportEvidenceGap::PrerequisiteVersionEvidence {
                support_action_id,
                repository_version,
                missing_evidence_kind,
                diagnostic,
            } => Self::PrerequisiteVersionEvidence {
                support_action_id,
                repository_version,
                missing_evidence_kind,
                diagnostic,
            },
            UnvalidatedSupportEvidenceGap::RepositoryHistoryEvidence {
                from_cursor,
                first_observed_version,
                missing_evidence_kind,
                diagnostic,
            } => Self::RepositoryHistoryEvidence {
                from_cursor,
                first_observed_version,
                missing_evidence_kind,
                diagnostic,
            },
            UnvalidatedSupportEvidenceGap::GlobalSupportEvidence {
                missing_evidence_kind,
                diagnostic,
            } => Self::GlobalSupportEvidence {
                missing_evidence_kind,
                diagnostic,
            },
        }
    }
}

impl<'de> Deserialize<'de> for SupportEvidenceGap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from(UnvalidatedSupportEvidenceGap::deserialize(deserializer)?)
            .validate_kind()
            .map_err(D::Error::custom)
    }
}

pub(crate) trait SupportHistoryOrderAuthority {
    fn compare_versions(
        &self,
        left: &RepositoryVersion,
        right: &RepositoryVersion,
    ) -> Result<Ordering, SupportContractError>;

    fn compare_cursors(
        &self,
        left: &RepositoryHistoryCursor,
        right: &RepositoryHistoryCursor,
    ) -> Result<Ordering, SupportContractError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SupportEvidenceGaps(Vec<SupportEvidenceGap>);

impl SupportEvidenceGaps {
    pub(crate) fn new(
        values: Vec<SupportEvidenceGap>,
        history_order: &dyn SupportHistoryOrderAuthority,
    ) -> Result<Self, SupportContractError> {
        if values.len() > MAX_GENERAL_ITEMS {
            return Err(SupportContractError(
                "support evidence-gap list is too large",
            ));
        }
        for pair in values.windows(2) {
            if compare_gap(&pair[0], &pair[1], history_order)? != Ordering::Less {
                return Err(SupportContractError(
                    "support evidence gaps must be unique and canonically ordered",
                ));
            }
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportEvidenceGap] {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn missing_evidence_kinds(&self) -> SupportMissingEvidenceKinds {
        let mut present = HashSet::new();
        for gap in &self.0 {
            present.insert(gap.missing_evidence_kind());
        }
        SupportMissingEvidenceKinds::new(
            SupportMissingEvidenceKind::ALL
                .iter()
                .copied()
                .filter(|kind| present.contains(kind))
                .collect(),
        )
        .expect("enum-order projection is canonical and bounded")
    }
}

impl JsonSchema for SupportEvidenceGaps {
    fn schema_name() -> Cow<'static, str> {
        "SupportEvidenceGaps".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": MAX_GENERAL_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportEvidenceGap>(),
        })
    }
}

fn compare_nullable_version(
    left: &RequiredNullable<RepositoryVersion>,
    right: &RequiredNullable<RepositoryVersion>,
    order: &dyn SupportHistoryOrderAuthority,
) -> Result<Ordering, SupportContractError> {
    match (left.as_ref(), right.as_ref()) {
        (None, None) => Ok(Ordering::Equal),
        (None, Some(_)) => Ok(Ordering::Less),
        (Some(_), None) => Ok(Ordering::Greater),
        (Some(left), Some(right)) => order.compare_versions(left, right),
    }
}

fn compare_optional_version(
    left: &Option<RepositoryVersion>,
    right: &Option<RepositoryVersion>,
    order: &dyn SupportHistoryOrderAuthority,
) -> Result<Ordering, SupportContractError> {
    match (left, right) {
        (None, None) => Ok(Ordering::Equal),
        (None, Some(_)) => Ok(Ordering::Less),
        (Some(_), None) => Ok(Ordering::Greater),
        (Some(left), Some(right)) => order.compare_versions(left, right),
    }
}

fn compare_gap(
    left: &SupportEvidenceGap,
    right: &SupportEvidenceGap,
    order: &dyn SupportHistoryOrderAuthority,
) -> Result<Ordering, SupportContractError> {
    let rank = left.rank().cmp(&right.rank());
    if rank != Ordering::Equal {
        return Ok(rank);
    }
    let kind_order = left
        .missing_evidence_kind()
        .cmp(&right.missing_evidence_kind());
    let identity_order = match (left, right) {
        (
            SupportEvidenceGap::CandidateEvidence {
                object_id: left_object,
                layer_id: left_layer,
                ..
            },
            SupportEvidenceGap::CandidateEvidence {
                object_id: right_object,
                layer_id: right_layer,
                ..
            },
        ) => left_object
            .cmp(right_object)
            .then(left_layer.cmp(right_layer)),
        (
            SupportEvidenceGap::SupportLayerRecoveryEvidence {
                layer_id: left_layer,
                ..
            },
            SupportEvidenceGap::SupportLayerRecoveryEvidence {
                layer_id: right_layer,
                ..
            },
        ) => left_layer.cmp(right_layer),
        (
            SupportEvidenceGap::UnidentifiedSupportLayerEvidence {
                layer_observation_digest: left_digest,
                ..
            },
            SupportEvidenceGap::UnidentifiedSupportLayerEvidence {
                layer_observation_digest: right_digest,
                ..
            },
        ) => left_digest.cmp(right_digest),
        (
            SupportEvidenceGap::ManualWorkingInfobaseEvidence {
                working_infobase_identity: left_identity,
                ..
            },
            SupportEvidenceGap::ManualWorkingInfobaseEvidence {
                working_infobase_identity: right_identity,
                ..
            },
        ) => left_identity.digest().cmp(right_identity.digest()),
        (
            SupportEvidenceGap::PrerequisiteVersionEvidence {
                support_action_id: left_action,
                repository_version: left_version,
                ..
            },
            SupportEvidenceGap::PrerequisiteVersionEvidence {
                support_action_id: right_action,
                repository_version: right_version,
                ..
            },
        ) => {
            let action = left_action.cmp(right_action);
            if action != Ordering::Equal {
                action
            } else {
                compare_nullable_version(left_version, right_version, order)?
            }
        }
        (
            SupportEvidenceGap::RepositoryHistoryEvidence {
                from_cursor: left_cursor,
                first_observed_version: left_version,
                ..
            },
            SupportEvidenceGap::RepositoryHistoryEvidence {
                from_cursor: right_cursor,
                first_observed_version: right_version,
                ..
            },
        ) => {
            let cursor = order.compare_cursors(left_cursor, right_cursor)?;
            if cursor != Ordering::Equal {
                cursor
            } else {
                compare_optional_version(left_version, right_version, order)?
            }
        }
        (
            SupportEvidenceGap::GlobalSupportEvidence { .. },
            SupportEvidenceGap::GlobalSupportEvidence { .. },
        ) => Ordering::Equal,
        _ => Ordering::Equal,
    };
    Ok(identity_order.then(kind_order))
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ManualSupportTargetMode {
    ReservedOriginal,
    SeparateWorkingInfobase,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportPreflightOutcome {
    Ready,
    ManualSupportRequired,
    VendorForbidsChanges,
    SupportPreflightInconclusive,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[allow(clippy::enum_variant_names)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportGateMismatchKind {
    CandidateSetChanged,
    CanonicalDeltaChanged,
    OrdinaryResultChanged,
    SupportGraphChanged,
    RecoveryDistributionSetChanged,
    SettingsChanged,
    SandboxResultChanged,
    CapabilityRowChanged,
    OriginalFingerprintChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportGateInputDigests {
    candidate_set_digest: Sha256Digest,
    canonical_delta_digest: Sha256Digest,
    ordinary_result_digest: Sha256Digest,
    support_graph_digest: Sha256Digest,
    support_recovery_distribution_set_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    sandbox_result_digest: Sha256Digest,
    capability_row_digest: Sha256Digest,
    original_fingerprint_digest: Sha256Digest,
}

impl SupportGateInputDigests {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        candidate_set_digest: Sha256Digest,
        canonical_delta_digest: Sha256Digest,
        ordinary_result_digest: Sha256Digest,
        support_graph_digest: Sha256Digest,
        support_recovery_distribution_set_digest: Sha256Digest,
        settings_digest: Sha256Digest,
        sandbox_result_digest: Sha256Digest,
        capability_row_digest: Sha256Digest,
        original_fingerprint_digest: Sha256Digest,
    ) -> Self {
        Self {
            candidate_set_digest,
            canonical_delta_digest,
            ordinary_result_digest,
            support_graph_digest,
            support_recovery_distribution_set_digest,
            settings_digest,
            sandbox_result_digest,
            capability_row_digest,
            original_fingerprint_digest,
        }
    }

    pub(crate) const fn candidate_set_digest(&self) -> &Sha256Digest {
        &self.candidate_set_digest
    }

    pub(crate) const fn canonical_delta_digest(&self) -> &Sha256Digest {
        &self.canonical_delta_digest
    }

    pub(crate) const fn ordinary_result_digest(&self) -> &Sha256Digest {
        &self.ordinary_result_digest
    }

    pub(crate) const fn support_graph_digest(&self) -> &Sha256Digest {
        &self.support_graph_digest
    }

    pub(crate) const fn support_recovery_distribution_set_digest(&self) -> &Sha256Digest {
        &self.support_recovery_distribution_set_digest
    }

    pub(crate) const fn settings_digest(&self) -> &Sha256Digest {
        &self.settings_digest
    }

    pub(crate) const fn sandbox_result_digest(&self) -> &Sha256Digest {
        &self.sandbox_result_digest
    }

    pub(crate) const fn capability_row_digest(&self) -> &Sha256Digest {
        &self.capability_row_digest
    }

    pub(crate) const fn original_fingerprint_digest(&self) -> &Sha256Digest {
        &self.original_fingerprint_digest
    }
}

/// Capability-adapter projection of every support layer reachable from the
/// configuration-root support-settings window.
///
/// This is an authority input, not a wire contract: recovery coverage must be
/// proven against it before any manual authorization is published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RootReachableSupportLayerSet {
    layer_ids: Vec<SupportLayerId>,
    support_graph_digest: Sha256Digest,
}

impl RootReachableSupportLayerSet {
    /// Fixture mint only until the support-graph resolver can return this
    /// opaque authority directly from its rich, digest-checked graph result.
    #[cfg(test)]
    pub(crate) fn from_capability_adapter(
        layer_ids: Vec<SupportLayerId>,
        support_graph_digest: Sha256Digest,
    ) -> Result<Self, SupportContractError> {
        if layer_ids.is_empty()
            || layer_ids.len() > MAX_GENERAL_ITEMS
            || layer_ids.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(SupportContractError(
                "root-reachable support layers must be non-empty, unique, bounded, and canonical",
            ));
        }
        Ok(Self {
            layer_ids,
            support_graph_digest,
        })
    }

    pub(crate) fn as_slice(&self) -> &[SupportLayerId] {
        &self.layer_ids
    }

    pub(crate) const fn support_graph_digest(&self) -> &Sha256Digest {
        &self.support_graph_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManualWorkingInfobaseIdentityDigestRecord {
    computer: RepositoryIdentityComponent,
    infobase: RepositoryIdentityComponent,
}

impl contract_digest_record_sealed::Sealed for ManualWorkingInfobaseIdentityDigestRecord {}
impl ContractDigestRecord for ManualWorkingInfobaseIdentityDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseIdentity {
    computer: RepositoryIdentityComponent,
    infobase: RepositoryIdentityComponent,
    digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedManualWorkingInfobaseIdentity {
    computer: RepositoryIdentityComponent,
    infobase: RepositoryIdentityComponent,
    digest: Sha256Digest,
}

impl ManualWorkingInfobaseIdentity {
    pub(crate) fn new(
        computer: RepositoryIdentityComponent,
        infobase: RepositoryIdentityComponent,
    ) -> Result<Self, SupportContractError> {
        let digest = contract_digest(
            &ManualWorkingInfobaseIdentityDigestRecord {
                computer: computer.clone(),
                infobase: infobase.clone(),
            },
            "manual working-infobase identity digest failed",
        )?;
        Ok(Self {
            computer,
            infobase,
            digest,
        })
    }

    pub(crate) const fn digest(&self) -> &Sha256Digest {
        &self.digest
    }
}

impl<'de> Deserialize<'de> for ManualWorkingInfobaseIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UnvalidatedManualWorkingInfobaseIdentity::deserialize(deserializer)?;
        let value = Self::new(wire.computer, wire.infobase).map_err(D::Error::custom)?;
        (value.digest == wire.digest)
            .then_some(value)
            .ok_or_else(|| D::Error::custom("manual working-infobase identity digest mismatch"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseBaselineDigestRecord {
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    repository_base_cursor: RepositoryHistoryCursor,
    recorded_object_version_map_digest: Sha256Digest,
    base_fingerprint: Sha256Digest,
    current_fingerprint: Sha256Digest,
    current_equals_base: TrueLiteral,
    support_graph_digest: Sha256Digest,
    baseline_inspection_receipt_id: UnicaId,
    exclusive_lease_capability_id: CapabilityRowId,
    lease_released_verified: TrueLiteral,
}

impl contract_digest_record_sealed::Sealed for ManualWorkingInfobaseBaselineDigestRecord {}
impl ContractDigestRecord for ManualWorkingInfobaseBaselineDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualWorkingInfobaseBaseline {
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    repository_base_cursor: RepositoryHistoryCursor,
    recorded_object_version_map_digest: Sha256Digest,
    base_fingerprint: Sha256Digest,
    current_fingerprint: Sha256Digest,
    current_equals_base: TrueLiteral,
    support_graph_digest: Sha256Digest,
    baseline_inspection_receipt_id: UnicaId,
    exclusive_lease_capability_id: CapabilityRowId,
    lease_released_verified: TrueLiteral,
    baseline_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedManualWorkingInfobaseBaseline {
    working_infobase_identity: ManualWorkingInfobaseIdentity,
    repository_base_cursor: RepositoryHistoryCursor,
    recorded_object_version_map_digest: Sha256Digest,
    base_fingerprint: Sha256Digest,
    current_fingerprint: Sha256Digest,
    current_equals_base: TrueLiteral,
    support_graph_digest: Sha256Digest,
    baseline_inspection_receipt_id: UnicaId,
    exclusive_lease_capability_id: CapabilityRowId,
    lease_released_verified: TrueLiteral,
    baseline_digest: Sha256Digest,
}

impl ManualWorkingInfobaseBaseline {
    /// Fixture mint only. Production promotion belongs to the exclusive
    /// working-IB inspection resolver and must compare the profile capability,
    /// gate graph, repository base, and observed clean state before minting.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        working_infobase_identity: ManualWorkingInfobaseIdentity,
        repository_base_cursor: RepositoryHistoryCursor,
        recorded_object_version_map_digest: Sha256Digest,
        base_fingerprint: Sha256Digest,
        current_fingerprint: Sha256Digest,
        support_graph_digest: Sha256Digest,
        baseline_inspection_receipt_id: UnicaId,
        exclusive_lease_capability_id: CapabilityRowId,
    ) -> Result<Self, SupportContractError> {
        if base_fingerprint != current_fingerprint {
            return Err(SupportContractError(
                "manual working-infobase current fingerprint differs from base",
            ));
        }
        let record = ManualWorkingInfobaseBaselineDigestRecord {
            working_infobase_identity,
            repository_base_cursor,
            recorded_object_version_map_digest,
            base_fingerprint,
            current_fingerprint,
            current_equals_base: TrueLiteral,
            support_graph_digest,
            baseline_inspection_receipt_id,
            exclusive_lease_capability_id,
            lease_released_verified: TrueLiteral,
        };
        let baseline_digest =
            contract_digest(&record, "manual working-infobase baseline digest failed")?;
        Ok(Self {
            working_infobase_identity: record.working_infobase_identity,
            repository_base_cursor: record.repository_base_cursor,
            recorded_object_version_map_digest: record.recorded_object_version_map_digest,
            base_fingerprint: record.base_fingerprint,
            current_fingerprint: record.current_fingerprint,
            current_equals_base: record.current_equals_base,
            support_graph_digest: record.support_graph_digest,
            baseline_inspection_receipt_id: record.baseline_inspection_receipt_id,
            exclusive_lease_capability_id: record.exclusive_lease_capability_id,
            lease_released_verified: record.lease_released_verified,
            baseline_digest,
        })
    }

    pub(crate) const fn working_infobase_identity(&self) -> &ManualWorkingInfobaseIdentity {
        &self.working_infobase_identity
    }

    pub(crate) const fn exclusive_lease_capability_id(&self) -> &CapabilityRowId {
        &self.exclusive_lease_capability_id
    }

    pub(crate) const fn support_graph_digest(&self) -> &Sha256Digest {
        &self.support_graph_digest
    }

    pub(crate) const fn baseline_digest(&self) -> &Sha256Digest {
        &self.baseline_digest
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VendorSupportDecision {
    ChangeTaskScope,
    UseNewerVendorDelivery,
    SafeAbandonment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct VendorSupportDecisions([VendorSupportDecision; 3]);

impl VendorSupportDecisions {
    pub(crate) const fn all() -> Self {
        Self([
            VendorSupportDecision::ChangeTaskScope,
            VendorSupportDecision::UseNewerVendorDelivery,
            VendorSupportDecision::SafeAbandonment,
        ])
    }

    pub(crate) const fn as_slice(&self) -> &[VendorSupportDecision] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for VendorSupportDecisions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let observed = <[VendorSupportDecision; 3]>::deserialize(deserializer)?;
        (observed == Self::all().0)
            .then_some(Self(observed))
            .ok_or_else(|| D::Error::custom("vendor support decisions must be the exact tuple"))
    }
}

impl JsonSchema for VendorSupportDecisions {
    fn schema_name() -> Cow<'static, str> {
        "VendorSupportDecisions".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [
                { "type": "string", "const": "changeTaskScope" },
                { "type": "string", "const": "useNewerVendorDelivery" },
                { "type": "string", "const": "safeAbandonment" }
            ],
            "items": false,
            "minItems": 3,
            "maxItems": 3,
        })
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportPrerequisiteMismatchKind {
    NoAuthorizedVersionObserved,
    VersionUnattributed,
    ReservedAccountUsed,
    MultipleAuthorizedVersions,
    UnauthorizedContentChanged,
    TargetModeMismatch,
    UnexpectedSupportTransition,
    SupportLayerChanged,
    OffSupportObserved,
    OverlappingExternalSupportChange,
    ArmingOrderViolated,
    RootLockRetained,
    ManualActorLockInventoryChanged,
    ReservedOriginalUsed,
    OriginalNotClean,
}

impl SupportPrerequisiteMismatchKind {
    pub(crate) const ALL: &'static [Self] = &[
        Self::NoAuthorizedVersionObserved,
        Self::VersionUnattributed,
        Self::ReservedAccountUsed,
        Self::MultipleAuthorizedVersions,
        Self::UnauthorizedContentChanged,
        Self::TargetModeMismatch,
        Self::UnexpectedSupportTransition,
        Self::SupportLayerChanged,
        Self::OffSupportObserved,
        Self::OverlappingExternalSupportChange,
        Self::ArmingOrderViolated,
        Self::RootLockRetained,
        Self::ManualActorLockInventoryChanged,
        Self::ReservedOriginalUsed,
        Self::OriginalNotClean,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum ExternalSupportOwnershipEvidence {
    SupportPrerequisiteReceipt {
        receipt_id: UnicaId,
        receipt_digest: Sha256Digest,
    },
    CapabilityProvenHistoryAttribution {
        repository_actor: RepositoryActorIdentity,
        attribution_evidence_digest: Sha256Digest,
        capability_row_id: CapabilityRowId,
    },
}

impl ExternalSupportOwnershipEvidence {
    pub(crate) const fn support_prerequisite_receipt(
        receipt_id: UnicaId,
        receipt_digest: Sha256Digest,
    ) -> Self {
        Self::SupportPrerequisiteReceipt {
            receipt_id,
            receipt_digest,
        }
    }

    pub(crate) const fn capability_proven_history_attribution(
        repository_actor: RepositoryActorIdentity,
        attribution_evidence_digest: Sha256Digest,
        capability_row_id: CapabilityRowId,
    ) -> Self {
        Self::CapabilityProvenHistoryAttribution {
            repository_actor,
            attribution_evidence_digest,
            capability_row_id,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[allow(clippy::enum_variant_names)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportArmStaleKind {
    HistoryChanged,
    SupportGateChanged,
    RelevantBaselineChanged,
    SupportGraphChanged,
    RecoveryDistributionSetChanged,
    OriginalFingerprintChanged,
}

impl SupportArmStaleKind {
    pub(crate) const ALL: &'static [Self] = &[
        Self::HistoryChanged,
        Self::SupportGateChanged,
        Self::RelevantBaselineChanged,
        Self::SupportGraphChanged,
        Self::RecoveryDistributionSetChanged,
        Self::OriginalFingerprintChanged,
    ];
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportActionPurpose {
    MainIntegrationPrerequisite,
    AbandonmentCleanup,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportRecoveryDisposition {
    RestoreThenReauthorize,
    PreserveExternalAndReauthorize,
    RestoreThenAbandon,
}

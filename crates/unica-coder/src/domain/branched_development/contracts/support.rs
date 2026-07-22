use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[path = "support/authorization.rs"]
mod authorization;
#[path = "support/evidence.rs"]
mod evidence;
#[path = "support/model.rs"]
mod model;
#[path = "support/preflight.rs"]
mod preflight;
#[path = "support/version_observation.rs"]
mod version_observation;

pub(crate) use authorization::*;
#[allow(unused_imports)]
pub(crate) use evidence::*;
pub(crate) use model::*;
#[allow(unused_imports)]
pub(crate) use preflight::*;
pub(crate) use version_observation::*;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SupportMissingEvidenceKind {
    CandidateClassificationUnavailable,
    DiagnosticCoverageIncomplete,
    SupportGraphIncomplete,
    RecoverySourceMissing,
    RecoveryArtifactMissing,
    RecoveryArtifactStale,
    RecoveryArtifactKindMismatch,
    ConfigurationUpdateRejected,
    RecoveryCapabilityUnproven,
    RecoveryLayerIdentityMismatch,
    RecoveryHandoffUnavailable,
    RecoveryHandoffUnreadable,
    RecoveryRetentionLeaseBroken,
    ManualLeaseBusy,
    ManualLeaseEffectUnknown,
    ManualBaselineDirty,
    ManualBaselineInspectionUnproven,
    ManualCapabilityUnproven,
    RepositoryActorUnavailable,
    ManualTargetModeUnavailable,
    WorkingInfobaseIdentityUnavailable,
    RootDeltaUnavailable,
    ContentDeltaUnavailable,
    OwnershipEvidenceUnavailable,
    SupportLayerIdentityUnavailable,
    RepositoryHistoryCoverageIncomplete,
}

impl SupportMissingEvidenceKind {
    pub(crate) const ALL: &'static [Self] = &[
        Self::CandidateClassificationUnavailable,
        Self::DiagnosticCoverageIncomplete,
        Self::SupportGraphIncomplete,
        Self::RecoverySourceMissing,
        Self::RecoveryArtifactMissing,
        Self::RecoveryArtifactStale,
        Self::RecoveryArtifactKindMismatch,
        Self::ConfigurationUpdateRejected,
        Self::RecoveryCapabilityUnproven,
        Self::RecoveryLayerIdentityMismatch,
        Self::RecoveryHandoffUnavailable,
        Self::RecoveryHandoffUnreadable,
        Self::RecoveryRetentionLeaseBroken,
        Self::ManualLeaseBusy,
        Self::ManualLeaseEffectUnknown,
        Self::ManualBaselineDirty,
        Self::ManualBaselineInspectionUnproven,
        Self::ManualCapabilityUnproven,
        Self::RepositoryActorUnavailable,
        Self::ManualTargetModeUnavailable,
        Self::WorkingInfobaseIdentityUnavailable,
        Self::RootDeltaUnavailable,
        Self::ContentDeltaUnavailable,
        Self::OwnershipEvidenceUnavailable,
        Self::SupportLayerIdentityUnavailable,
        Self::RepositoryHistoryCoverageIncomplete,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::CandidateClassificationUnavailable => "candidateClassificationUnavailable",
            Self::DiagnosticCoverageIncomplete => "diagnosticCoverageIncomplete",
            Self::SupportGraphIncomplete => "supportGraphIncomplete",
            Self::RecoverySourceMissing => "recoverySourceMissing",
            Self::RecoveryArtifactMissing => "recoveryArtifactMissing",
            Self::RecoveryArtifactStale => "recoveryArtifactStale",
            Self::RecoveryArtifactKindMismatch => "recoveryArtifactKindMismatch",
            Self::ConfigurationUpdateRejected => "configurationUpdateRejected",
            Self::RecoveryCapabilityUnproven => "recoveryCapabilityUnproven",
            Self::RecoveryLayerIdentityMismatch => "recoveryLayerIdentityMismatch",
            Self::RecoveryHandoffUnavailable => "recoveryHandoffUnavailable",
            Self::RecoveryHandoffUnreadable => "recoveryHandoffUnreadable",
            Self::RecoveryRetentionLeaseBroken => "recoveryRetentionLeaseBroken",
            Self::ManualLeaseBusy => "manualLeaseBusy",
            Self::ManualLeaseEffectUnknown => "manualLeaseEffectUnknown",
            Self::ManualBaselineDirty => "manualBaselineDirty",
            Self::ManualBaselineInspectionUnproven => "manualBaselineInspectionUnproven",
            Self::ManualCapabilityUnproven => "manualCapabilityUnproven",
            Self::RepositoryActorUnavailable => "repositoryActorUnavailable",
            Self::ManualTargetModeUnavailable => "manualTargetModeUnavailable",
            Self::WorkingInfobaseIdentityUnavailable => "workingInfobaseIdentityUnavailable",
            Self::RootDeltaUnavailable => "rootDeltaUnavailable",
            Self::ContentDeltaUnavailable => "contentDeltaUnavailable",
            Self::OwnershipEvidenceUnavailable => "ownershipEvidenceUnavailable",
            Self::SupportLayerIdentityUnavailable => "supportLayerIdentityUnavailable",
            Self::RepositoryHistoryCoverageIncomplete => "repositoryHistoryCoverageIncomplete",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SupportMissingEvidenceKind;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::schema_for;
    use serde_json::json;

    const WIRES: &[&str] = &[
        "candidateClassificationUnavailable",
        "diagnosticCoverageIncomplete",
        "supportGraphIncomplete",
        "recoverySourceMissing",
        "recoveryArtifactMissing",
        "recoveryArtifactStale",
        "recoveryArtifactKindMismatch",
        "configurationUpdateRejected",
        "recoveryCapabilityUnproven",
        "recoveryLayerIdentityMismatch",
        "recoveryHandoffUnavailable",
        "recoveryHandoffUnreadable",
        "recoveryRetentionLeaseBroken",
        "manualLeaseBusy",
        "manualLeaseEffectUnknown",
        "manualBaselineDirty",
        "manualBaselineInspectionUnproven",
        "manualCapabilityUnproven",
        "repositoryActorUnavailable",
        "manualTargetModeUnavailable",
        "workingInfobaseIdentityUnavailable",
        "rootDeltaUnavailable",
        "contentDeltaUnavailable",
        "ownershipEvidenceUnavailable",
        "supportLayerIdentityUnavailable",
        "repositoryHistoryCoverageIncomplete",
    ];

    #[test]
    fn missing_evidence_kind_has_the_exact_shared_wire_vocabulary() {
        assert_eq!(SupportMissingEvidenceKind::ALL.len(), WIRES.len());
        for (kind, wire) in SupportMissingEvidenceKind::ALL.iter().zip(WIRES) {
            assert_eq!(kind.as_str(), *wire);
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<SupportMissingEvidenceKind>(json!(wire)).unwrap(),
                *kind
            );
        }
        for invalid in ["repositoryHistoryMissing", "rootDelta", ""] {
            assert!(serde_json::from_value::<SupportMissingEvidenceKind>(json!(invalid)).is_err());
        }

        let schema = serde_json::to_value(schema_for!(SupportMissingEvidenceKind)).unwrap();
        audit_json_schema(&schema).unwrap();
        assert_eq!(schema["enum"], json!(WIRES));
    }
}

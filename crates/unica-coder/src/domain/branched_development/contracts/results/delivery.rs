use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::contracts::artifacts::{
    AcceptedArtifactKind, CompatibilityMode, ConfigurationIdentity, PlatformVersion,
    SafeResultCount,
};
use crate::domain::branched_development::contracts::repository::{
    RepositoryAnchor, RepositoryTargetIdentity,
};
use crate::domain::branched_development::contracts::scalars::{Diagnostic, NormalizedUtcInstant};
use crate::domain::branched_development::{Sha256Digest, SupportLayerId, UnicaId};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use std::borrow::Cow;
use std::fmt;

const MAX_DELIVERY_ITEMS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeliveryResultContractError(&'static str);

impl fmt::Display for DeliveryResultContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

#[cfg(test)]
mod deployment_tests {
    use super::*;
    use crate::domain::branched_development::contracts::artifacts::{
        ArtifactKind, ArtifactRole, CompatibilityMode, ConfigurationIdentity, PlatformVersion,
        SafeResultCount,
    };
    use crate::domain::branched_development::contracts::repository::{
        RepositoryAnchor, RepositoryAnchorObservationAuthority, RepositoryHistoryCursor,
    };
    use crate::domain::branched_development::contracts::scalars::{
        EmptyOrName, Name, NormalizedUtcInstant, RepositoryVersion,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::MetadataObjectId;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const CONFIGURATION_UUID: &str = "123e4567-e89b-12d3-a456-426614174020";
    const DISTRIBUTION_ID: &str = "123e4567-e89b-12d3-a456-426614174021";
    const VERIFICATION_ID: &str = "123e4567-e89b-12d3-a456-426614174022";
    const PROBE_ID: &str = "123e4567-e89b-12d3-a456-426614174023";
    const TASK_INFOBASE_ID: &str = "123e4567-e89b-12d3-a456-426614174024";
    const TASK_WORKSPACE_ID: &str = "123e4567-e89b-12d3-a456-426614174025";

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn identity() -> ConfigurationIdentity {
        ConfigurationIdentity::new(
            MetadataObjectId::parse(CONFIGURATION_UUID).unwrap(),
            Name::parse("Deployable configuration").unwrap(),
            EmptyOrName::parse("Vendor").unwrap(),
            EmptyOrName::parse("2.0").unwrap(),
        )
    }

    fn anchor() -> RepositoryAnchor {
        RepositoryAnchorObservationAuthority::test_only(
            digest('a'),
            RepositoryHistoryCursor::new(RepositoryVersion::parse("100").unwrap(), digest('b')),
            identity(),
            digest('c'),
        )
        .into_anchor()
        .unwrap()
    }

    fn inspection() -> ValidatedDeliveryInspectionAuthority {
        let counts = DistributionRuleCounts::new_test_only(
            SafeResultCount::new(1).unwrap(),
            SafeResultCount::new(0).unwrap(),
            SafeResultCount::new(1).unwrap(),
            SafeResultCount::new(0).unwrap(),
        );
        ValidatedDeliveryInspectionAuthority::from_authority(
            DeliveryInspectionAuthority::new_test_only(
                anchor(),
                true,
                true,
                true,
                PlatformVersion::parse("8.3.27.1000").unwrap(),
                CompatibilityMode::parse("Version8_3_24").unwrap(),
                DeliveryPermissions::new_test_only(true, false),
                counts,
                vec![],
                vec![],
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn distribution(role: ArtifactRole) -> DistributionData {
        let inspection = inspection();
        let preview = DistributionPreviewData::from_authority(
            DistributionPreviewAuthority::from_inspection_test_only(role, inspection).unwrap(),
        )
        .unwrap();
        let approval = ApprovedDistributionPreviewAuthority::approve_test_only(&preview);
        let observation = DistributionArtifactObservationAuthority::from_writer_test_only(
            &approval,
            UnicaId::parse(DISTRIBUTION_ID).unwrap(),
            digest('d'),
            NormalizedUtcInstant::parse("2026-07-22T02:03:04Z").unwrap(),
        );
        DistributionData::from_approved_preview(approval, observation).unwrap()
    }

    fn verification(artifact_id: UnicaId, sha256: Sha256Digest) -> ArtifactVerificationData {
        ArtifactVerificationData::unconstrained(
            ArtifactProbeObservationAuthority::from_probe_test_only(
                UnicaId::parse(VERIFICATION_ID).unwrap(),
                artifact_id,
                ArtifactKind::ConfigurationDistribution,
                sha256,
                UnicaId::parse(PROBE_ID).unwrap(),
                Some(identity()),
                Some(true),
                vec![],
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn selected() -> VerifiedBaselineDistributionAuthority {
        let distribution = distribution(ArtifactRole::BaselineDistribution);
        let verification = verification(
            distribution.artifact_id().clone(),
            distribution.sha256().clone(),
        );
        VerifiedBaselineDistributionAuthority::from_results_test_only(&distribution, &verification)
            .unwrap()
    }

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).unwrap()
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema::<T>())
            .unwrap()
            .is_valid(value)
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize> for T {}
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn assert_not_clone() {}
                }
                struct ImplementsClone;
                impl<T: ?Sized> AmbiguousIfClone<()> for T {}
                impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    assert_not_deserialize_owned!(VerifiedBaselineDistributionAuthority);
    assert_not_deserialize_owned!(DeploymentPlannedRoles);
    assert_not_deserialize_owned!(DeploymentPreviewData);
    assert_not_deserialize_owned!(DeploymentPreviewDigestRecord);
    assert_not_deserialize_owned!(ApprovedDeploymentPreviewAuthority);
    assert_not_deserialize_owned!(DeploymentObservationAuthority);
    assert_not_deserialize_owned!(DeploymentData);
    assert_not_clone!(VerifiedBaselineDistributionAuthority);
    assert_not_clone!(ApprovedDeploymentPreviewAuthority);
    assert_not_clone!(DeploymentObservationAuthority);

    fn assert_deployment_recursively_closed<T: JsonSchema>(valid: Value) {
        fn collect(value: &Value, pointer: String, output: &mut Vec<String>) {
            match value {
                Value::Object(object) => {
                    output.push(pointer.clone());
                    for (key, nested) in object {
                        let key = key.replace('~', "~0").replace('/', "~1");
                        collect(nested, format!("{pointer}/{key}"), output);
                    }
                }
                Value::Array(values) => {
                    for (index, nested) in values.iter().enumerate() {
                        collect(nested, format!("{pointer}/{index}"), output);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }

        assert!(schema_accepts::<T>(&valid));
        audit_json_schema(&schema::<T>()).unwrap();
        let mut pointers = Vec::new();
        collect(&valid, String::new(), &mut pointers);
        for pointer in pointers {
            let required_fields: Vec<_> = valid
                .pointer(&pointer)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            for required_field in required_fields {
                let mut invalid = valid.clone();
                invalid
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(&required_field);
                assert!(
                    !schema_accepts::<T>(&invalid),
                    "{} accepted omitted required field {required_field} at {pointer}",
                    T::schema_name()
                );
            }
            for forbidden in [
                "cwd",
                "localPath",
                "stateRoot",
                "workRoot",
                "pid",
                "processHandle",
                "password",
                "token",
                "secret",
                "credentialRef",
                "credentialReference",
                "rawConnection",
                "rawConnectionString",
                "serviceEndpoint",
            ] {
                let mut invalid = valid.clone();
                invalid
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(forbidden.to_owned(), json!("forbidden"));
                assert!(
                    !schema_accepts::<T>(&invalid),
                    "{} accepted {forbidden} at {pointer}",
                    T::schema_name()
                );
            }
        }
    }

    #[test]
    fn deployment_preview_is_closed_content_bound_and_has_the_exact_role_tuple() {
        let selected = selected();
        let preview = DeploymentPreviewData::from_verified_distribution(&selected).unwrap();
        let value = serde_json::to_value(&preview).unwrap();
        assert_eq!(value["distributionId"], DISTRIBUTION_ID);
        assert_eq!(value["distributionSha256"], digest('d').as_str());
        assert_eq!(value["destinationKind"], "ownedTaskInstance");
        assert_eq!(
            value["plannedRoles"],
            json!(["taskInfobase", "taskWorkspace"])
        );
        assert!(schema_accepts::<DeploymentPreviewData>(&value));
        audit_json_schema(&schema::<DeploymentPreviewData>()).unwrap();

        let roles = schema::<DeploymentPlannedRoles>();
        assert_eq!(roles["prefixItems"].as_array().unwrap().len(), 2);
        assert_eq!(roles["items"], false);
        assert_eq!(roles["minItems"], 2);
        assert_eq!(roles["maxItems"], 2);

        for forbidden in [
            "taskInfobaseId",
            "taskWorkspaceId",
            "vendorIdentity",
            "currentFingerprint",
            "vendorFingerprint",
            "currentEqualsVendor",
            "sourceFingerprint",
        ] {
            let mut invalid = value.clone();
            invalid
                .as_object_mut()
                .unwrap()
                .insert(forbidden.to_owned(), json!(DISTRIBUTION_ID));
            assert!(!schema_accepts::<DeploymentPreviewData>(&invalid));
        }

        let changed_selection = VerifiedBaselineDistributionAuthority {
            distribution_id: UnicaId::parse(DISTRIBUTION_ID).unwrap(),
            distribution_sha256: digest('e'),
            vendor_identity: identity(),
            source_fingerprint: digest('c'),
        };
        let changed_preview =
            DeploymentPreviewData::from_verified_distribution(&changed_selection).unwrap();
        assert_ne!(preview.preview_digest(), changed_preview.preview_digest());
    }

    #[test]
    fn only_a_matching_verified_baseline_distribution_can_be_selected() {
        let refresh = distribution(ArtifactRole::RefreshDistribution);
        let refresh_verification =
            verification(refresh.artifact_id().clone(), refresh.sha256().clone());
        assert!(
            VerifiedBaselineDistributionAuthority::from_results_test_only(
                &refresh,
                &refresh_verification,
            )
            .is_err()
        );

        let baseline = distribution(ArtifactRole::BaselineDistribution);
        let mismatched = verification(baseline.artifact_id().clone(), digest('e'));
        assert!(
            VerifiedBaselineDistributionAuthority::from_results_test_only(&baseline, &mismatched,)
                .is_err()
        );

        let selected = selected();
        let preview = DeploymentPreviewData::from_verified_distribution(&selected).unwrap();
        let substituted = VerifiedBaselineDistributionAuthority {
            distribution_id: selected.distribution_id.clone(),
            distribution_sha256: digest('e'),
            vendor_identity: selected.vendor_identity.clone(),
            source_fingerprint: selected.source_fingerprint.clone(),
        };
        assert!(
            ApprovedDeploymentPreviewAuthority::approve_test_only(&preview, substituted).is_err()
        );
    }

    #[test]
    fn deployment_apply_consumes_approval_and_projects_exact_lineage() {
        let selected = selected();
        let preview = DeploymentPreviewData::from_verified_distribution(&selected).unwrap();
        let preview_digest = preview.preview_digest().clone();
        let approval =
            ApprovedDeploymentPreviewAuthority::approve_test_only(&preview, selected).unwrap();
        let observation = DeploymentObservationAuthority::from_deployment_test_only(
            &approval,
            UnicaId::parse(TASK_INFOBASE_ID).unwrap(),
            UnicaId::parse(TASK_WORKSPACE_ID).unwrap(),
            digest('f'),
            digest('f'),
        )
        .unwrap();
        let data = DeploymentData::from_approved_preview(approval, observation).unwrap();
        let value = serde_json::to_value(&data).unwrap();

        assert_eq!(
            value["vendorIdentity"],
            serde_json::to_value(identity()).unwrap()
        );
        assert_eq!(value["sourceFingerprint"], digest('c').as_str());
        assert_eq!(value["currentFingerprint"], value["vendorFingerprint"]);
        assert_eq!(value["currentEqualsVendor"], true);
        assert_eq!(value["previewDigest"], preview_digest.as_str());
        assert!(schema_accepts::<DeploymentData>(&value));
        audit_json_schema(&schema::<DeploymentData>()).unwrap();
    }

    #[test]
    fn deployment_results_and_preview_digest_record_are_recursively_closed() {
        let selected = selected();
        let record = DeploymentPreviewDigestRecord {
            distribution_id: selected.distribution_id.clone(),
            distribution_sha256: selected.distribution_sha256.clone(),
            destination_kind: OwnedTaskInstanceDestination::Value,
            planned_roles: DeploymentPlannedRoles::canonical(),
        };
        assert_deployment_recursively_closed::<DeploymentPreviewDigestRecord>(
            serde_json::to_value(record).unwrap(),
        );

        let preview = DeploymentPreviewData::from_verified_distribution(&selected).unwrap();
        assert_deployment_recursively_closed::<DeploymentPreviewData>(
            serde_json::to_value(&preview).unwrap(),
        );
        let approval =
            ApprovedDeploymentPreviewAuthority::approve_test_only(&preview, selected).unwrap();
        let observation = DeploymentObservationAuthority::from_deployment_test_only(
            &approval,
            UnicaId::parse(TASK_INFOBASE_ID).unwrap(),
            UnicaId::parse(TASK_WORKSPACE_ID).unwrap(),
            digest('f'),
            digest('f'),
        )
        .unwrap();
        let data = DeploymentData::from_approved_preview(approval, observation).unwrap();
        assert_deployment_recursively_closed::<DeploymentData>(serde_json::to_value(data).unwrap());
    }

    #[test]
    fn deployment_observation_rejects_same_target_id_and_fingerprint_mismatch() {
        let selected = selected();
        let preview = DeploymentPreviewData::from_verified_distribution(&selected).unwrap();
        let approval =
            ApprovedDeploymentPreviewAuthority::approve_test_only(&preview, selected).unwrap();
        let same_id = UnicaId::parse(TASK_INFOBASE_ID).unwrap();
        assert!(DeploymentObservationAuthority::from_deployment_test_only(
            &approval,
            same_id.clone(),
            same_id,
            digest('f'),
            digest('f'),
        )
        .is_err());
        assert!(DeploymentObservationAuthority::from_deployment_test_only(
            &approval,
            UnicaId::parse(TASK_INFOBASE_ID).unwrap(),
            UnicaId::parse(TASK_WORKSPACE_ID).unwrap(),
            digest('f'),
            digest('e'),
        )
        .is_err());

        let observation = DeploymentObservationAuthority::from_deployment_test_only(
            &approval,
            UnicaId::parse(TASK_INFOBASE_ID).unwrap(),
            UnicaId::parse(TASK_WORKSPACE_ID).unwrap(),
            digest('f'),
            digest('f'),
        )
        .unwrap();
        let substituted_selection = VerifiedBaselineDistributionAuthority {
            distribution_id: UnicaId::parse(DISTRIBUTION_ID).unwrap(),
            distribution_sha256: digest('e'),
            vendor_identity: identity(),
            source_fingerprint: digest('c'),
        };
        let substituted_preview =
            DeploymentPreviewData::from_verified_distribution(&substituted_selection).unwrap();
        let substituted_approval = ApprovedDeploymentPreviewAuthority::approve_test_only(
            &substituted_preview,
            substituted_selection,
        )
        .unwrap();
        assert!(DeploymentData::from_approved_preview(substituted_approval, observation).is_err());
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    use crate::domain::branched_development::contracts::artifacts::{
        AcceptedArtifactKind, ArtifactKind, ConfigurationIdentity,
    };
    use crate::domain::branched_development::contracts::scalars::{Diagnostic, EmptyOrName, Name};
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::MetadataObjectId;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const VERIFICATION_ID: &str = "123e4567-e89b-12d3-a456-426614174010";
    const ARTIFACT_ID: &str = "123e4567-e89b-12d3-a456-426614174011";
    const PROBE_ID: &str = "123e4567-e89b-12d3-a456-426614174012";

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn identity() -> ConfigurationIdentity {
        ConfigurationIdentity::new(
            MetadataObjectId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap(),
            Name::parse("Vendor configuration").unwrap(),
            EmptyOrName::parse("Vendor").unwrap(),
            EmptyOrName::parse("1.0").unwrap(),
        )
    }

    fn diagnostics(values: &[&str]) -> Vec<Diagnostic> {
        values
            .iter()
            .map(|value| Diagnostic::parse(value).unwrap())
            .collect()
    }

    fn probe(
        kind: ArtifactKind,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<ArtifactProbeObservationAuthority, DeliveryResultContractError> {
        let (support_identity, current_equals_vendor) = match kind {
            ArtifactKind::ConfigurationDistribution => (Some(identity()), Some(true)),
            ArtifactKind::OrdinaryConfiguration
            | ArtifactKind::ConfigurationUpdate
            | ArtifactKind::InvalidArtifact => (None, None),
        };
        ArtifactProbeObservationAuthority::from_probe_test_only(
            UnicaId::parse(VERIFICATION_ID).unwrap(),
            UnicaId::parse(ARTIFACT_ID).unwrap(),
            kind,
            digest('d'),
            UnicaId::parse(PROBE_ID).unwrap(),
            support_identity,
            current_equals_vendor,
            diagnostics,
        )
    }

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).unwrap()
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema::<T>())
            .unwrap()
            .is_valid(value)
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize> for T {}
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn assert_not_clone() {}
                }
                struct ImplementsClone;
                impl<T: ?Sized> AmbiguousIfClone<()> for T {}
                impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    assert_not_deserialize_owned!(ArtifactVerificationDiagnostics);
    assert_not_deserialize_owned!(ArtifactVerificationDiagnosticsDigestRecord);
    assert_not_deserialize_owned!(ArtifactProbeObservationAuthority);
    assert_not_deserialize_owned!(ArtifactVerificationData);
    assert_not_deserialize_owned!(UnconstrainedDistributionArtifactVerificationData);
    assert_not_deserialize_owned!(ExpectedDistributionArtifactVerificationData);
    assert_not_deserialize_owned!(UnconstrainedOrdinaryArtifactVerificationData);
    assert_not_deserialize_owned!(ExpectedOrdinaryArtifactVerificationData);
    assert_not_clone!(ArtifactProbeObservationAuthority);

    fn assert_verification_recursively_closed<T: JsonSchema>(valid: Value) {
        fn collect(value: &Value, pointer: String, output: &mut Vec<String>) {
            match value {
                Value::Object(object) => {
                    output.push(pointer.clone());
                    for (key, nested) in object {
                        let key = key.replace('~', "~0").replace('/', "~1");
                        collect(nested, format!("{pointer}/{key}"), output);
                    }
                }
                Value::Array(values) => {
                    for (index, nested) in values.iter().enumerate() {
                        collect(nested, format!("{pointer}/{index}"), output);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }

        assert!(schema_accepts::<T>(&valid));
        audit_json_schema(&schema::<T>()).unwrap();
        let mut pointers = Vec::new();
        collect(&valid, String::new(), &mut pointers);
        for pointer in pointers {
            let required_fields: Vec<_> = valid
                .pointer(&pointer)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            for required_field in required_fields {
                let mut invalid = valid.clone();
                invalid
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(&required_field);
                assert!(
                    !schema_accepts::<T>(&invalid),
                    "{} accepted omitted required field {required_field} at {pointer}",
                    T::schema_name()
                );
            }
            for forbidden in [
                "cwd",
                "localPath",
                "stateRoot",
                "workRoot",
                "pid",
                "processHandle",
                "password",
                "token",
                "secret",
                "credentialRef",
                "credentialReference",
                "rawConnection",
                "rawConnectionString",
                "serviceEndpoint",
            ] {
                let mut invalid = valid.clone();
                invalid
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(forbidden.to_owned(), json!("forbidden"));
                assert!(
                    !schema_accepts::<T>(&invalid),
                    "{} accepted {forbidden} at {pointer}",
                    T::schema_name()
                );
            }
        }
    }

    #[test]
    fn verification_has_exactly_four_closed_physical_leaves() {
        let values = [
            ArtifactVerificationData::unconstrained(
                probe(ArtifactKind::ConfigurationDistribution, diagnostics(&["a"])).unwrap(),
            )
            .unwrap(),
            ArtifactVerificationData::expected(
                probe(ArtifactKind::ConfigurationDistribution, diagnostics(&["a"])).unwrap(),
                AcceptedArtifactKind::ConfigurationDistribution,
            )
            .unwrap(),
            ArtifactVerificationData::unconstrained(
                probe(ArtifactKind::OrdinaryConfiguration, diagnostics(&["a"])).unwrap(),
            )
            .unwrap(),
            ArtifactVerificationData::expected(
                probe(ArtifactKind::OrdinaryConfiguration, diagnostics(&["a"])).unwrap(),
                AcceptedArtifactKind::OrdinaryConfiguration,
            )
            .unwrap(),
        ];

        let serialized: Vec<_> = values
            .iter()
            .map(|value| serde_json::to_value(value).unwrap())
            .collect();
        assert!(serialized[0].get("expectedKind").is_none());
        assert_eq!(
            serialized[0]["supportIdentity"],
            serde_json::to_value(identity()).unwrap()
        );
        assert_eq!(serialized[1]["expectedKind"], "configurationDistribution");
        assert!(serialized[2].get("expectedKind").is_none());
        assert!(serialized[2].get("supportIdentity").is_none());
        assert_eq!(serialized[3]["expectedKind"], "ordinaryConfiguration");
        assert!(serialized
            .iter()
            .all(|value| value["expectationMatched"] == true));
        assert!(serialized
            .iter()
            .all(schema_accepts::<ArtifactVerificationData>));

        let union = schema::<ArtifactVerificationData>();
        assert_eq!(union["oneOf"].as_array().unwrap().len(), 4);
        audit_json_schema(&union).unwrap();
    }

    #[test]
    fn verification_diagnostics_are_bounded_and_strictly_utf8_ordered() {
        assert!(
            ArtifactVerificationDiagnostics::new_test_only(diagnostics(&[
                "A", "a", "e\u{301}", "é"
            ]))
            .is_ok()
        );
        assert!(ArtifactVerificationDiagnostics::new_test_only(diagnostics(&["a", "A"])).is_err());
        assert!(ArtifactVerificationDiagnostics::new_test_only(diagnostics(&["a", "a"])).is_err());
        let too_many: Vec<_> = (0..1025)
            .map(|index| Diagnostic::parse(&format!("diagnostic-{index:04}")).unwrap())
            .collect();
        assert!(ArtifactVerificationDiagnostics::new_test_only(too_many).is_err());

        let collection = schema::<ArtifactVerificationDiagnostics>();
        assert_eq!(collection["minItems"], 0);
        assert_eq!(collection["maxItems"], 1024);
        assert_eq!(collection["uniqueItems"], true);

        let first = ArtifactVerificationData::unconstrained(
            probe(ArtifactKind::OrdinaryConfiguration, diagnostics(&["a"])).unwrap(),
        )
        .unwrap();
        let second = ArtifactVerificationData::unconstrained(
            probe(ArtifactKind::OrdinaryConfiguration, diagnostics(&["b"])).unwrap(),
        )
        .unwrap();
        assert_ne!(first.diagnostics_digest(), second.diagnostics_digest());
    }

    #[test]
    fn verification_results_and_diagnostics_digest_record_are_recursively_closed() {
        let record = ArtifactVerificationDiagnosticsDigestRecord {
            artifact_id: UnicaId::parse(ARTIFACT_ID).unwrap(),
            probe_id: UnicaId::parse(PROBE_ID).unwrap(),
            kind: AcceptedArtifactKind::OrdinaryConfiguration,
            diagnostics: ArtifactVerificationDiagnostics::new_test_only(diagnostics(&["a"]))
                .unwrap(),
        };
        assert_verification_recursively_closed::<ArtifactVerificationDiagnosticsDigestRecord>(
            serde_json::to_value(record).unwrap(),
        );

        let unconstrained_distribution = ArtifactVerificationData::unconstrained(
            probe(ArtifactKind::ConfigurationDistribution, vec![]).unwrap(),
        )
        .unwrap();
        assert_verification_recursively_closed::<UnconstrainedDistributionArtifactVerificationData>(
            serde_json::to_value(unconstrained_distribution).unwrap(),
        );

        let expected_distribution = ArtifactVerificationData::expected(
            probe(ArtifactKind::ConfigurationDistribution, vec![]).unwrap(),
            AcceptedArtifactKind::ConfigurationDistribution,
        )
        .unwrap();
        assert_verification_recursively_closed::<ExpectedDistributionArtifactVerificationData>(
            serde_json::to_value(expected_distribution).unwrap(),
        );

        let unconstrained_ordinary = ArtifactVerificationData::unconstrained(
            probe(ArtifactKind::OrdinaryConfiguration, vec![]).unwrap(),
        )
        .unwrap();
        assert_verification_recursively_closed::<UnconstrainedOrdinaryArtifactVerificationData>(
            serde_json::to_value(unconstrained_ordinary).unwrap(),
        );

        let expected_ordinary = ArtifactVerificationData::expected(
            probe(ArtifactKind::OrdinaryConfiguration, vec![]).unwrap(),
            AcceptedArtifactKind::OrdinaryConfiguration,
        )
        .unwrap();
        assert_verification_recursively_closed::<ExpectedOrdinaryArtifactVerificationData>(
            serde_json::to_value(expected_ordinary).unwrap(),
        );
    }

    #[test]
    fn mismatch_and_unaccepted_probe_kinds_cannot_publish_completed_verification() {
        let mismatch = ArtifactVerificationData::expected(
            probe(ArtifactKind::ConfigurationDistribution, vec![]).unwrap(),
            AcceptedArtifactKind::OrdinaryConfiguration,
        );
        assert!(mismatch.is_err());
        assert!(probe(ArtifactKind::ConfigurationUpdate, vec![]).is_err());
        assert!(probe(ArtifactKind::InvalidArtifact, vec![]).is_err());

        let bad_vendor = ArtifactProbeObservationAuthority::from_probe_test_only(
            UnicaId::parse(VERIFICATION_ID).unwrap(),
            UnicaId::parse(ARTIFACT_ID).unwrap(),
            ArtifactKind::ConfigurationDistribution,
            digest('d'),
            UnicaId::parse(PROBE_ID).unwrap(),
            Some(identity()),
            Some(false),
            vec![],
        );
        assert!(bad_vendor.is_err());
    }

    #[test]
    fn verification_schema_rejects_cross_leaf_and_forbidden_fields() {
        let ordinary = serde_json::to_value(
            ArtifactVerificationData::unconstrained(
                probe(ArtifactKind::OrdinaryConfiguration, vec![]).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        for (field, value) in [
            ("supportIdentity", serde_json::to_value(identity()).unwrap()),
            ("currentEqualsVendor", json!(true)),
            ("cwd", json!("/workspace")),
            ("token", json!("secret")),
        ] {
            let mut invalid = ordinary.clone();
            invalid
                .as_object_mut()
                .unwrap()
                .insert(field.to_owned(), value);
            assert!(!schema_accepts::<ArtifactVerificationData>(&invalid));
        }
        let mut false_match = ordinary;
        false_match["expectationMatched"] = json!(false);
        assert!(!schema_accepts::<ArtifactVerificationData>(&false_match));
    }
}

impl std::error::Error for DeliveryResultContractError {}

macro_rules! string_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

macro_rules! literal_bool {
    ($name:ident, $value:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct $name;

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_bool($value)
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

string_literal!(ConfigurationScope, "configuration");
string_literal!(MetadataObjectScope, "metadataObject");
string_literal!(AllowedVerdict, "allowed");
string_literal!(ForbiddenVerdict, "forbidden");
string_literal!(BaselineDistributionRole, "baselineDistribution");
string_literal!(RefreshDistributionRole, "refreshDistribution");
string_literal!(ConfigurationDistributionKind, "configurationDistribution");
string_literal!(OrdinaryConfigurationKind, "ordinaryConfiguration");
string_literal!(OwnedTaskInstanceDestination, "ownedTaskInstance");
literal_bool!(TrueLiteral, true);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeliveryPermissions {
    distribution_allowed: bool,
    update_allowed: bool,
}

impl DeliveryPermissions {
    #[cfg(test)]
    pub(crate) const fn new_test_only(distribution_allowed: bool, update_allowed: bool) -> Self {
        Self {
            distribution_allowed,
            update_allowed,
        }
    }

    pub(crate) const fn distribution_allowed(&self) -> bool {
        self.distribution_allowed
    }

    pub(crate) const fn update_allowed(&self) -> bool {
        self.update_allowed
    }
}

macro_rules! distribution_rule_count_leaf {
    ($name:ident, $scope:ty, $verdict:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct $name {
            scope: $scope,
            verdict: $verdict,
            count: SafeResultCount,
        }

        impl $name {
            const fn new(count: SafeResultCount) -> Self {
                Self {
                    scope: <$scope>::Value,
                    verdict: <$verdict>::Value,
                    count,
                }
            }

            pub(crate) const fn count(&self) -> SafeResultCount {
                self.count
            }
        }
    };
}

distribution_rule_count_leaf!(
    ConfigurationAllowedDistributionRuleCount,
    ConfigurationScope,
    AllowedVerdict
);
distribution_rule_count_leaf!(
    ConfigurationForbiddenDistributionRuleCount,
    ConfigurationScope,
    ForbiddenVerdict
);
distribution_rule_count_leaf!(
    MetadataObjectAllowedDistributionRuleCount,
    MetadataObjectScope,
    AllowedVerdict
);
distribution_rule_count_leaf!(
    MetadataObjectForbiddenDistributionRuleCount,
    MetadataObjectScope,
    ForbiddenVerdict
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum DistributionRuleCount {
    ConfigurationAllowed(ConfigurationAllowedDistributionRuleCount),
    ConfigurationForbidden(ConfigurationForbiddenDistributionRuleCount),
    MetadataObjectAllowed(MetadataObjectAllowedDistributionRuleCount),
    MetadataObjectForbidden(MetadataObjectForbiddenDistributionRuleCount),
}

impl JsonSchema for DistributionRuleCount {
    fn schema_name() -> Cow<'static, str> {
        "DistributionRuleCount".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        super::super::schema::one_of_schema(vec![
            generator.subschema_for::<ConfigurationAllowedDistributionRuleCount>(),
            generator.subschema_for::<ConfigurationForbiddenDistributionRuleCount>(),
            generator.subschema_for::<MetadataObjectAllowedDistributionRuleCount>(),
            generator.subschema_for::<MetadataObjectForbiddenDistributionRuleCount>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct DistributionRuleCounts([DistributionRuleCount; 4]);

impl DistributionRuleCounts {
    #[cfg(test)]
    pub(crate) const fn new_test_only(
        configuration_allowed: SafeResultCount,
        configuration_forbidden: SafeResultCount,
        metadata_object_allowed: SafeResultCount,
        metadata_object_forbidden: SafeResultCount,
    ) -> Self {
        Self([
            DistributionRuleCount::ConfigurationAllowed(
                ConfigurationAllowedDistributionRuleCount::new(configuration_allowed),
            ),
            DistributionRuleCount::ConfigurationForbidden(
                ConfigurationForbiddenDistributionRuleCount::new(configuration_forbidden),
            ),
            DistributionRuleCount::MetadataObjectAllowed(
                MetadataObjectAllowedDistributionRuleCount::new(metadata_object_allowed),
            ),
            DistributionRuleCount::MetadataObjectForbidden(
                MetadataObjectForbiddenDistributionRuleCount::new(metadata_object_forbidden),
            ),
        ])
    }

    pub(crate) const fn as_slice(&self) -> &[DistributionRuleCount] {
        &self.0
    }
}

impl JsonSchema for DistributionRuleCounts {
    fn schema_name() -> Cow<'static, str> {
        "DistributionRuleCounts".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [
                generator.subschema_for::<ConfigurationAllowedDistributionRuleCount>(),
                generator.subschema_for::<ConfigurationForbiddenDistributionRuleCount>(),
                generator.subschema_for::<MetadataObjectAllowedDistributionRuleCount>(),
                generator.subschema_for::<MetadataObjectForbiddenDistributionRuleCount>(),
            ],
            "items": false,
            "minItems": 4,
            "maxItems": 4,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalSupportLayers(Vec<SupportLayerId>);

impl CanonicalSupportLayers {
    fn from_observation(values: Vec<SupportLayerId>) -> Result<Self, DeliveryResultContractError> {
        if values.len() > MAX_DELIVERY_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(DeliveryResultContractError(
                "support layers must be bounded, unique, and canonically ordered",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportLayerId] {
        &self.0
    }
}

impl JsonSchema for CanonicalSupportLayers {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalSupportLayers".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 0,
            "maxItems": MAX_DELIVERY_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<SupportLayerId>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalLocalDifferences(Vec<RepositoryTargetIdentity>);

impl CanonicalLocalDifferences {
    fn from_observation(
        values: Vec<RepositoryTargetIdentity>,
    ) -> Result<Self, DeliveryResultContractError> {
        if values.len() > MAX_DELIVERY_ITEMS || values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(DeliveryResultContractError(
                "local differences must be bounded, unique, and canonically ordered",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[RepositoryTargetIdentity] {
        &self.0
    }
}

impl JsonSchema for CanonicalLocalDifferences {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalLocalDifferences".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 0,
            "maxItems": MAX_DELIVERY_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<RepositoryTargetIdentity>(),
        })
    }
}

/// One inspection adapter observation. It is intentionally non-`Clone` and
/// non-`Deserialize`; a later handler/adapter task will add its only production
/// minting surface.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DeliveryInspectionAuthority {
    repository_anchor: RepositoryAnchor,
    binding_matches: bool,
    main_equals_repository: bool,
    main_equals_database_configuration: bool,
    platform_version: PlatformVersion,
    compatibility_mode: CompatibilityMode,
    delivery_permissions: DeliveryPermissions,
    distribution_rule_counts: DistributionRuleCounts,
    support_layers: CanonicalSupportLayers,
    local_differences: CanonicalLocalDifferences,
}

impl DeliveryInspectionAuthority {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_test_only(
        repository_anchor: RepositoryAnchor,
        binding_matches: bool,
        main_equals_repository: bool,
        main_equals_database_configuration: bool,
        platform_version: PlatformVersion,
        compatibility_mode: CompatibilityMode,
        delivery_permissions: DeliveryPermissions,
        distribution_rule_counts: DistributionRuleCounts,
        support_layers: Vec<SupportLayerId>,
        local_differences: Vec<RepositoryTargetIdentity>,
    ) -> Result<Self, DeliveryResultContractError> {
        Ok(Self {
            repository_anchor,
            binding_matches,
            main_equals_repository,
            main_equals_database_configuration,
            platform_version,
            compatibility_mode,
            delivery_permissions,
            distribution_rule_counts,
            support_layers: CanonicalSupportLayers::from_observation(support_layers)?,
            local_differences: CanonicalLocalDifferences::from_observation(local_differences)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeliveryInspectionStatusDigestRecord {
    configuration_identity: ConfigurationIdentity,
    repository_identity: Sha256Digest,
    binding_matches: bool,
    main_equals_repository: bool,
    main_equals_database_configuration: bool,
    platform_version: PlatformVersion,
    compatibility_mode: CompatibilityMode,
    delivery_permissions: DeliveryPermissions,
    distribution_rule_counts: DistributionRuleCounts,
    support_layers: CanonicalSupportLayers,
    local_differences: CanonicalLocalDifferences,
    warnings_are_errors: TrueLiteral,
}

impl contract_digest_record_sealed::Sealed for DeliveryInspectionStatusDigestRecord {}
impl ContractDigestRecord for DeliveryInspectionStatusDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeliveryInspectionAnchorBindingDigestRecord {
    status_digest: Sha256Digest,
    anchor_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for DeliveryInspectionAnchorBindingDigestRecord {}
impl ContractDigestRecord for DeliveryInspectionAnchorBindingDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeliveryInspectionData {
    configuration_identity: ConfigurationIdentity,
    repository_identity: Sha256Digest,
    binding_matches: bool,
    main_equals_repository: bool,
    main_equals_database_configuration: bool,
    platform_version: PlatformVersion,
    compatibility_mode: CompatibilityMode,
    delivery_permissions: DeliveryPermissions,
    distribution_rule_counts: DistributionRuleCounts,
    support_layers: CanonicalSupportLayers,
    local_differences: CanonicalLocalDifferences,
    warnings_are_errors: TrueLiteral,
    status_digest: Sha256Digest,
}

impl DeliveryInspectionData {
    pub(crate) fn from_authority(
        authority: DeliveryInspectionAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        Ok(ValidatedDeliveryInspectionAuthority::from_authority(authority)?.into_data())
    }

    fn from_status_digest_record(
        record: DeliveryInspectionStatusDigestRecord,
    ) -> Result<Self, DeliveryResultContractError> {
        let status_digest = canonical_contract_digest(&record, None)
            .map_err(|_| DeliveryResultContractError("delivery inspection status digest failed"))?;
        Ok(Self {
            configuration_identity: record.configuration_identity,
            repository_identity: record.repository_identity,
            binding_matches: record.binding_matches,
            main_equals_repository: record.main_equals_repository,
            main_equals_database_configuration: record.main_equals_database_configuration,
            platform_version: record.platform_version,
            compatibility_mode: record.compatibility_mode,
            delivery_permissions: record.delivery_permissions,
            distribution_rule_counts: record.distribution_rule_counts,
            support_layers: record.support_layers,
            local_differences: record.local_differences,
            warnings_are_errors: record.warnings_are_errors,
            status_digest,
        })
    }

    fn status_digest_record(&self) -> DeliveryInspectionStatusDigestRecord {
        DeliveryInspectionStatusDigestRecord {
            configuration_identity: self.configuration_identity.clone(),
            repository_identity: self.repository_identity.clone(),
            binding_matches: self.binding_matches,
            main_equals_repository: self.main_equals_repository,
            main_equals_database_configuration: self.main_equals_database_configuration,
            platform_version: self.platform_version.clone(),
            compatibility_mode: self.compatibility_mode.clone(),
            delivery_permissions: self.delivery_permissions.clone(),
            distribution_rule_counts: self.distribution_rule_counts.clone(),
            support_layers: self.support_layers.clone(),
            local_differences: self.local_differences.clone(),
            warnings_are_errors: TrueLiteral,
        }
    }

    fn recomputed_status_digest(&self) -> Result<Sha256Digest, DeliveryResultContractError> {
        canonical_contract_digest(&self.status_digest_record(), None)
            .map_err(|_| DeliveryResultContractError("delivery inspection status digest failed"))
    }

    pub(crate) const fn configuration_identity(&self) -> &ConfigurationIdentity {
        &self.configuration_identity
    }

    pub(crate) const fn repository_identity(&self) -> &Sha256Digest {
        &self.repository_identity
    }

    pub(crate) const fn binding_matches(&self) -> bool {
        self.binding_matches
    }

    pub(crate) const fn main_equals_repository(&self) -> bool {
        self.main_equals_repository
    }

    pub(crate) const fn main_equals_database_configuration(&self) -> bool {
        self.main_equals_database_configuration
    }

    pub(crate) const fn platform_version(&self) -> &PlatformVersion {
        &self.platform_version
    }

    pub(crate) const fn compatibility_mode(&self) -> &CompatibilityMode {
        &self.compatibility_mode
    }

    pub(crate) const fn delivery_permissions(&self) -> &DeliveryPermissions {
        &self.delivery_permissions
    }

    pub(crate) const fn distribution_rule_counts(&self) -> &DistributionRuleCounts {
        &self.distribution_rule_counts
    }

    pub(crate) fn support_layers(&self) -> &[SupportLayerId] {
        self.support_layers.as_slice()
    }

    pub(crate) fn local_differences(&self) -> &[RepositoryTargetIdentity] {
        self.local_differences.as_slice()
    }

    pub(crate) const fn status_digest(&self) -> &Sha256Digest {
        &self.status_digest
    }
}

/// One inseparable clean-inspection projection and repository anchor. It is
/// deliberately non-`Clone`, non-`Serialize`, non-`JsonSchema`, and
/// non-`Deserialize`: create preview must consume the exact atomic observation
/// rather than pair a wire inspection with an independently supplied anchor.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedDeliveryInspectionAuthority {
    data: DeliveryInspectionData,
    repository_anchor: RepositoryAnchor,
    binding_digest: Sha256Digest,
}

impl ValidatedDeliveryInspectionAuthority {
    pub(crate) fn from_authority(
        authority: DeliveryInspectionAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        let DeliveryInspectionAuthority {
            repository_anchor,
            binding_matches,
            main_equals_repository,
            main_equals_database_configuration,
            platform_version,
            compatibility_mode,
            delivery_permissions,
            distribution_rule_counts,
            support_layers,
            local_differences,
        } = authority;
        let record = DeliveryInspectionStatusDigestRecord {
            configuration_identity: repository_anchor.configuration_identity().clone(),
            repository_identity: repository_anchor.repository_identity().clone(),
            binding_matches,
            main_equals_repository,
            main_equals_database_configuration,
            platform_version,
            compatibility_mode,
            delivery_permissions,
            distribution_rule_counts,
            support_layers,
            local_differences,
            warnings_are_errors: TrueLiteral,
        };
        let data = DeliveryInspectionData::from_status_digest_record(record)?;
        let binding_record = DeliveryInspectionAnchorBindingDigestRecord {
            status_digest: data.status_digest.clone(),
            anchor_digest: repository_anchor.anchor_digest().clone(),
        };
        let binding_digest = canonical_contract_digest(&binding_record, None).map_err(|_| {
            DeliveryResultContractError("delivery inspection anchor binding digest failed")
        })?;
        Ok(Self {
            data,
            repository_anchor,
            binding_digest,
        })
    }

    fn validate_binding(&self) -> Result<(), DeliveryResultContractError> {
        if self.data.recomputed_status_digest()? != self.data.status_digest {
            return Err(DeliveryResultContractError(
                "delivery inspection data status digest no longer matches its content",
            ));
        }
        if self.repository_anchor.repository_identity() != self.data.repository_identity()
            || self.repository_anchor.configuration_identity() != self.data.configuration_identity()
        {
            return Err(DeliveryResultContractError(
                "repository anchor does not match the inspected clean original",
            ));
        }
        let binding_record = DeliveryInspectionAnchorBindingDigestRecord {
            status_digest: self.data.status_digest.clone(),
            anchor_digest: self.repository_anchor.anchor_digest().clone(),
        };
        let observed = canonical_contract_digest(&binding_record, None).map_err(|_| {
            DeliveryResultContractError("delivery inspection anchor binding digest failed")
        })?;
        if observed != self.binding_digest {
            return Err(DeliveryResultContractError(
                "delivery inspection anchor binding digest mismatch",
            ));
        }
        Ok(())
    }

    fn into_data(self) -> DeliveryInspectionData {
        self.data
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum DistributionRole {
    BaselineDistribution,
    RefreshDistribution,
}

impl DistributionRole {
    #[cfg(test)]
    fn from_artifact_role(
        role: crate::domain::branched_development::contracts::artifacts::ArtifactRole,
    ) -> Result<Self, DeliveryResultContractError> {
        use crate::domain::branched_development::contracts::artifacts::ArtifactRole;
        match role {
            ArtifactRole::BaselineDistribution => Ok(Self::BaselineDistribution),
            ArtifactRole::RefreshDistribution => Ok(Self::RefreshDistribution),
            ArtifactRole::OrdinaryResult | ArtifactRole::SupportRecoveryDistribution => Err(
                DeliveryResultContractError("artifact role is not a delivery distribution role"),
            ),
        }
    }
}

/// One approved create observation projected from a fresh inspection and its
/// repository anchor. Raw minting remains test-only until a later handler task.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DistributionPreviewAuthority {
    role: DistributionRole,
    configuration_identity: ConfigurationIdentity,
    repository_anchor: RepositoryAnchor,
    platform_version: PlatformVersion,
    inspection_digest: Sha256Digest,
}

impl DistributionPreviewAuthority {
    #[cfg(test)]
    pub(crate) fn from_inspection_test_only(
        role: crate::domain::branched_development::contracts::artifacts::ArtifactRole,
        inspection: ValidatedDeliveryInspectionAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        inspection.validate_binding()?;
        let ValidatedDeliveryInspectionAuthority {
            data: inspection,
            repository_anchor,
            binding_digest: _,
        } = inspection;
        if !inspection.binding_matches()
            || !inspection.main_equals_repository()
            || !inspection.main_equals_database_configuration()
            || !inspection.delivery_permissions().distribution_allowed()
            || !inspection.local_differences().is_empty()
        {
            return Err(DeliveryResultContractError(
                "distribution creation requires one fresh clean delivery inspection",
            ));
        }
        Ok(Self {
            role: DistributionRole::from_artifact_role(role)?,
            configuration_identity: inspection.configuration_identity().clone(),
            repository_anchor,
            platform_version: inspection.platform_version().clone(),
            inspection_digest: inspection.status_digest().clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DistributionPreviewDigestRecord {
    role: DistributionRole,
    configuration_identity: ConfigurationIdentity,
    repository_anchor: RepositoryAnchor,
    platform_version: PlatformVersion,
    inspection_digest: Sha256Digest,
    planned_artifact_kind: ConfigurationDistributionKind,
}

impl contract_digest_record_sealed::Sealed for DistributionPreviewDigestRecord {}
impl ContractDigestRecord for DistributionPreviewDigestRecord {}

macro_rules! distribution_preview_leaf {
    ($name:ident, $role:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            role: $role,
            configuration_identity: ConfigurationIdentity,
            repository_anchor: RepositoryAnchor,
            platform_version: PlatformVersion,
            inspection_digest: Sha256Digest,
            planned_artifact_kind: ConfigurationDistributionKind,
            preview_digest: Sha256Digest,
        }
    };
}

distribution_preview_leaf!(BaselineDistributionPreviewData, BaselineDistributionRole);
distribution_preview_leaf!(RefreshDistributionPreviewData, RefreshDistributionRole);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum DistributionPreviewData {
    Baseline(BaselineDistributionPreviewData),
    Refresh(RefreshDistributionPreviewData),
}

impl JsonSchema for DistributionPreviewData {
    fn schema_name() -> Cow<'static, str> {
        "DistributionPreviewData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        super::super::schema::one_of_schema(vec![
            generator.subschema_for::<BaselineDistributionPreviewData>(),
            generator.subschema_for::<RefreshDistributionPreviewData>(),
        ])
    }
}

impl DistributionPreviewData {
    pub(crate) fn from_authority(
        authority: DistributionPreviewAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        let record = DistributionPreviewDigestRecord {
            role: authority.role,
            configuration_identity: authority.configuration_identity,
            repository_anchor: authority.repository_anchor,
            platform_version: authority.platform_version,
            inspection_digest: authority.inspection_digest,
            planned_artifact_kind: ConfigurationDistributionKind::Value,
        };
        let preview_digest = canonical_contract_digest(&record, None)
            .map_err(|_| DeliveryResultContractError("distribution preview digest failed"))?;
        Ok(match record.role {
            DistributionRole::BaselineDistribution => {
                Self::Baseline(BaselineDistributionPreviewData {
                    role: BaselineDistributionRole::Value,
                    configuration_identity: record.configuration_identity,
                    repository_anchor: record.repository_anchor,
                    platform_version: record.platform_version,
                    inspection_digest: record.inspection_digest,
                    planned_artifact_kind: record.planned_artifact_kind,
                    preview_digest,
                })
            }
            DistributionRole::RefreshDistribution => {
                Self::Refresh(RefreshDistributionPreviewData {
                    role: RefreshDistributionRole::Value,
                    configuration_identity: record.configuration_identity,
                    repository_anchor: record.repository_anchor,
                    platform_version: record.platform_version,
                    inspection_digest: record.inspection_digest,
                    planned_artifact_kind: record.planned_artifact_kind,
                    preview_digest,
                })
            }
        })
    }

    pub(crate) const fn is_baseline(&self) -> bool {
        matches!(self, Self::Baseline(_))
    }

    pub(crate) const fn configuration_identity(&self) -> &ConfigurationIdentity {
        match self {
            Self::Baseline(value) => &value.configuration_identity,
            Self::Refresh(value) => &value.configuration_identity,
        }
    }

    pub(crate) const fn repository_anchor(&self) -> &RepositoryAnchor {
        match self {
            Self::Baseline(value) => &value.repository_anchor,
            Self::Refresh(value) => &value.repository_anchor,
        }
    }

    pub(crate) const fn platform_version(&self) -> &PlatformVersion {
        match self {
            Self::Baseline(value) => &value.platform_version,
            Self::Refresh(value) => &value.platform_version,
        }
    }

    pub(crate) const fn inspection_digest(&self) -> &Sha256Digest {
        match self {
            Self::Baseline(value) => &value.inspection_digest,
            Self::Refresh(value) => &value.inspection_digest,
        }
    }

    pub(crate) const fn preview_digest(&self) -> &Sha256Digest {
        match self {
            Self::Baseline(value) => &value.preview_digest,
            Self::Refresh(value) => &value.preview_digest,
        }
    }
}

/// Artifact-writer observation. `sha256` is the exact artifact-file byte hash,
/// never a JSON contract digest.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DistributionArtifactObservationAuthority {
    artifact_id: UnicaId,
    sha256: Sha256Digest,
    created_at: NormalizedUtcInstant,
    approved_preview_digest: Sha256Digest,
}

impl DistributionArtifactObservationAuthority {
    #[cfg(test)]
    pub(crate) fn from_writer_test_only(
        approval: &ApprovedDistributionPreviewAuthority,
        artifact_id: UnicaId,
        sha256: Sha256Digest,
        created_at: NormalizedUtcInstant,
    ) -> Self {
        Self {
            artifact_id,
            sha256,
            created_at,
            approved_preview_digest: approval.preview.preview_digest().clone(),
        }
    }
}

macro_rules! distribution_data_leaf {
    ($name:ident, $role:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            artifact_id: UnicaId,
            role: $role,
            kind: ConfigurationDistributionKind,
            sha256: Sha256Digest,
            configuration_identity: ConfigurationIdentity,
            repository_anchor: RepositoryAnchor,
            platform_version: PlatformVersion,
            created_at: NormalizedUtcInstant,
            preview_digest: Sha256Digest,
        }
    };
}

distribution_data_leaf!(BaselineDistributionData, BaselineDistributionRole);
distribution_data_leaf!(RefreshDistributionData, RefreshDistributionRole);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum DistributionData {
    Baseline(BaselineDistributionData),
    Refresh(RefreshDistributionData),
}

/// CAS-backed approval for exactly one distribution apply. The authority is
/// consumed by `DistributionData`, so cloning a wire preview cannot authorize
/// repeated effects.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedDistributionPreviewAuthority {
    preview: DistributionPreviewData,
}

impl ApprovedDistributionPreviewAuthority {
    #[cfg(test)]
    pub(crate) fn approve_test_only(preview: &DistributionPreviewData) -> Self {
        Self {
            preview: preview.clone(),
        }
    }
}

impl JsonSchema for DistributionData {
    fn schema_name() -> Cow<'static, str> {
        "DistributionData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        super::super::schema::one_of_schema(vec![
            generator.subschema_for::<BaselineDistributionData>(),
            generator.subschema_for::<RefreshDistributionData>(),
        ])
    }
}

impl DistributionData {
    pub(crate) fn from_approved_preview(
        approval: ApprovedDistributionPreviewAuthority,
        observation: DistributionArtifactObservationAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        if &observation.approved_preview_digest != approval.preview.preview_digest() {
            return Err(DeliveryResultContractError(
                "artifact-writer observation belongs to another approved distribution preview",
            ));
        }
        Ok(match approval.preview {
            DistributionPreviewData::Baseline(value) => Self::Baseline(BaselineDistributionData {
                artifact_id: observation.artifact_id,
                role: BaselineDistributionRole::Value,
                kind: ConfigurationDistributionKind::Value,
                sha256: observation.sha256,
                configuration_identity: value.configuration_identity,
                repository_anchor: value.repository_anchor,
                platform_version: value.platform_version,
                created_at: observation.created_at,
                preview_digest: value.preview_digest,
            }),
            DistributionPreviewData::Refresh(value) => Self::Refresh(RefreshDistributionData {
                artifact_id: observation.artifact_id,
                role: RefreshDistributionRole::Value,
                kind: ConfigurationDistributionKind::Value,
                sha256: observation.sha256,
                configuration_identity: value.configuration_identity,
                repository_anchor: value.repository_anchor,
                platform_version: value.platform_version,
                created_at: observation.created_at,
                preview_digest: value.preview_digest,
            }),
        })
    }

    pub(crate) const fn is_baseline(&self) -> bool {
        matches!(self, Self::Baseline(_))
    }

    pub(crate) const fn artifact_id(&self) -> &UnicaId {
        match self {
            Self::Baseline(value) => &value.artifact_id,
            Self::Refresh(value) => &value.artifact_id,
        }
    }

    pub(crate) const fn sha256(&self) -> &Sha256Digest {
        match self {
            Self::Baseline(value) => &value.sha256,
            Self::Refresh(value) => &value.sha256,
        }
    }

    pub(crate) const fn configuration_identity(&self) -> &ConfigurationIdentity {
        match self {
            Self::Baseline(value) => &value.configuration_identity,
            Self::Refresh(value) => &value.configuration_identity,
        }
    }

    pub(crate) const fn repository_anchor(&self) -> &RepositoryAnchor {
        match self {
            Self::Baseline(value) => &value.repository_anchor,
            Self::Refresh(value) => &value.repository_anchor,
        }
    }

    pub(crate) const fn platform_version(&self) -> &PlatformVersion {
        match self {
            Self::Baseline(value) => &value.platform_version,
            Self::Refresh(value) => &value.platform_version,
        }
    }

    pub(crate) const fn created_at(&self) -> &NormalizedUtcInstant {
        match self {
            Self::Baseline(value) => &value.created_at,
            Self::Refresh(value) => &value.created_at,
        }
    }

    pub(crate) const fn preview_digest(&self) -> &Sha256Digest {
        match self {
            Self::Baseline(value) => &value.preview_digest,
            Self::Refresh(value) => &value.preview_digest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ArtifactVerificationDiagnostics(Vec<Diagnostic>);

impl ArtifactVerificationDiagnostics {
    fn from_probe(values: Vec<Diagnostic>) -> Result<Self, DeliveryResultContractError> {
        if values.len() > MAX_DELIVERY_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].as_str().as_bytes() >= pair[1].as_str().as_bytes())
        {
            return Err(DeliveryResultContractError(
                "verification diagnostics must be bounded and strictly ordered by UTF-8 bytes",
            ));
        }
        Ok(Self(values))
    }

    #[cfg(test)]
    pub(crate) fn new_test_only(
        values: Vec<Diagnostic>,
    ) -> Result<Self, DeliveryResultContractError> {
        Self::from_probe(values)
    }

    pub(crate) fn as_slice(&self) -> &[Diagnostic] {
        &self.0
    }
}

impl JsonSchema for ArtifactVerificationDiagnostics {
    fn schema_name() -> Cow<'static, str> {
        "ArtifactVerificationDiagnostics".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 0,
            "maxItems": MAX_DELIVERY_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<Diagnostic>(),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AcceptedProbeKindAuthority {
    ConfigurationDistribution {
        support_identity: ConfigurationIdentity,
    },
    OrdinaryConfiguration,
}

impl AcceptedProbeKindAuthority {
    const fn accepted_kind(&self) -> AcceptedArtifactKind {
        match self {
            Self::ConfigurationDistribution { .. } => {
                AcceptedArtifactKind::ConfigurationDistribution
            }
            Self::OrdinaryConfiguration => AcceptedArtifactKind::OrdinaryConfiguration,
        }
    }
}

/// One strict probe observation. Unaccepted classifications and a distribution
/// that does not equal its vendor support identity cannot mint this authority.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArtifactProbeObservationAuthority {
    verification_id: UnicaId,
    artifact_id: UnicaId,
    kind: AcceptedProbeKindAuthority,
    sha256: Sha256Digest,
    probe_id: UnicaId,
    diagnostics: ArtifactVerificationDiagnostics,
}

impl ArtifactProbeObservationAuthority {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_probe_test_only(
        verification_id: UnicaId,
        artifact_id: UnicaId,
        kind: crate::domain::branched_development::contracts::artifacts::ArtifactKind,
        sha256: Sha256Digest,
        probe_id: UnicaId,
        support_identity: Option<ConfigurationIdentity>,
        current_equals_vendor: Option<bool>,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Self, DeliveryResultContractError> {
        use crate::domain::branched_development::contracts::artifacts::ArtifactKind;

        let kind = match (kind, support_identity, current_equals_vendor) {
            (ArtifactKind::ConfigurationDistribution, Some(support_identity), Some(true)) => {
                AcceptedProbeKindAuthority::ConfigurationDistribution { support_identity }
            }
            (ArtifactKind::OrdinaryConfiguration, None, None) => {
                AcceptedProbeKindAuthority::OrdinaryConfiguration
            }
            (ArtifactKind::ConfigurationUpdate | ArtifactKind::InvalidArtifact, _, _) => {
                return Err(DeliveryResultContractError(
                    "unaccepted artifact kind cannot publish a completed verification",
                ));
            }
            (ArtifactKind::ConfigurationDistribution, _, _) => {
                return Err(DeliveryResultContractError(
                    "distribution verification requires current vendor equality and support identity",
                ));
            }
            (ArtifactKind::OrdinaryConfiguration, _, _) => {
                return Err(DeliveryResultContractError(
                    "ordinary verification forbids distribution support fields",
                ));
            }
        };
        Ok(Self {
            verification_id,
            artifact_id,
            kind,
            sha256,
            probe_id,
            diagnostics: ArtifactVerificationDiagnostics::from_probe(diagnostics)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtifactVerificationDiagnosticsDigestRecord {
    artifact_id: UnicaId,
    probe_id: UnicaId,
    kind: AcceptedArtifactKind,
    diagnostics: ArtifactVerificationDiagnostics,
}

impl contract_digest_record_sealed::Sealed for ArtifactVerificationDiagnosticsDigestRecord {}
impl ContractDigestRecord for ArtifactVerificationDiagnosticsDigestRecord {}

macro_rules! unconstrained_verification_leaf {
    ($name:ident, $kind:ty $(, $support_field:ident : $support_type:ty)*) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            verification_id: UnicaId,
            artifact_id: UnicaId,
            kind: $kind,
            expectation_matched: TrueLiteral,
            sha256: Sha256Digest,
            probe_id: UnicaId,
            $($support_field: $support_type,)*
            diagnostics_digest: Sha256Digest,
        }
    };
}

macro_rules! expected_verification_leaf {
    ($name:ident, $kind:ty $(, $support_field:ident : $support_type:ty)*) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            verification_id: UnicaId,
            artifact_id: UnicaId,
            kind: $kind,
            expected_kind: $kind,
            expectation_matched: TrueLiteral,
            sha256: Sha256Digest,
            probe_id: UnicaId,
            $($support_field: $support_type,)*
            diagnostics_digest: Sha256Digest,
        }
    };
}

unconstrained_verification_leaf!(
    UnconstrainedDistributionArtifactVerificationData,
    ConfigurationDistributionKind,
    support_identity: ConfigurationIdentity,
    current_equals_vendor: TrueLiteral
);
expected_verification_leaf!(
    ExpectedDistributionArtifactVerificationData,
    ConfigurationDistributionKind,
    support_identity: ConfigurationIdentity,
    current_equals_vendor: TrueLiteral
);
unconstrained_verification_leaf!(
    UnconstrainedOrdinaryArtifactVerificationData,
    OrdinaryConfigurationKind
);
expected_verification_leaf!(
    ExpectedOrdinaryArtifactVerificationData,
    OrdinaryConfigurationKind
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ArtifactVerificationData {
    UnconstrainedDistribution(UnconstrainedDistributionArtifactVerificationData),
    ExpectedDistribution(ExpectedDistributionArtifactVerificationData),
    UnconstrainedOrdinary(UnconstrainedOrdinaryArtifactVerificationData),
    ExpectedOrdinary(ExpectedOrdinaryArtifactVerificationData),
}

impl JsonSchema for ArtifactVerificationData {
    fn schema_name() -> Cow<'static, str> {
        "ArtifactVerificationData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        super::super::schema::one_of_schema(vec![
            generator.subschema_for::<UnconstrainedDistributionArtifactVerificationData>(),
            generator.subschema_for::<ExpectedDistributionArtifactVerificationData>(),
            generator.subschema_for::<UnconstrainedOrdinaryArtifactVerificationData>(),
            generator.subschema_for::<ExpectedOrdinaryArtifactVerificationData>(),
        ])
    }
}

impl ArtifactVerificationData {
    pub(crate) fn unconstrained(
        observation: ArtifactProbeObservationAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        Self::from_observation(observation, None)
    }

    pub(crate) fn expected(
        observation: ArtifactProbeObservationAuthority,
        expected_kind: AcceptedArtifactKind,
    ) -> Result<Self, DeliveryResultContractError> {
        Self::from_observation(observation, Some(expected_kind))
    }

    fn from_observation(
        observation: ArtifactProbeObservationAuthority,
        expected_kind: Option<AcceptedArtifactKind>,
    ) -> Result<Self, DeliveryResultContractError> {
        let accepted_kind = observation.kind.accepted_kind();
        if expected_kind.is_some_and(|expected| expected != accepted_kind) {
            return Err(DeliveryResultContractError(
                "explicit expected artifact kind does not match the strict probe",
            ));
        }
        let record = ArtifactVerificationDiagnosticsDigestRecord {
            artifact_id: observation.artifact_id.clone(),
            probe_id: observation.probe_id.clone(),
            kind: accepted_kind,
            diagnostics: observation.diagnostics,
        };
        let diagnostics_digest = canonical_contract_digest(&record, None).map_err(|_| {
            DeliveryResultContractError("artifact verification diagnostics digest failed")
        })?;
        Ok(match (observation.kind, expected_kind) {
            (AcceptedProbeKindAuthority::ConfigurationDistribution { support_identity }, None) => {
                Self::UnconstrainedDistribution(UnconstrainedDistributionArtifactVerificationData {
                    verification_id: observation.verification_id,
                    artifact_id: observation.artifact_id,
                    kind: ConfigurationDistributionKind::Value,
                    expectation_matched: TrueLiteral,
                    sha256: observation.sha256,
                    probe_id: observation.probe_id,
                    support_identity,
                    current_equals_vendor: TrueLiteral,
                    diagnostics_digest,
                })
            }
            (
                AcceptedProbeKindAuthority::ConfigurationDistribution { support_identity },
                Some(AcceptedArtifactKind::ConfigurationDistribution),
            ) => Self::ExpectedDistribution(ExpectedDistributionArtifactVerificationData {
                verification_id: observation.verification_id,
                artifact_id: observation.artifact_id,
                kind: ConfigurationDistributionKind::Value,
                expected_kind: ConfigurationDistributionKind::Value,
                expectation_matched: TrueLiteral,
                sha256: observation.sha256,
                probe_id: observation.probe_id,
                support_identity,
                current_equals_vendor: TrueLiteral,
                diagnostics_digest,
            }),
            (AcceptedProbeKindAuthority::OrdinaryConfiguration, None) => {
                Self::UnconstrainedOrdinary(UnconstrainedOrdinaryArtifactVerificationData {
                    verification_id: observation.verification_id,
                    artifact_id: observation.artifact_id,
                    kind: OrdinaryConfigurationKind::Value,
                    expectation_matched: TrueLiteral,
                    sha256: observation.sha256,
                    probe_id: observation.probe_id,
                    diagnostics_digest,
                })
            }
            (
                AcceptedProbeKindAuthority::OrdinaryConfiguration,
                Some(AcceptedArtifactKind::OrdinaryConfiguration),
            ) => Self::ExpectedOrdinary(ExpectedOrdinaryArtifactVerificationData {
                verification_id: observation.verification_id,
                artifact_id: observation.artifact_id,
                kind: OrdinaryConfigurationKind::Value,
                expected_kind: OrdinaryConfigurationKind::Value,
                expectation_matched: TrueLiteral,
                sha256: observation.sha256,
                probe_id: observation.probe_id,
                diagnostics_digest,
            }),
            (
                AcceptedProbeKindAuthority::ConfigurationDistribution { .. },
                Some(AcceptedArtifactKind::OrdinaryConfiguration),
            )
            | (
                AcceptedProbeKindAuthority::OrdinaryConfiguration,
                Some(AcceptedArtifactKind::ConfigurationDistribution),
            ) => unreachable!("mismatched expectation was rejected before result construction"),
        })
    }

    pub(crate) const fn verification_id(&self) -> &UnicaId {
        match self {
            Self::UnconstrainedDistribution(value) => &value.verification_id,
            Self::ExpectedDistribution(value) => &value.verification_id,
            Self::UnconstrainedOrdinary(value) => &value.verification_id,
            Self::ExpectedOrdinary(value) => &value.verification_id,
        }
    }

    pub(crate) const fn artifact_id(&self) -> &UnicaId {
        match self {
            Self::UnconstrainedDistribution(value) => &value.artifact_id,
            Self::ExpectedDistribution(value) => &value.artifact_id,
            Self::UnconstrainedOrdinary(value) => &value.artifact_id,
            Self::ExpectedOrdinary(value) => &value.artifact_id,
        }
    }

    pub(crate) const fn accepted_kind(&self) -> AcceptedArtifactKind {
        match self {
            Self::UnconstrainedDistribution(_) | Self::ExpectedDistribution(_) => {
                AcceptedArtifactKind::ConfigurationDistribution
            }
            Self::UnconstrainedOrdinary(_) | Self::ExpectedOrdinary(_) => {
                AcceptedArtifactKind::OrdinaryConfiguration
            }
        }
    }

    pub(crate) const fn sha256(&self) -> &Sha256Digest {
        match self {
            Self::UnconstrainedDistribution(value) => &value.sha256,
            Self::ExpectedDistribution(value) => &value.sha256,
            Self::UnconstrainedOrdinary(value) => &value.sha256,
            Self::ExpectedOrdinary(value) => &value.sha256,
        }
    }

    pub(crate) const fn support_identity(&self) -> Option<&ConfigurationIdentity> {
        match self {
            Self::UnconstrainedDistribution(value) => Some(&value.support_identity),
            Self::ExpectedDistribution(value) => Some(&value.support_identity),
            Self::UnconstrainedOrdinary(_) | Self::ExpectedOrdinary(_) => None,
        }
    }

    pub(crate) const fn diagnostics_digest(&self) -> &Sha256Digest {
        match self {
            Self::UnconstrainedDistribution(value) => &value.diagnostics_digest,
            Self::ExpectedDistribution(value) => &value.diagnostics_digest,
            Self::UnconstrainedOrdinary(value) => &value.diagnostics_digest,
            Self::ExpectedOrdinary(value) => &value.diagnostics_digest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum DeploymentPlannedRole {
    #[serde(rename = "taskInfobase")]
    TaskInfobase,
    #[serde(rename = "taskWorkspace")]
    TaskWorkspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct DeploymentPlannedRoles([DeploymentPlannedRole; 2]);

impl DeploymentPlannedRoles {
    const fn canonical() -> Self {
        Self([
            DeploymentPlannedRole::TaskInfobase,
            DeploymentPlannedRole::TaskWorkspace,
        ])
    }
}

impl JsonSchema for DeploymentPlannedRoles {
    fn schema_name() -> Cow<'static, str> {
        "DeploymentPlannedRoles".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [
                { "type": "string", "const": "taskInfobase" },
                { "type": "string", "const": "taskWorkspace" },
            ],
            "items": false,
            "minItems": 2,
            "maxItems": 2,
        })
    }
}

/// Registry-backed selection of one verified baseline distribution. The raw
/// fixture mint is test-only; a later handler/CAS task owns production minting.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifiedBaselineDistributionAuthority {
    distribution_id: UnicaId,
    distribution_sha256: Sha256Digest,
    vendor_identity: ConfigurationIdentity,
    source_fingerprint: Sha256Digest,
}

impl VerifiedBaselineDistributionAuthority {
    #[cfg(test)]
    pub(crate) fn from_results_test_only(
        distribution: &DistributionData,
        verification: &ArtifactVerificationData,
    ) -> Result<Self, DeliveryResultContractError> {
        if !distribution.is_baseline() {
            return Err(DeliveryResultContractError(
                "only a baseline distribution can be deployed",
            ));
        }
        if verification.accepted_kind() != AcceptedArtifactKind::ConfigurationDistribution
            || verification.artifact_id() != distribution.artifact_id()
            || verification.sha256() != distribution.sha256()
        {
            return Err(DeliveryResultContractError(
                "distribution verification does not match the selected artifact",
            ));
        }
        let vendor_identity =
            verification
                .support_identity()
                .ok_or(DeliveryResultContractError(
                    "distribution verification lacks support identity",
                ))?;
        if vendor_identity != distribution.configuration_identity() {
            return Err(DeliveryResultContractError(
                "verified support identity differs from distribution configuration identity",
            ));
        }
        Ok(Self {
            distribution_id: distribution.artifact_id().clone(),
            distribution_sha256: distribution.sha256().clone(),
            vendor_identity: vendor_identity.clone(),
            source_fingerprint: distribution
                .repository_anchor()
                .configuration_fingerprint()
                .clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeploymentPreviewDigestRecord {
    distribution_id: UnicaId,
    distribution_sha256: Sha256Digest,
    destination_kind: OwnedTaskInstanceDestination,
    planned_roles: DeploymentPlannedRoles,
}

impl contract_digest_record_sealed::Sealed for DeploymentPreviewDigestRecord {}
impl ContractDigestRecord for DeploymentPreviewDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeploymentPreviewData {
    distribution_id: UnicaId,
    distribution_sha256: Sha256Digest,
    destination_kind: OwnedTaskInstanceDestination,
    planned_roles: DeploymentPlannedRoles,
    preview_digest: Sha256Digest,
}

impl DeploymentPreviewData {
    pub(crate) fn from_verified_distribution(
        authority: &VerifiedBaselineDistributionAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        let record = DeploymentPreviewDigestRecord {
            distribution_id: authority.distribution_id.clone(),
            distribution_sha256: authority.distribution_sha256.clone(),
            destination_kind: OwnedTaskInstanceDestination::Value,
            planned_roles: DeploymentPlannedRoles::canonical(),
        };
        let preview_digest = canonical_contract_digest(&record, None)
            .map_err(|_| DeliveryResultContractError("deployment preview digest failed"))?;
        Ok(Self {
            distribution_id: record.distribution_id,
            distribution_sha256: record.distribution_sha256,
            destination_kind: record.destination_kind,
            planned_roles: record.planned_roles,
            preview_digest,
        })
    }

    pub(crate) const fn distribution_id(&self) -> &UnicaId {
        &self.distribution_id
    }

    pub(crate) const fn distribution_sha256(&self) -> &Sha256Digest {
        &self.distribution_sha256
    }

    pub(crate) const fn preview_digest(&self) -> &Sha256Digest {
        &self.preview_digest
    }
}

/// One CAS-approved deployment apply. It consumes the selected-distribution
/// authority, preventing a cloned preview from authorizing repeated effects.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ApprovedDeploymentPreviewAuthority {
    distribution_id: UnicaId,
    vendor_identity: ConfigurationIdentity,
    source_fingerprint: Sha256Digest,
    preview_digest: Sha256Digest,
}

impl ApprovedDeploymentPreviewAuthority {
    #[cfg(test)]
    pub(crate) fn approve_test_only(
        preview: &DeploymentPreviewData,
        selected: VerifiedBaselineDistributionAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        if preview.distribution_id != selected.distribution_id
            || preview.distribution_sha256 != selected.distribution_sha256
        {
            return Err(DeliveryResultContractError(
                "deployment preview does not match selected distribution authority",
            ));
        }
        Ok(Self {
            distribution_id: selected.distribution_id,
            vendor_identity: selected.vendor_identity,
            source_fingerprint: selected.source_fingerprint,
            preview_digest: preview.preview_digest.clone(),
        })
    }
}

/// Post-deploy adapter observation. Equality is validated before a completed
/// deployment result can be constructed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DeploymentObservationAuthority {
    task_infobase_id: UnicaId,
    task_workspace_id: UnicaId,
    current_fingerprint: Sha256Digest,
    vendor_fingerprint: Sha256Digest,
    approved_preview_digest: Sha256Digest,
}

impl DeploymentObservationAuthority {
    #[cfg(test)]
    pub(crate) fn from_deployment_test_only(
        approval: &ApprovedDeploymentPreviewAuthority,
        task_infobase_id: UnicaId,
        task_workspace_id: UnicaId,
        current_fingerprint: Sha256Digest,
        vendor_fingerprint: Sha256Digest,
    ) -> Result<Self, DeliveryResultContractError> {
        if task_infobase_id == task_workspace_id {
            return Err(DeliveryResultContractError(
                "task infobase and workspace IDs must be distinct",
            ));
        }
        if current_fingerprint != vendor_fingerprint {
            return Err(DeliveryResultContractError(
                "deployed current fingerprint must equal vendor fingerprint",
            ));
        }
        Ok(Self {
            task_infobase_id,
            task_workspace_id,
            current_fingerprint,
            vendor_fingerprint,
            approved_preview_digest: approval.preview_digest.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeploymentData {
    task_infobase_id: UnicaId,
    task_workspace_id: UnicaId,
    distribution_id: UnicaId,
    vendor_identity: ConfigurationIdentity,
    current_fingerprint: Sha256Digest,
    vendor_fingerprint: Sha256Digest,
    current_equals_vendor: TrueLiteral,
    source_fingerprint: Sha256Digest,
    preview_digest: Sha256Digest,
}

impl DeploymentData {
    pub(crate) fn from_approved_preview(
        approval: ApprovedDeploymentPreviewAuthority,
        observation: DeploymentObservationAuthority,
    ) -> Result<Self, DeliveryResultContractError> {
        if observation.approved_preview_digest != approval.preview_digest {
            return Err(DeliveryResultContractError(
                "deployment observation belongs to another approved preview",
            ));
        }
        if observation.current_fingerprint != observation.vendor_fingerprint {
            return Err(DeliveryResultContractError(
                "deployment observation lost current/vendor equality",
            ));
        }
        Ok(Self {
            task_infobase_id: observation.task_infobase_id,
            task_workspace_id: observation.task_workspace_id,
            distribution_id: approval.distribution_id,
            vendor_identity: approval.vendor_identity,
            current_fingerprint: observation.current_fingerprint,
            vendor_fingerprint: observation.vendor_fingerprint,
            current_equals_vendor: TrueLiteral,
            source_fingerprint: approval.source_fingerprint,
            preview_digest: approval.preview_digest,
        })
    }

    pub(crate) const fn task_infobase_id(&self) -> &UnicaId {
        &self.task_infobase_id
    }

    pub(crate) const fn task_workspace_id(&self) -> &UnicaId {
        &self.task_workspace_id
    }

    pub(crate) const fn distribution_id(&self) -> &UnicaId {
        &self.distribution_id
    }

    pub(crate) const fn preview_digest(&self) -> &Sha256Digest {
        &self.preview_digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::artifacts::{
        ArtifactRole, CompatibilityMode, ConfigurationIdentity, PlatformVersion, SafeResultCount,
    };
    use crate::domain::branched_development::contracts::repository::{
        RepositoryAnchor, RepositoryAnchorObservationAuthority, RepositoryHistoryCursor,
        RepositoryTargetIdentity,
    };
    use crate::domain::branched_development::contracts::scalars::{
        EmptyOrName, Name, NormalizedUtcInstant, RepositoryVersion,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::{
        MetadataObjectId, Sha256Digest, SupportLayerId, UnicaId,
    };
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const UUID_A: &str = "123e4567-e89b-12d3-a456-426614174000";
    const UUID_B: &str = "123e4567-e89b-12d3-a456-426614174001";

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn configuration_identity() -> ConfigurationIdentity {
        ConfigurationIdentity::new(
            MetadataObjectId::parse(UUID_A).unwrap(),
            Name::parse("Example").unwrap(),
            EmptyOrName::parse("Vendor").unwrap(),
            EmptyOrName::parse("1.0").unwrap(),
        )
    }

    fn object_target() -> RepositoryTargetIdentity {
        serde_json::from_value(json!({
            "targetKind": "developmentObject",
            "objectId": UUID_B,
        }))
        .unwrap()
    }

    fn rule_counts() -> DistributionRuleCounts {
        DistributionRuleCounts::new_test_only(
            SafeResultCount::new(0).unwrap(),
            SafeResultCount::new(1).unwrap(),
            SafeResultCount::new(2).unwrap(),
            SafeResultCount::new(0).unwrap(),
        )
    }

    fn inspection_authority() -> DeliveryInspectionAuthority {
        DeliveryInspectionAuthority::new_test_only(
            repository_anchor(digest('a')),
            true,
            true,
            true,
            PlatformVersion::parse("8.3.27.1000").unwrap(),
            CompatibilityMode::parse("Version8_3_24").unwrap(),
            DeliveryPermissions::new_test_only(true, false),
            rule_counts(),
            vec![
                SupportLayerId::parse("base").unwrap(),
                SupportLayerId::parse("Вендор").unwrap(),
            ],
            vec![
                serde_json::from_value(json!({ "targetKind": "configurationRoot" })).unwrap(),
                object_target(),
            ],
        )
        .unwrap()
    }

    fn repository_anchor_with(
        repository_identity: Sha256Digest,
        repository_version: &str,
        history_prefix_digest: Sha256Digest,
        configuration_fingerprint: Sha256Digest,
    ) -> RepositoryAnchor {
        RepositoryAnchorObservationAuthority::test_only(
            repository_identity,
            RepositoryHistoryCursor::new(
                RepositoryVersion::parse(repository_version).unwrap(),
                history_prefix_digest,
            ),
            configuration_identity(),
            configuration_fingerprint,
        )
        .into_anchor()
        .unwrap()
    }

    fn repository_anchor(repository_identity: Sha256Digest) -> RepositoryAnchor {
        repository_anchor_with(repository_identity, "42", digest('b'), digest('c'))
    }

    fn clean_inspection_observation(
        repository_anchor: RepositoryAnchor,
    ) -> DeliveryInspectionAuthority {
        DeliveryInspectionAuthority::new_test_only(
            repository_anchor,
            true,
            true,
            true,
            PlatformVersion::parse("8.3.27.1000").unwrap(),
            CompatibilityMode::parse("Version8_3_24").unwrap(),
            DeliveryPermissions::new_test_only(true, false),
            rule_counts(),
            vec![],
            vec![],
        )
        .unwrap()
    }

    fn validated_clean_inspection_with(
        repository_anchor: RepositoryAnchor,
    ) -> ValidatedDeliveryInspectionAuthority {
        ValidatedDeliveryInspectionAuthority::from_authority(clean_inspection_observation(
            repository_anchor,
        ))
        .unwrap()
    }

    fn validated_clean_inspection() -> ValidatedDeliveryInspectionAuthority {
        validated_clean_inspection_with(repository_anchor(digest('a')))
    }

    fn clean_inspection() -> DeliveryInspectionData {
        validated_clean_inspection().into_data()
    }

    fn distribution_preview(role: ArtifactRole) -> DistributionPreviewData {
        let authority = DistributionPreviewAuthority::from_inspection_test_only(
            role,
            validated_clean_inspection(),
        )
        .unwrap();
        DistributionPreviewData::from_authority(authority).unwrap()
    }

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).unwrap()
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema::<T>())
            .unwrap()
            .is_valid(value)
    }

    fn collect_object_pointers(value: &Value, pointer: String, output: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                output.push(pointer.clone());
                for (key, nested) in object {
                    let key = key.replace('~', "~0").replace('/', "~1");
                    collect_object_pointers(nested, format!("{pointer}/{key}"), output);
                }
            }
            Value::Array(values) => {
                for (index, nested) in values.iter().enumerate() {
                    collect_object_pointers(nested, format!("{pointer}/{index}"), output);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    fn assert_recursively_closed<T: JsonSchema>(valid: Value) {
        assert!(
            schema_accepts::<T>(&valid),
            "invalid positive fixture: {valid}"
        );
        audit_json_schema(&schema::<T>()).unwrap();
        let mut pointers = Vec::new();
        collect_object_pointers(&valid, String::new(), &mut pointers);
        for pointer in pointers {
            let required_fields: Vec<_> = valid
                .pointer(&pointer)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            for required_field in required_fields {
                let mut invalid = valid.clone();
                invalid
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(&required_field);
                assert!(
                    !schema_accepts::<T>(&invalid),
                    "{} accepted omitted required field {required_field} at {pointer}",
                    T::schema_name()
                );
            }
            for forbidden in [
                "cwd",
                "localPath",
                "stateRoot",
                "workRoot",
                "pid",
                "processHandle",
                "password",
                "token",
                "secret",
                "credentialRef",
                "credentialReference",
                "rawConnection",
                "rawConnectionString",
                "serviceEndpoint",
            ] {
                let mut invalid = valid.clone();
                invalid
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(forbidden.to_owned(), json!("forbidden"));
                assert!(
                    !schema_accepts::<T>(&invalid),
                    "{} accepted {forbidden} at {pointer}",
                    T::schema_name()
                );
            }
        }
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize> for T {}
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn assert_not_clone() {}
                }
                struct ImplementsClone;
                impl<T: ?Sized> AmbiguousIfClone<()> for T {}
                impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    macro_rules! assert_not_serialize {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfSerialize<Marker> {
                    fn assert_not_serialize() {}
                }
                struct ImplementsSerialize;
                impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
                impl<T: serde::Serialize> AmbiguousIfSerialize<ImplementsSerialize> for T {}
                let _ = <$type as AmbiguousIfSerialize<_>>::assert_not_serialize;
            };
        };
    }

    macro_rules! assert_not_json_schema {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfJsonSchema<Marker> {
                    fn assert_not_json_schema() {}
                }
                struct ImplementsJsonSchema;
                impl<T: ?Sized> AmbiguousIfJsonSchema<()> for T {}
                impl<T: JsonSchema> AmbiguousIfJsonSchema<ImplementsJsonSchema> for T {}
                let _ = <$type as AmbiguousIfJsonSchema<_>>::assert_not_json_schema;
            };
        };
    }

    assert_not_deserialize_owned!(DeliveryInspectionAuthority);
    assert_not_deserialize_owned!(ValidatedDeliveryInspectionAuthority);
    assert_not_deserialize_owned!(DeliveryPermissions);
    assert_not_deserialize_owned!(DeliveryInspectionData);
    assert_not_deserialize_owned!(DeliveryInspectionStatusDigestRecord);
    assert_not_deserialize_owned!(DeliveryInspectionAnchorBindingDigestRecord);
    assert_not_deserialize_owned!(ConfigurationAllowedDistributionRuleCount);
    assert_not_deserialize_owned!(ConfigurationForbiddenDistributionRuleCount);
    assert_not_deserialize_owned!(MetadataObjectAllowedDistributionRuleCount);
    assert_not_deserialize_owned!(MetadataObjectForbiddenDistributionRuleCount);
    assert_not_deserialize_owned!(DistributionRuleCount);
    assert_not_deserialize_owned!(DistributionRuleCounts);
    assert_not_deserialize_owned!(CanonicalSupportLayers);
    assert_not_deserialize_owned!(CanonicalLocalDifferences);
    assert_not_deserialize_owned!(DistributionPreviewAuthority);
    assert_not_deserialize_owned!(DistributionPreviewData);
    assert_not_deserialize_owned!(DistributionPreviewDigestRecord);
    assert_not_deserialize_owned!(BaselineDistributionPreviewData);
    assert_not_deserialize_owned!(RefreshDistributionPreviewData);
    assert_not_deserialize_owned!(ApprovedDistributionPreviewAuthority);
    assert_not_deserialize_owned!(DistributionArtifactObservationAuthority);
    assert_not_deserialize_owned!(DistributionData);
    assert_not_deserialize_owned!(BaselineDistributionData);
    assert_not_deserialize_owned!(RefreshDistributionData);
    assert_not_clone!(DeliveryInspectionAuthority);
    assert_not_clone!(ValidatedDeliveryInspectionAuthority);
    assert_not_clone!(DistributionPreviewAuthority);
    assert_not_clone!(ApprovedDistributionPreviewAuthority);
    assert_not_clone!(DistributionArtifactObservationAuthority);
    assert_not_serialize!(ValidatedDeliveryInspectionAuthority);
    assert_not_json_schema!(ValidatedDeliveryInspectionAuthority);

    #[test]
    fn inspection_is_exact_closed_and_content_bound() {
        let data = DeliveryInspectionData::from_authority(inspection_authority()).unwrap();
        let value = serde_json::to_value(&data).unwrap();

        assert_eq!(value["warningsAreErrors"], true);
        assert!(value.get("repositoryAnchor").is_none());
        assert!(value.get("anchorBindingDigest").is_none());
        assert_eq!(
            value["distributionRuleCounts"],
            json!([
                { "scope": "configuration", "verdict": "allowed", "count": 0 },
                { "scope": "configuration", "verdict": "forbidden", "count": 1 },
                { "scope": "metadataObject", "verdict": "allowed", "count": 2 },
                { "scope": "metadataObject", "verdict": "forbidden", "count": 0 },
            ])
        );
        assert!(schema_accepts::<DeliveryInspectionData>(&value));
        audit_json_schema(&schema::<DeliveryInspectionData>()).unwrap();

        let second = DeliveryInspectionData::from_authority(inspection_authority()).unwrap();
        assert_eq!(data.status_digest(), second.status_digest());
        assert_ne!(data.status_digest(), &digest('a'));

        let changed_permissions = DeliveryInspectionData::from_authority(
            DeliveryInspectionAuthority::new_test_only(
                repository_anchor(digest('a')),
                true,
                true,
                true,
                PlatformVersion::parse("8.3.27.1000").unwrap(),
                CompatibilityMode::parse("Version8_3_24").unwrap(),
                DeliveryPermissions::new_test_only(true, true),
                rule_counts(),
                vec![],
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(
            clean_inspection().status_digest(),
            changed_permissions.status_digest()
        );
    }

    #[test]
    fn inspection_collections_require_exact_canonical_order_and_uniqueness() {
        let duplicate_layers = DeliveryInspectionAuthority::new_test_only(
            repository_anchor(digest('a')),
            true,
            true,
            true,
            PlatformVersion::parse("8.3.27.1000").unwrap(),
            CompatibilityMode::parse("Version8_3_24").unwrap(),
            DeliveryPermissions::new_test_only(true, true),
            rule_counts(),
            vec![
                SupportLayerId::parse("base").unwrap(),
                SupportLayerId::parse("base").unwrap(),
            ],
            vec![],
        );
        assert!(duplicate_layers.is_err());

        let reversed_targets = DeliveryInspectionAuthority::new_test_only(
            repository_anchor(digest('a')),
            true,
            true,
            true,
            PlatformVersion::parse("8.3.27.1000").unwrap(),
            CompatibilityMode::parse("Version8_3_24").unwrap(),
            DeliveryPermissions::new_test_only(true, true),
            rule_counts(),
            vec![],
            vec![
                object_target(),
                serde_json::from_value(json!({ "targetKind": "configurationRoot" })).unwrap(),
            ],
        );
        assert!(reversed_targets.is_err());
    }

    #[test]
    fn inspection_schema_rejects_policy_weakening_and_recursive_forbidden_fields() {
        let valid = serde_json::to_value(
            DeliveryInspectionData::from_authority(inspection_authority()).unwrap(),
        )
        .unwrap();

        let mut weakened = valid.clone();
        weakened["warningsAreErrors"] = json!(false);
        assert!(!schema_accepts::<DeliveryInspectionData>(&weakened));

        for (pointer, field) in [
            ("", "cwd"),
            ("/configurationIdentity", "password"),
            ("/deliveryPermissions", "processHandle"),
            ("/distributionRuleCounts/0", "rawConnectionString"),
            ("/localDifferences/0", "serviceEndpoint"),
        ] {
            let mut invalid = valid.clone();
            invalid
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(field.to_owned(), json!("forbidden"));
            assert!(
                !schema_accepts::<DeliveryInspectionData>(&invalid),
                "accepted forbidden {field} at {pointer}"
            );
        }
    }

    #[test]
    fn inspection_rule_counts_schema_is_the_exact_four_position_tuple() {
        let row_union = schema::<DistributionRuleCount>();
        assert_eq!(row_union["oneOf"].as_array().unwrap().len(), 4);
        audit_json_schema(&row_union).unwrap();

        let tuple = schema::<DistributionRuleCounts>();
        assert_eq!(tuple["type"], "array");
        assert_eq!(tuple["minItems"], 4);
        assert_eq!(tuple["maxItems"], 4);
        assert_eq!(tuple["items"], false);
        assert_eq!(tuple["prefixItems"].as_array().unwrap().len(), 4);

        let valid = serde_json::to_value(rule_counts()).unwrap();
        assert!(schema_accepts::<DistributionRuleCounts>(&valid));
        let mut swapped = valid.as_array().unwrap().clone();
        swapped.swap(0, 1);
        assert!(!schema_accepts::<DistributionRuleCounts>(&Value::Array(
            swapped
        )));
    }

    #[test]
    fn inspection_and_distribution_results_and_digest_records_are_recursively_closed() {
        let validated_inspection = validated_clean_inspection();
        let binding_record = DeliveryInspectionAnchorBindingDigestRecord {
            status_digest: validated_inspection.data.status_digest.clone(),
            anchor_digest: validated_inspection
                .repository_anchor
                .anchor_digest()
                .clone(),
        };
        assert_eq!(
            validated_inspection.binding_digest,
            canonical_contract_digest(&binding_record, None).unwrap()
        );
        assert_recursively_closed::<DeliveryInspectionAnchorBindingDigestRecord>(
            serde_json::to_value(&binding_record).unwrap(),
        );
        let inspection = validated_inspection.into_data();
        let inspection_record = DeliveryInspectionStatusDigestRecord {
            configuration_identity: inspection.configuration_identity.clone(),
            repository_identity: inspection.repository_identity.clone(),
            binding_matches: inspection.binding_matches,
            main_equals_repository: inspection.main_equals_repository,
            main_equals_database_configuration: inspection.main_equals_database_configuration,
            platform_version: inspection.platform_version.clone(),
            compatibility_mode: inspection.compatibility_mode.clone(),
            delivery_permissions: inspection.delivery_permissions.clone(),
            distribution_rule_counts: inspection.distribution_rule_counts.clone(),
            support_layers: inspection.support_layers.clone(),
            local_differences: inspection.local_differences.clone(),
            warnings_are_errors: TrueLiteral,
        };
        assert_recursively_closed::<DeliveryInspectionData>(
            serde_json::to_value(&inspection).unwrap(),
        );
        assert_recursively_closed::<DeliveryInspectionStatusDigestRecord>(
            serde_json::to_value(inspection_record).unwrap(),
        );

        let baseline = distribution_preview(ArtifactRole::BaselineDistribution);
        let refresh = distribution_preview(ArtifactRole::RefreshDistribution);
        assert_ne!(baseline.preview_digest(), refresh.preview_digest());
        let preview_record = DistributionPreviewDigestRecord {
            role: DistributionRole::BaselineDistribution,
            configuration_identity: baseline.configuration_identity().clone(),
            repository_anchor: baseline.repository_anchor().clone(),
            platform_version: baseline.platform_version().clone(),
            inspection_digest: baseline.inspection_digest().clone(),
            planned_artifact_kind: ConfigurationDistributionKind::Value,
        };
        assert_recursively_closed::<DistributionPreviewDigestRecord>(
            serde_json::to_value(preview_record).unwrap(),
        );
        assert_recursively_closed::<BaselineDistributionPreviewData>(
            serde_json::to_value(&baseline).unwrap(),
        );
        assert_recursively_closed::<RefreshDistributionPreviewData>(
            serde_json::to_value(&refresh).unwrap(),
        );

        let baseline_approval = ApprovedDistributionPreviewAuthority::approve_test_only(&baseline);
        let baseline_observation = DistributionArtifactObservationAuthority::from_writer_test_only(
            &baseline_approval,
            UnicaId::parse(UUID_B).unwrap(),
            digest('d'),
            NormalizedUtcInstant::parse("2026-07-22T01:02:03Z").unwrap(),
        );
        let baseline_data =
            DistributionData::from_approved_preview(baseline_approval, baseline_observation)
                .unwrap();
        assert_recursively_closed::<BaselineDistributionData>(
            serde_json::to_value(baseline_data).unwrap(),
        );

        let refresh_approval = ApprovedDistributionPreviewAuthority::approve_test_only(&refresh);
        let refresh_observation = DistributionArtifactObservationAuthority::from_writer_test_only(
            &refresh_approval,
            UnicaId::parse(UUID_B).unwrap(),
            digest('d'),
            NormalizedUtcInstant::parse("2026-07-22T01:02:03Z").unwrap(),
        );
        let refresh_data =
            DistributionData::from_approved_preview(refresh_approval, refresh_observation).unwrap();
        assert_recursively_closed::<RefreshDistributionData>(
            serde_json::to_value(refresh_data).unwrap(),
        );
    }

    #[test]
    fn distribution_preview_has_two_exact_content_bound_physical_leaves() {
        for (role, wire) in [
            (ArtifactRole::BaselineDistribution, "baselineDistribution"),
            (ArtifactRole::RefreshDistribution, "refreshDistribution"),
        ] {
            let preview = distribution_preview(role);
            let value = serde_json::to_value(&preview).unwrap();
            assert_eq!(value["role"], wire);
            assert_eq!(value["plannedArtifactKind"], "configurationDistribution");
            assert_eq!(
                value["inspectionDigest"],
                clean_inspection().status_digest().as_str()
            );
            assert!(schema_accepts::<DistributionPreviewData>(&value));
            for forbidden in [
                "artifactId",
                "sha256",
                "createdAt",
                "receiptId",
                "verificationId",
                "probeId",
                "taskInfobaseId",
                "taskWorkspaceId",
                "currentFingerprint",
                "vendorFingerprint",
                "sourceFingerprint",
            ] {
                let mut invalid = value.clone();
                invalid
                    .as_object_mut()
                    .unwrap()
                    .insert(forbidden.to_owned(), json!(UUID_A));
                assert!(
                    !schema_accepts::<DistributionPreviewData>(&invalid),
                    "preview accepted post-effect field {forbidden}"
                );
            }
        }

        let union = schema::<DistributionPreviewData>();
        assert_eq!(union["oneOf"].as_array().unwrap().len(), 2);
        audit_json_schema(&union).unwrap();

        let baseline = distribution_preview(ArtifactRole::BaselineDistribution);
        let record = DistributionPreviewDigestRecord {
            role: DistributionRole::BaselineDistribution,
            configuration_identity: baseline.configuration_identity().clone(),
            repository_anchor: baseline.repository_anchor().clone(),
            platform_version: baseline.platform_version().clone(),
            inspection_digest: baseline.inspection_digest().clone(),
            planned_artifact_kind: ConfigurationDistributionKind::Value,
        };
        assert_eq!(
            baseline.preview_digest(),
            &canonical_contract_digest(&record, None).unwrap()
        );

        let changed_anchor = repository_anchor_with(digest('a'), "43", digest('d'), digest('c'));
        let changed = DistributionPreviewData::from_authority(
            DistributionPreviewAuthority::from_inspection_test_only(
                ArtifactRole::BaselineDistribution,
                validated_clean_inspection_with(changed_anchor),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(baseline.inspection_digest(), changed.inspection_digest());
        assert_ne!(baseline.preview_digest(), changed.preview_digest());
    }

    #[test]
    fn distribution_preview_authority_rejects_dirty_tampered_and_non_distribution_inputs() {
        let dirty =
            ValidatedDeliveryInspectionAuthority::from_authority(inspection_authority()).unwrap();
        assert!(DistributionPreviewAuthority::from_inspection_test_only(
            ArtifactRole::BaselineDistribution,
            dirty,
        )
        .is_err());

        let mut tampered = validated_clean_inspection();
        tampered.data.repository_identity = digest('d');
        assert!(DistributionPreviewAuthority::from_inspection_test_only(
            ArtifactRole::BaselineDistribution,
            tampered,
        )
        .is_err());
        assert!(DistributionPreviewAuthority::from_inspection_test_only(
            ArtifactRole::OrdinaryResult,
            validated_clean_inspection(),
        )
        .is_err());
    }

    #[test]
    fn distribution_preview_rejects_foreign_history_cursor_from_the_same_repository_identity() {
        let mut inspection = validated_clean_inspection();
        inspection.repository_anchor =
            repository_anchor_with(digest('a'), "43", digest('d'), digest('c'));

        assert!(DistributionPreviewAuthority::from_inspection_test_only(
            ArtifactRole::BaselineDistribution,
            inspection,
        )
        .is_err());
    }

    #[test]
    fn distribution_preview_rejects_foreign_fingerprint_from_the_same_repository_identity() {
        let mut inspection = validated_clean_inspection();
        inspection.repository_anchor =
            repository_anchor_with(digest('a'), "42", digest('b'), digest('d'));

        assert!(DistributionPreviewAuthority::from_inspection_test_only(
            ArtifactRole::RefreshDistribution,
            inspection,
        )
        .is_err());
    }

    #[test]
    fn distribution_preview_rejects_foreign_anchor_binding_digest() {
        let mut inspection = validated_clean_inspection();
        let foreign = validated_clean_inspection_with(repository_anchor_with(
            digest('a'),
            "43",
            digest('d'),
            digest('c'),
        ));
        assert_eq!(inspection.data.status_digest, foreign.data.status_digest);
        assert_ne!(inspection.binding_digest, foreign.binding_digest);
        inspection.binding_digest = foreign.binding_digest;

        assert!(DistributionPreviewAuthority::from_inspection_test_only(
            ArtifactRole::BaselineDistribution,
            inspection,
        )
        .is_err());
    }

    #[test]
    fn applied_distribution_is_observed_byte_hash_bound_to_the_approved_preview() {
        for role in [
            ArtifactRole::BaselineDistribution,
            ArtifactRole::RefreshDistribution,
        ] {
            let preview = distribution_preview(role);
            let approval = ApprovedDistributionPreviewAuthority::approve_test_only(&preview);
            let observation = DistributionArtifactObservationAuthority::from_writer_test_only(
                &approval,
                UnicaId::parse(UUID_B).unwrap(),
                digest('d'),
                NormalizedUtcInstant::parse("2026-07-22T01:02:03Z").unwrap(),
            );
            let applied = DistributionData::from_approved_preview(approval, observation).unwrap();
            let value = serde_json::to_value(&applied).unwrap();
            assert_eq!(value["sha256"], digest('d').as_str());
            assert_eq!(value["previewDigest"], preview.preview_digest().as_str());
            assert_eq!(value["kind"], "configurationDistribution");
            assert_eq!(
                value["configurationIdentity"],
                serde_json::to_value(preview.configuration_identity()).unwrap()
            );
            assert!(schema_accepts::<DistributionData>(&value));

            let mut wrong_kind = value.clone();
            wrong_kind["kind"] = json!("ordinaryConfiguration");
            assert!(!schema_accepts::<DistributionData>(&wrong_kind));
            let mut injected = value;
            injected
                .as_object_mut()
                .unwrap()
                .insert("approvedPreviewDigest".to_owned(), json!(digest('e')));
            assert!(!schema_accepts::<DistributionData>(&injected));
        }

        let baseline = distribution_preview(ArtifactRole::BaselineDistribution);
        let refresh = distribution_preview(ArtifactRole::RefreshDistribution);
        let baseline_approval = ApprovedDistributionPreviewAuthority::approve_test_only(&baseline);
        let observation = DistributionArtifactObservationAuthority::from_writer_test_only(
            &baseline_approval,
            UnicaId::parse(UUID_B).unwrap(),
            digest('d'),
            NormalizedUtcInstant::parse("2026-07-22T01:02:03Z").unwrap(),
        );
        let refresh_approval = ApprovedDistributionPreviewAuthority::approve_test_only(&refresh);
        assert!(DistributionData::from_approved_preview(refresh_approval, observation).is_err());

        let union = schema::<DistributionData>();
        assert_eq!(union["oneOf"].as_array().unwrap().len(), 2);
        audit_json_schema(&union).unwrap();
    }
}

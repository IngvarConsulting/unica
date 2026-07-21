#[allow(dead_code)]
pub(crate) mod delivery;
#[allow(dead_code)]
pub(crate) mod merge;
#[allow(dead_code)]
pub(crate) mod task;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

macro_rules! request_one_of_schema {
    ($request:ty, $name:literal, [$($branch:ty),+ $(,)?]) => {
        impl schemars::JsonSchema for $request {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                $name.into()
            }

            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                crate::domain::branched_development::contracts::schema::one_of_schema(vec![
                    $(generator.subschema_for::<$branch>()),+
                ])
            }
        }
    };
}

pub(super) use request_one_of_schema;

macro_rules! boolean_literal {
    ($name:ident, $value:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(super) struct $name;

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_bool($value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = bool::deserialize(deserializer)?;
                if value == $value {
                    Ok(Self)
                } else {
                    Err(D::Error::custom(concat!("expected literal ", stringify!($value))))
                }
            }
        }

        impl JsonSchema for $name {
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "boolean", "const": $value })
            }
        }
    };
}

boolean_literal!(TrueLiteral, true);
boolean_literal!(FalseLiteral, false);

pub(super) fn execution_policy_for_json<T>(
    value: &Value,
    policy: impl FnOnce(&T) -> crate::domain::branched_development::ExecutionPolicy,
) -> Option<crate::domain::branched_development::ExecutionPolicy>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value::<T>(value.clone())
        .ok()
        .map(|request| policy(&request))
}

#[cfg(test)]
mod tests {
    use super::delivery::{
        DeliveryCreateRequest, DeliveryCreateRequestVariant, DeliveryDeployRequest,
        DeliveryDeployRequestVariant, DeliveryInspectRequest, DeliveryInspectRequestVariant,
        DeliveryVerifyRequest, DeliveryVerifyRequestVariant,
    };
    use super::task::{
        BranchedArchiveRequest, BranchedArchiveRequestVariant, BranchedCleanupRequest,
        BranchedCleanupRequestVariant, BranchedStartRequest, BranchedStartRequestVariant,
        BranchedStatusRequest, BranchedStatusRequestVariant, CommonMutationRequest,
        CommonTaskRequest,
    };
    use crate::domain::branched_development::contracts::schema::{
        audit_json_schema, is_i_json_lf_text, is_i_json_single_line_text,
        is_normalized_utc_instant, I_JSON_LF_TEXT_FORMAT, I_JSON_SINGLE_LINE_TEXT_FORMAT,
        NORMALIZED_UTC_INSTANT_FORMAT,
    };
    use crate::domain::branched_development::ExecutionPolicy;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const CWD: &str = "/original/project";
    const TASK_ID: &str = "TASK-142";
    const OPERATION_ID: &str = "123e4567-e89b-12d3-a456-426614174000";
    const RESOURCE_ID: &str = "223e4567-e89b-12d3-a456-426614174000";
    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn accepts<T: DeserializeOwned>(value: Value) -> T {
        serde_json::from_value(value).expect("fixture must satisfy the request contract")
    }

    fn rejects<T: DeserializeOwned>(value: Value) {
        assert!(
            serde_json::from_value::<T>(value.clone()).is_err(),
            "request contract accepted {value}"
        );
    }

    fn common_task() -> Value {
        json!({ "cwd": CWD, "taskId": TASK_ID })
    }

    fn common_mutation() -> Value {
        json!({ "cwd": CWD, "taskId": TASK_ID, "operationId": OPERATION_ID })
    }

    fn with(mut value: Value, fields: &[(&str, Value)]) -> Value {
        let object = value.as_object_mut().unwrap();
        for (name, field) in fields {
            object.insert((*name).to_owned(), field.clone());
        }
        value
    }

    fn without(mut value: Value, field: &str) -> Value {
        value.as_object_mut().unwrap().remove(field);
        value
    }

    fn assert_schema_is_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).expect("request schema must be recursively closed and typed");
    }

    fn assert_exact_one_of<T: JsonSchema>(expected_branches: usize) {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        assert_eq!(
            schema.get("oneOf").and_then(Value::as_array).map(Vec::len),
            Some(expected_branches)
        );
        assert!(
            !contains_keyword(&schema, "anyOf"),
            "schema retained an anyOf escape"
        );
    }

    fn contains_keyword(value: &Value, keyword: &str) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key(keyword)
                    || object
                        .values()
                        .any(|nested| contains_keyword(nested, keyword))
            }
            Value::Array(array) => array.iter().any(|nested| contains_keyword(nested, keyword)),
            _ => false,
        }
    }

    fn schema_validator<T: JsonSchema>() -> jsonschema::Validator {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .with_format(I_JSON_SINGLE_LINE_TEXT_FORMAT, is_i_json_single_line_text)
            .with_format(I_JSON_LF_TEXT_FORMAT, is_i_json_lf_text)
            .with_format(NORMALIZED_UTC_INSTANT_FORMAT, is_normalized_utc_instant)
            .should_validate_formats(true)
            .should_ignore_unknown_formats(false)
            .build(&schema)
            .expect("request schema must compile")
    }

    #[test]
    fn common_request_records_are_closed_and_physically_complete() {
        accepts::<CommonTaskRequest>(common_task());
        accepts::<CommonMutationRequest>(common_mutation());

        for field in ["cwd", "taskId"] {
            rejects::<CommonTaskRequest>(without(common_task(), field));
            rejects::<CommonMutationRequest>(without(common_mutation(), field));
        }
        rejects::<CommonMutationRequest>(without(common_mutation(), "operationId"));
        rejects::<CommonTaskRequest>(with(common_task(), &[("operationId", json!(OPERATION_ID))]));
        rejects::<CommonMutationRequest>(with(common_mutation(), &[("extra", json!(true))]));
    }

    #[test]
    fn start_and_status_accept_only_their_exact_physical_records() {
        let start = with(
            common_mutation(),
            &[
                ("profile", json!("safe-profile")),
                ("taskSummary", json!("Implement issue 137")),
            ],
        );
        let parsed = accepts::<BranchedStartRequest>(start.clone());
        assert_eq!(parsed.request_variant(), BranchedStartRequestVariant::Start);
        assert_eq!(parsed.execution_policy(), ExecutionPolicy::LocalJournaled);

        let status = accepts::<BranchedStatusRequest>(common_task());
        assert_eq!(
            status.request_variant(),
            BranchedStatusRequestVariant::Status
        );
        assert_eq!(status.execution_policy(), ExecutionPolicy::ReadOnly);

        for field in ["cwd", "taskId", "operationId", "profile", "taskSummary"] {
            rejects::<BranchedStartRequest>(without(start.clone(), field));
        }
        rejects::<BranchedStartRequest>(with(start.clone(), &[("profile", json!(""))]));
        rejects::<BranchedStartRequest>(with(start.clone(), &[("taskSummary", json!(""))]));
        rejects::<BranchedStartRequest>(with(
            start.clone(),
            &[("cwd", json!("/disallowed\tselector"))],
        ));
        rejects::<BranchedStartRequest>(with(start.clone(), &[("dryRun", json!(true))]));
        rejects::<BranchedStatusRequest>(without(common_task(), "cwd"));
        rejects::<BranchedStatusRequest>(with(
            common_task(),
            &[("operationId", json!(OPERATION_ID))],
        ));

        assert!(
            BranchedStartRequest::execution_policy_for_json(&without(start, "taskId")).is_none()
        );
        assert!(BranchedStatusRequest::execution_policy_for_json(&json!({
            "cwd": CWD,
            "taskId": TASK_ID,
            "extra": true
        }))
        .is_none());
    }

    #[test]
    fn archive_models_success_and_abandoned_preview_apply_leaves_exactly() {
        let success_preview = with(common_mutation(), &[("outcome", json!("success"))]);
        let success_preview_true = with(success_preview.clone(), &[("dryRun", json!(true))]);
        let success_apply = with(
            success_preview.clone(),
            &[
                ("dryRun", json!(false)),
                ("approvedPreviewDigest", json!(DIGEST)),
            ],
        );
        let abandoned_preview = with(
            common_mutation(),
            &[
                ("outcome", json!("abandoned")),
                ("reason", json!("No longer required")),
            ],
        );
        let abandoned_preview_true = with(abandoned_preview.clone(), &[("dryRun", json!(true))]);
        let abandoned_apply = with(
            abandoned_preview.clone(),
            &[
                ("dryRun", json!(false)),
                ("approvedPreviewDigest", json!(DIGEST)),
            ],
        );

        for (fixture, variant) in [
            (
                success_preview.clone(),
                BranchedArchiveRequestVariant::SuccessPreview,
            ),
            (
                success_preview_true,
                BranchedArchiveRequestVariant::SuccessPreview,
            ),
            (
                success_apply.clone(),
                BranchedArchiveRequestVariant::SuccessApply,
            ),
            (
                abandoned_preview.clone(),
                BranchedArchiveRequestVariant::AbandonedPreview,
            ),
            (
                abandoned_preview_true,
                BranchedArchiveRequestVariant::AbandonedPreview,
            ),
            (
                abandoned_apply.clone(),
                BranchedArchiveRequestVariant::AbandonedApply,
            ),
        ] {
            let parsed = accepts::<BranchedArchiveRequest>(fixture);
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(
                parsed.execution_policy(),
                ExecutionPolicy::PreviewedJournaledEffect
            );
        }

        rejects::<BranchedArchiveRequest>(with(
            success_preview.clone(),
            &[("reason", json!("forbidden"))],
        ));
        rejects::<BranchedArchiveRequest>(without(abandoned_preview.clone(), "reason"));
        rejects::<BranchedArchiveRequest>(with(abandoned_preview, &[("reason", json!(""))]));
        rejects::<BranchedArchiveRequest>(with(
            success_preview.clone(),
            &[("dryRun", Value::Null)],
        ));
        rejects::<BranchedArchiveRequest>(with(
            success_preview.clone(),
            &[("dryRun", json!(false))],
        ));
        rejects::<BranchedArchiveRequest>(with(
            success_preview.clone(),
            &[("approvedPreviewDigest", json!(DIGEST))],
        ));
        rejects::<BranchedArchiveRequest>(without(success_apply.clone(), "approvedPreviewDigest"));
        rejects::<BranchedArchiveRequest>(with(success_apply, &[("reason", json!("forbidden"))]));
        rejects::<BranchedArchiveRequest>(without(abandoned_apply, "operationId"));

        assert!(BranchedArchiveRequest::execution_policy_for_json(&with(
            success_preview,
            &[("dryRun", Value::Null)]
        ))
        .is_none());
    }

    #[test]
    fn cleanup_models_preview_and_apply_without_generic_boolean_or_approval() {
        let preview = with(common_mutation(), &[("archiveId", json!(RESOURCE_ID))]);
        let preview_true = with(preview.clone(), &[("dryRun", json!(true))]);
        let apply = with(
            preview.clone(),
            &[
                ("dryRun", json!(false)),
                ("approvedPreviewDigest", json!(DIGEST)),
            ],
        );

        for (fixture, variant) in [
            (preview.clone(), BranchedCleanupRequestVariant::Preview),
            (preview_true, BranchedCleanupRequestVariant::Preview),
            (apply.clone(), BranchedCleanupRequestVariant::Apply),
        ] {
            let parsed = accepts::<BranchedCleanupRequest>(fixture);
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(
                parsed.execution_policy(),
                ExecutionPolicy::PreviewedJournaledEffect
            );
        }

        rejects::<BranchedCleanupRequest>(without(preview.clone(), "archiveId"));
        rejects::<BranchedCleanupRequest>(without(preview.clone(), "operationId"));
        rejects::<BranchedCleanupRequest>(with(preview.clone(), &[("dryRun", Value::Null)]));
        rejects::<BranchedCleanupRequest>(with(
            preview.clone(),
            &[("approval", json!({ "digest": DIGEST }))],
        ));
        rejects::<BranchedCleanupRequest>(without(apply, "approvedPreviewDigest"));
    }

    #[test]
    fn inspect_is_read_only_and_has_no_tool_specific_fields() {
        let parsed = accepts::<DeliveryInspectRequest>(common_task());
        assert_eq!(
            parsed.request_variant(),
            DeliveryInspectRequestVariant::Inspect
        );
        assert_eq!(parsed.execution_policy(), ExecutionPolicy::ReadOnly);
        rejects::<DeliveryInspectRequest>(without(common_task(), "taskId"));
        rejects::<DeliveryInspectRequest>(with(
            common_task(),
            &[("operationId", json!(OPERATION_ID))],
        ));
        assert!(DeliveryInspectRequest::execution_policy_for_json(&json!({
            "cwd": CWD,
            "taskId": TASK_ID,
            "dryRun": true
        }))
        .is_none());
    }

    #[test]
    fn create_has_only_baseline_and_refresh_preview_apply_variants() {
        for (role, preview_variant, apply_variant) in [
            (
                "baselineDistribution",
                DeliveryCreateRequestVariant::BaselineDistributionPreview,
                DeliveryCreateRequestVariant::BaselineDistributionApply,
            ),
            (
                "refreshDistribution",
                DeliveryCreateRequestVariant::RefreshDistributionPreview,
                DeliveryCreateRequestVariant::RefreshDistributionApply,
            ),
        ] {
            let preview = with(
                common_mutation(),
                &[("role", json!(role)), ("inspectionDigest", json!(DIGEST))],
            );
            let preview_true = with(preview.clone(), &[("dryRun", json!(true))]);
            let apply = with(
                preview.clone(),
                &[
                    ("dryRun", json!(false)),
                    ("approvedPreviewDigest", json!(DIGEST)),
                ],
            );
            for fixture in [preview.clone(), preview_true] {
                let parsed = accepts::<DeliveryCreateRequest>(fixture);
                assert_eq!(parsed.request_variant(), preview_variant);
                assert_eq!(
                    parsed.execution_policy(),
                    ExecutionPolicy::PreviewedJournaledEffect
                );
            }
            let parsed = accepts::<DeliveryCreateRequest>(apply.clone());
            assert_eq!(parsed.request_variant(), apply_variant);
            assert_eq!(
                parsed.execution_policy(),
                ExecutionPolicy::PreviewedJournaledEffect
            );

            rejects::<DeliveryCreateRequest>(without(preview.clone(), "inspectionDigest"));
            rejects::<DeliveryCreateRequest>(without(preview.clone(), "operationId"));
            rejects::<DeliveryCreateRequest>(with(preview.clone(), &[("dryRun", Value::Null)]));
            rejects::<DeliveryCreateRequest>(with(
                preview,
                &[("approvedPreviewDigest", json!(DIGEST))],
            ));
            rejects::<DeliveryCreateRequest>(without(apply, "approvedPreviewDigest"));
        }

        for role in [
            "ordinaryResult",
            "supportRecoveryDistribution",
            "configurationDistribution",
        ] {
            rejects::<DeliveryCreateRequest>(with(
                common_mutation(),
                &[("role", json!(role)), ("inspectionDigest", json!(DIGEST))],
            ));
        }
    }

    #[test]
    fn verify_accepts_only_registered_artifact_id_and_accepted_kind_expectations() {
        let request = with(common_mutation(), &[("artifactId", json!(RESOURCE_ID))]);
        let parsed = accepts::<DeliveryVerifyRequest>(request.clone());
        assert_eq!(
            parsed.request_variant(),
            DeliveryVerifyRequestVariant::Verify
        );
        assert_eq!(parsed.execution_policy(), ExecutionPolicy::Contained);

        for expected_kind in ["configurationDistribution", "ordinaryConfiguration"] {
            let parsed = accepts::<DeliveryVerifyRequest>(with(
                request.clone(),
                &[("expectedKind", json!(expected_kind))],
            ));
            assert_eq!(
                parsed.request_variant(),
                DeliveryVerifyRequestVariant::Verify
            );
            assert_eq!(parsed.execution_policy(), ExecutionPolicy::Contained);
        }

        rejects::<DeliveryVerifyRequest>(without(request.clone(), "artifactId"));
        rejects::<DeliveryVerifyRequest>(without(request.clone(), "operationId"));
        rejects::<DeliveryVerifyRequest>(with(request.clone(), &[("expectedKind", Value::Null)]));
        for expected_kind in [
            "configurationUpdate",
            "invalidArtifact",
            "baselineDistribution",
        ] {
            rejects::<DeliveryVerifyRequest>(with(
                request.clone(),
                &[("expectedKind", json!(expected_kind))],
            ));
        }
    }

    #[test]
    fn deploy_models_preview_and_apply_for_a_verified_distribution_id() {
        let preview = with(common_mutation(), &[("distributionId", json!(RESOURCE_ID))]);
        let preview_true = with(preview.clone(), &[("dryRun", json!(true))]);
        let apply = with(
            preview.clone(),
            &[
                ("dryRun", json!(false)),
                ("approvedPreviewDigest", json!(DIGEST)),
            ],
        );

        for (fixture, variant) in [
            (preview.clone(), DeliveryDeployRequestVariant::Preview),
            (preview_true, DeliveryDeployRequestVariant::Preview),
            (apply.clone(), DeliveryDeployRequestVariant::Apply),
        ] {
            let parsed = accepts::<DeliveryDeployRequest>(fixture);
            assert_eq!(parsed.request_variant(), variant);
            assert_eq!(
                parsed.execution_policy(),
                ExecutionPolicy::PreviewedJournaledEffect
            );
        }

        rejects::<DeliveryDeployRequest>(without(preview.clone(), "distributionId"));
        rejects::<DeliveryDeployRequest>(without(preview.clone(), "operationId"));
        rejects::<DeliveryDeployRequest>(with(preview.clone(), &[("dryRun", Value::Null)]));
        rejects::<DeliveryDeployRequest>(with(
            preview,
            &[("approvedPreviewDigest", json!(DIGEST))],
        ));
        rejects::<DeliveryDeployRequest>(without(apply, "approvedPreviewDigest"));
    }

    #[test]
    fn every_task_and_delivery_request_schema_is_recursively_closed_and_typed() {
        assert_schema_is_closed::<CommonTaskRequest>();
        assert_schema_is_closed::<CommonMutationRequest>();
        assert_schema_is_closed::<BranchedStartRequest>();
        assert_schema_is_closed::<BranchedStatusRequest>();
        assert_schema_is_closed::<BranchedArchiveRequest>();
        assert_schema_is_closed::<BranchedCleanupRequest>();
        assert_schema_is_closed::<DeliveryInspectRequest>();
        assert_schema_is_closed::<DeliveryCreateRequest>();
        assert_schema_is_closed::<DeliveryVerifyRequest>();
        assert_schema_is_closed::<DeliveryDeployRequest>();

        assert_exact_one_of::<BranchedArchiveRequest>(6);
        assert_exact_one_of::<BranchedCleanupRequest>(3);
        assert_exact_one_of::<DeliveryCreateRequest>(6);
        assert_exact_one_of::<DeliveryVerifyRequest>(2);
        assert_exact_one_of::<DeliveryDeployRequest>(3);
    }

    #[test]
    fn generated_schemas_enforce_physical_preview_apply_and_subset_boundaries() {
        let archive = schema_validator::<BranchedArchiveRequest>();
        let success_preview = with(common_mutation(), &[("outcome", json!("success"))]);
        assert!(archive.is_valid(&success_preview));
        assert!(archive.is_valid(&with(success_preview.clone(), &[("dryRun", json!(true))])));
        assert!(archive.is_valid(&with(
            success_preview.clone(),
            &[
                ("dryRun", json!(false)),
                ("approvedPreviewDigest", json!(DIGEST))
            ]
        )));
        for invalid in [
            with(success_preview.clone(), &[("dryRun", Value::Null)]),
            with(success_preview.clone(), &[("dryRun", json!(false))]),
            with(
                success_preview.clone(),
                &[("approvedPreviewDigest", json!(DIGEST))],
            ),
            with(success_preview, &[("reason", json!("forbidden"))]),
        ] {
            assert!(
                !archive.is_valid(&invalid),
                "archive schema accepted {invalid}"
            );
        }

        let create = schema_validator::<DeliveryCreateRequest>();
        let create_preview = with(
            common_mutation(),
            &[
                ("role", json!("baselineDistribution")),
                ("inspectionDigest", json!(DIGEST)),
            ],
        );
        assert!(create.is_valid(&create_preview));
        assert!(create.is_valid(&with(create_preview.clone(), &[("dryRun", json!(true))])));
        assert!(!create.is_valid(&with(
            create_preview,
            &[("role", json!("supportRecoveryDistribution"))]
        )));

        let verify = schema_validator::<DeliveryVerifyRequest>();
        let verification = with(common_mutation(), &[("artifactId", json!(RESOURCE_ID))]);
        assert!(verify.is_valid(&verification));
        assert!(verify.is_valid(&with(
            verification.clone(),
            &[("expectedKind", json!("ordinaryConfiguration"))]
        )));
        assert!(!verify.is_valid(&with(
            verification.clone(),
            &[("expectedKind", Value::Null)]
        )));
        assert!(!verify.is_valid(&with(
            verification,
            &[("expectedKind", json!("configurationUpdate"))]
        )));
    }

    #[test]
    fn policy_selection_is_exact_and_malformed_payloads_select_none_for_every_tool() {
        let start = with(
            common_mutation(),
            &[
                ("profile", json!("safe-profile")),
                ("taskSummary", json!("Implement issue 137")),
            ],
        );
        let archive = with(common_mutation(), &[("outcome", json!("success"))]);
        let cleanup = with(common_mutation(), &[("archiveId", json!(RESOURCE_ID))]);
        let create = with(
            common_mutation(),
            &[
                ("role", json!("baselineDistribution")),
                ("inspectionDigest", json!(DIGEST)),
            ],
        );
        let verify = with(common_mutation(), &[("artifactId", json!(RESOURCE_ID))]);
        let deploy = with(common_mutation(), &[("distributionId", json!(RESOURCE_ID))]);

        assert_eq!(
            BranchedStartRequest::execution_policy_for_json(&start),
            Some(ExecutionPolicy::LocalJournaled)
        );
        assert_eq!(
            BranchedStatusRequest::execution_policy_for_json(&common_task()),
            Some(ExecutionPolicy::ReadOnly)
        );
        assert_eq!(
            BranchedArchiveRequest::execution_policy_for_json(&archive),
            Some(ExecutionPolicy::PreviewedJournaledEffect)
        );
        assert_eq!(
            BranchedCleanupRequest::execution_policy_for_json(&cleanup),
            Some(ExecutionPolicy::PreviewedJournaledEffect)
        );
        assert_eq!(
            DeliveryInspectRequest::execution_policy_for_json(&common_task()),
            Some(ExecutionPolicy::ReadOnly)
        );
        assert_eq!(
            DeliveryCreateRequest::execution_policy_for_json(&create),
            Some(ExecutionPolicy::PreviewedJournaledEffect)
        );
        assert_eq!(
            DeliveryVerifyRequest::execution_policy_for_json(&verify),
            Some(ExecutionPolicy::Contained)
        );
        assert_eq!(
            DeliveryDeployRequest::execution_policy_for_json(&deploy),
            Some(ExecutionPolicy::PreviewedJournaledEffect)
        );

        let malformed = json!({
            "cwd": CWD,
            "taskId": TASK_ID,
            "operationId": OPERATION_ID,
            "dryRun": null
        });

        assert!(BranchedStartRequest::execution_policy_for_json(&malformed).is_none());
        assert!(BranchedStatusRequest::execution_policy_for_json(&malformed).is_none());
        assert!(BranchedArchiveRequest::execution_policy_for_json(&malformed).is_none());
        assert!(BranchedCleanupRequest::execution_policy_for_json(&malformed).is_none());
        assert!(DeliveryInspectRequest::execution_policy_for_json(&malformed).is_none());
        assert!(DeliveryCreateRequest::execution_policy_for_json(&malformed).is_none());
        assert!(DeliveryVerifyRequest::execution_policy_for_json(&malformed).is_none());
        assert!(DeliveryDeployRequest::execution_policy_for_json(&malformed).is_none());
    }
}

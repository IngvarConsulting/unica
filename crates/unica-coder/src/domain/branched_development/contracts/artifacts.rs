use super::schema::one_of_schema;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ArtifactRole {
    BaselineDistribution,
    RefreshDistribution,
    OrdinaryResult,
    SupportRecoveryDistribution,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ArtifactKind {
    ConfigurationDistribution,
    OrdinaryConfiguration,
    ConfigurationUpdate,
    InvalidArtifact,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AcceptedArtifactKind {
    ConfigurationDistribution,
    OrdinaryConfiguration,
}

macro_rules! string_literal {
    ($name:ident, $variant:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            $variant,
        }
    };
}

string_literal!(
    ConfigurationDistributionKind,
    Value,
    "configurationDistribution"
);
string_literal!(OrdinaryConfigurationKind, Value, "ordinaryConfiguration");
string_literal!(BaselineDistributionRole, Value, "baselineDistribution");
string_literal!(RefreshDistributionRole, Value, "refreshDistribution");
string_literal!(OrdinaryResultRole, Value, "ordinaryResult");
string_literal!(
    SupportRecoveryDistributionRole,
    Value,
    "supportRecoveryDistribution"
);

macro_rules! kind_role_pair {
    ($name:ident, $kind:ty, $role:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct $name {
            kind: $kind,
            role: $role,
        }
    };
}

kind_role_pair!(
    BaselineDistributionKindRole,
    ConfigurationDistributionKind,
    BaselineDistributionRole
);
kind_role_pair!(
    RefreshDistributionKindRole,
    ConfigurationDistributionKind,
    RefreshDistributionRole
);
kind_role_pair!(
    SupportRecoveryDistributionKindRole,
    ConfigurationDistributionKind,
    SupportRecoveryDistributionRole
);
kind_role_pair!(
    OrdinaryResultKindRole,
    OrdinaryConfigurationKind,
    OrdinaryResultRole
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum ArtifactKindRole {
    BaselineDistribution(BaselineDistributionKindRole),
    RefreshDistribution(RefreshDistributionKindRole),
    SupportRecoveryDistribution(SupportRecoveryDistributionKindRole),
    OrdinaryResult(OrdinaryResultKindRole),
}

impl JsonSchema for ArtifactKindRole {
    fn schema_name() -> Cow<'static, str> {
        "ArtifactKindRole".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<BaselineDistributionKindRole>(),
            generator.subschema_for::<RefreshDistributionKindRole>(),
            generator.subschema_for::<SupportRecoveryDistributionKindRole>(),
            generator.subschema_for::<OrdinaryResultKindRole>(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::{AcceptedArtifactKind, ArtifactKind, ArtifactKindRole, ArtifactRole};
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::{schema_for, JsonSchema};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    fn accepts<T: DeserializeOwned>(value: Value) {
        serde_json::from_value::<T>(value.clone())
            .unwrap_or_else(|error| panic!("contract rejected {value}: {error}"));
    }

    fn rejects<T: DeserializeOwned>(value: Value) {
        assert!(
            serde_json::from_value::<T>(value.clone()).is_err(),
            "contract accepted {value}"
        );
    }

    fn assert_schema_is_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).expect("artifact vocabulary schema must be closed and typed");
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

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .expect("artifact vocabulary schema must compile")
            .is_valid(value)
    }

    #[test]
    fn artifact_vocabularies_have_exact_wire_literals() {
        for role in [
            "baselineDistribution",
            "refreshDistribution",
            "ordinaryResult",
            "supportRecoveryDistribution",
        ] {
            accepts::<ArtifactRole>(json!(role));
        }
        for invalid in [
            "baseline",
            "configurationDistribution",
            "recoveryDistribution",
        ] {
            rejects::<ArtifactRole>(json!(invalid));
        }

        for kind in [
            "configurationDistribution",
            "ordinaryConfiguration",
            "configurationUpdate",
            "invalidArtifact",
        ] {
            accepts::<ArtifactKind>(json!(kind));
        }
        for invalid in ["distribution", "ordinaryResult", "invalid"] {
            rejects::<ArtifactKind>(json!(invalid));
        }

        for kind in ["configurationDistribution", "ordinaryConfiguration"] {
            accepts::<AcceptedArtifactKind>(json!(kind));
        }
        for invalid in ["configurationUpdate", "invalidArtifact"] {
            rejects::<AcceptedArtifactKind>(json!(invalid));
        }
    }

    #[test]
    fn artifact_kind_role_is_exactly_the_four_workflow_pairs() {
        for (kind, role) in [
            ("configurationDistribution", "baselineDistribution"),
            ("configurationDistribution", "refreshDistribution"),
            ("configurationDistribution", "supportRecoveryDistribution"),
            ("ordinaryConfiguration", "ordinaryResult"),
        ] {
            accepts::<ArtifactKindRole>(json!({ "kind": kind, "role": role }));
        }

        for (kind, role) in [
            ("configurationDistribution", "ordinaryResult"),
            ("ordinaryConfiguration", "baselineDistribution"),
            ("configurationUpdate", "refreshDistribution"),
            ("invalidArtifact", "ordinaryResult"),
        ] {
            rejects::<ArtifactKindRole>(json!({ "kind": kind, "role": role }));
        }
        rejects::<ArtifactKindRole>(json!({
            "kind": "configurationDistribution",
            "role": "baselineDistribution",
            "extra": true
        }));
        rejects::<ArtifactKindRole>(json!({ "kind": "configurationDistribution" }));
    }

    #[test]
    fn artifact_vocabulary_schemas_are_closed_and_exact() {
        assert_schema_is_closed::<ArtifactRole>();
        assert_schema_is_closed::<ArtifactKind>();
        assert_schema_is_closed::<AcceptedArtifactKind>();
        assert_schema_is_closed::<ArtifactKindRole>();
        assert_exact_one_of::<ArtifactKindRole>(4);

        let role_schema = serde_json::to_value(schema_for!(ArtifactRole)).unwrap();
        assert_eq!(
            role_schema["enum"],
            json!([
                "baselineDistribution",
                "refreshDistribution",
                "ordinaryResult",
                "supportRecoveryDistribution"
            ])
        );
        let kind_schema = serde_json::to_value(schema_for!(ArtifactKind)).unwrap();
        assert_eq!(
            kind_schema["enum"],
            json!([
                "configurationDistribution",
                "ordinaryConfiguration",
                "configurationUpdate",
                "invalidArtifact"
            ])
        );
        let accepted_schema = serde_json::to_value(schema_for!(AcceptedArtifactKind)).unwrap();
        assert_eq!(
            accepted_schema["enum"],
            json!(["configurationDistribution", "ordinaryConfiguration"])
        );

        for valid in [
            json!({ "kind": "configurationDistribution", "role": "baselineDistribution" }),
            json!({ "kind": "configurationDistribution", "role": "refreshDistribution" }),
            json!({ "kind": "configurationDistribution", "role": "supportRecoveryDistribution" }),
            json!({ "kind": "ordinaryConfiguration", "role": "ordinaryResult" }),
        ] {
            assert!(schema_accepts::<ArtifactKindRole>(&valid));
        }
        for invalid in [
            json!({ "kind": "configurationDistribution", "role": "ordinaryResult" }),
            json!({ "kind": "ordinaryConfiguration", "role": "baselineDistribution" }),
            json!({ "kind": "configurationUpdate", "role": "refreshDistribution" }),
            json!({ "kind": "configurationDistribution", "role": "baselineDistribution", "extra": true }),
        ] {
            assert!(
                !schema_accepts::<ArtifactKindRole>(&invalid),
                "artifact schema accepted {invalid}"
            );
        }
    }
}

use super::scalars::{EmptyOrName, Name};
use super::schema::{one_of_schema, string_schema};
use crate::domain::branched_development::{MetadataObjectId, ProjectId, UnicaId};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

const MAX_I_JSON_RESULT_COUNT: u64 = 9_007_199_254_740_991;
const ARCHIVE_SCHEMA_VERSION: &str = "branchedArchiveV1";
const PLATFORM_VERSION_COMPONENT_PATTERN: &str = concat!(
    "(?:0|[1-9][0-9]{0,8}|[1-3][0-9]{9}|4[0-1][0-9]{8}|",
    "42[0-8][0-9]{7}|429[0-3][0-9]{6}|4294[0-8][0-9]{5}|",
    "42949[0-5][0-9]{4}|429496[0-6][0-9]{3}|4294967[0-1][0-9]{2}|",
    "42949672[0-8][0-9]|429496729[0-5])"
);
const ARCHIVE_ENTRY_PATTERN: &str = concat!(
    "^[A-Za-z0-9](?:[A-Za-z0-9._-]{0,126}[A-Za-z0-9])?",
    "(?:/[A-Za-z0-9](?:[A-Za-z0-9._-]{0,126}[A-Za-z0-9])?)*$"
);
const ARCHIVE_DEVICE_SEGMENT_PATTERN: &str = concat!(
    "(?:^|/)(?:[Cc][Oo][Nn]|[Pp][Rr][Nn]|[Aa][Uu][Xx]|[Nn][Uu][Ll]|",
    "[Cc][Oo][Mm][1-9]|[Ll][Pp][Tt][1-9])",
    "(?:\\.[A-Za-z0-9._-]*[A-Za-z0-9])?(?:/|$)"
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactVocabularyError {
    kind: &'static str,
    reason: &'static str,
}

impl ArtifactVocabularyError {
    const fn new(kind: &'static str, reason: &'static str) -> Self {
        Self { kind, reason }
    }
}

impl fmt::Display for ArtifactVocabularyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {}: {}", self.kind, self.reason)
    }
}

impl std::error::Error for ArtifactVocabularyError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct PlatformVersion(String);

impl PlatformVersion {
    pub(crate) fn parse(value: &str) -> Result<Self, ArtifactVocabularyError> {
        let mut components = value.split('.');
        for _ in 0..4 {
            let component = components.next().ok_or_else(|| {
                ArtifactVocabularyError::new(
                    "platform version",
                    "must contain exactly four decimal components",
                )
            })?;
            if component.is_empty()
                || component.len() > 1 && component.starts_with('0')
                || !component.bytes().all(|byte| byte.is_ascii_digit())
                || component.parse::<u32>().is_err()
            {
                return Err(ArtifactVocabularyError::new(
                    "platform version",
                    "contains a non-canonical u32 component",
                ));
            }
        }
        if components.next().is_some() {
            return Err(ArtifactVocabularyError::new(
                "platform version",
                "must contain exactly four decimal components",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlatformVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PlatformVersion {
    type Err = ArtifactVocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for PlatformVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for PlatformVersion {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "PlatformVersion".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let component = PLATFORM_VERSION_COMPONENT_PATTERN;
        let pattern = format!("^{component}\\.{component}\\.{component}\\.{component}$");
        string_schema(7, 43, Some(&pattern), None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct CompatibilityMode(String);

impl CompatibilityMode {
    pub(crate) fn parse(value: &str) -> Result<Self, ArtifactVocabularyError> {
        let mut bytes = value.bytes();
        let first = bytes.next().ok_or_else(|| {
            ArtifactVocabularyError::new("compatibility mode", "must not be empty")
        })?;
        if value.len() > 128
            || !first.is_ascii_alphabetic()
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(ArtifactVocabularyError::new(
                "compatibility mode",
                "must match [A-Za-z][A-Za-z0-9_]{0,127}",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CompatibilityMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CompatibilityMode {
    type Err = ArtifactVocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for CompatibilityMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for CompatibilityMode {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "CompatibilityMode".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        string_schema(1, 128, Some("^[A-Za-z][A-Za-z0-9_]{0,127}$"), None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct SafeResultCount(u64);

impl SafeResultCount {
    pub(crate) fn new(value: u64) -> Result<Self, ArtifactVocabularyError> {
        (value <= MAX_I_JSON_RESULT_COUNT)
            .then_some(Self(value))
            .ok_or_else(|| {
                ArtifactVocabularyError::new(
                    "result count",
                    "must be an I-JSON interoperable integer",
                )
            })
    }

    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for SafeResultCount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for SafeResultCount {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "SafeResultCount".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 0,
            "maximum": MAX_I_JSON_RESULT_COUNT,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct ArchiveEntryName(String);

impl ArchiveEntryName {
    pub(crate) fn parse(value: &str) -> Result<Self, ArtifactVocabularyError> {
        if value.is_empty() || value.len() > 1024 || !value.is_ascii() {
            return Err(ArtifactVocabularyError::new(
                "archive entry name",
                "must contain 1-1024 portable ASCII bytes",
            ));
        }
        for segment in value.split('/') {
            let bytes = segment.as_bytes();
            if bytes.is_empty()
                || bytes.len() > 128
                || !bytes.first().is_some_and(u8::is_ascii_alphanumeric)
                || !bytes.last().is_some_and(u8::is_ascii_alphanumeric)
                || !bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return Err(ArtifactVocabularyError::new(
                    "archive entry name",
                    "contains a non-portable segment",
                ));
            }
            let basename = segment.split('.').next().unwrap_or_default();
            if is_windows_device_basename(basename) {
                return Err(ArtifactVocabularyError::new(
                    "archive entry name",
                    "contains a reserved Windows device basename",
                ));
            }
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_windows_device_basename(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.eq_ignore_ascii_case("CON")
        || value.eq_ignore_ascii_case("PRN")
        || value.eq_ignore_ascii_case("AUX")
        || value.eq_ignore_ascii_case("NUL")
        || bytes.len() == 4
            && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
            && matches!(bytes[3], b'1'..=b'9')
}

impl fmt::Display for ArchiveEntryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ArchiveEntryName {
    type Err = ArtifactVocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for ArchiveEntryName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::parse(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl JsonSchema for ArchiveEntryName {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ArchiveEntryName".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "maxLength": 1024,
            "pattern": ARCHIVE_ENTRY_PATTERN,
            "not": {
                "type": "string",
                "pattern": ARCHIVE_DEVICE_SEGMENT_PATTERN,
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ArchiveSchemaVersion;

impl ArchiveSchemaVersion {
    pub(crate) const fn current() -> Self {
        Self
    }

    pub(crate) const fn as_str(self) -> &'static str {
        ARCHIVE_SCHEMA_VERSION
    }
}

impl Serialize for ArchiveSchemaVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ArchiveSchemaVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        (value == ARCHIVE_SCHEMA_VERSION)
            .then_some(Self)
            .ok_or_else(|| D::Error::custom("expected archive schema version branchedArchiveV1"))
    }
}

impl JsonSchema for ArchiveSchemaVersion {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ArchiveSchemaVersion".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "const": ARCHIVE_SCHEMA_VERSION })
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OriginalInfobaseKind {
    File,
    ClientServer,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RepositoryTransport {
    File,
    Server,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TargetKind {
    Task,
    Original,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConfigurationIdentity {
    metadata_uuid: MetadataObjectId,
    name: Name,
    vendor: EmptyOrName,
    version: EmptyOrName,
}

impl ConfigurationIdentity {
    pub(crate) fn new(
        metadata_uuid: MetadataObjectId,
        name: Name,
        vendor: EmptyOrName,
        version: EmptyOrName,
    ) -> Self {
        Self {
            metadata_uuid,
            name,
            vendor,
            version,
        }
    }

    pub(crate) const fn metadata_uuid(&self) -> &MetadataObjectId {
        &self.metadata_uuid
    }

    pub(crate) const fn name(&self) -> &Name {
        &self.name
    }

    pub(crate) const fn vendor(&self) -> &EmptyOrName {
        &self.vendor
    }

    pub(crate) const fn version(&self) -> &EmptyOrName {
        &self.version
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OwnedTargetRole {
    InstanceRoot,
    TaskInfobase,
    TaskWorkspace,
    Probe,
    Sandbox,
    Artifact,
    Quarantine,
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnedTargetLocator {
    project_id: ProjectId,
    instance_id: UnicaId,
    role: OwnedTargetRole,
}

impl OwnedTargetLocator {
    pub(crate) fn new(project_id: ProjectId, instance_id: UnicaId, role: OwnedTargetRole) -> Self {
        Self {
            project_id,
            instance_id,
            role,
        }
    }

    pub(crate) const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub(crate) const fn instance_id(&self) -> &UnicaId {
        &self.instance_id
    }

    pub(crate) const fn role(&self) -> OwnedTargetRole {
        self.role
    }
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
    use super::{
        AcceptedArtifactKind, ArchiveEntryName, ArchiveSchemaVersion, ArtifactKind,
        ArtifactKindRole, ArtifactRole, CompatibilityMode, ConfigurationIdentity,
        OriginalInfobaseKind, OwnedTargetLocator, OwnedTargetRole, PlatformVersion,
        RepositoryTransport, SafeResultCount, TargetKind,
    };
    use crate::domain::branched_development::contracts::scalars::{EmptyOrName, Name};
    use crate::domain::branched_development::contracts::schema::{
        audit_json_schema, is_i_json_lf_text, is_i_json_single_line_text,
        is_normalized_utc_instant, I_JSON_LF_TEXT_FORMAT, I_JSON_SINGLE_LINE_TEXT_FORMAT,
        NORMALIZED_UTC_INSTANT_FORMAT,
    };
    use crate::domain::branched_development::{MetadataObjectId, ProjectId, UnicaId};
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
            .with_format(I_JSON_SINGLE_LINE_TEXT_FORMAT, is_i_json_single_line_text)
            .with_format(I_JSON_LF_TEXT_FORMAT, is_i_json_lf_text)
            .with_format(NORMALIZED_UTC_INSTANT_FORMAT, is_normalized_utc_instant)
            .should_validate_formats(true)
            .should_ignore_unknown_formats(false)
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

    #[test]
    fn configuration_identity_requires_exact_bounded_identity_fields() {
        let valid = json!({
            "metadataUuid": "123e4567-e89b-12d3-a456-426614174000",
            "name": "Demo configuration",
            "vendor": "",
            "version": "8.3.27"
        });
        accepts::<ConfigurationIdentity>(valid.clone());
        assert!(schema_accepts::<ConfigurationIdentity>(&valid));

        for field in ["vendor", "version"] {
            let mut boundary = valid.clone();
            boundary[field] = json!("界".repeat(256));
            accepts::<ConfigurationIdentity>(boundary.clone());
            assert!(schema_accepts::<ConfigurationIdentity>(&boundary));
        }

        let mut invalid_values = vec![
            json!({
                "metadataUuid": "123e4567-e89b-12d3-a456-426614174000",
                "name": "Demo configuration",
                "version": "8.3.27"
            }),
            json!({
                "metadataUuid": "123e4567-e89b-12d3-a456-426614174000",
                "name": "Demo configuration",
                "vendor": null,
                "version": "8.3.27"
            }),
            json!({
                "metadataUuid": "123e4567-e89b-12d3-a456-426614174000",
                "name": "Demo configuration",
                "vendor": "vendor\nname",
                "version": "8.3.27"
            }),
            json!({
                "metadataUuid": "123e4567-e89b-12d3-a456-426614174000",
                "name": "Demo configuration",
                "vendor": "",
                "version": "8.3.27",
                "path": "/forbidden"
            }),
        ];
        for field in ["vendor", "version"] {
            let mut omitted = valid.clone();
            omitted.as_object_mut().unwrap().remove(field);
            invalid_values.push(omitted);

            let mut null = valid.clone();
            null[field] = Value::Null;
            invalid_values.push(null);

            let mut too_long = valid.clone();
            too_long[field] = json!("界".repeat(257));
            invalid_values.push(too_long);

            let mut control = valid.clone();
            control[field] = json!("invalid\tvalue");
            invalid_values.push(control);
        }
        for invalid in invalid_values {
            rejects::<ConfigurationIdentity>(invalid.clone());
            assert!(!schema_accepts::<ConfigurationIdentity>(&invalid));
        }
        assert_schema_is_closed::<ConfigurationIdentity>();
    }

    #[test]
    fn owned_target_locator_is_logical_and_has_the_exact_roles() {
        for role in [
            "instanceRoot",
            "taskInfobase",
            "taskWorkspace",
            "probe",
            "sandbox",
            "artifact",
            "quarantine",
        ] {
            let locator = json!({
                "projectId": "123e4567-e89b-12d3-a456-426614174000",
                "instanceId": "123e4567-e89b-12d3-a456-426614174001",
                "role": role
            });
            accepts::<OwnedTargetLocator>(locator.clone());
            assert!(schema_accepts::<OwnedTargetLocator>(&locator));
        }
        for invalid in [
            json!({
                "projectId": "123e4567-e89b-12d3-a456-426614174000",
                "instanceId": "123e4567-e89b-12d3-a456-426614174001",
                "role": "stateRoot"
            }),
            json!({
                "projectId": "123e4567-e89b-12d3-a456-426614174000",
                "instanceId": "123e4567-e89b-12d3-a456-426614174001",
                "role": "artifact",
                "path": "/forbidden"
            }),
        ] {
            rejects::<OwnedTargetLocator>(invalid.clone());
            assert!(!schema_accepts::<OwnedTargetLocator>(&invalid));
        }
        assert_schema_is_closed::<OwnedTargetLocator>();
    }

    #[test]
    fn configuration_identity_has_typed_construction_and_accessors() {
        let metadata_uuid =
            MetadataObjectId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let identity = ConfigurationIdentity::new(
            metadata_uuid.clone(),
            Name::parse("Demo configuration").unwrap(),
            EmptyOrName::parse("Demo vendor").unwrap(),
            EmptyOrName::parse("8.3.27").unwrap(),
        );

        assert_eq!(identity.metadata_uuid(), &metadata_uuid);
        assert_eq!(identity.name().as_str(), "Demo configuration");
        assert_eq!(identity.vendor().as_str(), "Demo vendor");
        assert_eq!(identity.version().as_str(), "8.3.27");
        assert_eq!(
            serde_json::to_value(identity).unwrap(),
            json!({
                "metadataUuid": "123e4567-e89b-12d3-a456-426614174000",
                "name": "Demo configuration",
                "vendor": "Demo vendor",
                "version": "8.3.27",
            })
        );
    }

    #[test]
    fn owned_target_locator_uses_canonical_typed_ordering() {
        let project_a = ProjectId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let project_b = ProjectId::parse("223e4567-e89b-42d3-a456-426614174000").unwrap();
        let instance_a = UnicaId::parse("123e4567-e89b-42d3-a456-426614174001").unwrap();
        let instance_b = UnicaId::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let mut locators = [
            OwnedTargetLocator::new(
                project_b.clone(),
                instance_a.clone(),
                OwnedTargetRole::InstanceRoot,
            ),
            OwnedTargetLocator::new(project_a.clone(), instance_b, OwnedTargetRole::InstanceRoot),
            OwnedTargetLocator::new(
                project_a.clone(),
                instance_a.clone(),
                OwnedTargetRole::TaskInfobase,
            ),
            OwnedTargetLocator::new(
                project_a.clone(),
                instance_a.clone(),
                OwnedTargetRole::InstanceRoot,
            ),
        ];

        locators.sort();

        assert_eq!(locators[0].project_id(), &project_a);
        assert_eq!(locators[0].instance_id(), &instance_a);
        assert_eq!(locators[0].role(), OwnedTargetRole::InstanceRoot);
        assert_eq!(locators[1].role(), OwnedTargetRole::TaskInfobase);
        assert_eq!(locators[2].project_id(), &project_a);
        assert_eq!(locators[3].project_id(), &project_b);
    }

    #[test]
    fn task12_platform_version_is_a_canonical_four_u32_tuple_with_schema_parity() {
        for value in [
            "0.0.0.0",
            "8.3.27.2074",
            "4294967295.4294967295.4294967295.4294967295",
        ] {
            let parsed = PlatformVersion::parse(value).unwrap();
            assert_eq!(parsed.as_str(), value);
            accepts::<PlatformVersion>(json!(value));
            assert!(schema_accepts::<PlatformVersion>(&json!(value)));
        }

        for value in [
            "",
            "8.3.27",
            "8.3.27.2074.1",
            "08.3.27.2074",
            "8.03.27.2074",
            "8.3.27.4294967296",
            "+8.3.27.2074",
            "8.3.27.-1",
            "8.3.27.２０７４",
            "8.3.27.2074\n",
        ] {
            assert!(PlatformVersion::parse(value).is_err(), "accepted {value}");
            rejects::<PlatformVersion>(json!(value));
            assert!(
                !schema_accepts::<PlatformVersion>(&json!(value)),
                "schema accepted {value}"
            );
        }
        assert_schema_is_closed::<PlatformVersion>();
    }

    #[test]
    fn task12_compatibility_mode_is_bounded_ascii_with_schema_parity() {
        let max = format!("A{}", "_".repeat(127));
        let too_long = format!("A{}", "_".repeat(128));
        for value in ["A", "Version8_3", max.as_str()] {
            let parsed = CompatibilityMode::parse(value).unwrap();
            assert_eq!(parsed.as_str(), value);
            accepts::<CompatibilityMode>(json!(value));
            assert!(schema_accepts::<CompatibilityMode>(&json!(value)));
        }

        for value in [
            "",
            "8_3",
            "Version-8",
            "Версия8_3",
            "Version8_3\n",
            too_long.as_str(),
        ] {
            assert!(CompatibilityMode::parse(value).is_err(), "accepted {value}");
            rejects::<CompatibilityMode>(json!(value));
            assert!(
                !schema_accepts::<CompatibilityMode>(&json!(value)),
                "schema accepted {value}"
            );
        }
        assert_schema_is_closed::<CompatibilityMode>();
    }

    #[test]
    fn task12_safe_result_count_is_an_i_json_integer() {
        for value in [0_u64, 1, 9_007_199_254_740_991] {
            let parsed = SafeResultCount::new(value).unwrap();
            assert_eq!(parsed.get(), value);
            accepts::<SafeResultCount>(json!(value));
            assert!(schema_accepts::<SafeResultCount>(&json!(value)));
        }

        assert!(SafeResultCount::new(9_007_199_254_740_992).is_err());
        for invalid in [json!(-1), json!(9_007_199_254_740_992_u64)] {
            rejects::<SafeResultCount>(invalid.clone());
            assert!(!schema_accepts::<SafeResultCount>(&invalid));
        }
        // Draft 2020-12 defines `integer` mathematically, so a parsed JSON
        // instance spelled `1.0` satisfies the schema even though Serde's u64
        // boundary deliberately rejects that forbidden wire spelling.
        assert!(serde_json::from_str::<SafeResultCount>("1.0").is_err());
        rejects::<SafeResultCount>(json!(1.0));
        assert!(schema_accepts::<SafeResultCount>(&json!(1.0)));
        assert_schema_is_closed::<SafeResultCount>();
    }

    #[test]
    fn task12_archive_entry_name_rejects_traversal_devices_and_non_ascii_with_schema_parity() {
        let max_segment = format!("a{}z", "_".repeat(126));
        for value in [
            "a",
            "manifest.json",
            "nested/member-01.bin",
            max_segment.as_str(),
        ] {
            let parsed = ArchiveEntryName::parse(value).unwrap();
            assert_eq!(parsed.as_str(), value);
            accepts::<ArchiveEntryName>(json!(value));
            assert!(schema_accepts::<ArchiveEntryName>(&json!(value)));
        }

        let too_long_segment = format!("a{}z", "_".repeat(127));
        let too_long_name = format!("a/{}", "b".repeat(1023));
        for value in [
            "",
            "/root",
            "root/",
            "a//b",
            ".",
            "..",
            "a/../b",
            "a\\b",
            "C:/member",
            "C:member",
            "a b",
            "a\n",
            "данные.bin",
            "CON",
            "con.txt",
            "dir/PrN.log",
            "AUX.tar.gz",
            "nul",
            "COM1.bin",
            "com9",
            "LPT1.txt",
            "lpt9",
            too_long_segment.as_str(),
            too_long_name.as_str(),
        ] {
            assert!(ArchiveEntryName::parse(value).is_err(), "accepted {value}");
            rejects::<ArchiveEntryName>(json!(value));
            assert!(
                !schema_accepts::<ArchiveEntryName>(&json!(value)),
                "schema accepted {value}"
            );
        }
        assert_schema_is_closed::<ArchiveEntryName>();
    }

    #[test]
    fn task12_archive_schema_version_is_the_exact_literal() {
        let version = ArchiveSchemaVersion::current();
        assert_eq!(version.as_str(), "branchedArchiveV1");
        assert_eq!(
            serde_json::to_value(version).unwrap(),
            json!("branchedArchiveV1")
        );
        accepts::<ArchiveSchemaVersion>(json!("branchedArchiveV1"));
        rejects::<ArchiveSchemaVersion>(json!("branchedArchiveV2"));
        assert!(schema_accepts::<ArchiveSchemaVersion>(&json!(
            "branchedArchiveV1"
        )));
        assert!(!schema_accepts::<ArchiveSchemaVersion>(&json!(
            "branchedArchiveV2"
        )));
        assert_schema_is_closed::<ArchiveSchemaVersion>();
    }

    #[test]
    fn task12_source_topology_enums_have_only_the_normative_literals() {
        for value in ["file", "clientServer"] {
            accepts::<OriginalInfobaseKind>(json!(value));
            assert!(schema_accepts::<OriginalInfobaseKind>(&json!(value)));
        }
        for value in ["server", "local", "client-server"] {
            rejects::<OriginalInfobaseKind>(json!(value));
        }

        for value in ["file", "server"] {
            accepts::<RepositoryTransport>(json!(value));
            assert!(schema_accepts::<RepositoryTransport>(&json!(value)));
        }
        for value in ["clientServer", "http", "local"] {
            rejects::<RepositoryTransport>(json!(value));
        }
        assert_schema_is_closed::<OriginalInfobaseKind>();
        assert_schema_is_closed::<RepositoryTransport>();

        for value in ["task", "original"] {
            accepts::<TargetKind>(json!(value));
            assert!(schema_accepts::<TargetKind>(&json!(value)));
        }
        for value in ["repository", "main", "development"] {
            rejects::<TargetKind>(json!(value));
        }
        assert_schema_is_closed::<TargetKind>();
    }
}

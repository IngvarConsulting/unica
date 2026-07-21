use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierError {
    kind: &'static str,
    reason: &'static str,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {}: {}", self.kind, self.reason)
    }
}

impl std::error::Error for IdentifierError {}

fn validated_string(
    value: &str,
    kind: &'static str,
    reason: &'static str,
    valid: impl FnOnce(&str) -> bool,
) -> Result<String, IdentifierError> {
    valid(value)
        .then(|| value.to_owned())
        .ok_or(IdentifierError { kind, reason })
}

fn valid_task_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b'-'))
}

fn valid_operation_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .map(|parsed| parsed.hyphenated().to_string() == value)
        .unwrap_or(false)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

macro_rules! identifier {
    ($name:ident, $kind:literal, $reason:literal, $valid:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: &str) -> Result<Self, IdentifierError> {
                validated_string(value, $kind, $reason, $valid).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct IdentifierVisitor;

                impl serde::de::Visitor<'_> for IdentifierVisitor {
                    type Value = $name;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str($reason)
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
                    where
                        E: serde::de::Error,
                    {
                        $name::parse(value).map_err(E::custom)
                    }
                }

                deserializer.deserialize_str(IdentifierVisitor)
            }
        }
    };
}

identifier!(
    TaskId,
    "task id",
    "must be a bounded ASCII task identifier",
    valid_task_id
);
identifier!(
    OperationId,
    "operation id",
    "must be a canonical lowercase hyphenated UUID",
    valid_operation_id
);
identifier!(
    Sha256Digest,
    "SHA-256 digest",
    "must be an exact lowercase SHA-256 hex digest",
    valid_sha256
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn task_id_accepts_only_the_bounded_ascii_contract() {
        assert_eq!(TaskId::from_str("TASK-142").unwrap().as_str(), "TASK-142");
        assert!(TaskId::from_str("a").is_ok());
        assert!(TaskId::from_str(&format!("A{}", "_".repeat(63))).is_ok());

        for invalid in ["", ".task", "task/name", "задача"] {
            assert!(TaskId::from_str(invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(TaskId::from_str(&format!("A{}", "_".repeat(64))).is_err());
    }

    #[test]
    fn operation_id_requires_canonical_lowercase_hyphenated_uuid() {
        let canonical = "123e4567-e89b-12d3-a456-426614174000";
        assert_eq!(
            OperationId::from_str(canonical).unwrap().as_str(),
            canonical
        );

        for invalid in [
            "123E4567-E89B-12D3-A456-426614174000",
            "123e4567e89b12d3a456426614174000",
            "{123e4567-e89b-12d3-a456-426614174000}",
            "not-a-uuid",
        ] {
            assert!(
                OperationId::from_str(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn sha256_digest_requires_exact_lowercase_hex() {
        let canonical = "0123456789abcdef".repeat(4);
        assert_eq!(
            Sha256Digest::from_str(&canonical).unwrap().as_str(),
            canonical
        );

        for invalid in [
            "0".repeat(63),
            "0".repeat(65),
            "G".repeat(64),
            "A".repeat(64),
        ] {
            assert!(
                Sha256Digest::from_str(&invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn identifier_json_is_transparent_and_deserialization_revalidates() {
        let task = TaskId::from_str("TASK-142").unwrap();
        assert_eq!(serde_json::to_string(&task).unwrap(), "\"TASK-142\"");
        assert_eq!(
            serde_json::from_str::<TaskId>("\"TASK-142\"").unwrap(),
            task
        );
        assert!(serde_json::from_str::<TaskId>("\"../task\"").is_err());
        assert!(serde_json::from_str::<OperationId>("\"NOT-A-UUID\"").is_err());
    }
}

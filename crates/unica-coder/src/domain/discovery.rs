use std::fmt;

pub(crate) const MAX_ARTIFACT_ID_BYTES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ArtifactId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactIdError {
    InvalidFormat,
    NormalizedBytesOutOfRange,
}

pub(crate) fn normalize_discovery_identity(value: &str) -> String {
    value.trim().chars().flat_map(char::to_lowercase).collect()
}

impl ArtifactId {
    pub(crate) fn parse(value: &str) -> Result<Self, ArtifactIdError> {
        let trimmed = value.trim();
        if trimmed.contains(['/', '\\']) || trimmed.starts_with('.') || trimmed.ends_with('.') {
            return Err(ArtifactIdError::InvalidFormat);
        }

        let mut segments = trimmed.split('.');
        let Some(kind) = segments.next() else {
            return Err(ArtifactIdError::InvalidFormat);
        };
        let Some(name) = segments.next() else {
            return Err(ArtifactIdError::InvalidFormat);
        };
        if kind.is_empty() || name.is_empty() || segments.any(str::is_empty) {
            return Err(ArtifactIdError::InvalidFormat);
        }

        let normalized = normalize_discovery_identity(trimmed);
        if !(1..=MAX_ARTIFACT_ID_BYTES).contains(&normalized.len()) {
            return Err(ArtifactIdError::NormalizedBytesOutOfRange);
        }
        Ok(Self(normalized))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => formatter.write_str("invalid canonical artifact identifier"),
            Self::NormalizedBytesOutOfRange => formatter
                .write_str("normalized artifact identifier must contain 1..=1024 UTF-8 bytes"),
        }
    }
}

impl std::error::Error for ArtifactIdError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_identity_uses_trim_plus_rust_unicode_lowercase_mapping() {
        let mixed = ArtifactId::parse(" Document.Order ").expect("mixed-case artifact");
        let lower = ArtifactId::parse("document.order").expect("lowercase artifact");

        assert_eq!(mixed.as_str(), "document.order");
        assert_eq!(mixed, lower);
    }

    #[test]
    fn discovery_identity_is_trim_plus_rust_unicode_lowercase_mapping() {
        assert_eq!(normalize_discovery_identity("  SERIES  "), "series");
        assert_eq!(normalize_discovery_identity("  СЕРИИ  "), "серии");
        assert_eq!(normalize_discovery_identity("Straße"), "straße");
        assert_ne!(
            normalize_discovery_identity("Straße"),
            normalize_discovery_identity("STRASSE")
        );
    }

    #[test]
    fn artifact_rejects_unicode_lowercase_expansion_beyond_byte_limit() {
        let raw = format!("K.{}", "\u{0130}".repeat(511));
        assert_eq!(raw.len(), MAX_ARTIFACT_ID_BYTES);

        let error = ArtifactId::parse(&raw).unwrap_err();

        assert_eq!(error, ArtifactIdError::NormalizedBytesOutOfRange);
    }

    #[test]
    fn artifact_order_uses_the_normalized_identity() {
        let mut artifacts = [
            ArtifactId::parse("Document.Zed").expect("zed artifact"),
            ArtifactId::parse("document.Alpha").expect("alpha artifact"),
        ];

        artifacts.sort();

        assert_eq!(artifacts[0].as_str(), "document.alpha");
        assert_eq!(artifacts[1].as_str(), "document.zed");
    }
}

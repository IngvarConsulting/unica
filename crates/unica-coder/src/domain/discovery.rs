use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ArtifactId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactIdError {
    InvalidFormat,
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

        let normalized = trimmed
            .chars()
            .flat_map(char::to_lowercase)
            .collect::<String>();
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
        }
    }
}

impl std::error::Error for ArtifactIdError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_identity_is_trimmed_and_case_normalized() {
        let mixed = ArtifactId::parse(" Document.Order ").expect("mixed-case artifact");
        let lower = ArtifactId::parse("document.order").expect("lowercase artifact");

        assert_eq!(mixed.as_str(), "document.order");
        assert_eq!(mixed, lower);
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

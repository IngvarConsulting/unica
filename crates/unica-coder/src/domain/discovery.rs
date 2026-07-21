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

        Ok(Self(trimmed.to_string()))
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

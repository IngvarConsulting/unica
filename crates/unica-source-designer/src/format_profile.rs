use std::cmp::Ordering;
use std::fmt;
use unica_source_contract::{ACTIVE_PLATFORM_LINE, ACTIVE_SERIALIZATION_FORMAT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatProfile {
    pub platform_line: &'static str,
    pub export_format: &'static str,
}

pub const ACTIVE_FORMAT_PROFILE: FormatProfile = FormatProfile {
    platform_line: ACTIVE_PLATFORM_LINE,
    export_format: ACTIVE_SERIALIZATION_FORMAT,
};

#[derive(Debug, Clone)]
pub struct ExportFormatVersion {
    components: Vec<u32>,
}

impl ExportFormatVersion {
    pub fn parse(raw: &str) -> Result<Self, FormatVersionError> {
        if raw.is_empty() {
            return Err(FormatVersionError::new(raw));
        }
        let components = raw
            .split('.')
            .map(|component| {
                if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(FormatVersionError::new(raw));
                }
                component
                    .parse::<u32>()
                    .map_err(|_| FormatVersionError::new(raw))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { components })
    }
}

impl PartialEq for ExportFormatVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ExportFormatVersion {}

impl Ord for ExportFormatVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let length = self.components.len().max(other.components.len());
        (0..length)
            .map(|index| {
                self.components
                    .get(index)
                    .copied()
                    .unwrap_or_default()
                    .cmp(&other.components.get(index).copied().unwrap_or_default())
            })
            .find(|ordering| *ordering != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for ExportFormatVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for ExportFormatVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = self
            .components
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".");
        formatter.write_str(&text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatCompatibility {
    Older { actual: ExportFormatVersion },
    Supported { actual: ExportFormatVersion },
    Newer { actual: ExportFormatVersion },
}

impl FormatCompatibility {
    pub fn actual(&self) -> &ExportFormatVersion {
        match self {
            Self::Older { actual } | Self::Supported { actual } | Self::Newer { actual } => actual,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Older { .. } => "older",
            Self::Supported { .. } => "supported",
            Self::Newer { .. } => "newer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatVersionError {
    raw: String,
}

impl FormatVersionError {
    fn new(raw: &str) -> Self {
        Self {
            raw: raw.to_string(),
        }
    }

    pub fn code(&self) -> &'static str {
        "formatVersionInvalid"
    }
}

impl fmt::Display for FormatVersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid export format version {:?}", self.raw)
    }
}

impl std::error::Error for FormatVersionError {}

pub fn classify_root_version(raw: Option<&str>) -> Result<FormatCompatibility, FormatVersionError> {
    let raw = raw.unwrap_or("1.0");
    let actual = ExportFormatVersion::parse(raw)?;
    let target = ExportFormatVersion::parse(ACTIVE_FORMAT_PROFILE.export_format)
        .expect("active export format is a valid constant");
    Ok(match actual.cmp(&target) {
        Ordering::Less => FormatCompatibility::Older { actual },
        Ordering::Equal if raw == ACTIVE_FORMAT_PROFILE.export_format => {
            FormatCompatibility::Supported { actual }
        }
        Ordering::Equal => return Err(FormatVersionError::new(raw)),
        Ordering::Greater => FormatCompatibility::Newer { actual },
    })
}

/// Why a write refuses a versioned platform XML root outside the writable
/// profile: the file, the version as written, the profile and what to do.
pub struct WritableProfileRefusal {
    pub code: &'static str,
    pub actual: String,
    pub message: String,
}

/// The refusal for a versioned root recording `raw`, or `None` for the
/// exact writable profile. One wording for every writer that checks it.
pub fn writable_profile_refusal(
    relative: impl fmt::Display,
    raw: Option<&str>,
) -> Option<WritableProfileRefusal> {
    let FormatProfile {
        platform_line,
        export_format,
    } = ACTIVE_FORMAT_PROFILE;
    let (code, actual, message) = match classify_root_version(raw) {
        Ok(FormatCompatibility::Supported { .. }) => return None,
        Ok(FormatCompatibility::Older { actual }) => (
            "formatMigrationAvailable",
            actual.to_string(),
            format!(
                "{relative}: export format {actual} is older than the writable profile {export_format} for 1C {platform_line}; re-export the sources with the platform before editing them"
            ),
        ),
        Ok(FormatCompatibility::Newer { actual }) => (
            "platformVersionUnsupported",
            actual.to_string(),
            format!(
                "{relative}: export format {actual} is newer than the writable profile {export_format} for 1C {platform_line}; Unica does not edit this export yet"
            ),
        ),
        Err(error) => {
            let raw = raw.unwrap_or_default();
            (
                error.code(),
                raw.to_string(),
                format!(
                    "{relative}: export format version {raw} is not the exact writable profile {export_format} for 1C {platform_line}; re-export the sources with the platform before editing them"
                ),
            )
        }
    };
    Some(WritableProfileRefusal {
        code,
        actual,
        message,
    })
}

pub fn format_compatibility_warning(compatibility: &FormatCompatibility) -> String {
    match compatibility {
        FormatCompatibility::Older { actual } => format!(
            "Export format {actual} is older than supported {}. Unica will not migrate it automatically; re-export the source explicitly with 1C:Enterprise {}, then retry.",
            ACTIVE_FORMAT_PROFILE.export_format,
            ACTIVE_FORMAT_PROFILE.platform_line
        ),
        FormatCompatibility::Newer { actual } => format!(
            "Export format {actual} is newer than supported {}; platform 1C 8.5 support is planned in upcoming releases.",
            ACTIVE_FORMAT_PROFILE.export_format
        ),
        FormatCompatibility::Supported { .. } => format!(
            "Export format is supported ({})",
            ACTIVE_FORMAT_PROFILE.export_format
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_root_version, ExportFormatVersion, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
    };

    #[test]
    fn active_profile_is_platform_8_3_27_format_2_20() {
        assert_eq!(ACTIVE_FORMAT_PROFILE.platform_line, "8.3.27");
        assert_eq!(ACTIVE_FORMAT_PROFILE.export_format.to_string(), "2.20");
    }

    #[test]
    fn classifies_missing_and_lower_versions_as_older() {
        assert!(matches!(
            classify_root_version(None).unwrap(),
            FormatCompatibility::Older { .. }
        ));
        assert!(matches!(
            classify_root_version(Some("2.19")).unwrap(),
            FormatCompatibility::Older { .. }
        ));
    }

    #[test]
    fn classifies_target_and_newer_versions() {
        assert!(matches!(
            classify_root_version(Some("2.20")).unwrap(),
            FormatCompatibility::Supported { .. }
        ));
        assert!(matches!(
            classify_root_version(Some("2.21")).unwrap(),
            FormatCompatibility::Newer { .. }
        ));
    }

    #[test]
    fn rejects_numeric_equivalents_of_the_exact_supported_literal() {
        for raw in ["2.20.0", "02.20", "2.020"] {
            let error = classify_root_version(Some(raw))
                .expect_err("only the exact raw literal 2.20 is supported");
            assert_eq!(error.code(), "formatVersionInvalid", "{raw}");
        }
    }

    #[test]
    fn equality_and_ordering_use_the_same_numeric_components() {
        let short = ExportFormatVersion::parse("2.20").unwrap();
        let trailing_zero = ExportFormatVersion::parse("2.20.0").unwrap();

        assert_eq!(short.cmp(&trailing_zero), std::cmp::Ordering::Equal);
        assert_eq!(short, trailing_zero);
        assert_eq!(short.to_string(), "2.20");
        assert_eq!(trailing_zero.to_string(), "2.20.0");
    }

    #[test]
    fn compares_numeric_components_instead_of_decimal_text() {
        assert!(matches!(
            classify_root_version(Some("2.9.1")).unwrap(),
            FormatCompatibility::Older { .. }
        ));
        assert!(matches!(
            classify_root_version(Some("2.100")).unwrap(),
            FormatCompatibility::Newer { .. }
        ));
    }

    #[test]
    fn rejects_non_numeric_versions() {
        assert_eq!(
            classify_root_version(Some("latest")).unwrap_err().code(),
            "formatVersionInvalid"
        );
    }
}

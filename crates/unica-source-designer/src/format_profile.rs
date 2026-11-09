use std::cmp::Ordering;
use std::fmt;
use unica_source_contract::format_identity::{
    FormatFamily, FormatFamilyId, FormatIdentity, FormatKey, PlatformLine,
};
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

/// The Designer export formats codecs write (INV.SOURCE.WRITABLE-PROFILE):
/// those with codec dialects and a reference export, oldest first.
pub const WRITABLE_EXPORT_FORMATS: [&str; 3] = [ACTIVE_SERIALIZATION_FORMAT, "2.21", "2.22"];

/// The newest writable export format and the platform line that writes it.
pub fn newest_writable_profile() -> FormatProfile {
    let export_format = WRITABLE_EXPORT_FORMATS[WRITABLE_EXPORT_FORMATS.len() - 1];
    FormatProfile {
        platform_line: platform_line_for_export_format(export_format)
            .expect("a writable export format has a platform line"),
        export_format,
    }
}

/// The writable export formats as a refusal lists them.
pub fn writable_export_formats_text() -> String {
    WRITABLE_EXPORT_FORMATS.join(", ")
}

/// Which platform line writes each version of the Designer export format
/// (INV.RUNTIME.PLATFORM-LINE). A line writes exactly one version, so the
/// line follows from the version of the dump. Every row is a measured export
/// of its line: 8.3.24, 8.3.25, 8.3.26, 8.3.27, 8.5.1 and 8.5.4.
pub const EXPORT_FORMAT_PLATFORM_LINES: &[(&str, &str)] = &[
    ("2.17", "8.3.24"),
    ("2.18", "8.3.25"),
    ("2.19", "8.3.26"),
    ("2.20", "8.3.27"),
    ("2.21", "8.5.1"),
    ("2.22", "8.5.4"),
];

/// The platform line that writes `export_format`, or `None` for a version
/// outside the table.
pub fn platform_line_for_export_format(export_format: &str) -> Option<&'static str> {
    EXPORT_FORMAT_PLATFORM_LINES
        .iter()
        .find(|(format, _)| *format == export_format)
        .map(|(_, line)| *line)
}

/// The Designer export format family; its wire name is `sourceFormat`'s.
pub const PLATFORM_XML: FormatFamilyId = FormatFamilyId::new("platform_xml");

/// The format of a Designer export recording `export_format` in its roots
/// (INV.SOURCE.FORMAT-IDENTITY): the tuple has the one version of the dump.
pub fn platform_xml_identity(export_format: &str) -> FormatIdentity {
    FormatIdentity::new(PLATFORM_XML, FormatKey::new([export_format]))
}

/// The platform line of a Designer export format: it follows from the version
/// by the table above. `None` for another family or a version outside it.
pub fn platform_xml_line(identity: &FormatIdentity) -> Option<PlatformLine> {
    let [version] = identity.key_in(PLATFORM_XML)?.parts() else {
        return None;
    };
    PlatformLine::parse(platform_line_for_export_format(version)?)
}

/// The order of two Designer export formats; `None` when either belongs to
/// another family or its version does not parse. Only the family orders its
/// tuples (INV.SOURCE.FORMAT-IDENTITY).
pub fn compare_platform_xml(left: &FormatIdentity, right: &FormatIdentity) -> Option<Ordering> {
    let version = |identity: &FormatIdentity| match identity.key_in(PLATFORM_XML)?.parts() {
        [version] => ExportFormatVersion::parse(version).ok(),
        _ => None,
    };
    Some(version(left)?.cmp(&version(right)?))
}

/// Export format versions with a measured reference corpus, oldest first.
const MEASURED_EXPORT_FORMATS: &[&str] = &["2.17", "2.18", "2.19", "2.20", "2.21", "2.22"];

/// The Designer export as a format family (INV.APP.FORMAT-FAMILY-EXTENSION):
/// a tuple of one export format version, its line by the table above.
pub struct PlatformXmlFamily;

impl FormatFamily for PlatformXmlFamily {
    fn id(&self) -> FormatFamilyId {
        PLATFORM_XML
    }

    fn compare(&self, left: &FormatKey, right: &FormatKey) -> Option<Ordering> {
        compare_platform_xml(
            &FormatIdentity::new(PLATFORM_XML, left.clone()),
            &FormatIdentity::new(PLATFORM_XML, right.clone()),
        )
    }

    fn known_keys(&self) -> Vec<FormatKey> {
        MEASURED_EXPORT_FORMATS
            .iter()
            .map(|version| FormatKey::new([*version]))
            .collect()
    }

    fn platform_line(&self, key: &FormatKey) -> Option<PlatformLine> {
        platform_xml_line(&FormatIdentity::new(PLATFORM_XML, key.clone()))
    }
}

/// Whether the four-part platform build `build` belongs to `line`
/// (`8.3.27.2342` to `8.3.27`).
pub fn build_in_platform_line(build: &[u32; 4], line: &str) -> bool {
    PlatformLine::parse(line).is_some_and(|line| line.contains_build(build))
}

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
    if WRITABLE_EXPORT_FORMATS.contains(&raw) {
        return Ok(FormatCompatibility::Supported { actual });
    }
    let writable = |version: &str| {
        ExportFormatVersion::parse(version).expect("a writable export format is a valid constant")
    };
    let oldest = writable(WRITABLE_EXPORT_FORMATS[0]);
    let newest = writable(WRITABLE_EXPORT_FORMATS[WRITABLE_EXPORT_FORMATS.len() - 1]);
    Ok(if actual < oldest {
        FormatCompatibility::Older { actual }
    } else if actual > newest {
        FormatCompatibility::Newer { actual }
    } else {
        // A numerically equal spelling of a writable version, or a version
        // between writable ones: only the literal versions are written.
        return Err(FormatVersionError::new(raw));
    })
}

/// Whether a root recording `raw` is in the export format codecs write.
pub fn is_writable_root_version(raw: Option<&str>) -> bool {
    writable_platform_xml_version(raw).is_some()
}

/// How a validation report names the version of a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootVersionReport {
    Ok(String),
    Warn(String),
    Error(String),
}

/// The validation report line for a root recording `raw` as written.
pub fn root_version_report(raw: Option<&str>) -> RootVersionReport {
    match classify_root_version(raw) {
        Ok(FormatCompatibility::Supported { .. }) => {
            RootVersionReport::Ok(format!("Export format: {}", raw.unwrap_or_default()))
        }
        Ok(compatibility) => RootVersionReport::Warn(format_compatibility_warning(&compatibility)),
        Err(error) => RootVersionReport::Error(error.to_string()),
    }
}

/// The Designer export versions with a dialect of their own: 2.20, and 2.21
/// and 2.22, whose all_kinds corpora every codec reads (#2730).
pub fn platform_xml_dialects() -> Vec<FormatKey> {
    WRITABLE_EXPORT_FORMATS
        .into_iter()
        .map(|version| FormatKey::new([version]))
        .collect()
}

/// The writable Designer export version a root records as `raw`, as a
/// static literal; `None` for any other spelling or version.
pub fn writable_platform_xml_version(raw: Option<&str>) -> Option<&'static str> {
    WRITABLE_EXPORT_FORMATS
        .into_iter()
        .find(|version| raw == Some(*version))
}

/// The writable Designer export version a platform line writes.
pub fn writable_platform_xml_version_of_line(line: PlatformLine) -> Option<&'static str> {
    EXPORT_FORMAT_PLATFORM_LINES
        .iter()
        .find(|(_, writer)| PlatformLine::parse(writer) == Some(line))
        .and_then(|(version, _)| writable_platform_xml_version(Some(version)))
}

/// Which dialect reads a root recording `raw` (INV.SOURCE.WRITABLE-PROFILE);
/// a root without `version` is format 1.0.
pub fn resolve_platform_xml_read(
    raw: Option<&str>,
) -> unica_source_contract::format_identity::ReadResolution {
    unica_source_contract::format_identity::resolve_read(
        &PlatformXmlFamily,
        &FormatKey::new([raw.unwrap_or("1.0")]),
        &platform_xml_dialects(),
    )
}

/// The known Designer export versions, oldest first, as a refusal lists them.
pub fn known_platform_xml_versions() -> Vec<&'static str> {
    MEASURED_EXPORT_FORMATS.to_vec()
}

/// `Ok` for a root in the export format codecs read, else why it is not.
pub fn require_supported_root_version(raw: Option<&str>) -> Result<(), String> {
    match classify_root_version(raw).map_err(|error| error.to_string())? {
        FormatCompatibility::Supported { .. } => Ok(()),
        compatibility => Err(format_compatibility_warning(&compatibility)),
    }
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
    let oldest = ACTIVE_FORMAT_PROFILE;
    let newest = newest_writable_profile();
    let writable = writable_export_formats_text();
    // The version as the owner file records it; a root without one is 1.0.
    let recorded = raw.unwrap_or("1.0");
    let (code, actual, message) = match classify_root_version(raw) {
        Ok(FormatCompatibility::Supported { .. }) => return None,
        Ok(FormatCompatibility::Older { .. }) => (
            "formatMigrationAvailable",
            recorded.to_string(),
            format!(
                "{relative}: export format {recorded} is older than the oldest writable format {} for 1C {}; re-export the sources with the platform before editing them",
                oldest.export_format, oldest.platform_line
            ),
        ),
        Ok(FormatCompatibility::Newer { .. }) => (
            "platformVersionUnsupported",
            recorded.to_string(),
            format!(
                "{relative}: export format {recorded} is newer than the newest writable format {} for 1C {}; Unica does not edit this export yet",
                newest.export_format, newest.platform_line
            ),
        ),
        Err(error) => (
            error.code(),
            recorded.to_string(),
            format!(
                "{relative}: export format version {recorded} is not literally one of the writable formats {writable}; re-export the sources with the platform before editing them"
            ),
        ),
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
            "Export format {actual} is newer than the newest supported {}; Unica does not support this export yet.",
            newest_writable_profile().export_format
        ),
        FormatCompatibility::Supported { actual } => {
            format!("Export format is supported ({actual})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_in_platform_line, classify_root_version, compare_platform_xml,
        platform_line_for_export_format, platform_xml_identity, platform_xml_line,
        ExportFormatVersion, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
        EXPORT_FORMAT_PLATFORM_LINES, PLATFORM_XML,
    };
    use std::cmp::Ordering;
    use unica_source_contract::format_identity::{
        FormatFamilyId, FormatIdentity, FormatKey, PlatformLine,
    };

    /// rule: INV.SOURCE.FORMAT-IDENTITY
    #[test]
    fn a_root_is_writable_and_reported_by_its_version() {
        use super::{
            is_writable_root_version, require_supported_root_version, root_version_report,
            RootVersionReport,
        };
        let writable = ACTIVE_FORMAT_PROFILE.export_format;
        assert!(is_writable_root_version(Some(writable)));
        assert!(!is_writable_root_version(None));
        assert!(!is_writable_root_version(Some("2.19")));
        assert!(!is_writable_root_version(Some("2.23")));
        assert_eq!(
            root_version_report(Some(writable)),
            RootVersionReport::Ok(format!("Export format: {writable}"))
        );
        assert!(matches!(
            root_version_report(Some("2.19")),
            RootVersionReport::Warn(_)
        ));
        assert!(matches!(
            root_version_report(Some("latest")),
            RootVersionReport::Error(_)
        ));
        assert!(require_supported_root_version(Some(writable)).is_ok());
        // 2.21 has a dialect of its own and is read without a finding; a
        // newer version without one is refused.
        assert!(require_supported_root_version(Some("2.21")).is_ok());
        assert!(require_supported_root_version(Some("2.23")).is_err());
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    /// The writable formats are 2.20, 2.21 and 2.22 as written; older and
    /// newer versions and other spellings are refused with their codes.
    #[test]
    fn writable_formats_are_exact_and_bounded_by_2_20_and_2_22() {
        use super::{is_writable_root_version, writable_profile_refusal};
        for version in ["2.20", "2.21", "2.22"] {
            assert!(is_writable_root_version(Some(version)), "{version}");
            assert!(
                writable_profile_refusal("x", Some(version)).is_none(),
                "{version}"
            );
        }
        for (version, code) in [
            ("2.19", "formatMigrationAvailable"),
            ("2.23", "platformVersionUnsupported"),
            ("2.21.0", "formatVersionInvalid"),
            ("02.22", "formatVersionInvalid"),
            ("2.20.5", "formatVersionInvalid"),
        ] {
            assert!(!is_writable_root_version(Some(version)), "{version}");
            let refusal = writable_profile_refusal("x", Some(version)).expect(version);
            assert_eq!(refusal.code, code, "{version}");
            assert_eq!(refusal.actual, version);
        }
        let older = writable_profile_refusal("x", Some("2.19")).unwrap().message;
        assert!(
            older.contains("2.20") && older.contains("8.3.27"),
            "{older}"
        );
        let newer = writable_profile_refusal("x", Some("2.23")).unwrap().message;
        assert!(newer.contains("2.22") && newer.contains("8.5.4"), "{newer}");
    }

    #[test]
    fn a_designer_export_format_is_its_version_and_the_line_follows_from_it() {
        // The family's name is the wire name of `sourceFormat`.
        assert_eq!(PLATFORM_XML.name(), "platform_xml");
        let current = platform_xml_identity("2.20");
        assert_eq!(
            platform_xml_line(&current),
            Some(PlatformLine::new(8, 3, 27))
        );
        assert_eq!(
            platform_xml_line(&platform_xml_identity("2.22")),
            Some(PlatformLine::new(8, 5, 4))
        );
        assert_eq!(platform_xml_line(&platform_xml_identity("2.16")), None);
        assert_ne!(current, platform_xml_identity("2.21"));

        // Another family's tuple with the same version is another format:
        // this family neither reads its line nor orders it.
        let foreign =
            FormatIdentity::new(FormatFamilyId::new("family_b"), FormatKey::new(["2.20"]));
        assert_ne!(current, foreign);
        assert_eq!(platform_xml_line(&foreign), None);
        assert_eq!(compare_platform_xml(&current, &foreign), None);
        assert_eq!(
            compare_platform_xml(&platform_xml_identity("2.17"), &current),
            Some(Ordering::Less)
        );
    }

    /// rule: INV.RUNTIME.PLATFORM-LINE
    #[test]
    fn each_export_format_is_written_by_one_platform_line_and_back() {
        assert_eq!(
            platform_line_for_export_format(ACTIVE_FORMAT_PROFILE.export_format),
            Some(ACTIVE_FORMAT_PROFILE.platform_line)
        );
        for (index, (format, line)) in EXPORT_FORMAT_PLATFORM_LINES.iter().enumerate() {
            for (other_format, other_line) in &EXPORT_FORMAT_PLATFORM_LINES[index + 1..] {
                assert_ne!(format, other_format);
                assert_ne!(line, other_line);
            }
        }
        assert_eq!(platform_line_for_export_format("2.22"), Some("8.5.4"));
        assert_eq!(platform_line_for_export_format("2.16"), None);
    }

    #[test]
    fn a_build_belongs_to_its_three_part_line_only() {
        assert!(build_in_platform_line(&[8, 3, 27, 2342], "8.3.27"));
        assert!(!build_in_platform_line(&[8, 3, 26, 1656], "8.3.27"));
        assert!(!build_in_platform_line(&[8, 5, 4, 1878], "8.5.1"));
        assert!(!build_in_platform_line(&[8, 3, 27, 2342], "8.3"));
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn a_designer_export_is_read_by_the_nearest_dialect_of_its_family() {
        use super::PlatformXmlFamily;
        use unica_source_contract::format_identity::{resolve_read, ReadResolution};
        let dialects = [FormatKey::new(["2.20"])];
        let resolve =
            |version: &str| resolve_read(&PlatformXmlFamily, &FormatKey::new([version]), &dialects);
        let unverified = ReadResolution::Unverified {
            read_as: FormatKey::new(["2.20"]),
        };
        assert_eq!(resolve("2.20"), ReadResolution::Dialect);
        // Known without a dialect of its own, and newer than every known one.
        for version in ["2.17", "2.18", "2.19", "2.21", "2.22", "2.23", "3.0"] {
            assert_eq!(resolve(version), unverified, "{version}");
        }
        // Between known versions, older than the oldest, or not a version.
        for version in ["2.17.5", "2.20.1", "2.16", "1.0", "latest"] {
            assert_eq!(resolve(version), ReadResolution::Unsupported, "{version}");
        }
    }

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
        for writable in ["2.21", "2.22"] {
            assert!(matches!(
                classify_root_version(Some(writable)).unwrap(),
                FormatCompatibility::Supported { .. }
            ));
        }
        assert!(matches!(
            classify_root_version(Some("2.23")).unwrap(),
            FormatCompatibility::Newer { .. }
        ));
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
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

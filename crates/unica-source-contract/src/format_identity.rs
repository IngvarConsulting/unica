//! Identity of a source format: a format family and the tuple of versions in
//! that family's own numbering (INV.SOURCE.FORMAT-IDENTITY).
//!
//! The contract names no family and interprets no version: a family crate
//! builds the identity, orders its tuples and says which platform line a tuple
//! means. Here an identity is only equal or not equal to another one.

use std::fmt;

/// A format family by its wire name, for example the name `sourceFormat`
/// carries. The set of families is open; a family crate declares its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormatFamilyId(&'static str);

impl FormatFamilyId {
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn name(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for FormatFamilyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

/// The tuple of versions of one format, as its family wrote it. It has no
/// order of its own: only its family compares tuples, and tuples of different
/// families are never compared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatKey {
    parts: Vec<String>,
}

impl FormatKey {
    pub fn new(parts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            parts: parts.into_iter().map(Into::into).collect(),
        }
    }

    /// The versions in the family's order; read by the family that wrote them.
    pub fn parts(&self) -> &[String] {
        &self.parts
    }
}

/// A format: a family and a tuple of its versions. Different tuples are
/// different formats, and the same tuple in another family is another format.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatIdentity {
    family: FormatFamilyId,
    key: FormatKey,
}

impl FormatIdentity {
    pub fn new(family: FormatFamilyId, key: FormatKey) -> Self {
        Self { family, key }
    }

    pub fn family(&self) -> FormatFamilyId {
        self.family
    }

    pub fn key(&self) -> &FormatKey {
        &self.key
    }

    /// The key of the format inside `family`, or `None` for another family.
    pub fn key_in(&self, family: FormatFamilyId) -> Option<&FormatKey> {
        (self.family == family).then_some(&self.key)
    }
}

impl fmt::Display for FormatIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}({})", self.family, self.key.parts.join(", "))
    }
}

/// A format family plugged into the product by a declaration
/// (INV.APP.FORMAT-FAMILY-EXTENSION). It alone orders its tuples and knows
/// which platform line a tuple means; the set of families is open.
pub trait FormatFamily: Send + Sync {
    fn id(&self) -> FormatFamilyId;

    /// The order of two tuples of this family; `None` when they are not
    /// comparable or either is not a tuple of this family.
    fn compare(&self, left: &FormatKey, right: &FormatKey) -> Option<std::cmp::Ordering>;

    /// The tuples the family supports: those with a measured reference corpus.
    fn known_keys(&self) -> Vec<FormatKey>;

    /// The platform line of a tuple: one of its versions or derived from them.
    fn platform_line(&self, key: &FormatKey) -> Option<PlatformLine>;
}

/// How a format family reads a tuple (INV.SOURCE.WRITABLE-PROFILE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadResolution {
    /// The tuple has a dialect of its own.
    Dialect,
    /// The tuple is read by the dialect of `read_as`, and every answer says so.
    Unverified { read_as: FormatKey },
    /// The tuple is not read.
    Unsupported,
}

/// Which dialect reads `key` of `family`, given the tuples that have one.
///
/// A known tuple (one with a reference corpus) is read by the nearest dialect
/// not newer than it, or, when every dialect is newer, by the oldest of them.
/// An unknown tuple newer than every known one is read by the nearest dialect
/// not newer than it. Any other unknown tuple — older than the oldest known,
/// between known ones, or not comparable with one — is not read: without a
/// corpus nothing says how the dialect would misread it.
pub fn resolve_read(
    family: &dyn FormatFamily,
    key: &FormatKey,
    dialects: &[FormatKey],
) -> ReadResolution {
    use std::cmp::Ordering;
    let order = |left: &FormatKey, right: &FormatKey| family.compare(left, right);
    if dialects
        .iter()
        .any(|dialect| order(dialect, key) == Some(Ordering::Equal))
    {
        return ReadResolution::Dialect;
    }
    let known = family.known_keys();
    let is_known = known
        .iter()
        .any(|candidate| order(candidate, key) == Some(Ordering::Equal));
    let newer_than_every_known = !known.is_empty()
        && known
            .iter()
            .all(|candidate| order(key, candidate) == Some(Ordering::Greater));
    if !is_known && !newer_than_every_known {
        return ReadResolution::Unsupported;
    }
    // The newest dialect not newer than the tuple: no other such dialect is newer.
    let not_newer = dialects
        .iter()
        .filter(|dialect| order(dialect, key) == Some(Ordering::Less))
        .collect::<Vec<_>>();
    let nearest_older = not_newer.iter().find(|dialect| {
        not_newer
            .iter()
            .all(|other| order(other, dialect) != Some(Ordering::Greater))
    });
    let read_as = match nearest_older {
        Some(dialect) => Some(*dialect),
        None if is_known => {
            let newer = dialects
                .iter()
                .filter(|dialect| order(dialect, key) == Some(Ordering::Greater))
                .collect::<Vec<_>>();
            newer
                .iter()
                .find(|dialect| {
                    newer
                        .iter()
                        .all(|other| order(other, dialect) != Some(Ordering::Less))
                })
                .copied()
        }
        None => None,
    };
    match read_as {
        Some(read_as) => ReadResolution::Unverified {
            read_as: read_as.clone(),
        },
        None => ReadResolution::Unsupported,
    }
}

/// A Designer export format beyond 2.20 that has dialects and a reference
/// corpus: a row of the source profile (#2730).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlatformXmlRelease {
    /// Format 2.21, written by platform line 8.5.1.
    Format2_21,
    /// Format 2.22, written by platform line 8.5.4.
    Format2_22,
}

impl PlatformXmlRelease {
    /// The release a root recording `raw` exactly belongs to.
    pub fn for_export_format(raw: &str) -> Option<Self> {
        [Self::Format2_21, Self::Format2_22]
            .into_iter()
            .find(|release| release.export_format() == raw)
    }

    pub const fn export_format(self) -> &'static str {
        match self {
            Self::Format2_21 => "2.21",
            Self::Format2_22 => "2.22",
        }
    }

    pub const fn platform_line(self) -> PlatformLine {
        match self {
            Self::Format2_21 => PlatformLine::new(8, 5, 1),
            Self::Format2_22 => PlatformLine::new(8, 5, 4),
        }
    }

    pub const fn platform_line_text(self) -> &'static str {
        match self {
            Self::Format2_21 => "8.5.1",
            Self::Format2_22 => "8.5.4",
        }
    }

    pub const fn canonical_id(self) -> &'static str {
        match self {
            Self::Format2_21 => "platform-xml-8.5.1-format-2.21",
            Self::Format2_22 => "platform-xml-8.5.4-format-2.22",
        }
    }

    pub const fn stable_discriminants(self) -> [u8; 3] {
        match self {
            Self::Format2_21 => [1, 3, 3],
            Self::Format2_22 => [1, 4, 4],
        }
    }
}

/// The source profile of a Designer export recording `raw` in its root.
pub(crate) fn platform_xml_profile_for_export_format(raw: &str) -> Option<crate::SourceProfile> {
    if raw == crate::ACTIVE_SERIALIZATION_FORMAT {
        return Some(crate::SourceProfile::platform_xml_8_3_27_format_2_20());
    }
    PlatformXmlRelease::for_export_format(raw).map(crate::SourceProfile::PlatformXmlRelease)
}

/// Whether codecs write sources of `profile` (INV.SOURCE.WRITABLE-PROFILE).
pub(crate) const fn is_writable_platform_xml_profile(profile: crate::SourceProfile) -> bool {
    matches!(
        profile,
        crate::SourceProfile::PlatformXml8_3_27Format2_20
            | crate::SourceProfile::PlatformXmlRelease(_)
    )
}

/// The platform lines whose capabilities are described, with their text.
pub const DESCRIBED_PLATFORM_LINES: &[(PlatformLine, &str)] = &[
    (PlatformLine::new(8, 3, 27), "8.3.27"),
    (PlatformLine::new(8, 5, 1), "8.5.1"),
    (PlatformLine::new(8, 5, 4), "8.5.4"),
];

/// A line of the 1C platform, `8.3.27` or `8.5.4`: three numbers, without a
/// build. One notion for every format family, so lines compare across them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlatformLine([u32; 3]);

impl PlatformLine {
    pub const fn new(major: u32, minor: u32, release: u32) -> Self {
        Self([major, minor, release])
    }

    /// `8.3.27`; anything but three dot-separated numbers is `None`.
    pub fn parse(raw: &str) -> Option<Self> {
        let parts = raw
            .split('.')
            .map(|part| {
                (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                    .then(|| part.parse::<u32>().ok())
                    .flatten()
            })
            .collect::<Option<Vec<_>>>()?;
        <[u32; 3]>::try_from(parts).ok().map(Self)
    }

    pub const fn parts(self) -> [u32; 3] {
        self.0
    }

    /// Whether the four-part build `8.3.27.2342` belongs to this line.
    pub fn contains_build(self, build: &[u32; 4]) -> bool {
        build[..3] == self.0
    }
}

impl fmt::Display for PlatformLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.0[0], self.0[1], self.0[2])
    }
}

/// A format family that exists only for tests: it proves that a family the
/// product has never seen plugs in by a declaration alone
/// (INV.APP.FORMAT-FAMILY-EXTENSION). Its tuple is a tool version and a
/// platform line, as with a tool whose versions run on several lines; tuples
/// are ordered part by part, so a newer tool on an older line is not comparable.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::{FormatFamily, FormatFamilyId, FormatKey, PlatformLine};
    use std::cmp::Ordering;

    pub const TOOL_LINE_FAMILY: FormatFamilyId = FormatFamilyId::new("tool_line_test");

    pub struct ToolLineFamily;

    fn parts(key: &FormatKey) -> Option<(u32, PlatformLine)> {
        let [tool, line] = key.parts() else {
            return None;
        };
        Some((tool.parse().ok()?, PlatformLine::parse(line)?))
    }

    impl FormatFamily for ToolLineFamily {
        fn id(&self) -> FormatFamilyId {
            TOOL_LINE_FAMILY
        }

        fn compare(&self, left: &FormatKey, right: &FormatKey) -> Option<Ordering> {
            let (left_tool, left_line) = parts(left)?;
            let (right_tool, right_line) = parts(right)?;
            match (left_tool.cmp(&right_tool), left_line.cmp(&right_line)) {
                (tool, line) if tool == line => Some(tool),
                (Ordering::Equal, other) | (other, Ordering::Equal) => Some(other),
                _ => None,
            }
        }

        fn known_keys(&self) -> Vec<FormatKey> {
            vec![FormatKey::new(["1", "8.3.27"])]
        }

        fn platform_line(&self, key: &FormatKey) -> Option<PlatformLine> {
            parts(key).map(|(_, line)| line)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FormatFamilyId, FormatIdentity, FormatKey, PlatformLine};

    const FAMILY_A: FormatFamilyId = FormatFamilyId::new("family_a");
    const FAMILY_B: FormatFamilyId = FormatFamilyId::new("family_b");

    /// rule: INV.SOURCE.FORMAT-IDENTITY
    #[test]
    fn a_format_is_its_family_and_its_whole_tuple() {
        let tool_and_line =
            |tool: &str, line: &str| FormatIdentity::new(FAMILY_A, FormatKey::new([tool, line]));
        // One tool version with two lines, and two tool versions on one line:
        // three different formats.
        let first = tool_and_line("2025.1", "8.3.27");
        assert_ne!(first, tool_and_line("2025.1", "8.3.26"));
        assert_ne!(first, tool_and_line("2026.1", "8.3.27"));
        assert_eq!(first, tool_and_line("2025.1", "8.3.27"));

        // The same tuple in another family is another format.
        let other = FormatIdentity::new(FAMILY_B, FormatKey::new(["2025.1", "8.3.27"]));
        assert_ne!(first, other);
        assert_eq!(first.key_in(FAMILY_A), Some(first.key()));
        assert_eq!(first.key_in(FAMILY_B), None);
        assert_eq!(first.to_string(), "family_a(2025.1, 8.3.27)");
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn an_unknown_tuple_is_read_only_when_newer_than_every_known_one() {
        use super::test_support::ToolLineFamily;
        use super::{resolve_read, ReadResolution};
        let key = |tool: &str, line: &str| FormatKey::new([tool, line]);
        let dialects = [key("1", "8.3.27")];
        let resolve = |tool, line| resolve_read(&ToolLineFamily, &key(tool, line), &dialects);

        assert_eq!(resolve("1", "8.3.27"), ReadResolution::Dialect);
        // Newer tool on the known line, newer line with the known tool, both:
        // newer than every known tuple, read as the known one.
        for (tool, line) in [("2", "8.3.27"), ("1", "8.5.1"), ("2", "8.5.1")] {
            assert_eq!(
                resolve(tool, line),
                ReadResolution::Unverified {
                    read_as: key("1", "8.3.27")
                },
                "{tool} {line}"
            );
        }
        // Older, or newer in one part and older in the other: not read.
        for (tool, line) in [
            ("0", "8.3.27"),
            ("1", "8.3.26"),
            ("2", "8.3.26"),
            ("x", "8.3.27"),
        ] {
            assert_eq!(
                resolve(tool, line),
                ReadResolution::Unsupported,
                "{tool} {line}"
            );
        }
    }

    #[test]
    fn a_platform_line_is_three_numbers_and_holds_its_builds() {
        let line = PlatformLine::parse("8.3.27").unwrap();
        assert_eq!(line, PlatformLine::new(8, 3, 27));
        assert_eq!(line.to_string(), "8.3.27");
        assert!(line.contains_build(&[8, 3, 27, 2342]));
        assert!(!line.contains_build(&[8, 3, 26, 1656]));
        assert!(line < PlatformLine::new(8, 5, 1));
        for raw in ["8.3", "8.3.27.2342", "8.3.x", "", "8..27"] {
            assert_eq!(PlatformLine::parse(raw), None, "{raw}");
        }
    }
}

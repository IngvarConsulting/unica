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

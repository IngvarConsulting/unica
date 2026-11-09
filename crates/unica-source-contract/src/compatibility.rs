//! The compatibility mode of a source set and the rules it sets
//! (INV.SOURCE.PROFILE-CAPABILITIES).
//!
//! The platform line of the format says what exists; the compatibility mode
//! says what is allowed. It never removes what exists. The rules are a table
//! by mode version, as appendix 7 of the platform developer guide lists them:
//! (б) configuration checks and property values, (в) extension operations by
//! the mode of the extension. Behaviour of the language at run time (а) is read
//! by BSL analysis and is not in this table yet.

use std::cmp::Ordering;
use std::fmt;

/// A compatibility mode as the set records it: `DontUse` (the behaviour of the
/// platform line) or `Version8_3_12` and the like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompatibilityMode {
    /// No compatibility: the set behaves as its platform line.
    DontUse,
    /// The behaviour of an older platform version, two or three numbers.
    Version([u32; 3]),
}

impl CompatibilityMode {
    /// `DontUse`, `Version8_1`, `Version8_2_13`, `Version8_3_12`; anything else
    /// is `None`.
    pub fn parse(raw: &str) -> Option<Self> {
        if raw == "DontUse" {
            return Some(Self::DontUse);
        }
        let parts = raw
            .strip_prefix("Version")?
            .split('_')
            .map(|part| {
                (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                    .then(|| part.parse::<u32>().ok())
                    .flatten()
            })
            .collect::<Option<Vec<_>>>()?;
        match parts.as_slice() {
            [major, minor] => Some(Self::Version([*major, *minor, 0])),
            [major, minor, release] => Some(Self::Version([*major, *minor, *release])),
            _ => None,
        }
    }

    /// The older version the set behaves as; `None` for `DontUse`.
    pub const fn version(self) -> Option<[u32; 3]> {
        match self {
            Self::DontUse => None,
            Self::Version(version) => Some(version),
        }
    }

    /// Whether the set behaves as `version` or an older one. `DontUse` is
    /// newer than every version.
    fn below(self, version: [u32; 3]) -> bool {
        match self {
            Self::DontUse => false,
            Self::Version(mode) => mode.cmp(&version) == Ordering::Less,
        }
    }
}

impl fmt::Display for CompatibilityMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DontUse => formatter.write_str("DontUse"),
            Self::Version([major, minor, 0]) if *major == 8 && *minor < 3 => {
                write!(formatter, "Version{major}_{minor}")
            }
            Self::Version([major, minor, release]) => {
                write!(formatter, "Version{major}_{minor}_{release}")
            }
        }
    }
}

/// An extension operation whose admission the mode of the extension decides
/// (appendix 7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionOperation {
    /// Add a child object (attribute, tabular section, form …) to a borrowed object.
    AddChildToBorrowed,
    /// Override a property of a borrowed object.
    OverrideBorrowedProperty,
    /// Add a catalog of the extension's own.
    AddOwnCatalog,
    /// Add an information register of the extension's own.
    AddOwnInformationRegister,
    /// Add an attribute to a borrowed catalog.
    AddAttributeToBorrowedCatalog,
    /// Add a tabular section to a borrowed catalog.
    AddTabularSectionToBorrowedCatalog,
}

impl ExtensionOperation {
    /// The oldest extension mode that admits the operation.
    const fn since(self) -> [u32; 3] {
        match self {
            Self::AddChildToBorrowed | Self::OverrideBorrowedProperty => [8, 3, 7],
            Self::AddOwnCatalog
            | Self::AddOwnInformationRegister
            | Self::AddAttributeToBorrowedCatalog
            | Self::AddTabularSectionToBorrowedCatalog => [8, 3, 10],
        }
    }
}

/// Whether a rule lets an operation through, and if not, which mode stops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Allowed,
    /// The mode is older than `since`, the oldest mode that admits it.
    Refused {
        mode: CompatibilityMode,
        since: [u32; 3],
    },
}

/// A configuration check or a property-value limit that depends on the mode
/// (appendix 7.2, kind (б)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckRule {
    /// A fixed-length string is at most 1024 characters.
    FixedStringLengthAtMost1024,
    /// The applicability of an extension to its configuration is checked.
    ExtensionApplicabilityCheckRequired,
}

/// Whether a check rule applies to a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleState {
    Applies,
    DoesNotApply,
}

/// The rules a compatibility mode sets. Only the mode decides them, never the
/// format or the platform line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompatibilityRules {
    mode: CompatibilityMode,
}

impl CompatibilityRules {
    pub const fn for_mode(mode: CompatibilityMode) -> Self {
        Self { mode }
    }

    pub const fn mode(&self) -> CompatibilityMode {
        self.mode
    }

    /// Kind (в): whether the mode of an extension admits `operation`.
    pub fn extension_allows(&self, operation: ExtensionOperation) -> Admission {
        let since = operation.since();
        if self.mode.below(since) {
            Admission::Refused {
                mode: self.mode,
                since,
            }
        } else {
            Admission::Allowed
        }
    }

    /// Kind (б): whether a configuration check or value limit applies.
    pub fn check(&self, rule: CheckRule) -> RuleState {
        let applies = match rule {
            CheckRule::FixedStringLengthAtMost1024 => {
                matches!(self.mode, CompatibilityMode::Version([8, minor, _]) if minor < 2)
            }
            CheckRule::ExtensionApplicabilityCheckRequired => {
                self.mode == CompatibilityMode::DontUse
            }
        };
        if applies {
            RuleState::Applies
        } else {
            RuleState::DoesNotApply
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Admission, CheckRule, CompatibilityMode, CompatibilityRules, ExtensionOperation, RuleState,
    };

    #[test]
    fn a_mode_is_dont_use_or_a_version_as_the_set_records_it() {
        assert_eq!(
            CompatibilityMode::parse("DontUse"),
            Some(CompatibilityMode::DontUse)
        );
        let raw = [
            "Version8_1",
            "Version8_2_13",
            "Version8_3_12",
            "Version8_5_4",
        ];
        for raw in raw {
            let mode = CompatibilityMode::parse(raw).unwrap();
            assert_eq!(mode.to_string(), raw);
        }
        assert_eq!(
            CompatibilityMode::parse("Version8_3_12").unwrap().version(),
            Some([8, 3, 12])
        );
        for raw in [
            "",
            "Version",
            "Version8",
            "Version8_3_x",
            "8.3.12",
            "Version8__3",
        ] {
            assert_eq!(CompatibilityMode::parse(raw), None, "{raw}");
        }
    }

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn the_mode_of_an_extension_admits_its_operations_from_the_appendix() {
        let rules = |raw| CompatibilityRules::for_mode(CompatibilityMode::parse(raw).unwrap());
        let refused = |raw, since| Admission::Refused {
            mode: CompatibilityMode::parse(raw).unwrap(),
            since,
        };
        let child = ExtensionOperation::AddChildToBorrowed;
        let catalog = ExtensionOperation::AddOwnCatalog;
        assert_eq!(
            rules("Version8_3_6").extension_allows(child),
            refused("Version8_3_6", [8, 3, 7])
        );
        assert_eq!(
            rules("Version8_3_7").extension_allows(child),
            Admission::Allowed
        );
        assert_eq!(
            rules("Version8_3_9").extension_allows(catalog),
            refused("Version8_3_9", [8, 3, 10])
        );
        assert_eq!(
            rules("Version8_3_10").extension_allows(catalog),
            Admission::Allowed
        );
        // No compatibility and a mode newer than the table admit everything.
        for raw in ["DontUse", "Version8_5_4", "Version8_9_99"] {
            for operation in [child, catalog] {
                assert_eq!(rules(raw).extension_allows(operation), Admission::Allowed);
            }
        }
    }

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn a_check_rule_follows_the_mode_not_the_platform_line() {
        let rules = |raw| CompatibilityRules::for_mode(CompatibilityMode::parse(raw).unwrap());
        let fixed = CheckRule::FixedStringLengthAtMost1024;
        assert_eq!(rules("Version8_1").check(fixed), RuleState::Applies);
        assert_eq!(rules("Version8_2_13").check(fixed), RuleState::DoesNotApply);
        assert_eq!(rules("DontUse").check(fixed), RuleState::DoesNotApply);
        let applicability = CheckRule::ExtensionApplicabilityCheckRequired;
        assert_eq!(rules("DontUse").check(applicability), RuleState::Applies);
        assert_eq!(
            rules("Version8_3_12").check(applicability),
            RuleState::DoesNotApply
        );
    }
}

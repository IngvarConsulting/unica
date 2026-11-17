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

/// The oldest compatibility mode a set may be edited in; a set in an older
/// mode is read, searched, analysed and checked, but not edited.
pub const MINIMUM_EDITABLE_MODE: [u32; 3] = [8, 3, 8];

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

    /// Whether a set in this mode may be edited (INV.SOURCE.WRITABLE-PROFILE):
    /// a mode older than [`MINIMUM_EDITABLE_MODE`] leaves the set read-only.
    pub fn editable(self) -> bool {
        !self.below(MINIMUM_EDITABLE_MODE)
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
/// (appendix 7.3). The thresholds are those the platform names when it refuses
/// the extension (`ibcmd config check`, 8.3.27.2342 and 8.5.4.2028); import
/// accepts such an extension and only the check and apply refuse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionOperation {
    /// Add a child object other than an attribute or a tabular section to a
    /// borrowed object; the appendix gives the oldest threshold.
    AddChildToBorrowed,
    /// Add an attribute to a borrowed object.
    AddAttributeToBorrowed,
    /// Add a tabular section to a borrowed object.
    AddTabularSectionToBorrowed,
    /// Override a property of a borrowed object.
    OverrideBorrowedProperty,
    /// Add a catalog of the extension's own.
    AddOwnCatalog,
    /// Add an information register of the extension's own.
    AddOwnInformationRegister,
}

impl ExtensionOperation {
    /// The oldest extension mode that admits the operation.
    const fn since(self) -> [u32; 3] {
        match self {
            Self::AddChildToBorrowed => [8, 3, 7],
            Self::OverrideBorrowedProperty => [8, 3, 8],
            // The platform refuses an attribute of a borrowed catalog
            // "in compatibility mode 8.3.8 and below".
            Self::AddAttributeToBorrowed | Self::AddTabularSectionToBorrowed => [8, 3, 9],
            // "Catalogs in extensions are not allowed in compatibility mode
            // 8.3.10 and below"; the appendix lists registers with catalogs.
            Self::AddOwnCatalog | Self::AddOwnInformationRegister => [8, 3, 11],
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
    /// A fixed-length string is at most 100 characters; only mode 8.1 allows
    /// more. The 1024 limit of any string is the database's, not the mode's.
    FixedStringLengthAtMost100,
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
            CheckRule::FixedStringLengthAtMost100 => {
                !matches!(self.mode, CompatibilityMode::Version([8, minor, _]) if minor < 2)
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

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn a_set_older_than_mode_8_3_8_is_read_only() {
        for raw in [
            "Version8_1",
            "Version8_2_16",
            "Version8_3_1",
            "Version8_3_7",
        ] {
            assert!(!CompatibilityMode::parse(raw).unwrap().editable(), "{raw}");
        }
        for raw in ["Version8_3_8", "Version8_3_27", "Version8_5_4", "DontUse"] {
            assert!(CompatibilityMode::parse(raw).unwrap().editable(), "{raw}");
        }
    }

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn the_mode_of_an_extension_admits_its_operations_where_the_platform_does() {
        use ExtensionOperation::{
            AddAttributeToBorrowed, AddChildToBorrowed, AddOwnCatalog, OverrideBorrowedProperty,
        };
        let rules = |raw| CompatibilityRules::for_mode(CompatibilityMode::parse(raw).unwrap());
        let refused = |raw, since| Admission::Refused {
            mode: CompatibilityMode::parse(raw).unwrap(),
            since,
        };
        // The last refused and the first admitted mode of each operation, as
        // the platform names them when it checks the extension.
        for (operation, last_refused, first_allowed, since) in [
            (
                AddChildToBorrowed,
                "Version8_3_6",
                "Version8_3_7",
                [8, 3, 7],
            ),
            (
                OverrideBorrowedProperty,
                "Version8_3_7",
                "Version8_3_8",
                [8, 3, 8],
            ),
            (
                AddAttributeToBorrowed,
                "Version8_3_8",
                "Version8_3_9",
                [8, 3, 9],
            ),
            (AddOwnCatalog, "Version8_3_10", "Version8_3_11", [8, 3, 11]),
        ] {
            assert_eq!(
                rules(last_refused).extension_allows(operation),
                refused(last_refused, since),
                "{operation:?}"
            );
            assert_eq!(
                rules(first_allowed).extension_allows(operation),
                Admission::Allowed,
                "{operation:?}"
            );
        }
        // An older mode is refused too; no compatibility and a mode newer
        // than the table admit everything.
        assert_eq!(
            rules("Version8_2_16").extension_allows(AddOwnCatalog),
            refused("Version8_2_16", [8, 3, 11])
        );
        for raw in ["DontUse", "Version8_5_4", "Version8_9_99"] {
            for operation in [AddChildToBorrowed, AddAttributeToBorrowed, AddOwnCatalog] {
                assert_eq!(rules(raw).extension_allows(operation), Admission::Allowed);
            }
        }
    }

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn a_check_rule_follows_the_mode_not_the_platform_line() {
        let rules = |raw| CompatibilityRules::for_mode(CompatibilityMode::parse(raw).unwrap());
        // Only mode 8.1 allows a fixed-length string longer than 100.
        let fixed = CheckRule::FixedStringLengthAtMost100;
        assert_eq!(rules("Version8_1").check(fixed), RuleState::DoesNotApply);
        assert_eq!(rules("Version8_2_13").check(fixed), RuleState::Applies);
        assert_eq!(rules("Version8_3_27").check(fixed), RuleState::Applies);
        assert_eq!(rules("DontUse").check(fixed), RuleState::Applies);
        let applicability = CheckRule::ExtensionApplicabilityCheckRequired;
        assert_eq!(rules("DontUse").check(applicability), RuleState::Applies);
        assert_eq!(
            rules("Version8_3_12").check(applicability),
            RuleState::DoesNotApply
        );
    }
}

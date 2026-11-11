//! Checks of metadata values that the compatibility mode of the set decides:
//! kind (б) of the compatibility rules (INV.SOURCE.PROFILE-CAPABILITIES).
//!
//! They run over the descriptors an apply changes, before it publishes, and
//! over a descriptor `unica.check` validates. A value the mode forbids is one
//! the platform refuses to import, so the check names the value, the limit and
//! the mode.

use std::path::Path;
use unica_source_contract::compatibility::{CheckRule, CompatibilityRules, RuleState};
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, StagedFileState,
};
use unica_source_contract::SourceSetKind;
use unica_source_designer::extension_mode::source_set_compatibility_mode;

const CORE: &str = "http://v8.1c.ru/8.1/data/core";
/// The longest fixed-length string outside mode 8.1.
// resource-limit-excluded: a value the platform allows in a descriptor, not a resource of Unica
const FIXED_STRING_LIMIT: u32 = 100;

/// One value of a descriptor the mode forbids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeFinding {
    pub message: String,
}

/// The values of one descriptor that `rules` forbid.
pub fn descriptor_mode_findings(descriptor: &[u8], rules: CompatibilityRules) -> Vec<ModeFinding> {
    if rules.check(CheckRule::FixedStringLengthAtMost100) != RuleState::Applies {
        return Vec::new();
    }
    let Ok((_, document)) = unica_xml_format::read::parse_metadata_image(descriptor) else {
        return Vec::new();
    };
    document
        .descendants()
        .filter(|node| node.has_tag_name((CORE, "StringQualifiers")))
        .filter_map(|qualifiers| {
            let text = |name: &str| {
                qualifiers
                    .children()
                    .find(|node| node.has_tag_name((CORE, name)))
                    .and_then(|node| node.text())
                    .map(str::trim)
            };
            let length = text("Length")?.parse::<u32>().ok()?;
            (text("AllowedLength") == Some("Fixed") && length > FIXED_STRING_LIMIT).then(|| {
                ModeFinding {
                    message: format!(
                        "a fixed-length string of {length} characters is longer than {FIXED_STRING_LIMIT}; compatibility mode {} allows at most {FIXED_STRING_LIMIT}, only Version8_1 allows more",
                        rules.mode()
                    ),
                }
            })
        })
        .collect()
}

/// Refuses an apply whose changed descriptors hold a value the mode of the
/// set forbids. A set without a known mode is not checked.
pub fn check_changed_descriptors(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
) -> Result<(), ApplyPlanError> {
    let changes = staged.planned_changes();
    if changes.is_empty() {
        return Ok(());
    }
    let configuration = staged
        .read(Path::new("Configuration.xml"))
        .map_err(|error| ApplyPlanError::staging(error, "ops"))?;
    let Some(mode) = configuration
        .as_deref()
        .and_then(|bytes| source_set_compatibility_mode(bytes, source_kind))
    else {
        return Ok(());
    };
    let rules = CompatibilityRules::for_mode(mode);
    for change in changes {
        let StagedFileState::Bytes(bytes) = &change.current else {
            continue;
        };
        if change
            .relative_path
            .extension()
            .and_then(|ext| ext.to_str())
            != Some("xml")
        {
            continue;
        }
        if let Some(finding) = descriptor_mode_findings(bytes, rules).into_iter().next() {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("{}: {}", change.relative_path.portable(), finding.message),
            )
            .at_path("ops"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::descriptor_mode_findings;
    use unica_source_contract::compatibility::{CompatibilityMode, CompatibilityRules};

    fn descriptor(length: u32, allowed: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Catalog><ChildObjects><Attribute><Properties><Name>Code</Name><Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>{length}</v8:Length><v8:AllowedLength>{allowed}</v8:AllowedLength></v8:StringQualifiers></Type></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#
        )
    }

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn a_fixed_string_longer_than_100_is_allowed_only_in_mode_8_1() {
        let rules = |raw| CompatibilityRules::for_mode(CompatibilityMode::parse(raw).unwrap());
        let findings = |length, allowed, mode| {
            descriptor_mode_findings(descriptor(length, allowed).as_bytes(), rules(mode)).len()
        };
        assert_eq!(findings(101, "Fixed", "DontUse"), 1);
        assert_eq!(findings(2000, "Fixed", "Version8_2_13"), 1);
        assert_eq!(findings(101, "Fixed", "Version8_1"), 0);
        assert_eq!(findings(100, "Fixed", "DontUse"), 0);
        assert_eq!(findings(1000, "Variable", "DontUse"), 0);
        let message =
            &descriptor_mode_findings(descriptor(150, "Fixed").as_bytes(), rules("Version8_3_12"))
                [0]
            .message;
        assert!(
            message.contains("150") && message.contains("Version8_3_12"),
            "{message}"
        );
    }
}

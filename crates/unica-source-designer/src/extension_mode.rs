//! Which extension operations the compatibility mode of the extension admits:
//! kind (в) of the compatibility rules (INV.SOURCE.PROFILE-CAPABILITIES).
//!
//! The mode is the extension's own `ConfigurationExtensionCompatibilityMode`
//! in its `Configuration.xml`. The property of the same name in the main
//! configuration is raised by the platform that loads it to its own line, so
//! it says nothing about the extension. The refusal comes before the edit is
//! staged and names the mode and the mode that admits the operation: the
//! platform imports such an extension and refuses it only at check and apply.

use std::path::Path;
use unica_source_contract::compatibility::{
    Admission, CompatibilityMode, CompatibilityRules, ExtensionOperation,
};
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_source_contract::SourceSetKind;

const PROPERTY: &str = "ConfigurationExtensionCompatibilityMode";

/// The mode an extension records in its own `Configuration.xml`; `None` when
/// the file does not parse or records no mode the rules know.
pub fn extension_compatibility_mode(configuration: &[u8]) -> Option<CompatibilityMode> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(configuration).ok()?;
    let object = document
        .root_element()
        .children()
        .find(|node| node.is_element())?;
    let properties = unica_xml_format::read::child_element(object, "Properties")?;
    let raw = unica_xml_format::read::child_direct_text(properties, PROPERTY)?;
    CompatibilityMode::parse(raw.trim())
}

/// The compatibility mode of a source set from its root `Configuration.xml`:
/// `CompatibilityMode` of a configuration, the extension's own
/// `ConfigurationExtensionCompatibilityMode` of an extension. External
/// processors and reports have no mode of their own (`None`).
pub fn source_set_compatibility_mode(
    configuration: &[u8],
    source_kind: SourceSetKind,
) -> Option<CompatibilityMode> {
    let property = match source_kind {
        SourceSetKind::Configuration => "CompatibilityMode",
        SourceSetKind::Extension => PROPERTY,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => return None,
    };
    let (_, document) = unica_xml_format::read::parse_metadata_image(configuration).ok()?;
    let object = document
        .root_element()
        .children()
        .find(|node| node.is_element())?;
    let properties = unica_xml_format::read::child_element(object, "Properties")?;
    let raw = unica_xml_format::read::child_direct_text(properties, property)?;
    CompatibilityMode::parse(raw.trim())
}

/// Whether a metadata descriptor is a borrowed object: its properties name
/// the object of the main configuration it extends.
pub fn descriptor_is_borrowed(descriptor: &[u8]) -> bool {
    let Ok((_, document)) = unica_xml_format::read::parse_metadata_image(descriptor) else {
        return false;
    };
    document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .and_then(|object| unica_xml_format::read::child_element(object, "Properties"))
        .and_then(|properties| {
            unica_xml_format::read::child_direct_text(properties, "ExtendedConfigurationObject")
        })
        .is_some_and(|id| uuid::Uuid::parse_str(id.trim()).is_ok())
}

/// What the operation does, as a refusal names it.
fn describe(operation: ExtensionOperation) -> &'static str {
    match operation {
        ExtensionOperation::AddChildToBorrowed => "adding child objects to borrowed objects",
        ExtensionOperation::AddAttributeToBorrowed => "adding attributes to borrowed objects",
        ExtensionOperation::AddTabularSectionToBorrowed => {
            "adding tabular sections to borrowed objects"
        }
        ExtensionOperation::OverrideBorrowedProperty => "overriding properties of borrowed objects",
        ExtensionOperation::AddOwnCatalog => "catalogs of the extension's own",
        ExtensionOperation::AddOwnInformationRegister => {
            "information registers of the extension's own"
        }
    }
}

/// `Ok` when the edited set is not an extension, records no known mode, or
/// its mode admits `operation`; otherwise a refusal that names both modes.
pub fn admit_extension_operation(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    operation: ExtensionOperation,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    if source_kind != SourceSetKind::Extension {
        return Ok(());
    }
    let configuration = staged
        .read(Path::new("Configuration.xml"))
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    admit_by_extension_configuration(configuration.as_deref(), operation, at_path)
}

/// The same admission when the extension's `Configuration.xml` is already read.
pub fn admit_by_extension_configuration(
    configuration: Option<&[u8]>,
    operation: ExtensionOperation,
    at_path: &str,
) -> Result<(), ApplyPlanError> {
    let Some(mode) = configuration.and_then(extension_compatibility_mode) else {
        return Ok(());
    };
    match CompatibilityRules::for_mode(mode).extension_allows(operation) {
        Admission::Allowed => Ok(()),
        Admission::Refused { mode, since } => {
            let [major, minor, release] = since;
            Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                format!(
                    "the extension compatibility mode {mode} does not allow {}; the platform allows it from compatibility mode Version{major}_{minor}_{release}",
                    describe(operation)
                ),
            )
            .at_path(at_path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{descriptor_is_borrowed, extension_compatibility_mode};
    use unica_source_contract::compatibility::CompatibilityMode;

    fn configuration(mode: &str) -> String {
        format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><Configuration><Properties><CompatibilityMode>Version8_3_27</CompatibilityMode><ConfigurationExtensionCompatibilityMode>{mode}</ConfigurationExtensionCompatibilityMode></Properties></Configuration></MetaDataObject>"
        )
    }

    /// rule: INV.SOURCE.PROFILE-CAPABILITIES
    #[test]
    fn an_extension_has_the_mode_of_its_own_configuration_property() {
        assert_eq!(
            extension_compatibility_mode(configuration("Version8_3_6").as_bytes()),
            CompatibilityMode::parse("Version8_3_6")
        );
        // The file's CompatibilityMode (Version8_3_27) is not the extension's mode.
        assert_eq!(
            extension_compatibility_mode(configuration("DontUse").as_bytes()),
            Some(CompatibilityMode::DontUse)
        );
        assert_eq!(
            extension_compatibility_mode(configuration("").as_bytes()),
            None
        );
        assert_eq!(extension_compatibility_mode(b"not xml"), None);
    }

    #[test]
    fn a_borrowed_descriptor_names_the_object_it_extends() {
        let descriptor = |extends: &str| {
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><Catalog><Properties><Name>Goods</Name>{extends}</Properties></Catalog></MetaDataObject>"
            )
        };
        assert!(descriptor_is_borrowed(
            descriptor("<ExtendedConfigurationObject>8a3c8c9e-2b0f-4f8d-9d2c-0f1b2c3d4e5f</ExtendedConfigurationObject>").as_bytes()
        ));
        assert!(!descriptor_is_borrowed(descriptor("").as_bytes()));
        assert!(!descriptor_is_borrowed(
            descriptor("<ExtendedConfigurationObject>no</ExtendedConfigurationObject>").as_bytes()
        ));
    }
}

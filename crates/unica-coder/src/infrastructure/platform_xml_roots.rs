//! Closed registry of the XML roots an 8.3.27 / export format 2.20 dump writes.
//!
//! Two independent consumers read this table and must never disagree about a
//! root, because they answer the same question from opposite ends: full-dump
//! publication decides whether a staged file may be written, and platform XML
//! owner resolution decides whose `version` attribute defines the format of a
//! file being read. Keeping two hand-maintained lists let them drift, and the
//! drift surfaced as issue #327 — `PredefinedData` was a publishable root here
//! and an unknown root there, so metadata removal refused to run on any
//! configuration that owned predefined data.
//!
//! The registry is fail-closed on both sides: an unlisted root discards a
//! staged dump and refuses a read whose file declares a version. Each entry is
//! the root of a file the platform itself produces; the comment above it names
//! that dump artifact.

/// How the platform versions a registered root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformXmlRootVersioning {
    /// The platform always writes the exact active export format on this root,
    /// and the root owns that format for the file it heads.
    ExactRootVersion,
    /// The platform writes this root without a version attribute, but the root
    /// still heads a format-bearing artifact and follows its container.
    InheritedRootVersion,
    /// The platform never writes a version on this root and the root owns no
    /// format of its own.
    Versionless,
}

/// Every root the platform writes, with the versioning it guarantees.
pub(crate) static PLATFORM_XML_ROOTS: &[(&str, &str, PlatformXmlRootVersioning)] = &[
    // Configuration.xml and every object owner
    (
        "http://v8.1c.ru/8.3/MDClasses",
        "MetaDataObject",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // Forms/*/Ext/Form.xml
    (
        "http://v8.1c.ru/8.3/xcf/logform",
        "Form",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // Ext/CommandInterface.xml
    (
        "http://v8.1c.ru/8.3/xcf/extrnprops",
        "CommandInterface",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // Ext/Help.xml
    (
        "http://v8.1c.ru/8.3/xcf/extrnprops",
        "Help",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // ExchangePlans/*/Ext/Content.xml
    (
        "http://v8.1c.ru/8.3/xcf/extrnprops",
        "ExchangePlanContent",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // Ext/HomePageWorkArea.xml
    (
        "http://v8.1c.ru/8.3/xcf/extrnprops",
        "HomePageWorkArea",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // CommonPictures/*/Ext/Picture.xml and Ext/Splash.xml
    (
        "http://v8.1c.ru/8.3/xcf/extrnprops",
        "ExtPicture",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // ScheduledJobs/*/Ext/Schedule.xml
    (
        "http://v8.1c.ru/8.3/xcf/extrnprops",
        "JobSchedule",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // Catalogs/*/Ext/Predefined.xml and the other predefined-data owners
    (
        "http://v8.1c.ru/8.3/xcf/predef",
        "PredefinedData",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // ConfigDumpInfo.xml
    (
        "http://v8.1c.ru/8.3/xcf/dumpinfo",
        "ConfigDumpInfo",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // BusinessProcesses/*/Ext/Flowchart.xml
    (
        "http://v8.1c.ru/8.3/xcf/scheme",
        "GraphicalSchema",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // Roles/*/Ext/Rights.xml
    (
        "http://v8.1c.ru/8.2/roles",
        "Rights",
        PlatformXmlRootVersioning::ExactRootVersion,
    ),
    // Ext/ClientApplicationInterface.xml
    (
        "http://v8.1c.ru/8.2/managed-application/core",
        "ClientApplicationInterface",
        PlatformXmlRootVersioning::InheritedRootVersion,
    ),
    // Templates/*/Ext/Template.xml for a data composition schema
    (
        "http://v8.1c.ru/8.1/data-composition-system/schema",
        "DataCompositionSchema",
        PlatformXmlRootVersioning::Versionless,
    ),
    // CommonTemplates/*/Ext/Template.xml for a report appearance template
    (
        "http://v8.1c.ru/8.1/data-composition-system/appearance-template",
        "AppearanceTemplate",
        PlatformXmlRootVersioning::Versionless,
    ),
    // Templates/*/Ext/Template.xml for a spreadsheet document
    (
        "http://v8.1c.ru/8.2/data/spreadsheet",
        "document",
        PlatformXmlRootVersioning::Versionless,
    ),
    // WSReferences/*/Ext/WSDefinition.xml, stored as the service published it
    (
        "http://schemas.xmlsoap.org/wsdl/",
        "definitions",
        PlatformXmlRootVersioning::Versionless,
    ),
];

/// Returns how the platform versions the qualified root, or `None` when the
/// root is outside the closed registry.
pub(crate) fn platform_xml_root_versioning(
    namespace: &str,
    local_name: &str,
) -> Option<PlatformXmlRootVersioning> {
    PLATFORM_XML_ROOTS
        .iter()
        .find(|(registered_namespace, registered_local_name, _)| {
            *registered_namespace == namespace && *registered_local_name == local_name
        })
        .map(|(_, _, versioning)| *versioning)
}

#[cfg(test)]
mod tests {
    use super::{platform_xml_root_versioning, PlatformXmlRootVersioning, PLATFORM_XML_ROOTS};
    use std::collections::BTreeSet;

    #[test]
    fn every_root_is_registered_once() {
        let mut seen = BTreeSet::new();
        for (namespace, local_name, _) in PLATFORM_XML_ROOTS {
            assert!(
                seen.insert((*namespace, *local_name)),
                "{{{namespace}}}{local_name} is registered twice"
            );
        }
    }

    #[test]
    fn every_versioned_root_observed_in_a_platform_dump_is_registered() {
        // Roots observed carrying a version attribute across real 8.3.27 /
        // format 2.20 Designer dumps. A root the platform writes with a version
        // and this table omits is refused as unsupported wherever it is read.
        for (namespace, local_name) in [
            ("http://v8.1c.ru/8.3/MDClasses", "MetaDataObject"),
            ("http://v8.1c.ru/8.3/xcf/logform", "Form"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "Help"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "ExtPicture"),
            ("http://v8.1c.ru/8.2/roles", "Rights"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "JobSchedule"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "CommandInterface"),
            ("http://v8.1c.ru/8.3/xcf/predef", "PredefinedData"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "ExchangePlanContent"),
            ("http://v8.1c.ru/8.3/xcf/scheme", "GraphicalSchema"),
            ("http://v8.1c.ru/8.3/xcf/dumpinfo", "ConfigDumpInfo"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "HomePageWorkArea"),
        ] {
            assert_eq!(
                platform_xml_root_versioning(namespace, local_name),
                Some(PlatformXmlRootVersioning::ExactRootVersion),
                "{{{namespace}}}{local_name} is written with a version by the platform"
            );
        }
    }

    #[test]
    fn unregistered_roots_have_no_versioning() {
        assert_eq!(
            platform_xml_root_versioning("http://example.invalid/unknown", "Anything"),
            None
        );
    }
}

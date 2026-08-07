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

/// One qualified root in the closed Platform XML registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformXmlRoot {
    pub(crate) namespace: &'static str,
    pub(crate) local_name: &'static str,
    pub(crate) versioning: PlatformXmlRootVersioning,
}

/// Every root the platform writes, with the versioning it guarantees.
pub(crate) static PLATFORM_XML_ROOTS: &[PlatformXmlRoot] = &[
    // Configuration.xml and every object owner
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/MDClasses",
        local_name: "MetaDataObject",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // Forms/*/Ext/Form.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/logform",
        local_name: "Form",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // Ext/CommandInterface.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "CommandInterface",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // Ext/Help.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "Help",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // ExchangePlans/*/Ext/Content.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "ExchangePlanContent",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // Ext/HomePageWorkArea.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "HomePageWorkArea",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // CommonPictures/*/Ext/Picture.xml and Ext/Splash.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "ExtPicture",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // ScheduledJobs/*/Ext/Schedule.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "JobSchedule",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // Catalogs/*/Ext/Predefined.xml and the other predefined-data owners
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/predef",
        local_name: "PredefinedData",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // ConfigDumpInfo.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/dumpinfo",
        local_name: "ConfigDumpInfo",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // BusinessProcesses/*/Ext/Flowchart.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/scheme",
        local_name: "GraphicalSchema",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // Roles/*/Ext/Rights.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.2/roles",
        local_name: "Rights",
        versioning: PlatformXmlRootVersioning::ExactRootVersion,
    },
    // Ext/ClientApplicationInterface.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.2/managed-application/core",
        local_name: "ClientApplicationInterface",
        versioning: PlatformXmlRootVersioning::InheritedRootVersion,
    },
    // Templates/*/Ext/Template.xml for a data composition schema
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.1/data-composition-system/schema",
        local_name: "DataCompositionSchema",
        versioning: PlatformXmlRootVersioning::Versionless,
    },
    // CommonTemplates/*/Ext/Template.xml for a report appearance template
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.1/data-composition-system/appearance-template",
        local_name: "AppearanceTemplate",
        versioning: PlatformXmlRootVersioning::Versionless,
    },
    // Templates/*/Ext/Template.xml for a spreadsheet document
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.2/data/spreadsheet",
        local_name: "document",
        versioning: PlatformXmlRootVersioning::Versionless,
    },
    // WSReferences/*/Ext/WSDefinition.xml, stored as the service published it
    PlatformXmlRoot {
        namespace: "http://schemas.xmlsoap.org/wsdl/",
        local_name: "definitions",
        versioning: PlatformXmlRootVersioning::Versionless,
    },
];

/// Returns how the platform versions the qualified root, or `None` when the
/// root is outside the closed registry.
pub(crate) fn platform_xml_root_versioning(
    namespace: &str,
    local_name: &str,
) -> Option<PlatformXmlRootVersioning> {
    PLATFORM_XML_ROOTS
        .iter()
        .find(|root| root.namespace == namespace && root.local_name == local_name)
        .map(|root| root.versioning)
}

#[cfg(test)]
mod tests {
    use super::{platform_xml_root_versioning, PlatformXmlRootVersioning, PLATFORM_XML_ROOTS};
    use std::collections::BTreeSet;

    #[test]
    fn every_root_is_registered_once() {
        let mut seen = BTreeSet::new();
        for root in PLATFORM_XML_ROOTS {
            assert!(
                seen.insert((root.namespace, root.local_name)),
                "{{{}}}{} is registered twice",
                root.namespace,
                root.local_name
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

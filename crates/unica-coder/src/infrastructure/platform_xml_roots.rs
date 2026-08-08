//! Closed catalog of the XML roots an 8.3.27 / export format 2.20 dump writes.
//!
//! Each QName records two independent decisions. Full-dump publication checks
//! whether the root must carry the exact active version, while owner resolution
//! decides whether a read of that document contributes a format owner. Keeping
//! the QNames together prevents spelling drift without turning every versioned
//! subordinate document into an independent owner.

/// How full-dump publication validates a registered root's `version` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformXmlPublicationPolicy {
    /// The platform writes the exact active export format on this root.
    ExactRootVersion,
    /// The platform writes this root without a version attribute.
    Versionless,
}

/// How a registered root participates in format-owner resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformXmlOwnerPolicy {
    /// `MetaDataObject` follows the source-set/standalone metadata route.
    MetadataDescriptor,
    /// The document's own version is an independent compatibility boundary.
    StandaloneVersionOwner,
    /// The document belongs to its containing source set and contributes no
    /// independent owner when it is merely read as a dependency.
    ContainerScoped,
    /// The document owns no export-format version.
    NoOwner,
}

/// One qualified root in the closed Platform XML catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlatformXmlRoot {
    pub(crate) namespace: &'static str,
    pub(crate) local_name: &'static str,
    pub(crate) publication: PlatformXmlPublicationPolicy,
    pub(crate) owner: PlatformXmlOwnerPolicy,
}

/// Every root the platform writes, with explicit policies for both consumers.
pub(crate) static PLATFORM_XML_ROOTS: &[PlatformXmlRoot] = &[
    // Configuration.xml and every object owner
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/MDClasses",
        local_name: "MetaDataObject",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::MetadataDescriptor,
    },
    // Forms/*/Ext/Form.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/logform",
        local_name: "Form",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::StandaloneVersionOwner,
    },
    // Ext/CommandInterface.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "CommandInterface",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::StandaloneVersionOwner,
    },
    // Ext/Help.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "Help",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::StandaloneVersionOwner,
    },
    // ExchangePlans/*/Ext/Content.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "ExchangePlanContent",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::StandaloneVersionOwner,
    },
    // Ext/HomePageWorkArea.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "HomePageWorkArea",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::StandaloneVersionOwner,
    },
    // CommonPictures/*/Ext/Picture.xml and Ext/Splash.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "ExtPicture",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::ContainerScoped,
    },
    // ScheduledJobs/*/Ext/Schedule.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/extrnprops",
        local_name: "JobSchedule",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::ContainerScoped,
    },
    // Catalogs/*/Ext/Predefined.xml and the other predefined-data owners
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/predef",
        local_name: "PredefinedData",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::ContainerScoped,
    },
    // ConfigDumpInfo.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/dumpinfo",
        local_name: "ConfigDumpInfo",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::NoOwner,
    },
    // BusinessProcesses/*/Ext/Flowchart.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.3/xcf/scheme",
        local_name: "GraphicalSchema",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::StandaloneVersionOwner,
    },
    // Roles/*/Ext/Rights.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.2/roles",
        local_name: "Rights",
        publication: PlatformXmlPublicationPolicy::ExactRootVersion,
        owner: PlatformXmlOwnerPolicy::StandaloneVersionOwner,
    },
    // Ext/ClientApplicationInterface.xml
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.2/managed-application/core",
        local_name: "ClientApplicationInterface",
        publication: PlatformXmlPublicationPolicy::Versionless,
        owner: PlatformXmlOwnerPolicy::ContainerScoped,
    },
    // Templates/*/Ext/Template.xml for a data composition schema
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.1/data-composition-system/schema",
        local_name: "DataCompositionSchema",
        publication: PlatformXmlPublicationPolicy::Versionless,
        owner: PlatformXmlOwnerPolicy::NoOwner,
    },
    // CommonTemplates/*/Ext/Template.xml for a report appearance template
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.1/data-composition-system/appearance-template",
        local_name: "AppearanceTemplate",
        publication: PlatformXmlPublicationPolicy::Versionless,
        owner: PlatformXmlOwnerPolicy::NoOwner,
    },
    // Templates/*/Ext/Template.xml for a spreadsheet document
    PlatformXmlRoot {
        namespace: "http://v8.1c.ru/8.2/data/spreadsheet",
        local_name: "document",
        publication: PlatformXmlPublicationPolicy::Versionless,
        owner: PlatformXmlOwnerPolicy::NoOwner,
    },
    // WSReferences/*/Ext/WSDefinition.xml, stored as the service published it
    PlatformXmlRoot {
        namespace: "http://schemas.xmlsoap.org/wsdl/",
        local_name: "definitions",
        publication: PlatformXmlPublicationPolicy::Versionless,
        owner: PlatformXmlOwnerPolicy::NoOwner,
    },
];

fn platform_xml_root(namespace: &str, local_name: &str) -> Option<&'static PlatformXmlRoot> {
    PLATFORM_XML_ROOTS
        .iter()
        .find(|root| root.namespace == namespace && root.local_name == local_name)
}

/// Returns the publication policy for a qualified root.
pub(crate) fn platform_xml_publication_policy(
    namespace: &str,
    local_name: &str,
) -> Option<PlatformXmlPublicationPolicy> {
    platform_xml_root(namespace, local_name).map(|root| root.publication)
}

/// Returns the owner-resolution policy for a qualified root.
pub(crate) fn platform_xml_owner_policy(
    namespace: &str,
    local_name: &str,
) -> Option<PlatformXmlOwnerPolicy> {
    platform_xml_root(namespace, local_name).map(|root| root.owner)
}

#[cfg(test)]
mod tests {
    use super::{
        platform_xml_owner_policy, platform_xml_publication_policy, PlatformXmlOwnerPolicy,
        PlatformXmlPublicationPolicy, PLATFORM_XML_ROOTS,
    };
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
    fn every_root_has_independently_recorded_publication_and_owner_policies() {
        use PlatformXmlOwnerPolicy::{
            ContainerScoped, MetadataDescriptor, NoOwner, StandaloneVersionOwner,
        };
        use PlatformXmlPublicationPolicy::{ExactRootVersion, Versionless};

        let expected = [
            (
                "http://v8.1c.ru/8.3/MDClasses",
                "MetaDataObject",
                ExactRootVersion,
                MetadataDescriptor,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/logform",
                "Form",
                ExactRootVersion,
                StandaloneVersionOwner,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/extrnprops",
                "CommandInterface",
                ExactRootVersion,
                StandaloneVersionOwner,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/extrnprops",
                "Help",
                ExactRootVersion,
                StandaloneVersionOwner,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/extrnprops",
                "ExchangePlanContent",
                ExactRootVersion,
                StandaloneVersionOwner,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/extrnprops",
                "HomePageWorkArea",
                ExactRootVersion,
                StandaloneVersionOwner,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/extrnprops",
                "ExtPicture",
                ExactRootVersion,
                ContainerScoped,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/extrnprops",
                "JobSchedule",
                ExactRootVersion,
                ContainerScoped,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/predef",
                "PredefinedData",
                ExactRootVersion,
                ContainerScoped,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/dumpinfo",
                "ConfigDumpInfo",
                ExactRootVersion,
                NoOwner,
            ),
            (
                "http://v8.1c.ru/8.3/xcf/scheme",
                "GraphicalSchema",
                ExactRootVersion,
                StandaloneVersionOwner,
            ),
            (
                "http://v8.1c.ru/8.2/roles",
                "Rights",
                ExactRootVersion,
                StandaloneVersionOwner,
            ),
            (
                "http://v8.1c.ru/8.2/managed-application/core",
                "ClientApplicationInterface",
                Versionless,
                ContainerScoped,
            ),
            (
                "http://v8.1c.ru/8.1/data-composition-system/schema",
                "DataCompositionSchema",
                Versionless,
                NoOwner,
            ),
            (
                "http://v8.1c.ru/8.1/data-composition-system/appearance-template",
                "AppearanceTemplate",
                Versionless,
                NoOwner,
            ),
            (
                "http://v8.1c.ru/8.2/data/spreadsheet",
                "document",
                Versionless,
                NoOwner,
            ),
            (
                "http://schemas.xmlsoap.org/wsdl/",
                "definitions",
                Versionless,
                NoOwner,
            ),
        ];

        assert_eq!(PLATFORM_XML_ROOTS.len(), expected.len());
        for (namespace, local_name, publication, owner) in expected {
            assert_eq!(
                platform_xml_publication_policy(namespace, local_name),
                Some(publication),
                "{{{namespace}}}{local_name} publication policy"
            );
            assert_eq!(
                platform_xml_owner_policy(namespace, local_name),
                Some(owner),
                "{{{namespace}}}{local_name} owner policy"
            );
        }
    }

    #[test]
    fn unregistered_roots_have_no_policies() {
        assert_eq!(
            platform_xml_publication_policy("http://example.invalid/unknown", "Anything"),
            None
        );
        assert_eq!(
            platform_xml_owner_policy("http://example.invalid/unknown", "Anything"),
            None
        );
    }
}

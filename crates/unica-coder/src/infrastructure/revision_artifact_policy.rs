use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use std::ffi::OsStr;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RevisionArtifactDisposition {
    Content,
    Presence,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformXmlSerializationFormat {
    Format2_20,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevisionArtifactProfile {
    LegacyV12,
    ActorPlatformXml8_3_27 {
        source_kind: SourceSetKind,
        serialization_format: PlatformXmlSerializationFormat,
    },
}

/// Closed revision-corpus authority. Actor-scoped services receive this value
/// from their authenticated source binding; it is never caller or wire input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RevisionArtifactPolicy {
    profile: RevisionArtifactProfile,
}

impl RevisionArtifactPolicy {
    pub(crate) const fn legacy_v12() -> Self {
        Self {
            profile: RevisionArtifactProfile::LegacyV12,
        }
    }

    pub(crate) fn for_actor(
        source_kind: SourceSetKind,
        source_format: SourceFormat,
        source_profile: SourceProfile,
    ) -> Result<Self, String> {
        #[cfg(test)]
        // Synthetic future profiles exist only to exercise the downstream
        // actor-authority rejection; revision capture must not intercept them.
        if source_format == SourceFormat::PlatformXml
            && matches!(
                source_profile,
                SourceProfile::TestPlatform8_3_28Format2_20
                    | SourceProfile::TestPlatform8_3_27Format2_21
            )
        {
            return Ok(Self::legacy_v12());
        }
        if source_format != SourceFormat::PlatformXml
            || source_profile != SourceProfile::platform_xml_8_3_27_format_2_20()
        {
            return Err(
                "source revision artifact policy requires Platform XML 8.3.27 format 2.20"
                    .to_string(),
            );
        }
        Ok(Self {
            profile: RevisionArtifactProfile::ActorPlatformXml8_3_27 {
                source_kind,
                serialization_format: PlatformXmlSerializationFormat::Format2_20,
            },
        })
    }

    pub(crate) fn classify(self, relative: &Path) -> RevisionArtifactDisposition {
        if has_legacy_content_extension(relative) {
            return RevisionArtifactDisposition::Content;
        }
        let RevisionArtifactProfile::ActorPlatformXml8_3_27 {
            source_kind,
            serialization_format: PlatformXmlSerializationFormat::Format2_20,
        } = self.profile
        else {
            return RevisionArtifactDisposition::Ignored;
        };
        let components = relative
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(component) => Some(component),
                _ => None,
            })
            .collect::<Vec<_>>();
        let owns_configuration_collections = matches!(
            source_kind,
            SourceSetKind::Configuration | SourceSetKind::Extension
        );

        if owns_configuration_collections
            && (matches_exact(&components, &["Ext", "ParentConfigurations.bin"])
                || matches_exact(&components, &["XDTOPackages", "*", "Ext", "Package.bin"]))
        {
            return RevisionArtifactDisposition::Content;
        }
        if owns_configuration_collections
            && components.len() == 3
            && components[0] == OsStr::new("Ext")
            && components[1] == OsStr::new("ParentConfigurations")
            && has_ascii_case_insensitive_extension(components[2], "cf")
        {
            return RevisionArtifactDisposition::Presence;
        }
        if is_template_resource(&components)
            || contains_fixed_subtree(&components, &["Ext", "Help"])
            || is_form_item_resource(&components)
        {
            return RevisionArtifactDisposition::Content;
        }
        RevisionArtifactDisposition::Ignored
    }
}

fn matches_exact(components: &[&OsStr], pattern: &[&str]) -> bool {
    components.len() == pattern.len()
        && components
            .iter()
            .zip(pattern)
            .all(|(component, expected)| *expected == "*" || *component == OsStr::new(expected))
}

fn has_ascii_case_insensitive_extension(component: &OsStr, expected: &str) -> bool {
    Path::new(component)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn is_template_resource(components: &[&OsStr]) -> bool {
    components.windows(4).enumerate().any(|(index, window)| {
        window[0] == OsStr::new("Templates")
            && window[2] == OsStr::new("Ext")
            && ((components.len() == index + 4
                && matches!(window[3].to_str(), Some("Template.bin" | "Template.txt")))
                || (window[3] == OsStr::new("Template") && components.len() > index + 4))
    })
}

fn contains_fixed_subtree(components: &[&OsStr], prefix: &[&str]) -> bool {
    components.len() > prefix.len()
        && components.windows(prefix.len()).any(|window| {
            window
                .iter()
                .zip(prefix)
                .all(|(component, expected)| *component == OsStr::new(expected))
        })
}

fn is_form_item_resource(components: &[&OsStr]) -> bool {
    components.windows(5).enumerate().any(|(index, window)| {
        window[0] == OsStr::new("Forms")
            && window[2] == OsStr::new("Ext")
            && window[3] == OsStr::new("Form")
            && window[4] == OsStr::new("Items")
            && components.len() > index + 5
    })
}

fn has_legacy_content_extension(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "bsl" | "xml" | "mdo" | "form" | "rights" | "xdto" | "command" | "yaml" | "yml"
            )
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    pub(crate) fn platform_xml_revision_artifact_profile_is_closed_and_legacy_is_unchanged() {
        let legacy = RevisionArtifactPolicy::legacy_v12();
        assert_eq!(
            legacy.classify(Path::new("Catalogs/Items/Ext/ObjectModule.bsl")),
            RevisionArtifactDisposition::Content
        );
        for ignored in [
            "scratch.bin",
            "XDTOPackages/Sample/Ext/Package.bin",
            "Ext/ParentConfigurations/Vendor.cf",
        ] {
            assert_eq!(
                legacy.classify(Path::new(ignored)),
                RevisionArtifactDisposition::Ignored,
                "legacy widened for {ignored}"
            );
        }

        let configuration = RevisionArtifactPolicy::for_actor(
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .unwrap();
        for content in [
            "Ext/ParentConfigurations.bin",
            "XDTOPackages/Sample/Ext/Package.bin",
            "Catalogs/Items/Templates/Binary/Ext/Template.bin",
            "Catalogs/Items/Templates/Text/Ext/Template.txt",
            "Catalogs/Items/Templates/Html/Ext/Template/page.html",
            "Catalogs/Items/Ext/Help/ru.html",
            "Catalogs/Items/Forms/Main/Ext/Form/Items/icon.png",
        ] {
            assert_eq!(
                configuration.classify(Path::new(content)),
                RevisionArtifactDisposition::Content,
                "missing content resource {content}"
            );
        }
        assert_eq!(
            configuration.classify(Path::new("Ext/ParentConfigurations/Vendor.CF")),
            RevisionArtifactDisposition::Presence
        );
        for ignored in [
            "scratch.bin",
            "Loose/Package.bin",
            "Ext/Nested/ParentConfigurations/Vendor.cf",
            "XDTOPackages/Sample/Package.bin",
            "Catalogs/Items/Templates/Binary/Ext/other.bin",
            "Catalogs/Items/Forms/Main/Ext/Items/icon.png",
        ] {
            assert_eq!(
                configuration.classify(Path::new(ignored)),
                RevisionArtifactDisposition::Ignored,
                "profile admitted unclassified resource {ignored}"
            );
        }

        let external = RevisionArtifactPolicy::for_actor(
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .unwrap();
        assert_eq!(
            external.classify(Path::new("Templates/Main/Ext/Template.bin")),
            RevisionArtifactDisposition::Content
        );
        assert_eq!(
            external.classify(Path::new("XDTOPackages/Sample/Ext/Package.bin")),
            RevisionArtifactDisposition::Ignored
        );
        assert!(RevisionArtifactPolicy::for_actor(
            SourceSetKind::Configuration,
            SourceFormat::Edt,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .is_err());
        assert!(RevisionArtifactPolicy::for_actor(
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::legacy_workspace_service_compatibility(),
        )
        .is_err());
        let synthetic_unsupported = RevisionArtifactPolicy::for_actor(
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::TestPlatform8_3_28Format2_20,
        )
        .unwrap();
        assert_eq!(
            synthetic_unsupported.classify(Path::new("XDTOPackages/Sample/Ext/Package.bin")),
            RevisionArtifactDisposition::Ignored
        );
    }
}

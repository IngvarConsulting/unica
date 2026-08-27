use crate::domain::metadata::{metadata_kind_collections, MetaCollection, MetadataKind};
use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
use crate::infrastructure::metadata_kinds::{supports_nested_form_or_command, METADATA_KINDS};
use crate::infrastructure::workspace_actor::ActorRevisionServiceAuthority;
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

    pub(crate) fn from_authenticated_actor(
        authority: &ActorRevisionServiceAuthority,
    ) -> Result<Self, String> {
        Self::from_actor_fields(
            authority.source_kind(),
            authority.source_format(),
            authority.source_profile(),
        )
    }

    fn from_actor_fields(
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

    #[cfg(test)]
    pub(crate) fn platform_xml_for_test(source_kind: SourceSetKind) -> Self {
        Self::from_actor_fields(
            source_kind,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .expect("active test profile is supported")
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
        let owned_resource = match source_kind {
            SourceSetKind::Configuration | SourceSetKind::Extension => {
                is_configuration_resource(&components)
            }
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
                is_external_resource(&components)
            }
        };
        if owned_resource {
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

fn is_configuration_resource(components: &[&OsStr]) -> bool {
    if is_help_resource(components) {
        return true;
    }
    let Some((owner_kind, owner_tail)) = configuration_owner_tail(components) else {
        return false;
    };
    let capabilities = configuration_owner_capabilities(owner_kind);
    if capabilities.help && is_help_resource(owner_tail) {
        return true;
    }
    match owner_kind {
        "CommonForm" => capabilities.forms && is_form_item_tail(owner_tail),
        "CommonTemplate" => capabilities.templates && is_template_body_tail(owner_tail),
        _ => {
            (capabilities.forms && is_named_form_resource(owner_tail))
                || (capabilities.templates && is_named_template_resource(owner_tail))
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ConfigurationOwnerCapabilities {
    forms: bool,
    templates: bool,
    help: bool,
}

fn configuration_owner_capabilities(owner_kind: &str) -> ConfigurationOwnerCapabilities {
    let metadata_kind = MetadataKind::parse(owner_kind).ok();
    ConfigurationOwnerCapabilities {
        forms: owner_kind == "CommonForm" || supports_nested_form_or_command(owner_kind),
        templates: owner_kind == "CommonTemplate"
            || metadata_kind.is_some_and(|kind| {
                metadata_kind_collections(kind).contains(&MetaCollection::Templates)
            }),
        // The typed metadata registry is the closed set whose XML owners carry
        // Ext/Help. Subsystems and the direct CommonForm layout are the two
        // established non-typed owners with the same capability.
        help: metadata_kind.is_some() || matches!(owner_kind, "Subsystem" | "CommonForm"),
    }
}

fn configuration_owner_tail<'a>(
    components: &'a [&OsStr],
) -> Option<(&'static str, &'a [&'a OsStr])> {
    let owner_kind = METADATA_KINDS
        .iter()
        .find(|kind| components.first() == Some(&OsStr::new(kind.directory)))?;
    components.get(1)?;
    let mut tail = 2;
    if components[0] == OsStr::new("Subsystems") {
        while components.get(tail) == Some(&OsStr::new("Subsystems"))
            && components.get(tail + 1).is_some()
        {
            tail += 2;
        }
    }
    Some((owner_kind.tag, &components[tail..]))
}

fn is_external_resource(components: &[&OsStr]) -> bool {
    components.len() >= 2
        && (is_help_resource(&components[1..])
            || is_named_form_resource(&components[1..])
            || is_named_template_resource(&components[1..]))
}

fn is_named_form_resource(tail: &[&OsStr]) -> bool {
    tail.len() >= 3
        && tail[0] == OsStr::new("Forms")
        && (is_help_resource(&tail[2..]) || is_form_item_tail(&tail[2..]))
}

fn is_named_template_resource(tail: &[&OsStr]) -> bool {
    tail.len() >= 3 && tail[0] == OsStr::new("Templates") && is_template_body_tail(&tail[2..])
}

fn is_help_resource(tail: &[&OsStr]) -> bool {
    tail.len() > 2 && tail[0] == OsStr::new("Ext") && tail[1] == OsStr::new("Help")
}

fn is_form_item_tail(tail: &[&OsStr]) -> bool {
    tail.len() > 3
        && tail[0] == OsStr::new("Ext")
        && tail[1] == OsStr::new("Form")
        && tail[2] == OsStr::new("Items")
}

fn is_template_body_tail(tail: &[&OsStr]) -> bool {
    tail.len() >= 2
        && tail[0] == OsStr::new("Ext")
        && ((tail.len() == 2 && matches!(tail[1].to_str(), Some("Template.bin" | "Template.txt")))
            || (tail[1] == OsStr::new("Template") && tail.len() > 2))
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

        let configuration = RevisionArtifactPolicy::from_actor_fields(
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .unwrap();
        for content in [
            "Ext/Help/ru.html",
            "Ext/ParentConfigurations.bin",
            "XDTOPackages/Sample/Ext/Package.bin",
            "Catalogs/Items/Templates/Binary/Ext/Template.bin",
            "Catalogs/Items/Templates/Text/Ext/Template.txt",
            "Catalogs/Items/Templates/Html/Ext/Template/page.html",
            "Catalogs/Items/Ext/Help/ru.html",
            "Catalogs/Items/Forms/Main/Ext/Form/Items/icon.png",
            "CommonForms/Main/Ext/Form/Items/icon.png",
            "CommonForms/Main/Ext/Help/ru.html",
            "CommonTemplates/Logo/Ext/Template.bin",
            "Subsystems/Sales/Subsystems/Retail/Ext/Help/ru.html",
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
            "Loose/Templates/Junk/Ext/Template.bin",
            "Loose/Ext/Help/asset.bin",
            "Loose/Forms/Junk/Ext/Form/Items/asset.bin",
            "Documents/Order/Ext/Template.bin",
            "Documents/Order/Forms/Main/Templates/Junk/Ext/Template.bin",
            "CommonForms/Main/Ext/Template.bin",
            "CommonTemplates/Logo/Ext/Form/Items/asset.bin",
            "CommonTemplates/Logo/Ext/Help/ru.html",
            "Roles/Seller/Templates/Fake/Ext/Template.bin",
            "Roles/Seller/Forms/Fake/Ext/Form/Items/icon.png",
            "Roles/Seller/Ext/Help/ru.html",
            "Languages/Russian/Templates/Fake/Ext/Template.bin",
            "XDTOPackages/Sample/Forms/Fake/Ext/Form/Items/icon.png",
            "Catalogs/Items/Templates/Binary/Ext/other.bin",
            "Catalogs/Items/Forms/Main/Ext/Items/icon.png",
        ] {
            assert_eq!(
                configuration.classify(Path::new(ignored)),
                RevisionArtifactDisposition::Ignored,
                "profile admitted unclassified resource {ignored}"
            );
        }

        // Hand-derived from the active Platform owner capability registries:
        // every known collection is exercised, including all impossible
        // owners, so adding a metadata kind cannot silently grant it Forms,
        // Templates or Help by sharing the same physical prefix shape.
        let extension = RevisionArtifactPolicy::from_actor_fields(
            SourceSetKind::Extension,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .unwrap();
        const FORM_OWNER_DIRECTORIES: &[&str] = &[
            "Catalogs",
            "Documents",
            "Constants",
            "Enums",
            "Reports",
            "DataProcessors",
            "InformationRegisters",
            "AccumulationRegisters",
            "AccountingRegisters",
            "CalculationRegisters",
            "ChartsOfAccounts",
            "ChartsOfCharacteristicTypes",
            "ChartsOfCalculationTypes",
            "ExchangePlans",
            "BusinessProcesses",
            "Tasks",
            "DocumentJournals",
            "Sequences",
            "DocumentNumerators",
        ];
        const TEMPLATE_OWNER_DIRECTORIES: &[&str] = &[
            "Catalogs",
            "Documents",
            "Enums",
            "Reports",
            "DataProcessors",
            "InformationRegisters",
            "AccumulationRegisters",
            "AccountingRegisters",
            "CalculationRegisters",
            "ChartsOfAccounts",
            "ChartsOfCharacteristicTypes",
            "ChartsOfCalculationTypes",
            "ExchangePlans",
            "BusinessProcesses",
            "Tasks",
            "DocumentJournals",
        ];
        const HELP_OWNER_DIRECTORIES: &[&str] = &[
            "Subsystems",
            "CommonForms",
            "Catalogs",
            "Documents",
            "Enums",
            "Constants",
            "Reports",
            "DataProcessors",
            "InformationRegisters",
            "AccumulationRegisters",
            "AccountingRegisters",
            "CalculationRegisters",
            "ChartsOfAccounts",
            "ChartsOfCharacteristicTypes",
            "ChartsOfCalculationTypes",
            "ExchangePlans",
            "BusinessProcesses",
            "Tasks",
            "DocumentJournals",
            "CommonModules",
            "ScheduledJobs",
            "EventSubscriptions",
            "HTTPServices",
            "WebServices",
            "DefinedTypes",
        ];
        for (source_label, policy) in [("configuration", configuration), ("extension", extension)] {
            for owner in METADATA_KINDS {
                let directory = owner.directory;
                let form = format!("{directory}/Owner/Forms/Main/Ext/Form/Items/icon.png");
                let template = format!("{directory}/Owner/Templates/Main/Ext/Template.bin");
                let help = format!("{directory}/Owner/Ext/Help/ru.html");
                assert_eq!(
                    policy.classify(Path::new(&form)),
                    if FORM_OWNER_DIRECTORIES.contains(&directory) {
                        RevisionArtifactDisposition::Content
                    } else {
                        RevisionArtifactDisposition::Ignored
                    },
                    "wrong {source_label} Form capability for {}",
                    owner.tag
                );
                assert_eq!(
                    policy.classify(Path::new(&template)),
                    if TEMPLATE_OWNER_DIRECTORIES.contains(&directory) {
                        RevisionArtifactDisposition::Content
                    } else {
                        RevisionArtifactDisposition::Ignored
                    },
                    "wrong {source_label} Template capability for {}",
                    owner.tag
                );
                assert_eq!(
                    policy.classify(Path::new(&help)),
                    if HELP_OWNER_DIRECTORIES.contains(&directory) {
                        RevisionArtifactDisposition::Content
                    } else {
                        RevisionArtifactDisposition::Ignored
                    },
                    "wrong {source_label} Help capability for {}",
                    owner.tag
                );
            }
        }

        for content in [
            "CommonTemplates/Logo/Ext/Template.bin",
            "Documents/Order/Ext/Help/ru.html",
            "Documents/Order/Forms/Main/Ext/Form/Items/icon.png",
        ] {
            assert_eq!(
                extension.classify(Path::new(content)),
                RevisionArtifactDisposition::Content,
                "extension omitted owned resource {content}"
            );
        }

        let external_processor = RevisionArtifactPolicy::from_actor_fields(
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .unwrap();
        for content in [
            "PriceLoader/Templates/Main/Ext/Template.bin",
            "PriceLoader/Ext/Help/ru.html",
            "PriceLoader/Forms/Main/Ext/Form/Items/icon.png",
        ] {
            assert_eq!(
                external_processor.classify(Path::new(content)),
                RevisionArtifactDisposition::Content,
                "external processor omitted owned resource {content}"
            );
        }
        assert_eq!(
            external_processor.classify(Path::new(
                "Loose/PriceLoader/Templates/Main/Ext/Template.bin"
            )),
            RevisionArtifactDisposition::Ignored
        );
        assert_eq!(
            external_processor.classify(Path::new("XDTOPackages/Sample/Ext/Package.bin")),
            RevisionArtifactDisposition::Ignored
        );

        let external_report = RevisionArtifactPolicy::from_actor_fields(
            SourceSetKind::ExternalReport,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .unwrap();
        for content in [
            "Sales/Templates/Main/Ext/Template.txt",
            "Sales/Ext/Help/ru.html",
            "Sales/Forms/Main/Ext/Form/Items/chart.png",
        ] {
            assert_eq!(
                external_report.classify(Path::new(content)),
                RevisionArtifactDisposition::Content,
                "external report omitted owned resource {content}"
            );
        }
        assert!(RevisionArtifactPolicy::from_actor_fields(
            SourceSetKind::Configuration,
            SourceFormat::Edt,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )
        .is_err());
        assert!(RevisionArtifactPolicy::from_actor_fields(
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::legacy_workspace_service_compatibility(),
        )
        .is_err());
        let synthetic_unsupported = RevisionArtifactPolicy::from_actor_fields(
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

    #[test]
    pub(crate) fn actor_revision_policy_has_no_raw_issuer_or_scoped_service_bypass() {
        use quote::ToTokens as _;

        fn inherent_methods(source: &str, target: &str) -> Vec<String> {
            let file = syn::parse_file(source).expect("production Rust must parse");
            file.items
                .into_iter()
                .filter_map(|item| match item {
                    syn::Item::Impl(item) => Some(item),
                    _ => None,
                })
                .filter(|item| {
                    item.trait_.is_none()
                        && matches!(
                            item.self_ty.as_ref(),
                            syn::Type::Path(path)
                                if path.path.segments.last().is_some_and(|segment| segment.ident == target)
                        )
                })
                .flat_map(|item| item.items)
                .filter_map(|item| match item {
                    syn::ImplItem::Fn(function) => Some(function.sig.ident.to_string()),
                    _ => None,
                })
                .collect()
        }

        let policy_methods = inherent_methods(
            include_str!("revision_artifact_policy.rs"),
            "RevisionArtifactPolicy",
        );
        let service_methods =
            inherent_methods(include_str!("source_revision.rs"), "SourceRevisionService");
        let service_file = syn::parse_file(include_str!("source_revision.rs"))
            .expect("source revision production Rust must parse");
        let actor_file = syn::parse_file(include_str!("workspace_actor.rs"))
            .expect("workspace actor production Rust must parse");
        let service_root_capability = service_file.items.iter().any(|item| {
            let syn::Item::Struct(item) = item else {
                return false;
            };
            item.ident == "SourceRevisionService"
                && item.fields.iter().any(|field| {
                    let ty = field.ty.to_token_stream().to_string();
                    ty.contains("Arc < RetainedDirectoryCapability >")
                })
        });
        let new_actor_body = service_file
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Impl(item) if item.trait_.is_none() => Some(item),
                _ => None,
            })
            .flat_map(|item| item.items.iter())
            .find_map(|item| match item {
                syn::ImplItem::Fn(function) if function.sig.ident == "new_actor" => {
                    Some(function.block.to_token_stream().to_string())
                }
                _ => None,
            })
            .expect("actor service constructor must exist");
        let legacy_body = actor_file
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Impl(item) if item.trait_.is_none() => Some(item),
                _ => None,
            })
            .flat_map(|item| item.items.iter())
            .find_map(|item| match item {
                syn::ImplItem::Fn(function) if function.sig.ident == "with_legacy_runtime" => {
                    Some(function.block.to_token_stream().to_string())
                }
                _ => None,
            })
            .expect("legacy adapter must exist");

        assert!(
            !policy_methods.iter().any(|method| method == "for_actor"),
            "raw source kind/format/profile policy issuer remains available"
        );
        assert!(
            !service_methods.iter().any(|method| method == "new_scoped"),
            "raw scope/root/policy scoped-service bypass remains available"
        );
        assert!(
            policy_methods
                .iter()
                .any(|method| method == "from_authenticated_actor"),
            "policy is not issued from authenticated actor authority"
        );
        assert!(
            service_methods.iter().any(|method| method == "new_actor"),
            "scoped service is not constructed from one actor authority"
        );
        assert!(
            service_root_capability,
            "actor service discards its exact retained root capability"
        );
        assert!(
            !new_actor_body.contains("fs :: canonicalize")
                && !new_actor_body.contains("RetainedDirectoryCapability :: open"),
            "actor service constructor reopens the ambient root instead of consuming authority"
        );
        assert!(
            legacy_body.contains("validate_legacy_workspace_identity"),
            "legacy actor adapter does not prove the exact legacy source tuple"
        );
    }
}

use crate::layout::METADATA_KIND_TAGS;
use crate::roots::{platform_xml_owner_policy, root_version_literal, PlatformXmlOwnerPolicy};
use roxmltree::Document;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use unica_source_contract::SourceSetKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformXmlOwnerKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
    Standalone,
}

impl PlatformXmlOwnerKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Extension => "extension",
            Self::ExternalProcessor => "external_processor",
            Self::ExternalReport => "external_report",
            Self::Standalone => "standalone",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlatformXmlOwner {
    pub kind: PlatformXmlOwnerKind,
    pub path: PathBuf,
    pub version: Option<String>,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PlatformXmlSourceSetOwnerEvidence {
    version: Option<String>,
    artifact_kind: String,
    artifact_name: Option<String>,
    registrations: BTreeSet<(String, String)>,
    configuration_extension: bool,
}

impl PlatformXmlSourceSetOwnerEvidence {
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn registers(&self, kind: &str, name: &str) -> bool {
        self.registrations
            .contains(&(kind.to_string(), name.to_string()))
    }

    pub fn artifact_kind(&self) -> &str {
        &self.artifact_kind
    }

    pub fn artifact_name(&self) -> Option<&str> {
        self.artifact_name.as_deref()
    }

    pub fn registrations(&self) -> impl Iterator<Item = (&str, &str)> {
        self.registrations
            .iter()
            .map(|(kind, name)| (kind.as_str(), name.as_str()))
    }

    pub const fn is_configuration_extension(&self) -> bool {
        self.configuration_extension
    }
}

#[derive(Debug, Clone)]
pub struct PlatformXmlOwnerError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum OwnerExpectation {
    SourceSet(SourceSetKind),
    Standalone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformXmlRootExpectation {
    pub namespace: &'static str,
    pub local_name: &'static str,
}

impl PlatformXmlRootExpectation {
    pub const fn new(namespace: &'static str, local_name: &'static str) -> Self {
        Self {
            namespace,
            local_name,
        }
    }
}

pub const MANAGED_FORM_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::new("http://v8.1c.ru/8.3/xcf/logform", "Form");

pub const DCS_ROOT: PlatformXmlRootExpectation = PlatformXmlRootExpectation::new(
    "http://v8.1c.ru/8.1/data-composition-system/schema",
    "DataCompositionSchema",
);

pub const MXL_ROOT: PlatformXmlRootExpectation =
    PlatformXmlRootExpectation::new("http://v8.1c.ru/8.2/data/spreadsheet", "document");

pub const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

pub fn prove_already_read_source_set_owner(
    path: &Path,
    raw: &[u8],
    configured_kind: SourceSetKind,
) -> Result<PlatformXmlSourceSetOwnerEvidence, PlatformXmlOwnerError> {
    let (source, document) = parse_platform_xml_document(path, raw)?;
    let root = document.root_element();
    validate_source_set_owner(root, configured_kind, path)?;
    let artifact = root
        .children()
        .find(|node| node.is_element())
        .expect("validated source-set owner has one direct artifact");
    let mut registrations = BTreeSet::new();
    for child_objects in artifact.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "ChildObjects"
    }) {
        for registration in child_objects
            .children()
            .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
        {
            if let Some(name) = registered_child_name(registration) {
                registrations.insert((registration.tag_name().name().to_string(), name));
            }
        }
    }
    Ok(PlatformXmlSourceSetOwnerEvidence {
        version: root_version_literal(source, root),
        artifact_kind: artifact.tag_name().name().to_string(),
        artifact_name: metadata_artifact_property(artifact, "Name"),
        registrations,
        configuration_extension: is_configuration_extension_artifact(artifact),
    })
}

pub fn prove_already_read_metadata_owner(
    path: &Path,
    raw: &[u8],
) -> Result<PlatformXmlSourceSetOwnerEvidence, PlatformXmlOwnerError> {
    parse_platform_xml_owner(path, raw.to_vec(), OwnerExpectation::Standalone)?;
    let (source, document) = parse_platform_xml_document(path, raw)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return invalid_owner(path, "metadata owner must use the MetaDataObject root");
    }
    let artifact = root
        .children()
        .find(|node| node.is_element())
        .expect("strict standalone metadata owner has one artifact child");
    let mut registrations = BTreeSet::new();
    for child_objects in artifact.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "ChildObjects"
    }) {
        for registration in child_objects
            .children()
            .filter(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
        {
            if let Some(name) = registered_child_name(registration) {
                registrations.insert((registration.tag_name().name().to_string(), name));
            }
        }
    }
    Ok(PlatformXmlSourceSetOwnerEvidence {
        version: root_version_literal(source, root),
        artifact_kind: artifact.tag_name().name().to_string(),
        artifact_name: metadata_artifact_property(artifact, "Name"),
        registrations,
        configuration_extension: false,
    })
}

pub fn metadata_artifact_property(artifact: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    artifact
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Properties"
        })?
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == name
        })?
        .text()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn parse_platform_xml_owner(
    path: &Path,
    raw: Vec<u8>,
    expectation: OwnerExpectation,
) -> Result<PlatformXmlOwner, PlatformXmlOwnerError> {
    let (source, document) = parse_platform_xml_document(path, &raw)?;
    let root = document.root_element();
    let root_qname = (root.tag_name().namespace(), root.tag_name().name());
    let artifact_children = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    let direct_artifact = match artifact_children.as_slice() {
        [artifact] => Some(*artifact),
        _ => None,
    };
    let is_extension = direct_artifact.is_some_and(is_configuration_extension_artifact);
    let kind = match expectation {
        OwnerExpectation::SourceSet(configured_kind) => {
            validate_source_set_owner(root, configured_kind, path)?;
            match configured_kind {
                SourceSetKind::ExternalProcessor => PlatformXmlOwnerKind::ExternalProcessor,
                SourceSetKind::ExternalReport => PlatformXmlOwnerKind::ExternalReport,
                SourceSetKind::Extension => PlatformXmlOwnerKind::Extension,
                SourceSetKind::Configuration if is_extension => PlatformXmlOwnerKind::Extension,
                SourceSetKind::Configuration => PlatformXmlOwnerKind::Configuration,
            }
        }
        OwnerExpectation::Standalone => {
            if root_qname == (Some(MD_CLASSES_NS), "MetaDataObject") {
                let Some(artifact) = direct_artifact else {
                    return invalid_owner(
                        path,
                        &format!(
                            "standalone metadata descriptor must contain exactly one direct {{{MD_CLASSES_NS}}} artifact child"
                        ),
                    );
                };
                if artifact.tag_name().namespace() != Some(MD_CLASSES_NS) {
                    return invalid_owner(
                        path,
                        &format!(
                            "standalone metadata descriptor direct artifact child must use namespace {{{MD_CLASSES_NS}}}"
                        ),
                    );
                }
                if !is_supported_metadata_artifact(artifact.tag_name().name()) {
                    return invalid_owner(
                        path,
                        &format!(
                            "unsupported 8.3.27 metadata artifact family {{{MD_CLASSES_NS}}}{}",
                            artifact.tag_name().name()
                        ),
                    );
                }
                match artifact.tag_name().name() {
                    "ExternalDataProcessor" => PlatformXmlOwnerKind::ExternalProcessor,
                    "ExternalReport" => PlatformXmlOwnerKind::ExternalReport,
                    "Configuration" if is_extension => PlatformXmlOwnerKind::Extension,
                    _ => PlatformXmlOwnerKind::Standalone,
                }
            } else if known_standalone_root(root_qname) {
                PlatformXmlOwnerKind::Standalone
            } else {
                return invalid_owner(
                    path,
                    &format!(
                        "unsupported standalone platform XML root {{{}}}{}",
                        root_qname.0.unwrap_or(""),
                        root_qname.1
                    ),
                );
            }
        }
    };
    Ok(PlatformXmlOwner {
        kind,
        path: path.to_path_buf(),
        version: root_version_literal(source, root),
        raw,
    })
}

pub fn parse_platform_xml_document<'a>(
    path: &Path,
    raw: &'a [u8],
) -> Result<(&'a str, Document<'a>), PlatformXmlOwnerError> {
    let text = std::str::from_utf8(raw).map_err(|error| PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!("failed to read {} as UTF-8: {error}", path.display()),
    })?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source).map_err(|error| PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!("failed to parse {}: {error}", path.display()),
    })?;
    Ok((source, document))
}

pub fn validate_source_set_owner(
    root: roxmltree::Node<'_, '_>,
    configured_kind: SourceSetKind,
    path: &Path,
) -> Result<(), PlatformXmlOwnerError> {
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return invalid_owner(
            path,
            "source-set owner root must be {http://v8.1c.ru/8.3/MDClasses}MetaDataObject",
        );
    }
    let expected_child = match configured_kind {
        SourceSetKind::Configuration | SourceSetKind::Extension => "Configuration",
        SourceSetKind::ExternalProcessor => "ExternalDataProcessor",
        SourceSetKind::ExternalReport => "ExternalReport",
    };
    let artifact_children = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if artifact_children.len() != 1
        || artifact_children[0].tag_name().namespace() != Some(MD_CLASSES_NS)
        || artifact_children[0].tag_name().name() != expected_child
    {
        return invalid_owner(
            path,
            &format!(
                "source-set owner must contain exactly one direct {{{MD_CLASSES_NS}}}{expected_child} artifact child"
            ),
        );
    }
    Ok(())
}

pub fn is_configuration_extension_artifact(artifact: roxmltree::Node<'_, '_>) -> bool {
    artifact.tag_name().namespace() == Some(MD_CLASSES_NS)
        && artifact.tag_name().name() == "Configuration"
        && artifact
            .children()
            .find(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                    && node.tag_name().name() == "Properties"
            })
            .is_some_and(|properties| {
                properties.children().any(|node| {
                    node.is_element()
                        && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                        && node.tag_name().name() == "ConfigurationExtensionPurpose"
                })
            })
}

/// The name a `ChildObjects` entry registers. Forms and templates are text
/// references (`<Form>Main</Form>`); a command is registered by its full
/// inline definition (`<Command uuid="…"><Properties><Name>…`) and has no
/// descriptor file of its own, exactly as Platform XML 8.3.27 emits it.
pub fn registered_child_name(registration: roxmltree::Node<'_, '_>) -> Option<String> {
    if let Some(name) = registration
        .text()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return Some(name.to_string());
    }
    let properties = registration.children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "Properties"
    })?;
    properties
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Name"
        })
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

pub fn is_supported_metadata_artifact(tag: &str) -> bool {
    METADATA_KIND_TAGS.contains(&tag)
        || matches!(
            tag,
            "Configuration"
                | "ExternalDataProcessor"
                | "ExternalReport"
                | "Form"
                | "Template"
                | "Command"
        )
}

/// Returns whether the qualified root is supported as a standalone Platform XML
/// owner.
///
/// Derived from the explicit owner axis of the shared QName profile. Publication
/// syntax is deliberately not enough to make a container-scoped sidecar an
/// independent format owner.
pub fn known_standalone_root(qname: (Option<&str>, &str)) -> bool {
    let (Some(namespace), local_name) = qname else {
        return false;
    };
    platform_xml_owner_policy(namespace, local_name)
        == Some(PlatformXmlOwnerPolicy::StandaloneVersionOwner)
}

pub fn invalid_owner<T>(path: &Path, reason: &str) -> Result<T, PlatformXmlOwnerError> {
    Err(PlatformXmlOwnerError {
        path: path.to_path_buf(),
        message: format!("invalid platform XML owner {}: {reason}", path.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_ownership_is_independent_from_publication_versioning() {
        for (namespace, local_name) in [
            ("http://v8.1c.ru/8.3/xcf/logform", "Form"),
            ("http://v8.1c.ru/8.2/roles", "Rights"),
        ] {
            assert!(known_standalone_root((Some(namespace), local_name)));
        }
        for (namespace, local_name) in [
            ("http://v8.1c.ru/8.3/xcf/predef", "PredefinedData"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "ExtPicture"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "JobSchedule"),
            ("http://v8.1c.ru/8.3/xcf/extrnprops", "AdditionalIndexes"),
            ("http://v8.1c.ru/8.3/xcf/dumpinfo", "ConfigDumpInfo"),
            (
                "http://v8.1c.ru/8.2/managed-application/core",
                "ClientApplicationInterface",
            ),
        ] {
            assert!(!known_standalone_root((Some(namespace), local_name)));
        }
    }

    #[test]
    fn inline_command_definitions_register_the_command_by_name() {
        let xml = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><Catalog><Properties><Name>Items</Name></Properties><ChildObjects><Form>ItemForm</Form><Command uuid="10000000-0000-4000-8000-000000000001"><Properties><Name>Refresh</Name><Synonym/></Properties></Command><Command uuid="10000000-0000-4000-8000-000000000002"><Properties><Comment/></Properties></Command></ChildObjects></Catalog></MetaDataObject>"#
        );
        let evidence =
            prove_already_read_metadata_owner(Path::new("Catalogs/Items.xml"), xml.as_bytes())
                .unwrap_or_else(|error| panic!("{}", error.message));
        let registrations = evidence.registrations().collect::<Vec<_>>();
        assert_eq!(
            registrations,
            vec![("Command", "Refresh"), ("Form", "ItemForm")],
            "a command is registered by its inline definition; a nameless one registers nothing"
        );
    }

    #[test]
    fn strict_metadata_owner_evidence_accepts_registered_physical_child_families() {
        for kind in ["Form", "Template", "Command"] {
            let xml = format!(
                r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><{kind}><Properties><Name>Main</Name></Properties></{kind}></MetaDataObject>"#
            );
            let evidence = prove_already_read_metadata_owner(
                Path::new(&format!("{kind}s/Main.xml")),
                xml.as_bytes(),
            )
            .unwrap_or_else(|error| panic!("{kind}: {}", error.message));
            assert_eq!(evidence.artifact_kind(), kind);
            assert_eq!(evidence.artifact_name(), Some("Main"));
        }
    }
}

use crate::layout::metadata_descriptor_relative;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOwnerDescriptorProof {
    SourceSet,
    Metadata,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOwnerDescriptorExpectation {
    pub relative: PathBuf,
    pub proof: EventOwnerDescriptorProof,
    pub expected_kind: String,
    pub expected_name: Option<String>,
    pub registered_by: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOwnerEvidencePlan {
    pub source_set_kind: SourceSetKind,
    pub descriptors: Vec<EventOwnerDescriptorExpectation>,
}
pub fn owner_evidence_plan(
    source_set_kind: SourceSetKind,
    target: &MetadataAddress,
    descriptor_requirements: &[PathBuf],
) -> Result<EventOwnerEvidencePlan, String> {
    let parts = target.as_str().split('.').collect::<Vec<_>>();
    let external = matches!(
        source_set_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    );
    let mut descriptors = Vec::new();
    if external {
        let [owner_kind, owner_name, ..] = parts.as_slice() else {
            return Err(String::from(
                "external event target has no typed owner identity",
            ));
        };
        let root_target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{owner_kind}.{owner_name}"),
        )
        .map_err(|error| error.to_string())?;
        descriptors.push(EventOwnerDescriptorExpectation {
            relative: metadata_descriptor_relative(&root_target, source_set_kind)?,
            proof: EventOwnerDescriptorProof::SourceSet,
            expected_kind: (*owner_kind).to_string(),
            expected_name: Some((*owner_name).to_string()),
            registered_by: None,
        });
    } else {
        descriptors.push(EventOwnerDescriptorExpectation {
            relative: PathBuf::from("Configuration.xml"),
            proof: EventOwnerDescriptorProof::SourceSet,
            expected_kind: match source_set_kind {
                SourceSetKind::Configuration => "Configuration",
                SourceSetKind::Extension => "Configuration",
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => unreachable!(),
            }
            .to_string(),
            expected_name: None,
            registered_by: None,
        });
    }

    for relative in descriptor_requirements {
        if descriptors.iter().any(|step| step.relative == *relative) {
            continue;
        }
        let index = descriptors.len();
        let identity_start = if external { 2 * index } else { 2 * (index - 1) };
        let identity = parts
            .get(identity_start..identity_start + 2)
            .ok_or_else(|| String::from("owner descriptor depth is inconsistent"))?;
        descriptors.push(EventOwnerDescriptorExpectation {
            relative: relative.clone(),
            proof: EventOwnerDescriptorProof::Metadata,
            expected_kind: identity[0].to_string(),
            expected_name: Some(identity[1].to_string()),
            registered_by: Some(index - 1),
        });
    }
    Ok(EventOwnerEvidencePlan {
        source_set_kind,
        descriptors,
    })
}

/// Revalidated resource paths whose field meaning follows the closed handle's
/// target kind:
///
/// - `Module`: `target_path` is the BSL module and `descriptor_paths` contains
///   the descriptors required by its layout; `module_paths` is empty.
/// - `MetadataObject`: `target_path` is the object descriptor and
///   `module_paths` contains its proven owned BSL modules; `descriptor_paths`
///   is empty.
/// - `SourceRoot`: `target_path` is the source root and `descriptor_paths`
///   contains `Configuration.xml`; `module_paths` is empty.
pub struct PlatformXmlResourceEvidence {
    pub target_path: PathBuf,
    pub source_root: PathBuf,
    pub descriptor_paths: Vec<PathBuf>,
    /// Proven BSL modules owned by a metadata object target. Empty for module
    /// and source-root targets, which are not module owners themselves.
    pub module_paths: Vec<PathBuf>,
    pub registration_path: PathBuf,
    pub module_owner: Option<String>,
}

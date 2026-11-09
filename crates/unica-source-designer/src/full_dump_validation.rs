//! Designer full-dump semantic validation of workspace-observed immutable bytes.
use crate::format_profile::{
    platform_line_for_export_format, writable_platform_xml_version, WRITABLE_EXPORT_FORMATS,
};
use crate::roots::{
    platform_xml_publication_policy as staged_root_version_policy, root_version_literal,
    PlatformXmlPublicationPolicy as StagedRootVersionPolicy,
};
use roxmltree::Document;
use std::path::Path;
use unica_source_contract::read::full_dump::{
    DumpValidationFailure, DumpValidationReason, DumpVersionRequirement,
};
use unica_source_contract::SourceSetKind;
use DumpValidationReason::*;
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
/// Validates the owner of a dump or of the source set it replaces and returns
/// its export format: one of the writable formats, written literally.
pub fn validate_required_owner_raw(
    path: &Path,
    kind: SourceSetKind,
    raw: &[u8],
) -> Result<&'static str, DumpValidationFailure> {
    let text = std::str::from_utf8(raw).map_err(|error| {
        DumpValidationFailure::new(
            InvalidXml,
            Some(path),
            format!("staged source-set owner is not UTF-8: {error}"),
        )
    })?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source).map_err(|error| {
        DumpValidationFailure::new(
            InvalidXml,
            Some(path),
            format!("failed to parse staged owner {}: {error}", path.display()),
        )
    })?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(DumpValidationFailure::new(
            InvalidOwner,
            Some(path),
            format!(
                "staged owner must be {{{MD_CLASSES_NS}}}MetaDataObject: {}",
                path.display()
            ),
        ));
    }
    let version = root_version_literal(source, root);
    let Some(format) = writable_platform_xml_version(version.as_deref()) else {
        return Err(DumpValidationFailure::new(WrongVersion, Some(path), format!(
            "staged owner export format must be the exact raw literal of a writable format (2.20, 2.21 or 2.22); found {} in {}",
            version.as_deref().unwrap_or("<missing>"),
            path.display()
        )).with_version(DumpVersionRequirement::OneOf(&WRITABLE_EXPORT_FORMATS)));
    };
    let children = root
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    if children.len() != 1
        || children[0].tag_name().namespace() != Some(MD_CLASSES_NS)
        || children[0].tag_name().name() != "Configuration"
    {
        return Err(DumpValidationFailure::new(InvalidOwner, Some(path), format!(
            "staged owner must contain exactly one direct {{{MD_CLASSES_NS}}}Configuration child: {}",
            path.display()
        )));
    }
    let is_extension = is_configuration_extension(children[0]);
    match (kind, is_extension) {
        (SourceSetKind::Configuration, false) | (SourceSetKind::Extension, true) => Ok(format),
        (SourceSetKind::Configuration, true) => Err(DumpValidationFailure::new(InvalidOwner, Some(path), format!(
            "CONFIGURATION source-set produced an extension owner: {}",
            path.display()
        ))),
        (SourceSetKind::Extension, false) => Err(DumpValidationFailure::new(InvalidOwner, Some(path), format!(
            "EXTENSION source-set produced a configuration owner without ConfigurationExtensionPurpose: {}",
            path.display()
        ))),
        _ => Err(DumpValidationFailure::new(InvalidOwner, Some(path), "external source-set kind reached guarded dump validation".into())),
    }
}

fn is_configuration_extension(configuration: roxmltree::Node<'_, '_>) -> bool {
    configuration
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

/// Validates one staged XML file of a dump whose owner records `format`.
pub fn validate_xml_raw(
    path: &Path,
    raw: &[u8],
    format: &'static str,
) -> Result<(), DumpValidationFailure> {
    let text = std::str::from_utf8(raw).map_err(|error| {
        DumpValidationFailure::new(
            InvalidXml,
            Some(path),
            format!("staged XML is not UTF-8 {}: {error}", path.display()),
        )
    })?;
    let source = text.trim_start_matches('\u{feff}');
    let document = Document::parse(source).map_err(|error| {
        DumpValidationFailure::new(
            InvalidXml,
            Some(path),
            format!("failed to parse staged XML {}: {error}", path.display()),
        )
    })?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or("");
    let local_name = root.tag_name().name();
    match staged_root_version_policy(namespace, local_name) {
        Some(StagedRootVersionPolicy::Versionless) => {
            if let Some(version) = root_version_literal(source, root) {
                return Err(DumpValidationFailure::new(WrongVersion, Some(path), format!(
                    "staged XML root {{{namespace}}}{local_name} is a registered versionless family and must not declare version={version} in {}",
                    path.display()
                )).with_root(namespace, local_name).with_version(DumpVersionRequirement::Absent));
            }
        }
        Some(StagedRootVersionPolicy::ExactRootVersion) => {
            let version = root_version_literal(source, root);
            if version.as_deref() != Some(format) {
                return Err(DumpValidationFailure::new(WrongVersion, Some(path), format!(
                    "staged XML root {{{namespace}}}{local_name} must use the exact raw version literal {format} of its owner; found {} (missing means legacy format 1.0) in {}",
                    version.as_deref().unwrap_or("<missing>"),
                    path.display()
                )).with_root(namespace, local_name).with_version(DumpVersionRequirement::Exact(format)));
            }
        }
        None => {
            return Err(DumpValidationFailure::new(UnsupportedRoot, Some(path), format!(
                "staged XML root {{{namespace}}}{local_name} is unsupported by the closed platform {} / export format {format} registry: {}",
                platform_line_for_export_format(format).unwrap_or_default(),
                path.display()
            )).with_root(namespace, local_name));
        }
    }
    Ok(())
}

use unica_source_contract::{
    read::full_dump::{DumpInventoryClassifier, DumpSemanticValidator, DumpTreeObservation},
    SourceFormat,
};

/// Binds the existing cross-format inventory classifier without owning source
/// discovery or introducing another marker table in this format codec.
pub struct DesignerDumpValidator {
    classify_inventory: DumpInventoryClassifier,
}
impl DesignerDumpValidator {
    pub const fn new(classify_inventory: DumpInventoryClassifier) -> Self {
        Self { classify_inventory }
    }
}
impl DumpSemanticValidator for DesignerDumpValidator {
    fn validate_stage(
        &self,
        tree: &DumpTreeObservation<'_>,
        kind: SourceSetKind,
    ) -> Result<(), DumpValidationFailure> {
        let root = tree.display_root();
        if !tree.is_directory() {
            return Err(DumpValidationFailure::new(
                EmptyDump,
                None,
                format!(
                    "v8-runner did not create the private staged dump directory: {}",
                    root.display()
                ),
            ));
        }
        if tree.entries().next().is_none() {
            return Err(DumpValidationFailure::new(
                EmptyDump,
                None,
                format!("private staged dump is empty: {}", root.display()),
            ));
        }
        let owner = tree
            .entries()
            .find(|entry| entry.relative_path() == Path::new("Configuration.xml"))
            .ok_or_else(|| {
                DumpValidationFailure::new(
                    MissingOwner,
                    Some(Path::new("Configuration.xml")),
                    format!(
                        "private staged dump has no Configuration.xml owner: {}",
                        root.display()
                    ),
                )
            })?;
        let raw = owner
            .read_exact("staged source-set owner")
            .map_err(|detail| {
                DumpValidationFailure::new(UnreadableSource, Some(owner.relative_path()), detail)
            })?;
        let format = validate_required_owner_raw(owner.relative_path(), kind, &raw)?;
        for entry in tree.entries() {
            if !entry.is_file()
                || !entry
                    .relative_path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
            {
                continue;
            }
            let raw = entry.read_exact("staged XML").map_err(|detail| {
                DumpValidationFailure::new(UnreadableSource, Some(entry.relative_path()), detail)
            })?;
            validate_xml_raw(entry.relative_path(), &raw, format)?;
        }
        Ok(())
    }
    fn validate_existing_target(
        &self,
        tree: &DumpTreeObservation<'_>,
        kind: SourceSetKind,
    ) -> Result<Option<&'static str>, String> {
        if !tree.is_directory() {
            return Ok(None);
        }
        let entries = tree
            .entries()
            .filter(|entry| entry.is_file())
            .collect::<Vec<_>>();
        let files = entries
            .iter()
            .map(|entry| entry.relative_path())
            .collect::<Vec<_>>();
        if files.is_empty() {
            return Ok(None);
        }
        let target = tree.display_root();
        match (self.classify_inventory)(kind, &files) {
            SourceFormat::PlatformXml => {
                let owner = entries.iter().find(|entry| entry.relative_path() == Path::new("Configuration.xml"))
                    .ok_or_else(|| format!("staged source-set owner is not a file in the captured tree snapshot: {}", target.join("Configuration.xml").display()))?;
                owner.read_exact("staged source-set owner").and_then(|raw| validate_required_owner_raw(&owner.display_path(), kind, &raw).map(Some).map_err(|error| error.to_string())).map_err(|error| format!("{error}; the existing DESIGNER target is not in a writable export format (2.20, 2.21 or 2.22) and will not be migrated automatically. Explicitly re-export it with platform 8.3.27, 8.5.1 or 8.5.4 and retry"))
            }
            SourceFormat::Edt => Err(format!("physical dump target is EDT and cannot be replaced by a DESIGNER YAML claim: {}", target.display())),
            SourceFormat::Invalid => Err(format!("physical dump target has mixed/invalid DESIGNER and EDT markers: {}", target.display())),
            SourceFormat::Unknown => Err(format!("physical dump target has no authoritative DESIGNER marker and cannot be replaced: {}", target.display())),
        }
    }
}

#[cfg(test)]
mod style_root_tests {
    use super::validate_xml_raw;
    use std::path::Path;

    fn style(version: &str) -> String {
        format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <Style xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" version=\"{version}\"/>"
        )
    }

    /// Конфигурация со стилем выгружает `Styles/*/Ext/Style.xml`: без этого
    /// корня полная выгрузка такой конфигурации не публикуется.
    #[test]
    fn a_full_dump_accepts_the_style_of_a_configuration() {
        let path = Path::new("Styles/Main/Ext/Style.xml");
        assert!(validate_xml_raw(path, style("2.20").as_bytes(), "2.20").is_ok());
    }

    /// rule: INV.SOURCE.WRITABLE-PROFILE
    #[test]
    fn a_dump_of_a_writable_format_is_validated_against_its_owner_format() {
        use super::validate_required_owner_raw;
        use unica_source_contract::SourceSetKind;
        let owner = |version: &str| {
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration><Properties/></Configuration></MetaDataObject>"#
            )
        };
        let path = Path::new("Configuration.xml");
        for version in ["2.20", "2.21", "2.22"] {
            assert_eq!(
                validate_required_owner_raw(
                    path,
                    SourceSetKind::Configuration,
                    owner(version).as_bytes()
                )
                .unwrap(),
                version
            );
        }
        for version in ["2.19", "2.23", "2.21.0"] {
            assert!(validate_required_owner_raw(
                path,
                SourceSetKind::Configuration,
                owner(version).as_bytes()
            )
            .is_err());
        }
        // Every versioned root of a dump follows its owner's format.
        let style_path = Path::new("Styles/Main/Ext/Style.xml");
        assert!(validate_xml_raw(style_path, style("2.21").as_bytes(), "2.21").is_ok());
        assert!(validate_xml_raw(style_path, style("2.20").as_bytes(), "2.21").is_err());
    }

    #[test]
    fn a_style_of_another_export_format_stays_refused() {
        let path = Path::new("Styles/Main/Ext/Style.xml");
        assert!(validate_xml_raw(path, style("2.19").as_bytes(), "2.20").is_err());
    }
}

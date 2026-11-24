//! Existing Configuration/Language profile reads, including lazy staged observations.
use crate::owner::MD_CLASSES_NS;
use std::path::Path;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_source_contract::SourceSetKind;
use unica_xml_format::names::xml_ncname_is_valid;
use unica_xml_format::read::{
    child_direct_text, child_element, child_elements_in_namespace, direct_text,
    parse_metadata_image,
};

/// Language of new multilingual values in external processors and reports.
///
/// An external processor or report has no `Configuration.xml` and no
/// `Languages` of its own: at run time it takes the languages of whatever
/// configuration opens it, so its dump carries no default language to read.
/// Unica's external templates are created with `ru`, and new values in them
/// keep that explicit fallback instead of guessing a profile.
pub const EXTERNAL_SOURCE_TEXT_LANGUAGE: &str = "ru";

/// `LanguageCode` of the default language of a configuration or extension.
///
/// `configuration` is the source set's root `Configuration.xml`; `read`
/// returns a file of the same source set by its relative path. The default
/// language is `Configuration/Properties/DefaultLanguage` (`Language.<Name>`),
/// and its code is `LanguageCode` of `Languages/<Name>.xml`. An extension has
/// its own `Configuration.xml` and adopted languages, so the same reading
/// serves both.
pub fn default_text_language(
    configuration: &[u8],
    read: impl FnOnce(&std::path::Path) -> Result<Option<Vec<u8>>, String>,
) -> Result<String, String> {
    resolve_text_language(configuration, read, LanguageSelection::Default)
}

#[derive(Clone, Copy)]
enum LanguageSelection {
    Default,
    ExtensionCreation,
}

fn resolve_text_language(
    configuration: &[u8],
    read: impl FnOnce(&Path) -> Result<Option<Vec<u8>>, String>,
    selection: LanguageSelection,
) -> Result<String, String> {
    let (_, document) = parse_metadata_image(configuration)?;
    let mut roots = document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, "Configuration")));
    let root = roots
        .next()
        .ok_or_else(|| "the source set root descriptor is not a Configuration".to_string())?;
    let selected = match selection {
        LanguageSelection::Default => child_element(root, "Properties")
            .and_then(|properties| child_direct_text(properties, "DefaultLanguage")),
        LanguageSelection::ExtensionCreation => {
            if roots.next().is_some() {
                return Err("the extension has duplicate Configuration descriptors".to_string());
            }
            let properties = child_elements_in_namespace(root, "Properties", MD_CLASSES_NS);
            let [properties] = properties.as_slice() else {
                return Err(
                    "the extension must contain one Configuration Properties element".to_string(),
                );
            };
            let defaults = properties
                .children()
                .filter(|child| child.is_element() && child.tag_name().name() == "DefaultLanguage")
                .collect::<Vec<_>>();
            match defaults.as_slice() {
                [] => return registered_creation_language(root, read),
                [node]
                    if node.tag_name().namespace() == Some(MD_CLASSES_NS)
                        && !node.children().any(|child| child.is_element()) =>
                {
                    Some(direct_text(*node))
                }
                _ => {
                    return Err(
                        "the extension has an invalid or duplicate DefaultLanguage".to_string()
                    )
                }
            }
        }
    };

    let default = selected
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "the configuration has no DefaultLanguage".to_string())?;
    let name = default.strip_prefix("Language.").unwrap_or(default);
    let path = std::path::Path::new("Languages").join(format!("{name}.xml"));
    // Messages name the descriptor by its source-set path with `/` on every OS;
    // `Path::display` would print `Languages\English.xml` on Windows.
    let descriptor = format!("Languages/{name}.xml");
    let bytes = read(&path)?
        .ok_or_else(|| format!("the default language descriptor {descriptor} was not found"))?;
    let (language, code) = inspect_language_image(&bytes)?
        .ok_or_else(|| format!("{descriptor} is not a Language descriptor"))?;
    if language != name {
        return Err(format!(
            "{descriptor} describes language `{language}`, not the default `{name}`"
        ));
    }
    Ok(code)
}

/// The synthesized synonym for a new object can use the only registered
/// extension language. This does not establish a default language for the CFE.
fn registered_creation_language(
    root: roxmltree::Node<'_, '_>,
    read: impl FnOnce(&Path) -> Result<Option<Vec<u8>>, String>,
) -> Result<String, String> {
    let children = child_elements_in_namespace(root, "ChildObjects", MD_CLASSES_NS);
    let languages = match children.as_slice() {
        [] => Vec::new(),
        [children] => child_elements_in_namespace(*children, "Language", MD_CLASSES_NS),
        _ => return Err("the extension has duplicate ChildObjects".to_string()),
    };
    let [language] = languages.as_slice() else {
        return Err(format!(
            "the extension has no DefaultLanguage and registers {} languages; object creation requires an explicit DefaultLanguage or exactly one registered language",
            languages.len()
        ));
    };
    let name = direct_text(*language);
    let name = name.trim();
    if language.children().any(|child| child.is_element()) || !xml_ncname_is_valid(name) {
        return Err("the extension has an invalid registered language name".to_string());
    }
    let descriptor = format!("Languages/{name}.xml");
    let bytes = read(Path::new(&descriptor))?
        .ok_or_else(|| format!("the registered language descriptor {descriptor} was not found"))?;
    let (_, document) = parse_metadata_image(&bytes)?;
    let declarations =
        child_elements_in_namespace(document.root_element(), "Language", MD_CLASSES_NS);
    let [declaration] = declarations.as_slice() else {
        return Err(format!("{descriptor} must contain one Language descriptor"));
    };
    let properties = child_elements_in_namespace(*declaration, "Properties", MD_CLASSES_NS);
    let [properties] = properties.as_slice() else {
        return Err(format!(
            "{descriptor} must contain one Language Properties element"
        ));
    };
    let field = |key| {
        let nodes = child_elements_in_namespace(*properties, key, MD_CLASSES_NS);
        match nodes.as_slice() {
            [node] if !node.children().any(|child| child.is_element()) => {
                let value = direct_text(*node);
                let value = value.trim();
                if !value.is_empty() {
                    return Ok(value.to_string());
                }
            }
            _ => {}
        }
        Err(format!("{descriptor} must contain one nonempty {key}"))
    };
    let actual = field("Name")?;
    if actual != name {
        return Err(format!(
            "{descriptor} describes language `{actual}`, not the registered `{name}`"
        ));
    }
    field("LanguageCode")
}

pub fn inspect_language_image(bytes: &[u8]) -> Result<Option<(String, String)>, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let root = document.root_element();
    let language = root.children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "Language"
    });
    let Some(language) = language else {
        return Ok(None);
    };
    let properties = child_element(language, "Properties");
    let name = properties
        .and_then(|properties| child_element(properties, "Name"))
        .map(direct_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Language Name is missing".to_string())?;
    let code = properties
        .and_then(|properties| child_element(properties, "LanguageCode"))
        .map(direct_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Language {name} has empty LanguageCode"))?;
    Ok(Some((name, code)))
}

/// `LanguageCode` of the default language of the edited source set: new
/// multilingual values are written in it (#909). It is read from the staged
/// `Configuration.xml` and `Languages/<Name>.xml`, so an extension uses its
/// own adopted languages. External processors and reports have no language
/// profile; see [`EXTERNAL_SOURCE_TEXT_LANGUAGE`].
pub fn staged_text_language(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    op_index: usize,
) -> Result<String, ApplyPlanError> {
    staged_language(staged, source_kind, op_index, LanguageSelection::Default)
}

pub fn staged_object_creation_language(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    op_index: usize,
) -> Result<String, ApplyPlanError> {
    let selection = if source_kind == SourceSetKind::Extension {
        LanguageSelection::ExtensionCreation
    } else {
        LanguageSelection::Default
    };
    staged_language(staged, source_kind, op_index, selection)
}

fn staged_language(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    op_index: usize,
    selection: LanguageSelection,
) -> Result<String, ApplyPlanError> {
    if matches!(
        source_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return Ok(EXTERNAL_SOURCE_TEXT_LANGUAGE.to_string());
    }
    let at_path = format!("ops[{op_index}].args.at");
    let unavailable = |message: String| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("the configuration default language is unavailable: {message}"),
        )
        .at_path(at_path.clone())
    };
    let configuration = staged
        .read(Path::new("Configuration.xml"))
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| unavailable("the source set has no Configuration.xml".to_string()))?;
    let mut staging_error = None;
    let language = resolve_text_language(
        &configuration,
        |relative| {
            staged.read(relative).map_err(|error| {
                let message = error.to_string();
                staging_error = Some(error);
                message
            })
        },
        selection,
    );
    if let Some(error) = staging_error {
        return Err(ApplyPlanError::staging(error, at_path));
    }
    language.map_err(unavailable)
}

/// Script naming profile of a configuration or extension. Missing properties
/// retain Unica's Russian compatibility fallback; invalid values do not.
pub fn script_variant(
    configuration: &[u8],
) -> Result<unica_source_contract::ScriptVariant, String> {
    use unica_source_contract::ScriptVariant;
    let (_, document) = parse_metadata_image(configuration)?;
    let roots = document
        .root_element()
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Configuration"
        })
        .collect::<Vec<_>>();
    let [root] = roots.as_slice() else {
        return Err("the source set root must contain one Configuration".to_string());
    };
    let properties = root
        .children()
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
        .collect::<Vec<_>>();
    let [properties] = properties.as_slice() else {
        return Err("Configuration must contain one Properties".to_string());
    };
    let variants = properties
        .children()
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ScriptVariant")))
        .collect::<Vec<_>>();
    match variants.as_slice() {
        [] => Ok(ScriptVariant::Russian),
        [variant] if variant.children().all(|child| child.is_text()) => {
            match direct_text(*variant).trim() {
                "Russian" => Ok(ScriptVariant::Russian),
                "English" => Ok(ScriptVariant::English),
                _ => Err("ScriptVariant must be Russian or English".to_string()),
            }
        }
        _ => Err("Configuration has an invalid or duplicate ScriptVariant".to_string()),
    }
}

/// Read through the proposal so both earlier operations and publication guards
/// include the script profile used to generate names. External artifacts have
/// no configuration profile and retain the existing Russian fallback.
pub fn staged_script_variant(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    at_path: &str,
) -> Result<unica_source_contract::ScriptVariant, ApplyPlanError> {
    if matches!(
        source_kind,
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
    ) {
        return Ok(unica_source_contract::ScriptVariant::Russian);
    }
    let path = at_path.to_string();
    let bytes = staged
        .read(Path::new("Configuration.xml"))
        .map_err(|error| ApplyPlanError::staging(error, path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the source set has no Configuration.xml",
            )
            .at_path(path.clone())
        })?;
    script_variant(&bytes).map_err(|error| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("the configuration script variant is unavailable: {error}"),
        )
        .at_path(path)
    })
}

#[cfg(test)]
mod script_variant_tests {
    use super::script_variant;
    use unica_source_contract::ScriptVariant;

    fn image(properties: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties>{properties}</Properties></Configuration></MetaDataObject>"#
        )
    }

    #[test]
    fn script_variant_is_independent_of_the_default_text_language() {
        for (variant, language, expected) in [
            ("English", "Russian", ScriptVariant::English),
            ("Russian", "English", ScriptVariant::Russian),
        ] {
            assert_eq!(script_variant(image(&format!("<DefaultLanguage>Language.{language}</DefaultLanguage><ScriptVariant>{variant}</ScriptVariant>")).as_bytes()).unwrap(), expected);
        }
        assert_eq!(
            script_variant(image("").as_bytes()).unwrap(),
            ScriptVariant::Russian
        );
    }

    #[test]
    fn invalid_script_profiles_do_not_fall_back_to_russian() {
        for properties in [
            "<ScriptVariant/>",
            "<ScriptVariant>Unknown</ScriptVariant>",
            "<ScriptVariant>English</ScriptVariant><ScriptVariant>Russian</ScriptVariant>",
            "<ScriptVariant><Value>English</Value></ScriptVariant>",
        ] {
            assert!(
                script_variant(image(properties).as_bytes()).is_err(),
                "{properties}"
            );
        }
        assert!(script_variant(b"<Configuration/>").is_err());
        assert!(script_variant(b"not xml").is_err());
    }
}

#[cfg(test)]
mod creation_language_tests {
    use super::*;

    fn configuration(default: &str, languages: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}"><Configuration><Properties>{default}</Properties><ChildObjects>{languages}</ChildObjects></Configuration></MetaDataObject>"#
        )
    }

    fn descriptor(name: &str, properties: &str) -> Vec<u8> {
        format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}"><Language><Properties><Name>{name}</Name>{properties}</Properties></Language></MetaDataObject>"#
        )
        .into_bytes()
    }

    #[test]
    fn object_creation_uses_registered_descriptor_code_only_in_extension_mode() {
        let image = configuration("", "<Language>Русский</Language>");
        let read = |path: &Path| {
            assert_eq!(path, Path::new("Languages/Русский.xml"));
            Ok(Some(descriptor(
                "Русский",
                "<LanguageCode>en</LanguageCode>",
            )))
        };
        assert_eq!(
            resolve_text_language(image.as_bytes(), read, LanguageSelection::ExtensionCreation),
            Ok("en".to_string())
        );
        assert!(
            default_text_language(image.as_bytes(), |_| panic!("unexpected descriptor read"))
                .is_err()
        );
    }

    #[test]
    fn explicit_default_keeps_priority_and_missing_reference_does_not_fall_back() {
        let image = configuration(
            "<DefaultLanguage>Language.English</DefaultLanguage>",
            "<Language>Русский</Language><Language>English</Language>",
        );
        let read = |path: &Path| {
            assert_eq!(path, Path::new("Languages/English.xml"));
            Ok(Some(descriptor(
                "English",
                "<LanguageCode>en</LanguageCode>",
            )))
        };
        assert_eq!(
            resolve_text_language(image.as_bytes(), read, LanguageSelection::ExtensionCreation),
            Ok("en".to_string())
        );
        let image = configuration(
            "<DefaultLanguage>Language.Missing</DefaultLanguage>",
            "<Language>Русский</Language>",
        );
        let error = resolve_text_language(
            image.as_bytes(),
            |path| {
                assert_eq!(path, Path::new("Languages/Missing.xml"));
                Ok(None)
            },
            LanguageSelection::ExtensionCreation,
        )
        .unwrap_err();
        assert!(error.contains("default language descriptor Languages/Missing.xml"));
    }

    #[test]
    fn missing_ambiguous_and_malformed_language_evidence_refuses_before_reading() {
        for (default, languages, expected) in [
            ("", "", "registers 0 languages"),
            (
                "",
                "<Language>Русский</Language><Language>English</Language>",
                "registers 2 languages",
            ),
            (
                "",
                "<Language>../Other</Language>",
                "invalid registered language name",
            ),
            (
                "<DefaultLanguage/>",
                "<Language>Русский</Language>",
                "has no DefaultLanguage",
            ),
            (
                "<DefaultLanguage/><DefaultLanguage/>",
                "<Language>Русский</Language>",
                "duplicate DefaultLanguage",
            ),
        ] {
            let image = configuration(default, languages);
            let error = resolve_text_language(
                image.as_bytes(),
                |_| panic!("unexpected descriptor read"),
                LanguageSelection::ExtensionCreation,
            )
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn duplicate_configuration_properties_cannot_hide_an_explicit_default() {
        let image = configuration("", "<Language>Русский</Language>").replace(
            "</Properties>",
            "</Properties><Properties><DefaultLanguage>Language.English</DefaultLanguage></Properties>",
        );
        let error = resolve_text_language(
            image.as_bytes(),
            |_| panic!("ambiguous properties must refuse before reading a descriptor"),
            LanguageSelection::ExtensionCreation,
        )
        .unwrap_err();
        assert!(error.contains("one Configuration Properties element"));
    }

    #[test]
    fn a_registered_language_requires_a_matching_descriptor_with_one_code() {
        let image = configuration("", "<Language>Русский</Language>");
        for (bytes, expected) in [
            (None, "descriptor Languages/Русский.xml was not found"),
            (
                Some(descriptor("English", "<LanguageCode>en</LanguageCode>")),
                "not the registered",
            ),
            (
                Some(descriptor("Русский", "<LanguageCode/>")),
                "one nonempty LanguageCode",
            ),
            (
                Some(descriptor(
                    "Русский",
                    "<LanguageCode>ru</LanguageCode><LanguageCode>en</LanguageCode>",
                )),
                "one nonempty LanguageCode",
            ),
        ] {
            let error = resolve_text_language(
                image.as_bytes(),
                |_| Ok(bytes),
                LanguageSelection::ExtensionCreation,
            )
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }
}

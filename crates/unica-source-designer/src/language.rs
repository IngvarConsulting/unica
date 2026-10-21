//! Existing Configuration/Language profile reads, including lazy staged observations.
use crate::owner::MD_CLASSES_NS;
use std::path::Path;
use unica_source_contract::proposal::{ApplyPlanError, ApplyPlanErrorKind, ApplyProposal};
use unica_source_contract::SourceSetKind;
use unica_xml_format::read::{child_direct_text, child_element, direct_text, parse_metadata_image};

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
    let (_, document) = parse_metadata_image(configuration)?;
    let root = document
        .root_element()
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Configuration"
        })
        .ok_or_else(|| "the source set root descriptor is not a Configuration".to_string())?;
    let default = child_element(root, "Properties")
        .and_then(|properties| child_direct_text(properties, "DefaultLanguage"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "the configuration has no DefaultLanguage".to_string())?;
    let name = default.strip_prefix("Language.").unwrap_or(&default);
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
    let language = default_text_language(&configuration, |relative| {
        staged.read(relative).map_err(|error| {
            let message = error.to_string();
            staging_error = Some(error);
            message
        })
    });
    if let Some(error) = staging_error {
        return Err(ApplyPlanError::staging(error, at_path));
    }
    language.map_err(unavailable)
}

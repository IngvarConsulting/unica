//! A LanguageCode edit is safe only for an unused additional language.

use roxmltree::{Document, Node};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyProposal, StagedFileState,
};
use unica_source_contract::SourceSetKind;

use crate::info_projection::{
    direct_meta_info_property, direct_text_content, meta_info_property_value_is_valid,
};
use unica_metadata::MetaInfoPropertyValueKind;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const CORE: &str = "http://v8.1c.ru/8.1/data/core";
const EXTRN: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

struct Language {
    name: String,
    uuid: String,
    code: String,
    range: Range<usize>,
    inherited_default_is_md: bool,
}

fn owner<'a, 'input>(
    document: &'a Document<'input>,
    kind: &str,
) -> Result<Node<'a, 'input>, String> {
    let root = document.root_element();
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    match owners.as_slice() {
        [node] if root.has_tag_name((MD, "MetaDataObject")) && node.has_tag_name((MD, kind)) => {
            Ok(*node)
        }
        _ => Err(format!(
            "descriptor must contain one {kind} owner in the metadata namespace"
        )),
    }
}
fn child<'a, 'input>(node: Node<'a, 'input>, tag: &str) -> Result<Node<'a, 'input>, String> {
    let matches = node
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == tag)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [node] if node.has_tag_name((MD, tag)) => Ok(*node),
        _ => Err(format!("{tag} must occur once in the metadata namespace")),
    }
}
fn scalar<'a, 'input>(
    properties: Node<'a, 'input>,
    kind: &str,
    tag: &str,
) -> Result<(Node<'a, 'input>, String), String> {
    let node = direct_meta_info_property(properties, kind, tag)
        .map_err(|error| error.message)?
        .ok_or_else(|| format!("{kind}.{tag} is missing"))?;
    if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::String) {
        return Err(format!("{kind}.{tag} is malformed"));
    }
    Ok((node, direct_text_content(node)))
}
fn inspect(text: &str) -> Result<Language, String> {
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let language = owner(&document, "Language")?;
    let properties = child(language, "Properties")?;
    let (_, name) = scalar(properties, "Language", "Name")?;
    if !unica_metadata::identifier::is_1c_identifier(&name) {
        return Err("Language.Name is invalid".to_string());
    }
    let uuid = language
        .attribute("uuid")
        .ok_or("Language UUID is missing")?;
    uuid::Uuid::parse_str(uuid).map_err(|_| "Language UUID is invalid")?;
    let (node, code) = scalar(properties, "Language", "LanguageCode")?;
    if code.is_empty() {
        return Err("Language.LanguageCode is empty".to_string());
    }
    Ok(Language {
        name,
        uuid: uuid.to_string(),
        code,
        range: node.range(),
        inherited_default_is_md: properties.lookup_namespace_uri(None) == Some(MD),
    })
}

pub fn read_code(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| error.to_string())?
        .trim_start_matches('\u{feff}');
    inspect(text).map(|language| language.code)
}

pub(crate) fn replace(text: &str, name: &str, code: &str) -> Result<String, String> {
    let language = inspect(text)?;
    if language.name != name {
        return Err(format!(
            "Language descriptor describes {}, not {name}",
            language.name
        ));
    }
    if language.code == code {
        return Ok(text.to_string());
    }
    let mut result = text.to_string();
    // Preserve scalar text through XML and the descriptor's line-ending policy.
    let escaped = unica_xml_format::text::escape_xml(code)
        .replace('\r', "&#13;")
        .replace('\n', "&#10;");
    let namespace = if language.inherited_default_is_md {
        String::new()
    } else {
        format!(" xmlns=\"{MD}\"")
    };
    result.replace_range(
        language.range,
        &format!("<LanguageCode{namespace}>{escaped}</LanguageCode>"),
    );
    Ok(result)
}

fn same(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}
fn error(kind: ApplyPlanErrorKind, path: &Path, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(kind, message).at_path(path.to_string_lossy().replace('\\', "/"))
}
fn source_text(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes)
        .map(|text| text.trim_start_matches('\u{feff}'))
        .map_err(|error| error.to_string())
}

fn registration(bytes: &[u8], target: &str) -> Result<Vec<String>, String> {
    let text = source_text(bytes)?;
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let configuration = owner(&document, "Configuration")?;
    let properties = child(configuration, "Properties")?;
    let (_, default) = scalar(properties, "Configuration", "DefaultLanguage")?;
    let default = default
        .strip_prefix("Language.")
        .ok_or("DefaultLanguage must name Language.<Name>")?;
    let children = child(configuration, "ChildObjects")?;
    let mut names: Vec<String> = Vec::new();
    for node in children
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Language")
    {
        if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::String) {
            return Err("registered Language is malformed".to_string());
        }
        let name = direct_text_content(node);
        if !unica_metadata::identifier::is_1c_identifier(&name)
            || names.iter().any(|other| same(other, &name))
        {
            return Err("registered languages must have distinct valid names".to_string());
        }
        names.push(name);
    }
    if !names.iter().any(|name| same(name, target)) {
        return Err(format!(
            "Language {target} is not registered in Configuration.ChildObjects"
        ));
    }
    if !names.iter().any(|name| same(name, default)) {
        return Err("DefaultLanguage is not registered".to_string());
    }
    if same(default, target) {
        return Err(
            "LanguageCode can change only for an additional language, not DefaultLanguage"
                .to_string(),
        );
    }
    Ok(names)
}

/// NStr's language labels are outside its single-quoted translations. A quote
/// doubled inside a translation is text, never the start of another label.
fn literal_languages(mut text: &str) -> Result<Vec<String>, String> {
    let mut labels = Vec::new();
    loop {
        text = text.trim_start();
        if text.is_empty() {
            return Ok(labels);
        }
        let (label, rest) = text
            .split_once('=')
            .ok_or("NStr literal has no language separator")?;
        let label = label.trim();
        if label.is_empty() {
            return Err("NStr literal has an empty language label".to_string());
        }
        labels.push(label.to_string());
        text = rest
            .trim_start()
            .strip_prefix('\'')
            .ok_or("NStr translation is not quoted")?;
        loop {
            let quote = text.find('\'').ok_or("NStr translation is not closed")?;
            text = &text[quote + 1..];
            if let Some(rest) = text.strip_prefix('\'') {
                text = rest;
            } else {
                break;
            }
        }
        text = text.trim_start();
        if text.is_empty() {
            return Ok(labels);
        }
        text = text
            .strip_prefix(';')
            .ok_or("NStr translations must be separated by semicolons")?;
    }
}
fn dependencies(bytes: &[u8], path: &Path, old: &str, new: &str) -> Result<(), String> {
    let uses = |code: &str| same(code, old) || same(code, new);
    let text = source_text(bytes)?;
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bsl"))
    {
        let parsed = unica_bsl_parser::parse(text);
        if !parsed.errors().is_empty() {
            return Err("cannot prove language independence of BSL with syntax errors".to_string());
        }
        for call in parsed.calls().into_iter().filter(|call| {
            matches!(call.path.as_slice(), [name] if same(name, "NStr") || same(name, "НСтр"))
        }) {
            let literal = call.first_string_argument.ok_or("cannot prove language independence of a dynamic NStr/НСтр argument")?;
            if literal_languages(&literal)?.iter().any(|code| uses(code)) {
                return Err("NStr/НСтр contains a translation for the old or new language code".to_string());
            }
        }
    } else {
        let document = Document::parse(text)
            .map_err(|error| format!("cannot inspect XML language dependencies: {error}"))?;
        for node in document.descendants().filter(Node::is_element) {
            if (node.has_tag_name((CORE, "lang"))
                || (node.has_tag_name((EXTRN, "Page"))
                    && document.root_element().has_tag_name((EXTRN, "Help"))))
                && uses(&direct_text_content(node))
            {
                return Err(
                    "XML translations or embedded help use the old or new language code"
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

/// Run after every operation family. Dependency guards cover the original
/// source and the final overlay, including creations absent from disk enumeration.
pub(crate) fn validate_final(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
) -> Result<(), ApplyPlanError> {
    let changes = staged.planned_changes();
    let originals = changes
        .iter()
        .map(|change| (change.relative_path.clone(), change.original.as_option()))
        .collect::<BTreeMap<_, _>>();
    let mut edits = Vec::new();
    for change in &changes {
        if change.relative_path.parent() != Some(Path::new("Languages"))
            || change
                .relative_path
                .extension()
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("xml"))
        {
            continue;
        }
        let (StagedFileState::Bytes(original), StagedFileState::Bytes(current)) =
            (&change.original, &change.current)
        else {
            continue;
        };
        let inspect_bytes = |bytes: &[u8]| {
            source_text(bytes).and_then(inspect).map_err(|message| {
                error(
                    ApplyPlanErrorKind::InvalidSource,
                    &change.relative_path,
                    message,
                )
            })
        };
        let before = inspect_bytes(original)?;
        let after = inspect_bytes(current)?;
        if before.code == after.code {
            continue;
        }
        if source_kind != SourceSetKind::Configuration {
            return Err(error(
                ApplyPlanErrorKind::BadValue,
                &change.relative_path,
                "LanguageCode changes require an ordinary configuration",
            ));
        }
        if before.name != after.name || before.uuid != after.uuid {
            return Err(error(
                ApplyPlanErrorKind::BadValue,
                &change.relative_path,
                "LanguageCode edits must preserve name and UUID",
            ));
        }
        unica_metadata::language_code::validate_language_code(&before.code).map_err(|message| {
            error(
                ApplyPlanErrorKind::InvalidSource,
                &change.relative_path,
                message,
            )
        })?;
        unica_metadata::language_code::validate_language_code(&after.code).map_err(|message| {
            error(ApplyPlanErrorKind::BadValue, &change.relative_path, message)
        })?;
        edits.push((change.relative_path.clone(), before, after));
    }
    if edits.is_empty() {
        return Ok(());
    }
    let configuration_path = Path::new("Configuration.xml");
    let configuration = staged
        .read_guarded_bounded(configuration_path, usize::MAX)
        .map_err(|e| ApplyPlanError::staging(e, "Configuration.xml".to_string()))?
        .ok_or_else(|| {
            error(
                ApplyPlanErrorKind::InvalidSource,
                configuration_path,
                "Configuration.xml is missing",
            )
        })?;
    let original_configuration = originals
        .get(configuration_path)
        .and_then(Option::as_deref)
        .unwrap_or(&configuration);
    for (path, before, after) in &edits {
        registration(original_configuration, &before.name)
            .map_err(|message| error(ApplyPlanErrorKind::BadValue, path, message))?;
        let registered = registration(&configuration, &after.name)
            .map_err(|message| error(ApplyPlanErrorKind::BadValue, path, message))?;
        for name in registered
            .into_iter()
            .filter(|name| !same(name, &after.name))
        {
            let relative = PathBuf::from("Languages").join(format!("{name}.xml"));
            let bytes = staged
                .read_guarded_bounded(&relative, usize::MAX)
                .map_err(|e| ApplyPlanError::staging(e, relative.to_string_lossy()))?
                .ok_or_else(|| {
                    error(
                        ApplyPlanErrorKind::InvalidSource,
                        &relative,
                        "registered Language descriptor is missing",
                    )
                })?;
            let other = source_text(&bytes)
                .and_then(inspect)
                .map_err(|message| error(ApplyPlanErrorKind::InvalidSource, &relative, message))?;
            if !same(&other.name, &name) {
                return Err(error(
                    ApplyPlanErrorKind::InvalidSource,
                    &relative,
                    "registered Language descriptor has another name",
                ));
            }
            if same(&other.code, &after.code) {
                return Err(error(
                    ApplyPlanErrorKind::BadValue,
                    path,
                    "LanguageCode conflicts with another registered language",
                ));
            }
        }
    }
    let mut files = staged
        .enumerate_tree(Path::new(""))
        .map_err(|e| ApplyPlanError::staging(e, "LanguageCode".to_string()))?
        .into_iter()
        .collect::<BTreeSet<_>>();
    files.extend(changes.iter().map(|change| change.relative_path.clone()));
    for relative in files {
        let extension = relative
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("");
        if extension.eq_ignore_ascii_case("html")
            && relative.parent().is_some_and(|parent| {
                parent.file_name().is_some_and(|name| name == "Help")
                    && parent
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|name| name == "Ext")
            })
        {
            if let Some(code) = relative.file_stem().and_then(|code| code.to_str()) {
                if edits
                    .iter()
                    .any(|(_, before, after)| same(code, &before.code) || same(code, &after.code))
                {
                    return Err(error(
                        ApplyPlanErrorKind::BadValue,
                        &relative,
                        "embedded help page uses the old or new language code",
                    ));
                }
            }
        }
        if !extension.eq_ignore_ascii_case("xml") && !extension.eq_ignore_ascii_case("bsl") {
            continue;
        }
        let current = staged
            .read_guarded_bounded(&relative, usize::MAX)
            .map_err(|e| ApplyPlanError::staging(e, relative.to_string_lossy()))?;
        let original = originals.get(&relative).and_then(Option::as_deref);
        for bytes in original.into_iter().chain(current.as_deref()) {
            for (_, before, after) in &edits {
                dependencies(bytes, &relative, &before.code, &after.code)
                    .map_err(|message| error(ApplyPlanErrorKind::BadValue, &relative, message))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor(properties: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\"><Language uuid=\"16520000-0000-4000-8000-000000000001\"><Properties><Name>English</Name><Synonym/>{properties}</Properties></Language></MetaDataObject>")
    }
    #[test]
    fn language_code_codec_preserves_identity_neighbors_and_equivalent_bytes() {
        let original = descriptor("<LanguageCode><![CDATA[en]]></LanguageCode>");
        assert_eq!(replace(&original, "English", "en").unwrap(), original);
        let updated = replace(&original, "English", "de").unwrap();
        assert_eq!(read_code(updated.as_bytes()).unwrap(), "de");
        assert_eq!(
            updated,
            original.replace(
                "<LanguageCode><![CDATA[en]]></LanguageCode>",
                "<LanguageCode>de</LanguageCode>"
            )
        );
        assert!(replace(&original, "Other", "de").is_err());
    }
    #[test]
    fn language_code_codec_refuses_ambiguous_and_damaged_properties() {
        for property in [
            "",
            "<LanguageCode/>",
            "<LanguageCode extra=\"x\">en</LanguageCode>",
            "<LanguageCode><Nested/></LanguageCode>",
            "<LanguageCode xmlns=\"foreign\">en</LanguageCode>",
            "<LanguageCode>en</LanguageCode><LanguageCode>en</LanguageCode>",
        ] {
            assert!(
                read_code(descriptor(property).as_bytes()).is_err(),
                "{property}"
            );
        }
        let duplicate = descriptor("<LanguageCode>en</LanguageCode>")
            .replace("</Language>", "<Properties/></Language>");
        assert!(replace(&duplicate, "English", "de").is_err());
    }
    #[test]
    fn language_code_dependencies_observe_labels_instead_of_translation_contents() {
        let xml = format!("<Title xmlns:v8=\"{CORE}\"><v8:item><v8:lang>en</v8:lang><v8:content>text</v8:content></v8:item></Title>");
        assert!(dependencies(xml.as_bytes(), Path::new("Form.xml"), "en", "de").is_err());
        assert!(dependencies(xml.as_bytes(), Path::new("Form.xml"), "fr", "de").is_ok());
        let help = format!("<Help xmlns=\"{EXTRN}\"><Page>en</Page></Help>");
        assert!(dependencies(help.as_bytes(), Path::new("Ext/Help.xml"), "en", "de").is_err());
        for text in [
            "Procedure Run()\nA = NStr(\"ru='text'; EN='translation'\");\nEndProcedure",
            "Procedure Run()\nA = НСтр(\"ru='text'\n|; de='translation'\");\nEndProcedure",
            "Procedure Run()\nA = NStr(Value);\nEndProcedure",
        ] {
            assert!(
                dependencies(text.as_bytes(), Path::new("Module.bsl"), "en", "de").is_err(),
                "{text}"
            );
        }
        let unrelated = "Procedure Run()\nA = NStr(\"ru='en='; fr='it''s text; de=inside'\");\n// NStr(\"en='comment'\");\nEndProcedure";
        assert!(dependencies(unrelated.as_bytes(), Path::new("Module.bsl"), "en", "de").is_ok());
    }
}

#[cfg(test)]
mod namespace_tests {
    #[test]
    fn language_code_replacement_keeps_metadata_namespace_in_prefixed_sources() {
        let md = "http://v8.1c.ru/8.3/MDClasses";
        let text = format!("<m:MetaDataObject xmlns:m=\"{md}\" xmlns=\"urn:other\"><m:Language uuid=\"16520000-0000-4000-8000-000000000001\"><m:Properties><m:Name>English</m:Name><m:LanguageCode>en</m:LanguageCode></m:Properties></m:Language></m:MetaDataObject>");
        let changed = super::replace(&text, "English", "de").unwrap();
        assert_eq!(super::read_code(changed.as_bytes()).unwrap(), "de");
    }
}

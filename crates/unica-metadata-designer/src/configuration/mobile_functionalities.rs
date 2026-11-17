use super::{CF_CAI_NS, CF_MD_NS};
use roxmltree::{Document, Node};
use std::collections::BTreeMap;
use unica_metadata::configuration::{
    mobile_functionalities_in, mobile_functionality_default, MOBILE_FUNCTIONALITIES,
};
use unica_source_contract::format_identity::PlatformLine;
use unica_source_designer::format_profile::platform_line_for_export_format;
use unica_xml_format::names::element_has_name;

const PROPERTY: &str = "UsedMobileApplicationFunctionalities";

type Flags<'a, 'input> = BTreeMap<String, (bool, Node<'a, 'input>)>;

pub(super) fn export_line(version: &str) -> Option<PlatformLine> {
    PlatformLine::parse(platform_line_for_export_format(version)?)
}

pub(super) fn enabled(
    section: Option<Node<'_, '_>>,
    line: Option<PlatformLine>,
) -> Result<Option<Vec<String>>, String> {
    if section.is_none() && line.is_none() {
        return Ok(None);
    }
    let flags = section
        .map(|node| read_flags(node, line))
        .transpose()?
        .unwrap_or_default();
    let names = known_flags(line);
    Ok(Some(
        names
            .iter()
            .copied()
            .filter(|name| {
                flags
                    .get(*name)
                    .map_or_else(|| mobile_functionality_default(name), |(value, _)| *value)
            })
            .map(str::to_string)
            .collect(),
    ))
}

fn known_flags(line: Option<PlatformLine>) -> Vec<&'static str> {
    // For an unclassified export, a present section can still name a known
    // flag. The writer only receives source sets admitted for editing.
    line.map_or_else(
        || MOBILE_FUNCTIONALITIES.to_vec(),
        |line| mobile_functionalities_in(Some(line)),
    )
}

fn elements<'a, 'input>(
    node: Node<'a, 'input>,
) -> Result<impl Iterator<Item = Node<'a, 'input>>, String> {
    if node
        .children()
        .any(|child| child.is_text() && !child.text().unwrap_or_default().trim().is_empty())
    {
        return Err(format!(
            "{} has unexpected scalar text",
            node.tag_name().name()
        ));
    }
    Ok(node.children().filter(|child| child.is_element()))
}

fn read_flags<'a, 'input>(
    section: Node<'a, 'input>,
    line: Option<PlatformLine>,
) -> Result<Flags<'a, 'input>, String> {
    let mut flags = BTreeMap::new();
    let allowed = known_flags(line);
    for item in elements(section)? {
        // Permission descriptions are independent of the enabled capabilities.
        if element_has_name(item, "permissionMessage", Some(CF_CAI_NS)) {
            continue;
        }
        if !element_has_name(item, "functionality", Some(CF_CAI_NS)) {
            return Err(format!(
                "{PROPERTY} has an unexpected list member {}",
                item.tag_name().name()
            ));
        }
        let mut name = None;
        let mut value = None;
        for child in elements(item)? {
            if element_has_name(child, "functionality", Some(CF_CAI_NS)) && name.is_none() {
                name = Some(child);
            } else if element_has_name(child, "use", Some(CF_CAI_NS)) && value.is_none() {
                value = Some(child);
            } else {
                return Err("mobile functionality has an unexpected or duplicate member".into());
            }
        }
        let (Some(name), Some(value)) = (name, value) else {
            return Err("mobile functionality must contain a name and use flag".into());
        };
        let name = super::cf_scalar_text(name)?;
        if !allowed.contains(&name.as_str()) {
            return Err(format!("unknown mobile functionality {name}"));
        }
        let flag = super::cf_boolean(value)?;
        if flags.insert(name.clone(), (flag, value)).is_some() {
            return Err(format!("duplicate mobile functionality {name}"));
        }
    }
    Ok(flags)
}

pub(crate) fn set_enabled(source: &str, wanted: &[String]) -> Result<String, String> {
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let root = document.root_element();
    let configuration = sole_element(root, "Configuration")?
        .ok_or("the configuration root has no Configuration element")?;
    let properties = sole_element(configuration, "Properties")?
        .ok_or("the configuration root has no Properties element")?;
    let line = root.attribute("version").and_then(export_line);
    let names = known_flags(line);
    if let Some(name) = wanted.iter().find(|name| !names.contains(&name.as_str())) {
        return Err(format!(
            "unsupported mobile functionality {name} in this export format"
        ));
    }
    let section = sole_element(properties, PROPERTY)?;
    let flags = section
        .map(|node| read_flags(node, line))
        .transpose()?
        .unwrap_or_default();
    let mut edits = Vec::new();
    let mut insertions = Vec::new();
    for name in names {
        let target = wanted.iter().any(|value| value == name);
        match flags.get(name) {
            Some((current, node)) if *current != target => {
                let mut text_nodes = node.children().filter(|child| child.is_text());
                let first = text_nodes.next().expect("validated use flag contains text");
                edits.push((first.range(), target.to_string()));
                edits.extend(text_nodes.map(|child| (child.range(), String::new())));
            }
            None if target != mobile_functionality_default(name) => insertions.push((name, target)),
            _ => {}
        }
    }
    if !insertions.is_empty() {
        edits.push(insertion_edit(source, properties, section, &insertions)?);
    }
    edits.sort_unstable_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut updated = source.to_string();
    for (range, replacement) in edits {
        updated.replace_range(range, &replacement);
    }
    Ok(updated)
}

fn sole_element<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let mut found = parent
        .children()
        .filter(|node| element_has_name(*node, name, Some(CF_MD_NS)));
    let first = found.next();
    if found.next().is_some() {
        return Err(format!("duplicate Configuration element {name}"));
    }
    Ok(first)
}

fn insertion_edit(
    source: &str,
    properties: Node<'_, '_>,
    section: Option<Node<'_, '_>>,
    flags: &[(&str, bool)],
) -> Result<(std::ops::Range<usize>, String), String> {
    let rows = flags.iter().map(|(name, value)| {
        format!(r#"<app:functionality xmlns:app="{CF_CAI_NS}"><app:functionality>{name}</app:functionality><app:use>{value}</app:use></app:functionality>"#)
    }).collect::<String>();

    if let Some(section) = section {
        let range = section.range();
        let xml = &source[range.clone()];
        if let Some(first_message) = section
            .children()
            .find(|node| element_has_name(*node, "permissionMessage", Some(CF_CAI_NS)))
        {
            let at = first_message.range().start;
            return Ok((at..at, rows));
        }
        if let Some(end) = xml.rfind("</") {
            let at = range.start + end;
            return Ok((at..at, rows));
        }
        let name = qualified_name(source, section);
        return Ok((range.end - 2..range.end, format!(">{rows}</{name}>")));
    }

    let qualified_properties = qualified_name(source, properties);
    let prefix = qualified_properties
        .split_once(':')
        .map_or(String::new(), |(prefix, _)| format!("{prefix}:"));
    let next_property = properties
        .children()
        .find(|node| element_has_name(*node, "DefaultLanguage", Some(CF_MD_NS)));
    let at = sole_element(properties, "RequiredMobileApplicationPermissions")?
        .map(|node| node.range().end)
        .or_else(|| next_property.map(|node| node.range().start))
        .or_else(|| {
            let range = properties.range();
            source[range.clone()]
                .rfind("</")
                .map(|end| range.start + end)
        })
        .ok_or("the configuration Properties element is empty")?;
    Ok((
        at..at,
        format!("<{prefix}{PROPERTY}>{rows}</{prefix}{PROPERTY}>"),
    ))
}

fn qualified_name<'s>(source: &'s str, node: Node<'_, '_>) -> &'s str {
    source[node.range().start + 1..]
        .split(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
        .next()
        .expect("parsed element")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGE: &str = r#"<mobile:permissionMessage><mobile:permission>Camera</mobile:permission><mobile:description><v8:item><v8:lang>ru</v8:lang><v8:content>Камера</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Camera</v8:content></v8:item></mobile:description></mobile:permissionMessage>"#;
    const LEGACY: &str = r#"<md:RequiredMobileApplicationPermissions><mobile:permission/></md:RequiredMobileApplicationPermissions>"#;

    fn descriptor(mobile: &str, version: &str) -> String {
        format!(
            r#"<md:MetaDataObject version="{version}" xmlns:md="{CF_MD_NS}" xmlns:mobile="{CF_CAI_NS}" xmlns:v8="http://v8.1c.ru/8.1/data/core"><md:Configuration><md:Properties><md:Name>Probe</md:Name>{LEGACY}{mobile}<md:DefaultLanguage/></md:Properties></md:Configuration></md:MetaDataObject>"#
        )
    }

    fn read(source: &str) -> Vec<String> {
        let doc = Document::parse(source).unwrap();
        let root = doc.root_element();
        let properties = sole_element(
            sole_element(root, "Configuration").unwrap().unwrap(),
            "Properties",
        )
        .unwrap()
        .unwrap();
        enabled(
            sole_element(properties, PROPERTY).unwrap(),
            root.attribute("version").and_then(export_line),
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn complete_flag_value_can_disable_platform_defaults_without_removing_messages() {
        let source = descriptor(
            &format!(
                r#"<md:{PROPERTY} xmlns:app="urn:foreign"><mobile:functionality><mobile:functionality>Camera</mobile:functionality><mobile:use>true</mobile:use></mobile:functionality>{MESSAGE}</md:{PROPERTY}>"#
            ),
            "2.20",
        );
        assert_eq!(read(&source), ["Biometrics", "Camera", "OSBackup"]);
        let changed = set_enabled(&source, &["Location".into()]).unwrap();
        assert_eq!(read(&changed), ["Location"]);
        assert!(changed.contains(MESSAGE));
        assert!(changed.contains(LEGACY));
        assert_eq!(
            set_enabled(&changed, &["Location".into()]).unwrap(),
            changed
        );
        let cleared = set_enabled(&changed, &[]).unwrap();
        assert!(read(&cleared).is_empty());
        assert!(cleared.contains(MESSAGE));
        assert!(cleared.contains(LEGACY));
    }

    #[test]
    fn absent_and_empty_flags_follow_native_defaults() {
        for content in [
            String::new(),
            format!(r#"<md:{PROPERTY} mode="preserve"/>"#),
        ] {
            let source = descriptor(&content, "2.20");
            let defaults = vec!["Biometrics".into(), "OSBackup".into()];
            assert_eq!(read(&source), defaults);
            assert_eq!(set_enabled(&source, &defaults).unwrap(), source);
            let cleared = set_enabled(&source, &[]).unwrap();
            assert!(read(&cleared).is_empty());
            assert!(cleared.contains(LEGACY));
            if !content.is_empty() {
                assert!(cleared.contains(r#"mode="preserve""#));
            }
        }
    }

    #[test]
    fn accepted_flags_follow_the_export_line() {
        let wanted = ["TextToSpeech".into()];
        assert!(set_enabled(&descriptor("", "2.17"), &wanted).is_err());
        assert_eq!(
            read(&set_enabled(&descriptor("", "2.18"), &wanted).unwrap()),
            wanted
        );
        assert!(set_enabled(&descriptor("", "2.20"), &["UnknownCapability".into()]).is_err());
    }
}

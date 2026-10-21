// XML 1.0 Fifth Edition NameStartChar minus colon, followed by the extra
// NameChar ranges. Both logical-name validation and published argument
// patterns use these exact ranges, including the supplementary planes.
const XML_NCNAME_START_RANGES: &[(char, char)] = &[
    ('A', 'Z'),
    ('_', '_'),
    ('a', 'z'),
    ('\u{00C0}', '\u{00D6}'),
    ('\u{00D8}', '\u{00F6}'),
    ('\u{00F8}', '\u{02FF}'),
    ('\u{0370}', '\u{037D}'),
    ('\u{037F}', '\u{1FFF}'),
    ('\u{200C}', '\u{200D}'),
    ('\u{2070}', '\u{218F}'),
    ('\u{2C00}', '\u{2FEF}'),
    ('\u{3001}', '\u{D7FF}'),
    ('\u{F900}', '\u{FDCF}'),
    ('\u{FDF0}', '\u{FFFD}'),
    ('\u{10000}', '\u{EFFFF}'),
];
const XML_NCNAME_EXTRA_RANGES: &[(char, char)] = &[
    ('-', '-'),
    ('.', '.'),
    ('0', '9'),
    ('\u{00B7}', '\u{00B7}'),
    ('\u{0300}', '\u{036F}'),
    ('\u{203F}', '\u{2040}'),
];

/// XML 1.0 Fifth Edition NCName validation shared by typed metadata addresses
/// and the XML readers that turn QName values back into logical targets.
pub fn xml_ncname_is_valid(value: &str) -> bool {
    let mut characters = value.chars();
    characters.next().is_some_and(xml_ncname_is_start) && characters.all(xml_ncname_is_character)
}

fn xml_ncname_is_start(character: char) -> bool {
    in_xml_name_ranges(character, XML_NCNAME_START_RANGES)
}

fn xml_ncname_is_character(character: char) -> bool {
    xml_ncname_is_start(character) || in_xml_name_ranges(character, XML_NCNAME_EXTRA_RANGES)
}

fn in_xml_name_ranges(character: char, ranges: &[(char, char)]) -> bool {
    ranges
        .iter()
        .any(|(lower, upper)| (*lower..=*upper).contains(&character))
}

fn xml_ncname_pattern_part() -> String {
    let character_class = |extra: bool| {
        let mut pattern = String::from("[");
        let extra_ranges: &[(char, char)] = if extra { XML_NCNAME_EXTRA_RANGES } else { &[] };
        for &(lower, upper) in XML_NCNAME_START_RANGES.iter().chain(extra_ranges.iter()) {
            if lower == '-' {
                pattern.push('\\');
            }
            pattern.push(lower);
            if lower != upper {
                pattern.push('-');
                pattern.push(upper);
            }
        }
        pattern.push(']');
        pattern
    };
    format!("{}{}*", character_class(false), character_class(true))
}

pub fn xml_ncname_pattern() -> String {
    format!("^{}$", xml_ncname_pattern_part())
}

pub fn prefixed_xml_qname_pattern() -> String {
    let name = xml_ncname_pattern_part();
    format!("^{name}:{name}$")
}

pub fn element_has_name(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
    namespace: Option<&str>,
) -> bool {
    node.is_element()
        && node.tag_name().name() == local_name
        && namespace
            .map(|expected| node.tag_name().namespace() == Some(expected))
            .unwrap_or(true)
}

/// Existing strict hyphenated UUID lexical spelling; no compact or braced aliases.
pub fn is_valid_uuid(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    let expected = [8usize, 4, 4, 4, 12];
    parts.len() == expected.len()
        && parts
            .iter()
            .zip(expected)
            .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

pub fn looks_like_uuid_prefix(value: &str) -> bool {
    value.len() >= 9
        && value.chars().take(8).all(|ch| ch.is_ascii_hexdigit())
        && value.as_bytes().get(8) == Some(&b'-')
}

pub fn stable_uuid(index: usize) -> String {
    format!("00000000-0000-0000-0000-{index:012x}")
}

pub fn has_element(doc: &roxmltree::Document<'_>, local_name: &str) -> bool {
    doc.descendants()
        .any(|node| node.is_element() && node.tag_name().name() == local_name)
}

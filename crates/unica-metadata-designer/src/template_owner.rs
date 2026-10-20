//! Existing byte-preserving Template parent-registration and MainDCS images.
use roxmltree::Document;
pub fn append_metadata_child_text(
    xml_text: &str,
    local_name: &str,
    item_name: &str,
) -> Option<String> {
    let doc = Document::parse(xml_text).ok()?;
    let object_node = doc
        .root_element()
        .children()
        .find(|node| node.is_element())?;
    let child_objects_node = object_node
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "ChildObjects")?;
    let range = child_objects_node.range();
    let element_text = &xml_text[range.clone()];
    let prefix = if element_text.trim_start().starts_with("<md:") {
        "md:"
    } else {
        ""
    };
    let item_name = unica_xml_format::text::escape_xml(item_name);

    let empty_tag = format!("<{prefix}ChildObjects/>");
    if element_text.trim() == empty_tag {
        let line_start = xml_text[..range.start].rfind('\n').map_or(0, |pos| pos + 1);
        let indent_candidate = &xml_text[line_start..range.start];
        let indent = if indent_candidate
            .chars()
            .all(|character| character == ' ' || character == '\t')
        {
            indent_candidate
        } else {
            ""
        };
        let replacement = format!(
            "<{prefix}ChildObjects>\n{indent}\t<{prefix}{local_name}>{item_name}</{prefix}{local_name}>\n{indent}</{prefix}ChildObjects>"
        );
        let mut result = String::with_capacity(xml_text.len() + replacement.len());
        result.push_str(&xml_text[..range.start]);
        result.push_str(&replacement);
        result.push_str(&xml_text[range.end..]);
        return Some(result);
    }

    let close = format!("</{prefix}ChildObjects>");
    let close_rel = element_text.rfind(&close)?;
    let index = range.start + close_rel;
    let line_start = xml_text[..index].rfind('\n').map_or(0, |pos| pos + 1);
    let closing_indent_candidate = &xml_text[line_start..index];
    let closing_indent = if closing_indent_candidate
        .chars()
        .all(|character| character == ' ' || character == '\t')
    {
        closing_indent_candidate
    } else {
        ""
    };
    let line =
        format!("\t<{prefix}{local_name}>{item_name}</{prefix}{local_name}>\n{closing_indent}");
    let mut result = String::with_capacity(xml_text.len() + line.len());
    result.push_str(&xml_text[..index]);
    result.push_str(&line);
    result.push_str(&xml_text[index..]);
    Some(result)
}

pub fn update_main_data_composition_schema_text(
    xml_text: &str,
    template_name: &str,
    set_main_dcs: bool,
) -> (String, bool, String) {
    let Some((object_type, object_start)) = ["ExternalReport", "Report"]
        .iter()
        .find_map(|name| find_open_tag(xml_text, name).map(|index| (*name, index)))
    else {
        return (xml_text.to_string(), false, String::new());
    };
    let object_name = first_tag_text_after(xml_text, "Name", object_start);
    let value = format!("{object_type}.{object_name}.Template.{template_name}");
    if let Some((open_start, content_start, close_start, close_end, open_tag, close_tag)) =
        unica_xml_format::text::find_element_bounds(
            xml_text,
            "MainDataCompositionSchema",
            object_start,
        )
    {
        let content = xml_text[content_start..close_start].trim();
        if !content.is_empty() && !set_main_dcs {
            return (xml_text.to_string(), false, String::new());
        }
        let replacement = format!("{open_tag}{value}{close_tag}");
        let mut result = String::with_capacity(xml_text.len() + value.len());
        result.push_str(&xml_text[..open_start]);
        result.push_str(&replacement);
        result.push_str(&xml_text[close_end..]);
        return (result, true, value);
    }

    let Some((open_start, open_end, tag)) =
        find_self_closing_element_bounds(xml_text, "MainDataCompositionSchema", object_start)
    else {
        return (xml_text.to_string(), false, String::new());
    };
    let replacement = format!("<{tag}>{value}</{tag}>");
    let mut result = String::with_capacity(xml_text.len() + replacement.len());
    result.push_str(&xml_text[..open_start]);
    result.push_str(&replacement);
    result.push_str(&xml_text[open_end..]);
    (result, true, value)
}

fn find_self_closing_element_bounds(
    xml_text: &str,
    local_name: &str,
    start: usize,
) -> Option<(usize, usize, String)> {
    for tag in [local_name.to_string(), format!("md:{local_name}")] {
        let open_needle = format!("<{tag}");
        let mut search_start = start;
        while let Some(open_rel) = xml_text[search_start..].find(&open_needle) {
            let open_start = search_start + open_rel;
            let name_end = open_start + open_needle.len();
            let Some(boundary) = xml_text[name_end..].chars().next() else {
                break;
            };
            if !boundary.is_ascii_whitespace() && boundary != '/' && boundary != '>' {
                search_start = name_end;
                continue;
            }
            let open_end = open_start + xml_text[open_start..].find('>')? + 1;
            if xml_text[open_start..open_end]
                .trim_end_matches('>')
                .trim_end()
                .ends_with('/')
            {
                return Some((open_start, open_end, tag));
            }
            search_start = open_end;
        }
    }
    None
}

pub fn find_open_tag(xml_text: &str, local_name: &str) -> Option<usize> {
    [format!("<{local_name}"), format!("<md:{local_name}")]
        .iter()
        .filter_map(|needle| xml_text.find(needle))
        .min()
}

pub fn first_tag_text_after(xml_text: &str, local_name: &str, start: usize) -> String {
    let Some((_, content_start, close_start, _, _, _)) =
        unica_xml_format::text::find_element_bounds(xml_text, local_name, start)
    else {
        return String::new();
    };
    xml_text[content_start..close_start].trim().to_string()
}

pub fn remove_owner_template_child_text(
    xml_text: &str,
    template_name: &str,
) -> Option<(String, bool)> {
    let document = Document::parse(xml_text).ok()?;
    let object = document
        .root_element()
        .children()
        .find(roxmltree::Node::is_element)?;
    let child_objects = object
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "ChildObjects")?;
    let Some(template) = child_objects.children().find(|node| {
        node.is_element()
            && node.tag_name().name() == "Template"
            && node.text().is_some_and(|text| text.trim() == template_name)
    }) else {
        return Some((xml_text.to_string(), false));
    };

    if child_objects
        .children()
        .filter(roxmltree::Node::is_element)
        .count()
        == 1
    {
        let range = child_objects.range();
        let qualified_name = xml_text[range.start + 1..]
            .split(|character: char| character.is_whitespace() || matches!(character, '/' | '>'))
            .next()?;
        let replacement = format!("<{qualified_name}/>");
        let mut updated = String::with_capacity(xml_text.len() - range.len() + replacement.len());
        updated.push_str(&xml_text[..range.start]);
        updated.push_str(&replacement);
        updated.push_str(&xml_text[range.end..]);
        return Some((updated, true));
    }

    let range = template.range();
    let line_start = xml_text[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let leading_is_indent = xml_text[line_start..range.start]
        .chars()
        .all(|character| character == ' ' || character == '\t');
    let remove_range = if leading_is_indent && xml_text[range.end..].starts_with('\n') {
        line_start..range.end + 1
    } else {
        range
    };
    let mut updated = String::with_capacity(xml_text.len() - remove_range.len());
    updated.push_str(&xml_text[..remove_range.start]);
    updated.push_str(&xml_text[remove_range.end..]);
    Some((updated, true))
}

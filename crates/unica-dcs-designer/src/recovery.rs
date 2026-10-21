//! Existing source-backed DCS recovery observations.
/// Путь набора с именем `name` в теле схемы: `["A", "B"]` для члена
/// объединения `A`; несколько наборов с этим именем — все их пути.
pub fn data_set_path_in_schema(
    bytes: &[u8],
    name: &str,
) -> unica_operations::recovery::NamedLookup {
    use crate::schema::DCS_SCHEMA_NS as S;
    use unica_operations::recovery::NamedLookup;
    fn walk(
        node: roxmltree::Node<'_, '_>,
        tag: &str,
        name: &str,
        path: &mut Vec<String>,
        found: &mut Vec<Vec<String>>,
    ) {
        for child in node
            .children()
            .filter(|child| child.is_element() && child.has_tag_name((S, tag)))
        {
            let Some(own) = child
                .children()
                .find(|item| item.is_element() && item.has_tag_name((S, "name")))
                .and_then(|item| item.text())
                .map(str::trim)
            else {
                continue;
            };
            path.push(own.to_string());
            if own == name {
                found.push(path.clone());
            }
            walk(child, "item", name, path, found);
            path.pop();
        }
    }
    let Some(document) = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .ok()
        .and_then(|text| roxmltree::Document::parse(text).ok())
    else {
        return NamedLookup::Missing;
    };
    let mut found = Vec::new();
    walk(
        document.root_element(),
        "dataSet",
        name,
        &mut Vec::new(),
        &mut found,
    );
    match found.len() {
        0 => NamedLookup::Missing,
        1 => NamedLookup::Found(found.remove(0)),
        _ => NamedLookup::Ambiguous(found),
    }
}

/// Путь элемента структуры варианта `variant` с именем `name`: имена
/// именованных предков и его собственное, как их адресует чтение
/// (`Setting.V.Item.A.Item.B`). Несколько элементов с этим именем — пути
/// тех, что адресуемы.
pub fn structure_item_path_in_schema(
    bytes: &[u8],
    variant: &str,
    name: &str,
) -> unica_operations::recovery::NamedLookup {
    use unica_operations::recovery::NamedLookup;
    let Some(data) = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .ok()
        .and_then(|text| {
            crate::schema::parse_dcs_info_xml(
                text,
                unica_metadata::support_state::ObjectSupportData {
                    state: unica_metadata::support_state::ObjectSupportState::NotSupported,
                    direct_edit_safe: None,
                },
            )
            .ok()
        })
    else {
        return NamedLookup::Missing;
    };
    let Some(rows) = data
        .variants
        .iter()
        .find(|candidate| candidate.name == variant)
        .and_then(|variant| serde_json::to_value(&variant.structure_items).ok())
    else {
        return NamedLookup::Missing;
    };
    let rows = rows.as_array().map(Vec::as_slice).unwrap_or_default();
    let (mut paths, found) = unica_dcs::structure::dcs_structure_paths(rows, name);
    match (found, paths.len()) {
        (0, _) => NamedLookup::Missing,
        (1, 1) => NamedLookup::Found(paths.remove(0)),
        _ => NamedLookup::Ambiguous(paths),
    }
}

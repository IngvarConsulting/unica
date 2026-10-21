//! Existing stable collection observations shared by operation codecs.
pub fn push_group_item(groups: &mut Vec<(String, Vec<String>)>, group: &str, item: String) {
    if let Some((_, items)) = groups.iter_mut().find(|(name, _)| name == group) {
        items.push(item);
    } else {
        groups.push((group.to_string(), vec![item]));
    }
}

pub fn duplicates_preserve_order(items: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for item in items {
        let count = items.iter().filter(|candidate| *candidate == item).count();
        if count > 1 && !result.iter().any(|existing| existing == item) {
            result.push(item.clone());
        }
    }
    result
}

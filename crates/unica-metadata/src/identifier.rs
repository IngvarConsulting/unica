//! Existing metadata and BSL identifier alphabet.
pub fn is_1c_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_1c_identifier_start(first) {
        return false;
    }
    chars.all(is_1c_identifier_part)
}
pub fn is_1c_identifier_start(ch: char) -> bool {
    ch == '_'
        || ch.is_ascii_alphabetic()
        || ('А'..='Я').contains(&ch)
        || ('а'..='я').contains(&ch)
        || ch == 'Ё'
        || ch == 'ё'
}
pub fn is_1c_identifier_part(ch: char) -> bool {
    is_1c_identifier_start(ch) || ch.is_ascii_digit()
}

pub fn split_camel_case(name: &str) -> String {
    if name.is_empty() {
        return name.to_string();
    }
    let mut result = String::new();
    let mut previous_lower = false;
    for ch in name.chars() {
        if previous_lower && ch.is_uppercase() {
            result.push(' ');
        }
        result.push(ch);
        previous_lower = ch.is_lowercase();
    }
    let mut chars = result.chars();
    let Some(first) = chars.next() else {
        return result;
    };
    format!("{}{}", first, chars.as_str().to_lowercase())
}

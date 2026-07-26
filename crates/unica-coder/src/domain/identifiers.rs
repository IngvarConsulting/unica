//! Exact 1C metadata identifier grammar shared by domain-facing adapters.

/// Returns whether `value` is a supported 1C metadata identifier.
///
/// The grammar deliberately permits only ASCII and Russian Cyrillic letters,
/// digits after the first character, and underscore. Do not replace this with
/// Unicode `is_alphabetic`: accepting an identifier in one adapter path and
/// rejecting it in another would make semantic identities inconsistent.
pub(crate) fn is_1c_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_1c_identifier_start(first) && chars.all(is_1c_identifier_part)
}

pub(crate) fn is_1c_identifier_start(ch: char) -> bool {
    ch == '_'
        || ch.is_ascii_alphabetic()
        || ('А'..='Я').contains(&ch)
        || ('а'..='я').contains(&ch)
        || ch == 'Ё'
        || ch == 'ё'
}

pub(crate) fn is_1c_identifier_part(ch: char) -> bool {
    is_1c_identifier_start(ch) || ch.is_ascii_digit()
}

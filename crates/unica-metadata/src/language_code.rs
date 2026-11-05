//! The supported language-code input, independent of the languages installed on a host.

// Platform 8.3.27 ObjectPropertiesLanguages lists the reserved characters.
// The rest of the exclusions are characters XML 1.0 cannot represent.
pub const LANGUAGE_CODE_PATTERN: &str =
    r#"^[^#*|\\:"<>?/\u0000-\u0008\u000B\u000C\u000E-\u001F\uFFFE\uFFFF]+$"#;

pub fn validate_language_code(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("LanguageCode must not be empty");
    }
    if value.chars().any(|character| {
        matches!(character, '#' | '*' | '|' | '\\' | ':' | '"' | '<' | '>' | '?' | '/')
            || !matches!(character, '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
    }) {
        return Err("LanguageCode contains a reserved platform character or a character XML 1.0 cannot represent");
    }
    Ok(())
}

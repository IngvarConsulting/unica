//! Names of source sets that run commands (pull, push, make) accept.

use unica_xml_format::names::xml_ncname_is_valid;

/// The compatibility adapter's existing bound is in UTF-8 bytes. Keeping it
/// also keeps previously accepted ASCII names and their size limit unchanged.
// resource-limit: run.source-set-name-bytes
// class: format
// unit: bytes
// scope: record
// value: 64
// resource: Длина имени набора исходников в байтах UTF-8 для pull, push и make.
// status: unproven
// origin: 4ca23b37d (#1098)
// problem: Сохраняет прежнюю границу адаптера совместимости и уже принятые ASCII-имена (комментарий в source_set_name.rs).
// on-reach: Имя считается недопустимым; отказ с подсказкой о форме имени sourceSet.
pub const MAX_SOURCE_SET_NAME_BYTES: usize = 64;

/// A Unicode XML NCName or one of the existing ASCII names (letters, digits,
/// `_`, `-` and `.` without a leading `.`), up to
/// [`MAX_SOURCE_SET_NAME_BYTES`] UTF-8 bytes.
pub fn valid_source_set_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SOURCE_SET_NAME_BYTES
        && (xml_ncname_is_valid(value)
            || (value.is_ascii()
                && !value.starts_with('.')
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
                })))
}

#[cfg(test)]
mod tests {
    use super::valid_source_set_name;

    #[test]
    fn run_source_set_names_preserve_ascii_and_bound_unicode_by_utf8_bytes() {
        for name in [
            "main",
            "1foo",
            "-foo",
            "ext-sales",
            "a.b",
            "Доработки",
            "_Доработки",
            "Доработки-2",
            "📦",
        ] {
            assert!(valid_source_set_name(name), "{name:?}");
        }
        assert!(valid_source_set_name(&"x".repeat(64)));
        assert!(!valid_source_set_name(&"x".repeat(65)));
        assert!(valid_source_set_name(&"Д".repeat(32)));
        assert!(!valid_source_set_name(&"Д".repeat(33)));

        for name in [
            "", ".foo", "../foo", "foo/bar", "foo\\bar", "foo:bar", "foo bar", "foo\nbar",
        ] {
            assert!(!valid_source_set_name(name), "{name:?}");
        }
    }
}

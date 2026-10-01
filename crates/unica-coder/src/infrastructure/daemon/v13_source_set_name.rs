use crate::domain::source_target::xml_ncname_is_valid;

/// The compatibility adapter's existing bound is in UTF-8 bytes. Keeping it
/// also keeps previously accepted ASCII names and their size limit unchanged.
const MAX_SOURCE_SET_NAME_BYTES: usize = 64;

pub(super) fn source_set_name_guidance(operation: &str) -> String {
    format!("{operation} sourceSet must be the name of a source set declared in v8project.yaml: up to {MAX_SOURCE_SET_NAME_BYTES} UTF-8 bytes, using a Unicode XML NCName or the existing ASCII letters, digits, `_`, `-` and `.` without a leading `.`")
}

pub(super) fn valid_source_set_name(value: &str) -> bool {
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

//! The original physical path assertions use their actual composed backend.

#[cfg(unix)]
#[test]
fn saved_source_diff_keeps_literal_backslash_and_nested_paths_distinct() {
    unica_workspace::test_support::saved_source_diff_keeps_literal_backslash_and_nested_paths_distinct(super::super::source_diff::layout);
}

#[cfg(target_os = "linux")]
#[test]
fn saved_source_diff_keeps_native_names_and_replacement_character_distinct() {
    unica_workspace::test_support::saved_source_diff_keeps_native_names_and_replacement_character_distinct(super::super::source_diff::layout, super::super::source_workspace::source_artifact_classifier().as_ref());
}

#[cfg(windows)]
#[test]
fn source_set_relative_path_accepts_a_verbatim_root_without_widening_containment() {
    unica_workspace::test_support::source_set_relative_path_accepts_a_verbatim_root_without_widening_containment(super::super::platform_xml_source_targets::source_set_relative_path);
}

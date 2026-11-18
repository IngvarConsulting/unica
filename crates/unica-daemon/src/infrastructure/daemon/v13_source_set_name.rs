pub(super) use unica_address::source_set_name::valid_source_set_name;
use unica_address::source_set_name::MAX_SOURCE_SET_NAME_BYTES;

pub(super) fn source_set_name_guidance(operation: &str) -> String {
    format!("{operation} sourceSet must be the name of a source set declared in v8project.yaml: up to {MAX_SOURCE_SET_NAME_BYTES} UTF-8 bytes, using a Unicode XML NCName or the existing ASCII letters, digits, `_`, `-` and `.` without a leading `.`")
}

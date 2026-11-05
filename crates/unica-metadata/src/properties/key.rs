//! `Key`: its typed property specification.
use super::*;

const KEY_PATTERN: &str = r"^[^\u0000-\u0008\u000B\u000C\u000E-\u001F\uFFFE\uFFFF]*$";

pub(super) const SPEC: MetadataPropertySpec = MetadataPropertySpec { string_max_length: Some(128), string_pattern: Some(KEY_PATTERN), ..property("Key", MetaPropertyKey::Key, MetaPropertyValueKind::String, SCHEDULED_JOB_KINDS) };

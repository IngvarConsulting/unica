//! Private Platform XML 2.20 writer implementation.
//!
//! The crate-level `operations` module owns format-neutral dispatch. Native
//! names, layout rules, parser/serializer code, and publication machinery are
//! confined to this version module.

pub(crate) mod cf;
pub(crate) mod cfe;
pub(crate) mod common;
pub(crate) mod compile_transaction;
pub(crate) mod dcs;
pub(crate) mod external;
pub(crate) mod filesystem;
pub(crate) mod form;
pub(crate) mod form_edit;
pub(crate) mod form_event_registry;
pub(crate) mod help;
pub(crate) mod interface;
pub(crate) mod meta;
pub(crate) mod module_locator;
pub(crate) mod mxl;
pub(crate) mod operation_descriptors;
pub(crate) mod platform_xml_owner;
pub(crate) mod project_source_types;
pub(crate) mod project_sources;
pub(crate) mod registry;
pub(crate) mod role;
pub(crate) mod single_file_publisher;
pub(crate) mod source_root_types;
pub(crate) mod source_roots;
pub(crate) mod subsystem;
pub(crate) mod support;
pub(crate) mod template;
#[cfg(test)]
pub(crate) mod testing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FormatProfile {
    pub(crate) platform_line: &'static str,
    pub(crate) export_format: &'static str,
}

pub(crate) const ACTIVE_FORMAT_PROFILE: FormatProfile = FormatProfile {
    platform_line: "8.3.27",
    export_format: "2.20",
};

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

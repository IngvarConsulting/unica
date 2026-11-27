//! Identity of a localized embedded help page, independent of export I/O.
use crate::{language_code::validate_language_code, metadata_kind_has_help, MetadataKind};
use unica_address::address::{AddressSegment, NodeKind};
use unica_address::source_target::MetadataAddress;

pub fn owner(segments: &[AddressSegment]) -> Option<MetadataAddress> {
    let [owner] = segments else { return None };
    let kind = MetadataKind::parse(owner.kind().as_str()).ok()?;
    if !metadata_kind_has_help(kind) {
        return None;
    }
    MetadataAddress::parse_logical(&format!("{}.{}", kind.as_str(), owner.name()?)).ok()
}

pub fn scope(segments: &[AddressSegment]) -> Option<(MetadataAddress, Option<&str>)> {
    let [object, help] = segments else {
        return None;
    };
    if help.kind() != NodeKind::Help {
        return None;
    }
    Some((owner(std::slice::from_ref(object))?, help.name()))
}

pub fn page_file_name(code: &str) -> Result<String, &'static str> {
    validate_language_code(code)?;
    if code.trim() != code || code.chars().any(char::is_control) || matches!(code, "." | "..") {
        return Err("help language code must be an unambiguous file component");
    }
    // On Windows the first part before any extension can name a device.
    let stem = code
        .split('.')
        .next()
        .unwrap_or(code)
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    {
        return Err("help language code names a reserved device");
    }
    Ok(format!("{code}.html"))
}

//! Ordinary source read projections; sessions are issued by trusted composition.
use std::io::{self, Read};
use std::path::Path;
use unica_source_contract::discovery::{FollowMode, ProjectDiscoveryRead};
use unica_xml_format::text::Utf8TextSnapshot;
pub fn path_exists(reader: &dyn ProjectDiscoveryRead, path: &Path) -> bool {
    reader.inspect_input(path, FollowMode::Follow).is_ok()
}

pub fn path_is_dir(reader: &dyn ProjectDiscoveryRead, path: &Path) -> bool {
    reader
        .inspect_input(path, FollowMode::Follow)
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

pub fn read_bytes(reader: &dyn ProjectDiscoveryRead, path: &Path) -> io::Result<Vec<u8>> {
    let mut file = reader.open_input(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn read_string(reader: &dyn ProjectDiscoveryRead, path: &Path) -> io::Result<String> {
    let mut file = reader.open_input(path)?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

pub fn read_utf8_sig_snapshot(
    reader: &dyn ProjectDiscoveryRead,
    path: &Path,
) -> Result<Utf8TextSnapshot, String> {
    let raw = read_bytes(reader, path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let text = std::str::from_utf8(&raw)
        .map_err(|err| format!("{} is not valid UTF-8: {err}", path.display()))?;
    let text = text.trim_start_matches('\u{feff}').to_string();
    Ok(Utf8TextSnapshot { raw, text })
}

pub fn file_stem_string(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

pub fn relative_display(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn read_utf8_sig(reader: &dyn ProjectDiscoveryRead, path: &Path) -> Result<String, String> {
    let mut text = read_string(reader, path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    while text.starts_with('\u{feff}') {
        text.remove(0);
    }
    Ok(text)
}

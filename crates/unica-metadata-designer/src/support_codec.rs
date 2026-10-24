//! Actual Designer support parsing, read projections, validation and in-memory edit algorithms.
use crate::support::{is_uuid_text, parse_quoted_support_strings};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use unica_metadata::support_edit::*;
use unica_source_contract::discovery::{FollowMode, ProjectDiscoveryRead};
use unica_source_contract::proposal::{
    DirectoryMembershipSnapshot, DirectoryTopologyEntry, DirectoryTopologyEntryKind,
};
use unica_source_designer::read::read_bytes;
pub type SupportVendorPayloadPreimage = (PathBuf, Vec<u8>);
pub type SupportVendorPayloadSnapshot = (
    DirectoryMembershipSnapshot,
    Vec<SupportVendorPayloadPreimage>,
);

pub fn decode_parent_configurations(raw: &[u8]) -> Result<String, String> {
    let data = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    String::from_utf8(data.to_vec())
        .map_err(|err| format!("ParentConfigurations.bin is not UTF-8: {err}"))
}

pub fn plan_capability_data(
    bin_path: &Path,
    text: &str,
    capability: SupportCapability,
) -> Result<(String, SupportEditData), String> {
    let global_target = capability.target_flag();
    // Only the header slot is the global capability. The later vendor/object slots use
    // support-rule semantics: writing the global `off` value (`1`) into the vendor slot makes
    // 8.3.27 omit the required ParentConfigurations/*.cf payload on its next export.
    let locked = SupportObjectRule::Locked.flag();
    let mut updated = replace_global_flag(text, global_target)?;
    updated = replace_vendor_rule_flags(&updated, locked);
    let object_count = replace_all_object_rule_flags(&mut updated, locked);
    Ok((
        updated,
        SupportEditData {
            action: "capability".to_string(),
            applied: true,
            reason: None,
            editing_enabled: Some(capability.enabled()),
            object_uuid: None,
            rule: None,
            records_changed: 0,
            object_rules_reset: object_count,
            mutation: unica_execution_contract::mutation::MutationData::new(true).updated(bin_path),
        },
    ))
}

pub fn plan_object_rule_data(
    bin_path: &Path,
    text: &str,
    object_uuid: &str,
    rule: SupportObjectRule,
) -> Result<(String, SupportEditData), String> {
    let mut updated = text.to_string();
    let changed = replace_object_rule_flags(&mut updated, object_uuid, rule.flag());
    if changed == 0 {
        let message = format!(
            "Объект (uuid {object_uuid}) не на поддержке (своё добавление или не найден в bin) — переключать нечего."
        );
        let data = noop_support_data("objectRule", message);
        return Ok((text.to_string(), data));
    }
    Ok((
        updated,
        SupportEditData {
            action: "objectRule".to_string(),
            applied: true,
            reason: None,
            editing_enabled: None,
            object_uuid: Some(object_uuid.to_string()),
            rule: Some(rule.state_text().to_string()),
            records_changed: changed,
            object_rules_reset: 0,
            mutation: unica_execution_contract::mutation::MutationData::new(true).updated(bin_path),
        },
    ))
}

pub fn parent_configurations_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

pub fn support_vendor_payload_preimages(
    reader: &dyn ProjectDiscoveryRead,
    config_dir: &Path,
) -> Result<SupportVendorPayloadSnapshot, String> {
    let (snapshot, paths) = support_vendor_payload_paths(reader, config_dir)?;
    let reads = paths
        .into_iter()
        .map(|path| {
            let preimage = read_bytes(reader, &path).map_err(|error| {
                format!(
                    "failed to read support vendor payload {}: {error}",
                    path.display()
                )
            })?;
            Ok::<_, String>((path, preimage))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((snapshot, reads))
}

pub fn support_vendor_payload_paths(
    reader: &dyn ProjectDiscoveryRead,
    config_dir: &Path,
) -> Result<(DirectoryMembershipSnapshot, Vec<PathBuf>), String> {
    let directory = config_dir.join("Ext").join("ParentConfigurations");
    let metadata = match reader.inspect_input(&directory, FollowMode::NoFollow) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok((DirectoryMembershipSnapshot::Absent, Vec::new()))
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect support vendor payload directory {}: {error}",
                directory.display()
            ))
        }
    };
    if metadata.is_link_or_reparse() {
        return Err(format!(
            "support vendor payload directory must not be a symbolic link or reparse point: {}",
            directory.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "support vendor payload path is not a directory: {}",
            directory.display()
        ));
    }

    let mut paths = reader
        .read_entries(&directory)
        .map_err(|error| {
            format!(
                "failed to enumerate support vendor payload directory {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to enumerate support vendor payload directory {}: {error}",
                directory.display()
            )
        })?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("cf"))
        })
        .collect::<Vec<_>>();
    paths.sort();

    let snapshot = DirectoryMembershipSnapshot::Present(
        paths
            .iter()
            .map(|path| DirectoryTopologyEntry {
                name: path
                    .file_name()
                    .expect("enumerated support payload must have a file name")
                    .to_os_string(),
                kind: DirectoryTopologyEntryKind::File,
            })
            .collect(),
    );
    for path in &paths {
        let metadata = reader
            .inspect_input(path, FollowMode::NoFollow)
            .map_err(|error| {
                format!(
                    "failed to inspect support vendor payload {}: {error}",
                    path.display()
                )
            })?;
        if metadata.is_link_or_reparse() {
            return Err(format!(
                "support vendor payload must not be a symbolic link or reparse point: {}",
                path.display()
            ));
        }
        if !metadata.is_file() {
            return Err(format!(
                "support vendor payload is not a regular file: {}",
                path.display()
            ));
        }
        reader.open_input(path).map_err(|error| {
            format!(
                "failed to open support vendor payload {} for reading: {error}",
                path.display()
            )
        })?;
    }
    Ok((snapshot, paths))
}

pub fn preflight_required_vendor_payloads<'a>(
    config_dir: &Path,
    parent_configurations: &str,
    vendor_count: usize,
    capability: SupportCapability,
    vendor_payloads: impl IntoIterator<Item = &'a Path>,
) -> Result<(), String> {
    let quoted = parse_quoted_support_strings(parent_configurations);
    let expected_fields = vendor_count.checked_mul(3).ok_or_else(|| {
        "Слишком много записей поставщиков в ParentConfigurations.bin".to_string()
    })?;
    if quoted.len() != expected_fields {
        return Err(format!(
            "Не удалось сопоставить записи поставщиков в ParentConfigurations.bin: заголовок объявляет {vendor_count}, строковых полей {}",
            quoted.len()
        ));
    }
    support_vendor_rule_flags(parent_configurations, vendor_count)?;
    let vendor_payloads = vendor_payloads.into_iter().collect::<Vec<_>>();
    let missing = quoted
        .as_chunks::<3>()
        .0
        .iter()
        .map(|fields| fields[2].as_str())
        .filter(|vendor_name| {
            !vendor_payloads.iter().any(|path| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem == *vendor_name)
                    && path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("cf"))
            })
        })
        .map(|vendor_name| {
            config_dir
                .join("Ext")
                .join("ParentConfigurations")
                .join(format!("{vendor_name}.cf"))
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    Err(format!(
        "Capability={} cannot be applied: missing required vendor payload(s): {}. ParentConfigurations.bin was not changed; export the matching vendor configuration payload with the 1C platform first.",
        capability.argument_value(),
        missing
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn support_vendor_rule_flags(text: &str, expected_count: usize) -> Result<Vec<u8>, String> {
    let mut flags = Vec::with_capacity(expected_count);
    let mut cursor = 0usize;
    while cursor < text.len() {
        if let Some((flag_start, flag_end)) = vendor_flag_span(text, cursor) {
            let flag = text[flag_start..flag_end].parse::<u8>().map_err(|_| {
                "Неизвестный флаг поставщика в ParentConfigurations.bin".to_string()
            })?;
            flags.push(flag);
            cursor = flag_end;
            continue;
        }
        let ch = text[cursor..]
            .chars()
            .next()
            .expect("cursor remains on a valid character boundary");
        cursor += ch.len_utf8();
    }
    if flags.len() != expected_count {
        return Err(format!(
            "Не удалось сопоставить флаги поставщиков в ParentConfigurations.bin: заголовок объявляет {expected_count}, разобрано {}",
            flags.len()
        ));
    }
    Ok(flags)
}

fn replace_global_flag(text: &str, target: u8) -> Result<String, String> {
    let prefix = "{6,";
    let Some(rest) = text.strip_prefix(prefix) else {
        return Err("Неизвестный формат ParentConfigurations.bin".to_string());
    };
    let Some(comma) = rest.find(',') else {
        return Err("Неизвестный формат ParentConfigurations.bin".to_string());
    };
    Ok(format!("{prefix}{target}{}", &rest[comma..]))
}

fn replace_vendor_rule_flags(text: &str, target: u8) -> String {
    let mut result = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < text.len() {
        if let Some((flag_start, flag_end)) = vendor_flag_span(text, i) {
            result.push_str(&text[i..flag_start]);
            result.push(char::from(b'0' + target));
            i = flag_end;
            continue;
        }
        let ch = text[i..].chars().next().expect("valid char boundary");
        result.push(ch);
        i += ch.len_utf8();
    }
    result
}

fn vendor_flag_span(text: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    if start + 36 >= bytes.len() {
        return None;
    }
    let first_uuid = text.get(start..start + 36)?;
    if !is_uuid_text(first_uuid) || bytes.get(start + 36) != Some(&b',') {
        return None;
    }
    let flag_start = start + 37;
    let mut flag_end = flag_start;
    while bytes
        .get(flag_end)
        .is_some_and(|byte| byte.is_ascii_digit())
    {
        flag_end += 1;
    }
    if flag_end == flag_start || bytes.get(flag_end) != Some(&b',') {
        return None;
    }
    let second_uuid_start = flag_end + 1;
    let second_uuid = text.get(second_uuid_start..second_uuid_start + 36)?;
    if !is_uuid_text(second_uuid) {
        return None;
    }
    Some((flag_start, flag_end))
}

/// Записи правил объектов — те же, что читает [`crate::support::object_rule_record`]:
/// с отметкой поставщика `0` и `-1` (#1976). Меняется только цифра правила.
fn rewrite_object_rule_flags(
    text: &mut String,
    target: u8,
    mut selects: impl FnMut(&str) -> bool,
) -> usize {
    let mut bytes = text.as_bytes().to_vec();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some((_, uuid, length)) = crate::support::object_rule_record(&text.as_bytes()[i..]) {
            if selects(uuid) {
                bytes[i] = b'0' + target;
                count += 1;
            }
            i += length;
            continue;
        }
        i += 1;
    }
    if count > 0 {
        *text = String::from_utf8(bytes).expect("single-byte digit replacement preserves UTF-8");
    }
    count
}

fn replace_all_object_rule_flags(text: &mut String, target: u8) -> usize {
    rewrite_object_rule_flags(text, target, |_| true)
}

fn replace_object_rule_flags(text: &mut String, object_uuid: &str, target: u8) -> usize {
    rewrite_object_rule_flags(text, target, |uuid| uuid.eq_ignore_ascii_case(object_uuid))
}

pub fn noop_support_data(action: &str, message: impl Into<String>) -> SupportEditData {
    let message = message.into();
    SupportEditData {
        action: action.to_string(),
        applied: false,
        reason: Some(message),
        editing_enabled: None,
        object_uuid: None,
        rule: None,
        records_changed: 0,
        object_rules_reset: 0,
        mutation: unica_execution_contract::mutation::MutationData::default(),
    }
}

#[cfg(test)]
mod platform_marker_tests {
    use super::*;

    /// Текст маркера без BOM — так его получают писатели.
    fn marker(name: &str) -> String {
        let bytes: &[u8] = match name {
            "off" => {
                include_bytes!("../../../tests/fixtures/platform_8_3_27/support-markers/off.bin")
                    .as_slice()
            }
            "editable" => include_bytes!(
                "../../../tests/fixtures/platform_8_3_27/support-markers/editable.bin"
            )
            .as_slice(),
            "locked" => {
                include_bytes!("../../../tests/fixtures/platform_8_3_27/support-markers/locked.bin")
                    .as_slice()
            }
            other => panic!("no support marker fixture `{other}`"),
        };
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        text.strip_prefix('\u{feff}').unwrap().to_owned()
    }

    /// Включение изменений пишет тот же маркер, что платформа 8.3.27 после
    /// «Включить возможность изменения» с правилом «на замке» (#1976).
    #[test]
    fn enabling_changes_writes_the_platform_marker() {
        let (written, data) = plan_capability_data(
            Path::new("ParentConfigurations.bin"),
            &marker("off"),
            SupportCapability::On,
        )
        .unwrap();
        assert_eq!(written, marker("locked"));
        assert_eq!(data.object_rules_reset, 5);
    }

    /// Правило объекта с отметкой поставщика «не рекомендуется» (`-1`)
    /// меняется так же, как с `0`; отметка сохраняется.
    #[test]
    fn an_object_rule_is_written_for_both_vendor_marks() {
        let uuid = "c27da06f-c6f6-4edf-b3d9-6c007fba4b0c";
        for mark in ["0", "-1"] {
            let text = marker("locked").replacen(
                &format!("0,0,{uuid},{uuid}"),
                &format!("0,{mark},{uuid},{uuid}"),
                1,
            );
            let (written, data) = plan_object_rule_data(
                Path::new("ParentConfigurations.bin"),
                &text,
                uuid,
                SupportObjectRule::Editable,
            )
            .unwrap();
            assert!(data.applied, "{mark}");
            assert!(
                written.contains(&format!("1,{mark},{uuid},{uuid}")),
                "{mark}"
            );
            assert_eq!(written.len(), text.len(), "{mark}");
        }
        // Включение изменений сбрасывает на замок и записи с `-1`.
        let text = marker("off").replacen(
            &format!("1,0,{uuid},{uuid}"),
            &format!("1,-1,{uuid},{uuid}"),
            1,
        );
        let (written, data) = plan_capability_data(
            Path::new("ParentConfigurations.bin"),
            &text,
            SupportCapability::On,
        )
        .unwrap();
        assert_eq!(data.object_rules_reset, 5);
        assert!(written.contains(&format!("0,-1,{uuid},{uuid}")));
    }
}

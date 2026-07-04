use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::AdapterOutcome;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

use super::common::{
    absolutize, find_support_config_dir, is_uuid_text, path_arg, read_support_state,
    support_object_uuid_for_path, support_root_uuid,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportCapability {
    On,
    Off,
}

impl SupportCapability {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    fn target_flag(self) -> u8 {
        match self {
            Self::On => 0,
            Self::Off => 1,
        }
    }

    fn enabled(self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportObjectRule {
    Locked,
    Editable,
    OffSupport,
}

impl SupportObjectRule {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "locked" => Some(Self::Locked),
            "editable" => Some(Self::Editable),
            "off-support" => Some(Self::OffSupport),
            _ => None,
        }
    }

    fn flag(self) -> u8 {
        match self {
            Self::Locked => 0,
            Self::Editable => 1,
            Self::OffSupport => 2,
        }
    }

    fn state_text(self) -> &'static str {
        match self {
            Self::Locked => "на замке (правка запрещена)",
            Self::Editable => {
                "редактируется с сохранением поддержки (объект продолжит получать обновления вендора — возможны конфликты при обновлении)"
            }
            Self::OffSupport => "снят с поддержки (обновления вендора по этому объекту прекращаются)",
        }
    }
}

enum SupportEditAction {
    Capability(SupportCapability),
    Set(SupportObjectRule),
}

pub(crate) fn invoke_mutation(
    operation: &str,
    _tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<AdapterOutcome> {
    match operation {
        "support-edit" => Some(edit_support(args, context)),
        _ => None,
    }
}

fn edit_support(args: &Map<String, Value>, context: &WorkspaceContext) -> AdapterOutcome {
    match edit_support_result(args, context) {
        Ok(outcome) => outcome,
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "support-edit failed".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error],
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
    }
}

fn edit_support_result(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<AdapterOutcome, String> {
    let action = support_edit_action(args)?;
    let target_path = support_target_path(args, context)?;
    if !target_path.exists() {
        return Err(format!("Путь не найден: {}", target_path.display()));
    }
    let resolved_path = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.clone());
    let Some(config_dir) = find_support_config_dir(&resolved_path) else {
        return Err(format!(
            "Не найден корень конфигурации (Configuration.xml) над путём: {}",
            resolved_path.display()
        ));
    };
    let bin_path = config_dir.join("Ext").join("ParentConfigurations.bin");
    if !bin_path.exists() {
        return Ok(noop_outcome(
            "Конфигурация не на поддержке (Ext/ParentConfigurations.bin отсутствует) — переключать нечего.",
        ));
    }
    let raw = fs::read(&bin_path)
        .map_err(|err| format!("failed to read {}: {err}", bin_path.display()))?;
    if raw.len() <= 32 {
        return Ok(noop_outcome(
            "Поддержка снята полностью (пустой ParentConfigurations.bin) — переключать нечего.",
        ));
    }
    let text = decode_parent_configurations(&raw)?;
    let Some(state) = read_support_state(&bin_path) else {
        return Err("Неизвестный формат ParentConfigurations.bin".to_string());
    };
    if state.removed() {
        return Ok(noop_outcome(
            "Поддержка снята полностью (пустой ParentConfigurations.bin) — переключать нечего.",
        ));
    }

    match action {
        SupportEditAction::Capability(capability) => {
            if state.global_editing_enabled() == capability.enabled() {
                let word = if capability.enabled() {
                    "включена"
                } else {
                    "выключена"
                };
                return Ok(noop_outcome(format!(
                    "Возможность изменения конфигурации уже {word} — изменений нет."
                )));
            }
            apply_capability(&bin_path, &text, capability, &resolved_path)
        }
        SupportEditAction::Set(rule) => {
            if !state.global_editing_enabled() {
                return Err(format!(
                    "Возможность изменения конфигурации выключена — пообъектное переключение недоступно.\n  Сначала: support-edit -Path {} -Capability on или unica.support.edit Path={} Capability=on",
                    resolved_path.display(),
                    resolved_path.display()
                ));
            }
            let object_uuid = support_object_uuid_for_path(&resolved_path)
                .or_else(|| support_root_uuid(&config_dir.join("Configuration.xml")))
                .ok_or_else(|| {
                    format!(
                        "Не удалось определить объект по пути: {}",
                        resolved_path.display()
                    )
                })?;
            apply_object_rule(&bin_path, &text, &object_uuid, rule, &resolved_path)
        }
    }
}

fn support_target_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    path_arg(args, &["Path", "path", "TargetPath", "targetPath"])
        .map(|path| absolutize(path, &context.cwd))
        .ok_or_else(|| "missing required argument: Path".to_string())
}

fn support_edit_action(args: &Map<String, Value>) -> Result<SupportEditAction, String> {
    let capability = string_arg(args, &["Capability", "capability"]);
    let set = string_arg(args, &["Set", "set"]);
    match (capability, set) {
        (Some(_), Some(_)) | (None, None) => Err(
            "Укажите ровно одно: Capability=on|off ЛИБО Set=editable|off-support|locked"
                .to_string(),
        ),
        (Some(value), None) => SupportCapability::parse(&value)
            .map(SupportEditAction::Capability)
            .ok_or_else(|| "Capability must be one of: on, off".to_string()),
        (None, Some(value)) => SupportObjectRule::parse(&value)
            .map(SupportEditAction::Set)
            .ok_or_else(|| "Set must be one of: editable, off-support, locked".to_string()),
    }
}

fn string_arg(args: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn decode_parent_configurations(raw: &[u8]) -> Result<String, String> {
    let data = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    String::from_utf8(data.to_vec())
        .map_err(|err| format!("ParentConfigurations.bin is not UTF-8: {err}"))
}

fn apply_capability(
    bin_path: &Path,
    text: &str,
    capability: SupportCapability,
    target_path: &Path,
) -> Result<AdapterOutcome, String> {
    let target = capability.target_flag();
    let mut updated = replace_global_flag(text, target)?;
    updated = replace_vendor_rule_flags(&updated, target);
    let object_count = replace_all_object_rule_flags(&mut updated, target);
    write_parent_configurations(bin_path, &updated)?;

    let summary = if capability.enabled() {
        "Возможность изменения конфигурации ВКЛЮЧЕНА"
    } else {
        "Возможность изменения конфигурации ВЫКЛЮЧЕНА"
    };
    let stdout = if capability.enabled() {
        format!(
            "{summary}. Все объекты поставщика — на замке.\nВключайте редактирование точечно: support-edit -Path <объект> -Set editable\n"
        )
    } else {
        format!("{summary}. Вся конфигурация стала read-only; пообъектные правила сброшены.\n")
    };

    Ok(AdapterOutcome {
        ok: true,
        summary: summary.to_string(),
        changes: vec![
            format!("updated {}", bin_path.display()),
            format!("set global editing flag to {target}"),
            format!("reset object support rules: {object_count}"),
        ],
        warnings: if capability.enabled() {
            vec![
                "Все объекты поставщика оставлены на замке; включайте editable/off-support точечно."
                    .to_string(),
            ]
        } else {
            Vec::new()
        },
        errors: Vec::new(),
        artifacts: vec![
            bin_path.display().to_string(),
            target_path.display().to_string(),
        ],
        stdout: Some(stdout),
        stderr: None,
        command: None,
    })
}

fn apply_object_rule(
    bin_path: &Path,
    text: &str,
    object_uuid: &str,
    rule: SupportObjectRule,
    target_path: &Path,
) -> Result<AdapterOutcome, String> {
    let mut updated = text.to_string();
    let changed = replace_object_rule_flags(&mut updated, object_uuid, rule.flag());
    if changed == 0 {
        let message = format!(
            "Объект (uuid {object_uuid}) не на поддержке (своё добавление или не найден в bin) — переключать нечего."
        );
        return Ok(noop_outcome(message));
    }
    write_parent_configurations(bin_path, &updated)?;
    let summary = format!("Объект uuid {object_uuid} → {}.", rule.state_text());
    Ok(AdapterOutcome {
        ok: true,
        summary: summary.clone(),
        changes: vec![
            format!("updated {}", bin_path.display()),
            format!("set object {object_uuid} support rule to {}", rule.flag()),
            format!("updated support records: {changed}"),
        ],
        warnings: if matches!(rule, SupportObjectRule::Editable) {
            vec![
                "Объект продолжит получать обновления вендора; при обновлении возможны конфликты."
                    .to_string(),
            ]
        } else {
            Vec::new()
        },
        errors: Vec::new(),
        artifacts: vec![
            bin_path.display().to_string(),
            target_path.display().to_string(),
        ],
        stdout: Some(format!(
            "{summary}\nЗаписей в bin изменено: {changed}. Цель: {}\n",
            target_path.display()
        )),
        stderr: None,
        command: None,
    })
}

fn noop_outcome(message: impl Into<String>) -> AdapterOutcome {
    let message = message.into();
    AdapterOutcome {
        ok: true,
        summary: message.clone(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: Vec::new(),
        stdout: Some(format!("{message}\n")),
        stderr: None,
        command: None,
    }
}

fn write_parent_configurations(path: &Path, text: &str) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(text.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(text.as_bytes());
    fs::write(path, bytes).map_err(|err| format!("failed to write {}: {err}", path.display()))
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

fn replace_all_object_rule_flags(text: &mut String, target: u8) -> usize {
    let mut bytes = text.as_bytes().to_vec();
    let mut count = 0usize;
    let mut i = 0usize;
    while i + 40 <= bytes.len() {
        if matches!(bytes[i], b'0'..=b'2')
            && bytes.get(i + 1..i + 4) == Some(b",0,")
            && text.get(i + 4..i + 40).is_some_and(is_uuid_text)
        {
            bytes[i] = b'0' + target;
            count += 1;
            i += 40;
            continue;
        }
        i += 1;
    }
    if count > 0 {
        *text = String::from_utf8(bytes).expect("single-byte digit replacement preserves UTF-8");
    }
    count
}

fn replace_object_rule_flags(text: &mut String, object_uuid: &str, target: u8) -> usize {
    let target_uuid = object_uuid.to_ascii_lowercase();
    let mut bytes = text.as_bytes().to_vec();
    let mut count = 0usize;
    let mut i = 0usize;
    while i + 40 <= bytes.len() {
        if matches!(bytes[i], b'0'..=b'2') && bytes.get(i + 1..i + 4) == Some(b",0,") {
            let uuid_start = i + 4;
            let uuid_end = uuid_start + 36;
            if let Some(uuid) = text.get(uuid_start..uuid_end) {
                if is_uuid_text(uuid)
                    && uuid.as_bytes().eq_ignore_ascii_case(target_uuid.as_bytes())
                {
                    bytes[i] = b'0' + target;
                    count += 1;
                    i = uuid_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    if count > 0 {
        *text = String::from_utf8(bytes).expect("single-byte digit replacement preserves UTF-8");
    }
    count
}

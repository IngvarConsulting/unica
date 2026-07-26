use crate::application::AdapterOutcome;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use serde_json::{Map, Value};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::common::{
    absolutize, find_support_config_dir, guard_active_format_dependencies,
    guard_exact_preimage_if_unprotected, is_uuid_text, parse_support_header, path_arg,
    support_root_uuid_from_bytes, support_uuid_dependency_paths,
};
use super::compile_transaction::{CompileTransaction, DirectoryMembershipSelector};

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
    let config_path = config_dir.join("Configuration.xml");
    let config_preimage = fs::read(&config_path).map_err(|err| {
        format!(
            "failed to read configuration owner {}: {err}",
            config_path.display()
        )
    })?;
    let uuid_dependency_reads = if matches!(&action, SupportEditAction::Set(_)) {
        support_uuid_dependency_paths(&resolved_path)
            .into_iter()
            .map(|path| {
                let preimage = if path == config_path {
                    config_preimage.clone()
                } else {
                    fs::read(&path).map_err(|err| {
                        format!(
                            "failed to read support UUID dependency {}: {err}",
                            path.display()
                        )
                    })?
                };
                let uuid = support_root_uuid_from_bytes(&preimage);
                Ok::<_, String>((path, preimage, uuid))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
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
    let Some((global_flag, vendor_count)) = parse_support_header(&text) else {
        return Err("Неизвестный формат ParentConfigurations.bin".to_string());
    };
    if vendor_count == 0 {
        return Ok(noop_outcome(
            "Поддержка снята полностью (пустой ParentConfigurations.bin) — переключать нечего.",
        ));
    }

    let (mut outcome, updated) = match action {
        SupportEditAction::Capability(capability) => {
            if (global_flag == 0) == capability.enabled() {
                let word = if capability.enabled() {
                    "включена"
                } else {
                    "выключена"
                };
                return Ok(noop_outcome(format!(
                    "Возможность изменения конфигурации уже {word} — изменений нет."
                )));
            }
            plan_capability(&bin_path, &text, capability, &resolved_path)
        }
        SupportEditAction::Set(rule) => {
            if global_flag != 0 {
                return Err(format!(
                    "Возможность изменения конфигурации выключена — пообъектное переключение недоступно.\n  Сначала: support-edit -Path {} -Capability on или unica.support.edit Path={} Capability=on",
                    resolved_path.display(),
                    resolved_path.display()
                ));
            }
            let object_uuid = if let Some(uuid) = uuid_dependency_reads
                .iter()
                .find_map(|(_, _, uuid)| uuid.clone())
            {
                uuid
            } else if uuid_dependency_reads
                .iter()
                .any(|(path, _, _)| path.extension().is_some_and(|ext| ext == "xml"))
            {
                return Err(format!(
                    "support UUID dependency does not contain a metadata UUID for path: {}",
                    resolved_path.display()
                ));
            } else {
                support_root_uuid_from_bytes(&config_preimage).ok_or_else(|| {
                    format!(
                        "Не удалось определить объект по пути: {}",
                        resolved_path.display()
                    )
                })?
            };
            plan_object_rule(&bin_path, &text, &object_uuid, rule, &resolved_path)
        }
    }?;
    if outcome.changes.is_empty() {
        return Ok(outcome);
    }

    let vendor_payload_reads = support_vendor_payload_preimages(&config_dir)?;
    let updated_bytes = parent_configurations_bytes(&updated);
    let mut transaction = CompileTransaction::new();
    transaction.replace_bytes(&bin_path, &raw, updated_bytes.clone())?;
    guard_exact_preimage_if_unprotected(&mut transaction, &config_path, &config_preimage)?;
    for (path, preimage, _) in &uuid_dependency_reads {
        guard_exact_preimage_if_unprotected(&mut transaction, path, preimage)?;
    }
    for (path, preimage) in &vendor_payload_reads {
        guard_exact_preimage_if_unprotected(&mut transaction, path, preimage)?;
    }
    let vendor_payload_directory = config_dir.join("Ext").join("ParentConfigurations");
    let vendor_payload_names = vendor_payload_reads
        .iter()
        .map(|(path, _)| {
            path.file_name()
                .expect("vendor payload path must have a file name")
                .to_os_string()
        })
        .collect();
    transaction.guard_or_verify_directory_membership(
        &vendor_payload_directory,
        DirectoryMembershipSelector::CfFilesAsciiCaseInsensitive,
        vendor_payload_names,
    )?;
    let mut format_dependencies = vec![config_path.as_path()];
    for (path, _, _) in &uuid_dependency_reads {
        format_dependencies.push(path.as_path());
    }
    guard_active_format_dependencies(&mut transaction, &format_dependencies, context)?;
    let report = transaction.commit_with_post_validation(|| {
        let actual = fs::read(&bin_path)
            .map_err(|err| format!("failed to verify {}: {err}", bin_path.display()))?;
        if actual != updated_bytes {
            return Err(format!(
                "support-edit post-write validation failed for {}",
                bin_path.display()
            ));
        }
        let actual_text = decode_parent_configurations(&actual)?;
        parse_support_header(&actual_text).ok_or_else(|| {
            format!(
                "support-edit post-write validation could not parse {}",
                bin_path.display()
            )
        })?;
        Ok(())
    })?;
    outcome.warnings.extend(report.cleanup_warnings);
    Ok(outcome)
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

pub(crate) fn support_edit_reads_uuid_dependency(args: &Map<String, Value>) -> bool {
    matches!(support_edit_action(args), Ok(SupportEditAction::Set(_)))
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

fn plan_capability(
    bin_path: &Path,
    text: &str,
    capability: SupportCapability,
    target_path: &Path,
) -> Result<(AdapterOutcome, String), String> {
    let global_target = capability.target_flag();
    // Only the header slot is the global capability. The later vendor/object slots use
    // support-rule semantics: writing the global `off` value (`1`) into the vendor slot makes
    // 8.3.27 omit the required ParentConfigurations/*.cf payload on its next export.
    let locked = SupportObjectRule::Locked.flag();
    let mut updated = replace_global_flag(text, global_target)?;
    updated = replace_vendor_rule_flags(&updated, locked);
    let object_count = replace_all_object_rule_flags(&mut updated, locked);
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

    Ok((
        AdapterOutcome {
            ok: true,
            summary: summary.to_string(),
            changes: vec![
                format!("updated {}", bin_path.display()),
                format!("set global editing flag to {global_target}"),
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
        },
        updated,
    ))
}

fn plan_object_rule(
    bin_path: &Path,
    text: &str,
    object_uuid: &str,
    rule: SupportObjectRule,
    target_path: &Path,
) -> Result<(AdapterOutcome, String), String> {
    let mut updated = text.to_string();
    let changed = replace_object_rule_flags(&mut updated, object_uuid, rule.flag());
    if changed == 0 {
        let message = format!(
            "Объект (uuid {object_uuid}) не на поддержке (своё добавление или не найден в bin) — переключать нечего."
        );
        return Ok((noop_outcome(message), text.to_string()));
    }
    let summary = format!("Объект uuid {object_uuid} → {}.", rule.state_text());
    Ok((
        AdapterOutcome {
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
        },
        updated,
    ))
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

fn parent_configurations_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

fn support_vendor_payload_preimages(config_dir: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let directory = config_dir.join("Ext").join("ParentConfigurations");
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "failed to inspect support vendor payload directory {}: {error}",
                directory.display()
            ))
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) {
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

    let mut paths = fs::read_dir(&directory)
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

    paths
        .into_iter()
        .map(|path| {
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                format!(
                    "failed to inspect support vendor payload {}: {error}",
                    path.display()
                )
            })?;
            if metadata_is_link_or_reparse_point(&metadata) {
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
            let preimage = fs::read(&path).map_err(|error| {
                format!(
                    "failed to read support vendor payload {}: {error}",
                    path.display()
                )
            })?;
            Ok((path, preimage))
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::super::single_file_publisher::with_before_commit_hook;
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct SupportFixture {
        root: PathBuf,
        context: WorkspaceContext,
        config_path: PathBuf,
        bin_path: PathBuf,
    }

    impl SupportFixture {
        fn new(label: &str, version: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "unica-support-native-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let source = root.join("src");
            fs::create_dir_all(source.join("Ext")).unwrap();
            fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            let config_path = source.join("Configuration.xml");
            fs::write(&config_path, configuration_xml(version)).unwrap();
            let bin_path = source.join("Ext/ParentConfigurations.bin");
            fs::write(&bin_path, parent_configurations()).unwrap();
            let context = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 0,
            };
            Self {
                root,
                context,
                config_path,
                bin_path,
            }
        }

        fn capability_off_args(&self) -> Map<String, Value> {
            json!({
                "Path": "src",
                "Capability": "off"
            })
            .as_object()
            .unwrap()
            .clone()
        }

        fn object_target(&self, name: &str, version: &str, uuid: &str) -> PathBuf {
            let object_dir = self.root.join("src/Catalogs").join(name);
            let target = object_dir.join("Ext/ObjectModule.bsl");
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(&target, "Процедура Тест() Экспорт\nКонецПроцедуры\n").unwrap();
            fs::write(
                object_dir.with_extension("xml"),
                metadata_object_xml(name, version, uuid),
            )
            .unwrap();
            target
        }

        fn set_editable_args(&self, target: &Path) -> Map<String, Value> {
            json!({
                "Path": target.display().to_string(),
                "Set": "editable"
            })
            .as_object()
            .unwrap()
            .clone()
        }
    }

    impl Drop for SupportFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn configuration_xml(version: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}">
  <Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa">
    <Properties><Name>Demo</Name></Properties>
    <ChildObjects/>
  </Configuration>
</MetaDataObject>
"#
        )
    }

    fn metadata_object_xml(name: &str, version: &str, uuid: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}">
  <Catalog uuid="{uuid}">
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects/>
  </Catalog>
</MetaDataObject>
"#
        )
    }

    fn metadata_object_xml_without_uuid(name: &str, version: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}">
  <Catalog>
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects/>
  </Catalog>
</MetaDataObject>
"#
        )
    }

    fn parent_configurations() -> String {
        "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}"
            .to_string()
    }

    #[test]
    fn support_edit_replaces_parent_configurations_transactionally() {
        let fixture = SupportFixture::new("transaction", "2.20");

        let outcome =
            edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap();

        assert!(outcome.ok, "{outcome:?}");
        assert!(fs::read_to_string(&fixture.bin_path)
            .unwrap()
            .contains("{6,1,"));
    }

    #[test]
    fn support_capability_off_only_changes_global_flag_and_locks_object_rules() {
        let fixture = SupportFixture::new("capability-off-semantics", "2.20");
        let vendor_dir = fixture.root.join("src/Ext/ParentConfigurations");
        fs::create_dir(&vendor_dir).unwrap();
        let vendor_payload = vendor_dir.join("VendorConf.cf");
        let vendor_bytes = b"platform vendor payload".to_vec();
        fs::write(&vendor_payload, &vendor_bytes).unwrap();

        let outcome =
            edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap();

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&fixture.bin_path).unwrap();
        assert!(
            updated.contains(
                "dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
            ),
            "{updated}"
        );
        assert!(
            updated.contains(",0,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
            "{updated}"
        );
        assert!(
            updated.contains(",0,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            "{updated}"
        );
        assert_eq!(fs::read(vendor_payload).unwrap(), vendor_bytes);
    }

    #[test]
    fn support_edit_rejects_a_concurrent_parent_configurations_change() {
        let fixture = SupportFixture::new("bin-race", "2.20");
        let mut concurrent = fs::read(&fixture.bin_path).unwrap();
        concurrent.extend_from_slice(b" ");
        let bin_for_hook = fixture.bin_path.clone();
        let concurrent_for_hook = concurrent.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&bin_for_hook, &concurrent_for_hook).unwrap(),
            || edit_support_result(&fixture.capability_off_args(), &fixture.context),
        )
        .unwrap_err();

        assert!(
            outcome.contains("changed") || outcome.contains("preimage"),
            "{outcome}"
        );
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), concurrent);
    }

    #[test]
    fn support_edit_rejects_a_concurrent_vendor_payload_change() {
        let fixture = SupportFixture::new("vendor-payload-race", "2.20");
        let vendor_dir = fixture.root.join("src/Ext/ParentConfigurations");
        fs::create_dir(&vendor_dir).unwrap();
        let vendor_payload = vendor_dir.join("VendorConf.cf");
        let original_vendor = b"original vendor payload".to_vec();
        let concurrent_vendor = b"concurrent vendor payload".to_vec();
        fs::write(&vendor_payload, &original_vendor).unwrap();
        let payload_for_hook = vendor_payload.clone();
        let concurrent_for_hook = concurrent_vendor.clone();
        let bin_before = fs::read(&fixture.bin_path).unwrap();

        let error = with_before_commit_hook(
            move |_| fs::write(&payload_for_hook, &concurrent_for_hook).unwrap(),
            || edit_support_result(&fixture.capability_off_args(), &fixture.context),
        )
        .unwrap_err();

        assert!(error.contains("read guard"), "{error}");
        assert_eq!(fs::read(&vendor_payload).unwrap(), concurrent_vendor);
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
    }

    #[test]
    fn support_edit_rejects_a_concurrent_vendor_payload_create_case_insensitively() {
        let fixture = SupportFixture::new("vendor-payload-create-race", "2.20");
        let vendor_dir = fixture.root.join("src/Ext/ParentConfigurations");
        fs::create_dir(&vendor_dir).unwrap();
        let existing_payload = vendor_dir.join("VendorConf.cf");
        fs::write(&existing_payload, b"original vendor payload").unwrap();
        let concurrent_payload = vendor_dir.join("ConcurrentVendor.CF");
        let concurrent_for_hook = concurrent_payload.clone();
        let bin_before = fs::read(&fixture.bin_path).unwrap();

        let error = with_before_commit_hook(
            move |_| fs::write(&concurrent_for_hook, b"concurrent vendor payload").unwrap(),
            || edit_support_result(&fixture.capability_off_args(), &fixture.context),
        )
        .unwrap_err();

        assert!(error.contains("directory membership guard"), "{error}");
        assert_eq!(
            fs::read(&concurrent_payload).unwrap(),
            b"concurrent vendor payload"
        );
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
    }

    #[test]
    fn support_edit_rejects_a_concurrent_configuration_owner_change() {
        let fixture = SupportFixture::new("owner-race", "2.20");
        let bin_before = fs::read(&fixture.bin_path).unwrap();
        let concurrent =
            configuration_xml("2.20").replace("<ChildObjects/>", "<!-- concurrent -->");
        let config_for_hook = fixture.config_path.clone();
        let concurrent_for_hook = concurrent.clone();

        let outcome = with_before_commit_hook(
            move |_| fs::write(&config_for_hook, &concurrent_for_hook).unwrap(),
            || edit_support_result(&fixture.capability_off_args(), &fixture.context),
        )
        .unwrap_err();

        assert!(outcome.contains("read guard"), "{outcome}");
        assert_eq!(
            fs::read_to_string(&fixture.config_path).unwrap(),
            concurrent
        );
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
    }

    #[test]
    fn support_edit_rejects_a_concurrent_uuid_descriptor_change() {
        let fixture = SupportFixture::new("uuid-descriptor-race", "2.20");
        let target = fixture.object_target("Items", "2.20", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        let descriptor_path = fixture.root.join("src/Catalogs/Items.xml");
        let concurrent =
            metadata_object_xml("Items", "2.20", "cccccccc-cccc-cccc-cccc-cccccccccccc");
        let descriptor_for_hook = descriptor_path.clone();
        let concurrent_for_hook = concurrent.clone();
        let bin_before = fs::read(&fixture.bin_path).unwrap();

        let error = with_before_commit_hook(
            move |_| fs::write(&descriptor_for_hook, &concurrent_for_hook).unwrap(),
            || edit_support_result(&fixture.set_editable_args(&target), &fixture.context),
        )
        .unwrap_err();

        assert!(error.contains("read guard"), "{error}");
        assert_eq!(fs::read_to_string(&descriptor_path).unwrap(), concurrent);
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
    }

    #[test]
    fn support_edit_rejects_a_concurrent_earlier_uuid_probe_change() {
        let fixture = SupportFixture::new("uuid-probe-race", "2.20");
        let form = fixture
            .root
            .join("src/Catalogs/Items/Forms/Item/Ext/Form.xml");
        let wrapper = fixture.root.join("src/Catalogs/Items/Forms/Item.xml");
        fs::create_dir_all(form.parent().unwrap()).unwrap();
        fs::write(
            &form,
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#,
        )
        .unwrap();
        fs::write(
            &wrapper,
            metadata_object_xml("Item", "2.20", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        let concurrent = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><!-- concurrent --></Form>"#;
        let form_for_hook = form.clone();
        let bin_before = fs::read(&fixture.bin_path).unwrap();

        let error = with_before_commit_hook(
            move |_| fs::write(&form_for_hook, concurrent).unwrap(),
            || edit_support_result(&fixture.set_editable_args(&form), &fixture.context),
        )
        .unwrap_err();

        assert!(error.contains("read guard"), "{error}");
        assert_eq!(fs::read_to_string(&form).unwrap(), concurrent);
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
    }

    #[test]
    fn support_edit_does_not_fall_back_past_an_xml_descriptor_without_uuid() {
        let fixture = SupportFixture::new("uuid-missing", "2.20");
        let target = fixture.object_target("Items", "2.20", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        let descriptor_path = fixture.root.join("src/Catalogs/Items.xml");
        fs::write(
            &descriptor_path,
            metadata_object_xml_without_uuid("Items", "2.20"),
        )
        .unwrap();
        let bin_before = fs::read(&fixture.bin_path).unwrap();

        let error =
            edit_support_result(&fixture.set_editable_args(&target), &fixture.context).unwrap_err();

        assert!(
            error.contains("does not contain a metadata UUID"),
            "{error}"
        );
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
    }

    #[test]
    fn support_edit_refuses_a_newer_configuration_owner_without_writing_bin() {
        let fixture = SupportFixture::new("newer-owner", "2.21");
        let bin_before = fs::read(&fixture.bin_path).unwrap();

        let error =
            edit_support_result(&fixture.capability_off_args(), &fixture.context).unwrap_err();

        assert!(error.contains("newer than supported 2.20"), "{error}");
        assert!(error.contains("1C 8.5 support is planned"), "{error}");
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
    }

    #[test]
    fn support_capability_uses_direct_object_xml_only_as_a_config_locator() {
        let fixture = SupportFixture::new("capability-object-locator", "2.20");
        let target = fixture.object_target("Items", "2.21", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        let descriptor = fixture.root.join("src/Catalogs/Items.xml");
        let args = json!({
            "Path": descriptor.display().to_string(),
            "Capability": "off"
        })
        .as_object()
        .unwrap()
        .clone();

        let outcome = edit_support_result(&args, &fixture.context).unwrap();

        assert!(outcome.ok, "{outcome:?}");
        assert!(fs::read_to_string(&fixture.bin_path)
            .unwrap()
            .contains("{6,1,"));
        assert!(target.exists());
    }

    #[test]
    fn support_edit_refuses_a_newer_nearest_uuid_descriptor_without_writing_bin() {
        let fixture = SupportFixture::new("newer-uuid-descriptor", "2.20");
        let target = fixture.object_target("Items", "2.21", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        let bin_before = fs::read(&fixture.bin_path).unwrap();

        let error =
            edit_support_result(&fixture.set_editable_args(&target), &fixture.context).unwrap_err();

        assert!(error.contains("newer than supported 2.20"), "{error}");
        assert!(error.contains("1C 8.5 support is planned"), "{error}");
        assert_eq!(fs::read(&fixture.bin_path).unwrap(), bin_before);
    }

    #[test]
    fn support_edit_ignores_a_newer_unrelated_neighbor_descriptor() {
        let fixture = SupportFixture::new("unrelated-newer-descriptor", "2.20");
        let target = fixture.object_target("Items", "2.20", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
        fixture.object_target("Unrelated", "2.21", "cccccccc-cccc-cccc-cccc-cccccccccccc");

        let outcome =
            edit_support_result(&fixture.set_editable_args(&target), &fixture.context).unwrap();

        assert!(outcome.ok, "{outcome:?}");
        let updated = fs::read_to_string(&fixture.bin_path).unwrap();
        assert!(
            updated.contains("1,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            "{updated}"
        );
    }
}

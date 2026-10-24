//! Политика сетевого выхода поставщиков документации: `unica.toml` в корне
//! проекта с локальным оверлеем `unica.local.toml` и нижним общим слоем.
//!
//! Это файл запрета, и правила разбора продиктованы именно этим. Отсутствие
//! проектных файлов даёт умолчания — сеть разрешена. Явно заданный общий
//! файл обязателен: его отсутствие вызывает отказ. Всё остальное неясное — отказ:
//! неразбираемый файл, неизвестная секция или ключ, неизвестный идентификатор
//! поставщика, неизвестное значение политики. Молчаливый откат к разрешающему
//! поведению недопустим: правило запрета, которое не прочиталось и стало
//! разрешением, — единственный по-настоящему опасный отказ этого файла.
//! Опечатка в идентификаторе внутри правила запрета иначе оставила бы выход
//! открытым, а автор считал бы его закрытым.
//!
//! Исключение — удалённые поставщики: их секция осталась у пользователей от
//! прежних версий. Отказ на ней отключил бы всю документацию, поэтому она
//! принимается с предупреждением и целиком игнорируется — запрета, который
//! стал бы разрешением, в ней уже нет: поставщика самого нет.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) const MACHINE_POLICY_ENV: &str = "UNICA_DOCUMENTATION_POLICY_FILE";

/// Поставщики, удалённые из Unica: `kb-1ci` — #1892.
pub(crate) const RETIRED_PROVIDERS: &[&str] = &["kb-1ci"];

pub(crate) fn network_failure(provider: &str, error: impl std::fmt::Display) -> String {
    format!("{error}. Чтобы отключить сетевой поставщик {provider}, добавьте в unica.local.toml секцию [providers.{provider}] с network = \"deny\".")
}

use unica_configuration::{
    parse_workspace_config_root, WorkspaceConfigRootError, WorkspaceConfigRootErrorKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAccess {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Default)]
struct ProviderPolicy {
    network: Option<NetworkAccess>,
    endpoint: Option<String>,
}

#[derive(Debug, Default)]
pub struct DocumentationPolicy {
    default_network: Option<NetworkAccess>,
    providers: BTreeMap<String, ProviderPolicy>,
    warnings: Vec<String>,
}

impl DocumentationPolicy {
    /// Общий файл из окружения демона, затем проект и его локальный оверлей.
    /// Каждый слой проверяется до наложения: ошибка запрета не становится allow.
    pub fn load(workspace_root: &Path, known: &[&str]) -> Result<DocumentationPolicy, String> {
        let machine_path = std::env::var_os(MACHINE_POLICY_ENV).map(PathBuf::from);
        Self::load_layers(workspace_root, known, machine_path.as_deref())
    }

    fn load_layers(
        workspace_root: &Path,
        known: &[&str],
        machine_path: Option<&Path>,
    ) -> Result<Self, String> {
        if machine_path.is_some_and(|path| !path.is_absolute()) {
            return Err(format!(
                "{MACHINE_POLICY_ENV}: требуется абсолютный путь к файлу политики"
            ));
        }
        let mut policy = DocumentationPolicy::default();
        let paths = machine_path
            .map(|path| (path.to_path_buf(), true))
            .into_iter()
            .chain(
                [("unica.toml", false), ("unica.local.toml", false)]
                    .into_iter()
                    .map(|(name, required)| (workspace_root.join(name), required)),
            );
        for (path, required) in paths {
            let file_name = path.display();
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {
                    match std::fs::symlink_metadata(&path) {
                        Err(metadata_error)
                            if metadata_error.kind() == std::io::ErrorKind::NotFound =>
                        {
                            continue
                        }
                        Ok(_) | Err(_) => return Err(format!("{file_name}: файл не читается")),
                    }
                }
                Err(error) => return Err(format!("{file_name}: файл не читается: {error}")),
            };
            let mut parsed =
                parse_policy(&text, known).map_err(|error| format!("{file_name}: {error}"))?;
            for warning in &mut parsed.warnings {
                *warning = format!("{file_name}: {warning}");
            }
            policy.overlay(parsed);
        }
        Ok(policy)
    }

    /// По-ключевое наложение: значение оверлея перекрывает одноимённое, а
    /// незаданные ключи остаются из нижнего слоя.
    fn overlay(&mut self, upper: DocumentationPolicy) {
        if upper.default_network.is_some() {
            self.default_network = upper.default_network;
        }
        self.warnings.extend(upper.warnings);
        for (id, incoming) in upper.providers {
            let entry = self.providers.entry(id).or_default();
            if incoming.network.is_some() {
                entry.network = incoming.network;
            }
            if incoming.endpoint.is_some() {
                entry.endpoint = incoming.endpoint;
            }
        }
    }

    /// Сетевой выход поставщика: собственное правило, иначе `[network].default`,
    /// иначе встроенное умолчание — разрешено.
    pub fn network(&self, provider: &str) -> NetworkAccess {
        self.providers
            .get(provider)
            .and_then(|policy| policy.network)
            .or(self.default_network)
            .unwrap_or(NetworkAccess::Allow)
    }

    /// Предупреждения разбора по всем слоям: секции удалённых поставщиков.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Endpoint поставщика из файла, если задан.
    pub fn endpoint(&self, provider: &str) -> Option<String> {
        self.providers
            .get(provider)
            .and_then(|policy| policy.endpoint.clone())
    }
}

fn network_access(value: &toml::Value) -> Result<NetworkAccess, String> {
    match value.as_str() {
        Some("allow") => Ok(NetworkAccess::Allow),
        Some("deny") => Ok(NetworkAccess::Deny),
        Some(other) => Err(format!(
            "неизвестное значение политики {other:?}; допустимо allow или deny"
        )),
        None => Err("значение политики обязано быть строкой".to_string()),
    }
}

/// Строгий разбор одного файла. Общий парсер корня отвергает неизвестные
/// соседние секции, а этот потребитель отвергает неизвестные поля сети:
/// derive здесь молча отбросил бы оба класса ошибок.
fn parse_policy(text: &str, known: &[&str]) -> Result<DocumentationPolicy, String> {
    let table = parse_workspace_config_root(text).map_err(root_error_message)?;
    let mut policy = DocumentationPolicy::default();
    for (section, body) in &table {
        match section.as_str() {
            "network" => {
                let network = body
                    .as_table()
                    .ok_or_else(|| "[network] обязана быть таблицей".to_string())?;
                for (key, entry) in network {
                    match key.as_str() {
                        "default" => policy.default_network = Some(network_access(entry)?),
                        other => {
                            return Err(format!("неизвестный ключ [network].{other}"));
                        }
                    }
                }
            }
            "providers" => {
                let providers = body
                    .as_table()
                    .ok_or_else(|| "[providers] обязана быть таблицей".to_string())?;
                for (id, entry) in providers {
                    if RETIRED_PROVIDERS.contains(&id.as_str()) {
                        policy.warnings.push(format!(
                            "[providers.{id}]: поставщик {id} удалён, ключ можно убрать"
                        ));
                        continue;
                    }
                    if !known.contains(&id.as_str()) {
                        return Err(format!(
                            "неизвестный поставщик {id:?}; известны: {}",
                            known.join(", ")
                        ));
                    }
                    let body = entry
                        .as_table()
                        .ok_or_else(|| format!("[providers.{id}] обязана быть таблицей"))?;
                    let mut provider = ProviderPolicy::default();
                    for (key, value) in body {
                        match key.as_str() {
                            "network" => provider.network = Some(network_access(value)?),
                            "endpoint" => {
                                provider.endpoint = Some(
                                    value
                                        .as_str()
                                        .ok_or_else(|| {
                                            format!("[providers.{id}].endpoint обязан быть строкой")
                                        })?
                                        .to_string(),
                                )
                            }
                            other => {
                                return Err(format!("неизвестный ключ [providers.{id}].{other}"));
                            }
                        }
                    }
                    policy.providers.insert(id.clone(), provider);
                }
            }
            // Секции общего корня, которыми владеют другие потребители.
            _ => {}
        }
    }
    Ok(policy)
}

fn root_error_message(error: WorkspaceConfigRootError) -> String {
    match error.kind() {
        WorkspaceConfigRootErrorKind::InvalidToml => "файл не разбирается".to_string(),
        WorkspaceConfigRootErrorKind::UnknownField => {
            format!("неизвестная секция [{}]", error.field_path())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &[&str] = &["v8std", "configuration-help"];

    fn workspace() -> tempfile::TempDir {
        tempfile::tempdir().expect("временный каталог")
    }

    #[test]
    fn machine_policy_applies_to_multiple_workspaces_and_project_overlays_win_per_key() {
        let common = workspace();
        let path = common.path().join("policy.toml");
        std::fs::write(&path, "[providers.configuration-help]\nnetwork = \"deny\"\n[providers.v8std]\nnetwork = \"allow\"\nendpoint = \"http://common/mcp\"\n").unwrap();
        let first = workspace();
        let second = workspace();
        for dir in [&first, &second] {
            let policy = DocumentationPolicy::load_layers(dir.path(), KNOWN, Some(&path)).unwrap();
            assert_eq!(policy.network("configuration-help"), NetworkAccess::Deny);
            assert_eq!(policy.network("v8std"), NetworkAccess::Allow);
            assert_eq!(
                policy.endpoint("v8std").as_deref(),
                Some("http://common/mcp")
            );
        }
        std::fs::write(
            first.path().join("unica.toml"),
            "[network]\ndefault = \"deny\"\n[providers.configuration-help]\nnetwork = \"allow\"\n",
        )
        .unwrap();
        std::fs::write(
            first.path().join("unica.local.toml"),
            "[providers.v8std]\nendpoint = \"http://local/mcp\"\n",
        )
        .unwrap();
        let policy = DocumentationPolicy::load_layers(first.path(), KNOWN, Some(&path)).unwrap();
        assert_eq!(policy.network("configuration-help"), NetworkAccess::Allow);
        assert_eq!(
            policy.network("v8std"),
            NetworkAccess::Allow,
            "provider override remains stronger than a higher-layer default"
        );
        assert_eq!(
            policy.endpoint("v8std").as_deref(),
            Some("http://local/mcp")
        );
        assert_eq!(
            DocumentationPolicy::load_layers(second.path(), KNOWN, Some(&path))
                .unwrap()
                .network("configuration-help"),
            NetworkAccess::Deny
        );
    }

    #[test]
    fn explicit_machine_policy_failure_cannot_be_masked_by_project_allow() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[network]\ndefault = \"allow\"\n",
        )
        .unwrap();
        let common = workspace();
        let path = common.path().join("policy.toml");
        assert!(
            DocumentationPolicy::load_layers(dir.path(), KNOWN, Some(&path)).is_err(),
            "explicit missing file must refuse"
        );
        for text in [
            "{invalid",
            "[providers.unknown]\nnetwork = \"deny\"\n",
            "[providers.configuration-help]\nnetwork = \"denny\"\n",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(DocumentationPolicy::load_layers(dir.path(), KNOWN, Some(&path)).is_err());
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(
            DocumentationPolicy::load_layers(dir.path(), KNOWN, Some(&path)).is_err(),
            "directory is not a readable policy file"
        );
    }

    #[test]
    fn machine_policy_requires_an_absolute_nonempty_path() {
        let dir = workspace();
        for path in [Path::new(""), Path::new("policy.toml")] {
            assert!(DocumentationPolicy::load_layers(dir.path(), KNOWN, Some(path)).is_err());
        }
    }

    /// Строка удалённого поставщика из прежней версии не ломает документацию:
    /// в любом слое — с любым телом, даже неверным, — она принимается,
    /// называется предупреждением с именем файла и ничего не настраивает.
    #[test]
    fn a_retired_provider_is_accepted_with_a_warning_and_ignored_in_every_layer() {
        for layer in 0..3 {
            let common = workspace();
            let dir = workspace();
            let paths = [
                common.path().join("policy.toml"),
                dir.path().join("unica.toml"),
                dir.path().join("unica.local.toml"),
            ];
            std::fs::write(&paths[0], "").unwrap();
            std::fs::write(
                &paths[layer],
                "[providers.kb-1ci]\nnetwork = \"denny\"\nendpoint = \"http://ignored.invalid\"\nretry = 3\n",
            )
            .unwrap();
            let policy =
                DocumentationPolicy::load_layers(dir.path(), KNOWN, Some(&paths[0])).unwrap();
            assert_eq!(policy.warnings().len(), 1, "{:?}", policy.warnings());
            let warning = &policy.warnings()[0];
            assert!(
                warning.contains(&paths[layer].display().to_string())
                    && warning.contains("поставщик kb-1ci удалён, ключ можно убрать"),
                "{warning}"
            );
            assert_eq!(policy.network("kb-1ci"), NetworkAccess::Allow);
            assert_eq!(policy.endpoint("kb-1ci"), None);
            assert_eq!(policy.network("v8std"), NetworkAccess::Allow);
        }
        // Неизвестный поставщик по-прежнему отказ: исключение только для удалённых.
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[providers.kb-1c]\nnetwork = \"deny\"\n",
        )
        .unwrap();
        assert!(DocumentationPolicy::load(dir.path(), KNOWN).is_err());
    }

    #[test]
    fn absent_file_gives_permissive_defaults() {
        let dir = workspace();
        let policy = DocumentationPolicy::load(dir.path(), KNOWN).expect("умолчания");
        assert_eq!(policy.network("v8std"), NetworkAccess::Allow);
        assert_eq!(policy.network("configuration-help"), NetworkAccess::Allow);
        assert_eq!(policy.endpoint("v8std"), None);
    }

    #[test]
    fn documentation_policy_ignores_valid_operational_subtree() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[network]\ndefault = \"deny\"\n\n[operational.code_diagnostics]\nanalyze_timeout_seconds = 60\n",
        )
        .expect("write mixed workspace config");

        let policy = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect("operational sibling must not be an unknown network section");

        assert_eq!(policy.network("v8std"), NetworkAccess::Deny);
    }

    /// Characterizes consumer locality: the policy reader owns `network` and
    /// `providers`, while a syntactically present operational sibling is not
    /// part of its subtree contract.
    #[test]
    fn characterization_policy_ignores_invalid_operational_subtree() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[network]\ndefault = \"deny\"\n\n[operational.code_intelligence]\nunknown_timeout = 60\n",
        )
        .expect("write mixed workspace config");

        let policy = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect("operational subtree errors belong to the operational consumer");

        assert_eq!(policy.network("v8std"), NetworkAccess::Deny);
    }

    #[test]
    fn documentation_policy_rejects_unknown_root_fields_after_shared_root_validation() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[network]\ndefault = \"deny\"\n\n[transport]\nretry = 3\n",
        )
        .expect("write invalid workspace config");

        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("unknown root must remain a policy refusal");

        assert!(error.contains("transport"), "unexpected error: {error}");
    }

    #[test]
    fn unparseable_file_is_a_hard_refusal() {
        let dir = workspace();
        std::fs::write(dir.path().join("unica.toml"), "[[[не toml").expect("файл");
        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("неразбираемый файл обязан отказывать");
        assert!(
            error.contains("unica.toml"),
            "отказ обязан назвать файл, получено {error}"
        );
    }

    /// Нечитаемый файл — не то же, что отсутствующий: отказ чтения,
    /// проглоченный на unica.toml, превращал бы правило запрета в молчаливое
    /// разрешение — единственный по-настоящему опасный отказ этого файла.
    /// Нечитаемость моделируется каталогом по имени файла: `read_to_string`
    /// на нём отказывает не-NotFound ошибкой на любой ОС, без платформенных
    /// прав и `cfg`-веток вне фасада (INV-платформенной границы).
    #[test]
    fn an_unreadable_file_is_a_refusal_not_a_silent_default() {
        let dir = workspace();
        std::fs::create_dir(dir.path().join("unica.toml")).expect("каталог по имени файла");
        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("нечитаемый файл обязан отказывать");
        assert!(
            error.contains("unica.toml"),
            "отказ обязан назвать файл, получено {error}"
        );
    }

    #[test]
    fn an_unknown_provider_id_is_a_refusal_not_a_silent_skip() {
        let dir = workspace();
        // Опечатка в идентификаторе внутри правила запрета: тихое
        // игнорирование оставило бы выход открытым.
        std::fs::write(
            dir.path().join("unica.toml"),
            "[providers.v8sdt]\nnetwork = \"deny\"\n",
        )
        .expect("файл");
        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("неизвестный поставщик обязан отказывать");
        assert!(
            error.contains("v8sdt"),
            "отказ обязан назвать поставщика, получено {error}"
        );
    }

    #[test]
    fn an_unknown_network_value_is_a_refusal() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[providers.v8std]\nnetwork = \"maybe\"\n",
        )
        .expect("файл");
        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("неизвестное значение обязано отказывать");
        assert!(
            error.contains("maybe"),
            "отказ обязан назвать значение, получено {error}"
        );
    }

    #[test]
    fn an_unknown_key_is_a_refusal() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[providers.v8std]\nretry = 3\n",
        )
        .expect("файл");
        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("неизвестный ключ обязан отказывать");
        assert!(
            error.contains("retry"),
            "отказ обязан назвать ключ, получено {error}"
        );

        std::fs::write(dir.path().join("unica.toml"), "[transport]\nx = 1\n").expect("файл");
        let error = DocumentationPolicy::load(dir.path(), KNOWN)
            .expect_err("неизвестная секция обязана отказывать");
        assert!(
            error.contains("transport"),
            "отказ обязан назвать секцию, получено {error}"
        );
    }

    #[test]
    fn the_local_overlay_wins_per_key() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[providers.v8std]\nnetwork = \"deny\"\nendpoint = \"http://127.0.0.1:8080/mcp\"\n",
        )
        .expect("основной файл");
        std::fs::write(
            dir.path().join("unica.local.toml"),
            "[providers.v8std]\nnetwork = \"allow\"\n",
        )
        .expect("оверлей");
        let policy = DocumentationPolicy::load(dir.path(), KNOWN).expect("политика");
        assert_eq!(
            policy.network("v8std"),
            NetworkAccess::Allow,
            "локальное значение перекрывает по-ключево"
        );
        assert_eq!(
            policy.endpoint("v8std").as_deref(),
            Some("http://127.0.0.1:8080/mcp"),
            "непере крытый ключ приходит из основного файла"
        );
    }

    #[test]
    fn default_deny_denies_only_providers_without_their_own_allow() {
        let dir = workspace();
        std::fs::write(
            dir.path().join("unica.toml"),
            "[network]\ndefault = \"deny\"\n\n[providers.v8std]\nnetwork = \"allow\"\n",
        )
        .expect("файл");
        let policy = DocumentationPolicy::load(dir.path(), KNOWN).expect("политика");
        assert_eq!(policy.network("v8std"), NetworkAccess::Allow);
        assert_eq!(
            policy.network("configuration-help"),
            NetworkAccess::Deny,
            "умолчание deny действует на поставщика без собственного правила"
        );
    }
}

//! Политика сетевого выхода поставщиков документации: `unica.toml` в корне
//! проекта с машинным оверлеем `unica.local.toml` (ADR-0032, п.6–7).
//!
//! Это файл запрета, и правила разбора продиктованы именно этим. Отсутствие
//! файла даёт умолчания — сеть разрешена. Всё остальное неясное — отказ:
//! неразбираемый файл, неизвестная секция или ключ, неизвестный идентификатор
//! поставщика, неизвестное значение политики. Молчаливый откат к разрешающему
//! поведению недопустим: правило запрета, которое не прочиталось и стало
//! разрешением, — единственный по-настоящему опасный отказ этого файла.
//! Опечатка в идентификаторе внутри правила запрета иначе оставила бы выход
//! открытым, а автор считал бы его закрытым.

use std::collections::BTreeMap;
use std::path::Path;

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
}

impl DocumentationPolicy {
    /// Читает политику проекта: `unica.toml`, затем оверлей
    /// `unica.local.toml` — локальные значения перекрывают по-ключево.
    /// `known` — идентификаторы поставщиков, которым разрешено фигурировать
    /// в файле; любой другой — отказ.
    pub fn load(workspace_root: &Path, known: &[&str]) -> Result<DocumentationPolicy, String> {
        let mut policy = DocumentationPolicy::default();
        for file_name in ["unica.toml", "unica.local.toml"] {
            let path = workspace_root.join(file_name);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let parsed =
                parse_policy(&text, known).map_err(|error| format!("{file_name}: {error}"))?;
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

/// Строгий разбор одного файла. Через `toml::Value`, а не derive: файл
/// запрета обязан отказывать на неизвестных секциях и ключах, а derive
/// молча их отбрасывает.
fn parse_policy(text: &str, known: &[&str]) -> Result<DocumentationPolicy, String> {
    let value = text
        .parse::<toml::Value>()
        .map_err(|error| format!("файл не разбирается: {error}"))?;
    let table = value
        .as_table()
        .ok_or_else(|| "корень файла обязан быть таблицей".to_string())?;
    let mut policy = DocumentationPolicy::default();
    for (section, body) in table {
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
            other => {
                return Err(format!("неизвестная секция [{other}]"));
            }
        }
    }
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &[&str] = &["v8std", "kb-1ci"];

    fn workspace() -> tempfile::TempDir {
        tempfile::tempdir().expect("временный каталог")
    }

    #[test]
    fn absent_file_gives_permissive_defaults() {
        let dir = workspace();
        let policy = DocumentationPolicy::load(dir.path(), KNOWN).expect("умолчания");
        assert_eq!(policy.network("v8std"), NetworkAccess::Allow);
        assert_eq!(policy.network("kb-1ci"), NetworkAccess::Allow);
        assert_eq!(policy.endpoint("v8std"), None);
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
            policy.network("kb-1ci"),
            NetworkAccess::Deny,
            "умолчание deny действует на поставщика без собственного правила"
        );
    }
}

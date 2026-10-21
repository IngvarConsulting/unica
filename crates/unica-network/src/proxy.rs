//! Прокси из переменных окружения.
//!
//! Порядок переменных тот же, что у curl, потому что их пользователь уже
//! настроил для curl, git и пакетных менеджеров. Подсети (CIDR) в `NO_PROXY`,
//! которые curl понимает, здесь не поддерживаются:
//!
//! - адрес `https://` идёт через `https_proxy`, затем `HTTPS_PROXY`, затем
//!   `all_proxy`, `ALL_PROXY`;
//! - адрес `http://` идёт через `http_proxy`, затем `all_proxy`, `ALL_PROXY`.
//!   Заглавную `HTTP_PROXY` curl не читает: в CGI её подставляет заголовок
//!   запроса `Proxy:` (httpoxy), и мы её тоже не читаем;
//! - `no_proxy`, затем `NO_PROXY` перечисляют узлы, к которым идём напрямую.
//!
//! Пустое значение равно отсутствию переменной. Неразборчивое значение или
//! неподдерживаемая схема — отказ с именем переменной, а не молчаливое прямое
//! соединение: молчание и было дефектом #592.

use std::fmt;

/// Где искать переменную. В продукте — окружение процесса; тесты подают свою
/// таблицу, не трогая окружение соседних тестов.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

const HTTPS_VARIABLES: &[&str] = &["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"];
const HTTP_VARIABLES: &[&str] = &["http_proxy", "all_proxy", "ALL_PROXY"];
const NO_PROXY_VARIABLES: &[&str] = &["no_proxy", "NO_PROXY"];

/// Выбранный прокси: откуда взят и куда ведёт.
#[derive(Clone)]
pub(crate) struct ProxyChoice {
    /// Имя переменной, из которой взято значение.
    pub variable: &'static str,
    /// Значение как есть: ключ агента и вход для `ureq::Proxy`.
    raw: String,
    /// Значение без пароля: только оно попадает в тексты и журналы.
    pub shown: String,
    pub proxy: ureq::Proxy,
}

impl ProxyChoice {
    pub fn key(&self) -> &str {
        &self.raw
    }
}

impl fmt::Debug for ProxyChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}={}", self.variable, self.shown)
    }
}

/// Ошибка настройки прокси. Текст уже без пароля.
#[derive(Debug)]
pub(crate) struct ProxySettingError {
    pub message: String,
}

/// Настройка схемы: выбранный прокси, прямой путь или ошибка в переменной.
/// Ошибка всплывает только у запроса этой схемы: неверный `http_proxy`
/// не мешает загрузке по HTTPS.
pub(crate) type SchemeProxy = Result<Option<ProxyChoice>, ProxySettingError>;

#[derive(Debug)]
pub(crate) struct ProxySettings {
    pub https: SchemeProxy,
    pub http: SchemeProxy,
    pub bypass: NoProxy,
}

impl ProxySettings {
    pub fn from_lookup(lookup: EnvLookup<'_>) -> Self {
        Self {
            https: choose(lookup, HTTPS_VARIABLES),
            http: choose(lookup, HTTP_VARIABLES),
            bypass: NoProxy::parse(first_value(lookup, NO_PROXY_VARIABLES).map(|(_, v)| v)),
        }
    }

    /// Прокси для адреса, `None` для прямого пути или ошибка переменной.
    pub fn route(&self, url: &url::Url) -> Result<Option<&ProxyChoice>, &ProxySettingError> {
        let scheme = match url.scheme() {
            "https" => &self.https,
            "http" => &self.http,
            _ => return Ok(None),
        };
        if url.host_str().is_some_and(|host| self.bypass.matches(host)) {
            return Ok(None);
        }
        scheme.as_ref().map(Option::as_ref)
    }

    /// Все разобранные прокси: для них заранее собираются агенты.
    pub fn choices(&self) -> impl Iterator<Item = &ProxyChoice> {
        [&self.https, &self.http]
            .into_iter()
            .filter_map(|scheme| scheme.as_ref().ok().and_then(Option::as_ref))
    }
}

fn first_value(lookup: EnvLookup<'_>, names: &[&'static str]) -> Option<(&'static str, String)> {
    names.iter().find_map(|name| {
        lookup(name)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|value| (*name, value))
    })
}

/// Проверяется только действующая для схемы переменная: `ALL_PROXY` со схемой
/// SOCKS не мешает, когда задан `HTTPS_PROXY`.
fn choose(lookup: EnvLookup<'_>, names: &[&'static str]) -> SchemeProxy {
    let Some((variable, raw)) = first_value(lookup, names) else {
        return Ok(None);
    };
    let shown = without_password(&raw);
    let refuse = |why: &str| ProxySettingError {
        message: format!("{variable}={shown}: {why}"),
    };
    if let Some((scheme, _)) = raw.split_once("://") {
        if !scheme.eq_ignore_ascii_case("http") {
            return Err(refuse(&format!(
                "proxy scheme `{scheme}` is not supported; Unica connects only through an HTTP proxy (http://host:port)"
            )));
        }
    }
    let proxy = ureq::Proxy::new(&raw).map_err(|error| refuse(&error.to_string()))?;
    Ok(Some(ProxyChoice {
        variable,
        raw,
        shown,
        proxy,
    }))
}

/// `user:password@host` → `user:***@host`. Пароль в прокси — секрет, а текст
/// отказа bootstrap пишет в журнал попыток на диске.
pub(crate) fn without_password(raw: &str) -> String {
    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, raw),
    };
    let masked = match rest.rsplit_once('@') {
        Some((credentials, host)) => match credentials.split_once(':') {
            Some((user, _)) => format!("{user}:***@{host}"),
            None => format!("{credentials}@{host}"),
        },
        None => rest.to_owned(),
    };
    match scheme {
        Some(scheme) => format!("{scheme}://{masked}"),
        None => masked,
    }
}

/// Узлы из `NO_PROXY`.
///
/// Запись совпадает с самим узлом и с любым его поддоменом: `example.com`
/// и `.example.com` пропускают `example.com` и `api.example.com`. `*` — все
/// узлы. IP-адрес сравнивается целиком, подсети (CIDR) не поддерживаются.
/// Порт в записи отбрасывается. Неявного исключения для `localhost` нет,
/// как и у curl: его перечисляют явно.
#[derive(Debug, Default)]
pub(crate) struct NoProxy {
    everything: bool,
    entries: Vec<String>,
}

impl NoProxy {
    fn parse(value: Option<String>) -> Self {
        let mut result = Self::default();
        for entry in value.iter().flat_map(|value| value.split(',')) {
            let entry = entry.trim();
            if entry == "*" {
                result.everything = true;
                continue;
            }
            let entry = normalize_entry(entry);
            if !entry.is_empty() {
                result.entries.push(entry);
            }
        }
        result
    }

    pub fn matches(&self, host: &str) -> bool {
        if self.everything {
            return true;
        }
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_end_matches('.')
            .to_ascii_lowercase();
        self.entries.iter().any(|entry| {
            host == *entry
                || (host.len() > entry.len()
                    && host.ends_with(entry.as_str())
                    && host.as_bytes()[host.len() - entry.len() - 1] == b'.')
        })
    }
}

fn normalize_entry(entry: &str) -> String {
    let entry = entry.trim_start_matches("*.").trim_start_matches('.');
    // `[::1]:8080`, `[::1]`, `host:8080`; голый IPv6 без скобок оставляем как есть.
    let entry = if let Some(rest) = entry.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else if entry.matches(':').count() == 1 {
        entry.split(':').next().unwrap_or_default()
    } else {
        entry
    };
    entry.trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn settings(pairs: &[(&str, &str)]) -> ProxySettings {
        let table: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        ProxySettings::from_lookup(&move |name| table.get(name).cloned())
    }

    fn route_of(settings: &ProxySettings, url: &str) -> Option<&'static str> {
        settings
            .route(&url::Url::parse(url).expect("url"))
            .expect("valid proxy setting")
            .map(|choice| choice.variable)
    }

    #[test]
    fn https_proxy_wins_over_all_proxy_and_lowercase_over_uppercase() {
        let both = settings(&[
            ("ALL_PROXY", "http://all:1"),
            ("HTTPS_PROXY", "http://upper:1"),
            ("https_proxy", "http://lower:1"),
        ]);
        assert_eq!(route_of(&both, "https://github.com/x"), Some("https_proxy"));

        let all_only = settings(&[("ALL_PROXY", "http://all:1")]);
        assert_eq!(
            route_of(&all_only, "https://github.com/x"),
            Some("ALL_PROXY")
        );
        assert_eq!(
            route_of(&all_only, "http://github.com/x"),
            Some("ALL_PROXY")
        );
    }

    #[test]
    fn uppercase_http_proxy_is_not_read_for_plain_http() {
        // httpoxy: в CGI-окружении `HTTP_PROXY` приходит из заголовка запроса.
        let only_upper = settings(&[("HTTP_PROXY", "http://upper:1")]);
        assert_eq!(route_of(&only_upper, "http://example.com/"), None);
        assert_eq!(route_of(&only_upper, "https://example.com/"), None);
    }

    #[test]
    fn empty_variables_mean_direct() {
        let empty = settings(&[("HTTPS_PROXY", "  "), ("ALL_PROXY", "")]);
        assert_eq!(route_of(&empty, "https://github.com/"), None);
    }

    #[test]
    fn an_unsupported_scheme_is_refused_by_name_without_the_password() {
        let settings = settings(&[("ALL_PROXY", "socks5://alice:s3cret@proxy.corp:1080")]);
        let error = settings
            .route(&url::Url::parse("https://github.com/").expect("url"))
            .expect_err("socks is refused");
        assert!(error.message.starts_with("ALL_PROXY="), "{}", error.message);
        assert!(error.message.contains("socks5"), "{}", error.message);
        assert!(!error.message.contains("s3cret"), "{}", error.message);
        assert!(
            error.message.contains("alice:***@proxy.corp"),
            "{}",
            error.message
        );
    }

    #[test]
    fn a_socks_all_proxy_is_not_consulted_when_https_proxy_is_set() {
        let settings = settings(&[
            ("ALL_PROXY", "socks5://proxy.corp:1080"),
            ("HTTPS_PROXY", "http://proxy.corp:3128"),
        ]);
        // Для https действует HTTPS_PROXY, и неподдерживаемый ALL_PROXY ему
        // не мешает; отказ получит только запрос по http, которому он и адресован.
        assert_eq!(
            route_of(&settings, "https://github.com/"),
            Some("HTTPS_PROXY")
        );
        assert!(settings
            .route(&url::Url::parse("http://github.com/").expect("url"))
            .is_err());
    }

    #[test]
    fn no_proxy_matches_hosts_and_subdomains_but_not_lookalikes() {
        let settings = settings(&[
            ("HTTPS_PROXY", "http://proxy.corp:3128"),
            (
                "NO_PROXY",
                " .githubusercontent.com, Example.COM:443,10.0.0.1,[::1]",
            ),
        ]);
        assert_eq!(
            route_of(&settings, "https://release-assets.githubusercontent.com/a"),
            None
        );
        assert_eq!(route_of(&settings, "https://githubusercontent.com/a"), None);
        assert_eq!(route_of(&settings, "https://api.example.com/"), None);
        assert_eq!(route_of(&settings, "https://10.0.0.1/"), None);
        assert_eq!(route_of(&settings, "https://[::1]:8443/"), None);
        assert_eq!(
            route_of(&settings, "https://notexample.com/"),
            Some("HTTPS_PROXY")
        );
        assert_eq!(
            route_of(&settings, "https://github.com/"),
            Some("HTTPS_PROXY")
        );
        assert_eq!(
            route_of(&settings, "https://10.0.0.10/"),
            Some("HTTPS_PROXY")
        );
    }

    #[test]
    fn a_star_in_no_proxy_bypasses_everything() {
        let settings = settings(&[("https_proxy", "http://p:1"), ("no_proxy", "*")]);
        assert_eq!(route_of(&settings, "https://github.com/"), None);
    }

    #[test]
    fn a_password_never_reaches_the_shown_value() {
        assert_eq!(
            without_password("http://bob:pa:ss@proxy:8080"),
            "http://bob:***@proxy:8080"
        );
        assert_eq!(without_password("bob@proxy:8080"), "bob@proxy:8080");
        assert_eq!(without_password("proxy:8080"), "proxy:8080");
    }
}

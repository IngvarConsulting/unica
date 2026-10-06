//! Единственный HTTP-клиент Unica.
//!
//! Через него ходят загрузка ядра, доставка движков и сетевые поставщики
//! документации. Здесь одно место, где настраиваются TLS и прокси:
//!
//! - цепочку сертификатов проверяет ОС через `rustls-platform-verifier`:
//!   Keychain на macOS, хранилище сертификатов на Windows, системный набор
//!   корней на Linux (`SSL_CERT_FILE`/`SSL_CERT_DIR` его заменяют). Корень
//!   корпоративного шлюза, установленный в ОС, принимается так же, как
//!   браузером. Вшитого набора корней нет;
//! - прокси берётся из переменных окружения, порядок описан в [`proxy`];
//! - редиректы проходятся здесь, а не в `ureq`: для каждого шага заново
//!   выбирается маршрут по `NO_PROXY`, и ни один шаг не уводит с HTTPS на HTTP.
//!
//! Клиент собирается один раз на процесс ([`NetworkClient::shared`]) и
//! держит пул соединений. Неудачная сборка не запоминается: исправленную
//! настройку подхватит следующий запрос того же процесса.

mod failure;
mod proxy;
#[cfg(any(test, feature = "network-test-support"))]
pub mod test_support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use failure::{NetworkError, NetworkFailure};
use proxy::{EnvLookup, ProxyChoice, ProxySettings};
use rustls_platform_verifier::BuilderVerifierExt;

/// Сколько редиректов проходим. Столько же разрешал прежний агент.
const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Ожидание следующей порции данных, а не всей передачи: медленный, но живой
/// канал не обрывается.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// HTTP-клиент процесса: одна конфигурация TLS, одни настройки прокси.
///
/// `ureq` 2 задаёт прокси на весь агент и не знает `NO_PROXY`, поэтому внутри
/// по агенту на маршрут: прямой и по одному на каждый различный прокси. Все
/// делят одну конфигурацию TLS.
pub struct NetworkClient {
    settings: ProxySettings,
    direct: ureq::Agent,
    proxied: HashMap<String, ureq::Agent>,
}

/// Заголовки запроса: имя и значение.
pub type Headers<'a> = &'a [(&'a str, &'a str)];

impl NetworkClient {
    /// Клиент процесса: собирается при первом запросе из окружения процесса
    /// и хранилища корней ОС.
    pub fn shared() -> Result<Arc<NetworkClient>, NetworkError> {
        static SHARED: Mutex<Option<Arc<NetworkClient>>> = Mutex::new(None);
        let mut slot = SHARED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(client) = slot.as_ref() {
            return Ok(Arc::clone(client));
        }
        let client = Arc::new(Self::from_environment(&|name| std::env::var(name).ok())?);
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }

    /// Клиент с переменными прокси из `lookup` и доверием ОС.
    pub fn from_environment(lookup: EnvLookup<'_>) -> Result<Self, NetworkError> {
        let settings = ProxySettings::from_lookup(lookup);
        let tls = platform_tls()?;
        let build = |proxy: Option<&ProxyChoice>| {
            let mut builder = ureq::AgentBuilder::new()
                .tls_config(Arc::clone(&tls))
                .try_proxy_from_env(false)
                .timeout_connect(CONNECT_TIMEOUT)
                .timeout_read(READ_TIMEOUT)
                .redirects(0);
            if let Some(choice) = proxy {
                builder = builder.proxy(choice.proxy.clone());
            }
            builder.build()
        };
        let mut proxied = HashMap::new();
        for choice in settings.choices() {
            proxied
                .entry(choice.key().to_owned())
                .or_insert_with(|| build(Some(choice)));
        }
        Ok(Self {
            direct: build(None),
            proxied,
            settings,
        })
    }

    /// GET с проходом редиректов. Ответ `4xx`/`5xx` — отказ
    /// [`NetworkFailure::Status`].
    pub fn get(
        &self,
        url: &str,
        headers: Headers<'_>,
        timeout: Option<Duration>,
    ) -> Result<ureq::Response, NetworkError> {
        let mut current = parse(url)?;
        for _ in 0..=MAX_REDIRECTS {
            let (agent, route) = self.route(&current)?;
            let mut request = agent.request_url("GET", &current);
            for (name, value) in headers {
                request = request.set(name, value);
            }
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            let response = request
                .call()
                .map_err(|error| NetworkError::from_transport(error, host(&current), &route))?;
            if !(300..400).contains(&response.status()) {
                return Ok(response);
            }
            let Some(location) = response.header("Location") else {
                return Ok(response);
            };
            let next = current.join(location).map_err(|error| {
                NetworkError::new(
                    NetworkFailure::Transport,
                    format!("{current} redirected to an invalid location {location}: {error}"),
                )
            })?;
            if current.scheme() == "https" && next.scheme() != "https" {
                return Err(NetworkError::new(
                    NetworkFailure::InsecureRedirect,
                    format!("{current} redirected to a non-HTTPS URL: {next}"),
                ));
            }
            current = next;
        }
        Err(NetworkError::new(
            NetworkFailure::Transport,
            format!("{url}: more than {MAX_REDIRECTS} redirects"),
        ))
    }

    /// POST тела без прохода редиректов: перевод JSON-RPC в GET бессмыслен.
    pub fn post(
        &self,
        url: &str,
        headers: Headers<'_>,
        body: &str,
        timeout: Option<Duration>,
    ) -> Result<ureq::Response, NetworkError> {
        let current = parse(url)?;
        let (agent, route) = self.route(&current)?;
        let mut request = agent.request_url("POST", &current);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        request
            .send_string(body)
            .map_err(|error| NetworkError::from_transport(error, host(&current), &route))
    }

    /// Агент и описание маршрута для текста отказа.
    fn route(&self, url: &url::Url) -> Result<(&ureq::Agent, String), NetworkError> {
        match self.settings.route(url) {
            Ok(Some(choice)) => Ok((
                self.proxied
                    .get(choice.key())
                    .expect("an agent is built for every configured proxy"),
                format!("through proxy {} from {}", choice.shown, choice.variable),
            )),
            Ok(None) => Ok((&self.direct, "direct connection".to_owned())),
            Err(error) => Err(NetworkError::proxy_setting(error.message.clone())),
        }
    }
}

fn parse(url: &str) -> Result<url::Url, NetworkError> {
    url::Url::parse(url).map_err(|error| {
        NetworkError::new(
            NetworkFailure::Configuration,
            format!("invalid URL {url}: {error}"),
        )
    })
}

fn host(url: &url::Url) -> &str {
    url.host_str().unwrap_or("<no host>")
}

/// TLS поверх доверия ОС. Криптопровайдер — `ring`, как у самого `ureq`:
/// второй провайдер в сборке сделал бы выбор по умолчанию неоднозначным.
fn platform_tls() -> Result<Arc<rustls::ClientConfig>, NetworkError> {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| NetworkError::trust_store(&error))?
    .with_platform_verifier()
    .map_err(|error| NetworkError::trust_store(&error))?
    .with_no_client_auth();
    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

    /// HTTP-стенд: `/start` уводит относительным редиректом на `/final`,
    /// `/final` отвечает телом. Запоминает строку запроса и `Range`.
    fn redirecting_stand() -> (String, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut request_line = String::new();
                reader.read_line(&mut request_line).expect("request line");
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Range: ") {
                        range = Some(value.trim().to_owned());
                    }
                }
                let request_line = request_line.trim().to_owned();
                let answer = if request_line.starts_with("GET /start ")
                    || request_line.starts_with("POST /start ")
                {
                    "HTTP/1.1 302 Found\r\nLocation: final\r\nContent-Length: 0\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndone"
                };
                log.lock().expect("log").push((request_line, range));
                let _ = stream.write_all(answer.as_bytes());
            }
        });
        (base, seen)
    }

    #[test]
    fn a_redirect_is_followed_with_the_same_headers_and_a_relative_location() {
        let (base, seen) = redirecting_stand();
        let client = NetworkClient::from_environment(&|_| None).expect("client");

        let response = client
            .get(&format!("{base}/start"), &[("Range", "bytes=10-")], None)
            .expect("redirect followed");

        assert_eq!(response.into_string().expect("body"), "done");
        assert_eq!(
            *seen.lock().expect("log"),
            vec![
                (
                    "GET /start HTTP/1.1".to_owned(),
                    Some("bytes=10-".to_owned())
                ),
                (
                    "GET /final HTTP/1.1".to_owned(),
                    Some("bytes=10-".to_owned())
                ),
            ],
            "докачка на новом адресе просит тот же хвост"
        );
    }

    #[test]
    fn a_post_is_not_redirected() {
        let (base, seen) = redirecting_stand();
        let client = NetworkClient::from_environment(&|_| None).expect("client");

        let response = client
            .post(&format!("{base}/start"), &[], "{}", None)
            .expect("answer");

        assert_eq!(response.status(), 302);
        assert_eq!(seen.lock().expect("log").len(), 1);
    }
}

//! Единственный HTTP-клиент Unica.
//!
//! Через него ходят загрузка ядра, доставка движков и сетевые поставщики
//! документации. Здесь одно место, где настраиваются TLS и прокси:
//!
//! - цепочку сертификатов проверяет ОС через `rustls-platform-verifier`:
//!   Keychain на macOS, хранилище сертификатов на Windows, системный набор
//!   корней на Linux (`SSL_CERT_FILE`/`SSL_CERT_DIR` его заменяют). Корень
//!   корпоративного шлюза, установленный в ОС, принимается так же, как
//!   браузером. Вшитый набор корней Mozilla — только запас на случай, когда
//!   в хранилище ОС нет ни одного корня (Linux-образ без `ca-certificates`);
//!   два набора не смешиваются, см. [`TrustSource`];
//! - прокси берётся из переменных окружения, порядок описан в [`proxy`];
//! - редиректы проходятся здесь, в этом модуле: для каждого шага заново
//!   выбирается маршрут по `NO_PROXY`, и ни один шаг не уводит с HTTPS на HTTP;
//! - каждый запрос называет себя [`USER_AGENT`]: имя и версия Unica, без
//!   сведений о машине и пользователе. Поставщики документации уточняют
//!   назначение через [`DOCS_USER_AGENT`].
//!
//! Клиент собирается один раз на процесс ([`NetworkClient::shared`]) и
//! держит пул соединений. Неудачная сборка не запоминается: исправленную
//! настройку подхватит следующий запрос того же процесса.

mod failure;
mod proxy;
mod response;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use failure::{NetworkError, NetworkFailure};
use proxy::{EnvLookup, ProxyChoice, ProxySettings};
use response::run_io;
pub use response::NetworkResponse;
use rustls_platform_verifier::BuilderVerifierExt;

/// Значение `User-Agent` с назначением запроса. Только имя и версия пакета
/// и адрес проекта: ни узла, ни пути, ни пользователя.
macro_rules! user_agent {
    ($($purpose:literal)?) => {
        concat!(
            "unica/",
            env!("CARGO_PKG_VERSION"),
            " (",
            $($purpose, "; ",)?
            "+https://github.com/IngvarConsulting/unica)"
        )
    };
}

/// `User-Agent` любого запроса клиента, если вызывающий не уточнил своё.
pub const USER_AGENT: &str = user_agent!();
/// `User-Agent` сетевого поставщика документации (`v8std`).
pub const DOCS_USER_AGENT: &str = user_agent!("docs");

/// Сколько редиректов проходим. Столько же разрешал прежний агент.
// resource-limit: network.max-redirects
// class: format
// unit: count
// scope: request
// value: 5
// resource: Число переходов по редиректам одного сетевого запроса.
// status: unproven
// origin: #1266
// problem: Столько же разрешал прежний агент.
// on-reach: ошибка «<url>: more than 5 redirects»
const MAX_REDIRECTS: usize = 5;
// resource-limit: network.connect-timeout
// class: deadline
// unit: ms
// scope: connection
// value: 30000
// resource: Срок установления сетевого соединения агента загрузки.
// status: unproven
// origin: #1266
// problem: неизвестно
// on-reach: запрос завершается ошибкой соединения по сроку
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Ожидание следующей порции данных, а не всей передачи: медленный, но живой
/// канал не обрывается.
// resource-limit: network.read-timeout
// class: deadline
// unit: ms
// scope: connection
// value: 60000
// resource: Ожидание следующей порции данных сетевого ответа, а не всей передачи.
// status: unproven
// origin: #1266
// problem: Медленный, но живой канал не должен обрываться.
// on-reach: запрос завершается ошибкой чтения по сроку
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// HTTP-клиент процесса: одна конфигурация TLS, одни настройки прокси.
///
/// Для явной маршрутизации внутри по HTTP-клиенту на маршрут: прямой и по
/// одному на каждый различный прокси. Все
/// делят одну конфигурацию TLS.
pub struct NetworkClient {
    settings: ProxySettings,
    trust: TrustSource,
    direct: reqwest::Client,
    proxied: HashMap<String, reqwest::Client>,
}

/// Чему доверяет клиент при проверке цепочки сертификатов.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustSource {
    /// Хранилище ОС через `rustls-platform-verifier`. Обычный случай.
    OperatingSystem,
    /// Вшитый набор корней Mozilla (`webpki-roots`): в хранилище ОС нет ни
    /// одного корня. Так работал загрузчик до перехода на хранилище ОС, и
    /// поэтому минимальный Linux-образ без `ca-certificates` не теряет сеть.
    BundledMozillaRoots,
}

/// Так верификатор Linux сообщает, что из хранилища ОС не загружен ни один
/// корень (`rustls-platform-verifier`, `verification/others.rs`). Сюда же
/// попадают нечитаемые `SSL_CERT_FILE`/`SSL_CERT_DIR`: верификатор их ошибки
/// только пишет в журнал, поэтому текст отказа называет и этот случай. Других
/// причин отказа его конструктор с непустым хранилищем не даёт; на macOS
/// и Windows конструктор хранилище не читает и этой ошибки не бывает.
/// Если формулировка у верификатора изменится, запас перестанет включаться,
/// и это поймает тест пустого хранилища на Linux.
const EMPTY_OS_STORE: &str = "No CA certificates were loaded from the system";

/// Выбрать доверие по исходу сборки верификатора ОС: пустое хранилище —
/// вшитый набор, любая другая ошибка — отказ, успех — только ОС.
fn select_trust<T>(platform: Result<T, rustls::Error>) -> Result<Option<T>, rustls::Error> {
    match platform {
        Ok(verified) => Ok(Some(verified)),
        Err(rustls::Error::General(message)) if message == EMPTY_OS_STORE => Ok(None),
        Err(error) => Err(error),
    }
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
        let (tls, trust) = tls_config()?;
        Self::build(settings, tls, trust, READ_TIMEOUT)
    }

    fn build(
        settings: ProxySettings,
        tls: Arc<rustls::ClientConfig>,
        trust: TrustSource,
        read_timeout: Duration,
    ) -> Result<Self, NetworkError> {
        let build_client = |proxy: Option<&ProxyChoice>| {
            let mut builder = reqwest::Client::builder()
                .tls_backend_preconfigured((*tls).clone())
                .no_proxy()
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(read_timeout)
                .connection_verbose(false)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .http1_only()
                .pool_max_idle_per_host(1);
            if let Some(choice) = proxy {
                builder = builder.proxy(choice.proxy.clone());
            }
            builder.build().map_err(|_| {
                NetworkError::new(
                    NetworkFailure::Configuration,
                    "cannot build the shared HTTP transport",
                )
            })
        };
        let mut proxied = HashMap::new();
        for choice in settings.choices() {
            if !proxied.contains_key(choice.key()) {
                proxied.insert(choice.key().to_owned(), build_client(Some(choice))?);
            }
        }
        Ok(Self {
            trust,
            direct: build_client(None)?,
            proxied,
            settings,
        })
    }

    /// Чему доверяет этот клиент.
    pub fn trust_source(&self) -> TrustSource {
        self.trust
    }

    /// GET with explicit redirects and an optional total request deadline.
    pub fn get(
        &self,
        url: &str,
        headers: Headers<'_>,
        timeout: Option<Duration>,
    ) -> Result<NetworkResponse, NetworkError> {
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        let mut current = parse(url)?;
        for _ in 0..=MAX_REDIRECTS {
            let response = self.send(reqwest::Method::GET, &current, headers, None, deadline)?;
            if !(300..400).contains(&response.status()) {
                return Ok(response);
            }
            let Some(location) = response.header("Location") else {
                return Ok(response);
            };
            current = next_hop(&current, location)?;
        }
        Err(NetworkError::new(
            NetworkFailure::Transport,
            format!(
                "{}: more than {MAX_REDIRECTS} redirects",
                diagnostic_url(url)
            ),
        ))
    }

    /// POST без редиректов: перевод JSON-RPC в GET бессмыслен.
    pub fn post(
        &self,
        url: &str,
        headers: Headers<'_>,
        body: &str,
        timeout: Option<Duration>,
    ) -> Result<NetworkResponse, NetworkError> {
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        self.send(
            reqwest::Method::POST,
            &parse(url)?,
            headers,
            Some(body),
            deadline,
        )
    }

    fn send(
        &self,
        method: reqwest::Method,
        url: &url::Url,
        headers: Headers<'_>,
        body: Option<&str>,
        deadline: Option<Instant>,
    ) -> Result<NetworkResponse, NetworkError> {
        use reqwest::header::{
            HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, USER_AGENT as HEADER_AGENT,
        };

        let (client, route) = self.route(url)?;
        let mut request_headers = HeaderMap::new();
        request_headers.insert(HEADER_AGENT, HeaderValue::from_static(USER_AGENT));
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid_headers())?;
            let mut value = HeaderValue::from_str(value).map_err(|_| invalid_headers())?;
            value.set_sensitive(matches!(
                name.as_str(),
                "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
            ));
            request_headers.insert(name, value);
        }

        // Keep URL credentials out of library request state and all request
        // formatters. Preserve the old Basic auth behavior (escaped username
        // and password from the URL) when the caller supplies no header.
        let mut request = client.request(method, without_userinfo(url));
        let password = url.password().unwrap_or_default();
        if !request_headers.contains_key(AUTHORIZATION)
            && (!url.username().is_empty() || !password.is_empty())
        {
            request = request.basic_auth(url.username(), Some(password));
        }
        request = request.headers(request_headers);
        if let Some(body) = body {
            request = request.body(body.to_owned());
        }
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(NetworkError::timed_out(host(url), &route));
            }
            request = request.timeout(remaining);
        }
        let addr = host(url).to_owned();
        let trust = self.trust;
        run_io(async move {
            let response = request
                .send()
                .await
                .map_err(|error| NetworkError::from_transport(&error, &addr, &route, trust))?;
            let status = response.status();
            if status.is_client_error() || status.is_server_error() {
                return Err(NetworkError::new(
                    NetworkFailure::Status(status.as_u16()),
                    format!("{addr} answered HTTP {} ({route})", status.as_u16()),
                ));
            }
            Ok(NetworkResponse::new(response, addr, route, trust))
        })?
    }

    /// Клиент и описание маршрута для текста отказа.
    fn route(&self, url: &url::Url) -> Result<(&reqwest::Client, String), NetworkError> {
        match self.settings.route(url) {
            Ok(Some(choice)) => Ok((
                self.proxied
                    .get(choice.key())
                    .expect("a client is built for every configured proxy"),
                format!("through proxy {} from {}", choice.shown, choice.variable),
            )),
            Ok(None) => Ok((&self.direct, "direct connection".to_owned())),
            Err(error) => Err(NetworkError::proxy_setting(error.message.clone())),
        }
    }
}

fn invalid_headers() -> NetworkError {
    NetworkError::new(NetworkFailure::Configuration, "invalid HTTP request header")
}

/// Куда ведёт редирект. Относительный адрес разрешается от текущего;
/// уход с HTTPS на HTTP запрещён на любом шаге.
fn next_hop(current: &url::Url, location: &str) -> Result<url::Url, NetworkError> {
    let next = current.join(location).map_err(|_| {
        NetworkError::new(
            NetworkFailure::Transport,
            format!("{} redirected to an invalid URL", host(current)),
        )
    })?;
    if current.scheme() == "https" && next.scheme() != "https" {
        return Err(NetworkError::new(
            NetworkFailure::InsecureRedirect,
            format!("{} redirected to a non-HTTPS URL", host(current)),
        ));
    }
    Ok(next)
}

fn parse(url: &str) -> Result<url::Url, NetworkError> {
    url::Url::parse(url).map_err(|error| {
        NetworkError::new(
            NetworkFailure::Configuration,
            format!("invalid request URL: {error}"),
        )
    })
}

fn without_userinfo(url: &url::Url) -> url::Url {
    let mut sanitized = url.clone();
    let _ = sanitized.set_password(None);
    let _ = sanitized.set_username("");
    sanitized
}

/// An address safe for user-facing errors. Paths and query values can carry
/// credentials (including signed asset URLs).
pub fn diagnostic_url(raw: &str) -> String {
    url::Url::parse(raw)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?;
            Some(match url.port() {
                Some(port) => format!("{}://{host}:{port}", url.scheme()),
                None => format!("{}://{host}", url.scheme()),
            })
        })
        .unwrap_or_else(|| "<invalid URL>".to_owned())
}

/// Preserve an ordinary endpoint as provenance. For configured credentials or
/// query strings, only the origin is suitable for outgoing diagnostics.
pub fn diagnostic_endpoint(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(url)
            if url.has_host()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none() =>
        {
            raw.to_owned()
        }
        _ => diagnostic_url(raw),
    }
}

fn host(url: &url::Url) -> &str {
    url.host_str().unwrap_or("<no host>")
}

/// Вшитый набор корней Mozilla — весь, без добавлений.
fn bundled_roots() -> rustls::RootCertStore {
    rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
}

/// TLS поверх доверия ОС, а при пустом хранилище ОС — поверх вшитого набора
/// Mozilla. Криптопровайдер явно выбирается здесь.
fn tls_config() -> Result<(Arc<rustls::ClientConfig>, TrustSource), NetworkError> {
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| NetworkError::trust_store(&error))?;
    let (config, trust) = match select_trust(builder.clone().with_platform_verifier())
        .map_err(|error| NetworkError::trust_store(&error))?
    {
        Some(platform) => (platform.with_no_client_auth(), TrustSource::OperatingSystem),
        None => (
            builder
                .with_root_certificates(bundled_roots())
                .with_no_client_auth(),
            TrustSource::BundledMozillaRoots,
        ),
    };
    Ok((Arc::new(config), trust))
}

#[cfg(test)]
mod medium_tests {
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
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("Range") {
                            range = Some(value.trim().to_owned());
                        }
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
    fn the_user_agent_names_only_unica_its_version_and_the_purpose() {
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            USER_AGENT,
            format!("unica/{version} (+https://github.com/IngvarConsulting/unica)")
        );
        assert_eq!(
            DOCS_USER_AGENT,
            format!("unica/{version} (docs; +https://github.com/IngvarConsulting/unica)")
        );
        assert!(!version.is_empty());
    }

    #[test]
    fn every_request_carries_the_unica_user_agent_unless_the_caller_narrows_it() {
        let stand = test_support::HttpStand::start("ok", "text/plain");
        let client = NetworkClient::from_environment(&|_| None).expect("client");

        client.get(&stand.url("a"), &[], None).expect("get");
        client.post(&stand.url("b"), &[], "{}", None).expect("post");
        client
            .get(&stand.url("c"), &[("User-Agent", DOCS_USER_AGENT)], None)
            .expect("narrowed get");

        assert_eq!(
            stand.header("User-Agent"),
            vec![
                vec![USER_AGENT.to_owned()],
                vec![USER_AGENT.to_owned()],
                vec![DOCS_USER_AGENT.to_owned()],
            ],
            "уточнённое значение заменяет общее, а не добавляется вторым"
        );
    }

    /// rule: INV.PKG.NETWORK-TRUST-AND-PROXY
    #[test]
    fn a_redirect_from_https_to_plain_http_is_refused_at_any_step() {
        let current = url::Url::parse("https://github.com/release").expect("url");
        let error = next_hop(&current, "http://objects.example/asset").expect_err("понижение");
        assert_eq!(error.failure(), NetworkFailure::InsecureRedirect);
        assert_eq!(
            next_hop(&current, "/asset")
                .expect("относительный")
                .as_str(),
            "https://github.com/asset"
        );
        let plain = url::Url::parse("http://mirror.example/a").expect("url");
        assert!(next_hop(&plain, "https://github.com/a").is_ok());
    }

    /// rule: INV.PKG.NETWORK-PROXY-ROUTING
    #[test]
    fn each_redirect_step_chooses_its_own_route() {
        // Первый узел в NO_PROXY, второй — нет: второй шаг обязан пойти
        // через прокси, хотя первый шёл напрямую.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let first = format!("http://{}/start", listener.local_addr().expect("address"));
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 2 {
                    line.clear();
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://proxied.example/final\r\nContent-Length: 0\r\n\r\n",
                );
            }
        });
        let proxy = test_support::ConnectProxy::start();
        let proxy_url = proxy.url();
        let client = NetworkClient::from_environment(&move |name| match name {
            "http_proxy" => Some(proxy_url.clone()),
            "NO_PROXY" => Some("127.0.0.1".to_owned()),
            _ => None,
        })
        .expect("client");

        let error = client
            .get(&first, &[], None)
            .expect_err("прокси стенда отвечает 405");

        assert_eq!(error.failure(), NetworkFailure::Status(405));
        assert_eq!(
            proxy.targets(),
            vec!["http://proxied.example/final".to_owned()]
        );
    }

    /// rule: INV.PKG.NETWORK-TRUST-AND-PROXY
    #[test]
    fn an_empty_os_store_falls_back_to_bundled_roots_and_nothing_else_does() {
        // Пустое хранилище ОС — единственный повод взять вшитый набор.
        assert_eq!(
            select_trust::<()>(Err(rustls::Error::General(EMPTY_OS_STORE.to_owned()))),
            Ok(None)
        );
        // Непустое хранилище — только ОС, без смешивания с вшитым набором.
        assert_eq!(select_trust(Ok("platform")), Ok(Some("platform")));
        // Любая другая ошибка верификатора — отказ, а не тихий запас.
        let other = rustls::Error::General("verifier failed".to_owned());
        assert_eq!(select_trust::<()>(Err(other.clone())), Err(other));
    }

    /// rule: INV.PKG.NETWORK-TRUST-AND-PROXY
    #[test]
    fn the_fallback_trusts_exactly_the_bundled_mozilla_roots() {
        let roots = bundled_roots();
        assert!(!roots.is_empty());
        assert_eq!(roots.len(), webpki_roots::TLS_SERVER_ROOTS.len());
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

#[cfg(test)]
mod transport_tests;

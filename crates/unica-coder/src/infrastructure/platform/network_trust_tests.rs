//! Сетевые поставщики `unica.docs` с корнем из хранилища ОС: Linux.
//!
//! Те же проверки, что у загрузчика (`unica-bootstrap`, модуль
//! `platform::network_trust_tests`): `SSL_CERT_FILE` заменяет системный набор
//! корней Linux и задаётся дочернему процессу этого же тестового бинаря.
//! В дочернем процессе поставщики работают со штатными транспортами —
//! общим клиентом, собранным из окружения процесса. Отказ на недоверенном
//! корне и маршрут через прокси на всех ОС проверяют тесты в `kb_1ci.rs`
//! и `standards_documentation.rs`.

use std::sync::Arc;
use std::time::Duration;

use crate::domain::documentation::{
    DocumentationContext, DocumentationProvider, DocumentationSearchRequest,
    DocumentationSectionStatus, SourceKind,
};
use crate::infrastructure::documentation_policy::NetworkAccess;

const SCENARIO: &str = "UNICA_DOCS_TRUST_SCENARIO";
const ADDRESS: &str = "UNICA_DOCS_TRUST_ADDRESS";

/// Дочерний процесс: поиск поставщиком со штатным транспортом.
/// Без переменной сценария — не дочерний процесс, и делать ему нечего.
#[test]
fn docs_network_trust_child_process() {
    let Ok(scenario) = std::env::var(SCENARIO) else {
        return;
    };
    let address = std::env::var(ADDRESS).expect("address");
    let cancellation = crate::domain::cancellation::CancellationToken::default();
    let context = DocumentationContext {
        platform_version: Some("8.3.27".to_string()),
        installation_root: None,
    };
    match scenario.as_str() {
        "v8std" => {
            let provider =
                crate::infrastructure::standards_documentation::V8StdDocumentationProvider {
                    endpoint: address,
                    network: NetworkAccess::Allow,
                    http: crate::infrastructure::internal_adapters::shared_http_client(),
                    cancellation,
                    search_cache_ttl: Duration::from_secs(60),
                    revalidate_search: false,
                };
            let request = DocumentationSearchRequest {
                query: "ссылка".to_string(),
                source_kinds: Vec::new(),
                limit: 20,
                language: "ru".to_string(),
            };
            let sections = provider.search(&request, &context);
            assert!(
                matches!(sections[0].status, DocumentationSectionStatus::Ok),
                "{:?}",
                sections[0].status
            );
            assert_eq!(sections[0].hits[0].document_id, "https://v8std.ru/std/702/");
        }
        "kb" => {
            let provider = crate::infrastructure::kb_1ci::Kb1ciProvider {
                base: address,
                network: NetworkAccess::Allow,
                transport: Arc::new(crate::infrastructure::kb_1ci::NetworkKbTransport::default()),
                cancellation,
                cache_ttl: Duration::from_secs(60),
                lexicon: Arc::new(crate::infrastructure::kb_1ci::InstallationLexiconSource),
            };
            let request = DocumentationSearchRequest {
                query: "URL".to_string(),
                source_kinds: vec![SourceKind::PlatformHelp],
                limit: 20,
                language: "en".to_string(),
            };
            // Стенд отдаёт пустое оглавление, и поставщик отвечает, что
            // раздела платформы нет. Это ответ площадки, дошедший по сети;
            // сетевой отказ называл бы маршрут соединения.
            for section in provider.search(&request, &context) {
                if let DocumentationSectionStatus::Unavailable { detail, .. } = &section.status {
                    assert!(
                        !detail.contains("direct connection") && !detail.contains("through proxy"),
                        "сеть не прошла: {detail}"
                    );
                }
            }
        }
        other => panic!("неизвестный сценарий {other}"),
    }
}

// Родительские проверки ниже идут только на Linux: там хранилище корней
// задаётся через `SSL_CERT_FILE`. Дочерний тест выше собирается везде, чтобы
// отбор по размеру видел модуль на любой ОС.

#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(target_os = "linux")]
use unica_bootstrap::network::test_support::{ConnectProxy, TestRoot, TlsStand};

#[cfg(target_os = "linux")]
const CHILD: &str =
    "infrastructure::platform::network_trust_tests::docs_network_trust_child_process";

#[cfg(target_os = "linux")]
/// Ответ сервера стандартов: конверт JSON-RPC с одним результатом.
const STANDARDS_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"query\":\"ссылка\",\"results\":[{\"id\":\"std702\",\"type\":\"standard\",\"title\":\"Реквизит Ссылка #std702\",\"description\":\"Через команду Еще.\",\"url\":\"https://v8std.ru/std/702/\",\"score\":1.0}]}"}]}}"#;

#[cfg(target_os = "linux")]
/// Переменные машины, которые изменили бы маршрут или хранилище в дочернем
/// процессе.
const INHERITED: &[&str] = &[
    "https_proxy",
    "HTTPS_PROXY",
    "http_proxy",
    "HTTP_PROXY",
    "all_proxy",
    "ALL_PROXY",
    "no_proxy",
    "NO_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

#[cfg(target_os = "linux")]
fn run_child(scenario: &str, address: &str, root: &TestRoot, extra: &[(&str, &str)]) {
    let scratch = tempfile::tempdir().expect("scratch");
    let trusted = scratch.path().join("roots.pem");
    root.write_pem(&trusted);
    let mut command = Command::new(std::env::current_exe().expect("test binary"));
    command.args(["--exact", CHILD, "--nocapture", "--test-threads=1"]);
    for name in INHERITED {
        command.env_remove(name);
    }
    command
        .env(SCENARIO, scenario)
        .env(ADDRESS, address)
        .env("SSL_CERT_FILE", &trusted);
    for (name, value) in extra {
        command.env(name, value);
    }
    let output = command.output().expect("run child test process");
    assert!(
        output.status.success(),
        "дочерний процесс: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn standards_answer_over_a_root_from_the_os_store() {
    let root = TestRoot::generate("Unica v8std trusted root");
    let stand = TlsStand::start(&root, STANDARDS_BODY, "application/json");

    run_child("v8std", &stand.url("mcp"), &root, &[]);

    assert_eq!(stand.requests(), vec!["POST /mcp HTTP/1.1"]);
}

#[cfg(target_os = "linux")]
#[test]
fn standards_answer_through_https_proxy() {
    let root = TestRoot::generate("Unica v8std proxy root");
    let stand = TlsStand::start(&root, STANDARDS_BODY, "application/json");
    let proxy = ConnectProxy::start();

    run_child(
        "v8std",
        &stand.url("mcp"),
        &root,
        &[("HTTPS_PROXY", &proxy.url())],
    );

    assert_eq!(proxy.targets(), vec![stand.authority()]);
    // Через HTTP-прокси `ureq` пишет адрес запроса в абсолютной форме даже
    // внутри туннеля CONNECT; сервер обязан её принимать (RFC 9112, 3.2.2).
    assert_eq!(
        stand.requests(),
        vec![format!("POST {} HTTP/1.1", stand.url("mcp"))]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn kb_reads_over_a_root_from_the_os_store_and_through_https_proxy() {
    let root = TestRoot::generate("Unica kb trusted root");
    let stand = TlsStand::start(&root, "[]", "application/json");
    run_child("kb", stand.url("").trim_end_matches('/'), &root, &[]);
    assert!(!stand.requests().is_empty(), "оглавление прочитано");

    let stand = TlsStand::start(&root, "[]", "application/json");
    let proxy = ConnectProxy::start();
    run_child(
        "kb",
        stand.url("").trim_end_matches('/'),
        &root,
        &[("HTTPS_PROXY", &proxy.url())],
    );
    assert!(!stand.requests().is_empty(), "оглавление прочитано");
    assert_eq!(proxy.targets(), vec![stand.authority()]);
}

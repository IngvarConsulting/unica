//! Доверие корню через хранилище ОС: Linux.
//!
//! На Linux системное хранилище корней читается из набора ОС, а
//! `SSL_CERT_FILE` его заменяет. Это тот же путь, которым администратор
//! подкладывает корень корпоративного шлюза, и его можно задать дочернему
//! процессу, не трогая машину. На macOS и Windows хранилище в тесте не
//! изменить без прав администратора: там проверяются отказ и маршрут
//! (`download.rs`).
//!
//! Переменная читается из окружения процесса, а под `cargo test` тесты идут
//! потоками одного процесса. Поэтому загрузка идёт в отдельном процессе этого
//! же тестового бинаря, а клиент там собирается штатно — из окружения.

use std::path::PathBuf;

use crate::{Downloader, Failure, HttpDownloader, SilentDownload};

const SCENARIO: &str = "UNICA_NETWORK_TRUST_SCENARIO";
const URL: &str = "UNICA_NETWORK_TRUST_URL";
const DESTINATION: &str = "UNICA_NETWORK_TRUST_DESTINATION";

/// Дочерний процесс: качает штатным загрузчиком из окружения процесса.
/// Без переменной сценария — не дочерний процесс, и делать ему нечего.
#[test]
fn network_trust_child_process() {
    let Ok(scenario) = std::env::var(SCENARIO) else {
        return;
    };
    let url = std::env::var(URL).expect("url");
    let destination = PathBuf::from(std::env::var_os(DESTINATION).expect("destination"));
    let result = HttpDownloader::default().download(&url, &destination, &SilentDownload);
    match scenario.as_str() {
        "download" => {
            result.expect("загрузка с доверенным корнем");
            println!(
                "trust={:?}",
                crate::network::NetworkClient::shared()
                    .expect("client")
                    .trust_source()
            );
        }
        "refuse" => {
            let error = result.expect_err("корень не доверен");
            assert_eq!(error.failure(), Failure::Network);
            println!("{}", error.diagnosis());
            println!(
                "trust={:?}",
                crate::network::NetworkClient::shared()
                    .expect("client")
                    .trust_source()
            );
        }
        other => panic!("неизвестный сценарий {other}"),
    }
}

// Родительские проверки ниже идут только на Linux: там хранилище корней
// задаётся через `SSL_CERT_FILE`. Дочерний тест выше собирается везде, чтобы
// отбор по размеру видел модуль на любой ОС.

#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::{Command, Output};

#[cfg(target_os = "linux")]
use crate::network::test_support::{ConnectProxy, TestRoot, TlsStand};

#[cfg(target_os = "linux")]
const CHILD: &str = "platform::network_trust_tests::network_trust_child_process";

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
struct Scratch(PathBuf);

#[cfg(target_os = "linux")]
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "unica-network-trust-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("scratch");
        Self(path)
    }
}

#[cfg(target_os = "linux")]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(target_os = "linux")]
fn run_child(
    scenario: &str,
    url: &str,
    destination: &Path,
    trusted_file: &Path,
    extra: &[(&str, &str)],
) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("test binary"));
    command.args(["--exact", CHILD, "--nocapture", "--test-threads=1"]);
    for name in INHERITED {
        command.env_remove(name);
    }
    command
        .env(SCENARIO, scenario)
        .env(URL, url)
        .env(DESTINATION, destination)
        .env("SSL_CERT_FILE", trusted_file);
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
    output
}

#[cfg(target_os = "linux")]
fn body() -> Vec<u8> {
    (0..4096).map(|index| (index % 251) as u8).collect()
}

#[cfg(target_os = "linux")]
#[test]
fn a_root_in_the_os_store_lets_the_download_through() {
    let scratch = Scratch::new("trusted");
    let root = TestRoot::generate("Unica trusted test root");
    let trusted = scratch.0.join("roots.pem");
    root.write_pem(&trusted);
    let stand = TlsStand::start(&root, body(), "application/octet-stream");
    let destination = scratch.0.join("artifact.tar.gz");

    run_child(
        "download",
        &stand.url("artifact.tar.gz"),
        &destination,
        &trusted,
        &[],
    );

    assert_eq!(std::fs::read(&destination).expect("downloaded"), body());
    assert_eq!(stand.requests(), vec!["GET /artifact.tar.gz HTTP/1.1"]);
}

#[cfg(target_os = "linux")]
#[test]
fn a_download_through_https_proxy_reaches_a_trusted_server() {
    let scratch = Scratch::new("proxied");
    let root = TestRoot::generate("Unica trusted proxy root");
    let trusted = scratch.0.join("roots.pem");
    root.write_pem(&trusted);
    let stand = TlsStand::start(&root, body(), "application/octet-stream");
    let proxy = ConnectProxy::start();
    let destination = scratch.0.join("artifact.tar.gz");

    run_child(
        "download",
        &stand.url("artifact.tar.gz"),
        &destination,
        &trusted,
        &[("HTTPS_PROXY", &proxy.url())],
    );

    assert_eq!(std::fs::read(&destination).expect("downloaded"), body());
    assert_eq!(proxy.targets(), vec![stand.authority()]);
    assert_eq!(
        stand.requests(),
        vec![format!("GET {} HTTP/1.1", stand.url("artifact.tar.gz"))],
        "запрос прошёл туннелем прокси до сервера"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_root_missing_from_the_os_store_is_refused_with_the_interception_cure() {
    let scratch = Scratch::new("untrusted");
    let served = TestRoot::generate("Unica gateway root");
    let installed = TestRoot::generate("Unica unrelated root");
    let trusted = scratch.0.join("roots.pem");
    installed.write_pem(&trusted);
    let stand = TlsStand::start(&served, body(), "application/octet-stream");
    let destination = scratch.0.join("artifact.tar.gz");

    let output = run_child(
        "refuse",
        &stand.url("artifact.tar.gz"),
        &destination,
        &trusted,
        &[],
    );

    let diagnosis = String::from_utf8_lossy(&output.stdout);
    for expected in ["UnknownIssuer", "intercepted", "trust store", "HTTPS_PROXY"] {
        assert!(diagnosis.contains(expected), "{expected}: {diagnosis}");
    }
    assert!(stand.requests().is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn a_non_empty_os_store_selects_the_os_branch() {
    // Корень стенда есть только в хранилище ОС, во вшитом наборе его нет:
    // загрузка проходит, и выбрана ветка ОС. Отсутствие смешивания с вшитым
    // набором держит устройство `tls_config` и тест `select_trust`.
    let scratch = Scratch::new("os-only");
    let root = TestRoot::generate("Unica OS-only root");
    let trusted = scratch.0.join("roots.pem");
    root.write_pem(&trusted);
    let stand = TlsStand::start(&root, body(), "application/octet-stream");

    let output = run_child(
        "download",
        &stand.url("artifact.tar.gz"),
        &scratch.0.join("artifact.tar.gz"),
        &trusted,
        &[],
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("trust=OperatingSystem"), "{stdout}");
}

#[cfg(target_os = "linux")]
#[test]
fn an_empty_os_store_falls_back_to_the_bundled_mozilla_roots() {
    // Хранилище ОС пусто: клиент собирается на вшитом наборе, а корень,
    // которого в наборе нет, по-прежнему отвергается с объяснением.
    let scratch = Scratch::new("empty-store");
    let empty = scratch.0.join("empty.pem");
    std::fs::write(&empty, "").expect("empty store");
    let root = TestRoot::generate("Unica root outside Mozilla");
    let stand = TlsStand::start(&root, body(), "application/octet-stream");

    let output = run_child(
        "refuse",
        &stand.url("artifact.tar.gz"),
        &scratch.0.join("artifact.tar.gz"),
        &empty,
        &[],
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("trust=BundledMozillaRoots"), "{stdout}");
    assert!(stdout.contains("bundled Mozilla roots"), "{stdout}");
    assert!(stdout.contains("UnknownIssuer"), "{stdout}");
}

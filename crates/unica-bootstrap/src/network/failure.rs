//! Почему не удался сетевой запрос и что с этим делать.

use std::error::Error as StdError;
use std::fmt;

use rustls::CertificateError;

/// Класс отказа. По нему загрузчик выбирает код выхода, а текст — совет.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkFailure {
    /// Запрос нельзя даже начать: неверная переменная прокси, адрес или пустое
    /// хранилище корней ОС.
    Configuration,
    /// ОС не доверяет цепочке сертификатов сервера. Чаще всего это подмена
    /// сертификата средством защиты или шлюзом.
    UntrustedCertificate,
    /// Цепочка построена, но сертификат отвергнут: просрочен, выдан другому
    /// имени, отозван или не предназначен для сервера.
    RejectedCertificate,
    /// Редирект увёл с HTTPS на HTTP.
    InsecureRedirect,
    /// Сервер ответил кодом `4xx`/`5xx`.
    Status(u16),
    /// Всё прочее на пути: имя, соединение, обрыв, отказ прокси, редирект.
    Transport,
}

/// Отказ сетевого запроса: что произошло и, если есть, что исправить.
#[derive(Debug)]
pub struct NetworkError {
    failure: NetworkFailure,
    message: String,
    cure: Option<String>,
}

/// Совет одной строкой: параметры читаются при старте процесса.
const RESTART_NOTE: &str = "Unica reads proxy variables and the trust store when its process starts: restart the host session (and the Unica daemon, if it is running) after changing them";

impl NetworkError {
    pub(crate) fn new(failure: NetworkFailure, message: impl Into<String>) -> Self {
        Self {
            failure,
            message: message.into(),
            cure: None,
        }
    }

    pub(crate) fn with_cure(mut self, cure: impl Into<String>) -> Self {
        self.cure = Some(cure.into());
        self
    }

    pub fn failure(&self) -> NetworkFailure {
        self.failure
    }

    /// Что произошло, с адресом и маршрутом.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Что исправить, если отказ это знает.
    pub fn cure(&self) -> Option<&str> {
        self.cure.as_deref()
    }

    pub fn status(&self) -> Option<u16> {
        match self.failure {
            NetworkFailure::Status(code) => Some(code),
            _ => None,
        }
    }

    /// Отказ из-за переменной прокси.
    pub(crate) fn proxy_setting(message: String) -> Self {
        Self::new(
            NetworkFailure::Configuration,
            format!("invalid proxy setting {message}"),
        )
        .with_cure(format!(
            "set the variable to an HTTP proxy such as http://proxy.example:3128, or unset it to connect directly. {RESTART_NOTE}"
        ))
    }

    /// Не удалось собрать TLS-клиент поверх хранилища ОС.
    pub(crate) fn trust_store(error: &rustls::Error) -> Self {
        Self::new(
            NetworkFailure::Configuration,
            format!("cannot verify TLS certificates with the operating system trust store: {error}"),
        )
        .with_cure(format!(
            "install the system CA certificates (for example the ca-certificates package on Linux) or point SSL_CERT_FILE or SSL_CERT_DIR to a CA bundle. {RESTART_NOTE}"
        ))
    }

    /// Разобрать отказ `ureq`. `host` и `route` называют, куда и как шли.
    pub(crate) fn from_transport(error: ureq::Error, host: &str, route: &str) -> Self {
        match error {
            ureq::Error::Status(code, response) => Self::new(
                NetworkFailure::Status(code),
                format!(
                    "{} answered HTTP {code} {} ({route})",
                    response.get_url(),
                    response.status_text()
                ),
            ),
            error => match find_tls_error(&error) {
                Some(rustls::Error::InvalidCertificate(reason)) => {
                    certificate_refusal(reason, &error.to_string(), host, route)
                }
                _ => Self::new(
                    NetworkFailure::Transport,
                    format!("request to {host} failed ({route}): {error}"),
                ),
            },
        }
    }
}

fn certificate_refusal(
    reason: &CertificateError,
    error: &str,
    host: &str,
    route: &str,
) -> NetworkError {
    match reason {
        // `Other` — так macOS и Windows сообщают о недоверенном корне, когда
        // их код не сводится к `UnknownIssuer`.
        CertificateError::UnknownIssuer
        | CertificateError::BadSignature
        | CertificateError::Other(_) => NetworkError::new(
            NetworkFailure::UntrustedCertificate,
            format!(
                "TLS connection to {host} failed ({route}): the operating system does not trust the certificate chain the server presented: {error}"
            ),
        )
        .with_cure(format!(
            "most often the TLS connection is intercepted: security software that scans HTTPS (an antivirus) or a corporate gateway presents its own certificate, and its root is not trusted by this operating system. Add {host} to the HTTPS-scanning exclusions, install the gateway root into the operating system trust store, or route the traffic through the corporate proxy with HTTPS_PROXY (NO_PROXY lists the exceptions). {RESTART_NOTE}"
        )),
        CertificateError::Expired
        | CertificateError::ExpiredContext { .. }
        | CertificateError::NotValidYet
        | CertificateError::NotValidYetContext { .. } => NetworkError::new(
            NetworkFailure::RejectedCertificate,
            format!("TLS connection to {host} failed ({route}): the certificate is outside its validity period: {error}"),
        )
        .with_cure("check the system date, time and time zone"),
        _ => NetworkError::new(
            NetworkFailure::RejectedCertificate,
            format!("TLS connection to {host} failed ({route}): the certificate was rejected: {error}"),
        )
        .with_cure(format!(
            "if the certificate belongs to another name or purpose, the connection may be intercepted by a gateway: check the proxy settings (HTTPS_PROXY, NO_PROXY) and the HTTPS-scanning exclusions for {host}"
        )),
    }
}

/// Найти ошибку rustls в цепочке источников. `ureq` заворачивает её в
/// `io::Error`, а `io::Error::source` отдаёт не сам вложенный объект, а его
/// источник, поэтому вложенный объект достаём через `get_ref`.
fn find_tls_error<'a>(error: &'a (dyn StdError + 'static)) -> Option<&'a rustls::Error> {
    let mut current = Some(error);
    while let Some(item) = current {
        if let Some(tls) = item.downcast_ref::<rustls::Error>() {
            return Some(tls);
        }
        if let Some(io) = item.downcast_ref::<std::io::Error>() {
            if let Some(tls) = io
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>())
            {
                return Some(tls);
            }
        }
        current = item.source();
    }
    None
}

impl fmt::Display for NetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(cure) = &self.cure {
            write!(formatter, ". What to do: {cure}")?;
        }
        Ok(())
    }
}

impl StdError for NetworkError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(reason: CertificateError) -> NetworkError {
        certificate_refusal(&reason, "invalid peer certificate", "github.com", "direct")
    }

    #[test]
    fn the_tls_error_is_found_inside_the_io_error_ureq_wraps_it_in() {
        // Так `ureq` отдаёт сбой рукопожатия: `io::Error` с ошибкой rustls внутри.
        let io = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer),
        );
        assert!(matches!(
            find_tls_error(&io),
            Some(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer
            ))
        ));
        assert!(find_tls_error(&std::io::Error::other("reset")).is_none());
    }

    #[test]
    fn an_unknown_issuer_names_interception_and_its_cures() {
        let error = classify(CertificateError::UnknownIssuer);
        assert_eq!(error.failure(), NetworkFailure::UntrustedCertificate);
        let cure = error.cure().expect("cure");
        for expected in ["intercepted", "github.com", "trust store", "HTTPS_PROXY"] {
            assert!(cure.contains(expected), "{expected}: {cure}");
        }
    }

    #[test]
    fn an_os_specific_untrusted_root_is_also_untrusted() {
        let error = classify(CertificateError::Other(rustls::OtherError(
            std::sync::Arc::new(std::io::Error::other("errSecNotTrusted")),
        )));
        assert_eq!(error.failure(), NetworkFailure::UntrustedCertificate);
    }

    #[test]
    fn an_expired_certificate_points_at_the_clock_not_the_antivirus() {
        let error = classify(CertificateError::Expired);
        assert_eq!(error.failure(), NetworkFailure::RejectedCertificate);
        let cure = error.cure().expect("cure");
        assert!(cure.contains("date"), "{cure}");
        assert!(!cure.contains("antivirus"), "{cure}");
    }

    #[test]
    fn a_wrong_name_or_revocation_is_rejected_not_untrusted() {
        for reason in [CertificateError::NotValidForName, CertificateError::Revoked] {
            assert_eq!(
                classify(reason).failure(),
                NetworkFailure::RejectedCertificate
            );
        }
    }
}

//! Почему не удался сетевой запрос и что с этим делать.

use std::error::Error as StdError;
use std::fmt;

use rustls::CertificateError;

/// Класс отказа. По нему загрузчик выбирает код выхода, а текст — совет.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkFailure {
    /// Запрос нельзя даже начать: неверная переменная прокси, адрес или
    /// не собранная проверка сертификатов.
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

/// Почему проверку вёл вшитый набор.
const BUNDLED_NOTE: &str = "the OS trust store yielded no certificates (none installed, or SSL_CERT_FILE/SSL_CERT_DIR could not be read), so Unica checked the chain against its bundled Mozilla roots";

/// Совет одной строкой: параметры читаются при старте процесса.
const RESTART_NOTE: &str = "proxy variables and the trust store are read when a Unica process starts: close every agent session that uses Unica and wait until its background process exits after 15 minutes without work, or end the background unica process, then start a new session";

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
            "the TLS certificate verifier could not be built; check the operating system trust store settings. {RESTART_NOTE}"
        ))
    }

    pub(crate) fn timed_out(host: &str, route: &str) -> Self {
        Self::new(
            NetworkFailure::Transport,
            format!("request to {host} timed out ({route})"),
        )
    }

    /// Use the backend's typed cause without exposing diagnostic URL or headers.
    pub(crate) fn from_transport(
        error: &reqwest::Error,
        host: &str,
        route: &str,
        trust: super::TrustSource,
    ) -> Self {
        if let Some(rustls::Error::InvalidCertificate(reason)) = find_tls_error(error) {
            return certificate_refusal(
                reason,
                "certificate verification failed",
                host,
                route,
                trust,
            );
        }
        if error.is_timeout() {
            return Self::timed_out(host, route);
        }
        let reason = if error.is_connect() {
            "could not connect"
        } else if error.is_body() || error.is_decode() {
            "response body could not be read completely"
        } else {
            "HTTP transport failed"
        };
        Self::new(
            NetworkFailure::Transport,
            format!("request to {host} failed ({route}): {reason}"),
        )
    }
}

fn certificate_refusal(
    reason: &CertificateError,
    error: &str,
    host: &str,
    route: &str,
    trust: super::TrustSource,
) -> NetworkError {
    let refusal = certificate_refusal_by_os(reason, error, host, route);
    match (trust, refusal.failure) {
        // Проверку вёл вшитый набор: корень шлюза в хранилище ОС пока
        // не поможет, и отказ говорит об этом прямо.
        (super::TrustSource::BundledMozillaRoots, NetworkFailure::UntrustedCertificate) => {
            NetworkError::new(
                NetworkFailure::UntrustedCertificate,
                format!(
                    "TLS connection to {host} failed ({route}): the certificate chain is not issued by any bundled Mozilla root: {error}"
                ),
            )
            .with_cure(format!(
                "{BUNDLED_NOTE}. If HTTPS is intercepted by security software or a corporate gateway, install the system CA certificates together with the gateway root, or point SSL_CERT_FILE to a readable bundle that contains it. {RESTART_NOTE}"
            ))
        }
        _ => refusal,
    }
}

/// Почему верификатор отверг цепочку, когда ему доверяет ОС.
fn certificate_refusal_by_os(
    reason: &CertificateError,
    error: &str,
    host: &str,
    route: &str,
) -> NetworkError {
    match apple_reason(reason).unwrap_or(reason) {
        // `Other` — так macOS и Windows сообщают о недоверенном корне, когда
        // их код не сводится к `UnknownIssuer` (на macOS это
        // `errSecNotTrusted`, -67843).
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

/// Верификатор macOS сводит к своим вариантам только четыре кода, а
/// остальные отдаёт как `Other` с кодом `OSStatus` в тексте. Просроченный
/// сертификат и неверное назначение среди них, и совет про антивирус для
/// них ложен. Коды взяты из `SecBase.h`.
fn apple_reason(reason: &CertificateError) -> Option<&'static CertificateError> {
    static EXPIRED: CertificateError = CertificateError::Expired;
    static NOT_VALID_YET: CertificateError = CertificateError::NotValidYet;
    static INVALID_PURPOSE: CertificateError = CertificateError::InvalidPurpose;
    let CertificateError::Other(other) = reason else {
        return None;
    };
    let text = other.to_string();
    if text.ends_with(": -67818") {
        Some(&EXPIRED)
    } else if text.ends_with(": -67819") {
        Some(&NOT_VALID_YET)
    } else if text.ends_with(": -67609") || text == "certificate had invalid extensions" {
        // errSecInvalidExtendedKeyUsage и его прямое сопоставление (`EkuError`).
        Some(&INVALID_PURPOSE)
    } else {
        None
    }
}

/// Найти ошибку rustls в цепочке источников. Транспорт заворачивает её в
/// `io::Error`, а `io::Error::source` отдаёт не сам вложенный объект, а его
/// источник, поэтому вложенный объект достаём через `get_ref`.
fn find_tls_error<'a>(error: &'a (dyn StdError + 'static)) -> Option<&'a rustls::Error> {
    if let Some(tls) = error.downcast_ref::<rustls::Error>() {
        return Some(tls);
    }
    if let Some(inner) = error
        .downcast_ref::<std::io::Error>()
        .and_then(|io| io.get_ref())
    {
        return find_tls_error(inner);
    }
    error.source().and_then(find_tls_error)
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
        certificate_refusal(
            &reason,
            "invalid peer certificate",
            "github.com",
            "direct",
            crate::TrustSource::OperatingSystem,
        )
    }

    #[test]
    fn the_tls_error_is_found_inside_the_io_error_ureq_wraps_it_in() {
        // Транспорт отдаёт сбой рукопожатия как: `io::Error` с ошибкой rustls внутри.
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

    /// rule: INV.PKG.NETWORK-TRUST-AND-PROXY
    #[test]
    fn an_expired_certificate_points_at_the_clock_not_the_antivirus() {
        let error = classify(CertificateError::Expired);
        assert_eq!(error.failure(), NetworkFailure::RejectedCertificate);
        let cure = error.cure().expect("cure");
        assert!(cure.contains("date"), "{cure}");
        assert!(!cure.contains("antivirus"), "{cure}");
    }

    fn apple_other(text: &str) -> CertificateError {
        CertificateError::Other(rustls::OtherError(std::sync::Arc::new(
            std::io::Error::other(text.to_owned()),
        )))
    }

    /// rule: INV.PKG.NETWORK-TRUST-AND-PROXY
    #[test]
    fn macos_codes_for_expiry_and_purpose_are_not_read_as_interception() {
        // Так верификатор macOS отдаёт коды, которые не сводит к своим вариантам.
        let expired = classify(apple_other("“github.com” has expired: -67818"));
        assert_eq!(expired.failure(), NetworkFailure::RejectedCertificate);
        assert!(expired.cure().expect("cure").contains("date"));
        let purpose = classify(apple_other("certificate had invalid extensions"));
        assert_eq!(purpose.failure(), NetworkFailure::RejectedCertificate);
        let untrusted = classify(apple_other("“Root” certificate is not trusted: -67843"));
        assert_eq!(untrusted.failure(), NetworkFailure::UntrustedCertificate);
    }

    /// rule: INV.PKG.NETWORK-TRUST-AND-PROXY
    #[test]
    fn with_bundled_roots_an_unknown_issuer_names_the_empty_os_store() {
        let error = certificate_refusal(
            &CertificateError::UnknownIssuer,
            "invalid peer certificate",
            "github.com",
            "direct",
            crate::TrustSource::BundledMozillaRoots,
        );
        assert_eq!(error.failure(), NetworkFailure::UntrustedCertificate);
        assert!(!error.message().contains("operating system does not trust"));
        let cure = error.cure().expect("cure");
        assert!(cure.contains("bundled Mozilla roots"), "{cure}");
        assert!(cure.contains("SSL_CERT_FILE"), "{cure}");
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

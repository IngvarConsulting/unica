---
id: INV.PKG.NETWORK-TRUST-AND-PROXY
check:
  - crates/unica-bootstrap/src/platform/network_trust_tests.rs::a_root_in_the_os_store_lets_the_download_through
  - crates/unica-bootstrap/src/platform/network_trust_tests.rs::a_download_through_https_proxy_reaches_a_trusted_server
  - crates/unica-bootstrap/src/platform/network_trust_tests.rs::a_root_missing_from_the_os_store_is_refused_with_the_interception_cure
  - crates/unica-bootstrap/src/platform/network_trust_tests.rs::a_non_empty_os_store_selects_the_os_branch
  - crates/unica-bootstrap/src/platform/network_trust_tests.rs::an_empty_os_store_falls_back_to_the_bundled_mozilla_roots
  - crates/unica-bootstrap/src/network/mod.rs::an_empty_os_store_falls_back_to_bundled_roots_and_nothing_else_does
  - crates/unica-bootstrap/src/network/mod.rs::the_fallback_trusts_exactly_the_bundled_mozilla_roots
  - crates/unica-bootstrap/src/network/failure.rs::with_bundled_roots_an_unknown_issuer_names_the_empty_os_store
  - crates/unica-bootstrap/src/download.rs::a_root_the_os_does_not_trust_is_refused_with_the_interception_cure
  - crates/unica-bootstrap/src/network/failure.rs::an_expired_certificate_points_at_the_clock_not_the_antivirus
  - crates/unica-bootstrap/src/network/failure.rs::macos_codes_for_expiry_and_purpose_are_not_read_as_interception
  - crates/unica-bootstrap/src/network/mod.rs::a_redirect_from_https_to_plain_http_is_refused_at_any_step
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::an_untrusted_certificate_reaches_the_caller_as_its_own_class_with_the_cure
  - crates/unica-coder/src/infrastructure/platform/network_trust_tests.rs::standards_answer_over_a_root_from_the_os_store
  - crates/unica-coder/src/infrastructure/platform/network_trust_tests.rs::kb_reads_over_a_root_from_the_os_store_and_through_https_proxy
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_over_a_root_the_os_does_not_trust_names_interception
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_over_a_root_the_os_does_not_trust_names_interception
---

# Сетевые запросы используют общее доверие TLS

Загрузка ядра, доставка движков и сетевые поставщики `unica.docs`
(`kb-1ci`, `v8std`) используют один HTTP-клиент процесса. Настройки TLS
и [прокси](network-proxy-routing.md) задаются только в нём.
Окружение и хранилище считываются при первом успешном создании общего
клиента. Неуспешное создание не запоминается: следующий запрос повторяет
его с действующими настройками. Успешный клиент сохраняет конфигурацию
для всего процесса, общего для сессий; новые значения применяются после
его завершения и повторного запуска.

Цепочку сертификатов проверяет ОС; корень корпоративного шлюза из её
хранилища принимается. На Linux используется системный набор корней,
который заменяют `SSL_CERT_FILE` и `SSL_CERT_DIR`; отзыв сертификатов
на Linux не проверяется.

Только если на Linux не загружен ни один корень ОС, включая нечитаемые
`SSL_CERT_FILE`/`SSL_CERT_DIR`, используется вшитый набор Mozilla
(`webpki-roots`). Наборы не смешиваются: при непустом хранилище вшитые
корни не используются, а пустое не даёт доверия корню шлюза. Любая другая
ошибка сборки проверки ОС даёт отказ. На macOS и Windows этого перехода
нет: верификатор ОС хранилище заранее не читает. Отказ при вшитом наборе
называет его и причину выбора. Переход с HTTPS на HTTP запрещён
на любом шаге редиректа.

Недоверенная цепочка даёт отказ с вероятной причиной перехвата TLS
и советом: исключение для узла, корень в хранилище ОС или прокси.
Доставка движка передаёт отдельный класс `untrusted-certificate`
с тем же советом, без адреса узла. Просроченный сертификат даёт совет
проверить часы.

Успех с корнем ОС и выбор вшитого набора при пустом хранилище проверяются
на Linux. На macOS и Windows хранилище в тестах не меняется: там проверяются
отказ на недоверенном корне и маршрут через прокси.

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
  - crates/unica-bootstrap/src/download.rs::https_proxy_carries_the_download_to_the_proxy
  - crates/unica-bootstrap/src/download.rs::no_proxy_sends_the_download_past_the_proxy
  - crates/unica-bootstrap/src/download.rs::an_unsupported_proxy_is_a_configuration_refusal_without_the_password
  - crates/unica-bootstrap/src/network/proxy.rs::https_proxy_wins_over_all_proxy_and_lowercase_over_uppercase
  - crates/unica-bootstrap/src/network/proxy.rs::no_proxy_matches_hosts_and_subdomains_but_not_lookalikes
  - crates/unica-bootstrap/src/network/failure.rs::an_expired_certificate_points_at_the_clock_not_the_antivirus
  - crates/unica-bootstrap/src/network/failure.rs::macos_codes_for_expiry_and_purpose_are_not_read_as_interception
  - crates/unica-bootstrap/src/network/mod.rs::each_redirect_step_chooses_its_own_route
  - crates/unica-bootstrap/src/network/mod.rs::a_redirect_from_https_to_plain_http_is_refused_at_any_step
  - crates/unica-coder/src/infrastructure/engine_delivery.rs::an_untrusted_certificate_reaches_the_caller_as_its_own_class_with_the_cure
  - crates/unica-coder/src/infrastructure/platform/network_trust_tests.rs::standards_answer_over_a_root_from_the_os_store
  - crates/unica-coder/src/infrastructure/platform/network_trust_tests.rs::standards_answer_through_https_proxy
  - crates/unica-coder/src/infrastructure/platform/network_trust_tests.rs::kb_reads_over_a_root_from_the_os_store_and_through_https_proxy
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_over_a_root_the_os_does_not_trust_names_interception
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_requests_go_through_https_proxy_and_bypass_it_by_no_proxy
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_over_a_root_the_os_does_not_trust_names_interception
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_requests_go_through_https_proxy_and_bypass_it_by_no_proxy
---

# Сетевые запросы доверяют хранилищу ОС и идут через прокси из окружения

Загрузка ядра, доставка движков и сетевые поставщики `unica.docs`
(`kb-1ci`, `v8std`) ходят в сеть через один HTTP-клиент процесса
(`crates/unica-bootstrap/src/network/`). Настройки TLS и прокси заданы
только в нём.

Цепочку сертификатов проверяет ОС. Корень, установленный в её хранилище,
например корень корпоративного шлюза, принимается. На Linux хранилищем
служит системный набор корней, а `SSL_CERT_FILE` и `SSL_CERT_DIR` его
заменяют. Отзыв сертификатов на Linux не проверяется.

Если на Linux из хранилища ОС не загружен ни один корень, например в образе
без `ca-certificates` или при нечитаемых `SSL_CERT_FILE`/`SSL_CERT_DIR`,
цепочка проверяется по вшитому набору корней Mozilla (`webpki-roots`), как
до перехода на хранилище ОС. Отказ сертификата в этом случае называет
вшитый набор и причину его выбора. На macOS и Windows такого перехода нет:
верификатор ОС хранилище заранее не читает. Наборы не смешиваются:
при непустом хранилище вшитый набор не используется, а при пустом корень
шлюза не принимается. Любая другая ошибка сборки проверки ОС даёт отказ,
а не переход на вшитый набор.

Адрес `https://` идёт через `https_proxy`, `HTTPS_PROXY`, затем `all_proxy`,
`ALL_PROXY`; адрес `http://` — через `http_proxy`, затем `all_proxy`,
`ALL_PROXY`. Узлы из `no_proxy` или `NO_PROXY` и их поддомены соединяются
напрямую. Маршрут выбирается заново на каждом шаге редиректа, а переход
с HTTPS на HTTP запрещён на любом шаге. Неподдерживаемая
схема или неразборчивое значение действующей переменной дают отказ с именем
переменной и без пароля, а не прямое соединение.

Если ОС не доверяет цепочке, отказ называет вероятную причину — перехват TLS
средством защиты или шлюзом — и что исправить: исключение для узла, корень
в хранилище ОС или прокси. Доставка движка передаёт это отдельным классом
отказа `untrusted-certificate` с тем же советом, без адреса узла.
Просроченный сертификат даёт совет проверить часы.

Переменные и хранилище читаются при старте процесса Unica; фоновый процесс
общий для сессий, поэтому новые значения действуют после его завершения.

Успешное соединение с корнем из хранилища ОС и выбор вшитого набора при
пустом хранилище проверяются на Linux. На macOS и Windows хранилище в тестах
не меняется: там проверяются отказ на недоверенном корне и маршрут через
прокси.

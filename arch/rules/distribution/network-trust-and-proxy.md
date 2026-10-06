---
id: INV.PKG.NETWORK-TRUST-AND-PROXY
check:
  - crates/unica-bootstrap/src/platform/network_trust_tests.rs::a_root_in_the_os_store_lets_the_download_through
  - crates/unica-bootstrap/src/platform/network_trust_tests.rs::a_download_through_https_proxy_reaches_a_trusted_server
  - crates/unica-bootstrap/src/platform/network_trust_tests.rs::a_root_missing_from_the_os_store_is_refused_with_the_interception_cure
  - crates/unica-bootstrap/src/download.rs::a_root_the_os_does_not_trust_is_refused_with_the_interception_cure
  - crates/unica-bootstrap/src/download.rs::https_proxy_carries_the_download_to_the_proxy
  - crates/unica-bootstrap/src/download.rs::no_proxy_sends_the_download_past_the_proxy
  - crates/unica-bootstrap/src/download.rs::an_unsupported_proxy_is_a_configuration_refusal_without_the_password
  - crates/unica-bootstrap/src/network/proxy.rs::https_proxy_wins_over_all_proxy_and_lowercase_over_uppercase
  - crates/unica-bootstrap/src/network/proxy.rs::no_proxy_matches_hosts_and_subdomains_but_not_lookalikes
  - crates/unica-bootstrap/src/network/failure.rs::an_expired_certificate_points_at_the_clock_not_the_antivirus
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
например корень корпоративного шлюза, принимается; своего набора корней
у Unica нет. На Linux хранилищем служит системный набор корней, а
`SSL_CERT_FILE` и `SSL_CERT_DIR` его заменяют. Отзыв сертификатов на Linux
не проверяется.

Адрес `https://` идёт через `https_proxy`, `HTTPS_PROXY`, затем `all_proxy`,
`ALL_PROXY`; адрес `http://` — через `http_proxy`, затем `all_proxy`,
`ALL_PROXY`. Узлы из `no_proxy` или `NO_PROXY` и их поддомены соединяются
напрямую. Маршрут выбирается заново на каждом шаге редиректа. Неподдерживаемая
схема или неразборчивое значение действующей переменной дают отказ с именем
переменной и без пароля, а не прямое соединение.

Если ОС не доверяет цепочке, отказ называет вероятную причину — перехват TLS
средством защиты или шлюзом — и что исправить: исключение для узла, корень
в хранилище ОС или прокси. Просроченный сертификат даёт совет проверить часы.

Успешное соединение с корнем из хранилища ОС проверяется на Linux. На macOS
и Windows хранилище в тестах не меняется: там проверяются отказ на
недоверенном корне и маршрут через прокси.

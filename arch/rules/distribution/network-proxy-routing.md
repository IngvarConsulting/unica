---
id: INV.PKG.NETWORK-PROXY-ROUTING
check:
  - crates/unica-bootstrap/src/download.rs::https_proxy_carries_the_download_to_the_proxy
  - crates/unica-bootstrap/src/download.rs::no_proxy_sends_the_download_past_the_proxy
  - crates/unica-bootstrap/src/download.rs::an_unsupported_proxy_is_a_configuration_refusal_without_the_password
  - crates/unica-bootstrap/src/network/proxy.rs::https_proxy_wins_over_all_proxy_and_lowercase_over_uppercase
  - crates/unica-bootstrap/src/network/proxy.rs::no_proxy_matches_hosts_and_subdomains_but_not_lookalikes
  - crates/unica-bootstrap/src/network/mod.rs::each_redirect_step_chooses_its_own_route
  - crates/unica-coder/src/infrastructure/platform/network_trust_tests.rs::standards_answer_through_https_proxy
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_requests_go_through_https_proxy_and_bypass_it_by_no_proxy
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_requests_go_through_https_proxy_and_bypass_it_by_no_proxy
---

# Сетевой запрос выбирает маршрут по настройкам прокси

Все потребители [общего HTTP-клиента](network-trust-and-proxy.md) используют
одни настройки прокси. Для `https://` приоритет имеют `https_proxy`,
`HTTPS_PROXY`, затем `all_proxy`, `ALL_PROXY`; для `http://` —
`http_proxy`, затем `all_proxy`, `ALL_PROXY`. `HTTP_PROXY` не используется.
Пустое значение равно отсутствию переменной.

`no_proxy`, а при его отсутствии `NO_PROXY`, отправляет перечисленные
узлы и их поддомены напрямую, без совпадения с похожим чужим именем.
Маршрут выбирается заново на каждом шаге редиректа. Граница доверия TLS
и запрет понижения HTTPS сохраняются по правилу общего клиента.

Неподдерживаемая схема или неразборчивое значение действующей переменной
дают отказ с её именем и без пароля; прямое соединение не подменяет
ошибочную настройку. Переменная меньшего приоритета не влияет на выбранную
настройку, а ошибка настройки другой схемы не мешает этому запросу.

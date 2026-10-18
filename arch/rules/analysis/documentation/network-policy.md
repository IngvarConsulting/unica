---
id: INV.APP.DOCUMENTATION-NETWORK-POLICY
check:
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::default_deny_denies_only_providers_without_their_own_allow
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::unparseable_file_is_a_hard_refusal
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::an_unreadable_file_is_a_refusal_not_a_silent_default
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::dangling_policy_config_links_fail_closed_as_present_files
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::an_unknown_provider_id_is_a_refusal_not_a_silent_skip
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::an_unknown_network_value_is_a_refusal
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::an_unknown_key_is_a_refusal
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::the_local_overlay_wins_per_key
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::machine_policy_applies_to_multiple_workspaces_and_project_overlays_win_per_key
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::explicit_machine_policy_failure_cannot_be_masked_by_project_allow
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::machine_policy_requires_an_absolute_nonempty_path
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::kb_endpoint_is_rejected_in_every_policy_layer
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::resolve_standards_endpoint_prefers_config_then_env_then_builtin
---

# Настройка сети не заменяет запрет разрешающим умолчанием

Общий файл политики задаётся абсолютным путём в `UNICA_DOCUMENTATION_POLICY_FILE`
в окружении демона. Поверх него по ключам накладываются `unica.toml`
и `unica.local.toml` проекта. Это нижний слой настроек, а не запрет,
который проект не может перекрыть. Без файлов сеть разрешена.
Явно заданный отсутствующий общий файл вызывает отказ. Присутствующий нечитаемый или неверный файл,
неизвестный ключ, поставщик или значение политики вызывают отказ.

Собственное `network` поставщика сильнее `network.default`: например,
при общем `deny` его явное `allow` разрешает сеть.

Адрес сервера стандартов выбирается из локального файла, затем основного,
затем общего файла политики, затем `UNICA_STANDARDS_MCP_URL`, затем встроенного значения.
`endpoint` у `kb-1ci` отвергается при разборе, а не игнорируется.
Это правило проверяет настройку; её исполнение описано отдельно.

Доверие сертификатам и прокси для разрешённых поставщиков описаны
в [правиле сетевого клиента](../../distribution/network-trust-and-proxy.md).

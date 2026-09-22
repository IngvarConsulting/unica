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
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::resolve_standards_endpoint_prefers_config_then_env_then_builtin
---

# Настройка сети не заменяет запрет разрешающим умолчанием

Политика читается из `unica.toml`; `unica.local.toml` перекрывает её по ключам.
Без файлов сеть разрешена. Присутствующий нечитаемый или неверный файл,
неизвестный ключ, поставщик или значение политики вызывают отказ.

Собственное `network` поставщика сильнее `network.default`: например,
при общем `deny` его явное `allow` разрешает сеть.

Адрес сервера стандартов выбирается из локального файла, затем основного,
затем `UNICA_STANDARDS_MCP_URL`, затем встроенного значения.
Это правило проверяет настройку; её исполнение описано отдельно.

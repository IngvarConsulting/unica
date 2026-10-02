---
id: INV.PKG.PREFETCH-DEVELOPMENT-REFUSAL
check:
  - crates/unica-bootstrap/tests/host/cli_contract.rs::a_development_checkout_has_nothing_to_prefetch_and_says_so
---

# Прогрев отказывает манифесту разработки

Команда `unica-bootstrap prefetch --plugin-root <путь>` отказывает манифесту
с `development: true`: он не содержит артефактов для установки.
Команда завершается кодом 78 — ошибка конфигурации — и указывает
в диагностике, что использован манифест разработки.

---
id: INV.APP.DOCUMENTATION-NETWORK-POLICY
check:
  - crates/unica-coder/src/infrastructure/documentation_policy.rs::default_deny_denies_only_providers_without_their_own_allow
---

# Сетевое разрешение задаётся отдельно для поставщика справки

При `network.default = deny` поставщик с собственным `network = allow`
получает разрешение на сеть. Поставщик без собственного правила сохраняет
общий запрет.

Связанный тест проверяет вычисление разрешения из политики проекта.
Он не проверяет исполнение сетевого запрета каждым поставщиком.

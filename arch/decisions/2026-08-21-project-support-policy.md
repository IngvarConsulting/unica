---
id: DEC.2026-08-21.PROJECT-SUPPORT-POLICY
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/support_guard.rs::project_editing_policy_is_the_closed_support_guard_downgrade_source
supersedes: []
superseded-by: null
establishes: [INV.SAFETY.SUPPORT-POLICY-DOWNGRADE]
---

# Ослабление защиты задаётся политикой проекта

**Решение.** Состояние поддержки объекта по-прежнему определяет наличие
блокировки, а ослабить реакцию с `deny` до `warn` или `off` может только
`editingAllowedCheck` ближайшего подходящего `.v8-project.json`; неизвестное,
отсутствующее или повреждённое значение закрывается как `deny`.

**Почему.** Политика проекта наблюдаема до планирования и одинакова для
предпросмотра и применения, тогда как изменение состояния поддержки меняет сам
объект и не является настройкой реакции клиента.

**Цена.** Это осознанно не прежняя формулировка v1, где источником ослабления
называлось состояние поддержки конфигурации.

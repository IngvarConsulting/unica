---
id: DEC.2026-08-26.RETAINED-APPLY-EFFECT-PUBLICATION-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_effect_result_contract_is_complete
supersedes: []
superseded-by: null
establishes: [INV.CACHE.RETAINED-APPLY-EFFECT-RESULT]
---

# Actor-owned apply публикует один типизированный effect receipt

**Решение.** Один actor-admitted prepared apply один раз потребляет
`PlannedApplyEffects`, сохраняет их stable first-occurrence order и выводит из
этих же событий один `CacheReport` внутри существующей закрытой retained
transaction.

После всех dry-run gates prepared subject становится `Projected` без записи.
После единственного успешного retained commit тот же subject становится
`Committed`; до успеха commit такая disposition недоступна. Любая ошибка
уничтожает prepared subject и не возвращает события или успешный cache report.

**Почему.** B1b planner и B2a transaction foundation уже владеют нужными
типизированными значениями, но без удерживаемого actor receipt terminal result
теряет причинную связь между планом и публикацией.

**Цена.** Result остаётся crate-private; daemon routing, уведомления и публичная
форма принадлежат последующим slices.

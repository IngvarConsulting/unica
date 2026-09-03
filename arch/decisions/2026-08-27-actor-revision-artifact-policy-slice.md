---
id: DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_artifact_policy_contract_is_complete
supersedes: []
superseded-by: null
establishes: [CTR.SOURCE.REVISION-ARTIFACT-PROFILE, INV.SOURCE.REVISION-PROJECTION-CAPTURE-EQUALITY]
design: docs/design/2026-08-27-actor-revision-artifact-policy-design.md
---

# Actor владеет единым профилем артефактов ревизии

**Решение.** Actor выдаёт одну неподделываемую authority, которая связывает
retained root, state scope и доказанные kind, format и profile выбранного source
set. Из неё одновременно строятся закрытый `RevisionArtifactPolicy` и scoped
revision service; raw production-конструкторов этих частей нет. Одна политика
определяет корпус initial ambient scan, retained scan, incremental reconciliation
и retained-apply projection. Legacy-конструктор сохраняет прежний v0.12 корпус.

Platform XML 8.3.27 format 2.20 включает содержимое известных XDTO, support,
template, help и form-item ресурсов. Прямая membership поставок `.cf` влияет
присутствием и именем без чтения payload. Неизвестный или presence-only staged
файл отклоняется до публикации.

Все три capture-фазы хешируют content общим bounded chunked механизмом с
одинаковым per-file/aggregate accounting и cancellation/deadline checkpoints.

Типизированные manifest kinds сохраняют старые значения directory/content и
добавляют presence. Алгоритм, record schema, wire shape, transaction participants,
порядок публикации и rollback не меняются.

**Почему.** Проектор и scanner не могут независимо определять ревизию: иначе
успешно спланированный postimage невоспроизводим при публикации или restart.

**Цена.** Новые Platform XML ресурсы обязаны войти в закрытый профиль отдельным
архитектурным изменением; локальная операция не может расширить корпус сама.

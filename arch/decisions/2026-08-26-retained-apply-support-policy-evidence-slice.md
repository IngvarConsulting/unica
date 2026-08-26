---
id: DEC.2026-08-26.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_support_policy_evidence_contract_is_complete
supersedes: []
superseded-by: null
establishes: [INV.APP.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE]
---

# Retained apply удерживает support-policy как read-only actor evidence

**Решение.** Apply admission удерживает ordered bounded цепочку fixed-name
`.v8-project.json`, выведенную только из workspace actor и admitted source root.
Для предшествующих кандидатов удерживаются identity retained parent и
доказательство отсутствия, а для выбранного regular file — retained identity и
exact bytes. Wrong-kind, unreadable, malformed, unknown и oversized policy дают
fail-closed `Deny` без глобального отказа admission; их evidence удерживает
достаточную для неизменности `Deny` категорию. Только exact regular bytes могут
дать `Warn` или `Off`.

Evidence проверяется под actor mutation lane до публикации, после dry-run
revision confirmation и в retained final gate после postimages. Ошибка до
публикации write-free, поздняя ошибка использует существующий reverse rollback.
Evidence ничего не публикует и не становится третьим transaction participant:
writers остаются ровно `Source + WorkspaceCache`.
Публичный wire-контракт не меняется.

**Почему.** Pure v0.13 planners не должны получать `WorkspaceContext`, path или
сырой policy, но обязаны сохранять V12 authorisation semantics и закрывать
policy TOCTOU вплоть до revision installation.

**Цена.** Policy больше 32 MiB не может авторизовать `Warn`/`Off` и трактуется
как `Deny`; source-map provenance и `SourceSetKind` остаются отдельным C0b/15D.

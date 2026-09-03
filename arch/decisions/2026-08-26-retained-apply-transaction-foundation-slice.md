---
id: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_transaction_foundation_contract_is_complete
supersedes: []
superseded-by: null
establishes: [INV.APP.RETAINED-APPLY-CLOSED-PARTICIPANTS, INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK, INV.CACHE.RETAINED-APPLY-DETERMINISTIC-ORDER, INV.SOURCE.RETAINED-APPLY-WRITE-FREE]
---

# Retained apply связывает source, cache и revision до публичного v0.13 cutover

**Решение.** Скрытый apply получает от одного workspace actor закрытые
`Source` и `WorkspaceCache` participants под одной writer authority. Cache
authority удерживает существующий root либо ближайшего существующего предка с
точной missing chain; свободный root или третий participant не принимается.
Exact `.build/unica` может быть потомком workspace-root source set, потому что
Source role не адресует ни один `.build` component, а revision manifest его
исключает; обратное вложение и совпадение logical roots запрещены.

Source postimages публикуются первыми, eager cache metadata следующими,
revision record затем и `state.json` последним. Revision candidate готовится
без записи, проверяется по временно опубликованному retained source и становится
видимым в памяти только после postimage и final actor/revision gates. Ошибка до
этой точки откатывает journal в обратном порядке вместе с batch-owned пустыми
каталогами; cleanup после успеха остаётся bounded diagnostic.

Apply admission, planning и dry run не создают cache tree, не пишут revision
record и не продвигают revision machine. Existing logical-read fence capability
и v0.12 routing не меняются.

**Почему.** Task 15B2a нужен самостоятельно проверяемый transaction foundation,
не активирующий широкое v0.13 wire/runtime решение до daemon integration.

**Цена.** Контур остаётся crate-private; B1b events/result projection и
публичный one-commit result принадлежат следующему slice.

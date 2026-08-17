# План: ядро #290 — единый dryRun-контракт мутаторов (v2.1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Цель:** предусловие волны v0.13 (план #517, фаза 2; объём v2.1 из журнала) —
нормативный контракт «preview валидирует как apply», единая форма `data` у
предпросмотра и применения, cache report в режиме dry-run, явный
`preview_unavailable`, структурная квитанция мутации; табличные тесты,
обкатанные на выживших мутаторах (`meta.edit`, `form.edit`, `code.patch`),
починка пустого preview `cf.init`, срез «structured mutation result» — это
одновременно фикс #534 (пустые `changes`/`artifacts` у meta).

**Ветка:** `feature/preview-contract-core` от main (НЕ от #546; конфликт с
PR #546 в typed_result.rs при ребейзе тривиален — там сняты арки
template/help).

## Собранная карта (сессия 18.08)

- Реестр стратегий уже есть (PR #458): `PreviewStrategy{PostImage,
  PlannedCommand}` + тотальность — [mod.rs:104-130](crates/unica-coder/src/application/mod.rs);
  тест `every_mutating_tool_declares_a_preview_strategy`
  (tool_contracts.rs:4920).
- Гейт «только вне предпросмотра» — typed_result.rs:106 (`if !dry_run`), в
  main ~13 операций (после мержа #546 — минус template-add/template-remove;
  help-add уходит целиком). Ядро НЕ чинит всех: список замораживается тестом
  как закрытый переходный (каждый пункт с судьбой: срезы волны #377/#380/#382,
  runtime за #465), чинится только `cf-init`.
- Конверт meta: `metadata_success` → `AdapterOutcome::ok(summary)` с пустыми
  changes/artifacts ([metadata.rs:200-273](crates/unica-coder/src/application/metadata.rs));
  пути файлов живут в infrastructure (TypedChildResourcePlan.file_mutations,
  descriptor/registration paths в publisher.rs). Протяжка: report (и preview
  соответственно) получает список затронутых путей (created/updated/removed,
  workspace-relative), конверт заполняет `changes`/`artifacts`; в preview —
  те же пути как план (формулировка «would create/update»).
- cf-init: typed-обработчик за гейтом — preview отвечает ok:true без data.
  Починка: preview строит тот же typed `data` (перечень создаваемых файлов
  скаффолда), без записи.
- Выжившие с честным preview уже сейчас: meta.edit/meta.add/meta.remove
  (data в preview есть — MetaMutationData), form.edit, code.patch, role.edit,
  xdto.edit (ADR-0071 effects). Их parity закрепляется табличным тестом.

## Task 1: ADR-0073 + план (commit checkpoint)
- [ ] `spec/decisions/0073-chestnyy-preview-mutatora.md` (accepted в PR):
  нормы: (1) PostImage: preview = та же валидация/разрешение целей, та же
  форма `data`, различие — execution state; general ok без data недопустим;
  (2) PlannedCommand: квитанция несёт команду; платформенный шаг без
  выводимого результата — `preview_unavailable`/fail-closed, молчаливый успех
  запрещён; (3) cache report preview: mode="dry-run", только projected events;
  (4) структурная квитанция: changes/artifacts называют затронутые пути в
  apply и план в preview; (5) закрытый переходный список gated-операций с
  судьбами, тест не даёт ему расти. Check: табличные тесты ядра.
- [ ] Индекс решений; этот план закоммичен.

## Task 2: срез structured mutation result + #534 — TDD
- [ ] Красный тест: `unica.meta.add` applied — `changes`/`artifacts` называют
  созданные файлы (descriptor, Ext/*, Configuration.xml); preview — те же
  пути как план; `unica.meta.edit`/`remove` аналогично.
- [ ] Протяжка путей из publisher.rs (plan/publish) в report/preview →
  metadata.rs конверт.
- [ ] Зелёно, fmt/clippy, commit.

## Task 3: parity-тесты выживших + freeze-список + cf.init — TDD
- [ ] Табличный тест parity: для {meta.edit, form.edit, code.patch, cf.init}
  preview и apply дают data одной формы (совпадающий набор верхних ключей,
  различие — state/receipt-поля); preview не пишет файлов; cache mode
  "dry-run"; повтор preview идемпотентен.
- [ ] Freeze-тест gated-списка: точный список операций за `!dry_run` ==
  константа из ADR; новый мутатор не может попасть в список.
- [ ] Починка cf-init: preview с typed data (план создаваемых файлов).
- [ ] Зелёно всеми таргетами, python ci/dev, commit, push, PR
  «feat(mutations): ядро dryRun-контракта (#290, ADR-0073)». PR НЕ закрывает
  #290 целиком? — v2.1: ядро и есть остаток задачи в v0.13 → Closes #290
  (применение к новым инструментам волны — внутри их срезов, это записано и
  в #517, и в комментарии #290).

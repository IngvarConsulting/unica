---
id: DEC.2026-09-08.DAEMON-V3-RETIREMENT
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/identity.rs::production_identity_is_the_frozen_v5_digest_and_the_only_daemon_protocol
supersedes: []
superseded-by: null
establishes: [INV.APP.V13-USEFUL-PARTIAL-MODES]
design: docs/design/2026-09-07-daemon-v5-production-cutover-design.md
---

# Протокол v3 снят: у daemon одна wire identity и один рантайм

**Решение.** В системе остаётся один daemon — протокола v5 с identity
`unica-daemon-jsonl-5`; клиент, цикл сервера, исполнитель и протокольные
типы v3 удалены, каталог состояния любой core identity — `daemon-p5-…`, а
вход `--daemon` поднимает только рантайм v5 и отвергает любую identity,
кроме production. Общий предметный сервис canonical v0.13 и
`InvocationRequest` — единственное, что протокол v3 оставил после себя.
Свидетельства реестра, которые держались на исполнителе v3, доказываются на
каноническом рантайме v5 и на живом рантайме v5 в потоке под теми же
именами; восстановление хранилища доказывают хранилища v5 — терминализация
без повторного domain-вызова. Кадры v3 живут дальше только как рукописные
байты пробы контракта ledger: рантайм v5 обязан отвергать их и впредь.

**Почему.** Production ходил по v5 с `DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER`
и доказан на трёх ОС, полным контуром и оценкой BSP; параллельный код v3
держал две топологии, два набора тестов и правила, чьи проверки жили на
снятом пути.

**Цена.** Идентичности, отличной от production, daemon больше не поднять —
разделение endpoint-ов по несовместимым identity доказывается только
отказом рантайма; обязательство «actor остаётся у идущей попытки» доказано
семантикой отмены v5 (просьба, не захват), а не fail-stop исполнителя v3.

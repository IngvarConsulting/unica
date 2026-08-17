- Date: `2026-08-17`
- Status: `approved`
- Decision: `ADR-0069`

# Фаза 1 v0.13 (MCP-протокол): от красного гейта к версиям по SDK

## Итог в одну строку

Гейт показал, что 2026-07-28 не говорит ни один заявленный хост; после трёх
ревизий постановки #490 фаза закрыта рамкой «версии обслуживаются по
документации SDK, гарантируются по матрице хостов» (ADR-0069, реализация в
main), поверхность переведена на schema-only baseline с −79% токенов, а
отложенная доставка больших чтений вынесена в транспортно-нейтральный
ADR-0070 (proposed) и от протокола не зависит.

## Хроника решений (16–17.08.2026)

1. **v1 (план фазы):** переход только на 2026-07-28; красный гейт → «ветка Б»
   — отдельный релизный поезд с внешним триггером.
2. **Гейт (17.08, утро):** красный. Claude Code 2.1.220/2.1.233 шлёт legacy
   `initialize` (максимум 2025-11-25), на `-32601` рвёт соединение,
   `server/discover` не пробует. Решение владельца (v2): два поддерживаемых
   протокола для любого клиента.
3. **v3 (прогон Codex):** матрица хендшейка на 38 сценариев (приложение Б)
   опровергла «Codex говорит 2026-07-28»: он всегда предлагает 2025-06-18 и
   принимает любую непустую строку выбора сервера, оставаясь в legacy
   lifecycle; ошибка на `initialize` для него фатальна.
4. **v4 (решение владельца, вечер):** exact-allowlist признан
   исследовательским ограничением. Рамка: обслуживается всё, что знает SDK;
   гарантируются `2025-06-18` (Codex — родная), `2025-11-25` (Claude Code),
   `2026-07-28` (эталонный клиент). Prompts и tasks выведены из вехи; Qwen —
   только research-цель.

## Эмпирическая база

### Гейт Claude Code (2.1.220 и npm latest 2.1.233 — идентичны)

Методика — логирующая stdio-заглушка 2026-07-28-only: `initialize`/`ping` →
`-32601`; `server/discover` → полный discovery result; `tools/list` и
`tools/call` требуют reserved `_meta`. Подключение через
`claude -p … --mcp-config … --strict-mcp-config`, свежий процесс на прогон,
JSONL-журнал каждого кадра.

Решающий wire-фрагмент (2.1.233, дословно):

```text
in : {"method":"initialize","params":{"protocolVersion":"2025-11-25",...},"id":0}
out: {"jsonrpc":"2.0","id":0,"error":{"code":-32601,"message":"Method not found: initialize"}}
in : <EOF — клиент закрыл поток; server/discover так и не отправлен>
```

Спека прямо разрешает discover как backward-compatibility probe — у
актуального Claude Code этой ветки нет вовсе. Дополнительно измерено:
модерн-поля в legacy-конвертах терпимы end-to-end; полный JSON Schema 2020-12
(`$defs`/`$ref`/`oneOf`/`const`) доходит до модели и даёт корректный вызов.

### Матрица хендшейка Codex (38 сценариев)

Сводка (полный отчёт зонда — приложение Б): ChatGPT desktop 26.810.52044 /
codex-cli 0.148.0-alpha.9 всегда открывается `initialize` с `2025-06-18`;
принимает **любую непустую строку** в `InitializeResult.protocolVersion` —
включая `2026-07-28`, будущую `2099-01-01` и `banana` — и продолжает legacy
lifecycle до реального `tools/call`; выбор `2026-07-28` не переключает его на
канонический 2026 (в `_meta` только `progressToken`; строгий сервер → цикл
`-32602`); ошибка на `initialize` (`-32601`/`-32022`) фатальна — fallback и
discover отсутствуют; feature flag `mcp_2026_07_28` не меняет ни кадра.

Следствия: терпимость к выбору сервера — свойство парсера, а не поддержка
версии; отвечать legacy-клиенту современной версией нельзя (гибрид или
мёртвый клиент); host claim выводится только из wire-журнала.

### Скан клиентских SDK экосистемы (по манифестам, 17.08.2026)

| Агент | SDK | Предложит | Примечание |
| --- | --- | --- | --- |
| gemini-cli | TS SDK 1.23.0 | `2025-06-18` | SUPPORTED без `2025-11-25`; клиент TS SDK жёстко валидирует выбор сервера — живёт только на эхе своей версии |
| qwen-code | TS SDK ^1.30.0 | `2025-11-25` | research-цель #336, официально не поддерживается |
| opencode (sst) | TS SDK 1.29.0 | `2025-11-25` | |
| goose (Block) | rmcp 3.0.0 | режим-зависимо | единственный на SDK с `2026-07-28`; Auto-режим клиента rmcp = discover-first |
| crush | не виден в go.mod | ? | только зонд |

Факт уровня экосистемы: в TypeScript SDK (npm latest 1.30.0) ревизии
`2026-07-28` нет вовсе — современную ветку ещё долго проверяет только
эталонный клиент. Клиентская валидация TS SDK строгая: выбор сервера вне
`SUPPORTED_PROTOCOL_VERSIONS` → исключение.

### Сверка rmcp 3.1.2 по исходникам

- `serve` принимает два opener-а: legacy `initialize` (с pre-init `ping`) и
  прямой первый запрос с полным modern `_meta`; частичный набор отклоняется
  до handler-а.
- Согласование версии применяется транспортным циклом поверх ответа
  handler-а: эхо предложенной версии, если она в
  `supported_protocol_versions()` (дефолт — `KNOWN_VERSIONS`, все пять
  ревизий), иначе — версия из `InitializeResult` handler-а. Отсюда пин
  `V_2025_11_25` и невозможность «контр-оффера» для версии из списка — отказ
  был бы единственной альтернативой, и она отвергнута: ошибка на `initialize`
  измеренно фатальна для Codex-класса.
- `resultType: "complete"` срезается для пиров старше `2026-07-28`
  автоматически; `ttlMs`/`cacheScope` смоделированы только у `DiscoverResult`.
- `server/discover` имеет дефолтную реализацию из `get_info()` — identity и
  instructions едины для обоих представлений без кода.
- Семантика смешанного случая, зафиксированная тестом (моё априорное
  прочтение было опровергнуто): запрос с полным modern `_meta` внутри
  legacy-сессии получает современно кодированный ответ, но сессию не
  переключает.

### Зонды необязательных примитивов

Resources: research #336 закрыт (записка
`2026-08-17-mcp-resources-portability-research.md`, PR #525) — fixed URI
переносимы как статичная справка, templates мертвы на Claude Code,
`list_changed` инертен на обоих транспортах, большие ресурсы читаются целиком.
Prompts: Claude Code исполняет только пользовательский `/mcp__*`, Codex
серверные prompts не запрашивает — выведены из вехи. Tasks: не поддерживает
ни один хост (непрошеный хендл у Claude Code молча теряется) — выведены из
вехи; отложенная доставка поэтому живёт в payload (ADR-0070).

## Что отгружено этой фазой

1. **Schema-only baseline** (PR #526): все описания сняты с wire-границы;
   `tools/list` 263 120 → 55 122 токена o200k_base (−79%); >2000 токенов
   остались только глубокие union `meta.edit`/`meta.add` — подтверждение
   фокуса #479 §2. Прежние тексты — история v0.12 и реестр исходника.
2. **Версии по SDK** (PR #528, ADR-0069 accepted): rmcp 3.1.2, ноль
   собственной политики версий, пин запасной версии, политика курсора,
   проводная матрица трёх гарантируемых версий, bootstrap-верификация обоих
   жизненных циклов, поправка ADR-0013, инвариант `INV-MCP-VERSION-TIERS`.
3. **ADR-0070 (proposed)** — отложенная доставка больших чтений: манифест с
   `suggestedSelections`, `ResultStore`, продолжение побайтно из снимка;
   задачная цель — срез большой роли ≈2 000 токенов вместо 39 000. Номер
   0069 занят протокольным решением, черновик отложенной доставки
   переименован.

## Остаток фазы

- Реализация #479 §3 по ADR-0070 (переводит его в accepted; разблокирует
  приёмку #379/#380/#381).
- Каталог фикстурных задач и метрика «токены на решение» (#479 §1,
  дизайн-часть).
- Гейт-монитор `scripts/dev/mcp-host-protocol-gate.py` (packaged Claude Code,
  packaged Codex — после квоты 20.08, эталонный клиент).
- Расширение python-смоука на современную ветку (bootstrap уже кроет обе).
- README/release notes: два уровня поддержки, tested builds.

## Приложение А: семантика смешанного случая (тест)

```text
initialize(2025-06-18) → initialized
tools/list {_meta: {protocolVersion: 2026-07-28, clientCapabilities: {}}}
  → result.resultType = "complete"        (современный ответ на сам запрос)
tools/list {}
  → result без resultType                 (сессия осталась legacy)
```

## Приложение Б: отчёт матрицы хендшейка Codex (дословно, зонд 17.08.2026)

Сырые JSONL-журналы содержат thread-метаданные Codex и публикуются только
после редактирования; отчёт ниже — без них.

```markdown
# Codex configured-stdio MCP handshake matrix

Date: 2026-08-17

Tested client:

- ChatGPT desktop bundled `codex-cli 0.148.0-alpha.9`
- configured stdio MCP server
- `mcp_2026_07_28`: both default `false` and explicit `--enable`
- verdict is based on captured JSON-RPC frames, not Codex process exit status

## Stable observations across all 38 valid scenarios

- Every Codex connection started with `initialize` and offered `2025-06-18`.
- No connection started direct-first.
- `server/discover` was never sent, including after `-32601`, `-32022`, or
  with the feature enabled.
- Enabling `mcp_2026_07_28` did not change any observed configured-stdio
  frame or verdict.

## InitializeResult protocolVersion matrix

Each PASS means `notifications/initialized`, `tools/list`, and one real
`tools/call` were observed.

| Server-selected value | Feature off, modern result | Feature on, modern result | Feature off, legacy result | Feature on, legacy result |
| --- | --- | --- | --- | --- |
| `2024-11-05` | PASS | PASS | PASS | PASS |
| `2025-03-26` | PASS | PASS | PASS | PASS |
| `2025-06-18` | PASS | PASS | PASS | PASS |
| `2025-11-25` | PASS | PASS | PASS | PASS |
| `2026-07-28` | PASS | PASS | PASS | PASS |
| `2099-01-01` | PASS | PASS | PASS | PASS |
| `banana` | PASS | PASS | not rerun | not rerun |

`modern result` includes 2026-only result bookkeeping such as `resultType`
and cache hints. `legacy result` omits it. Codex accepted both forms for
every dated string tested, including `2026-07-28` and the unknown future
value.

## Invalid or error InitializeResult matrix

Each FAIL means two `initialize` attempts, zero `tools/list`, zero
`tools/call`, and zero `server/discover`.

| Server response | Feature off | Feature on |
| --- | --- | --- |
| missing `protocolVersion` | FAIL | FAIL |
| `protocolVersion: null` | FAIL | FAIL |
| numeric `protocolVersion` | FAIL | FAIL |
| JSON-RPC `-32601` for `initialize` | FAIL | FAIL |
| JSON-RPC `-32022` for `initialize` | FAIL | FAIL |

Therefore Codex validates that `InitializeResult.protocolVersion` is a
non-null string, but the tested client does not validate that it is a
supported revision or that it matches the offered revision.

## Selected `2026-07-28` does not switch Codex to the 2026 lifecycle

When the server selected `2026-07-28`, Codex still sent:

1. `notifications/initialized`
2. `tools/list`
3. `tools/call`

The `tools/list` request `_meta` contained only `progressToken`. Neither
list nor call contained the required 2026 per-request keys:

- `io.modelcontextprotocol/protocolVersion`
- `io.modelcontextprotocol/clientCapabilities`

A strict 2026 server rejected `tools/list` with `-32602`. Codex restarted
the legacy connection once and repeated the same invalid request. This
happened with the feature both off and on. No real tool call was reached.

So Codex accepting an InitializeResult that says `2026-07-28` is only
parser/compatibility tolerance inside the legacy session lifecycle. It is
not canonical MCP 2026-07-28 support.

## Consequence for Unica #490

- Do not describe Codex as rejecting a server-selected `2026-07-28`; it
  accepts it.
- Do not describe that acceptance as 2026 support; Codex continues the
  pre-2026 lifecycle and fails a strict 2026 server.
- Codex itself never offered `2026-07-28` in this matrix; it always offered
  `2025-06-18`.
- Unica should choose `2025-11-25` for any accepted `initialize` flow and
  reserve `2026-07-28` for canonical direct-first requests with complete
  reserved `_meta`.
- Returning `2026-07-28` from legacy `initialize` would create a
  non-standard third compatibility mode. Codex tolerance is not a reason to
  add that mode to Unica's public contract.
```

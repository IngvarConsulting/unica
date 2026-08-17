- Date: `2026-08-17`
- Status: `draft`
- Decision: `none` — research-записка по #336; изменение публичного контракта Resources оформляется отдельным решением после подтверждения рекомендации

# Переносимость MCP Resources между Claude Code, Codex и Qwen Code (#336)

## Итог в одну строку

Fixed Resources переносимы как статичная необязательная справка (автономное
обнаружение и чтение работают на Claude Code по обоим транспортам), но
resource templates как примитив и динамический каталог (`list_changed`)
непереносимы, а большой ресурс читается целиком без экономии контекста —
поэтому рекомендация из трёх исходов постановки: **использовать только
фиксированные Resources**, малым статичным каталогом, не вынося из
`inputSchema` ничего invocation-critical.

## Методика

Fixture — логирующий MCP-сервер
[`scripts/dev/mcp-resources-probe-server.py`](../../scripts/dev/mcp-resources-probe-server.py):
stdio и Streamable HTTP (`FIXTURE_MODE=http`, `POST/GET /mcp`, SSE для
server→client уведомлений, `Mcp-Session-Id`), JSONL-журнал каждого кадра
(`FIXTURE_LOG`). Публикует:

- fixed: `unica://meta/contracts/Catalog` (application/json, маркер
  `CATALOG-MARKER-7A41`, кодовое слово АКВАМАРИН), `.../guide`
  (text/markdown, содержит текстовую подсказку «для каждого вида есть
  `unica://meta/contracts/<Kind>`»), `.../big` (242 744 байта, 512 маркеров
  `BIGMARK-93F1`);
- resource template `unica://meta/contracts/{kind}` — чтение любого
  template-URI отвечает генерированным контрактом с маркером
  `TEMPLATE-<kind>-MARKER-E1F0`;
- инструмент `unica_add_contract {kind}` — добавляет fixed-ресурс и шлёт
  `notifications/resources/list_changed`.

Самотест raw-клиентом пройден по обоим транспортам до хостовых прогонов
(list/read/templates/чтение template-URI/tool+`list_changed`, по SSE в HTTP).

Запуск сценариев (каждый — свежий процесс сервера и свой журнал):

```bash
claude -p "<промпт>" --mcp-config mcp.json --strict-mcp-config \
  --dangerously-skip-permissions --max-turns 8
# stdio mcp.json: {"mcpServers":{"res336":{"command":"python3",
#   "args":[".../mcp-resources-probe-server.py"],"env":{"FIXTURE_LOG":"<путь>.jsonl"}}}}
# http mcp.json:  {"mcpServers":{"res336":{"type":"http","url":"http://127.0.0.1:8907/mcp"}}}

/Applications/ChatGPT.app/Contents/Resources/codex exec --skip-git-repo-check \
  -c 'approval_policy="never"' \
  -c 'mcp_servers={"res336"={"command"="python3","args"=["...probe-server.py"],"env"={"FIXTURE_LOG"="..."}}}' "<промпт>"

npx -y @qwen-code/qwen-code@latest -p "<промпт>"   # + .qwen/settings.json c mcpServers
```

Сценарии: S1/H1 — автономное обнаружение («не зная URI, найди контракт вида
Catalog»); S2 — вид Document, которого нет в fixed-списке; S3/H3 — большой
ресурс, пересчёт маркеров; S4/S4b/H4 — `unica_add_contract` и требование
«заново получи список свежим вызовом листинга».

## Таблица совместимости (прогоны 2026-08-17, macOS 26.5.2)

| Возможность | Claude Code 2.1.220 stdio | Claude Code 2.1.220 Streamable HTTP | Codex 0.148.0-alpha.9 stdio | Qwen Code 0.21.13 |
| --- | --- | --- | --- | --- |
| `resources/list` при подключении | жадно, до хода модели | жадно | **не вызывается** (на connect только `tools/list`) | не измерено |
| Автономное обнаружение (list → read без URI) | работает: маркер и кодовое слово процитированы | работает | не измерено (usage limit) | не измерено |
| `resources/read` fixed URI | работает | работает | работает как model-visible операция (зонд матрицы 17.08) | не измерено |
| `resources/templates/list` | **не вызывается никогда** (включая сценарий, где template был нужен) | не вызывается | работает как model-visible операция (зонд 17.08) | не измерено |
| Чтение template-URI | работает по угаданному URI: модель вывела его из **содержимого** guide-ресурса, не из объявления template | аналогично | работает (зонд 17.08) | не измерено |
| Большой ресурс 242 КБ | прочитан целиком, 512/512 маркеров, без усечения | прочитан целиком, 512/512 | не измерено | не измерено |
| `notifications/resources/list_changed` | **инертен**: уведомление отправлено — повторного `resources/list` нет | **инертен при доказанной доставке**: клиент сам открыл GET SSE, уведомление доставлено (`sse-delivered`) — повторного list нет | не измерено | не измерено |
| Свежий листинг по явной просьбе модели | **не доходит до сервера**: модель отчиталась «сделала свежий листинг», но кадра нет — хост отдаёт кеш снимка подключения | то же | не измерено | не измерено |
| Транспорт | — | `MCP-Protocol-Version: 2025-11-25` на пост-init запросах; `Mcp-Session-Id` соблюдён; GET SSE открыт сразу после initialized | — | — |

Ограничения прогона: Codex — model-side сценарии заблокированы usage limit
аккаунта (до 2026-08-20 07:19; connect-поведение записано до отказа); Qwen —
headless требует настроенного auth до старта MCP-клиентов («No auth type is
selected»), MCP-кадров ноль; Qwen — только research-цель, официально не
поддерживается (решение владельца 17.08.2026).

## Вердикты

- **Claude Code — partially supported**: fixed URI — supported (автономное
  обнаружение, чтение, оба транспорта); resource templates — unsupported как
  примитив (объявление не читается; доступ только по URI-конвенции,
  выведенной из содержимого другого ресурса); динамический каталог —
  unsupported (`list_changed` игнорируется, листинг модели заморожен на
  снимке подключения даже при доставленном по SSE уведомлении).
- **Codex — partially supported**: fixed list/read и templates/list доступны
  как явные model-visible операции; жадного листинга нет; автономное
  обнаружение и динамика не доказаны.
- **Qwen Code — not measured**: auth-стена до старта MCP; по манифесту —
  TS SDK `^1.30.0` (класс поведения TS-стека), runtime-проверка отложена.

## Рекомендация для Unica

Исход постановки — **«использовать только фиксированные Resources»**, с
границами:

1. Малый статичный плоский каталог, состав неизменен в течение сессии:
   динамика невидима хостом (list_changed инертен, листинг кеширован).
2. Большие корпуса не публиковать: чтение всегда полное, 242 КБ входят в
   контекст целиком — Resources не экономят контекст (совпадает с допущением
   #479).
3. Resource templates контрактом не заявлять. Если нужен параметрический
   доступ — только как документированная в содержимом фиксированного
   «гида» URI-конвенция (измерено: модель строит URI из содержимого guide);
   это degraded-режим, а не контракт — предпочтительный канал остаётся
   инструментом.
4. Граница данных: всё invocation-critical (аргументы, required, enum,
   единицы) остаётся в `inputSchema`; в Resources — только необязательная
   справка, отсутствие которой не меняет корректность вызова (`#479` E).
5. Корректность `unica.*` от Resources не зависит ни при каком исходе.

## Остаток исследования

- Codex model-side: автономное обнаружение и list_changed — после
  2026-08-20 либо квотой самого Codex.
- Qwen Code runtime: нужен доступ (qwen.ai OAuth или OpenAI-совместимый
  ключ); те же четыре сценария.
- Packaged-путь плагина: после появления Resources в реализации Unica
  (#490 их не вводит).
- Вопрос 7 постановки (недоверенный workspace, OAuth): неприменим к
  текущему deployment (локальный stdio); становится актуален только с
  remote HTTP.

## Приложение: ключевые wire-трейсы

S4b, stdio (динамика; модели явно велено сделать свежий листинг):

```text
in  initialize / initialized / tools/list / resources/list   (3 ресурса, до хода модели)
in  tools/call unica_add_contract{kind=Report}
out notifications/resources/list_changed
<повторного resources/list нет; ответ модели: «в свежем списке Report отсутствует»>
```

H4, Streamable HTTP (то же с доказанной доставкой):

```text
in  initialize / initialized ; note sse-open (GET, Mcp-Session-Id, MCP-Protocol-Version)
in  tools/list / resources/list
in  tools/call unica_add_contract
out notifications/resources/list_changed  [sse-queued → sse-delivered]
<повторного resources/list нет>
```

S2, stdio (template-вид без fixed-ресурса):

```text
in  resources/list
in  resources/read unica://meta/contracts/guide      (модель ищет подсказку)
in  resources/read unica://meta/contracts/Document   (URI построен из содержимого guide)
<resources/templates/list не вызван ни разу ни в одном сценарии>
```

Codex C1, connect до usage-отказа:

```text
in  initialize (protocolVersion 2025-06-18) / notifications/initialized / tools/list
<resources/list на подключении отсутствует>
```

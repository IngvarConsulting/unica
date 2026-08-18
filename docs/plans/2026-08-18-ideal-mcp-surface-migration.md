# Ideal MCP Surface Migration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Привести публичную поверхность `unica` от 71 инструмента в трёх моделях обращения к восьми входам модели `docs/design/2026-08-18-ideal-mcp-surface-design.md`, гейтуя каждый шаг замером «токены на решение» (#479).

**Architecture:** Пять фаз. Фаза 0 строит измерительный каркас — без него ни один следующий шаг не принимается. Фаза 1 — механические срезы текущей поверхности, не зависящие от целевой модели (пер-инструментные схемы вместо союза аргументов, аннотации, компактная сериализация). Фазы 2–5 — прототип `view`/`find`, прототип `apply`, консолидация остальных входов, переключение — каждая аддитивна, гейтуется замером и получает собственный детальный план после прохождения гейта предыдущей. Этот документ полностью детализирует фазы 0–1 и фиксирует контракты, гейты и политику совместимости фаз 2–5.

**Tech Stack:** Rust 2021 (`crates/unica-coder`), rmcp 3.1.2, Python 3.12 (`/opt/homebrew/bin/python3.12` локально), `tiktoken` (o200k_base), unittest.

## Global Constraints

- Проза и документы — по-русски; код, идентификаторы и сообщения коммитов — по-английски.
- Локальные тесты гоняются только `/opt/homebrew/bin/python3.12 -m unittest` (единственный интерпретатор с полным зелёным `tests/ci` и `tests/dev`).
- Коммиты без GPG-подписи (`git -c commit.gpgsign=false commit …`) — принятый fallback этой среды.
- Проводные гарантии не трогаются ни одной фазой: версии протокола ровно `{2025-06-18, 2025-11-25, 2026-07-28}`, SEP-2549 поля на list-результатах современной ветки, пагинация `tools/list` по 25 (`crates/unica-coder/src/interfaces/mcp.rs`).
- `INV-MCP-SURFACE-SYNC`: после любого изменения поверхности — перегенерация `spec/architecture/tool-surface.md` (`python3 scripts/ci/generate-tool-surface.py`) и правка `tool-surface-review.json`; гайд — `spec/architecture/change-checklist.md`.
- `INV-MCP-NAMESPACE`: все новые публичные инструменты — в пространстве `unica.*`.
- Новые отдельные `*.validate`-инструменты не заводятся (действующее решение: валидация живёт в `dryRun` писателя); консолидация существующих в `check` — только фазой 4 со своим ADR.
- До фазы 5 все изменения аддитивны или совместимы: ни одно опубликованное имя инструмента и ни один обязательный аргумент не исчезает и не переименовывается.
- Приватные корпуса (вендорские дампы) в репозиторий не попадают: сырые замеры — в игнорируемом `docs-local/`, в Git — только санированные агрегаты; путь к внешнему корпусу передаётся переменной окружения.
- Каждая фаза заканчивается отдельным PR (или серией PR) на `main`; PR не стекуются на открытые головы.
- Найденный по дороге дефект, не внесённый текущим PR, уходит отдельным PR от `main` (RED → GREEN), а не чинится попутно.

## Дорожная карта и гейты

| Фаза | Что делает | Аддитивность | Гейт на выход |
| --- | --- | --- | --- |
| 0 | Измерительный каркас #479: wire-разрез, фикстурные задачи, baseline текущей поверхности | только новые файлы | каркас детерминирован (двойной прогон бит-в-бит), baseline снят и записан |
| 1 | Срезы текущей поверхности: пер-инструментные схемы, аннотации, компактная сериализация, эксперимент с описаниями | контракт аргументов сужается только до фактически читаемых | discovery-слой ≤ 35 000 токенов wire; ни один сценарий ведомости не потерял обязательного аргумента |
| 2 | Прототип `unica.view` + `unica.find` поверх существующих читателей; парсер адреса с правилом чередования | новые инструменты рядом со старыми | вывод адреса доказан для всех 184 веток карты; wire-стоимость read-задач каталога через `view` ≤ текущей; закрывает сценарии #548 |
| 3 | Прототип `unica.apply` + `can` в результатах `view`; триада ошибок; `rev`/`ifRev`; ADR на квитанцию без конверта | новый инструмент рядом со старыми | first-call success `apply` ≥ текущих писателей на задачах каталога; A/B «скелеты в can» против «только имена op» решён замером |
| 4 | `search`/`check`/`run`/`docs`: консолидация читателей кода, валидаторов, build/runtime, справки | новые входы — тонкие фасады над существующими обработчиками | паритет сценариев ведомости для каждого поглощаемого инструмента |
| 5 | Переключение: 73 скилла переписаны на 8 входов, старые 71 сняты, мажорный релиз | ломающая, единственная | полный каталог задач на новой поверхности ≤ 50 % wire-стоимости baseline фазы 0; зонды на живых Claude Code и Codex зелёные |

Фазы 2–5 получают собственные планы имплементации в `docs/plans/` после прохождения гейта предыдущей фазы — их детализация здесь была бы планированием поверх незамеренных развилок (правило scope-check скилла writing-plans). Контракты, которые эти планы обязаны соблюдать, зафиксированы в спеке и в разделе «Политика совместимости» ниже.

## Политика совместимости

- **Двойная поверхность в фазах 2–4.** Новые входы публикуются рядом со старыми; временный рост `tools/list` на ~2–3 тысячи токенов принимается (новые входы малы по построению).
- **#548 (`meta.list`) не реализуется отдельным инструментом.** Его сценарии — приёмочные для `unica.view` фазы 2; при старте фазы 2 в issue уходит комментарий со ссылкой на этот план. До тех пор issue остаётся открытым как реестр потребности.
- **Скиллы (73 шт.) не трогаются до фазы 5**, кроме упоминаний, которые становятся фактически неверными (тогда — точечная правка в PR той фазы, которая сделала их неверными).
- **ADR заводятся на:** грамматику адреса и границу «узел/данные» (фаза 2), контракт квитанции и триаду ошибок (фаза 3), консолидацию `check`/`run` (фаза 4), снятие 71 инструмента (фаза 5). Фазы 0–1 архитектурных контрактов не меняют — сужение аргументов до фактически читаемых есть исправление ведомости, а не новый контракт.
- **Донорский паритет:** если сужение схем меняет вывод, затрагивающий `donor-relations.json`, паритет обновляется по процедуре из памяти проекта (перезапись фикстуры с обоснованием в PR).

## Замеры: что и как меряется на каждом шаге

Метрика — из `docs/design/2026-08-17-token-cost-task-catalog.md`: **токены на решение** (o200k_base, wire-разрез: сериализованные кадры запросов и результатов плюс discovery-доля) и **first-call success** (эталонный вызов корректен с первой попытки). Wire-разрез детерминирован и живёт в репозитории; model-visible и first-call — прогоны с моделью вне CI, оформляются отчётом в `docs-local/` с санированным агрегатом в PR.

---

# Фаза 0 — измерительный каркас

### Task 1: Учёт токенов эпизода поверх stdio

**Files:**
- Create: `scripts/dev/measure-token-cost.py`
- Test: `tests/dev/test_token_cost_harness.py`

**Interfaces:**
- Produces: CLI `measure-token-cost.py --binary <path> --episode <file.json> [--tokenizer o200k_base|bytes] [--report <out.json>]`; модуль `measure_token_cost.run_episode(binary, episode) -> EpisodeReport` с полями `calls[] {tool, request_tokens, result_tokens, is_error}`, `discovery_tokens` (tools/list), `total_tokens`.
- Consumes: собранный бинарь `target/release/unica` (или путь из `--binary`).

Эпизод — JSON-список шагов `{"tool": "unica.project.map", "arguments": {…}}`; раннер сам делает `initialize` (2025-06-18) + `notifications/initialized` + `tools/list`, затем шаги по порядку; считает токены каждого сериализованного кадра.

- [ ] **Step 1: Написать падающий тест**

```python
# tests/dev/test_token_cost_harness.py
"""Deterministic wire-cut token accounting for the #479 metric.

The harness is a *report*, not a gate: it must be bit-for-bit reproducible
for the same binary and episode, and its accounting must equal the sum of
its parts, so surface changes are attributable to the frames they change.
"""
import json
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "measure-token-cost.py"
BINARY = REPO_ROOT / "target" / "release" / "unica"


@unittest.skipUnless(BINARY.is_file(), "release binary is not built")
class WireCutAccountingTests(unittest.TestCase):
    def run_harness(self, episode: list, tokenizer: str) -> dict:
        episode_file = REPO_ROOT / "tests" / "fixtures" / "token_cost" / "_tmp_episode.json"
        episode_file.parent.mkdir(parents=True, exist_ok=True)
        episode_file.write_text(json.dumps(episode), encoding="utf-8")
        try:
            out = subprocess.run(
                [sys.executable, str(SCRIPT), "--binary", str(BINARY),
                 "--episode", str(episode_file), "--tokenizer", tokenizer],
                capture_output=True, text=True, check=True)
        finally:
            episode_file.unlink()
        return json.loads(out.stdout)

    def test_total_is_the_sum_of_discovery_and_calls(self) -> None:
        episode = [{"tool": "unica.project.map", "arguments": {"cwd": str(REPO_ROOT)}}]
        report = self.run_harness(episode, tokenizer="bytes")
        self.assertEqual(len(report["calls"]), 1)
        self.assertEqual(
            report["total_tokens"],
            report["discovery_tokens"]
            + sum(c["request_tokens"] + c["result_tokens"] for c in report["calls"]))
        self.assertGreater(report["discovery_tokens"], 0)

    def test_two_runs_are_bit_identical(self) -> None:
        episode = [{"tool": "unica.project.map", "arguments": {"cwd": str(REPO_ROOT)}}]
        first = self.run_harness(episode, tokenizer="bytes")
        second = self.run_harness(episode, tokenizer="bytes")
        self.assertEqual(first, second)
```

- [ ] **Step 2: Прогнать тест, убедиться в падении**

Run: `/opt/homebrew/bin/python3.12 -m unittest tests.dev.test_token_cost_harness -v`
Expected: FAIL/ERROR — `measure-token-cost.py` не существует.

- [ ] **Step 3: Реализовать раннер**

`scripts/dev/measure-token-cost.py`: подпроцесс бинаря, JSON-RPC по строкам (по образцу `scripts/ci/smoke-unica-mcp.py` — переиспользовать его паттерн handshake, не импортируя приватности); `--tokenizer bytes` считает `len(frame.encode())` (для CI без зависимостей), `o200k_base` — через `tiktoken` с понятной ошибкой, если пакет не установлен. Отчёт — JSON в stdout, сортировка ключей, без временных меток (детерминизм).

- [ ] **Step 4: Прогнать тест, убедиться в зелёном**

Run: `/opt/homebrew/bin/python3.12 -m unittest tests.dev.test_token_cost_harness -v`
Expected: PASS (оба теста).

- [ ] **Step 5: Commit**

```bash
git add scripts/dev/measure-token-cost.py tests/dev/test_token_cost_harness.py
git -c commit.gpgsign=false commit -m "feat(dev): wire-cut token accounting harness for the #479 metric"
```

### Task 2: Фикстурные задачи каталога и baseline

**Files:**
- Create: `tests/fixtures/token_cost/tasks/*.json` (эпизоды каталога)
- Create: `tests/fixtures/token_cost/README.md`
- Modify: `scripts/dev/measure-token-cost.py` (режим `--suite <dir>`)
- Test: `tests/dev/test_token_cost_harness.py` (дописать)

**Interfaces:**
- Produces: `--suite tests/fixtures/token_cost/tasks --report <out>` — сводный отчёт по всем задачам; задачи, требующие внешний корпус, помечены `"requires": "UNICA_TOKEN_COST_CORPUS"` и пропускаются без этой переменной окружения с явной строкой `skipped` в отчёте (no silent caps).
- Consumes: Task 1.

Задачи — эпизоды из `2026-08-17-token-cost-task-catalog.md`, переведённые в последовательности вызовов текущей поверхности. В репозиторий кладутся те, что работают на малой фикстурной конфигурации (создание объекта, правка реквизита, чтение формы, валидация); задачи на большой роли/подсистеме/MXL получают `requires` и путь внутри внешнего корпуса относительным полем `cwdRelative`.

- [ ] **Step 1: Дописать падающий тест** — `test_suite_reports_every_task_and_names_skips`: прогнать `--suite`, проверить, что в отчёте ровно столько записей, сколько файлов задач, и что задачи с `requires` без переменной окружения имеют `"status": "skipped"`, а не отсутствуют.
- [ ] **Step 2: Убедиться в падении** (`--suite` не реализован).
- [ ] **Step 3: Реализовать `--suite` и авторизовать 8–12 задач** по каталогу; у каждой в JSON — поле `"goal"` со строкой критерия успеха из каталога (для будущего first-call прогона с моделью).
- [ ] **Step 4: Зелёный прогон** тем же интерпретатором.
- [ ] **Step 5: Снять baseline** текущего бинаря: `python3 scripts/dev/measure-token-cost.py --suite … --tokenizer o200k_base --report docs-local/token-cost/2026-08-18-baseline.json`; санированный агрегат (итоги по задачам, без путей машины) — в `tests/fixtures/token_cost/README.md` таблицей с датой и коммитом бинаря.
- [ ] **Step 6: Commit** — `feat(dev): token-cost task suite and current-surface baseline`.

---

# Фаза 1 — срезы текущей поверхности

### Task 3: Инвентарь фактически читаемых аргументов

**Files:**
- Create: `docs-local/surface/args-inventory.md` (рабочий, не в Git) и итог — в описании PR
- Test: нет (разведка)

Для каждого native-семейства собрать список аргументов, которые обработчик реально читает:

- [ ] **Step 1:** `rg -o '"(BodyLimit|BorrowMainAttribute|…)"' crates/unica-coder/src/infrastructure/native_operations/form.rs | sort -u` — по файлу на домен (`form.rs`, `dcs.rs`, `mxl.rs`, `role.rs`, `subsystem.rs`, `cf.rs`, `cfe.rs`, `meta.rs`, остальные по `ls crates/unica-coder/src/infrastructure/native_operations/`). Практичнее обратный ход: `rg -n 'args\.(get|remove)\(' <file>` и выписать строковые ключи.
- [ ] **Step 2:** Свести таблицу «операция → читаемые аргументы → обязательные (из `required_args`)»; сверить с колонкой «Селектор» ведомости `spec/architecture/tool-surface.md` — селекторные ветви (`sourceSet+metadataPath` XOR `FormPath` и т.п.) обязаны сохраниться целиком.
- [ ] **Step 3:** Зафиксировать инвентарь в PR-описании фазы (таблица), рабочий файл остаётся в `docs-local/`.

### Task 4: Пер-инструментные списки вместо fallback в союз

**Files:**
- Modify: `crates/unica-coder/src/application/tool_contracts.rs` (константа `NATIVE_XML_DSL_ARGS` на :213, диспетчер `native_args_for` на :2764)
- Test: там же, `#[cfg(test)]` рядом с существующим тестом на :7583

**Interfaces:**
- Produces: `native_args_for(operation)` возвращает узкий список для каждой операции; fallback-ветка `_ => NATIVE_XML_DSL_ARGS` удалена, невозможность распознать операцию — ошибка компиляции по невозможности (полный match) либо `unreachable!` с именем операции.
- Consumes: инвентарь Task 3.

Механизм уже существует (`EXTERNAL_INIT_ARGS`, `XDTO_INFO_ARGS`, `ROLE_EDIT_ARGS` и мост ADR-0049, добавляющий `sourceSet`/`metadataPath` селекторным читателям) — работа состоит в доопределении списков для остальных операций, по одному коммиту на домен.

- [ ] **Step 1: Падающий тест** — обобщить существующий тест «published < union»:

```rust
#[test]
fn no_native_tool_publishes_the_legacy_argument_union() {
    for spec in tools() {
        let ToolHandler::NativeOperation { operation, .. } = spec.handler else { continue };
        let schema = input_schema_for_tool(&spec);
        let published = schema["properties"].as_object().unwrap().len();
        assert!(
            published <= 24,
            "{} publishes {published} args; the ledger budget after the union \
             removal is 24 (selector branches plus the domain's own payload \
             arguments — adjust only with a ledger-backed justification)",
            spec.name
        );
    }
}
```

- [ ] **Step 2: Убедиться в падении** — `cargo test -p unica-coder no_native_tool_publishes_the_legacy_argument_union` падает на первом же form-инструменте (136 аргументов).
- [ ] **Step 3–N: По домену за коммит** — добавить `FORM_ARGS`, `DCS_ARGS`, `MXL_ARGS`, `ROLE_COMPILE_ARGS`, `SUBSYSTEM_ARGS`, `CF_ARGS`, `CFE_ARGS`, `INTERFACE_ARGS`, `SUPPORT_ARGS` (имена и состав — из инвентаря Task 3), расширяя match в `native_args_for`; после каждого домена — `cargo test -p unica-coder` и точечный прогон затронутых сценариев ведомости (`ls`-инвентарь тестов: `rg -l '<operation>' crates/unica-coder/src --type rust | rg test`).
- [ ] **Step N+1: Удалить fallback** `_ => NATIVE_XML_DSL_ARGS` и саму константу; полный `cargo test`.
- [ ] **Step N+2: Перегенерировать ведомость** — `cargo build --release && python3 scripts/ci/generate-tool-surface.py`; обновить счётчики в `tool-surface-review.json` («Публикуют больше 20 аргументов из общего списка: 0»); `/opt/homebrew/bin/python3.12 -m unittest tests.ci.test_architecture_sync_guard tests.ci.test_meta_surface_contract`.
- [ ] **Step N+3: Замер** — Task 2 suite до/после; discovery-доля и полные определения в отчёт PR.
- [ ] **Step N+4: Commit + PR** — `feat(mcp): per-operation argument schemas replace the NATIVE_XML_DSL_ARGS union`.

### Task 5: Аннотации поведения из имени

**Files:**
- Modify: `crates/unica-coder/src/interfaces/mcp.rs` (`tool_definitions`, район :437–:466)
- Test: `crates/unica-coder/src/interfaces/mcp.rs` (тестовый модуль файла)

**Interfaces:**
- Produces: каждый Tool в `tools/list` несёт `annotations`: `readOnlyHint`/`idempotentHint: true` для суффиксов `info|map|status|search|resolve|children|locate|resources|read|list|logs|wait|graph|outline|definition|diff|decompile|get|explain|validate`; `destructiveHint: true` для `remove|cancel`; `readOnlyHint: false, destructiveHint: false` для остальных писателей; `openWorldHint` не ставится (нет инструментов, ходящих во внешний мир, кроме build/runtime — им `openWorldHint: true`).
- Consumes: суффикс — последний сегмент имени после последней точки.

- [ ] **Step 1:** Проверить модель rmcp: `rg -n "struct Tool\b" -A 25 $(find ~/.cargo/registry/src -maxdepth 2 -type d -name 'rmcp-3.1.2')/src/model.rs` (или `src/model/`), найти поле аннотаций и его тип.
- [ ] **Step 2: Падающий тест** — таблично: для каждого имени из `tools()` классификатор возвращает ожидание; отдельные assert'ы на реперные точки (`unica.meta.info` → readOnly, `unica.meta.remove` → destructive, `unica.build.update` → openWorld, `unica.meta.edit` → писатель-не-деструктив).
- [ ] **Step 3:** Реализовать `fn tool_annotations(name: &str) -> …` и подключить в `tool_definitions`.
- [ ] **Step 4:** `cargo test -p unica-coder`; перегенерировать ведомость (Step N+2 Task 4 повторно, если генератор показывает аннотации).
- [ ] **Step 5: Commit** — `feat(mcp): behavioural annotations derived from tool-name suffixes`.

### Task 6: Компактная сериализация результата и structuredContent для типизированных

**Files:**
- Modify: `crates/unica-coder/src/interfaces/mcp.rs` (`render_tool_result`, район :468; `structured_tools` в `UnicaServer::new`)
- Test: тестовый модуль `mcp.rs`

**Interfaces:**
- Produces: текстовый путь сериализуется `serde_json::to_string` (без pretty-print); все инструменты, чей результат — типизированный `data` по ведомости (45 шт.), отдают `structuredContent` (без объявления новых `outputSchema` — существующие шесть деклараций не меняются, чтобы не раздувать каталог).
- Consumes: список типизированных — колонка `contract: typed` из `spec/architecture/tool-surface-review.json`.

- [ ] **Step 1: Падающий тест** — `call_tool_text` на `unica.project.map`: результат не содержит `"\n  "` (нет pretty-отступов); второй тест: вызов типизированного инструмента через SDK-слой возвращает `structuredContent` (по образцу существующих тестов шести structured-инструментов в этом файле — найти: `rg -n structured crates/unica-coder/src/interfaces/mcp.rs`).
- [ ] **Step 2:** Убедиться в падении.
- [ ] **Step 3:** Реализация: `to_string_pretty` → `to_string`; `structured_tools` расширить с трёх хендлеров до предиката «результат типизирован» (данные — из реестра contracts, а не хардкод-списка: добавить признак в `ToolSpec` или вывести из `tool-surface-review.json` на этапе генерации — выбрать меньшую правку, зафиксировать в PR).
- [ ] **Step 4:** `cargo test -p unica-coder`; замер Task 2 suite — стоимость результатов до/после в отчёт PR.
- [ ] **Step 5: Commit** — `feat(mcp): compact result serialization and structuredContent for all typed tools`. Envelope-поля (обязательные `changes/warnings/…` и `cache`) в этой фазе **не** трогаются — их снятие связано контрактом шести `outputSchema` и уходит в ADR фазы 3.

### Task 7: Эксперимент «пустое описание против одной строки»

**Files:**
- Create: `docs-local/token-cost/2026-08-XX-descriptions-experiment.md` (сырой отчёт)
- Modify (по итогам решения): `crates/unica-coder/src/application/mod.rs` (описания в реестре), `crates/unica-coder/src/interfaces/mcp.rs` (`tool.description = None` на :452 — снять, если решение «вернуть»)

Schema-only baseline (#479 §1) — решение владельца, принятое без замера пустого описания; EDT-MCP мерил только «полная проза против одной строки» и никогда — «пустое». Закрыть развилку данными:

- [ ] **Step 1:** Собрать три арма бинаря: A — как есть (пустые описания), B — одна строка на инструмент (авторский черновик: первое предложение сценария из ведомости), C — одна строка + имя скилла-указателя. Армы — локальные ветки, в Git не уходят до решения.
- [ ] **Step 2:** Wire-разрез: discovery-доля армов через Task 1 (`--binary` на каждый арм).
- [ ] **Step 3:** Model-visible прогон вне CI: задачи Task 2 с реальным агентом на армах, метрики «выбран верный инструмент с первой попытки» и «полные токены эпизода»; протокол прогона и агрегат — в `docs-local/`, санированная таблица — комментарием в #479.
- [ ] **Step 4:** Решение владельца по данным; если «вернуть строку» — правка реестра описаний одним коммитом, перегенерация ведомости, PR со ссылкой на агрегат.

---

# Фазы 2–5 — контракты для последующих планов

Детальные планы этих фаз пишутся после гейта предыдущей. Обязательства, которые они наследуют отсюда и из спеки:

## Уход от файлового DSL: что в вехе v0.13 уже про это

Веха v0.13 несёт отдельную программу — увод каждого домена со схемы
«декомпиляция → правка файла → компиляция → валидация» на «логический адрес +
типизированные операции». Это не соседняя работа: **это первые две фазы данной
модели, разложенные по доменам**. ADR-0025 §10 уже объявил файловый
`Meta JSON DSL` неподдерживаемым публичным контрактом; веха распространяет то
же решение на остальные домены.

| Issue | Домен | Что снимает |
| --- | --- | --- |
| [#283](https://github.com/IngvarConsulting/unica/issues/283) | все | тезис волны: девять `*.validate` и цикл decompile→compile; `dryRun` становится единственным валидационным гейтом |
| [#371](https://github.com/IngvarConsulting/unica/issues/371) | формы | `form.*` на логическую адресацию по образцу `meta.*` |
| [#377](https://github.com/IngvarConsulting/unica/issues/377) | СКД | `dcs.*` на логический адрес и типизированные `operations` |
| [#378](https://github.com/IngvarConsulting/unica/issues/378) | макеты | спроектировать алгебру операций табличного документа (замены `edit` ещё нет) |
| [#381](https://github.com/IngvarConsulting/unica/issues/381) | макеты | переключить на `mxl.edit` и снять `compile`/`decompile`/`validate` |
| [#379](https://github.com/IngvarConsulting/unica/issues/379) | роли | дать `role.edit`, перевести `role.*` на логический адрес |
| [#380](https://github.com/IngvarConsulting/unica/issues/380) | подсистемы | `subsystem.*` на логический адрес и типизированные `operations` |
| [#382](https://github.com/IngvarConsulting/unica/issues/382) | комм. интерфейс | решить судьбу `interface.*`, снять `interface.validate` |
| [#376](https://github.com/IngvarConsulting/unica/issues/376) | роли, подсистемы | `meta.add` учится видам Role и Subsystem — предусловие снятия `role.compile`/`subsystem.compile` |
| [#484](https://github.com/IngvarConsulting/unica/issues/484) | модули | логическая адресация всех типов модулей Platform XML |
| [#489](https://github.com/IngvarConsulting/unica/issues/489), [#529](https://github.com/IngvarConsulting/unica/issues/529) | формы, HTTP | самостоятельная общая форма и HTTPService целиком через `meta.add`/`meta.edit` |
| [#374](https://github.com/IngvarConsulting/unica/issues/374), [#375](https://github.com/IngvarConsulting/unica/issues/375) | XDTO, макеты/справка | закрыты: `xdto.edit` типизирован, `template.*`/`help.add` растворены |

**Что это значит для модели.** Первая редакция спеки предполагала, что
`*.compile` превращается в `apply(op="create")` с DSL-документом в `args`. Это
противоречило и ADR-0025 §10, и всей вехе; спека исправлена. Точная граница:

- **умирает** DSL как документ, читаемый из файла: аргументы `JsonPath` и
  `DefinitionFile` исчезают вместе с парами `compile`/`decompile`;
- **остаётся** массовое изменение одной операцией с типизированным массивом
  элементов — как `operations` у сегодняшнего `meta.edit`.

Довод сильнее токенов: полная пересборка артефакта теряет байты, которые
форматный контракт обязан сохранять — порядок узлов, объявления префиксов
пространств имён, BOM, наблюдённый стиль перевода строки (#283). Это вопрос
целостности данных, а не эргономики.

**Порядок работ.** Доменные issue вехи — это и есть содержание фаз 2–3 по
доменам, а не параллельная ветка. Их не надо переделывать под модель: `info`
каждого домена становится входом в `view`, `edit` — операцией `apply`, и чем
больше их закрыто до фазы 2, тем меньше фасад тащит внутри переводчик
адрес→путь. Единственная явная зависимость: **#378 (алгебра операций макета)
блокирует и #381, и полноту `apply`** — это самый большой непроектированный
кусок волны.

## Восьмой вход: `diff`

`unica.cfe.diff` в первой редакции карты был отнесён к `view` с пометкой
«открытый вопрос»: сравнение расширения с базовой конфигурацией — не чтение
узла. Решение владельца: это самостоятельный вход `diff` с двумя источниками
на входе и собственной формой ответа (что добавлено, что снято, что
разошлось).

Он же закрывает недостающую возможность: сравнения **конфигурация против
конфигурации** сегодня нет вовсе, хотя оно нужно не меньше расширенческого —
сверить дамп с дампом, набор исходников с набором. Отдельный `unica.cf.diff`
заводить не нужно: под одним входом это выбор источников, а не новый
инструмент.

Работы: спроектировать форму ответа и множество сравнимых пар (расширение ↔
база, набор ↔ набор, объект ↔ объект), затем реализовать в фазе 4 вместе с
остальными фасадами. Предметного issue в вехе на это нет — его надо завести.

## Архитектурные предусловия фазы 2

Разбор кодовой базы 18.08 показал, что переход дешевле, чем следует из размера
поверхности: три из четырёх несущих деталей уже построены под другие задачи.

**Что уже есть и работает на модель:**

- **Правило чередования — действующий доменный закон.**
  `MetadataAddress::target_kind()` (`crates/unica-coder/src/domain/source_target.rs`)
  выводит вид узла из чётности сегментов, с явным обоснованием «arity decides
  the kind, not the spelling of the last segment», под инвариантом
  `INV-SOURCE-LOGICAL-IDENTITY`. Грамматику адреса из спеки вводить не надо —
  надо снять с неё одно ограничение (см. ниже).
- **`rev` / `ifRev` уже реализованы, и шире нужного.**
  `crates/unica-coder/src/domain/source_revision.rs` несёт `SourceRevision`
  (поколение, дайджест, алгоритм) и машину доверия
  `Trusted`/`Untrusted`/`Reconciling` с типизированными причинами потери.
  Модели нужен один токен в `view` и его сверка в `apply`.
- **Таблица для `can[]` уже написана.**
  `metadata_kind_collections(kind) -> &[MetaCollection]`
  (`crates/unica-coder/src/domain/metadata/operations.rs`) — чистый справочник
  «какие коллекции есть у вида». Сегодня его читают только чтобы отклонить
  неверную операцию через `validate_metadata_operation_capabilities`. `can[]` —
  тот же справочник, прочитанный вперёд; новый источник истины не нужен.
- **Реестр инструментов — данные, а не проводка** (ADR-0001), а фаза 1
  доказала разделение публикации и приёма: провод можно менять, не ломая
  вызывающих.

**Что блокирует, по убыванию несущести:**

1. **`TargetKind` не умеет ветку — единственное блокирующее изменение.**
   Значений три: `SourceRoot | MetadataObject | Module`. По чётности
   `Catalog` (1 сегмент) парсер отвергает, а `Catalog.Валюты.Attribute`
   (3 сегмента) читает как *модуль*. Ветка — то, ради чего вводится `view`, —
   сегодня неотличима от модуля. Нужен вариант `Branch` и словарь видовых
   токенов в `AddressProfile::parse` вместо чистой арифметики чётности.
   Затрагивает `INV-SOURCE-LOGICAL-IDENTITY` и все `match` по `TargetKind`.
2. **Физическая адресация несущая в обработчиках.** `logical_selector.rs`
   наводит мост, но нативные обработчики принимают `FormPath`/`TemplatePath`/
   `RightsPath` и резолвят сами. Пока это так, `view`/`apply` переводят адрес в
   путь на каждый домен — фасад тащит внутри ту самую таблицу, которую модель
   удаляет снаружи. Менять надо вход обработчика, а не фасад.
3. **Конверт `OperationResult` имеет форму мутации и обязателен** — семь
   обязательных полей, включая `cache` с девятью подполями; для `view` это
   чистые накладные (замер: 148 токенов из 209). Шесть инструментов имеют
   `outputSchema`, который на него ссылается, поэтому снятие конверта — ADR,
   а не рефакторинг.
4. **Таблица возможностей покрывает только `meta`.** У форм, ролей, макетов и
   СКД правила живут процедурно внутри валидаторов; `can[]` на их узлах
   потребует таких же справочников там.
5. **`SubsystemAddress` — намеренно отдельный тип** (`INV-SOURCE-SUBSYSTEM-TOPOLOGY`,
   плоский диалект БСП). Под одной грамматикой он либо объединяется с
   `MetadataAddress`, либо остаётся явным исключением; решение принимается
   вслух до фазы 2, а не обнаруживается в ней.

**Порядок работ до фазы 2** — по отношению «разблокирует / стоит»:
`TargetKind::Branch` и словарь токенов в парсере (без этого `view` не
существует); инверсия `metadata_kind_collections` в генератор
`affordances_for`; перевод обработчиков на логический адрес как единственный
вход. Пункты 3 и 4 влияют на цену ответа, а не на возможность его дать, и
могут идти внутри фаз 2–3.

**Фаза 2 (`unica.view`, `unica.find`).** В гейт входит кодировочный A/B по
разделу «Форма результата» спеки: армы «JSON-слоты» против «markdown-сводка»
на read-задачах каталога; решающая метрика — точность follow-up вызова
(модель копирует `at` из результата без искажений), вторая — wire-токены;
внутри JSON-арма отдельно сверяются объектная и колоночная форма листингов. Парсер адреса — правило чередования `Вид.Имя`, двуязычный вид, программное имя; таблично-управляемый тест выводимости адреса для всех 184 веток карты (источник истины — `spec/architecture/*` и `references/specs/1c-config-objects-spec.md`; сама карта — чек-лист, не контракт). `view` — слоты `at/kind/title/props/branches/can/limits/set`, `branches[].at` копируемый, `props` только заданное; реализация — фасад над `meta.info`, `form.info`, `role.info`, `dcs.info`, `mxl.info`, `subsystem.info`, `cf.info`, `xdto.info` и деревом `source.*` без дублирования их разбора. `find` — фасад над `source.resolve`/`locate` с поиском по голому имени (снимает сегодняшний отказ `MetadataAddressInvalid` на `Валюты`). Приёмка — сценарии #548 (перечислить справочники ≤ 2 вызовов и ≤ 2 000 токенов на конфигурации в 933 справочника; формы объекта — 1 вызов) плюс read-задачи каталога.

**Фаза 3 (`unica.apply`).** Диспетчер `op` → существующие native-операции; словарь `op` печатается в `can` узла; триада ошибок `unknown_op` (+`can`), `bad_value` (+`expected`), `not_found` (+`nearest`); `dryRun` — существующий механизм; `rev` в `view` + `ifRev` в `apply`. ADR: квитанция `{ok, at, did, changed, next}` без конверта, пустое не сериализуется; шесть существующих `outputSchema` не меняются (старые инструменты сохраняют контракт до фазы 5). Гейт-эксперимент: армы «`can` со скелетами» против «`can` только имена + скелет в ошибке» на write-задачах каталога.

**Фаза 4 (`search`/`check`/`run`/`docs`).** Тонкие фасады: `search` над `code.search`/`definition`/`graph` (`scope` — адрес); `check` над девятью валидаторами и `project.status` (ADR о судьбе `*.validate` — консолидация, не новые инструменты); `run` над `build.*`/`runtime.*` с самоописанием `run()`; `docs` над `documentation.*`/`standards.*`. Каждый фасад принимается паритетом сценариев ведомости поглощаемых инструментов.

**Фаза 5 (переключение).** Мажорный релиз по `docs/release-runbook.md`; переписывание 73 скиллов на 8 входов (механическая замена имён + пересмотр прозы выбора инструмента — прозы станет меньше по построению); снятие 71 инструмента из реестра; перегенерация ведомости до 8 записей; зонды на живых хостах (Claude Code, Codex) по протоколу зондов из памяти проекта; финальный замер полного каталога против baseline фазы 0.

---

## Self-review

- Покрытие спеки: адрес → фаза 2; `view`/`can` → фазы 2–3; `apply`/ошибки/`ifRev` → фаза 3; остальные входы → фаза 4; «тяжёлые домены» → контракты фаз 2–3 (граница «узел/данные» в ADR фазы 2); риски 1–2 → гейты фаз 3; риск 4 → таблично-управляемый тест фазы 2; риск 3 — принятый размен, в план не входит. Не покрыто намеренно: проводной протокол и DSL-спеки («Не-цели» спеки).
- Типы согласованы: `EpisodeReport` из Task 1 потребляется Task 2 (`--suite` агрегирует те же поля) и Task 7 (арм-сравнение discovery-доли).
- Плейсхолдеров нет: каждая задача фаз 0–1 несёт команды, пути и код/структуры; фазы 2–5 — контракты с явной отсылкой к последующим планам, а не «TBD».


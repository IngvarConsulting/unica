# Применённый `unica.runtime.execute` поверх долговременной записи — план реализации

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Применённый `unica.runtime.execute` исполняет операцию поверх долговременной записи задания, публикует прогресс по фазам и возвращает терминальный результат в исходном `tools/call`.

**Architecture:** Ветка `ToolHandler::RuntimeAdapter` с `dryRun: false` заводит задание через `RuntimeJobService`, ждёт его запись циклом опроса, публикует снимки прогресса через обобщённый транспортно-нейтральный синк и собирает `OperationResult` из терминального снимка. Отмена уважает `unsafe_phase`, обрыв вызова задание не отменяет.

**Tech Stack:** Rust (`unica-coder`), rmcp, существующие `runtime_jobs`, `runtime_admission`, `platform::process`; Python-гварды в `tests/ci`.

## Global Constraints

- Решение — ADR-0074; производные записи реестра: `INV-MCP-RUNTIME-RECEIPT`, `REQ-OBS-DETACHED-PROGRESS`.
- Целевая ветка — `release-v0.12`; версия поставки — 0.12.2; порт в `main` выполняется отдельным изменением.
- `dryRun: true` остаётся синхронным предпросмотром без побочных эффектов.
- Неклассифицированная операция по-прежнему отказывает кодом `runtime_operation_unbounded`.
- Прогресс не изображает процент: `total` — число фаз.
- Гейт: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test -p unica-coder`, `python3.12 -m unittest discover -s tests/ci`, `-s tests/dev`.

---

## Файловая карта

- `crates/unica-coder/src/application/mod.rs` — трейт синка прогресса и его снимок становятся доменно-нейтральными.
- `crates/unica-coder/src/interfaces/mcp.rs` — ключ меты берётся из снимка, а не подписывается константой поиска.
- `crates/unica-coder/src/application/runtime_admission.rs` — карта причин отдаёт предупреждение вместо отказа для классифицированных операций.
- `crates/unica-coder/src/infrastructure/application_ports.rs` — применённая ветка `RuntimeAdapter` заводит задание, ждёт запись и собирает результат.
- `plugins/unica/skills/v8-runner/SKILL.md`, `plugins/unica/references/use-cases/workspace-runtime.md` — маршрутизация применённого вызова.
- `tests/ci/test_unica_skills.py` — гвард инструкций.

---

### Task 1: Обобщить синк прогресса

**Files:**
- Modify: `crates/unica-coder/src/application/mod.rs` (трейт `SearchProgressSink`, структура `SearchProgressSnapshot`)
- Modify: `crates/unica-coder/src/interfaces/mcp.rs:327-360`
- Test: `crates/unica-coder/src/interfaces/mcp.rs` (модуль тестов файла)

**Interfaces:**
- Produces: `trait ProgressSink { fn publish(&self, snapshot: ProgressSnapshot); }`, `struct ProgressSnapshot { pub meta_key: &'static str, pub progress: u64, pub total: u64, pub message: String, pub detail: Value }`.
- Consumes: существующий вызов поиска, который передаёт свои роли в `detail`.

- [ ] **Step 1: Написать падающий тест** — снимок с ключом `io.unica/runtimeProgress` доходит до `ProgressNotificationParam` с этим же ключом меты.
- [ ] **Step 2: Прогнать и убедиться, что падает** — `cargo test -p unica-coder --lib progress_snapshot_meta_key`; ожидаемо FAIL: ключ жёстко `io.unica/searchProgress`.
- [ ] **Step 3: Реализовать** — переименовать трейт и снимок, добавить поле `meta_key`, поиск передаёт `io.unica/searchProgress` и свои роли в `detail`.
- [ ] **Step 4: Прогнать тесты** — `cargo test -p unica-coder --lib progress`; ожидаемо PASS, тесты поиска не тронуты по смыслу.
- [ ] **Step 5: Коммит** — `refactor(progress): make the progress seam domain-neutral`.

---

### Task 2: Классификация риска вместо отказа

**Files:**
- Modify: `crates/unica-coder/src/application/runtime_admission.rs`
- Test: тот же файл, модуль `tests`

**Interfaces:**
- Produces: `pub(crate) fn runtime_risk_notice(tool_name: &str, args: &Map<String, Value>) -> Result<RuntimeRiskOutcome, String>`, где `enum RuntimeRiskOutcome { Warned(RuntimeRiskNotice), Refused(RuntimeAdmissionFailure) }`, а `RuntimeRiskNotice { code: &'static str, message: String }` несёт ту же причину, что раньше уходила в отказ.
- Consumes: существующая `runtime_completion_capability`.

- [ ] **Step 1: Написать падающий тест** — `build` возвращает `Warned` с причиной про непрерываемую фазу; неизвестная операция возвращает `Refused` с кодом `runtime_operation_unbounded`.
- [ ] **Step 2: Прогнать и убедиться, что падает** — `cargo test -p unica-coder runtime_admission`; FAIL: функции нет.
- [ ] **Step 3: Реализовать** — переиспользовать карту причин; `Unclassified` остаётся отказом, остальные становятся предупреждением.
- [ ] **Step 4: Прогнать тесты** — `cargo test -p unica-coder runtime_admission`; PASS.
- [ ] **Step 5: Коммит** — `feat(runtime): classify applied risk instead of refusing it`.

---

### Task 3: Применённый вызов заводит задание и возвращает его результат

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs:575-590`
- Modify: `crates/unica-coder/src/application/mod.rs:1066-1075` (снять раннее возвращение отказа)
- Test: `crates/unica-coder/src/infrastructure/application_ports.rs`, модуль тестов файла

**Interfaces:**
- Consumes: `RuntimeJobService::enqueue(cache_root, &RuntimeJobRequest) -> JobResult<RuntimeJobSnapshot>`, `RuntimeJobService::poll(&self, id) -> JobResult<RuntimeJobSnapshot>`, поля снимка `phase`, `exit_code`, `artifact_path`, `stdout_path`, `stderr_path`, `warnings`.
- Produces: применённый `OperationResult`, у которого поле `job` несёт `jobId`, а `warnings` — причину риска из Task 2.

- [ ] **Step 1: Написать падающий тест** — применённый `unica.runtime.execute` с подставным раннером, который завершается успешно, возвращает `ok: true`, `job.jobId` и код выхода записи.
- [ ] **Step 2: Прогнать и убедиться, что падает** — `cargo test -p unica-coder applied_runtime_execute`; FAIL: сейчас возвращается отказ.
- [ ] **Step 3: Реализовать** — в применённой ветке собрать `RuntimeJobRequest` из тех же аргументов, что и синхронный путь, вызвать `enqueue`, войти в цикл `poll` с ограниченным интервалом и собрать результат из терминального снимка.
- [ ] **Step 4: Прогнать тесты** — `cargo test -p unica-coder applied_runtime_execute`; PASS.
- [ ] **Step 5: Коммит** — `feat(runtime): run applied execute on the durable record`.

---

### Task 4: Прогресс по фазам во время ожидания

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs` (цикл ожидания из Task 3)
- Test: тот же файл

**Interfaces:**
- Consumes: `ProgressSink` из Task 1, поля снимка `phase`, `heartbeat_at_ms`.

- [ ] **Step 1: Написать падающий тест** — подставной синк получает снимок на каждой смене фазы, `meta_key` равен `io.unica/runtimeProgress`, `total` равен числу фаз, а не проценту.
- [ ] **Step 2: Прогнать и убедиться, что падает** — `cargo test -p unica-coder applied_runtime_progress`; FAIL: публикаций нет.
- [ ] **Step 3: Реализовать** — публиковать снимок при изменении `phase` или `heartbeat_at_ms`.
- [ ] **Step 4: Прогнать тесты** — `cargo test -p unica-coder applied_runtime_progress`; PASS.
- [ ] **Step 5: Коммит** — `feat(runtime): publish applied phase progress`.

---

### Task 5: Отмена уважает небезопасную фазу

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs` (цикл ожидания)
- Test: тот же файл

**Interfaces:**
- Consumes: `RuntimeJobService::request_cancel_at(cache_root, id)`, поля снимка `unsafe_phase`, `cancel_deferred`, `cancelled`.

- [ ] **Step 1: Написать падающий тест (два случая)** — отмена вне небезопасной фазы приводит к отменённому результату и запросу отмены задания; отмена при непустом `unsafe_phase` возвращает результат с `job.jobId`, названной фазой и без запроса отмены.
- [ ] **Step 2: Прогнать и убедиться, что падает** — `cargo test -p unica-coder applied_runtime_cancellation`; FAIL.
- [ ] **Step 3: Реализовать** — по срабатыванию токена отмены прочитать снимок: пустой `unsafe_phase` → `request_cancel_at`; непустой → выйти из ожидания с квитанцией.
- [ ] **Step 4: Прогнать тесты** — `cargo test -p unica-coder applied_runtime_cancellation`; PASS.
- [ ] **Step 5: Коммит** — `feat(runtime): defer cancellation inside an unsafe phase`.

---

### Task 6: Инструкции описывают применённый вызов

**Files:**
- Modify: `plugins/unica/skills/v8-runner/SKILL.md`
- Modify: `plugins/unica/references/use-cases/workspace-runtime.md`
- Modify: `tests/ci/test_unica_skills.py`

**Interfaces:**
- Consumes: контракт ADR-0074.

- [ ] **Step 1: Написать падающий тест** — гвард требует, чтобы `v8-runner` описывал применённый `unica.runtime.execute` с предупреждением о риске и наблюдением через `job.*`, и запрещал выдавать предпросмотр за исполнение.
- [ ] **Step 2: Прогнать и убедиться, что падает** — `python3.12 -m unittest tests.ci.test_unica_skills`; FAIL.
- [ ] **Step 3: Реализовать** — переписать раздел маршрутизации и таблицу быстрого выбора.
- [ ] **Step 4: Прогнать гварды** — `python3.12 -m unittest discover -s tests/ci`; PASS.
- [ ] **Step 5: Коммит** — `docs(skills): route applied runtime through execute`.

---

## Проверка целиком

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test -p unica-coder`
- [ ] `/opt/homebrew/bin/python3.12 -m unittest discover -s tests/ci`
- [ ] `/opt/homebrew/bin/python3.12 -m unittest discover -s tests/dev`

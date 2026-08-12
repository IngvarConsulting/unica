# Issue 462 Cyrillic Diagnostics Regression Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Закрепить сохранение кириллического пути на default analyze-маршруте `unica.code.diagnostics`.

**Architecture:** Один unit-level adapter test подменяет только `ProcessRunner`, но выполняет реальную маршрутизацию `BslAnalyzerMcpAdapter`, JSONL-парсер и формирование typed payload. Production-код не меняется.

**Tech Stack:** Rust, `serde_json`, встроенный test harness Cargo.

## Global Constraints

- Публичный контракт MCP не меняется.
- Тест не зависит от Windows или наличия внешнего бинарника.
- Ожидаемый кириллический путь задаётся независимым строковым литералом.

---

### Task 1: Cyrillic diagnostics path regression

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Test: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

**Interfaces:**
- Consumes: `BslAnalyzerMcpAdapter::with_process_runner`, `RecordingProcessRunner`, JSONL events `start`, `file`, `done`.
- Produces: test `diagnostics_analyze_preserves_cyrillic_paths_through_typed_jsonl`.

- [x] **Step 1: Add the regression test**

  Build a complete one-file JSONL stream whose path is
  `CommonModules/РеактивныйКлиент/Ext/Module.bsl`. Invoke default
  `unica.code.diagnostics` through `BslAnalyzerMcpAdapter` and assert the literal
  normalized path, absent `stdout`, and forced `--format jsonl` argument.

- [x] **Step 2: Run the exact test**

  Run:
  `cargo test -p unica-coder diagnostics_analyze_preserves_cyrillic_paths_through_typed_jsonl --lib`

  Expected: PASS on current `main`, because PR #425 already contains the production fix.

- [x] **Step 3: Perform a mutation check**

  Temporarily change the adapter's forced analyze format from `jsonl` to `console` and
  verify that the exact test fails on its command contract. Restore the production line
  immediately and rerun the exact test. Do not retain the mutation.

- [x] **Step 4: Run related regression suites**

  Run:
  `cargo test -p unica-coder diagnostics_analyze --lib`

  Run:
  `cargo test -p unica-coder diagnostics_jsonl --lib`

  Expected: all selected tests PASS.

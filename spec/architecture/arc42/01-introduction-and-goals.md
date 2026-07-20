# 1. Введение и цели

## Цель

Unica предоставляет Codex-плагин для повседневной разработки 1C:Enterprise:
инициализация workspace, работа с XML-исходниками конфигурации, формами, СКД,
MXL, ролями, сборкой, диагностикой, справочной информацией и полным безопасным
циклом задач для конфигураций, связанных с хранилищем 1С.

## Главная архитектурная цель

Для LLM должен существовать один публичный MCP server: `unica`. Все остальные
движки являются внутренними adapters, чтобы синхронизация кешей, индексов и
workspace state происходила внутри orchestrator, а не через модель.

## Stakeholders

- AI agent: вызывает стабильные tools `unica.*` и получает компактный structured
  result.
- 1C developer: получает operation skills and MCP tools without needing to run
  skill-local operation files directly.
- Maintainer: обновляет bundled tools, skills, Rust orchestrator и specs без
  нарушения public MCP contract.

## Goals

1. Один публичный MCP contract.
2. Минимальный расход контекста LLM на инфраструктурную координацию.
3. Явное владение cache/state внутри Rust orchestrator.
4. Native Rust MCP handlers own command semantics for operation backends.
5. Проверяемый packaging и fresh Codex visibility.
6. Воспроизводимый branched-development от чистого distribution baseline до
   одного repository commit, verified unlock, archive, recovery и owned cleanup.

---
id: INV.DOC.ARCHIVE-FROZEN
status: active
governs: process
decision: DEC.2026-08-18.ARCHITECTURE-RESET
check: tests/arch/test_registry.py::test_archive_matches_frozen_manifest
scope: [docs]
---

# Архив v1 заморожен по содержимому

Каждый файл `docs/arch-v1/**`, кроме самого `MANIFEST.sha256`, назван в
манифесте точным SHA-256; набор путей и байты совпадают с ним. Слой не служит
источником действующего правила: судьба каждого прежнего предмета названа в
`FATE.md`.

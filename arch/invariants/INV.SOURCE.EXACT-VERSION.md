---
id: INV.SOURCE.EXACT-VERSION
status: active
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/format_guard.rs
scope: [source]
---

# Версия формата — точный литерал, а корень — точный QName

Поддерживаемая версия сравнивается как литерал, а не как число; цель записи опознаётся по
точному QName корня; версию решает корень-владелец, и отказ наступает до первой записи.

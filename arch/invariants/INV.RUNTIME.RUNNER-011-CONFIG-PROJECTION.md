---
id: INV.RUNTIME.RUNNER-011-CONFIG-PROJECTION
status: active
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check: crates/unica-coder/src/infrastructure/daemon/runner_011.rs::projection_preserves_overlay_paths_and_removes_private_files_on_failure
scope: [wire, app, product]
---

# Проекция конфигурации раннера сохраняет цель и пользовательские файлы

Проекция целевого проекта в 0.11 не переписывает исходные файлы проекта.
Она сохраняет наложение local-слоя и базу относительных путей, удаляет приватные
файлы и при ошибке. Поддержана только origin; другие именованные цели не
заменяются origin, неподдерживаемые сочетания не отбрасываются молча.

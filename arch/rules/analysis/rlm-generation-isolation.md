---
id: INV.CACHE.GENERATION-CUTOVER
check:
  - crates/unica-coder/src/infrastructure/workspace_index.rs::legacy_ready_marker_requires_an_isolated_build_before_becoming_active
  - crates/unica-coder/src/infrastructure/workspace_index.rs::legacy_status_and_lock_do_not_gate_builder_15
  - crates/unica-coder/src/infrastructure/workspace_index.rs::current_pair_scoped_builder_14_bytes_are_ignored_and_preserved
  - crates/unica-coder/src/infrastructure/workspace_index.rs::active_build_rejects_db_outside_declared_build_directory
  - crates/unica-coder/src/infrastructure/workspace_index.rs::builder_17_uses_a_new_generation_and_leaves_older_generations_untouched
---

# Несовместимая версия RLM строит отдельный индекс

Если новая версия RLM требует несовместимого формата индекса, она получает
отдельные файлы данных, состояния и блокировки — новое поколение индекса.
Запуск и построение нового поколения не меняют старое. Готовый старый индекс
не позволяет объявить новый готовым.

Поколение формата указывает совместимость схемы хранения, а не конкретную
сборку данных. Отдельные каталоги сборок и закрепление сборки на время чтения
описаны в [правиле доступности индекса](rlm-stale-results.md), прекращение
повторного использования сессии — в [жизненном цикле сессии](rlm-session-reuse.md).
Условия удаления прежнего поколения задаёт [правило уборки](rlm-generation-collection.md).

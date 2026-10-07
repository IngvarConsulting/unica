---
id: INV.CACHE.GENERATION-CUTOVER
check:
  - crates/unica-coder/src/infrastructure/workspace_index.rs::legacy_ready_marker_requires_an_isolated_build_before_becoming_active
  - crates/unica-coder/src/infrastructure/workspace_index.rs::legacy_status_and_lock_do_not_gate_builder_15
  - crates/unica-coder/src/infrastructure/workspace_index.rs::current_pair_scoped_builder_14_bytes_are_ignored_and_preserved
  - crates/unica-coder/src/infrastructure/workspace_index.rs::builder_15_never_accepts_a_ready_db_reported_from_builder_14_storage
  - crates/unica-coder/src/infrastructure/workspace_index.rs::builder_17_uses_a_new_generation_and_leaves_older_generations_untouched
  - crates/unica-coder/src/infrastructure/workspace_index.rs::active_build_collects_an_idle_previous_generation_but_not_the_current_one
  - crates/unica-coder/src/infrastructure/rlm_generation_collection.rs::only_an_idle_previous_generation_is_removed_in_every_location
  - crates/unica-coder/src/infrastructure/rlm_generation_collection.rs::a_recent_file_deep_in_a_generation_keeps_it
  - crates/unica-coder/src/infrastructure/rlm_generation_collection.rs::a_generation_whose_lock_is_held_is_kept_until_released
  - crates/unica-coder/src/infrastructure/rlm_generation_collection.rs::a_link_named_like_a_generation_is_neither_followed_nor_removed
  - crates/unica-coder/src/infrastructure/rlm_generation_collection.rs::a_linked_generation_parent_is_not_walked
---

# Несовместимая версия RLM строит отдельный индекс

Если новая версия RLM требует несовместимого формата индекса, она получает
отдельные файлы данных, состояния и блокировки — новое поколение индекса.
Запуск и построение нового поколения не меняют старое. Готовый старый индекс
не позволяет объявить новый готовым.

Поколение формата указывает совместимость схемы хранения, а не конкретную
сборку данных. Несколько сборок одного формата имеют разные идентификаторы и
отдельные каталоги. Читающая сессия закрепляет идентификатор активной сборки;
переключение активной записи не изменяет каталог уже начатого чтения.

## Уборка прежних поколений

Чтение активной сборки индекса запускает в фоне уборку прежних поколений
той же пары рабочего пространства и исходников. Ошибка или пропуск уборки
не влияет на чтение и не становится отказом; один процесс осматривает пару
не чаще раза в час.

Кандидат — только каталог с именем `index-v<число>`, отличным от текущего
поколения, в одном из трёх мест: данные `rlm-bsl/`, `caches/rlm-bsl/` и
`locks/rlm-bsl/`. Прочие каталоги и корзины других механизмов не трогаются.
Поколение удаляется целиком, только если:

- ни один компонент пути от корня пары до любой его части не является
  ссылкой или точкой повторной обработки; иначе поколение остаётся, по ссылке
  уборка не переходит;
- все файлы и каталоги поколения не менялись не меньше суток;
- его блокировку индекса `bsl_index.lock` удалось взять; занятая блокировка
  оставляет поколение до следующей попытки.

Части поколения сначала переименовываются в корзину `.trash-rlm-*` своего
каталога и только потом удаляются, без перехода по ссылкам. Прерванная уборка
оставляет целую часть под прежним именем или корзину, которую уберёт
следующая попытка.

Блокировку держит только построение индекса, и только оно защищено ею.
На Unix блокировка удерживается, пока части поколения переносятся.
На Windows она отпускается перед переносом, иначе сорвала бы перенос своего
каталога. Построение, взявшее её в этом окне, может потерять уже перенесённые
данные и кэш. Порядок переноса — данные, кэш, блокировка — даёт открытой базе
сорвать первый же перенос, и поколение тогда остаётся целым.

Чтение не меняет времени изменения файлов и активностью не считается.
Читающую сессию сборки Unica с прежней версией RLM уборка доказать не может;
её защищает только суточный простой. На Windows открытая база срывает
переименование, на Unix открытый файл продолжает читаться после удаления
имени. При возврате к старой версии удалённый индекс разрешено построить
заново. Хосты с разными версиями Unica на одном рабочем пространстве поэтому
могут перестраивать индекс прежнего поколения после суточного простоя.

Тесты проверяют разделение поколений 14 и 15, переход на поколение 17 без
изменения поколения 15, запуск уборки при чтении, удаление прежнего
поколения во всех трёх местах, сохранение поколения со свежим файлом,
пропуск занятой блокировки и ссылок на любом уровне пути. Проверки простоя
и запуска уборки при чтении состаривают каталоги, а Windows не даёт этого
сделать простым открытием каталога; там эти два теста ничего не доказывают.

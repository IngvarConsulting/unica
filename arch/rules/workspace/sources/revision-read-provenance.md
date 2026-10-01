---
id: INV.SOURCE.REVISION-READ-PROVENANCE
check:
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_snapshot_never_mixes_a_replaced_root_name_with_the_open_tree
  - crates/unica-coder/src/infrastructure/source_revision.rs::ambient_manifest_cannot_satisfy_a_retained_fast_path
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_retained_manifest_cannot_satisfy_an_ambient_fast_path_after_root_swap
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_manifest_uses_the_existing_source_digest_algorithm
---

# Ревизия описывает тот же физический источник, из которого прочитаны данные

При чтении через удерживаемый каталог нельзя использовать кеш ревизии,
полученный обычным чтением другого каталога по тому же пути. Обратная
подмена также запрещена: после замены каталога обычное чтение не получает
ревизию прежнего удерживаемого дерева.

Если имя корня заменено, чтение удерживаемого дерева сохраняет прежнюю
согласованную ревизию либо отказывает. Байты одного дерева не смешиваются
с ревизией другого. Для одинакового дерева удерживаемое и обычное чтение
вычисляют одинаковый дайджест содержимого.

Проверки подмены открытого каталога выполняются на ОС, которые её допускают.

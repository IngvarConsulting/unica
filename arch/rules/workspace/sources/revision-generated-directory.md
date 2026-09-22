---
id: INV.SOURCE.REVISION-EXCLUDES-GENERATED-DIRECTORY
check:
  - crates/unica-coder/src/infrastructure/source_revision.rs::corpus_digest_tracks_content_and_path_but_ignores_generated_cache
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_scan_resolves_child_name_policy_once_per_directory
---

# Содержимое .build не входит в ревизию исходников

При вычислении ревизии Unica не обходит `.build` на любом уровне дерева
исходников и не учитывает его содержимое. Имя сравнивается по правилам
файловой системы содержащего каталога.

Это не делает такой каталог допустимой частью исходников: проверка
готовности отдельно сообщает о `.build` в корне набора.

Проверки охватывают обычный обход и удерживаемое дерево с управляемыми
правилами сравнения имён вложенного каталога.

---
id: INV.APP.BOUNDED-OPEN-DESCRIPTORS
check:
  - crates/unica-workspace/src/apply.rs::reference_scan_descriptor_count_does_not_grow_with_inputs
  - crates/unica-workspace/src/apply.rs::reference_guard_without_descriptors_rejects_nested_input_substitution
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::saved_apply_plans_do_not_retain_per_change_descriptors
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::saved_apply_rechecks_same_inode_same_length_hash_before_first_write
---

# Число открытых дескрипторов не растёт с размером конфигурации

Планирование и предпросмотр `apply` держат одновременно открытыми не больше
ограниченного числа файлов и каталогов, не зависящего от числа файлов набора
исходников. Сохранённый ожидающий план не удерживает дескриптор каждого
прочитанного или заменяемого файла и каталога его маршрута. Защиту от подмены
между планированием и исполнением дают сохранённые идентичности маршрута и
файла, длина и хеш содержимого, которые проверяются заново от удерживаемого
корня. Изменённый, подменённый или исчезнувший вход по-прежнему отклоняет
устаревший план без записи.

На время одного исполнения проверенные файлы и каталоги могут быть удержаны
для привязки последующей записи к тем же физическим объектам. Число таких
дескрипторов ограничено числом изменений плана, а не размером конфигурации.
Они не переживают исполнение и не принадлежат сохранённому плану.

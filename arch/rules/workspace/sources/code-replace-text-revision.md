---
id: INV.SOURCE.CODE-REPLACE-TEXT-REVISION
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_body_of_a_module_answers_with_the_revision_of_its_text
  - crates/unica-coder/tests/platform/code_patch_composition.rs::a_replacement_from_a_stale_read_is_refused_and_a_fresh_one_plans
  - crates/unica-bsl/src/text.rs::a_module_text_revision_follows_every_byte_of_the_file
  - crates/unica-coder/tests/platform/code_patch_composition.rs::if_rev_belongs_to_code_replace_and_names_a_revision
gap: https://github.com/IngvarConsulting/unica/issues/2225
---

# Замена кода по прочитанному тексту не затирает чужую правку

Чтение `Body` модуля или его метода, в том числе диапазона строк, отдаёт
в `rev` ревизию текста этого модуля. Она меняется при любом изменении байтов
файла модуля и только при нём. Ту же ревизию отдаёт служба LSP в ответе
`workspace/textDocumentContent` для документа модуля.

`code.replace` принимает необязательный `ifRev` в аргументах операции;
верхний уровень `apply` его по-прежнему [не принимает](../apply-revision-fence.md).
Если ревизия текста модуля к моменту плана другая, план отказывает до записи
кодом `stale_revision` по пути `ops[<i>].args.ifRev` и не применяет ни одной
операции вызова. Без `ifRev` замена работает как прежде. Между планом
и исполнением текст защищает [равенство входов плана](projected-revision-equality.md).

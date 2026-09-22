---
id: INV.SOURCE.MODULE-METHOD-DECLARATION
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_projection_uses_ast_and_omits_body_text
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::explicit_body_preserves_lines_paginates_and_filters_without_method_duplication
---

# Описание метода отделяет декларацию от тела

Проекция метода сохраняет исходную декларацию, документацию, канонический
вид (`procedure` или `function`) и признак экспорта. Текст тела приходит
через `Body` и не дублируется в описании метода.
Проверка проходит построитель проекции BSL.

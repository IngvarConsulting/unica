---
id: INV.SOURCE.MODULE-METHOD-DECLARATION
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_projection_uses_ast_and_omits_body_text
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::method_text_spans_doc_comment_directive_declaration_and_end_without_preprocessor
---

# Описание метода отделяет декларацию от тела

Проекция метода сохраняет исходную декларацию, документацию, канонический
вид (`procedure` или `function`) и признак экспорта. Текст тела приходит
через `Body` и не дублируется в описании метода.
Проверка проходит построитель проекции BSL.

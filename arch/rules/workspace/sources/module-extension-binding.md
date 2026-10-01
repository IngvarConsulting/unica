---
id: INV.SOURCE.MODULE-EXTENSION-BINDING
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::extension_annotations_resolve_independently_from_compilation_directives
gap: https://github.com/IngvarConsulting/unica/issues/973
---

# Перехват метода расширением сохраняет аннотацию и доказанную цель

Аннотации `Перед`, `После`, `Вместо` и `ИзменениеИКонтроль` описываются
отдельно от директив компиляции. Исходное написание аннотации сохраняется.
`targetAt` появляется только при однозначно установленном базовом методе;
неизвестная или неоднозначная цель не выдумывается.

Проверка передаёт построителю заранее установленные цели. Подключение их
разрешения к текущему `view` остаётся разрывом из `gap`.

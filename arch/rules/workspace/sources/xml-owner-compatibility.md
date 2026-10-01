---
id: INV.SOURCE.EXACT-VERSION
check:
  - crates/unica-coder/src/infrastructure/format_guard.rs::version_owning_target_cannot_hide_behind_supported_source_set_owner
  - crates/unica-coder/src/infrastructure/format_guard.rs::code_patch_inside_older_source_set_uses_the_same_format_boundary
  - crates/unica-coder/src/infrastructure/platform_xml_owner.rs::existing_form_content_resolves_exact_wrapper_and_source_set_owners
---

# Совместимый набор не скрывает несовместимую версию цели

При определении версии существующей цели учитываются сама цель и её
владельцы — XML-документы, задающие формат содержащего её объекта и набора
исходников. Для содержимого управляемой формы это её собственный XML,
точное описание этой формы и корень набора исходников.

Проверка формата отклоняет изменение при неподдерживаемой версии цели
или её владельца. Совместимый корень конфигурации не разрешает изменить
форму более нового формата. Отсутствие `version` в BSL-модуле также
не разрешает `unica.code.patch` внутри набора старого формата.

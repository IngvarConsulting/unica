---
id: INV.SOURCE.EXACT-ROOT-QNAME
check:
  - crates/unica-coder/src/infrastructure/platform_xml_owner.rs::exact_declared_target_rejects_a_source_set_owner_with_the_wrong_root
  - crates/unica-coder/src/infrastructure/platform_xml_owner.rs::exact_declared_versionless_roots_and_absent_outputs_have_no_owner
---

# XML-цель должна соответствовать объявленному виду документа

При разрешении существующей XML-цели с объявленным видом документа её корень проверяется
по имени и пространству имён. Документ другого вида отклоняется, даже
если окружающий набор исходников имеет поддерживаемый формат. Например,
описание конфигурации нельзя принять за табличный документ MXL.

При таком разрешении самостоятельные СКД и MXL не получают собственной версии формата:
для их корней атрибут `version` недопустим. Действительно отсутствующему
новому файлу версия также не приписывается. Эти исключения не отменяют
проверку версии содержащего их набора исходников.

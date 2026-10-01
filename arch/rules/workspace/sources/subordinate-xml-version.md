---
id: INV.SOURCE.ROOT-POLICY-OWNERSHIP
check:
  - crates/unica-coder/src/infrastructure/platform_xml_owner.rs::container_scoped_versioned_roots_never_become_standalone_owners
---

# Версия дочернего XML не делает его владельцем формата

При определении владельца формата подчинённый XML-документ не становится
самостоятельным источником версии из-за наличия атрибута `version`.
Это относится к предопределённым данным, описанию картинки и расписанию:
их собственный атрибут не задаёт отдельную границу совместимости.

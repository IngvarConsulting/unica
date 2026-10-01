---
id: INV.SOURCE.ROOT-POLICIES-CLOSED
check:
  - crates/unica-coder/src/infrastructure/platform_xml_roots.rs::every_root_has_independently_recorded_publication_and_owner_policies
  - crates/unica-coder/src/infrastructure/platform_xml_roots.rs::unregistered_roots_have_no_policies
---

# Неизвестному корню XML не назначается политика по умолчанию

Правила записи XML и определения владельца его формата выбираются по
зарегистрированной паре имени корня и пространства имён. Знакомое имя
в чужом пространстве имён не получает правила знакомого документа.
Незарегистрированной паре обе политики не назначаются.

Проверки относятся к каталогу политик. Конкретный отказ операции зависит
от обработчика документа; это правило не задаёт единый код отказа `apply`.

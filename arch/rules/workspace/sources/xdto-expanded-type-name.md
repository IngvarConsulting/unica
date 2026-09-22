---
id: INV.SOURCE.XDTO-EXPANDED-TYPE-NAME
check:
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs::expanded_xdto_names_do_not_depend_on_the_xml_prefix
  - crates/unica-coder/src/infrastructure/native_operations/meta/info_projection_tests.rs::web_service_details_preserve_packages_operations_and_expanded_qnames
---

# Префикс XML не меняет имя типа XDTO

При чтении описания веб-сервиса тип XDTO определяется пространством имён и локальным именем. Замена XML-префикса при том же пространстве имён не меняет прочитанный тип. Это относится к возвращаемому типу операции и типам её параметров.

Проверки относятся к профилю чтения XML, а не к форме ответа всех публичных инструментов.

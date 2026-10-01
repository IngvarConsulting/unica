---
id: INV.SOURCE.MINIMAL-REGISTER-HAS-NO-INVENTED-RESOURCE
check:
  - crates/unica-coder/src/infrastructure/native_operations/meta/template_catalog_tests.rs::minimal_templates_never_invent_content
---

# Шаблон регистра не придумывает ресурс

Минимальные шаблоны регистра бухгалтерии и регистра расчёта не добавляют
ресурс за вызывающего. Нужные ресурсы задаются явным изменением.
Это не означает, что пустой регистр готов к применению платформой.

---
id: INV.SOURCE.MINIMAL-REGISTER-HAS-NO-INVENTED-RESOURCE
check:
  - crates/unica-metadata-designer/tests/platform/template_catalog.rs::minimal_templates_never_invent_content
---

# Шаблон регистра не придумывает ресурс

Минимальные шаблоны регистра бухгалтерии и регистра расчёта не добавляют
ресурс за вызывающего. Нужные ресурсы задаются явным изменением.
Это не означает, что пустой регистр готов к применению платформой.

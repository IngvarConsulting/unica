---
id: INV.SOURCE.HTML-TEMPLATE-VALIDATION
check:
  - crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs::html_template_pages_must_match_registered_language_codes
  - crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs::html_template_requires_both_its_descriptor_and_language_page
  - crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs::html_template_accepts_every_declared_language_page
  - crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs::html_template_page_accepts_registered_utf8_html_without_xml_grammar
  - crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs::html_template_page_rejects_non_utf8_bytes
gap: https://github.com/IngvarConsulting/unica/issues/965
---

# HTML-страница макета не обязана быть корректным XML

Валидатор HTML-макета проверяет XML-описание и наличие объявленных страниц.
Сами страницы должны быть UTF-8, но не разбираются как XML: допустимы
HTML DOCTYPE, HTML-сущности и незакрытые по правилам XML элементы.

При изменении другого свойства макета байты страницы сохраняются без
нормализации. Существующие проверки валидатора не закрывают эту проверку
сохранения при публикации.

Это проверка состава и кодировки, а не правильности HTML-разметки,
внешних ресурсов или результата в браузере.

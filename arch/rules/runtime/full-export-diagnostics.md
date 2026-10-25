---
id: CTR.RUNTIME.FULL-EXPORT-DIAGNOSTICS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::full_export_rejects_invalid_results_before_replacing_sources
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::full_export_reports_unconfirmed_private_location_without_changing_publication_facts
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export/publication_protocol.rs::validation_projection_excludes_untrusted_text_and_keeps_uncertain_publication
---

# Отказ полной выгрузки объясняет причину и состояние сохранённых данных

При отказе семантической проверки staged-дерева MCP сохраняет `invalid_result`
и сообщает безопасную структурированную причину также в тексте диагностики.
Различаются неподдерживаемый XML-корень, нарушение требования версии,
повреждённый XML, недопустимый или отсутствующий владелец и пустая выгрузка.
Когда отказ относится к файлу, ответ указывает допустимый относительный путь;
для распознанного корня передаётся безопасное представление QName. Ожидание
версии различает точный литерал и отсутствие атрибута. Сырой текст parser,
stdout/stderr провайдера, распознаваемые секреты и приватные абсолютные пути
в этот отчёт не входят.

Очистка сообщает свою причину отдельно от validation и результата публикации.
Recovery указывается только подтверждённым путём внутри workspace. Если его
текущее расположение не подтверждено, ответ явно сообщает это; известное
прежнее имя остаётся относительным и не становится recovery-артефактом.
Отказ очистки сохраняет факты публикации по
[правилу полной выгрузки](full-export-publication.md): неопределённое состояние
не выдаётся за отсутствие публикации, а опубликованные исходники — за
неопубликованные.

---
id: INV.APP.ADMISSION-FAILURE-ISOLATION
check:
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::root_view_keeps_the_same_task_and_daemon_past_the_admission_grace
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::root_check_keeps_the_same_task_and_original_inspection_deadline
gap: https://github.com/IngvarConsulting/unica/issues/930
---

# Затянувшаяся подготовка одного задания не останавливает весь демон

Срок ожидания клиента ограничивает прямой ответ, а не время подготовки
фонового задания. Затянувшуюся проверку аргументов или допуск к рабочему
пространству Unica ограничивает отдельно от исправных независимых заданий.

После локальной отмены или отказа задержанный исполнитель не должен начать
операцию, опубликовать результат или воспользоваться уже переданным правом
на запись. Если безопасно изолировать его невозможно, общая остановка
допустима. [Неопределённая запись в хранилище](receipt-store-fail-stop.md)
по-прежнему требует защитной остановки.

Сейчас подготовка, не завершившаяся за две секунды после выдачи задания,
может остановить весь демон. Замена этого поведения и проверки изоляции
описаны в `gap`.

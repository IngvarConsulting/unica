# Ответы анализатора

`resident-baseline-authors-0.2.86.json` — настоящий ответ diagnostics file
bsl-analyzer v0.2.86, upstream commit 53d8765288adf3b9277b7a4112dba167e51690b7.
На синтетическом модуле с тем же телом, что в workspace-diagnostics,
в копии workspace-code создан Git baseline автора
vendor@example.invalid; baseline оставляет только UnusedLocalVariable.
Ответ одновременно сообщает baseline known=1/new=2 и authors=2. Только
result_id заменён на captured-fixture; остальные поля сохранены. Производные
контрпримеры в парсерном тесте явно отмечены; их сочетания не выдаются за
захваченный живой ответ. Отдельный delivery-тест воспроизводит оба фильтра
на настоящем закреплённом binary.

## Управляемые ошибки JSONL

`jsonl-fault-producer.rs` — самостоятельный тестовый нативный producer,
не опубликованный bsl-analyzer. Общий профиль `fault-injection` использует
его в настоящем ProcessRunner/parser/coordinator и публичном `unica.check`.
Тестовый manifest называет версию `0.0.0-test-fixture`, исходник, реальный
SHA256 бинаря и `publishedArtifact:false`; поставляемый pin не меняется.

Три закрытых случая (`empty`, `invalid-event`, `unknown-severity`) завершаются
с exit 0, чтобы проверить повреждение потока, а не ошибку процесса.
Секрет и приватный путь в повреждённом входе — синтетические маркеры;
проверка всего публичного ответа запрещает их утечку. Неизвестный случай
отклоняется до вызова. Эта проверка не доказывает исправление неизвестной
причины исторического сбоя в Unica 0.12.3 с bsl-analyzer 0.2.67.

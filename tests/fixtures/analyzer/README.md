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

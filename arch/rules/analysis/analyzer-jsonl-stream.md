---
id: INV.ANALYSIS.ANALYZER-JSONL-STREAM
check:
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::diagnostics_analyze_parses_more_than_the_legacy_stdout_capture_limit
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::diagnostics_analyze_rejects_one_oversized_line_without_publishing_it
gap: https://github.com/IngvarConsulting/unica/issues/958
---

# Большой ответ анализатора разбирается без усечения начала потока

Суммарный размер JSONL больше 1 MiB не обрезает результат анализа.
Одна строка длиннее 8 MiB отклоняется. Исходный поток не выдаётся как
текстовый дубль результата, в том числе при ошибке разбора.

После ошибки разбора stdout и stderr продолжают вычитываться, чтобы
дочерний процесс не завис на заполненной трубе. Эта граница ещё
требует проверки с процессом, который продолжает писать после ошибки.

Проверки адаптера используют управляемый исполнитель процесса. Они
не измеряют общий расход памяти анализатора или Unica.

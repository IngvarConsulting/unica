---
id: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
---

# Завершение retained apply

Revision manifest исключает компонент `.build` из исходников.

Revision candidate готовится без записи, проверяется по временно
опубликованному retained source и становится
видимым в памяти только после postimage и final actor/revision gates. Ошибка до
этой точки откатывает journal вместе с batch-owned пустыми
каталогами; cleanup после успеха остаётся bounded diagnostic.

Existing logical-read fence capability не меняется при apply admission,
planning и dry run.

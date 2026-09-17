---
id: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
---

# Порядок публикации и завершения retained apply

Revision manifest исключает компонент `.build` из исходников.

Source postimages публикуются первыми, eager cache metadata следующими,
revision record затем и `state.json` последним. Revision candidate готовится
без записи, проверяется по временно опубликованному retained source и становится
видимым в памяти только после postimage и final actor/revision gates. Ошибка до
этой точки откатывает journal в обратном порядке вместе с batch-owned пустыми
каталогами; cleanup после успеха остаётся bounded diagnostic.

Existing logical-read fence capability не меняется при apply admission,
planning и dry run.

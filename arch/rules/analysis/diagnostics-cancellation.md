---
id: INV.APP.DIAGNOSTIC-CANCELLATION
check:
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_cancellation_discards_partial_items
---

# Отмена диагностики не возвращает частичный успех

При отмене общего запроса координатор передаёт отмену поставщикам
и возвращает `cancelled`. Уже полученные находки не публикуются как результат
отменённой проверки. Тест совмещает готовую находку одного поставщика
с ожидающим отмены вторым поставщиком.

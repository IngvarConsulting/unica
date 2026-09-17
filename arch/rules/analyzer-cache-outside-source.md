---
id: INV.CACHE.STATE-OUTSIDE-SOURCE
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::bsl_analyzer_cache_stays_outside_a_workspace_wide_source_root
---

# Путь кеша анализатора находится вне исходников

Успешно рассчитанный путь кеша BSL-анализатора находится вне корня исходников,
даже если настроенный корень кеша попадает внутрь него. Путь сохраняет ключ
нормализованной идентичности источника. Проверка охватывает расчёт пути
при корне исходников, совпадающем с рабочим пространством.

---
id: INV.APP.DEPENDENCY-DIRECTION
gap: https://github.com/IngvarConsulting/unica/issues/983
---

# Внутренние слои не зависят от транспорта и инфраструктурных реализаций

Слой `domain` не ссылается на `application`, `infrastructure`
или `interfaces`. Слой `application` не ссылается на `infrastructure`
или `interfaces`. Инфраструктурный код не обращается к `interfaces`.

Продуктивный `application` получает состояние Git через инфраструктуру
и не запускает процесс Git самостоятельно. Проверка прямого запуска пока
распознаёт полное имя `std::process::Command::new` с литералом `git`;
обход через импорт, псевдоним или переменную остаётся в `gap`.

---
id: INV.APP.DEPENDENCY-DIRECTION
gap: https://github.com/IngvarConsulting/unica/issues/983
---

# Внутренние слои не зависят от транспорта и инфраструктурных реализаций

Слой `domain` не ссылается на `application`, `infrastructure`
или `interfaces`. Слой `application` не ссылается на `infrastructure`
или `interfaces`. Инфраструктурный код не обращается к `interfaces`.

Слой `domain` находится в пакете `unica-domain`, `application` — в
`unica-application`, `infrastructure` и `interfaces` — в `unica-coder`. Пакет
`unica-domain` не зависит от `unica-application`, а тот — от `unica-coder`,
поэтому между ними направление задают зависимости пакетов. Внутри
`unica-coder` группировка импортов и псевдонимы не меняют направление
зависимости, и граница проверяется по Rust-коду.

Продуктивный `application` получает состояние Git через инфраструктуру
и не запускает процесс Git самостоятельно. Проверка прямого запуска пока
распознаёт полное имя `std::process::Command::new` с литералом `git`;
обход через импорт, псевдоним или переменную остаётся в `gap`.

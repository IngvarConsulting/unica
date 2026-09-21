---
id: INV.WIRE.ADMISSION-NAMES-ITS-CAUSE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_admission_names_why_no_source_set_was_admitted
---

# Отказ допуска различает ненастроенный проект и неподходящие исходники

Вызов, которому нужен набор PlatformXml, объясняет, почему не смог его
получить, и предлагает доступный следующий шаг:

- Нет настроек проекта и автоматически найденных наборов — `invalid_state`
  с исходом `needsHuman` и переходами к `unica.view`, `unica.check`, `unica.run`.
- Объявлен только набор EDT — `invalid_source` с исходом `fixSource`.
  Ответ называет набор и его формат, предлагает те же три перехода.
- Некорректный `v8project.yaml` — `invalid_state` с переходами к `unica.view`
  и `unica.check`. Пояснение причины совпадает с результатом корневого просмотра.

Проверка выполняет предметные вызовы через демон с реальными каталогами
проекта. Она не покрывает отказ чтения корня, сбой обхода и истечение срока допуска.

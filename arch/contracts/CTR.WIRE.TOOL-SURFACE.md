---
id: CTR.WIRE.TOOL-SURFACE
status: active
governs: product
version: 8
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
producer: scripts/ci/generate-tool-surface.py
consumers: [review, docs]
check: crates/unica-coder/src/interfaces/mcp.rs::production_mcp_surface_exposes_only_canonical_v13_tools_and_task_compatibility
scope: [wire]
---

# Package-selected поверхность содержит восемь или одиннадцать описанных инструментов

Ведомость публичной поверхности порождается из `tools/list` собранного бинаря и
руками не пишется: имена, описания и аргументы принадлежат реестру инструментов,
а ведомость лишь показывает их рядом. Native Tasks профиль содержит ровно
восемь предметных `unica.*`; compatibility-профиль добавляет ровно три
`unica.task.*`. Имена v0.12 в обоих профилях отсутствуют. Ручной правке подлежит
только контракт результата и сценарии.

`unica.view` принимает пустой объект для bootstrap-наблюдения рабочего
пространства или `at` для логического чтения. Все tools и опубликованные
аргументы описаны; compatibility payload ограничен 16 KiB.

`unica.run` без `op` доступен до source admission и возвращает целевой словарь
раннера 1.0 с описанием, `argsSchema`, `effects`, `execution`, `implemented`,
требованиями preview/fence и `support`. `support.state` — supported, limited
или unavailable; ограниченная операция называет `supportedArgs`, адаптер и
причину ограничения. `implemented` не равен готовности платформы в окружении.
Цель выбирается верхнеуровневым `infobase`, сейчас поддержана только origin;
другая цель отклоняется без подмены. `unica.apply` и runtime-операция `apply`
относятся к разным предметам.

`download`, `infobase.dump`, `infobase.restore`, `make`, `launch`,
`extensions.list` и `extensions.set` исполняются в пределах закрытых схем
адаптера 0.11. `push` поддерживает только `delete` установленного расширения.
Остальные режимы целевого каталога отвечают отказом до платформенного запуска.
PreviewApply использует явный dryRun и ifRev; terminal launch сохраняет свой
режим. Сырые CLI-аргументы, stdout/stderr и пароли в успех не публикуются.

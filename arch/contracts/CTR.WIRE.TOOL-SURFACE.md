---
id: CTR.WIRE.TOOL-SURFACE
status: active
governs: product
version: 7
decision: DEC.2026-09-22.INSTALLED-EXTENSIONS-AND-RUNNER-RECEIPTS
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

`unica.run` без `op` доступен до source admission и возвращает закрытый словарь
направленных runtime-намерений и операций установленных расширений вместе с назначением, `effects`,
режимом `execution`, честным `implemented` и требованиями preview/fence.
Имена отдельно выражают source build, CF/CFE configuration transfer и DT
infobase transfer. Реализованная операция также сообщает точную `argsSchema`;
для ещё не реализованных операций это поле равно `null`, чтобы модель не
угадывала будущий контракт. Операции `previewApply` принимают `dryRun`;
applied-вызов связывается с preview через `ifRev`.

`cf.export` и `infobase.export` реализованы до source
admission. Их schema не принимает выбор provider: v8-runner выбирает Designer
или ibcmd и возвращает этот выбор данными preview. Apply повторяет preview,
сверяет `ifRev` и публикует проверенную квитанцию CF/CFE/DT через Task transport.

`extension.list/info/create/delete/activate` используют previewApply через
раннер с квитанцией provider. Чтение состава запускает платформенный сеанс;
create регистрирует пустое расширение, CFE загружается через cf.import.

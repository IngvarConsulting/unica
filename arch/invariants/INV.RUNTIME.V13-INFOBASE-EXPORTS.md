---
id: INV.RUNTIME.V13-INFOBASE-EXPORTS
status: active
governs: product
decision: DEC.2026-09-03.INFOBASE-EXPORT-RUN-SLICE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::v5_infobase_exports_prepare_before_source_admission_and_keep_the_revision_gate
scope: [app, product, wire]
---

# Выгрузки ИБ проходят неисполняющий preview и проверяемый apply

`infobase.configuration.export` и `infobase.dump` доступны без source set.
Preview вызывает только неисполняющий `v8-runner --dry-run`, связывает выбранный
provider, конфиги и состояние назначения с revision и не создаёт output. Apply
повторяет такой preview, принимает только совпавший `ifRev`, затем запускает
provider и независимо подтверждает непустой regular CF/CFE/DT внутри workspace.
Публичный ответ не раскрывает командную строку, stdout, stderr или credentials.
V3 и V5 обязаны подготовить эту операцию до admission PlatformXml source set и
исполнять один и тот же `PreparedInfobaseExport`; транспорт не создаёт второй
контракт preview/apply.

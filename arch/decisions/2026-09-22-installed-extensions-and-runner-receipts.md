---
id: DEC.2026-09-22.INSTALLED-EXTENSIONS-AND-RUNNER-RECEIPTS
status: superseded
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::all_extension_operations_preview_then_apply_with_a_provider_receipt
supersedes: []
superseded-by: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
establishes: [INV.APP.V13-RUN-DICTIONARY, INV.WIRE.RUN-NAMES-ARE-LAYER-AND-DIRECTION, CTR.WIRE.TOOL-SURFACE, INV.RUNTIME.EXTENSION-OPERATIONS, INV.RUNTIME.RUNNER-PROVIDER-RECEIPT]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-22-runner-011-extensions-design.md
---

# Расширения базы проходят Run с квитанцией исполнителя

**Решение.** Поставляемый v8-runner обновляется до 0.11.0. CF/DT операции
читают provider receipt вместо снятого selection; публичный план не обещает
кандидатов, которых раннер больше не отдаёт. Примеры проектного файла не
содержат снятого builder; исполнителя выбирает раннер по providers операции.

Словарь Run дополняется extension.list/info/create/delete/activate. Чтение
состава — previewApply: платформа открывает сеанс даже ради списка, что
отличает его от чтения исходников. Create регистрирует пустое расширение;
содержимое устанавливается существующим cf.import. Active принимает boolean.

Preview не запускает провайдера. Apply связан с аргументами, двумя файлами
проекта, версией раннера и его provider receipt. Ответ сверяется с предметом
и действием; неопределённое состояние базы не выдаётся за проверенное Unica.
Сырой plan/message и пути провайдера не публикуются. Префикс имени читается
из исходников, а не выдумывается в inventory базы.

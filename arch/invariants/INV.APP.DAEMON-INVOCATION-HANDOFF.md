---
id: INV.APP.DAEMON-INVOCATION-HANDOFF
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/daemon/mod.rs::daemon_invocation_receipt_deadline_is_single_and_never_replenished
scope: [app]
---

# Седьмая секунда разделяет direct response и durable Task

Завершение до 7000 мс может вернуть direct DomainResult. В 7000 мс
незавершённая Invocation уже имеет durable Task; нулевой бюджет материализует её
до execution. Конкурентные completion и handoff публикуют один terminal result
при одном execution.

Подготовка result к transport не продлевает frontend deadline: разрешён один
заранее вычтенный запас 125 мс. Результат больше 16 KiB, завершившийся при
остатке не более этого запаса, материализуется как тот же durable Task без
повторного execution; малый результат в 6999 мс сохраняет direct-семантику.

Daemon захватывает один opaque absolute deadline своим executor clock сразу
после приёма request JSONL и до strict validation, actor admission/binding и
service preparation. Переданный frontend remaining budget может только сузить
его. Actor-bound/prepared invocation и response writer используют тот же
deadline; ни один этап daemon не прибавляет duration к новому `now`.

---
id: INV.APP.DAEMON-INVOCATION-HANDOFF
---

# Границы подготовки и передачи ответа

Нулевой бюджет материализует Invocation как durable Task до execution.

Подготовка result к transport не продлевает frontend deadline: разрешён один
заранее вычтенный запас 125 мс. Результат больше 16 KiB, завершившийся при
остатке не более этого запаса, материализуется как тот же durable Task без
повторного execution; малый результат в 6999 мс сохраняет direct-семантику.

Daemon захватывает один opaque absolute deadline своим executor clock сразу
после приёма request JSONL и до strict validation, actor admission/binding и
service preparation. Переданный frontend remaining budget может только сузить
его. Actor-bound/prepared invocation и response writer сохраняют тот же private
`Arc<Clock>` authority и те же границы; чужой clock с равными `Instant`
отклоняется до direct result, store и execution. Ни один этап daemon не
прибавляет duration к новому `now`.

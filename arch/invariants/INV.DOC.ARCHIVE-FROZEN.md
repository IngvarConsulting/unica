---
id: INV.DOC.ARCHIVE-FROZEN
status: active
decision: DEC.2026-08-19.ARCHIVE-DRIFT-IS-RECORDED
check: tests/arch/test_registry.py::test_archive_drift_is_recorded
scope: [docs]
---

# Расхождение замороженного слоя объяснено

`docs/arch-v1/**` сверяется с моментом заморозки по содержимому: каждый
отличающийся файл назван в разделе прибытий `FATE.md`. Слой не служит
источником действующего правила — что из него умерло, а что пересмотрено,
сказано там же.

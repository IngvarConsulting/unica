---
id: INV.SOURCE.CODE-BORROWED-MODULE-STATE
status: active
governs: product
decision: DEC.2026-09-14.CODE-BORROWED-MODULE-STATE
check: crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_insert_and_replace_stage_module_state_atomically
scope: [source, app, cache]
---

# Запись заимствованного модуля включает его состояние в apply

При `code.insert` и `code.replace` в заимствованном общем модуле Extension
недостающее `PropertyState Module=Extended` планируется и публикуется вместе
с BSL. XML остаётся исходным до публикации, а изменение дескриптора несёт
событие `MetadataChanged`.

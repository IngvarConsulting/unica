---
id: DEC.2026-08-27.RETAINED-APPLY-REVISION-TRANSIENT-AUTHORITY-SLICE
---

# Ограничения на передачу внутреннего права исключения recovery-файлов

Journal выдаёт один sealed borrowed batch. Authority не клонируется,
не сериализуется и прекращает существовать до изменения journal,
rollback или cleanup.

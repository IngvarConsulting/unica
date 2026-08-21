---
id: INV.SOURCE.OWNER-VERSION-GATE
status: active
governs: product
decision: DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE
check: crates/unica-coder/src/infrastructure/format_guard.rs::owner_version_read_write_gate_is_complete
scope: [source]
---

# Версию решает корень-владелец до первой записи

Подчинённый XML наследует формат точного корня-владельца. Более новая выгрузка
предупреждает при безопасном чтении, старая и отсутствующая версия известного
владельца классифицируются точно, запись в старый внешний набор блокируется, а
известный DCS или MXL без собственной версии не получает выдуманного владельца.
Неизвестный версионированный корень отказывает закрыто.

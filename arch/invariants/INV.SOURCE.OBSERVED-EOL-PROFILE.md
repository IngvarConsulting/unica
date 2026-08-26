---
id: INV.SOURCE.OBSERVED-EOL-PROFILE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::observed_line_ending_profile_is_closed
scope: [product, source]
---

# Профиль перевода строки выводится из наблюдаемых байтов

Снимок различает отсутствие EOL, единый LF, CRLF или CR и смешанный профиль с
точными счётчиками и признаком завершающего EOL. `Preserve` использует локальный
контекст или единый профиль и отказывает на недоказуемом смешанном выборе;
явная политика LF обслуживает текст без EOL.

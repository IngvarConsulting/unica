---
id: INV.TOKEN.RUNTIME-LOG-TAIL
check:
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::runtime_job_lifecycle_and_log_bounds_are_complete
---

# Чтение журналов возвращает только запрошенный хвост

Сервис заданий ограничивает stdout и stderr по отдельности заданным числом
последних символов. Предшествующий текст не попадает в ответ, а многобайтовые
символы не разрезаются. Например, хвост длиной три для `stdout-абвгд` — `вгд`.

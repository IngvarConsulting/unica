---
id: INV.SOURCE.COMMAND-VISIBILITY-ROLE-SIGNAL
status: active
governs: product
decision: DEC.2026-09-09.COMMAND-VISIBILITY-NAMES-ITS-ROLES
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::command_visibility_says_when_roles_override_it
scope: [product, source]
---

# `visible` не выдаётся за всю видимость

Команда командного интерфейса несёт `roleOverrides` — число ролей,
переопределяющих общее значение. Признак присутствует всегда: ноль говорит,
что переопределений нет, и отличим от несчитанного.

Читатель, получивший `visible` без этого признака, достроил бы недостающее
сам: на боевой конфигурации так читалась бы неверно каждая одиннадцатая
команда.

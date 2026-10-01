---
id: INV.SOURCE.COMMAND-VISIBILITY-ROLE-SIGNAL
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::command_visibility_says_when_roles_override_it
---

# Общая видимость команды сопровождается числом переопределений по ролям

При чтении командного интерфейса `visible` показывает общее значение
видимости команды, а `roleOverrides` — число её ролевых переопределений.
Поле присутствует и при отсутствии переопределений: тогда оно равно нулю.
Так общее значение не выдаётся за одинаковую видимость для всех ролей.

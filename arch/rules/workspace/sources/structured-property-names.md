---
id: INV.SOURCE.STRUCTURED-PROPERTY-NAMES
check:
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests/property_name_parity.rs::dcs_and_xdto_final_props_have_explicit_roles_and_match_corresponding_can
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests/property_name_parity.rs::profile_guard_detects_unknown_props_changed_operands_and_wrong_anonymous_context
  - crates/unica-coder/src/infrastructure/v13_read/tests/structured_property_names.rs::xdto_explicit_bounds_keep_their_xml_names_and_all_observed_facts
---

# Имена свойств DCS и XDTO согласованы с операцией записи

Публичный профиль чтения DCS и XDTO явно различает имя аргумента записи,
наблюдаемое свойство без записи и вычисленный факт. Новое поле не становится
исключением из проверки только потому, что запись его не принимает.

Имя записываемого свойства в `props` совпадает с именем в опубликованном
`Can` соответствующей операции и её конкретного аргумента. Для создания
это контракт родителя, для изменения — контракт самого узла. Сплющенные
сведения об анонимном типе сверяются с аргументом его определения, а не
со схемой именованного типа. Имена исправляемых XML-свойств берутся из
формата DCS или XDTO: например, `useRestriction`, `field`, `lowerBound`
и `upperBound`.

Наблюдение не предоставляет право записи. Наблюдаемые свойства без записи
и вычисленные факты сохраняют значения и состав независимо от доступных
операций. Профиль называет их происхождение. Счётчик фильтров не является
определением фильтра, адрес разрешённой основы — её QName, а эффективные
границы не заменяют различие отсутствующего и явно заданного XML-атрибута.

Совпадение имён не обещает одинаковую форму значений: QName чтения может
содержать исходное написание и разрешённое пространство имён, а запись
принимать строку. Явные границы XDTO читаются исходными строками под именами
`lowerBound` и `upperBound`; `bounds` сохраняет эффективные границы,
признаки явного задания и неограниченности. Доступные для записи значения
по-прежнему определяет конкретный `Can` и проверка плана.

---
id: INV.APP.DIAGNOSTIC-FOCUS
check:
  - crates/unica-coder/src/infrastructure/diagnostics.rs::diagnostic_location_preserves_exact_metadata_focus_and_weakens_unknown_elements
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_metadata_object_scope_excludes_separately_addressable_children
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_zero_width_focus_keeps_its_position_and_respects_the_requested_range
  - crates/unica-coder/src/domain/diagnostics.rs::zero_width_observation_range_matches_the_requested_range_that_contains_its_caret
---

# Диагностика указывает только доказанное место внутри объекта

Место находки — цель целиком, диапазон текста либо цепочка элементов
метаданных с именами коллекций, свойством и языком. Недоказанный элемент
или частное поле XML не выдаётся за точное место: находка относится к цели целиком.

Выбор объекта включает его внутренние элементы, но не отдельно адресуемые
дочерние цели. Например, диагностика реквизита относится к объекту,
а диагностика его отдельно адресуемого модуля — к модулю.

Текстовый диапазон нумеруется от нуля, конец не включается.
Нулевая ширина означает позицию каретки, а не весь объект. Такая находка
попадает только в диапазон, содержащий эту позицию.

Проверки исполняют модель, отображение местоположения и координатор;
они не добавляют параметры диапазона в публичный `unica.check`.

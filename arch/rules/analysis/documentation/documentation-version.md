---
id: INV.APP.DOCUMENTATION-VERSION-SELECTION
check:
  - crates/unica-coder/src/infrastructure/application_ports.rs::project_platform_line_sits_between_the_call_argument_and_the_newest_install
  - crates/unica-coder/src/infrastructure/application_ports.rs::select_platform_version_requires_an_exact_directory_name_match
  - crates/unica-coder/src/infrastructure/application_ports.rs::select_platform_version_without_a_request_picks_the_numerically_newest_entry
  - crates/unica-coder/src/infrastructure/application_ports.rs::the_projects_platform_path_pin_names_the_installation_directly
  - crates/unica-coder/src/infrastructure/application_ports.rs::version_constraints_still_apply_to_a_path_pinned_installation
  - crates/unica-coder/src/infrastructure/platform_help/installation.rs::each_corpus_resolves_its_own_locale
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_a_build_pin_is_answered_by_its_family_guide_with_a_disclosure
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_a_build_pin_of_an_absent_family_is_still_refused
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_provider_names_available_versions_when_the_requested_one_is_absent
gap: https://github.com/IngvarConsulting/unica/issues/597
---

# Справка называет выбранную версию и допустимые подстановки

Справка учитывает закреплённую версию проекта: полная версия ограничивает
выбор до сборки, короткая — до семейства. Без ограничения используется
численно старшая подходящая установка в первом подходящем корне поиска. Если нужной версии нет, соседняя не подставляется.

`tools.platform.path` задаёт установку прямо, включая путь на её каталог `bin`.
Несовместимость с ограничением версии означает отказ, а не поиск другой установки.
Язык выбирается отдельно для каждого корпуса: доступный язык той же установки
может заменить отсутствующий запрошенный, но ответ называет фактический язык.

База знаний может ответить на сборку `8.3.27.2074` руководством её семейства
`8.3.27`. Попадание называет версию руководства, предупреждение — обе версии.
Чужое семейство не подставляется; отказ перечисляет доступные версии.

Если выбранная локальная установка не содержит нужной справки, Unica
сообщает об этом и называет доступные установки со справкой. Материал другой
версии в ответ не подставляется; закреплённая для выполнения версия платформы
не меняется. Проверка отказа с перечнем альтернатив остаётся в `gap`.

---
id: INV.ANALYSIS.PLATFORM-TYPE-CATALOG
check:
  - crates/unica-bsl-types/src/tests.rs::one_record_per_type_and_member_in_both_languages_with_types_by_page
  - crates/unica-bsl-types/src/page_tests.rs::a_method_page_gives_its_variants_parameters_returns_and_contexts
  - crates/unica-bsl-types/src/page_tests.rs::a_property_reads_its_type_and_use_and_a_type_page_its_collection
  - crates/unica-bsl-types/src/page_tests.rs::types_named_in_text_are_language_types_or_stay_unknown
  - crates/unica-bsl-types/src/page_tests.rs::an_array_element_is_named_in_the_same_sentence_only
  - crates/unica-bsl-types/src/cache.rs::tests::a_catalog_is_kept_per_user_build_and_generator_and_stale_when_the_help_changes
gap: https://github.com/IngvarConsulting/unica/issues/2238
---

# Каталог платформы следует справке выбранной установки

Каталог платформы строится из справки той установки платформы, которая
выбрана для рабочей области. Типы, их члены, параметры, возвращаемые типы и
места исполнения в каталоге такие, как их описывает справка этой установки;
если справка не называет тип, он остаётся неизвестным и не подменяется
догадкой. Русское и английское имя одного типа, члена или параметра — одна
запись каталога. Каталог одной установки строится один раз и используется
всеми рабочими областями пользователя, пока не изменились установка или
правила построения каталога.

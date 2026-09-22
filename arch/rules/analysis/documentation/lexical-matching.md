---
id: INV.APP.DOCUMENTATION-LEXICAL-MATCHING
check:
  - crates/unica-coder/src/infrastructure/documentation_retrieval.rs::word_order_and_morphology_do_not_matter
  - crates/unica-coder/src/infrastructure/documentation_retrieval.rs::typo_falls_back_to_fuzzy_with_discount
  - crates/unica-coder/src/infrastructure/documentation_retrieval.rs::title_match_outranks_body_match
  - crates/unica-coder/src/infrastructure/documentation_retrieval.rs::shorter_page_outranks_long_enumeration_for_equal_field
  - crates/unica-coder/src/infrastructure/documentation_retrieval.rs::lexicon_maps_ru_tokens_to_en_segment_tokens
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_an_english_query_matches_title_tokens_not_the_whole_substring
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_a_russian_query_expands_through_the_installation_lexicon
gap: https://github.com/IngvarConsulting/unica/issues/953
---

# Поиск справки сопоставляет слова запроса, а не только целую подстроку

Перестановка слов и русские окончания не должны прятать документ в проверенных
поисковых примерах. Небольшая опечатка при отсутствии прямого совпадения
допускает нечёткое совпадение с более низкой оценкой.

Допускается совпадение части слов запроса. При прочих равных дополнительное
совпавшее слово повышает оценку, но полное покрытие не получает абсолютного
приоритета над редким словом в заголовке.

При сопоставимых совпадениях заголовок весит больше текста, а короткая
страница — больше длинного перечисления. Проверки сравнивают конкретные
корпуса; правило не обещает одинаковое ранжирование любого запроса.

Двуязычные заголовки дают словарь русских и английских имён. База знаний
использует такой словарь для расширения русского запроса; отсутствие
словаря не разрешает придумывать совпадения. Проверки используют заданные
пары имён, а не произвольный машинный перевод.

//! Лексическое ядро `unica.documentation.search` (ADR-0037): токенизация,
//! стемминг ru/en, ограниченная нечёткость и BM25F-скоринг, общие для
//! локальных поставщиков документации. Ядро не знает ни о поставщиках, ни о
//! секциях: оно превращает запрос и поля документов в постинги и оценки, а
//! независимость секций и локальность оценок остаются заботой вызывающих
//! (INV-MCP-DOCUMENTATION-SECTIONS). Состояния на диске нет — индекс живёт
//! там, где его построил вызывающий (ADR-0029 п.11).

/// Токены нижнего регистра: разбиение по не-алфанумерик и по границам
/// CamelCase, включая кириллический («ТаблицаЗначений» → «таблица»,
/// «значений»). Идентификаторы 1С — русский CamelCase, поэтому без этого
/// разреза пословный поиск не увидел бы имён вовсе.
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    fn flush(tokens: &mut Vec<String>, current: &mut String) {
        if !current.is_empty() {
            tokens.push(std::mem::take(current));
        }
    }
    let characters: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for (position, &character) in characters.iter().enumerate() {
        if !character.is_alphanumeric() {
            flush(&mut tokens, &mut current);
            continue;
        }
        if character.is_uppercase() && !current.is_empty() {
            // `current` непуст — значит, предыдущий символ алфавитно-цифровой
            // (не-алфанумерик выше уже сбросил бы токен).
            let previous = characters[position - 1];
            let next_is_lower = characters
                .get(position + 1)
                .is_some_and(|next| next.is_lowercase());
            // Две границы CamelCase: «строчная→Заглавная» и конец
            // аббревиатуры «ЗАГЛАВНЫЕ→Заглавная+строчная» (HTTPСоединение).
            if previous.is_lowercase()
                || previous.is_numeric()
                || (previous.is_uppercase() && next_is_lower)
            {
                flush(&mut tokens, &mut current);
            }
        }
        current.extend(character.to_lowercase());
    }
    flush(&mut tokens, &mut current);
    tokens
}

/// Стем токена: кириллический токен стеммится русским Snowball, остальные —
/// английским. Выбор по письменности самого токена, а не по локали запроса:
/// корпус двуязычен прямо в заголовках («Свернуть (GroupBy)»), и один запрос
/// спокойно несёт слова обеих письменностей.
pub(crate) fn stem_token(token: &str) -> String {
    use rust_stemmers::{Algorithm, Stemmer};
    use std::sync::LazyLock;
    static RUSSIAN: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::Russian));
    static ENGLISH: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::English));
    let cyrillic = token
        .chars()
        .any(|character| ('\u{0400}'..='\u{04FF}').contains(&character));
    let stemmer: &Stemmer = if cyrillic { &RUSSIAN } else { &ENGLISH };
    stemmer.stem(token).into_owned()
}

/// Порог нечёткости от длины токена: короткие имена не размываются вовсе,
/// у средних допустима одна правка, у длинных — две.
pub(crate) fn fuzzy_cap(token_chars: usize) -> usize {
    match token_chars {
        0..=3 => 0,
        4..=8 => 1,
        _ => 2,
    }
}

/// Ограниченное расстояние Дамерау–Левенштейна (restricted: правки —
/// вставка, удаление, замена, транспозиция соседних). `None`, когда
/// расстояние заведомо больше `cap`: полоса ширины `2*cap+1` с ранним
/// выходом держит промах дешёвым — сверка идёт со словарём термов, а не со
/// страницами (#415).
pub(crate) fn bounded_damerau_levenshtein(a: &str, b: &str, cap: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > cap {
        return None;
    }
    if a == b {
        return Some(0);
    }
    if cap == 0 {
        return None;
    }
    let infinity = cap + 1;
    let width = b.len() + 1;
    let mut two_back = vec![infinity; width];
    let mut previous: Vec<usize> = (0..width)
        .map(|j| if j <= cap { j } else { infinity })
        .collect();
    for i in 1..=a.len() {
        let mut current = vec![infinity; width];
        let low = i.saturating_sub(cap);
        let high = (i + cap).min(b.len());
        if low == 0 && i <= cap {
            current[0] = i;
        }
        let mut row_minimum = current[0];
        for j in low.max(1)..=high {
            let substitution = usize::from(a[i - 1] != b[j - 1]);
            let mut value = previous[j - 1].saturating_add(substitution);
            value = value.min(previous[j].saturating_add(1));
            value = value.min(current[j - 1].saturating_add(1));
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                value = value.min(two_back[j - 2].saturating_add(1));
            }
            let value = value.min(infinity);
            current[j] = value;
            row_minimum = row_minimum.min(value);
        }
        // Вся полоса выше порога — дальше расстояние только растёт.
        if row_minimum > cap {
            return None;
        }
        two_back = std::mem::replace(&mut previous, current);
    }
    let distance = previous[b.len()];
    (distance <= cap).then_some(distance)
}

/// Поля документа для индексации. Веса полей фиксированы контрактом ядра:
/// заголовок 4.0, сигнатура 2.0, текст 1.0 — заголовок у справочных корпусов
/// несёт имя API и потому решает.
pub(crate) struct RetrievalFields<'a> {
    pub title: &'a str,
    pub signature: &'a str,
    pub body: &'a str,
}

/// Попадание запроса: номер документа в порядке подачи в `build` и локальная
/// BM25F-оценка. Порядок при равных оценках — по возрастанию номера, поэтому
/// выдача детерминирована при неизменном корпусе.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RetrievalHit {
    pub document: usize,
    pub score: f32,
}

/// Инвертированный индекс в памяти: термы — стемы токенов, TF взвешен полями
/// на этапе построения (BM25F с простой схемой weighted-field). Ключевание и
/// время жизни индекса — забота вызывающего (ADR-0029 п.11).
pub(crate) struct RetrievalIndex {
    vocabulary: std::collections::BTreeMap<String, usize>,
    postings: Vec<Vec<(u32, f32)>>,
    weighted_lengths: Vec<f32>,
    average_weighted_length: f32,
    raw_title_tokens: Vec<Vec<String>>,
}

const TITLE_WEIGHT: f32 = 4.0;
const SIGNATURE_WEIGHT: f32 = 2.0;
const BODY_WEIGHT: f32 = 1.0;
const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;
/// Точный (нестеммированный) токен запроса среди сырых токенов заголовка —
/// признак, что спрошено именно это имя; ADR-0037 требует ранжировать точное
/// выше стеммированного и нечёткого.
const EXACT_TITLE_BONUS: f32 = 1.2;
const EXPANSION_DISCOUNT: f32 = 0.9;
const FUZZY_DISCOUNT_ONE_EDIT: f32 = 0.7;
const FUZZY_DISCOUNT_TWO_EDITS: f32 = 0.5;

impl RetrievalIndex {
    pub fn build<'a>(documents: impl IntoIterator<Item = RetrievalFields<'a>>) -> RetrievalIndex {
        let mut vocabulary = std::collections::BTreeMap::new();
        let mut postings: Vec<Vec<(u32, f32)>> = Vec::new();
        let mut weighted_lengths = Vec::new();
        let mut raw_title_tokens = Vec::new();
        for (document, fields) in documents.into_iter().enumerate() {
            // BTreeMap, а не HashMap: идентификаторы термов и порядок
            // суммирования оценок обязаны не зависеть от случайности хешей,
            // иначе близкие оценки меняли бы порядок от процесса к процессу.
            let mut weighted_tf: std::collections::BTreeMap<String, f32> =
                std::collections::BTreeMap::new();
            let mut weighted_length = 0.0f32;
            for (text, weight) in [
                (fields.title, TITLE_WEIGHT),
                (fields.signature, SIGNATURE_WEIGHT),
                (fields.body, BODY_WEIGHT),
            ] {
                for token in tokenize(text) {
                    weighted_length += weight;
                    *weighted_tf.entry(stem_token(&token)).or_insert(0.0) += weight;
                }
            }
            raw_title_tokens.push(tokenize(fields.title));
            weighted_lengths.push(weighted_length);
            for (term, term_frequency) in weighted_tf {
                let term_id = match vocabulary.get(&term) {
                    Some(&term_id) => term_id,
                    None => {
                        let term_id = postings.len();
                        vocabulary.insert(term, term_id);
                        postings.push(Vec::new());
                        term_id
                    }
                };
                postings[term_id].push((document as u32, term_frequency));
            }
        }
        let document_count = weighted_lengths.len();
        let average_weighted_length = if document_count == 0 {
            1.0
        } else {
            (weighted_lengths.iter().sum::<f32>() / document_count as f32).max(1.0)
        };
        RetrievalIndex {
            vocabulary,
            postings,
            weighted_lengths,
            average_weighted_length,
            raw_title_tokens,
        }
    }

    /// `expansions[i]` — дополнительные термы-синонимы i-го токена запроса
    /// (уже стеммированные, например английские имена из двуязычного
    /// лексикона). Они прибавляют оценку документам, где найдены, и не
    /// штрафуют документы, где их нет.
    pub fn query(
        &self,
        query: &str,
        limit: usize,
        expansions: &[Vec<String>],
    ) -> Vec<RetrievalHit> {
        let document_count = self.weighted_lengths.len();
        if document_count == 0 || limit == 0 {
            return Vec::new();
        }
        let raw_tokens = tokenize(query);
        if raw_tokens.is_empty() {
            return Vec::new();
        }
        // Вклады термов, слитые по максимальному множителю: повтор слова в
        // запросе не должен удваивать его вес.
        let mut contributions: std::collections::BTreeMap<usize, f32> =
            std::collections::BTreeMap::new();
        let merge = |contributions: &mut std::collections::BTreeMap<usize, f32>,
                     term_id: usize,
                     multiplier: f32| {
            let entry = contributions.entry(term_id).or_insert(0.0);
            if multiplier > *entry {
                *entry = multiplier;
            }
        };
        for (position, raw_token) in raw_tokens.iter().enumerate() {
            let stem = stem_token(raw_token);
            if let Some(&term_id) = self.vocabulary.get(&stem) {
                merge(&mut contributions, term_id, 1.0);
            } else {
                let cap = fuzzy_cap(raw_token.chars().count());
                if cap > 0 {
                    let stem_chars = stem.chars().count();
                    let mut best: Option<(usize, Vec<usize>)> = None;
                    for (candidate, &term_id) in &self.vocabulary {
                        if candidate.chars().count().abs_diff(stem_chars) > cap {
                            continue;
                        }
                        let Some(distance) = bounded_damerau_levenshtein(&stem, candidate, cap)
                        else {
                            continue;
                        };
                        match &mut best {
                            Some((best_distance, term_ids)) => {
                                if distance < *best_distance {
                                    *best_distance = distance;
                                    term_ids.clear();
                                    term_ids.push(term_id);
                                } else if distance == *best_distance {
                                    term_ids.push(term_id);
                                }
                            }
                            None => best = Some((distance, vec![term_id])),
                        }
                    }
                    if let Some((distance, term_ids)) = best {
                        let multiplier = if distance <= 1 {
                            FUZZY_DISCOUNT_ONE_EDIT
                        } else {
                            FUZZY_DISCOUNT_TWO_EDITS
                        };
                        for term_id in term_ids {
                            merge(&mut contributions, term_id, multiplier);
                        }
                    }
                }
            }
            if let Some(extra_terms) = expansions.get(position) {
                for expansion in extra_terms {
                    if let Some(&term_id) = self.vocabulary.get(expansion) {
                        merge(&mut contributions, term_id, EXPANSION_DISCOUNT);
                    }
                }
            }
        }
        let mut scores: std::collections::BTreeMap<u32, f32> = std::collections::BTreeMap::new();
        for (term_id, multiplier) in contributions {
            let posting = &self.postings[term_id];
            let document_frequency = posting.len() as f32;
            let idf = (1.0
                + (document_count as f32 - document_frequency + 0.5) / (document_frequency + 0.5))
                .ln();
            for &(document, weighted_tf) in posting {
                let length_ratio =
                    self.weighted_lengths[document as usize] / self.average_weighted_length;
                let denominator = weighted_tf + BM25_K1 * (1.0 - BM25_B + BM25_B * length_ratio);
                let contribution = multiplier * idf * weighted_tf * (BM25_K1 + 1.0) / denominator;
                *scores.entry(document).or_insert(0.0) += contribution;
            }
        }
        let mut hits: Vec<RetrievalHit> = scores
            .into_iter()
            .map(|(document, mut score)| {
                let title_tokens = &self.raw_title_tokens[document as usize];
                if raw_tokens
                    .iter()
                    .any(|raw| title_tokens.iter().any(|title| title == raw))
                {
                    score *= EXACT_TITLE_BONUS;
                }
                RetrievalHit {
                    document: document as usize,
                    score,
                }
            })
            .collect();
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.document.cmp(&right.document))
        });
        hits.truncate(limit);
        hits
    }
}

/// Двуязычный словарь ru↔en, выводимый из заголовков корпуса установки вида
/// «ТаблицаЗначений.Свернуть (ValueTable.GroupBy)»: русская и английская
/// части режутся по точкам, сегменты сопоставляются по позиции, и каждый
/// русский токен сегмента отображается в термы парного английского сегмента.
/// Словарь расширяет русские запросы для корпусов с английскими заголовками
/// (kb-1ci), когда установка доступна (ADR-0037 п.4).
pub(crate) struct BilingualLexicon {
    map: std::collections::BTreeMap<String, Vec<String>>,
}

impl BilingualLexicon {
    pub fn from_titles<'a>(titles: impl IntoIterator<Item = &'a str>) -> BilingualLexicon {
        fn has_cyrillic(text: &str) -> bool {
            text.chars()
                .any(|character| ('\u{0400}'..='\u{04FF}').contains(&character))
        }
        let mut map: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for title in titles {
            let title = title.trim();
            let Some(open) = title.rfind('(') else {
                continue;
            };
            let Some(closed) = title[open..].find(')') else {
                continue;
            };
            let russian = title[..open].trim();
            let english = &title[open + 1..open + closed];
            if !has_cyrillic(russian) || has_cyrillic(english) {
                continue;
            }
            for (russian_segment, english_segment) in russian.split('.').zip(english.split('.')) {
                let english_terms: Vec<String> = tokenize(english_segment)
                    .iter()
                    .map(|token| stem_token(token))
                    .collect();
                if english_terms.is_empty() {
                    continue;
                }
                for russian_token in tokenize(russian_segment) {
                    if !has_cyrillic(&russian_token) {
                        continue;
                    }
                    let entry = map.entry(stem_token(&russian_token)).or_default();
                    for term in &english_terms {
                        if !entry.contains(term) {
                            entry.push(term.clone());
                        }
                    }
                }
            }
        }
        BilingualLexicon { map }
    }

    /// Расширения для `RetrievalIndex::query`: на каждый токен запроса —
    /// английские термы его сегмента (пусто для нерусских и ненайденных).
    pub fn expansions(&self, query: &str) -> Vec<Vec<String>> {
        tokenize(query)
            .iter()
            .map(|token| {
                self.map
                    .get(&stem_token(token))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexicon_maps_ru_tokens_to_en_segment_tokens() {
        let lexicon = BilingualLexicon::from_titles([
            "ТаблицаЗначений.Свернуть (ValueTable.GroupBy)",
            "Глобальный контекст.СтрНайти (Global context.StrFind)",
        ]);
        let expansions = lexicon.expansions("свернуть таблицу");
        assert_eq!(expansions.len(), 2);
        assert_eq!(expansions[0], vec![stem_token("group"), stem_token("by")]);
        assert_eq!(
            expansions[1],
            vec![stem_token("value"), stem_token("table")]
        );
        let empty = lexicon.expansions("group by");
        assert!(empty.iter().all(Vec::is_empty), "{empty:?}");
    }

    fn corpus(pairs: &[(&'static str, &'static str)]) -> RetrievalIndex {
        RetrievalIndex::build(pairs.iter().map(|(title, body)| RetrievalFields {
            title,
            signature: "",
            body,
        }))
    }

    #[test]
    fn word_order_and_morphology_do_not_matter() {
        let index = corpus(&[
            (
                "ТаблицаЗначений.Свернуть (ValueTable.GroupBy)",
                "Группирует строки таблицы значений по колонкам.",
            ),
            (
                "Массив.Удалить (Array.Delete)",
                "Удаляет элемент массива по индексу.",
            ),
        ]);
        let hits = index.query("свернуть таблицу значений", 5, &[]);
        assert_eq!(hits[0].document, 0, "{hits:?}");
        let hits = index.query("как удалить элемент массива", 5, &[]);
        assert_eq!(hits[0].document, 1, "{hits:?}");
    }

    #[test]
    fn title_match_outranks_body_match() {
        let index = corpus(&[
            ("Сочетания клавиш", "Команда Свернуть доступна в меню окна."),
            ("Свернуть (GroupBy)", "Группирует строки."),
        ]);
        let hits = index.query("Свернуть", 5, &[]);
        assert_eq!(hits[0].document, 1, "{hits:?}");
        assert!(hits[0].score > hits[1].score, "{hits:?}");
    }

    #[test]
    fn typo_falls_back_to_fuzzy_with_discount() {
        let index = corpus(&[(
            "Глобальный контекст.СтрНайти (Global context.StrFind)",
            "Ищет подстроку в строке.",
        )]);
        let exact = index.query("СтрНайти", 5, &[]);
        let typo = index.query("СтрНайтти", 5, &[]);
        assert_eq!(typo[0].document, 0, "{typo:?}");
        assert!(typo[0].score < exact[0].score, "{typo:?} против {exact:?}");
    }

    #[test]
    fn shorter_page_outranks_long_enumeration_for_equal_field() {
        let long_enumeration = "Свернуть окно раздел приложение команда список действие \
             панель клавиша сочетание переход навигация закладка история буфер обмена \
             копирование вставка удаление отмена повтор поиск замена печать предварительный \
             просмотр масштаб сетка линейка ориентация поля колонтитул страница разрыв";
        let index = corpus(&[
            ("Первая", "Свернуть группирует строки таблицы."),
            ("Вторая", long_enumeration),
        ]);
        let hits = index.query("Свернуть", 5, &[]);
        assert_eq!(hits[0].document, 0, "{hits:?}");
    }

    #[test]
    fn expansions_add_terms_without_penalizing_originals() {
        let index = corpus(&[
            (
                "Working with value tables",
                "How to group rows of a value table.",
            ),
            ("Working with arrays", "How to delete an element."),
        ]);
        let without = index.query("свернуть таблицу значений", 5, &[]);
        assert!(without.is_empty(), "{without:?}");
        let expansions = vec![
            vec![stem_token("group")],
            vec![stem_token("table")],
            vec![stem_token("value")],
        ];
        let with = index.query("свернуть таблицу значений", 5, &expansions);
        assert_eq!(with[0].document, 0, "{with:?}");
    }

    #[test]
    fn ties_break_by_document_index_deterministically() {
        let index = corpus(&[
            ("Свернуть", "Одинаковый текст."),
            ("Свернуть", "Одинаковый текст."),
        ]);
        let hits = index.query("Свернуть", 5, &[]);
        assert_eq!(
            hits.iter().map(|hit| hit.document).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn bounded_distance_respects_cap_and_transposition() {
        assert_eq!(
            bounded_damerau_levenshtein("стрнайтти", "стрнайти", 2),
            Some(1)
        );
        assert_eq!(
            bounded_damerau_levenshtein("свренуть", "свернуть", 2),
            Some(1)
        );
        assert_eq!(bounded_damerau_levenshtein("массив", "запрос", 2), None);
        assert_eq!(
            bounded_damerau_levenshtein("одинаковый", "одинаковый", 1),
            Some(0)
        );
        assert_eq!(bounded_damerau_levenshtein("кот", "котлета", 2), None);
        assert_eq!(fuzzy_cap(3), 0);
        assert_eq!(fuzzy_cap(4), 1);
        assert_eq!(fuzzy_cap(8), 1);
        assert_eq!(fuzzy_cap(9), 2);
    }

    #[test]
    fn stem_token_uses_script_matched_snowball() {
        assert_eq!(stem_token("таблицу"), stem_token("таблица"));
        assert_eq!(stem_token("значений"), stem_token("значения"));
        assert_eq!(stem_token("tables"), stem_token("table"));
        assert_ne!(stem_token("регистр"), stem_token("регламент"));
    }

    #[test]
    fn tokenize_splits_camel_case_including_cyrillic() {
        assert_eq!(
            tokenize("ТаблицаЗначений.Свернуть"),
            vec!["таблица", "значений", "свернуть"]
        );
        assert_eq!(
            tokenize("ValueTable.GroupBy"),
            vec!["value", "table", "group", "by"]
        );
        assert_eq!(tokenize("СтрНайти"), vec!["стр", "найти"]);
        assert_eq!(
            tokenize("как удалить элемент — массива?"),
            vec!["как", "удалить", "элемент", "массива"]
        );
        assert_eq!(tokenize("HTTPСоединение2"), vec!["http", "соединение2"]);
    }
}

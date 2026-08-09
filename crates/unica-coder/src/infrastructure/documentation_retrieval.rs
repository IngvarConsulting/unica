//! Лексическое ядро `unica.documentation.search` (ADR-0035): токенизация,
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

#[cfg(test)]
mod tests {
    use super::*;

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

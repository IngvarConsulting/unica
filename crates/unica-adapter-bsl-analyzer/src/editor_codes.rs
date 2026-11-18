//! Unica's wording for bsl-analyzer diagnostics in the editor.
//! The rule code stays intact. Rules without a message use neutral wording
//! based on the diagnostic severity.

use unica_diagnostics::model::{generic_editor_message, DiagnosticSeverity};

/// Analyzer rule and its Unica message.
const TABLE: &[(&str, &str)] = &[
    ("LineLength", "Строка длиннее допустимого"),
    (
        "MissingSpace",
        "Не хватает пробела у оператора или разделителя",
    ),
    (
        "QueryNestedFieldsByDot",
        "Поле запроса получено через несколько точек",
    ),
    ("AssignAliasFieldsInQuery", "У поля запроса нет псевдонима"),
    (
        "SpaceAtStartComment",
        "Комментарий не отделён пробелом от //",
    ),
    ("MagicNumber", "Число без имени в коде"),
    (
        "DuplicateStringLiteral",
        "Одинаковая строка повторяется в модуле",
    ),
    (
        "IfElseIfEndsWithElse",
        "Цепочка ИначеЕсли без завершающего Иначе",
    ),
    ("UsingThisForm", "Используется устаревшее ЭтаФорма"),
    (
        "CognitiveComplexity",
        "Метод трудно читать: высокая когнитивная сложность",
    ),
    ("UnusedParameters", "Параметр метода не используется"),
    (
        "NestedFunctionInParameters",
        "Вызов функции вложен в параметры другого вызова",
    ),
    ("ConsecutiveEmptyLines", "Несколько пустых строк подряд"),
    ("UsingServiceTag", "Служебная пометка в комментарии"),
    (
        "TransferringParametersBetweenClientAndServer",
        "Параметр передаётся между клиентом и сервером без необходимости",
    ),
    (
        "NestedStatements",
        "Слишком глубокая вложенность конструкций",
    ),
    ("MissingParameterDescription", "У параметра нет описания"),
    ("IfConditionComplexity", "Слишком сложное условие"),
    (
        "LogicalOrInTheWhereSectionOfQuery",
        "ИЛИ в условии запроса мешает индексу",
    ),
    ("SetPrivilegedMode", "Включается привилегированный режим"),
    ("SemicolonPresence", "Оператор не завершён точкой с запятой"),
    ("CyclomaticComplexity", "Слишком много ветвлений в методе"),
    ("DeprecatedMethodCall", "Вызывается устаревший метод"),
    ("RedundantAccessToObject", "Лишнее обращение к ЭтотОбъект"),
    ("MethodSize", "Метод слишком длинный"),
    (
        "LatinAndCyrillicSymbolInWord",
        "В слове смешаны латиница и кириллица",
    ),
    (
        "UnusedLocalVariable",
        "Локальная переменная не используется",
    ),
    (
        "MissingCodeTryCatchEx",
        "Исключение перехватывается без обработки",
    ),
    ("CodeOutOfRegion", "Код вне стандартной области"),
    (
        "IfElseDuplicatedCodeBlock",
        "Ветви условия повторяют один и тот же код",
    ),
    (
        "ExcessiveAutoTestCheck",
        "Лишняя проверка режима автотестирования",
    ),
    ("IncorrectLineBreak", "Перенос строки в неподходящем месте"),
    (
        "DeprecatedPlatformApi",
        "Используется устаревший программный интерфейс платформы",
    ),
    ("QueryParseError", "Текст запроса не разбирается"),
    ("TypeMismatch", "Тип значения не подходит"),
    ("UnresolvedField", "Поле не найдено у значения"),
    (
        "PublicMethodsDescription",
        "У экспортного метода нет описания",
    ),
    (
        "MissingVariablesDescription",
        "У переменной модуля нет описания",
    ),
    (
        "CanonicalSpellingKeywords",
        "Ключевое слово написано не в каноническом виде",
    ),
    (
        "JoinWithVirtualTable",
        "Соединение с виртуальной таблицей вместо её параметров",
    ),
    ("EmptyCodeBlock", "Пустой блок кода"),
    ("ParseError", "Код модуля не разбирается"),
];

pub(crate) fn editor_message(code: &str, severity: DiagnosticSeverity) -> &'static str {
    TABLE
        .iter()
        .find(|(rule, _)| *rule == code)
        .map_or_else(|| generic_editor_message(severity), |(_, message)| *message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_table_names_each_rule_once_with_neutral_wording() {
        let mut rules = BTreeSet::new();
        for (rule, message) in TABLE {
            assert!(rules.insert(rule), "{rule} is listed twice");
            assert!(
                !message.is_empty() && !message.contains(rule),
                "{rule}: {message}"
            );
        }
    }

    #[test]
    fn known_and_unlisted_rules_both_get_unica_messages() {
        assert_eq!(
            editor_message("LineLength", DiagnosticSeverity::Info),
            "Строка длиннее допустимого"
        );
        assert_eq!(
            editor_message("SomeFutureRule", DiagnosticSeverity::Warning),
            "Предупреждение анализа кода"
        );
    }
}

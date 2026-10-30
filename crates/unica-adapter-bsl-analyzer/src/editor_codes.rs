//! Unica's words for bsl-analyzer findings (#2176). The editor sees a Unica
//! code `bsl.<family>.<name>` and a fixed Unica message, never the
//! analyzer's rule name or text. The table starts with the rules most frequent
//! on 1С:УТ 11.5 (bsl-analyzer 0.2.86: the first 40 rules here are 96 % of its
//! findings); a rule not listed is told by its severity alone.

use unica_diagnostics::model::{DiagnosticSeverity, EditorFinding};

/// Analyzer rule, Unica code, Unica message.
const TABLE: &[(&str, &str, &str)] = &[
    (
        "LineLength",
        "bsl.style.line_length",
        "Строка длиннее допустимого",
    ),
    (
        "MissingSpace",
        "bsl.style.missing_space",
        "Не хватает пробела у оператора или разделителя",
    ),
    (
        "QueryNestedFieldsByDot",
        "bsl.query.nested_field_by_dot",
        "Поле запроса получено через несколько точек",
    ),
    (
        "AssignAliasFieldsInQuery",
        "bsl.query.missing_alias",
        "У поля запроса нет псевдонима",
    ),
    (
        "SpaceAtStartComment",
        "bsl.style.comment_space",
        "Комментарий не отделён пробелом от //",
    ),
    (
        "MagicNumber",
        "bsl.code.magic_number",
        "Число без имени в коде",
    ),
    (
        "DuplicateStringLiteral",
        "bsl.code.duplicate_string",
        "Одинаковая строка повторяется в модуле",
    ),
    (
        "IfElseIfEndsWithElse",
        "bsl.code.missing_else",
        "Цепочка ИначеЕсли без завершающего Иначе",
    ),
    (
        "UsingThisForm",
        "bsl.code.this_form",
        "Используется устаревшее ЭтаФорма",
    ),
    (
        "CognitiveComplexity",
        "bsl.complexity.cognitive",
        "Метод трудно читать: высокая когнитивная сложность",
    ),
    (
        "UnusedParameters",
        "bsl.code.unused_parameter",
        "Параметр метода не используется",
    ),
    (
        "NestedFunctionInParameters",
        "bsl.code.nested_call_argument",
        "Вызов функции вложен в параметры другого вызова",
    ),
    (
        "ConsecutiveEmptyLines",
        "bsl.style.empty_lines",
        "Несколько пустых строк подряд",
    ),
    (
        "UsingServiceTag",
        "bsl.style.service_tag",
        "Служебная пометка в комментарии",
    ),
    (
        "TransferringParametersBetweenClientAndServer",
        "bsl.client_server.parameter_transfer",
        "Параметр передаётся между клиентом и сервером без необходимости",
    ),
    (
        "NestedStatements",
        "bsl.complexity.nesting",
        "Слишком глубокая вложенность конструкций",
    ),
    (
        "MissingParameterDescription",
        "bsl.docs.parameter",
        "У параметра нет описания",
    ),
    (
        "IfConditionComplexity",
        "bsl.complexity.condition",
        "Слишком сложное условие",
    ),
    (
        "LogicalOrInTheWhereSectionOfQuery",
        "bsl.query.or_in_where",
        "ИЛИ в условии запроса мешает индексу",
    ),
    (
        "SetPrivilegedMode",
        "bsl.security.privileged_mode",
        "Включается привилегированный режим",
    ),
    (
        "SemicolonPresence",
        "bsl.style.semicolon",
        "Оператор не завершён точкой с запятой",
    ),
    (
        "CyclomaticComplexity",
        "bsl.complexity.cyclomatic",
        "Слишком много ветвлений в методе",
    ),
    (
        "DeprecatedMethodCall",
        "bsl.code.deprecated_method",
        "Вызывается устаревший метод",
    ),
    (
        "RedundantAccessToObject",
        "bsl.code.redundant_this_object",
        "Лишнее обращение к ЭтотОбъект",
    ),
    (
        "MethodSize",
        "bsl.complexity.method_size",
        "Метод слишком длинный",
    ),
    (
        "LatinAndCyrillicSymbolInWord",
        "bsl.style.mixed_alphabet",
        "В слове смешаны латиница и кириллица",
    ),
    (
        "UnusedLocalVariable",
        "bsl.code.unused_variable",
        "Локальная переменная не используется",
    ),
    (
        "MissingCodeTryCatchEx",
        "bsl.code.empty_exception_handler",
        "Исключение перехватывается без обработки",
    ),
    (
        "CodeOutOfRegion",
        "bsl.style.outside_region",
        "Код вне стандартной области",
    ),
    (
        "IfElseDuplicatedCodeBlock",
        "bsl.code.duplicate_branch",
        "Ветви условия повторяют один и тот же код",
    ),
    (
        "ExcessiveAutoTestCheck",
        "bsl.code.autotest_check",
        "Лишняя проверка режима автотестирования",
    ),
    (
        "IncorrectLineBreak",
        "bsl.style.line_break",
        "Перенос строки в неподходящем месте",
    ),
    (
        "DeprecatedPlatformApi",
        "bsl.platform.deprecated_api",
        "Используется устаревший программный интерфейс платформы",
    ),
    (
        "QueryParseError",
        "bsl.query.parse_error",
        "Текст запроса не разбирается",
    ),
    (
        "TypeMismatch",
        "bsl.typing.mismatch",
        "Тип значения не подходит",
    ),
    (
        "UnresolvedField",
        "bsl.typing.unresolved_field",
        "Поле не найдено у значения",
    ),
    (
        "PublicMethodsDescription",
        "bsl.docs.public_method",
        "У экспортного метода нет описания",
    ),
    (
        "MissingVariablesDescription",
        "bsl.docs.variable",
        "У переменной модуля нет описания",
    ),
    (
        "CanonicalSpellingKeywords",
        "bsl.style.keyword_spelling",
        "Ключевое слово написано не в каноническом виде",
    ),
    (
        "JoinWithVirtualTable",
        "bsl.query.join_virtual_table",
        "Соединение с виртуальной таблицей вместо её параметров",
    ),
    ("EmptyCodeBlock", "bsl.code.empty_block", "Пустой блок кода"),
    (
        "ParseError",
        "bsl.code.parse_error",
        "Код модуля не разбирается",
    ),
];

/// The Unica code and message of an analyzer finding.
pub(crate) fn editor_finding(code: &str, severity: DiagnosticSeverity) -> EditorFinding {
    TABLE.iter().find(|(rule, _, _)| *rule == code).map_or_else(
        || EditorFinding::generic(severity),
        |(_, code, message)| EditorFinding { code, message },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_table_names_each_rule_once_with_a_unica_code_and_text() {
        let rules = TABLE
            .iter()
            .map(|(rule, _, _)| *rule)
            .collect::<BTreeSet<_>>();
        let codes = TABLE
            .iter()
            .map(|(_, code, _)| *code)
            .collect::<BTreeSet<_>>();
        assert_eq!(rules.len(), TABLE.len(), "a rule is listed twice");
        assert_eq!(codes.len(), TABLE.len(), "a Unica code is used twice");
        for (rule, code, message) in TABLE {
            let parts = code.split('.').collect::<Vec<_>>();
            assert!(
                parts.len() == 3
                    && parts[0] == "bsl"
                    && parts[1..].iter().all(|part| {
                        !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                    }),
                "{code} is not bsl.<family>.<name>"
            );
            assert!(
                !message.is_empty() && !message.contains(rule),
                "{rule}: {message}"
            );
        }
    }

    #[test]
    fn a_listed_rule_speaks_unica_and_an_unlisted_one_only_its_severity() {
        let listed = editor_finding("LineLength", DiagnosticSeverity::Info);
        assert_eq!(listed.code, "bsl.style.line_length");
        let unlisted = editor_finding("SomeFutureRule", DiagnosticSeverity::Warning);
        assert_eq!(
            unlisted,
            EditorFinding::generic(DiagnosticSeverity::Warning)
        );
        assert_eq!(unlisted.code, "bsl.finding");
    }
}

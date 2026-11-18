//! Состояние поля разобранной справки, общее для статьи справки платформы
//! и документирующего комментария метода конфигурации.
//!
//! Разбор различает три состояния: раздела нет ([`Field::Empty`]), раздел
//! разобран ([`Field::Parsed`]) и раздел есть, но не разобран
//! ([`Field::Unparsed`]) — с причиной и сырым текстом. Пустое значение и
//! неразобранный раздел никогда не совпадают: иначе «параметров нет» стало
//! бы неотличимо от «параметры не разобрались», и вызов собрался бы без
//! обязательных аргументов.

/// Почему раздел остался неразобранным. Причина называет, чего в разделе не
/// нашлось, а не что потребителю делать: решение — за ним.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnparsedReason {
    /// В разделе «Параметры» нет ни одной рубрики параметра.
    NoParameterRubric,
    /// В разделе нет строки «Тип: …», которой он типизируется.
    NoTypeLine,
    /// В разделе «Пример» нет блока кода.
    NoCodeBlock,
    /// В разделе «Доступность» не распознано ни одного контекста.
    NoContext,
    /// В разделе «Использование в версии» нет версии появления.
    NoVersion,
    /// Раздел «Использование» говорит не о доступе на чтение и запись.
    UnknownUse,
    /// В разделе «Синтаксис» нет строки вызова.
    NoCallLine,
    /// В разделе «Параметры» комментария нет ни одной записи вида
    /// «Имя - Тип - описание»: имени параметра в разделе не нашлось.
    NoParameterEntry,
    /// В разделе «Возвращаемое значение» комментария нет текста типа.
    NoReturnType,
    /// В разделе нет ни одной строки, кроме его заголовка.
    EmptySection,
}

impl UnparsedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            UnparsedReason::NoParameterRubric => "no-parameter-rubric",
            UnparsedReason::NoTypeLine => "no-type-line",
            UnparsedReason::NoCodeBlock => "no-code-block",
            UnparsedReason::NoContext => "no-context",
            UnparsedReason::NoVersion => "no-version",
            UnparsedReason::UnknownUse => "unknown-use",
            UnparsedReason::NoCallLine => "no-call-line",
            UnparsedReason::NoParameterEntry => "no-parameter-entry",
            UnparsedReason::NoReturnType => "no-return-type",
            UnparsedReason::EmptySection => "empty-section",
        }
    }
}

/// Значение раздела статьи.
///
/// `Empty` и `Parsed` различают «раздела в статье нет» и «раздел разобран»;
/// `Unparsed` держит причину и сырой текст раздела, чтобы потребитель мог
/// показать исходное вместо выдуманного. Пустое значение и неразобранный
/// раздел никогда не совпадают: иначе «параметров нет» стало бы неотличимо
/// от «параметры не разобрались».
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Field<T> {
    #[default]
    Empty,
    Parsed(T),
    Unparsed {
        reason: UnparsedReason,
        text: String,
    },
}

impl<T> Field<T> {
    pub fn parsed(&self) -> Option<&T> {
        match self {
            Field::Parsed(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Field::Empty)
    }

    /// Разобранное значение либо названная причина: раздел с текстом, из
    /// которого значения не вышло, остаётся неразобранным, а не пустым.
    pub(crate) fn from(value: Option<T>, reason: UnparsedReason, text: &str) -> Self {
        match value {
            Some(value) => Field::Parsed(value),
            None if text.trim().is_empty() => Field::Empty,
            None => Field::Unparsed {
                reason,
                text: text.to_owned(),
            },
        }
    }
}

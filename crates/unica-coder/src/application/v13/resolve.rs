//! Аварийный двусторонний мост между адресным пространством и файловой
//! раскладкой.
//!
//! Обычному вызывающему путь не нужен: логический слой существует ровно затем,
//! чтобы модель не работала с файлами. Путь нужен ровно в двух случаях — он
//! пришёл снаружи (из диффа, лога сборки, трассы) и его надо перевести в адрес,
//! либо файл действительно нужно открыть вне Unica. Оба случая редки, поэтому
//! ответ здесь точный или его нет: догадок этот инструмент не делает.

use crate::domain::address::QualifiedAddress;
use crate::domain::refusal::RefusalCode;
use serde::Serialize;

const MAX_PATH_CHARS: usize = 4_096;

/// Откуда пришёл вопрос. Ровно одна сторона: спрашивать обе разом бессмысленно,
/// а не спрашивать ни одной — нечего разрешать.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolveRequest {
    /// Адрес известен, нужен файл.
    Address(QualifiedAddress),
    /// Путь пришёл снаружи, нужен адрес.
    Path(String),
}

impl ResolveRequest {
    pub(crate) fn new(at: Option<&str>, path: Option<&str>) -> Result<Self, ResolveError> {
        match (at, path) {
            (Some(_), Some(_)) => Err(ResolveError::new(
                RefusalCode::BadValue,
                "resolve takes either `at` or `path`, not both",
            )),
            (None, None) => Err(ResolveError::new(
                RefusalCode::BadValue,
                "resolve requires `at` or `path`",
            )),
            (Some(at), None) => {
                let address = QualifiedAddress::parse(at)
                    .map_err(|error| ResolveError::new(RefusalCode::BadValue, error.to_string()))?;
                Ok(Self::Address(address))
            }
            (None, Some(path)) => {
                let path = path.trim();
                if path.is_empty() {
                    return Err(ResolveError::new(
                        RefusalCode::BadValue,
                        "resolve path must not be empty",
                    ));
                }
                if path.chars().count() > MAX_PATH_CHARS {
                    return Err(ResolveError::new(
                        RefusalCode::BadValue,
                        format!("resolve path must not exceed {MAX_PATH_CHARS} characters"),
                    ));
                }
                Ok(Self::Path(path.to_string()))
            }
        }
    }
}

/// Есть ли у предмета строки — закрытый признак, а не отсутствие поля.
///
/// Если поля просто нет, читатель не отличит «здесь строк не бывает» от
/// «строки не посчитались»: либо решит, что весь файл и есть предмет, либо
/// пойдёт искать причину, которой нет.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum ResolvedLines {
    /// Источник строчный, диапазон настоящий: BSL.
    Range { from: usize, to: usize },
    /// Источник древовидный. Физические строки существуют, но единицей не
    /// являются и поедут при переформатировании: обещать их значило бы
    /// обещать то, что развалится.
    NotLineBased,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedSource {
    at: String,
    kind: String,
    /// Путь относительно корня набора исходников.
    path: String,
    lines: ResolvedLines,
}

impl ResolvedSource {
    pub(crate) fn new(
        at: impl Into<String>,
        kind: impl Into<String>,
        path: impl Into<String>,
        lines: ResolvedLines,
    ) -> Self {
        Self {
            at: at.into(),
            kind: kind.into(),
            path: path.into(),
            lines,
        }
    }

    pub(crate) fn address(&self) -> &str {
        &self.at
    }

    #[cfg(test)]
    pub(crate) const fn lines(&self) -> ResolvedLines {
        self.lines
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolveError {
    code: RefusalCode,
    message: String,
}

impl ResolveError {
    pub(crate) fn new(code: RefusalCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> RefusalCode {
        self.code
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_takes_exactly_one_side_of_the_bridge() {
        assert_eq!(
            ResolveRequest::new(Some("main:Configuration"), Some("src/x.xml"))
                .unwrap_err()
                .code(),
            RefusalCode::BadValue
        );
        assert_eq!(
            ResolveRequest::new(None, None).unwrap_err().code(),
            RefusalCode::BadValue
        );
        assert!(matches!(
            ResolveRequest::new(Some("main:Configuration"), None).unwrap(),
            ResolveRequest::Address(_)
        ));
        assert!(matches!(
            ResolveRequest::new(None, Some(" src/x.xml ")).unwrap(),
            ResolveRequest::Path(path) if path == "src/x.xml"
        ));
    }

    #[test]
    fn absent_lines_are_named_rather_than_omitted() {
        let tree = ResolvedSource::new(
            "main:Catalog.Товары.Attribute.Цена",
            "Attribute",
            "src/Catalogs/Товары.xml",
            ResolvedLines::NotLineBased,
        );
        let value = serde_json::to_value(&tree).unwrap();
        assert_eq!(value["lines"], serde_json::json!({"state": "notLineBased"}));

        let lined = ResolvedSource::new(
            "main:CommonModule.Продажи.Method.ПередЗаписью",
            "Method",
            "src/CommonModules/Продажи/Ext/Module.bsl",
            ResolvedLines::Range { from: 12, to: 40 },
        );
        assert_eq!(
            serde_json::to_value(&lined).unwrap()["lines"],
            serde_json::json!({"state": "range", "from": 12, "to": 40})
        );
    }
}

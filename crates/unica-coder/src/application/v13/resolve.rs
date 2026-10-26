//! Аварийный двусторонний мост между адресным пространством и файловой
//! раскладкой.
//!
//! Обычному вызывающему путь не нужен: логический слой существует ровно затем,
//! чтобы модель не работала с файлами. Путь нужен ровно в двух случаях — он
//! пришёл снаружи (из диффа, лога сборки, трассы) и его надо перевести в адрес,
//! либо файл действительно нужно открыть вне Unica. Оба случая редки, поэтому
//! ответ здесь точный или его нет: догадок этот инструмент не делает.

use serde::Serialize;
use unica_address::address::QualifiedAddress;
use unica_refusal::RefusalCode;

/// Откуда пришёл вопрос. Ровно одна сторона: спрашивать обе разом бессмысленно,
/// а не спрашивать ни одной — нечего разрешать.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolveRequest {
    /// Адрес известен, нужен файл.
    Address(QualifiedAddress),
    /// Путь пришёл снаружи, нужен адрес.
    Path(String),
    /// Пакет путей и строк пришёл снаружи — обычно совпадения поиска
    /// средствами хоста. Каждый элемент разбирается сам: неверный элемент
    /// отказывает только себе, а не всему пакету.
    Paths(Vec<Result<PathLine, ResolveError>>),
}

impl ResolveRequest {
    pub(crate) fn new(
        at: Option<&str>,
        path: Option<&str>,
        paths: Option<&[&str]>,
    ) -> Result<Self, ResolveError> {
        match (at, path, paths) {
            (Some(at), None, None) => {
                let address = QualifiedAddress::parse(at)
                    .map_err(|error| ResolveError::new(RefusalCode::BadValue, error.to_string()))?;
                Ok(Self::Address(address))
            }
            (None, Some(path), None) => {
                let path = path.trim();
                if path.is_empty() {
                    return Err(ResolveError::new(
                        RefusalCode::BadValue,
                        "resolve path must not be empty",
                    ));
                }
                Ok(Self::Path(path.to_string()))
            }
            (None, None, Some(paths)) => {
                if paths.is_empty() {
                    return Err(ResolveError::new(
                        RefusalCode::BadValue,
                        "resolve paths must name at least one path",
                    ));
                }
                Ok(Self::Paths(
                    paths.iter().map(|item| PathLine::parse(item)).collect(),
                ))
            }
            (None, None, None) => Err(ResolveError::new(
                RefusalCode::BadValue,
                "resolve requires `at`, `path` or `paths`",
            )),
            _ => Err(ResolveError::new(
                RefusalCode::BadValue,
                "resolve takes exactly one of `at`, `path` or `paths`",
            )),
        }
    }
}

/// Один элемент пакета: путь и, если задана, физическая строка файла,
/// нумерованная с единицы, как у `grep -n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathLine {
    path: String,
    line: Option<usize>,
}

impl PathLine {
    /// `путь` или `путь:строка`. Строкой считается только последний
    /// сегмент из одних цифр, поэтому `C:\x\Module.bsl` остаётся путём.
    /// Числовой сегмент не в конце — строка совпадения с колонкой или
    /// текстом, как её печатают `grep -n` и `rg --vimgrep`, — не угадывается,
    /// а отклоняется: молча принятый за часть пути, он дал бы ложный
    /// `not_found`.
    pub(crate) fn parse(item: &str) -> Result<Self, ResolveError> {
        let item = item.trim();
        let numeric =
            |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
        let segments = item.split(':').collect::<Vec<_>>();
        if segments.len() > 2 && segments[1..segments.len() - 1].iter().any(|s| numeric(s)) {
            return Err(ResolveError::new(
                RefusalCode::BadValue,
                format!(
                    "`{item}` carries more than a line number; pass `path:line` without a column or the match text"
                ),
            ));
        }
        let (path, line) = match item.rsplit_once(':') {
            Some((path, line)) if numeric(line) => {
                let line = line
                    .parse::<usize>()
                    .ok()
                    .filter(|line| *line > 0)
                    .ok_or_else(|| {
                        ResolveError::new(
                            RefusalCode::BadValue,
                            format!("`{item}` names no file line; lines are numbered from 1"),
                        )
                    })?;
                (path.trim_end(), Some(line))
            }
            _ => (item, None),
        };
        if path.is_empty() {
            return Err(ResolveError::new(
                RefusalCode::BadValue,
                "resolve paths items must name a path",
            ));
        }
        Ok(Self {
            path: path.to_string(),
            line,
        })
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) const fn line(&self) -> Option<usize> {
        self.line
    }

    /// Одно написание пути на оба разделителя: пакет разрешает такой путь
    /// один раз, сколько бы строк и повторов в нём ни было.
    pub(crate) fn key(&self) -> String {
        self.path.replace('\\', "/")
    }
}

/// Где лежит физическая строка модуля BSL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModuleLine {
    /// Внутри собственного диапазона метода — того, что `resolve` называет
    /// для `Method`: от первой аннотации до строки окончания.
    Method(QualifiedAddress),
    /// Между методами: объявления модуля, области, комментарии.
    OutsideMethod,
    /// За концом файла, в котором `lines` строк.
    PastEnd { lines: usize },
}

/// Ответ пакета на один элемент: логический адрес и вид предмета. Признак
/// `line` есть ровно у элементов со строкой и закрыт: строка внутри метода,
/// вне всех методов (адрес тогда — модуль) или в древовидном файле, где
/// строка предметом не является (адрес — объект файла).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ResolvedPath {
    at: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<LinePlace>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum LinePlace {
    InMethod,
    OutsideMethod,
    NotLineBased,
}

impl ResolvedPath {
    pub(crate) fn new(
        at: impl Into<String>,
        kind: impl Into<String>,
        line: Option<LinePlace>,
    ) -> Self {
        Self {
            at: at.into(),
            kind: kind.into(),
            line,
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

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn at(&self) -> &str {
        &self.at
    }

    pub(crate) fn kind(&self) -> &str {
        &self.kind
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
            ResolveRequest::new(Some("main:Configuration"), Some("src/x.xml"), None)
                .unwrap_err()
                .code(),
            RefusalCode::BadValue
        );
        for (at, path) in [
            (Some("main:Configuration"), None),
            (None, Some("src/x.xml")),
        ] {
            assert_eq!(
                ResolveRequest::new(at, path, Some(&["src/x.xml"]))
                    .unwrap_err()
                    .code(),
                RefusalCode::BadValue
            );
        }
        assert_eq!(
            ResolveRequest::new(None, None, None).unwrap_err().code(),
            RefusalCode::BadValue
        );
        assert_eq!(
            ResolveRequest::new(None, None, Some(&[]))
                .unwrap_err()
                .code(),
            RefusalCode::BadValue
        );
        assert!(matches!(
            ResolveRequest::new(Some("main:Configuration"), None, None).unwrap(),
            ResolveRequest::Address(_)
        ));
        assert!(matches!(
            ResolveRequest::new(None, Some(" src/x.xml "), None).unwrap(),
            ResolveRequest::Path(path) if path == "src/x.xml"
        ));
        assert!(matches!(
            ResolveRequest::new(None, None, Some(&["src/x.xml"])).unwrap(),
            ResolveRequest::Paths(items) if items.len() == 1
        ));
    }

    #[test]
    fn resolve_paths_items_split_a_trailing_line_and_refuse_only_themselves() {
        let parsed = |item: &str| {
            PathLine::parse(item).map(|entry| (entry.path().to_string(), entry.line()))
        };
        assert_eq!(parsed(" src/a.bsl:12 "), Ok(("src/a.bsl".into(), Some(12))));
        assert_eq!(parsed("src/a.bsl"), Ok(("src/a.bsl".into(), None)));
        assert_eq!(
            parsed(r"C:\w\src\a.bsl"),
            Ok((r"C:\w\src\a.bsl".into(), None))
        );
        assert_eq!(
            parsed(r"C:\w\src\a.bsl:7"),
            Ok((r"C:\w\src\a.bsl".into(), Some(7)))
        );
        for refused in [
            "",
            "  ",
            ":12",
            "src/a.bsl:0",
            "src/a.bsl:12:5",
            "src/a.bsl:12:    Сообщить(1);",
            r"C:\w\a.bsl:12:5:Сообщить(1)",
            "src/a.bsl:99999999999999999999999",
        ] {
            assert_eq!(
                PathLine::parse(refused).unwrap_err().code(),
                RefusalCode::BadValue,
                "{refused:?}"
            );
        }
        let ResolveRequest::Paths(items) =
            ResolveRequest::new(None, None, Some(&["src/a.bsl:0", "src/a.bsl:3"])).unwrap()
        else {
            panic!("paths is a batch request");
        };
        assert!(items[0].is_err());
        assert_eq!(items[1].as_ref().unwrap().line(), Some(3));
        assert_eq!(
            PathLine::parse(r"src\a.bsl").unwrap().key(),
            PathLine::parse("src/a.bsl:4").unwrap().key()
        );
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

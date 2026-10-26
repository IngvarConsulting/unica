//! Маршрут LSP в протоколе демона: дверь редактора в тот же демон.
//!
//! Методы, уведомления и исходы — закрытые варианты, как и остальной
//! протокол v5. Тексты документов и адреса передаются как есть; их смысл
//! разбирает служба LSP за портом [`crate::lsp_lane::LspHost`].

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use uuid::Uuid;

/// Длиннейший адрес документа или рабочей папки.
const MAX_LSP_URI_BYTES: usize = 4 * 1024;
/// Длина пространства — сегмента адреса `unica://`.
const MAX_LSP_SPACE_BYTES: usize = 64;
/// Рабочих папок у одного соединения.
const MAX_LSP_WORKSPACE_FOLDERS: usize = 64;
const MAX_LSP_FOLDER_NAME_BYTES: usize = 1024;
const MAX_LSP_LANGUAGE_ID_BYTES: usize = 64;
/// Сообщение уведомления редактору.
const MAX_LSP_LOG_MESSAGE_BYTES: usize = 4 * 1024;

/// Сессия LSP: выдаётся демоном на `lsp_open` и живёт, пока открыто
/// соединение, на котором она открыта.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct V5LspSessionId(String);

impl V5LspSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id names a session and is checked without carrying it into the
    /// error: a refused frame's text reaches logs.
    pub(super) fn validate(&self) -> Result<(), String> {
        let canonical = Uuid::parse_str(&self.0).is_ok_and(|parsed| {
            parsed.get_version() == Some(uuid::Version::Random)
                && parsed.get_variant() == uuid::Variant::RFC4122
                && parsed.hyphenated().to_string() == self.0
        });
        if canonical {
            Ok(())
        } else {
            Err("v5 lsp session is not a canonical UUIDv4".to_string())
        }
    }
}

impl Default for V5LspSessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspWorkspaceFolder {
    pub uri: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspRange {
    pub start: V5LspPosition,
    pub end: V5LspPosition,
}

/// Запрос редактора, на который ждут ответа.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum V5LspCall {
    #[serde(rename = "textDocument/hover")]
    Hover {
        uri: String,
        position: V5LspPosition,
    },
    /// Текст документа по адресу `unica://`, который редактор открывает
    /// без файла на диске.
    #[serde(rename = "workspace/textDocumentContent")]
    TextDocumentContent { uri: String },
    #[serde(rename = "textDocument/documentSymbol")]
    DocumentSymbol { uri: String },
    #[serde(rename = "textDocument/foldingRange")]
    FoldingRange { uri: String },
    #[serde(rename = "textDocument/semanticTokens/full")]
    SemanticTokensFull { uri: String },
    #[serde(rename = "textDocument/signatureHelp")]
    SignatureHelp {
        uri: String,
        position: V5LspPosition,
    },
    #[serde(rename = "textDocument/completion")]
    Completion {
        uri: String,
        position: V5LspPosition,
    },
    /// Элемент, который служба выдала в `completion`, как его вернул
    /// редактор: документ служба записала в его `data`.
    #[serde(rename = "completionItem/resolve")]
    CompletionResolve { item: Value },
    #[serde(rename = "textDocument/definition")]
    Definition {
        uri: String,
        position: V5LspPosition,
    },
    #[serde(rename = "textDocument/references")]
    References {
        uri: String,
        position: V5LspPosition,
    },
    #[serde(rename = "textDocument/prepareCallHierarchy")]
    PrepareCallHierarchy {
        uri: String,
        position: V5LspPosition,
    },
    /// Элемент иерархии, который служба выдала в `prepareCallHierarchy`.
    #[serde(rename = "callHierarchy/incomingCalls")]
    IncomingCalls { item: Value },
    #[serde(rename = "callHierarchy/outgoingCalls")]
    OutgoingCalls { item: Value },
}

impl V5LspCall {
    /// Документ, к состоянию которого относится ответ.
    pub fn document_uri(&self) -> Option<&str> {
        match self {
            Self::Hover { uri, .. }
            | Self::TextDocumentContent { uri }
            | Self::DocumentSymbol { uri }
            | Self::FoldingRange { uri }
            | Self::SemanticTokensFull { uri }
            | Self::SignatureHelp { uri, .. }
            | Self::Completion { uri, .. }
            | Self::Definition { uri, .. }
            | Self::References { uri, .. }
            | Self::PrepareCallHierarchy { uri, .. } => Some(uri),
            Self::CompletionResolve { .. }
            | Self::IncomingCalls { .. }
            | Self::OutgoingCalls { .. } => None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::CompletionResolve { item }
            | Self::IncomingCalls { item }
            | Self::OutgoingCalls { item }
                if item.is_object() =>
            {
                Ok(())
            }
            Self::CompletionResolve { .. }
            | Self::IncomingCalls { .. }
            | Self::OutgoingCalls { .. } => Err("v5 lsp item must be an object".to_string()),
            _ => validate_uri(self.document_uri().unwrap_or_default()),
        }
    }
}

/// Одна правка `didChange`: без диапазона — весь текст документа.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspContentChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<V5LspRange>,
    pub text: String,
}

/// Уведомление редактора о документах. Порядок уведомлений одной сессии
/// сохраняется: они идут по соединению, открывшему сессию.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum V5LspNotification {
    #[serde(rename = "textDocument/didOpen")]
    DidOpen {
        uri: String,
        language_id: String,
        version: i32,
        text: String,
    },
    #[serde(rename = "textDocument/didChange")]
    DidChange {
        uri: String,
        version: i32,
        changes: Vec<V5LspContentChange>,
    },
    #[serde(rename = "textDocument/didClose")]
    DidClose { uri: String },
}

impl V5LspNotification {
    pub fn document_uri(&self) -> &str {
        match self {
            Self::DidOpen { uri, .. } | Self::DidChange { uri, .. } | Self::DidClose { uri } => uri,
        }
    }

    /// Примет ли демон это уведомление. Frontend проверяет его до отправки:
    /// отвергнутый кадр закрыл бы соединение, а с ним и сессию.
    pub fn validate(&self) -> Result<(), String> {
        validate_uri(self.document_uri())?;
        match self {
            Self::DidOpen { language_id, .. } => {
                validate_short_text(language_id, MAX_LSP_LANGUAGE_ID_BYTES, "v5 lsp language id")
            }
            Self::DidChange { .. } | Self::DidClose { .. } => Ok(()),
        }
    }
}

// Тексты документов — исходники пользователя; в журнал они не попадают.
impl fmt::Debug for V5LspNotification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DidOpen { version, .. } => formatter
                .debug_struct("V5LspDidOpen")
                .field("version", version)
                .finish_non_exhaustive(),
            Self::DidChange {
                version, changes, ..
            } => formatter
                .debug_struct("V5LspDidChange")
                .field("version", version)
                .field("changes", &changes.len())
                .finish_non_exhaustive(),
            Self::DidClose { .. } => formatter.write_str("V5LspDidClose"),
        }
    }
}

/// Как служба принимает тексты документов.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5LspTextSync {
    None,
    Full,
    Incremental,
}

/// Возможности, которые служба готова обслужить сейчас.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspCapabilities {
    pub text_document_sync: V5LspTextSync,
    pub hover: bool,
    /// `workspace/textDocumentContent` для адресов `unica://`.
    pub text_document_content: bool,
    pub document_symbols: bool,
    pub folding_ranges: bool,
    pub signature_help: bool,
    /// `completion` с `completionItem/resolve`.
    pub completion: bool,
    pub definition: bool,
    pub references: bool,
    pub call_hierarchy: bool,
    /// Легенда `semanticTokens/full`, если служба их выдаёт.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_tokens_legend: Option<Value>,
}

impl V5LspCapabilities {
    pub const NONE: Self = Self {
        text_document_sync: V5LspTextSync::None,
        hover: false,
        text_document_content: false,
        document_symbols: false,
        folding_ranges: false,
        signature_help: false,
        completion: false,
        definition: false,
        references: false,
        call_hierarchy: false,
        semantic_tokens_legend: None,
    };
}

/// Чем стала одна рабочая папка соединения: пространством адресов
/// `unica://<space>/...` или отказом. Ровно одно из двух.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct V5LspWorkspaceSpace {
    /// Адрес папки ровно в том виде, в каком его прислал редактор.
    pub folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<V5LspFailure>,
}

impl V5LspWorkspaceSpace {
    pub fn opened(folder: impl Into<String>, space: impl Into<String>) -> Self {
        Self {
            folder: folder.into(),
            space: Some(space.into()),
            failure: None,
        }
    }

    pub fn refused(folder: impl Into<String>, failure: V5LspFailure) -> Self {
        Self {
            folder: folder.into(),
            space: None,
            failure: Some(failure),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_uri(&self.folder)?;
        match (&self.space, self.failure) {
            (Some(space), None) => validate_space(space),
            (None, Some(_)) => Ok(()),
            _ => Err("v5 lsp workspace space names exactly one of space and failure".to_string()),
        }
    }
}

/// Ответ службы на открытие сессии.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V5LspOpening {
    pub capabilities: V5LspCapabilities,
    /// По одной записи на рабочую папку, в порядке папок.
    pub workspaces: Vec<V5LspWorkspaceSpace>,
}

/// Почему запрос не получил ответа. Закрытый набор кодов Unica: поставщик
/// анализа сюда не просачивается.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5LspFailure {
    /// Метод не входит в объявленные возможности.
    NotSupported,
    /// Сессия не открыта или уже закрыта.
    SessionUnknown,
    /// Запрос с этим номером уже исполняется в сессии.
    DuplicateRequest,
    /// Очередь LSP полна.
    Busy,
    DeadlineExceeded,
    /// Служба не смогла ответить.
    Unavailable,
    /// Рабочая папка не открылась как рабочая область Unica.
    WorkspaceUnavailable,
    /// Адрес не называет модуль, доступный этому соединению.
    DocumentUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum V5LspOutcome {
    /// Markdown подсказки; `None` — подсказки нет.
    Hover {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contents: Option<String>,
    },
    /// Текст документа.
    Text {
        text: String,
    },
    /// Результат метода LSP, который построил код Unica; `null` — ответа нет.
    Json {
        value: Value,
    },
    /// Запрос отменён до ответа; его результат не выдаётся.
    Cancelled {},
    Failed {
        failure: V5LspFailure,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5LspMessageLevel {
    Error,
    Warning,
    Info,
}

/// Уведомление демона редактору.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum V5LspServerNotification {
    LogMessage {
        level: V5LspMessageLevel,
        message: String,
    },
}

impl V5LspServerNotification {
    pub(super) fn validate(&self) -> Result<(), String> {
        match self {
            Self::LogMessage { message, .. } if message.len() > MAX_LSP_LOG_MESSAGE_BYTES => {
                Err("v5 lsp log message exceeds its limit".to_string())
            }
            Self::LogMessage { .. } => Ok(()),
        }
    }
}

pub(super) fn validate_open(folders: &[V5LspWorkspaceFolder]) -> Result<(), String> {
    if folders.len() > MAX_LSP_WORKSPACE_FOLDERS {
        return Err("v5 lsp session has too many workspace folders".to_string());
    }
    for folder in folders {
        validate_uri(&folder.uri)?;
        if folder.name.len() > MAX_LSP_FOLDER_NAME_BYTES
            || folder.name.chars().any(char::is_control)
        {
            return Err("v5 lsp workspace folder name is not short text".to_string());
        }
    }
    Ok(())
}

pub(super) fn validate_call(call: &V5LspCall) -> Result<(), String> {
    call.validate()
}

pub(super) fn validate_notification(notification: &V5LspNotification) -> Result<(), String> {
    notification.validate()
}

/// Пространство — сегмент адреса `unica://`: только буквы, цифры, `-`, `_`.
fn validate_space(space: &str) -> Result<(), String> {
    if space.is_empty()
        || space.len() > MAX_LSP_SPACE_BYTES
        || !space
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("v5 lsp workspace space is not an address segment".to_string());
    }
    Ok(())
}

fn validate_uri(uri: &str) -> Result<(), String> {
    validate_short_text(uri, MAX_LSP_URI_BYTES, "v5 lsp document uri")
}

fn validate_short_text(value: &str, limit: usize, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(format!("{field} must be non-empty short text"));
    }
    Ok(())
}

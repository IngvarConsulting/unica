//! The existing pure View refusal and canonical-result conversion.
use crate::invocation::DomainResult;
use serde_json::Value;
use unica_refusal::{RefusalCode, RefusalDetail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewError {
    code: RefusalCode,
    detail: Option<RefusalDetail>,
    message: String,
    next: Vec<Value>,
}

impl ViewError {
    pub fn new(code: RefusalCode, message: impl Into<String>) -> Self {
        Self {
            code,
            detail: None,
            message: message.into(),
            next: Vec::new(),
        }
    }

    /// Отказ с уточнением: код берётся из уточнения, поэтому пару «код и не
    /// его уточнение» составить нельзя.
    pub fn detailed(detail: RefusalDetail, message: impl Into<String>) -> Self {
        Self {
            code: detail.code(),
            detail: Some(detail),
            message: message.into(),
            next: Vec::new(),
        }
    }

    pub const fn code(&self) -> RefusalCode {
        self.code
    }

    pub const fn detail(&self) -> Option<RefusalDetail> {
        self.detail
    }

    /// Маршрут к следующему вопросу. Отказ, который знает альтернативу, обязан
    /// её назвать — иначе агент останавливается там, где путь есть. Каждый
    /// вызов добавляет маршрут в конец списка.
    pub fn set_next(&mut self, next: Value) {
        self.next.push(next);
    }

    pub fn next(&self) -> &[Value] {
        &self.next
    }
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub fn error_result(at: Option<String>, error: ViewError) -> DomainResult {
    let mut result = match error.detail {
        Some(detail) => DomainResult::canonical_rejection_detailed(at, detail, error.message),
        None => DomainResult::canonical_rejection(at, error.code, error.message),
    };
    result.next.extend(error.next);
    result
}

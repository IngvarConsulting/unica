//! Text needed by hosts that omit structured results on a domain refusal.
//! Admission and transport share the builders and account for JSON escaping.

use crate::invocation::DomainResult;
use crate::result_size::{canonical_result_size, CanonicalResultSizeError, MAX_TASK_RECORD_BYTES};
use crate::source_text_budget::maximum_delivery_refusal;
use serde::Serialize;
use std::convert::Infallible;
use std::io::{self, Write};

pub const MAX_ERROR_DIAGNOSTIC_TEXT_BYTES: usize = 4_096;
pub const ERROR_TEXT_TRUNCATED: &str = "\n[truncated; full diagnostics are in structuredContent]";

// Keys, content-block wrappers and the largest Task metadata, excluding the
// canonical result and quoted text strings. The transport tests check this
// reserve against real SDK serialization, including cancellation and maxima.
pub const ERROR_DELIVERY_ENVELOPE_BYTES: usize = 1_024;

pub struct ErrorDeliveryText {
    pub diagnostic: String,
    pub recovery: Option<String>,
}

type DeliveryError = CanonicalResultSizeError<Infallible>;

struct DeliverySizeWriter {
    bytes: usize,
    too_large: bool,
}

impl Write for DeliverySizeWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = self.bytes.saturating_add(buffer.len());
        if next > MAX_TASK_RECORD_BYTES {
            self.too_large = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "result delivery exceeds limit",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn error_delivery_text(result: &DomainResult) -> Result<ErrorDeliveryText, DeliveryError> {
    #[derive(Serialize)]
    struct Diagnostic<'a> {
        summary: &'a str,
        diagnostics: &'a [serde_json::Value],
    }
    #[derive(Serialize)]
    struct Recovery<'a> {
        next: &'a [serde_json::Value],
    }

    let canonical_bytes = canonical_result_size(result)?;
    let mut diagnostic = serde_json::to_string(&Diagnostic {
        summary: &result.summary,
        diagnostics: &result.diagnostics,
    })
    .map_err(|_| DeliveryError::Serialization)?;
    if diagnostic.len() > MAX_ERROR_DIAGNOSTIC_TEXT_BYTES {
        let mut end = MAX_ERROR_DIAGNOSTIC_TEXT_BYTES - ERROR_TEXT_TRUNCATED.len();
        while !diagnostic.is_char_boundary(end) {
            end -= 1;
        }
        diagnostic.truncate(end);
        diagnostic.push_str(ERROR_TEXT_TRUNCATED);
    }
    let recovery = if result.next.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&Recovery { next: &result.next })
                .map_err(|_| DeliveryError::Serialization)?,
        )
    };
    let mut writer = DeliverySizeWriter {
        bytes: canonical_bytes + ERROR_DELIVERY_ENVELOPE_BYTES,
        too_large: false,
    };
    for text in std::iter::once(&diagnostic).chain(recovery.iter()) {
        // TextContent serializes the JSON text as a JSON string once more.
        let encoded = serde_json::to_writer(&mut writer, text);
        if writer.too_large {
            return Err(DeliveryError::TooLarge);
        }
        encoded.map_err(|_| DeliveryError::Serialization)?;
    }
    Ok(ErrorDeliveryText {
        diagnostic,
        recovery,
    })
}

/// Validate the additional refusal forms before first durable admission; the
/// calling canonicalizer checks the full original result's canonical size.
/// A future client's smaller Body budget can add a range hint; reserve it too.
/// Restoring an older canonical terminal does not reapply this policy.
pub fn validate_result_delivery(
    result: &DomainResult,
    max_supported_body_response_bytes: usize,
) -> Result<(), DeliveryError> {
    if let Some(refused) = maximum_delivery_refusal(result, max_supported_body_response_bytes) {
        error_delivery_text(&refused)?;
    } else if !result.ok {
        error_delivery_text(result)?;
    }
    Ok(())
}

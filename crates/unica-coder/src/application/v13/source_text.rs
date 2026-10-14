//! Исходный текст BSL, который `view` отдаёт по адресам `Body` модуля
//! и `Method.<имя>.Body`, и предел его объёма.

use crate::application::v13::view::ViewError;
use crate::domain::refusal::RefusalCode;
use serde_json::{json, Value};

/// Предел текста `Body` модуля и `Method.<имя>.Body` в байтах UTF-8.
///
/// Решение владельца 14.10.2026 (#1458): 16 КиБ для модуля и метода. Текст
/// сверх предела не режется на части: ответ отказывает `result_too_large`.
///
/// Предел ставит самый строгий из поддерживаемых хостов: по умолчанию он
/// оставляет от ответа инструмента 10 000 токенов и выбрасывает середину.
/// Другой основной хост допускает 25 000 токенов и 50 000 символов.
/// Источники пределов названы в обсуждении #1458.
///
/// Замер на выгрузке УТ (12 290 модулей): кириллический BSL занимает
/// 1,69–1,83 байта UTF-8 на символ (p5–p95), в среднем 1,77. Токенизатор
/// для замера не был доступен; при консервативных двух символах на токен
/// 10 000 токенов — около 34 КБ текста. 16 КиБ оставляют двукратный запас
/// на конверт ответа и на более плотную токенизацию.
pub(crate) const MAX_SOURCE_TEXT_BYTES: usize = 16 * 1024;

/// Текст модуля, прочитанный не дальше предела.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModuleText {
    /// Файл без начальных U+FEFF; разделители строк — как в файле.
    Text(String),
    /// Текст длиннее [`MAX_SOURCE_TEXT_BYTES`]; `bytes` — его полный размер.
    TooLarge { bytes: u64 },
}

/// Что именно не поместилось в предел.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OversizedText<'a> {
    Module,
    Method { at: &'a str },
}

/// Отказ для текста сверх предела. Частичного текста нет: `next` ведёт
/// к структурным адресам, по которым модуль читается по смыслу.
pub(crate) fn text_too_large(module_at: &str, subject: OversizedText<'_>, bytes: u64) -> ViewError {
    let limit = MAX_SOURCE_TEXT_BYTES;
    let subject_name = match subject {
        OversizedText::Module => "module",
        OversizedText::Method { .. } => "method",
    };
    let mut error = ViewError::new(
        RefusalCode::ResultTooLarge,
        format!(
            "{subject_name} text is {bytes} bytes, over the {limit}-byte Body limit; read the module by regions and methods"
        ),
    );
    if let OversizedText::Method { at } = subject {
        error.set_next(view_next(at, "подпись, контексты и строки метода"));
    }
    error.set_next(view_next(
        &format!("{module_at}.Region"),
        "области модуля с их методами",
    ));
    error.set_next(view_next(
        &format!("{module_at}.Method"),
        "методы модуля; текст метода — по адресу <метод>.Body",
    ));
    error
}

fn view_next(at: &str, reason: &str) -> Value {
    json!({"tool": "unica.view", "args": {"at": at}, "reason": reason})
}

/// Проверка предела для уже прочитанного текста; `Err` несёт его размер.
pub(crate) fn admit_text(text: &str) -> Result<(), u64> {
    if text.len() > MAX_SOURCE_TEXT_BYTES {
        Err(text.len() as u64)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{text_too_large, OversizedText, MAX_SOURCE_TEXT_BYTES};
    use crate::domain::refusal::RefusalCode;

    #[test]
    fn source_text_limit_is_the_owner_decided_sixteen_kib() {
        assert_eq!(MAX_SOURCE_TEXT_BYTES, 16_384);
        // Предел считается в байтах UTF-8, а не в символах.
        let cyrillic = "я".repeat(MAX_SOURCE_TEXT_BYTES / 2);
        assert!(super::admit_text(&cyrillic).is_ok());
        assert_eq!(
            super::admit_text(&format!("{cyrillic}a")),
            Err(MAX_SOURCE_TEXT_BYTES as u64 + 1)
        );
    }

    #[test]
    fn oversized_method_refusal_routes_to_the_method_and_module_structure() {
        let module = "main:CommonModule.Пример";
        let method = format!("{module}.Method.Большой");
        let error = text_too_large(
            module,
            OversizedText::Method { at: &method },
            MAX_SOURCE_TEXT_BYTES as u64 + 1,
        );
        assert_eq!(error.code(), RefusalCode::ResultTooLarge);
        let targets = error
            .next()
            .iter()
            .map(|next| next["args"]["at"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            [
                method,
                format!("{module}.Region"),
                format!("{module}.Method")
            ]
        );
    }
}

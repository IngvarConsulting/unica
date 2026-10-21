//! Исходный текст BSL, который `view` отдаёт по адресам `Body` модуля
//! и `Method.<имя>.Body`, и предел его объёма.
use unica_address::address::BodyLineRange;
#[cfg(test)]
use unica_execution_contract::source_text_budget::complete_prefix;

/// Наибольший текст `Body` модуля и `Method.<имя>.Body`, который `view`
/// вообще отдаёт: наибольший бюджет ответа среди известных хостов. Сколько
/// получит конкретный клиент, решает его бюджет ([`unica_execution_contract::source_text_budget::apply_response_budget`]),
/// по умолчанию 16 КиБ. Текст сверх предела не режется на части: ответ
/// отказывает `result_too_large`.
pub(crate) const MAX_SOURCE_TEXT_BYTES: usize = unica_bootstrap::MAX_RESPONSE_BUDGET;

/// Текст модуля, прочитанный не дальше предела.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModuleText {
    /// Файл без начальных U+FEFF; разделители строк — как в файле.
    Text(String),
    /// Текст длиннее [`MAX_SOURCE_TEXT_BYTES`]; `bytes` — его полный размер.
    TooLarge {
        bytes: u64,
        prefix: String,
        lines: usize,
    },
}

/// Собирает текст модуля из потока байтов, не удерживая больше предела:
/// начальные U+FEFF снимаются, после предела считается только размер.
#[derive(Debug, Default)]
pub(crate) struct ModuleTextAccumulator {
    bom_prefix: usize,
    past_bom: bool,
    text: Vec<u8>,
    bytes: u64,
    range: Option<BodyLineRange>,
    newlines: usize,
    last: Option<u8>,
}

impl ModuleTextAccumulator {
    pub(crate) fn with_range(range: BodyLineRange) -> Self {
        Self {
            range: Some(range),
            ..Self::default()
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.newlines + usize::from(self.last.is_some_and(|last| last != b'\n'))
    }

    const BOM: [u8; 3] = [0xef, 0xbb, 0xbf];

    pub(crate) fn push(&mut self, chunk: &[u8]) {
        for byte in chunk {
            if !self.past_bom && *byte == Self::BOM[self.bom_prefix % 3] {
                self.bom_prefix += 1;
                continue;
            }
            if !self.past_bom {
                self.past_bom = true;
                // Неполная последовательность U+FEFF в начале — уже текст.
                let partial = self.bom_prefix % 3;
                self.bom_prefix -= partial;
                for bom_byte in &Self::BOM[..partial] {
                    self.keep(*bom_byte);
                }
            }
            self.keep(*byte);
        }
    }

    fn keep(&mut self, byte: u8) {
        let line = self.newlines + 1;
        self.newlines += usize::from(byte == b'\n');
        self.last = Some(byte);
        if self
            .range
            .is_some_and(|range| line < range.from || line > range.to)
        {
            return;
        }
        self.bytes += 1;
        if self.text.len() <= MAX_SOURCE_TEXT_BYTES {
            self.text.push(byte);
        }
    }

    #[cfg(test)]
    fn retained(&self) -> usize {
        self.text.capacity()
    }

    /// Текст в пределе или его полный размер; `Err` — текст не UTF-8.
    pub(crate) fn finish(mut self) -> Result<ModuleText, ()> {
        if !self.past_bom {
            // Файл из одних U+FEFF пуст; хвост неполной метки — уже текст.
            let partial = self.bom_prefix % 3;
            for bom_byte in &Self::BOM[..partial] {
                self.keep(*bom_byte);
            }
        }
        if self.bytes > MAX_SOURCE_TEXT_BYTES as u64 {
            let end = self.text[..MAX_SOURCE_TEXT_BYTES]
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |last| last + 1);
            let prefix = String::from_utf8(self.text[..end].to_vec()).map_err(|_| ())?;
            return Ok(ModuleText::TooLarge {
                bytes: self.bytes,
                prefix,
                lines: self.line_count(),
            });
        }
        String::from_utf8(self.text)
            .map(ModuleText::Text)
            .map_err(|_| ())
    }
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
    use super::MAX_SOURCE_TEXT_BYTES;

    #[test]
    fn source_text_limit_is_the_largest_known_host_budget() {
        assert_eq!(MAX_SOURCE_TEXT_BYTES, unica_bootstrap::MAX_RESPONSE_BUDGET);
        const { assert!(MAX_SOURCE_TEXT_BYTES >= unica_bootstrap::DEFAULT_RESPONSE_BUDGET) };
        // Предел считается в байтах UTF-8, а не в символах.
        let cyrillic = "я".repeat(MAX_SOURCE_TEXT_BYTES / 2);
        assert!(super::admit_text(&cyrillic).is_ok());
        assert_eq!(
            super::admit_text(&format!("{cyrillic}a")),
            Err(MAX_SOURCE_TEXT_BYTES as u64 + 1)
        );
    }

    fn accumulate(chunks: &[&[u8]]) -> Result<super::ModuleText, ()> {
        let mut text = super::ModuleTextAccumulator::default();
        for chunk in chunks {
            text.push(chunk);
        }
        text.finish()
    }

    #[test]
    fn streamed_module_text_keeps_memory_bounded_and_counts_the_whole_size() {
        let mut text = super::ModuleTextAccumulator::default();
        let chunk = vec![b'a'; 64 * 1024];
        for _ in 0..320 {
            text.push(&chunk);
            assert!(text.retained() <= 2 * (MAX_SOURCE_TEXT_BYTES + 1));
        }
        assert_eq!(
            text.finish(),
            Ok(super::ModuleText::TooLarge {
                bytes: 320 * 64 * 1024,
                prefix: String::new(),
                lines: 1
            })
        );
    }

    #[test]
    fn streamed_module_text_strips_repeated_bom_across_chunks_and_keeps_a_partial_one() {
        use super::ModuleText::Text;
        let bom = [0xef_u8, 0xbb, 0xbf];
        assert_eq!(
            accumulate(&[&bom[..2], &bom[2..], &bom, "а\r\n".as_bytes()]),
            Ok(Text("а\r\n".into()))
        );
        assert_eq!(accumulate(&[&bom, &bom]), Ok(Text(String::new())));
        // Неполная метка — это байты текста, а не U+FEFF; такой текст не UTF-8.
        assert_eq!(accumulate(&[&bom, &bom[..2], b"a"]), Err(()));
        assert_eq!(accumulate(&[&bom[..1]]), Err(()));
        // Метка в середине текста остаётся текстом.
        assert_eq!(accumulate(&[b"a", &bom]), Ok(Text("a\u{feff}".into())));
    }

    #[test]
    fn module_text_limit_counts_bytes_after_the_bom() {
        let bom = [0xef_u8, 0xbb, 0xbf];
        let exact = vec![b'a'; MAX_SOURCE_TEXT_BYTES];
        assert!(matches!(
            accumulate(&[&bom, &exact]),
            Ok(super::ModuleText::Text(text)) if text.len() == MAX_SOURCE_TEXT_BYTES
        ));
        assert_eq!(
            accumulate(&[&bom, &exact, b"a"]),
            Ok(super::ModuleText::TooLarge {
                bytes: MAX_SOURCE_TEXT_BYTES as u64 + 1,
                prefix: String::new(),
                lines: 1
            })
        );
    }
}

#[cfg(test)]
mod body_chunks {
    use super::*;

    #[test]
    fn oversized_read_keeps_only_a_complete_bounded_prefix() {
        let line = "я\r\n";
        let text = line.repeat(MAX_SOURCE_TEXT_BYTES / line.len() + 10);
        let mut accumulator = ModuleTextAccumulator::default();
        for chunk in text.as_bytes().chunks(13) {
            accumulator.push(chunk);
        }
        let ModuleText::TooLarge {
            bytes,
            prefix,
            lines,
        } = accumulator.finish().unwrap()
        else {
            panic!("oversized");
        };
        assert_eq!(bytes, text.len() as u64);
        assert!(prefix.len() <= MAX_SOURCE_TEXT_BYTES);
        assert!(prefix.ends_with("\r\n"));
        assert_eq!(lines, text.lines().count());
        assert_eq!(
            prefix,
            complete_prefix(&text, unica_bootstrap::MAX_RESPONSE_BUDGET)
        );
    }
}

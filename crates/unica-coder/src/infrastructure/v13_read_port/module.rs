use super::{ProviderReadAuthority, ViewError};
use crate::application::v13::source_text::{ModuleText, ModuleTextAccumulator};
use unica_address::source_target::MetadataAddress;
use unica_refusal::{RefusalCode, RefusalDetail};

impl ProviderReadAuthority {
    pub(crate) fn module_source(
        &self,
        target: &MetadataAddress,
    ) -> Result<Option<String>, ViewError> {
        #[cfg(test)]
        {
            *self
                .module_source_reads
                .lock()
                .expect("module source read counter is not poisoned")
                .entry(target.as_str().to_string())
                .or_default() += 1;
        }
        let relative = self.module_relative(target)?;
        self.read_optional_relative_whole(&relative)?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map(|text| text.trim_start_matches('\u{feff}').to_string())
                    .map_err(|_| {
                        ViewError::detailed(
                            RefusalDetail::SourceUnreadable,
                            "BSL module is not UTF-8",
                        )
                    })
            })
            .transpose()
    }

    /// Only proves that the BSL file is present; the text is not read.
    pub(crate) fn module_source_exists(&self, target: &MetadataAddress) -> Result<bool, ViewError> {
        let relative = self.module_relative(target)?;
        self.relative_source_exists(&relative)
    }

    /// Counts Body lines the way the Body snapshot numbers them, without
    /// copying or decoding the module. An absent file is `None`, not zero.
    pub(crate) fn module_line_count(
        &self,
        target: &MetadataAddress,
        checkpoint: impl FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<usize>, ViewError> {
        const BOM: [u8; 3] = [0xef, 0xbb, 0xbf];
        let mut newlines = 0_usize;
        let mut bom_prefix = 0_u64;
        let mut in_bom_prefix = true;
        let mut last = None;
        let Some(len) = self.visit_module_utf8_chunks(target, checkpoint, |chunk| {
            for byte in chunk {
                if in_bom_prefix && *byte == BOM[(bom_prefix % 3) as usize] {
                    bom_prefix += 1;
                } else {
                    in_bom_prefix = false;
                }
            }
            newlines += chunk.iter().filter(|byte| **byte == b'\n').count();
            last = chunk.last().copied().or(last);
            Ok(())
        })?
        else {
            return Ok(None);
        };
        // A file of leading U+FEFF only is an empty Body, as in the snapshot.
        if len == bom_prefix {
            return Ok(Some(0));
        }
        Ok(Some(newlines + usize::from(last != Some(b'\n'))))
    }

    /// Текст модуля для `Body`: без начальных U+FEFF и не длиннее
    /// `MAX_SOURCE_TEXT_BYTES`. Больший файл дочитывается без накопления
    /// ради проверки UTF-8 и полного размера. Отсутствующий файл — пустой
    /// текст, как у прежнего `Body`.
    pub(crate) fn module_text_within_limit(
        &self,
        target: &MetadataAddress,
        checkpoint: impl FnMut() -> Result<(), ViewError>,
    ) -> Result<ModuleText, ViewError> {
        let mut text = ModuleTextAccumulator::default();
        let len = self.visit_module_utf8_chunks(target, checkpoint, |chunk| {
            text.push(chunk);
            Ok(())
        })?;
        if len.is_none() {
            return Ok(ModuleText::Text(String::new()));
        }
        text.finish().map_err(|_| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, "BSL module is not UTF-8")
        })
    }

    /// Retains only the requested complete file lines, while validating the
    /// entire stream and preserving retained-source identity/cancellation.
    pub(crate) fn module_text_range(
        &self,
        target: &MetadataAddress,
        range: unica_address::address::BodyLineRange,
        checkpoint: impl FnMut() -> Result<(), ViewError>,
    ) -> Result<ModuleText, ViewError> {
        let mut text = ModuleTextAccumulator::with_range(range);
        let exists = self.visit_module_utf8_chunks(target, checkpoint, |chunk| {
            text.push(chunk);
            Ok(())
        })?;
        if exists.is_none() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "the BSL module source file does not exist",
            ));
        }
        if range.to > text.line_count() {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                "Body line selector exceeds the file's last line",
            ));
        }
        text.finish().map_err(|_| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, "BSL module is not UTF-8")
        })
    }

    /// Streams the module only after one shared UTF-8 check, so every Body
    /// reader sees the same text and numbers the same lines. An absent file
    /// is `None`; the length is in bytes.
    fn visit_module_utf8_chunks(
        &self,
        target: &MetadataAddress,
        mut checkpoint: impl FnMut() -> Result<(), ViewError>,
        mut visit: impl FnMut(&[u8]) -> Result<(), ViewError>,
    ) -> Result<Option<u64>, ViewError> {
        let relative = self.module_relative(target)?;
        if let Some(buffer) = self.open_buffer(&relative) {
            checkpoint()?;
            visit(buffer.text.as_bytes())?;
            return Ok(Some(buffer.text.len() as u64));
        }
        let mut carry = Vec::<u8>::new();
        let interrupted = std::cell::RefCell::new(None);
        let result = self.root.visit_relative_regular_chunks(
            &relative,
            || {
                checkpoint().map_err(|error| {
                    *interrupted.borrow_mut() = Some(error);
                    std::io::Error::other("BSL module read interrupted")
                })
            },
            |chunk| {
                let mut candidate = Vec::with_capacity(carry.len() + chunk.len());
                candidate.extend_from_slice(&carry);
                candidate.extend_from_slice(chunk);
                match std::str::from_utf8(&candidate) {
                    Ok(_) => carry.clear(),
                    Err(error) if error.error_len().is_none() => {
                        carry = candidate[error.valid_up_to()..].to_vec();
                    }
                    Err(_) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "BSL module is not UTF-8",
                        ))
                    }
                }
                visit(chunk).map_err(|error| {
                    *interrupted.borrow_mut() = Some(error);
                    std::io::Error::other("BSL module chunk was refused")
                })
            },
        );
        let len = match result {
            Ok(len) => len,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(interrupted.into_inner().unwrap_or_else(|| {
                    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
                }))
            }
        };
        if !carry.is_empty() {
            return Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                "BSL module is not UTF-8",
            ));
        }
        Ok(Some(len))
    }

    #[cfg(test)]
    pub(crate) fn module_source_read_count(&self, target: &str) -> usize {
        self.module_source_reads
            .lock()
            .expect("module source read counter is not poisoned")
            .get(target)
            .copied()
            .unwrap_or_default()
    }
}

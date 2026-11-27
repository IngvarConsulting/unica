use super::ProviderReadAuthority;
use crate::application::v13::source_text::MAX_SOURCE_TEXT_BYTES;
use crate::domain::node_view::SourceText;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use unica_address::source_target::MetadataAddress;
use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;
use unica_execution_contract::view_error::ViewError;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_source_contract::read::{SourceEntry, SourceFile};

impl ProviderReadAuthority {
    pub(crate) fn embedded_help_manifest(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<(PathBuf, Vec<String>)>, ViewError> {
        let relative = self.attached_resource_relative(target, "Help.xml")?;
        let Some(file) = self.exact_help_file(&relative, checkpoint)? else {
            return Ok(None);
        };
        let bytes = if let Some(buffer) = self.open_buffer(&relative) {
            if buffer.text.len() > MAX_CANONICAL_RESULT_BYTES {
                return Err(ViewError::new(
                    RefusalCode::ResultTooLarge,
                    "Help.xml exceeds the source result bound",
                ));
            }
            buffer.text.as_bytes().to_vec()
        } else {
            file.read_bounded(MAX_CANONICAL_RESULT_BYTES)
                .map_err(unreadable)?
        };
        let pages = crate::infrastructure::family_codecs::metadata_read_codec()
            .help_pages(&bytes)
            .map_err(unreadable)?;
        file.validate_named_identity().map_err(unreadable)?;
        checkpoint()?;
        Ok(Some((relative, pages)))
    }

    pub(crate) fn embedded_help_exists(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<bool, ViewError> {
        self.exact_help_file(relative, checkpoint)
            .map(|file| file.is_some())
    }

    pub(crate) fn embedded_help_text(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<SourceText, ViewError> {
        let file = self.exact_help_file(relative, checkpoint)?.ok_or_else(|| {
            ViewError::new(RefusalCode::NotFound, "registered help HTML was not found")
        })?;
        let text = if let Some(buffer) = self.open_buffer(relative) {
            bounded_help_text(buffer.text.as_bytes(), checkpoint)?
        } else {
            bounded_help_text(
                file.open_named_identity_for_read().map_err(unreadable)?,
                checkpoint,
            )?
        };
        file.validate_named_identity().map_err(unreadable)?;
        Ok(text)
    }

    // The retained open may accept a case alias. Prove the exact entry spelling
    // of every component on the same parent before it can name the read target.
    fn exact_help_file(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Option<SourceFile>, ViewError> {
        let mut parent = self.root.as_ref().clone();
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            checkpoint()?;
            let Component::Normal(name) = component else {
                return Err(unreadable("embedded help path is not relative"));
            };
            let child = match parent.retain_immediate_child_nofollow(name) {
                Ok(child) => child,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(unreadable(error)),
            };
            let mut found = false;
            let mut interrupted = None;
            let visited = parent.visit_immediate_names::<io::Error>(|candidate| {
                checkpoint().map_err(|error| {
                    interrupted = Some(error);
                    io::Error::other("embedded help lookup interrupted")
                })?;
                if candidate == name {
                    found = true;
                    return Err(io::Error::other("exact name found"));
                }
                Ok(())
            });
            if !found {
                visited.map_err(|error| interrupted.unwrap_or_else(|| unreadable(error)))?;
                return Ok(None);
            }
            match child {
                SourceEntry::Directory(directory) if components.peek().is_some() => {
                    parent = directory
                }
                SourceEntry::RegularFile(file) if components.peek().is_none() => {
                    file.validate_named_identity().map_err(unreadable)?;
                    return Ok(Some(file));
                }
                _ => return Err(unreadable("embedded help path is not a regular local file")),
            }
        }
        Ok(None)
    }
}

fn bounded_help_text(
    mut reader: impl Read,
    checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
) -> Result<SourceText, ViewError> {
    let mut contents = Vec::new();
    let mut chunk = [0; 8192];
    let mut carry = 0;
    let mut bytes = 0_u64;
    loop {
        checkpoint()?;
        let count = reader.read(&mut chunk[carry..]).map_err(unreadable)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        let end = carry + count;
        if bytes <= MAX_SOURCE_TEXT_BYTES as u64 {
            contents.extend_from_slice(&chunk[carry..end]);
        }
        carry = match std::str::from_utf8(&chunk[..end]) {
            Ok(_) => 0,
            Err(error) if error.error_len().is_none() => {
                let start = error.valid_up_to();
                chunk.copy_within(start..end, 0);
                end - start
            }
            Err(_) => return Err(unreadable("help HTML is not UTF-8")),
        };
    }
    if carry != 0 {
        return Err(unreadable("help HTML is not UTF-8"));
    }
    if bytes > MAX_SOURCE_TEXT_BYTES as u64 {
        return Ok(SourceText::Oversized {
            bytes,
            prefix: String::new(),
        });
    }
    String::from_utf8(contents)
        .map(SourceText::Whole)
        .map_err(unreadable)
}

fn unreadable(error: impl ToString) -> ViewError {
    ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_help_text_bounds_retention_and_validates_utf8_past_the_bound() {
        let text = format!("\u{feff}{}я\r\n", "x".repeat(8188));
        assert_eq!(
            bounded_help_text(text.as_bytes(), &mut || Ok(())).unwrap(),
            SourceText::Whole(text)
        );

        let mut bytes = vec![b'a'; MAX_SOURCE_TEXT_BYTES + 1];
        assert_eq!(
            bounded_help_text(bytes.as_slice(), &mut || Ok(())).unwrap(),
            SourceText::Oversized {
                bytes: bytes.len() as u64,
                prefix: String::new()
            }
        );
        bytes.push(0xff);
        assert!(bounded_help_text(bytes.as_slice(), &mut || Ok(())).is_err());
    }
}

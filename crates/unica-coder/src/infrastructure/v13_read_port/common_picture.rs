use super::{ProviderReadAuthority, ViewError};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};
use std::{io, path::Path};
use unica_address::source_target::MetadataAddress;
use unica_execution_contract::result_size::{
    MAX_CANONICAL_RESULT_BYTES, MAX_TASK_RECORD_ENVELOPE_BYTES,
};
use unica_metadata_designer::common_picture::{picture_source, validate_png, PictureSource};
use unica_refusal::{RefusalCode, RefusalDetail};

// resource-limit: view.common-picture-input-bytes
// class: step
// unit: bytes
// scope: file
// value: 6242304
// derived: true
// resource: Сохраняемые байты одного входа ветви Picture перед декодированием и выдачей Base64.
// status: unproven
// origin: #3170
// problem: Неделимый двоичный ресурс возвращается целиком в JSON.
// on-reach: Ветвь возвращает unsupported, карточка остаётся доступна.
// issue: #3170
// evidence: Base64 расходует четыре байта конверта на три байта PNG. Из предела канонического результата вычитается существующий запас служебных полей. Эталон 2.20 Probe занимает 2825 байт.
const COMMON_PICTURE_INPUT_BYTES: usize =
    (MAX_CANONICAL_RESULT_BYTES - MAX_TASK_RECORD_ENVELOPE_BYTES) / 4 * 3;

enum PictureInput {
    Bytes(Vec<u8>),
    Missing,
    TooLarge,
}

impl ProviderReadAuthority {
    pub(crate) fn common_picture(
        &self,
        target: &MetadataAddress,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<Map<String, Value>, ViewError> {
        let descriptor = self.attached_resource_relative(target, "Picture.xml")?;
        let state = match self.picture_input(&descriptor, checkpoint)? {
            PictureInput::Bytes(xml) => picture_source(&xml),
            PictureInput::Missing => PictureSource::Missing,
            PictureInput::TooLarge => PictureSource::Unsupported,
        };
        if state != PictureSource::Png {
            return picture_fallback(state);
        }

        let resource = descriptor.with_file_name("Picture").join("Picture.png");
        let bytes = match self.picture_input(&resource, checkpoint)? {
            PictureInput::Bytes(bytes) => bytes,
            PictureInput::Missing => return picture_fallback(PictureSource::Missing),
            PictureInput::TooLarge => return picture_fallback(PictureSource::Unsupported),
        };
        let state = validate_png(&bytes, checkpoint)?;
        if state != PictureSource::Png {
            return picture_fallback(state);
        }

        Ok(Map::from_iter([
            ("state".into(), json!("available")),
            ("mediaType".into(), json!("image/png")),
            ("encoding".into(), json!("base64")),
            ("data".into(), json!(STANDARD.encode(bytes))),
        ]))
    }

    fn picture_input(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), ViewError>,
    ) -> Result<PictureInput, ViewError> {
        checkpoint()?;
        let mut bytes = Vec::new();
        let mut interrupted = None;
        let mut too_large = false;
        let visited = self.root.visit_relative_regular_chunks(
            relative,
            || {
                checkpoint().map_err(|error| {
                    interrupted = Some(error);
                    io::Error::other("picture read interrupted")
                })
            },
            |chunk| {
                if chunk.len() > COMMON_PICTURE_INPUT_BYTES - bytes.len() {
                    too_large = true;
                    return Err(io::Error::other("picture input exceeds the read limit"));
                }
                bytes.try_reserve(chunk.len()).map_err(io::Error::other)?;
                bytes.extend_from_slice(chunk);
                Ok(())
            },
        );
        if let Some(error) = interrupted {
            return Err(error);
        }
        checkpoint()?;
        match visited {
            Ok(_) => Ok(PictureInput::Bytes(bytes)),
            Err(_) if too_large => Ok(PictureInput::TooLarge),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PictureInput::Missing),
            Err(error) => Err(ViewError::detailed(
                RefusalDetail::SourceUnreadable,
                error.to_string(),
            )),
        }
    }
}

fn picture_fallback(state: PictureSource) -> Result<Map<String, Value>, ViewError> {
    let state = match state {
        PictureSource::Missing => "missing",
        PictureSource::Damaged => "damaged",
        PictureSource::Unsupported => "unsupported",
        PictureSource::InvalidPath => {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                "CommonPicture contains an invalid resource reference",
            ))
        }
        PictureSource::Png => unreachable!("available pictures carry the validated PNG"),
    };
    Ok(Map::from_iter([("state".into(), json!(state))]))
}

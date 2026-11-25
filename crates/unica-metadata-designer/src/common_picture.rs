//! The display resource declared by the platform's ExtPicture companion.

use roxmltree::Document;
use std::io::Cursor;

const EXTERNAL_PROPERTIES: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
const READABLE: &str = "http://v8.1c.ru/8.3/xcf/readable";

#[derive(Debug, PartialEq, Eq)]
pub enum PictureSource {
    Png,
    Missing,
    Damaged,
    Unsupported,
    InvalidPath,
}

pub fn picture_source(xml: &[u8]) -> PictureSource {
    let Ok(text) = std::str::from_utf8(xml) else {
        return PictureSource::Damaged;
    };
    let Ok(doc) = Document::parse(text.trim_start_matches('\u{feff}')) else {
        return PictureSource::Damaged;
    };
    let root = doc.root_element();
    if !root.has_tag_name((EXTERNAL_PROPERTIES, "ExtPicture")) {
        return PictureSource::Damaged;
    }
    let mut pictures = root.children().filter(roxmltree::Node::is_element);
    let Some(picture) = pictures.next() else {
        return PictureSource::Damaged;
    };
    if !picture.has_tag_name((EXTERNAL_PROPERTIES, "Picture")) || pictures.next().is_some() {
        return PictureSource::Damaged;
    }
    let mut references = picture
        .children()
        .filter(|child| child.has_tag_name((READABLE, "Abs")));
    let Some(reference) = references.next() else {
        return if picture.children().any(|child| {
            child.is_element()
                || (child.is_text() && child.text().is_some_and(|text| !text.trim().is_empty()))
        }) {
            PictureSource::Damaged
        } else {
            PictureSource::Missing
        };
    };
    if references.next().is_some() {
        return PictureSource::Damaged;
    }
    let Some(name) = scalar_text(reference) else {
        return PictureSource::Damaged;
    };
    match name.trim() {
        "Picture.png" if preserves_native_pixels(picture, reference) => PictureSource::Png,
        "" => PictureSource::Damaged,
        "." | ".." => PictureSource::InvalidPath,
        other if other.contains(['/', '\\', ':']) => PictureSource::InvalidPath,
        _ => PictureSource::Unsupported,
    }
}

// The platform can apply a separate color key before displaying the picture.
fn preserves_native_pixels(
    picture: roxmltree::Node<'_, '_>,
    reference: roxmltree::Node<'_, '_>,
) -> bool {
    let mut transparency = picture
        .children()
        .filter(|node| node.is_element() && *node != reference);
    match transparency.next() {
        None => true,
        Some(flag) => {
            flag.has_tag_name((READABLE, "LoadTransparent"))
                && matches!(
                    scalar_text(flag).as_deref().map(str::trim),
                    Some("false" | "0")
                )
                && transparency.next().is_none()
        }
    }
}

fn scalar_text(node: roxmltree::Node<'_, '_>) -> Option<String> {
    if node.children().any(|child| child.is_element()) {
        return None;
    }
    Some(
        node.children()
            .filter(roxmltree::Node::is_text)
            .filter_map(|child| child.text())
            .collect(),
    )
}

/// Check decoded rows and the end of the stream, without retaining a framebuffer.
pub fn validate_png<E>(
    bytes: &[u8],
    checkpoint: &mut dyn FnMut() -> Result<(), E>,
) -> Result<PictureSource, E> {
    let mut source = Cursor::new(bytes);
    let mut decoder = png::Decoder::new(&mut source);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let result = (|| -> Result<PictureSource, E> {
        let mut reader = match decoder.read_info() {
            Ok(reader) => reader,
            Err(error) => return Ok(classify_png_error(error)),
        };
        if reader.info().animation_control.is_some() {
            return Ok(PictureSource::Unsupported);
        }
        loop {
            checkpoint()?;
            match reader.next_row() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(error) => return Ok(classify_png_error(error)),
            }
        }
        Ok(match reader.finish() {
            Ok(()) => PictureSource::Png,
            Err(error) => classify_png_error(error),
        })
    })()?;
    checkpoint()?;
    if result == PictureSource::Png && source.position() != bytes.len() as u64 {
        return Ok(PictureSource::Damaged);
    }
    Ok(result)
}

fn classify_png_error(error: png::DecodingError) -> PictureSource {
    match error {
        png::DecodingError::LimitsExceeded => PictureSource::Unsupported,
        _ => PictureSource::Damaged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &[u8] = include_bytes!(
        "../../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/CommonPictures/Probe/Ext/Picture.xml"
    );

    #[test]
    fn color_key_requires_platform_rendering() {
        assert_eq!(picture_source(PROBE), PictureSource::Png);
        let with_key = std::str::from_utf8(PROBE).unwrap().replace(
            "false</xr:LoadTransparent>",
            "true</xr:LoadTransparent><xr:TransparentPixel x=\"0\" y=\"0\"/>",
        );
        assert_eq!(
            picture_source(with_key.as_bytes()),
            PictureSource::Unsupported
        );
    }

    #[test]
    fn comments_preserve_scalar_text_and_do_not_declare_a_picture() {
        let empty = br#"<ExtPicture xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"><Picture><!-- empty --></Picture></ExtPicture>"#;
        assert_eq!(picture_source(empty), PictureSource::Missing);

        let text = std::str::from_utf8(PROBE).unwrap();
        let split_filename = text.replace("Picture.png", "Picture<!-- split -->.png");
        assert_eq!(
            picture_source(split_filename.as_bytes()),
            PictureSource::Png
        );
        let malformed_flag = text.replace("false</", "false<!-- split -->true</");
        assert_eq!(
            picture_source(malformed_flag.as_bytes()),
            PictureSource::Unsupported
        );
    }

    #[test]
    fn png_must_finish_at_the_end_of_the_resource() {
        let png = include_bytes!(
            "../../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src/CommonPictures/Probe/Ext/Picture/Picture.png"
        );
        let validate = |bytes: &[u8]| validate_png(bytes, &mut || Ok::<_, ()>(())).unwrap();
        assert_eq!(validate(png), PictureSource::Png);
        let trailing = [png.as_slice(), b"trailing"].concat();
        assert_eq!(validate(&trailing), PictureSource::Damaged);
    }
}

use quick_xml::{events::Event, Reader};
use roxmltree::Document;

use crate::domain::navigation_limits::MAX_NAVIGATION_NESTING_DEPTH;

pub(crate) const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
pub(crate) const SPREADSHEET_DOCUMENT_NS: &str = "http://v8.1c.ru/8.2/data/spreadsheet";
pub(crate) const DATA_COMPOSITION_SCHEMA_NS: &str =
    "http://v8.1c.ru/8.1/data-composition-system/schema";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundedXmlError {
    InvalidUtf8,
    Malformed,
    ResourceLimit,
}

pub(crate) fn parse_bounded_xml_document<'a>(
    bytes: &'a [u8],
) -> Result<(&'a str, Document<'a>), BoundedXmlError> {
    let xml = std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
        .map_err(|_| BoundedXmlError::InvalidUtf8)?;
    preflight_xml_nesting(xml)?;
    let document = Document::parse(xml).map_err(|_| BoundedXmlError::Malformed)?;
    Ok((xml, document))
}

pub(crate) fn preflight_xml_nesting(xml: &str) -> Result<(), BoundedXmlError> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(_)) => depth = increment_depth(depth)?,
            Ok(Event::End(_)) => {
                depth = depth.checked_sub(1).ok_or(BoundedXmlError::Malformed)?;
            }
            Ok(Event::Empty(_)) => {
                let _ = increment_depth(depth)?;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(BoundedXmlError::Malformed),
        }
    }
    if depth == 0 {
        Ok(())
    } else {
        Err(BoundedXmlError::Malformed)
    }
}

fn increment_depth(depth: usize) -> Result<usize, BoundedXmlError> {
    let next_depth = depth.checked_add(1).ok_or(BoundedXmlError::ResourceLimit)?;
    if next_depth > MAX_NAVIGATION_NESTING_DEPTH {
        return Err(BoundedXmlError::ResourceLimit);
    }
    Ok(next_depth)
}

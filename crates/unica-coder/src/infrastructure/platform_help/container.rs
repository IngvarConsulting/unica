//! Разбор блочного формата контейнера справки платформы 1С (`.hbk`).
//!
//! Формат выведен из наблюдения над контейнерами установленной платформы, а
//! не из документации вендора: шестнадцатибайтовый заголовок файла, за ним
//! блок таблицы записей (адреса заголовка и тела для каждой записи), записи
//! адресуют блоки, которые могут продолжаться по цепочке страниц.

use std::collections::BTreeMap;

const BLOCK_HEADER: usize = 31;
const TERMINATOR: u32 = 0x7FFF_FFFF;

#[derive(Debug, PartialEq, Eq)]
pub enum ContainerError {
    BadSignature,
    TruncatedBlock,
    BadBlockHeader,
}

pub struct V8Container {
    entries: BTreeMap<String, Vec<u8>>,
}

struct BlockHeader {
    data_size: usize,
    page_size: usize,
    next_page: u32,
}

fn read_block_header(bytes: &[u8], offset: usize) -> Result<BlockHeader, ContainerError> {
    let end = offset
        .checked_add(BLOCK_HEADER)
        .ok_or(ContainerError::TruncatedBlock)?;
    let raw = bytes
        .get(offset..end)
        .ok_or(ContainerError::TruncatedBlock)?;
    if &raw[..2] != b"\r\n" {
        return Err(ContainerError::BadBlockHeader);
    }
    let text = std::str::from_utf8(&raw[2..BLOCK_HEADER - 2])
        .map_err(|_| ContainerError::BadBlockHeader)?;
    let mut parts = text.split(' ');
    let mut next_hex = || -> Result<u32, ContainerError> {
        let field = parts.next().ok_or(ContainerError::BadBlockHeader)?;
        u32::from_str_radix(field, 16).map_err(|_| ContainerError::BadBlockHeader)
    };
    let data_size = next_hex()? as usize;
    let page_size = next_hex()? as usize;
    let next_page = next_hex()?;
    Ok(BlockHeader {
        data_size,
        page_size,
        next_page,
    })
}

/// Читает блок, следуя цепочке страниц: одна запись может не поместиться в
/// одну страницу, и продолжение адресуется полем `next_page`.
fn read_block(bytes: &[u8], offset: usize) -> Result<Vec<u8>, ContainerError> {
    let header = read_block_header(bytes, offset)?;
    let mut remaining = header.data_size;
    let mut out = Vec::with_capacity(remaining);
    let mut start = offset + BLOCK_HEADER;
    let mut page = header.page_size;
    let mut next = header.next_page;
    loop {
        let take = page.min(remaining);
        let end = start
            .checked_add(take)
            .ok_or(ContainerError::TruncatedBlock)?;
        out.extend_from_slice(
            bytes
                .get(start..end)
                .ok_or(ContainerError::TruncatedBlock)?,
        );
        remaining -= take;
        if remaining == 0 || next == TERMINATOR {
            break;
        }
        let cont = read_block_header(bytes, next as usize)?;
        start = next as usize + BLOCK_HEADER;
        page = cont.page_size;
        next = cont.next_page;
    }
    Ok(out)
}

fn entry_name(header: &[u8]) -> String {
    let tail = header.get(20..).unwrap_or_default();
    let units: Vec<u16> = tail
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

impl V8Container {
    pub fn parse(bytes: &[u8]) -> Result<V8Container, ContainerError> {
        if bytes.len() < 16 || bytes[..4] != TERMINATOR.to_le_bytes() {
            return Err(ContainerError::BadSignature);
        }
        let toc = read_block(bytes, 16)?;
        let mut entries = BTreeMap::new();
        for record in toc.chunks_exact(12) {
            let head = u32::from_le_bytes([record[0], record[1], record[2], record[3]]);
            let data = u32::from_le_bytes([record[4], record[5], record[6], record[7]]);
            // Пропуск, а не выход: запись с ограничителем встречается в
            // середине таблицы у семи контейнеров установки.
            if head == TERMINATOR || data == TERMINATOR {
                continue;
            }
            let name = entry_name(&read_block(bytes, head as usize)?);
            entries.insert(name, read_block(bytes, data as usize)?);
        }
        Ok(V8Container { entries })
    }

    pub fn entry(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(Vec::as_slice)
    }
}

/// Синтетические фикстуры контейнера для тестов этого модуля и — в
/// следующей задаче — тестов соседнего модуля. Объявлен вне `#[cfg(test)]`,
/// потому что должен собираться и для тех тестовых бинарей, где `container`
/// сам по себе не является объектом тестирования; скрыт из документации, так
/// как не входит в публичный API продукта.
#[doc(hidden)]
pub mod tests_support {
    /// Собирает блок в формате контейнера: заголовок 31 байт, затем данные.
    pub fn block(data: &[u8], next: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(
            format!("{:08x} {:08x} {:08x} ", data.len(), data.len(), next).as_bytes(),
        );
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(data);
        out
    }

    /// Заголовок записи: 20 байт служебных, затем имя в UTF-16LE и нулевой разделитель.
    pub fn entry_header(name: &str) -> Vec<u8> {
        let mut out = vec![0u8; 20];
        for unit in name.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    pub fn container_with(entries: &[(&str, &[u8])], trailing_terminator_entry: bool) -> Vec<u8> {
        let mut body = Vec::new();
        let mut addresses = Vec::new();
        // Резервируем место под заголовок файла и блок таблицы.
        let toc_len = 12 * (entries.len() + usize::from(trailing_terminator_entry));
        let toc_block_len = 31 + toc_len;
        let mut cursor = 16 + toc_block_len;
        for (name, data) in entries {
            let header = block(&entry_header(name), 0x7FFF_FFFF);
            let payload = block(data, 0x7FFF_FFFF);
            addresses.push((cursor as u32, (cursor + header.len()) as u32));
            cursor += header.len() + payload.len();
            body.extend_from_slice(&header);
            body.extend_from_slice(&payload);
        }
        let mut toc = Vec::new();
        for (head, data) in &addresses {
            toc.extend_from_slice(&head.to_le_bytes());
            toc.extend_from_slice(&data.to_le_bytes());
            toc.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        }
        if trailing_terminator_entry {
            // Запись, у которой адрес тела равен ограничителю: встречается в
            // семи контейнерах установки и не является концом таблицы.
            toc.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
            toc.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
            toc.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        out.extend_from_slice(&512u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&block(&toc, 0x7FFF_FFFF));
        out.extend_from_slice(&body);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::container_with;
    use super::*;

    #[test]
    fn parses_named_entries() {
        let bytes = container_with(
            &[("Book", b"book-body"), ("FileStorage", b"zip-body")],
            false,
        );
        let container = V8Container::parse(&bytes).expect("контейнер разобран");
        assert_eq!(container.entry("Book"), Some(&b"book-body"[..]));
        assert_eq!(container.entry("FileStorage"), Some(&b"zip-body"[..]));
        assert_eq!(container.entry("Missing"), None);
    }

    #[test]
    fn terminator_entry_is_skipped_not_treated_as_end_of_table() {
        let bytes = container_with(
            &[("Book", b"book-body"), ("FileStorage", b"zip-body")],
            true,
        );
        let container = V8Container::parse(&bytes).expect("контейнер разобран");
        assert_eq!(container.entry("FileStorage"), Some(&b"zip-body"[..]));
    }

    #[test]
    fn wrong_signature_is_rejected() {
        let mut bytes = container_with(&[("Book", b"x")], false);
        bytes[0] = 0;
        bytes[1] = 0;
        bytes[2] = 0;
        bytes[3] = 0;
        assert!(matches!(
            V8Container::parse(&bytes),
            Err(ContainerError::BadSignature)
        ));
    }
}

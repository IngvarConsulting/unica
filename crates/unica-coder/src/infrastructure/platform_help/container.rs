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
    // Объявленная длина — восьмизначное шестнадцатеричное поле из файла, то
    // есть до 4 ГиБ по неподтверждённому числу. Ни выделять по нему, ни
    // выдавать найденный обрезок за целый блок нельзя: усечённый `.hbk` от
    // прерванной установки — реалистичный повод, а отказ выделения прерывает
    // процесс целиком, в отличие от паники, которую перехватывает
    // `spawn_blocking`. Блок не может быть длиннее файла, в котором лежит.
    if header.data_size > bytes.len() {
        return Err(ContainerError::TruncatedBlock);
    }
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
        // Страница, не несущая ни байта, не двигает `remaining`, поэтому
        // цепочка `next` с нулевым `page_size` крутится вечно. Это
        // единственный способ не завершиться: когда страница несёт хоть байт,
        // `remaining` строго убывает, и любая, даже замкнутая, цепочка
        // конечна.
        if take == 0 {
            return Err(ContainerError::TruncatedBlock);
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

/// Фикстуры контейнера. Под `cfg(test)`, чтобы вспомогательный код не попадал
/// в релизный бинарник; `pub(crate)` — чтобы их видели тесты соседних модулей.
#[cfg(test)]
pub(crate) mod tests_support {
    /// Собирает блок в формате контейнера: заголовок 31 байт, затем данные.
    pub(crate) fn block(data: &[u8], next: u32) -> Vec<u8> {
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
    pub(crate) fn entry_header(name: &str) -> Vec<u8> {
        let mut out = vec![0u8; 20];
        for unit in name.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    /// `terminator_at` — индекс в таблице, куда вставить запись-ограничитель.
    /// Тест на пропуск обязан ставить её ПЕРЕД реальной записью: в конце
    /// таблицы `continue` и `break` дают одинаковый результат, и тест
    /// перестаёт различать исправление и дефект.
    pub(crate) fn container_with(
        entries: &[(&str, &[u8])],
        terminator_at: Option<usize>,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        let mut addresses = Vec::new();
        // Резервируем место под заголовок файла и блок таблицы.
        let toc_len = 12 * (entries.len() + usize::from(terminator_at.is_some()));
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
        let terminator_record = [0x7FFF_FFFFu32; 3];
        let mut toc = Vec::new();
        for (index, (head, data)) in addresses.iter().enumerate() {
            if terminator_at == Some(index) {
                // Запись, у которой адрес тела равен ограничителю: встречается в
                // семи контейнерах установки и не является концом таблицы.
                for field in terminator_record {
                    toc.extend_from_slice(&field.to_le_bytes());
                }
            }
            toc.extend_from_slice(&head.to_le_bytes());
            toc.extend_from_slice(&data.to_le_bytes());
            toc.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        }
        if terminator_at.is_some_and(|index| index >= addresses.len()) {
            for field in terminator_record {
                toc.extend_from_slice(&field.to_le_bytes());
            }
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

    /// Контейнер с единственной записью `Book`, без `FileStorage`: нужен
    /// тестам следующей задачи.
    pub(crate) fn container_without_file_storage() -> Vec<u8> {
        container_with(&[("Book", b"book-body")], None)
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
            None,
        );
        let container = V8Container::parse(&bytes).expect("контейнер разобран");
        assert_eq!(container.entry("Book"), Some(&b"book-body"[..]));
        assert_eq!(container.entry("FileStorage"), Some(&b"zip-body"[..]));
        assert_eq!(container.entry("Missing"), None);
    }

    #[test]
    fn terminator_entry_is_skipped_not_treated_as_end_of_table() {
        // Ограничитель стоит ПЕРЕД `FileStorage`: с `break` эта запись стала бы
        // недостижимой, и тест бы упал. С ограничителем в конце таблицы тест
        // прошёл бы при любом поведении и ничего не доказывал.
        let bytes = container_with(
            &[("Book", b"book-body"), ("FileStorage", b"zip-body")],
            Some(1),
        );
        let container = V8Container::parse(&bytes).expect("контейнер разобран");
        assert_eq!(container.entry("Book"), Some(&b"book-body"[..]));
        assert_eq!(
            container.entry("FileStorage"),
            Some(&b"zip-body"[..]),
            "запись после ограничителя обязана остаться достижимой"
        );
    }

    /// Заголовок блока с произвольными полями. `tests_support::block`
    /// выводит `data_size` и `page_size` из длины данных и потому не умеет
    /// собрать блок, который лжёт о своей длине, — а именно так выглядит
    /// усечённый `.hbk` от прерванной установки.
    fn raw_block_header(data_size: u32, page_size: u32, next: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(format!("{data_size:08x} {page_size:08x} {next:08x} ").as_bytes());
        out.extend_from_slice(b"\r\n");
        out
    }

    /// Восьмизначное шестнадцатеричное поле длины из файла управляет
    /// `Vec::with_capacity` — до 4 ГиБ по неподтверждённому числу, и отказ
    /// выделения прерывает процесс, в отличие от паники, которую ловит
    /// `spawn_blocking`. Наблюдаемое следствие того же доверия проверяется
    /// здесь: блок, объявивший больше данных, чем весит весь файл, до правки
    /// молча возвращал то немногое, что нашлось, выдавая обрезок за целое.
    #[test]
    fn block_claiming_more_data_than_the_file_holds_is_refused() {
        let mut bytes = raw_block_header(0x0010_0000, 0x0000_0004, TERMINATOR);
        bytes.extend_from_slice(b"data");
        assert!(
            bytes.len() < 0x0010_0000,
            "фикстура обязана быть меньше объявленной длины"
        );
        assert_eq!(
            read_block(&bytes, 0),
            Err(ContainerError::TruncatedBlock),
            "объявленная длина больше файла — отказ, а не обрезок"
        );
    }

    /// Страница с нулевым `page_size` не несёт ни байта, поэтому `remaining`
    /// не убывает; при нетерминальном `next_page` цикл `read_block`
    /// возвращается к тому же заголовку навсегда. Это единственный способ не
    /// завершиться: когда страница несёт хоть байт, `remaining` строго
    /// убывает, и любая, даже циклическая, цепочка `next` конечна.
    #[test]
    fn a_page_that_carries_no_bytes_is_refused_instead_of_looping() {
        // `next` указывает на этот же заголовок: цепочка замкнута на себя.
        let bytes = raw_block_header(0x0000_000a, 0x0000_0000, 0);
        assert_eq!(bytes.len(), BLOCK_HEADER);
        assert_eq!(
            read_block(&bytes, 0),
            Err(ContainerError::TruncatedBlock),
            "нулевая страница обязана давать отказ, а не вечный цикл"
        );
    }

    #[test]
    fn wrong_signature_is_rejected() {
        let mut bytes = container_with(&[("Book", b"x")], None);
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

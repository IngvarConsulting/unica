//! Кадры LSP поверх stdio: заголовки `Content-Length` и тело JSON-RPC.
use serde_json::Value;
use std::io::{self, BufRead, Read, Write};

/// Тело одного сообщения редактора. Полный текст большого модуля в
/// `didOpen` укладывается с запасом; дальше кадр протокола демона.
pub(super) const MAX_LSP_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_LSP_HEADER_LINE_BYTES: usize = 1024;

/// Прочитать тело следующего сообщения. `None` — поток закрыт между
/// сообщениями. Ошибка означает, что границы кадров потеряны.
pub(super) fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length = None;
    let mut started = false;
    loop {
        let mut line = Vec::new();
        let read = reader
            .by_ref()
            .take(MAX_LSP_HEADER_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            return if started {
                Err(invalid("LSP header ended before its body"))
            } else {
                Ok(None)
            };
        }
        if line.last() != Some(&b'\n') {
            return Err(invalid("LSP header line is too long or unterminated"));
        }
        started = true;
        let line = line
            .strip_suffix(b"\r\n")
            .ok_or_else(|| invalid("LSP header line must end with CRLF"))?;
        if line.is_empty() {
            break;
        }
        let line = std::str::from_utf8(line).map_err(|_| invalid("LSP header is not UTF-8"))?;
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("LSP header has no name"))?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid("LSP Content-Length is not a number"))?;
            if length.replace(parsed).is_some() {
                return Err(invalid("LSP message repeats Content-Length"));
            }
        }
    }
    let length = length.ok_or_else(|| invalid("LSP message has no Content-Length"))?;
    if length > MAX_LSP_MESSAGE_BYTES {
        return Err(invalid("LSP message exceeds its size limit"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

pub(super) fn write_message(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

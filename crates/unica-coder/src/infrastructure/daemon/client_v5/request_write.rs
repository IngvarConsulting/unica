use super::V5TransportError;
use std::io::Write;

pub(super) fn write_request<W: Write, T: serde::Serialize>(
    writer: &mut W,
    value: &T,
    stage: &'static str,
    configure: impl FnOnce(&mut W) -> Result<(), String>,
    after_write: impl FnOnce() -> Result<(), String>,
) -> Result<(), V5TransportError> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|_| V5TransportError::RequestNotSent(format!("serialize protocol-v5 {stage}")))?;
    bytes.push(b'\n');
    configure(writer).map_err(V5TransportError::RequestNotSent)?;
    // Once writing starts, conservatively recover by receipt: an I/O failure
    // or an expired post-write checkpoint must never authorize a resubmission.
    writer.write_all(&bytes).map_err(|error| {
        V5TransportError::ResponseLost(format!("write protocol-v5 {stage}: {error}"))
    })?;
    after_write().map_err(V5TransportError::ResponseLost)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[derive(Default)]
    struct ControlledWriter {
        bytes: Vec<u8>,
        fail_after: Option<usize>,
    }

    impl Write for ControlledWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = match self.fail_after {
                Some(limit) if self.bytes.len() >= limit => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "injected write failure",
                    ));
                }
                Some(limit) => bytes.len().min(limit - self.bytes.len()),
                None => bytes.len(),
            };
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn serialization_exhausting_budget_does_not_start_writing() {
        struct ExhaustBudget<'a>(&'a std::cell::Cell<bool>);
        impl serde::Serialize for ExhaustBudget<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.set(true);
                serializer.serialize_unit()
            }
        }
        let expired = std::cell::Cell::new(false);
        let mut writer = ControlledWriter::default();
        let result = write_request(
            &mut writer,
            &ExhaustBudget(&expired),
            "test",
            |_| {
                if expired.get() {
                    Err("deadline".to_string())
                } else {
                    Ok(())
                }
            },
            || Err("deadline".to_string()),
        );
        assert!(matches!(result, Err(V5TransportError::RequestNotSent(_))));
        assert!(writer.bytes.is_empty());
    }

    #[test]
    fn pre_write_deadline_cannot_send_a_frame() {
        let mut writer = ControlledWriter::default();
        let error = write_request(
            &mut writer,
            &serde_json::json!({"kind": "ping"}),
            "test",
            |_| Err("deadline".to_string()),
            || panic!("post-write checkpoint must not run"),
        )
        .unwrap_err();
        assert!(matches!(error, V5TransportError::RequestNotSent(_)));
        assert!(writer.bytes.is_empty());
    }

    #[test]
    fn partial_write_failure_requires_receipt_recovery() {
        let mut writer = ControlledWriter {
            fail_after: Some(4),
            ..Default::default()
        };
        let error = write_request(
            &mut writer,
            &serde_json::json!({"kind": "ping"}),
            "test",
            |_| Ok(()),
            || panic!("post-write checkpoint must not run"),
        )
        .unwrap_err();
        assert!(
            matches!(error, V5TransportError::ResponseLost(_)),
            "{error:?}"
        );
        assert_eq!(writer.bytes, b"{\"ki");
        assert!(!writer.bytes.contains(&b'\n'));
    }

    #[test]
    fn complete_write_followed_by_deadline_requires_receipt_recovery() {
        let mut writer = ControlledWriter::default();
        let error = write_request(
            &mut writer,
            &serde_json::json!({"kind": "ping"}),
            "test",
            |_| Ok(()),
            || Err("deadline".to_string()),
        )
        .unwrap_err();
        assert_eq!(writer.bytes, b"{\"kind\":\"ping\"}\n");
        assert!(
            matches!(error, V5TransportError::ResponseLost(_)),
            "{error:?}"
        );
    }

    #[test]
    fn complete_write_with_budget_sends_one_complete_frame() {
        let mut writer = ControlledWriter::default();
        write_request(
            &mut writer,
            &serde_json::json!({"kind": "ping"}),
            "test",
            |_| Ok(()),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(writer.bytes, b"{\"kind\":\"ping\"}\n");
    }
}

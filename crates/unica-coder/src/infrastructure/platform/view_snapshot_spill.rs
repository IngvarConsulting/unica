//! Элементы снимков продолжений `view` и `check` во временном файле.
//!
//! Один анонимный файл на процесс: у него нет имени в каталоге, и он исчезает
//! вместе с дескриптором, в том числе при аварийном завершении процесса.
//! Снимки только дописываются в его конец, поэтому число открытых файлов
//! не растёт с числом снимков. В памяти остаются только границы элементов.

use serde_json::Value;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use unica_navigation::result_store::{
    spilled_payload_charge, SnapshotSpill, SpilledItems, ViewCursorError,
};

/// Удерживаемые снимки продолжений `view` и `check`: элементы на диске, узлы
/// и границы в памяти. Временное число без замеров (#1119, #1312): выбрано
/// так, чтобы не срабатывать на рабочих конфигурациях.
pub(crate) const VIEW_SNAPSHOT_MAX_RETAINED_BYTES: usize = 4 * 1024 * 1024 * 1024;

/// Общий файл снимков процесса; открывается при первом снимке.
#[derive(Default)]
pub(crate) struct TempFileSpill {
    spool: Mutex<Option<Arc<Spool>>>,
}

struct Spool {
    /// Файл и его длина меняются вместе: дописывание и чтение не пересекаются.
    file: Mutex<SpoolFile>,
}

struct SpoolFile {
    file: std::fs::File,
    len: u64,
}

impl TempFileSpill {
    fn spool(&self) -> Result<Arc<Spool>, ViewCursorError> {
        let mut spool = self
            .spool
            .lock()
            .map_err(|_| ViewCursorError::OwnerUnavailable)?;
        if spool.is_none() {
            let file = tempfile::tempfile().map_err(|_| ViewCursorError::StorageUnavailable)?;
            *spool = Some(Arc::new(Spool {
                file: Mutex::new(SpoolFile { file, len: 0 }),
            }));
        }
        Ok(Arc::clone(spool.as_ref().expect("spool was just opened")))
    }

    #[cfg(test)]
    fn spool_len_for_test(&self) -> u64 {
        self.spool().unwrap().file.lock().unwrap().len
    }
}

impl SnapshotSpill for TempFileSpill {
    fn payload_charge(&self, node: &Value, items: &Vec<Value>) -> Result<usize, ViewCursorError> {
        spilled_payload_charge(node, items)
    }

    fn store(&self, items: Vec<Value>) -> Result<Box<dyn SpilledItems>, ViewCursorError> {
        let spool = self.spool()?;
        let mut bounds = Vec::new();
        bounds
            .try_reserve_exact(items.len().saturating_add(1))
            .map_err(|_| ViewCursorError::AllocationUnavailable)?;
        {
            let mut target = spool
                .file
                .lock()
                .map_err(|_| ViewCursorError::OwnerUnavailable)?;
            let start = target.len;
            target
                .file
                .seek(SeekFrom::Start(start))
                .map_err(|_| ViewCursorError::StorageUnavailable)?;
            bounds.push(start);
            let mut writer = CountingWriter {
                inner: BufWriter::new(&mut target.file),
                bytes: start,
            };
            for item in &items {
                serde_json::to_writer(&mut writer, item)
                    .map_err(|_| ViewCursorError::StorageUnavailable)?;
                bounds.push(writer.bytes);
            }
            let end = writer.bytes;
            writer
                .inner
                .flush()
                .map_err(|_| ViewCursorError::StorageUnavailable)?;
            drop(writer);
            // A failed append leaves the length unchanged: its partial bytes
            // are overwritten by the next snapshot and never read.
            target.len = end;
        }
        Ok(Box::new(FileItems {
            spool,
            bounds: bounds.into_boxed_slice(),
        }))
    }
}

struct FileItems {
    spool: Arc<Spool>,
    /// `bounds[i]..bounds[i + 1]` — байты i-го элемента в общем файле.
    bounds: Box<[u64]>,
}

impl SpilledItems for FileItems {
    fn len(&self) -> usize {
        self.bounds.len() - 1
    }

    fn read(&self, start: usize, count: usize) -> Result<Vec<Value>, ViewCursorError> {
        let end = start.saturating_add(count).min(self.len());
        if start >= end {
            return Ok(Vec::new());
        }
        let from = self.bounds[start];
        let length = usize::try_from(self.bounds[end] - from)
            .map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| ViewCursorError::AllocationUnavailable)?;
        {
            let mut source = self
                .spool
                .file
                .lock()
                .map_err(|_| ViewCursorError::OwnerUnavailable)?;
            source
                .file
                .seek(SeekFrom::Start(from))
                .map_err(|_| ViewCursorError::StorageUnavailable)?;
            (&mut source.file)
                .take(length as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| ViewCursorError::StorageUnavailable)?;
        }
        if bytes.len() != length {
            return Err(ViewCursorError::StorageUnavailable);
        }
        (start..end)
            .map(|index| {
                let lower = usize::try_from(self.bounds[index] - from)
                    .map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
                let upper = usize::try_from(self.bounds[index + 1] - from)
                    .map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
                serde_json::from_slice(&bytes[lower..upper])
                    .map_err(|_| ViewCursorError::StorageUnavailable)
            })
            .collect()
    }
}

struct CountingWriter<W: Write> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(chunk)?;
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use unica_navigation::result_store::{
        ViewCursorBinding, ViewCursorStore, DEFAULT_MAX_TOTAL_BYTES,
    };

    fn binding() -> ViewCursorBinding {
        ViewCursorBinding {
            canonical_at: "main:Catalog".into(),
            projection: "default".into(),
            source_set_identity: "main".into(),
            snapshot_id: "rev-1".into(),
            page_limit: 20,
        }
    }

    #[test]
    fn snapshot_beyond_the_former_memory_quota_pages_exactly_from_disk() {
        let text = "я".repeat(512 * 1024);
        // 72 MiB of item text: more than the former 64 MiB in-memory budget.
        let items = (0..72)
            .map(|index| json!({"index": index, "text": text}))
            .collect::<Vec<_>>();
        assert!(serde_json::to_vec(&items).unwrap().len() > DEFAULT_MAX_TOTAL_BYTES);
        let spill = Arc::new(TempFileSpill::default());
        let store =
            ViewCursorStore::new(VIEW_SNAPSHOT_MAX_RETAINED_BYTES).with_spill(spill.clone());
        let binding = binding();
        let token = store
            .insert_snapshot(
                binding.clone(),
                json!({"kind": "Catalog"}),
                items.clone(),
                1,
            )
            .unwrap();
        let page = store.read(&token, &binding.question()).unwrap();
        assert_eq!(page.snapshot.len(), 72);
        assert_eq!(page.snapshot.node(), &json!({"kind": "Catalog"}));
        assert_eq!(
            page.snapshot.read_items(1, 2).unwrap(),
            items[1..3].to_vec()
        );
        assert_eq!(
            page.snapshot.read_items(71, 5).unwrap(),
            items[71..].to_vec()
        );
        assert!(page.snapshot.read_items(72, 1).unwrap().is_empty());
        // The items are on disk: the shared spool holds exactly their bytes,
        // while memory keeps the node and the bounds.
        let item_bytes: usize = items
            .iter()
            .map(|item| serde_json::to_vec(item).unwrap().len())
            .sum();
        assert_eq!(spill.spool_len_for_test(), item_bytes as u64);
        let usage = store.retained_usage_for_test();
        assert!(usage.retained_bytes > DEFAULT_MAX_TOTAL_BYTES);
        assert!(usage.retained_bytes <= VIEW_SNAPSHOT_MAX_RETAINED_BYTES);
    }

    #[test]
    fn equal_snapshots_share_one_file_and_replay_the_same_pages() {
        let store = ViewCursorStore::new(VIEW_SNAPSHOT_MAX_RETAINED_BYTES)
            .with_spill(Arc::new(TempFileSpill::default()));
        let binding = binding();
        let items = vec![
            json!({"line": 1}),
            json!("кириллица"),
            json!(null),
            json!([1, 2]),
        ];
        let first = store
            .insert_snapshot(binding.clone(), json!({}), items.clone(), 1)
            .unwrap();
        let mut changed = binding.clone();
        changed.snapshot_id = "rev-2".into();
        let second = store
            .insert_snapshot(changed, json!({}), items.clone(), 1)
            .unwrap();
        let usage = store.retained_usage_for_test();
        assert_eq!((usage.records, usage.unique_payloads), (2, 1));
        for token in [&first, &second] {
            let page = store.read(token, &binding.question()).unwrap();
            assert_eq!(page.snapshot.items(), items);
            let next = store.insert_next(&page, 2, token).unwrap();
            assert_eq!(store.insert_next(&page, 2, token).unwrap(), next);
            let successor = store.read(&next, &binding.question()).unwrap();
            assert_eq!(
                successor.snapshot.read_items(successor.offset, 10).unwrap(),
                items[2..].to_vec()
            );
        }
    }
}

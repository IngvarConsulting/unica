use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must follow epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "unica-compile-transaction-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("temporary root must be created");
    root
}

pub(super) fn configuration_bytes() -> Vec<u8> {
    let text = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" ",
        "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
        "xmlns:cfg=\"urn:kept-only-as-qname\" xsi:type=\"cfg:MetaDataObject\">\r\n",
        "\t<Configuration>\r\n",
        "\t\t<ChildObjects>\r\n",
        "\t\t\t<Catalog>Items</Catalog>\r\n",
        "\t\t</ChildObjects>\r\n",
        "\t</Configuration>\r\n",
        "</MetaDataObject><!--tail stays exact-->"
    );
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

pub(super) fn assert_no_bare_lf(bytes: &[u8]) {
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            assert!(index > 0 && bytes[index - 1] == b'\r', "bare LF at {index}");
        }
    }
}

pub(super) fn transaction_debris(root: &Path) -> Vec<PathBuf> {
    fn visit(path: &Path, result: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.contains(".unica-stage-")
                        || name.contains(".unica-backup-")
                        || name.contains(".unica-recovery-")
                })
            {
                result.push(path.clone());
            }
            if path.is_dir() {
                visit(&path, result);
            }
        }
    }

    let mut result = Vec::new();
    visit(root, &mut result);
    result
}

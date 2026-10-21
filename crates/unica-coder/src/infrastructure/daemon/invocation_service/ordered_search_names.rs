//! Directory names in native `OsString` order, without a corpus entry quota.
//! A batch bounds resident names, not accepted input. Pairwise merge rounds
//! keep only three run handles open, irrespective of directory width.
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const BATCH_NAMES: usize = 1024;

pub(super) enum OrderedNames {
    Memory {
        names: Vec<OsString>,
        forward: usize,
        backward: usize,
    },
    Disk {
        // The private directory owns every run, including partially written
        // runs on cancellation/error. Drop the reader before its directory.
        reader: BufReader<File>,
        forward: u64,
        backward: u64,
        end: u64,
        _temporary: tempfile::TempDir,
    },
}

fn run_path(root: &Path, generation: usize, index: usize) -> PathBuf {
    root.join(format!("{generation}-{index}"))
}

fn write_name(writer: &mut impl Write, name: &OsStr) -> io::Result<()> {
    // Serde's OsString representation preserves Unix bytes / Windows wide
    // units, including names that cannot be represented as UTF-8.
    let bytes = serde_json::to_vec(name).map_err(io::Error::other)?;
    let length = u64::try_from(bytes.len()).map_err(io::Error::other)?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.write_all(&length.to_le_bytes())
}

fn read_name(reader: &mut impl Read) -> io::Result<Option<OsString>> {
    let mut length = [0; 8];
    if reader.read(&mut length[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut length[1..])?;
    let size = usize::try_from(u64::from_le_bytes(length)).map_err(io::Error::other)?;
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    let mut footer = [0; 8];
    reader.read_exact(&mut footer)?;
    if footer != length {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "search name spool record is corrupt",
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

fn merge(
    left: &Path,
    right: &Path,
    output: &Path,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let mut left = BufReader::new(File::open(left)?);
    let mut right = BufReader::new(File::open(right)?);
    let mut output = BufWriter::new(File::create(output)?);
    let mut a = read_name(&mut left)?;
    let mut b = read_name(&mut right)?;
    while a.is_some() || b.is_some() {
        checkpoint()?;
        if b.is_none() || a.as_ref().is_some_and(|a| Some(a) <= b.as_ref()) {
            write_name(&mut output, a.as_ref().expect("selected left name"))?;
            a = read_name(&mut left)?;
        } else {
            write_name(&mut output, b.as_ref().expect("selected right name"))?;
            b = read_name(&mut right)?;
        }
    }
    output.flush()
}

impl OrderedNames {
    pub(super) fn collect(
        directory: &RetainedDirectoryCapability,
        checkpoint: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        Self::collect_with_temporary(directory, checkpoint, || {
            tempfile::Builder::new()
                .prefix("unica-search-names-")
                .tempdir()
        })
    }

    fn collect_with_temporary(
        directory: &RetainedDirectoryCapability,
        mut checkpoint: impl FnMut() -> io::Result<()>,
        mut create_temporary: impl FnMut() -> io::Result<tempfile::TempDir>,
    ) -> io::Result<Self> {
        let mut names: Vec<OsString> = Vec::with_capacity(BATCH_NAMES);
        let mut temporary: Option<tempfile::TempDir> = None;
        let mut count = 0_usize;
        directory.visit_immediate_names(|name| -> io::Result<()> {
            checkpoint()?;
            if names.len() == BATCH_NAMES {
                if temporary.is_none() {
                    temporary = Some(create_temporary()?);
                }
                let temporary = temporary.as_ref().expect("created search spool directory");
                names.sort();
                let mut writer =
                    BufWriter::new(File::create(run_path(temporary.path(), 0, count))?);
                for name in names.drain(..) {
                    checkpoint()?;
                    write_name(&mut writer, &name)?;
                }
                writer.flush()?;
                count = count
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("search run count overflow"))?;
            }
            names.push(name);
            Ok(())
        })?;
        names.sort();
        let Some(temporary) = temporary else {
            return Ok(Self::Memory {
                backward: names.len(),
                names,
                forward: 0,
            });
        };
        let mut writer = BufWriter::new(File::create(run_path(temporary.path(), 0, count))?);
        for name in names {
            checkpoint()?;
            write_name(&mut writer, &name)?;
        }
        writer.flush()?;
        drop(writer);
        count = count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("search run count overflow"))?;
        let mut generation = 0;
        while count > 1 {
            let next_count = count.div_ceil(2);
            for index in 0..next_count {
                checkpoint()?;
                let left = run_path(temporary.path(), generation, index * 2);
                let output = run_path(temporary.path(), generation + 1, index);
                if index * 2 + 1 < count {
                    let right = run_path(temporary.path(), generation, index * 2 + 1);
                    merge(&left, &right, &output, &mut checkpoint)?;
                    std::fs::remove_file(left)?;
                    std::fs::remove_file(right)?;
                } else {
                    std::fs::rename(left, output)?;
                }
            }
            count = next_count;
            generation += 1;
        }
        let file = File::open(run_path(temporary.path(), generation, 0))?;
        let end = file.metadata()?.len();
        Ok(Self::Disk {
            reader: BufReader::new(file),
            forward: 0,
            backward: end,
            end,
            _temporary: temporary,
        })
    }

    pub(super) fn next(&mut self, reverse: bool) -> io::Result<Option<OsString>> {
        match self {
            Self::Memory {
                names,
                forward,
                backward,
            } => {
                if reverse {
                    if *backward == 0 {
                        return Ok(None);
                    }
                    *backward -= 1;
                    Ok(Some(names[*backward].clone()))
                } else if *forward == names.len() {
                    Ok(None)
                } else {
                    let name = names[*forward].clone();
                    *forward += 1;
                    Ok(Some(name))
                }
            }
            Self::Disk {
                reader,
                forward,
                backward,
                end,
                ..
            } => {
                if reverse {
                    if *backward == 0 {
                        return Ok(None);
                    }
                    let footer_position = backward
                        .checked_sub(8)
                        .ok_or_else(|| io::Error::other("search name spool is truncated"))?;
                    reader.seek(SeekFrom::Start(footer_position))?;
                    let mut length = [0; 8];
                    reader.read_exact(&mut length)?;
                    *backward = footer_position
                        .checked_sub(u64::from_le_bytes(length))
                        .and_then(|position| position.checked_sub(8))
                        .ok_or_else(|| io::Error::other("search name spool is truncated"))?;
                    reader.seek(SeekFrom::Start(*backward))?;
                } else {
                    if *forward == *end {
                        return Ok(None);
                    }
                    reader.seek(SeekFrom::Start(*forward))?;
                }
                let name = read_name(reader)?.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "search name spool is truncated",
                    )
                })?;
                if !reverse {
                    *forward = reader.stream_position()?;
                }
                Ok(Some(name))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide_directory() -> (
        tempfile::TempDir,
        RetainedDirectoryCapability,
        Vec<OsString>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let mut expected = Vec::new();
        for index in (0..(BATCH_NAMES * 3 + 17)).rev() {
            let name = OsString::from(format!("имя-{index:05}"));
            File::create(root.path().join(&name)).unwrap();
            expected.push(name);
        }
        expected.sort();
        let directory =
            RetainedDirectoryCapability::open(&root.path().canonicalize().unwrap()).unwrap();
        (root, directory, expected)
    }

    #[test]
    fn multiple_merge_rounds_preserve_native_order_in_both_directions_and_clean_up() {
        let (_root, directory, expected) = wide_directory();
        let mut names = OrderedNames::collect(&directory, || Ok(())).unwrap();
        let temporary = match &names {
            OrderedNames::Disk { _temporary, .. } => _temporary.path().to_owned(),
            _ => panic!("wide directory must spill"),
        };
        let mut actual = Vec::new();
        while let Some(name) = names.next(false).unwrap() {
            actual.push(name);
        }
        assert_eq!(actual, expected);
        let mut reverse = Vec::new();
        while let Some(name) = names.next(true).unwrap() {
            reverse.push(name);
        }
        assert_eq!(reverse, expected.into_iter().rev().collect::<Vec<_>>());
        drop(names);
        assert!(!temporary.exists());
    }

    #[test]
    fn cancelled_merge_releases_every_temporary_run() {
        let (_root, directory, _) = wide_directory();
        let parent = tempfile::tempdir().unwrap();
        let mut checkpoints = 0;
        let result = OrderedNames::collect_with_temporary(
            &directory,
            || {
                checkpoints += 1;
                // Enumeration and writing initial runs each visit every name.
                if checkpoints > 2 * (BATCH_NAMES * 3 + 17) + 10 {
                    Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
                } else {
                    Ok(())
                }
            },
            || tempfile::tempdir_in(parent.path()),
        );
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::Interrupted));
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }

    #[test]
    fn merge_read_failure_is_not_a_complete_empty_run() {
        let temporary = tempfile::tempdir().unwrap();
        let left = temporary.path().join("left");
        let right = temporary.path().join("right");
        let output = temporary.path().join("output");
        let mut file = File::create(&left).unwrap();
        write_name(&mut file, OsStr::new("a")).unwrap();
        file.write_all(&[1, 2]).unwrap();
        File::create(&right).unwrap();
        let error = merge(&left, &right, &output, &mut || Ok(())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn temporary_storage_failure_propagates_and_early_drop_cleans_up() {
        let (_root, directory, _) = wide_directory();
        let result = OrderedNames::collect_with_temporary(
            &directory,
            || Ok(()),
            || {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "temporary storage unavailable",
                ))
            },
        );
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::NotFound));
        let mut names = OrderedNames::collect(&directory, || Ok(())).unwrap();
        let temporary = match &names {
            OrderedNames::Disk { _temporary, .. } => _temporary.path().to_owned(),
            _ => panic!("wide directory must spill"),
        };
        assert!(names.next(false).unwrap().is_some());
        drop(names);
        assert!(!temporary.exists());
    }

    #[test]
    fn spool_record_preserves_every_native_name() {
        let mut names = vec![OsString::from("Unicode-имя-😀"), OsString::from("a")];
        // OsString deserialization supplies the platform-native invalid
        // sequence without introducing OS-specific operations outside platform.
        let native = serde_json::to_value(OsString::new()).unwrap();
        let invalid = if native.get("Unix").is_some() {
            serde_json::json!({"Unix": [255, 97]})
        } else {
            serde_json::json!({"Windows": [55296, 97]})
        };
        names.push(serde_json::from_value(invalid).unwrap());
        for name in names {
            let mut bytes = Vec::new();
            write_name(&mut bytes, &name).unwrap();
            assert_eq!(read_name(&mut bytes.as_slice()).unwrap(), Some(name));
        }
    }
}

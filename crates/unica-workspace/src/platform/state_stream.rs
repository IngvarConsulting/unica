//! Existing service-state stream modes; no source publication capability.
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

pub(crate) fn open_state_reader(path: &Path) -> io::Result<File> {
    File::open(path)
}

pub(crate) fn open_state_lock(path: &Path, create_if_absent: bool) -> io::Result<File> {
    OpenOptions::new()
        .create(create_if_absent)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
}

pub(crate) fn create_new_state_stage(path: &Path) -> io::Result<File> {
    OpenOptions::new().create_new(true).write(true).open(path)
}

pub(crate) fn create_state_stage(path: &Path) -> io::Result<File> {
    File::create(path)
}

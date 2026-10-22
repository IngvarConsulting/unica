//! Owned child log streams, opened before command construction at caller checkpoints.
use std::fs::File;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct ProcessLog {
    file: File,
}

impl ProcessLog {
    pub fn create(path: &Path) -> io::Result<Self> {
        File::create(path).map(|file| Self { file })
    }

    pub fn attach_stdout(self, command: &mut Command) {
        command.stdout(Stdio::from(self.file));
    }

    pub fn attach_stderr(self, command: &mut Command) {
        command.stderr(Stdio::from(self.file));
    }
}

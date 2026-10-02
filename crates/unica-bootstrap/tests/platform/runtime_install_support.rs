use std::{fs, io, path::Path};

pub fn open_for_timestamp(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true);
    if path.is_dir() {
        options.write(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0200_0000); // FILE_FLAG_BACKUP_SEMANTICS
            options.access_mode(0x0100); // FILE_WRITE_ATTRIBUTES for SetFileTime
        }
    }
    options.open(path)
}

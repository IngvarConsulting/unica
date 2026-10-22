//! Each marked line calls one disallowed method and must be rejected.
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

pub fn production(path: &Path, owned: &PathBuf) {
    let _ = path.exists(); // probe: std::path::Path::exists
    let _ = path.try_exists(); // probe: std::path::Path::try_exists
    let _ = owned.is_file(); // probe: std::path::Path::is_file
    let _ = owned.is_dir(); // probe: std::path::Path::is_dir
    let _ = path.is_symlink(); // probe: std::path::Path::is_symlink
    let _ = path.metadata(); // probe: std::path::Path::metadata
    let _ = path.symlink_metadata(); // probe: std::path::Path::symlink_metadata
    let _ = path.read_link(); // probe: std::path::Path::read_link
    let _ = File::open(path); // probe: std::fs::File::open
    let _ = File::create(path); // probe: std::fs::File::create
    let _ = File::create_new(path); // probe: std::fs::File::create_new
    let _ = OpenOptions::new().open(path); // probe: std::fs::OpenOptions::open
}

#[cfg(test)]
mod tests {
    // Unit tests live in `src/` and stay under the same boundary.
    pub fn unit(path: &std::path::Path) {
        let _ = path.read_dir(); // probe: std::path::Path::read_dir
        let _ = path.canonicalize(); // probe: std::path::Path::canonicalize
    }
}

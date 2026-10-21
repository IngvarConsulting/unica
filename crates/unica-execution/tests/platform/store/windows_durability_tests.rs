#![cfg(windows)]

use std::ffi::OsStr;
use std::{fs, io};
use unica_workspace::state_fs::*;

fn directory(path: &std::path::Path) -> StateDirectory {
    StateDirectory::open(&fs::canonicalize(path).unwrap()).unwrap()
}

#[cfg(windows)]
#[test]
fn unsupported_durability_refuses_before_creating_a_stage_or_namespace() {
    use crate::store::namespace::AuthorityRoot;
    use crate::store::OpenError;
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    assert_eq!(
        check_durability_support().unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(
        Stage::create(&parent, OsStr::new(".stage.test"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(
        create_owner_only_directory(&parent, OsStr::new("new"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);

    // Exercise the public bootstrap facade as well as its primitives. A
    // missing target must not create a root; an existing directory must fail
    // with Unsupported before treating its contents as a bootstrap candidate.
    let named_root = temp.path().join("authority");
    fs::write(temp.path().join("sentinel"), b"foreign root contents").unwrap();
    for result in [
        AuthorityRoot::provision(&named_root, [1; 32]),
        AuthorityRoot::open_existing(temp.path(), [1; 32]),
    ] {
        match result {
            Err(OpenError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::Unsupported),
            Err(error) => panic!("unsupported bootstrap returned another failure: {error:?}"),
            Ok(_) => panic!("unsupported bootstrap admitted a physical owner"),
        }
    }
    assert!(!named_root.exists());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(
        fs::read(temp.path().join("sentinel")).unwrap(),
        b"foreign root contents"
    );
}

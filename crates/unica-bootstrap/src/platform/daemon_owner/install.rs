use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use unica_artifact_delivery::{HostTarget, RuntimeManifest};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};

const BOOTSTRAP: &str = "bootstrap/bin/win-x64/unica-bootstrap.exe";
const MANIFEST: &str = "runtime-manifest.json";

pub(super) struct InstalledOwner {
    pub(super) identity: String,
    pub(super) entrypoint: FilePin,
    pub(super) bootstrap: FilePin,
    pub(super) provider: FilePin,
    pub(super) manifest: FilePin,
    // Keeps the approved installation's directory identity stable.
    _plugin: FilePin,
}

impl InstalledOwner {
    pub(super) fn bind(
        plugin: &Path,
        provider: &Path,
        entrypoint: &Path,
        runtime: &RuntimeManifest,
    ) -> io::Result<Self> {
        if runtime.development {
            return Err(invalid(
                "the daemon owner requires an installed release package",
            ));
        }
        let host = HostTarget::current().map_err(|error| invalid(error.to_string()))?;
        let target = runtime
            .target(host)
            .map_err(|error| invalid(error.to_string()))?;
        let expected = target
            .files
            .iter()
            .find(|file| target.entrypoint.as_deref() == Some(file.path.as_str()))
            .ok_or_else(|| invalid("the installed core has no manifest digest"))?;
        let plugin = FilePin::directory(plugin)?;
        let provider = FilePin::directory(provider)?;
        let bootstrap = FilePin::file(&plugin.path.join(BOOTSTRAP))?;
        if !bootstrap.matches(&std::env::current_exe()?)? {
            return Err(invalid(
                "daemon owner does not belong to the approved installation",
            ));
        }
        let manifest = FilePin::file(&plugin.path.join(MANIFEST))?;
        // The caller's installer verified the delivery. Bind to the exact
        // files it checked so a later cache or manifest replacement is refused.
        let retained =
            RuntimeManifest::load(&manifest.path).map_err(|error| invalid(error.to_string()))?;
        let retained = retained
            .target(host)
            .map_err(|error| invalid(error.to_string()))?;
        if retained.asset.sha256 != target.asset.sha256 {
            return Err(invalid("the installed runtime manifest changed"));
        }
        let entrypoint = FilePin::file(entrypoint)?;
        let core_digest = entrypoint.digest()?;
        if core_digest != expected.sha256 {
            return Err(invalid("the installed Unica executable changed"));
        }
        let mut digest = Sha256::new();
        for pin in [&plugin, &provider, &bootstrap, &manifest, &entrypoint] {
            for value in pin.id {
                digest.update(value.to_le_bytes());
            }
        }
        for bytes in [manifest.digest()?, bootstrap.digest()?, core_digest] {
            digest.update(bytes.as_bytes());
        }
        Ok(Self {
            identity: format!("{:x}", digest.finalize()),
            entrypoint,
            bootstrap,
            provider,
            manifest,
            _plugin: plugin,
        })
    }
}

pub(super) struct FilePin {
    pub(super) path: PathBuf,
    file: File,
    id: [u32; 3],
}

impl FilePin {
    pub(super) fn file(path: &Path) -> io::Result<Self> {
        Self::open(path, false)
    }

    fn directory(path: &Path) -> io::Result<Self> {
        Self::open(path, true)
    }

    fn open(path: &Path, directory: bool) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(invalid("daemon owner paths must be absolute"));
        }
        let path = path.canonicalize()?;
        let mut options = OpenOptions::new();
        options.read(true).share_mode(FILE_SHARE_READ);
        if directory {
            options
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
        }
        let file = options.open(&path)?;
        if file.metadata()?.is_dir() != directory {
            return Err(invalid("daemon owner path has an unexpected file type"));
        }
        Ok(Self {
            path,
            id: file_id(&file)?,
            file,
        })
    }

    pub(super) fn matches(&self, path: &Path) -> io::Result<bool> {
        Ok(self.id == Self::file(path)?.id)
    }

    fn digest(&self) -> io::Result<String> {
        let mut file = self.file.try_clone()?;
        // Duplicated file handles share a cursor on Windows.
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(0))?;
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                return Ok(format!("{:x}", digest.finalize()));
            }
            digest.update(&buffer[..count]);
        }
    }
}

fn file_id(file: &File) -> io::Result<[u32; 3]> {
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: the file remains open and Windows initializes the output on success.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call initialized the data.
    let info = unsafe { info.assume_init() };
    Ok([
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ])
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod medium_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn approved_image_rejects_a_different_file_with_the_same_bytes() {
        let mut original = tempfile::NamedTempFile::new().unwrap();
        let mut copy = tempfile::NamedTempFile::new().unwrap();
        for file in [&mut original, &mut copy] {
            file.write_all(b"fixture executable").unwrap();
            file.flush().unwrap();
        }
        let original = original.into_temp_path();
        let copy = copy.into_temp_path();
        let pin = FilePin::file(&original).unwrap();
        assert!(pin.matches(&original).unwrap());
        assert!(!pin.matches(&copy).unwrap());
        assert_eq!(
            pin.digest().unwrap(),
            FilePin::file(&copy).unwrap().digest().unwrap()
        );
    }
}

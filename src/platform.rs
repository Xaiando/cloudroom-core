//! Files inherit the state root's Windows ACL. Unix permission bits have no Windows meaning.
use std::{fs, io, path::Path, process::ExitStatus};

pub(crate) trait OpenOptionsExt {
    fn unix_mode(&mut self, mode: u32) -> &mut Self;
}
impl OpenOptionsExt for fs::OpenOptions {
    fn unix_mode(&mut self, _mode: u32) -> &mut Self {
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(self, _mode);
        self
    }
}
pub(crate) trait DirBuilderExt {
    fn unix_mode(&mut self, mode: u32) -> &mut Self;
}
impl DirBuilderExt for fs::DirBuilder {
    fn unix_mode(&mut self, _mode: u32) -> &mut Self {
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(self, _mode);
        self
    }
}

pub(crate) fn sync_directory(_path: &Path) -> io::Result<()> {
    // Windows cannot FlushFileBuffers on a directory opened by std::fs::File. Records are
    // flushed before replacement; recovery still validates the contiguous journal prefix.
    #[cfg(unix)]
    fs::File::open(_path)?.sync_all()?;
    Ok(())
}
pub(crate) fn exit_signal(_status: ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        std::os::unix::process::ExitStatusExt::signal(&_status)
    }
    #[cfg(windows)]
    {
        None
    }
}
pub(crate) fn device(_metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        std::os::unix::fs::MetadataExt::dev(_metadata)
    }
    // Linux storage policies are rejected on Windows; the device comparison is never an admission check.
    #[cfg(windows)]
    {
        0
    }
}
pub(crate) fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (a.dev(), a.ino()) == (b.dev(), b.ino())
    }
    #[cfg(windows)]
    {
        // Pi is not enabled on Windows yet; do not infer file identity from timestamps.
        let _ = (a, b);
        false
    }
}

pub(crate) fn random(bytes: &mut [u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Read;
        fs::File::open("/dev/urandom")?.read_exact(bytes)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
        };
        let length = u32::try_from(bytes.len()).map_err(io::Error::other)?;
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                length,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 {
            Err(io::Error::other("Windows random generation failed"))
        } else {
            Ok(())
        }
    }
}

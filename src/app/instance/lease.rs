//! Reusable lock files avoid stale-PID checks and unlink/recreate lock races.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

pub(super) struct Lease {
    // Closing the descriptor releases flock, including after process death.
    _file: File,
}
impl Lease {
    pub(super) fn acquire(id: &str) -> Result<Option<Self>> {
        // A desktop normally supplies XDG_RUNTIME_DIR. The fallback is private
        // to this uid, never a predictable socket or writable shared directory.
        let root = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(root) => {
                let root = PathBuf::from(root);
                private_directory(&root, false)?;
                root.join("talis")
            }
            None => {
                std::env::temp_dir().join(format!("talis-runtime-{}", unsafe { libc::geteuid() }))
            }
        };
        private_directory(&root, true)?;
        Self::at(&root.join(format!("{id}.lock")))
    }

    pub(super) fn at(path: &Path) -> Result<Option<Self>> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .context("Open desktop instance lease")?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Unsafe desktop instance lease"
        );
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            Ok(Some(Self { _file: file }))
        } else {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                Ok(None)
            } else {
                Err(error).context("Lock desktop instance lease")
            }
        }
    }
}
fn private_directory(path: &Path, create: bool) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "Desktop runtime directory must be absolute"
    );
    if create {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error).context("Create desktop runtime directory"),
        }
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "Desktop runtime directory must be a private, owned directory"
    );
    Ok(())
}

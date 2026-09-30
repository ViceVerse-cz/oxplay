// SPDX-License-Identifier: GPL-3.0-or-later
//! Admission for one previously measured synthetic MP4, not an egress sandbox.
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const NAME: &str = "native-child-fixture.mp4";
const MAX_BYTES: u64 = 128 * 1024 * 1024;
// Published prior evidence: docs/soak-local.md. No new hash was computed during
// the exclusive measurement hold. This is a content identity, not a filename.
const SHA256: [u8; 32] = [
    0xd5, 0xbd, 0x61, 0x30, 0x43, 0x5a, 0xad, 0x2f, 0x07, 0xb0, 0x0f, 0xf1, 0x02, 0xc5, 0x6d, 0x04,
    0xf6, 0x49, 0xc7, 0x0e, 0x47, 0x9e, 0xc7, 0x62, 0xfe, 0x58, 0xf6, 0x59, 0xb9, 0xba, 0xab, 0xa0,
];

pub struct Fixture {
    path: PathBuf,
    #[cfg(unix)]
    directory: File,
}

fn invalid() -> io::Error {
    io::Error::other(
        "Native child requires the exact previously measured synthetic MP4 in a new private profile",
    )
}

impl Fixture {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Startup only, before UI/engine creation. Reads one opened no-follow
    /// regular file and plays the verified owned copy, closing a hash/reopen gap.
    pub fn prepare(source: &Path, root: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::{
                fd::{AsRawFd, FromRawFd},
                unix::fs::{MetadataExt, OpenOptionsExt},
            };
            let mut source = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(source)
                .map_err(|_| invalid())?;
            let metadata = source.metadata().map_err(|_| invalid())?;
            if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_BYTES {
                return Err(invalid());
            }
            let directory = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(root)
                .map_err(|_| invalid())?;
            let metadata = directory.metadata().map_err(|_| invalid())?;
            // SAFETY: getuid has no pointer/lifetime requirements.
            if !metadata.is_dir()
                || metadata.mode() & 0o777 != 0o700
                || metadata.uid() != unsafe { libc::getuid() }
            {
                return Err(invalid());
            }
            // SAFETY: live directory descriptor and constant NUL-terminated
            // basename; create_new refuses existing files and symlinks.
            let descriptor = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    c"native-child-fixture.mp4".as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if descriptor < 0 {
                return Err(invalid());
            }
            // SAFETY: successful openat transfers one owned descriptor.
            let mut output = unsafe { File::from_raw_fd(descriptor) };
            let fixture = Self {
                path: root.join(NAME),
                directory,
            };
            // Fixture owns cleanup from here, including all partial-copy failures.
            copy_verified(&mut source, &mut output, MAX_BYTES, &SHA256)?;
            output.sync_all().map_err(|_| invalid())?;
            Ok(fixture)
        }
        #[cfg(not(unix))]
        {
            let _ = (source, root);
            Err(invalid())
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: cleanup remains confined to the retained root descriptor.
            if unsafe {
                libc::unlinkat(
                    self.directory.as_raw_fd(),
                    c"native-child-fixture.mp4".as_ptr(),
                    0,
                )
            } != 0
            {
                eprintln!("native diagnostic fixture cleanup incomplete");
            }
        }
    }
}

fn copy_verified(
    source: &mut impl Read,
    output: &mut impl Write,
    limit: u64,
    expected: &[u8; 32],
) -> io::Result<()> {
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer).map_err(|_| invalid())?;
        if count == 0 {
            break;
        }
        bytes = bytes.checked_add(count as u64).ok_or_else(invalid)?;
        if bytes > limit {
            return Err(invalid());
        }
        output.write_all(&buffer[..count]).map_err(|_| invalid())?;
        digest.update(&buffer[..count]);
    }
    if bytes == 0 || digest.finish().as_ref() != expected {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_complete_matching_content_is_admitted_and_copy_is_bounded() {
        let bytes = b"Synthetic test content, not a playable fixture";
        let expected: [u8; 32] = ring::digest::digest(&ring::digest::SHA256, bytes)
            .as_ref()
            .try_into()
            .unwrap();
        let mut output = Vec::new();
        copy_verified(&mut &bytes[..], &mut output, bytes.len() as u64, &expected).unwrap();
        assert_eq!(output, bytes);
        assert!(
            copy_verified(
                &mut &bytes[..bytes.len() - 1],
                &mut Vec::new(),
                MAX_BYTES,
                &expected
            )
            .is_err()
        );
        assert!(copy_verified(&mut &b""[..], &mut Vec::new(), MAX_BYTES, &expected).is_err());
        let mut changed = bytes.to_vec();
        changed[0] ^= 1;
        assert!(
            copy_verified(
                &mut changed.as_slice(),
                &mut Vec::new(),
                MAX_BYTES,
                &expected
            )
            .is_err()
        );
        let mut bounded = Vec::new();
        assert!(copy_verified(&mut &bytes[..], &mut bounded, 4, &expected).is_err());
        assert!(bounded.len() <= 4);
    }

    #[cfg(unix)]
    #[test]
    fn failed_admission_cleans_only_its_created_copy_and_rejects_links_and_fifo() {
        use std::{
            ffi::CString,
            os::unix::{ffi::OsStrExt, fs::DirBuilderExt},
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = std::env::temp_dir().join(format!(
            "oxplay-native-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let _cleanup = Cleanup(root.clone());
        let source = root.join("synthetic.txt");
        std::fs::write(&source, b"Synthetic non-video content").unwrap();
        assert!(Fixture::prepare(&source, &root).is_err());
        assert!(
            !root.join(NAME).exists(),
            "failed verification leaked its private copy"
        );
        std::os::unix::fs::symlink(&source, root.join("link")).unwrap();
        assert!(Fixture::prepare(&root.join("link"), &root).is_err());
        let fifo = root.join("fifo");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: temporary NUL-terminated pathname; no reader/writer is opened.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(Fixture::prepare(&fifo, &root).is_err());
        std::fs::write(root.join(NAME), b"Synthetic preexisting owned file").unwrap();
        assert!(Fixture::prepare(&source, &root).is_err());
        assert_eq!(
            std::fs::read(root.join(NAME)).unwrap(),
            b"Synthetic preexisting owned file"
        );
    }
}

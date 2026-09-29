// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit developer fixture capability. Remote metadata cannot create one.
//! Admission happens before the UI; compressed bytes are read only by the image worker.
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    sync::Arc,
};

pub const THUMBNAILS: usize = 30;
pub const ITEMS: usize = 10_000;
pub const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024;
const MANIFEST: &str = ".serein-library-resource-fixture-v1";
const HEADER: &str = "SEREIN_LIBRARY_RESOURCE_FIXTURE_V1\nitems=10000\nimages=30\n";

#[derive(Clone)]
pub struct Config {
    source: Arc<FixtureSource>,
}
pub struct FixtureSource {
    directory: File,
    lengths: [u64; THUMBNAILS],
}
fn invalid() -> io::Error {
    io::Error::other("Invalid or unavailable private library resource fixture")
}

impl Config {
    pub fn admit(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            if !path.is_absolute() {
                return Err(invalid());
            }
            let directory = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)?;
            private_directory(&directory)?;
            let manifest = open_regular(&directory, MANIFEST, 4096)?;
            let mut text = String::new();
            manifest.take(4097).read_to_string(&mut text)?;
            let lengths = parse_manifest(&text)?;
            for (index, expected) in lengths.iter().enumerate() {
                let file = open_regular(&directory, &filename(index), MAX_IMAGE_BYTES)?;
                if file.metadata()?.len() != *expected {
                    return Err(invalid());
                }
            }
            let profile = open_at(&directory, "Serein", libc::O_DIRECTORY)?;
            private_directory(&profile)?;
            let database = open_regular(&profile, "library.sqlite3", 16 * 1024 * 1024)?;
            if database.metadata()?.len() == 0 {
                return Err(invalid());
            }
            Ok(Self {
                source: Arc::new(FixtureSource { directory, lengths }),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(invalid())
        }
    }
    pub fn source(&self) -> Arc<FixtureSource> {
        self.source.clone()
    }
}
impl FixtureSource {
    pub fn read(&self, index: usize) -> io::Result<Vec<u8>> {
        let expected = *self.lengths.get(index).ok_or_else(invalid)?;
        #[cfg(unix)]
        {
            let file = open_regular(&self.directory, &filename(index), MAX_IMAGE_BYTES)?;
            if file.metadata()?.len() != expected {
                return Err(invalid());
            }
            let mut bytes = Vec::with_capacity(expected as usize);
            file.take(expected + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 != expected {
                return Err(invalid());
            }
            Ok(bytes)
        }
        #[cfg(not(unix))]
        {
            let _ = expected;
            Err(invalid())
        }
    }
}
fn filename(index: usize) -> String {
    format!("thumb-{index:02}.png")
}
fn parse_manifest(text: &str) -> io::Result<[u64; THUMBNAILS]> {
    let mut lines = text.strip_prefix(HEADER).ok_or_else(invalid)?.lines();
    let mut lengths = [0; THUMBNAILS];
    for (index, length) in lengths.iter_mut().enumerate() {
        let prefix = format!("{} ", filename(index));
        let raw = lines
            .next()
            .and_then(|line| line.strip_prefix(&prefix))
            .ok_or_else(invalid)?;
        if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }
        *length = raw.parse().map_err(|_| invalid())?;
        if !(1..=MAX_IMAGE_BYTES).contains(length) {
            return Err(invalid());
        }
    }
    if lines.next().is_some() || !text.ends_with('\n') {
        return Err(invalid());
    }
    Ok(lengths)
}

/// Reuse the persistent production feed only after the ordinary library worker
/// acknowledges a bounded SQLite page. No fixture provider result is invented.
pub fn publish(app: &crate::App, state: &crate::UiState, videos: &[serein_core::VideoSummary]) {
    use slint::ComponentHandle;
    if state.library_fixture.is_none() {
        return;
    }
    if videos.len() > 100 {
        app.set_status("TEST FIXTURE — refused an oversized library page".into());
        return;
    }
    state.thumbnails.borrow_mut().replace(Vec::new());
    state.thumbnail_range.set((usize::MAX, usize::MAX));
    state.thumbnail_attempted.borrow_mut().clear();
    crate::feed_focus::reset(app, state);
    state.model.replace(
        videos
            .iter()
            .map(|video| {
                let mut row = crate::video_row(video);
                row.kind = "TEST FIXTURE".into();
                row
            })
            .collect(),
    );
    state.groups.replace(&state.model);
    app.set_catalog_title(format!("TEST FIXTURE — {ITEMS} local library items").into());
    app.set_catalog_subtitle(
        "Offline raster fixtures · 100 items per bounded page · Playback and search disabled"
            .into(),
    );
    app.set_guest_scope(3);
    app.set_guest_can_back(false);
    app.set_guest_can_follow(false);
    let library = app.global::<crate::LibraryUi>();
    app.set_has_more(library.get_next());
    app.set_has_previous(library.get_previous());
    app.set_page(0);
    app.set_status("TEST FIXTURE — local synthetic metadata; no online catalog results".into());
    app.invoke_refresh_visible();
}
/// Install only after ordinary bindings, only for an explicitly admitted fixture.
/// No diagnostic identity may reach a provider or launch an external browser.
pub fn bind(app: &crate::App, state: &std::rc::Rc<crate::UiState>) {
    use slint::ComponentHandle;
    if state.library_fixture.is_none() {
        return;
    }
    macro_rules! block {
        ($callback:ident, ($($argument:ident),*)) => {{
            let weak = app.as_weak();
            app.$callback(move |$($argument),*| {
                $(let _ = $argument;)*
                if let Some(app) = weak.upgrade() {
                    app.set_status("TEST FIXTURE — online actions and media playback are disabled".into());
                }
            });
        }};
    }
    block!(on_search, (query));
    block!(on_select_video, (index));
    block!(on_guest_back, ());
    block!(on_search_kind_changed, (index));
    block!(on_channel_tab_changed, (index));
    block!(on_follow_guest_channel, ());
    block!(on_account_import, ());
    block!(on_account_import_path, (path, remember));
    block!(on_account_reconnect, ());
    block!(on_account_tab, (tab));
    block!(on_account_open, (index));
    block!(on_account_action, (index));
    block!(on_account_play, (index));
    block!(on_account_next, ());
    block!(on_account_reconcile, ());
    block!(on_account_rating, ());
    block!(on_open_youtube, ());
    block!(on_open_export_guide, ());
    let weak = app.as_weak();
    app.on_more(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<crate::LibraryUi>().invoke_page(true);
        }
    });
    let weak = app.as_weak();
    app.on_guest_previous(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<crate::LibraryUi>().invoke_page(false);
        }
    });
}
#[cfg(unix)]
fn private_directory(directory: &File) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = directory.metadata()?;
    if !metadata.is_dir()
        || metadata.mode() & 0o777 != 0o700
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(invalid());
    }
    Ok(())
}
#[cfg(unix)]
fn open_at(directory: &File, name: &str, extra: i32) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = std::ffi::CString::new(name).map_err(|_| invalid())?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK | extra,
        )
    };
    if fd < 0 {
        return Err(invalid());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
#[cfg(unix)]
fn open_regular(directory: &File, name: &str, maximum: u64) -> io::Result<File> {
    use std::os::unix::fs::MetadataExt;
    let file = open_at(directory, name, 0)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.mode() & 0o777 != 0o600
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.len() > maximum
    {
        return Err(invalid());
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> String {
        format!(
            "{HEADER}{}",
            (0..THUMBNAILS)
                .map(|i| format!("{} 64\n", filename(i)))
                .collect::<String>()
        )
    }
    #[test]
    fn manifest_accepts_only_fixed_bounded_file_set() {
        let text = manifest();
        assert_eq!(parse_manifest(&text).unwrap(), [64; THUMBNAILS]);
        for altered in [
            text.replace("items=10000", "items=10001"),
            text.replace("thumb-00.png", "../secret"),
            text.replace(" 64", " 0"),
            text.replace(" 64", " 2097153"),
            text.replace(" 64", " -1"),
            format!("{text}extra\n"),
            text.trim_end().to_string(),
        ] {
            assert!(parse_manifest(&altered).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn fixture_reads_stay_anchored_and_reject_symlinks_and_special_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = std::env::temp_dir().join(format!(
            "serein-image-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let profile = root.join("Serein");
        std::fs::create_dir(&profile).unwrap();
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700)).unwrap();
        let put = |path: &Path, bytes: &[u8]| {
            std::fs::write(path, bytes).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        };
        put(
            &profile.join("library.sqlite3"),
            b"test fixture admission only",
        );
        put(&root.join(MANIFEST), manifest().as_bytes());
        for index in 0..THUMBNAILS {
            put(&root.join(filename(index)), &[index as u8; 64]);
        }
        let config = Config::admit(&root).unwrap();
        assert_eq!(config.source.read(3).unwrap(), vec![3; 64]);
        assert!(config.source.read(THUMBNAILS).is_err());
        let moved = root.with_extension("moved");
        std::fs::rename(&root, &moved).unwrap();
        symlink(&moved, &root).unwrap();
        assert!(Config::admit(&root).is_err());
        assert_eq!(config.source.read(4).unwrap(), vec![4; 64]);
        let image = moved.join(filename(0));
        std::fs::remove_file(&image).unwrap();
        symlink(filename(1), &image).unwrap();
        assert!(config.source.read(0).is_err());
        std::fs::remove_file(&image).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let fifo = std::ffi::CString::new(image.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(config.source.read(0).is_err());
        std::fs::remove_file(root).unwrap();
        std::fs::remove_dir_all(moved).unwrap();
    }
}

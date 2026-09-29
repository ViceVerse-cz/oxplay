// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit developer fixture generator; never linked into the application.
//! Refuses an existing destination and never writes to a discovered user profile.
use serein_core::{VideoId, VideoSummary};
use serein_storage::LocalStore;
use std::{fs::OpenOptions, path::PathBuf, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("supply an explicit new database path")?);
    if args.next().is_some() || !path.is_absolute() {
        return Err("usage: fixture_library /absolute/new/library.sqlite3".into());
    }
    let parent = path.parent().ok_or("database needs a parent directory")?;
    std::fs::create_dir_all(parent)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    drop(options.open(&path)?);
    let store = LocalStore::open(&path)?;
    let playlist = store.create_playlist("TEST FIXTURE — 10,000 synthetic items")?;
    for index in 0..10_000 {
        store.save_video(
            playlist,
            &VideoSummary {
                id: VideoId::new(&format!("f{index:010}"))?,
                title: format!("TEST FIXTURE — synthetic library item {index:05}"),
                channel: "TEST FIXTURE — no online metadata or media".into(),
                channel_id: None,
                duration: Some(Duration::from_secs(60 + index)),
                thumbnail_url: None,
            },
        )?;
    }
    eprintln!(
        "Created 10,000 explicitly labeled synthetic library rows. History remains off; no network was used."
    );
    Ok(())
}

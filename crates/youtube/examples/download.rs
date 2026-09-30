// SPDX-License-Identifier: GPL-3.0-or-later
//! Manual, opt-in diagnostic: download one public video with the same runner
//! the app uses. It contacts YouTube and is never run by automated tests.
//!
//! cargo run -p oxplay-youtube --example download -- \
//!     --yt-dlp /opt/homebrew/bin/yt-dlp --deno /opt/homebrew/bin/deno \
//!     [--ffmpeg /opt/homebrew/bin/ffmpeg] [--height 360] URL EMPTY_ABSOLUTE_DIR
use oxplay_core::{CancellationToken, OperationContext, VideoId};
use oxplay_youtube::{
    YtDlp,
    download::{Request, Stage},
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (mut yt_dlp, mut deno, mut ffmpeg, mut height) = (None, None, None, 360u16);
    let mut positional = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--yt-dlp" => yt_dlp = args.next().map(PathBuf::from),
            "--deno" => deno = args.next().map(PathBuf::from),
            "--ffmpeg" => ffmpeg = args.next().map(PathBuf::from),
            "--height" => height = args.next().ok_or("--height needs a value")?.parse()?,
            _ => positional.push(arg),
        }
    }
    let [url, staging] = positional.as_slice() else {
        return Err("expected URL and an empty absolute staging directory".into());
    };
    let id = VideoId::from_url(url).or_else(|_| VideoId::new(url))?;
    let staging = PathBuf::from(staging);
    std::fs::create_dir_all(&staging)?;
    let mut provider = YtDlp::new(yt_dlp.ok_or("--yt-dlp is required")?)?;
    if let Some(deno) = deno {
        provider = provider.with_deno(deno)?;
    }
    let operation = OperationContext {
        request_id: 1,
        session_generation: 0,
        cancel: CancellationToken::default(),
    };
    let started = std::time::Instant::now();
    let outcome = provider.download(
        &Request {
            id: &id,
            max_height: height,
            merger: ffmpeg.as_deref(),
            staging: &staging,
        },
        &operation,
        &mut |progress| {
            let percent = progress.fraction.map_or(-1.0, |f| f * 100.0);
            let stage = match progress.stage {
                Stage::Downloading => "downloading",
                Stage::Finishing => "finishing",
            };
            println!(
                "{stage} {percent:5.1}% {} / {:?} bytes, speed {:?} B/s, eta {:?} s",
                progress.downloaded, progress.total, progress.speed, progress.eta
            );
        },
    )?;
    let size = std::fs::metadata(&outcome.media)?.len();
    println!(
        "done in {:.1}s: {} ({size} bytes, {}p, merged: {}), thumbnail: {:?}, title: {:?}, channel: {:?}",
        started.elapsed().as_secs_f64(),
        outcome.media.display(),
        outcome.metadata.height.unwrap_or(0),
        outcome.merged,
        outcome.thumbnail,
        outcome.metadata.title,
        outcome.metadata.channel,
    );
    Ok(())
}

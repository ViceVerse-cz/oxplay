// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in guest probe. Logs counts/results only, never signed URLs or caption text.
use serein_core::{OperationContext, VideoId};
use serein_youtube::YtDlp;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id = VideoId::new(
        &std::env::args()
            .nth(1)
            .ok_or("pass an explicit public video ID")?,
    )?;
    let provider = YtDlp::new("/opt/homebrew/bin/yt-dlp")?.with_deno("/opt/homebrew/bin/deno")?;
    let operation = OperationContext {
        request_id: 1,
        session_generation: 0,
        cancel: Default::default(),
    };
    let item = provider.resolve(&id, &operation)?;
    println!(
        "guest tracks={} manual={} automatic_or_translated={} truncated={}",
        item.subtitles.len(),
        item.subtitles.iter().filter(|s| !s.automatic).count(),
        item.subtitles.iter().filter(|s| s.automatic).count(),
        item.subtitles_truncated
    );
    let track = item
        .subtitles
        .iter()
        .find(|s| s.language == "en")
        .or_else(|| item.subtitles.first())
        .ok_or("no supported guest VTT tracks returned")?;
    let data = provider.caption(track, &id, &operation)?;
    println!("validated VTT bytes={}", data.as_bytes().len());
    Ok(())
}

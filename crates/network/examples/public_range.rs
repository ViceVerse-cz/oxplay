// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit opt-in anonymous network diagnostic; never logs signed addresses.
use serein_core::{OperationContext, VideoId};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argument = std::env::args()
        .nth(1)
        .ok_or("Pass an explicit public YouTube URL")?;
    let video = VideoId::from_url(&argument)?;
    let dns_helper = std::env::args().nth(2).map(std::path::PathBuf::from);
    let config = serein_network::NetworkConfig::new(dns_helper)?;
    let provider = serein_youtube::YtDlp::new("/opt/homebrew/bin/yt-dlp")?
        .with_deno("/opt/homebrew/bin/deno")?;
    let operation = OperationContext {
        request_id: 1,
        session_generation: 0,
        cancel: Default::default(),
    };
    let item = provider.resolve(&video, &operation)?;
    for (kind, track) in std::iter::once(("video", &item.video_track))
        .chain(item.audio_track.as_ref().map(|track| ("audio", track)))
    {
        let source = serein_network::HttpSource::guest(track, &config)?;
        let (mut reader, cancel) = source.open();
        let size = reader.size()?;
        let mut buffer = vec![0u8; 64 * 1024];
        let first = reader.read(&mut buffer)?;
        let tail = size.saturating_sub(4096);
        reader.seek(tail)?;
        let last = reader.read(&mut buffer)?;
        reader.seek(0)?;
        let resumed = reader.read(&mut buffer)?;
        cancel.cancel();
        assert!(reader.read(&mut buffer).is_err());
        println!(
            "{kind}: first={first} tail={last} resumed={resumed}; size_known=true; cancellation=passed"
        );
    }
    Ok(())
}

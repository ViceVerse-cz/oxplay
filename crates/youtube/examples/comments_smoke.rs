// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit guest network probe. Logs only counts, never comment text or media URLs.
use serein_core::{CancellationToken, OperationContext, VideoId};
use serein_youtube::YtDlp;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = YtDlp::new("/opt/homebrew/bin/yt-dlp")?.with_deno("/opt/homebrew/bin/deno")?;
    let op = OperationContext {
        request_id: 1,
        session_generation: 0,
        cancel: CancellationToken::default(),
    };
    let id = VideoId::new("aqz-KE-bpKQ")?;
    let first = provider.comments(&id, None, &op)?;
    println!(
        "guest comments first page={} next={}",
        first.comments.len(),
        first.next.is_some()
    );
    if let Some(cursor) = first.next {
        let next = provider.comments(&id, Some(&cursor), &op)?;
        println!(
            "guest comments second page={} next={}",
            next.comments.len(),
            next.next.is_some()
        );
    }
    Ok(())
}

//! Explicit opt-in network smoke. Outputs counts/status only, never signed URLs or titles.
use oxplay_core::{CancellationToken, OperationContext, VideoId};
use oxplay_youtube::YtDlp;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let binary = args
        .next()
        .ok_or("usage: guest_smoke /absolute/yt-dlp /absolute/deno search|resolve query-or-url")?;
    let deno = args.next().ok_or("missing reviewed deno path")?;
    let action = args.next().ok_or("missing action")?;
    let input = args.next().ok_or("missing input")?;
    let provider = YtDlp::new(binary)?.with_deno(deno)?;
    let operation = OperationContext {
        request_id: 1,
        session_generation: 0,
        cancel: CancellationToken::default(),
    };
    match action.as_str() {
        "search" => {
            let page = provider.search(&input, None, &operation)?;
            println!(
                "real_video_results={} next_page={}",
                page.videos.len(),
                page.next.is_some()
            );
            if let Some(cursor) = page.next {
                let page = provider.search(&input, Some(&cursor), &operation)?;
                println!(
                    "second_page_results={} next_page={}",
                    page.videos.len(),
                    page.next.is_some()
                );
            }
        }
        "resolve" => {
            let playback = provider.resolve(&VideoId::from_url(&input)?, &operation)?;
            println!(
                "resolved=true width={:?} height={:?} separate_audio={} guest={} expiry_present={}",
                playback.video_track.width,
                playback.video_track.height,
                playback.audio_track.is_some(),
                playback.guest,
                playback.expires_at.is_some()
            );
        }
        _ => return Err("action must be search or resolve".into()),
    }
    Ok(())
}

// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit guest network probe of the watch page: native InnerTube `next`
//! (metadata, related, chapters, two comment pages) timed against the yt-dlp
//! resolve/comment paths. Logs only counts, dates and timings, never comment
//! text, author identities, tokens or media URLs.
//!
//! `cargo run --locked -p serein-youtube --example comments_smoke -- [--native-only] [VIDEO_ID...]`
use serein_core::{CancellationToken, OperationContext, VideoId};
use serein_youtube::{YtDlp, comments, innertube::GuestTransport, watch};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut native_only = false;
    let mut ids = Vec::new();
    for arg in std::env::args().skip(1) {
        if arg == "--native-only" {
            native_only = true;
        } else {
            ids.push(VideoId::new(&arg)?);
        }
    }
    if ids.is_empty() {
        ids = vec![VideoId::new("OBJZw3bF0dg")?, VideoId::new("aqz-KE-bpKQ")?];
    }
    let op = OperationContext {
        request_id: 1,
        session_generation: 0,
        cancel: CancellationToken::default(),
    };
    let transport = GuestTransport::new()?;
    let provider = YtDlp::new("/opt/homebrew/bin/yt-dlp")?.with_deno("/opt/homebrew/bin/deno")?;
    for id in &ids {
        println!("== {}", id.as_str());
        let started = Instant::now();
        let page = watch::watch_page(&transport, id, &op)?;
        println!(
            "native next: {} ms · title={} channel={} channel_id={} avatar={} description_chars={} \
             upload_date={} views={:?} likes={:?} comments≈{:?} subscribers≈{:?} \
             chapter_markers={} related={} related_partial={} comment_start={}",
            started.elapsed().as_millis(),
            page.title.is_some(),
            page.channel.is_some(),
            page.channel_id.is_some(),
            page.channel_avatar_url.is_some(),
            page.description.as_ref().map_or(0, |d| d.chars().count()),
            page.upload_date.as_deref().unwrap_or("-"),
            page.view_count,
            page.like_count,
            page.comment_count,
            page.channel_subscriber_count,
            page.has_chapter_markers(),
            page.related.len(),
            page.related_partial,
            page.comments.is_some(),
        );
        let started = Instant::now();
        let first = comments::native_comments(&transport, id, page.comments.as_ref(), &op)?;
        println!(
            "native comments page 1: {} ms · rows={} with_avatar={} next={}",
            started.elapsed().as_millis(),
            first.comments.len(),
            first
                .comments
                .iter()
                .filter(|c| c.author_thumbnail_url.is_some())
                .count(),
            first.next.is_some()
        );
        if let Some(cursor) = &first.next {
            let started = Instant::now();
            let second = comments::native_comments(&transport, id, Some(cursor), &op)?;
            let overlap = second
                .comments
                .iter()
                .filter(|c| first.comments.iter().any(|f| f.id == c.id))
                .count();
            println!(
                "native comments page 2: {} ms · rows={} overlap_with_page1={} next={}",
                started.elapsed().as_millis(),
                second.comments.len(),
                overlap,
                second.next.is_some()
            );
        }
        if native_only {
            continue;
        }
        let started = Instant::now();
        match provider.resolve(id, &op) {
            Ok(resolved) => {
                // Native-only chapters (closed with the resolved duration),
                // compared with the extractor's ranges by whole-second starts.
                let native = page
                    .merge_details(&Default::default(), resolved.video.duration)
                    .chapters;
                let same_starts = native.len() == resolved.details.chapters.len()
                    && native.iter().zip(&resolved.details.chapters).all(|(a, b)| {
                        a.start.as_secs() == b.start.as_secs() && a.end.as_secs() == b.end.as_secs()
                    });
                println!(
                    "yt-dlp resolve: {} ms · description_chars={} upload_date={} views={:?} likes={:?} \
                     comments≈{:?} subscribers≈{:?} chapters={} · native chapters={} same_ranges={}",
                    started.elapsed().as_millis(),
                    resolved
                        .details
                        .description
                        .as_ref()
                        .map_or(0, |d| d.chars().count()),
                    resolved.details.upload_date.as_deref().unwrap_or("-"),
                    resolved.details.view_count,
                    resolved.details.like_count,
                    resolved.details.comment_count,
                    resolved.details.channel_subscriber_count,
                    resolved.details.chapters.len(),
                    native.len(),
                    same_starts,
                );
            }
            Err(error) => println!(
                "yt-dlp resolve: {} ms · failed: {error}",
                started.elapsed().as_millis()
            ),
        }
        // Extractor comment failures (for example prefix-replay order changes)
        // are reported, not fatal: they are the fallback being compared.
        let started = Instant::now();
        let first = match provider.comments(id, None, &op) {
            Ok(first) => first,
            Err(error) => {
                println!(
                    "yt-dlp comments page 1: {} ms · failed: {error}",
                    started.elapsed().as_millis()
                );
                continue;
            }
        };
        println!(
            "yt-dlp comments page 1: {} ms · rows={} next={}",
            started.elapsed().as_millis(),
            first.comments.len(),
            first.next.is_some()
        );
        if let Some(cursor) = first.next {
            let started = Instant::now();
            match provider.comments(id, Some(&cursor), &op) {
                Ok(next) => println!(
                    "yt-dlp comments page 2: {} ms · rows={} next={}",
                    started.elapsed().as_millis(),
                    next.comments.len(),
                    next.next.is_some()
                ),
                Err(error) => println!(
                    "yt-dlp comments page 2: {} ms · failed: {error}",
                    started.elapsed().as_millis()
                ),
            }
        }
    }
    Ok(())
}

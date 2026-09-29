// SPDX-License-Identifier: GPL-3.0-or-later
//! Generated synthetic URL inputs; no requests, credentials or external helpers.
use super::*;

const VIDEO: &str = "abcdefghijk";
const CHANNEL: &str = "UCabcdefghijklmnopqrstuv";

#[test]
fn duplicate_video_ids_are_ambiguous_even_when_equal_or_percent_encoded() {
    for suffix in [
        "&v=ZYXWVUTSRQP",
        "&v=abcdefghijk",
        "&%76=ZYXWVUTSRQP",
        "&v=",
    ] {
        assert!(
            VideoId::from_url(&format!("https://www.youtube.com/watch?v={VIDEO}{suffix}")).is_err()
        );
    }
    let id = VideoId::from_url(&format!(
        "https://www.youtube.com/watch?feature=share&v={VIDEO}&t=42#description"
    ))
    .unwrap();
    assert_eq!(id.as_str(), VIDEO);
    assert_eq!(
        id.watch_url(),
        format!("https://www.youtube.com/watch?v={VIDEO}")
    );
}

#[test]
fn every_address_type_rejects_raw_controls_and_oversized_input() {
    let video = VideoId::new(VIDEO).unwrap();
    let bases = [
        format!("https://www.youtube.com/watch?v={VIDEO}"),
        format!("https://www.youtube.com/channel/{CHANNEL}"),
        "https://www.youtube.com/playlist?list=PLsynthetic".to_owned(),
        "https://r1.googlevideo.com/videoplayback?signature=synthetic".to_owned(),
        format!("https://www.youtube.com/api/timedtext?v={VIDEO}&fmt=vtt"),
    ];
    for base in bases {
        for bad in [
            format!("\n{base}"),
            format!("{base}\t"),
            format!("{base}\0"),
            format!("{base}{}", "x".repeat(MAX_URL_BYTES)),
        ] {
            assert!(VideoId::from_url(&bad).is_err());
            assert!(ChannelId::from_url(&bad).is_err());
            assert!(PlaylistId::from_url(&bad).is_err());
            assert!(MediaUrl::parse(&bad).is_err());
            assert!(CaptionUrl::parse(&bad, &video).is_err());
        }
    }
    let prefix = "https://r1.googlevideo.com/videoplayback?signature=";
    let boundary = format!("{prefix}{}", "a".repeat(MAX_URL_BYTES - prefix.len()));
    assert!(MediaUrl::parse(&boundary).is_ok());
    assert!(MediaUrl::parse(&(boundary + "a")).is_err());
    // Input can fit the byte bound yet expand beyond it during percent encoding.
    let expanded = format!("{prefix}{}", "é".repeat(4000));
    assert!(expanded.len() < MAX_URL_BYTES);
    assert!(MediaUrl::parse(&expanded).is_err());
}

fn assert_accepted_invariants(input: &str) {
    if let Ok(id) = VideoId::from_url(input) {
        let canonical = id.watch_url();
        assert!(canonical.starts_with("https://www.youtube.com/watch?v="));
        assert_eq!(VideoId::from_url(&canonical).unwrap().as_str(), id.as_str());
        assert_eq!(id.as_str().len(), 11);
    }
    if let Ok(id) = ChannelId::from_url(input) {
        assert_eq!(
            ChannelId::from_url(&id.browse_url()).unwrap().as_str(),
            id.as_str()
        );
        assert_eq!(id.as_str().len(), 24);
    }
    if let Ok(id) = PlaylistId::from_url(input) {
        assert_eq!(
            PlaylistId::from_url(&id.browse_url()).unwrap().as_str(),
            id.as_str()
        );
        assert!((2..=128).contains(&id.as_str().len()));
    }
    if let Ok(media) = MediaUrl::parse(input) {
        let parsed = Url::parse(media.expose_url()).unwrap();
        assert_eq!(parsed.scheme(), "https");
        assert!(parsed.host_str().unwrap().ends_with(".googlevideo.com"));
        assert!(parsed.username().is_empty() && parsed.password().is_none());
        assert!(parsed.port().is_none() && parsed.fragment().is_none());
        assert!(media.expose_url().len() <= MAX_URL_BYTES);
        assert_eq!(
            MediaUrl::parse(media.expose_url()).unwrap().expose_url(),
            media.expose_url()
        );
        assert_eq!(
            format!("{media:?} {media}"),
            "MediaUrl([redacted]) [redacted media URL]"
        );
    }
    let video = VideoId::new(VIDEO).unwrap();
    if let Ok(caption) = CaptionUrl::parse(input, &video) {
        let parsed = Url::parse(caption.expose_url()).unwrap();
        assert_eq!(
            parsed.origin().ascii_serialization(),
            "https://www.youtube.com"
        );
        assert_eq!(parsed.path(), "/api/timedtext");
        assert_eq!(caption.video_id().as_str(), VIDEO);
        assert_eq!(
            parsed.query_pairs().filter(|(key, _)| key == "v").count(),
            1
        );
        assert_eq!(
            parsed.query_pairs().filter(|(key, _)| key == "fmt").count(),
            1
        );
        assert_eq!(
            CaptionUrl::parse(caption.expose_url(), &video)
                .unwrap()
                .expose_url(),
            caption.expose_url()
        );
        assert_eq!(
            format!("{caption:?} {caption}"),
            "CaptionUrl([redacted]) [redacted caption URL]"
        );
    }
}

#[test]
fn generated_boundary_insertions_preserve_typed_canonicalization_and_redaction() {
    let seeds = [
        format!("https://youtu.be/{VIDEO}"),
        format!("https://www.youtube.com/watch?v={VIDEO}&feature=share"),
        format!("https://www.youtube.com/channel/{CHANNEL}/videos"),
        "https://www.youtube.com/playlist?list=PLsynthetic".to_owned(),
        "https://r1.googlevideo.com/videoplayback?signature=synthetic".to_owned(),
        format!("https://www.youtube.com/api/timedtext?v={VIDEO}&fmt=vtt"),
    ];
    let fragments = [
        "", ":", "/", "\\", "@", "?", "#", "&", "%00", "%2F", "%76=bad&", "\0", "\n", "\t",
        "\u{7f}", "é", "💻",
    ];
    let mut checked = 0;
    for seed in seeds {
        for offset in seed
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([seed.len()])
        {
            for fragment in fragments {
                let input = format!("{}{fragment}{}", &seed[..offset], &seed[offset..]);
                assert_accepted_invariants(&input);
                checked += 1;
            }
        }
    }
    assert!(checked > 5000);
}

#[test]
fn generated_origin_confusion_cannot_become_a_media_or_catalog_capability() {
    let video = VideoId::new(VIDEO).unwrap();
    for host in [
        "127.0.0.1",
        "[::1]",
        "localhost",
        "youtube.com.evil.test",
        "r1.googlevideo.com.evil.test",
    ] {
        for scheme in ["https", "http", "file", "ftp", "javascript"] {
            for path in [
                format!("/watch?v={VIDEO}"),
                format!("/channel/{CHANNEL}"),
                "/playlist?list=PLsynthetic".to_owned(),
                "/videoplayback".to_owned(),
                format!("/api/timedtext?v={VIDEO}&fmt=vtt"),
            ] {
                let value = format!("{scheme}://{host}{path}");
                assert!(VideoId::from_url(&value).is_err());
                assert!(ChannelId::from_url(&value).is_err());
                assert!(PlaylistId::from_url(&value).is_err());
                assert!(MediaUrl::parse(&value).is_err());
                assert!(CaptionUrl::parse(&value, &video).is_err());
            }
        }
    }
}

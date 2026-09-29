// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit synthetic loopback-only HTTPS qualification. Run via the fixture
//! harness; this module never reads account credentials or remote media URLs.
use super::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn fixture_url(variable: &str) -> String {
    let value = std::env::var(variable).expect("run scripts/test_media_tls.py");
    let port = value
        .strip_prefix("https://127.0.0.1:")
        .and_then(|v| v.strip_suffix("/fixture.wav"))
        .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|p| p.parse::<u16>().ok())
        .filter(|p| *p >= 1024);
    assert!(
        port.is_some(),
        "TLS fixture URL must be fixed-path loopback HTTPS on an unprivileged port"
    );
    value
}
fn play(url: &str, ca: Option<&std::path::Path>, success: bool) {
    let player = Player::new_with_ca_file(|| {}, ca).unwrap();
    // Production constructor/options are retained, except explicitly silent,
    // headless outputs for this synthetic trust test. No renderer is created.
    player.command(&["set", "ao", "null"]).unwrap();
    player.command(&["set", "vo", "null"]).unwrap();
    player.load_file(url, None, None, 0., true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let snapshot = player.drain_events();
        if success && snapshot.state == PlaybackState::Paused && snapshot.duration > 0. {
            assert!(
                snapshot.error.is_none(),
                "trusted fixture returned media error"
            );
            assert_eq!(
                snapshot.audio_output, "null",
                "TLS fixture must not use audible output"
            );
            break;
        }
        if snapshot.state == PlaybackState::Failed {
            assert!(
                !success,
                "trusted certificate fixture failed: {:?}",
                snapshot.error
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "bounded TLS fixture did not reach expected outcome"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "requires explicit synthetic HTTPS servers from scripts/test_media_tls.py"]
fn synthetic_loopback_certificate_and_hostname_verification() {
    let good = fixture_url("SEREIN_TLS_FIXTURE_GOOD");
    let untrusted = fixture_url("SEREIN_TLS_FIXTURE_UNTRUSTED");
    let mismatch = fixture_url("SEREIN_TLS_FIXTURE_MISMATCH");
    let ca =
        PathBuf::from(std::env::var_os("SEREIN_TLS_FIXTURE_CA").expect("fixture CA path required"));
    assert!(ca.is_absolute() && ca.is_file());
    assert_ne!(
        good, untrusted,
        "TLS trust cases need separate HTTP accounting"
    );
    play(&untrusted, None, false);
    play(&good, Some(&ca), true);
    play(&mismatch, Some(&ca), false);
}

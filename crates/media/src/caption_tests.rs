// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wait(player: &Player, condition: impl Fn(&Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = player.drain_events();
        assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
        if condition(&snapshot) {
            return;
        }
        assert!(Instant::now() < deadline, "caption observation timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
// FILE_LOADED identifies the file but can arrive before PLAYBACK_RESTART.
// Wait for the paused fixture to become usable, not just for its metadata/load
// count; add_subtitle correctly refuses startup Buffering.
fn wait_for_paused_caption_load(player: &Player) {
    let request = player.snapshot().load_request_id;
    assert_ne!(request, 0, "fixture load must have been accepted");
    wait(player, |s| {
        s.file_loads == 1
            && s.load_request_id == request
            && s.active_load_request_id == request
            && s.playback_restarted
            && s.state == PlaybackState::Paused
            && s.paused
            && !s.stop_pending
            && player.inner.pause_intent.borrow().settled(s.paused)
    });
}
fn fixture() -> Fixture {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "serein-caption-test-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    let fixture = Fixture(path);
    let media = fixture.0.join("silent.wav");
    let first = fixture.0.join("first.vtt");
    let second = fixture.0.join("second.vtt");
    let mut bytes = b"RIFF".to_vec();
    bytes.extend(48_036u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(48_000u32.to_le_bytes());
    bytes.extend(96_000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(48_000u32.to_le_bytes());
    bytes.resize(48_044, 0);
    std::fs::write(&media, bytes).unwrap();
    std::fs::write(&first, "WEBVTT\n\n00:00.000 --> 00:00.400\nFirst fixture\n").unwrap();
    std::fs::write(
        &second,
        "WEBVTT\n\n00:00.000 --> 00:00.400\nSecond fixture\n",
    )
    .unwrap();
    fixture
}
fn player() -> Player {
    let player = Player::new(|| {}).unwrap();
    player.command(&["set", "ao", "null"]).unwrap();
    player.command(&["set", "vo", "null"]).unwrap();
    player
}
#[test]
fn observed_speed_survives_new_load_replacement_and_stop() {
    let fixture = fixture();
    let media = fixture.0.join("silent.wav");
    let player = player();
    wait(&player, |s| s.speed_observed && s.speed == 1.);
    let before = player.snapshot().speed_updates;
    player.set_speed(1.5).unwrap();
    wait(&player, |s| {
        s.speed_observed && s.speed == 1.5 && s.speed_updates > before
    });
    for loads in 1..=2 {
        player.load_local_at(&media, 0., true).unwrap();
        wait(&player, |s| {
            s.file_loads == loads && s.active_load_request_id == s.load_request_id
        });
        assert!(player.snapshot().speed_observed);
        assert_eq!(player.snapshot().speed, 1.5);
    }
    player.stop().unwrap();
    wait(&player, |s| !s.stop_pending);
    assert_eq!(player.snapshot().speed, 1.5);
    player.set_speed(1.).unwrap();
    wait(&player, |s| s.speed_observed && s.speed == 1.);
    player.load_local_at(&media, 0., true).unwrap();
    wait(&player, |s| s.file_loads == 3);
    assert_eq!(player.snapshot().speed, 1.);
}
#[test]
fn exact_caption_selection_off_race_and_cached_reselection_are_observed() {
    let fixture = fixture();
    let media = fixture.0.join("silent.wav");
    let first = fixture.0.join("first.vtt");
    let second = fixture.0.join("second.vtt");
    let player = player();
    player.load_local_at(&media, 0., true).unwrap();
    wait_for_paused_caption_load(&player);
    player
        .add_subtitle(&first, "First", "en", Arc::new(()))
        .unwrap();
    wait(&player, |s| {
        s.subtitle_id.is_some() && player.subtitle_matches(&first)
    });
    assert!(!player.subtitle_matches(&second));
    player
        .add_subtitle(&second, "Second", "fr", Arc::new(()))
        .unwrap();
    player.disable_subtitles().unwrap();
    wait(&player, |s| {
        s.subtitle_selection_observed
            && s.subtitle_id.is_none()
            && player.inner.pending_commands.get() == 0
            && player.inner.subtitle_leases.borrow().is_empty()
            && !player.subtitle_matches(&second)
    });
    player
        .add_subtitle(&second, "Second", "fr", Arc::new(()))
        .unwrap();
    wait(&player, |s| {
        s.subtitle_id.is_some() && player.subtitle_matches(&second)
    });
    assert!(!player.subtitle_matches(&first));
    player
        .add_subtitle(&first, "First", "en", Arc::new(()))
        .unwrap();
    wait(&player, |s| {
        s.subtitle_id.is_some() && player.subtitle_matches(&first)
    });
    assert_eq!(
        player.snapshot().file_loads,
        1,
        "caption changes must not reload media"
    );
    player.stop().unwrap();
    assert!(!player.subtitle_matches(&first));
}

#[test]
fn attached_caption_lease_survives_off_until_native_end_file() {
    let fixture = fixture();
    let player = player();
    player
        .load_local_at(&fixture.0.join("silent.wav"), 0., true)
        .unwrap();
    wait_for_paused_caption_load(&player);
    let lease = Arc::new(());
    let weak = Arc::downgrade(&lease);
    let path = fixture.0.join("first.vtt");
    player
        .add_subtitle(&path, "Synthetic", "en", lease)
        .unwrap();
    wait(&player, |_| {
        player.subtitle_matches(&path) && player.inner.subtitle_leases.borrow().is_empty()
    });
    assert!(
        weak.upgrade().is_some(),
        "a parsed caption can still own its demuxer"
    );
    player.disable_subtitles().unwrap();
    wait(&player, |s| {
        s.subtitle_selection_observed && s.subtitle_id.is_none()
    });
    assert!(
        weak.upgrade().is_some(),
        "Off must not unlink a caption usable by later seeks/reselection"
    );
    player.stop().unwrap();
    assert!(
        weak.upgrade().is_some(),
        "stop submission is not native unload completion"
    );
    wait(&player, |_| weak.upgrade().is_none());
    assert!(player.inner.playback_subtitle_leases.borrow().is_empty());
}

#[test]
fn pending_caption_blocks_replacement_and_retains_lease_until_stop_reply_and_unload() {
    let fixture = fixture();
    let player = player();
    let media = fixture.0.join("silent.wav");
    player.load_local_at(&media, 0., true).unwrap();
    wait_for_paused_caption_load(&player);
    let lease = Arc::new(());
    let weak = Arc::downgrade(&lease);
    player
        .add_subtitle(&fixture.0.join("first.vtt"), "Synthetic", "en", lease)
        .unwrap();
    assert!(
        player.load_local_at(&media, 0., true).is_err(),
        "a pending sub-add may attach to replacement playback"
    );
    player.stop().unwrap();
    assert!(weak.upgrade().is_some());
    wait(&player, |_| weak.upgrade().is_none());
    assert!(player.inner.subtitle_leases.borrow().is_empty());
    assert!(player.inner.playback_subtitle_leases.borrow().is_empty());
    assert_eq!(player.snapshot().file_loads, 1);
}

#[test]
fn engine_destruction_releases_attached_caption_after_termination() {
    let fixture = fixture();
    let player = player();
    player
        .load_local_at(&fixture.0.join("silent.wav"), 0., true)
        .unwrap();
    wait_for_paused_caption_load(&player);
    let lease = Arc::new(());
    let weak = Arc::downgrade(&lease);
    let path = fixture.0.join("first.vtt");
    player
        .add_subtitle(&path, "Synthetic", "en", lease)
        .unwrap();
    wait(&player, |_| {
        player.subtitle_matches(&path) && player.inner.subtitle_leases.borrow().is_empty()
    });
    assert!(weak.upgrade().is_some());
    drop(player);
    assert!(weak.upgrade().is_none());
}

#[test]
fn fresh_resume_reply_is_tokened_cancellable_and_invalidated_by_transport() {
    let fixture = fixture();
    let player = player();
    player
        .load_local_at(&fixture.0.join("silent.wav"), 0.2, true)
        .unwrap();
    wait(&player, |s| {
        s.file_loads == 1
            && s.state == PlaybackState::Paused
            && player.clock_identity().is_some()
            && player.inner.pause_intent.borrow().settled(s.paused)
    });
    let first = player.request_resume_position().unwrap();
    assert!(player.request_resume_position().is_err());
    player.cancel_resume_position(first);
    let second = player.request_resume_position().unwrap();
    player.cancel_resume_position(first);
    wait(&player, |s| {
        s.resume_position_reply
            .is_some_and(|(token, _)| token == second)
    });
    let (_, position) = player.snapshot().resume_position_reply.unwrap();
    assert!((position.unwrap() - 0.2).abs() < 0.02);
    player.set_volume(42.).unwrap();
    assert_eq!(player.snapshot().resume_position_reply.unwrap().0, second);
    player.set_paused(true).unwrap();
    assert_eq!(
        player.snapshot().resume_position_reply,
        Some((second, None))
    );
    wait(&player, |s| {
        player.inner.pause_intent.borrow().settled(s.paused)
    });
    let third = player.request_resume_position().unwrap();
    player.seek(0.3).unwrap();
    assert!(
        player.request_resume_position().is_err(),
        "a query must not overtake an accepted seek"
    );
    assert_eq!(player.snapshot().resume_position_reply, Some((third, None)));
    wait(&player, |s| {
        // A command reply can precede SEEK/PLAYBACK_RESTART. The previous
        // Paused snapshot is not evidence that this seek has settled.
        player.inner.pending_commands.get() == 0
            && s.state == PlaybackState::Paused
            && player.clock_identity().is_some()
            && player.inner.pause_intent.borrow().settled(s.paused)
    });
    assert_eq!(player.snapshot().resume_position_reply, Some((third, None)));
    let fourth = player.request_resume_position().unwrap();
    player.stop().unwrap();
    wait(&player, |_| player.inner.pending_commands.get() == 0);
    assert_eq!(player.snapshot().resume_position_reply, None);
    player.cancel_resume_position(fourth);
}

#[test]
fn rapid_loads_correlate_native_entries_and_stale_command_failure_is_ignored() {
    let fixture = fixture();
    let player = player();
    player
        .load_local_at(&fixture.0.join("missing.wav"), 0., true)
        .unwrap();
    let old = player.snapshot().load_request_id;
    // Inject a real failing asynchronous native command with an old load token
    // to exercise stale COMMAND_REPLY handling independently of file errors.
    player
        .command_with_reply(&["set", "serein-nonexistent-property", "yes"], old)
        .unwrap();
    player
        .load_local_at(&fixture.0.join("silent.wav"), 0., true)
        .unwrap();
    let latest = player.snapshot().load_request_id;
    assert_ne!(old, latest);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = player.drain_events();
        if snapshot.active_load_request_id == latest
            && snapshot.file_loads > 0
            && player.inner.pending_commands.get() == 0
        {
            assert_eq!(snapshot.failed_load_request_id, None);
            assert_eq!(snapshot.error, None);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "latest native load was not identified: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    player
        .command_with_reply(&["set", "serein-nonexistent-property", "yes"], latest)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while player.drain_events().failed_load_request_id != Some(latest) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    player.stop().unwrap();
    assert_eq!(player.snapshot().load_request_id, 0);
    assert_eq!(player.snapshot().failed_load_request_id, None);
}

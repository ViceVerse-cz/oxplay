// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[test]
fn creating_a_playlist_with_its_first_video_commits_or_rolls_back_together() {
    let mut store = LocalStore::in_memory().unwrap();
    let video = VideoSummary {
        id: VideoId::new("abcdefghijk").unwrap(),
        title: "Synthetic first video".into(),
        channel: "Synthetic channel".into(),
        channel_id: None,
        duration: Some(Duration::from_secs(12)),
        thumbnail_url: None,
    };
    // Exercise an actual SQL failure after the collection has been inserted.
    store.connection.execute_batch("CREATE TRIGGER fail_save BEFORE INSERT ON local_playlist_items BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
    assert_eq!(
        store.create_playlist_with_video("Synthetic collection", &video),
        Err(StorageError::Unavailable)
    );
    assert!(store.playlists(None, 100).unwrap().items.is_empty());
    store
        .connection
        .execute_batch("DROP TRIGGER fail_save;")
        .unwrap();
    let collection = store
        .create_playlist_with_video("Synthetic collection", &video)
        .unwrap();
    assert_eq!(
        store.playlists(None, 100).unwrap().items,
        vec![collection.clone()]
    );
    let saved = store.playlist_videos(collection.id, None, 100).unwrap();
    assert_eq!(saved.items.len(), 1);
    assert_eq!(saved.items[0].id, video.id);
    store.save_video(collection.id, &video).unwrap();
    assert_eq!(
        store
            .playlist_videos(collection.id, None, 100)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn invalid_first_video_and_name_leave_no_empty_collection() {
    let mut store = LocalStore::in_memory().unwrap();
    let mut video = VideoSummary {
        id: VideoId::new("abcdefghijk").unwrap(),
        title: "Synthetic video".into(),
        channel: "Synthetic channel".into(),
        channel_id: None,
        duration: None,
        thumbnail_url: None,
    };
    for name in [
        "".to_owned(),
        " ".to_owned(),
        "a".repeat(1025),
        "bad\0name".to_owned(),
    ] {
        assert_eq!(
            store.create_playlist_with_video(&name, &video),
            Err(StorageError::InvalidInput)
        );
    }
    video.title.clear();
    assert_eq!(
        store.create_playlist_with_video("Synthetic name", &video),
        Err(StorageError::InvalidInput)
    );
    assert!(store.playlists(None, 100).unwrap().items.is_empty());
}

#[test]
fn playback_preferences_migrate_v3_without_changing_existing_privacy_or_library() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v3.sqlite3");
    {
        let connection = Connection::open(&path).unwrap();
        for schema in [
            include_str!("schema_v1.sql"),
            include_str!("schema_v2.sql"),
            include_str!("schema_v3.sql"),
        ] {
            connection.execute_batch(schema).unwrap();
        }
        connection
            .execute(
                "INSERT INTO local_playlists(name) VALUES('Synthetic existing playlist')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE local_preferences SET volume_percent=37,theme='dark',local_history=1",
                [],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
    }
    let store = LocalStore::open(&path).unwrap();
    let prefs = store.preferences().unwrap();
    assert_eq!(prefs.playback, PlaybackPreferences::default());
    assert_eq!(prefs.volume_percent, 37);
    assert_eq!(prefs.theme, Theme::Dark);
    assert!(prefs.privacy.local_history);
    assert!(
        !prefs.privacy.autoplay
            && !prefs.privacy.telemetry
            && !prefs.privacy.thumbnail_previews
            && !prefs.privacy.background_refresh
    );
    assert_eq!(store.playlists(None, 10).unwrap().items.len(), 1);
    assert_eq!(store.schema_version().unwrap(), 4);
}

#[test]
fn every_supported_playback_default_survives_reopen_and_clear_resets_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("playback.sqlite3");
    for quality in QualityCeiling::ALL {
        for speed in PlaybackSpeed::ALL {
            let prefs = LocalPreferences {
                playback: PlaybackPreferences { quality, speed },
                ..Default::default()
            };
            LocalStore::open(&path)
                .unwrap()
                .set_preferences(prefs)
                .unwrap();
            assert_eq!(
                LocalStore::open(&path).unwrap().preferences().unwrap(),
                prefs
            );
        }
    }
    let mut store = LocalStore::open(&path).unwrap();
    store.clear_local_data().unwrap();
    assert_eq!(store.preferences().unwrap(), LocalPreferences::default());
}

#[test]
fn playback_storage_constraints_and_decode_reject_invalid_values() {
    let store = LocalStore::in_memory().unwrap();
    for height in [-1, 0, 145, 2160] {
        assert!(
            store
                .connection
                .execute("UPDATE local_preferences SET quality_height=?1", [height])
                .is_err()
        );
    }
    for speed in [-1, 0, 750, 1250, 4000] {
        assert!(
            store
                .connection
                .execute("UPDATE local_preferences SET speed_millis=?1", [speed])
                .is_err()
        );
    }
    assert_eq!(store.preferences().unwrap(), LocalPreferences::default());
    store
        .connection
        .execute_batch(
            "PRAGMA ignore_check_constraints=ON; UPDATE local_preferences SET speed_millis=1250;",
        )
        .unwrap();
    assert_eq!(store.preferences(), Err(StorageError::CorruptData));
}

#[test]
fn failed_v4_migration_preserves_v3_and_rolls_back_the_first_added_column() {
    let mut connection = Connection::open_in_memory().unwrap();
    for schema in [
        include_str!("schema_v1.sql"),
        include_str!("schema_v2.sql"),
        include_str!("schema_v3.sql"),
    ] {
        connection.execute_batch(schema).unwrap();
    }
    connection
        .execute_batch(
            "ALTER TABLE local_preferences ADD COLUMN speed_millis INTEGER; PRAGMA user_version=3;",
        )
        .unwrap();
    assert!(migrate(&mut connection).is_err());
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        3
    );
    assert!(
        connection
            .prepare("SELECT quality_height FROM local_preferences")
            .is_err()
    );
}

fn video(index: u32) -> VideoSummary {
    VideoSummary {
        id: VideoId::new(&format!("{index:011}")).unwrap(),
        title: format!("Synthetic test video {index}"),
        channel: "Synthetic fixture channel".into(),
        channel_id: Some(ChannelId("UC0000000000000000000000".into())),
        duration: Some(Duration::from_secs(60)),
        // A URL must never be retained by save_video.
        thumbnail_url: Some("https://example.invalid/sensitive-test-marker".into()),
    }
}

#[test]
fn persistence_and_private_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("library.sqlite3");
    let id;
    {
        let store = LocalStore::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(store.preferences().unwrap(), LocalPreferences::default());
        id = store.create_playlist("Local test collection").unwrap();
        store.save_video(id, &video(1)).unwrap();
        store
            .follow_channel(&video(1).channel_id.unwrap(), "Local following")
            .unwrap();
        let prefs = LocalPreferences {
            volume_percent: 42,
            theme: Theme::Dark,
            ..Default::default()
        };
        store.set_preferences(prefs).unwrap();
    }
    let store = LocalStore::open(&path).unwrap();
    assert_eq!(store.playlists(None, 10).unwrap().items[0].id, id);
    assert_eq!(store.preferences().unwrap().volume_percent, 42);
    assert_eq!(store.preferences().unwrap().theme, Theme::Dark);
    assert_eq!(store.preferences().unwrap().privacy, Preferences::default());
    assert_eq!(store.subscriptions(None, 10).unwrap().items.len(), 1);
    let page = store.playlist_videos(id, None, 10).unwrap();
    assert_eq!(page.items[0].id.as_str(), "00000000001");
    assert!(page.items[0].thumbnail_url.is_none());
    let contents = std::fs::read(path).unwrap();
    assert!(
        !contents
            .windows(b"sensitive-test-marker".len())
            .any(|w| w == b"sensitive-test-marker")
    );
}

#[test]
fn migration_preserves_existing_v1_library() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v1.sqlite3");
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("schema_v1.sql"))
            .unwrap();
        connection
            .execute(
                "INSERT INTO local_playlists(name) VALUES (?1)",
                ["Existing playlist"],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
    }
    let store = LocalStore::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(
        store.playlists(None, 10).unwrap().items[0].name,
        "Existing playlist"
    );
    assert_eq!(store.preferences().unwrap(), LocalPreferences::default());
}

#[test]
fn failed_migration_rolls_back_every_change() {
    let mut connection = Connection::open_in_memory().unwrap();
    // Force v2 migration to fail after v1 would have created its tables.
    connection
        .execute_batch("CREATE TABLE local_preferences(existing TEXT);")
        .unwrap();
    assert!(migrate(&mut connection).is_err());
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 0);
    let count: u32 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='local_playlists'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn newer_schema_is_not_downgraded() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    assert_eq!(migrate(&mut connection), Err(StorageError::NewerSchema));
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        SCHEMA_VERSION + 1
    );
}

#[test]
fn pages_are_bounded_and_survive_deletion_before_cursor() {
    let store = LocalStore::in_memory().unwrap();
    let playlist = store
        .create_playlist("10,000-item synthetic fixture")
        .unwrap();
    // A transaction accelerates fixture construction without changing production API.
    store.connection.execute_batch("BEGIN").unwrap();
    for index in 0..10_000 {
        store.save_video(playlist, &video(index)).unwrap();
    }
    store.connection.execute_batch("COMMIT").unwrap();
    assert!(store.playlist_videos(playlist, None, 0).is_err());
    assert!(
        store
            .playlist_videos(playlist, None, MAX_PAGE_SIZE + 1)
            .is_err()
    );
    let first = store.playlist_videos(playlist, None, 100).unwrap();
    assert_eq!(first.items.len(), 100);
    let mut cursor = first.next;
    store.remove_video(playlist, &video(0).id).unwrap();
    let mut count = first.items.len();
    while let Some(next) = cursor {
        let page = store.playlist_videos(playlist, Some(next), 100).unwrap();
        assert!(!page.items.is_empty() && page.items.len() <= 100);
        assert_eq!(page.items[0].id.as_str(), format!("{count:011}"));
        count += page.items.len();
        cursor = page.next;
    }
    assert_eq!(count, 10_000);
}

#[test]
fn updates_preserve_identity_and_removal_is_local() {
    let mut store = LocalStore::in_memory().unwrap();
    let one = store.create_playlist("One").unwrap();
    let two = store.create_playlist("Two").unwrap();
    let mut saved = video(1);
    store.save_video(one, &saved).unwrap();
    store.save_video(two, &saved).unwrap();
    saved.title = "Renamed".into();
    store.save_video(one, &saved).unwrap();
    let page = store.playlist_videos(one, None, 100).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].title, "Renamed");
    store.rename_playlist(one, "New local name").unwrap();
    let channel = saved.channel_id.unwrap();
    store.follow_channel(&channel, "Old").unwrap();
    store.follow_channel(&channel, "New").unwrap();
    assert_eq!(store.subscriptions(None, 100).unwrap().items.len(), 1);
    assert_eq!(store.subscriptions(None, 100).unwrap().items[0].name, "New");
    assert!(store.unfollow_channel(&channel).unwrap());
    assert!(!store.unfollow_channel(&channel).unwrap());
    assert!(store.delete_playlist(one).unwrap());
    assert!(
        store
            .playlist_videos(one, None, 100)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        store.playlist_videos(two, None, 100).unwrap().items.len(),
        1
    );
    store.clear_local_data().unwrap();
    assert!(store.playlists(None, 100).unwrap().items.is_empty());
    assert_eq!(store.preferences().unwrap(), LocalPreferences::default());
}

#[test]
fn backup_is_consistent_and_never_overwrites() {
    let directory = tempfile::tempdir().unwrap();
    let backup = directory.path().join("backup.sqlite3");
    let store = LocalStore::in_memory().unwrap();
    let id = store.create_playlist("Preserved").unwrap();
    store.save_video(id, &video(7)).unwrap();
    store.backup_to(&backup).unwrap();
    assert_eq!(store.backup_to(&backup), Err(StorageError::BackupExists));
    store.delete_playlist(id).unwrap();
    let restored = LocalStore::open(&backup).unwrap();
    assert_eq!(
        restored.playlists(None, 100).unwrap().items[0].name,
        "Preserved"
    );
    assert_eq!(
        restored.playlist_videos(id, None, 100).unwrap().items[0]
            .id
            .as_str(),
        "00000000007"
    );
}

#[test]
fn values_are_validated_without_echoing_sensitive_input() {
    let store = LocalStore::in_memory().unwrap();
    for name in ["", " ", "embedded\0nul"] {
        assert!(store.create_playlist(name).is_err());
    }
    assert!(
        store
            .create_playlist(&"x".repeat(MAX_TEXT_BYTES + 1))
            .is_err()
    );
    assert!(
        store
            .follow_channel(&ChannelId("invalid".into()), "Name")
            .is_err()
    );
    assert!(
        store
            .set_preferences(LocalPreferences {
                volume_percent: 101,
                ..Default::default()
            })
            .is_err()
    );
    let id = store
        .create_playlist("Robert'); DROP TABLE local_playlists;--")
        .unwrap();
    assert!(store.delete_playlist(id).unwrap());
    assert_eq!(store.save_video(id, &video(0)), Err(StorageError::NotFound));
    assert_eq!(
        store.rename_playlist(id, "Renamed"),
        Err(StorageError::NotFound)
    );
}

#[test]
fn local_export_import_is_atomic_and_has_no_urls_or_privacy_preferences() {
    let source = LocalStore::in_memory().unwrap();
    let playlist = source
        .create_playlist("Exported synthetic collection")
        .unwrap();
    source.save_video(playlist, &video(3)).unwrap();
    source
        .follow_channel(&video(3).channel_id.unwrap(), "Synthetic follow")
        .unwrap();
    let bytes = source.export_library_json().unwrap();
    let json = std::str::from_utf8(&bytes).unwrap();
    for omitted in [
        "sensitive-test-marker",
        "thumbnail_url",
        "cookie",
        "preferences",
        "local_history",
    ] {
        assert!(!json.contains(omitted));
    }
    let mut destination = LocalStore::in_memory().unwrap();
    let summary = destination.import_library_json(&bytes).unwrap();
    assert_eq!(
        summary,
        ImportSummary {
            playlists_created: 1,
            videos_saved: 1,
            channels_followed: 1
        }
    );
    let restored = destination.playlists(None, 100).unwrap().items[0].id;
    assert_eq!(
        destination
            .playlist_videos(restored, None, 100)
            .unwrap()
            .items[0]
            .id
            .as_str(),
        video(3).id.as_str()
    );
    assert_eq!(
        destination.preferences().unwrap(),
        LocalPreferences::default()
    );
    let mut invalid: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    invalid["playlists"][0]["videos"][0]["id"] = "invalid".into();
    assert!(
        destination
            .import_library_json(&serde_json::to_vec(&invalid).unwrap())
            .is_err()
    );
    assert_eq!(destination.playlists(None, 100).unwrap().items.len(), 1);
    invalid = serde_json::from_slice(&bytes).unwrap();
    invalid["credentials"] = "synthetic forbidden unknown field".into();
    assert!(
        destination
            .import_library_json(&serde_json::to_vec(&invalid).unwrap())
            .is_err()
    );
    invalid = serde_json::from_slice(&bytes).unwrap();
    invalid["serein_local_library_version"] = 2.into();
    assert!(
        destination
            .import_library_json(&serde_json::to_vec(&invalid).unwrap())
            .is_err()
    );
}

#[test]
fn local_import_rolls_back_on_database_failure() {
    let source = LocalStore::in_memory().unwrap();
    let id = source.create_playlist("Synthetic rollback").unwrap();
    source.save_video(id, &video(1)).unwrap();
    let export = source.export_library_json().unwrap();
    let mut target = LocalStore::in_memory().unwrap();
    target.connection.execute_batch("CREATE TRIGGER reject_synthetic_import BEFORE INSERT ON local_playlist_items BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(target.import_library_json(&export).is_err());
    assert!(target.playlists(None, 100).unwrap().items.is_empty());
}

#[test]
fn local_history_requires_opt_in_and_retention_deletes_rows() {
    use std::time::SystemTime;
    let mut store = LocalStore::in_memory().unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(200 * 86400);
    assert_eq!(store.history_retention_days().unwrap(), 30);
    assert!(
        !store
            .record_history(&video(1), Duration::from_secs(5), now)
            .unwrap()
    );
    assert!(store.history(None, 100, now).unwrap().0.is_empty());
    let mut prefs = store.preferences().unwrap();
    prefs.privacy.local_history = true;
    store.set_preferences(prefs).unwrap();
    assert!(
        store
            .record_history(&video(1), Duration::from_secs(5), now)
            .unwrap()
    );
    assert!(
        store
            .record_history(
                &video(2),
                Duration::from_secs(10),
                now + Duration::from_secs(86400)
            )
            .unwrap()
    );
    let (first, next) = store
        .history(None, 1, now + Duration::from_secs(86400))
        .unwrap();
    assert_eq!(first[0].video.id.as_str(), video(2).id.as_str());
    assert_eq!(first[0].position, Duration::from_secs(10));
    let (second, last) = store
        .history(next.as_ref(), 1, now + Duration::from_secs(86400))
        .unwrap();
    assert_eq!(second[0].video.id.as_str(), video(1).id.as_str());
    assert!(last.is_none());
    assert!(store.set_history_retention_days(0, now).is_err());
    store
        .set_history_retention_days(1, now + Duration::from_secs(2 * 86400))
        .unwrap();
    assert_eq!(
        store
            .history(None, 100, now + Duration::from_secs(2 * 86400))
            .unwrap()
            .0
            .len(),
        1
    );
    prefs.privacy.local_history = false;
    store.set_preferences(prefs).unwrap();
    let rows: u32 = store
        .connection
        .query_row("SELECT count(*) FROM local_history", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    prefs.privacy.local_history = true;
    store.set_preferences(prefs).unwrap();
    assert!(
        store
            .history(None, 100, now + Duration::from_secs(2 * 86400))
            .unwrap()
            .0
            .is_empty()
    );
}

#[test]
fn v2_database_migrates_to_history_without_enabling_it() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(include_str!("schema_v1.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("schema_v2.sql"))
        .unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    migrate(&mut connection).unwrap();
    let store = LocalStore { connection };
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(!store.preferences().unwrap().privacy.local_history);
    assert_eq!(store.history_retention_days().unwrap(), 30);
}

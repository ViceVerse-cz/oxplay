#!/usr/bin/env python3
"""Offline tests of real-site admission and source/player privacy boundaries."""
import copy
import json
import socket
import unittest
from unittest import mock

import youtube_browser_baseline as site


def player(codec="h264", decoder="VideoToolboxVideoDecoder", platform="true"):
    return {"kVideoDecoderName": decoder, "kIsPlatformVideoDecoder": platform,
            "kVideoTracks": json.dumps([{"codec": codec, "color space": {"transfer": "BT709"}, "hdr metadata": {}}]),
            "kAudioTracks": json.dumps([{"codec": "opus", "samples per second": 48000, "channels": 2}])}


def snapshot():
    return {"identity_ok": True, "video_id": "abcdefghijk", "selected_video_id": "abcdefghijk",
            "ad_showing": False, "consent_showing": False, "in_view": True, "current_time": 15., "duration": 300.,
            "creation_time": 1000., "time_origin": 500., "changes": 3, "playback_rate": 1,
            "width": 1920, "height": 1080, "paused": False, "muted": False, "volume": 1,
            "ready_state": 4, "error": None, "visibility": "visible", "focused": True,
            "css_width": 692., "css_height": 389., "dpr": 2, "viewport_scale": 1,
            "total_frames": 900, "dropped_frames": 0, "source_hash": "private-source-identity"}


class AdmissionTests(unittest.TestCase):
    def test_only_one_public_video_and_no_credential_or_tracking_inputs(self):
        for url in ("https://www.youtube.com/watch?v=abcdefghijk", "https://youtu.be/abcdefghijk"):
            self.assertEqual(site.public_video(url), ("abcdefghijk", "https://www.youtube.com/watch?v=abcdefghijk&hl=en"))
        for url in ("http://youtube.com/watch?v=abcdefghijk", "https://user:secret@youtube.com/watch?v=abcdefghijk",
                    "https://youtube.com:443/watch?v=abcdefghijk", "https://youtube.com/watch?v=abcdefghijk&list=private",
                    "https://youtube.com/watch?v=abcdefghijk&v=other", "https://youtube.com/watch?v=abcdefghijk#private",
                    "https://youtube.com.example/watch?v=abcdefghijk", "https://youtu.be/abcdefghijk/extra",
                    "https://youtube.com/watch?v=too-short", "file:///private/profile"):
            with self.subTest(url=url), self.assertRaises(ValueError):
                site.public_video(url)

    def test_requires_actual_hardware_codec_geometry_and_content_state(self):
        good = snapshot()
        decoder = site.decoder_evidence(player())
        site.qualify(good, decoder, "abcdefghijk", 1384, 778, "h264", 61)
        changes = {"identity_ok": False, "ad_showing": True, "consent_showing": True, "video_id": "lmnopqrstuv",
                   "selected_video_id": "lmnopqrstuv", "in_view": False, "paused": True,
                   "muted": True, "volume": 0, "playback_rate": 2, "width": 1280,
                   "height": 720, "focused": False, "visibility": "hidden", "dpr": 1,
                   "css_height": 380., "ready_state": 2, "duration": 30., "error": 4,
                   "current_time": float("nan")}
        for key, value in changes.items():
            with self.subTest(key=key), self.assertRaises(ValueError):
                site.qualify({**good, key: value}, decoder, "abcdefghijk", 1384, 778, "h264", 61)
        for bad in (player(decoder="FFmpegVideoDecoder", platform="false"), player(codec="vp9")):
            with self.assertRaises(ValueError):
                site.qualify(good, site.decoder_evidence(bad), "abcdefghijk", 1384, 778, "h264", 61)

    def test_track_evidence_rejects_hdr_multiple_tracks_and_arbitrary_strings(self):
        for tracks in ([{"codec": "h264", "color space": {"transfer": "SMPTEST2084"}, "hdr metadata": {}}],
                       [{"codec": "https://signed.invalid/?token=private"}], [{"codec": "h264"}] * 2):
            with self.assertRaises(ValueError):
                site.decoder_evidence({**player(), "kVideoTracks": json.dumps(tracks)})
        evidence = site.decoder_evidence({**player(), "provider_secret": "private-cookie"})
        self.assertNotIn("private", json.dumps(evidence))

    def test_explicit_default_scope_accepts_observed_software_codec_but_preserves_quality_gates(self):
        good = snapshot()
        decoder = site.decoder_evidence(player(codec="av1", decoder="Dav1dVideoDecoder", platform="false"))
        site.qualify(good, decoder, "abcdefghijk", 1384, 779, "h264", 61, "default-playback")
        with self.assertRaises(ValueError):
            site.qualify(good, decoder, "abcdefghijk", 1384, 778, "h264", 61)
        for change in ({"height": 720}, {"paused": True}, {"muted": True},
                       {"css_height": 390.}, {"identity_ok": False}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                site.qualify({**good, **change}, decoder, "abcdefghijk", 1384, 778, "h264", 61, "default-playback")

    def test_sanitized_snapshot_never_exports_sources_or_arbitrary_provider_fields(self):
        raw = {**snapshot(), "current_time": "https://signed.invalid/?private", "cookie": "private-cookie",
               "video_id": "https://signed.invalid/?private", "visibility": "private-string",
               "dropped_frames": float("inf"), "identity_ok": "private-string"}
        evidence = json.dumps(site.sanitized_snapshot(raw), allow_nan=False)
        self.assertNotIn("private", evidence)
        self.assertNotIn("source_hash", evidence)
        self.assertNotIn("cookie", evidence)

    def test_partial_decoder_diagnostics_survive_missing_and_malformed_tracks(self):
        for value in (None, "not-json-private", json.dumps([{"codec": "private-token"}]),
                      json.dumps([{}] * 100)):
            evidence = site.decoder_diagnostics({"kVideoTracks": value,
                "kVideoDecoderName": "private-token", "kAudioTracks": "private-cookie"})
            serialized = json.dumps(evidence, allow_nan=False)
            self.assertNotIn("private", serialized)
            self.assertFalse(evidence["decoder_known"])
            self.assertIsNone(evidence["video_decoder"])
            if evidence["video_track_count"] is not None:
                self.assertLessEqual(evidence["video_track_count"], 32)
        evidence = site.decoder_diagnostics(player(codec="vp9"))
        self.assertEqual(evidence["video_codec"], "vp9")
        self.assertEqual(evidence["audio_sample_rate"], 48000)
        self.assertEqual(evidence["audio_channels"], 2)

    def test_startup_probe_never_exports_arbitrary_values(self):
        raw = {"width": 1920, "video_count": 1, "paused": False, "error": "private",
               "current_time": float("nan"), "muted": "private", "provider": "private"}
        evidence = site.sanitized_startup_probe(raw)
        self.assertEqual(evidence["width"], 1920)
        self.assertIsNone(evidence["error"])
        self.assertNotIn("private", json.dumps(evidence, allow_nan=False))

    def test_forward_progress_rejects_stalls_reloads_and_non_sixty_fps(self):
        first = snapshot()
        end = {**first, "current_time": 75., "creation_time": 61000., "total_frames": 4500}
        self.assertEqual(site.browser.qualify_interval(first, end, 60), (3600, 0))
        for changes in ({"current_time": 40.}, {"total_frames": 2700}, {"changes": 4}, {"time_origin": 501.}):
            with self.assertRaises(ValueError):
                site.browser.qualify_interval(first, {**end, **changes}, 60)

    def test_online_preflight_and_measured_interval_reject_a_single_drop(self):
        for elapsed in (2, 60):
            with self.subTest(elapsed=elapsed):
                first = {**snapshot(), "dropped_frames": 16}
                end = {**first, "current_time": first["current_time"] + elapsed,
                       "creation_time": first["creation_time"] + 1000 * elapsed,
                       "total_frames": first["total_frames"] + 60 * elapsed}
                # Startup drops preceding the interval do not qualify as new
                # drops, and the shared local helper retains its wider policy.
                self.assertEqual(site.qualify_online_interval(first, end, elapsed), (60 * elapsed, 0))
                dropped = {**end, "dropped_frames": 17}
                self.assertEqual(site.browser.qualify_interval(first, dropped, elapsed), (60 * elapsed, 1))
                with self.assertRaises(ValueError):
                    site.qualify_online_interval(first, dropped, elapsed)


class AssociationTests(unittest.TestCase):
    def setUp(self):
        self.client, self.server = socket.socketpair()
        self.addCleanup(self.client.close)
        self.addCleanup(self.server.close)
        self.cdp = site.OnlineCDP(self.client, self.client)
        self.cdp.media_session = "selected"

    def event(self, method, params):
        self.cdp.event({"sessionId": "selected", "method": method, "params": params})

    def create(self, identity, url, properties, node=None):
        self.event("Media.playerCreated", {"player": {"playerId": identity, "domNodeId": node}})
        self.event("Media.playerEventsAdded", {"playerId": identity, "events": [
            {"value": json.dumps({"event": "kLoad", "url": url})}]})
        self.event("Media.playerPropertiesChanged", {"playerId": identity, "properties": [
            {"name": key, "value": value} for key, value in properties.items()]})

    def test_idle_drain_preserves_partial_event_until_complete(self):
        frame = json.dumps({"sessionId": "selected", "method": "Media.playerCreated",
                            "params": {"player": {"playerId": "current"}}}).encode() + b"\0"
        self.server.sendall(frame[:30])
        first = self.cdp.drain_idle_events()
        self.assertEqual(first["messages"], 0)
        self.assertTrue(first["partial_frame"])
        self.assertEqual(self.cdp.buffer, frame[:30])
        self.server.sendall(frame[30:])
        second = self.cdp.drain_idle_events()
        self.assertEqual(second["messages"], 1)
        self.assertIn("current", self.cdp.live)
        self.assertEqual(self.cdp.buffer, b"")

    def test_idle_drain_message_cap_retains_events_and_numeric_telemetry(self):
        frame = json.dumps({"sessionId": "selected", "method": "Media.playerCreated",
                            "params": {"player": {"playerId": "current"}}}).encode() + b"\0"
        self.cdp.buffer = bytearray(frame * 300)
        with mock.patch.object(site.time, "monotonic", return_value=10):
            first = self.cdp.drain_idle_events()
            self.assertEqual(first["messages"], 256)
            self.assertTrue(first["budget_exhausted"])
            self.assertEqual(self.cdp.buffer, frame * 44)
            second = self.cdp.drain_idle_events()
        self.assertEqual(second["messages"], 44)
        self.assertFalse(second["pending_input"])
        self.assertTrue(all(type(value) in (int, float, bool) for value in first.values()))

    def test_idle_drain_byte_and_time_budgets_preserve_unparsed_frame(self):
        frame = json.dumps({"method": "unobserved", "params": {"data": "x" * (256 * 1024)}}).encode() + b"\0"
        self.cdp.buffer = bytearray(frame)
        with mock.patch.object(site.time, "monotonic", return_value=10):
            result = self.cdp.drain_idle_events()
        self.assertTrue(result["budget_exhausted"])
        self.assertEqual(result["parsed_bytes"], 0)
        self.assertEqual(self.cdp.buffer, frame)
        self.cdp.buffer = bytearray(b'{"method":"unobserved"}\0')
        with mock.patch.object(site.time, "monotonic", side_effect=(10, 10.006, 10.006)):
            result = self.cdp.drain_idle_events()
        self.assertTrue(result["budget_exhausted"])
        self.assertEqual(self.cdp.buffer, b'{"method":"unobserved"}\0')

    def test_idle_drain_never_consumes_unexpected_command_response(self):
        frame = b'{"id":123,"result":{}}\0'
        self.server.sendall(frame)
        with self.assertRaises(ValueError):
            self.cdp.drain_idle_events()
        self.assertEqual(self.cdp.buffer, frame)

    def test_idle_drain_closed_pipe_fails_without_waiting(self):
        self.server.close()
        with self.assertRaises(EOFError):
            self.cdp.drain_idle_events()

    def test_current_source_does_not_inherit_historical_ad_hardware_decoder(self):
        self.create("old-ad", "https://signed.invalid/?old-private-token", player())
        self.event("Media.playerEventsAdded", {"playerId": "old-ad", "events": [
            {"value": json.dumps({"event": "kWebMediaPlayerDestroyed"})}]})
        source = "blob:https://www.youtube.com/new-private-identity"
        self.create("current", source, player(decoder="FFmpegVideoDecoder", platform="false"))
        identity, method = self.cdp.current_player(42, site.source_digest(source))
        self.assertEqual((identity, method), ("current", "sole_live_player_exact_current_source"))
        self.assertFalse(site.decoder_evidence(self.cdp.players[identity])["platform_video_decoder"])
        self.assertNotIn("private", json.dumps(self.cdp.players))
        self.assertNotIn("private", json.dumps(self.cdp.source_hashes))
        with self.assertRaises(ValueError):
            self.cdp.current_player(42, site.source_digest("wrong-current-source"))

    def test_same_source_multiple_live_players_is_not_unique_and_change_advances_epoch(self):
        source = "blob:https://www.youtube.com/synthetic"
        self.create("one", source, player())
        epoch = self.cdp.epochs["one"]
        self.event("Media.playerPropertiesChanged", {"playerId": "one", "properties": [
            {"name": "kVideoDecoderName", "value": "FFmpegVideoDecoder"}]})
        self.assertGreater(self.cdp.epochs["one"], epoch)
        self.create("two", source, player())
        with self.assertRaises(ValueError):
            self.cdp.current_player(42, site.source_digest(source))

    def test_dom_association_and_session_scope(self):
        source = "blob:https://www.youtube.com/synthetic"
        self.create("current", source, player(), node=42)
        self.assertEqual(self.cdp.current_player(42, "unused"), ("current", "backend_dom_node"))
        self.cdp.event({"sessionId": "unselected", "method": "Media.playerCreated", "params": {
            "player": {"playerId": "foreign", "domNodeId": 42}}})
        self.assertNotIn("foreign", self.cdp.live)

    def test_failed_association_exports_only_bounded_counts_and_partial_track_state(self):
        source = "blob:https://www.youtube.com/private-identity"
        self.create("private-player-id", source, player(codec="vp9"), node=42)
        evidence = self.cdp.association_diagnostics(43, site.source_digest("other-private-source"))
        self.assertEqual(evidence["live_player_count"], 1)
        self.assertEqual(evidence["dom_association_count"], 0)
        self.assertEqual(evidence["exact_source_count"], 0)
        self.assertEqual(evidence["players"][0]["video_codec"], "vp9")
        self.assertNotIn("private", json.dumps(evidence, allow_nan=False))
        self.assertNotIn("source_hash", json.dumps(evidence))


if __name__ == "__main__":
    unittest.main()

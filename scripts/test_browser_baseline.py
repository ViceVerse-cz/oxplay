#!/usr/bin/env python3
"""Offline archive, decoder-admission and bounded CDP tests; never launch a browser."""
import io
import json
from pathlib import Path
import socket
import stat
import tempfile
import time
import unittest
from unittest import mock
import subprocess
import zipfile

import browser_baseline as baseline


def archive(entries):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as output:
        for name, content, mode in entries:
            info = zipfile.ZipInfo(name)
            info.external_attr = mode << 16
            output.writestr(info, content)
    buffer.seek(0)
    return zipfile.ZipFile(buffer)


class ArchiveTests(unittest.TestCase):
    def test_rejects_traversal_absolute_special_and_symlink_parents(self):
        for entries in [
            [("chrome-mac-arm64/../../escape", b"x", stat.S_IFREG)],
            [("/chrome-mac-arm64/escape", b"x", stat.S_IFREG)],
            [("chrome-mac-arm64/pipe", b"", stat.S_IFIFO)],
            [("chrome-mac-arm64/link", b"../../outside", stat.S_IFLNK)],
            [("chrome-mac-arm64/link", b"target", stat.S_IFLNK),
             ("chrome-mac-arm64/link/file", b"x", stat.S_IFREG)],
        ]:
            with self.subTest(entries=entries), archive(entries) as source:
                with self.assertRaises(ValueError):
                    baseline.archive_members(source)

    def test_preserves_safe_framework_link_and_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "browser.zip"
            with archive([("chrome-mac-arm64/Versions/1/executable", b"payload", stat.S_IFREG | 0o755),
                          ("chrome-mac-arm64/Versions/Current", b"1", stat.S_IFLNK)]) as source:
                path.write_bytes(source.fp.getvalue())
            out = root / "out"
            baseline.extract(path, out)
            file = out / "chrome-mac-arm64/Versions/Current/executable"
            self.assertEqual(file.read_bytes(), b"payload")
            self.assertEqual(file.stat().st_mode & 0o777, 0o755)

    def test_validation_precedes_any_extraction(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with archive([("chrome-mac-arm64/valid", b"x", stat.S_IFREG),
                          ("../invalid", b"x", stat.S_IFREG)]) as source:
                path = root / "browser.zip"
                path.write_bytes(source.fp.getvalue())
            with self.assertRaises(ValueError):
                baseline.extract(path, root / "out")
            self.assertFalse((root / "out").exists())


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.client, self.server = socket.socketpair()
        self.addCleanup(self.client.close)
        self.addCleanup(self.server.close)
        self.cdp = baseline.CDP(self.client, self.client)

    def test_partial_and_multiple_frames(self):
        self.server.sendall(b'{"id":1}\0{"id":2}\0')
        self.assertEqual(self.cdp.message(time.monotonic() + 1), {"id": 1})
        self.assertEqual(self.cdp.message(time.monotonic() + 1), {"id": 2})

    def test_timeout_and_peer_close(self):
        with self.assertRaises(TimeoutError):
            self.cdp.message(time.monotonic() + 0.02)
        self.server.close()
        with self.assertRaises(EOFError):
            self.cdp.message(time.monotonic() + 1)

    def test_unrelated_properties_never_enter_evidence(self):
        self.cdp.event({"method": "Media.playerPropertiesChanged", "params": {
            "playerId": "fixture", "properties": [
                {"name": "kFrameUrl", "value": "private-path"},
                {"name": "kVideoDecoderName", "value": "VideoToolboxVideoDecoder"}]}})
        self.assertNotIn("private-path", json.dumps(self.cdp.players))
        self.assertEqual(len(self.cdp.players["fixture"]), 1)

    def test_player_association_is_session_and_dom_node_scoped(self):
        self.cdp.media_session = "selected"
        created = {"method": "Media.playerCreated", "sessionId": "other", "params": {
            "player": {"playerId": "old", "domNodeId": 42}}}
        self.cdp.event(created)
        with self.assertRaises(ValueError):
            self.cdp.player_for_node(42)
        created["sessionId"] = "selected"
        self.cdp.event(created)
        self.cdp.event({"method": "Media.playerPropertiesChanged", "sessionId": "selected", "params": {
            "playerId": "old", "properties": [{"name": "kVideoDecoderName", "value": "VideoToolboxVideoDecoder"}]}})
        self.assertEqual(self.cdp.player_for_node(42), "old")
        with self.assertRaises(ValueError):
            self.cdp.player_for_node(99)
        created["params"]["player"]["playerId"] = "replacement"
        self.cdp.event(created)
        with self.assertRaises(ValueError):
            self.cdp.player_for_node(42)

    def test_missing_optional_node_requires_sole_exact_fixture_load(self):
        self.cdp.expected_media_url = "file:///synthetic/clip.mp4"
        self.cdp.player_nodes = {"fixture": None}
        self.cdp.players = {"fixture": {"kVideoDecoderName": "VideoToolboxVideoDecoder"}}
        def load(url):
            self.cdp.event({"method": "Media.playerEventsAdded", "params": {
                "playerId": "fixture", "events": [{"value": json.dumps({"event": "kLoad", "url": url})}]}})
        load("file:///different/clip.mp4")
        with self.assertRaises(ValueError):
            self.cdp.player_for_node(7)
        load(self.cdp.expected_media_url)
        self.assertEqual(self.cdp.player_for_node(7), "fixture")
        self.assertNotIn("file:", json.dumps(self.cdp.fixture_loads))
        self.cdp.players["other"] = {}
        with self.assertRaises(ValueError):
            self.cdp.player_for_node(7)

    def test_property_removal_revokes_decoder_evidence(self):
        event = {"method": "Media.playerPropertiesChanged", "params": {
            "playerId": "fixture", "properties": [{"name": "kVideoDecoderName", "value": "VideoToolboxVideoDecoder"}]}}
        self.cdp.event(event)
        epoch = self.cdp.decoder_epoch
        event["params"]["properties"][0]["value"] = None
        self.cdp.event(event)
        self.assertNotIn("kVideoDecoderName", self.cdp.players["fixture"])
        self.assertGreater(self.cdp.decoder_epoch, epoch)


class AdmissionTests(unittest.TestCase):
    def setUp(self):
        self.media = {"paused": False, "muted": False, "volume": 1, "error": None,
                      "visibility": "visible", "focused": True, "ready_state": 4,
                      "width": 1920, "height": 1080, "viewport_scale": 1,
                      "css_width": 692, "css_height": 389, "dpr": 2}
        self.media.update(identity_ok=True, in_view=True, playback_rate=1)
        self.players = {"fixture": {"kVideoDecoderName": "VideoToolboxVideoDecoder",
                                    "kIsPlatformVideoDecoder": "true"}}

    def test_actual_decoder_and_matched_geometry_required(self):
        baseline.qualify(self.media, self.players, 1384, 778, "fixture")
        for field, wrong in [("paused", True), ("focused", False), ("muted", True),
                             ("css_width", 693), ("width", 1280)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                baseline.qualify({**self.media, field: wrong}, self.players, 1384, 778, "fixture")
        for name in ("FFmpegVideoDecoder", "MojoVideoDecoder", ""):
            with self.subTest(name=name), self.assertRaises(ValueError):
                baseline.qualify(self.media, {"fixture": {"kVideoDecoderName": name,
                                  "kIsPlatformVideoDecoder": "true"}}, 1384, 778, "fixture")

    def test_unrelated_hardware_decoder_cannot_qualify_selected_software_player(self):
        players = {**self.players, "actual": {"kVideoDecoderName": "FFmpegVideoDecoder"}}
        with self.assertRaises(ValueError):
            baseline.qualify(self.media, players, 1384, 778, "actual")
        for field, wrong in [("identity_ok", False), ("in_view", False), ("playback_rate", 2)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                baseline.qualify({**self.media, field: wrong}, self.players, 1384, 778, "fixture")

    def test_interval_rejects_counter_reset_visibility_change_seek_and_drop_regression(self):
        start = {"creation_time": 1, "time_origin": 100, "changes": 3, "duration": 90,
                 "total_frames": 600, "dropped_frames": 2, "current_time": 10}
        end = {**start, "creation_time": 10001, "total_frames": 1200, "dropped_frames": 3, "current_time": 20}
        self.assertEqual(baseline.qualify_interval(start, end, 10), (600, 1))
        for field, wrong in [("creation_time", 2), ("time_origin", 101), ("changes", 4),
                             ("total_frames", 10), ("dropped_frames", 1),
                             ("dropped_frames", 603), ("current_time", 30)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                baseline.qualify_interval(start, {**end, field: wrong}, 10)


class CleanupTests(unittest.TestCase):
    def test_process_inventory_preserves_pid_crosscheck_without_paths_or_argv(self):
        table = {10: (1, 100, 1., "/private/path/Chrome"),
                 11: (10, 50, 0.5, "/private/path/GPU"),
                 12: (1, 30, 0.2, "/private/path/VTDecoderXPCService"),
                 13: (1, 100, 3., "/unrelated/program")}
        result = baseline.process_inventory(10, {p: table[p] for p in (10, 11, 12)},
                                            table, [{"id": 10}, {"id": 11}, {"id": 15}])
        self.assertEqual(result["cdp_not_in_ps_selection"], [15])
        self.assertEqual(result["ps_selection_not_in_cdp"], [12])
        self.assertEqual(result["processes"][-1]["attribution"], "new_vt_temporal")
        self.assertNotIn("/private/", json.dumps(result))

    def test_stop_failure_preserves_measurements_and_live_profile(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)
            (path / "profile").mkdir()
            (path / "clip.mp4").write_bytes(b"fixture")
            result = {"status": "completed_not_automatically_qualified", "samples": [{"cpu": 4}]}
            with mock.patch.object(baseline, "stop_browser", side_effect=OSError("injected stop failure")):
                baseline.finalize(path, result, object(), object())
            saved = json.loads((path / "result.json").read_text())
            self.assertEqual(saved["samples"], [{"cpu": 4}])
            self.assertEqual(saved["status"], "incomplete")
            self.assertTrue(saved["profile_retained_for_live_or_unknown_group"])
            self.assertTrue((path / "profile").exists())
            self.assertTrue((path / "clip.mp4").exists())

    def test_finalization_only_deletes_after_group_exit_confirmation(self):
        for gone in (False, True):
            with self.subTest(gone=gone), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary)
                (path / "profile").mkdir()
                (path / "clip.mp4").write_bytes(b"fixture")
                outcome = {"process_group_gone": gone, "forced_termination": False,
                           "cleanup_errors": [], "exit_code": 0}
                with mock.patch.object(baseline, "stop_browser", return_value=outcome):
                    baseline.finalize(path, {"status": "completed_not_automatically_qualified"}, object(), object())
                self.assertEqual((path / "profile").exists(), not gone)
                self.assertEqual((path / "clip.mp4").exists(), not gone)

    def test_sigkill_without_confirmed_exit_keeps_cleanup_incomplete_and_closes_pipes(self):
        process, cdp = mock.Mock(pid=123, returncode=0), mock.Mock()
        with mock.patch.object(baseline, "group_exists", return_value=True), \
                mock.patch.object(baseline, "wait_group_gone", return_value=False), \
                mock.patch.object(baseline.os, "killpg") as kill:
            outcome = baseline.stop_browser(process, cdp)
        self.assertFalse(outcome["process_group_gone"])
        self.assertTrue(outcome["forced_termination"])
        self.assertEqual(kill.call_args_list, [mock.call(123, baseline.signal.SIGTERM), mock.call(123, baseline.signal.SIGKILL)])
        cdp.reader.close.assert_called_once()
        cdp.writer.close.assert_called_once()

    def test_removal_failure_preserves_result_and_records_incomplete(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)
            (path / "profile").mkdir()
            result = {"status": "completed_not_automatically_qualified"}
            with mock.patch.object(baseline.shutil, "rmtree", side_effect=OSError("injected unlink failure")):
                baseline.finalize(path, result, None, None)
            self.assertEqual(json.loads((path / "result.json").read_text())["status"], "incomplete")
            self.assertTrue(result["cleanup_errors"])


if __name__ == "__main__":
    unittest.main()

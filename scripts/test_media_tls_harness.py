#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline supervision regressions; no OpenSSL, Cargo, server or account access."""
import os
from pathlib import Path
import signal
import tempfile
import unittest
from unittest import mock

import test_media_tls as tls


class CleanupTests(unittest.TestCase):
    def test_signal_precedes_reaping_and_positive_absence_is_required(self):
        events = []
        process = mock.Mock(pid=123)
        process.wait.side_effect = lambda **_: events.append("wait")

        def kill(_pid, value):
            events.append(value)
            if value == 0:
                raise ProcessLookupError()

        with mock.patch.object(tls.os, "killpg", side_effect=kill):
            tls.reap_group(process)
        self.assertEqual(events, [signal.SIGKILL, "wait", 0])

    def test_permission_denial_does_not_mean_group_disappeared(self):
        process = mock.Mock(pid=123)
        with mock.patch.object(tls.os, "killpg", side_effect=PermissionError()), \
                mock.patch.object(tls.time, "monotonic", side_effect=[0, 3]):
            with self.assertRaisesRegex(RuntimeError, "process group remains"):
                tls.reap_group(process)
        process.wait.assert_called_once_with(timeout=5)

    def test_private_fixture_is_retained_only_when_cleanup_unconfirmed(self):
        for confirmed in (False, True):
            with self.subTest(confirmed=confirmed), tempfile.TemporaryDirectory() as parent:
                private = Path(parent) / "synthetic"
                private.mkdir(mode=0o700)
                commands = tls.Commands()
                commands.safe_to_remove = confirmed
                with mock.patch.object(tls.tempfile, "mkdtemp", return_value=str(private)), \
                        mock.patch.object(tls.sys, "stderr"):
                    with self.assertRaisesRegex(ValueError, "synthetic failure"):
                        with tls.fixture_directory(commands) as selected:
                            (selected / "synthetic.key").write_bytes(b"not-a-real-key")
                            raise ValueError("synthetic failure")
                self.assertEqual(private.exists(), not confirmed)


@unittest.skipUnless(hasattr(os, "WNOWAIT"), "Requires Unix unreaped-leader primitive")
class CommandTests(unittest.TestCase):
    def test_success_keeps_leader_unreaped_until_group_cleanup(self):
        process = mock.Mock(pid=123)
        status = mock.Mock(si_code=os.CLD_EXITED, si_status=0)
        commands = tls.Commands()
        with mock.patch.object(tls.subprocess, "Popen", return_value=process) as spawn, \
                mock.patch.object(tls.os, "waitid", return_value=status) as waitid, \
                mock.patch.object(tls, "reap_group") as reap:
            commands.run(["synthetic-command"])
        self.assertTrue(spawn.call_args.kwargs["start_new_session"])
        self.assertEqual(waitid.call_args.args[2], os.WEXITED | os.WNOWAIT | os.WNOHANG)
        process.poll.assert_not_called()
        process.wait.assert_not_called()
        reap.assert_called_once_with(process)
        self.assertTrue(commands.safe_to_remove)

    def test_deadline_still_reaps_the_owned_group(self):
        process = mock.Mock(pid=123)
        commands = tls.Commands()
        with mock.patch.object(tls.subprocess, "Popen", return_value=process), \
                mock.patch.object(tls.os, "waitid", return_value=None), \
                mock.patch.object(tls.time, "monotonic", side_effect=[0, 21]), \
                mock.patch.object(tls, "reap_group") as reap:
            with self.assertRaises(TimeoutError):
                commands.run(["synthetic-command"])
        reap.assert_called_once_with(process)
        self.assertTrue(commands.safe_to_remove)

    def test_interrupt_still_reaps_the_owned_group(self):
        process = mock.Mock(pid=123)
        commands = tls.Commands()
        with mock.patch.object(tls.subprocess, "Popen", return_value=process), \
                mock.patch.object(tls.os, "waitid", side_effect=KeyboardInterrupt()), \
                mock.patch.object(tls, "reap_group") as reap:
            with self.assertRaises(KeyboardInterrupt):
                commands.run(["synthetic-command"])
        reap.assert_called_once_with(process)
        self.assertTrue(commands.safe_to_remove)

    def test_failed_cleanup_blocks_fixture_removal_even_after_successful_exit(self):
        commands = tls.Commands()
        status = mock.Mock(si_code=os.CLD_EXITED, si_status=0)
        with mock.patch.object(tls.subprocess, "Popen", return_value=mock.Mock(pid=123)), \
                mock.patch.object(tls.os, "waitid", return_value=status), \
                mock.patch.object(tls, "reap_group", side_effect=RuntimeError("cleanup failed")):
            with self.assertRaisesRegex(RuntimeError, "cleanup failed"):
                commands.run(["synthetic-command"])
        self.assertFalse(commands.safe_to_remove)


if __name__ == "__main__":
    unittest.main()

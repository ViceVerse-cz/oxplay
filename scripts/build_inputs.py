#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Bounded offline working-input provenance; never claims a binary association."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import stat
import subprocess
import time

from git_source_export import reap_group
from source_coverage import CoverageError

MAX_FILES = 20000
MAX_FILE = 16 * 1024 * 1024
MAX_TOTAL = 128 * 1024 * 1024
MAX_GIT_OUTPUT = 8 * 1024 * 1024
TOP_LEVEL = {"Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml"}
CONFIGS = {".cargo/config", ".cargo/config.toml"}
PATHS = ["crates", *sorted(TOP_LEVEL), *sorted(CONFIGS)]


class InputError(Exception):
    """Only static error messages may be displayed."""


def selected(path: str) -> bool:
    return path.startswith("crates/") or path in TOP_LEVEL or path in CONFIGS


def relative(raw: bytes) -> str:
    path = raw.decode("utf-8")
    if (not path or len(raw) > 4096 or "\\" in path or ":" in path
            or any(ord(char) < 32 or ord(char) == 127 for char in path)
            or any(part in ("", ".", "..") for part in path.split("/"))):
        raise InputError("Unsupported Git input path")
    return path


class Git:
    def __init__(self, root: Path):
        self.root = root
        self.deadline = time.monotonic() + 60
        self.env = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "TZ": "UTC",
                    "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
                    "GIT_NO_REPLACE_OBJECTS": "1", "GIT_NO_LAZY_FETCH": "1",
                    "GIT_TERMINAL_PROMPT": "0", "GIT_OPTIONAL_LOCKS": "0"}

    def run(self, args: list[str]) -> bytes:
        if not all(hasattr(os, name) for name in ("waitid", "WNOWAIT", "WEXITED", "WNOHANG")):
            raise InputError("Bounded Git supervision is unsupported on this host")
        deadline = min(self.deadline, time.monotonic() + 10)
        if time.monotonic() >= deadline:
            raise InputError("Git input inventory time budget exceeded")
        process = subprocess.Popen(
            ["/usr/bin/git", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null",
             "-c", "protocol.allow=never", "-c", "core.quotePath=false", *args],
            cwd=self.root, env=self.env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True,
        )
        result = bytearray()
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                while selector.get_map():
                    if time.monotonic() >= deadline:
                        raise InputError("Git input command timed out")
                    for key, _ in selector.select(min(0.1, max(0, deadline - time.monotonic()))):
                        block = os.read(key.fd, 65536)
                        if not block:
                            selector.unregister(key.fileobj)
                        else:
                            if len(result) + len(block) > MAX_GIT_OUTPUT:
                                raise InputError("Git input listing exceeds byte limit")
                            result.extend(block)
                while True:
                    status = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                    if status is not None:
                        break
                    if time.monotonic() >= deadline:
                        raise InputError("Git input command timed out")
                    time.sleep(0.01)
                if status.si_code != os.CLD_EXITED or status.si_status != 0:
                    raise InputError("Git input command failed")
            return bytes(result)
        finally:
            try:
                reap_group(process)
            except CoverageError:
                raise InputError("Git input process cleanup could not be confirmed") from None
            finally:
                process.stdout.close()


def file_list(raw: bytes) -> set[str]:
    if not raw:
        return set()
    parts = raw.split(b"\0")
    if parts[-1] or len(parts) > MAX_FILES + 1:
        raise InputError("Incomplete or oversized Git input listing")
    # A cached file may also occur in the untracked result after index changes;
    # the final inventory is keyed by path, and committed comparison uses HEAD.
    return {relative(part) for part in parts[:-1] if selected(relative(part))}


def tree_list(raw: bytes, oid_length: int) -> dict[str, tuple[str, str]]:
    if not raw:
        return {}
    parts = raw.split(b"\0")
    if parts[-1] or len(parts) > MAX_FILES + 1:
        raise InputError("Incomplete or oversized committed tree listing")
    result = {}
    for row in parts[:-1]:
        header, path = row.split(b"\t", 1)
        mode, kind, oid = header.decode("ascii").split(" ")
        name = relative(path)
        if not selected(name):
            continue
        if name in result or kind != "blob" or mode not in ("100644", "100755") or not re.fullmatch(r"[a-f0-9]{%d}" % oid_length, oid):
            raise InputError("Committed input is duplicate, linked or unsupported")
        result[name] = (mode, oid)
    return result


def inspect_file(root_fd: int, path: str, object_format: str, deadline: float) -> tuple[dict | None, int]:
    if time.monotonic() >= deadline:
        raise InputError("Build input read time budget exceeded")
    parent = os.dup(root_fd)
    try:
        parts = path.split("/")
        for part in parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            os.close(parent)
            parent = child
        descriptor = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        with os.fdopen(descriptor, "rb") as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_FILE:
                raise InputError("Build input is not a bounded regular file")
            sha256 = hashlib.sha256()
            blob = hashlib.new(object_format)
            blob.update(f"blob {before.st_size}\0".encode())
            count = 0
            while True:
                if time.monotonic() >= deadline:
                    raise InputError("Build input read time budget exceeded")
                block = stream.read(65536)
                if time.monotonic() >= deadline:
                    raise InputError("Build input read time budget exceeded")
                if not block:
                    break
                count += len(block)
                if count > MAX_FILE or count > before.st_size:
                    raise InputError("Build input grew during capture")
                sha256.update(block)
                blob.update(block)
            after = os.fstat(stream.fileno())
            if count != before.st_size or (before.st_size, before.st_mtime_ns, before.st_ctime_ns, before.st_mode) != (after.st_size, after.st_mtime_ns, after.st_ctime_ns, after.st_mode):
                raise InputError("Build input changed during capture")
            return {"sha256": sha256.hexdigest(), "bytes": count,
                    "git_blob_oid": blob.hexdigest(), "mode": "100755" if before.st_mode & stat.S_IXUSR else "100644"}, count
    except FileNotFoundError:
        return None, 0
    finally:
        os.close(parent)


def capture(root: Path) -> dict:
    root = root.resolve(strict=True)
    git = Git(root)
    top = git.run(["rev-parse", "--show-toplevel"]).decode("utf-8").rstrip("\n")
    if Path(top) != root:
        raise InputError("Select the repository root rather than a subdirectory")
    object_format = git.run(["rev-parse", "--show-object-format"]).decode("ascii").strip()
    if object_format not in ("sha1", "sha256"):
        raise InputError("Unsupported Git object format")
    length = 40 if object_format == "sha1" else 64
    revision = git.run(["rev-parse", "--verify", "HEAD^{commit}"]).decode("ascii").strip()
    if not re.fullmatch(r"[a-f0-9]{%d}" % length, revision):
        raise InputError("Invalid committed revision")
    tracked = file_list(git.run(["ls-files", "--cached", "-z", "--", *PATHS]))
    listed = file_list(git.run(["ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", *PATHS]))
    committed = tree_list(git.run(["ls-tree", "-r", "-z", revision, "--", *PATHS]), length)
    paths = sorted(listed | committed.keys())
    if len(paths) > MAX_FILES or not {"Cargo.toml", "Cargo.lock"}.issubset(paths):
        raise InputError("Missing required or oversized workspace input set")
    files = []
    total = 0
    root_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for path in paths:
            if time.monotonic() > git.deadline:
                raise InputError("Build input capture time budget exceeded")
            info, count = inspect_file(root_fd, path, object_format, git.deadline)
            total += count
            if total > MAX_TOTAL:
                raise InputError("Aggregate build input byte limit exceeded")
            original = committed.get(path)
            state = ("missing" if info is None else "matches_commit" if original == (info["mode"], info["git_blob_oid"])
                     else "modified" if original else "added_index" if path in tracked else "untracked")
            files.append({"path": path, "state": state, "committed_mode": original[0] if original else None,
                          "committed_blob_oid": original[1] if original else None, **(info or {})})
    finally:
        os.close(root_fd)
    if (git.run(["rev-parse", "--verify", "HEAD^{commit}"]).decode("ascii").strip() != revision
            or file_list(git.run(["ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", *PATHS])) != listed
            or file_list(git.run(["ls-files", "--cached", "-z", "--", *PATHS])) != tracked):
        raise InputError("Git revision or input membership changed during capture")
    return {"schema": 1, "purpose": "working_build_input_provenance_only", "git_revision": revision,
            "git_object_format": object_format, "inputs_match_commit": all(item["state"] == "matches_commit" for item in files),
            "repository_clean": None, "binary_build_association_verified": False,
            "file_count": len(files), "total_file_bytes": total, "files": files,
            "scope": ["crates/** including Rust, native glue, build scripts and assets", *sorted(TOP_LEVEL), *sorted(CONFIGS)],
            "limits": ["Untracked ignored files are excluded by git ls-files --exclude-standard; tracked ignored files remain included",
                       "HEAD tree comparison uses raw file bytes/modes, not editable index content or Git clean filters",
                       "Only selected build inputs are compared; unrelated repository changes are not a repository-clean claim",
                       "Capture is not atomic across files; keep inputs quiescent throughout capture and the separately recorded build",
                       "Filesystem deadline checks are cooperative between reads; one blocked regular-file operation cannot be preempted",
                       "No Cargo/build command or binary inspection ran; this inventory alone proves no binary association",
                       "Environment overrides, external/generated inputs, dependency source, compiler/SDK/native libraries are outside this inventory",
                       "Bounded Unix Git supervision is required; other hosts are not qualified"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path, required=True, help="New JSON file, never overwritten")
    args = parser.parse_args()
    try:
        report = capture(args.root)
        descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w") as stream:
            json.dump(report, stream, indent=2, sort_keys=True)
            stream.write("\n")
    except (InputError, OSError, ValueError, KeyError):
        print("Build input capture failed; repository, input bounds or output are invalid.")
        return 1
    print("Build input inventory written; binary association remains unverified.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

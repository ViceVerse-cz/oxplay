#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Opt-in, offline SHA-1 Git object export; never read an editable checkout.

Only built-in ls-tree/cat-file commands run, in an isolated bare repository with
local object alternates. Tar files are our deterministic format, not git archive.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import selectors
import signal
import subprocess
import tarfile
import time
import unicodedata

try:
    import resource
except ImportError:  # The optional exporter requires the reviewed Unix supervisor.
    resource = None

from source_coverage import CoverageError, child, label, read

MAX_OBJECTS = 30000
MAX_TREE_BYTES = 16 * 1024 * 1024
MAX_BLOB = 128 * 1024 * 1024
MAX_TOTAL = 512 * 1024 * 1024
MAX_DATABASES = 64
MAX_REVISIONS = 16
MAX_DEPTH = 64
TOTAL_SECONDS = 120
OID = re.compile(r"[a-f0-9]{40}")


def private_write(path: Path, raw: bytes) -> None:
    descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(raw)


def safe_path(raw: bytes) -> str:
    value = raw.decode("utf-8")
    parts = value.split("/")
    if (not value or len(raw) > 4096 or len(parts) > MAX_DEPTH
            or any(p in ("", ".", "..") or p.lower() == ".git" for p in parts)
            or "\\" in value or ":" in value or any(ord(c) < 32 or ord(c) == 127 for c in value)):
        raise CoverageError("Git tree contains an unsupported member path")
    return value


def safe_link(path: str, raw: bytes) -> str:
    target = raw.decode("utf-8")
    if (not target or len(raw) > 4096 or target.startswith("/") or "\\" in target or ":" in target
            or any(ord(c) < 32 or ord(c) == 127 for c in target)):
        raise CoverageError("Git symlink has an unsafe target")
    stack = list(PurePosixPath(path).parent.parts)
    for part in target.split("/"):
        if part in ("", "."):
            continue
        if part == "..":
            if not stack:
                raise CoverageError("Git symlink escapes the archive root")
            stack.pop()
        else:
            if part.lower() == ".git":
                raise CoverageError("Git symlink targets repository metadata")
            stack.append(part)
    return target


def folded_path(path: str) -> str:
    return unicodedata.normalize("NFC", unicodedata.normalize("NFC", path).casefold())


def reject_member_aliases(paths) -> None:
    folded = [folded_path(path) for path in paths]
    if len(folded) != len(set(folded)):
        raise CoverageError("Git tree has case or Unicode-normalization path aliases")


def verify_link_chains(links: dict[str, str]) -> None:
    """Lexical containment alone is insufficient when targets traverse symlinks."""
    reject_member_aliases(links)
    for fold in (False, True):
        lookup = {(folded_path(k) if fold else k): v for k, v in links.items()}
        _verify_link_lookup(links, lookup, fold)


def _verify_link_lookup(links: dict[str, str], lookup: dict[str, str], fold: bool) -> None:
    for path, target in links.items():
        remaining = list(PurePosixPath(path).parent.parts) + target.split("/")
        resolved = []
        followed = 0
        while remaining:
            part = remaining.pop(0)
            if part in ("", "."):
                continue
            if part == "..":
                if not resolved:
                    raise CoverageError("Composed Git symlink escapes archive root")
                resolved.pop()
                continue
            resolved.append(part)
            link = "/".join(resolved)
            if fold:
                link = folded_path(link)
            if link in lookup:
                followed += 1
                if followed > 40:
                    raise CoverageError("Git symlink chain is cyclic or too deep")
                resolved.pop()
                remaining = lookup[link].split("/") + remaining


def object_hash(kind: str, raw: bytes) -> str:
    return hashlib.sha1(f"{kind} {len(raw)}\0".encode() + raw).hexdigest()


def parse_listing(raw: bytes) -> dict[str, tuple[str, str, str]]:
    rows = raw.split(b"\0")
    if rows[-1] != b"" or len(rows) > MAX_OBJECTS + 1:
        raise CoverageError("Git listing is incomplete or too large")
    result = {}
    for row in rows[:-1]:
        header, path_raw = row.split(b"\t", 1)
        mode, kind, oid = header.decode("ascii").split(" ")
        path = safe_path(path_raw)
        if (path in result or not OID.fullmatch(oid)
                or (mode, kind) not in {("040000", "tree"), ("100644", "blob"),
                                       ("100755", "blob"), ("120000", "blob"), ("160000", "commit")}):
            raise CoverageError("Git listing has unsupported or duplicate entries")
        result[path] = mode, kind, oid
    return result


def reap_group(process) -> None:
    # Darwin can return EPERM for a process group containing only an unreaped
    # zombie. EPERM is not success: reap the pinned leader and require ESRCH from
    # the subsequent group-existence probe. Never send another signal after reap.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        raise CoverageError("Git process could not be reaped within cleanup deadline") from None
    deadline = time.monotonic() + 2
    while True:
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            return
        except PermissionError:
            pass  # A group still exists; do not promote inaccessible to absent.
        if time.monotonic() >= deadline:
            raise CoverageError("Git process-group shutdown could not be confirmed")
        time.sleep(0.01)


class LocalGit:
    def __init__(self, root: Path, objects: Path, deadline: float):
        self.root, self.deadline = root, deadline
        self.serial = 0
        root.mkdir(mode=0o700)
        for directory in ("objects", "objects/info", "refs", "home"):
            (root / directory).mkdir(mode=0o700)
        private_write(root / "HEAD", b"ref: refs/heads/unborn\n")
        private_write(root / "config", b"[core]\nrepositoryformatversion = 0\nbare = true\n")
        if "\n" in str(objects) or "\r" in str(objects):
            raise CoverageError("Unsupported object database path")
        private_write(root / "objects/info/alternates", os.fsencode(objects) + b"\n")
        self.env = {"PATH": "/usr/bin:/bin", "HOME": str(root / "home"),
                    "XDG_CONFIG_HOME": str(root / "home"), "LC_ALL": "C", "TZ": "UTC",
                    "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null",
                    "GIT_NO_REPLACE_OBJECTS": "1", "GIT_NO_LAZY_FETCH": "1",
                    "GIT_TERMINAL_PROMPT": "0", "GIT_CONFIG_COUNT": "2",
                    "GIT_CONFIG_KEY_0": "protocol.allow", "GIT_CONFIG_VALUE_0": "never",
                    "GIT_CONFIG_KEY_1": "core.hooksPath", "GIT_CONFIG_VALUE_1": "/dev/null"}

    def run(self, args: list[str], *, data: bytes = b"", limit: int = MAX_TREE_BYTES,
            allow_missing: bool = False) -> Path | None:
        """Single trusted Git builtin, no shell/filters/network; bounded disk output.

        Retain the leader unreaped until process-group termination, so its PID
        cannot be reused as another process group's identity during cleanup.
        """
        if not hasattr(os, "WNOWAIT"):
            raise CoverageError("Bounded Git supervision is unsupported on this host")
        self.serial += 1
        stdin = self.root / f"input-{self.serial}"
        output = self.root / f"output-{self.serial}"
        private_write(stdin, data)
        timeout = min(20.0, self.deadline - time.monotonic())
        if timeout <= 0:
            raise CoverageError("Git source export exceeded its time budget")
        with stdin.open("rb") as source, output.open("xb") as destination:
            os.chmod(output, 0o600)
            def limits():
                resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
            process = subprocess.Popen(["/usr/bin/git", "--no-optional-locks", f"--git-dir={self.root}", *args],
                                       stdin=source, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                       env=self.env, cwd=self.root, start_new_session=True,
                                       preexec_fn=limits)
            ended = False
            try:
                end = time.monotonic() + timeout
                captured = 0
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    while selector.get_map():
                        if time.monotonic() >= end:
                            raise CoverageError("Git source export command timed out")
                        for key, _ in selector.select(min(0.1, max(0, end - time.monotonic()))):
                            block = os.read(key.fd, 65536)
                            if not block:
                                selector.unregister(key.fileobj)
                            else:
                                captured += len(block)
                                if captured > limit:
                                    raise CoverageError("Git source output exceeded its byte limit")
                                destination.write(block)
                    while time.monotonic() < end:
                        status = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                        if status is not None and status.si_pid == process.pid:
                            ended = True
                            break
                        time.sleep(0.01)
            finally:
                try:
                    reap_group(process)
                finally:
                    process.stdout.close()
        if not ended:
            raise CoverageError("Git source export command timed out")
        if process.returncode != 0:
            if allow_missing and process.returncode in (1, 128):
                return None
            raise CoverageError("Git source objects are absent, damaged or exceed limits")
        if output.stat().st_size > limit:
            raise CoverageError("Git source output exceeded its byte limit")
        return output


def read_objects(git: LocalGit, expected: dict[str, str], budget: int = MAX_TOTAL) -> tuple[Path, dict[str, tuple[str, int, int]]]:
    ids = sorted(expected)
    if len(ids) > MAX_OBJECTS:
        raise CoverageError("Git object count exceeds limit")
    request = ("\n".join(ids) + "\n").encode()
    check = git.run(["cat-file", "--batch-check"], data=request)
    lines = read(check, MAX_TREE_BYTES).decode("ascii").splitlines()
    sizes = {}
    if len(lines) != len(ids):
        raise CoverageError("Git object size inventory is incomplete")
    for oid, line in zip(ids, lines):
        fields = line.split(" ")
        if len(fields) != 3 or fields[:2] != [oid, expected[oid]] or not fields[2].isdigit():
            raise CoverageError("Git object is missing or has the wrong type")
        size = int(fields[2])
        if size > (MAX_BLOB if expected[oid] == "blob" else MAX_TREE_BYTES):
            raise CoverageError("Git object exceeds size limit")
        sizes[oid] = size
    if sum(sizes.values()) > budget:
        raise CoverageError("Git source object bytes exceed limit")
    output = git.run(["cat-file", "--batch"], data=request,
                     limit=sum(sizes.values()) + len(ids) * 100)
    index = {}
    with output.open("rb") as stream:
        for oid in ids:
            kind, size = expected[oid], sizes[oid]
            if stream.readline(100) != f"{oid} {kind} {size}\n".encode():
                raise CoverageError("Git object stream header mismatch")
            offset = stream.tell()
            digest = hashlib.sha1(f"{kind} {size}\0".encode())
            remaining = size
            while remaining:
                block = stream.read(min(remaining, 1024 * 1024))
                if not block:
                    raise CoverageError("Truncated Git object stream")
                digest.update(block)
                remaining -= len(block)
            if digest.hexdigest() != oid or stream.read(1) != b"\n":
                raise CoverageError("Git object identity verification failed")
            index[oid] = kind, offset, size
        if stream.read(1):
            raise CoverageError("Unexpected trailing Git object stream")
    return output, index


def verify_tree(revision: str, listing: dict, stream, index: dict) -> tuple[str, list[dict]]:
    def body(oid, kind):
        actual, offset, size = index[oid]
        if actual != kind:
            raise CoverageError("Git tree references an object of the wrong type")
        stream.seek(offset)
        raw = stream.read(size)
        if len(raw) != size or object_hash(kind, raw) != oid:
            raise CoverageError("Git source object changed during export")
        return raw
    commit = body(revision, "commit")
    first = commit.split(b"\n", 1)[0]
    if not re.fullmatch(rb"tree [a-f0-9]{40}", first):
        raise CoverageError("Git commit has no canonical root tree")
    root = first[5:].decode("ascii")
    actual = {}
    def walk(oid, prefix, depth):
        if depth > MAX_DEPTH:
            raise CoverageError("Git tree nesting exceeds limit")
        raw, cursor = body(oid, "tree"), 0
        while cursor < len(raw):
            end = raw.index(b"\0", cursor)
            mode_raw, name = raw[cursor:end].split(b" ", 1)
            if b"/" in name or end + 21 > len(raw):
                raise CoverageError("Malformed Git tree entry")
            path = safe_path(prefix + name)
            oid = raw[end + 1:end + 21].hex()
            mode = mode_raw.decode("ascii").zfill(6)
            kind = {"040000": "tree", "160000": "commit", "100644": "blob",
                    "100755": "blob", "120000": "blob"}.get(mode)
            if kind is None or path in actual or len(actual) >= MAX_OBJECTS:
                raise CoverageError("Unsupported or duplicate Git tree entry")
            actual[path] = mode, kind, oid
            if kind == "tree":
                walk(oid, (path + "/").encode(), depth + 1)
            cursor = end + 21
    walk(root, b"", 0)
    if actual != listing:
        raise CoverageError("Git listing does not cover the exact verified commit tree")
    # A tracked child beneath a symlink is impossible in a canonical tree; the
    # independently traversed tree comparison also rules out archive path aliases.
    return root, [{"path": p, "revision": row[2], "status": "gitlink_source_not_exported"}
                  for p, row in sorted(listing.items()) if row[0] == "160000"]


def write_archive(path: Path, listing: dict, stream, index: dict, budget: int = MAX_TOTAL) -> dict:
    reject_member_aliases(listing)
    logical_bytes = 0
    lfs = []
    with path.open("xb") as output:
        os.chmod(path, 0o600)
        with tarfile.open(fileobj=output, mode="w", format=tarfile.PAX_FORMAT) as archive:
            for name, (mode, kind, oid) in sorted(listing.items()):
                member = tarfile.TarInfo(name)
                member.uid = member.gid = member.mtime = 0
                member.uname = member.gname = ""
                if mode in ("040000", "160000"):
                    member.type, member.mode = tarfile.DIRTYPE, 0o755
                    archive.addfile(member)
                    continue
                _, offset, size = index[oid]
                logical_bytes += size
                if logical_bytes > budget:
                    raise CoverageError("Expanded Git tree bytes exceed limit")
                stream.seek(offset)
                raw = stream.read(size)
                if object_hash("blob", raw) != oid:
                    raise CoverageError("Git blob changed during export")
                if mode == "120000":
                    member.type, member.mode = tarfile.SYMTYPE, 0o777
                    member.linkname = safe_link(name, raw)
                else:
                    member.size, member.mode = size, 0o755 if mode == "100755" else 0o644
                    if raw.startswith(b"version https://git-lfs.github.com/spec/v1\n"):
                        lfs.append(name)
                archive.addfile(member, io.BytesIO(raw) if member.isfile() else None)
    verify_archive(path, listing)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return {"archive": path.name, "sha256": digest.hexdigest(), "bytes": path.stat().st_size,
            "logical_tree_bytes": logical_bytes, "git_lfs_pointer_paths_not_materialized": lfs}


def verify_archive(path: Path, listing: dict) -> None:
    reject_member_aliases(listing)
    seen, links = set(), {}
    with tarfile.open(path, "r:") as archive:
        for member in archive:
            name = safe_path(member.name.encode())
            if name not in listing or name in seen:
                raise CoverageError("Source archive has an unexpected or duplicate member")
            seen.add(name)
            mode, _, oid = listing[name]
            if member.uid or member.gid or member.mtime or member.uname or member.gname:
                raise CoverageError("Source archive metadata is not normalized")
            if mode in ("040000", "160000"):
                if not member.isdir() or member.mode != 0o755 or member.size:
                    raise CoverageError("Source archive directory differs from verified tree")
            elif mode == "120000":
                if not member.issym() or member.mode != 0o777 or member.size:
                    raise CoverageError("Source archive symlink type mismatch")
                raw = member.linkname.encode()
                links[name] = safe_link(name, raw)
                if object_hash("blob", raw) != oid:
                    raise CoverageError("Source archive symlink differs from verified tree")
            else:
                if (not member.isfile() or member.mode != (0o755 if mode == "100755" else 0o644)
                        or not 0 <= member.size <= MAX_BLOB):
                    raise CoverageError("Source archive file type or size mismatch")
                digest = hashlib.sha1(f"blob {member.size}\0".encode())
                source = archive.extractfile(member)
                while block := source.read(1024 * 1024):
                    digest.update(block)
                if digest.hexdigest() != oid:
                    raise CoverageError("Source archive file differs from verified tree")
    if seen != set(listing):
        raise CoverageError("Source archive omits verified tree entries")
    verify_link_chains(links)


def export_git_sources(cargo: list[dict], cargo_home: Path, destination: Path) -> dict:
    if resource is None or os.name != "posix" or not hasattr(os, "WNOWAIT"):
        raise CoverageError("Bounded Git source export is unsupported on this host")
    if len(cargo) > 2000:
        raise CoverageError("Selected Cargo package count exceeds export limit")
    selected = {}
    for package in cargo:
        if package.get("kind") == "git":
            revision = package["revision"]
            if not OID.fullmatch(revision):
                raise CoverageError("Locked Git revision is not a SHA-1 object identity")
            selected.setdefault(revision, []).append({"name": label(package["name"]),
                                                     "version": label(package["version"])})
    if len(selected) > MAX_REVISIONS:
        raise CoverageError("Locked Git revision count exceeds export limit")
    destination = destination.absolute()
    for ancestor in (*reversed(destination.parents), destination):
        if ancestor.is_symlink():
            raise CoverageError("Git export destination contains a symlink")
    destination.mkdir(mode=0o700)  # Existing output is never reused or overwritten.
    private_write(destination / "INCOMPLETE", b"No successful source export manifest yet.\n")
    database_root = child(cargo_home, "git", "db")
    databases = []
    if database_root.is_dir():
        for entry in database_root.iterdir():
            if len(databases) >= MAX_DATABASES:
                raise CoverageError("Local Git database count exceeds limit")
            if entry.is_symlink() or not entry.is_dir():
                raise CoverageError("Unsupported local Git database entry")
            objects = child(entry, "objects")
            if not objects.is_dir():
                raise CoverageError("Local Git database has no object directory")
            for name in ("alternates", "http-alternates"):
                alternate = child(objects, "info", name)
                if alternate.exists() and read(alternate, 4096).strip():
                    raise CoverageError("Chained local Git alternates are unsupported")
            databases.append(objects.absolute())
    deadline = time.monotonic() + TOTAL_SECONDS
    budget = MAX_TOTAL
    archive_budget = MAX_TOTAL
    results = []
    for number, revision in enumerate(sorted(selected)):
        found = None
        for candidate, objects in enumerate(sorted(databases)):
            git = LocalGit(destination / f"objects-{number}-{candidate}", objects, deadline)
            if git.run(["cat-file", "-e", revision + "^{commit}"], limit=4096, allow_missing=True) is not None:
                found = git
                break
        if found is None:
            results.append({"revision": revision, "packages": selected[revision], "status": "local_revision_unavailable"})
            continue
        listing = parse_listing(read(found.run(["ls-tree", "-r", "-t", "-z", "--full-tree", revision]), MAX_TREE_BYTES))
        # Read the commit once to obtain the root tree, then include it in the
        # batch whose objects are independently hashed below.
        commit = read(found.run(["cat-file", "commit", revision]), MAX_TREE_BYTES)
        if object_hash("commit", commit) != revision or not re.match(rb"tree [a-f0-9]{40}\n", commit):
            raise CoverageError("Locked Git commit identity mismatch")
        root = commit[5:45].decode("ascii")
        expected = {revision: "commit", root: "tree"}
        for mode, kind, oid in listing.values():
            if mode != "160000":
                if oid in expected and expected[oid] != kind:
                    raise CoverageError("Conflicting Git object types")
                expected[oid] = kind
        objects_path, index = read_objects(found, expected, budget)
        budget -= sum(size for _, _, size in index.values())
        with objects_path.open("rb") as stream:
            root, submodules = verify_tree(revision, listing, stream, index)
            result = write_archive(destination / f"git-{revision}.tar", listing, stream, index, archive_budget)
            archive_budget -= result["logical_tree_bytes"]
        results.append({**result, "revision": revision, "tree": root, "packages": selected[revision],
                        "status": "verified_locked_git_tree", "entries": len(listing), "gitlinks": submodules})
    report = {"schema": 1, "purpose": "immutable_locked_git_tree_archives_only",
              "complete_corresponding_source": False, "publisher_authenticated": False, "exports": results,
              "archive_format": "normalized_uncompressed_pax_v1",
              "verified_object_payload_bytes": MAX_TOTAL - budget,
              "archived_tree_payload_bytes": MAX_TOTAL - archive_budget,
              "bounds": {"revisions": MAX_REVISIONS, "databases": MAX_DATABASES,
                         "objects_per_revision": MAX_OBJECTS, "tree_depth": MAX_DEPTH,
                         "blob_bytes": MAX_BLOB, "aggregate_object_bytes": MAX_TOTAL,
                         "aggregate_tree_payload_bytes": MAX_TOTAL,
                         "command_seconds": 20, "shared_command_budget_seconds": TOTAL_SECONDS},
              "limits": ["Local SHA-1 Git objects are independently hashed; commit signatures/publisher identity are not authenticated",
                         "Archives use our normalized PAX format; committed export-ignore/export-subst attributes are not applied",
                         "Gitlinks are empty directories with explicitly uncovered source; Git LFS pointers are not materialized",
                         "Generated sources, build tools, SDKs, external resources, other dependencies and licensing remain separate obligations",
                         "No network, editable checkout reads, cache mutation, build or archive extraction",
                         "Private diagnostic object streams may be retained; distribute only reviewed manifest and verified tar files",
                         "Inputs must remain quiescent; deadlines bound supervised commands, not hard real-time OS scheduling or filesystem operations"]}
    raw = (json.dumps(report, sort_keys=True, indent=2) + "\n").encode()
    private_write(destination / "git-source-manifest.json", raw)
    (destination / "INCOMPLETE").unlink()
    return {"manifest": "git-source-manifest.json", "sha256": hashlib.sha256(raw).hexdigest(),
            "exports": results, "complete_corresponding_source": False}

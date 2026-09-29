#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline, bounded first-stage Deno source/notice inventory; never runs Cargo.

Only the reviewed Homebrew Deno 2.9.7 source checksum is accepted by the CLI.
Cargo.lock is an unselected candidate inventory, not a target/feature graph.
No archive member is extracted or executed. Optional notice copies retain exact
archive bytes in a NEW private directory with content-addressed filenames.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import tarfile
import time
import tomllib

VERSION = "2.9.7"
SOURCE_URL = "https://github.com/denoland/deno/releases/download/v2.9.7/deno_src.tar.gz"
SOURCE_SHA256 = "21069d2f4dd65b6832e3f5c373c24a43a8d35cb3d68d3841e15d0582bed39ea8"
MAX_COMPRESSED = 1024 * 1024 * 1024
MAX_EXPANDED = 4 * 1024 * 1024 * 1024
MAX_MEMBER = 512 * 1024 * 1024
MAX_METADATA = 8 * 1024 * 1024
MAX_EXTENSION = 64 * 1024
MAX_COLLECTED = 64 * 1024 * 1024
MAX_MEMBERS = 100000
MAX_PACKAGES = 10000
MAX_SECONDS = 120
PAX_KEYS = {"path", "linkpath", "size", "mtime", "atime", "ctime", "uid", "gid",
            "uname", "gname", "charset", "comment"}


class InventoryError(Exception):
    """Static messages intentionally exclude input paths and archive contents."""


def digest(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def checked_path(value: str) -> str:
    # No extraction occurs, but report paths must remain portable and unambiguous.
    if (not value or len(value) > 2048 or "\\" in value or ":" in value
            or any(ord(char) < 32 or ord(char) == 127 for char in value)):
        raise InventoryError("Unsupported archive path")
    while value.startswith("./"):
        value = value[2:]
    value = value.rstrip("/")
    path = PurePosixPath(value)
    if not value or path.is_absolute() or any(part in ("", ".", "..") for part in value.split("/")):
        raise InventoryError("Unsafe archive path")
    return value


def pax_fields(raw: bytes) -> dict[str, str]:
    result = {}
    while raw:
        space = raw.find(b" ")
        if space < 1 or space > 8 or not raw[:space].isdigit():
            raise InventoryError("Invalid PAX record")
        length = int(raw[:space])
        if length <= space + 2 or length > len(raw) or raw[length - 1:length] != b"\n":
            raise InventoryError("Invalid PAX record length")
        pair = raw[space + 1:length - 1].decode("utf-8")
        key, sep, value = pair.partition("=")
        if not sep or key not in PAX_KEYS or key in result:
            raise InventoryError("Unsupported or duplicate PAX field")
        result[key] = value
        raw = raw[length:]
    return result


class Reader:
    def __init__(self, stream, deadline: float):
        self.stream = stream
        self.deadline = deadline
        self.count = 0

    def read(self, count: int) -> bytes:
        if count < 0 or count > MAX_METADATA or self.count + count > MAX_EXPANDED:
            raise InventoryError("Expanded archive byte limit exceeded")
        if time.monotonic() > self.deadline:
            raise InventoryError("Archive inspection time budget exceeded")
        raw = self.stream.read(count)
        self.count += len(raw)
        if len(raw) != count:
            raise InventoryError("Truncated source archive")
        return raw

    def consume(self, count: int, keep: bool = False) -> bytes:
        if keep and count > MAX_METADATA:
            raise InventoryError("Collected member exceeds metadata limit")
        parts = []
        while count:
            block = self.read(min(count, 65536))
            if keep:
                parts.append(block)
            count -= len(block)
        return b"".join(parts)


def is_notice(path: str) -> bool:
    name = PurePosixPath(path).name.upper()
    return any(name == prefix or name.startswith(prefix + suffix)
               for prefix in ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "AUTHORS")
               for suffix in (".", "-", "_"))


def inspect_tar(stream, deadline: float) -> tuple[dict, dict[str, bytes]]:
    """Read ordinary tar headers ourselves to bound PAX/GNU extension payloads.

    Sparse archives and other transformations are deliberately unsupported. Links
    are counted, never followed, and never mistaken for collected notice bytes.
    """
    reader = Reader(stream, deadline)
    seen = set()
    metadata = {}
    notices = []
    original_notices = {}
    local_pax = {}
    global_pax = {}
    long_name = None
    long_link = None
    collected = 0
    links = 0
    members = 0
    for _ in range(MAX_MEMBERS):
        block = reader.read(512)
        if block == bytes(512):
            if reader.read(512) != bytes(512) or local_pax or long_name is not None or long_link is not None:
                raise InventoryError("Malformed archive terminator")
            # Reject concatenated archives/nonzero trailing bytes; consume gzip
            # footer as well so a corrupt compressed stream cannot be accepted.
            while True:
                if time.monotonic() > deadline:
                    raise InventoryError("Archive inspection time budget exceeded")
                tail = stream.read(65536)
                reader.count += len(tail)
                if reader.count > MAX_EXPANDED:
                    raise InventoryError("Expanded archive byte limit exceeded")
                if not tail:
                    break
                if any(tail):
                    raise InventoryError("Unexpected data after archive terminator")
            break
        header = tarfile.TarInfo.frombuf(block, "utf-8", "strict")
        size = header.size
        if size < 0 or size > MAX_MEMBER:
            raise InventoryError("Archive member size exceeds limit")
        if header.type in (tarfile.XHDTYPE, tarfile.XGLTYPE, tarfile.GNUTYPE_LONGNAME, tarfile.GNUTYPE_LONGLINK):
            if size > MAX_EXTENSION:
                raise InventoryError("Archive extension exceeds limit")
            raw = reader.consume(size, True)
            reader.consume((-size) % 512)
            if header.type in (tarfile.XHDTYPE, tarfile.XGLTYPE):
                fields = pax_fields(raw)
                if header.type == tarfile.XGLTYPE:
                    if any(key in fields for key in ("path", "linkpath", "size")):
                        raise InventoryError("Unsupported global PAX transformation")
                    global_pax.update(fields)
                else:
                    if local_pax:
                        raise InventoryError("Repeated local PAX header")
                    local_pax = fields
            elif header.type == tarfile.GNUTYPE_LONGNAME:
                if long_name is not None:
                    raise InventoryError("Repeated GNU path header")
                long_name = raw.rstrip(b"\0").decode("utf-8")
            else:
                if long_link is not None:
                    raise InventoryError("Repeated GNU link header")
                long_link = raw.rstrip(b"\0").decode("utf-8")
            continue
        fields = global_pax | local_pax
        if long_name is not None and "path" in fields or long_link is not None and "linkpath" in fields:
            raise InventoryError("Ambiguous archive path override")
        name = checked_path(fields.get("path", long_name if long_name is not None else header.name))
        if name in seen:
            raise InventoryError("Duplicate archive member")
        seen.add(name)
        if "size" in fields:
            if not re.fullmatch(r"[0-9]{1,12}", fields["size"]):
                raise InventoryError("Invalid PAX member size")
            size = int(fields["size"])
            if size > MAX_MEMBER:
                raise InventoryError("PAX member exceeds limit")
        local_pax, long_name, long_link = {}, None, None
        if header.type not in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE, tarfile.SYMTYPE, tarfile.LNKTYPE):
            raise InventoryError("Unsupported special archive member")
        if not header.isfile() and size:
            raise InventoryError("Non-file archive member has content")
        members += 1
        if header.issym() or header.islnk():
            links += 1
        selected = header.isfile() and (PurePosixPath(name).name in ("Cargo.toml", "Cargo.lock") or is_notice(name))
        if selected:
            collected += size
            if collected > MAX_COLLECTED:
                raise InventoryError("Collected metadata byte limit exceeded")
        raw = reader.consume(size, selected)
        reader.consume((-size) % 512)
        if selected:
            info = {"path": name, "bytes": size, "sha256": digest(raw)}
            if is_notice(name):
                notices.append(info)
                original_notices.setdefault(info["sha256"], raw)
            else:
                metadata[name] = (info, raw)
    else:
        raise InventoryError("Archive header count exceeds limit")
    roots = [name for name in metadata if name == "Cargo.lock" or name.count("/") == 1 and name.endswith("/Cargo.lock")]
    if len(roots) != 1:
        raise InventoryError("Expected exactly one root Cargo.lock")
    root_lock = roots[0]
    prefix = root_lock[:-len("Cargo.lock")]
    required = [prefix + "Cargo.toml", prefix + "cli/Cargo.toml"]
    if any(name not in metadata for name in required):
        raise InventoryError("Required Deno manifests absent")
    lock = tomllib.loads(metadata[root_lock][1].decode("utf-8"))
    workspace = tomllib.loads(metadata[required[0]][1].decode("utf-8"))
    cli = tomllib.loads(metadata[required[1]][1].decode("utf-8"))
    package = cli.get("package")
    workspace_table = workspace.get("workspace")
    if not isinstance(package, dict) or not isinstance(workspace_table, dict):
        raise InventoryError("Invalid Deno workspace/package manifest")
    version = package.get("version")
    if isinstance(version, dict) and version == {"workspace": True}:
        inherited = workspace_table.get("package")
        if not isinstance(inherited, dict):
            raise InventoryError("Invalid inherited package manifest")
        version = inherited.get("version")
    if version != VERSION or package.get("name") != "deno":
        raise InventoryError("Archive CLI identity does not match supported Deno release")
    candidates = lock_candidates(lock)
    return {"members": members, "expanded_bytes": reader.count,
            "unfollowed_link_members": links, "root_lock_sha256": metadata[root_lock][0]["sha256"],
            "manifest_and_lock_files": [metadata[name][0] for name in sorted(metadata)],
            "lock_candidates": candidates, "original_notice_files": sorted(notices, key=lambda item: item["path"])}, original_notices


def lock_candidates(lock: dict) -> list[dict]:
    packages = lock.get("package")
    if not isinstance(packages, list) or len(packages) > MAX_PACKAGES:
        raise InventoryError("Invalid or oversized lock package inventory")
    result = []
    identities = set()
    for package in packages:
        if not isinstance(package, dict):
            raise InventoryError("Invalid lock package record")
        name, version = package.get("name"), package.get("version")
        if not all(isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_.+\-]{1,150}", value) for value in (name, version)):
            raise InventoryError("Invalid lock package identity")
        source = package.get("source")
        if source is not None and not isinstance(source, str):
            raise InventoryError("Invalid lock source")
        source_hash = digest(source.encode()) if source is not None else None
        identity = (name, version, source_hash)
        if identity in identities:
            raise InventoryError("Duplicate lock package identity")
        identities.add(identity)
        checksum = package.get("checksum")
        if checksum is not None and (not isinstance(checksum, str) or not re.fullmatch(r"[a-f0-9]{64}", checksum)):
            raise InventoryError("Invalid lock package checksum")
        result.append({"name": name, "version": version,
                       "source_kind": "workspace_or_path" if source is None else "registry" if source.startswith("registry+") else "git" if source.startswith("git+") else "other",
                       "source_identifier_sha256": source_hash, "checksum": checksum,
                       "selected_for_homebrew_build": None})
    return sorted(result, key=lambda item: (item["name"], item["version"], item["source_identifier_sha256"] or ""))


def inventory(path: Path) -> tuple[dict, dict[str, bytes]]:
    deadline = time.monotonic() + MAX_SECONDS
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_COMPRESSED:
            raise InventoryError("Source input type or compressed size exceeds limit")
        hasher = hashlib.sha256()
        count = 0
        while block := stream.read(65536):
            count += len(block)
            if count > MAX_COMPRESSED or time.monotonic() > deadline:
                raise InventoryError("Source verification budget exceeded")
            hasher.update(block)
        if hasher.hexdigest() != SOURCE_SHA256:
            raise InventoryError("Source archive does not match pinned formula checksum")
        stream.seek(0)
        with gzip.GzipFile(fileobj=stream) as expanded:
            report, notices = inspect_tar(expanded, deadline)
        after = os.fstat(stream.fileno())
        if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise InventoryError("Source archive changed during inspection")
    return {"schema": 1, "purpose": "verified_source_candidates_and_original_notice_inventory",
            "deno_version": VERSION, "source_url": SOURCE_URL, "source_sha256": SOURCE_SHA256,
            "compressed_bytes": count, "archive": report,
            "requested_cargo_configuration": {"manifest": "cli/Cargo.toml", "default_features": False,
                                               "features": ["deno_core/v8", "v8/v8"], "target": "aarch64-apple-darwin"},
            "configuration_provenance": "reviewed installed Homebrew 2.9.7 recipe; not evaluated by this tool",
            "selected_graph_status": "absent_not_derivable_from_Cargo.lock",
            "complete_corresponding_source": False, "license_clearance": False,
            "binary_source_association_verified": False, "publisher_authenticated": False,
            "gaps": ["Lock candidates include unselected target, dev and optional packages; no Cargo feature resolution was run",
                     "Nested manifests/notices are inventoried, not proven to be compiled or applicable",
                     "Only named regular notice files are retained; inline comments and generated notices are not collected",
                     "Link members are not followed; linked notices and source content remain uncollected",
                     "V8, generated snapshots, embedded JavaScript, native dependencies and build-tool closure are unverified",
                     "No comparison with the installed Homebrew bottle, build logs or binary was performed",
                     "Fixed byte/count limits and cooperative time checks are not a hard real-time I/O guarantee",
                     "Inputs must remain quiescent; archive checksum is an integrity association, not publisher authentication"]}, notices


def write_output(output: Path, report: dict, notices: dict[str, bytes]) -> None:
    output.mkdir(mode=0o700, parents=False, exist_ok=False)
    marker = output / "INCOMPLETE"
    marker.write_text("Collection has not completed.\n")
    directory = output / "original-notices"
    directory.mkdir(mode=0o700)
    for checksum, raw in notices.items():
        target = directory / (checksum + ".txt")
        with target.open("xb") as stream:
            stream.write(raw)
        target.chmod(0o600)
    with (output / "inventory.json").open("x") as stream:
        json.dump(report, stream, indent=2, sort_keys=True)
        stream.write("\n")
    (output / "inventory.json").chmod(0o600)
    marker.unlink()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True, help="Existing exact Deno 2.9.7 release source archive")
    parser.add_argument("--output", type=Path, required=True, help="NEW private output directory; never overwritten")
    args = parser.parse_args()
    try:
        report, notices = inventory(args.archive)
        write_output(args.output, report, notices)
    except (InventoryError, OSError, ValueError, TypeError, KeyError, tarfile.TarError, EOFError) as error:
        print(str(error) if isinstance(error, InventoryError) else "Deno source inventory failed; inputs or output are invalid")
        return 1
    print("Source/notice inventory written; selected dependency and distribution closure remain unverified.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Create the exact committed source archive attached to every release.

The release workflow runs this on the commit the release tag points at: the
planned head for nightlies, or the version commit for production releases.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MAX_SOURCE_BYTES = 512 * 1024 * 1024
MAX_METADATA_BYTES = 16 * 1024 * 1024
MAX_FILES = 100_000
REQUIRED = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE"}


class ReleaseError(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ReleaseError(message)


def git(repo: Path, *arguments: str) -> bytes:
    result = subprocess.run(["git", "-C", str(repo), *arguments],
                            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=60, check=False)
    require(result.returncode == 0, "Git could not read the committed source")
    require(len(result.stdout) <= MAX_METADATA_BYTES, "Git metadata exceeds the release limit")
    return result.stdout


def validate_tag(tag: str, version: str) -> str:
    """Return the tag's channel. Stable and local preview tags must match the
    committed version; nightly manifests are versioned at build time only."""
    number = r"(?:0|[1-9][0-9]*)"
    require(re.fullmatch(rf"{number}\.{number}\.{number}", version) is not None,
            "Workspace version must be an unqualified numeric version")
    if tag == f"v{version}":
        return "production"
    if re.fullmatch(rf"v{number}\.{number}\.{number}-nightly\.[0-9]{{8}}\.{number}", tag):
        return "nightly"
    require(re.fullmatch(rf"v{re.escape(version)}-(dev|alpha|beta|rc)\.{number}", tag) is not None,
            "Tag must be v<version>, a nightly tag or v<version>-dev|alpha|beta|rc.N")
    return "preview"


def next_tag(version: str, tags: list[str], channel: str = "dev") -> str:
    """First unused prerelease number for this version, never reusing a tag."""
    validate_tag(f"v{version}-{channel}.1", version)
    pattern = re.compile(rf"v{re.escape(version)}-{channel}\.([1-9][0-9]*)")
    used = [int(match[1]) for tag in tags if (match := pattern.fullmatch(tag))]
    return f"v{version}-{channel}.{max(used, default=0) + 1}"


def tracked_files(repo: Path, revision: str) -> dict[str, tuple[str, str]]:
    entries = {}
    for record in git(repo, "ls-tree", "-r", "-z", revision).split(b"\0"):
        if not record:
            continue
        metadata, raw_name = record.split(b"\t", 1)
        mode, kind, oid = metadata.decode("ascii").split()
        name = raw_name.decode("utf-8")
        require(kind == "blob" and mode in ("100644", "100755", "120000"),
                "Submodules and unsupported Git entries cannot form a complete source preview")
        require(name not in entries and len(entries) < MAX_FILES, "Invalid committed file set")
        entries[name] = (mode, oid)
    require(REQUIRED <= entries.keys(), "The committed snapshot lacks required source/license/toolchain files")
    return entries


def inspect_archive(path: Path, prefix: str, entries: dict[str, tuple[str, str]]) -> None:
    """Reject export-ignore/export-subst omissions or changes to committed blobs."""
    found = set()
    total = 0
    with tarfile.open(path, "r:") as archive:
        for member in archive:
            if member.isdir() and member.name.rstrip("/") == prefix.rstrip("/"):
                continue
            require(member.name.startswith(prefix), "Source archive has an unexpected prefix")
            name = member.name[len(prefix):]
            if member.isdir():
                continue
            require(name in entries and name not in found, "Source archive has an unexpected or duplicate file")
            mode, oid = entries[name]
            if mode == "120000":
                require(member.issym(), "Source archive changed a symbolic link")
                target = PurePosixPath(member.linkname)
                require(not target.is_absolute(), "Absolute source symlink cannot be released")
                depth = len(PurePosixPath(name).parent.parts)
                for part in target.parts:
                    depth += -1 if part == ".." else 0 if part == "." else 1
                    require(depth >= 0, "Source symlink escapes the archive root")
                content = member.linkname.encode("utf-8")
                checksum = hashlib.new("sha1" if len(oid) == 40 else "sha256")
                checksum.update(f"blob {len(content)}\0".encode())
                checksum.update(content)
                total += len(content)
            else:
                require(member.isreg() and not member.sparse, "Source archive contains a nonregular file")
                require(bool(member.mode & 0o111) == (mode == "100755"), "Source archive changed executable mode")
                total += member.size
                require(0 <= total <= MAX_SOURCE_BYTES, "Source archive exceeds the release limit")
                source = archive.extractfile(member)
                require(source is not None, "Source archive file is unreadable")
                checksum = hashlib.new("sha1" if len(oid) == 40 else "sha256")
                checksum.update(f"blob {member.size}\0".encode())
                with source:
                    while block := source.read(1024 * 1024):
                        checksum.update(block)
            require(checksum.hexdigest() == oid, "Source archive differs from a committed Git blob")
            found.add(name)
    require(found == entries.keys(), "Git archive omitted committed files (check export-ignore attributes)")


def digest(path: Path) -> str:
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            checksum.update(block)
    return checksum.hexdigest()


def release(repo: Path, tag: str | None, output: Path, revision: str = "HEAD") -> dict:
    require(re.fullmatch(r"HEAD|[0-9a-f]{40}|[0-9a-f]{64}", revision) is not None, "Invalid source revision")
    revision = git(repo, "rev-parse", "--verify", f"{revision}^{{commit}}").decode("ascii").strip()
    require(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", revision) is not None, "Invalid source revision")
    manifest = tomllib.loads(git(repo, "show", f"{revision}:Cargo.toml").decode("utf-8"))
    version = manifest["workspace"]["package"]["version"]
    require(isinstance(version, str), "Workspace version is missing")
    if tag is None:
        tag = next_tag(version, git(repo, "tag", "--list").decode("utf-8").split())
    channel = validate_tag(tag, version)
    toolchain = tomllib.loads(git(repo, "show", f"{revision}:rust-toolchain.toml").decode("utf-8"))["toolchain"]
    require(isinstance(toolchain, dict) and isinstance(toolchain.get("channel"), str),
            "Committed Rust toolchain is missing")
    entries = tracked_files(repo, revision)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.mkdir(mode=0o700, exist_ok=False)
    archive_name = f"oxplay-{tag}-source.tar.gz"
    prefix = f"oxplay-{tag}/"
    # Staging stays inside our new output directory. No user file or existing
    # release is overwritten; failed output remains visible for inspection.
    with tempfile.TemporaryDirectory(prefix=".source-", dir=output) as temporary:
        tar = Path(temporary) / "source.tar"
        with tar.open("xb") as destination:
            result = subprocess.run(["git", "-C", str(repo), "archive", "--format=tar",
                                     f"--prefix={prefix}", revision],
                                    stdin=subprocess.DEVNULL, stdout=destination,
                                    stderr=subprocess.PIPE, timeout=120, check=False)
        require(result.returncode == 0, "Git could not archive the committed source")
        require(tar.stat().st_size <= MAX_SOURCE_BYTES, "Source tar exceeds the release limit")
        inspect_archive(tar, prefix, entries)
        with tar.open("rb") as source, (output / archive_name).open("xb") as destination:
            with gzip.GzipFile(filename="", mode="wb", fileobj=destination, mtime=0) as compressed:
                shutil.copyfileobj(source, compressed, length=1024 * 1024)
    metadata = {
        "schema": 1, "tag": tag, "channel": channel, "revision": revision, "version": version,
        "build_version": tag[1:], "toolchain": toolchain, "source_only": True,
        "prerelease": channel != "production", "production_qualified": False,
        "platform_qualified": False, "contains_binaries": False,
        "dependency_sources_bundled": False, "complete_corresponding_source": False,
        "archive": archive_name, "archive_sha256": digest(output / archive_name),
        "tracked_files": len(entries),
        "scope": ("Describes the source archive only. Exact tracked application repository at the recorded commit; "
                  "excludes untracked working files and separately fetched dependency/native/helper builds. "
                  "Nightly binaries carry build_version in their manifests, set at build time"),
    }
    (output / "release.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    names = sorted([archive_name, "release.json"])
    (output / "SHA256SUMS.txt").write_text("".join(f"{digest(output / name)}  {name}\n" for name in names),
                                           encoding="ascii")
    return metadata


def github_output(path: Path, metadata: dict) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "a", encoding="utf-8") as destination:
        require(stat.S_ISREG(os.fstat(destination.fileno()).st_mode), "GitHub output must be a regular file")
        for name in ("revision", "tag", "archive"):
            destination.write(f"{name}={metadata[name]}\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", help="Release tag; defaults to the next unused v<version>-dev.N preview tag")
    parser.add_argument("--revision", default="HEAD", help="Full commit ID to archive (default HEAD)")
    parser.add_argument("--output", required=True, type=Path, help="Fresh directory; never overwritten")
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    try:
        metadata = release(ROOT, args.tag, args.output, args.revision)
        if args.github_output is not None:
            github_output(args.github_output, metadata)
    except (ReleaseError, OSError, ValueError, KeyError, TypeError, UnicodeError,
            subprocess.SubprocessError, tarfile.TarError):
        print("Source archive creation failed; check committed manifests, tag, revision and fresh output path.", file=sys.stderr)
        return 1
    print(json.dumps({name: metadata[name] for name in ("revision", "tag", "archive")}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

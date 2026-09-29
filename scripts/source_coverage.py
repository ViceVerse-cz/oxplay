#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Bounded offline source-availability audit of existing developer package evidence.

Default audit does not run Cargo, Git, Homebrew, package code or network requests.
An explicit Git-source export can run bounded offline Git object-reader commands.
Presence is distinguished from verified archive bytes and complete correspondence.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import tarfile
import tomllib
from urllib.parse import unquote, urlsplit

MAX_METADATA = 8 * 1024 * 1024
MAX_FILE = 128 * 1024 * 1024
MAX_ENTRIES = 20000
MPV_PATCHES = {
    "75b2ccfeb1ce4ed5a40ac9860fa74f3d1265e13f": "3906b98b02071a0d5747a400406494ca69cef7afd8d3eee4a99fdbe40dc90c1f",
    "c5d391adba7bd024954d0df1e0405f5749f4d4ca": "769b218df220738cc1cf9f81cf696c16518c5dfe56a5ef028e33b22536e0e924",
}


class CoverageError(Exception):
    """Messages are static: no paths, provider content or exception strings."""


def label(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_.+@\-]{1,150}", value):
        raise CoverageError("Invalid package label")
    return value


def sha(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[a-f0-9]{64}", value):
        raise CoverageError("Invalid SHA-256 metadata")
    return value


def child(root: Path, *parts: str) -> Path:
    current = root
    for part in parts:
        if part in ("", ".", "..") or "/" in part or "\\" in part:
            raise CoverageError("Unsafe evidence path")
        current = current / part
        try:
            if stat.S_ISLNK(current.lstat().st_mode):
                raise CoverageError("Symlink in an audited input")
        except FileNotFoundError:
            pass
    return current


def read(path: Path, limit: int = MAX_METADATA) -> bytes:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
            raise CoverageError("Input type or size exceeds audit limits")
        raw = stream.read(limit + 1)
        after = os.fstat(stream.fileno())
        if len(raw) > limit or (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            raise CoverageError("Input changed or exceeded audit limits")
        return raw


def parse_json(raw: bytes) -> dict:
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise CoverageError("Duplicate JSON key")
            result[key] = value
        return result
    value = json.loads(raw, object_pairs_hook=pairs,
                       parse_constant=lambda _: (_ for _ in ()).throw(CoverageError("Nonfinite JSON")))
    if not isinstance(value, dict):
        raise CoverageError("Expected metadata object")
    return value


class Budget:
    def __init__(self, maximum: int = 256 * 1024 * 1024):
        self.remaining = maximum
        self.hashed = 0

    def digest(self, path: Path) -> str | None:
        size = path.lstat().st_size
        if size > MAX_FILE or size > self.remaining:
            return None
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(descriptor, "rb") as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode) or before.st_size != size:
                raise CoverageError("Hash input changed or is not a regular file")
            hasher = hashlib.sha256()
            count = 0
            while block := stream.read(min(1024 * 1024, size - count + 1)):
                count += len(block)
                if count > size:
                    raise CoverageError("Hash input grew while reading")
                hasher.update(block)
            after = os.fstat(stream.fileno())
            if count != size or (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
                raise CoverageError("Hash input changed while reading")
        self.remaining -= count
        self.hashed += count
        return hasher.hexdigest()


def entries(directory: Path, *, directories: bool) -> list[Path]:
    if not directory.exists():
        return []
    result = []
    with os.scandir(directory) as iterator:
        for index, item in enumerate(iterator):
            if index >= MAX_ENTRIES:
                raise CoverageError("Directory entry limit exceeded")
            if item.is_symlink():
                continue
            if item.is_dir(follow_symlinks=False) if directories else item.is_file(follow_symlinks=False):
                result.append(Path(item.path))
    return sorted(result)


def lock_from_archive(path: Path, expected_archive: str, expected_lock: str, budget: Budget) -> dict:
    if budget.digest(path) != sha(expected_archive):
        raise CoverageError("Application source archive hash mismatch or audit budget exhausted")
    # The packager writes plain PAX tar. Bound raw header sizes before tarfile
    # interprets extension records, which may otherwise request a huge read.
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        archive_size = os.fstat(stream.fileno()).st_size
        for _ in range(MAX_ENTRIES):
            block = stream.read(512)
            if block == bytes(512):
                break
            if len(block) != 512:
                raise CoverageError("Incomplete source archive header")
            header = tarfile.TarInfo.frombuf(block, "utf-8", "surrogateescape")
            if header.size < 0 or header.size > MAX_FILE:
                raise CoverageError("Oversized source archive entry")
            if header.type in (tarfile.XHDTYPE, tarfile.XGLTYPE, tarfile.GNUTYPE_LONGNAME, tarfile.GNUTYPE_LONGLINK) and header.size > MAX_METADATA:
                raise CoverageError("Oversized source archive metadata")
            end = stream.tell() + (header.size + 511) // 512 * 512
            if end > archive_size:
                raise CoverageError("Truncated source archive entry")
            stream.seek(end)
        else:
            raise CoverageError("Source archive header count exceeds limit")
    lock = None
    declared_bytes = 0
    with tarfile.open(path, "r:") as archive:
        for index, member in enumerate(archive):
            name = PurePosixPath(member.name)
            declared_bytes += member.size
            if (index >= MAX_ENTRIES or declared_bytes > MAX_FILE or name.is_absolute()
                    or ".." in name.parts or "\\" in member.name or not (member.isfile() or member.isdir())):
                raise CoverageError("Unsafe or oversized application source archive")
            if member.name == "Cargo.lock":
                if lock is not None or member.size > MAX_METADATA:
                    raise CoverageError("Duplicate or oversized archived lockfile")
                stream = archive.extractfile(member)
                if stream is None:
                    raise CoverageError("Archived lockfile is not readable")
                lock = stream.read(MAX_METADATA + 1)
    if lock is None or hashlib.sha256(lock).hexdigest() != sha(expected_lock):
        raise CoverageError("Archived lockfile hash mismatch")
    return tomllib.loads(lock.decode())


def verify_candidates(candidates: list[Path], expected: str, budget: Budget) -> dict:
    mismatches = 0
    limited = False
    for candidate in candidates:
        actual = budget.digest(candidate)
        if actual is None:
            limited = True
        elif actual == expected:
            return {"status": "verified_archive_bytes", "sha256": expected,
                    "bytes": candidate.stat().st_size, "candidate_label": "matching-local-cache-input"}
        else:
            mismatches += 1
    return {"status": "hash_budget_or_file_limit" if limited else "hash_mismatch" if mismatches else "absent",
            "sha256": expected, "mismatching_candidates": mismatches}


def git_head(checkout: Path) -> str | None:
    git = child(checkout, ".git")
    if not git.is_dir():
        return None  # Worktree pointer files are not followed outside the supplied root.
    try:
        head = read(child(git, "HEAD"), 4096).decode().strip()
        if head.startswith("ref: "):
            ref = head[5:]
            if not re.fullmatch(r"refs/[A-Za-z0-9_./\-]+", ref) or ".." in ref.split("/"):
                return None
            target = child(git, *ref.split("/"))
            if target.is_file():
                head = read(target, 4096).decode().strip()
            else:
                packed = child(git, "packed-refs")
                head = next((line.split(" ", 1)[0] for line in read(packed).decode().splitlines()
                             if line.endswith(" " + ref)), "") if packed.is_file() else ""
        return head if re.fullmatch(r"[a-f0-9]{40}", head) else None
    except FileNotFoundError:
        return None


def cargo_coverage(packages: list[dict], lock: dict, cargo_home: Path, budget: Budget) -> list[dict]:
    locked = {(p["name"], p["version"], p.get("source")): p for p in lock["package"]}
    caches = entries(child(cargo_home, "registry", "cache"), directories=True)
    sources = entries(child(cargo_home, "registry", "src"), directories=True)
    repos = entries(child(cargo_home, "git", "checkouts"), directories=True)
    if max(len(caches), len(sources), len(repos)) > 64:
        raise CoverageError("Cargo cache root count exceeds limit")
    checkouts = [checkout for repo in repos
                 for checkout in entries(repo, directories=True)]
    if len(checkouts) > 256:
        raise CoverageError("Git checkout count exceeds limit")
    heads = {head for checkout in checkouts if (head := git_head(checkout))}
    output = []
    if len(packages) > 2000:
        raise CoverageError("Cargo package count exceeds limit")
    for package in sorted(packages, key=lambda p: (p["name"], p["version"])):
        name, version, source = label(package["name"]), label(package["version"]), package.get("source")
        key = (name, version, source)
        if key not in locked:
            raise CoverageError("Selected Cargo package is absent from archived lockfile")
        item = {"name": name, "version": version}
        if source is None:
            item.update(kind="workspace", status="application_source_archive_present")
        elif source.startswith("registry+"):
            expected = sha(locked[key].get("checksum"))
            candidates = [p for cache in caches if (p := child(cache, f"{name}-{version}.crate")).is_file()]
            item.update(kind="registry", **verify_candidates(candidates, expected, budget))
            item["extracted_tree_present_unverified"] = any(child(src, f"{name}-{version}").is_dir() for src in sources)
        elif source.startswith("git+") and re.search(r"#[a-f0-9]{40}$", source):
            revision = source.rsplit("#", 1)[1]
            item.update(kind="git", revision=revision,
                        status="checkout_head_matches_unverified_worktree" if revision in heads else "absent",
                        limitation="HEAD identity alone does not verify objects, tracked/untracked files, submodules or source completeness")
        else:
            item.update(kind="unknown", status="unsupported_source_kind")
        output.append(item)
    return output


def formula_inputs(raw: bytes) -> list[dict]:
    """Only literal adjacent Ruby URL/SHA declarations; never execute a formula."""
    text = raw.decode()
    result = []
    pattern = r'^\s*url "([^"\n]+)"[^\n]*\n(?:\s*#[^\n]*\n)*\s*sha256 "([a-f0-9]{64})"'
    for match in re.finditer(pattern, text, re.M):
        url, checksum = match.groups()
        if "#{" in url:
            result.append({"line": text[:match.start()].count("\n") + 1, "status": "computed_url_unsupported"})
            continue
        parsed = urlsplit(url)
        if parsed.scheme != "https" or parsed.username or parsed.password or parsed.fragment:
            raise CoverageError("Unsupported formula URL declaration")
        base = unquote(PurePosixPath(parsed.path).name)
        if not re.fullmatch(r"[A-Za-z0-9_.+\-]{1,180}", base):
            raise CoverageError("Unsafe formula download basename")
        result.append({"line": text[:match.start()].count("\n") + 1,
                       "sha256": checksum, "url_sha256": hashlib.sha256(url.encode()).hexdigest(),
                       "basename": base, "kind": "patch" if ".patch" in base else "source_or_resource",
                       "mpv_patch_revision": next((rev for rev in MPV_PATCHES if rev + ".patch" == base), None)})
    return result


def native_coverage(manifest: dict, evidence: Path, cache: Path, budget: Budget) -> list[dict]:
    files = entries(cache, directories=False)
    result = []
    recipes = manifest.get("homebrew_provenance", [])
    if len(recipes) > 512:
        raise CoverageError("Native recipe count exceeds limit")
    for recipe in sorted(recipes, key=lambda r: (r["formula"], r["installed_version"])):
        name, version = label(recipe["formula"]), label(recipe["installed_version"])
        row = {"formula": name, "installed_version": version, "inputs": [],
               "recipe_coverage": "partial_literal_url_sha_pairs_only; computed URLs, git resources, build tools and transitive embedded sources may be missing"}
        for filename, metadata in sorted(recipe["metadata"].items()):
            if not filename.endswith(".rb"):
                continue
            label(filename)
            raw = read(child(evidence, "homebrew", name, version, filename))
            if hashlib.sha256(raw).hexdigest() != sha(metadata.get("sha256")):
                raise CoverageError("Packaged source formula hash mismatch")
            for item in formula_inputs(raw):
                if "sha256" not in item:
                    row["inputs"].append(item)
                    continue
                rev = item["mpv_patch_revision"]
                if name == "mpv" and rev and item["sha256"] != MPV_PATCHES[rev]:
                    raise CoverageError("Recorded mpv patch hash differs from inspected recipe")
                candidates = [f for f in files if ".bottle" not in f.name and
                              (f.name == item["basename"] or f.name.endswith("--" + item["basename"]) or
                               f.name.startswith(item["url_sha256"] + "--"))]
                # Publish only hashes and recipe line numbers, not filenames/URLs that may contain local data.
                item.pop("basename")
                item.update(verify_candidates(candidates, item["sha256"], budget))
                row["inputs"].append(item)
        if name == "mpv":
            found = {item.get("mpv_patch_revision") for item in row["inputs"]}
            row["expected_patches_missing_from_recipe"] = sorted(set(MPV_PATCHES) - found)
        result.append(row)
    return result


def audit(evidence: Path, cargo_home: Path, brew_cache: Path, max_hash_bytes: int) -> dict:
    manifest_bytes = read(child(evidence, "build-manifest.json"))
    manifest = parse_json(manifest_bytes)
    graph_bytes = read(child(evidence, "cargo-graph.json"))
    graph_hash = hashlib.sha256(graph_bytes).hexdigest()
    recorded = [item["sha256"] for item in manifest.get("evidence_input_files", []) if item.get("path") == "cargo-graph.json"]
    if recorded != [graph_hash]:
        raise CoverageError("Packaged Cargo graph hash mismatch")
    budget = Budget(max_hash_bytes)
    lock = lock_from_archive(child(evidence, "application-source.tar"), manifest["application_source_archive_sha256"],
                             manifest["cargo_lock_sha256"], budget)
    cargo = cargo_coverage(parse_json(graph_bytes)["packages"], lock, cargo_home, budget)
    native = native_coverage(manifest, evidence, brew_cache, budget)
    return {"schema": 1, "purpose": "offline_source_availability_only", "complete_corresponding_source": False,
            "publisher_authenticated": False, "packaged_manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
            "cargo_graph_sha256": graph_hash, "cargo_lock_sha256": manifest["cargo_lock_sha256"],
            "application_source_archive_sha256": manifest["application_source_archive_sha256"],
            "hashed_input_bytes": budget.hashed, "max_hashed_input_bytes": max_hash_bytes,
            "cargo": cargo, "native_recipe_inputs": native,
            "limits": ["No download, build, extraction, native tool execution or cache mutation performed",
                       "Package evidence and lockfile must come from an independently trusted artifact; matching hashes do not authenticate a publisher",
                       "Verified archive bytes prove local availability, not patch application, source completeness or reproducible compilation",
                       "Git checkout HEAD and extracted registry tree presence are explicitly unverified source trees",
                       "Literal recipe parsing is partial; binary bottles are never source coverage",
                       "Deno embedded Rust/V8/JavaScript dependency source and notices are not fully inventoried",
                       "Slint, native resources, toolchain, generated code, license/SDK review and installation information remain separate obligations",
                       "Inputs must remain quiescent; this is not an adversarial concurrent-installation verifier"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True, help="Existing package Contents/Resources/BuildInfo directory")
    parser.add_argument("--cargo-home", type=Path, default=Path.home() / ".cargo")
    parser.add_argument("--brew-cache", type=Path, default=Path.home() / "Library/Caches/Homebrew/downloads")
    parser.add_argument("--max-hash-mib", type=int, default=256)
    parser.add_argument("--output", type=Path, required=True, help="New report file; existing files are never overwritten")
    parser.add_argument("--export-git-sources", type=Path,
                        help="Opt in to local immutable Git object export into a NEW private directory; no downloads")
    args = parser.parse_args()
    try:
        if not 1 <= args.max_hash_mib <= 2048:
            raise CoverageError("Hash budget must be between 1 and 2048 MiB")
        report = audit(args.evidence.resolve(strict=True), args.cargo_home.resolve(), args.brew_cache.resolve(), args.max_hash_mib * 1024 * 1024)
        if args.export_git_sources is not None:
            from git_source_export import export_git_sources, CoverageError as GitCoverageError
            try:
                report["git_source_export"] = export_git_sources(report["cargo"], args.cargo_home.resolve(), args.export_git_sources)
            except GitCoverageError:
                # Direct script execution names this module __main__; the helper
                # imports source_coverage and therefore has its own class identity.
                raise CoverageError("Git source export failed") from None
            report["limits"][0] = "No download, build, extraction or cache mutation; explicit Git export ran isolated local object-reader commands"
        raw = (json.dumps(report, indent=2, sort_keys=True) + "\n").encode()
        descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(raw)
        print("Source-availability report written; complete corresponding source remains unverified.")
        return 0
    except (OSError, ValueError, KeyError, TypeError, AttributeError, IndexError,
            RecursionError, UnicodeError, tarfile.TarError, CoverageError):
        print("Source audit failed: missing, unsafe, inconsistent or oversized local input; no successful report.")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

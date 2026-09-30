#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Read-only development bundle integrity audit; never executes or extracts files.

An inventory hash authenticates nothing unless obtained through an independently
trusted channel. This tool does not verify code signatures or grant release status.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import tarfile
import unicodedata

JSON_LIMIT = 16 * 1024 * 1024
SOURCE_LIMIT = 128 * 1024 * 1024
MAX_FILES = 100_000
MAX_FILE_BYTES = 2 * 1024 * 1024 * 1024
MAX_TREE_BYTES = 8 * 1024 * 1024 * 1024
BUILD = "Contents/Resources/BuildInfo/"
MANIFEST = BUILD + "build-manifest.json"
SOURCE = BUILD + "application-source.tar"


class VerificationError(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise VerificationError(message)


def sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def valid_hash(value: object) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def path_parts(value: object) -> list[str]:
    require(isinstance(value, str) and 0 < len(value) <= 4096, "Invalid inventory path")
    require(not any(ord(char) < 32 for char in value) and "\\" not in value,
            "Nonportable inventory path")
    parts = value.split("/")
    require(len(parts) <= 32 and all(part not in ("", ".", "..") for part in parts),
            "Absolute, traversal or excessive-depth path")
    require(all(":" not in part for part in parts), "Nonportable inventory path")
    return parts


def unique_name(value: str, names: set[str]) -> None:
    key = unicodedata.normalize("NFC", value).casefold()
    require(key not in names, "Duplicate or filesystem-ambiguous path")
    names.add(key)


def strict_json(data: bytes) -> dict:
    require(len(data) <= JSON_LIMIT, "JSON exceeds audit limit")

    def pairs(items: list[tuple[str, object]]) -> dict:
        result = {}
        for key, value in items:
            require(key not in result, "Duplicate JSON key")
            result[key] = value
        return result

    def constant(_: str) -> None:
        raise VerificationError("Nonfinite JSON number")

    try:
        result = json.loads(data, object_pairs_hook=pairs, parse_constant=constant)
    except (ValueError, RecursionError) as error:
        raise VerificationError("Malformed JSON") from error
    require(isinstance(result, dict), "Expected JSON object")
    return result


def open_regular(path: str | Path, *, parent: int | None = None) -> int:
    # O_NONBLOCK prevents a raced-in FIFO/device from blocking before fstat.
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    if not stat.S_ISREG(os.fstat(fd).st_mode):
        os.close(fd)
        raise VerificationError("Expected a regular file; links and special files are forbidden")
    return fd


def read_bounded(fd: int, limit: int) -> bytes:
    require(os.fstat(fd).st_size <= limit, "File exceeds audit limit")
    parts = []
    size = 0
    while chunk := os.read(fd, min(1024 * 1024, limit + 1 - size)):
        size += len(chunk)
        require(size <= limit, "File changed or exceeds audit limit")
        parts.append(chunk)
    return b"".join(parts)


def records(value: object, *, sizes: bool = False) -> dict[str, dict]:
    require(isinstance(value, list) and 0 < len(value) <= MAX_FILES, "Invalid file inventory count")
    output = {}
    names: set[str] = set()
    for item in value:
        require(isinstance(item, dict), "Invalid file record")
        path_parts(item.get("path"))
        unique_name(item["path"], names)
        require(valid_hash(item.get("sha256")), "Invalid recorded file hash")
        if sizes:
            require(type(item.get("bytes")) is int and 0 <= item["bytes"] <= MAX_FILE_BYTES,
                    "Invalid recorded file size")
        output[item["path"]] = item
    return output


class Tree:
    """Descriptor-relative traversal never follows a bundle directory symlink."""
    def __init__(self, root: Path):
        self.fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)

    def close(self) -> None:
        os.close(self.fd)

    def file(self, path: str) -> int:
        parts = path_parts(path)
        directory = os.dup(self.fd)
        try:
            for part in parts[:-1]:
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
                os.close(directory)
                directory = child
            return open_regular(parts[-1], parent=directory)
        finally:
            os.close(directory)

    def snapshot(self) -> dict[str, tuple]:
        found = {}
        names: set[str] = set()

        def walk(fd: int, prefix: str = "") -> None:
            with os.scandir(fd) as entries:
                for entry in entries:
                    path = prefix + entry.name
                    path_parts(path)
                    unique_name(path, names)
                    require(len(names) <= MAX_FILES * 2, "Tree exceeds entry limit")
                    info = entry.stat(follow_symlinks=False)
                    require(not stat.S_ISLNK(info.st_mode), "Bundle symlink rejected by regular-file inventory policy")
                    if stat.S_ISDIR(info.st_mode):
                        child = os.open(entry.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                        try:
                            walk(child, path + "/")
                        finally:
                            os.close(child)
                    else:
                        require(stat.S_ISREG(info.st_mode), "Special bundle file rejected")
                        require(info.st_nlink == 1, "Hard-linked bundle file rejected")
                        found[path] = (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
                        require(len(found) <= MAX_FILES, "Tree exceeds file limit")
        walk(self.fd)
        return found

    def data(self, path: str, limit: int) -> bytes:
        fd = self.file(path)
        try:
            return read_bounded(fd, limit)
        finally:
            os.close(fd)


def verify_source(tree: Tree, manifest: dict) -> int:
    fd = tree.file(SOURCE)
    fingerprints = []
    names: set[str] = set()
    total = 0
    lock_hash = None
    with os.fdopen(fd, "rb") as stream:
        require(os.fstat(stream.fileno()).st_size <= SOURCE_LIMIT, "Source archive exceeds audit limit")
        # Streaming uncompressed tar only; never extract any member to disk.
        with tarfile.open(fileobj=stream, mode="r|") as archive:
            for member in archive:
                path_parts(member.name)
                unique_name(member.name, names)
                require(len(names) <= MAX_FILES and member.isreg() and not member.sparse,
                        "Source archive has excess or nonregular members")
                total += member.size
                require(0 <= member.size <= JSON_LIMIT * 4 and total <= SOURCE_LIMIT,
                        "Source archive member exceeds audit limit")
                file = archive.extractfile(member)
                require(file is not None, "Unreadable source member")
                with file:
                    content = file.read(member.size + 1)
                require(len(content) == member.size, "Truncated source member")
                checksum = sha256(content)
                fingerprints.append((member.name, checksum))
                if member.name == "Cargo.lock":
                    lock_hash = checksum
    payload = json.dumps(sorted(fingerprints, key=lambda item: PurePosixPath(item[0])), separators=(",", ":")).encode()
    require(sha256(payload) == manifest["source_fingerprint"], "Source fingerprint mismatch")
    require(lock_hash == manifest["cargo_lock_sha256"], "Archived Cargo.lock mismatch")
    return len(fingerprints)


def verify(bundle: Path, inventory: Path, expected_inventory_hash: str | None = None) -> dict:
    if expected_inventory_hash is not None:
        require(valid_hash(expected_inventory_hash), "Invalid trusted inventory hash")
    fd = open_regular(inventory)
    try:
        raw = read_bounded(fd, JSON_LIMIT)
    finally:
        os.close(fd)
    actual_inventory_hash = sha256(raw)
    require(expected_inventory_hash is None or expected_inventory_hash == actual_inventory_hash,
            "Inventory hash differs from supplied digest")
    index = strict_json(raw)
    require(type(index.get("schema")) is int and index["schema"] == 1, "Unsupported inventory schema")
    require(index.get("development_only") is True, "Only development inventories are supported")
    path_parts(index.get("bundle"))
    require("/" not in index["bundle"] and index["bundle"].endswith(".app"), "Invalid bundle label")
    require(valid_hash(index.get("input_fingerprint")), "Invalid input fingerprint")
    expected = records(index.get("files"), sizes=True)
    require(MANIFEST in expected and SOURCE in expected, "Required build evidence is absent")
    require(sum(item["bytes"] for item in expected.values()) <= MAX_TREE_BYTES, "Bundle exceeds audit limit")
    tree = Tree(bundle)
    try:
        before = tree.snapshot()
        require(before.keys() == expected.keys(), "Bundle file tree differs from inventory")
        for path, item in expected.items():
            fd = tree.file(path)
            try:
                info = os.fstat(fd)
                require(info.st_size == item["bytes"], "Bundle file size mismatch")
                require(info.st_nlink == 1, "Hard-linked bundle file rejected")
                checksum = hashlib.sha256()
                count = 0
                while chunk := os.read(fd, 1024 * 1024):
                    count += len(chunk)
                    require(count <= item["bytes"], "Bundle file changed during verification")
                    checksum.update(chunk)
                require(count == item["bytes"] and checksum.hexdigest() == item["sha256"],
                        "Bundle file hash mismatch")
            finally:
                os.close(fd)
        manifest = strict_json(tree.data(MANIFEST, JSON_LIMIT))
        require(type(manifest.get("schema")) is int and manifest["schema"] == 1,
                "Unsupported manifest schema")
        require(manifest.get("development_only") is True and manifest.get("portable") is False,
                "Manifest overstates development artifact status")
        require(manifest.get("target") == "aarch64-apple-darwin", "Unqualified manifest target")
        for key in ("source_fingerprint", "cargo_lock_sha256", "application_source_archive_sha256", "input_fingerprint"):
            require(valid_hash(manifest.get(key)), "Invalid manifest hash")
        require(manifest["input_fingerprint"] == index["input_fingerprint"], "Manifest/inventory fingerprint mismatch")
        payload = {key: value for key, value in manifest.items() if key != "input_fingerprint"}
        require(sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()) == manifest["input_fingerprint"],
                "Manifest input fingerprint mismatch")
        require(expected[SOURCE]["sha256"] == manifest["application_source_archive_sha256"], "Source archive hash mismatch")
        for path, item in records(manifest.get("evidence_input_files")).items():
            require(BUILD + path in expected and expected[BUILD + path]["sha256"] == item["sha256"],
                    "Manifest evidence file mismatch")
        certificate = manifest.get("media_certificate_resource")
        require(isinstance(certificate, dict), "Missing media trust resource")
        require(certificate.get("bundle_path") == "Contents/Resources/Certificates/mozilla.pem",
                "Unexpected media trust resource path")
        require(certificate["bundle_path"] in expected and expected[certificate["bundle_path"]]["sha256"] == certificate.get("sha256"),
                "Media trust resource hash mismatch")
        first_party = manifest.get("first_party_helpers")
        if first_party is not None:
            require(isinstance(first_party, list) and len(first_party) == 1,
                    "Invalid first-party helper inventory")
            helper = first_party[0]
            require(isinstance(helper, dict) and helper.get("name") == "oxplay-dns"
                    and helper.get("cargo_package") == "oxplay-network"
                    and helper.get("license") == "GPL-3.0-or-later"
                    and helper.get("bundle_path") == "Contents/Helpers/oxplay-dns"
                    and helper["bundle_path"] in expected
                    and valid_hash(helper.get("original_sha256"))
                    and type(helper.get("source_build_performed")) is bool,
                    "Invalid first-party DNS helper record")
            # Native input bytes change during relocation/signing. The original
            # digest must bind to recorded native inputs, not equal final bytes.
            native_inputs = manifest.get("native_inputs")
            require(isinstance(native_inputs, dict)
                    and isinstance(native_inputs.get(helper.get("source")), dict)
                    and native_inputs[helper["source"]].get("sha256") == helper["original_sha256"],
                    "First-party helper native input mismatch")
        helpers = manifest.get("bundled_helper_resources")
        if helpers is not None:
            require(isinstance(helpers, dict) and isinstance(helpers.get("files"), list), "Invalid helper inventory")
            require(0 < len(helpers["files"]) <= MAX_FILES and all(isinstance(item, dict) for item in helpers["files"]),
                    "Invalid helper file count or record")
            rows = records([dict(item, path=item.get("target")) for item in helpers["files"]], sizes=True)
            for path, item in rows.items():
                require(path in expected and type(item.get("macho")) is bool, "Invalid helper file record")
                # Native hashes identify original inputs, before relocation/signing.
                if not item["macho"]:
                    require(expected[path]["sha256"] == item["sha256"] and expected[path]["bytes"] == item["bytes"],
                            "Helper resource mismatch")
            require("Contents/Helpers/yt-dlp" in expected and "Contents/Helpers/deno" in expected,
                    "Bundled helper launcher is missing")
        source_count = verify_source(tree, manifest)
        require(tree.snapshot() == before, "Bundle changed during verification")
        return {"schema": 1, "integrity": "passed", "files": len(expected),
                "bytes": sum(item["bytes"] for item in expected.values()), "source_files": source_count,
                "inventory_sha256": actual_inventory_hash, "supplied_inventory_digest_matched": expected_inventory_hash is not None,
                "publisher_authenticated": False, "code_signatures_checked": False,
                "development_only": True, "portable_qualified": False}
    finally:
        tree.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--inventory", type=Path, help="Defaults to BUNDLE.inventory.json")
    parser.add_argument("--inventory-sha256", help="Expected digest obtained independently; not an embedded trust root")
    args = parser.parse_args()
    try:
        report = verify(args.bundle, args.inventory or Path(str(args.bundle) + ".inventory.json"), args.inventory_sha256)
    except VerificationError as error:
        # VerificationError messages are fixed strings, never metadata values.
        print(f"Bundle integrity verification failed: {error}.", file=sys.stderr)
        raise SystemExit(1) from None
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError, RecursionError):
        # Never echo untrusted metadata, filenames, source paths or raw exceptions.
        print("Bundle integrity verification failed.", file=sys.stderr)
        raise SystemExit(1) from None
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Explicit maintainer-only network audit for omitted Rust package notices.

Downloads text only from official GitHub repositories at the revision recorded
in the installed crate's .cargo_vcs_info.json. Never invoked by the app or the
offline packager. Does not download or execute software or use GitHub credentials.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path, PurePosixPath
import posixpath
import re
import shutil
import tempfile
import urllib.parse
import urllib.request

import package_macos as packaging

PATTERNS = ("LICENSE*", "LICENCE*", "COPYING*", "COPYRIGHT*", "NOTICE*", "GPL*", "LGPL*", "AUTHORS*")


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "Serein-notice-audit/1"})
    with urllib.request.urlopen(request, timeout=30) as response:
        if urllib.parse.urlparse(response.url).hostname not in {"api.github.com", "raw.githubusercontent.com"}:
            raise packaging.PackagingError("Notice request redirected outside the reviewed source hosts")
        body = response.read(8 * 1024 * 1024 + 1)
    if len(body) > 8 * 1024 * 1024:
        raise packaging.PackagingError("Notice source exceeded its size bound")
    return body


def has_local_notice(directory: Path) -> bool:
    return any(path.is_file() for pattern in PATTERNS for path in directory.glob(pattern))


def notice_name(name: str) -> bool:
    return any(name.upper().startswith(prefix) for prefix in ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "AUTHORS", "GPL", "LGPL"))


def source_group(group: tuple[tuple[str, str], list[dict]]) -> list[tuple[dict, dict[str, bytes]]]:
    (repository, revision), packages = group
    slug = urllib.parse.urlparse(repository).path.strip("/").removesuffix(".git")
    response = json.loads(fetch(f"https://api.github.com/repos/{slug}/git/trees/{revision}?recursive=1"))
    if response.get("truncated"):
        raise packaging.PackagingError("Upstream tree was truncated; notice selection requires review")
    tree = {entry["path"]: entry for entry in response["tree"] if entry["type"] == "blob"}
    downloaded: dict[str, bytes] = {}

    def contents(path: str, seen: set[str] | None = None) -> tuple[str, bytes]:
        seen = seen or set()
        if path in seen or len(seen) >= 8 or path not in tree:
            raise packaging.PackagingError("Invalid upstream notice symlink")
        seen.add(path)
        if path not in downloaded:
            url = f"https://raw.githubusercontent.com/{slug}/{revision}/{urllib.parse.quote(path, safe='/')}"
            body = fetch(url)
            expected = tree[path]["sha"]
            actual = hashlib.sha1(b"blob " + str(len(body)).encode() + b"\0" + body).hexdigest()
            if actual != expected:
                raise packaging.PackagingError("Upstream notice bytes do not match the inspected Git blob")
            downloaded[path] = body
        body = downloaded[path]
        if tree[path].get("mode") == "120000":
            target = posixpath.normpath(str(PurePosixPath(path).parent / body.decode().strip()))
            if target.startswith("../") or target.startswith("/"):
                raise packaging.PackagingError("Upstream notice symlink escapes the source repository")
            return contents(target, seen)
        body.decode("utf-8")
        return path, body

    results = []
    for package in packages:
        package_path = PurePosixPath(package["path_in_vcs"] or ".")
        ancestors = {str(parent) for parent in (package_path, *package_path.parents)} | {"."}
        candidates = []
        for path in sorted(tree):
            source = PurePosixPath(path)
            if str(source.parent) in ancestors and notice_name(source.name):
                candidates.append(path)
            elif source.parent.name.lower() in {"licenses", "licences"} and str(source.parent.parent) in ancestors:
                candidates.append(path)
        files, copied, resolved = [], {}, set()
        for candidate in candidates:
            path, body = contents(candidate)
            if path in resolved:
                continue
            resolved.add(path)
            filename = f"{len(files):03}-{PurePosixPath(path).name}"
            copied[filename] = body
            files.append({"path": filename, "sha256": hashlib.sha256(body).hexdigest(),
                          "upstream_path": path, "git_blob_sha1": tree[path]["sha"],
                          "source_url": f"https://raw.githubusercontent.com/{slug}/{revision}/{urllib.parse.quote(path, safe='/')}"})
        record = {**package, "repository": repository, "revision": revision,
                  "git_tree_sha1": response["sha"], "files": files}
        if repository == "https://github.com/madsmtm/objc2" and any(b"## Apple SDKs" in body and b"Permission is hereby granted" not in body for body in copied.values()):
            record["review_required"] = "Original upstream policy document contains license links rather than full grant text and identifies Apple SDK-derived bindings as requiring licensing review; collecting this file does not resolve those questions."
        if not files:
            record["unresolved_reason"] = "The exact upstream tree contains no original license/notice file in the crate or its ancestor directories; a manifest license expression alone is not a recovered notice."
            record["tree_source_url"] = f"https://api.github.com/repos/{slug}/git/trees/{revision}?recursive=1"
        results.append((record, copied))
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=packaging.ROOT / "third_party/notices")
    args = parser.parse_args()
    output = args.output.absolute()
    if output.exists():
        raise packaging.PackagingError("Choose a new notice evidence directory; existing evidence is never overwritten")
    cargo = packaging.cargo_graph("aarch64-apple-darwin")
    groups: dict[tuple[str, str], list[dict]] = {}
    for package in cargo["packages"]:
        if not (package["source"] or "").startswith("registry+"):
            continue
        directory = Path(package["manifest_path"]).parent
        if has_local_notice(directory):
            continue
        info_file = directory / ".cargo_vcs_info.json"
        info = json.loads(info_file.read_text())
        revision = info.get("git", {}).get("sha1", "")
        if info.get("git", {}).get("dirty") or not re.fullmatch(r"[a-f0-9]{40}", revision):
            raise packaging.PackagingError("Missing or dirty upstream revision needs manual notice review")
        manifest = packaging.tomllib.loads(Path(package["manifest_path"]).read_text())["package"]
        repository = manifest["repository"].replace("http://github.com/", "https://github.com/").rstrip("/")
        parsed = urllib.parse.urlparse(repository)
        if parsed.scheme != "https" or parsed.hostname != "github.com" or len(parsed.path.strip("/").split("/")) != 2:
            raise packaging.PackagingError("An omitted notice comes from an unreviewed source host")
        record = {"name": package["name"], "version": package["version"], "declared_license": package["license"],
                  "path_in_vcs": info.get("path_in_vcs", ""), "cargo_vcs_info_sha256": packaging.digest(info_file)}
        groups.setdefault((repository, revision), []).append(record)
    # Bounded concurrent, read-only public-source requests; no automatic retries.
    with ThreadPoolExecutor(max_workers=4) as pool:
        results = [item for group in pool.map(source_group, groups.items()) for item in group]
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=".serein-notices-", dir=output.parent))
    try:
        records = []
        for record, copied in sorted(results, key=lambda item: (item[0]["name"], item[0]["version"])):
            directory = stage / f"{record['name']}-{record['version']}"
            directory.mkdir()
            for name, content in copied.items():
                (directory / name).write_bytes(content)
            records.append(record)
        packaging.json_write(stage / "manifest.json", {"schema": 1, "purpose": "Original upstream notices omitted from published Cargo archives; not complete corresponding source or distribution clearance", "packages": records})
        stage.rename(output)
        recovered = sum(bool(record["files"]) for record in records)
        print(f"Recovered original notices for {recovered} packages at {len(groups)} exact revisions; {len(records) - recovered} notice gaps remain explicit.")
    finally:
        if stage.exists():
            shutil.rmtree(stage)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"Notice audit stopped: {error}")
        raise SystemExit(1) from None

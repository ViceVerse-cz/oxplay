#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Prepare an explicitly isolated offline resource fixture, never a user profile.

Run outside the application and outside measurement windows. The selected local
clip supplies actual raster stills; metadata is conspicuously synthetic. A
90-second clip is required for thirty distinct three-second samples.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys

HEADER = "SEREIN_LIBRARY_RESOURCE_FIXTURE_V1\nitems=10000\nimages=30\n"
MAX_IMAGE = 2 * 1024 * 1024


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def private_text(path):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    return os.fdopen(descriptor, "w")


def prepare(root, clip, ffmpeg):
    if not root.is_absolute() or not clip.is_absolute():
        raise ValueError("fixture root and local clip must be absolute")
    source = clip.resolve(strict=True)
    if not source.is_file():
        raise ValueError("local clip must be a regular file")
    # Deliberately no parents=True or exist_ok=True; existing files, symlinks,
    # directories and ordinary profiles are never reused or erased.
    root.mkdir(mode=0o700)
    profile = root / "Serein"
    profile.mkdir(mode=0o700)
    database = profile / "library.sqlite3"
    fd = os.open(database, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    os.close(fd)
    repository = Path(__file__).resolve().parents[1]
    schemas = repository / "crates/storage/src"
    connection = sqlite3.connect(database)
    try:
        connection.execute("PRAGMA foreign_keys=ON")
        connection.executescript("BEGIN IMMEDIATE;\n" + "\n".join(
            (schemas / f"schema_v{version}.sql").read_text()
            for version in range(1, 5)
        ))
        connection.execute("INSERT INTO local_playlists(name) VALUES (?)", (
            "TEST FIXTURE — 10,000 offline synthetic library items",))
        connection.executemany(
            "INSERT INTO local_playlist_items(playlist_id,video_id,title,channel_name,duration_seconds) VALUES (1,?,?,?,?)",
            ((f"f{index:010}", f"TEST FIXTURE — offline item {index:05}",
              "TEST FIXTURE — local raster; no online metadata or playback", 60 + index)
             for index in range(10_000)),
        )
        connection.execute("PRAGMA user_version=4")
        connection.commit()
        count = connection.execute("SELECT count(*) FROM local_playlist_items").fetchone()[0]
        if count != 10_000:
            raise ValueError("fixture database count mismatch")
    finally:
        connection.close()

    command = [str(ffmpeg), "-nostdin", "-hide_banner", "-loglevel", "error",
               "-protocol_whitelist", "file,pipe", "-threads", "1", "-i", str(source),
               "-an", "-vf", "fps=1/3,scale=640:360:flags=bilinear", "-frames:v", "30",
               "-threads", "1", "-start_number", "0", str(root / "thumb-%02d.png")]
    # stderr can contain the user's path: do not forward it to public diagnostics.
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, start_new_session=True)
    try:
        result = process.wait(timeout=300)
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        raise
    if result:
        raise ValueError("local raster extraction failed; incomplete fixture is not admitted")
    rows = []
    provenance = {}
    for index in range(30):
        name = f"thumb-{index:02}.png"
        image = root / name
        metadata = image.lstat()
        if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= MAX_IMAGE:
            raise ValueError("raster fixture is missing, nonregular or oversized")
        image.chmod(0o600)
        rows.append(f"{name} {metadata.st_size}\n")
        provenance[name] = {"sha256": digest(image), "compressed_bytes": metadata.st_size}
    if len({item["sha256"] for item in provenance.values()}) != 30:
        raise ValueError("fixture requires thirty distinct raster samples, not duplicate placeholders")
    evidence = {
        "fixture": "explicit offline synthetic metadata and decoded local raster samples",
        "items": 10_000, "images": provenance, "raster_dimensions": [640, 360],
        "local_source_sha256": digest(source), "ffmpeg_sha256": digest(ffmpeg),
        "initial_database_sha256": digest(database),
        "schema_version": 4,
        "schema_sha256": {f"v{version}": digest(schemas / f"schema_v{version}.sql") for version in range(1, 5)},
        "visible_thumbnail_requirement": "Not asserted: native viewport geometry must be measured separately",
    }
    with private_text(root / "fixture-provenance.json") as stream:
        json.dump(evidence, stream, indent=2)
        stream.write("\n")
    # Written last: interrupted/failed preparation has no valid admission marker.
    with private_text(root / ".serein-library-resource-fixture-v1") as stream:
        stream.write(HEADER + "".join(rows))
        stream.flush()
        os.fsync(stream.fileno())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="absolute new private fixture directory")
    parser.add_argument("clip", type=Path, help="absolute local clip, at least90seconds with distinct content")
    parser.add_argument("--ffmpeg", type=Path, default=None)
    args = parser.parse_args()
    executable = args.ffmpeg or shutil.which("ffmpeg")
    if not executable:
        parser.error("ffmpeg is required to prepare raster fixtures")
    old_umask = os.umask(0o077)
    try:
        prepare(args.root, args.clip, Path(executable).resolve(strict=True))
    except (OSError, ValueError, sqlite3.Error, subprocess.TimeoutExpired):
        print("Fixture preparation failed. No valid marker was written; existing data was not reused.", file=sys.stderr)
        return 1
    finally:
        os.umask(old_umask)
    print("Prepared 10,000 labeled rows and 30 distinct raster fixtures; no network used. Native visible count remains unverified.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

# SPDX-License-Identifier: GPL-3.0-or-later
"""Shared third-party notice text for the Linux and Windows release payloads."""
from __future__ import annotations

import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
LICENSES = ROOT / "packaging/licenses"
HELPER_LICENSES = ("deno-LICENSE.md", "yt-dlp-LICENSE", "yt-dlp-THIRD_PARTY_LICENSES.txt")


def pinned_inputs(directory: Path) -> list[dict]:
    record = directory / "pinned-inputs.json"
    if not record.is_file():
        raise ValueError("Pinned helper provenance (pinned-inputs.json) is missing; fetch helpers first")
    return json.loads(record.read_text(encoding="utf-8"))


def render(inputs: list[dict], mpv: str) -> str:
    lines = ["# Oxplay third-party components", "",
             "Oxplay itself is GPL-3.0-or-later (see LICENSE). This package also contains:", "",
             f"* libmpv: {mpv}"]
    for item in inputs:
        if item["name"].startswith(("yt-dlp", "deno", "mpv-dev")):
            lines.append(f"* {item['name']} {item['version']}: {item['license']}. Source: {item['source']}. "
                         f"Downloaded from {item['url']} (SHA-256 {item['sha256']}).")
    lines += ["", "License texts for the bundled helpers are in the licenses/ directory. Rust crate",
              "notices are in the source archive attached to the same release (third_party/ and",
              "each crate's own license files); complete corresponding-source coverage for native",
              "builds is still being audited (docs/licensing.md).", ""]
    return "\n".join(lines)


def copy_helper_licenses(destination: Path) -> list[Path]:
    destination.mkdir(parents=True, exist_ok=True)
    copied = []
    for name in HELPER_LICENSES:
        shutil.copyfile(LICENSES / name, destination / name)
        copied.append(destination / name)
    return copied

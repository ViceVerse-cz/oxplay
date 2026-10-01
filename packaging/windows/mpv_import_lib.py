#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Prepare MPV_DIR from the pinned shinchiro mpv-dev archive for MSVC builds.

The archive ships include/mpv/*.h, libmpv-2.dll and a MinGW libmpv.dll.a.
MSVC links against an import library, so this writes mpv.def from the DLL's
export table (dumpbin) and mpv.lib (lib.exe), both located through vswhere.
The result: MPV_DIR/{include/mpv/client.h, libmpv-2.dll, libmpv.dll.a, mpv.def, mpv.lib}.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

VSWHERE = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Microsoft Visual Studio/Installer/vswhere.exe"


def exports_from_dumpbin(text: str) -> list[str]:
    """Named exports from `dumpbin /exports` (ordinal, hint, RVA, name)."""
    names = []
    for line in text.splitlines():
        match = re.match(r"^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]{8}\s+([A-Za-z_][A-Za-z0-9_@?$]*)(?:\s|$)", line)
        if match:
            names.append(match[1])
    return names


def definition(names: list[str], library: str = "libmpv-2.dll") -> str:
    if not names or not any(name == "mpv_create" for name in names):
        raise ValueError("The DLL export table lacks the libmpv client API")
    return f"LIBRARY {library}\nEXPORTS\n" + "".join(f"    {name}\n" for name in sorted(set(names)))


def msvc_tool(name: str) -> Path:
    installation = subprocess.run([str(VSWHERE), "-latest", "-products", "*", "-requires",
                                   "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property",
                                   "installationPath"], capture_output=True, text=True, check=True).stdout.strip()
    candidates = sorted(Path(installation, "VC/Tools/MSVC").glob(f"*/bin/Hostx64/x64/{name}"))
    if not candidates:
        raise ValueError(f"MSVC {name} was not found")
    return candidates[-1]


def prepare(directory: Path) -> Path:
    dll = directory / "libmpv-2.dll"
    if not dll.is_file() or not (directory / "include/mpv/client.h").is_file():
        raise ValueError("Expected an extracted mpv-dev archive with libmpv-2.dll and include/mpv/client.h")
    dump = subprocess.run([str(msvc_tool("dumpbin.exe")), "/nologo", "/exports", str(dll)],
                          capture_output=True, text=True, check=True).stdout
    (directory / "mpv.def").write_text(definition(exports_from_dumpbin(dump)), encoding="ascii")
    subprocess.run([str(msvc_tool("lib.exe")), "/nologo", "/machine:x64", f"/def:{directory / 'mpv.def'}",
                    f"/out:{directory / 'mpv.lib'}"], check=True, stdout=subprocess.DEVNULL)
    return directory / "mpv.lib"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mpv_dir", type=Path)
    parser.add_argument("--github-env", type=Path, help="Append MPV_DIR=<path> for later workflow steps")
    args = parser.parse_args()
    try:
        library = prepare(args.mpv_dir.resolve(strict=True))
        if args.github_env:
            with args.github_env.open("a", encoding="utf-8") as environment:
                environment.write(f"MPV_DIR={args.mpv_dir.resolve()}\n")
        print(library)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"libmpv import library preparation stopped: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Prepare an isolated, verified mpv timer ON/OFF build plan. Never builds/installs.

Explicit --download permits only pinned inputs. No package manager is invoked.
command-plan.json is private machine-local evidence, not a public report.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shlex
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
from urllib.parse import urlsplit

MIB = 1024 * 1024
MAX_ARCHIVE = 32 * MIB
MAX_PATCH = 2 * MIB
MAX_EXPANDED = 128 * MIB
MAX_MEMBER = 16 * MIB
MAX_ENTRIES = 12000
MAX_NATIVE_BYTES = 2 * 1024 * MIB
MAX_NATIVE_FILE = 256 * MIB
FORMULA_SHA = "75ccdb2dcc87a829323f276466c902a1f97f836d0f09c7e7341957b9eb7db9e6"
PATCH_SHA = "1d4908ba5e481fe8cf7b7b22895c1cae5dec6ad71437df80740f2ab031ea1f8e"
INPUTS = (
    ("mpv-0.41.0.tar.gz", "https://github.com/mpv-player/mpv/archive/refs/tags/v0.41.0.tar.gz", "ee21092a5ee427353392360929dc64645c54479aefdb5babc5cfbb5fad626209", MAX_ARCHIVE),
    ("75b2ccf.patch", "https://github.com/mpv-player/mpv/commit/75b2ccfeb1ce4ed5a40ac9860fa74f3d1265e13f.patch?full_index=1", "3906b98b02071a0d5747a400406494ca69cef7afd8d3eee4a99fdbe40dc90c1f", MAX_PATCH),
    ("c5d391a.patch", "https://github.com/mpv-player/mpv/commit/c5d391adba7bd024954d0df1e0405f5749f4d4ca.patch?full_index=1", "769b218df220738cc1cf9f81cf696c16518c5dfe56a5ef028e33b22536e0e924", MAX_PATCH),
)
TOOLS = ("meson", "ninja", "pkgconf", "git", "clang", "clang++", "swiftc", "xcrun", "python3")
STD_MESON = '["--prefix=#{prefix}", "--libdir=#{libdir}", "--buildtype=release", "--wrap-mode=nofallback"]'


class DiagnosticError(Exception):
    """Static public errors do not expose local paths or subprocess output."""


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_file(path: Path, limit: int) -> bytes:
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
            raise DiagnosticError("Input is not a bounded regular file")
        data = stream.read(limit + 1)
        after = os.fstat(stream.fileno())
        if len(data) > limit or (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            raise DiagnosticError("Input changed or exceeded its limit")
        return data


def private_write(path: Path, data: bytes) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)


def safe_relative(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if (not name or len(name) > 512 or "\\" in name or "\x00" in name
            or path.is_absolute() or any(p in ("", ".", "..") for p in name.rstrip("/").split("/"))):
        raise DiagnosticError("Unsafe archive path")
    if not path.parts or path.parts[0] != "mpv-0.41.0":
        raise DiagnosticError("Unexpected archive root")
    return path


def extract_source(data: bytes, destination: Path) -> None:
    """No tar.extract/extractall. Bound decompression before any PAX parsing."""
    if destination.exists():
        raise DiagnosticError("Source destination already exists")
    with gzip.GzipFile(fileobj=io.BytesIO(data)) as stream:
        expanded = stream.read(MAX_EXPANDED + 1)
    if len(expanded) > MAX_EXPANDED:
        raise DiagnosticError("Expanded archive limit exceeded")
    # Tarfile can allocate extension payloads before returning the next member.
    # Preflight raw headers first; permit bounded PAX, reject GNU sparse/longname.
    offset = entries = 0
    while offset + 512 <= len(expanded):
        block = expanded[offset:offset + 512]
        if block == bytes(512):
            if any(expanded[offset:]):
                raise DiagnosticError("Unexpected data after archive terminator")
            break
        header = tarfile.TarInfo.frombuf(block, "utf-8", "surrogateescape")
        entries += 1
        if entries > MAX_ENTRIES or header.size < 0 or header.size > MAX_MEMBER:
            raise DiagnosticError("Archive entry limit exceeded")
        if header.type not in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE, tarfile.XHDTYPE, tarfile.XGLTYPE):
            raise DiagnosticError("Archive links or special entries are unsupported")
        if header.type in (tarfile.XHDTYPE, tarfile.XGLTYPE) and header.size > 64 * 1024:
            raise DiagnosticError("PAX metadata limit exceeded")
        offset += 512 + ((header.size + 511) // 512) * 512
        if offset > len(expanded):
            raise DiagnosticError("Truncated archive")
    else:
        raise DiagnosticError("Missing archive terminator")
    members = []
    seen = set()
    total = 0
    with tarfile.open(fileobj=io.BytesIO(expanded), mode="r:") as archive:
        for member in archive:
            path = safe_relative(member.name)
            if path in seen or member.issym() or member.islnk() or not (member.isdir() or member.isfile()) or member.sparse:
                raise DiagnosticError("Duplicate or unsupported archive member")
            if len(members) >= MAX_ENTRIES or not 0 <= member.size <= MAX_MEMBER:
                raise DiagnosticError("Archive member limit exceeded")
            if any(key.startswith("GNU.sparse") for key in member.pax_headers):
                raise DiagnosticError("Sparse archives are unsupported")
            seen.add(path)
            total += member.size
            if total > MAX_EXPANDED:
                raise DiagnosticError("Archive payload limit exceeded")
            members.append((member, path))
        # Full path/type preflight before the first write.
        types = {path: member.isdir() for member, path in members}
        for _, path in members:
            if any(parent in types and not types[parent] for parent in path.parents):
                raise DiagnosticError("Archive parent is a file")
        destination.mkdir(mode=0o700)
        for member, path in members:
            target = destination.joinpath(*path.parts[1:])
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True, mode=0o700)
            else:
                target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                stream = archive.extractfile(member)
                if stream is None:
                    raise DiagnosticError("Missing archive payload")
                payload = stream.read(member.size + 1)
                if len(payload) != member.size:
                    raise DiagnosticError("Truncated archive payload")
                private_write(target, payload)
                # Preserve only executable intent, never special/group/world bits.
                if member.mode & 0o111:
                    target.chmod(0o700)


class PinnedRedirects(urllib.request.HTTPRedirectHandler):
    max_redirections = 3
    max_repeats = 1

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        parsed = urlsplit(newurl)
        if (parsed.scheme != "https" or parsed.hostname not in ("github.com", "codeload.github.com")
                or parsed.username or parsed.password or parsed.port not in (None, 443)):
            raise DiagnosticError("Download redirect violates source policy")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def download_bytes(url: str, limit: int) -> bytes:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), PinnedRedirects())
    deadline = time.monotonic() + 60
    with opener.open(urllib.request.Request(url, headers={"User-Agent": "Serein-source-audit/1"}), timeout=5) as response:
        if response.status != 200:
            raise DiagnosticError("Source download failed")
        chunks = []
        count = 0
        while True:
            if time.monotonic() >= deadline:
                raise DiagnosticError("Source download deadline exceeded")
            block = response.read1(min(65536, limit - count + 1))
            if not block:
                return b"".join(chunks)
            chunks.append(block)
            count += len(block)
            if count > limit:
                raise DiagnosticError("Source download byte limit exceeded")


def download(url: str, limit: int) -> bytes:
    # DNS/connect/TLS initialization can block below urllib's socket timeout.
    # A supervised transient process gives the caller a whole-operation deadline.
    index = next((i for i, item in enumerate(INPUTS) if item[1] == url and item[3] == limit), None)
    if index is None:
        raise DiagnosticError("Download is not a pinned source input")
    temp = tempfile.mkdtemp(prefix="serein-mpv-download-")
    env = {"HOME": temp, "TMPDIR": temp, "PATH": "/usr/bin:/bin", "LC_ALL": "C",
           "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1"}
    # On command/cleanup failure retain the private directory for explicit cleanup;
    # never delete storage while a descendant's shutdown remains unconfirmed.
    result = command([sys.executable, str(Path(__file__).resolve()), "--internal-fetch", str(index)],
                     env, Path(temp), timeout=60, max_output=limit)
    Path(temp).rmdir()
    return result


def checked_input(path: Path | None, url: str, expected: str, limit: int, allow_download: bool) -> bytes:
    if path is not None and path.exists():
        data = read_file(path, limit)
    elif allow_download:
        data = download(url, limit)
    else:
        raise DiagnosticError("Pinned source input is absent; provide offline inputs or explicit download")
    if digest(data) != expected:
        raise DiagnosticError("Source checksum mismatch")
    return data


def reap_group(process) -> None:
    # Darwin can report EPERM for a zombie-only group. Reaping the pinned
    # leader must be followed by positive group-absence confirmation, never a
    # second terminating signal that could hit a reused process-group ID.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        raise DiagnosticError("Process could not be reaped; private output retained") from None
    deadline = time.monotonic() + 2
    while True:
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            return
        except PermissionError:
            pass  # Permission failure still means absence has not been proven.
        if time.monotonic() >= deadline:
            raise DiagnosticError("Process group shutdown could not be confirmed; private output retained")
        time.sleep(0.01)


def command(args: list[str], env: dict[str, str], cwd: Path, timeout: int = 30, max_output: int = 2 * MIB) -> bytes:
    """Bounded probes, patch application and isolated downloads; never build commands."""
    import selectors
    if not all(hasattr(os, name) for name in ("waitid", "WNOWAIT", "WEXITED", "WNOHANG")):
        raise DiagnosticError("Host lacks the required unreaped-leader wait primitive")
    process = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
    output = bytearray()
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                if time.monotonic() >= deadline:
                    raise DiagnosticError("Prerequisite or patch command timed out")
                for key, _ in selector.select(min(0.1, max(0, deadline - time.monotonic()))):
                    block = os.read(key.fd, 65536)
                    if not block:
                        selector.unregister(key.fileobj)
                    else:
                        output.extend(block)
                        if len(output) > max_output:
                            raise DiagnosticError("Command output limit exceeded")
            # Observe exit without reaping: retain the leader PID/PGID until the
            # group kill, so reuse cannot direct that signal at another process.
            while True:
                status = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT | os.WNOHANG)
                if status is not None:
                    break
                if time.monotonic() >= deadline:
                    raise DiagnosticError("Prerequisite or patch command timed out")
                time.sleep(0.01)
            if status.si_code != os.CLD_EXITED or status.si_status != 0:
                raise DiagnosticError("Prerequisite or patch command failed")
        return bytes(output)
    finally:
        # Reap the direct child and terminate same-group descendants even on success.
        try:
            reap_group(process)
        finally:
            if process.stdout is not None:
                process.stdout.close()


def tool_paths(directories: list[Path]) -> dict[str, Path]:
    tools = {}
    for name in (*TOOLS, "rst2html"):
        alternatives = (name, "rst2html.py") if name == "rst2html" else (name,)
        found = next((directory / n for directory in directories for n in alternatives
                      if (directory / n).is_file() and os.access(directory / n, os.X_OK)), None)
        if found is None:
            raise DiagnosticError("Missing prerequisite: " + name + "; no automatic installation")
        tools[name] = found.absolute()
    return tools


def recipe_arguments(formula: bytes, homebrew_formula: bytes) -> tuple[list[str], dict]:
    if digest(formula) != FORMULA_SHA:
        raise DiagnosticError("Installed mpv recipe differs from reviewed 0.41.0_10")
    text = formula.decode()
    match = re.search(r"def install\s+args = %W\[(.*?)\]", text, re.S)
    if not match:
        raise DiagnosticError("Cannot derive installed recipe arguments")
    args = match[1].split()
    expected = ["--sysconfdir=#{etc}", "-Dbuild-date=false", "-Dhtml-build=enabled", "-Djavascript=enabled", "-Dlibmpv=true", "-Dlua=luajit", "-Dlibarchive=enabled", "-Duchardet=enabled", "-Dvulkan=enabled"]
    if args != expected:
        raise DiagnosticError("Recipe argument audit mismatch")
    std = re.search(r"def std_meson_args\([^\n]*\)\s*\n\s*(\[[^\n]+\])\s*\n\s*end", homebrew_formula.decode())
    if not std or std[1] != STD_MESON:
        raise DiagnosticError("Homebrew standard Meson arguments changed; review required")
    return args, {"mpv_formula_sha256": digest(formula), "homebrew_formula_sha256": digest(homebrew_formula), "std_meson_args_body": STD_MESON}


def native_inventory(brew: Path, keg: Path) -> tuple[dict, list[Path]]:
    cellar = (brew / "Cellar").resolve(strict=True)
    receipt = json.loads(read_file(keg / "INSTALL_RECEIPT.json", 2 * MIB))
    if receipt.get("source", {}).get("versions", {}).get("stable") != "0.41.0" or receipt.get("arch") != "arm64":
        raise DiagnosticError("Expected installed arm64 mpv 0.41.0 receipt")
    roots = [keg.resolve(strict=True)]
    dependencies = receipt.get("runtime_dependencies", [])
    if not isinstance(dependencies, list) or len(dependencies) > 128:
        raise DiagnosticError("Native dependency count exceeded")
    for dep in dependencies:
        name, version = dep.get("full_name"), dep.get("pkg_version")
        if not all(isinstance(v, str) and re.fullmatch(r"[A-Za-z0-9_.+@-]{1,100}", v) for v in (name, version)):
            raise DiagnosticError("Unsupported receipt dependency identity")
        roots.append((cellar / name / version).resolve(strict=True))
    files = {}
    pc_dirs = set()
    count = total = 0
    visited_directories = set()
    for root in sorted(set(roots)):
        if not root.is_relative_to(cellar):
            raise DiagnosticError("Native dependency escapes Cellar")
        for relative in ("lib/pkgconfig", "share/pkgconfig"):
            directory = root / relative
            if directory.is_dir():
                resolved = directory.resolve(strict=True)
                if not resolved.is_relative_to(root):
                    raise DiagnosticError("pkg-config directory escapes its captured keg")
                pc_dirs.add(resolved)
        for base, directories, names in os.walk(root, followlinks=True):
            resolved_base = Path(base).resolve(strict=True)
            if not resolved_base.is_relative_to(cellar):
                raise DiagnosticError("Native directory escapes Cellar")
            if resolved_base in visited_directories:
                directories[:] = []
                continue
            visited_directories.add(resolved_base)
            if len(visited_directories) > 50000:
                raise DiagnosticError("Native directory limit exceeded")
            directories.sort()
            for name in sorted(names):
                count += 1
                if count > 50000:
                    raise DiagnosticError("Native inventory entry limit exceeded")
                p = Path(base) / name
                if p.suffix not in (".h", ".hpp", ".a", ".so", ".dylib", ".pc", ".rb") and name != "INSTALL_RECEIPT.json":
                    continue
                real = p.resolve(strict=True)
                if not real.is_relative_to(cellar):
                    raise DiagnosticError("Native file escapes Cellar")
                label = "cellar/" + real.relative_to(cellar).as_posix()
                if label in files:
                    continue
                size = real.stat().st_size
                if total + size > MAX_NATIVE_BYTES:
                    raise DiagnosticError("Native hash budget exceeded")
                data = read_file(real, MAX_NATIVE_FILE)
                total += len(data)
                files[label] = digest(data)
    return {"files": files, "hashed_bytes": total, "mpv_receipt_sha256": digest(read_file(keg / "INSTALL_RECEIPT.json", 2 * MIB)), "limits": "Selected receipt closure headers, libraries, pkg-config, formula and receipt bytes; not a full SDK or transitive embedded-source inventory."}, sorted(pc_dirs)


def system_pkgconfig(brew: Path, major: str) -> tuple[dict, Path]:
    """Capture the actual Homebrew macOS system .pc policy without running Ruby."""
    if not re.fullmatch(r"[1-9][0-9]", major) or int(major) < 11:
        raise DiagnosticError("Unsupported macOS system pkg-config version")
    homebrew = brew / "Library/Homebrew"
    policy = read_file(homebrew / "extend/os/mac/extend/ENV/super.rb", MIB)
    shim = read_file(homebrew / "shims/mac/super/bin/pkg-config", MIB)
    if (b"#{HOMEBREW_LIBRARY}/Homebrew/os/mac/pkgconfig/#{MacOS.version}" not in policy
            or b"--define-variable=homebrew_sdkroot=${HOMEBREW_SDKROOT}" not in shim):
        raise DiagnosticError("Homebrew system pkg-config policy changed; review required")
    directory = homebrew / "os/mac/pkgconfig" / major
    if directory.is_symlink() or not directory.is_dir():
        raise DiagnosticError("Homebrew system pkg-config directory is unavailable")
    files = {}
    for path in directory.iterdir():
        if len(files) >= 64 or path.suffix != ".pc" or not re.fullmatch(r"[A-Za-z0-9_.+-]+", path.name):
            raise DiagnosticError("Unsupported system pkg-config input")
        files[path.name] = digest(read_file(path, 64 * 1024))
    if not {"zlib.pc", "bzip2.pc"}.issubset(files):
        raise DiagnosticError("Required system compression-library metadata is missing")
    return {"macos_major": major, "files": files, "policy_sha256": digest(policy),
            "shim_sha256": digest(shim), "sdk_override": "homebrew_sdkroot is explicitly set to the captured selected SDK"}, directory


def plans(output: Path, tools: dict[str, Path], recipe: list[str], env: dict[str, str]) -> list[dict]:
    result = []
    for variant, value in (("on", "true"), ("off", "false")):
        build = output / ("build-" + variant)
        prefix = output / ("prefix-" + variant)
        args = ["--sysconfdir=" + str(prefix / "etc") if arg == "--sysconfdir=#{etc}" else arg for arg in recipe]
        args += ["--prefix=" + str(prefix), "--libdir=lib", "--buildtype=release", "--wrap-mode=nofallback", "-Dgpu-pass-timers=" + value]
        result.append({"variant": variant, "cwd": str(output), "environment": env,
                       "commands": [[str(tools["meson"]), "setup", str(build), str(output / "source"), *args],
                                    [str(tools["meson"]), "compile", "-C", str(build), "--jobs=2", "--verbose"],
                                    [str(tools["meson"]), "install", "-C", str(build), "--no-rebuild"]],
                       "required_timeout_seconds": [300, 1800, 120]})
    return result


def source_inventory(root: Path) -> dict:
    result = {}
    total = 0
    for p in sorted(root.rglob("*")):
        if p.is_symlink():
            raise DiagnosticError("Patched source contains a symlink")
        if p.is_file():
            if len(result) >= MAX_ENTRIES:
                raise DiagnosticError("Patched source entry limit exceeded")
            data = read_file(p, MAX_MEMBER)
            total += len(data)
            if total > MAX_EXPANDED:
                raise DiagnosticError("Patched source hash limit exceeded")
            result[p.relative_to(root).as_posix()] = digest(data)
    return result


def prepare(output: Path, inputs: Path | None, allow_download: bool, brew: Path, keg: Path, directories: list[Path]) -> dict:
    if sys.platform != "darwin":
        raise DiagnosticError("This diagnostic plan targets macOS only")
    repository = Path(__file__).resolve().parent.parent
    artifacts = repository / "artifacts"
    if not output.is_absolute():
        output = repository / output
    # Follow no output symlink, including ancestors; never reuse a previous run.
    if not output.is_relative_to(artifacts) or ".." in output.parts:
        raise DiagnosticError("Output must be a fresh directory under repository artifacts")
    for parent in (output, *output.parents):
        if parent.is_symlink():
            raise DiagnosticError("Output path contains a symlink")
    if output.exists() or not output.parent.is_dir():
        raise DiagnosticError("Output already exists or parent is missing")
    formula = read_file(keg / ".brew/mpv.rb", MIB)
    recipe, recipe_evidence = recipe_arguments(formula, read_file(brew / "Library/Homebrew/formula.rb", 2 * MIB))
    tools = tool_paths(directories)
    output.mkdir(mode=0o700)
    (output / "home").mkdir(mode=0o700)
    (output / "tmp").mkdir(mode=0o700)
    (output / "inputs").mkdir(mode=0o700)
    private_write(output / "INCOMPLETE", b"Preparation has not completed. Do not build.\n")
    env = {"HOME": str(output / "home"), "TMPDIR": str(output / "tmp"), "PATH": os.pathsep.join(str(p) for p in directories), "LC_ALL": "C", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CEILING_DIRECTORIES": str(output), "CC": str(tools["clang"]), "CXX": str(tools["clang++"]), "PKG_CONFIG": str(tools["pkgconf"]), "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1", "DENO_NO_UPDATE_CHECK": "1"}
    # /usr/bin compiler shims are not the actual selected Xcode compiler bytes.
    for name in ("clang", "clang++", "swiftc"):
        path = Path(command([str(tools["xcrun"]), "--find", name], env, output).decode().strip())
        if not path.is_absolute() or not path.is_file() or not os.access(path, os.X_OK):
            raise DiagnosticError("Selected SDK compiler is unavailable")
        tools[name] = path
    env["CC"] = str(tools["clang"])
    env["CXX"] = str(tools["clang++"])
    env["SWIFTC"] = str(tools["swiftc"])
    tool_evidence = {}
    for name, path in tools.items():
        version_args = [str(path), "--version"]
        if name == "xcrun":
            version_args = [str(path), "--show-sdk-version"]
        raw = command(version_args, env, output)
        # Public evidence contains hashes, never tool output or absolute home paths.
        tool_evidence[name] = {"invocation_name": path.name, "executable_sha256": digest(read_file(path.resolve(strict=True), MAX_NATIVE_FILE)), "version_output_sha256": digest(raw)}
        private_write(output / ("tool-" + name.replace("+", "p") + ".txt"), raw)
    sdk = Path(command([str(tools["xcrun"]), "--show-sdk-path"], env, output).decode().strip())
    if not sdk.is_absolute() or not sdk.is_dir():
        raise DiagnosticError("SDK discovery failed")
    env["SDKROOT"] = str(sdk)
    sdk_settings = read_file(sdk / "SDKSettings.json", 2 * MIB)
    native, pc_dirs = native_inventory(brew, keg)
    system_pc, system_pc_dir = system_pkgconfig(brew, platform.mac_ver()[0].split(".")[0])
    pc_dirs.append(system_pc_dir)
    env["PKG_CONFIG_LIBDIR"] = os.pathsep.join(str(p) for p in pc_dirs)
    env["PKG_CONFIG_PATH"] = env["PKG_CONFIG_LIBDIR"]
    # Meson splits PKG_CONFIG as an argument list. Match the inspected Homebrew
    # shim's SDK override without invoking its compiler/superenv wrappers.
    env["PKG_CONFIG"] = shlex.join([str(tools["pkgconf"]), "--define-variable=homebrew_sdkroot=" + str(sdk)])
    for name, url, expected, limit in INPUTS:
        data = checked_input(inputs / name if inputs else None, url, expected, limit, allow_download)
        private_write(output / "inputs" / name, data)
    patch = read_file(repository / "docs/experiments/mpv-pass-timers-diagnostic.patch", MAX_PATCH)
    if digest(patch) != PATCH_SHA:
        raise DiagnosticError("Diagnostic patch differs from reviewed bytes")
    private_write(output / "inputs/diagnostic.patch", patch)
    extract_source(read_file(output / "inputs/mpv-0.41.0.tar.gz", MAX_ARCHIVE), output / "source")
    for name in [INPUTS[1][0], INPUTS[2][0], "diagnostic.patch"]:
        patch_path = str(output / "inputs" / name)
        command([str(tools["git"]), "apply", "--check", patch_path], env, output / "source")
        command([str(tools["git"]), "apply", patch_path], env, output / "source")
    commands = plans(output, tools, recipe, env)
    private_write(output / "command-plan.json", (json.dumps(commands, indent=2) + "\n").encode())
    report = {"schema": 1, "status": "prepared_not_built", "recipe": recipe_evidence,
              "inputs": [{"name": name, "url": url, "sha256": expected} for name, url, expected, _ in INPUTS],
              "diagnostic_patch_sha256": PATCH_SHA, "driver_sha256": digest(read_file(Path(__file__), MIB)),
              "tools": tool_evidence, "sdk_settings_sha256": digest(sdk_settings), "native": native,
              "homebrew_system_pkgconfig": system_pc,
              "patched_source_files": source_inventory(output / "source"),
              "command_plan_sha256": digest(read_file(output / "command-plan.json", 2 * MIB)),
              "limitations": ["No compilation, installation, library loading or performance qualification performed.", "Not a reproduction of Homebrew bottle bytes or its unrecorded superenv compiler flags.", "Before/after each future build reverify source/tool/native hashes; compare both Meson dependency/option snapshots. Stop on differences except variant, prefix and gpu-pass-timers.", "command-plan.json and tool outputs contain machine-local paths; retain privately, do not publish raw.", "Future runner must impose the per-command timeouts, kill/reap groups and verify actual loaded libmpv before functional/ABBA runs."]}
    private_write(output / "provenance.json", (json.dumps(report, indent=2) + "\n").encode())
    (output / "INCOMPLETE").unlink()
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--inputs", type=Path, help="Offline directory with the three fixed input filenames")
    parser.add_argument("--download", action="store_true", help="Explicitly permit pinned HTTPS source downloads")
    parser.add_argument("--homebrew-root", type=Path, default=Path("/opt/homebrew"))
    parser.add_argument("--mpv-keg", type=Path, default=Path("/opt/homebrew/Cellar/mpv/0.41.0_10"))
    parser.add_argument("--tool-dir", type=Path, action="append", default=[])
    args = parser.parse_args()
    directories = [*args.tool_dir, args.homebrew_root / "bin", Path("/usr/bin"), Path("/bin")]
    if not all(p.is_absolute() for p in [args.homebrew_root, args.mpv_keg, *directories]):
        parser.error("Tool and Homebrew roots must be absolute")
    try:
        prepare(args.output, args.inputs, args.download, args.homebrew_root, args.mpv_keg, directories)
    except (DiagnosticError, OSError, ValueError, tarfile.TarError, EOFError, subprocess.SubprocessError) as error:
        print(str(error) if isinstance(error, DiagnosticError) else "Diagnostic preparation failed; no build was performed", file=sys.stderr)
        return 1
    print("Prepared verified private source, provenance and command-plan.json. No build or installation was run.")
    return 0


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--internal-fetch":
        try:
            index = int(sys.argv[2])
            if not 0 <= index < len(INPUTS):
                raise ValueError
            _, url, _, limit = INPUTS[index]
            sys.stdout.buffer.write(download_bytes(url, limit))
        except (DiagnosticError, OSError, ValueError):
            raise SystemExit(1)
    else:
        raise SystemExit(main())

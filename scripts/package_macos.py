#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline macOS developer bundle and exact-input dependency inventory.

No downloads, signing identities, publication, helper updates or portable-package
claim. --bundle-helpers includes the reviewed installed Python/yt-dlp/EJS/Deno closure.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import plistlib
import posixpath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import package_helpers

ROOT = Path(__file__).resolve().parents[1]
SYSTEM_PREFIXES = ("/System/Library/", "/usr/lib/")
ID = "org.serein.desktop.development"


class PackagingError(Exception):
    pass


def run(*args: str | Path, cwd: Path = ROOT) -> str:
    result = subprocess.run([str(arg) for arg in args], cwd=cwd, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if result.returncode:
        detail = (": " + str(sanitized(result.stderr[-2000:])).strip()
                  if Path(args[0]).name in ("codesign", "install_name_tool") else "")
        raise PackagingError(f"{Path(args[0]).name} failed (exit {result.returncode}); no package was published{detail}")
    return result.stdout


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def json_write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def sanitized(value: object) -> object:
    """Receipts can contain the installing user's Homebrew cache path."""
    if isinstance(value, str):
        return value.replace(str(Path.home()), "${HOME}")
    if isinstance(value, list):
        return [sanitized(item) for item in value]
    if isinstance(value, dict):
        return {sanitized(key): sanitized(item) for key, item in value.items()}
    return value


def is_system(path: str) -> bool:
    return posixpath.normpath(path).startswith(SYSTEM_PREFIXES)


def parse_otool(text: str) -> list[str]:
    return [line.strip().split(" (compatibility version", 1)[0]
            for line in text.splitlines()[1:] if " (compatibility version" in line]


def load_commands(path: Path) -> tuple[list[str], str]:
    text = run("otool", "-l", path)
    rpaths = re.findall(r"cmd LC_RPATH\n.*?\n\s*path (.*?) \(offset", text)
    versions = re.findall(r"\bminos ([0-9.]+)", text)
    versions += re.findall(r"cmd LC_VERSION_MIN_MACOSX\n.*?\n\s*version ([0-9.]+)", text)
    return rpaths, max(versions, key=version_tuple, default="0.0")


def version_tuple(value: str) -> tuple[int, ...]:
    return tuple(int(part) for part in value.split("."))


def resolve_load(load: str, loader: Path, executable: Path, rpaths: list[str]) -> Path:
    def expand(value: str) -> str:
        return value.replace("@loader_path", str(loader.parent)).replace("@executable_path", str(executable.parent))
    if load.startswith("@rpath/"):
        candidates = [Path(expand(path)) / load.removeprefix("@rpath/") for path in rpaths]
    else:
        candidates = [Path(expand(load))]
    for candidate in candidates:
        if candidate.is_absolute() and candidate.is_file():
            return candidate.resolve(strict=True)
    raise PackagingError("An unresolved non-system Mach-O load prevents bundling")


def native_closure(executable: Path, graph: dict[Path, dict] | None = None) -> dict[Path, dict]:
    pending = [executable.resolve(strict=True)]
    if graph is None:
        graph = {}
    while pending:
        path = pending.pop()
        if path in graph:
            continue
        rpaths, minimum = load_commands(path)
        identities = run("otool", "-D", path).splitlines()[1:]
        own_id = identities[0].strip() if identities else None
        dependencies = []
        for load in parse_otool(run("otool", "-L", path)):
            if load == own_id:
                continue
            if is_system(load):
                dependencies.append({"load": load, "system": True})
            else:
                resolved = resolve_load(load, path, executable, rpaths)
                dependencies.append({"load": load, "system": False, "resolved": str(resolved)})
                pending.append(resolved)
        graph[path] = {"sha256": digest(path), "bytes": path.stat().st_size,
                       "architectures": run("lipo", "-archs", path).strip().split(),
                       "minimum_macos": minimum, "rpaths": rpaths, "dependencies": dependencies,
                       "install_id": own_id}
        if len(graph) > 512:
            raise PackagingError("Native dependency bound exceeded")
    return graph


def keg_for(path: Path) -> Path | None:
    cellar = Path("/opt/homebrew/Cellar")
    resolved = path.resolve()
    if not resolved.is_relative_to(cellar):
        return None
    relative = resolved.relative_to(cellar).parts
    if len(relative) < 2:
        return None
    return cellar / relative[0] / relative[1]


def source_files() -> list[Path]:
    names = run("git", "ls-files", "--cached", "--others", "--exclude-standard", "-z").split("\0")
    roots = {"crates", "scripts", "docs", "third_party"}
    top = {"Cargo.toml", "Cargo.lock", "LICENSE", "README.md", "CONTRIBUTING.md", "SECURITY.md", "CODE_OF_CONDUCT.md", "SPEC.md"}
    return sorted({ROOT / name for name in names if name and (Path(name).parts[0] in roots or name in top
                   or Path(name).parts[:2] == ("tools", "media-baseline"))
                   and (ROOT / name).is_file() and not (ROOT / name).is_symlink()})


def source_fingerprint(files: list[Path]) -> str:
    payload = [(str(path.relative_to(ROOT)), digest(path)) for path in files]
    return hashlib.sha256(json.dumps(payload, separators=(",", ":")).encode()).hexdigest()


def cargo_graph(target: str) -> dict:
    metadata = json.loads(run("cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--filter-platform", target))
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = next(package["id"] for package in packages.values() if package["name"] == "serein")
    # `metadata.resolve` includes inactive optional dependencies. Cargo tree is
    # authoritative for both first-party executables' normal+build graph.
    selected = run("cargo", "tree", "--locked", "--offline", "-p", "serein", "-p", "serein-network", "--target", target,
                   "-e", "normal,build", "--prefix", "depth", "--format", "{p}")
    by_name: dict[tuple[str, str], list[str]] = {}
    for key, package in packages.items():
        by_name.setdefault((package["name"], package["version"]), []).append(key)
    visited, edge_pairs, stack = set(), set(), []
    for line in selected.splitlines():
        match = re.match(r"^(\d+)([^ ]+) v([^ ]+)(?: |$)", line)
        if not match:
            continue
        depth, name, version = int(match[1]), match[2], match[3]
        candidates = by_name.get((name, version), [])
        if len(candidates) != 1:
            raise PackagingError("An ambiguous Cargo tree package requires explicit source disambiguation")
        key = candidates[0]
        if depth > len(stack):
            raise PackagingError("Cargo tree depth could not be interpreted")
        stack = stack[:depth]
        if stack:
            edge_pairs.add((stack[-1], key))
        stack.append(key)
        visited.add(key)
    if root not in visited:
        raise PackagingError("Cargo tree did not contain the selected application")
    edges = []
    for parent, child in sorted(edge_pairs):
        dependency = next(item for item in nodes[parent]["deps"] if item["pkg"] == child)
        kinds = sorted({kind["kind"] or "normal" for kind in dependency["dep_kinds"] if kind["kind"] != "dev"})
        edges.append({"from": parent, "to": child, "kinds": kinds})
    return {"root": root, "packages": [{"id": key, "name": packages[key]["name"], "version": packages[key]["version"],
             "source": packages[key]["source"], "license": packages[key]["license"], "license_file": packages[key]["license_file"],
             "manifest_path": packages[key]["manifest_path"], "features": nodes[key]["features"]} for key in sorted(visited)],
             "dependencies": edges}



def copy_notices(source: Path, destination: Path, explicit: str | None = None) -> list[str]:
    destination.mkdir(parents=True, exist_ok=True)
    candidates = set()
    for pattern in ("LICENSE*", "LICENCE*", "COPYING*", "COPYRIGHT*", "NOTICE*", "GPL*", "LGPL*", "AUTHORS*", "license*", "licence*", "copyright*"):
        candidates.update(source.glob(pattern))
    if explicit:
        candidates.add(source / explicit)
    copied = []
    for index, path in enumerate(sorted(candidates)):
        if path.is_file() and path.stat().st_size <= 4 * 1024 * 1024:
            name = f"{index:03}-{path.name}"
            shutil.copyfile(path, destination / name)
            copied.append(name)
    return copied


def ca_notice_evidence(keg: Path, destination: Path) -> tuple[list[str], dict | None]:
    """Bind original Mozilla source/license evidence to this exact public CA input."""
    cache = ROOT / "third_party/ca-certificates" / keg.name
    source = cache / "manifest.json"
    if not source.exists():
        return [], None  # A newer CA input stays an explicit notice gap until audited.
    if source.is_symlink() or source.stat().st_size > 64 * 1024:
        raise PackagingError("CA notice manifest must be a regular repository file")
    evidence = json.loads(source.read_text())
    bundle = keg / "share/ca-certificates/cacert.pem"
    if (not isinstance(evidence, dict) or type(evidence.get("schema")) is not int
            or evidence["schema"] != 1 or evidence.get("formula") != "ca-certificates"
            or evidence.get("version") != keg.name or evidence.get("declared_license") != "MPL-2.0"
            or bundle.is_symlink() or digest(bundle) != evidence.get("bundle_sha256")):
        raise PackagingError("CA notice evidence does not match the exact bundled certificate input")
    # This is the immutable public Mozilla input, never the Keychain-merged etc file.
    header = bundle.read_bytes().split(b"-----BEGIN CERTIFICATE-----", 1)[0]
    match = re.search(rb"^## SHA256: ([0-9a-f]{64})$", header, re.MULTILINE)
    if match is None or match[1].decode() != evidence.get("certdata_sha256"):
        raise PackagingError("CA bundle does not identify the retained Mozilla source")
    files = evidence.get("files", [])
    if (not isinstance(files, list) or len(files) != 2
            or not all(isinstance(item, dict) and isinstance(item.get("path"), str) for item in files)
            or {item.get("path") for item in files} != {"LICENSE-MPL-2.0", "certdata.txt"}):
        raise PackagingError("CA notice evidence has an unexpected file set")
    destination.mkdir(parents=True, exist_ok=True)
    copied = []
    for item in files:
        path = cache / item["path"]
        if (path.is_symlink() or not path.is_file() or path.stat().st_size > 2 * 1024 * 1024
                or digest(path) != item.get("sha256")
                or (item["path"] == "certdata.txt" and item["sha256"] != evidence["certdata_sha256"])):
            raise PackagingError("CA original source/license evidence failed verification")
        name = "upstream-" + item["path"]
        shutil.copyfile(path, destination / name)
        copied.append(name)
    json_write(destination / "upstream-provenance.json", evidence)
    copied.append("upstream-provenance.json")
    return copied, evidence


def supplemental_notices(package: dict, destination: Path) -> tuple[list[str], dict | None]:
    root = ROOT / "third_party/notices"
    manifest = root / "manifest.json"
    if not manifest.is_file():
        return [], None
    records = json.loads(manifest.read_text())["packages"]
    entry = next((item for item in records if item["name"] == package["name"] and item["version"] == package["version"]), None)
    if entry is None:
        return [], None
    directory = Path(package["manifest_path"]).parent
    vcs_file = directory / ".cargo_vcs_info.json"
    if not vcs_file.is_file() or digest(vcs_file) != entry["cargo_vcs_info_sha256"]:
        raise PackagingError("Cached notice evidence does not match the selected crate's source revision")
    vcs = json.loads(vcs_file.read_text())
    cargo = tomllib.loads(Path(package["manifest_path"]).read_text())["package"]
    repository = cargo.get("repository", "").replace("http://github.com/", "https://github.com/").rstrip("/")
    if (entry["revision"] != vcs.get("git", {}).get("sha1") or entry["repository"] != repository
            or entry["declared_license"] != package["license"]):
        raise PackagingError("Cached notice/source association failed verification")
    copied = []
    for item in entry["files"]:
        name = item["path"]
        if Path(name).name != name or name in (".", ".."):
            raise PackagingError("Unsafe cached notice path")
        source = root / f"{package['name']}-{package['version']}" / name
        if source.is_symlink() or not source.is_file() or digest(source) != item["sha256"]:
            raise PackagingError("Cached original notice failed its integrity check")
        output = "upstream-" + name
        shutil.copyfile(source, destination / output)
        copied.append(output)
    return copied, entry


def inventory_helpers(yt_dlp: Path, deno: Path) -> tuple[dict, set[Path]]:
    inputs, kegs = {}, set()
    for role, path in (("yt-dlp", yt_dlp), ("deno", deno)):
        if not path.is_absolute() or not path.is_file():
            raise PackagingError("An explicitly declared external helper is missing")
        real = path.resolve(strict=True)
        inputs[role] = {"path": str(path), "resolved": str(real), "sha256": digest(real), "bundled": False}
        keg = keg_for(real)
        if keg:
            kegs.add(keg)
    with yt_dlp.resolve().open("rb") as file:
        lines = file.read(4096).splitlines()
    if not lines:
        raise PackagingError("The extractor launcher is empty")
    launcher = lines[0]
    if not launcher.startswith(b"#!/"):
        raise PackagingError("This developer tool expects the inspected Homebrew yt-dlp Python launcher")
    interpreter = Path(launcher[2:].decode())
    if not interpreter.is_file():
        raise PackagingError("The declared extractor interpreter is missing")
    inputs["python"] = {"path": str(interpreter), "resolved": str(interpreter.resolve()), "sha256": digest(interpreter.resolve()), "bundled": False}
    keg = keg_for(interpreter.resolve())
    if keg is None:
        raise PackagingError("Extractor interpreter is outside the explicitly supported Homebrew installation")
    kegs.add(keg)
    # Inspect metadata using the same isolated environment policy as the supervisor.
    # No user-site directories, browser profiles or account/session files are read.
    program = "import importlib.metadata as m,json,sys; print(json.dumps({'paths':sys.path,'packages':[{'name':d.metadata.get('Name'),'version':d.version,'license':d.metadata.get('License-Expression') or d.metadata.get('License'),'location':str(d.locate_file('')),'metadata_path':str(d._path),'files':[str(f) for f in (d.files or []) if not str(f).endswith('.pyc')]} for d in m.distributions()]}))"
    result = subprocess.run([str(interpreter), "-I", "-c", program], cwd="/", env={"PATH": "/usr/bin:/bin", "LANG": "en_US.UTF-8", "PYTHONNOUSERSITE": "1"}, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False, timeout=60)
    if result.returncode:
        raise PackagingError("Installed helper metadata inspection failed")
    python = json.loads(result.stdout)
    unique = {(item["name"], item["version"], str(Path(item["location"]).resolve())): item for item in python["packages"]}
    python["packages"] = sorted(unique.values(), key=lambda item: (item["name"] or "", item["version"]))
    inputs["python_environment"] = python
    for package in python["packages"]:
        keg = keg_for(Path(package["location"]))
        if keg:
            kegs.add(keg)
        file_names = package.pop("files")
        package["file_inventory_method"] = "wheel-record"
        if not file_names and package_helpers.normalized(package["name"]) in package_helpers.RUNTIME_PACKAGES and keg_for(Path(package["location"])) is not None:
            file_names = package_helpers.distribution_files(package, sys.modules[__name__])
            package["file_inventory_method"] = "reviewed-package-roots-record-absent"
        elif not file_names:
            package["file_inventory_method"] = "unavailable-not-a-selected-runtime-package"
        if len(file_names) > 20000:
            raise PackagingError("An external Python package exceeds the inventory bound")
        package_files = []
        for name in sorted(file_names):
            file = (Path(package["location"]) / name).resolve()
            # Do not let package metadata turn this audit into a home-directory scan.
            file_keg = keg_for(file)
            if file_keg is None:
                raise PackagingError("Python package metadata refers outside the explicit Homebrew installation")
            if file.is_file():
                package_files.append({"path": name, "sha256": digest(file), "bytes": file.stat().st_size})
        package["installed_files"] = package_files
        package["installed_tree_sha256"] = hashlib.sha256(json.dumps(package_files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return inputs, kegs


def icon_provenance() -> dict:
    directory = ROOT / "crates/app/ui/icons"
    source = (directory / "SOURCE.md").read_text()
    match = re.search(r"revision \[?`([a-f0-9]{40})`", source)
    if not match:
        raise PackagingError("Vendored icons lack an exact recorded source revision")
    files = []
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        checksum, name = line.split(maxsplit=1)
        name = name.strip().removeprefix("*")
        if not re.fullmatch(r"[a-f0-9]{64}", checksum) or Path(name).name != name or name in (".", ".."):
            raise PackagingError("Malformed vendored icon checksum record")
        file = directory / name
        if file.is_symlink() or not file.is_file() or digest(file) != checksum:
            raise PackagingError("Vendored icon/license integrity check failed")
        files.append({"path": name, "sha256": checksum})
    return {"upstream": "https://github.com/lucide-icons/lucide", "revision": match[1], "licenses": "ISC AND MIT", "files": files}


def write_spdx(path: Path, manifest: dict, cargo: dict, native: dict[Path, dict], helpers: dict, epoch: int) -> None:
    packages, relationships = [], []
    ids = {}
    for index, item in enumerate(cargo["packages"]):
        package_id = f"SPDXRef-Rust-{index}"
        ids[item["id"]] = package_id
        packages.append({"SPDXID": package_id, "name": item["name"], "versionInfo": item["version"], "downloadLocation": item["source"] or "NOASSERTION",
                         "filesAnalyzed": False, "licenseConcluded": "NOASSERTION", "licenseDeclared": item["license"] or "NOASSERTION",
                         "copyrightText": "NOASSERTION", "comment": "Cargo target graph includes build and normal dependencies; not proof that every package is shipped."})
    relationships.append({"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": ids[cargo["root"]]})
    for edge in cargo["dependencies"]:
        relationships.append({"spdxElementId": ids[edge["from"]], "relationshipType": "DEPENDS_ON", "relatedSpdxElement": ids[edge["to"]], "comment": ",".join(edge["kinds"])})
    for index, (source, info) in enumerate(sorted(native.items())):
        package_id = f"SPDXRef-Native-{index}"
        packages.append({"SPDXID": package_id, "name": source.name, "downloadLocation": "NOASSERTION", "filesAnalyzed": False,
                         "licenseConcluded": "NOASSERTION", "licenseDeclared": "GPL-3.0-or-later" if any(item["original_sha256"] == info["sha256"] for item in manifest.get("first_party_helpers", [])) else "NOASSERTION", "copyrightText": "NOASSERTION",
                         "checksums": [{"algorithm": "SHA256", "checksumValue": info["sha256"]}],
                         "comment": "Checksum identifies original pre-relocation Mach-O input. First-party helper source/license is in the application source archive; native dependency formula/SBOM/license evidence is separate. License conclusion remains pending."})
        relationships.append({"spdxElementId": ids[cargo["root"]], "relationshipType": "DEPENDS_ON", "relatedSpdxElement": package_id})
    for index, role in enumerate(("yt-dlp", "deno", "python")):
        item = helpers[role]
        package_id = f"SPDXRef-ExternalHelper-{index}"
        packages.append({"SPDXID": package_id, "name": role, "downloadLocation": "NOASSERTION", "filesAnalyzed": False,
                         "licenseConcluded": "NOASSERTION", "licenseDeclared": "NOASSERTION", "copyrightText": "NOASSERTION",
                         "checksums": [{"algorithm": "SHA256", "checksumValue": item["sha256"]}], "comment": ("Bundled runtime; checksum identifies original installed input (yt-dlp launcher replaced by isolated app launcher)." if item["bundled"] else "Required external development dependency, NOT bundled.") + " Exact installed provenance is in build-manifest.json."})
        relationships.append({"spdxElementId": ids[cargo["root"]], "relationshipType": "DEPENDS_ON", "relatedSpdxElement": package_id})
    for index, item in enumerate(helpers["python_environment"]["packages"]):
        bundled = "bundled_helper_resources" in manifest
        if bundled and (package_helpers.normalized(item["name"]) not in package_helpers.RUNTIME_PACKAGES or not item["installed_files"]):
            continue
        package_id = f"SPDXRef-ExternalPython-{index}"
        packages.append({"SPDXID": package_id, "name": item["name"] or "unnamed-python-distribution", "versionInfo": item["version"], "downloadLocation": "NOASSERTION", "filesAnalyzed": False,
                         "licenseConcluded": "NOASSERTION", "licenseDeclared": "NOASSERTION", "copyrightText": "NOASSERTION",
                         "comment": ("Selected Python runtime distribution bundled from recorded files." if bundled else "Observed external Python environment distribution; not necessarily used and not bundled.") + " Metadata licenses and exact installed-file fingerprints are in build-manifest.json."})
        relationships.append({"spdxElementId": "SPDXRef-ExternalHelper-0", "relationshipType": "DEPENDS_ON", "relatedSpdxElement": package_id})
    icon = manifest["icon_provenance"]
    packages.append({"SPDXID": "SPDXRef-LucideIcons", "name": "Lucide vendored SVG icons", "versionInfo": icon["revision"],
                     "downloadLocation": icon["upstream"] + "/tree/" + icon["revision"], "filesAnalyzed": False,
                     "licenseConcluded": "NOASSERTION", "licenseDeclared": icon["licenses"], "copyrightText": "NOASSERTION",
                     "comment": "Full ISC and retained Feather-derived MIT notices and exact vendored checksums are included."})
    relationships.append({"spdxElementId": ids[cargo["root"]], "relationshipType": "CONTAINS", "relatedSpdxElement": "SPDXRef-LucideIcons"})
    references = {reference for package in packages for reference in re.findall(r"LicenseRef-[A-Za-z0-9.-]+", package["licenseDeclared"])}
    extracted = []
    for reference in sorted(references):
        license_path = next((parent / "LICENSES" / (reference + ".md")
                             for package in cargo["packages"]
                             if "github.com/slint-ui/slint" in (package["source"] or "")
                             for parent in list(Path(package["manifest_path"]).parents)[:6]
                             if (parent / "LICENSES" / (reference + ".md")).is_file()), None)
        if license_path is None:
            raise PackagingError("A custom SPDX license reference lacks inspected license text")
        extracted.append({"licenseId": reference, "extractedText": license_path.read_text(), "comment": "Upstream alternative license text preserved for declared SPDX expression; Serein selects Slint GPL-3.0-only, not this alternative."})
    json_write(path, {"spdxVersion": "SPDX-2.3", "dataLicense": "CC0-1.0", "SPDXID": "SPDXRef-DOCUMENT", "name": "Serein development build inventory",
                     "documentNamespace": f"https://serein.invalid/spdx/{manifest['input_fingerprint']}",
                     "creationInfo": {"created": dt.datetime.fromtimestamp(epoch, dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "creators": ["Tool: serein-package-macos.py"]},
                     "packages": packages, "relationships": relationships, "hasExtractedLicensingInfos": extracted,
                     "comment": "Development inventory. Corresponding source, native resource closure and redistribution review remain incomplete. No platform approval is asserted."})


def build_executables() -> dict[str, Path]:
    """Resolve both first-party executables from one locked Cargo invocation."""
    messages = run("cargo", "build", "--locked", "--release", "-p", "serein",
                   "-p", "serein-network", "--bins", "--message-format=json-render-diagnostics")
    artifacts: dict[str, list[Path]] = {"serein": [], "serein-dns": []}
    for line in messages.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        target = message.get("target", {})
        name = target.get("name")
        if (message.get("reason") == "compiler-artifact" and name in artifacts
                and "bin" in target.get("kind", []) and message.get("executable")):
            artifacts[name].append(Path(message["executable"]))
    if any(len(paths) != 1 for paths in artifacts.values()):
        raise PackagingError("Cargo did not identify both first-party executables exactly once")
    return {name: paths[0] for name, paths in artifacts.items()}


def validate_dns_helper(executable: Path) -> dict:
    """Malformed local input must fail before entering native DNS resolution."""
    with tempfile.TemporaryDirectory(prefix="serein-dns-package-probe-") as directory:
        result = subprocess.run([str(executable)], input=b"", stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=5, check=False,
                                env={"PATH": "/usr/bin:/bin", "HOME": directory, "TMPDIR": directory},
                                cwd=directory)
    if result.returncode != 2 or result.stdout or result.stderr:
        raise PackagingError("Bundled DNS helper did not reject empty input privately")
    return {"probe": "empty stdin EOF rejected before DNS initialization", "exit_code": 2,
            "stdout_bytes": 0, "stderr_bytes": 0, "network_requested": False,
            "cancellation_and_system_dns_qualified": False}


def package(args: argparse.Namespace) -> None:
    if sys.platform != "darwin":
        raise PackagingError("Only the available macOS developer target is implemented")
    output = args.output.absolute()
    if output.exists() or Path(str(output) + ".inventory.json").exists():
        raise PackagingError("Choose a fresh output path; existing files are never overwritten")
    if args.command == "bundle" and output.suffix != ".app":
        raise PackagingError("Bundle output must end in .app")
    output.parent.mkdir(parents=True, exist_ok=True)
    files = source_files()
    fingerprint = source_fingerprint(files)
    if args.build:
        if (args.binary != ROOT / "target/release/serein"
                or args.dns_helper != ROOT / "target/release/serein-dns"):
            raise PackagingError("With --build, executables must be selected from Cargo's build result")
        executables = build_executables()
        args.binary, args.dns_helper = executables["serein"], executables["serein-dns"]
        if fingerprint != source_fingerprint(source_files()):
            raise PackagingError("Sources changed during compilation; retry from a stable tree")
    binary = args.binary.resolve(strict=True)
    dns_helper = args.dns_helper.resolve(strict=True)
    if dns_helper == binary:
        raise PackagingError("Application and DNS helper must be distinct executables")
    native = native_closure(binary)
    native_closure(dns_helper, native)
    if any(native[path]["architectures"] != ["arm64"] for path in (binary, dns_helper)) or any("arm64" not in item["architectures"] for item in native.values()):
        raise PackagingError("Only the available Apple Silicon developer closure has been qualified by this tool")
    arch = run("rustc", "-vV")
    target = "aarch64-apple-darwin"
    cargo = cargo_graph(target)
    helpers, kegs = inventory_helpers(args.yt_dlp, args.deno)
    media_ca, media_ca_hash = package_helpers.mozilla_bundle(sys.modules[__name__])
    kegs.add(keg_for(media_ca))
    helper_resources = None
    if args.bundle_helpers:
        if args.command != "bundle":
            raise PackagingError("--bundle-helpers requires bundle mode")
        helper_resources = package_helpers.helper_plan(helpers, sys.modules[__name__])
        kegs |= {keg for item in helper_resources["files"] if (keg := keg_for(Path(item["source"])))}
        for item in helper_resources["files"]:
            if item["macho"]:
                native_closure(Path(item["source"]), native)
        for role in ("yt-dlp", "deno", "python"):
            helpers[role]["bundled"] = True
        if any("arm64" not in item["architectures"] for item in native.values()):
            raise PackagingError("A bundled helper native input lacks the available Apple Silicon target")
    kegs |= {keg for path in native if (keg := keg_for(path))}
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH") or run("git", "show", "-s", "--format=%ct", "HEAD").strip())
    minimum = max((item["minimum_macos"] for item in native.values()), key=version_tuple)
    stage = Path(tempfile.mkdtemp(prefix=".serein-package-", dir=output.parent))
    try:
        app = stage / output.name
        evidence = app / "Contents/Resources/BuildInfo" if args.command == "bundle" else app
        evidence.mkdir(parents=True)
        manifest = {"schema": 1, "development_only": True, "portable": False, "source_build_performed": args.build,
                    "prebuilt_source_association_verified": args.build, "source_fingerprint": fingerprint,
                    "source_date_epoch": epoch,
                    "macos_version": run("sw_vers", "-productVersion").strip(),
                    "macos_build": run("sw_vers", "-buildVersion").strip(),
                    "git_head": run("git", "rev-parse", "HEAD").strip(), "working_tree_dirty": bool(run("git", "status", "--porcelain")),
                    "cargo_lock_sha256": digest(ROOT / "Cargo.lock"), "target": target, "rustc": arch.strip(),
                    "cargo": run("cargo", "--version").strip(), "minimum_macos_load_commands": minimum,
                    "media_certificate_resource": {"source": str(media_ca), "sha256": media_ca_hash,
                                                   "bundle_path": "Contents/Resources/Certificates/mozilla.pem",
                                                   "policy": "Immutable Mozilla source; host Keychain-merged trust excluded"},
                    "first_party_helpers": [{"name": "serein-dns", "cargo_package": "serein-network",
                                             "bundle_path": "Contents/Helpers/serein-dns",
                                             "source": sanitized(str(dns_helper)),
                                             "original_sha256": native[dns_helper]["sha256"],
                                             "source_build_performed": args.build,
                                             "license": "GPL-3.0-or-later"}],
                    "external_helpers": sanitized(helpers), "native_inputs": sanitized({str(path): info for path, info in native.items()}),
                    "limitations": ["yt-dlp, Python, Python packages and Deno remain external Homebrew dependencies", "Dynamic dlopen/plugin/resource closure not proved by Mach-O load commands", "Ad-hoc signing is not Developer ID signing or notarization", "Complete corresponding-source/notice/redistribution review remains outstanding", "No helper/app automatic updater or trust-root configuration is supplied"]}
        if helper_resources is not None:
            manifest["bundled_helper_resources"] = sanitized(helper_resources)
            manifest["limitations"][0] = "Helper runtime files and native modules are bundled; clean-machine operation and non-library resource closure remain unqualified"
        rust_notices = []
        for index, item in enumerate(cargo["packages"]):
            copied = copy_notices(Path(item["manifest_path"]).parent, evidence / "licenses/rust" / f"{index:04}-{item['name']}-{item['version']}", item["license_file"])
            if not copied and item["source"] is None:
                shutil.copyfile(ROOT / "LICENSE", evidence / "licenses/rust" / f"{index:04}-{item['name']}-{item['version']}" / "LICENSE-GPL-3.0")
                copied = ["LICENSE-GPL-3.0 (workspace license)"]
            if "github.com/slint-ui/slint" in (item["source"] or ""):
                license_directory = next((parent / "LICENSES" for parent in list(Path(item["manifest_path"]).parents)[:6]
                                          if (parent / "LICENSES/GPL-3.0-only.txt").is_file()), None)
                if license_directory:
                    destination = evidence / "licenses/slint-upstream"
                    if not destination.exists():
                        shutil.copytree(license_directory, destination)
                    copied += ["../../slint-upstream (same pinned source repository; GPLv3 route selected for framework)"]
            recovered = None
            if not copied:
                extra, recovered = supplemental_notices(item, evidence / "licenses/rust" / f"{index:04}-{item['name']}-{item['version']}")
                copied += extra
            record = {"name": item["name"], "version": item["version"], "files": copied}
            if recovered is not None:
                record["upstream_evidence"] = recovered
            rust_notices.append(record)
        manifest["rust_notice_inventory"] = rust_notices
        manifest["rust_notice_gaps"] = [item for item in rust_notices if not item["files"]]
        manifest["rust_upstream_review_items"] = [item for item in rust_notices if item.get("upstream_evidence", {}).get("review_required")]
        cargo_public = {**cargo, "packages": [{key: value for key, value in item.items() if key not in {"manifest_path", "license_file"}} for item in cargo["packages"]]}
        json_write(evidence / "cargo-graph.json", sanitized(cargo_public))
        native_evidence = []
        for keg in sorted(kegs):
            destination = evidence / "homebrew" / keg.parent.name / keg.name
            destination.mkdir(parents=True)
            record = {"formula": keg.parent.name, "installed_version": keg.name, "metadata": {}}
            for name in ("INSTALL_RECEIPT.json", "sbom.spdx.json"):
                source = keg / name
                if source.is_file():
                    record["metadata"][name] = {"original_sha256": digest(source), "stored_copy_home_path_redacted": True}
                    json_write(destination / name, sanitized(json.loads(source.read_text())))
            for formula in sorted((keg / ".brew").glob("*.rb")):
                shutil.copyfile(formula, destination / formula.name)
                record["metadata"][formula.name] = {"sha256": digest(formula)}
            header_notice = "include/sqlite3.h" if keg.parent.name == "sqlite" else None
            record["notices"] = copy_notices(keg, destination / "licenses", header_notice)
            if header_notice is not None:
                record["notice_source"] = "Original installed SQLite header includes upstream copyright disclaimer/blessing"
            if keg.parent.name == "ca-certificates":
                extra, original = ca_notice_evidence(keg, destination / "licenses")
                record["notices"].extend(extra)
                if original is not None:
                    record["upstream_evidence"] = original
            native_evidence.append(record)
        manifest["homebrew_provenance"] = native_evidence
        manifest["native_notice_gaps"] = [item["formula"] for item in native_evidence if not item["notices"]]
        # Original application source only; this is NOT all dependency corresponding source.
        with tarfile.open(evidence / "application-source.tar", "w", format=tarfile.PAX_FORMAT) as archive:
            for file in files:
                info = archive.gettarinfo(file, arcname=str(file.relative_to(ROOT)))
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = epoch
                with file.open("rb") as content:
                    archive.addfile(info, content)
        shutil.copyfile(ROOT / "LICENSE", evidence / "LICENSE-GPL-3.0")
        icon_evidence = evidence / "licenses/lucide-icons"
        icon_evidence.mkdir(parents=True)
        for name in ("LICENSE-LUCIDE", "SOURCE.md", "SHA256SUMS"):
            icon_file = ROOT / "crates/app/ui/icons" / name
            if not icon_file.is_file():
                raise PackagingError("Vendored icon license/provenance evidence is missing")
            shutil.copyfile(icon_file, icon_evidence / name)
        manifest["icon_provenance"] = icon_provenance()
        manifest["application_source_archive_sha256"] = digest(evidence / "application-source.tar")
        if fingerprint != source_fingerprint(source_files()):
            raise PackagingError("Sources changed during inventory; retry from a stable tree")
        if args.command == "bundle":
            macos, frameworks = app / "Contents/MacOS", app / "Contents/Frameworks"
            macos.mkdir(parents=True)
            frameworks.mkdir()
            certificate = app / manifest["media_certificate_resource"]["bundle_path"]
            certificate.parent.mkdir(parents=True)
            shutil.copyfile(media_ca, certificate)
            if digest(certificate) != media_ca_hash:
                raise PackagingError("The media certificate resource changed during packaging")
            helper_directory = app / "Contents/Helpers"
            helper_directory.mkdir(parents=True)
            destinations = {binary: macos / "serein", dns_helper: helper_directory / "serein-dns"}
            if helper_resources is not None:
                for item in helper_resources["files"]:
                    source, destination = Path(item["source"]), app / item["target"]
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(source, destination)
                    if digest(destination) != item["sha256"]:
                        raise PackagingError("A helper resource changed during packaging")
                    destination.chmod(0o755 if item["macho"] else 0o644)
                    if item["macho"]:
                        if source in destinations and destinations[source] != destination:
                            raise PackagingError("One native helper input has conflicting destinations")
                        destinations[source] = destination
                launcher = app / "Contents/Helpers/yt-dlp"
                (app / "Contents/Resources/HelperRuntime/yt_dlp_bootstrap.py").write_text(package_helpers.BOOTSTRAP)
                launcher_source = ROOT / "scripts/helper_launcher_macos.c"
                run("xcrun", "clang", "-arch", "arm64", "-Os", "-Wall", "-Wextra", "-Werror",
                    "-mmacosx-version-min=" + minimum, launcher_source, "-o", launcher)
                launcher_graph = native_closure(launcher)
                if len(launcher_graph) != 1:
                    raise PackagingError("The exec-only helper launcher unexpectedly links a non-system library")
                manifest["helper_launcher"] = {"source_sha256": digest(launcher_source), "binary_sha256_before_signing": digest(launcher),
                                               "compiler": run("xcrun", "clang", "--version").strip(), "minimum_macos": minimum}
                run("codesign", "--force", "--sign", "-", "--timestamp=none", launcher)
            names = set()
            for path in sorted(native):
                if path in destinations:
                    continue
                if path.name in names:
                    raise PackagingError("Conflicting native library basenames require explicit resolution")
                names.add(path.name)
                destinations[path] = frameworks / path.name
            for source, destination in destinations.items():
                shutil.copyfile(source, destination)
                if digest(destination) != native[source]["sha256"]:
                    raise PackagingError("A native input changed during packaging")
                destination.chmod(0o755)
                changes = []
                for dependency in native[source]["dependencies"]:
                    if not dependency["system"]:
                        relative = os.path.relpath(destinations[Path(dependency["resolved"])], destination.parent)
                        changes.extend(["-change", dependency["load"], "@loader_path/" + relative])
                for rpath in native[source]["rpaths"]:
                    if rpath.startswith("/") and not is_system(rpath + "/"):
                        changes.extend(["-delete_rpath", rpath])
                if native[source]["install_id"] is not None:
                    changes.extend(["-id", "@rpath/" + destination.name])
                if changes:
                    run("install_name_tool", *changes, destination)
                run("codesign", "--force", "--sign", "-", "--timestamp=none", destination)
            copied = native_closure(macos / "serein")
            native_closure(helper_directory / "serein-dns", copied)
            if helper_resources is not None:
                native_closure(app / "Contents/Helpers/yt-dlp", copied)
                for item in helper_resources["files"]:
                    if item["macho"]:
                        native_closure(app / item["target"], copied)
            if len(copied) != len(native) + int(helper_resources is not None) or any(not path.is_relative_to(app) for path in copied):
                raise PackagingError("Relocated Mach-O closure still requires a non-system external library")
            version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
            with (app / "Contents/Info.plist").open("wb") as file:
                plistlib.dump({"CFBundleExecutable": "serein", "CFBundleIdentifier": ID, "CFBundleName": "Serein Development", "CFBundleDisplayName": "Serein Development", "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version, "CFBundleVersion": version, "LSMinimumSystemVersion": minimum, "NSHighResolutionCapable": True}, file, sort_keys=True)
            (app / "Contents/PkgInfo").write_bytes(b"APPL????")
            manifest["relocated_native_count"] = len(copied)
            manifest["dns_helper_offline_validation"] = validate_dns_helper(helper_directory / "serein-dns")
            if helper_resources is not None:
                manifest["helper_offline_validation"] = package_helpers.validate_helpers(app, helper_resources, sys.modules[__name__])
        manifest["evidence_input_files"] = [{"path": str(file.relative_to(evidence)), "sha256": digest(file)}
                                            for file in sorted(evidence.rglob("*")) if file.is_file()]
        manifest["input_fingerprint"] = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        json_write(evidence / "build-manifest.json", manifest)
        write_spdx(evidence / "sbom.spdx.json", manifest, cargo, native, helpers, epoch)
        helper_note = ("The exact Python/yt-dlp/EJS/Deno runtime and native helper modules are bundled; clean-machine qualification is still required." if helper_resources is not None else "Network operations still require the exact external Homebrew yt-dlp/Python/Deno inputs recorded in build-manifest.json.")
        (evidence / "READ-ME-FIRST.txt").write_text("Serein development evidence; NOT a portable release.\nNative dylibs are relocated only in bundle mode. " + helper_note + "\nAd-hoc signed only; no Developer ID or notarization. Package redistribution and complete corresponding-source obligations remain unaudited.\nInspect docs/packaging.md in application-source.tar. No account credentials are included.\n")
        if args.command == "bundle":
            run("codesign", "--force", "--sign", "-", "--timestamp=none", "--identifier", ID, app)
            run("codesign", "--verify", "--deep", "--strict", app)
        final_files = [{"path": str(file.relative_to(app)), "sha256": digest(file), "bytes": file.stat().st_size} for file in sorted(app.rglob("*")) if file.is_file()]
        final = {"schema": 1, "bundle": output.name, "development_only": True, "input_fingerprint": manifest["input_fingerprint"], "files": final_files}
        app.rename(output)
        with Path(str(output) + ".inventory.json").open("x") as file:
            file.write(json.dumps(final, indent=2, sort_keys=True) + "\n")
        print(f"Created {output.name}: {len(native)} Mach-O inputs; minimum macOS {minimum}; helpers {'bundled' if helper_resources is not None else 'external'}.")
    finally:
        shutil.rmtree(stage, ignore_errors=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("inspect", "bundle"))
    parser.add_argument("--output", type=Path, required=True, help="fresh evidence directory or .app path; never overwritten")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/serein")
    parser.add_argument("--dns-helper", type=Path, default=ROOT / "target/release/serein-dns", help="Exact first-party macOS DNS helper; always bundled")
    parser.add_argument("--yt-dlp", type=Path, default=Path("/opt/homebrew/bin/yt-dlp"))
    parser.add_argument("--deno", type=Path, default=Path("/opt/homebrew/bin/deno"))
    parser.add_argument("--bundle-helpers", action="store_true", help="Include the exact reviewed installed Python/yt-dlp/EJS/Deno runtime")
    parser.add_argument("--build", action="store_true", help="build both first-party binaries with the lockfile; otherwise source association is explicitly unverified")
    try:
        package(parser.parse_args())
    except (PackagingError, OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        print(f"Packaging stopped: {error}", file=sys.stderr)
        raise SystemExit(1) from None


if __name__ == "__main__":
    main()

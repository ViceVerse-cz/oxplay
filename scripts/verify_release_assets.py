"""Verify the final downloaded release payload before promotion."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat
import zipfile


def digest(stream):
    result = hashlib.sha256()
    while block := stream.read(1024 * 1024):
        result.update(block)
    return result.hexdigest()


parser = argparse.ArgumentParser()
parser.add_argument("assets", type=Path)
parser.add_argument("--head", required=True)
parser.add_argument("--receipt", type=Path, required=True)
args = parser.parse_args()
tag = "v0.1.0"
assets = {p.name: p for p in args.assets.iterdir()}
assert all(p.is_file() and not p.is_symlink() for p in assets.values())
fixed = {
    f"oxplay-{tag}-macOS-ARM64.zip", f"oxplay-{tag}-macOS-ARM64.inventory.json",
    f"oxplay-{tag}-Windows-X64.zip", f"oxplay-{tag}-Windows-X64-Setup.exe",
    f"oxplay-{tag}-Linux-X64.AppImage", f"oxplay-{tag}-Linux-X64.AppImage.zsync",
    f"oxplay-{tag}-Linux-X64.tar.gz",
}
patterns = [rf"oxplay-{tag}-Linux-ubuntu-24\.04-.+\.deb",
            rf"oxplay-{tag}-Linux-fedora-44-.+\.rpm",
            rf"oxplay-{tag}-Linux-arch-.+\.pkg\.tar\.zst"]
expected = set(fixed)
for pattern in patterns:
    matches = [name for name in assets if re.fullmatch(pattern, name)]
    assert len(matches) == 1, (pattern, matches)
    expected.update(matches)
assert set(assets) == expected, (set(assets) - expected, expected - set(assets))
assert len(assets) == 10
checksums = {}
for name, path in sorted(assets.items()):
    assert path.stat().st_size > 0
    with path.open("rb") as stream:
        checksums[name] = {"bytes": path.stat().st_size, "sha256": digest(stream)}

with zipfile.ZipFile(assets[f"oxplay-{tag}-macOS-ARM64.zip"]) as archive:
    manifest = json.loads(archive.read("Oxplay.app/Contents/Resources/BuildInfo/build-manifest.json"))
    assert manifest["git_head"] == args.head
    assert manifest["source_build_performed"] is True
    assert manifest["inputs_match_commit"] is True
    assert manifest["working_tree_dirty"] is False
    assert manifest["native_media"]["backend"] == "metal"

with zipfile.ZipFile(assets[f"oxplay-{tag}-Windows-X64.zip"]) as archive:
    files = [p for p in archive.infolist() if not p.is_dir()]
    assert len({p.filename for p in files}) == len(files)
    assert sum(p.file_size for p in files) < 3 * 1024 ** 3
    for member in files:
        path = PurePosixPath(member.filename)
        assert not path.is_absolute() and ".." not in path.parts
        assert "\\" not in member.filename and ":" not in member.filename
        assert not stat.S_ISLNK(member.external_attr >> 16)
    info = json.loads(archive.read("BUILD-INFO.json"))
    assert info["version"] == "0.1.0" and info["rendering"] == "dx12-d3d11"
    assert info["native_media"]["native_render_abi"] == 1
    assert info["native_media"]["platform"] == "windows-d3d11"
    av1 = info["native_media"]["software_codec_checks"]["av1"]
    assert av1["fixture_sha256"] == "7639f033a9c91d1d7b6dbe833f003820ac3b3615de78d49e9db23e912fbb6448"
    assert av1["stdout"] == "Software AV1 decode PASS: decoder=libdav1d frames=8 distinct=8 size=64x64 format=yuv420p"
    assert any("dav1d" in name.lower() for name in info["native_media"]["runtime_dlls_sha256"])
    assert {p.filename for p in files} == set(info["files"]) | {"BUILD-INFO.json"}
    for name, checksum in info["files"].items():
        with archive.open(name) as stream:
            assert digest(stream) == checksum, name
    for name, checksum in info["native_media"]["runtime_dlls_sha256"].items():
        assert info["files"][name] == checksum

receipt = {"schema": 1, "version": "0.1.0", "build_revision": args.head,
           "assets": checksums, "windows_files_verified": len(info["files"]),
           "windows_native_render_abi": 1, "macos_native_backend": "metal",
           "hardware_performance_qualification": False}
args.receipt.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
print(json.dumps(receipt, indent=2, sort_keys=True))

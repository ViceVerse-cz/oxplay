#!/usr/bin/env python3
"""Read Linux package members and authenticated source bundles without extraction."""
from __future__ import annotations
import argparse
from contextlib import contextmanager
import hashlib
import json
import runpy
from pathlib import Path, PurePosixPath
import struct
import subprocess
import tarfile
import tempfile
import threading

MAX_MEMBER = 512 * 1024 * 1024
MAX_METADATA = 4 * 1024 * 1024
MAX_MEMBERS = 100_000
FIXTURE = '7639f033a9c91d1d7b6dbe833f003820ac3b3615de78d49e9db23e912fbb6448'
AV1_PASS = 'Software AV1 decode PASS: decoder=libdav1d frames=8 distinct=8 size=64x64 format=yuv420p'


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def safe_name(name):
    path = PurePosixPath(name)
    require(name and not path.is_absolute() and '..' not in path.parts and '\\' not in name,
            f'Unsafe archive member: {name}')
    return path.as_posix()


def digest(stream, bound=MAX_MEMBER):
    value, total = hashlib.sha256(), 0
    while block := stream.read(1024 * 1024):
        total += len(block)
        require(total <= bound, 'Archive member exceeds its byte bound')
        value.update(block)
    return value.hexdigest()


@contextmanager
def pipe(commands):
    """Read a finite member through archive readers; never execute its contents."""
    processes = []
    expired = threading.Event()
    def stop():
        expired.set()
        for process in processes:
            if process.poll() is None:
                process.kill()
    with tempfile.TemporaryFile() as errors:
        timer = threading.Timer(180, stop)
        timer.start()
        try:
            previous = None
            for command in commands:
                process = subprocess.Popen(command, stdin=previous or subprocess.DEVNULL,
                                           stdout=subprocess.PIPE, stderr=errors)
                processes.append(process)
                if previous:
                    previous.close()
                previous = process.stdout
            yield previous
            previous.close()
            for process in reversed(processes):
                require(process.wait(timeout=10) == 0, 'Archive reader rejected payload')
            require(not expired.is_set(), 'Archive reader exceeded its deadline')
        finally:
            timer.cancel()
            for process in processes:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=10)


def squashfs_offset(path):
    size = path.stat().st_size
    with path.open('rb') as source:
        header = source.read(32 * 1024 * 1024)
    require(header.startswith(b'\x7fELF') and header[8:11] == b'AI\x02', 'Invalid Type 2 AppImage')
    offsets, position = [], 0
    while (position := header.find(b'hsqs', position)) >= 0:
        if position + 96 <= len(header):
            major, minor = struct.unpack_from('<HH', header, position + 28)
            used = struct.unpack_from('<Q', header, position + 40)[0]
            if (major, minor) == (4, 0) and 96 <= used <= size - position:
                offsets.append(position)
        position += 4
    require(len(offsets) == 1, 'Expected one bounded SquashFS filesystem in AppImage')
    return offsets[0]


class Archive:
    def __init__(self, path):
        self.path = path
        self.appimage = path.name.endswith('.AppImage')
        self.deb = path.suffix == '.deb'
        self.outer = []
        if self.appimage:
            self.offset = squashfs_offset(path)
            # A known package-relative path; no payload is run or extracted.
            self.members = None
            self.doc = 'usr/share/doc/oxplay/oxplay-native/'
            return
        if self.deb:
            names = self._listing([['bsdtar', '-tf', str(path)]])
            data = [name for name in names if name.startswith('data.tar.')]
            require(len(data) == 1, 'Debian package lacks one data archive')
            self.outer = [['bsdtar', '-xOf', str(path), data[0]]]
        self.members = self._listing(self.outer + [['bsdtar', '-tf', '-' if self.deb else str(path)]])
        matches = [name for name in self.members if name.endswith('/oxplay-native/native-media-provenance.json')]
        require(len(matches) == 1, 'Package lacks one native media provenance receipt')
        self.doc = matches[0].removesuffix('native-media-provenance.json')

    def _listing(self, commands):
        with pipe(commands) as stream:
            data = stream.read(MAX_METADATA + 1)
            require(len(data) <= MAX_METADATA, 'Package file list exceeds metadata bound')
        names = data.decode('utf-8').splitlines()
        require(len(names) <= MAX_MEMBERS, 'Package has too many members')
        canonical = [safe_name(name) for name in names]
        require(len(set(canonical)) == len(names), 'Duplicate package member')
        return names

    @contextmanager
    def stream(self, name):
        safe_name(name)
        if self.appimage:
            commands = [['unsquashfs', '-o', str(self.offset), '-cat', str(self.path), name]]
        else:
            require(name in self.members, f'Missing package member: {name}')
            commands = self.outer + [['bsdtar', '-xOf', '-' if self.deb else str(self.path), name]]
        with pipe(commands) as stream:
            yield stream

    def read(self, name):
        with self.stream(name) as stream:
            data = stream.read(MAX_METADATA + 1)
            require(len(data) <= MAX_METADATA, 'Package metadata exceeds bound')
        return data

    def json(self, name):
        return json.loads(self.read(self.doc + name))

    def checksum(self, name):
        with self.stream(name) as stream:
            return digest(stream)

    def checksums(self, wanted):
        # One streamed pass avoids repeatedly inflating a portable source closure.
        if self.appimage:
            return {name: self.checksum(name) for name in wanted}
        canonical = {safe_name(name): name for name in wanted}
        require(len(canonical) == len(wanted), 'Duplicate normalized verification member')
        commands = self.outer + [['bsdtar', '-cf', '-', '@-' if self.deb else '@' + str(self.path)]]
        values, total = {}, 0
        with pipe(commands) as stream, tarfile.open(fileobj=stream, mode='r|') as tar:
            for index, member in enumerate(tar):
                require(index < MAX_MEMBERS, 'Too many streamed package members')
                name = safe_name(member.name)
                total += member.size
                require(0 <= member.size <= MAX_MEMBER and total <= 3 * 1024**3, 'Package exceeds its byte bounds')
                if name in canonical:
                    require(member.isreg() and name not in values, 'Duplicate or nonregular verified member')
                    values[name] = digest(tar.extractfile(member))
            while stream.read(1024 * 1024):
                pass
        require(canonical.keys() <= values.keys(), 'Package omits a corresponding source/library member')
        return {original: values[name] for name, original in canonical.items()}


def verify_package(archive, source):
    record, sources = archive.json('native-media-provenance.json'), archive.json('sources.json')
    pins = runpy.run_path(str(source / 'scripts/native-media/platform_build.py'))
    for name, fields in {'mpv': {'revision': pins['MPV_REVISION'], 'sha256': pins['MPV_SHA256']}, 'libplacebo': {'version': pins['PLACEBO_VERSION'], 'sha256': pins['PLACEBO_SHA256']}, 'ffmpeg': {'version': pins['FFMPEG_VERSION'], 'sha256': pins['FFMPEG_SHA256']}}.items():
        require(all(record[name].get(key) == value for key, value in fields.items()), 'Native source pins differ from validated application source')
    require(record['auxiliary_sources'] == {name: {'repository': repository, 'revision': revision, 'sha256': checksum} for name, (repository, revision, checksum) in pins['AUXILIARY'].items()}, 'Auxiliary source pins differ')
    require(record.get('native_render_abi') == sources.get('native_render_abi') == 1 and
            record.get('platform') == sources.get('platform') == 'linux-vulkan', 'Missing native Vulkan ABI 1')
    require(b'#define OXPLAY_NATIVE_RENDER_ABI 1' in archive.read(archive.doc + 'render_vk.h'), 'Missing installed native ABI header')
    av1 = record['software_codec_checks']['av1']
    fixture_metadata = json.loads((source / 'scripts/native-media/fixtures/moving-av1-64x64.json').read_text())
    require(av1.get('fixture_sha256') == FIXTURE and av1.get('stdout') == AV1_PASS and av1.get('result') == fixture_metadata['expected'], 'Missing real software AV1 moving-content proof')
    require(archive.json('software-av1-decode.json') == av1, 'Software decoder receipt differs')
    bundle = archive.doc + safe_name(sources['bundle'])
    require(sources['bundle'] == 'sources.tar.gz' and archive.checksum(bundle) == sources['bundle_sha256'] == record['source_bundle_sha256'], 'Native source bundle SHA differs')
    require([item['file'] for item in record['patches']] == ['common-mpv-gpu-next.patch', 'linux-vulkan-interop.patch'], 'Unexpected native patch set')
    expected = {'upstream/' + safe_name(item['file']): item['sha256'] for item in sources['upstream_archives']}
    require(len(expected) == len(sources['upstream_archives']) and len(expected) == 7, 'Incomplete pinned native source archives')
    pinned = {record[name]['sha256'] for name in ('mpv', 'libplacebo', 'ffmpeg')}
    pinned.update(item['sha256'] for item in record['auxiliary_sources'].values())
    require(set(expected.values()) == pinned and len(pinned) == 7, 'Pinned upstream source inventory differs')
    require(sources['patches'] == record['patches'], 'Native patch receipts differ')
    for item in record['patches']:
        path = source / 'scripts/native-media/patches' / safe_name(item['file'])
        require(hashlib.sha256(path.read_bytes()).hexdigest() == item['sha256'], 'Packaged native patch differs from validated application source')
        expected['patches/' + item['file']] = item['sha256']
    recipes = {name: source / 'scripts/native-media' / name for name in ('platform_build.py', 'linux.py', 'windows.py', 'check_abi.c', 'check_software_av1.c')}
    recipes['install-build-deps-linux.sh'] = source / 'packaging/linux/install-build-deps.sh'
    recipes.update({'fixtures/' + path.name: path for path in (source / 'scripts/native-media/fixtures').iterdir() if path.is_file()})
    expected.update({'build/' + name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in recipes.items()})
    total, found = 0, set()
    with archive.stream(bundle) as stream, tarfile.open(fileobj=stream, mode='r|gz') as tar:
        for index, member in enumerate(tar):
            require(index < MAX_MEMBERS, 'Source bundle has too many members')
            name = safe_name(member.name)
            require(name not in found, 'Duplicate source bundle member')
            found.add(name)
            require(member.isdir() or member.isreg(), 'Unexpected native source bundle member type')
            total += member.size
            require(0 <= member.size <= MAX_MEMBER and total <= MAX_MEMBER, 'Native source bundle exceeds bound')
            if name in expected:
                require(member.isreg() and digest(tar.extractfile(member)) == expected[name], f'Authenticated native source differs: {name}')
        # Drain the reader to complete its checksum/decompression, including tar padding.
        while stream.read(1024 * 1024):
            pass
    require(expected.keys() <= found, 'Native source bundle omits authenticated sources or patches')
    runtime_files, payload_checksums = 0, {}
    for entry in sources['runtime_sources']:
        require(entry['files'], 'Empty corresponding runtime source record')
        for item in entry['files']:
            name = archive.doc + safe_name(item['path'])
            require(name not in payload_checksums, 'Duplicate runtime source member')
            payload_checksums[name] = item['sha256']
            runtime_files += 1
    portable = archive.appimage or archive.path.name.endswith('-Linux-X64.tar.gz')
    if portable:
        require(runtime_files > 0 and sources.get('runtime_source_verification'), 'Portable closure lacks corresponding distribution sources')
    else:
        libraries = archive.json('payload-libraries.json')
        require('libmpv.so.2' in libraries and any(name.startswith('libplacebo.so.') for name in libraries), 'Package lacks private native media libraries')
        for name, checksum in libraries.items():
            safe_name(name)
            matches = [member for member in archive.members if member.endswith('/usr/lib/oxplay/' + name) or member in {'usr/lib/oxplay/' + name, './usr/lib/oxplay/' + name}]
            require(len(matches) == 1, 'Missing private native library payload')
            payload_checksums[matches[0]] = checksum
    require(archive.checksums(payload_checksums) == payload_checksums, 'Corresponding source or private library payload SHA differs')
    return {'native_render_abi': 1, 'platform': 'linux-vulkan', 'software_av1': av1,
            'source_bundle_sha256': sources['bundle_sha256'], 'authenticated_native_members': len(expected),
            'distribution_source_files_verified': runtime_files}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('assets', type=Path)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--receipt', required=True, type=Path)
    args = parser.parse_args()
    candidates = sorted(path for path in args.assets.iterdir() if path.name.endswith(('.deb', '.rpm', '.pkg.tar.zst', '-Linux-X64.tar.gz', '.AppImage')))
    require(len(candidates) == 5, 'Expected five Linux binary package payloads')
    result = {path.name: verify_package(Archive(path), args.source) for path in candidates}
    args.receipt.write_text(json.dumps({'schema': 1, 'packages': result, 'payload_execution': False, 'payload_extraction': False}, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()

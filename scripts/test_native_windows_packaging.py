#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check native DLL/source provenance and exact installer cleanup offline."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import zipfile

from test_release_packaging import helpers_directory, windows


class NativeWindowsPayload(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.helpers = helpers_directory(self.base, windows_names=True)
        self.prefix = self.base / 'native'
        self.prefix.mkdir()
        pe = (self.helpers / 'deno.exe').read_bytes()
        self.binary = self.base / 'oxplay.exe'
        self.binary.write_bytes(pe)
        inventory = {}
        for name in ('libmpv-2.dll', 'libplacebo-360.dll', 'avcodec-63.dll'):
            (self.prefix / name).write_bytes(pe)
            inventory[name] = hashlib.sha256(pe).hexdigest()
        header = self.prefix / 'include/mpv/render_d3d11.h'
        header.parent.mkdir(parents=True)
        header.write_text('#define OXPLAY_NATIVE_RENDER_ABI 1\n')
        self.sources = self.prefix / 'share/oxplay-native'
        self.sources.mkdir(parents=True)
        (self.sources / 'sources.tar.gz').write_bytes(b'synthetic corresponding-source archive')
        self.evidence = {'native_render_abi': 1, 'platform': 'windows-d3d11',
                         'runtime_dlls_sha256': inventory,
                         'source_bundle': 'share/oxplay-native/sources.tar.gz',
                         'source_bundle_sha256': windows.sha256(self.sources / 'sources.tar.gz')}
        self.source_records = {'schema': 1, 'native_render_abi': 1, 'platform': 'windows-d3d11',
                               'bundle': 'sources.tar.gz', 'bundle_sha256': self.evidence['source_bundle_sha256'],
                               'runtime_sources': []}
        (self.sources / 'sources.json').write_text(json.dumps(self.source_records))
        self.write_manifest()

    def write_manifest(self):
        (self.prefix / 'native-media-provenance.json').write_text(json.dumps(self.evidence))

    def stage(self):
        return windows.stage(self.binary, self.prefix, self.helpers, '0.1.0', self.base / 'dist', native=True)

    def test_native_payload_and_cleanup_follow_the_verified_closure(self):
        staged = self.stage()
        info = json.loads((staged / 'BUILD-INFO.json').read_text())
        self.assertEqual(info['rendering'], 'dx12-d3d11')
        self.assertIn('native-media/sources.tar.gz', info['files'])
        self.assertNotIn('shinchiro', (staged / 'THIRD-PARTY-NOTICES.txt').read_text())
        include = self.base / 'uninstall.nsh'
        windows.installer_include(staged, include)
        text = include.read_text()
        for path in staged.rglob('*'):
            if path.is_file():
                name = path.relative_to(staged).as_posix().replace('/', '\\')
                self.assertIn(f'Delete "$INSTDIR\\{name}"', text)
        self.assertNotIn('RMDir /r', text)
        self.assertLess(text.index('Delete "$INSTDIR\\native-media\\sources.tar.gz"'),
                        text.index('RMDir "$INSTDIR\\native-media"'))

    def test_tampered_dll_is_rejected_before_staging(self):
        (self.prefix / 'avcodec-63.dll').write_bytes(b'tampered')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            self.stage()
        self.assertFalse((self.base / 'dist').exists())

    def test_epoch_source_archive_is_preserved_in_zip_and_exact_cleanup(self):
        archive = self.sources / 'runtime-sources/mingw-w64-spirv-cross-1~1.4.357.0-1.src.tar.zst'
        archive.parent.mkdir()
        archive.write_bytes(b'epoch-versioned corresponding source')
        self.source_records['runtime_sources'] = [{
            'source_archive': archive.relative_to(self.prefix).as_posix(),
            'source_archive_sha256': windows.sha256(archive)}]
        (self.sources / 'sources.json').write_text(json.dumps(self.source_records))
        staged = self.stage()
        relative = 'native-media/runtime-sources/' + archive.name
        include = self.base / 'uninstall.nsh'
        windows.installer_include(staged, include)
        name = relative.replace('/', '\\')
        self.assertIn(f'Delete "$INSTDIR\\{name}"', include.read_text())
        with zipfile.ZipFile(windows.archive(staged, self.base / 'payload.zip')) as bundle:
            self.assertEqual(bundle.read(relative), archive.read_bytes())

    def test_installer_rejects_nsis_interpolation_and_quoting(self):
        staged = self.stage()
        include = self.base / 'uninstall.nsh'
        for name in ('$WINDIR.txt', 'quote".txt', 'control\n.txt'):
            with self.subTest(name=name):
                path = staged / name
                # Windows cannot create quote/control filenames. Feed the same
                # invalid inventory to the generator on every test platform.
                with mock.patch.object(Path, 'rglob', return_value=[path]), \
                        mock.patch.object(Path, 'is_file', return_value=True), \
                        mock.patch.object(Path, 'is_dir', return_value=False):
                    with self.assertRaisesRegex(ValueError, 'unsupported path'):
                        windows.installer_include(staged, include)
                self.assertFalse(include.exists())

    def test_stock_or_incomplete_media_is_rejected(self):
        self.evidence['native_render_abi'] = 0
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'required Windows ABI'):
            self.stage()
        self.evidence['native_render_abi'] = 1
        self.evidence['runtime_dlls_sha256'].pop('libplacebo-360.dll')
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'lacks libmpv or libplacebo'):
            self.stage()

    def test_windows_filename_collision_is_rejected(self):
        # This also runs on case-insensitive APFS: both entries can name the same file.
        self.evidence['runtime_dlls_sha256']['LIBMPV-2.dll'] = self.evidence['runtime_dlls_sha256']['libmpv-2.dll']
        (self.prefix / 'LIBMPV-2.dll').write_bytes(self.binary.read_bytes())
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'colliding Windows filenames'):
            self.stage()

    def test_changed_source_archive_is_rejected(self):
        (self.sources / 'sources.tar.gz').write_bytes(b'tampered source')
        with self.assertRaisesRegex(ValueError, 'source archive checksum mismatch'):
            self.stage()

    def test_source_symlink_is_rejected(self):
        try:
            (self.sources / 'foreign').symlink_to(self.binary)
        except OSError:
            self.skipTest('symlinks unavailable')
        with self.assertRaisesRegex(ValueError, 'contains a symlink'):
            self.stage()

    def test_runtime_source_hash_is_checked_before_staging(self):
        archive = self.sources / 'runtime-sources/dependency.src.tar.zst'
        archive.parent.mkdir()
        archive.write_bytes(b'original source')
        self.source_records['runtime_sources'] = [{
            'source_archive': archive.relative_to(self.prefix).as_posix(),
            'source_archive_sha256': windows.sha256(archive)}]
        (self.sources / 'sources.json').write_text(json.dumps(self.source_records))
        archive.write_bytes(b'changed source')
        with self.assertRaisesRegex(ValueError, 'runtime source archive checksum mismatch'):
            self.stage()


if __name__ == '__main__':
    unittest.main()

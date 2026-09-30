# SPDX-License-Identifier: GPL-3.0-or-later
"""Offline closure for the explicitly inspected Homebrew Python helper build."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

# The installed yt-dlp venv's packages and its reviewed cross-keg dependencies.
# Deliberately exclude unrelated global pip/wheel/VapourSynth installations.
RUNTIME_PACKAGES = frozenset({
    'brotli', 'certifi', 'cffi', 'charset-normalizer', 'curl-cffi', 'idna',
    'mutagen', 'pycparser', 'pycryptodomex', 'requests', 'urllib3', 'websockets',
    'yt-dlp', 'yt-dlp-ejs',
})
PACKAGE_ROOTS = {
    'brotli': ('brotli.py', '_brotli.cpython-314-darwin.so'),
    'certifi': ('certifi',), 'cffi': ('cffi', '_cffi_backend.cpython-314-darwin.so'),
    'charset-normalizer': ('charset_normalizer',), 'curl-cffi': ('curl_cffi',),
    'idna': ('idna',), 'mutagen': ('mutagen',), 'pycparser': ('pycparser',),
    'pycryptodomex': ('Cryptodome',), 'requests': ('requests',), 'urllib3': ('urllib3',),
    'websockets': ('websockets',), 'yt-dlp': ('yt_dlp',), 'yt-dlp-ejs': ('yt_dlp_ejs',),
}
MACHO_MAGICS = {bytes.fromhex(value) for value in (
    'feedface', 'cefaedfe', 'feedfacf', 'cffaedfe', 'cafebabe', 'bebafeca',
    'cafebabf', 'bfbafeca',
)}
BOOTSTRAP = '''# SPDX-License-Identifier: GPL-3.0-or-later
import pathlib
import sys
# -I -S excludes environment, user site, site initialization and all .pth execution.
root = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(root / "Python" / "packages"))
if sys.argv[1:] == ["--serein-runtime-audit"]:
    import importlib
    import importlib.metadata
    import json
    names = ("ssl", "ctypes", "sqlite3", "bz2", "lzma", "zlib", "hashlib",
             "yt_dlp", "yt_dlp_ejs", "requests", "curl_cffi", "Cryptodome.Cipher.AES",
             "brotli", "cffi", "websockets")
    modules = {name: getattr(importlib.import_module(name), "__file__", None) for name in names}
    import certifi
    import ssl
    print(json.dumps({"python": sys.version, "prefix": sys.prefix,
                      "paths": sys.path, "modules": modules,
                      "certificate_file": certifi.where(),
                      "certificate_count": len(ssl.create_default_context(cafile=certifi.where()).get_ca_certs()),
                      "packages": {d.metadata["Name"]: d.version for d in importlib.metadata.distributions()}}))
else:
    from yt_dlp import main
    sys.argv[0] = "yt-dlp"
    sys.exit(main())
'''


def normalized(name: str) -> str:
    return re.sub(r'[-_.]+', '-', name).lower()


def is_macho(path: Path) -> bool:
    with path.open('rb') as file:
        return file.read(4) in MACHO_MAGICS


def distribution_files(package: dict, api) -> list[str]:
    """Homebrew strips wheel RECORD; enumerate explicitly reviewed package roots."""
    location = Path(package['location'])
    roots = [location / name for name in PACKAGE_ROOTS[normalized(package['name'])]]
    roots.append(Path(package['metadata_path']))
    files = []
    for source in roots:
        real = source.resolve(strict=True)
        if api.keg_for(real) is None:
            raise api.PackagingError('Python package root leaves the reviewed Homebrew installation')
        for file in sorted(real.rglob('*')) if real.is_dir() else [real]:
            if file.is_dir():
                continue
            relative = file.relative_to(real) if real.is_dir() else Path()
            if '__pycache__' in relative.parts or file.suffix in ('.pyc', '.pth'):
                continue
            if file.is_symlink():
                if normalized(package['name']) == 'certifi' and relative == Path('cacert.pem'):
                    # Homebrew replaces this with host Keychain-merged trust.
                    # Do not inspect or ship it; use immutable public CA input.
                    continue
                raise api.PackagingError('A Python package resource symlink requires explicit review')
            files.append(str(source.relative_to(location) / relative))
    return files


def mozilla_bundle(api) -> tuple[Path, str]:
    source = Path('/opt/homebrew/opt/ca-certificates/share/ca-certificates/cacert.pem').resolve(strict=True)
    keg = api.keg_for(source)
    if keg is None:
        raise api.PackagingError('The immutable Mozilla CA bundle is unavailable')
    formula = (keg / '.brew/ca-certificates.rb').read_text()
    checksum = re.search(r'^  sha256 "([a-f0-9]{64})"$', formula, re.MULTILINE)
    if checksum is None or api.digest(source) != checksum[1]:
        raise api.PackagingError('Mozilla CA input does not match the installed source formula')
    return source, checksum[1]


def validate_runtime_paths(audit: dict, bundle: Path, api) -> None:
    for value in [audit['prefix'], audit['certificate_file'], *audit['paths'], *audit['modules'].values()]:
        if value is not None and not Path(value).resolve().is_relative_to(bundle.resolve()):
            raise api.PackagingError('The bundled Python runtime imports or trusts an external resource')


def validate_helpers(bundle: Path, plan: dict, api) -> dict:
    """Execute offline probes before publishing; fail closed on host fallback."""
    results = {}
    with tempfile.TemporaryDirectory(prefix='serein-helper-audit-') as temporary:
        environment = {'PATH': '/usr/bin:/bin', 'LANG': 'en_US.UTF-8',
                       'HOME': temporary, 'DENO_DIR': str(Path(temporary) / 'deno'),
                       'DENO_NO_UPDATE_CHECK': '1', 'DYLD_PRINT_LIBRARIES': '1',
                       'PYTHONHOME': '/nonexistent/serein-untrusted-python',
                       'PYTHONPATH': '/nonexistent/serein-untrusted-packages'}
        commands = {
            'python': [str(bundle / 'Contents/Helpers/yt-dlp'), '--serein-runtime-audit'],
            'yt_dlp_version': [str(bundle / 'Contents/Helpers/yt-dlp'), '--ignore-config', '--no-config-locations', '--no-plugin-dirs', '--version'],
            'deno_version': [str(bundle / 'Contents/Helpers/deno'), '--version'],
            'deno_script': [str(bundle / 'Contents/Helpers/deno'), 'run', '--ext=js', '--no-prompt', '--no-config', '--no-remote', '--no-npm', '--no-lock', '--no-code-cache', '-'],
        }
        for name, command in commands.items():
            program = "console.log('serein-offline-runtime-check')" if name == 'deno_script' else None
            result = subprocess.run(command, input=program, cwd='/', env=environment, capture_output=True, text=True, timeout=45)
            if result.returncode or len(result.stdout) > 128 * 1024 or len(result.stderr) > 2 * 1024 * 1024:
                raise api.PackagingError('A bundled helper failed its isolated offline runtime probe: ' + name)
            loaded = sorted(set(re.findall(r'dyld\[\d+\]: <[^>]+> (/.+)', result.stderr)))
            external = [path for path in loaded
                        if not api.is_system(path) and not Path(path).resolve().is_relative_to(bundle.resolve())]
            if not loaded or external:
                raise api.PackagingError('A bundled helper loaded an external non-system native library: '
                                         + name + ' -> ' + (', '.join(external) or 'no dyld output'))
            if name == 'python':
                audit = json.loads(result.stdout)
                validate_runtime_paths(audit, bundle, api)
                expected = {normalized(item['name']): item['version'] for item in plan['selected_packages']}
                if {normalized(k): v for k, v in audit['packages'].items()} != expected or audit['certificate_count'] <= 0:
                    raise api.PackagingError('Bundled Python dependencies or public certificate resources differ from the inventory')
                results[name] = audit
            else:
                results[name] = result.stdout.strip()
            results[name + '_native_paths'] = loaded
    # Stage paths are nondeterministic and must not enter reproducible evidence.
    return json.loads(json.dumps(results).replace(str(bundle), '${BUNDLE}'))


def helper_plan(inputs: dict, api) -> dict:
    """Return exact source->bundle-relative copies; no mutations or downloads."""
    python = Path(inputs['python']['resolved'])
    prefix = python.parent.parent
    stdlib = prefix / 'lib/python3.14'
    if not stdlib.is_dir() or not (stdlib / 'encodings/__init__.py').is_file():
        raise api.PackagingError('Only the inspected Python 3.14 framework layout is supported')
    records: dict[str, dict] = {}

    def add(source: Path, target: str, expected: str | None = None) -> None:
        real = source.resolve(strict=True)
        if api.keg_for(real) is None or not real.is_file():
            raise api.PackagingError('Helper input leaves the explicit Homebrew installation')
        checksum = api.digest(real)
        if expected is not None and checksum != expected:
            raise api.PackagingError('An inventoried helper input changed')
        record = {'source': str(real), 'target': target, 'sha256': checksum,
                  'bytes': real.stat().st_size, 'macho': is_macho(real)}
        previous = records.get(target)
        if previous and previous['sha256'] != checksum:
            raise api.PackagingError('Conflicting helper package file paths')
        records[target] = record
        if len(records) > 25000:
            raise api.PackagingError('Helper resource bound exceeded')

    # Framework bin/python3.14 is a posix_spawn trampoline to Python.app.
    # Bundle the actual interpreter, not a trampoline with that fixed layout.
    interpreter = prefix / 'Resources/Python.app/Contents/MacOS/Python'
    if not interpreter.is_file():
        raise api.PackagingError('The real Python framework interpreter is missing')
    add(interpreter, 'Contents/Resources/HelperRuntime/Python/bin/python3.14')
    add(Path(inputs['deno']['resolved']), 'Contents/Helpers/deno', inputs['deno']['sha256'])
    # Include runtime stdlib and all dynamic modules. Development linking files,
    # generated bytecode and external site-packages are not runtime inputs.
    for file in sorted(stdlib.rglob('*')):
        relative = file.relative_to(stdlib)
        if any(part == '__pycache__' or part == 'site-packages' or part.startswith('config-3.14-') for part in relative.parts):
            continue
        if file.is_symlink():
            raise api.PackagingError('Unexpected stdlib symlink requires explicit resource review')
        if file.is_file():
            add(file, 'Contents/Resources/HelperRuntime/Python/lib/python3.14/' + str(relative))
    selected = {}
    for package in inputs['python_environment']['packages']:
        name = normalized(package['name'])
        if name not in RUNTIME_PACKAGES:
            continue
        if not package['installed_files']:
            continue
        if name in selected:
            if selected[name]['version'] != package['version']:
                raise api.PackagingError('Conflicting installed Python dependency versions')
            continue
        selected[name] = package
    if selected.keys() != RUNTIME_PACKAGES:
        raise api.PackagingError('Reviewed Python helper dependency set is incomplete; missing: '
                                 + ', '.join(sorted(RUNTIME_PACKAGES - selected.keys())))
    for package in selected.values():
        if not package['installed_files']:
            raise api.PackagingError('A selected Python package has no inventoried runtime files')
        for item in package['installed_files']:
            relative = Path(item['path'])
            # Distribution console scripts are intentionally replaced by one
            # app-owned isolated launcher; .pth files must never be executed.
            if relative.is_absolute() or '..' in relative.parts:
                if relative.parts[:3] == ('..', '..', '..') and len(relative.parts) > 3 and relative.parts[3] == 'bin':
                    continue
                raise api.PackagingError('Python distribution record has an unreviewed external resource')
            if relative.suffix in ('.pyc', '.pth'):
                continue
            add(Path(package['location']) / relative,
                'Contents/Resources/HelperRuntime/Python/packages/' + str(relative), item['sha256'])
    ca_bundle, ca_hash = mozilla_bundle(api)
    add(ca_bundle, 'Contents/Resources/HelperRuntime/Python/packages/certifi/cacert.pem', ca_hash)
    return {'files': sorted(records.values(), key=lambda item: item['target']),
            'selected_packages': [{'name': p['name'], 'version': p['version'], 'installed_tree_sha256': p['installed_tree_sha256']} for p in selected.values()],
            'excluded_global_packages': sorted({p['name'] for p in inputs['python_environment']['packages'] if normalized(p['name']) not in RUNTIME_PACKAGES}),
            'stdlib_source': str(stdlib),
            'certificate_trust': {'policy': 'Mozilla source bundle only; host Keychain-merged Homebrew trust deliberately excluded',
                                  'source': str(ca_bundle), 'sha256': ca_hash},
            'bootstrap_sha256': hashlib.sha256(BOOTSTRAP.encode()).hexdigest(),
            'python_options': ['-I', '-S', '-B']}

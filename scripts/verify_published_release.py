"""Verify the public release against the exact local upload payload."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('assets', type=Path)
parser.add_argument('--build', required=True)
parser.add_argument('--revision', required=True)
parser.add_argument('--receipt', type=Path, required=True)
args = parser.parse_args()
repo = 'ViceVerse-cz/oxplay'
tag = 'v0.1.0'
assert re.fullmatch(r'[0-9a-f]{40}', args.build)
assert re.fullmatch(r'[0-9a-f]{40}', args.revision)

def api(endpoint):
    return json.loads(subprocess.check_output(['gh', 'api', f'repos/{repo}/{endpoint}'], timeout=120))

def sha256(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        while block := stream.read(1024 * 1024):
            value.update(block)
    return value.hexdigest()

local = {p.name: p for p in args.assets.iterdir()}
assert len(local) == 13 and all(p.is_file() and not p.is_symlink() for p in local.values())
release = api(f'releases/tags/{tag}')
assert release['draft'] is False and release['prerelease'] is False
assert release['tag_name'] == tag and release['target_commitish'] == args.revision
assert api('releases/latest')['id'] == release['id']
remote = {asset['name']: asset for asset in release['assets']}
assert len(remote) == len(release['assets']) and remote.keys() == local.keys()
checksums = {}
for name, path in sorted(local.items()):
    checksum = sha256(path)
    asset = remote[name]
    assert asset['state'] == 'uploaded' and asset['size'] == path.stat().st_size, name
    assert asset['digest'] == 'sha256:' + checksum, name
    assert asset['browser_download_url'] == f'https://github.com/{repo}/releases/download/{tag}/{name}'
    checksums[name] = checksum
ref = api(f'git/ref/tags/{tag}')
assert ref['object']['type'] == 'commit' and ref['object']['sha'] == args.revision
commit = api(f'commits/{args.revision}')
assert [parent['sha'] for parent in commit['parents']] == [args.build]
changed = {item['filename'] for item in commit['files']}
assert changed <= {'Cargo.toml', 'Cargo.lock', 'Casks/oxplay.rb'} and 'Casks/oxplay.rb' in changed
assert api('git/ref/heads/main')['object']['sha'] == args.revision
cask = base64.b64decode(api(f'contents/Casks/oxplay.rb?ref={tag}')['content']).decode()
assert '  version "0.1.0"' in cask
assert f'  sha256 "{checksums[f"oxplay-{tag}-macOS-ARM64.zip"]}"' in cask
source = json.loads(local['release.json'].read_text())
assert source['revision'] == args.revision and source['tag'] == tag
manifest = local['SHA256SUMS.txt'].read_bytes()
remote_manifest = subprocess.check_output([
    'gh', 'api', f'repos/{repo}/releases/assets/{remote["SHA256SUMS.txt"]["id"]}',
    '-H', 'Accept: application/octet-stream'], timeout=120)
assert remote_manifest == manifest
lines = {}
for line in manifest.decode().splitlines():
    checksum, name = line.split(None, 1)
    name = name.removeprefix('*').removeprefix('./')
    assert name not in lines and re.fullmatch(r'[0-9a-f]{64}', checksum)
    lines[name] = checksum
assert lines == {name: checksum for name, checksum in checksums.items() if name != 'SHA256SUMS.txt'}
receipt = {'schema': 1, 'release_url': release['html_url'], 'build_revision': args.build,
           'tag_revision': args.revision, 'published': True, 'prerelease': False, 'latest': True,
           'all_remote_digests_match': True, 'remote_checksum_manifest_matches': True,
           'source_tag_and_cask_verified': True, 'assets': checksums}
args.receipt.write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
print(json.dumps(receipt, indent=2, sort_keys=True))

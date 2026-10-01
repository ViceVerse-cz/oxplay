#!/usr/bin/env python3
"""One-time v0.1.0 promoter. Controls stay outside the released source tree."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

REPOSITORY = 'ViceVerse-cz/oxplay'
RUN = 36839089100
HEAD = '360eda2b7dd871791742fdaf8302b3c26f29dc9b'
TAG = 'v0.1.0'
CONTROLS = Path(__file__).resolve().parent
RUN_URL = f'https://github.com/{REPOSITORY}/actions/runs/{RUN}'
REQUIRED_JOBS = {
    'prepare': ('Release tooling regression (offline)', 'Calculate semantic version and notes', 'Apply planned version to manifests'),
    'macOS ARM64 app bundle': ('Build pinned private Metal media libraries', 'Build and bundle the app', 'Verify and archive Mac package'),
    'Windows X64 zip and installer': ('Build private D3D11 media libraries and runtime source closure', 'Generate private media MSVC import library', 'Check, test and build locked native Windows release', 'Native Windows packaging regression (offline)', 'Stage and archive Windows package', 'Build Windows installer'),
    'linux / Ubuntu 24.04 native Vulkan deb, AppImage and tarball': ('Build pinned native media, verify C ABI and client loading', 'Build locked default-native release binary', 'Packaging regression (offline)', 'Build and inspect the Debian package', 'Build and inspect the AppImage and portable tarball'),
    'linux / fedora-44 native Vulkan rpm': ('Build, package and inspect',),
    'linux / arch native Vulkan arch': ('Build, package and inspect',),
    'checks / macOS ARM64 native Metal build and headless checks': ('Build pinned private native media stack', 'Check formatting and locked default native features', 'Run default native workspace and dependency tests', 'Test bounded native renderer caches without a GPU', 'Build locked default native release workspace', 'Compile native ABI and moving-frame smoke', 'Check headless GPU and hardware decoder availability', 'Run headless native capacity, content and teardown smoke'),
    'checks / Windows x86_64 MSVC legacy OpenGL build and tests': ('Check formatting', 'Lint locked legacy comparison features', 'Run automated Rust tests', 'Exercise synthetic Credential Manager roundtrip', 'Build locked legacy comparison workspace'),
    'checks / Linux X11 + Wayland legacy OpenGL build and tests': ('Check formatting', 'Lint locked legacy comparison features', 'Run automated Rust tests', 'Test packaging and source-audit tools', 'Build locked legacy comparison workspace'),
    'checks / macOS ARM64 legacy OpenGL build and tests': ('Check formatting', 'Lint locked legacy comparison features', 'Run automated Rust tests', 'Test packaging and source-audit tools', 'Build locked legacy comparison workspace'),
    'Assemble release (dry run, nothing published)': ('Local release commit (production)', 'Source archive of the tagged commit', 'Checksums', 'Release notes', 'Summarize the release that would be published'),
}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def command(args, *, cwd=None, env=None, log=None, timeout=1800):
    result = subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True, timeout=timeout)
    if log:
        Path(log).write_text(result.stdout + result.stderr, encoding='utf-8')
    require(result.returncode == 0, f'Command failed: {args[:2]}: {result.stderr[-2000:]}')
    return result.stdout.strip()


def api(endpoint):
    return json.loads(command(['gh', 'api', f'repos/{REPOSITORY}/{endpoint}'], timeout=120))


def validate_run(data, jobs, main):
    require(data.get('id') == RUN and data.get('status') == 'completed' and data.get('conclusion') == 'success', 'Source run is not completed/success')
    require(data.get('head_sha') == HEAD and data.get('head_branch') == 'main', 'Source run revision/branch changed')
    require(data.get('path') == '.github/workflows/release.yml' and data.get('event') == 'workflow_dispatch', 'Wrong source workflow/event')
    require(data.get('repository', {}).get('full_name') == REPOSITORY, 'Wrong source repository')
    require(main.get('object', {}).get('sha') == HEAD and main['object'].get('type') == 'commit', 'Actual remote main moved beyond the validated build')
    by_name = {job['name']: job for job in jobs}
    require(len(by_name) == len(jobs), 'Duplicate source job names')
    require(REQUIRED_JOBS.keys() <= by_name.keys(), 'Mandatory source job is missing')
    for name, required_steps in REQUIRED_JOBS.items():
        job = by_name[name]
        require(job.get('status') == 'completed' and job.get('conclusion') == 'success', f'Mandatory job did not pass: {name}')
        steps = {step['name']: step for step in job.get('steps', [])}
        require(len(steps) == len(job.get('steps', [])), f'Duplicate steps: {name}')
        for step_name in required_steps:
            step = steps.get(step_name, {})
            require(step.get('status') == 'completed' and step.get('conclusion') == 'success', f'Mandatory test/build/smoke was skipped or failed: {name}/{step_name}')
    for name in by_name.keys() - REQUIRED_JOBS.keys():
        require((name in {'publish', 'repositories'} or name.startswith('repositories /')) and by_name[name].get('conclusion') == 'skipped', f'Unexpected source job: {name}')


def validate_release_absent(data):
    repository = data.get('data', {}).get('repository')
    require(not data.get('errors') and isinstance(repository, dict) and {'ref', 'release'} <= repository.keys() and repository.get('ref') is None and repository.get('release') is None,
            'A production v0.1.0 tag or release already exists')


def require_release_absent():
    query = 'query { repository(owner:"ViceVerse-cz", name:"oxplay") { ref(qualifiedName:"refs/tags/v0.1.0") { target { oid } } release(tagName:"v0.1.0") { id } } }'
    data = json.loads(command(['gh', 'api', 'graphql', '-f', 'query=' + query], timeout=120))
    validate_release_absent(data)


def validate(source, evidence):
    require(os.environ.get('PROMOTE_CHANNEL') == 'production' and os.environ.get('PROMOTE_DRY_RUN') == 'false', 'Only explicit production/false dispatch is accepted')
    require(os.environ.get('GITHUB_REPOSITORY') == REPOSITORY, 'Wrong dispatch repository')
    require(command(['git', 'rev-parse', 'HEAD'], cwd=source) == HEAD, 'Source checkout is not the exact binary build revision')
    command(['git', 'diff', '--exit-code', 'HEAD'], cwd=source)
    data, main = api(f'actions/runs/{RUN}'), api('git/ref/heads/main')
    jobs = []
    for page in range(1, 5):
        batch = api(f'actions/runs/{RUN}/jobs?filter=latest&per_page=100&page={page}')
        jobs.extend(batch['jobs'])
        if len(jobs) == batch['total_count']:
            break
    require(len(jobs) == batch['total_count'], 'Source jobs exceeded the bounded inventory')
    validate_run(data, jobs, main)
    require_release_absent()
    evidence.mkdir(parents=True, exist_ok=True)
    receipt = {'schema': 1, 'run': data, 'jobs': jobs, 'remote_main': main, 'required_jobs': list(REQUIRED_JOBS)}
    (evidence / 'validated-source-run.json').write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
    return receipt


def sha256(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        while block := stream.read(1024 * 1024):
            value.update(block)
    return value.hexdigest()


def checksum_manifest(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size < 64 * 1024, 'Invalid preview checksum manifest')
    checksums = {}
    for line in path.read_text(encoding='ascii').splitlines():
        match = re.fullmatch(r'([a-f0-9]{64})  (?:\./)?([A-Za-z0-9_.+-]+)', line)
        require(match is not None, 'Malformed preview checksum entry')
        checksum, name = match.groups()
        require(name not in checksums, 'Duplicate preview checksum filename')
        checksums[name] = checksum
    require(len(checksums) == 12 and {'release.json', f'oxplay-{TAG}-source.tar.gz'} <= checksums.keys(), 'Preview must cover the exact ten binary assets plus two source files')
    return checksums


def compare_preview(assets, preview, receipt):
    candidates = list(preview.rglob('SHA256SUMS.txt'))
    require(len(candidates) == 1, 'Expected one assembled preview checksum manifest')
    expected = checksum_manifest(candidates[0])
    recorded = receipt['assets']
    require(len(recorded) == 10 and set(recorded) == set(expected) - {'release.json', f'oxplay-{TAG}-source.tar.gz'}, 'Preview payload inventory differs')
    for name, item in recorded.items():
        require(item['sha256'] == expected[name] and sha256(assets / name) == expected[name], f'Binary asset changed since source preview: {name}')
    plans = list(preview.rglob('release-plan.json'))
    require(len(plans) == 1, 'Expected one source preview plan')
    plan = json.loads(plans[0].read_text())
    require(plan.get('schema') == 1 and plan.get('channel') == 'production' and plan.get('dry_run') is True and plan.get('git_head') == HEAD and plan.get('tag') == TAG and plan.get('version') == '0.1.0', 'Source preview is not the requested dry-run production build')
    return candidates[0], plans[0]


def production_environment():
    # Call only after validating the actual remote main and exact source checkout.
    return {**os.environ, 'GITHUB_REF_NAME': 'main', 'RELEASE_CHANNEL': 'production', 'DRY_RUN': 'false'}


def promote(source, evidence):
    validate(source, evidence)
    assets, preview = source / 'release-assets', source / 'preview'
    command([sys.executable, str(CONTROLS / 'verify_release_assets.py'), str(assets), '--head', HEAD, '--receipt', str(evidence / 'binary-payload-verification.json')], log=evidence / 'verify-binary-assets.log')
    receipt = json.loads((evidence / 'binary-payload-verification.json').read_text())
    preview_checksums, preview_plan = compare_preview(assets, preview, receipt)
    command([sys.executable, str(CONTROLS / "verify_linux_native_payloads.py"), str(assets), "--source", str(source), "--receipt", str(evidence / "linux-native-source-verification.json")], log=evidence / "verify-linux-native-sources.log")
    shutil.copy2(preview_checksums, evidence / 'preview-SHA256SUMS.txt')
    shutil.copy2(preview_plan, evidence / 'preview-release-plan.json')
    env = production_environment()
    command([sys.executable, 'scripts/release.py', 'plan'], cwd=source, env=env, log=evidence / 'production-plan.log')
    plan_path = source / 'target/release-plan.json'
    plan = json.loads(plan_path.read_text())
    require(plan.get('schema') == 1 and plan.get('channel') == 'production' and plan.get('dry_run') is False and plan.get('git_head') == HEAD and plan.get('tag') == TAG and plan.get('version') == '0.1.0', 'Regenerated production plan differs from validated binaries')
    command([sys.executable, 'scripts/release.py', 'apply-version', '0.1.0'], cwd=source, env=env, log=evidence / 'manifest-version.log')
    for name in ('Cargo.toml', 'Cargo.lock'):
        expected = subprocess.check_output(['git', 'show', f'{HEAD}:{name}'], cwd=source, timeout=60)
        require((source / name).read_bytes() == expected, f'{name} bytes differ from the binary build')
    shutil.copy2(plan_path, evidence / 'production-release-plan.json')
    # Repeat all live main/run checks immediately before the first remote mutation.
    validate(source, evidence)
    commit_output = evidence / 'commit-output.txt'
    env['GITHUB_OUTPUT'] = str(commit_output)
    command([sys.executable, 'scripts/release.py', 'commit-version'], cwd=source, env=env, log=evidence / 'commit-version.log')
    outputs = dict(line.split('=', 1) for line in commit_output.read_text().splitlines())
    revision = outputs.get('commit', '')
    require(re.fullmatch(r'[a-f0-9]{40}', revision) is not None, 'Standard release commit output is missing')
    command(['git', 'fetch', '--no-tags', 'origin', revision], cwd=source, log=evidence / 'fetch-release-commit.log')
    commit = api(f'commits/{revision}')
    require([parent['sha'] for parent in commit['parents']] == [HEAD], 'Release commit has the wrong source parent')
    changed = {entry['filename'] for entry in commit['files']}
    require('Casks/oxplay.rb' in changed and changed <= {'Cargo.toml', 'Cargo.lock', 'Casks/oxplay.rb'}, 'Release commit changed application source')
    require(api('git/ref/heads/main')['object']['sha'] == revision, 'Main does not point at the standard release commit')
    for name in ('Cargo.toml', 'Cargo.lock'):
        require(subprocess.check_output(['git', 'show', f'{revision}:{name}'], cwd=source, timeout=60) == (source / name).read_bytes(), f'Release commit changed built {name} bytes')
    source_output = source / 'target/promoted-source'
    command([sys.executable, 'scripts/release_source.py', '--tag', TAG, '--revision', revision, '--output', str(source_output)], cwd=source, env=env, log=evidence / 'source-export.log')
    for name in ('release.json', f'oxplay-{TAG}-source.tar.gz'):
        shutil.copy2(source_output / name, assets / name)
    shutil.copy2(source_output / 'release.json', evidence / 'source-release.json')
    (assets / 'SHA256SUMS.txt').write_text(''.join(f'{sha256(path)}  {path.name}\n' for path in sorted(assets.iterdir()) if path.is_file()), encoding='ascii')
    require(len(list(assets.iterdir())) == 13, 'Final payload must contain exactly thirteen release assets')
    shutil.copy2(assets / 'SHA256SUMS.txt', evidence / 'published-SHA256SUMS.txt')
    command([sys.executable, 'scripts/release.py', 'notes'], cwd=source, env=env, log=evidence / 'release-notes.log')
    notes = source / 'target/release-notes.md'
    text = notes.read_text()
    previous = 'This production release (0.1.0) was built by CI from the tagged commit with the pinned\nRust toolchain and `--locked` dependencies.'
    require(text.count(previous) == 1, 'Standard tagged-build note changed; review required')
    quote = chr(96)
    replacement = f'The exact binary assets were built by CI from {quote}{HEAD}{quote} with the pinned Rust toolchain and locked dependencies, then passed [validation run {RUN}]({RUN_URL}). The tag targets the subsequent standard release commit {quote}{revision}{quote}; application manifest bytes match the validated build.'
    notes.write_text(text.replace(previous, replacement))
    shutil.copy2(notes, evidence / 'published-release-notes.md')
    (evidence / 'promotion.json').write_text(json.dumps({'schema': 1, 'build_revision': HEAD, 'source_run': RUN, 'source_run_url': RUN_URL, 'release_revision': revision, 'tag': TAG}, indent=2) + '\n')
    require(api('git/ref/heads/main')['object']['sha'] == revision, 'Actual main changed before publication')
    require_release_absent()
    command([sys.executable, 'scripts/release.py', 'publish', '--target', revision], cwd=source, env=env, log=evidence / 'publish.log')
    command([sys.executable, str(CONTROLS / 'verify_published_release.py'), str(assets), '--build', HEAD, '--revision', revision, '--receipt', str(evidence / 'public-release-verification.json')], cwd=source, env=env, log=evidence / 'verify-public-release.log')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['validate', 'promote'])
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--evidence', required=True, type=Path)
    args = parser.parse_args()
    source, evidence = args.source.resolve(), args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    try:
        (validate if args.command == 'validate' else promote)(source, evidence)
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        (evidence / 'failure.txt').write_text(str(error) + '\n')
        print(f'Promotion stopped: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

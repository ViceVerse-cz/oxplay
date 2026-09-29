#!/usr/bin/env python3
"""One explicitly launched local-only lifecycle/capture diagnostic; not a benchmark."""
from pathlib import Path
import hashlib
import json
import os
import re
import struct
import subprocess
import sys
import tempfile
import time

REPO = Path('<workspace>')
sys.path.insert(0, str(REPO / 'scripts'))
from git_source_export import reap_group

BASE = REPO / 'artifacts/native-child-timer-pair-v1'
ORIGINAL = REPO / 'artifacts/native-child-v2'
CLIP = REPO / 'artifacts/local-1080p60.mp4'
CLIP_SHA = 'd5bd6130435aad2f07b00ff102c56d04f649c70e479ec762fe58f659b9baaba0'
STAGES = [5, 8, 11, 14, 17, 20, 23, 26, 29, 32, 35, 38, 41, 44]
CAPTURE_TIMES = [9.5, 18.5, 33.5, 42.5]
MAX_LOG = 8 * 1024 * 1024
MAX_PNG = 32 * 1024 * 1024


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def bounded_log(path):
    with path.open('rb') as stream:
        data = stream.read(MAX_LOG + 1)
    if len(data) > MAX_LOG:
        raise RuntimeError('diagnostic log exceeded bound')
    return '\n'.join(line for line in data.decode('utf-8', errors='strict').splitlines() if not line.startswith('dyld[')) + '\n'


def status(process):
    state = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
    if state is None:
        return None
    return state.si_status if state.si_code == os.CLD_EXITED else -state.si_status


def validate_log(text):
    observed = [int(value) for value in re.findall(r'^native-child lifecycle observed stage=(\d+) ', text, re.M)]
    passed = [int(value) for value in re.findall(r'^native-child lifecycle stage=(\d+) passed; compositor pixels not measured$', text, re.M)]
    if observed != STAGES or passed != STAGES:
        raise RuntimeError('missing, duplicate, reordered or unexpected lifecycle checkpoint')
    events = [(kind, int(stage)) for kind, stage in re.findall(r'^native-child lifecycle (observed stage|stage)=(\d+) ', text, re.M)]
    if events != [(kind, stage) for stage in STAGES for kind in ('observed stage', 'stage')]:
        raise RuntimeError('lifecycle observation/pass ordering mismatch')
    if re.search(r'panicked at|native-child lifecycle stage=\d+ failed:|Error:', text):
        raise RuntimeError('native diagnostic reported failure')
    return {'observed_stages': observed, 'passed_stages': passed, 'exact_order_validated': True,
            'historical_14_stage_driver': True}


def main():
    if len(sys.argv) != 3 or sys.argv[1] != '--run' or sys.argv[2] not in ('on', 'off'):
        raise SystemExit('Explicit --run on|off required.')
    variant = sys.argv[2]
    preparation_path = BASE / 'preparation.json'
    preparation_bytes = preparation_path.read_bytes()
    preparation = json.loads(preparation_bytes)
    selected = preparation['variants'][variant]
    library = REPO / selected['library_relative_path']
    if sha(library) != selected['libmpv_sha256']:
        raise RuntimeError('selected paired library changed')
    identity_path = ORIGINAL / 'binary.json'
    identity_bytes = identity_path.read_bytes()
    identity = json.loads(identity_bytes)
    inventory_path = ORIGINAL / 'inputs-after-release.json'
    inventory_bytes = inventory_path.read_bytes()
    inventory = json.loads(inventory_bytes)
    if (hashlib.sha256(inventory_bytes).hexdigest() != identity['source_inventory_sha256']
            or inventory.get('inputs_match_commit') is not True
            or inventory.get('git_revision') != identity['source_revision']
            or inventory.get('file_count') != identity['build_input_count']
            or len(inventory.get('files', [])) != identity['build_input_count']
            or any(row.get('state') != 'matches_commit' for row in inventory['files'])):
        raise RuntimeError('frozen committed-input association mismatch')
    binary = REPO / selected['app_relative_path']
    if not identity.get('inputs_before_after_identical'):
        raise RuntimeError('frozen source association not confirmed')
    if sha(binary) != selected['app_sha256'] or sha(CLIP) != CLIP_SHA:
        raise RuntimeError('frozen executable or exact local fixture digest mismatch')
    out = BASE / (variant + '-functional')
    out.mkdir(mode=0o700, exist_ok=False)
    (out / 'harness.py').write_bytes(Path(__file__).read_bytes())
    (out / 'binary.json').write_bytes(identity_bytes)
    (out / 'source-inventory.json').write_bytes(inventory_bytes)
    private_parent = Path(tempfile.mkdtemp(prefix='serein-workflows-child.'))
    profile = private_parent / 'data'  # App requires this leaf to be absent.
    path_record = out / 'private-profile-path'
    path_record.write_text(str(profile) + '\n')
    path_record.chmod(0o600)
    args = ['--ui-size', '1320x860', '--ui-theme', 'dark', '--data-root', str(profile),
            '--local', str(CLIP), '--demo-related', '--diagnostics', '--native-video-child',
            '--native-video-child-smoke-test', '--quit-after', '46']
    env = {key: value for key, value in os.environ.items() if not key.startswith(('SEREIN_', 'SLINT_', 'DYLD_'))}
    env['DYLD_PRINT_LIBRARIES'] = '1'
    logpath = out / 'private-loader.log'
    app = wake = None
    failure = None
    code = None
    forced = False
    app_clean = wake_clean = False
    captures = []
    validation = None
    started = time.monotonic()
    deadline = started + 65
    try:
        wake = subprocess.Popen(['/usr/bin/caffeinate', '-d', '-u', '-t', '65'], start_new_session=True,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(1)
        with logpath.open('xb') as log:
            app = subprocess.Popen([str(binary), *args], cwd=REPO, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
            launched = time.monotonic()
            while True:
                code = status(app)
                if code is not None:
                    break
                now = time.monotonic()
                if now >= deadline:
                    forced = True
                    raise TimeoutError('absolute lifecycle diagnostic deadline')
                index = len(captures)
                if index < len(CAPTURE_TIMES) and now - launched >= CAPTURE_TIMES[index]:
                    if now - launched - CAPTURE_TIMES[index] > 0.8:
                        raise RuntimeError('capture schedule missed its finite state window')
                    numbers = re.findall(r'^native child owning window number: (\d+)$', bounded_log(logpath), re.M)
                    if len(numbers) != 1 or int(numbers[0]) <= 0:
                        raise RuntimeError('owning window identity missing or ambiguous')
                    shot = out / f'window-{index + 1}.png'
                    requested = time.monotonic() - launched
                    cap = subprocess.Popen(['/usr/sbin/screencapture', '-x', '-o', '-l', numbers[0], str(shot)],
                                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
                    cap_clean = False
                    try:
                        cap_deadline = min(deadline, time.monotonic() + 8)
                        while (cap_code := status(cap)) is None:
                            if time.monotonic() >= cap_deadline:
                                raise TimeoutError('owning-window capture deadline')
                            time.sleep(0.05)
                        if cap_code != 0:
                            raise RuntimeError('owning-window capture failed')
                    finally:
                        reap_group(cap)
                        cap_clean = True
                    if not shot.is_file() or not 24 <= shot.stat().st_size <= MAX_PNG:
                        raise RuntimeError('invalid bounded capture output')
                    with shot.open('rb') as image:
                        header = image.read(24)
                    if header[:8] != b'\x89PNG\r\n\x1a\n' or header[12:16] != b'IHDR':
                        raise RuntimeError('capture is not a PNG')
                    width, height = struct.unpack('>II', header[16:24])
                    if not 0 < width <= 16384 or not 0 < height <= 16384:
                        raise RuntimeError('capture dimensions exceeded bound')
                    captures.append({'file': shot.name, 'sha256': sha(shot), 'bytes': shot.stat().st_size,
                                     'width': width, 'height': height, 'scheduled_seconds': CAPTURE_TIMES[index],
                                     'requested_seconds': requested, 'completed_seconds': time.monotonic() - launched,
                                     'capture_group_absent': cap_clean})
                time.sleep(0.05)
        if code != 0:
            raise RuntimeError('native lifecycle application exited unsuccessfully')
        if len(captures) != len(CAPTURE_TIMES):
            raise RuntimeError('native lifecycle ended without all required captures')
        native_text = bounded_log(logpath)
        (out / 'native.log').write_text(native_text)
        validation = validate_log(native_text)
        raw = logpath.read_text()
        loaded = [line for line in raw.splitlines() if line.startswith('dyld[') and 'libmpv' in line]
        if len(loaded) != 1 or not loaded[0].endswith(str(library)):
            raise RuntimeError('actual loaded paired libmpv path not uniquely verified')
        if sha(library) != selected['libmpv_sha256'] or preparation_path.read_bytes() != preparation_bytes:
            raise RuntimeError('paired library/preparation changed during diagnostic')
        validation['actual_loaded_mpv_count'] = len(loaded)
        validation['actual_selected_library_path_verified'] = True
        if (sha(binary) != selected['app_sha256'] or identity_path.read_bytes() != identity_bytes
                or inventory_path.read_bytes() != inventory_bytes):
            raise RuntimeError('frozen executable or source record changed during diagnostic')
        if os.path.lexists(profile / 'native-child-fixture.mp4'):
            raise RuntimeError('owned copied fixture survived normal app teardown')
        validation['frozen_identity_unchanged'] = True
        validation['owned_fixture_removed'] = True
    except BaseException as error:
        failure = type(error).__name__ + ': ' + str(error)
    finally:
        if app is not None:
            try:
                reap_group(app)
                app_clean = True
            except BaseException as error:
                failure = failure or type(error).__name__ + ': app cleanup failed'
        if wake is not None:
            try:
                reap_group(wake)
                wake_clean = True
            except BaseException as error:
                failure = failure or type(error).__name__ + ': display assertion cleanup failed'
    summary = {**identity, 'app_sha256': selected['app_sha256'], 'unmodified_app_sha256': identity['app_sha256'], 'variant': variant, 'libmpv_sha256': selected['libmpv_sha256'], 'preparation_sha256': hashlib.sha256(preparation_bytes).hexdigest(), 'mode': 'native-lifecycle-capture', 'binary_metadata_sha256': sha(identity_path),
               'harness_sha256': sha(Path(__file__)), 'cleanup_implementation_sha256': sha(REPO / 'scripts/git_source_export.py'),
               'fixture_sha256': CLIP_SHA, 'exit_code': code, 'forced_termination': forced,
               'failure': failure, 'group_absence_after_reap_confirmed': app_clean,
               'display_assertion_group_absent': wake_clean, 'elapsed_seconds': time.monotonic() - started,
               'requested_app_seconds': 46, 'absolute_deadline_seconds': 65,
               'real_account': False, 'performance_measurement': False, 'bounded_external_display_assertion': True,
               'captures': captures, 'lifecycle_validation': validation,
               'private_loader_log_sha256': sha(logpath) if logpath.exists() else None,
               'native_log_sha256': sha(out / 'native.log') if (out / 'native.log').exists() else None,
               'capture_scope': 'Finite external owning-window captures; stages validate native state. Pixels require separate visual inspection. No cadence, A/V or resource qualification.'}
    (out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    return 0 if code == 0 and failure is None and app_clean and wake_clean and validation else 1


if __name__ == '__main__':
    raise SystemExit(main())

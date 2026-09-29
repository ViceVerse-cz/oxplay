#!/usr/bin/env python3
"""Explicit finite macOS related-focus check against an already frozen release."""
from pathlib import Path
import argparse, hashlib, json, os, re, stat, subprocess, sys, time

STAGES=[4,6,8,10,12,14,16,19,22,24,26,28,30,33,36,39]
FIXTURE_SHA='d5bd6130435aad2f07b00ff102c56d04f649c70e479ec762fe58f659b9baaba0'
COMPLETE='related focus check complete: wide_compact=true ordinary_tab=true fullscreen_offset=true search_cancel=true fullscreen_cancel=true catalog_delta=0 loads=1 screen_reader=not_tested compositor_visibility=not_measured'

def require(condition, reason):
    if not condition: raise ValueError(reason)

def read(path, limit):
    descriptor=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC)
    with os.fdopen(descriptor,'rb') as source:
        info=os.fstat(source.fileno())
        require(stat.S_ISREG(info.st_mode) and 0<info.st_size<=limit,'invalid bounded input file')
        data=source.read(limit+1)
        require(len(data)==info.st_size and len(data)<=limit,'input changed or exceeded bound')
        return data

def sha(path, limit=1024*1024*1024):
    descriptor=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC)
    digest=hashlib.sha256()
    with os.fdopen(descriptor,'rb') as source:
        before=os.fstat(source.fileno())
        require(stat.S_ISREG(before.st_mode) and 0<before.st_size<=limit,'invalid hash input')
        total=0
        while block:=source.read(1024*1024):
            total+=len(block);require(total<=limit,'hash input grew beyond bound');digest.update(block)
        after=os.fstat(source.fileno())
        require(total==before.st_size==after.st_size and before.st_mtime_ns==after.st_mtime_ns,'hash input changed')
    return digest.hexdigest()

def write_json(path, data):
    with path.open('x') as out: json.dump(data,out,indent=2);out.write('\n')

def wait_status(process, deadline):
    while True:
        status=os.waitid(os.P_PID,process.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
        if status is not None:
            return status.si_status if status.si_code==os.CLD_EXITED else -status.si_status
        if time.monotonic()>=deadline: raise TimeoutError('finite native deadline')
        time.sleep(.1)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo',type=Path,default=Path.cwd())
    parser.add_argument('--inventory',type=Path,help='Defaults to frozen base/inputs-after-release.json')
    args=parser.parse_args()
    require(sys.platform=='darwin' and hasattr(os,'WNOWAIT'),'macOS anchored supervision required')
    os.umask(0o077)
    repo=args.repo.resolve(strict=True)
    sys.path.insert(0,str(repo/'scripts'))
    from git_source_export import reap_group
    base=repo/'artifacts/ui-workflows-v1'
    app=base/'serein-frozen';identity_path=base/'binary.json'
    inventory_path=args.inventory or base/'inputs-after-release.json'
    identity_bytes=read(identity_path,1024*1024);identity=json.loads(identity_bytes)
    inventory_bytes=read(inventory_path,8*1024*1024);inventory=json.loads(inventory_bytes)
    require(re.fullmatch('[0-9a-f]{40}',identity['source_revision']) is not None,'invalid source revision')
    require(re.fullmatch('[0-9a-f]{64}',identity['app_sha256']) is not None,'invalid binary digest')
    require(sha(app)==identity['app_sha256'],'frozen app digest mismatch')
    require(hashlib.sha256(inventory_bytes).hexdigest()==identity['source_inventory_sha256'],'inventory digest mismatch')
    require(identity.get('inputs_before_after_identical') is True and inventory.get('inputs_match_commit') is True
            and inventory['git_revision']==identity['source_revision']
            and inventory['file_count']==identity['build_input_count']==len(inventory['files'])
            and all(row.get('state')=='matches_commit' for row in inventory['files']), 'incomplete recorded build/source association')
    clip=repo/'artifacts/local-1080p60.mp4'
    require(sha(clip,128*1024*1024)==FIXTURE_SHA,'published synthetic fixture mismatch')
    out=base/'related-release';out.mkdir(mode=0o700,exist_ok=False)
    profile=out/'private-profile' # Remains absent until the application atomically creates it.
    (out/'binary.json').write_bytes(identity_bytes)
    (out/'source-inventory.json').write_bytes(inventory_bytes)
    (out/'harness.py').write_bytes(read(Path(__file__),1024*1024))
    provenance={**identity,'harness_sha256':sha(out/'harness.py'),'supervision_source_sha256':sha(repo/'scripts/git_source_export.py'),
        'fixture_sha256':FIXTURE_SHA,'fresh_private_profile':True,'existing_profile_used':False,
        'real_account':False,'performance_measurement':False,'source_association_scope':'Frozen build record and matching committed-input inventory; no rebuild in this harness',
        'display_fixture':'Bounded external display assertion; no application power claim',
        'process_scope':'Own app and wake groups only; no system or XPC process signalled',
        'utc_started':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())}
    write_json(out/'provenance.json',provenance)
    env={key:value for key,value in os.environ.items() if not key.startswith(('SEREIN_','SLINT_','DYLD_'))}
    command=[str(app),'--related-focus-check','--local',str(clip),'--demo-related','--data-root',str(profile)]
    process=wake=None;start=time.monotonic()
    result={'exit_code':None,'failure_type':None,'forced_termination':False,'process_group_absent':False,'wake_group_absent':False,
            'native_stages_passed':False,'private_fixture_removed':False,'binary_unchanged':False,'source_records_unchanged':False}
    try:
        wake=subprocess.Popen(['/usr/bin/caffeinate','-d','-u','-t','65'],start_new_session=True,
                              stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        time.sleep(1)
        with (out/'native.log').open('xb') as log:
            process=subprocess.Popen(command,cwd=repo,env=env,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
            result['exit_code']=wait_status(process,time.monotonic()+55)
    except BaseException as error:
        result['failure_type']=type(error).__name__
        result['forced_termination']=isinstance(error,TimeoutError)
    finally:
        for child,field in [(process,'process_group_absent'),(wake,'wake_group_absent')]:
            if child is not None:
                try:reap_group(child);result[field]=True
                except BaseException as error:result['failure_type']=result['failure_type'] or type(error).__name__
    try:
        result['binary_unchanged']=sha(app)==identity['app_sha256']
        result['source_records_unchanged']=(read(identity_path,1024*1024)==identity_bytes and read(inventory_path,8*1024*1024)==inventory_bytes)
        result['private_fixture_removed']=not os.path.lexists(profile/'native-child-fixture.mp4')
        require(result['exit_code']==0 and result['failure_type'] is None and not result['forced_termination'],'native process did not pass')
        require(result['process_group_absent'] and result['wake_group_absent'],'owned process cleanup incomplete')
        require(result['binary_unchanged'] and result['source_records_unchanged'],'frozen identity changed')
        require(profile.is_dir() and not profile.is_symlink() and stat.S_IMODE(profile.stat().st_mode)==0o700,'private profile admission missing')
        require(result['private_fixture_removed'],'owned copied fixture survived teardown')
        raw=read(out/'native.log',8*1024*1024);text=raw.decode('utf-8')
        observed=[int(value) for value in re.findall(r'^related focus stage=(\d+).*result=Ok\(\(\)\)$',text,re.M)]
        require(observed==STAGES,'missing duplicated or out-of-order native checkpoint')
        require(text.splitlines().count(COMPLETE)==1,'terminal completion marker missing or duplicated')
        require(text.splitlines().count('related fullscreen retirement: player=true parking=false serial=0')==1,'player focus retirement missing')
        require('related focus failed:' not in text and 'result=Err(' not in text and "panicked at" not in text,'native diagnostic failure present')
        result['native_stages_passed']=True;result['observed_stages']=observed
    except BaseException as error:
        result['validation_failure_type']=type(error).__name__
        # Only static validation errors are exposed; never exception strings from OS/file APIs.
        if type(error) is ValueError:result['validation_reason']=str(error)
    result['elapsed_seconds']=time.monotonic()-start
    result['native_log_sha256']=sha(out/'native.log',8*1024*1024) if (out/'native.log').is_file() else None
    result.update({'app_sha256':identity['app_sha256'],'source_revision':identity['source_revision'],
                   'real_account':False,'performance_measurement':False})
    write_json(out/'summary.json',result)
    print(json.dumps(result,indent=2))
    return 0 if result['native_stages_passed'] else 1

if __name__=='__main__':raise SystemExit(main())

from pathlib import Path
import hashlib,json,os,shutil,subprocess,sys,time
R=Path('<workspace>');sys.path.insert(0,str(R/'scripts'))
from git_source_export import reap_group
BASE=R/'artifacts/native-child-v2';PAIR=R/'artifacts/mpv-timers-pair-20260929-v2';OUT=R/'artifacts/native-child-timer-pair-v1'
EXPECTED='0734e6d46cef1c22eb527a67e0e135eedc69c63f3ca3458dd54b513ceb93afd9'
LIBS={'on':'fc873c29f7f7cac23f419c7030c9351ee70a4662f4d257f974f87a7a1bca93c5','off':'895e653209230e86dd46f58ffe8a8ab983c8686308ef4383830ff65f41e4224e'}
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
 return h.hexdigest()
def run(args,name):
 p=None
 with (OUT/(name+'.log')).open('xb') as f:
  try:
   p=subprocess.Popen(args,stdout=f,stderr=subprocess.STDOUT,start_new_session=True,stdin=subprocess.DEVNULL)
   deadline=time.monotonic()+15
   while True:
    s=os.waitid(os.P_PID,p.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
    if s is not None:
     if s.si_code!=os.CLD_EXITED or s.si_status:raise RuntimeError('tool failed: '+name)
     break
    if time.monotonic()>deadline:raise TimeoutError('tool deadline: '+name)
    time.sleep(.025)
  finally:
   if p is not None:reap_group(p)
 return (OUT/(name+'.log')).read_text()
if sys.argv[1:]!=['--prepare']:raise SystemExit('explicit --prepare required; never launches an app')
original=BASE/'serein-frozen';identity=json.loads((BASE/'binary.json').read_text());assert identity['app_sha256']==EXPECTED==sha(original)
audit=json.loads((PAIR/'linked-output-audit.json').read_text());assert audit['normalized_dependency_records_equal'] and audit['effective_meson_option_differences']==['gpu-pass-timers']
OUT.mkdir(mode=0o700,exist_ok=False);(OUT/'preparer.py').write_bytes(Path(__file__).read_bytes());(OUT/'original-binary.json').write_bytes((BASE/'binary.json').read_bytes())
records=[]
for variant in audit['variants']:
 for label,row in variant['linked_closures']['libmpv'].items():
  if label.startswith('cellar/'):path=Path('/opt/homebrew/Cellar')/label.removeprefix('cellar/')
  elif label.startswith('pair/'):path=PAIR/label.removeprefix('pair/')
  else:raise RuntimeError('unexpected closure label')
  observed=sha(path);assert observed==row['sha256'],label
  records.append({'variant':variant['variant'],'label':label,'sha256':observed})
meta={**identity,'unmodified_app_sha256':EXPECTED,'preparer_sha256':sha(Path(__file__)),'prior_linked_audit_sha256':sha(PAIR/'linked-output-audit.json'),'method':'Private a45 executable copies: one LC_LOAD_DYLIB replacement, then ad-hoc sign. Original executable and both paired libraries remain unchanged. No app launch.','system_dyld_cache_hashed':False,'current_non_system_dependency_records_verified':records,'runtime_loaded_library_verified':False,'performance_qualified':False,'variants':{}}
old='/opt/homebrew/opt/mpv/lib/libmpv.2.dylib'
initial=run(['/usr/bin/otool','-L',str(original)],'original-linkage')
initialdeps=[line.strip() for line in initial.splitlines()[1:]];assert sum(line.startswith(old+' ') for line in initialdeps)==1
for variant,expected in LIBS.items():
 lib=PAIR/('prefix-'+variant)/'lib/libmpv.2.dylib';assert sha(lib)==expected
 app=OUT/('serein-'+variant);shutil.copyfile(original,app);app.chmod(0o700);assert sha(app)==EXPECTED
 run(['/usr/bin/install_name_tool','-change',old,str(lib),str(app)],variant+'-relink')
 run(['/usr/bin/codesign','--force','--sign','-','--timestamp=none',str(app)],variant+'-sign')
 run(['/usr/bin/codesign','--verify','--strict','--verbose=2',str(app)],variant+'-verify')
 signing=run(['/usr/bin/codesign','-dvv',str(app)],variant+'-signature')
 assert 'Signature=adhoc' in signing and 'TeamIdentifier=not set' in signing
 linkage=run(['/usr/bin/otool','-L',str(app)],variant+'-linkage')
 deps=[line.strip() for line in linkage.splitlines()[1:]]
 assert deps==[line.replace(old,str(lib),1) if line.startswith(old+' ') else line for line in initialdeps]
 assert sha(lib)==expected and sha(original)==EXPECTED
 meta['variants'][variant]={'app_relative_path':str(app.relative_to(R)),'app_sha256':sha(app),'library_relative_path':str(lib.relative_to(R)),'libmpv_sha256':sha(lib),'signature_verified':True,'signature_route':'Ad-hoc diagnostic copy only; no Developer ID/notarization claim','static_linkage_only_mpv_changed':True,'runtime_loaded_library_verified':False}
(OUT/'preparation.json').write_text(json.dumps(meta,indent=2)+'\n')
print(json.dumps({'preparation_sha256':sha(OUT/'preparation.json'),'variants':meta['variants'],'verified_non_system_records':len(records),'launched':False},indent=2))

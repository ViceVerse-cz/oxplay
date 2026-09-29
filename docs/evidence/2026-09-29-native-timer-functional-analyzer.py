from pathlib import Path
import hashlib,json,re,subprocess,os,time,sys
R=Path('<workspace>');sys.path.insert(0,str(R/'scripts'))
from git_source_export import reap_group
B=R/'artifacts/native-child-timer-pair-v1';P=R/'artifacts/mpv-timers-pair-20260929-v2'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
meta=json.loads((B/'preparation.json').read_text());audit=json.loads((P/'linked-output-audit.json').read_text())
results={};loadedsets={}
for variant in ('on','off'):
 run=B/(variant+'-functional');summary=json.loads((run/'summary.json').read_text());selected=meta['variants'][variant]
 assert summary['exit_code']==0 and summary['failure'] is None and summary['lifecycle_validation']['actual_selected_library_path_verified']
 assert sha(R/selected['app_relative_path'])==selected['app_sha256'] and sha(R/selected['library_relative_path'])==selected['libmpv_sha256']
 raw=(run/'private-loader.log').read_text();loaded={};transitions=0
 for line in raw.splitlines():
  if line.startswith('dyld['):
   m=re.fullmatch(r'dyld\[\d+\]: <([0-9A-F-]+)> (/.+)',line)
   if m is None:
    assert re.fullmatch(r'dyld\[\d+\]: move (?:loaded to delayed|delayed to loaded): [A-Za-z0-9_.+-]+',line),line
    transitions+=1
    continue
   loaded[str(Path(m[2]).resolve())]=m[1]
 expected=next(v['linked_closures']['libmpv'] for v in audit['variants'] if v['variant']==variant)
 rows=[]
 for label,row in expected.items():
  p=(Path('/opt/homebrew/Cellar')/label.removeprefix('cellar/')) if label.startswith('cellar/') else (P/label.removeprefix('pair/'))
  assert sha(p)==row['sha256'];assert str(p.resolve()) in loaded,label
  rows.append({'label':label,'sha256':row['sha256'],'loaded_uuid':loaded[str(p.resolve())]})
 libpath=str((R/selected['library_relative_path']).resolve())
 loadedsets[variant]={path:uuid for path,uuid in loaded.items() if path!=libpath and path!=str((R/selected['app_relative_path']).resolve())}
 for c in summary['captures']:assert sha(run/c['file'])==c['sha256']
 results[variant]={'app_sha256':selected['app_sha256'],'libmpv_sha256':selected['libmpv_sha256'],'elapsed_seconds':summary['elapsed_seconds'],'actual_loaded_non_system_mpv_closure':rows,'loader_state_transition_lines':transitions,'loaded_image_count':len(loaded),'actual_unique_mpv_image':True,'groups_absent':summary['group_absence_after_reap_confirmed'] and summary['display_assertion_group_absent'],'fixture_removed':summary['lifecycle_validation']['owned_fixture_removed'],'stages':summary['lifecycle_validation']['passed_stages']}
# The app's two private executable paths and libmpv images differ intentionally.
assert loadedsets['on']==loadedsets['off']
roi={'x':520,'y':216,'width':1328,'height':747};expected_bytes=roi['width']*roi['height']*3
ffmpeg=Path('/opt/homebrew/bin/ffmpeg');buffers=[]
for variant in ('on','off'):
 run=B/(variant+'-functional');image=run/'window-1.png';out=B/(variant+'-paused.rgb');err=B/(variant+'-pixel-decode.log');process=None
 try:
  with out.open('xb') as sink,err.open('xb') as log:
   process=subprocess.Popen([str(ffmpeg),'-nostdin','-v','error','-i',str(image),'-vf',f"crop={roi['width']}:{roi['height']}:{roi['x']}:{roi['y']}",'-frames:v','1','-f','rawvideo','-pix_fmt','rgb24','pipe:1'],stdout=sink,stderr=log,stdin=subprocess.DEVNULL,start_new_session=True)
   deadline=time.monotonic()+10
   while True:
    status=os.waitid(os.P_PID,process.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
    if status is not None:
     assert status.si_code==os.CLD_EXITED and status.si_status==0
     break
    if time.monotonic()>deadline:raise TimeoutError('finite offline PNG decode')
    time.sleep(.025)
 finally:
  if process is not None:reap_group(process)
 data=out.read_bytes();assert len(data)==expected_bytes;buffers.append(data)
a,b=buffers;different=0;maximum=0;total=0
for i in range(0,len(a),3):
 d=[abs(a[i+k]-b[i+k]) for k in range(3)];different+=any(d);maximum=max(maximum,*d);total+=sum(d)
report={'source_revision':meta['source_revision'],'unmodified_app_sha256':meta['unmodified_app_sha256'],'preparation_sha256':sha(B/'preparation.json'),'variants':results,'all_other_actual_loaded_image_paths_and_uuids_equal':True,'system_loaded_images_hashed':False,'paused_video_comparison':{'source':'Finite external owning-window capture1 in each variant after actual paused seek20. Logged geometry determines ROI; no scaling or pixel alteration.','roi_px':roi,'pixels':roi['width']*roi['height'],'different_pixels':different,'maximum_channel_delta':maximum,'sum_absolute_channel_deltas':total,'exact_pixel_equality':different==0,'on_png_sha256':sha(B/'on-functional/window-1.png'),'off_png_sha256':sha(B/'off-functional/window-1.png'),'on_rgb_sha256':hashlib.sha256(a).hexdigest(),'off_rgb_sha256':hashlib.sha256(b).hexdigest(),'decoder_tool_sha256':sha(ffmpeg.resolve())},'performance_measurement':False,'full_spec_pass':False,'production_patch_adopted':False,'limitations':['Finite lifecycle and one paused frame comparison are not continuous display or perceptual A/V qualification.','Other agents may compile; no CPU/RAM/energy inference.','System dyld-cache images have UUID identity but are not byte-hashed.','No original application or library modified; both executables are ad-hoc diagnostic copies.']}
(B/'functional-analysis.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'paused_video_comparison':report['paused_video_comparison'],'actual_loaded_closure_counts':{v:len(r['actual_loaded_non_system_mpv_closure']) for v,r in results.items()},'all_other_images_equal':True},indent=2))

from pathlib import Path
import xml.etree.ElementTree as E,collections,json,hashlib,re,sys
mode=sys.argv[1];p=Path('artifacts/native-child-v2')/('xctrace-'+mode+'-v1');source=p/'cpu-samples.xml';r=E.parse(source).getroot();ids={e.attrib['id']:e for e in r.iter() if 'id' in e.attrib}
def real(e):
 while 'ref' in e.attrib:e=ids[e.attrib['ref']]
 return e
def frames(e):return [real(f) for f in real(e).findall('frame')] if e is not None else []
summary=json.loads((p/'summary.json').read_text());attach=summary['trace_command'];expected_pid=attach[attach.index('--attach')+1]
rows=list(r.iter('row'));states=collections.Counter();thread=collections.Counter();inclusive=collections.Counter();leaf=collections.Counter();leafbinary=collections.Counter();categories=collections.Counter();times=[];selection=collections.Counter()
want=['mpv_render_context_render','gl_video_render_frame','gl_timer_stop','gl_timer_start','glEndQuery_Exec','glGetQueryObjectui64v_Exec','serein_child_flush','CGLFlushDrawable','GLDContextRec::flushContext(bool)','glTexImage2D_Exec','glDrawArrays_GL3Exec','gldGetQueryInfo','gl_sc_addf','bstr_xappend_vasprintf','gl_sc_generate']
for row in rows:
 w=int(real(row.find('weight')).text);assert w>0
 state=real(row.find('thread-state')).text;states[state]+=w;assert state=='Running'
 proc=real(row.find('process'));assert proc.attrib['fmt']=='serein-frozen ('+expected_pid+')'
 name=real(row.find('thread')).attrib.get('fmt','unnamed').split(' (0x',1)[0];name=re.sub(r'0x[0-9a-fA-F]+','<pointer>',name);thread[name]+=w
 fs=frames(row.find('tagged-backtrace'));symbols=[f.attrib.get('name','unknown') for f in fs] or ['<missing backtrace>']
 for s in set(symbols):inclusive[s]+=w
 leaf[symbols[0]]+=w;binary=fs[0].find('binary') if fs else None;leafbinary[real(binary).attrib['name'] if binary is not None else '<unknown>']+=w
 for k in want:
  if k in symbols:selection[k]+=w
 if not fs:cat='missing backtrace'
 elif 'mpv_render_context_render' in symbols:cat='libmpv render subtree'
 elif 'serein_child_flush' in symbols:cat='native child flush subtree'
 elif any('FemtoVGRenderer' in s and 'render' in s for s in symbols):cat='Slint renderer excluding nested media/native flush'
 elif name=='Main Thread':cat='other main-thread work'
 else:cat='other threads'
 categories[cat]+=w;times.append(int(real(row.find('sample-time')).text))
total=sum(states.values());assert sum(thread.values())==sum(categories.values())==sum(leaf.values())==total
out={'mode':mode,'rows':len(rows),'raw_cpu_xml_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'row_states_weight_ms':{k:v/1e6 for k,v in states.items()},'total_running_sample_weight_ms':total/1e6,'sample_time_first_last_ns':[min(times),max(times)],'thread_running_weight_ms':{k:v/1e6 for k,v in thread.most_common()},'disjoint_stack_categories_weight_ms':{k:v/1e6 for k,v in categories.items()},'selected_inclusive_stack_weight_ms':{k:selection[k]/1e6 for k in want},'leaf_binary_weight_ms':{k:v/1e6 for k,v in leafbinary.most_common()},'top_leaf_symbols_weight_ms':{k:v/1e6 for k,v in leaf.most_common(20)},'scope':'All exported CPU rows target the owned application and are Running, weight1ms. Statistical weights are not an independently measured exact CPU-time total, call counts, or a resource benchmark. Inclusive selected stack weights overlap; disjoint categories sum to total. Raw XML/trace remains private and includes automatic host/environment metadata in other tables.'}
(p/'cpu-analysis.json').write_text(json.dumps(out,indent=2)+'\n');print(json.dumps(out,indent=2))

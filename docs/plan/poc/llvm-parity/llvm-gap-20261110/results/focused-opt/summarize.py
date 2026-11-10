"""Summarize round-paired candidate ratios; no pooling across hosts/runs."""
import json,math,random,statistics as st,sys
from pathlib import Path
root,out=map(Path,sys.argv[1:]);rng=random.Random(20261110)
def ratio(a,b):
 assert a.keys()==b.keys()
 xs=[a[k]/b[k] for k in sorted(a)]
 boots=sorted(st.median(rng.choices(xs,k=len(xs))) for _ in range(4000))
 return dict(median=st.median(xs),bootstrap_95=[boots[100],boots[3899]],pairs=len(xs))
result={'runtime':[],'compile':[],'sizes':{},'tests':{},'method':{'candidate':'all','baseline':'start (ranges plus cycle regression, rebuilt in focused source path)','timing':'21 alternating round pairs, 4x suite iterations, separate perf run','bootstrap':'4000 bootstrap resamples of round-paired ratios; seed 20261110; within-run uncertainty','driver':'LLVM-built common driver; library backend differential, not full program replacement'}}
for c in ['semver','base64','unicode-segmentation','humantime']:
 data=json.loads((root/'suite/runtime'/f'{c}.json').read_text())
 for work,d in data['workloads'].items():
  samples={a:{s['round']:s['elapsed_ns'] for s in d['samples'] if s['arm']==a and s['round']>=0} for a in data['arms']}
  assert all(s['exit_code']==0 for s in d['samples']) and len({s['checksum'] for s in d['samples']})==1
  assert all(len(s)==21 for s in samples.values())
  row={'crate':c,'workload':work,'candidate_over_start':ratio(samples['all'],samples['start']),'candidate_over_llvm':ratio(samples['all'],samples['llvm']),'arms_over_start':{a:ratio(s,samples['start']) for a,s in samples.items() if a not in ['start','llvm']},'instructions_over_start':d['arms']['all']['instructions']/d['arms']['start']['instructions'],'instructions_over_llvm':d['arms']['all']['instructions']/d['arms']['llvm']['instructions']}
  result['runtime'].append(row)
 result['sizes'][c]={a:json.loads((root/'suite'/a/(c+'.json')).read_text())['size'] for a in ['llvm','start','h1','address','checked','h1_address','all']}
 result['tests'][c]=json.loads((root/'suite/all'/(c+'.json')).read_text())['tests']
p=root/'suite/compile-final.json'
if p.exists():
 rows=json.loads(p.read_text());assert all(r['exit_code']==0 and r['objects_identical_to_reference'] for r in rows)
 for c in result['sizes']:
  d={a:{r['round']:r for r in rows if r['arm']==a and r['crate']==c} for a in ['start','all']}
  assert len(d['start'])==len(d['all'])==21
  rec={'crate':c}
  for name,key in [('wall','wall_seconds'),('cpu',None),('rss','maxrss_kib'),('instructions','instructions')]:
   q={a:{n:r[key] if key else r['user_seconds']+r['sys_seconds'] for n,r in values.items()} for a,values in d.items()}
   rec[name]=ratio(q['all'],q['start'])
  result['compile'].append(rec)
result['runtime_geomean_ratio']=math.exp(st.mean(math.log(r['candidate_over_start']['median']) for r in result['runtime']))
result['llvm_runtime_geomean_ratio']=math.exp(st.mean(math.log(r['candidate_over_llvm']['median']) for r in result['runtime']))
if result['compile']:result['compile_geomean_ratio']=math.exp(st.mean(math.log(r['wall']['median']) for r in result['compile']))
out.write_text(json.dumps(result,indent=2)+'\n')
for row in result['runtime']:print(row['crate'],row['workload'],row['candidate_over_start'],row['candidate_over_llvm']['median'])
for row in result['compile']:print('COMPILE',row['crate'],row['wall'])
print({k:v for k,v in result.items() if k.endswith('geomean_ratio')})

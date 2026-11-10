"""Keep the 13-workload run and ASCII confirmation distinct; verify binary inheritance."""
from pathlib import Path
import json,math,statistics as st,random,sys
base,natural,output=map(Path,sys.argv[1:]);rng=random.Random(20261110)
def ratio(a,b):
 assert a.keys()==b.keys() and len(a)==21
 xs=[a[k]/b[k] for k in sorted(a)]
 boots=sorted(st.median(rng.choices(xs,k=len(xs))) for _ in range(4000))
 return {'median':st.median(xs),'bootstrap_95':[boots[100],boots[3899]],'pairs':len(xs)}
identity=json.loads((natural/'natural-identity.json').read_text());assert all(r['objects_identical'] and r['driver_identical'] and r['driver_objects_identical'] for r in identity.values())
d=json.loads((natural/'suite/runtime/unicode-segmentation-ascii-recheck.json').read_text())['workloads']['graphemes-ascii'];assert all(s['exit_code']==0 for s in d['samples']) and len({s['checksum'] for s in d['samples']})==1
arms={a:{s['round']:s['elapsed_ns'] for s in d['samples'] if s['arm']==a and s['round']>=0} for a in ['start','natural']}
rec={'source_overlay_sha256':json.loads((natural/'accepted-fingerprint.json').read_text())['source_overlay_sha256'],'backend_sha256':json.loads((natural/'accepted-fingerprint.json').read_text())['backend_sha256'],'runtime_inheritance':identity,'ascii_confirmation':{'candidate_over_start':ratio(arms['natural'],arms['start']),'instructions_over_start':d['arms']['natural']['instructions']/d['arms']['start']['instructions'],'cycles_over_start':d['arms']['natural']['cycles']/d['arms']['start']['cycles'],'samples_source':'suite/runtime/unicode-segmentation-ascii-recheck.json'},'compile':[],'method':{'runtime':'Other 12 workloads inherit selected exact executable bytes; original 13-workload measurements remain separately published. ASCII has a distinct 21-pair confirmation, not pooled with original samples.','compile':'21 alternating start/natural pairs per crate; same source/sysroot/flags; produced objects hash-identical to each arm reference','bootstrap':'4000 bootstrap samples of paired ratios, seed20261110, within-run uncertainty only'}}
rows=json.loads((natural/'suite/compile-natural.json').read_text());assert len(rows)==168 and all(r['exit_code']==0 and r['objects_identical_to_reference'] for r in rows)
for c in ['semver','base64','unicode-segmentation','humantime']:
 a={arm:{r['round']:r for r in rows if r['crate']==c and r['arm']==arm} for arm in ['start','natural']};r={'crate':c}
 for name,key in [('wall','wall_seconds'),('cpu',None),('rss','maxrss_kib'),('instructions','instructions')]:
  vals={arm:{i:v[key] if key else v['user_seconds']+v['sys_seconds'] for i,v in data.items()} for arm,data in a.items()};r[name]=ratio(vals['natural'],vals['start'])
 rec['compile'].append(r)
rec['compile_geomean_ratio']=math.exp(st.mean(math.log(r['wall']['median']) for r in rec['compile']))
output.write_text(json.dumps(rec,indent=2)+'\n')
print('ASCII',rec['ascii_confirmation'])
for r in rec['compile']:print('COMPILE',r['crate'],r['wall'],r['instructions'])
print('COMPILE GEOMEAN',rec['compile_geomean_ratio'])

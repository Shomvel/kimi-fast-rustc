from pathlib import Path
import json,statistics,math,random
import sys
r=Path(sys.argv[1]);rng=random.Random(1110);rows=[]
for c in ['semver','base64','unicode-segmentation','humantime']:
 data=json.loads((r/'runtime-raw'/(c+'.json')).read_text())
 for workload,w in data['workloads'].items():
  pairs={a:[] for a in ['shared','data','llvm']}
  rel=[]
  for iteration in range(31):
   s={x['arm']:x['elapsed_ns'] for x in w['samples'] if x['round']==iteration and x['exit_code']==0}
   assert len(s)==4,(c,workload,iteration)
   for a in pairs:pairs[a].append(s[a]/s['baseline'])
   rel.append(s['data']/s['llvm'])
  def ci(xs):
   estimates=sorted(statistics.median(rng.choices(xs,k=len(xs))) for _ in range(10000));return [estimates[249],estimates[9749]]
  row=dict(crate=c,workload=workload,paired_ratio={a:statistics.median(p) for a,p in pairs.items()},bootstrap95={a:ci(p) for a,p in pairs.items()},candidate_over_llvm=statistics.median(rel),median_ns={a:v['median_ns'] for a,v in w['arms'].items()},instructions={a:v.get('instructions') for a,v in w['arms'].items()});rows.append(row)
  print(c,workload,row['paired_ratio'],row['bootstrap95']['data'],row['candidate_over_llvm'],flush=True)
sizes=json.loads((r/'library-sizes.json').read_text())
summary=dict(protocol='31 alternating rounds after warmup, paired median ratios and percentile bootstrap 10000 draws, seed1110',workloads=rows,sizes=sizes,geomean={a:math.exp(statistics.mean(math.log(row['paired_ratio'][a]) for row in rows)) for a in ['llvm','shared','data']},candidate_over_llvm_geomean=math.exp(statistics.mean(math.log(row['candidate_over_llvm']) for row in rows)))
Path(sys.argv[2]).write_text(json.dumps(summary,indent=2)+'\n')
print('GEOMEAN',summary['geomean'],'CANDIDATE/LLVM',summary['candidate_over_llvm_geomean'],flush=True)
print('SIZES',json.dumps({c:{a:s['text_bytes'] for a,s in arms.items()} for c,arms in sizes.items()}),flush=True)

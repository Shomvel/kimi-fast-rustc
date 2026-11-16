from pathlib import Path
import json, statistics, math, random, sys
out=Path(sys.argv[1])
rows=json.loads((out/'compile-raw.json').read_text())
arms=['llvm','baseline','shared','data']
crates=[{'name':n} for n in ['semver','base64','unicode-segmentation','humantime']]
rng=random.Random(1110);summary=[]
for c in crates:
 n=c['name'];ratios={};ci={}
 for a in arms:
  paired=[]
  for iteration in range(1,22):
   values={x['arm']:x['wall_seconds'] for x in rows if x['round']==iteration and x['crate']==n};paired.append(values[a]/values['baseline'])
  ratios[a]=statistics.median(paired);estimates=sorted(statistics.median(rng.choices(paired,k=21)) for _ in range(10000));ci[a]=[estimates[249],estimates[9749]]
 row=dict(crate=n,paired_ratio=ratios,bootstrap95=ci,metrics={a:{k:statistics.median(x[k] for x in rows if x['crate']==n and x['arm']==a and x['round']>0) for k in ['wall_seconds','user_seconds','sys_seconds','maxrss_kib']} for a in arms});summary.append(row);print(n,ratios,ci,flush=True)
Path(sys.argv[2]).write_text(json.dumps(dict(warmups=1,rounds=21,protocol='rotating/reversing four arms, serial commands, warm filesystem, fresh rustc to rlib; checker object equality verified',crates=summary,geomean={a:math.exp(statistics.mean(math.log(x['paired_ratio'][a]) for x in summary)) for a in arms}),indent=2)+'\n')

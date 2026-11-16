from pathlib import Path
import importlib.util,json,subprocess,statistics,math,time,os,random
spec=importlib.util.spec_from_file_location('suite','/workspace/baseline/tools/llvm-parity-suite/suite.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
r=Path('/workspace/results/formal-suite');out=Path('/workspace/results/compile50/final-bench');out.mkdir(parents=True,exist_ok=True)
backends={'baseline':('/workspace/results/bits.so','1'),'serial':('/workspace/results/compile50/parallel-body.so','1'),'parallel':('/workspace/results/compile50/parallel-body.so','8')}
crates=[c for c in m.CONFIG['crates'] if c['name']!='regex-syntax'];rows=[]
def run(c,arm,round):
 so,threads=backends[arm];d=out/c['name']/arm;d.mkdir(parents=True,exist_ok=True);env=m.clean_env();env.update(KFRC_CODEGEN_THREADS=threads,KFRC_REGALLOC_CHECK='0');cmd=m.library_cmd(r,c,so,d/'lib.rlib');stem=d/f'round-{round:02}'
 with Path(str(stem)+'.stdout').open('w') as stdout,Path(str(stem)+'.stderr').open('w') as stderr:
  start=time.perf_counter_ns();proc=subprocess.Popen(cmd,stdout=stdout,stderr=stderr,env=env);pid,status,usage=os.wait4(proc.pid,0);wall=(time.perf_counter_ns()-start)/1e9;proc.returncode=os.waitstatus_to_exitcode(status);assert proc.returncode==0,(c['name'],arm)
 row={'crate':c['name'],'arm':arm,'round':round,'wall_seconds':wall,'user_seconds':usage.ru_utime,'sys_seconds':usage.ru_stime,'maxrss_kib':usage.ru_maxrss,'command':cmd,'environment':{'KFRC_CODEGEN_THREADS':threads,'KFRC_REGALLOC_CHECK':'0'}};rows.append(row);Path(str(stem)+'.json').write_text(json.dumps(row,indent=2)+'\n')
for c in crates:
 for arm in backends:run(c,arm,0)
for round in range(1,22):
 for c in (crates if round%2 else crates[::-1]):
  arms=list(backends);arms=arms[round%3:]+arms[:round%3]
  if round%2:arms=arms[::-1]
  for arm in arms:run(c,arm,round)
 if round%7==0:print('round',round,flush=True)
(out/'raw.json').write_text(json.dumps(rows,indent=2)+'\n');summary=[];rng=random.Random(1110)
for c in crates:
 n=c['name'];metrics={a:{k:statistics.median(x[k] for x in rows if x['crate']==n and x['arm']==a and x['round']>0) for k in ['wall_seconds','user_seconds','sys_seconds','maxrss_kib']} for a in backends}
 pairs={a:[] for a in ['serial','parallel']}
 for j in range(1,22):
  samples={x['arm']:x['wall_seconds'] for x in rows if x['crate']==n and x['round']==j}
  for a in pairs:pairs[a].append(samples[a]/samples['baseline'])
 intervals={}
 for a,ratios in pairs.items():
  estimates=sorted(statistics.median(rng.choices(ratios,k=len(ratios))) for _ in range(10000));intervals[a]=[estimates[249],estimates[9749]]
 objects={}
 for arm in backends:
  d=out/n/arm;objects[arm]={p.name:m.sha(p) for p in m.unpack(d/'lib.rlib',d/'objects')}
 ref={p.name:m.sha(p) for p in (r/'bits'/n/'ref/objects').glob('*.o')};assert all(h==ref for h in objects.values())
 row={'crate':n,'metrics':metrics,'paired_median_ratio':{a:statistics.median(p) for a,p in pairs.items()},'bootstrap95':intervals,'objects_sha256':ref};summary.append(row);print(row['crate'],metrics,row['paired_median_ratio'],intervals,flush=True)
(out/'summary.json').write_text(json.dumps({'warmups':1,'rounds':21,'protocol':'rotating/reversing three arms, same sandbox serial commands, blocking wait4, warm filesystem caches; build/profile/test excluded','backends':{a:{'sha256':m.sha(so),'threads':t} for a,(so,t) in backends.items()},'crates':summary,'geomean_ratio':{a:math.exp(statistics.mean(math.log(x['paired_median_ratio'][a]) for x in summary)) for a in pairs}},indent=2)+'\n')

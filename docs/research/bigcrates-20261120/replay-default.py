#!/usr/bin/env python3
"""Crate-level default pipeline A/B; LLVM dependencies are intentionally reused."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

p=argparse.ArgumentParser()
p.add_argument('--project',default='ripgrep')
p.add_argument('--baseline',required=True,type=Path)
p.add_argument('--candidate',required=True,type=Path)
p.add_argument('--label',required=True)
p.add_argument('--units',help='comma-separated formal crate names')
p.add_argument('--env-supplement',type=Path)
args=p.parse_args()
supplements=json.loads(args.env_supplement.read_text()) if args.env_supplement else {}
root=Path('/workspace/bigcrates')
out=root/'default-replays'/args.label/args.project
out.mkdir(parents=True,exist_ok=False)
index=[]
for capture in sorted((root/'captures'/args.project).glob('*.json')):
 r=json.loads(capture.read_text());argv=r['argv']
 if '--target' not in argv or '--crate-name' not in argv: continue
 if not any(a.startswith('--emit=') and 'link' in a.split('=',1)[1].split(',') for a in argv): continue
 crate=argv[argv.index('--crate-name')+1]
 if args.units and crate not in args.units.split(','):continue
 for arm,backend in [('baseline',args.baseline),('candidate',args.candidate)]:
  unit=out/(crate+'-'+capture.stem.split('-')[-1])/arm
  (unit/'objects').mkdir(parents=True)
  command=[];i=0
  while i<len(argv):
   a=argv[i]
   if a=='--out-dir': command.extend([a,str(unit/'objects')]);i+=2
   elif a.startswith('--emit='):command.append('--emit=obj');i+=1
   else:command.append(a);i+=1
  command.append('-Zcodegen-backend='+str(backend))
  env=dict(r['env'],RUSTC_BOOTSTRAP='1',KFRC_MACHINE='cranelift',KFRC_CODEGEN_THREADS='1')
  env.update(supplements.get(str(capture),{}))
  receipt={'capture':str(capture),'crate':crate,'arm':arm,'argv':command,'cwd':r['cwd'],'env':env,
           'backend_sha256':hashlib.sha256(backend.read_bytes()).hexdigest()}
  if str(capture) in supplements:
   receipt['env_supplement']={'path':str(args.env_supplement),'sha256':hashlib.sha256(args.env_supplement.read_bytes()).hexdigest(),'values':supplements[str(capture)]}
  (unit/'invocation.json').write_text(json.dumps(receipt,indent=2)+'\n')
  started=time.time()
  with (unit/'stdout.log').open('w') as stdout,(unit/'stderr.log').open('w') as stderr:
   try:
    result=subprocess.run(command,cwd=r['cwd'],env=env,stdout=stdout,stderr=stderr,timeout=1800)
    receipt['exit_code']=result.returncode
   except subprocess.TimeoutExpired:receipt['exit_code']=None;receipt['timeout']=True
  receipt['seconds']=time.time()-started
  receipt['objects']=[{'path':str(path),'bytes':path.stat().st_size,'sha256':hashlib.sha256(path.read_bytes()).hexdigest()} for path in sorted((unit/'objects').glob('*.o'))]
  receipt['diagnostics']=[]
  for line in (unit/'stderr.log').read_text().splitlines():
   try:
    diagnostic=json.loads(line)
    if diagnostic.get('level')=='error':receipt['diagnostics'].append(diagnostic.get('message'))
   except json.JSONDecodeError: pass
  (unit/'result.json').write_text(json.dumps(receipt,indent=2)+'\n')
  index.append({'crate':crate,'arm':arm,'unit':str(unit),'exit_code':receipt['exit_code'],'objects':receipt['objects'],'first_error':next(iter(receipt['diagnostics']),None)})
  (out/'index.json').write_text(json.dumps(index,indent=2)+'\n')
  print(f'DEFAULT_COMPLETE {crate} {arm} exit={receipt["exit_code"]} objects={len(receipt["objects"])}',flush=True)
print('DEFAULT_REPLAY_COMPLETE',flush=True)

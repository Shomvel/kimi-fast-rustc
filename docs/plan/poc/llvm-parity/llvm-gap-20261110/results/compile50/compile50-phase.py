from pathlib import Path
import json,subprocess,importlib.util,statistics
spec=importlib.util.spec_from_file_location('suite','/workspace/baseline/tools/llvm-parity-suite/suite.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
out=Path('/workspace/results/compile50/phase');out.mkdir(exist_ok=True);root=Path('/workspace/results/formal-suite')
for c in m.CONFIG['crates']:
 if c['name']=='regex-syntax':continue
 for mode,backend,threads in [('baseline','/workspace/results/bits.so','1'),('candidate','/workspace/results/compile50/parallel-body.so','8')]:
  d=out/c['name']/mode;d.mkdir(parents=True,exist_ok=True)
  for i in range(3):
   env=m.clean_env();env['KFRC_CODEGEN_THREADS']=threads;env['KFRC_REGALLOC_CHECK']='0';env['KFRC_PROFILE_TOTAL']=str(d/f'total-{i}.jsonl')
   cmd=m.library_cmd(root,c,backend,d/'lib.rlib')+['-Ztime-passes'];code,wall,usage=m.run_logged(cmd,d/f'compile-{i}',env=env);assert code==0
   print(c['name'],mode,wall,usage.ru_utime+usage.ru_stime,usage.ru_maxrss,flush=True)

import importlib.util,json,os,pathlib,statistics,shutil
spec=importlib.util.spec_from_file_location('suite','/workspace/suite/suite.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
root=pathlib.Path('/workspace/results/integrated-final1/suite')
orig=m.clean_env
def cost_env():
 e=orig();e['KFRC_REGALLOC_CHECK']='0';return e
m.clean_env=cost_env
arms=['llvm','c12-phi','integrated']
backends={'llvm':'llvm','c12-phi':'/workspace/results/compile-c12-base/backend.so','integrated':'/workspace/results/integrated-final1/backend.so'}
rows=[]
for c in m.CONFIG['crates']:
 name=c['name'];records={a:json.loads((root/a/f'{name}.json').read_text()) for a in arms}
 valid=[a for a in arms if records[a]['status']=='ok']
 full={a:[] for a in valid};meta={a:[] for a in valid}
 for r in range(3):
  for a in valid if r%2==0 else list(reversed(valid)):
   d=root/a/name/'cost-check0'/f'full-{r}'
   row=m.perf_compile(m.library_cmd(root,c,backends[a],d/'lib.rlib'),d)
   assert row['exit_code']==0,(name,a,r,'full')
   ref=root/a/name/'ref'/'objects'
   hashes={x.name:m.sha(x) for x in m.unpack(d/'lib.rlib',d/'objects')}
   row['objects_identical_to_reference']=hashes=={x.name:m.sha(x) for x in ref.glob('*.o')}
   assert row['objects_identical_to_reference'],(name,a,r,'objects')
   shutil.rmtree(d/'objects');full[a].append(row)
   d=root/a/name/'cost-check0'/f'metadata-{r}'
   row=m.perf_compile(m.library_cmd(root,c,backends[a],d/'lib.rmeta',emit='metadata'),d)
   assert row['exit_code']==0,(name,a,r,'metadata');meta[a].append(row)
 for a in valid:
  rec=records[a]
  rec['compile_checker1_not_formal']=rec['compile']
  rec['compile']={'rounds':3,'checker':0,'order':'three rounds: llvm,c12-phi,integrated then reverse then forward; same sandbox serial','full':full[a],'metadata_only':meta[a],
  'instructions_median':m.med(full[a],'instructions'),'cycles_median':m.med(full[a],'cycles'),'wall_median':m.med(full[a],'wall_seconds'),'user_median':m.med(full[a],'user_seconds'),'maxrss_kib_median':m.med(full[a],'maxrss_kib'),'metadata_instructions_median':m.med(meta[a],'instructions'),'metadata_wall_median':m.med(meta[a],'wall_seconds')}
  rec['compile']['codegen_instructions_estimate']=rec['compile']['instructions_median']-rec['compile']['metadata_instructions_median']
  (root/a/f'{name}.json').write_text(json.dumps(rec,indent=2)+'\n')
  rows.append({'crate':name,'arm':a,**rec['compile']})
 print('CHECKER0 COST PASS '+name,flush=True)
(root/'cost-check0-summary.json').write_text(json.dumps(rows,indent=2)+'\n')

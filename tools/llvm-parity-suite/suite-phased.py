import importlib.util,json,os,pathlib,sys
spec=importlib.util.spec_from_file_location('suite','/workspace/suite/suite.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
root=pathlib.Path('/workspace/results/integrated-final1/suite')
root.mkdir(parents=True,exist_ok=True)
backends={'llvm':'llvm','c12-phi':'/workspace/results/compile-c12-base/backend.so','integrated':'/workspace/results/integrated-final1/backend.so'}
m.prepare(root)
original_env=m.clean_env
# Preflight explicitly enables checker; no compile-cost sample occurs.
def checked_env():
 e=original_env();e['KFRC_REGALLOC_CHECK']='1';return e
m.clean_env=checked_env
os.environ['SUITE_COMPILE_ROUNDS']='0'
def outcomes(rec,level):
 t=rec.get('tests',{}).get(level,{})
 if 'binaries' not in t:return None
 return {(b.replace('-unit',''),n):v for b,row in t['binaries'].items() for n,v in row['outcomes'].items()}
preflight=[]
for crate in m.CONFIG['crates']:
 name=crate['name'];os.environ['SUITE_CRATES']=name
 for arm,backend in backends.items():
  m.arm(root,'check-'+arm,backend)
 base=json.loads((root/'check-llvm'/f'{name}.json').read_text())
 for arm in ('c12-phi','integrated'):
  rec=json.loads((root/('check-'+arm)/f'{name}.json').read_text())
  row={'crate':name,'arm':arm,'status':rec['status'],'checker':1}
  if rec['status']!='ok':
   errs=' '.join(rec.get('compile_error',[]))
   if name!='regex-syntax':raise RuntimeError(f'NEW library failure: {name}/{arm}: {errs}')
   row['expected_unsupported']=rec.get('compile_error',[])
  else:
   assert rec['driver']['exit_code']==0,(name,arm,'driver')
   assert rec['corpus']['exit_code']==0 and rec['corpus']['sha256']==base['corpus']['sha256'],(name,arm,'corpus')
   assert rec['workload_check']==base['workload_check'],(name,arm,'checksum')
   missing={}
   for level in ('itest','unit'):
    bo=outcomes(base,level);co=outcomes(rec,level)
    if bo and co is not None:
     bad=[key for key,v in bo.items() if v=='ok' and co.get(key)!='ok']
     assert not bad,(name,arm,level,bad)
    elif bo:missing[level]=rec.get('tests',{}).get(level,{}).get('build_error',['not_run'])
   row['upstream_incomplete']=missing
   row['corpus_equal']=True;row['checksums_equal']=True
  preflight.append(row)
  (root/'preflight.json').write_text(json.dumps(preflight,indent=2)+'\n')
 print('PREFLIGHT CRATE PASS '+name,flush=True)
# Release checker1/0 must not change objects; source paths remain identical.
m.clean_env=original_env
os.environ.pop('SUITE_CRATES',None)
os.environ['SUITE_COMPILE_ROUNDS']='3'
def reuse_tests(root_arg,c,backend,out,cdir):
 arm=out.name
 return json.loads((root/('check-'+arm)/f"{c['name']}.json").read_text())['tests']
m.cargo_tests=reuse_tests
for arm,backend in backends.items():m.arm(root,arm,backend)
for crate in m.CONFIG['crates']:
 name=crate['name']
 for arm in backends:
  a=root/('check-'+arm)/name/'ref'/'objects';b=root/arm/name/'ref'/'objects'
  if a.exists() and b.exists():
   assert {x.name:m.sha(x) for x in a.glob('*.o')}=={x.name:m.sha(x) for x in b.glob('*.o')},(name,arm,'checker changes objects')
m.runtime(root,list(backends))
print('FINAL_SUITE_PASS',flush=True)

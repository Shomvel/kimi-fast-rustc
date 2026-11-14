#!/usr/bin/env python3
import argparse,hashlib,json,os,pathlib,subprocess
p=argparse.ArgumentParser();p.add_argument('--inputs',type=pathlib.Path,required=True);p.add_argument('--backend',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);a=p.parse_args();a.out.mkdir(parents=True,exist_ok=False)
env={k:v for k,v in os.environ.items() if not k.startswith('KFRC_')};env['RUSTC_BOOTSTRAP']='1'
def run(cmd,path,extra=None):
 with path.with_suffix('.stdout').open('w') as o,path.with_suffix('.stderr').open('w') as e:cp=subprocess.run(cmd,env=env|(extra or {}),stdout=o,stderr=e,timeout=300)
 path.with_suffix('.result.json').write_text(json.dumps({'command':cmd,'exit_code':cp.returncode},indent=2)+'\n')
 if cp.returncode:raise SystemExit(f'first failure: {path}')
run(['cc','-O2','-c',str(a.inputs/'driver.c'),'-o',str(a.out/'driver.o')],a.out/'driver-build')
ref=None;records=[]
for arm,opt in [('llvm',None),('off','0'),('on','1')]:
 d=a.out/arm;d.mkdir();cmd=['rustc','+1.94.1','--crate-name=pressure','--crate-type=rlib','--edition=2021',str(a.inputs/'lib.rs'),'-Cpanic=abort','-Coverflow-checks=off','-Cdebug-assertions=off','-Copt-level=3','-Ccodegen-units=1','-Cembed-bitcode=no','-Zmir-opt-level=0','-o',str(d/'libpressure.rlib')]
 if opt is not None:cmd+=['-Zcodegen-backend='+str(a.backend)]
 run(cmd,d/'build',{'KFRC_OPTIMIZE':opt} if opt is not None else {})
 run(['rustc','+1.94.1','--edition=2021',str(a.inputs/'driver.rs'),'-Cpanic=abort','-Copt-level=3','--extern','pressure='+str(d/'libpressure.rlib'),'-Clink-arg='+str(a.out/'driver.o'),'-o',str(d/'driver')],d/'link')
 run([str(d/'driver')],d/'execute');output=(d/'execute.stdout').read_bytes()
 if ref is None:ref=output
 assert output==ref,arm
 records.append({'arm':arm,'output_lines':len(output.splitlines()),'stdout_sha256':hashlib.sha256(output).hexdigest(),'library_sha256':hashlib.sha256((d/'libpressure.rlib').read_bytes()).hexdigest()})
(a.out/'summary.json').write_text(json.dumps({'records':records,'backend_sha256':hashlib.sha256(a.backend.read_bytes()).hexdigest()},indent=2)+'\n');print(json.dumps(records,indent=2))

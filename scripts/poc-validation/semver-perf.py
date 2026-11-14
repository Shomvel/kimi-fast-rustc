#!/usr/bin/env python3
"""Same-host, serial, alternating semver compiler/runtime measurements.

Inputs: repository providing runner, extracted semver 1.0.28 (verified separately),
output directory, JSON map arm=>backend.so or null for LLVM. No network or secrets.
"""
import argparse, hashlib, json, os, pathlib, statistics, subprocess, time
p=argparse.ArgumentParser(); p.add_argument('--source',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--arms',type=pathlib.Path,required=True);p.add_argument('--levels',default='0,3');p.add_argument('--rounds',type=int,default=5);p.add_argument('--runtime-rounds',type=int,default=7);p.add_argument('--sysroot',type=pathlib.Path);p.add_argument('--backend-profile',default='unspecified; inspect backend build receipt');p.add_argument('--driver',type=pathlib.Path)
a=p.parse_args();a.out.mkdir(parents=True,exist_ok=False)
arms=json.loads(a.arms.read_text()); compiler='rustc'; records=[]
driver=a.driver or pathlib.Path(__file__).resolve().with_name('semver-perf-driver.rs')
sysroot_flags=['--sysroot='+str(a.sysroot.resolve())] if a.sysroot else []
clean_env=os.environ.copy()
for key in ('KFRC_PROFILE', 'KFRC_PROFILE_TOTAL', 'KFRC_LLVM_PROFILE', 'KFRC_SPECIALIZATION_STATS','KFRC_INLINE','KFRC_OPTIMIZE','KFRC_DUMP_GRAPH','KFRC_DUMP_BODY','KFRC_INTERPRET','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS'):
    clean_env.pop(key,None)
clean_env['RUSTC_BOOTSTRAP']='1'
def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def run(cmd,stem):
    start=time.perf_counter()
    with (stem.with_suffix('.stdout')).open('w') as o, (stem.with_suffix('.stderr')).open('w') as e:
        cp=subprocess.run(cmd,stdout=o,stderr=e,env=clean_env)
    result={'command':cmd,'exit_code':cp.returncode,'wall_observed':time.perf_counter()-start}
    stem.with_suffix('.command.json').write_text(json.dumps(result,indent=2)+'\n')
    if cp.returncode: raise SystemExit(f'failed; stopped before repeat: {stem}')
    return result
source_hashes={str(x.relative_to(a.source)):digest(x) for x in sorted(a.source.rglob('*')) if x.is_file()}
(a.out/'inputs.json').write_text(json.dumps({'source_files':source_hashes,'driver_sha256':digest(driver),'backends':{k:digest(pathlib.Path(v)) if v else 'LLVM' for k,v in arms.items()},'cache_policy':'fresh output per compile, warm OS page cache; incremental disabled; no cache flush','compiler_optimization':a.backend_profile,'cgu_by_level':{'0':4,'3':16},'lto':'default; LLVM O3 with 16 CGU uses ThinLocal; kfrc has its own pipeline','mir':'-Zmir-opt-level=0; shared rebuilt std specified' if a.sysroot else '-Zmir-opt-level=0; existing LLVM std, not build-std MIR0; not full formal PoC claim','sysroot':str(a.sysroot.resolve()) if a.sysroot else None,'sysroot_library_sha256':{str(x.relative_to(a.sysroot)):digest(x) for x in sorted(a.sysroot.rglob('*')) if x.is_file()} if a.sysroot else None,'driver_scope':'same LLVM O3 source; std/libtest and some cross-crate generic/inline code use LLVM; no LTO'},indent=2)+'\n')
run([compiler,'+1.94.1','-vV'],a.out/'rustc-version')
for level in map(int,a.levels.split(',')):
    target=a.out/f'o{level}';target.mkdir();latest={}
    for round in range(a.rounds):
        order=list(arms) if round%2==0 else list(reversed(arms))
        for arm in order:
            d=target/f'compile-{round}-{arm}';d.mkdir(); lib=d/'libsemver.rlib'
            flags=['--crate-name','semver','--crate-type','rlib','--edition=2021',str(a.source/'src/lib.rs'),'--cfg','feature="std"','-Cpanic=abort',f'-Copt-level={level}',f'-Ccodegen-units={4 if level == 0 else 16}','-Cembed-bitcode=no','-Cdebuginfo=0','-Coverflow-checks=off','-Cdebug-assertions=off','-Zmir-opt-level=0','-o',str(lib)]
            if arms[arm]:flags+=['-Zcodegen-backend='+arms[arm]]
            t=d/'resource.json'
            cmd=['/usr/bin/time','-f','{"wall_seconds":%e,"user_seconds":%U,"sys_seconds":%S,"maxrss_kib":%M}','-o',str(t),compiler,'+1.94.1']+sysroot_flags+flags
            run(cmd,d/'build'); rec={'kind':'compile','level':level,'arm':arm,'round':round,**json.loads(t.read_text()),'rlib_bytes':lib.stat().st_size,'rlib_sha256':digest(lib)};records.append(rec);latest[arm]=d
            (a.out/'samples.json').write_text(json.dumps(records,indent=2)+'\n')
    for arm,d in latest.items():
        obj=d/'objects';obj.mkdir();subprocess.run(['ar','x',str(d/'libsemver.rlib')],cwd=obj,check=True)
        sections=subprocess.check_output(['size','-A',*[str(x) for x in obj.glob('*.o')]],text=True)
        (d/'size-sections.txt').write_text(sections);text=sum(int(s.split()[1]) for s in sections.splitlines() if s.startswith('.text'))
        exe=target/f'driver-{arm}';run([compiler,'+1.94.1',*sysroot_flags,'--edition=2021',str(driver),'-Cpanic=abort','-Copt-level=3','-Cdebuginfo=0','-Zmir-opt-level=0','--extern','semver='+str(d/'libsemver.rlib'),'-o',str(exe)],target/f'driver-build-{arm}')
        rawsize=exe.stat().st_size; stripped=target/f'driver-{arm}.stripped';subprocess.run(['strip','--strip-all','-o',str(stripped),str(exe)],check=True)
        records.append({'kind':'size','level':level,'arm':arm,'object_text_bytes':text,'object_bytes':sum(x.stat().st_size for x in obj.glob('*.o')),'executable_bytes':rawsize,'executable_stripped_bytes':stripped.stat().st_size,'executable_sha256':digest(exe)})
    for mode,count in [('parse',100000),('req',50000),('identifier',10000)]:
        expected=None
        for round in range(-1,a.runtime_rounds):
            for arm in (list(arms) if round%2==0 else list(reversed(arms))):
                stem=target/f'run-{mode}-{round}-{arm}';run([str(target/f'driver-{arm}'),mode,str(count)],stem);res=json.loads(stem.with_suffix('.stdout').read_text());key=(res['iterations'],res['checksum'])
                if expected is None:expected=key
                if key!=expected:raise SystemExit(f'output mismatch: {stem}')
                records.append({'kind':'runtime','level':level,'arm':arm,'round':round,'warmup':round==-1,**res})
                (a.out/'samples.json').write_text(json.dumps(records,indent=2)+'\n')
if source_hashes!={str(x.relative_to(a.source)):digest(x) for x in sorted(a.source.rglob('*')) if x.is_file()}: raise SystemExit('source changed')
summary=[]
for level in map(int,a.levels.split(',')):
 for arm in arms:
    c=[r for r in records if r['kind']=='compile' and r['level']==level and r['arm']==arm]
    row={'level':level,'arm':arm,'compile_wall_median':statistics.median(r['wall_seconds'] for r in c),'compile_user_median':statistics.median(r['user_seconds'] for r in c),'maxrss_kib_median':statistics.median(r['maxrss_kib'] for r in c)}
    row.update(next(r for r in records if r['kind']=='size' and r['level']==level and r['arm']==arm))
    row['runtime_ns_medians']={mode:statistics.median(r['elapsed_ns'] for r in records if r['kind']=='runtime' and not r['warmup'] and r['level']==level and r['arm']==arm and r['workload']==mode) for mode in ['parse','req','identifier']};summary.append(row)
(a.out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary,indent=2))

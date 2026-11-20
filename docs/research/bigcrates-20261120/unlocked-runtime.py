#!/usr/bin/env python3
"""Exercise newly unlocked rlibs with LLVM drivers and linker-map provenance."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

root=Path('/workspace/bigcrates')
work=root/'unlocked-runtime/atomic-v5'
work.mkdir(parents=True,exist_ok=False)
captures={}
for path in sorted((root/'captures/ripgrep').glob('*.json')):
 r=json.loads(path.read_text());a=r['argv']
 if '--crate-name' in a:
  name=a[a.index('--crate-name')+1]
  if name in ['log','walkdir'] and any(x.startswith('--emit=') and 'link' in x for x in a):captures[name]=r
drivers={
'log':r'''use std::sync::atomic::{AtomicUsize,Ordering};
static EVENTS: AtomicUsize=AtomicUsize::new(0);
struct Logger;
static LOGGER: Logger=Logger;
impl log::Log for Logger {
 fn enabled(&self, m: &log::Metadata)->bool {m.level()<=log::Level::Info}
 fn log(&self,r:&log::Record) {if self.enabled(r.metadata()){EVENTS.fetch_add(1,Ordering::Relaxed);println!("{}:{}",r.level(),r.args());}}
 fn flush(&self) {}
}
fn main(){log::set_logger(&LOGGER).unwrap();log::set_max_level(log::LevelFilter::Info);log::info!("hello {}",7);log::debug!("hidden");assert_eq!(EVENTS.load(Ordering::Relaxed),1);println!("max={}",log::max_level());}
''',
'walkdir':r'''fn main(){let root=std::env::args().nth(1).unwrap();let mut paths:Vec<_>=walkdir::WalkDir::new(&root).into_iter().map(|e|e.unwrap().path().strip_prefix(&root).unwrap().to_string_lossy().into_owned()).collect();paths.sort();for p in paths {println!("{}",p);} }
'''}
tree=work/'tree';(tree/'nested').mkdir(parents=True);(tree/'a.txt').write_text('a');(tree/'nested/b.txt').write_text('b')
receipt={}
for name,record in captures.items():
 outputs=[];receipt[name]={}
 for arm in ['llvm','kfrc']:
  out=work/name/arm;out.mkdir(parents=True)
  command=[];argv=record['argv'];i=0
  while i<len(argv):
   if argv[i]=='--out-dir':command.extend(['--out-dir',str(out)]);i+=2
   else:command.append(argv[i]);i+=1
  env=dict(record['env'],RUSTC_BOOTSTRAP='1',KFRC_MACHINE='cranelift')
  if arm=='kfrc':command.append('-Zcodegen-backend='+str(root/'backends/atomic-v5.so'))
  commands=[command]
  (out/'driver.rs').write_text(drivers[name])
  for index,cmd in enumerate(commands):
   r=subprocess.run(cmd,cwd=record['cwd'],env=env,capture_output=True,timeout=180)
   (out/f'{index}.command.json').write_text(json.dumps(cmd,indent=2)+'\n');(out/f'{index}.stdout').write_bytes(r.stdout);(out/f'{index}.stderr').write_bytes(r.stderr);(out/f'{index}.exit').write_text(str(r.returncode))
   assert r.returncode==0,r.stderr.decode(errors='replace')
  rlib=next(out.glob('lib'+name+'-*.rlib'))
  link=['rustc','--edition=2024','-Cpanic=abort','-Copt-level=0','-Zmir-opt-level=0',str(out/'driver.rs'),'--extern',name+'='+str(rlib),'-L','dependency='+str(root/'targets/ripgrep-llvm/x86_64-unknown-linux-gnu/release/deps'),'-Clink-arg=-Wl,-Map='+str(out/'link.map'),'-o',str(out/'driver')]
  r=subprocess.run(link,env=env,capture_output=True,timeout=180)
  (out/'1.command.json').write_text(json.dumps(link,indent=2)+'\n');(out/'1.stdout').write_bytes(r.stdout);(out/'1.stderr').write_bytes(r.stderr);(out/'1.exit').write_text(str(r.returncode))
  assert r.returncode==0,r.stderr.decode(errors='replace')
  run=[str(out/'driver')]+([str(tree)] if name=='walkdir' else [])
  r=subprocess.run(run,env=env,capture_output=True,timeout=30)
  (out/'2.command.json').write_text(json.dumps(run,indent=2)+'\n');(out/'2.stdout').write_bytes(r.stdout);(out/'2.stderr').write_bytes(r.stderr);(out/'2.exit').write_text(str(r.returncode))
  assert r.returncode==0,r.stderr.decode(errors='replace')
  outputs.append(r.stdout)
  members=subprocess.check_output(['ar','t',str(rlib)],text=True)
  (out/'archive-members.txt').write_text(members)
  obj_members=[m for m in members.splitlines() if m.endswith('.o')]
  assert obj_members
  linkmap=(out/'link.map').read_text()
  used=[m for m in obj_members if m in linkmap]
  assert used,(name,arm,'rlib object absent from link map')
  receipt[name][arm]={'rlib_sha256':hashlib.sha256(rlib.read_bytes()).hexdigest(),'driver_sha256':hashlib.sha256((out/'driver').read_bytes()).hexdigest(),'object_members':obj_members,'linked_object_members':used,'stdout':r.stdout.decode()}
 assert outputs[0]==outputs[1],(name,outputs)
 print('UNLOCKED_RUNTIME_PASS',name,outputs[1].decode(),flush=True)
(work/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
print('UNLOCKED_RUNTIME_COMPLETE',flush=True)

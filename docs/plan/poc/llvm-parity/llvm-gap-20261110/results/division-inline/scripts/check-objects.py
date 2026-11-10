from pathlib import Path
import importlib.util, struct, hashlib, json
spec=importlib.util.spec_from_file_location('hot','/workspace/baseline/docs/plan/poc/llvm-parity/llvm-gap-20261110/analyze_hotspots.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
objdump='/opt/rustup/toolchains/1.94.1-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-objdump'
r=Path('/workspace/results/suite');out=Path('/workspace/results/static');out.mkdir(exist_ok=True)
rows=[]
for arm in ['llvm','baseline','shared','data']:
 for c in ['semver','base64','unicode-segmentation','humantime']:
  for p in (r/arm/c/'ref/objects').glob('*.o'):
   for f in m.inspect(p,objdump,out/'asm'/arm/c/(p.stem+'.asm')):
    rows.append(dict(arm=arm,crate=c,object_sha256=hashlib.sha256(p.read_bytes()).hexdigest(),**f))
(out/'functions.json').write_text(json.dumps(rows,indent=2)+'\n')
focus=['internal_encode','decode_chunk_8','Rfc3339Timestamp as core::fmt','Parser>::parse_unit','Identifier','FormattedDuration','parse_rfc3339_weak']
(out/'hotspots.json').write_text(json.dumps([r for r in rows if any(n in r['name'] for n in focus)],indent=2)+'\n')
print('STATIC_PASS',len(rows),flush=True)

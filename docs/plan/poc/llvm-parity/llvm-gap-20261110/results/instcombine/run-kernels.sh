set -euo pipefail
export RUSTC_BOOTSTRAP=1 KFRC_REGALLOC_CHECK=1
mkdir -p /workspace/results/kernels
cp /workspace/kernels.rs /workspace/driver.rs /workspace/results/kernels/
cd /workspace/results/kernels
flags=(--edition=2024 --sysroot=/workspace/shared-std-mir0/sysroot -Cpanic=abort -Coverflow-checks=off -Copt-level=3 -Ccodegen-units=1 -Cembed-bitcode=no -Zmir-opt-level=0)
for arm in llvm baseline instcombine-v2; do
 mkdir -p "$arm/objects"
 backend=()
 if [ "$arm" != llvm ]; then backend=(-Zcodegen-backend=/workspace/results/$arm.so); fi
 rustc +1.94.1 "${flags[@]}" "${backend[@]}" --crate-type=rlib --crate-name=kernels kernels.rs -o "$arm/libkernels.rlib"
 rustc +1.94.1 "${flags[@]}" driver.rs --extern=kernels="$arm/libkernels.rlib" -o "$arm/driver"
 "$arm/driver" > "$arm/stdout"
 (cd "$arm/objects" && ar x ../libkernels.rlib)
 objdump -d "$arm"/objects/*.o > "$arm/disassembly"
 nm -S --size-sort "$arm"/objects/*.o > "$arm/symbols"
done
python3 - <<'PY'
from pathlib import Path
import json,re
rows={}
for a in ['llvm','baseline','instcombine-v2']:
 data={}
 for l in Path(a+'/symbols').read_text().splitlines():
  z=l.split()
  if len(z)==4 and z[2]=='T':data[z[3]]=int(z[1],16)
 rows[a]=data
print(json.dumps(rows,indent=2));Path('summary.json').write_text(json.dumps(rows,indent=2)+'\n')
assert sum(rows['instcombine-v2'].values()) < sum(rows['baseline'].values())
PY

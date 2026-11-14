#!/usr/bin/env bash
# Build one LLVM std with MIR optimization disabled for both compiler arms.
set -euo pipefail
out=${1:?fresh output directory}
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
target=x86_64-unknown-linux-gnu
if [ -e "$out" ]; then printf 'refusing existing output %s\n' "$out" >&2; exit 2; fi
mkdir -p "$out/project/src"
out=$(realpath "$out")
cat > "$out/project/Cargo.toml" <<'TOML'
[package]
name = "kfrc-shared-std-anchor"
version = "0.0.0"
edition = "2021"
[workspace]
[profile.release]
panic = "abort"
opt-level = 3
codegen-units = 16
lto = false
debug = 0
incremental = false
TOML
printf '#![no_std]\npub fn anchor() {}\n' > "$out/project/src/lib.rs"
original_sysroot=$(rustc +"$toolchain" --print sysroot)
rustc +"$toolchain" -Vv > "$out/toolchain.txt"
command=(env -u KFRC_PROFILE -u KFRC_PROFILE_TOTAL -u KFRC_LLVM_PROFILE -u KFRC_OPTIMIZE -u KFRC_INLINE RUSTC_BOOTSTRAP=1 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" CARGO_ENCODED_RUSTFLAGS=$'-Zmir-opt-level=0\x1f-Cpanic=abort' cargo +"$toolchain" build --manifest-path "$out/project/Cargo.toml" --release --target "$target" -Zbuild-std=std,panic_abort)
printf '%q ' "${command[@]}" > "$out/build.command"
printf '\n' >> "$out/build.command"
status=0
"${command[@]}" > "$out/build.log" 2>&1 || status=$?
printf '{"exit_code":%s}\n' "$status" > "$out/build.result.json"
if [ "$status" != 0 ]; then tail -100 "$out/build.log" >&2; exit "$status"; fi
libdir="$out/sysroot/lib/rustlib/$target/lib"
mkdir -p "$libdir"
# Copy only the newly built Rust libraries. Keeping the prebuilt rlibs alongside
# these can silently select the wrong std or yield duplicate-crate errors.
find "$out/project/target/$target/release/deps" -maxdepth 1 -name '*.rlib' ! -name 'libkfrc_shared_std_anchor*' -exec cp '{}' "$libdir/" ';'
find "$original_sysroot/lib/rustlib/$target/lib" -maxdepth 1 -name '*.o' -exec cp '{}' "$libdir/" ';'
python3 - "$out" "$original_sysroot" <<'PY'
from pathlib import Path
import hashlib,json,sys
out=Path(sys.argv[1]);sysroot=out/'sysroot'
libs=sorted((sysroot/'lib/rustlib/x86_64-unknown-linux-gnu/lib').glob('*'))
assert any(p.name.startswith('libstd-') for p in libs)
assert any(p.name.startswith('libcore-') for p in libs)
assert any(p.name.startswith('liballoc-') for p in libs)
record={'scope':'shared LLVM std, opt-level=3, MIR opt-level=0, panic=abort, CGU=16', 'original_sysroot':sys.argv[2], 'sysroot':str(sysroot), 'libraries':{str(p.relative_to(sysroot)):hashlib.sha256(p.read_bytes()).hexdigest() for p in libs if p.is_file()}}
(out/'manifest.json').write_text(json.dumps(record,indent=2)+'\n')
PY
printf '%s\n' "$out/sysroot"

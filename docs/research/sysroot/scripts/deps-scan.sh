#!/usr/bin/env bash
# 逐 crate 统计依赖闭包对编译器的额外需求。输入是 deps-closure.sh 的输出；源码只做文本读取。
#
# 每个 crate 扫描 src/（没有 src/ 时扫描整个目录，均排除 tests/benches/examples/fuzz）与 build.rs。
# 输出列：
#   role name ver pm build links rs_lines unsafe arch_lines x86_fns(llvm/asm/simd/other)
#   tf_attrs detect asm global_asm repr_simd features(always/cfg) rustc_probe c_src custom_cfgs
# 其中 x86_fns 按 llvm-asm-arch.sh list 的分类表统计“该 crate 引用的不同 core::arch x86 函数”。
# rustc_probe：build.rs 以 "RUSTC" 为参数读环境变量，或使用 autocfg/rustc_version/version_check 探测编译器。
# custom_cfgs：cfg(...) 中出现、且不属于 rustc 内建名称的 cfg 名（多由 build.rs 的 rustc-cfg 设置）。
#
# 用法：deps-scan.sh CLOSURE_FILE X86_FNS_FILE [summary|x86]
#   summary  只输出合计行
#   x86      输出每个 crate 引用的 x86 intrinsic 明细（crate 名称 实现方式）
set -euo pipefail
export LC_ALL=C
closure=${1:?usage: deps-scan.sh CLOSURE_FILE X86_FNS_FILE [summary|x86]}
fns=${2:?usage: deps-scan.sh CLOSURE_FILE X86_FNS_FILE [summary|x86]}
mode=${3:-table}
tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT
[ "$mode" = x86 ] && { x86out=$tmp/x86all; : > "$x86out"; }
awk '{print $2, $3}' "$fns" | sort -u > "$tmp/fns"

builtin_cfgs='^(target_arch|target_os|target_family|target_env|target_abi|target_vendor|target_endian|target_pointer_width|target_feature|target_has_atomic|target_has_atomic_load_store|target_has_atomic_equal_alignment|target_thread_local|panic|unix|windows|test|doc|doctest|debug_assertions|miri|feature|all|any|not|proc_macro|overflow_checks|relocation_model|sanitize|fmt_debug|ub_checks|contract_checks|emscripten_wasm_eh|target_has_reliable_f16|target_has_reliable_f128|clippy|rustfmt|docsrs|rustdoc|loom|kani|fuzzing|nightly)$'

scan_one() {
  local dir=$1 files
  if [ -d "$dir/src" ]; then files=$(find "$dir/src" -name '*.rs'); else files=$(find "$dir" -name '*.rs'); fi
  files=$(echo "$files" | grep -v -E '/(tests|benches|examples|fuzz|test)/' || true)
  [ -f "$dir/build.rs" ] && files="$files
$dir/build.rs"
  echo "$files" | grep -v '^$' > "$tmp/files" || true
  if [ ! -s "$tmp/files" ]; then echo "0 0 0 0/0/0/0 0 0 0 0 0 0/0 -"; return; fi
  # shellcheck disable=SC2046
  xargs cat < "$tmp/files" | grep -v -E '^[[:space:]]*//' > "$tmp/src" || true
  local lines unsafe archl tfa det asm gasm rsimd fa fc
  lines=$(wc -l < "$tmp/src" | tr -d ' ')
  unsafe=$(grep -oE '\bunsafe\b' "$tmp/src" | wc -l | tr -d ' ')
  archl=$(grep -cE '(core|std)::arch::' "$tmp/src" || true)
  tfa=$(grep -cE '#\[target_feature\(' "$tmp/src" || true)
  det=$(grep -cE 'is_x86_feature_detected!|is_[a-z0-9_]+_feature_detected!' "$tmp/src" || true)
  asm=$(grep -cE '\b(asm|naked_asm)!' "$tmp/src" || true)
  gasm=$(grep -cE '\bglobal_asm!' "$tmp/src" || true)
  rsimd=$(grep -cE 'repr\((C, )?simd' "$tmp/src" || true)
  fa=$(grep -cE '^[[:space:]]*#!\[feature\(' "$tmp/src" || true)
  fc=$(grep -cE '^[[:space:]]*#!\[cfg_attr\(.*feature\(' "$tmp/src" || true)
  # 引用到的 x86 intrinsic 名称（_mm*、_bit*、__cpuid 等，以分类表为准）
  grep -oE '\b_{1,2}[A-Za-z0-9_]+\b' "$tmp/src" | sort -u > "$tmp/ids" || true
  local xk
  join "$tmp/ids" "$tmp/fns" 2>/dev/null | sort -u > "$tmp/x86used" || true
  if [ -n "${x86out:-}" ]; then awk -v c="$2" '{print c, $1, $2}' "$tmp/x86used" >> "$x86out"; fi
  xk=$(awk '{c[$2]++} END {printf "%d/%d/%d/%d", c["llvm"], c["asm"], c["simd"], c["other"]}' "$tmp/x86used")
  local cfgs
  # cfg 名：#[cfg(...)] 与 cfg!(...) 的谓词，去掉字符串值后取标识符，排除内建名
  cfgs=$({ grep -oE '#!?\[cfg\(([^()]|\(([^()]|\([^()]*\))*\))*\)\]|cfg!\(([^()]|\([^()]*\))*\)' "$tmp/src" || true; } | sed -E 's/"[^"]*"//g; s/[$][A-Za-z_]+//g' |
         { grep -oE '[A-Za-z_][A-Za-z0-9_]*' || true; } | sort -u |
         { grep -v -E "$builtin_cfgs|^cfg$" || true; } | tr '\n' ',' | sed 's/,$//')
  echo "$lines $unsafe $archl $xk $tfa $det $asm $gasm $rsimd $fa/$fc ${cfgs:--}"
}

while read -r role name ver pm dir; do
  build=-; links=-; probe=-; csrc=0
  if [ -f "$dir/Cargo.toml" ]; then
    if [ -f "$dir/build.rs" ] || grep -qE '^build = "' "$dir/Cargo.toml"; then
      grep -qE '^build = false' "$dir/Cargo.toml" || build=yes
    fi
    l=$(grep -E '^links = "' "$dir/Cargo.toml" | sed -E 's/links = "([^"]*)"/\1/' | head -1 || true)
    [ -n "$l" ] && links=$l
  fi
  if [ "$build" = yes ] && [ -f "$dir/build.rs" ]; then
    grep -qE '\("RUSTC"\)|autocfg|rustc_version|version_check' "$dir/build.rs" && probe=yes
  fi
  csrc=$(find "$dir" \( -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.S' -o -name '*.s' \) 2>/dev/null | wc -l | tr -d ' ')
  echo "$role $name $ver $pm $build $links $(scan_one "$dir" "$name") $probe $csrc"
done < "$closure" |
  awk '{ # 把 rustc_probe 与 c_src 挪到 custom_cfgs 之前，便于阅读
    n = NF; cf = $(n - 2); pr = $(n - 1); cs = $n
    out = $1; for (i = 2; i <= n - 3; i++) out = out " " $i
    print out, pr, cs, cf }' > "$tmp/table"

if [ "$mode" = table ]; then
  echo "# role name ver pm build links rs_lines unsafe arch_lines x86_fns(l/a/s/o) tf_attrs detect asm global_asm repr_simd features(a/c) rustc_probe c_src custom_cfgs"
  cat "$tmp/table"
fi
echo "# 合计（role=target 与 both 计入 target 侧；host 单列）"
awk '{
  side = ($1 == "host") ? "host" : "target"
  n[side]++; L[side] += $7; U[side] += $8
  if ($4 == "yes") pm[side]++
  if ($5 == "yes") b[side]++
  if ($6 != "-") lk[side]++
  if ($9 > 0) ar[side]++
  if ($11 > 0) tf[side]++
  if ($13 > 0) as[side]++
  if ($15 > 0) rs[side]++
  split($16, f, "/"); if (f[1] > 0) fa[side]++; if (f[2] > 0) fc[side]++
  if ($17 == "yes") pr[side]++
  if ($18 > 0) cs[side]++
  if ($19 != "-") cc[side]++
  if ($2 ~ /-sys$/) sy[side]++
}
END {
  for (s in n)
    printf "%s crates=%d rs_lines=%d unsafe=%d proc_macro=%d build_rs=%d links=%d sys=%d core_arch=%d target_feature=%d asm=%d repr_simd=%d feature_always=%d feature_cfg=%d rustc_probe=%d c_src=%d custom_cfg=%d\n",
      s, n[s], L[s], U[s], pm[s], b[s], lk[s], sy[s], ar[s], tf[s], as[s], rs[s], fa[s], fc[s], pr[s], cs[s], cc[s]
}' "$tmp/table" | sort
if [ "$mode" = x86 ]; then
  echo "# 每个 crate 引用的 x86 intrinsic：crate 名称 实现方式"
  sort -u "$tmp/x86all"
fi
if [ "$mode" = x86 ]; then
  echo "# 合计：不同 x86 intrinsic 名称（全部 crate 取并集）：total llvm asm simd other"
  awk '{k[$2] = $3} END {for (n in k) {t++; c[k[n]]++}; printf "%d %d %d %d %d\n", t, c["llvm"], c["asm"], c["simd"], c["other"]}' "$tmp/x86all"
  echo "# 需要自实现的 LLVM intrinsic 名称（实现方式为 llvm 的 x86 函数）"
  awk '$3 == "llvm" {print $2}' "$tmp/x86all" | sort -u | tr '\n' ' '; echo
fi

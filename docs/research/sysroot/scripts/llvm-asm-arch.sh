#!/usr/bin/env bash
# core::arch::x86/x86_64 的平台 intrinsic 清单，以及 sysroot 中直接调用 LLVM intrinsic
# （#[link_name = "llvm.*"]）、asm!/global_asm!/naked_asm! 的使用点。
#
# x86 公开函数按实现方式分类（函数体内首先命中的优先级 llvm > asm > simd > other）：
#   llvm   调用本文件 extern 块中 link_name = "llvm.*" 声明的函数（kfrc 无 LLVM，需自实现）
#   asm    函数体含 asm!
#   simd   只用通用 simd_* intrinsic / simd_shuffle!（kfrc 后端要实现通用 SIMD 语义）
#   other  其余（标量 Rust、转调其他 _mm*、transmute 等）
# 每个文件在 `mod tests` 处截断；注释行不计。
#
# 用法：llvm-asm-arch.sh LIBRARY_DIR [list]
#   list  只输出逐函数表（文件 名称 实现方式 target_feature），供 deps-scan.sh 分类使用
# 注：Homebrew 的 rust-src 里有一份重复的 library/library 目录，递归 grep 时排除。
set -euo pipefail
lib=${1:?usage: llvm-asm-arch.sh LIBRARY_DIR [list]}
mode=${2:-summary}
arch=$lib/stdarch/crates/core_arch/src
x86files=$(find "$arch/x86" "$arch/x86_64" -name '*.rs' ! -name test.rs | sort)

classify() {
  awk '
    FNR == 1 { pass++ }
    /^[[:space:]]*\/\// { next }
    pass == 1 {
      if ($0 ~ /link_name = "llvm\./) want = 1
      else if (want && match($0, /fn [A-Za-z_0-9]+/)) { llvm[substr($0, RSTART + 3, RLENGTH - 3)] = 1; want = 0 }
      next
    }
    /^[[:space:]]*(pub )?mod tests/ { stop = 1 }
    stop { next }
    /#\[target_feature\(enable = "/ { tf = $0; sub(/.*enable = "/, "", tf); sub(/".*/, "", tf) }
    /^[[:space:]]*pub (const )?(unsafe )?fn _/ {
      emit()
      match($0, /fn _[A-Za-z_0-9]+/); cur = substr($0, RSTART + 3, RLENGTH - 3); curtf = tf; tf = ""
      kind = ""; next
    }
    cur != "" {
      t = $0
      while (match(t, /[A-Za-z_0-9]+\(/)) {
        w = substr(t, RSTART, RLENGTH - 1)
        if (w in llvm) hasl = 1
        t = substr(t, RSTART + RLENGTH)
      }
      if ($0 ~ /asm!/) hasa = 1
      if ($0 ~ /simd_[a-z_0-9]+(!|\(|::)/) hass = 1
    }
    function emit() {
      if (cur == "") return
      k = hasl ? "llvm" : (hasa ? "asm" : (hass ? "simd" : "other"))
      print FILENAME, cur, k, (curtf == "" ? "-" : curtf)
      cur = ""; hasl = hasa = hass = 0
    }
    END { emit() }' "$1" "$1"
}

tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT
for f in $x86files; do classify "$f"; done | sed "s|$arch/||" > "$tmp/fns"
if [ "$mode" = list ]; then cat "$tmp/fns"; exit 0; fi

echo "# core::arch x86/x86_64 公开 intrinsic 函数总数: $(wc -l < "$tmp/fns" | tr -d ' ')（不同名称 $(awk '{print $2}' "$tmp/fns" | sort -u | wc -l | tr -d ' ')）"
echo "# 按实现方式：kind count"
awk '{c[$3]++} END {for (k in c) print k, c[k]}' "$tmp/fns" | sort -k2,2nr
echo "# 按 target_feature（取 enable 的第一个特性）：feature total llvm asm simd other"
awk '{f = $4; sub(/,.*/, "", f); t[f]++; v[f, $3]++}
     END {for (f in t) print f, t[f], v[f, "llvm"] + 0, v[f, "asm"] + 0, v[f, "simd"] + 0, v[f, "other"] + 0}' "$tmp/fns" |
  sort -k2,2nr
echo "# baseline x86_64（sse/sse2，含 x86_64/ 下同名特性）: $(awk '{f = $4; sub(/,.*/, "", f)} f == "sse" || f == "sse2"' "$tmp/fns" | wc -l | tr -d ' ')"

echo "# link_name = \"llvm.*\"：区域 声明数 不同名称数"
for d in x86 x86_64; do
  n=$(grep -rhE 'link_name = "llvm\.' "$arch/$d" | wc -l | tr -d ' ')
  u=$(grep -rhoE 'link_name = "llvm\.[^"]+' "$arch/$d" | sort -u | wc -l | tr -d ' ')
  echo "core_arch/$d $n $u"
done
n=$(grep -rhE 'link_name = "llvm\.' "$arch" | wc -l | tr -d ' ')
echo "core_arch/全部架构 $n -"
others=$(grep -rlE 'link_name = "llvm\.' "$lib" --include='*.rs' | grep -v -E "^$lib/library/|/stdarch/|/tests?/" || true)
echo "core_arch 之外: $(echo "$others" | grep -c . || true) 个文件 ${others//$lib\//}"
echo "# x86+x86_64 的 llvm 名称按族（llvm.x86.<族>）：family count"
grep -rhoE 'link_name = "llvm\.[^"]+' "$arch/x86" "$arch/x86_64" | sort -u |
  sed -E 's/link_name = "llvm\.//' | awk -F. '{f = ($1 == "x86") ? "x86." $2 : $1; c[f]++} END {for (k in c) print k, c[k]}' |
  sort -k2,2nr | head -25

echo "# asm!/global_asm!/naked_asm! 使用点（x86_64 Linux 相关文件；按 crate/文件）"
{
  find "$arch/x86" "$arch/x86_64" "$lib/core/src" "$lib/alloc/src" "$lib/std/src" "$lib/std_detect/src" \
       "$lib/unwind/src" "$lib/panic_unwind/src" "$lib/panic_abort/src" \
       "$lib/compiler-builtins/compiler-builtins/src" -name '*.rs'
  [ -d "$lib/backtrace/src" ] && find "$lib/backtrace/src" -name '*.rs'
} | grep -v -E '/tests?(/|\.rs$)|/benches/' |
  xargs grep -cE '^[^/]*\b(asm|global_asm|naked_asm)!' 2>/dev/null | grep -v ':0$' | sed "s|$lib/||" |
  awk -F: '{
    f = $1; x = (f ~ /x86|i686|i386/) ? "x86" : ((f ~ /aarch64|arm|riscv|wasm|mips|powerpc|s390|loongarch|sparc|hexagon|avr|msp430|nvptx|xtensa|solid|sgx|uefi|hermit|xous|zkvm|teeos|vexos|trusty|motor|windows|uwp|wasi|cuda|bpf|m68k|csky/) ? "other" : "generic")
    print $2, x, f }' | sort -k3,3 > "$tmp/asm"
awk '{c[$2] += $1; n[$2]++} END {for (k in c) print "合计", k, "文件", n[k], "使用点", c[k]}' "$tmp/asm" | sort
awk '$2 != "other" {print $1, $2, $3}' "$tmp/asm"
echo "# 按顶层目录合计（不分架构）：dir files sites"
awk '{split($3, p, "/"); d = p[1]; if (d == "stdarch") d = "stdarch/core_arch-" p[5]; f[d]++; s[d] += $1}
  END {for (k in f) print k, f[k], s[k]}' "$tmp/asm" | sort

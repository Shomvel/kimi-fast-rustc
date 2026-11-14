#!/usr/bin/env bash
# 列出 LLVM 后端（legacy PM，addPassesToEmitFile）实际运行的 pass 结构。
#
# 1. rustc 自身：-Zprint-llvm-passes 等价于 -Cllvm-args=-debug-pass=Structure（llvm_util.rs），
#    宿主为 aarch64-apple-darwin，按 -Copt-level 0..3 各跑一次（CGU=1、LTO 关）。
# 2. x86-64：本机 rustc 只有 aarch64 标准库，改用同版本 llc 对一个小 IR 文件取
#    -mtriple=x86_64-unknown-linux-gnu 的结构（-O0 与 -O3，PIC、function/data sections，
#    与 rustc 的 TargetMachine 选项一致的部分）。pass 结构不依赖 IR 内容。
#
# 用法：codegen-passes.sh WORKDIR [RUSTC] [LLC]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${1:?usage: codegen-passes.sh WORKDIR [RUSTC] [LLC]}
rustc=${2:-rustc}
llc=${3:-llc}
"$here/sample-crate.sh" "$work"
out=$work/codegen
mkdir -p "$out"
rm -f "$out"/*
"$llc" --version | head -2 > "$out/llc-version.txt"

largest_block() { # STRUCTURE_FILE -> 行数最多的一段 "Pass Arguments" 结构（rustc 还会为分配器 shim 模块
  # 以 O0 打印一段，另有几段只含 domtree 等分析的小段）
  local f=$1 starts best=0 bs=0 be=0 prev=0 total
  total=$(wc -l < "$f")
  starts=$(grep -n "^Pass Arguments" "$f" | cut -d: -f1; echo $((total + 1)))
  for s in $starts; do
    if [ "$prev" -gt 0 ] && [ $((s - prev)) -gt "$best" ]; then best=$((s - prev)); bs=$prev; be=$((s - 1)); fi
    prev=$s
  done
  sed -n "${bs},${be}p" "$f"
}

passlist() { # STRUCTURE_FILE -> 最大一段里去掉缩进的 pass 名（保持顺序，去掉不运行的不可变分析）
  largest_block "$1" | grep -E "^ {4,}[A-Za-z]" | grep -v "Pass Manager$" | sed -E "s/^ +//" || true
}

blocks() { grep -c "^Pass Arguments" "$1" || true; }

for o in 0 1 2 3; do
  RUSTC_BOOTSTRAP=1 "$rustc" --edition=2021 --crate-type=bin --crate-name sample "$work/src/main.rs" \
    -o "$out/rustc-O$o.bin" -Copt-level=$o -Ccodegen-units=1 -Clto=off -Zprint-llvm-passes \
    > /dev/null 2> "$out/rustc-O$o.structure.txt"
  passlist "$out/rustc-O$o.structure.txt" > "$out/rustc-O$o.passes.txt"
done

cat > "$out/tiny.ll" <<'LL'
define i64 @f(ptr %p, i64 %n) {
entry:
  br label %loop
loop:
  %i = phi i64 [ 0, %entry ], [ %i1, %loop ]
  %s = phi i64 [ 0, %entry ], [ %s1, %loop ]
  %a = getelementptr i64, ptr %p, i64 %i
  %v = load i64, ptr %a
  %s1 = add i64 %s, %v
  %i1 = add i64 %i, 1
  %c = icmp ult i64 %i1, %n
  br i1 %c, label %loop, label %exit
exit:
  ret i64 %s1
}
LL
for o in 0 1 2 3; do
  "$llc" -mtriple=x86_64-unknown-linux-gnu -O$o -relocation-model=pic -function-sections -data-sections \
    -filetype=obj -o /dev/null -debug-pass=Structure "$out/tiny.ll" 2> "$out/llc-x86-O$o.structure.txt"
  passlist "$out/llc-x86-O$o.structure.txt" > "$out/llc-x86-O$o.passes.txt"
done

{
  printf 'config\tblocks\tpasses\tisel\tregalloc\tsched\n'
  for f in "$out"/*.passes.txt; do
    n=${f##*/}
    n=${n%.passes.txt}
    isel=$(grep -E 'IRTranslator|Legalizer|RegBankSelect|InstructionSelect|DAG->DAG|FastISel|Instruction Selection' "$f" | sort -u | paste -sd';' - || true)
    ra=$(grep -E 'Register Allocator' "$f" | sort -u | paste -sd';' - || true)
    sc=$(grep -E 'Scheduler|scheduler' "$f" | sort -u | paste -sd';' - || true)
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$n" "$(blocks "$out/$n.structure.txt")" "$(grep -c . "$f")" "$isel" "$ra" "$sc"
  done
} > "$out/summary.tsv"
echo "done: $out"

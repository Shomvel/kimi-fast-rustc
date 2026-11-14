#!/usr/bin/env bash
# 导出 rustc 实际交给 LLVM new PM 的优化流水线，并按 opt-level × CGU × LTO 汇总。
#
# 机制：PassWrapper.cpp 的 LLVMRustOptimize 在 PrintPasses=true 时打印 MPM.printPipeline()，
# 每调用一次（每个模块、每个阶段）打印一行；该参数由 -Zautodiff=PrintPasses 打开，
# 只打印、不启用 autodiff。稳定版 rustc 需要 RUSTC_BOOTSTRAP=1 才接受 -Z。
# -Cllvm-args=-print-pipeline-passes 由 opt 驱动处理，rustc 里没有输出；
# -Cllvm-args=-debug-pass-manager 在 rustc 里是未知参数。
# 多个 LLVM worker 并发写 stdout 会把行打碎，所以加 -Zno-parallel-backend（串行跑 LLVM，
# 保留 CGU 与 ThinLTO，不改变流水线）。
#
# 用法：pipelines.sh WORKDIR [RUSTC]
# 输出：WORKDIR/out/{labels.tsv,matrix.tsv,counts.tsv,pass-table.tsv,diff-*.txt,pipeline-*.txt,tree-*.txt}
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${1:?usage: pipelines.sh WORKDIR [RUSTC]}
rustc=${2:-rustc}
"$here/sample-crate.sh" "$work"
raw=$work/raw
out=$work/out
mkdir -p "$raw" "$out"
rm -f "$raw"/* "$out"/*
"$rustc" -vV > "$out/rustc-version.txt"

run() { # NAME FLAGS...
  local name=$1
  shift
  RUSTC_BOOTSTRAP=1 "$rustc" --edition=2021 --crate-type=bin --crate-name sample \
    "$work/src/main.rs" -o "$raw/$name.bin" -Zautodiff=PrintPasses -Zno-parallel-backend "$@" \
    > "$raw/$name.txt" 2> "$raw/$name.err"
}

hash_lines() { # FILE -> 每行流水线一个短哈希
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    printf '%s' "$line" | shasum -a 256 | cut -c1-12
  done < "$1"
}

nth_line() { sed -n "${2}p" "$1"; }

label_ref() { # RUNNAME LABEL1 [LABEL2]：单模块参考运行，第 i 行流水线记为 LABELi
  local name=$1
  shift
  local n
  n=$(grep -c . "$raw/$name.txt" || true)
  if [ "$n" -ne "$#" ]; then
    echo "warn: $name printed $n pipelines, expected $#" >&2
  fi
  local i=1
  for label in "$@"; do
    local h
    h=$(nth_line "$raw/$name.txt" "$i" | tr -d '\n' | shasum -a 256 | cut -c1-12)
    printf '%s\t%s\n' "$h" "$label" >> "$out/labels.raw"
    [ -f "$out/pipeline-$label.txt" ] || nth_line "$raw/$name.txt" "$i" > "$out/pipeline-$label.txt"
    i=$((i + 1))
  done
}

# 1. 参考运行：CGU=1 时只有一个模块，打印顺序确定（先 pre-link，后 LTO 阶段）。
for o in 0 1 2 3 s z; do
  run "ref-O$o-nolto" -Copt-level=$o -Ccodegen-units=1
  run "ref-O$o-thinlocal" -Copt-level=$o -Ccodegen-units=1 -Zthinlto=yes
  run "ref-O$o-fat" -Copt-level=$o -Ccodegen-units=1 -Clto=fat
  if [ "$o" = 0 ]; then
    label_ref "ref-O$o-nolto" "O0"
    label_ref "ref-O$o-thinlocal" "ThinPre-O0" "ThinPost-O0"
    label_ref "ref-O$o-fat" "FatPre-O0" "Fat-O0"
  else
    label_ref "ref-O$o-nolto" "PerModule-O$o"
    label_ref "ref-O$o-thinlocal" "ThinPre-O$o" "ThinPost-O$o"
    label_ref "ref-O$o-fat" "FatPre-O$o" "Fat-O$o"
  fi
done
# 同一哈希可能有多个标签（流水线文本相同），合并为 A=B。
cut -f1 "$out/labels.raw" | sort -u | while read -r h; do
  printf '%s\t%s\n' "$h" "$(grep "^$h" "$out/labels.raw" | cut -f2 | sort -u | paste -sd= -)"
done > "$out/labels.tsv"

label_of() { grep "^$1" "$out/labels.tsv" | cut -f2 || true; }

compose() { # RUNNAME -> "16×ThinPre-O3 + 16×ThinPost-O3"
  hash_lines "$raw/$1.txt" | while read -r h; do
    l=$(label_of "$h")
    echo "${l:-?$h}"
  done | sort | uniq -c | sed -E 's/^ *([0-9]+) (.*)$/\1×\2/' | paste -sd'+' - | sed 's/+/ + /g'
}

# 2. 矩阵：opt-level × CGU × LTO。
printf 'config\tpipelines\tcomposition\n' > "$out/matrix.tsv"
for o in 0 1 2 3 s z; do
  for cgu in 1 16; do
    for lto in default off thin fat; do
      name="m-O$o-cgu$cgu-$lto"
      if [ "$lto" = default ]; then
        run "$name" -Copt-level=$o -Ccodegen-units=$cgu
      else
        run "$name" -Copt-level=$o -Ccodegen-units=$cgu -Clto=$lto
      fi
      printf 'O%s cgu=%s lto=%s\t%s\t%s\n' "$o" "$cgu" "$lto" \
        "$(grep -c . "$raw/$name.txt" || true)" "$(compose "$name")" >> "$out/matrix.tsv"
    done
  done
done
# --emit=llvm-ir 不写 -Ccodegen-units（config.rs：强制 CGU=1 并关闭本地 ThinLTO）。
run "m-O3-emit-ir" -Copt-level=3 --emit=llvm-ir,link
printf 'O3 --emit=llvm-ir,link\t%s\t%s\n' "$(grep -c . "$raw/m-O3-emit-ir.txt" || true)" \
  "$(compose m-O3-emit-ir)" >> "$out/matrix.tsv"
# 显式 -Ccodegen-units=16 且带 -o：rustc 警告并重置为 1。
run "m-O3-emit-ir-cgu16" -Copt-level=3 -Ccodegen-units=16 --emit=llvm-ir,link
printf 'O3 cgu=16 --emit=llvm-ir,link -o\t%s\t%s\n' "$(grep -c . "$raw/m-O3-emit-ir-cgu16.txt" || true)" \
  "$(compose m-O3-emit-ir-cgu16)" >> "$out/matrix.tsv"
grep -h 'codegen-units' "$raw/m-O3-emit-ir-cgu16.err" > "$out/emit-ir-cgu16-warning.txt" || true

# 3. 每条流水线的叶子 pass 计数、缩进大纲。
sep='~'
tokens() { # PIPELINE_FILE -> 每行一个记号："name(" 开、")" 闭、其余为叶子 pass
  if grep -q "$sep" "$1"; then echo "separator $sep occurs in $1" >&2; exit 1; fi
  sed -e 's/require<\([^>]*\)>/require:\1/g' -e 's/invalidate<\([^>]*\)>/invalidate:\1/g' \
      -e 's/<[^>]*>//g' -e "s/(/($sep/g" -e "s/)/$sep)$sep/g" "$1" \
    | tr ",$sep" '\n\n' | grep -v '^$'
}
printf 'label\tpass\tcount\n' > "$out/counts.tsv"
for f in "$out"/pipeline-*.txt; do
  label=${f##*/pipeline-}
  label=${label%.txt}
  depth=0
  tokens "$f" | while IFS= read -r t; do
    case "$t" in
      ')') depth=$((depth - 1)) ;;
      *'(') printf '%*s%s\n' $((depth * 2)) '' "${t%(}"; depth=$((depth + 1)) ;;
      *) printf '%*s%s\n' $((depth * 2)) '' "$t" ;;
    esac
  done > "$out/tree-$label.txt"
  tokens "$f" | grep -v '($' | grep -v '^)$' | sort | uniq -c \
    | sed -E "s/^ *([0-9]+) (.*)$/$label	\2	\1/" >> "$out/counts.tsv"
done

count_of() { # LABEL PASS
  local c
  c=$(grep "^$1	$2	" "$out/counts.tsv" | cut -f3 || true)
  echo "${c:-0}"
}
total_of() { grep "^$1	" "$out/counts.tsv" | cut -f3 | paste -sd+ - | bc; }

cols="O0 PerModule-O1 ThinPre-O1 ThinPost-O1 PerModule-O3 ThinPre-O3 ThinPost-O3 FatPre-O3 Fat-O3 ThinPre-Os ThinPost-Oz"
passes="sroa mem2reg instcombine aggressive-instcombine instsimplify simplifycfg early-cse gvn newgvn sccp ipsccp
inline always-inline function-attrs argpromotion deadargelim globalopt globaldce elim-avail-extern
mergefunc constmerge jump-threading correlated-propagation constraint-elimination dse memcpyopt
mldst-motion licm loop-rotate indvars loop-idiom loop-deletion loop-unroll loop-unroll-full
simple-loop-unswitch loop-vectorize slp-vectorizer vector-combine loop-distribute loop-load-elim
reassociate adce bdce tailcallelim div-rem-pairs float2int lower-expect called-value-propagation"
{
  printf 'pass'
  for c in $cols; do printf '\t%s' "$c"; done
  printf '\n'
  printf 'leaf-total'
  for c in $cols; do printf "\t%s" "$(total_of "$c")"; done
  printf '\n'
  for p in $passes; do
    printf '%s' "$p"
    for c in $cols; do printf '\t%s' "$(count_of "$c" "$p")"; done
    printf '\n'
  done
} > "$out/pass-table.tsv"

leafset() { grep "^$1	" "$out/counts.tsv" | cut -f2 | sort -u; }
comm -13 <(leafset ThinPre-O3) <(leafset ThinPost-O3) > "$out/diff-thinpost-only-O3.txt"
comm -23 <(leafset ThinPre-O3) <(leafset ThinPost-O3) > "$out/diff-thinpre-only-O3.txt"
comm -3 <(leafset PerModule-O3) <(cat <(leafset ThinPre-O3) <(leafset ThinPost-O3) | sort -u) > "$out/diff-permodule-vs-thin-O3.txt"
comm -3 <(leafset ThinPre-O2) <(leafset ThinPre-O3) > "$out/diff-thinpre-O2-vs-O3.txt"
comm -3 <(leafset ThinPost-O2) <(leafset ThinPost-O3) > "$out/diff-thinpost-O2-vs-O3.txt"
comm -3 <(leafset ThinPost-O3) <(leafset ThinPost-Oz) > "$out/diff-thinpost-O3-vs-Oz.txt"
echo "done: $out"

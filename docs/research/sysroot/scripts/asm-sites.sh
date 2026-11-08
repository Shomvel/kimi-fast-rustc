#!/usr/bin/env bash
# 依赖闭包中 asm!/naked_asm!/global_asm! 的逐文件使用点，按路径粗分架构：
#   x86      路径含 x86/x86_64/amd64
#   other    路径含其他架构或非 Linux 平台名
#   generic  其余（多为同一文件内用 cfg(target_arch) 区分，需人工核对）
# 同时打印 generic 文件中 asm 附近出现的 target_arch 值，辅助判断 x86_64 是否会编译。
#
# 用法：asm-sites.sh CLOSURE_FILE
set -euo pipefail
closure=${1:?usage: asm-sites.sh CLOSURE_FILE}
other_re='aarch64|arm|riscv|mips|powerpc|s390|loongarch|sparc|wasm|windows|apple|darwin|macos|freebsd|netbsd|openbsd|solaris|illumos|android|hexagon|msp430|avr|m68k|csky|xtensa|thumb|i686|x86/|_x86\.rs|x86_32|bpf|sgx|xous|vexos|teeos|fortanix|hermit|uefi|solid|zkvm|trusty'
echo "# crate file arch count [generic 文件中出现的 target_arch 值]"
while read -r role name ver pm dir <&3; do
  [ "$role" = host ] && continue
  base=$dir; [ -d "$dir/src" ] && base=$dir/src
  find "$base" -name '*.rs' 2>/dev/null | grep -v -E '/(tests|benches|examples|fuzz)/' |
    { xargs grep -cHE '^[^/]*\b(asm|naked_asm|global_asm)!' 2>/dev/null || true; } | { grep -v ':0$' || true; } |
    while IFS=: read -r f n; do
      rel=${f#"$dir"/}
      if echo "$rel" | grep -qE 'x86_64|amd64'; then a=x86
      elif echo "$rel" | grep -qE "$other_re"; then a=other
      else a=generic; fi
      extra=
      if [ "$a" = generic ]; then
        extra=$({ grep -oE 'target_arch = "[a-z0-9_]+"' "$f" || true; } | sed -E 's/.*"(.*)"/\1/' | sort -u | tr '\n' ',' | sed 's/,$//')
      fi
      echo "$name $rel $a $n ${extra:+[$extra]}"
    done
done 3< "$closure"

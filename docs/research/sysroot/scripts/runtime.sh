#!/usr/bin/env bash
# sysroot 中与“运行时与链接”相关的使用点：panic runtime、分配器 shim、unwind、TLS、链接属性、
# compiler-builtins 导出的符号等。只做文本统计，路径按目标平台粗分（含 windows/wasm/sgx 等
# 目录名的文件记为 other-target，其余记为 linux-candidate，需人工再按 cfg 核对）。
#
# 用法：runtime.sh LIBRARY_DIR
set -euo pipefail
lib=${1:?usage: runtime.sh LIBRARY_DIR}
cd "$lib"

other_re='windows|wasm|wasi|sgx|uefi|hermit|xous|zkvm|teeos|vexos|trusty|motor|solid|itron|apple|darwin|macos|ios|fuchsia|horizon|vita|espidf|emscripten|aix|haiku|nto|qnx|redox|illumos|solaris|freebsd|netbsd|openbsd|dragonfly|android|cygwin|l4re|vxworks|seh|emcc|hexagon|miri|gcc_seh|arm|aarch64|riscv|mips|powerpc|s390|loongarch|sparc|nvptx|avr|msp430|m68k|csky|xtensa|i686|x86/'

files() { find "$@" -name '*.rs' | grep -v -E '/(tests|benches|examples)/|/tests\.rs$'; }
sites() { # pattern dirs...
  local re=$1; shift
  files "$@" | xargs grep -nE -e "$re" 2>/dev/null | grep -v -E '^[^:]+:[0-9]+:[[:space:]]*//' || true
}
report() { # label pattern dirs...
  local label=$1 re=$2; shift 2
  local s
  s=$(sites "$re" "$@")
  local all lin
  all=$(echo "$s" | grep -c . || true)
  lin=$(echo "$s" | cut -d: -f1 | grep -v -E "$other_re" | grep -c . || true)
  echo "$label total=$all linux-candidate=$lin"
}
detail() { # label pattern dirs...  打印 linux-candidate 的使用点
  local label=$1 re=$2; shift 2
  echo "## $label"
  sites "$re" "$@" | awk -F: -v o="$other_re" '$1 !~ o {sub(/^[[:space:]]+/, "", $3); print $1 ":" $2 ": " substr($3, 1, 90)}'
}

rt="core/src alloc/src std/src panic_unwind/src panic_abort/src unwind/src"
echo "# 计数：项目 total linux-candidate"
report 'thread_local 属性' '#\[thread_local\]' $rt
report 'thread_local! 宏使用' 'thread_local! *\{' $rt
report 'linkage 属性' '#\[linkage *=' $rt
report 'link_section' 'link_section *=' $rt
report 'used 属性' '#\[used(\(|\])' $rt
report '#[link(...)] 外部库' '#\[link\(' $rt
report 'weak!/syscall!/dlsym! 弱符号宏调用' '(^|[^a-z_])(weak|syscall|dlsym)! *[({]' $rt
report 'extern "C-unwind"' 'extern "C-unwind"' $rt
report 'rustc_std_internal_symbol' 'rustc_std_internal_symbol' $rt
report 'no_mangle' 'no_mangle' $rt
report 'catch_unwind/r#try 内建' 'intrinsics::catch_unwind' $rt
report '_Unwind_* 引用' '_Unwind_[A-Za-z]+' $rt
report 'global_allocator/alloc_error_handler' '#\[(global_allocator|alloc_error_handler)\]' $rt
report 'naked 函数' '#\[unsafe\(naked\)\]|#\[naked\]' $rt compiler-builtins/compiler-builtins/src

detail '#[link(...)]' '#\[link\(' std/src unwind/src
detail 'link_section / used' 'link_section *=|#\[used(\(|\])' std/src
detail 'linkage' '#\[linkage *=' std/src
detail 'panic runtime 入口与分配器 shim 符号' '__rust_(start_panic|panic_cleanup|alloc|dealloc|realloc|alloc_zeroed|alloc_error_handler|no_alloc_shim_is_unstable|drop_panic|foreign_exception|probestack)' std/src alloc/src panic_unwind/src panic_abort/src

echo "# compiler-builtins：导出函数（intrinsics! 宏内 extern \"C\"/\"unadjusted\" fn __*）按模块"
files compiler-builtins/compiler-builtins/src |
  xargs grep -cE '^[[:space:]]*(pub )?(unsafe )?extern "(C|unadjusted|aapcs|sysv64)" fn __[a-z]' 2>/dev/null | grep -v ':0$' |
  sed 's|compiler-builtins/compiler-builtins/src/||' | awk -F: '{split($1, p, "/"); m = (p[2] == "") ? $1 : p[1]; c[m] += $2; t += $2}
    END {for (k in c) print k, c[k]; print "TOTAL", t}' | sort -k2,2nr
echo "# compiler-builtins：mem 模块（memcpy 等）的启用条件"
grep -nE 'mod mem|feature = "mem"' compiler-builtins/compiler-builtins/src/lib.rs || true
echo "# compiler-builtins：Cargo features 与 links"
sed -n '/^\[features\]/,/^\[/p' compiler-builtins/compiler-builtins/Cargo.toml | grep -E '^[a-z-]+ *=' || true
grep -E '^links' compiler-builtins/compiler-builtins/Cargo.toml || true
echo "# compiler-builtins：crate 级属性"
grep -nE '^#!\[(no_builtins|compiler_builtins|no_std|cfg_attr\(.*compiler_builtins)' compiler-builtins/compiler-builtins/src/lib.rs || true
echo "# unwind：linux 链接方式（libgcc_s / libunwind）"
grep -nE 'link\(name|target_env = "gnu"|libgcc|gcc_s' unwind/src/lib.rs | head -20 || true

# 运行时与链接：panic、unwind、分配器、入口、TLS、compiler-builtins

脚本：[runtime.sh](scripts/runtime.sh)。路径按平台粗分：含 windows、wasm、sgx 等目录名的文件记为其他平台，其余记为“linux-candidate”。下表右列是按 cfg 人工核对的结论。

## 计数

```
# 计数：项目 total linux-candidate
thread_local 属性 total=13 linux-candidate=10
thread_local! 宏使用 total=6 linux-candidate=6
linkage 属性 total=6 linux-candidate=3
link_section total=6 linux-candidate=4
used 属性 total=6 linux-candidate=3
#[link(...)] 外部库 total=62 linux-candidate=61
weak!/syscall!/dlsym! 弱符号宏调用 total=23 linux-candidate=23
extern "C-unwind" total=4 linux-candidate=3
rustc_std_internal_symbol total=29 linux-candidate=26
no_mangle total=16 linux-candidate=0
catch_unwind/r#try 内建 total=1 linux-candidate=1
_Unwind_* 引用 total=169 linux-candidate=152
global_allocator/alloc_error_handler total=1 linux-candidate=1
naked 函数 total=19 linux-candidate=10
```

## 逐项要求

| 机制 | x86_64-unknown-linux-gnu 上的实际情况 | kfrc 要做的事 | 现有任务 |
|---|---|---|---|
| **panic runtime** | `panic_unwind`（`#![panic_runtime]`）导出 `__rust_start_panic`、`__rust_panic_cleanup`；std 声明并调用它们（`std/src/panicking.rs` L59–L82）；`panic_abort` 是替代实现 | 一个产物只能链接一个 panic runtime；`-C panic=abort` 时选 `panic_abort`，并拒绝要求 unwind 的 crate | T27、T33 |
| **unwind 与 personality** | std 在 `sys/personality/gcc.rs` 用 `#[lang = "eh_personality"]` 定义 `rust_eh_personality`，自行解析 LSDA；`unwind` crate 在 gnu 上链接 `gcc_s`（`unwind/src/lib.rs` L142，静态时为 `gcc_eh`）；`_Unwind_*` 引用 152 处 | 每个函数输出 `.eh_frame` CFI；带 landing pad 的函数输出 GCC 格式的 LSDA（`.gcc_except_table`：call-site 表、action 表），格式必须是 std 的 personality 能解析的；`catch_unwind` intrinsic 降成 invoke + 捕获块；`extern "C-unwind"` 与 `rustc_nounwind` 的边界 | T27（只写了“异常表、personality”，没写“与 std 的 personality 实现兼容”） |
| **分配器 shim** | alloc 只**声明** `__rust_alloc`/`__rust_dealloc`/`__rust_realloc`/`__rust_alloc_zeroed`/`__rust_alloc_error_handler`/`__rust_no_alloc_shim_is_unstable_v2`（`alloc/src/alloc.rs` L20–L37、L503）；std 提供默认实现 `__rdl_alloc` 等（`std/src/alloc.rs` L437 起）；有 `#[global_allocator]` 时改用 `__rg_*` | **由编译器在最终产物里生成** shim：`__rust_alloc` → `__rdl_alloc` 或 `__rg_alloc`，`__rust_alloc_error_handler` → `__rdl_alloc_error_handler` 或 `__rg_oom`，并定义 `__rust_no_alloc_shim_is_unstable_v2`（rustc 侧的规格在 [`rustc_ast/src/expand/allocator.rs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast/src/expand/allocator.rs)）；符号按 `#[rustc_std_internal_symbol]` 规则命名 | 缺：T32 只写了“分配器 runtime” |
| **入口** | `#[lang = "start"]` 在 `std/src/rt.rs` L198 | 编译器为 bin crate 生成 C `main(argc, argv)`，调用 `lang_start(main, argc, argv, sigpipe)`；`#![no_main]` 时不生成 | 缺（T10/T33 都没写） |
| **TLS** | `#[thread_local]` 10 处候选（`thread_local!` 的 native 实现 `sys/thread_local/native/mod.rs`、TLS 析构链表、`sys/io/error/unix.rs` 等）；析构通过 `__cxa_thread_atexit_impl`（`extern_weak`，`sys/thread_local/destructors/linux_like.rs` L28–L31） | ELF TLS：可执行文件里用 local-exec/initial-exec，PIC/dylib 用 general-dynamic（`__tls_get_addr`），并生成相应重定位 | T27 写了“TLS”，没写模型与重定位 |
| **弱符号** | `#[linkage = "extern_weak"]` 3 处（`weak_linkage.rs` L9 与上面的 TLS 析构）；`weak!`/`syscall!`/`dlsym!` 宏调用 23 处 | 生成弱未定义符号，取地址可为空 | T19 |
| **init_array** | glibc 专用：`std/src/sys/args/unix.rs` L119–L120 用 `#[used]` + `#[unsafe(link_section = ".init_array.00099")]` 捕获 argc/argv（`configure_builtins.rs` 的 `.init_array.90` 只针对 aarch64 LSE） | 支持 `link_section`、`#[used]`，并保证段名排序语义 | T19 |
| **链接的系统库** | `#[link(...)]` 61 处候选，其中 Linux 实际生效的是 `unwind` 里的 `gcc_s`/`gcc_eh`、std 的 `dl`/`pthread`/`rt` 等（按 `cfg` 与 `crt-static` 选）；libc crate 的 `#[link(name = "c")]` | 支持 `#[link(name, kind, modifiers, cfg(...))]`（`link_cfg`），按 cfg 求值后交给链接器 | T28/T33 |
| **naked 函数** | 10 处候选，Linux x86_64 上实际编译的是 compiler-builtins 的 `__rust_probestack`（`extern "custom"`） | 见 [llvm-asm-arch](llvm-asm-arch.md) | 缺 |

## compiler-builtins

```
# compiler-builtins：导出函数（intrinsics! 宏内 extern "C"/"unadjusted" fn __*）按模块
TOTAL 169
float 107
int 46
arm.rs 12 ...
# Cargo features 与 links
default = ["compiler-builtins"]   c = ["dep:cc"]   no-asm = []   mem = []   mangled-names = []
rustc-dep-of-std = ["compiler-builtins", "dep:core"]
links = "compiler-rt"
# crate 级属性
1:#![cfg_attr(feature = "compiler-builtins", compiler_builtins)]
16:#![no_builtins]
```

要求：

1. **`#![compiler_builtins]` 与 `#![no_builtins]` 的语义。** 在这个 crate 里，kfrc 不能把 `u128 / u128` 降成对 `__udivti3` 的调用（否则无限递归），也不能把循环识别成 `memcpy`。它在链接顺序上排在最后，符号可以被其他实现覆盖。
2. **libcall 名表。** kfrc 降低 i128 除法/取余/乘法溢出、浮点与整数互转、f16/f128 算术时，生成的外部调用名必须和 compiler-builtins 导出的一致（float 模块 107 个，int 模块 46 个）。
3. **只有 C ABI。** 导出函数都是 `extern "C"`（少数 `extern "unadjusted"`），所以用官方 rustc 编译的 compiler-builtins 对象（或 libgcc/compiler-rt）和 kfrc 代码混链，只依赖 SysV C ABI，不依赖 `repr(Rust)`。这是唯一 ABI 风险很低的混链点，见 [mixed-link](mixed-link.md)。
4. **`c` feature 不需要。** 发行版用它从 LLVM compiler-rt 编部分 C 实现；Rust 实现是完整的。`mem` feature 在 Linux 上不开（memcpy 等由 libc 提供），但 `mem/x86_64.rs` 的 asm 仍会被编译（见 [llvm-asm-arch](llvm-asm-arch.md)）。
5. **build.rs 要运行**：设置 `f16_enabled`/`f128_enabled` 等自定义 cfg（[tiers](tiers.md) 中把 sysroot 的 4 个 build script 列为 hello world 档需求）。

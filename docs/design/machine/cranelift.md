# Cranelift 的版本与补丁

## 版本

`cranelift-codegen`、`cranelift-frontend` 固定在 `=0.134.4`：这是最低 Rust 版本仍为 1.94 的最新发布线，
与外壳固定的 rustc 1.94.1 一致。启用的特性只有 `std` 与 `x86`。

## 目标描述

x86-64 System V ELF，SSE2 基线（rustc 对 `x86_64-unknown-linux-gnu` 的假设）。设置集中在
`crates/kfrc-cranelift/src/machine.rs`：

| 设置 | 值 | 含义 |
|---|---|---|
| `opt_level` | `speed`（`KFRC_CRANELIFT_OPT` 可选 `none`、`speed_and_size`） | 中端改写的范围 |
| `is_pic` | `false`，符号一律 colocated | 符号经 RIP 相对寻址与 rel32 调用到达，代码位置无关且不经 GOT |
| `tls_model` | `elf_gd` | 通用动态序列，链接器负责放宽 |
| `unwind_info`、`preserve_frame_pointers` | `false` | 当前产物不带展开表 |
| `enable_probestack` | `true`，`inline` | 大栈帧的内联探测 |
| `enable_llvm_abi_extensions` | `true` | 与 rustc 的 Cranelift 后端一致 |

`KFRC_CRANELIFT_VERIFY=1` 对每个导出的函数运行 Cranelift 的 IR 验证器和清理后的寄存器分配检查器。

## 补丁

`third_party/cranelift-codegen` 是 crates.io 的 0.134.4 加上 kfrc 的改动，两个 workspace 用 `[patch.crates-io]` 选中它。
改动清单在 `third_party/cranelift-codegen/KFRC-PATCHES.md`，源码中以 `kfrc:` 标注。

| # | 改动 | 理由 |
|---|---|---|
| 1 | System V 栈帧在不需要 RBP 时省略帧指针（无栈上入参、无尾调用、未要求帧指针或展开信息）；栈帧非空或有调用时补 8 字节保持 RSP 的 16 字节对齐 | 上游对每个函数生成 `push rbp; mov rbp, rsp` 与对应的恢复 |

## 已知的上游行为

- x64 的跳转一律是 rel32 形式（条件跳转 6 字节、无条件跳转 5 字节）。
- 被调者保存寄存器用 `mov` 存取栈槽。
- 带溢出标志的运算与其后的分支之间经过 `setcc`。

它们各自对应一块可量化的体积或指令数；是否值得再打补丁由门禁数据决定。

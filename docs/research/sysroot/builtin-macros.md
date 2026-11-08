# 内建宏

脚本：[builtin-macros.sh](scripts/builtin-macros.sh)。机制（名字绑定、急切展开、`format_args!` 的 1.94 降低形状、derive 框架）已在 [rustc/macros-resolve/builtin-macros](../rustc/macros-resolve/builtin-macros.md) 讲过，这里只补 sysroot 的用量。ripgrep 的用量也在那篇里。

## 声明与登记

```
# 编译器登记：bang 29，attr 16，derive 12
# library 用 #[rustc_builtin_macro] 声明的宏: 58
Clone CoercePointee ConstParamTy Copy Debug Default Eq From Hash Ord PartialEq PartialOrd
alloc_error_handler asm assert autodiff_forward autodiff_reverse bench cfg cfg_accessible cfg_eval
cfg_select column compile_error concat concat_bytes const_format_args contracts_ensures
contracts_requires core_panic define_opaque derive derive_const eii eii_declaration env file
format_args format_args_nl global_allocator global_asm include include_bytes include_str iter line
log_syntax module_path naked_asm option_env pattern_type std_panic stringify test test_case
trace_macros unreachable unsafe_eii
# 编译器登记但 library 未声明
eii_shared_macro
```

编 core 时，名称解析会碰到全部 58 个声明，每个都要能绑定到 kfrc 的展开器；没实现的也要在**被调用时**才报错，不能在声明处报错。

## core/alloc/std 中的直接调用

```
# macro core alloc std
stringify 1239 0 10      assert 1209 246 466     concat 1117 0 4
cfg 60 5 84              format_args 43 6 16     include_str 32 0 5
unreachable 27 15 46     cfg_select 13 0 79      pattern_type 8 0 0
column 4 0 2             file 4 0 2              compile_error 3 0 1
asm 2 0 27               include 2 0 2           const_format_args 2 0 1
env 2 0 1                format_args_nl 1 0 3    line 1 0 2
concat_bytes 1 0 0       include_bytes 1 0 0     iter 1 0 0
option_env 1 0 0         global_asm 0 0 2
```

```
# 派生：derive core alloc std
Debug 250 25 225   Clone 221 25 318   PartialEq 139 13 100   Copy 99 3 226   Eq 99 13 97
Hash 40 2 42       Default 33 2 55    PartialOrd 22 4 24     Ord 20 4 23
CoercePointee 8 0 0    ConstParamTy 2 0 0
```

这些计数包含宏模板里的出现，是上界。要点：

- **`stringify!`/`concat!` 的输出文本会进入可观察行为。** core 中各有一千多处，多数在数值类型的宏模板里（每个整数类型展开一次）拼文档与消息。`stringify!` 用 rustc pretty printer 的空格规则；`assert!` 无消息时把条件表达式的 pretty print 结果写进 panic 消息。只要 panic 消息或 `Debug` 输出会被对拍，就要逐字一致。
- **`cfg_select!`**（std 79 处）是 1.94 的新内建宏，取代了 `cfg_if!` 风格的平台分派；std 的 `sys/` 大量使用。T23 的内建宏清单要包含它。
- `pattern_type!` 只在 core 的 `pat.rs` 出现（给指针模式类型写 `CoerceUnsized`、`Freeze` 等 impl），要求类型位置能解析 `pattern_type!(T is !null)`、`pattern_type!(u32 is 1..)`。
- `const_format_args!`：const 上下文里的 panic 消息，CTFE 要能求值 `fmt::Arguments`。
- derive 中 `CoercePointee`（8）与 `ConstParamTy`（2）只在 core，前者为智能指针生成 `CoerceUnsized`/`DispatchFromDyn` impl。
- std 的 `asm!` 27 处与 `global_asm!` 2 处全部位于非 Linux 平台目录（windows、sgx、xous、vexos、teeos、fortanix，见 [llvm-asm-arch](llvm-asm-arch.md)）。

# unstable feature gate

脚本：[features.sh](scripts/features.sh)。读取各 crate 根文件中的 `#![feature(...)]`，包括 `cfg_attr` 条件打开的。用 rustc 的 [`rustc_feature/src/unstable.rs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_feature/src/unstable.rs) 与 `accepted.rs` 区分语言 feature 和库 feature。库 feature 只解锁 `#[unstable]` API，编译器不需要为它做任何事。

## 总量

```
# 每个 crate 的 feature 数：crate always cfg
std 121 6    alloc 111 1    core 102 0    compiler_builtins 11 3
panic_unwind 9 0    proc_macro 9 0    test 9 0    panic_abort 4 0    std_detect 4 0    unwind 4 1
core+alloc+std 去重后的 feature 数: 235
全部 sysroot crate（不含 standalone 两项）去重后的 feature 数: 246
# 按 feature 种类（去重）：kind core alloc std all_sysroot
unstable 56 21 32 71
internal 16 9 10 21
incomplete 0 1 0 1
accepted 0 0 0 1
library 30 81 85 152
```

**真正需要编译器支持的是 unstable 71 + internal 21 + incomplete 1 个语言 feature。** 另有 152 个库 feature（`core_intrinsics`、`allocator_api`、`fmt_internals` 等）只需要“feature 名合法”这一条检查；唯一的 accepted 项（`naked_functions`，compiler_builtins）已经稳定。

## 语言 feature 按影响分组

脚本只给出名单，分组是人工判断。

| 组 | feature | 对 kfrc 的影响 |
|---|---|---|
| 类型系统与 trait 求解（工作量最大） | `const_trait_impl`、`const_destruct`、`const_try`、`const_precise_live_drops`、`min_specialization`、`negative_impls`、`with_negative_coherence`、`auto_traits`、`marker_trait_attr`、`fundamental`、`freeze_impls`、`never_type`、`adt_const_params`、`unsized_fn_params`、`type_alias_impl_trait`、`trait_alias`、`dropck_eyepatch`、`unboxed_closures`、`transparent_unions`、`pattern_types`、`extern_types`、`reborrow`、`ergonomic_clones`（incomplete） | T21 的 HostEffect、特化、负 impl、余归纳；`never_type` 改变 `!` 的回退规则（只在打开它的 crate 内）；TAIT 需要 opaque 定义域；dropck `#[may_dangle]` |
| 语法与宏 | `decl_macro`、`macro_metavar_expr`、`macro_metavar_expr_concat`、`try_blocks`、`if_let_guard`、`stmt_expr_attributes`、`prelude_import`、`no_core`、`allow_internal_unstable`、`allow_internal_unsafe`、`offset_of_enum` | `macro` 2.0 的 def-site hygiene；`${concat(..)}`、`${count(..)}` 等元变量表达式；`try {}` 块；match guard 里的 `if let` |
| 目标、ABI 与代码生成 | `f16`、`f128`、`repr_simd`、`simd_ffi`、`abi_unadjusted`、`abi_custom`、`c_variadic`、`thread_local`、`linkage`、`used_with_arg`、`link_llvm_intrinsics`、`compiler_builtins`、`panic_runtime`、`needs_panic_runtime`、`alloc_error_handler`、`allocator_internals`、`link_cfg`、`cfg_target_has_atomic(_equal_alignment)`、`cfg_target_thread_local`、`cfg_target_has_reliable_f16_f128`、`cfg_ub_checks`、`cfg_sanitize`、`ffi_const`、`optimize_attribute`、`coverage_attribute`、`x86_amx_intrinsics`、`rtm_target_feature`、`avx10_target_feature`、其他架构的 `*_target_feature`、`asm_experimental_arch` | 见 [llvm-asm-arch](llvm-asm-arch.md)、[runtime-link](runtime-link.md)；其他架构的 target feature 与 `asm_experimental_arch` 在 x86_64 上只需认名 |
| 内部、诊断、文档（只认名） | `rustc_attrs`、`staged_api`、`lang_items`、`intrinsics`、`rustc_allow_const_fn_unstable`、`rustdoc_internals`、`doc_cfg`、`doc_masked`、`doc_notable_trait`、`intra_doc_pointers`、`deprecated_suggestion`、`diagnostic_on_const`、`must_not_suspend`、`strict_provenance_lints`、`multiple_supertrait_upcastable`、`custom_test_frameworks`、`cfg_emscripten_wasm_eh`、`cfi_encoding`、`cfg_sanitizer_cfi` | 解锁对应属性；`custom_test_frameworks` 只给 test harness |

## 指定关注的 feature 由谁打开

```
specialization: -                          min_specialization: core alloc std
const_trait_impl: core alloc std           negative_impls: core alloc std proc_macro
auto_traits: core                          never_type: core alloc std
decl_macro: core alloc std proc_macro      rustc_attrs: core alloc std panic_unwind panic_abort compiler_builtins proc_macro
staged_api: core alloc std panic_unwind panic_abort unwind std_detect proc_macro test
allow_internal_unstable: core alloc std std_detect proc_macro
lang_items: core alloc std panic_unwind    intrinsics: core alloc
core_intrinsics: core alloc std panic_unwind compiler_builtins（库 feature）
link_llvm_intrinsics: core                 naked_functions: compiler_builtins
asm_experimental_arch: core std compiler_builtins（只影响非 x86 架构）
```

完整的 `specialization` 没有打开，只用 `min_specialization`；`asm!` 在 x86_64 上是稳定特性，不需要 gate。

## 对应语言构造的出现次数

文本近似，宏模板里的出现也计入；core 一列包含 core_arch x86/x86_64 与 core_simd。

```
impl_const 407 6 8                 tilde_or_bracket_const 282 3 4      const_trait_decl 69 0 0
default_fn 90 34 10                negative_impl 24 9 27               auto_trait 7 0 0
never_type_in_sig 71 11 63         decl_macro 53 1 20                  macro_rules 273 18 54
const_fn_decl 8446 70 197          const_item 1195 27 3169             static_item 37 1 140
try_block 48 10 5                  tait_decl 0 0 1                     track_caller 272 25 26
asm_macro 66 0 29                  global_asm 0 0 2                    link_name_llvm 1046 0 0
extern_unadjusted 7 0 0            repr_simd 3 0 0                     pattern_type 5 0 0
thread_local_attr 0 0 13           linkage_attr 0 0 6                  target_feature_attr 6265 0 0
may_dangle 1 13 1                  super_let 1 0 0                     f16_f128 781 0 159
```

几条结论：

- const trait 是 core 的主体写法：`impl const` 407 处、`[const]` 约束 282 处、`const trait` 69 处。T21 不覆盖 HostEffect，core 就无法类型检查，这不是可以“遇到再报错”的边角。
- `const fn` 在 core 中有 8446 处（x86 core_arch 的 intrinsic 已大量改为 `const fn`），std 的 `const` 项 3169 处主要是 libc/平台常量。CTFE 至少要能求出所有 `const`/`static` 的初值（[tiers](tiers.md)）。
- `default fn`（core/alloc/std 各 90/34/10 处）和负 impl（24/9/27 处）分布在三个 crate，都要特化与负 impl 语义。
- `asm!`、`#[thread_local]`、`#[linkage]` 的位置见 [runtime-link](runtime-link.md)。

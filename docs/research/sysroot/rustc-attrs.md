# `#[rustc_*]` 属性：种类、数量与编译器要求

脚本：[rustc-attrs.sh](scripts/rustc-attrs.sh)。core 的计数包含 `#[path]` 引入的 core_arch（全部架构）与 core_simd；`core_arch_x86` 一列是其中 x86/x86_64 目录的子计数。宏模板里的属性按文本计入。

## 总量

```
# 每个 crate 的 rustc_* 属性总数与种类数：crate uses kinds
core 9775 44
core_arch_x86 4860 2
std 376 20
alloc 218 22
panic_unwind 5 1
panic_abort 4 1
compiler-builtins 3 1
# 不同种类（…不含 core_arch_x86 重复计数）: 54
# 编译器内建 rustc_* 属性数: 101；sysroot 用到其中: 54
```

编译器在 [`builtin_attrs.rs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_feature/src/builtin_attrs.rs) 里登记了 101 种，sysroot 用到 54 种。下面按“对 kfrc 的要求”分三组，括号里是脚本给出的总次数。

## A. 改变语义或代码生成：必须实现（31 种）

| 属性（次数） | 作用 | kfrc 要做的事 | 任务 |
|---|---|---|---|
| `rustc_layout_scalar_valid_range_start/end`（2/1） | `NonNull` 与 `niche_types.rs` 宏生成的 `NonZero*`、`Alignment` 等的标量有效范围 | 布局里登记 valid range → niche；CTFE 有效性检查；构造时要求 unsafe | T15、T24 |
| `rustc_inherit_overflow_checks`（53） | core 中运算符 impl 的溢出检查跟随**生成代码的那个 crate** 的 `-C overflow-checks` | 实例化时按使用方配置决定是否插入溢出断言 | T22、T25 |
| `rustc_legacy_const_generics`（3368，x86 1544） | `_mm_alignr_epi8(a, b, 3)` 这类调用的第 N 个实参其实是 const 泛型参数 | 名称解析后改写调用：实参移到 const 泛型位置，并检查为常量 | T21（ripgrep 前必须） |
| `rustc_intrinsic`（299） | 声明 intrinsic | 见 [intrinsics](intrinsics.md) | T24、T27 |
| `rustc_nounwind`（310） | 函数不会 unwind | 不生成 landing pad；unwind 越界时 abort（`panic_cannot_unwind`） | T27 |
| `rustc_specialization_trait`（18）、`rustc_unsafe_specialization_marker`（9） | `min_specialization` 允许作为特化条件的 trait | 特化合法性检查与特化选择 | T21 |
| `rustc_coinductive`（4） | `Sized`/`MetaSized` 等 trait 余归纳 | search graph 的环判定 | T21 |
| `rustc_deny_explicit_impl`（11）、`rustc_do_not_implement_via_object`（11） | 禁止手写 impl；`dyn Trait` 不自动实现该 trait | coherence 与 object 候选 | T21 |
| `rustc_reservation_impl`（1） | `impl<T> From<!> for T` 只占位 | coherence 视为存在，选择时视为不存在 | T21 |
| `rustc_paren_sugar`（6） | `Fn(A) -> B` 语法 | 解析成 `Fn<(A,), Output = B>` | T06 |
| `rustc_coherence_is_core`（1）、`rustc_allow_incoherent_impl`（198）、`rustc_has_incoherent_inherent_impls`（3） | core 可以给基本类型写固有 impl；alloc/std 可以给 `str`、`[T]`、`f64` 等追加固有方法（`to_vec`、`to_uppercase`、`sqrt` 等） | 固有 impl 按“简化类型”跨 crate 收集，方法查找要看到 alloc/std 的追加 | T21 |
| `rustc_skip_during_method_dispatch`（1） | 数组的 `IntoIterator`：2021 之前版本的 `.into_iter()` 跳过它 | 方法查找按 edition 跳过候选 | T21 |
| `rustc_promotable`（11） | 允许在 `&expr` 中把对 const fn 的调用提升成 `'static` | 提升规则 | T24 |
| `rustc_builtin_macro`（58）、`rustc_macro_transparency`（20） | 内建宏绑定；`macro` 2.0 的 hygiene 透明度 | 见 [builtin-macros](builtin-macros.md) | T23 |
| `rustc_std_internal_symbol`（32） | `__rust_alloc`、`__rust_start_panic` 等跨 crate 契约符号 | 符号命名与可见性规则固定，分配器 shim 用同一规则 | T19、T32 |
| `rustc_allocator`、`rustc_deallocator`、`rustc_reallocator`、`rustc_allocator_zeroed(_variant)`（各 1） | 标记 `__rust_alloc` 等声明 | 优化时作为分配函数（可选），shim 生成时识别 | T32 |
| `rustc_pass_indirectly_in_non_rustic_abis`（6） | `VaList` 在 C ABI 下按指针传 | ABI 计算 | T18 |
| `rustc_align_static`（4） | std 中 static 的对齐要求 | 数据对象对齐 | T19 |
| `rustc_deprecated_safe_2024`（3） | `env::set_var` 等在 edition 2024 是 unsafe，之前的 edition 安全 | unsafety 检查按调用处 edition | T22 |
| `rustc_preserve_ub_checks`（3）、`rustc_no_mir_inline`（4） | 控制 rustc 的 MIR 优化 | kfrc 若有自己的内联，需尊重 `ub_checks` 按使用方求值；否则忽略 | T25 |

## B. 常量检查与稳定性：可以推迟，但要标出（7 种）

`rustc_const_unstable`（4201，x86 3316）、`rustc_const_stable`（895）、`rustc_allow_const_fn_unstable`（49）、`rustc_intrinsic_const_stable_indirect`（84）、`rustc_const_stable_indirect`（9）、`rustc_do_not_const_check`（5）、`rustc_allowed_through_unstable_modules`（4），加上 `#[stable]`/`#[unstable]`：

```
stable 12363 1167 3693        # core alloc std
unstable 5718 660 842
deprecated 86 1 29
allow_internal_unstable 54 2 12
```

它们只决定“下游 crate 能不能用”：调用了未打开 feature 的 unstable API，或在 const 上下文调用非 const-stable 的函数，就报错。编 sysroot 本身时可以全部跳过检查（rustc 自己也对 staged_api crate 放宽）；ripgrep/Cargo 不使用 unstable API，所以 M4/M5 的正例都不会受影响。代价是反例差分：用户代码误用 unstable API 时 kfrc 会错误地接受。建议 M4 只做“`#[unstable]` 项 + 未开 feature → 报错”这一条，const 稳定性检查列入未覆盖清单。

## C. 只服务诊断和 lint：忽略（16 种）

`rustc_diagnostic_item`（382）、`rustc_should_not_be_called_on_const_items`（97）、`rustc_confusables`（50）、`rustc_doc_primitive`（27）、`rustc_never_returns_null_ptr`（24）、`rustc_lint_query_instability`（23）、`rustc_on_unimplemented`（21）、`rustc_as_ptr`（15）、`rustc_insignificant_dtor`（13）、`rustc_pub_transparent`（13）、`rustc_no_implicit_autorefs`（12）、`rustc_nonnull_optimization_guaranteed`（8，只影响 `improper_ctypes` lint，niche 来自 A 组的 valid range）、`rustc_conversion_suggestion`（3）、`rustc_trivial_field_reads`（2）、`rustc_objc_class/selector`（Apple 专用）。

解析器要认识这些名字并跳过，不能因为“未知属性”报错。

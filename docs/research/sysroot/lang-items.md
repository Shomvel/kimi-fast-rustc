# lang item：数量、类别与编译器要做的事

脚本：[lang-items.sh](scripts/lang-items.sh)。库侧统计 core/alloc/std/panic_unwind/panic_abort/unwind/compiler_builtins 中的 `#[lang = "..."]`，另加 core 用宏 `#[lang = stringify!($lang)]` 生成的 `panic_const_*` 系列；编译器侧读 [`language_item_table!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir/src/lang_items.rs#L165)。

## 总量

```
# 库中定义：crate count（去重后，按 crate）
core 194
alloc 4
std 4
panic_unwind 1
库中不同 lang item 名称数: 203
编译器 language_item_table! 条目数: 207
两者交集: 203
# 编译器表中有、库中没有定义
default_trait1 default_trait2 default_trait3 default_trait4
```

除了 4 个实验性的 `default_trait*`，编译器认识的 lang item 在 1.94.1 的 sysroot 里全部有定义，**而且几乎都在 core**。T12 用的 mini core 只覆盖其中一小部分；编 core 时 203 个都要识别并接到对应的语义上。

按编译器表的目标种类：

```
Trait 77 73      Fn 49 49      Struct 31 31    Method 17 17    AssocTy 12 12
Variant 8 8      AssocConst 6 6    Enum 5 5    Static 1 1      Union 1 1
```

## 按语义类别（脚本分组，名单见脚本输出）

| 类别 | 数量 | 代表 | 编译器侧要做的事 | 对应任务 |
|---|---:|---|---|---|
| panic-unwind | 39 | `panic`、`panic_fmt`、`panic_bounds_check`、`panic_nounwind`、`panic_cannot_unwind`、`panic_in_cleanup`、22 个 `panic_const_*`、`eh_personality`、`eh_catch_typeinfo`、`panic_impl`、`panic_location` | MIR 断言（溢出、除零、越界、协程状态）降成对 `panic_const_*` 的调用；nounwind 边界插入 `panic_cannot_unwind`/`panic_in_cleanup`；`#[track_caller]` 传 `Location` | T22、T27 |
| type-marker | 35 | `sized`、`meta_sized`、`pointee_sized`、`copy`、`clone`、`sync`、`freeze`、`unsize`、`coerce_unsized`、`dispatch_from_dyn`、`destruct`、`drop`、`phantom_data`、`manually_drop`、`unsafe_cell`、`maybe_uninit`、`discriminant_kind`、`pointee_trait`、`dyn_metadata`、`transmute_trait`、`structural_peq`、`const_param_ty`、`mem_size_const`/`mem_align_const` | 求解器 builtin 候选、unsize 强转、`UnsafeCell` 影响 `Freeze` 与别名、`ManuallyDrop` 不生成 drop、`MaybeUninit` 不做有效性检查 | T21、T15、T22 |
| operator | 33 | `add`…`shr_assign`、`neg`、`not`、`index(_mut)`、`deref(_mut)`、`deref_target`、`deref_pure`、`receiver`、`legacy_receiver`、`eq`、`partial_ord` | 运算符脱糖到 trait 方法；内建类型直接用原语操作；`receiver` 决定可用的 `self` 类型 | T06、T21 |
| iter-async-coroutine | 30 | `iterator`、`into_iter`、`next`、`fused_iterator`、`future_trait`、`into_future`、`poll`、`Poll`、`Ready`、`Pending`、`Context`、`get_context`、`coroutine*`、`async_drop*`、`pin`、`unpin`、`ResumeTy`、`AsyncGen*` | `for` 脱糖；`async fn`/`.await`/协程变换。**sysroot 本体几乎不含 async 代码**（手工 grep：alloc、std 为 0 处，core 只在 `join!` 等宏模板里有 async 块，展开时才需要）；Cargo 0.95.0 默认构建也不编译 async 代码（gix 经 `maybe-async` 改写为同步，其余在未开 feature 下，见 [deps-cargo](deps-cargo.md)）；0.99.0 的 `cargo` 本身用 `async fn` | T06、M3 未列出（见 [plan-gaps](plan-gaps.md)） |
| std-type-hook | 19 | `Option`/`Some`/`None`、`Ok`/`Err`、`branch`、`from_output`、`from_residual`、`from_yeet`、`into_try_type`、`Continue`/`Break`、`Ordering`、`String`、`CStr`、`c_void`、`va_list`、`slice_len_fn` | `?`、`try` 块、`if let`/`while let` 脱糖，字面量类型 | T06 |
| closure-fn | 15 | `fn`、`fn_mut`、`fn_once`、`fn_once_output`、`async_fn*`、`call_once_future`、`call_ref_future`、`fn_ptr_trait`、`fn_ptr_addr`、`tuple_trait` | 闭包与函数指针的 `Fn*` builtin 候选；`rustc_paren_sugar` 的 `Fn(A) -> B` 语法 | T21、T22(c) |
| range | 14 | `Range`、`RangeFrom`、`RangeFull`、`RangeInclusive`、`RangeTo(Inclusive)`、`range_inclusive_new`、`RangeCopy` 系列 | `a..b` 字面量脱糖；`RangeMin/Max/Sub` 用于模式 | T06 |
| alloc-drop-entry | 6 | `owned_box`、`exchange_malloc`、`drop_in_place`、`alloc_layout`、`start`、`termination` | `Box` 是内建类型（可直接 move 出、`Box<dyn>` unsize）；`box` 分配；drop glue 入口；**`start` 决定编译器生成的 C `main`** | T22、T25、T32、T33 |
| format | 2 | `format_argument`、`format_arguments` | `format_args!` 降低（见 [builtin-macros](builtin-macros.md)） | T23 |
| other | 10 | `type_id`、`type_info`、`offset_of`、`compiler_copy`、`compiler_move`、`reborrow`、`coerce_shared`、`contract_*` | `TypeId` 的值与比较、`offset_of!`；`compiler_copy/move`、`reborrow`、`coerce_shared`、`contract_*` 属于实验特性，没有使用者时可以只识别 | T24、T25 |

## 入口类属性（相当于弱 lang item）

```
27 #[rustc_std_internal_symbol]   16 #[unsafe(no_mangle)]   2 #![panic_runtime]
 1 #[panic_handler]                1 #[alloc_error_handler]  1 #![needs_panic_runtime]
 1 #![needs_allocator]             1 #![default_lib_allocator]
```

这些属性决定的链接期契约（panic runtime 只能有一个，分配器 shim 由编译器生成，`#[rustc_std_internal_symbol]` 的符号命名）见 [runtime-link](runtime-link.md)。

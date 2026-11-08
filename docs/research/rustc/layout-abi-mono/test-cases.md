# 边界用例与 SysV 对拍形状

本检出只有 `tests/ui`；compiletest 的 C 侧 helper（上游 `tests/auxiliary/rust_test_helpers.c`）不在快照里，FFI 用例的 C 端需要按下表自写。链接前缀固定到 `e408947b`。

## rustc 自带的 oracle 工具（nightly 属性，仅用于生成对照数据）

- `#[rustc_layout(debug)]`：打印完整 `LayoutData`（[tests/ui/layout/debug.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/debug.rs)）。
- `#[rustc_abi(debug)]` / `#[rustc_abi(assert_eq)]`：打印 `FnAbi` / 断言两个函数 ABI 相同（[tests/ui/abi/debug.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/debug.rs)）。
- `-Zprint-type-sizes`：变体与字段大小（`record_layout_for_printing`，[layout.rs#L794](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L794)）。
- 这些需要 `#![feature(rustc_attrs)]` / `-Z`，属于 [differential-testing.md](../../../plan/README.md#验收) 所说的“单列环境”：只产出 rustc 端的期望值，被测程序本身仍用稳定 Rust。

## 布局（`tests/ui/layout/`、`enum-discriminant/`、`structs-enums/`）

| 用例 | 覆盖点 |
|---|---|
| [layout/debug.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/debug.rs) | uninhabited 变体、ZST 字段、union、`Result<i32,i32>`、零长数组 union 的完整布局输出 |
| [layout/null-pointer-optimization-sizes.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/null-pointer-optimization-sizes.rs)、[null-pointer-optimization.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/null-pointer-optimization.rs) | `Option<T>`/自定义 enum 借 niche 不增大 |
| [layout/unsafe-cell-hides-niche.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/unsafe-cell-hides-niche.rs) | `UnsafeCell` 隐藏 niche、transparent 保留 niche |
| [layout/enum-scalar-pair-int-ptr.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/enum-scalar-pair-int-ptr.rs) | int/ptr 混合变体用 ptr 作为 ScalarPair 元素 |
| [layout/issue-96158-scalarpair-payload-might-be-uninit.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/issue-96158-scalarpair-payload-might-be-uninit.rs) | ScalarPair 第二元素为 `Scalar::Union` |
| [layout/issue-96185-overaligned-enum.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/issue-96185-overaligned-enum.rs) | `repr(align)` 变体与 tag |
| [layout/zero-sized-array-enum-niche.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/zero-sized-array-enum-niche.rs) | `[T;0]` 不提供 niche、对齐影响 |
| [layout/issue-112048-unsizing-field-order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/issue-112048-unsizing-field-order.rs)、[issue-112048-unsizing-niche.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/issue-112048-unsizing-niche.rs) | 可 unsize 的 struct 布局不依赖尾部 |
| [layout/too-big-with-padding.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/too-big-with-padding.rs)、[big-type-no-err.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/big-type-no-err.rs) | `SizeOverflow` 边界 |
| [enum-discriminant/niche.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/niche.rs)、[niche-prefer-zero.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/niche-prefer-zero.rs)、[ptr_niche.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/ptr_niche.rs)、[wrapping_niche.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/wrapping_niche.rs) | niche 编码/解码、偏好 0、回绕区间 |
| [enum-discriminant/discriminant_size.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/discriminant_size.rs)、[structs-enums/enum-discrim-autosizing.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/structs-enums/enum-discrim-autosizing.rs) | tag 宽度选择（有/无符号） |
| [enum-discriminant/repr128.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/repr128.rs)、[repr-c-big-discriminant1.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/repr-c-big-discriminant1.rs) | 128 位与超出 `c_int` 的 `repr(C)` 判别值 |
| [enum-discriminant/get_discr.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/get_discr.rs)、[discriminant_value.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/enum-discriminant/discriminant_value.rs) | 读判别值 |
| [structs-enums/enum-non-c-like-repr-c-and-int.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/structs-enums/enum-non-c-like-repr-c-and-int.rs)、[enum-non-c-like-repr-c.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/structs-enums/enum-non-c-like-repr-c.rs)、[enum-non-c-like-repr-int.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/structs-enums/enum-non-c-like-repr-int.rs) | RFC 2195 带字段 enum 的 `repr(C)`/`repr(u8)`/`repr(C,u8)` 布局，可直接与 C 对拍 |

## ABI（`tests/ui/abi/`）

| 用例 | 覆盖点 |
|---|---|
| [abi-sysv64-arg-passing.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/abi-sysv64-arg-passing.rs) | 汇总：窄整数扩展、`TwoU8s…TwoU64s` 传入/返回、空 struct、byval、`Quad`、float struct、tagged enum、issue-62350 寄存器耗尽 |
| [abi-sysv64-register-usage.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/abi-sysv64-register-usage.rs) | 用 inline asm 检查参数寄存器 |
| [extern/](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/extern/extern-pass-TwoU64s.rs)、[struct-enums/struct-return.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/struct-enums/struct-return.rs)、[foreign/foreign-fn-with-byval.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/foreign/foreign-fn-with-byval.rs) | 小聚合传/返、sret、byval |
| [numbers-arithmetic/i128-ffi.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/numbers-arithmetic/i128-ffi.rs)、[float-struct.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/numbers-arithmetic/float-struct.rs)、[return-float.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/numbers-arithmetic/return-float.rs) | i128、浮点聚合、浮点返回 |
| [extern-c-two-doubles-x86_64-5754.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/extern-c-two-doubles-x86_64-5754.rs)、[union/union-c-interop.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/union/union-c-interop.rs) | `{f64,f64}`、union 分类 |
| [large-byval-align.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/large-byval-align.rs) | 高对齐 byval |
| [c-zst.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/c-zst.rs)、[sysv64-zst.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/sysv64-zst.rs) | C ABI 下 ZST 参数（x86_64 Linux 为 Ignore） |
| [cabi-int-widening.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/cabi-int-widening.rs)、[variadic-ffi.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/variadic-ffi.rs) | 符号扩展、variadic |
| [compatibility.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/compatibility.rs)、[nullable-pointer-ffi-compat.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/nullable-pointer-ffi-compat.rs) | 哪些类型对在 fn 指针调用下 ABI 兼容；`Option<&T>` 与可空指针 |
| [rustcall-generic.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/rustcall-generic.rs) | `"rust-call"` 参数展开 |
| [rfcs/rfc-2091-track-caller/](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/rfcs/rfc-2091-track-caller/tracked-fn-ptr.rs)（`tracked-fn-ptr*.rs`、`tracked-trait-obj.rs`、`call-chain.rs`、`const-caller-location.rs`） | 位置参数传递、ReifyShim、vtable、CTFE 中的位置 |

## 单态化与常量求值

| 用例 | 覆盖点 |
|---|---|
| [consts/required-consts/collect-in-dead-fn.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/required-consts/collect-in-dead-fn.rs) 及同目录 | 死代码中的失败常量在 `-O0`/`-O` 下一致报错（mentioned items） |
| [recursion/issue-38591-non-regular-dropck-recursion.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/recursion/issue-38591-non-regular-dropck-recursion.rs)、[recursion/issue-83150.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/recursion/issue-83150.rs) | 多态递归撞实例化递归限制 |
| [traits/vtable/](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/traits/vtable/impossible-method.rs) | `Vacant` 槽、超 trait vtable |
| [consts/const-eval/ub-enum.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/ub-enum.rs)、[ub-nonnull.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/ub-nonnull.rs)、[ub-ref-ptr.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/ub-ref-ptr.rs)、[ub-wide-ptr.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/ub-wide-ptr.rs)、[ub-invalid-values.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/ub-invalid-values.rs) | 最终值有效性检查 |
| [consts/const-eval/infinite_loop.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/infinite_loop.rs) | `long_running_const_eval` |
| [consts/const-eval/ptr_fragments_in_final.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/ptr_fragments_in_final.rs)、[raw-pointer-ub.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/raw-pointer-ub.rs)、[const_raw_ptr_ops.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/const_raw_ptr_ops.rs) | 部分指针、裸指针操作限制 |
| [consts/const-eval/heap/](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/heap/alloc_intrinsic_transient.rs) | const 堆分配 |
| [consts/recursive-static-write.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/recursive-static-write.rs)、[static-cycle-error.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/static-cycle-error.rs)、[const-size_of-cycle.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-size_of-cycle.rs) | static 递归、常量环 |
| [consts/const-eval/const-eval-overflow-2.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/const-eval-overflow-2.rs)、[promoted_const_fn_fail.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/const-eval/promoted_const_fn_fail.rs) | 溢出、promoted 失败 |
| [const-generics/core-types.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/const-generics/core-types.rs)、[different_generic_args.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/const-generics/different_generic_args.rs) | const 参数值树相等 |

## x86_64 SysV 双向对拍形状（C↔Rust，两个方向都测：C 调 kfrc、kfrc 调 C）

每个形状同时测“作为参数”“作为返回值”“作为 callback 参数”，并在前面垫 0/5/6/7 个整型参数或 0/7/8 个 double 制造寄存器耗尽。

| 组 | 形状 | 期望分类 |
|---|---|---|
| 标量 | `i8/u8/i16/u16/bool`（负值与 0x80/0x8000 边界） | INT，参数 32 位扩展、返回不扩展 |
| | `i32/u32/i64/u64/usize/*const T/fn ptr` | INT |
| | `f32/f64` | SSE |
| | `i128/u128`（含奇数个前置整型参数） | 两个 INT，对齐 16 |
| 小聚合 | `{u8,u8}`、`{u16×5}`、`{u32,u32}`、`{u64,u64}` | 1–2 个 INT（`Reg{Integer,n}` 尾块） |
| | `{f32,f32}`、`{f64,f64}`、`{f32,f32,f32}`、`{f32×4}` | SSE 打包 |
| | `{f64,u64}`、`{u64,f64}`、`{f32,u32}`、`{u8,f32}` | 混合；同一 eightbyte 内 INT 胜 |
| 内存类 | `{u64×3}`、`{[u8;17]}`、`#[repr(C, align(32))]`、`#[repr(C, packed)] {u8,u32}` | MEMORY：byval / sret；错位字段 |
| enum/union | `#[repr(C)]` fieldless、`#[repr(u8)]`、`#[repr(C,u8)] {A(u64,u64),B}`、`union {f64,u64}` | 按 tag + union 分类 |
| 特殊 | 空 `#[repr(C)] struct`、`Option<&T>`、`Option<extern "C" fn()>` | Ignore；可空指针 |
| 返回 | 上述每种的返回；`-> !`（不返回） | RAX/RDX/XMM0/XMM1/sret |
| variadic | `printf("%d %f %s", ..)` 混合整型与 double；超过 8 个 double | AL 设置、栈上 variadic |
| 栈 | 9+ 个混合参数、栈上 i128 与 byval 的对齐 | 16 字节调用对齐 |

`long double`、`_Complex` 属于 X87/复数类，Rust 无法表达，rustc 分类器也没有对应类别；`__m128`/`__m256` 按值传递依赖 target feature 与 `simd_types_indirect`，单列后做。

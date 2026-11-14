# 代表性 tests/ui 用例

路径相对 `tests/ui/`（官方源码 `e408947b`）。每条说明考察的规则和对应笔记。“M1”标出标量首轮就能对拍的用例。

## 整数字面量与 fallback

| 用例 | 规则 | 笔记 |
|---|---|---|
| `inference/tutorial-suffix-inference-test.rs` | 字面量变量被第一次使用定型（`identity_u8(x)` 后 `x: u8`，再传 `u16` 报错）；`let y: i32 = 3` 不受后续调用影响 | [expectation-and-fallback §3](expectation-and-fallback.md)（M1） |
| `inference/infer-binary-operand-behind-reference.rs` | `let _: u8 = 0 + 0;` 外层类型经内建运算结果类型（= lhs）约束字面量；剥一层 `&` 的内建判定；无期望时 fallback `i32` | [autoderef-and-methods §5](autoderef-and-methods.md)（M1 中不含 `&` 的部分） |
| `inference/bitwise-integer-inference-68951.rs` | `b & 0xf`、`(b >> 4) & 0xf`（`b: &u8`）：位运算统一两侧，移位不统一 | 同上 |
| `cast/cast-does-fallback.rs` | `let cap = 512 * 512; cap as u8;`：cast 在 fallback 之后检查，`cap` 为 `i32`，运行时不为 0 | [casts.md §1](casts.md)（M1） |
| `lint/issue-109529.rs` | `256 as u8` 中字面量直接取 `u8`，触发 overflowing literal | [casts.md §1](casts.md)（M1） |
| `cast/cast-as-bool.rs` | `5 as bool` 报 “cannot cast `i32` as `bool`”：字面量不取 `bool`，fallback 为 `i32` 后 cast 被拒 | casts.md（M1） |
| `feature-gates/feature-gate-negate-unsigned.rs` | `let _: usize = -1;` 与 `-x`（`x: u8`）报 E0600：一元 `-` 把期望传给操作数，无符号类型无 `Neg` | expectation-and-fallback §3（M1） |
| `methods/method-on-ambiguous-numeric-type.rs` | `2.0.neg()`、`x.pow(2)`（`x: {integer}`）报 E0689：数值变量没有 inherent 候选 | [autoderef-and-methods §2](autoderef-and-methods.md) |
| `inference/order-dependent-cast-inference.rs` | `0 as *const _` 在 cast 检查时 pointee 仍未知 → “unknown kind” 错误；推导顺序敏感（#48270） | casts.md |
| `cast/cast-to-infer-ty.rs` | `let i: *const i32 = 0 as _;` 合法 | casts.md |

## 分支、块与 never

| 用例 | 规则 | 笔记 |
|---|---|---|
| `expr/if/if-branch-types.rs` | `if { 10i32 } else { 10u32 }`：LUB 失败，错误指向 else 分支 | [coercion §4](coercion.md)（M1） |
| `expr/if/if-without-else-result.rs` | 无 else 的 `if` 被强制为 `()`，`let a = if c { true };` 报 E0317 | expectation-and-fallback §3（M1） |
| `coercion/coerce-unify.rs` | if/else 链、match、数组字面量对各种排列顺序都得到同一 LUB | coercion §4 |
| `never_type/never_coercions.rs` | match 中 `return` arm 的 `!` 参与 LUB，不影响其他 arm 的 `&Vec`→`&[u32]` | coercion §2 |
| `never_type/adjust_never.rs` | `let y: u32 = x;`（`x: !`）通过 `NeverToAny` | coercion §2（M1，需 `!` 类型） |
| `never_type/diverging-fallback-unconstrained-return.rs` | `if c { f() } else { panic!() }` 中泛型返回值：2021 fallback 为 `()`（触发 Deny 级 lint），2024 为 `!`（trait 不满足报错） | [expectation-and-fallback §6](expectation-and-fallback.md) |
| `never_type/defaulted-never-note.rs`、`never_type/fallback-closure-ret.rs` | `let x = return;` 与 `|| panic!()` 的返回类型随 edition 落到 `()` / `!` | 同上 |

## coercion 与引用

| 用例 | 规则 | 笔记 |
|---|---|---|
| `coercion/coerce-reborrow-multi-arg-fail.rs` | `test<T>(&mut 7, &7)`：`T` 由第一个实参定为 `&mut`，`&` 不能 coerce 到 `&mut` | [references §4](references.md) |
| `coercion/coerce-mut.rs` | `f(&x)` 传给 `&mut i32` 形参：`coerce_mutbls` 拒绝 | references §4 |
| `coercion/coerce-overloaded-autoderef.rs` | `use_ref(&&&&&&t)`、`use_mut(t)`（`t: &mut Box<T>`）：deref coercion 沿 autoderef 链找第一个能 unify 的步骤，`&mut` 实参被重借用 | coercion §3 |
| `coercion/coerce-block-tail.rs` | `let _: &str = &{ String::from(..) }` 通过，`let _: &i32 = &{ Box::new(1) }` 失败：pointee 是 unsized 时只给 `ExpectRvalueLikeUnsized`，sized 时块尾被要求是 `i32` | references §2 |
| `coercion/coerce-expect-unsized.rs` | `Box<[isize]> = Box::new(if .. {[1,2,3]} else {..})`：期望 unsized 时 coercion 发生在指针层，右值本身保持 sized | expectation-and-fallback §2 |
| `coercion/lub_coercion_handles_safety.rs` | 两个不同 fn item（一个 unsafe）LUB 到 `unsafe fn()` | coercion §4 |
| `coercion/fudge-inference/fn-ret-trait-object-propagated-to-inputs-issue-149379-1.rs` | 由期望返回类型 fudge 出的实参提示可能不 WF，导致误报（FIXME 用例） | [calls-and-closures §2](calls-and-closures.md) |
| `borrowck/two-phase-*.rs` | 方法接收者/实参 autoref 带两阶段标记后的借用检查行为 | references §3 |

## 方法与 autoderef

| 用例 | 规则 | 笔记 |
|---|---|---|
| `methods/method-lookup-order.rs` | 31 种 inherent/trait、by-value/`&`/`&mut` 组合的选择结果（含歧义组合） | autoderef-and-methods §2 |
| `autoref-autoderef/autoderef-method-priority.rs` | `x: Box<usize>`，`impl double for usize` 与 `impl double for Box<usize>` 都存在时，第 0 步 by-value 的 `Box<usize>` 先命中 | 同上 |
| `autoref-autoderef/autoderef-method-twice-but-not-thrice.rs` | `self: Box<Self>` 接收者需要恰好停在正确的 deref 步 | 同上 |
| `methods/trait-method-resolution-over-inherent-22684.rs` | 不可见的 inherent 方法不参与，选中 trait 方法 | 同上 |
| `methods/inherent-methods-same-name.rs` | `impl Foo<usize>` 与 `impl Foo<isize>` 同名方法由接收者类型区分 | 同上 |

## 模式

| 用例 | 规则 | 笔记 |
|---|---|---|
| `rfcs/rfc-2005-default-binding-mode/explicit-mut.rs` | 经过 `&` 后默认模式只能是 `ref`，`*n += 1` 报错 | [patterns §2](patterns.md) |
| `rfcs/rfc-2005-default-binding-mode/reset-mode.rs` | `let (_, &b) = &(1, &2);`：`&` 模式重置默认模式（2021 行为） | patterns §2 |
| `rfcs/rfc-2005-default-binding-mode/lit.rs` | 引用类型的字面量模式（`"abc"`）不触发自动剥引用 | patterns §2 |
| `pattern/rfc-3627-match-ergonomics-2024/min_match_ergonomics_fail.rs` | edition 2024 下在继承引用内写 `&`、`mut`、`ref` 是硬错误 | patterns §3 |
| `pattern/rfc-3627-match-ergonomics-2024/migration_lint.rs` | edition 2021 下同类写法只触发 `rust_2024_incompatible_pat` lint（测试里 `deny`），并给出机器可应用的修复 | patterns §3 |

## 使用建议

- M1 对拍集先取标“M1”的用例的**标量改写版**（去掉 `&`、`Box`），每个都有确定的 rustc 退出码与输出；fallback 相关用例必须在 kfrc 支持 `i32` 后才能纳入（见 [README](README.md)）。
- 涉及 edition 的用例（never fallback、模式）要按 revision 分别对拍，并在 kfrc 的用例元数据里记 edition。

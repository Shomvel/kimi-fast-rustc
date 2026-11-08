# Coercion：一对一与 LUB

源码：`compiler/rustc_hir_typeck/src/coercion.rs`；设计说明见 dev-guide `src/doc/rustc-dev-guide/src/hir-typeck/coercions.md`。链接指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. 入口与结果

- **`FnCtxt::coerce(expr, expr_ty, target, allow_two_phase, cause)`**（[coercion.rs#L962](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L962)）：`commit_if_ok(|_| Coerce::coerce(source, target))`，成功后注册 obligation、`apply_adjustments(expr, adjs)`，返回 `target`。失败时什么都不留。`demand_coerce`（[demand.rs#L229](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/demand.rs#L229)）在其上加诊断。
- **`Coerce`**（[#L71](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L71)）：`use_lub`（一对一用 `sup(target, source)`，LUB 用 `lub`）、`allow_two_phase`、`coerce_never`（表达式是否构成读取）。结果 `CoerceResult = InferResult<(Vec<Adjustment>, Ty)>`。
- `unify_raw`（[#L137](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L137)）是所有尝试的底层：`commit_if_ok` 包住 `sup`/`lub`；需要时做 leak check（fn 指针、fn item 的 binder）。**coercion 失败的兜底就是子类型**：没有专门规则匹配时走 `unify`，所以 `?X` coerce 到 `u32` 等价于 `?X = u32`。

## 2. `Coerce::coerce` 的判定顺序

`coerce`（[#L221](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L221)），两边先 `shallow_resolve`：

1. **source 是 `!`**：`coerce_never` 时返回 `[NeverToAny → target]`；否则只能 unify。
2. **source 是未解 `TyVar`**：`coerce_from_inference_variable`（[#L315](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L315)）。target 也是变量时登记 `PredicateKind::Coerce(a, b)` obligation（LUB 模式新建公共目标、登记两条）；否则 unify。挂起的 `Coerce` 谓词之后只按子类型处理，不会再插 adjustment。
3. **unsize**：`commit_if_ok(coerce_unsized)`（[#L508](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L508)）。任一边是 `TyVar` 直接失败；target 为标量、数组、slice、元组、fn、`dyn` 等内建类型时快速失败；`&str→&str` 快速失败。成功路径：两边都是引用时先重借用 `[Deref, Borrow(Ref(m_b), 无两阶段)]`（`&→*` 用 `Borrow(RawPtr)`），再 `unify_and(?T, target, Pointer(Unsize))`，然后在旧求解器下用一个**局部选择循环** `coerce_unsized_old_solver`（[#L664](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L664)）当场证明 `Source: CoerceUnsized<?T>` 及嵌套的 `Unsize`：歧义即失败（唯一例外：`?V: Unsize<dyn Trait>` 且 `?V: Sized` 已知），无 impl 即失败。
4. **按 target 分派**：`RawPtr` → `coerce_to_raw_ptr`；`Ref` → `coerce_to_ref`；`Pin`（需 feature）。
5. **按 source 分派**：`FnDef` → `coerce_from_fn_item`；`FnPtr` → `coerce_from_fn_pointer`；`Closure` → `coerce_closure_to_fn`；其他 unify。

注意 unsize 在 deref coercion 之前尝试，因此 `&[T; N] → &[T]` 走 unsize 而不是 autoderef。

## 3. 各类 coercion

| 种类 | 函数 | adjustment |
|---|---|---|
| 重借用 / `&mut T→&T` / deref coercion | `coerce_to_ref`（[#L364](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L364)） | autoderef 步骤 + `Borrow(Ref(m_b))` |
| `&T→*const T`、`&mut T→*mut T` | `coerce_to_raw_ptr`（[#L913](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L913)） | `[Deref(None), Borrow(RawPtr(m))]` |
| `*mut T→*const T` | 同上 | `Pointer(MutToConstPointer)` |
| fn item → fn ptr | `coerce_from_fn_item`（[#L868](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L868)） | `Pointer(ReifyFnPointer(safety))` |
| 安全 fn ptr → unsafe fn ptr | `coerce_from_fn_pointer`（[#L849](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L849)） | `Pointer(UnsafeFnPointer)` |
| 无捕获闭包 → fn ptr | `coerce_closure_to_fn`（[#L894](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L894)） | `Pointer(ClosureFnPointer(safety))` |
| unsize（`&[T;N]→&[T]`、`Box<T>→Box<dyn Tr>`） | `coerce_unsized` | 可选重借用 + `Pointer(Unsize)` |
| never | `coerce` 首分支 | `NeverToAny` |

`coerce_to_ref` 细节：

- source 必须是引用，且 `coerce_mutbls(m_a, m_b)`（`&`→`&mut` 拒绝）；否则 unify。
- 沿 `autoderef(source)` 逐步尝试：第 0 步跳过（否则 `&T` 会被 coerce 成 `&&T`）；第 k 步把 `&m_b deref_ty` 与 target unify，第一个成功的即结果。比较的是“autoref 后的引用类型”而不是 pointee，以保持 `&`/`&mut` 不同的 pointee variance。
- 成功后 adjustment = `adjust_steps(autoderef)`（每步 `Deref(None)` 或 `Deref(Some(Overloaded))`）+ `Borrow(Ref(AutoBorrowMutability::new(m_b, allow_two_phase)))`。
- `&T → &T` 且只走一步时不产生 adjustment；`&mut` 永远产生重借用（[#L461](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L461)）。
- 一次 coercion 只能做 deref **或** unsize，不能组合：`let a: &[u8] = &Foo;`（`Foo: Deref<Target=[u8;2]>`）报错。

## 4. LUB coercion：`CoerceMany`

- **`CoerceMany { expected_ty, final_ty, expressions }`**（[#L1381](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L1381)）用于 if/else、match arm、块尾、`break` 值、`return`/函数体尾、数组元素。
- `coerce_inner`（[#L1468](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L1468)）：第一个表达式做**一对一** `fcx.coerce(e, ty, expected_ty)`；之后每个用 `try_find_coercion_lub(cause, prev_exprs, merged_ty, new_e, new_ty)`；无表达式的“强制 unit”（缺 else、空 `break`、`return;`）直接 `eq(merged, ())`。错误时 `final_ty` 不更新并 taint。
- `complete`（[#L1922](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L1922)）：一个表达式都没有（全部发散或零 arm）时类型为 `!`。
- `try_find_coercion_lub`（[#L1168](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L1168)）三步：
  1. 两边都是 `FnDef`/闭包：先试直接 `lub`（同一函数的不同实例），失败则两边都 reify 成 fn 指针（安全/unsafe 取较弱者），给**所有**已有表达式和新表达式加 `Reify`/`ClosureFnPointer`。
  2. 若新表达式尚无 adjustment，尝试 `coerce(new_ty → prev_ty)`（`use_lub = true`），成功只调整新表达式。
  3. 否则 `coerce(prev_ty → new_ty)`，成功后把同一组 adjustment 加到**所有**先前表达式。这会产生“传递 coercion”，dev-guide 记载其中一些组合在 stable 上 ICE。
- LUB 用 `LatticeOp`：遇到变量时建新变量并登记两条子类型（见 [infer-ctxt.md §4](infer-ctxt.md)）；region 取 GLB（在 typeck 中被忽略）。
- 期望类型是 `TyVar` 时不作为 LUB 初值（`try_structurally_resolve_and_adjust_for_branches`），避免 `FnDef(a)` 先把外层变量钉死（dev-guide “Unnecessary inference constraints”）。

## 5. 调用位置的 coercion 参数

| 位置 | 函数 | two-phase |
|---|---|---|
| `let x: T = e` | `check_expr_coercible_to_type` | No |
| 函数/方法实参 | `check_argument_types` → `coerce` | **Yes** |
| 赋值右侧 | `check_expr_assign` → `demand_coerce` | No |
| if/match/块尾/break/return | `CoerceMany`（首个 `coerce`，其余 LUB） | No |
| `as` 的 coercion-cast | `CastCheck::try_coercion_cast` | No |
| 二元运算左操作数 | 先 coerce 到新变量（[op.rs#L200](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/op.rs#L200)） | No |

## 对 kfrc 的含义

- 标量 M1 只需要三件事：`!`→任意（`NeverToAny`）、“coercion 失败退回合一”、`CoerceMany` 的 LUB 流程（首个一对一、其余 LUB、全发散为 `!`、缺 else 强制 `()`）。这三件已足够决定 if/match/loop/return 的结果类型。
- `CoerceMany` 会**回头修改先前分支的 adjustment**；kfrc 若在 typeck 时就生成 Body，就无法回改。应该先在 side table 中记录 per-expr adjustment，等整个函数 typeck（含 fallback）结束后再建 Body。
- M2 引入引用时按 §2 的固定顺序实现：never → 变量 → unsize → 按 target（`&`、`*`）→ 按 source（fn item、闭包），不要各自零散实现。unsize 与 `CoerceUnsized` 需要 trait 求解器的“局部选择”接口（不是普通 obligation 注册），这是 typeck 与 trait 模块之间的第二种调用方式。

# 表达式降低：as_place / as_operand / as_rvalue / into 与求值顺序

源码：`compiler/rustc_mir_build/src/builder/expr/`。这里决定了 move 的具体位置、临时值何时产生、哪些检查（溢出、越界）成为可 unwind 的终结符。

## 四种降低入口

[`Category`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/category.rs#L4)：`Place`（`x`、`x.f`、`*p`、`a[i]`）、`Constant`（字面量）、`Rvalue(Into)`（分支/调用类，写入目的地）、`Rvalue(AsRvalue)`（算术、聚合等）。

- [`expr_into_dest(dest, block, expr)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L27)：把值写入给定 place，是主入口。Place 类表达式降为 `dest = Use(copy_or_move(place))`（[L738](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L738)）。
- [`as_place`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_place.rs#L417)（`expr_as_place`）：Place 类直接组投影；非 Place 类先 `as_temp`，临时值取该表达式在 region tree 中的 temp scope（可能被延长）。`as_read_only_place` 用于共享借用和 `&raw const`，非 Place 时建不可变临时。
- [`as_operand`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_operand.rs#L115)：`Constant` 且（不 needs_drop 或 `NeedsTemporary::No`）→ 常量操作数；**其余一律 `as_temp` 后 `Move(temp)`**，包括 `f(x)` 中的 `x`（`_t = move x; f(move _t)`）。`as_local_operand` 的临时值放在当前栈顶 scope（[`local_temp_lifetime`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_operand.rs#L14)）。
- [`as_temp`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_temp.rs#L16)：新 local，`StorageLive`，调度 `Storage` drop，`expr_into_dest(temp)`，再调度 `Value` drop。

Copy/Move 在这里定：[`consume_by_copy_or_move`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/misc.rs#L61) 按 `type_is_copy_modulo_regions` 选 `Operand::Copy` 或 `Operand::Move`。

## 各表达式的顺序

| 表达式 | 顺序与产物 | 位置 |
|---|---|---|
| 调用 `f(a, b)` | callee 操作数 → 参数从左到右（各进临时）→ `record_operands_moved` → `Call{dest, target, unwind: Continue}` → `diverge_from` | [into.rs L369](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L369) |
| 赋值 `lhs = rhs` | **先 RHS**（`as_local_rvalue`）→ 再 LHS place → lhs 类型 needs_drop 时 `Drop{replace}` + 写入，否则直接写入 | [stmt.rs L33](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/stmt.rs#L33) |
| 复合赋值（内建） | 先 RHS 操作数 → 再 LHS place → `lhs = op(copy lhs, rhs)`；重载版本已是方法调用，左到右 | [stmt.rs L59](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/stmt.rs#L59) |
| 二元运算 | lhs 操作数 → rhs 操作数 → 开启溢出检查时 `AddWithOverflow` 等 + `Assert`（可 unwind） | [`build_binary_op`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_rvalue.rs#L498) |
| 下标 `a[i]` | base place → 下标进新的不可变临时 → 取长度（数组常量 + `FakeRead(ForIndex)`；slice 用 `PtrMetadata`）→ `Lt` + `Assert` | [`lower_index_expression`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_place.rs#L618)、[`bounds_check`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_place.rs#L724) |
| 结构体字面量 | 字段按**源码书写顺序**求值进临时 → FRU 的 base 之后读缺失字段 → 一条 `Aggregate` 写目的地 | [into.rs L483](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L483) |
| `&&` / `\|\|` | 控制流：LHS 走 `then_else_break`，短路边直接写常量 | [into.rs L159](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L159) |
| `if` | 条件在 if-then scope 里降低，else 树收集失败路径 drop；无 else 时 else 块写 `()` | [into.rs L68](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L68) |
| `loop` | 头块 `FalseUnwind` + `diverge_from`；体写 unit 临时；只能由 break 退出 | [into.rs L207](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L207) |
| `&e` / `&mut e` | 共享借用用 `as_read_only_place`，其余 `as_place`，再 `Ref` | [into.rs L458](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L458) |

## tricky 边界

- 嵌套下标在 slice 上的 bounds check 要求后续下标求值期间 base 的指针不变：[`add_fake_borrows_of_base`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_place.rs#L755) 对 deref 前缀加 `Fake(Shallow)` 借用，最外层下标处 `FakeRead`；这些只给 borrowck 用，`CleanupPostBorrowck` 删除。
- 赋值先 RHS 后 LHS 与 region tree 中 `AssignOp` 的访问顺序一致；`*{side_effect(); p} = v` 时副作用在 RHS 之后。
- 结构体字面量字段若在第 k 个 panic，前 k-1 个临时值通过 unwind 树 drop；成功后这些临时值已被 move，正常路径上的 Drop 由 drop 显式化判成 `Dead` 删掉（Adt 不调用 `record_operands_moved`）。
- 常量操作数若类型 needs_drop 且要求临时（`NeedsTemporary::Maybe`），也会进临时以便调度 drop。

## 对 kfrc 的含义

- Typed Body 的“Operand = Copy(place) | Move(place) | Const”三分法是移植 move 分析的前提；Copy 判定需要 trait 求解（`Copy` impl、泛型约束），与 [T12](../../../plan/m1-vertical-chain.md#t12)/[T21](../../../plan/m3-semantics.md#t21) 耦合。首轮可只支持标量 + 无 Copy 的结构体两类（推断）。
- 溢出检查、越界检查、除零检查都是**可 unwind 的 Assert 终结符**，需要 cleanup 边；kfrc 现有标量链若把它们当纯值，T09 必须改。
- “参数总是先 move 进新临时”带来大量临时 local；SoN 构造时它们消失，但 Body 层和 drop/unwind 树要保留，否则 unwind 时的 drop 集合会不同。

# 两阶段借用与重借用

链接固定到 `e408947b`。冲突矩阵总表见 [loans.md](loans.md)。

## 1. 两阶段借用从哪来

只有 HIR typeck 明确标记的 `&mut` 自动借用才是两阶段（`AllowTwoPhase` 的说明，[rustc_middle/src/ty/adjustment.rs#L140](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/adjustment.rs#L140)）：

| 来源 | 位置 |
|---|---|
| 方法调用接收者 autoref（`v.push(..)`） | [method/confirm.rs#L195](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/confirm.rs#L195) |
| 重载二元/复合赋值运算的 autoref（`a += b`） | [op.rs#L266](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/op.rs#L266)、[#L278](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/op.rs#L278) |
| 函数实参强制转换插入的重借用（`foo(x, *x)`，`x: &mut u32`；见 `two-phase-nonrecv-autoref.rs` 的 `deref_coercion`） | [fn_ctxt/checks.rs#L391](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L391) → `coerce_to_ref`（[coercion.rs#L493](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L493)） |

其余强制转换、显式 `&mut x`、索引运算（[place_op.rs#L414](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/place_op.rs#L414)）都是 `AllowTwoPhase::No`。THIR 构造时 `AutoBorrowMutability::Mut { allow_two_phase_borrow: Yes }` 变成 `MutBorrowKind::TwoPhaseBorrow`（[rustc_mir_build/src/thir/cx/expr.rs#L1481](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/expr.rs#L1481)）；`allows_two_phase_borrow` 只对它为真（[statement.rs#L845](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/statement.rs#L845)）。

## 2. MIR 形状与激活点

```text
_t = &two_phase mut v;        // reservation R
_n = Vec::len(copy &v) ...    // R 与 A 之间：可读 v
Vec::push(move _t, move _n)   // activation A：_t 的唯一一次使用
```

- `GatherBorrows::insert_as_pending_if_two_phase`（[borrow_set.rs#L341](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/borrow_set.rs#L341)）：两阶段借用必须赋给一个**局部**临时（否则 `span_bug`），登记为 pending。
- `visit_local`（[#L276](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/borrow_set.rs#L276)）：该临时之后的第一次 use 是激活点，记入 `activation_map`；出现第二次 use 直接 `span_bug`。从未使用则保持 `NotActivated`。
- `TwoPhaseActivation { NotTwoPhase, NotActivated, ActivatedAt(Location) }`（[#L66](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/borrow_set.rs#L66)）。

## 3. 检查规则

- 预留点：`consume_rvalue` 发 `(Deep, Reservation(MutableBorrow))`（[lib.rs#L1465](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1465)）。可变性检查只在预留点做。预留与已有 Shared loan **兼容**（#56254 之后），与已有 Mut loan 冲突，也与自己冲突（循环里会撞上上一轮的自己）。
- 预留到激活之间：别处的 `Read` 碰到这个 loan 时，若 `is_active` 为假就放行（[lib.rs#L1301](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1301)）；`Write`（包括另一个 `&mut`）照常冲突。
- 激活点：`check_activations`（[lib.rs#L1819](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1819)）在该 location 最开始发 `(Deep, Activation(MutableBorrow, idx))`，与除自身外所有在作用域、重叠的 loan 冲突——中间新建且仍活着的共享借用在这里报错（`v.extend(&v)`）。
- `is_active`（[path_utils.rs#L60](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/path_utils.rs#L60)）用支配树：被 A 支配 ⇒ active；被 R 的下一个点支配 ⇒ 未激活；否则（例如绕回循环头）⇒ active。前提是 R 支配 A、A 后支配 R（忽略 unwind）。

## 4. 重借用

- **typeck 插入**：期望类型是引用时，`coerce_to_ref`（[coercion.rs#L364](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L364)）沿 autoderef 找到可统一的一步，生成 `Deref…` + `Borrow` adjustment，即 MIR 里的 `&mut *p` / `&*p`。`&T → &T` 且只 deref 一步时跳过（no-op，[#L462](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L462)）；`&mut` 总是重借用，所以 `foo(self.x)` 不会 move 掉 `self.x`。源码注释说明这个分支会影响闭包捕获和借用检查结果。
- **region 约束** `add_reborrow_constraint`（[type_check/mod.rs#L2319](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L2319)）：从最外层投影往里走，每遇到对 `&'r` 的 deref 加 `'r: 'borrow`；遇到 `&` 的 deref 或裸指针就停；`&mut` 和 `Box` 继续往里。例：
  - `&**x`，`x: &'a &'b u32`：只加 `'b: 'r`（外层 `&'b` 是共享的，停）。返回 `&'a u32` 需要 `'b: 'a`，靠隐含界成立（`region-lbr1-does-outlive-lbr2-because-implied-bound.rs`）。
  - `&mut *p.pointer`，`p: &'a mut S<'b>`、`pointer: &'b mut isize`：加 `'b: 'r` 和 `'a: 'r`；返回 `S<'b>` 要 `'r: 'b`，推出要求 `'a: 'b` 而无证据 ⇒ 报错（`borrowck-reborrow-from-shorter-lived-andmut.rs`）。
- **loan 层面**：`&mut *p` 的 loan 借的是 `*p`（局部 `p`）。在它活着时使用/move `p` 是深访问 ⇒ 冲突；**给 `p` 整体赋新值**是 `Shallow(None)` 写，浅访问遇到借用路径里的 `Deref` 停下 ⇒ 不冲突，并且 `kill_borrows_on_place(p)` 杀掉 `*p` 上的旧 loan。经 `&` 的重借用不记 loan（`ignore_borrow`）。

## 5. tricky 边界

- 两阶段 loan 的“未激活”区间由支配关系判断，不经数据流传播；loan 被带回循环头时 `is_active` 为真，下一轮的预留撞上自己（`two-phase-across-loop.rs`）。
- 显式写的 `&mut a` 不是两阶段，unsize/索引强制转换也不是：`double_access(&mut a, &a)`、`i[i[3]] = 4` 报 E0502，而 `v.push(v.len())`、`a.m(a.i(10))` 通过（均见 `two-phase-nonrecv-autoref.rs`）。
- 嵌套的 `&mut self` 调用仍然报错：内层是 `Write`，不是 `Read`（`two-phase-cannot-nest-mut-self-calls.rs`）。
- `v[0].push_str({ v.push(..); .. })`：外层借用经过 `IndexMut`，不是两阶段，报 E0499（`two-phase-sneaky.rs`）。

## 6. 对 kfrc 的含义

- Typed Body 要显式记录 typeck 的调整：`Deref`（内建/重载）、`Borrow(kind, two_phase: bool)`。两阶段标记只在上表三类位置打；多打会接受 rustc 拒绝的程序。
- Body 构造必须保证两阶段临时“赋值一次、使用至多一次”，借用检查依赖这个结构。需要 Body CFG 的支配树。
- 重借用约束只依赖投影链和每层基类型，规则短，可直接移植；它是 [T13](../../roadmap/m1-vertical-chain.md#t13) “可变借用与重借用”最小闭包的一部分。

# Loan：生成、失效与冲突判定

源码根：`compiler/rustc_borrowck/src/`，链接固定到 `e408947b`。

## 1. 数据结构

- **`BorrowSet`**（[borrow_set.rs#L16](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/borrow_set.rs#L16)）：`location_map: FxIndexMap<Location, BorrowData>`（下标即 `BorrowIndex`，一个 location 至多一个 loan）、`activation_map`、`local_map: Local → {BorrowIndex}`、`locals_state_at_exit`。按 CFG **前序**收集（[#L189](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/borrow_set.rs#L189)）。
- **`BorrowData`**（[#L73](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/borrow_set.rs#L73)）：`reserve_location`、`activation_location: TwoPhaseActivation`、`kind: BorrowKind`、`region: RegionVid`（`Rvalue::Ref` 上的 region）、`borrowed_place`、`assigned_place`。
- **`BorrowKind`**（[rustc_middle/src/mir/syntax.rs#L155](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/syntax.rs#L155)）：`Shared`、`Fake(Shallow|Deep)`（match 守卫/判别用）、`Mut { Default | TwoPhaseBorrow | ClosureCapture }`（[#L202](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/syntax.rs#L202)）。`ClosureCapture` 即“unique immutable”，见 [closures.md](closures.md)。

## 2. 哪些借用不成为 loan（`ignore_borrow`，[place_ext.rs#L15](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/place_ext.rs#L15)）

- 经过 `&T` 或裸指针的 deref：`&(*r).f`（`r: &T`）不记 loan——原路径是 `Copy` 的，用户对它做的事都不会使新引用失效。只靠 region 约束（[two-phase-and-reborrow.md](two-phase-and-reborrow.md)）保证安全。线程局部静态的第一层 deref 例外。
- 非 fn/闭包 body（const、static）里：从未 `StorageDead` 也未被 move 的不可变局部。fn 和闭包是 `AllAreInvalidated`，此规则不适用。

## 3. `Borrows` 数据流（loan 何时在作用域）

前向、gen/kill，域 `MixedBitSet<BorrowIndex>`（[dataflow.rs#L520](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/dataflow.rs#L520)）。合流取并集（may）。

- **early effect**（每条语句和 terminator 之前）：`kill_loans_out_of_scope_at_location`（[#L446](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/dataflow.rs#L446)）——杀掉预先算好的“此点超出 NLL 作用域”的 loan。先 kill 后 gen，同一点重新借用不会被误杀。
- **primary effect**（[#L544](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/dataflow.rs#L544)）：
  - `Assign(lhs, Ref(..))`：gen 该 loan；然后对任何 `Assign` 执行 `kill_borrows_on_place(lhs)`。
  - `StorageDead(l)`：`kill_borrows_on_place(l)`。
  - inline asm 输出同赋值。
- **`kill_borrows_on_place`**（[#L468](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/dataflow.rs#L468)）：lhs 是无投影局部 ⇒ 杀掉该局部上的**所有** loan（包括穿过它 deref 的 `(*list).value`）；有投影 ⇒ 只杀 `places_conflict(.., NoOverlap)` 确定重叠的。这是“给引用变量重新赋值就释放旧 loan”的来源（`list = n` 循环，见 [test-cases.md](test-cases.md)）。

### NLL 作用域终点（`OutOfScopePrecomputer`，[#L181](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/dataflow.rs#L181)）

对每个 loan，从 `reserve_location` 起沿 CFG DFS；每个块找第一个**不在** `region(loan)` 值里的语句下标（`first_non_contained_inclusive`，[values.rs#L304](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/values.rs#L304)），在那里登记 kill 并停止该分支；块内全包含则继续到后继（[#L211](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/dataflow.rs#L211)）。因此 loan 作用域 = 从借用点出发、region 连续包含的那段 CFG。

## 4. 冲突判定

`check_access_for_conflict`（[lib.rs#L1246](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1246)）经 `each_borrow_involving_path`（[path_utils.rs#L13](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/path_utils.rs#L13)）：只看 `local_map[place.local]` 中在作用域内、且 `borrow_conflicts_with_place(.., Overlap)` 为真的 loan。

| 访问 \ 已有 loan | Shared / Fake | Mut |
|---|---|---|
| `Read(_)` | 允许 | 冲突（未激活的两阶段 loan 除外，`is_active`） |
| `Read(Borrow(Fake Shallow))` | 允许 | 允许 |
| `Reservation(_)` | 允许（#56254 后） | 冲突 |
| `Write(Move)` | Shared、Fake Deep 冲突；Fake Shallow 允许（交给初始化检查） | 冲突 |
| `Write(_)` / `Activation(_)` | 冲突 | 冲突（激活自己的 loan 除外） |

错误种类按 `WriteKind` 分：`MutableBorrow` → E0499/E0502；`StorageDeadOrDrop` → “does not live long enough”（E0597 等）；`Mutate`/`Replace` → E0506；`Move` → E0505；`Read(Copy)` 撞 Mut → E0503。

## 5. 路径重叠（`places_conflict.rs`）

`place_components_conflict`（[#L131](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/places_conflict.rs#L131)）从局部开始逐个投影比较（[`place_projection_conflict`#L290](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/places_conflict.rs#L290)）：

- 不同局部：不冲突。同字段 / 同 variant / `Deref`-`Deref`：继续；不同 struct 字段、不同 enum variant：不相交；不同 union 字段：`Arbitrary` ⇒ 冲突。
- `Index` 对任何索引：按 bias——冲突检查用 `Overlap`（当作可能相等），kill 用 `NoOverlap`（当作不等）。两个常量索引按 offset/from_end 精确比较。
- 访问路径较短（借 `a.b.c`，访问 `a.b`）：深访问冲突；浅访问遇到借用路径里的 `Deref` 就停（写 `x` 不碰 `*x`）；`Drop` 访问穿过 `&mut` deref 不冲突，经过有 `Drop` impl 的 ADT 字段必冲突。
- 借用路径较短：冲突；唯一例外是 Fake Shallow 借用对更长的访问不冲突。

## 6. 出口与可变性

- `check_for_invalidation_at_exit`（[lib.rs#L1756](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1756)）：`Return`/`UnwindResume` 时仍在作用域、借的是本地数据的 loan 报错。返回 `&local` 之所以被拒，是因为 loan 的 region 流入返回类型的 universal region、作用域覆盖到出口。
- `check_access_permissions`（[#L2341](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2341)）+ `is_mutable`（[#L2501](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2501)）：`&mut` deref 后改为“根局部可变性无关”（`let r = &mut x; *r = 1` 合法）；`&` deref 和 `*const` 拒绝；`Box` deref 继承根局部。对未曾初始化的不可变局部赋值（`let x; x = 1;`）合法，靠 `EverInitializedPlaces` 判断。

## 7. 对 kfrc 的含义

- 需要的事实：每个 loan 的 `(location, kind, region 槽, borrowed Place, assigned Place)`；每个 location 的访问列表（Place + 深/浅/drop + 读/写种类）；`StorageDead` 等价的作用域结束点。全部是 Body 层可直接产出的结构化数据，不依赖 TyCtxt（`places_conflict` 只需要投影基类型：是否 union、是否 `&`/`&mut`/裸指针/`Box`、ADT 是否有 `Drop`）。
- `Borrows` 必须吃 region 推导的结果才能算 kill；“loan 事实”和“region 推导”不能拆成两个独立实验。
- Place 投影要保留 `Downcast(variant)`、常量索引（`from_end`、`min_length`）、`Subslice`，否则 enum 分支借用和切片模式的接受集合会偏离 rustc。
- 错误分类建议用 `WriteKind × BorrowKind` 这张表产出“错误码 + 主 span”，与 oracle 比较时不比较消息文本。

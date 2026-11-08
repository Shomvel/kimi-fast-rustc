# 总体流程：从 query 到报错

源码根：`compiler/rustc_borrowck/src/`。链接前缀省略为 `…/blob/e408947b…/`，全部指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. 入口与调度单位

- **query `mir_borrowck(def)`**（[lib.rs#L117](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L117)）只对 typeck root 调用；闭包、coroutine、inline const 随父函数一起检查。输入是 `mir_promoted`：已建好的 MIR，含 `StorageDead`、`FakeRead`、fake borrow、未展开的 `Drop`。
- **调用点**：`mir_drops_elaborated_and_const_checked`（[rustc_mir_transform/src/lib.rs#L516](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/lib.rs#L516)）先调 `mir_borrowck`，再 steal `mir_promoted` 做后续 pass（含 drop 展开）。借用检查在**克隆**的 body 上做，结果只用来 taint body；返回值是 opaque type 的 hidden type 表。
- **`BorrowCheckRootCtxt::do_mir_borrowck`**（[root_cx.rs#L242](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/root_cx.rs#L242)）：
  1. 对 `nested_bodies_within(root)` 再加 root，逐个 `borrowck_collect_region_constraints`；`FxIndexMap` 保证子 body 在父 body 之前。
  2. `apply_closure_requirements_modulo_opaques`（[#L144](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/root_cx.rs#L144)）：把子 body 的 region 要求加到父 body；不依赖 opaque 的 body 当场完成检查。
  3. `handle_opaque_type_uses`（[#L85](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/root_cx.rs#L85)）：所有 body 一起推 opaque hidden type（见 [opaques.md](opaques.md)）。
  4. 剩余 body 应用最终闭包要求，跑 `borrowck_check_region_constraints`。
- 1.94 没有单独的 `do_mir_borrowck` 自由函数；原先的流程拆成上面两半。

## 2. 每个 body 的两半

**收集**（`borrowck_collect_region_constraints`，[lib.rs#L309](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L309)）：

1. 克隆 body；`nll::replace_regions_in_mir`（[nll.rs#L56](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/nll.rs#L56)）先建 `UniversalRegions`，再 `renumber_mir` 把 body 里每个 region 换成新的 NLL 变量。
2. `MoveData::gather_moves`（move path，见 [../ownership/](../ownership/README.md)）。
3. `BorrowSet::build`（[borrow_set.rs#L169](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/borrow_set.rs#L169)）：每个 `Rvalue::Ref` 一个 loan。
4. `DenseLocationMap`：`Location` ↔ 连续 `PointIndex`。
5. `type_check::type_check`（[type_check/mod.rs#L98](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L98)）：MIR 类型检查，产出 outlives 约束、liveness、type test、placeholder；末尾 `liveness::generate`。

**检查**（`borrowck_check_region_constraints`，[lib.rs#L389](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L389)）：

6. `nll::compute_regions`（[nll.rs#L113](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/nll.rs#L113)）：SCC → `RegionInferenceContext::new` → `solve`，得到每个 region 的点集、闭包要求、region 错误。
7. promoted body 只查非法 move。
8. 报 region 错误（或延迟的 opaque 错误）。
9. `get_flow_results`（[lib.rs#L597](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L597)）：`Borrows`、`MaybeUninitializedPlaces`、`EverInitializedPlaces` 三个前向数据流各自到不动点，再拼成 `BorrowckDomain`。
10. 按逆后序访问每个 location，`MirBorrowckCtxt` 作为 `ResultsVisitor`（[lib.rs#L786](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L786)）检查访问。
11. 汇报 move 错误、`unused_mut` lint；返回 `PropagatedBorrowCheckResults { closure_requirements, used_mut_upvars }`。

## 3. MIR 语句 → 访问种类

`visit_after_early_statement_effect` / `_terminator_effect`（[lib.rs#L787](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L787)–[#L1034](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1034)）。每个 location 先 `check_activations`（两阶段借用激活）。

| MIR | 访问（`AccessDepth`, `ReadOrWrite`） |
|---|---|
| `Assign(lhs, rv)` | 先 `consume_rvalue(rv)`，再 `mutate_place(lhs, Shallow(None))` |
| `&place` / fake deep | `(Deep, Read(Borrow))` |
| `&mut place`（两阶段） | `(Deep, Reservation(MutableBorrow))`，激活点再 `(Deep, Activation)` |
| `&mut place` | `(Deep, Write(MutableBorrow))` |
| fake shallow borrow（match） | `(Shallow(FakeBorrow), Read(Borrow))` |
| `Operand::Copy(p)` | `(Deep, Read(Copy))` + 初始化检查 |
| `Operand::Move(p)` | `check_movable_place` + `(Deep, Write(Move))` + 初始化检查 |
| `StorageDead(l)` | `(Shallow(None), Write(StorageDeadOrDrop))` |
| `Drop{place, replace}` | `(AccessDepth::Drop, Write(StorageDeadOrDrop / Replace))` |
| `Call` | 消费 func/args，`mutate_place(dest, Deep)` |
| `Return`/`UnwindResume` | 对每个仍在作用域的 loan 做 `check_for_invalidation_at_exit` |

`access_place`（[lib.rs#L1179](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1179)）= `check_access_permissions`（可变性）+ `check_access_for_conflict`（loan 冲突，见 [loans.md](loans.md)）。同一 (place, span) 只报一次。

## 4. NLL 还是 Polonius（已核实）

- `-Zpolonius` 默认 `Off`（[rustc_session/src/config.rs#L3352](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/config.rs#L3352)）；`legacy` 走 datalog `polonius-engine`，`next` 是树内原型（[options.rs#L2570](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/options.rs#L2570)）。**1.94 稳定版行为 = NLL。**
- NLL：region 值是“CFG 点 + end('a)”集合，由 liveness 决定；outlives 约束传播**不看位置**（`propagate_constraints` 只做集合并）。所以 NLL problem case #3 被拒绝（[get_default.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/nll/get_default.rs) 的 `[nll]` 注释）。
- `next`：把约束按位置“局部化”成图，做 loan 可达性（[polonius/loan_liveness.rs#L15](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/polonius/loan_liveness.rs#L15)），再用 `PoloniusOutOfScopePrecomputer`（[dataflow.rs#L294](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/dataflow.rs#L294)）算 loan 作用域。

## 5. 对 kfrc 的含义

- 只实现 NLL。oracle 是 stable rustc，接受集合以 NLL 为准；Polonius 测试只作“将来可能放宽”的备注。
- CheckedBody 不需要携带 region 解：rustc 下游重新取未改动的 MIR，region 在 codegen 前擦除。借用检查输出只有：错误、闭包 region 要求、`used_mut_upvars`、opaque hidden type。
- 顺序：借用检查在 drop 展开**之前**，面对的是未展开的 `Drop`；与 [T13](../../roadmap/m1-vertical-chain.md#t13) “先分析再 drop 显式化”一致。
- 调度单位是 typeck root（父函数 + 全部嵌套闭包），子 body 先于父 body。任务图、缓存键要按 root 建，而不是按单个 body。

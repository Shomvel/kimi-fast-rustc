# match / 模式降低：决策树、false edge、fake borrow、guard 与 or-pattern

源码：`compiler/rustc_mir_build/src/builder/matches/{mod.rs,match_pair.rs,buckets.rs,test.rs,util.rs}`。穷尽性检查在 `rustc_pattern_analysis`（THIR 阶段），不在本文。`let`、`if let`、`let-else`、函数参数模式都复用同一套机器。

## 总流程

[`match_expr`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L352)：

1. [`lower_scrutinee`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L394)：scrutinee 以 **place** 降低（不 move），加 `PlaceMention`。
2. [`lower_match_tree`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1574)：每个 arm 一个 `Candidate`；收集 fake borrow；`match_candidates` 生成测试；补 false edge；不可反驳时 otherwise 块 `FakeRead(ForMatchedPlace)` + `Unreachable`（[L1662](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1662)，让 `let x: !; match x {}` 读到未初始化）。
3. [`lower_match_arms`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L415)：每个 arm 在 arm scope（+ guard 时 `MatchGuard` scope）内声明 binding、绑定/跑 guard（[`bind_pattern`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L518)）、降低 body，所有 arm 汇合到 end 块。

## 数据结构

- [`FlatPat`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L989)：`match_pairs + extra_data{bindings, ascriptions, is_never}`，由 [`MatchPairTree::for_pattern`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/match_pair.rs#L90) 把模式拍平：不需测试的部分（绑定、通配、结构体/元组外壳）直接消解成 binding 或子 pair。
- [`MatchPairTree`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1276)：`place` + [`TestableCase`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1231)（`Variant / Constant / Range / Slice / Deref / Never / Or`）+ `subpairs`（父测试通过后才能测的，如 `Some(3)` 里的 `3`）。
- [`Candidate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1025)：剩余 `match_pairs`（or-pattern 排在最后）、`subcandidates`（or 展开后的树）、`has_guard`、`pre_binding_block`、`otherwise_block`、`false_edge_start_block`。
- [`TestKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1320)：`Switch{adt}`（读判别式 + `SwitchInt`）、`SwitchInt`（整数/char）、`If`（bool）、`StringEq`（调用 `<str as PartialEq>::eq`，[`string_compare`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/test.rs#L412)，是 Call，可 unwind）、`ScalarEq`、`Range`、`SliceLen{len, op: Eq|Ge}`、`Deref`（调用 `deref`/`deref_mut`）、`Never`。生成在 [`perform_test`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/test.rs#L68)。

## 决策树：回溯自动机

[`match_candidates_inner`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1759) 循环处理候选前缀：

- 首候选 match pair 已空 → [`select_matched_candidate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1831)：当前块即 pre-binding 块，新建 otherwise 块作为下一候选的起点；
- 有候选以 or-pattern 开头 → [`expand_and_match_or_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1851)；
- 否则 [`test_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L2279)：取**首候选的首 pair** 定测试（保证进展），[`partition_candidates_into_buckets`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/buckets.rs#L58) 把候选按测试结果分桶并改写其 pair；遇到第一个“测试对它无意义”的候选就停，剩下的从 `remainder_start` 继续。每个桶递归 `match_candidates`，桶的 otherwise 跳 `remainder_start`。

候选永不复制（`&mut Candidate`），代价是同一 place 可能被测多次；代码规模线性而非指数。

## or-pattern

展开规则（L1851 注释）：只展开开头是 or-pattern 的候选；若它后面还有别的 pair，展开到它为止（否则 otherwise 无法区分来源）。[`create_or_subcandidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1951) 为每个分支建子候选；降低后 [`merge_trivial_subcandidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L2023) 在无 guard、子候选都无 binding 时合并成一个 pre-binding 块；剩余 pair 由 `test_remaining_match_pairs_after_or` 复制给每个叶子。

一个 arm 因此可能有多个 sub-branch，**每个 sub-branch 各自绑定、各自跑一份 guard**，再汇到同一 arm 块。drop 只为最后降低的 sub-branch 调度；sub-branch 逆序降低，使 drop 顺序跟随第一个分支中 binding 的出现顺序（#142163，[`bind_pattern`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L518)）。

## false edge

[`lower_match_tree`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L1613) 逆序给每个叶子候选在 pre-binding 前插 `FalseEdge{real, imaginary: 下一候选的 false_edge_start_block}`；有 guard 时 guard 失败处也插。效果：borrowck 认为“任何之前的 arm 和 guard 都可能执行过、任何 arm 都可能被选中”，结论不依赖具体决策树形状（match_expr 文档 L278–L330 的四个例子，测试 `tests/ui/nll/match-cfg-fake-edges.rs`）。`CleanupPostBorrowck` 之后 `FalseEdge` 当作 Goto real。

## guard、binding 与 fake borrow

[`bind_and_guard_matched_candidate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L2401)：

1. guard 前 [`bind_matched_candidate_for_guard`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L2645)：by-value binding 在 guard 内是**另一个 local**，存 `&place`，guard 中的 `x` 解释为 `*ref`；by-ref binding 先建真实引用，guard 用它的引用。所以 guard 永远不 move。
2. 写入 fake borrow：[`collect_fake_borrows`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/util.rs#L69) 只在有 guard 时收集：被测试的 place 及其 deref 前缀用 `Fake(Shallow)`（不冲突于子字段的 `&mut`），deref pattern 用 `Fake(Deep)`；被绑定的 place 只借它的 deref 前缀（所以 `match u { _ if { u = true; false } => .., x => .. }` 合法）。guard 成功处 `FakeRead(ForMatchGuard)` 让它们活过 guard，阻止 guard 修改 scrutinee。生成代码前由 `CleanupPostBorrowck` 删除（它们可能指向非活跃 variant）。
3. guard 在 `in_if_then_scope(match_scope)` 中降低，失败路径 drop guard 中的临时值/绑定并跳 otherwise。
4. guard 通过后才 [`bind_matched_candidate_for_arm_body`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L2710)：`StorageLive`、调度 drop、`x = copy_or_move(place)` 或 `x = &place`。move 发生在这里。

`if let` / let 链：[`lower_let_expr`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L2344) 建一个可反驳 match tree，otherwise 走 `break_for_else`。`let pat = e;` 的简单绑定直接写入变量，复杂模式走 `place_into_pattern`。

## tricky 边界

- scrutinee 是 place：`match x { y => .. }` 在 arm 内才 move `x`；`match f() {..}` 的 `f()` 是临时值，活到整个 match 所在语句结束。
- guard 中 move by-value binding 不可能；guard 内对 scrutinee 的修改被 fake borrow 拒绝，但对未被测试的兄弟字段 `&mut` 允许。
- 字符串/deref 测试是调用，带 unwind 边，unwind 时 drop 已有临时值。
- 同一 binding 在 or-pattern 不同分支来自不同 place，sub-branch 分开绑定。

## 对 kfrc 的含义

- 首轮实验（T13）只需整数/bool/enum variant 测试、无 guard、无 or-pattern；guard + fake borrow + or-pattern 是 T22 的独立子包。
- Body 需要 `FalseEdge`（或等价的“仅检查可见”边）与 `FakeRead`、`Fake` 借用这类只供检查的语句，并有一个删除它们的规范化步骤；SoN builder 只接收删除后的 Body。
- 移植量（推断）：matches/ 约 4600 行，算法本身依赖少（类型只用于测试种类和判别值），但与 THIR 模式表示、穷尽性检查输出强耦合，需先冻结 kfrc 的模式 IR。

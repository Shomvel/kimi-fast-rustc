# 初始化数据流与 move 错误判定

源码：`compiler/rustc_mir_dataflow/src/impls/initialized.rs`、`compiler/rustc_mir_dataflow/src/drop_flag_effects.rs`、`compiler/rustc_borrowck/src/lib.rs`。借用冲突、loan、region 不在本文，见 [../borrowck/](../borrowck/README.md)。

## 三个分析

| 分析 | 域 | 初值 | join | 用途 |
|---|---|---|---|---|
| [`MaybeInitializedPlaces`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/impls/initialized.rs#L138) | `MaybeReachable<MixedBitSet<MovePathIndex>>` | 入口：参数子树置位 | 并 | drop 显式化；NLL 的 drop-liveness |
| [`MaybeUninitializedPlaces`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/impls/initialized.rs#L228) | `MixedBitSet<MovePathIndex>` | 入口：全置位，再清掉参数子树 | 并 | borrowck 的“使用未初始化/已 move”；drop 显式化 |
| [`EverInitializedPlaces`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/impls/initialized.rs#L312) | `MixedBitSet<InitIndex>` | 入口：参数的 init | 并 | 不可变变量是否被二次赋值（E0384） |

“一定已初始化” = 不在 maybe-uninit 中；“需要 drop flag” = 同时在 maybe-init 和 maybe-uninit 中。

borrowck 只跑 `Borrows + MaybeUninitializedPlaces + EverInitializedPlaces` 三个（[`get_flow_results`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L597)）；`MaybeInitializedPlaces` 在 borrowck 里只给 liveness 用（[trace.rs L487](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L487)）。

## 转移函数：`drop_flag_effects_for_location`

[`drop_flag_effects_for_location`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/drop_flag_effects.rs#L106) 是 maybe-init/maybe-uninit 共用的 gen/kill 源，顺序固定：

1. 本位置所有 `MoveOut`：对路径**及其全部后代**回调 `Absent`（[`on_all_children_bits`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/drop_flag_effects.rs#L65)）。
2. 本位置是 `Drop{place}` 且精确命中：整棵子树 `Absent`（drop 不是 move，但之后视为未初始化）。
3. 本位置的 init（[`for_location_inits`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/drop_flag_effects.rs#L137)）：`Deep` → 子树 `Present`；`Shallow` → 只自身；`NonPanicPathOnly` → 不处理。

Call destination 的初始化放在 `apply_call_return_effect`（[maybe-init L431](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/impls/initialized.rs#L431)），框架只在正常返回边上调用，unwind 边看不到。

精确 enum drop（`-Zprecise-enum-drop-elaboration`，默认开，[options.rs L2576](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/options.rs#L2576)）：

- `SwitchInt` 判别 `Discriminant(enum_place)` 时，按边把非活跃 variant 的子路径 kill（maybe-init）或 gen（maybe-uninit，仅 `mark_inactive_variants_as_uninit`，只有 drop 显式化打开）。实现 [`on_all_inactive_variants`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/drop_flag_effects.rs#L176)。
- 同一开关下，`&mut place` / `&raw place` 会把 place 子树标成 maybe-init（#90752，防止通过指针写入后漏 drop）。

`EverInitializedPlaces` 语句效果：gen 本位置所有 init；遇到 `StorageDead(local)` 时 kill 该 local 的全部 init（[L643](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/impls/initialized.rs#L643)），所以循环体内 `let x; x = 1;` 每轮都合法。终结符效果过滤掉 `NonPanicPathOnly`，正常返回边再补上。

## borrowck 怎样用这些结果判定 move 错误

入口 [`consume_operand`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1702)：`Copy` 做读访问 + 初始化检查；`Move` 先 `check_movable_place`（见 [move-paths.md](move-paths.md)），再做 `Deep Write(Move)` 访问（与 loan 冲突），再做初始化检查。

- [`check_if_full_path_is_moved`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1977)：`move_path_closest_to(place)` 找最近的被跟踪前缀，若它 maybe-uninit 就报错。覆盖“move 了 `a.b`，用 `a.b.c`”和“未初始化的 `r: &T`，用 `*r`”。
- [`check_if_path_or_subpath_is_moved`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2082)：再查精确路径的所有后代，任何一个 maybe-uninit 就报（部分 move 后使用整体，E0382）。数组子切片走 [`check_if_subslice_element_is_moved`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2043)，只看区间内元素。
- 赋值 [`mutate_place`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1431) → [`check_if_assigned_path_is_moved`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2182)：从内向外看投影——`*P` 要求 `P` 完整初始化；`P.f` 若 `P` 有 Drop impl 要求 `P` 及子路径都已初始化；否则对 ADT/元组走 [`check_parent_of_field`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2250)：沿“浅前缀”（不跨 deref）找最短的 maybe-uninit 前缀，有就报 `PartialAssignment`。这就是“`let s: S; s.x = 1;` 被拒绝”（#21232）的实现；union 在先前 move 后重新赋字段是例外。
- 不可变局部的二次赋值：[`check_access_permissions`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2341) 发现不可变后，只在 [`is_local_ever_initialized`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2460) 命中时才报错（E0384）；所以 `let x; x = 1;` 合法，两条分支各赋一次也合法。
- `Drop` 终结符：做 `AccessDepth::Drop` 的写访问（`replace` 时 `WriteKind::Replace`，[L893](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L893)），**不**做初始化检查——建 MIR 时插的 Drop 可能作用在已 move 的值上。
- `StorageDead`：浅写访问（[L845](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L845)），主要用于“借用活得不够久”。

错误报告 `report_use_of_moved_or_uninitialized`（[conflict_errors.rs L73](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/diagnostics/conflict_errors.rs#L73)）区分“从未初始化”（E0381，含“可能未初始化”）和“已 move”（E0382），并回溯 move 位置给出 note。

## tricky 边界

- match 的 false edge 让 borrowck 看见“任意之前的 arm/guard 可能执行过”，因此 `match` 后一个 arm 中的 move 会让 match 之后的使用报错，哪怕实际不可达（[match-lowering.md](match-lowering.md)）。
- 循环回边：作用域出口的 `StorageDead` 让循环体内局部在下一轮开始时是未初始化的；如果 Body 没有作用域结束事件，E0384/E0381 在循环里会误判。
- `NonPanicPathOnly`：`let x = f();` 里 `f` unwind 时 `x` 不算初始化；kfrc 若把 Call 结果写入放在 Call 节点上而不区分边，会在 unwind 路径上多 drop 一次。
- borrowck 的 move/初始化检查用的 `MaybeUninitializedPlaces` 不带 variant 信息（`mark_inactive_variants_as_uninit` 只有 drop 显式化打开）；但 NLL liveness 用的 `MaybeInitializedPlaces` 默认带 variant 边效果和 `&mut` 规则，会影响 drop-liveness（归 [../borrowck/](../borrowck/README.md)）。

## 对 kfrc 的含义

- 需要一个通用前向数据流框架：位集域、按语句/终结符的 gen/kill、**按边**的效果（call 正常返回边、SwitchInt 每条目标边）、到达不动点。rustc 的 `rustc_mir_dataflow::framework` 是可借算法；kfrc 可以只实现前向 + 并集 join（推断：首轮 ~400 行）。
- `MaybeReachable::Unreachable` 作为 maybe-init 的 bottom，避免不可达块污染结果；kfrc 用 RPO + 入口块初始化即可等价。
- 三个分析共享同一个 `MoveData`，只是解释不同；Body 改写（drop 显式化）前后要重建 move data。

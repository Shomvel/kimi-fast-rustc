# Drop 显式化、drop glue 与析构顺序

源码：`compiler/rustc_mir_transform/src/elaborate_drops.rs`（pass，按 move path 决定风格和 drop flag）、`compiler/rustc_mir_transform/src/elaborate_drop.rs`（展开引擎，pass 与 drop shim 共用）、`compiler/rustc_mir_transform/src/shim.rs`。dev-guide：[mir/drop-elaboration.md](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/src/doc/rustc-dev-guide/src/mir/drop-elaboration.md)。

## 在流水线中的位置

建 MIR 时在每个作用域出口无条件插 `Drop(local)`（见 [scopes-and-cleanup.md](scopes-and-cleanup.md)）→ borrowck（Drop 只当写访问）→ [`run_runtime_lowering_passes`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/lib.rs#L626)：`CriticalCallEdges` → `PostAnalysisNormalize` → `Subtyper` → **`ElaborateDrops`**（[L634](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/lib.rs#L634)）→ `AbortUnwindingCalls` → …。之后每个 `Drop` 都表示“调用该类型的 drop glue”。

## ElaborateDrops pass

[`run_pass`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L52)：`gather_moves(filter = needs_drop)`；跑 maybe-init（`exclude_inactive_in_otherwise` + `skipping_unreachable_unwind`）；[`compute_dead_unwinds`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L99) 找出“drop 的东西一定未初始化”的 Drop，让 maybe-uninit 不走它们的 unwind 边；再跑 maybe-uninit（打开 variant 规则）。然后 [`elaborate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L276) 依次：

1. [`collect_drop_flags`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L289)：对每个 Drop 的路径子树，凡在该点 `maybe_init && maybe_uninit` 的路径，建一个 `bool` 临时作为 drop flag（每条 move path 至多一个）。
2. [`elaborate_drops`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L335)：类型不 `needs_drop` 的直接改 Goto；精确命中的调 `elaborate_drop`；`Parent(Some)` 只允许 `replace` 的 drop（指针后面赋值），保留无条件 drop。unwind 目标转换：`Continue` → resume 块，`Unreachable` → unreachable cleanup 块，cleanup 块内 → `Unwind::InCleanup`。
3. [`drop_flags_on_init`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L421)：入口把所有 flag 置 false；`drop_flags_for_args` 把参数置 true。
4. [`drop_flags_for_fn_rets`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L430)：有 cleanup 的 Call，在**目标块开头**置 destination flag（`CriticalCallEdges` 保证目标块只有这一个前驱）。
5. [`drop_flags_for_locs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L461)：只遍历**原有**块，在每个 move/init 位置按 `drop_flag_effects_for_location` 写 flag；无 cleanup 的 Call 在调用前置 destination flag。新生成的块自己维护 flag，避免读之前被覆盖。

## DropStyle 与展开

[`drop_style`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L170)（Deep 模式聚合子树）：

| (maybe_init, maybe_uninit, 子树路径数>1) | [`DropStyle`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L20) | 生成 |
|---|---|---|
| (false, _, _) | `Dead` | Goto，删掉 drop |
| (true, false, _) | `Static` | 原样 `Drop` |
| (true, true, false) | `Conditional` | `if flag { Drop }` |
| (true, true, true) | `Open` | 逐字段展开（drop ladder） |

[`DropCtxt::elaborate_drop`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L472) 按风格分派；[`open_drop`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L1276) 按类型：

- 元组/闭包 upvar：[`open_drop_for_tuple`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L684)。
- ADT [`open_drop_for_adt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L744)：union/`ManuallyDrop` 不展开内容；struct 走 drop ladder；enum 走 [`open_drop_for_multivariant`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L812)：读判别式 `SwitchInt`（[`adt_switch_block`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L918)），每个有 move path 的 variant 一条 ladder，其余 variant 走整体 drop；**每个 variant 单独复制一份 unwind 半梯**（MSVC funclet 约束）。有 `Drop` impl 时先 `drop_flag_test_block` 再调用 `<T as Drop>::drop(&mut place)`（[`destructor_call_block_sync`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L954)），成功和 unwind 都接到字段 ladder。
- Box：[`open_drop_for_box_contents`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L704) 把内部指针 transmute 成 `*const T`，先 drop `*ptr`（若 `*b` 被 move 出则按子路径风格），再调 Box 的 `Drop`（释放）。
- 数组：[`open_drop_for_array`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L1110) 有常量下标子路径时，按“被 move 的单元素（Keep）/其余区间（Subslice）”建 ladder；否则转成 slice 指针走 [`drop_loop_trio_for_slice`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L1220)：三条循环（正常、unwind、coroutine drop），下标从 0 递增（[`drop_loop`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L1027)）。
- `dyn`：`complete_drop`，交给 vtable 中的 drop 函数。

**drop ladder**（[`drop_ladder`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L640)）：只保留 `needs_drop` 的字段；先建 unwind 半梯（每步 unwind 时 drop 剩余字段，unwind 内再 panic 即 `InCleanup`），再建正常半梯，第 i 步的 unwind 指向 unwind 半梯第 i+1 步。底部 [`drop_ladder_bottom`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L592) 清本路径的 shallow flag。没有子路径的“剩余字段”用父路径的 flag 条件 drop。flag 清除块在 unwind 路径上省略（[`drop_flag_reset_block`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L1329)）。

## drop glue / drop_in_place shim

- `Drop` 终结符在 codegen 中调用 `drop_in_place::<T>`：[`codegen_drop_terminator`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir/block.rs#L589)；实例解析时 `needs_drop` 为假得到 `DropGlue(_, None)`，直接跳过（[instance.rs L37](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/instance.rs#L37)）；`dyn` 从 vtable 取。
- `DropGlue(_, Some(T))` 的 MIR 由 [`build_drop_shim`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/shim.rs#L334) 生成：同一个 `elaborate_drop` 引擎，配 [`DropShimElaborator`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/shim.rs#L430)：顶层 Deep → `Open`，字段 Shallow → `Static`，无 flag、无子路径。即 glue = “调用 `Drop::drop`（若有）→ 按声明顺序对每个 needs_drop 字段递归 `drop_in_place`”，每步带 unwind 半梯。

## 析构顺序（源码可推出的规则）

- 用户 `Drop::drop` 先于字段；字段按**声明顺序**；enum 只 drop 活跃 variant；数组/slice 从下标 0 开始；Box 先内容后释放。
- 某个字段的 drop panic 后，其余字段在 unwind 半梯中继续 drop；cleanup 中再次 panic → `UnwindTerminate(InCleanup)`（中止）。
- 局部变量与临时值的顺序由作用域决定，见 [scopes-and-cleanup.md](scopes-and-cleanup.md)。

## tricky 边界

- 带 `Drop` impl 的类型不会被 open drop（字段不可 move），union/`ManuallyDrop` 不 drop 内容；“open drop 部分 move 的 union”是 `span_bug`。
- 部分 move 的数组元素在 slice 循环路径上不精确处理（`FIXME(#34708)`，[L1263](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drop.rs#L1263)）。
- 赋值 `x = v`（`x` needs_drop）建为 `Drop{replace: true}` + 正常块与 cleanup 块**都**写入新值（[`build_drop_and_replace`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1718)）：旧值 drop panic 时新值仍写入并在 unwind 中被 drop。
- 通过 `&mut` 写入后 maybe-init 被抬高（#90752），否则会漏 drop。

## 对 kfrc 的含义

- 把“展开引擎”做成独立模块，两种 elaborator：Body 内（有 flag、按 move path）和 drop glue 生成（无 flag）。T22 与 T25 共享它。
- drop flag 是普通 `bool` 局部，写在 Body 上；SoN 构造时它们只是普通 SSA 值，可以被优化掉。不需要特殊节点。
- 必须保留“块是否为 cleanup”的标记和 `Unwind::To/InCleanup` 区分；enum ladder 的 unwind 半梯复制只为 MSVC，Linux 目标可省（推断，影响代码体积不影响语义）。
- 工作量（推断）：elaborate_drop 引擎 ~1500 行、pass ~500 行，依赖 `needs_drop`、ADT 字段/variant/判别值、`Drop` impl 查找和 lang item（`drop_in_place`、`Drop::drop`）。

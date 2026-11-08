# Move path：哪些 place 被跟踪、哪些不能 move out

源码：`compiler/rustc_mir_dataflow/src/move_paths/{mod.rs,builder.rs}`；非法 move 的判定在 `compiler/rustc_borrowck/src/lib.rs`。

## 数据结构

- [`MovePath`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/mod.rs#L58)：`parent / first_child / next_sibling / place` 组成一棵**以 local 为根的前缀树**。`x`、`x.f`、`(x.f as Some).0`、`*b`（仅 Box）各是一个节点。
- [`MoveData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/mod.rs#L168)：
  - `move_paths: IndexVec<MovePathIndex, MovePath>`；
  - `moves: IndexVec<MoveOutIndex, MoveOut{path, source: Location}>`，`loc_map[loc]`、`path_map[mpi]` 两个反查表；
  - `inits: IndexVec<InitIndex, Init{path, location, kind}>`，`init_loc_map`、`init_path_map`；
  - `rev_lookup: MovePathLookup`：`locals[local] -> Option<mpi>` + `projections: FxHashMap<(mpi, MoveSubPath), mpi>`。
- [`InitKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/mod.rs#L266)：`Deep`（初始化整棵子树）、`Shallow`（只初始化自身，`ShallowInitBox` 用）、`NonPanicPathOnly`（Call 的 destination，只在正常返回边生效）。`InitLocation` 区分参数和语句。
- [`MoveSubPath`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/mod.rs#L391)：子路径只来自 `Deref / Field / ConstantIndex(from_end=false) / Downcast / UnwrapUnsafeBinder`。[`MoveSubPath::of`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/mod.rs#L408)：`OpaqueCast` 跳过；`Index`、`from_end` 的常量下标和子切片返回 `Stop`；正向 `Subslice` 单独处理。
- [`LookupResult`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/mod.rs#L309)：`find` 不建新节点，返回 `Exact(mpi)` 或最近祖先 `Parent(Option<mpi>)`。所有分析都靠“精确命中 / 落到祖先”这个二分。

## 构建算法

[`gather_moves(body, tcx, filter)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/builder.rs#L340)：先给每个通过 `filter` 的 local 建根（`CopyForDeref` 产生的 deref 临时不建），参数记一条 `Deep` init，然后按块、按语句线性扫描：

- `Assign(place, rv)`：为 `place` 建路径并记 init（`ShallowInitBox` 额外建 `*place` 并记 `Shallow`），再对 rvalue 里每个 `Operand::Move` 记 move。
- `StorageDead(local)`：**记为对 local 的 move**（[L409](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/builder.rs#L409)）。作用域出口因此把变量变回未初始化。
- `Call`：func/args 里的 move；有 `target` 时给 destination 记 `NonPanicPathOnly` init（[L500](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/builder.rs#L500)）。
- `Drop` 终结符不记 move；它的“变未初始化”效果在 `drop_flag_effects_for_location` 里单独处理（见 [init-dataflow.md](init-dataflow.md)）。
- `gather_init` 写 union 字段时把 init 记到整个 union 上。

路径创建 [`move_path_for`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/builder.rs#L107) 沿投影逐级下降，遇到以下情况**停在父节点**（不建子路径）：

| 投影 | 停止条件 |
|---|---|
| `Deref` | 基类型是 `&`/`*const`/`*mut`（只有 Box 的 deref 有子路径） |
| `Field` | 基类型是带 `Drop` impl 的 ADT（[L179](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/builder.rs#L179)）；union 字段 → 整个 union 作为 move 对象 |
| `ConstantIndex` | 基类型是 slice |
| `Index`、`from_end` 下标 | 一律 `Stop` |
| 子类型不满足 `filter` | 停 |

数组上的正向 `Subslice{from,to}` 被拆成 `from..to` 个 `ConstantIndex` 子路径，保证路径两两不相交——drop 显式化依赖这点（[L234](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_dataflow/src/move_paths/builder.rs#L234)）。

两个使用者传不同 `filter`：borrowck 用 `|_| true`（[borrowck/lib.rs L334](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L334)，Copy 类型也跟踪，用于“使用未初始化”），drop 显式化用 `ty.needs_drop`（[elaborate_drops.rs L59](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs#L59)）。

## 非法 move 的判定（borrowck 侧）

move data 构建本身不报错。[`check_movable_place`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1845) 在每个 `Operand::Move` 上沿投影检查，产生 [`IllegalMoveOriginKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/diagnostics/move_errors.rs#L23)：

- `BorrowedContent`：经过 `&`/裸指针的 deref（E0507）；
- `InteriorOfTypeWithDestructor`：从带 `Drop` 的 ADT 中移出字段（E0509）；
- `InteriorOfSliceOrArray{is_index}`：slice 上的常量下标/子切片，或任意 `Index`（E0508）。数组的常量下标和正向子切片合法。

`Downcast`、`OpaqueCast`、`UnwrapUnsafeBinder` 不影响判定。诊断再经 `group_move_errors` 合并（同一 match 的多个 binding 合成一条）。

## tricky 边界

- 带 `Drop` 的类型不能部分 move，所以它的子路径永远不会出现，drop 显式化可以假设“有 Drop impl ⇒ 要么整体初始化要么整体没有”。
- 写入有 `Drop` 的结构体的字段 `t.f = v` 要求 `t` 已初始化（见 [init-dataflow.md](init-dataflow.md)）。
- move 出 union 字段等于 move 整个 union；对 union 字段重新赋值算重新初始化整个 union。
- `*b`（Box）可以 move out，之后 drop 只释放 Box 外壳（open drop，见 [drop-elaboration.md](drop-elaboration.md)）。
- 不跟踪 static、`&` 后面的东西；`Parent(None)` 表示根本不跟踪。

## 对 kfrc 数据模型的含义

- Typed Body 的 `Place` 必须是“local + 结构化投影列表”，投影种类至少有 Field/Deref/Downcast/ConstantIndex(含 from_end, min_length)/Subslice/Index；字符串化或“表达式树里的路径”无法支撑前缀树。
- 每个 Operand 必须显式标 Copy/Move（rustc 在建 MIR 时按 `type_is_copy_modulo_regions` 决定，见 [eval-order.md](eval-order.md)），move path 收集只看 `Move`。
- 需要 `StorageDead`（或等价的“作用域结束”事件）进 Body：它是 move data 的一个 move 源，也是 `EverInitializedPlaces` 的清除点。
- Call 必须有“正常后继 / unwind 后继”两条不同的边，destination 只在正常边上初始化。
- 移植量（推断）：builder + lookup 约 800 行 Rust，几乎不依赖 `TyCtxt`，只需要“类型是否有 Drop impl / 是否 Box / 是否 union / 数组长度 / needs_drop”五个查询。

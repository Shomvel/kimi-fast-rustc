# Liveness：region 在哪些点必须有效

NLL 的核心：existential region 的值 = 它出现在其类型中的变量**可能还会被用到**的点集。实现位于 `compiler/rustc_borrowck/src/type_check/liveness/`，在 MIR typeck 末尾运行（`liveness::generate`，[liveness/mod.rs#L29](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/mod.rs#L29)）。产物写入 `MirTypeckRegionConstraints::liveness_constraints: LivenessValues`（每个 RegionVid 一组 `PointIndex` 区间，[values.rs#L41](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/values.rs#L41)）。

## 1. 三个来源

1. **变量 use-live**：变量 X 在点 p 可能被普通使用 ⇒ X 类型中所有 region 在 p live。
2. **变量 drop-live**：X 在 p 之后可能只被 drop ⇒ 只有 dropck 认为析构会碰到的 region 在 p live。
3. **出现在语句里的 region**：`record_regular_live_regions`（[mod.rs#L139](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/mod.rs#L139)）——rvalue/调用里出现的 region、泛型实参、类型，在该 location live（例如 `&'r x` 的 `'r` 在借用点 live）。另外调用点实例化的 late-bound region 在调用点 live（[type_check/mod.rs#L836](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L836)）。

## 2. 只算“相关”局部变量

- `regions_that_outlive_free_regions`（[mod.rs#L101](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/mod.rs#L101)）：在**反向**约束图上从所有 universal region 出发 DFS，得到“已知要比某个 universal region 活得长”的 region 集合。这些 region 反正包含全部点。
- `compute_relevant_live_locals`（[#L76](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/mod.rs#L76)）：类型里的 free region 全在上述集合里的局部是 boring，跳过 liveness，只为它们跑 dropck 以发现递归溢出（`dropck_boring_locals`，[trace.rs#L161](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L161)）。
- 这一剪枝依赖“约束位置无关”；`-Zpolonius=next` 下关闭（mod.rs 注释）。

## 3. def / use / drop 分类

`def_use::categorize`（[def_use.rs#L13](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/def_use.rs#L13)）：

- **Def**：整局部 `Store`、`Call` 目的地、asm 输出、`Yield` resume、`StorageLive`、`StorageDead`。
- **Use**：投影访问（含对 `x.f` 的写，因为写字段不杀死整个 x）、各种借用、copy/move、`PlaceMention`、`AscribeUserType`、`Retag`。
- **Drop**：`MutatingUse(Drop)`。
- debug info、`BackwardIncompatibleDropHint` 不算。

`LocalUseMap`（[local_use_map.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/local_use_map.rs)）只为相关局部建 def/use/drop 链表。

## 4. 回溯算法（`trace.rs`）

`trace`（[trace.rs#L40](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L40)）→ `compute_for_all_locals`（[#L131](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L131)），每个相关局部：

1. **use-live**（`compute_use_live_points_for`，[#L241](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L241)）：从每个 use 点反向走。块内回退到最近的 def（不含）或已标记点；碰到块首则把所有前驱块的 terminator 压栈。结果是 `IntervalSet<PointIndex>`。
2. **drop-live**（`compute_drop_live_points_for`，[#L295](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L295)）：只从**该局部可能已初始化**的 drop 点出发（用 `MaybeInitializedPlaces` 判断，`initialized_at_terminator` [#L518](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L518)），反向走到 def 或 use-live 点为止。
3. `add_use_live_facts_for`：类型里**所有** region 加上 use-live 点（`make_all_regions_live`，[#L602](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L602)）。
4. `add_drop_live_facts_for`（[#L547](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L547)）：对类型跑 `DropckOutlives`（`compute_drop_data`，[#L634](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/liveness/trace.rs#L634)，按类型缓存），把返回的 `kinds` 里的 region 加上 drop-live 点；dropck 产生的 region 约束在每个 drop 点登记。

`DropckOutlives` 由 trait 层实现（[rustc_trait_selection/src/traits/query/dropck_outlives.rs#L92](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/query/dropck_outlives.rs#L92)）：ADT 的字段总是递归；若 ADT 有 `Drop` impl，再把未标 `#[may_dangle]` 的泛型参数加入“会被访问”（`adt_dtorck_constraint`，[rustc_traits/src/dropck_outlives.rs#L33](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_traits/src/dropck_outlives.rs#L33)）；`&T` 本身的 drop 不碰任何东西。

## 5. tricky 边界

- 写 `x.f = …` 是 Use 不是 Def，所以不会截断 x 的 liveness；只有整局部赋值才截断。
- `drop-may-dangle.rs` / `drop-no-may-dangle.rs`：同样形状，`Drop` impl 有无 `#[may_dangle]` 决定 `else` 分支里写 `v[0]` 是否报错（见 [test-cases.md](test-cases.md)）。
- 被 move 走的局部在 drop 点不算已初始化，因而不产生 drop-live。这把 liveness 与初始化数据流（[../ownership/](../ownership/README.md)）耦合在一起。
- `Call` 的目的地在 unwind 边也算 def（def_use.rs 注释承认不精确，但不可观察）。

## 6. 对 kfrc 的含义

- 首轮（[T13](../../../plan/m1-vertical-chain.md#t13)）：use-live 回溯 + “出现即 live” 两条规则即可覆盖字段读、重借用、分支、提前返回。算法是纯 CFG 回溯，输入只有 Body 的 def/use 表和类型中的 region 槽。
- drop-live 需要 dropck。首轮可用近似：字段递归；有 `Drop` impl 的类型其所有泛型参数都算会被访问（等于不支持 `#[may_dangle]`，只会多拒绝）。对不含 `may_dangle` 的自写程序与 rustc 一致；std 的 `Vec<&T>` 等依赖 `may_dangle`，用 std 之前（[T22](../../../plan/m3-semantics.md#t22)）要移植真 dropck。
- 剪枝（boring locals）是性能优化，正确性上可后加；但它依赖 universal region 先建好、约束先收集完，次序要和 rustc 一致：先全量 MIR typeck，再 liveness。
- liveness 需要 `MaybeInitializedPlaces`，所以 ownership 的初始化数据流必须在 region 推导之前可用。

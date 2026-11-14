# 单态化收集与分区

Instance 种类与解析见 [instances.md](instances.md)。源码：[collector.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs)、[partitioning.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/partitioning.rs)；概览见 dev-guide [monomorph.md](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/src/doc/rustc-dev-guide/src/backend/monomorph.md)。

## 总体

`collect_crate_mono_items(strategy)`（[#L1792](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1792)）：

1. `collect_roots` 遍历本 crate HIR 得到根（[#L1462](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1462)）。
2. 对每个根并行 `collect_items_root` → `collect_items_rec` 深度优先（[#L387](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L387)）；共享 `SharedState{visited, mentioned, usage_map}`（[#L256](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L256)），`visited` 去重保证每个 `MonoItem` 只展开一次。
3. 结果用稳定哈希排序（`into_sorted`），**集合与遍历顺序、并行度无关**。`usage_map` 记录 user→used 边供分区内联用。

## 根（`RootCollector`，[#L1501](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1501)）

- **Lazy**（默认非增量）：非泛型且满足其一的 fn——入口 `main`、`is_reachable_non_generic`（导出的非泛型项，含 `#[no_mangle]`/`pub extern` 等）、`RUSTC_STD_INTERNAL_SYMBOL`、`EXTERNALLY_IMPLEMENTABLE_ITEM`（`is_root`，[#L1638](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1638)）。
- 所有 `static`、`global_asm!`。
- `main` 存在时额外加 `lang_start::<main 的返回类型>`（[#L1675](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1675)）。
- pub async fn 的协程体。
- **Eager**（增量/`-Clink-dead-code`）：额外加所有非泛型 fn、非泛型 ADT 的 drop glue、非泛型 const 的分配、非泛型 impl 中未覆盖的默认方法（[#L1717](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1717)）。
- 过滤 `is_instantiable`（谓词不可能成立的项不生成）。

## 展开一个项（[#L387](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L387)）

- **`Static(def)`**：`visit_drop_use(静态类型)`（非 nested static）；`eval_static_initializer` 后对分配里每个 provenance 调 `collect_alloc`；需要时加 `ThreadLocalShim`。
- **`Fn(instance)`**：先 `check_recursion_limit`（按 DefId 计深度，`drop_in_place` 深度除以 4，[#L654](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L654)），再 `items_of_instance` → `collect_items_of_instance`（[#L1307](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1307)）：
  1. `instance_mir(instance.def)`（优化后的 MIR 或 shim MIR）。
  2. **used**：只遍历 `mono_reachable` 的基本块——单态化后能常量折叠的 `SwitchInt` 只走被选中的分支（[traversal.rs#L299](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/traversal.rs#L299)）。完整 debuginfo 时也访问 `var_debug_info`。
  3. **mentioned**：`body.required_consts()` 全部求值；`body.mentioned_items()` 中没在 used 里出现过的，按 `visit_mentioned_item` 处理（[#L1389](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1389)）。
- **`GlobalAsm`**：`sym fn`/`sym static` 操作数。

## used 边：`MirUsedCollector`（[#L687](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L687)）

所有类型先 `monomorphize`（用实例 args 实例化并规范化）。

| MIR 形式 | 产生的项 |
|---|---|
| `Call`/`TailCall` 的 callee `FnDef` | `expect_resolve` → `visit_instance_use`（直接调用）（[#L821](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L821)） |
| `ReifyFnPointer` cast | `resolve_for_fn_ptr` → 可能是 `ReifyShim`（[#L733](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L733)） |
| `ClosureFnPointer` cast | `resolve_closure(.., FnOnce)` |
| `Unsize` cast 到 `dyn Trait` | `find_tails_for_unsizing` 找到源/目标尾部 → `create_mono_items_for_vtable_methods`：vtable 的每个 `Method` 实例 + `needs_drop` 时 drop glue（非直接调用）（[#L1213](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1213)） |
| `Drop` terminator | `visit_drop_use` → `DropGlue` |
| 常量操作数 | 求值成 `ConstValue`，`collect_const_value` → `collect_alloc`（[#L815](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L815)、[#L1440](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1440)） |
| `ThreadLocalRef` | `Static(def)` |
| `Assert` | 对应 panic lang item（越界 `PanicBoundsCheck`，其余 `msg.panic_function()`） |
| `UnwindTerminate` / `UnwindAction::Terminate` | 对应 lang item |
| `InlineAsm` 的 `sym` | fn 或 static |

`visit_instance_use`（[#L981](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L981)）：只收 `should_codegen_locally` 的实例；`Virtual`/`Intrinsic` 不产生项（intrinsic 可能带出 fallback body 或 `panic_nounwind`）；`DropGlue(_, None)` 只有非直接调用（取地址）时才收；其他 kind 都收。

## 分配引用 `collect_alloc`（[#L1262](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1262)）

`GlobalAlloc::Static` → `Static`；`Memory` → 递归其 provenance；`Function{instance}` → `Fn`；`VTable(ty, dyn)` → 生成 vtable 分配再递归（从而收集方法和 drop glue）；`TypeId` → 无。**常量和 vtable 本身不是 MonoItem**：它们作为数据在每个引用它们的 CGU 里按需发射（文件头注释，[#L23](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L23)）。

## `should_codegen_locally`（[#L1054](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1054)）

foreign item 否；本地项是；上游 `is_reachable_non_generic` 或已有上游单态（`upstream_monomorphization`，受 `-Zshare-generics` 影响）否；上游 static 否；其余需要上游 MIR 可用（否则致命错误 `NoOptimizedMir`），然后在本 crate 生成。

## mentioned items 的意义（文件头，[#L156](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L156)）

“used”集合依赖优化（死代码删除），会导致“debug 构建因某个常量求值失败报错、release 不报”。所以 MIR 在依赖优化级别的 pass 之前记录 mentioned items（`rustc_mir_transform::mentioned_items`），收集器对每个 used 项递归检查其 mentioned 项的常量都能求值，但不为 mentioned-only 项生成代码。用例：[tests/ui/consts/required-consts/](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/consts/required-consts/collect-in-dead-fn.rs)。

## 分区概要（[partitioning.rs#L142](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/partitioning.rs#L142)）

`place_mono_items`（按定义所在模块分“稳定/泛型”两类 CGU）→ `merge_codegen_units`（合并到 CGU 上限）→ `internalize_symbols`（只被本 CGU 用的改内部链接）→ 按名排序。每项的 `instantiation_mode`（[mono.rs#L130](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/mono.rs#L130)）：static、入口、`#[no_mangle]`/naked → `GloballyShared{may_conflict:false}`；`-O0` 下 drop glue 全局共享；`#[inline(always)]` → `LocalCopy`；不可跨 crate 内联的 → 全局共享；其余 `-O0` 全局共享（`may_conflict`）、优化时 `LocalCopy`。

## tricky 边界

- 收集发生在单态后：泛型代码里的常量错误只在实例化时报（post-mono error），且要在 used/mentioned 两种模式下一致。
- 递归限制按 DefId 计数，多态递归（`f::<T>` 调 `f::<Vec<T>>`）会撞限制；`drop_in_place` 宽松 4 倍。
- 静态初始化里的函数指针、vtable、嵌套分配都是可达性的来源，不能只看函数体。
- drop glue 对 `!needs_drop` 类型是 `DropGlue(_, None)`，不进实例图，除非被取地址。

## 对 kfrc 的含义

- kfrc 的“实例图”= `visited` 集合 + `usage_map`；节点身份用 `InstanceKey`，边来自 ConcreteBody 而非 SoN（rustc 在 MIR 上收集，codegen 前完成）。
- 首轮可以不做 `mono_reachable` 剪枝（遍历全部块）：结果是 rustc 的超集，报错集合不变（mentioned 项的常量 rustc 也会求值），只多生成代码（推断）。
- 并行收集只要最终排序、去重即可确定；递归深度计数在并行下是“每条路径”的，按 rustc 的 DFS 语义实现（`recursion_depths` 在每个根的 DFS 内维护）。

# rustc 前端为什么慢（一）：查询、驻留、metadata

源码 = rustc 1.94.1（`e408947b`）核实的行；“推断”= 由源码读出的机制，未实测。链接前缀 `L/` = `https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/`。实测数字来自 [time-breakdown.md](time-breakdown.md) 的同一批 self-profile。

## 1. 查询系统：非增量时便宜，增量时贵

| 事实 | 位置 | 含义 | 性质 |
|---|---|---|---|
| 每次 `tcx.q(k)` 先查缓存；**命中也记 profiler 事件并调用 `read_index`** | [query/inner.rs#L20-L35](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/query/inner.rs#L20-L35)、[plumbing.rs#L249-L262](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/query/plumbing.rs#L249-L262) | 热路径 = 哈希 + 探测 + 两个分支 | 源码 |
| 未命中：哈希 key、锁 active 分片、插 `QueryJob`、压 TLS `ImplicitCtxt`；完成时再锁两次 | [plumbing.rs#L335-L403](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/query/plumbing.rs#L335-L403)、[#L179-L208](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/query/plumbing.rs#L179-L208)、[query_impl plumbing.rs#L133-L158](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_impl/src/plumbing.rs#L133-L158) | 每次执行有固定开销；细粒度查询（`impl_trait_header` 一次 cargo release 构建执行 3,262,147 次）把它放大 | 源码 + 实测调用数 |
| **非增量不建依赖图**：`DepGraph::new_disabled`，节点号只是原子计数，结果不哈希 | [persist/load.rs#L211-L238](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_incremental/src/persist/load.rs#L211-L238)、[plumbing.rs#L478-L512](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/query/plumbing.rs#L478-L512) | release/check（`CARGO_INCREMENTAL=0`）不付指纹成本 | 源码 |
| 增量：`with_task` 收集边、`hash_result` 用 SipHasher128 算 128 位指纹、`try_mark_previous_green` 沿旧边递归验证、从盘加载的结果每 32 个重哈希 1 个 | [graph.rs#L317-L353](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/dep_graph/graph.rs#L317-L353)、[#L103-L110](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/dep_graph/graph.rs#L103-L110)、[#L967-L1000](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/dep_graph/graph.rs#L967-L1000)、[plumbing.rs#L596-L614](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/query/plumbing.rs#L596-L614) | 增量的固定成本与依赖图大小成正比，不与改动成正比 | 源码 |
| 增量单行编辑时 rg 的 typeck + borrowck “大部分是 green 验证” | 0.07 + 0.08 s；会话目录准备 + 结果持久化 0.14 + 0.11 s | 前期 rustc-10x-rewrite §2（本机 `-Ztime-passes`） | 实测 |
| 只有 62/318 个查询 `cache_on_disk_if`；`evaluate_obligation`、`mir_borrowck` 不落盘 | [query/mod.rs#L2515-L2519](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/query/mod.rs#L2515-L2519)、[#L1275-L1280](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/query/mod.rs#L1275-L1280) | 增量时这些结果只能靠上游 green 截断，不能直接复用 | 源码 |

增量场景的实测占比（`fe_incr_overhead` 类）见 [time-breakdown.md §3](time-breakdown.md#3-增量编辑)。

## 2. 驻留与锁

| 事实 | 位置 | 含义 | 性质 |
|---|---|---|---|
| 26 个驻留集，每个是 `ShardedHashMap`；`intern_ty` 每次都哈希 + 探测，未命中才算 flags 并 arena 分配 | [ty/context.rs#L924-L957](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L924-L957)、[#L996-L1015](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L996-L1015) | 每个 `mk_*` 都付一次哈希；推导期的临时类型也进全局表 | 源码 |
| 稳定哈希只在增量时驻留缓存，否则每次 `HashStable` 重算 | [#L1044-L1061](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L1044-L1061)、[ty_info.rs#L101-L121](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/ty_info.rs#L101-L121) | 非增量下 SVH、metadata 中的哈希反复计算类型 | 源码 |
| `Lock` 在构造时按 `threads > 1` 选真互斥或 `Cell` 标志；分片单线程时只有 1 片，并行时 32 片 | [sync/lock.rs#L80-L104](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_data_structures/src/sync/lock.rs#L80-L104)、[sharded.rs#L14-L42](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_data_structures/src/sharded.rs#L14-L42) | 单线程模式几乎无锁成本；并行模式下全局 trait 缓存仍是**单把锁**（[cache.rs#L10-L58](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/cache.rs#L10-L58)） | 源码 |
| 官方：超过 4 线程出现锁争用 | rustc-dev-guide “Parallel compilation”（自标过时） | 见 [parallel-frontend.md](parallel-frontend.md) | 官方资料 |

## 3. metadata：编码在关键路径上，解码按查询懒惰但粒度很细

| 事实 | 位置 / 数字 | 性质 |
|---|---|---|
| metadata 在 codegen 之前编码；`encode_crate_root` 串行；`prefetch_mir` 只在 `threads != 1` 时并行 | [passes.rs#L1253](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L1253)、[encoder.rs#L604-L725](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/rmeta/encoder.rs#L604-L725)、[#L2462-L2473](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/rmeta/encoder.rs#L2462-L2473) | 源码 |
| 编码本身便宜，贵的是它拉起的 MIR 选择、optimized MIR、导出符号 | rg 关键链每 crate 编码 ≤0.06 s；元数据阶段 90–98% 可后移 | 前期 P2a §2（实测） |
| **SVH 要哈希全部 HIR**：凡输出 metadata 的构建（含 `cargo check`）都对每个 HIR owner 做稳定哈希 | [ast_lowering/lib.rs#L707-L709](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/lib.rs#L707-L709)、[ty/context.rs#L1995-L2010](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L1995-L2010)、[hir/map.rs#L1125-L1198](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/hir/map.rs#L1125-L1198) | 源码；“每个 lib 构建”为推断 |
| SVH 覆盖函数体与 span → 改注释也连锁重编下游 | release 下改注释重做 28.7%，与改函数体相同 | 前期 rustc-10x-rewrite §2（实测） |
| 解码：rmeta mmap；`def_path_hash_map` 零拷贝；每个外部查询按需解码自己的表项 | [locator.rs#L934-L951](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/locator.rs#L934-L951)、[cstore_impl.rs#L94-L175](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/rmeta/decoder/cstore_impl.rs#L94-L175) | 源码 |
| 解码按“查询 × DefId”进行，结果只在本会话缓存；每个下游 crate 重新解码 std 的同一批条目 | cargo release：`metadata_decode_entry_impl_trait_header` 3,207,143 次、1.86 s；`implementations_of_trait` 947,974 次 | `top-labels.sh`（实测） |
| `md_decode` 占前端 | release：rg 8.3%、Cargo 7.8%；check 见 [time-breakdown.md](time-breakdown.md) | `phase-breakdown.sh`（实测） |

## 4. 小结：哪些是“架构税”

| 成本 | 冷构建 | 增量 | kfrc 能否避开 |
|---|---|---|---|
| 查询调度（哈希、job、TLS） | 有，分散在每个查询里，self-profile 不单列 | 有 | 能：语义快照 + 任务内直接调用（[design/frontend/overview.md](../../design/frontend/overview.md)） |
| 依赖图 + 指纹 | 无 | 主要固定成本 | 部分：按产物记依赖清单（[design/build/cache.md](../../design/build/cache.md)），粒度粗于查询 |
| SVH 全 HIR 哈希 | 有（metadata 构建） | 有 | 能：接口哈希只覆盖接口（[recommendations.md](recommendations.md)） |
| 每 crate 重复解码 std | 有 | 有 | 能：sysroot 接口按 durability 预索引、常驻或共享映射 |
| MIR 优化在 rmeta 前 | 有 | 有 | 已避开：R 级在接口之后（C4） |

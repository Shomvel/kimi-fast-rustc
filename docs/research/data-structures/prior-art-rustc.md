# 先例（三）：rustc 的驻留、arena、索引与并行

【来源】标注为“源码”的项读自 rustc 1.94.1 官方源码（路径相对 `compiler/`）；PR 数字来自各 PR 页面上的 rust-timer 结果。

## 驻留与 arena（源码）

| 结构 | 1.94.1 中的形态 | 位置 |
|---|---|---|
| `Ty<'tcx>` | `Interned<'tcx, WithCachedTypeInfo<TyKind>>`，即 arena 中的一个引用；相等比较是指针比较 | `rustc_middle/src/ty/mod.rs`、`rustc_data_structures/src/intern.rs` |
| `TyKind` | 24 B（`static_assert_size!`） | `rustc_middle/src/ty/sty.rs` |
| `WithCachedTypeInfo<TyKind>` | 48 B = `TyKind` + `stable_hash: Fingerprint`（16 B）+ `flags` + `outer_exclusive_binder` | 同上、`rustc_type_ir/src/ty_info.rs` |
| 驻留表 | `CtxtInterners.type_: ShardedHashMap<InternedInSet<…>, ()>`，FxHash；未命中时分配到 `WorkerLocal<Arena>`（每线程一个 arena） | `rustc_middle/src/ty/context.rs` |
| 何时算 stable hash | 只在增量模式且不含推导变量时算，否则写 `Fingerprint::ZERO`（注释：“非增量时很少对类型做 stable hash”） | 同上 `CtxtInterners::stable_hash` |
| `TypedArena` | 首块 4 KiB，之后每块翻倍，上限 2 MiB（`PAGE`、`HUGE_PAGE`） | `rustc_arena/src/lib.rs` |
| `IndexVec<I, T>` | 就是 `Vec<T>` 加类型化下标 | `rustc_index/src/vec.rs` |
| `Sharded<T>` | `SHARD_BITS = 5` → 32 片，每片 `CacheAligned<Lock<T>>`；单线程时是 `Single(Lock<T>)`；分片号取哈希高位（避开 hashbrown 用的最高 7 位和低位） | `rustc_data_structures/src/sharded.rs` |
| `Lock` | `Mode::NoSync`（`Cell<bool>`）或 `Mode::Sync`（真互斥），创建时按是否可能多线程选择 | `rustc_data_structures/src/sync/lock.rs` |
| `SmallVec` | 广泛使用，如 MIR 前驱表 `IndexVec<BasicBlock, SmallVec<[BasicBlock; 4]>>` | `rustc_middle/src/mir/basic_blocks.rs` |
| metadata 编码 | `opaque` 编码器用 LEB128；类型用“shorthand”（已写过的类型写回指偏移，`SHORTHAND_OFFSET = 0x80`） | `rustc_serialize/src/opaque.rs`、`rustc_middle/src/ty/codec.rs` |

### rustc 各层节点大小（源码，`static_assert_size!`）

| 层 | 类型 | 字节 |
|---|---|---|
| AST | `ast::Expr` / `ast::Ty` | 72 / 64 |
| HIR | `hir::Expr` / `hir::Ty` | 64 / 48 |
| THIR | `thir::Expr` | 64 |
| MIR | `Statement` / `Terminator` / `BasicBlockData` | 56 / 96 / 152 |
| MIR | `Rvalue` / `Place` / `StatementKind` | 40 / 16 / 16 |

`StatementKind` 只有 16 B，是因为 `Assign` 把 `(Place, Rvalue)` 装箱，每条赋值一次堆分配。

## 并行前端与驻留锁（PR 与博客）

| 事实 | 数字 | 来源 | 年份 |
|---|---|---|---|
| 并行前端博客 | `-Z threads=8` 编译时间最多 −50%；单线程模式比串行前端慢 0–2%；**内存最多 +35%**；ripgrep 前端 10.2 → 5.9 s。博客未提分片 | [blog 2023-11-09](https://blog.rust-lang.org/2023/11/09/parallel-rustc/) | 2023 |
| MCP #681 | 8 线程 clean build 平均约 −30%（serde −47.1%，ripgrep −11.3%）；单线程开销平均 1.8%；48 核上 32 线程不再提升 | [compiler-team#681](https://github.com/rust-lang/compiler-team/issues/681) | 2023 |
| PR #114860：`Sharded` 单线程变体 | 1 线程 wall −1.33%（hyper −2.90%），周期 −3.3% | [#114860](https://github.com/rust-lang/rust/pull/114860) | 2023 |
| PR #111713：条件同步的 `Lock` | 1 线程时间 −1.79% | [#111713](https://github.com/rust-lang/rust/pull/111713) | 2023 |
| `sharded.rs` 注释 | 32 片只在一台 8 核 Ryzen 7 1700 上测过 | 源码 | — |
| PR #115747：查询哈希表传入预计算哈希 | 本地 check −0.34%，icount −0.2%，max-rss +4.1%（3 个基准） | [#115747](https://github.com/rust-lang/rust/pull/115747) | 2025 |
| PR #137354：驻留表预分配 | 启动多约 1 MB，icount −0.3%；更大的预分配 max-rss +15.9%、周期 +3.2%，被拒 | [#137354](https://github.com/rust-lang/rust/pull/137354) | 2025 |
| rustc-hash 2.0（新算法） | 进入 rustc：icount −0.1%，周期 −2.1%（次要基准），max-rss −1.1% | [#131949](https://github.com/rust-lang/rust/pull/131949) | 2024 |
| PR #94299：类型里缓存 stable hash | icount 平均 −1.0%，最多 −9.6%（增量）；评审指出 `TyS` 从 40 B 涨到 56 B | [#94299](https://github.com/rust-lang/rust/pull/94299) | 2022 |
| PR #155329：删除 `WithCachedTypeInfo::stable_hash` | 类型、谓词、region 各小 16 B；icount +0.5%（增量回退），max-rss −1.7%；结论“略正面”，“icount 在这里有误导性” | [#155329](https://github.com/rust-lang/rust/pull/155329) | 2026-04（晚于 1.94.1） |

## 对 kfrc 的启示（【推断】）

| rustc 做法 | 评价 | kfrc 取舍 |
|---|---|---|
| 类型 = arena 指针，指针相等 | 比较快，但 8 B 引用、旁表只能用哈希 | `TyId(u32)` 下标进只追加的表：4 B，旁表可用 `IndexVec`，持久化时换成持久键（[ids.md](../../design/ids.md) 已如此规定） |
| 驻留时立刻算 16 B fingerprint | 两次 PR 来回：省时间但每类型多 16 B，2026 年删除 | 会话内只存 64 位 Fx 哈希；128 位指纹只在写缓存/metadata 时按需算，见 [interning-and-keys.md](interning-and-keys.md) |
| 32 片 `Sharded` + 单线程 `NoSync` | 单线程开销被压到 1–2%；分片数没有系统测过 | T6 实测：只分片不解决缓存行搬运，要加每任务本地缓存，见 [parallel-memory.md](parallel-memory.md) |
| `WorkerLocal<Arena>` | 分配无锁 | 每任务/每线程 arena，任务结束整体释放 |
| MIR `Assign` 装箱、每块 `Vec<Statement>` | 每条语句一次分配 | Body 用扁平数组，见 [body-and-artifacts.md](body-and-artifacts.md) |
| metadata：LEB128 + 类型 shorthand | 紧凑，但要整体解码 | 冷的图用 varint 流，热的表用定宽零拷贝 |
| 并行前端内存 +35% | 已写进 [compile-time.md](../../design/goals/compile-time.md#预算) | 预算按字节计，见 [parallel-memory.md](parallel-memory.md) |

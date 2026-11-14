# 并行：arena、全局驻留、峰值内存

对应 [pipeline.md](../../design/build/pipeline.md#cpu-与内存预算)、[compile-time.md](../../design/goals/compile-time.md#预算)。

## 先例（【来源】）

| 系统 | 做法 | 代价 / 收益 |
|---|---|---|
| rustc | 驻留表 32 片 `Sharded`，每片缓存行对齐；单线程时 `Single` + `NoSync` 锁；arena 是 `WorkerLocal` | 单线程开销压到 1–2%（#114860、#111713）；8 线程最多 −50%，内存最多 +35%（2023 博客）；32 片只在 8 核机器上测过 |
| Zig | InternPool：每线程 `Local` + 2 的幂个 `Shard`，线程号编进索引高位 | 开多线程 wall −33.3%，峰值 RSS +28.1%（PR #20528，2024） |
| Carbon | `ValueStore` 只追加分块 | 单线程设计，无并行数据 |
| C2 | 每次编译一个 node arena，“delete 是空操作”，两个 arena 轮换 | 编译结束整体释放 |

## 实测（【实测】，T6，见 [bench-io-parallel.md](bench-io-parallel.md#t6-并行驻留)）

| 方案 | 8 线程相对单线程 | 锁争用率 |
|---|---|---|
| 单锁 | 0.65× | ≤0.3% |
| 32 分片（rustc 式） | 1.05× | 0.4–2.3% |
| 32 分片 + 每任务本地缓存 | 3.37× | ≈0.01% |

争用率很低而吞吐不涨，说明代价在于**每次加锁都把锁的缓存行搬到本核**，与是否真的等锁无关。所以“驻留锁等待时间”这个指标不够，还要测每次驻留的缓存行迁移（见 [metrics.md](metrics.md)）。

## 建议（【推断】）

### 每任务 arena

| 数据 | 归属 | 释放 |
|---|---|---|
| `FunctionGraph`（节点、列、溢出池、常量池） | 任务独占的 `Vec` 们 | 任务结束或发布后整体 drop；不需要通用 arena |
| pass 内临时数据（worklist、转发表、users 索引、位集） | 每个 worker 一份可复用的 scratch，按图大小 `clear()` + `reserve` | 不随任务释放，避免反复分配 |
| Body、推导表、借用检查数据 | typeck root 任务的 bump arena（`bumpalo` 式或 `TypedArena` 式，首块 4 KiB、翻倍） | 发布 CheckedBody 后丢弃 |
| 已发布产物（R 图、接口） | 不可变；序列化字节或 `Arc<[u8]>` | 按依赖者计数或构建结束 |

### 全局驻留

```text
intern(k):
  local = task_cache.get(k)          // 每任务（或每 worker）FxHashMap，读不碰共享内存
  if hit: return
  h = fx(k); shard = shards[h 高位]
  lock(shard) → 查表 / 分配 id = (shard << 26) | shard.len    // 片内编号，无全局计数器
  task_cache.insert(k, id)
读 TyData：values.get(id)            // 只追加分块存储，无锁读
```

| 设计点 | 理由 |
|---|---|
| 本地缓存在任务结束时丢弃 | 有界；T6 中本地缓存是主要收益 |
| 片内编号（片号进高位，同 Zig 的线程号） | 免掉全局 `AtomicU32` 计数器这一个共享缓存行 |
| 存储只追加、块不移动 | 读路径无锁；扩容只影响新块 |
| 单线程构建不建分片 | 同 rustc `Single`：jobs=1 时用一个无同步的表 |
| `TyId` 数值不确定 | 只在会话内用，输出和键按持久键排序（[ids.md](../../design/ids.md#规则)） |

不采纳：无锁共享哈希表（开放寻址 + CAS）。T6 说明本地缓存后共享表的访问只占约 5%，再把共享表做成无锁收益有限，正确性和确定性审查成本高。M5 后若 profile 显示共享表仍是瓶颈再评估。

### 峰值内存预算

| 项 | 估算方法 | cargo 量级的数字（【推断】） |
|---|---|---|
| 节点 | 存活节点数 × 字节/节点 | 431 万节点 × 26 B ≈ 0.11 GB（现状布局 ≈ 0.41 GB 以上） |
| GVN 表 | 纯节点数 × 19 B，只在图被处理时存在 | 并发图数 × 单图大小，可忽略 |
| 同时存活的 S 图 | `jobs × 最大单图字节`，受字节预算 | 预算按 `bytes/节点 × 节点数` 预估，完成后用实测值校正 |
| 每任务本地驻留缓存 | 每条约 20 B；任务结束释放 | 每任务 KiB 级 |
| 驻留表 | 类型数 × (20 B 数据 + 8 B 表项) | 待测；rustc 每类型 48 B 数据 + 表项 |

规则：

1. 调度器的字节预算用“估算字节 = 节点数 × 当前布局字节/节点”准入（[pipeline.md](../../design/build/pipeline.md#cpu-与内存预算) 已要求按估算/实测字节限制队列）。
2. 并行带来的峰值 RSS 增量单独报告，对照 rustc +35%、Zig +28.1%。
3. 超大单图（内联后 >10⁵ 节点）走专用准入，不与普通图共享预算。

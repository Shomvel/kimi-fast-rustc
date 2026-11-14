# 微基准：方法与限制

源码在仓库外的草稿目录 `kfrc-walkthroughs-20261108/data-structures/nodebench/`（单个 `main.rs`，依赖 smallvec、rustc-hash、hashbrown、blake3、sha2、siphasher、parking_lot）。结果表由程序直接输出，原样贴在 [bench-layout.md](bench-layout.md) 和 [bench-io-parallel.md](bench-io-parallel.md)，没有手改数字。

## 环境

| 项 | 值 |
|---|---|
| 机器 | Apple M3 Pro，6 个性能核 + 6 个能效核，36 GiB |
| 缓存 | 性能核 L1d 128 KiB/核，L2 16 MiB（簇内共享） |
| 工具链 | rustc 1.94.1，`opt-level=3`、`codegen-units=1`、thin LTO |
| 计时 | 每项重复 7（N≈1M）/ 31（N≈64k）/ 201（N≈4k）次，取**最小值**（macOS 不能绑核，最小值最少受能效核和调度干扰） |
| 内存 | 计数分配器统计“构建后存活堆字节”；不含 malloc 的头部和取整，所以小分配多的布局被**低估** |

**只能作相对比较。** x86-64 服务器的 L1d 通常 32–48 KiB，是这台机器的 1/4–1/3；同样的布局差异在 x86 上会更早显现。L1d miss 没有测：macOS 上没有 `perf`，两台 Linux 主机也没有可用的 `perf`（`perf_event_paranoid=4`）。要测 L1d 需在 E2B 沙箱用 `perf stat`，见 [metrics.md](metrics.md)。

## 合成图

`gen_spec(N)` 生成一个类 SoN 的图，按创建顺序编号（输入总在前面）：

| 节点 | 比例 | 输入 |
|---|---|---|
| Const / Param | 12% / 3% | 无（立即数进常量池） |
| Binary / Unary / Compare | 35% / 8% / 7% | 2 / 1 / 2 个数据；Binary 中 10% 复制已有表达式（制造 GVN 命中） |
| If / Region / Proj | 3% / 3% / 4% | 控制 + 0–1 个数据 |
| Phi | 5% | Region + 2 个数据 |
| Load / Store | 10% / 6% | 控制 + 1–2 个数据 + 效果 |
| Call | 4% | 控制 + 3–5 个数据 + 效果 |

数据输入 85% 取最近 32 个值之一，15% 均匀取任意更早的值，模拟“大多数使用离定义近”。这是假设，不是从真实 Rust 图统计来的；真实分布要等 T09 的 R 图建出后用 [metrics.md](metrics.md) 的计数器重测。

## 比较的布局

| 名称 | 结构 | 对应 |
|---|---|---|
| Vec3（现状） | `Vec<Option<Node>>`，`Node { kind: NodeKind(16B), control: Vec, data: Vec, memory: Vec }`，88 B/节点 | [son.rs](../../../src/son.rs) |
| SmallVec（契约草案） | `kind + ty: u32 + control: Option<u32> + SmallVec<[u32;3]> + SmallVec<[(u32,u32);1]>`，80 B/节点 | [graph.md](../../design/ir/graph.md#节点) |
| Packed16（AoS） | `{ op: u8, flags: u8, n: u16, s: [u32; 3] }`，16 B；>3 个输入时 `s[0]` 指向共享溢出池；立即数进常量池 | Cranelift `InstructionData`、Carbon `Inst` |
| SoA | 同上，但 `op`、`n`、`s` 各为一列 | Zig `MultiArrayList` |
| CSR（冻结） | `op` 列 + `start` 偏移 + 连续边数组；不能加边，只能原地改 | 只读的已发布图 |

## 测量项

| 表 | 内容 | 模拟的编译器操作 |
|---|---|---|
| T1 | 构建、克隆、顺序扫描、DFS 标记（创建顺序存放 / 随机打乱存放）、随机顺序访问 | builder、specialize 复制 R 图、verifier、DCE、worklist 访问顺序 |
| T2 | 替换 N/20 个节点的全部使用 | 现状 `replace_uses` vs use-list vs 转发表 |
| T3 | 构建期 GVN 的查找 + 插入 | Click 式 hash-consing |
| T4 | 序列化：varint 节点流 vs 原样记录 | metadata 中的 R 图、缓存 |
| T5 | 内容哈希 | 答案键、R 图哈希、缓存键 |
| T6 | 多线程类型驻留 | 并行前端的 `TyId` 驻留 |

“构建 ns/节点”包含从合成 spec（每节点一个 `Vec`）读取的开销，不代表真实 builder 的绝对耗时，只看相对关系。

## 重跑

```sh
cargo run --release --offline -- all     # 或 layout / uses / gvn / ser / hash / intern
```

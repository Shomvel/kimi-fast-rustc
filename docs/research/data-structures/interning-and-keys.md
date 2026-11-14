# 驻留、孔位、答案键与指纹

对应 [ids.md](../../design/ids.md)、[types.md](../../design/frontend/types.md)、[generics-specialization.md](../../design/ir/generics-specialization.md)、[cache.md](../../design/build/cache.md)。

## 两类哈希，分开用

| 用途 | 算法 | 位数 | 实测成本（T5，1M 节点图） | 理由 |
|---|---|---|---|---|
| 会话内哈希表（GVN、驻留、旁表） | Fx（rustc-hash 2） | 64 | 3.5 ns/节点 | 最快；不跨进程，碰撞只影响性能 |
| 持久键（R 图哈希、答案键、`ArtifactKey`、`DefPathHash`） | SipHash-1-3-128（rustc `StableHasher` 同族）或 BLAKE3 截断到 128 | 128 | 6.6 / 12.4 ns/节点 | 碰撞会导致错误命中，必须足够长 |
| 不用 | SHA-256 | 256 | 64（整块）/ 107（逐字段，现状写法） | 比 SipHash-128 慢 10–16 倍，安全性在这里无用 |
| 不用 | 标准库 `HashMap` 默认的 SipHash-1-3 | 64 | T3：GVN 查找慢 2.4–3.8 倍 | 防 HashDoS 在编译器内部无意义 |

为什么持久键要 128 位（【推断】）：n 个键在 b 位哈希下的碰撞概率约 n²/2^(b+1)。缓存跨构建、跨项目共享，键的总数按 10⁸ 计，64 位时约 3×10⁻⁴，128 位时约 10⁻²³。rustc 的 `Fingerprint` 也是 128 位【来源】。

## `TyId` 驻留

| 设计点 | 建议 | 依据 |
|---|---|---|
| 存储 | 只追加的分块向量 `ValueStore<TyId, TyData>`（4 KiB 块）；读不加锁，因为已发布的块不再移动 | Carbon `ValueStore` 4 KiB 块比 1 MiB 快【来源】 |
| 去重 | `HashTable<(TyId, u32 哈希)>`，键留在存储里 | Carbon `CanonicalValueStore`、T3 的存哈希变体【实测】 |
| `TyData` 大小 | `TyKind` 压到 ≤16 B（参数列表另驻留为 `ArgsId`/`TyListId`）+ 4 B 标志位 = 20 B | rustc 是 48 B（`TyKind` 24 + fingerprint 16 + 标志 8）【来源】 |
| 不存 fingerprint | 持久指纹只在写 metadata/缓存时按需算，记在 `IndexVec<TyId, Option<Fingerprint>>` 备忘表里 | rustc 2022 年加缓存 fingerprint（icount −1%），2026 年又删掉（每类型省 16 B，max-rss −1.7%）【来源】 |
| 编号与确定性 | 并行驻留时 `TyId` 的数值取决于线程交错，**不得**进入输出顺序、符号、键；需要排序时按持久键排 | [pipeline.md](../../design/build/pipeline.md#确定性) 要求 jobs=1/2/4 字节相同 |
| 并行 | 分片 + 每任务本地缓存，见 [parallel-memory.md](parallel-memory.md) | T6【实测】 |

## 孔位表

图内的 `Hole` 只用会话 ID（`DefId`、`TyId`、`ArgsId`），每条定长 12 B：`{ kind: u8, a: u32, b: u32 }`；按首次出现去重，节点用 `HoleId(u32)` 引用。契约里 `Resolve { item: DefKey, … }` 的 `DefKey` 含 `CrateKey` 指纹，放进每个孔位会让表膨胀到 40 B 以上；持久形式只在发布 R 图时写出，此时 `DefId` 换成 metadata 中定义表的下标（【推断】）。

## 答案与答案键

```rust
pub struct Answers { per_hole: IndexVec<HoleId, AnswerId> }   // AnswerId(u32) 指向驻留的答案值
pub struct SpecializationKey { generic: Fingerprint128, answers: Fingerprint128 }
```

| 设计点 | 建议 |
|---|---|
| 答案值驻留 | 布局形状、`ScalarShape`、`ValidRange`、vtable 都驻留，`AnswerId` 相等即相同；同一个 `(Hole 种类, TyId)` 的答案在会话内备忘 |
| `AnswersHash` | 按孔位顺序，对每个答案的**持久编码**做 128 位哈希；`Resolve` 的答案是被调者的 `SpecializationKey`（已是 128 位），直接写入 |
| 递归 | 调用图 SCC 内先按 `InstanceKey` 分组，再做类细化（设计已定）；类细化的工作数组用 `IndexVec`，不用嵌套 `HashMap` |
| 去重表 | 键（32 B）放 `ValueStore`，表里只存 `(u32 id, u32 哈希)` |

成本估算（【推断】）：每个实例的孔位数按 5–30 计，每个答案的持久编码 ≤16 B，哈希一个实例的答案不到 1 µs；cargo 量级（约 10⁵ 个实例）合计不到 0.1 s。答案键的成本在**算答案**（布局查询、trait 选择），不在哈希。要测的是孔位数分布和答案备忘命中率，不是哈希速度。

## R 图哈希 = 序列化字节的哈希

建议 `GraphHash = hash128(canonical_bytes(压实后的 R 图))`：

1. 压实（按原顺序重编号）后按 [body-and-artifacts.md](body-and-artifacts.md) 的 varint 流写出；会话 `TyId` 换成文件内类型表下标，类型表本身按持久指纹写入。
2. 同一遍得到字节和哈希；字节进内容寻址存储，哈希就是地址。T4：varint 流只有原样记录 29% 的字节，T5：128 位 SipHash 约 6 ns/节点。
3. 键里不出现 `NodeId` 原始数值和删除历史。

对照现状：`native_cache.rs` 用自写 SHA-256 对**整个程序图**逐字段哈希，包括每个槽位的 `NodeId` 和 tombstone 标记。语义相同但删除历史不同的图得到不同的键（只会多失效，不会错误命中），而且每次全程序重算。

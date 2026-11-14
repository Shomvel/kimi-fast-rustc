# 先例（一）：紧凑 IR——Zig、Carbon、Cranelift

全部为【来源】。源码按标签或提交读取；“二手”表示只找到转录或笔记。

## Zig：data-oriented design

| 事实 | 数字 / 细节 | 来源 | 年份 |
|---|---|---|---|
| 演讲 | Andrew Kelley, *A Practical Guide to Applying Data-Oriented Design*, Handmade Seattle（[页面](https://media.handmade-seattle.com/practical-data-oriented-design/)） | 原始 | 2021-11 |
| 演讲中的数字 | token 64 → 5 B；AST 节点 120 → 15.6 B（平均，含 extra）；ZIR 指令 54.0 → 20.3 B（平均），wall −39% | [转录](https://www.josherich.me/podcast/andrew-kelley-practical-data-oriented-design-dod)，**二手**，视频未逐字核对 | 2021 |
| 演讲中的手法 | 用下标不用指针；布尔值放到带外；`MultiArrayList`（SoA）消除 padding；稀疏数据放哈希表；用“编码”代替 OOP 多态 | 同上 + 社区笔记，二手 | 2021 |
| PR [#7920](https://github.com/ziglang/zig/pull/7920) “Rework AST memory layout” | token = 1 B tag + 4 B 字节偏移 = **5 B**（不存结束位置，需要时重新词法分析）；AST 节点 = tag u8 + main_token u32 + lhs u32 + rhs u32 = **13 B**，SoA 所以不补到 16 | 原始 | 2021-02 |
| 同 PR：只看解析器 | cache miss −15%，指令 −28%，周期 −26%，wall −22% | 原始 | 2021 |
| 同 PR：`zig fmt` 整个 std | 峰值内存 84.7 → 68.4 MiB（−19.3%），wall −11.4%，但 **cache miss +34.5%**；评审者（trishume）事先提醒过 SoA 遍历可能更多 miss | 原始 | 2021 |
| [0.8.0 发布说明](https://ziglang.org/download/0.8.0/release-notes.html)：ZIR 重写 | print1mil：wall 0.93 → 0.57 s（−39%），峰值内存 645 → 386 MiB（−40%），cache miss −53% | 原始 | 2021 |
| ZIR 存储 | 4 个数组：u8 tag、(u32,u32) data、u8 字符串表、u32 extra；一次 `writev` 写盘。`std/os.zig`：26,586 条 ZIR 指令 234 KB，57,179 个 extra 项 223 KB | 0.8.0 发布说明 | 2021 |
| AstGen 吞吐 | 8.9 M 行/s（29 万行，release，冷缓存） | 0.8.0 发布说明 | 2021 |
| `Ast.Node`、ZIR、AIR | 都是 `MultiArrayList(Inst)` + `extra: []u32`；`Tag = enum(u8)`，Data 断言 8 B | `lib/std/zig/Ast.zig`、`Zir.zig`、`src/Air.zig` @0.14.0 | 2025 |
| InternPool PR [#15569](https://github.com/ziglang/zig/pull/15569) | 类型和值全部驻留。ghostty maxrss 1130 → 463 MiB、快 1.06×；Groove Basin 慢 1.11×；2Pew 慢 1.04× | 原始 | 2023-06 |
| 线程安全 InternPool PR [#20528](https://github.com/ziglang/zig/pull/20528) | 不开多线程：wall +5.7%、指令 −11%；codegen 单独线程：wall −33.3%、峰值 RSS **+28.1%** | 原始 | 2024-07 |
| InternPool 分片 | `locals: []Local`（每线程一个），`shards: []Shard`（2 的幂 = 同时写者上限），线程号放在索引高位（`tid_shift_*`） | `src/InternPool.zig` @0.14.0 | 2025 |

## Carbon toolchain

| 事实 | 数字 / 细节 | 来源 | 年份 |
|---|---|---|---|
| 设计模式“向量化存储” | 数据放在向量里，到处传 flyweight（下标），避免指针式堆分配；“相邻条目一起被缓存”；分阶段比回调局部性好 | [toolchain/docs/README.md](https://github.com/carbon-language/carbon-lang/blob/trunk/toolchain/docs/README.md) | 2026 |
| Parse 节点 | `NodeImpl` = kind + error 位 + token 下标，`static_assert(sizeof == 8)`；后序存放，每个节点记子树大小 | `toolchain/parse/tree.h`、`docs/parse.md` | 2026 |
| Token | kind 8 位 + 1 位 + 23 位载荷 + `int32_t` 字节偏移 = 8 B；kind/载荷是“最热字段”，偏移词法分析后变冷 | `toolchain/lex/token_info.h` | 2026 |
| SemIR `Inst` | `kind: i32, type_id, arg0, arg1` → `static_assert(sizeof == 16)`；注释说可压到 12 B，但“16 字节是理想的寄存器尺寸” | `toolchain/sem_ir/inst.h` | 2026 |
| `ValueStore` | 下标类型 ↔ 值类型，向量式存储；`Add` 返回 ID，`Get(id)` 取引用；4 KiB 分块（在 Linux-x64 和 Mac-arm64 上比 1 MiB 块快） | `docs/idioms.md`、`base/value_store.h` | 2026 |
| `CanonicalValueStore` | 去重的不可变值：`ValueStore` + 只存 ID 的哈希 `Set<IdT>` | `base/canonical_value_store.h` | 2026 |
| 速度目标 | 词法+语法约 100 ns/行，语义约 1 µs/行 | Carruth CppNow 2023 演讲；仓库内**未找到原始来源**，仅二手转述 | 2023 |

## Cranelift

| 事实 | 数字 / 细节 | 来源 | 年份 |
|---|---|---|---|
| 实体引用 | `entity_impl!` 包一个 `u32`，`u32::MAX` 保留作 None（`PackedOption`）；普通 `Option<T>` “会让表大一倍” | [cranelift-entity](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/entity/src/packed_option.rs) | 2026 |
| 不删除 | README：“不提供删除实体的方式”，换来不需要槽位复用簿记和版本号 | [entity/README.md](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/entity/README.md) | 2026 |
| `PrimaryMap` / `SecondaryMap` | 前者分配键，后者用同一键挂旁表（值需 `Default`） | 同上 | 2026 |
| `EntityList` | **4 B**（`Vec<T>` 是 24 B）：池中下标；池是一个 `Vec<T>`，每个列表 = [len, 元素, 余量]，总长取 2 的幂，按 LIFO 分配器设计 | `entity/src/list.rs` | 2026 |
| `InstructionData` | 测试断言 16 B；注释：“大小对性能很重要，不应超过 16 字节”，更大的格式用 `Box` 外置 | `codegen/src/ir/instructions.rs` | 2026 |
| `DataFlowGraph` | `insts: PrimaryMap<Inst, InstructionData>`；`results: SecondaryMap<Inst, ValueList>`；`values: PrimaryMap<Value, ValueDataPacked>`（**10 B**：tag 2 位 + 类型 14 位 + x u32 + y u32）；`value_lists` 池放不下内联的操作数、结果、块参数 | `codegen/src/ir/dfg.rs` | 2026 |
| `Layout` | 指令/块的程序顺序是 `SecondaryMap` 上的双向链表，与 DFG 分开；“不记录数据依赖” | `codegen/src/ir/layout.rs` | 2026 |
| use 信息 | DFG 无 use-list；lowering 时按需算 `ValueUseState ∈ {Unused, Once, Multiple}` | `machinst/lower.rs` | 2026 |

## 对 kfrc 的启示（【推断】）

| 共同做法 | kfrc 对应 |
|---|---|
| 节点是 8–16 B 定长记录，变长部分进共享 `extra`/池 | 节点记录 16 B + 溢出池，见 [node-storage.md](node-storage.md) |
| 用 u32 下标代替指针，旁表按同一下标 | `NodeId`、`TyId` 已是 u32；旁表用 `IndexVec`/`SecondaryMap`，不用 `HashMap<NodeId, _>` |
| 驻留/去重表只存 ID，键留在存储里 | GVN 表、`TyId` 驻留表，见 [interning-and-keys.md](interning-and-keys.md) |
| SoA 不是免费的：Zig 的 `zig fmt` cache miss +34.5% | 默认 AoS，只把冷字段拆成列（T1 中 AoS/SoA 差距 ≤20%） |
| 多线程带来峰值 RSS：Zig +28.1%、rustc 最多 +35% | 并行预算按字节计，见 [parallel-memory.md](parallel-memory.md) |

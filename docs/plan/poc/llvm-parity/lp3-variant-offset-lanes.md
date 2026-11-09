# LP3：多变体 enum 载荷按字节偏移共享 lane

2026-11-09。基线 c6-frame（`94ea0fb`），semver O3/16 CGU 全部 `.text` 67,650 B，LLVM O3 18,515 B。

## 问题

`Parts::Variant { variant, variants: Vec<Vec<Value>> }` 为每个 (variant, field) 叶子各建一个值。
semver 的 `ErrorKind` 有 13 个变体，载荷只有 `Position`(u8@1) 与 `char`(u32@4)，
却在合流、解码、写回时被当作约 13 条 lane：解码是 `cmp tag,k; je; load` 链，
写回是 `cmp tag,k; je; store` 链，所有 lane 同时活跃，寄存器和帧槽随之膨胀。
LLVM SROA 按字节切片 alloca，同一偏移、同一尺寸的字段落在同一切片，非活跃变体的切片只是 undef。

## 表示

对类型 `T` 计算一次 lane 计划，lane 键为 `(字节偏移, 叶子类型)`：

- 只对 `TagEncoding::Direct` 的 enum 生效：tag 与所有载荷字节不重叠。
  Niche／Single／Empty 保持现有表示。
- 字段“可平铺”：非零尺寸标量（≤8 字节，非 vector／union／MaybeUninit）、
  全部变体无字段的 Direct enum（叶子是其物理 tag），或由可平铺字段组成的 product（不含数组）。
  零尺寸叶子不占 lane。
- 叶子类型取 `TyId`，同偏移同类型自然共享；同尺寸不同类型（如 u32 与 char）暂不合并，留作后续。
- 某字段任一叶子与另一个不同键的叶子字节区间重叠，整个字段退回原表示（按变体守卫）。
  因此所有 lane 两两不相交，可以无条件读写。

值表示不新增 `Parts` 变体：可平铺字段仍是普通的 `Value` 树，但其叶子节点按 lane 键共享，
同一 lane 的所有变体字段指向同一个 SSA 节点。不变量：同一 `T` 的 `Parts::Variant`
生产者（解码、Phi、Undefined）按同一计划共享叶子。

## 各操作

| 操作 | 新行为 |
|---|---|
| 内存解码 | 读 tag 后，每个被需求的 lane 无条件 `Load` 一次；仅不可平铺字段保留按变体守卫的读取 |
| Phi | 每个被需求的 lane 一个 Phi；构造器入边只填活跃变体的 lane，其余 lane 填 poison |
| 变体→变体合流 | 共享叶子同一节点，逐 lane 填一次；来源不一致时报 `Malformed` |
| 写回 | 动态变体先无条件写全部 lane，再按变体守卫写不可平铺字段，最后写 tag；常量变体与构造器路径不变 |
| 字段读取／switch／tag 查询 | 不变：字段仍从 `variants[v][f]` 取，读到的是共享叶子 |
| 边界运输 | 被证书选中的字段仍走私有快照；根值只流向提取者，不进入 Phi 或写回 |

## 语义边界

- 非活跃变体的载荷字节不可观察：共享 lane 可以携带其他变体的位，读出后只在活跃变体守卫下使用。
- L 层 `Load` 读未初始化字节得 poison 标量，不报错；`Store` poison 使字节回到未初始化。
  所以无条件读写不新增可观察行为；Rust 层 definedness 要求仍在 S 层解释。
- 写回顺序保证正确性：lane 互不重叠且先于守卫写；守卫写只在其变体活跃时发生，可覆盖 lane 留下的字节。
- MaybeUninit、数组、union、Niche 编码、未知字段形状一律保持现有表示。

## 不变的部分

tag 域（`VariantScalar`）、`KnownVariant` 构造器、`EncodedVariant`／`VariantTag` 快路径、
owned／boundary 运输、需求 worklist 的键和传播。
`owned_transport::scalar_leaves` 的表示成本估计在第二步改为按 lane 计数，单独测量。

## 验证

- 回归：kfrc-son 语义测试覆盖共享 lane 的构造／Phi／内存往返／字段读取，与 S 解释器对拍，
  并断言 L 层 lane Phi 数、无按变体守卫读取；重叠区间退回原表示；循环 Phi。
- E2E：semver 体积（size-iter）、c5-t64 全量验收与 runtime。

## 实测（2026-11-09）

冻结包 `38f399b0…`（提交 `a913167`）在同一 E2B 沙箱通过完整验收：Linux 单元 85 套 763 项、
1 项按原配置忽略；语义三臂 631 行、标量 565 行、生成 5×40、压力 2048 行、semver 34 项、
7 套旧语料、CLI、特性探针、rs3 形状、u128 拒绝均与 c6-frame 一致。

semver O3/16 CGU 全部 `.text` 67,650 → 58,393 B（−13.7%），49 个符号缩小、无符号增大。
`from_str` 6,445 → 5,431 B，L Phi 200 → 136、Undefined 114 → 51；`version_req` 4,368 → 2,553 B；
`Version/Comparator/VersionReq::parse` 各缩小约 70%。同轮 runtime 相对 c6-frame：
parse −5.9%、req −5.6%、identifier −0.1%。

第二步把 `owned_transport::scalar_leaves` 改为按 lane 计数，semver 字节完全不变且两个
私有槽回归改变选择，已撤回。剩余：`<Error as Display>::fmt` 的 11 路比较链是
VariantSwitch 逐例分支（LP5 switch 降低），`parse::comparator` 体积主要来自寄存器 mov，
均不属于 lane 表示。

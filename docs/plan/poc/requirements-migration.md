# 要求与凭据：实施批次

契约以 [requirements-and-facts.md](../../design/ir/requirements-and-facts.md) 为唯一
设计来源。2026-11-09 已逐项核对 graph、builder、specializer、interpreter、IPO、
layout 与 x86；该设计文件已有完整概念表，不再另建一份重复契约。

保留旧协议基线 candidate11：
`b69fb91feeaf0db7776a0da5b21fd5325123aca5903ec6dc1000feed7216d356`。
其 identity 优化减少 ABI 往返，但新增观察成本；不能用节点下降声称运行收益。

## 已核对的实施决策

- `RequireDefined(effect,value)` 固定在原控制位置，返回 `(Control,Effect)`，
  不返回业务值。只消费 poison；MU/raw mask、padding、inactive payload 保持既有规则。
- 类型有效性目前由 typed decode/转换的明确契约承担；不新增没有实现的
  `RequireValid` 空壳，不声称引用/provenance/完整 validity 已验证。
- R/S 的 Ignore ABI 参数、调用结果与 Return 保留源码值；只在 L 决定传输位。
  Never、无正常返回调用与普通 ZST 分开处理。
- 发布 S→L 在任何 expand/representation demand 之前擦除非运行时要求，
  按源 defined 行为保持的 refinement 验证。R/S 仍解释要求；严格诊断测试不全局放宽。
- 第二批用无机器值的 VariantWitness、固定 Assume 及实际消费者放置依赖
  迁移通用 Refine；Proven 失败是编译器错误，不进入源 UB 宽容条件。
- 源 `LoadValue` 是 owned snapshot；不把擦除要求扩展成延后原地址读取。
  仅证明需求不触发 payload decode，真实调用/检查/必要内存操作不被连带删除。

## 源码边界核对与当前覆盖

下表记录实际实现，不把 IR 内部允许的部分值误称为完整 Rust 值有效性验证。
Rust 的有效性义务涉及值产生、类型化读／转换、传参、返回；本轮拆分并未将这些
义务缩到调用边界，也没有补齐 Rust 的全部规则。

| 边界／操作 | 当前解释与检查 | 发布阶段处理／限制 |
|---|---|---|
| Aggregate、MakeVariant、字段更新、Phi | 允许内部部分值与 poison；enum 只持有活动字段；Phi 同时选择前一轮输入 | 结构构造不是递归有效性检查；只为真实消费者产生表示 |
| LoadValue | 在原控制／效果点产生 owned snapshot；decode 检查已建模 bool/char、tag 与内存访问；未初始化字段可形成 poison | 有真实消费者时保留读取时刻；仅证明消费者消失时可按发布 refinement 省去普通非 volatile decode |
| StoreValue／部分赋值 | 保存初始化 mask 和活动表示，允许 IR 中间部分状态 | 不把每次字段写入变成完整 T 有效性检查；真实写入与排序保留 |
| TransmuteValue、MaybeUninit→T | 检查表示大小、源／目标 poison，并按目标类型 decode；原始 MU 内容含未初始化字节合法 | 保留实际转换；不以 RequireDefined 代替目标类型规则；padding 不要求全部初始化 |
| 函数参数、Call、Return | 保留源码类型和值，包括 Ignore；检查值形状及 poison，返回结构由类型检查 | 内联删除边界时按需留下 RequireDefined，L 才适配 ABI；形状检查不是完整引用 validity |
| RequireDefined | 活动语义组成不含 poison；MU mask、padding、inactive payload 不被全字节观察 | 表示需求分析前擦除；无业务结果，无机器数据需求 |
| Assume | bool 必须 defined 且为真，否则明确源 UB；控制／效果排序保留到擦除点 | false 不代表真实 panic；源码 assert/Switch 仍为真实控制流 |
| VariantWitness／带凭据 VariantField | 证明来源、原值、变体、控制范围、动态实例一致；错误是 Malformed／verifier 错误 | 凭据只证明 tag，既不证明所有 payload defined，也不证明地址可解引用；不生成机器值 |
| Unit、零数组、ZST enum、Never | Unit/空数组无待观察元素；ZST enum 仍有语义变体，只有唯一 inhabited 变体可补零位运输；Never 不构造正常值 | Ignore 不是“无语义”；不把 Never 当 Unit，不猜测多变体 ZST |

实现入口：`interpret/value.rs`、`interpret/semantic.rs`、`interpret/semantic_memory.rs`、
`semantic_value.rs`、`transforms/ipo/splice.rs`、`lower_layout/observation.rs`。
当前引用／provenance、完整 validity 与权限模型仍是明确缺口；本轮不引入空壳
RequireValid，也不允许优化器使用尚未证明的有效性事实。

## Ownership 与冻结顺序

| Owner | 第一批职责 |
|---|---|
| contract | graph 节点契约、arity/type/stage verifier，保留后续 witness 接口设计 |
| builder | build.rs 与 build/ 的源码返回/调用值和要求构造 |
| generic | interpreter/、specialize/ 的要求解释和纯代入源码值保留 |
| optimizations | transforms/ipo、memory observation/promotion、scalar/control 的要求消费者迁移 |
| layout | lower_layout/ 与 x86 的发布擦除、需求分析、Ignore ABI 运输 |
| root | 设计/状态、analysis 接缝、验证关系辅助与跨模块整合 |
| validation | 集中编译/E2B、冻结候选、原始语料与性能证据 |

先迁移要求和源码值，集中测试、冻结；再迁移 witness/Assume/placement，集中
测试、冻结。不能留下旧 Observe/Refine 与新表示两套生产入口。
每批首次错误后先静态同类排查并批量修复，再恢复动态验证。

## 验收

保留源 defined 输入上的结果、调用轨迹、内存和终止强断言；source UB 可允许
发布目标正常返回，但 Proven/Malformed/UnknownCallee/OutOfFuel 不作为 source UB。
纯 R→S 相等与显式优化/refinement 分开测试，不以节点编号相同代替诊断来源对应。

结构门槛：合法 enum 数据流增加冗余 definedness 要求后，发布机器分支、load/store、
搬运不增；witness 不产生整树 pin；仅证明引用不触发物化。覆盖 MU/padding/
inactive/nested/零数组/ZST/Never、分支域、循环实例、别名快照、调用和真实 panic。
semver 同时报 S/L、分支、load/store、mov、text、编译 instructions 与 runtime。
原始 regex 的 caller-location adapter 缺口单独保留，不混入这一契约迁移。

## 第一批实现记录（candidate12 已验收）

已将生产 `ObserveValue` 替换为无业务值输出的 `RequireDefined`，解释器继续在
原控制位置消费 poison。builder/specializer 不再根据 Ignore 删除或重造源码值；
IPO 用原 SSA 值，MemoryFacts 把要求当作不捕获指针的语义使用，不派生新指针。

发布 lowering 在表示需求分析之前擦除要求，并删除失去用途的纯证明锥；
没有真实结果消费者的非 volatile `LoadValue` 不再解码 payload。
有实际用途的 snapshot 仍在原读取点捕获，Call、Store、Opaque 与陷入操作保留。
Ignore 的零位运输适配仅在 L 入口执行，合法 ZST enum 必须有唯一 inhabited 变体。

结构用例比较 0、1、16 个冗余要求下的完整机器指令序列，并使用未提供物理布局
答案的证明专用 enum。R/S 的 MU/padding/poison 与调用轨迹强断言保持；
发布差分的分类辅助只允许明确的语言 UB，不把编译器或解释器错误放行。
此时通用 Refine 仍保留，属于第二批，不能把第一批称为整个重设计完成。


第一批实际验收见 [candidate12 回执](../../../evidence/poc-semantic-only-20261109/candidate12-acceptance.json)。
Linux 592 passed / 1 ignored、真实三臂功能、381×3 语义语料、完整 corpus、CLI 和
semver 原版测试通过；性能及限制见 [当前状态](semantic-ir-status.md)。

## 第二批实现记录（candidate13 已验收）

- `VariantWitness` 采用独立 Witness 类型，只引用原值和证明控制；不是可执行根，
  不分配机器值。`VariantField` 可附 witness，也保留无 witness 的全函数 poison 形式。
- verifier 检查值／变体身份、证明来源、消费者控制和 Phi 前驱；循环动态定义重新执行
  后不能使用旧凭据。事实错误归为编译器错误，不能作为源 UB 放过。
- `Assume` 独立固定在控制／效果链上，无业务结果。两类 rustc MIR 入口均保留它，
  Typed Body 与定义级 R 都有 producer；真实运行时 Switch／panic 不改成 Assume。
- `PredicateFacts` 查询分支与 Assume 延续中的标量谓词；不从非空推导内存许可，
  不引入永久指针属性。尚未实现完整范围求解或内存权限类型系统。
- IPO、特化、GVN、CFG 重写均重映射凭据；无法沿用证明的重写恢复保守字段投影。
- 发布布局在需求分析前擦除要求，消费 witness 后移除证明引用；删除整树 pin 与
  x86 Refine Mov。真实 load／调用／陷入操作的控制和效果保留，owned 快照不延后重读。
- 新增回归覆盖冗余要求机器序列不变、witness 无搬运、嵌套／错误分支／循环实例、
  假设 false／poison、Move 和动态索引、实际检查，以及真实泛型 Assume 入口。

第二批回执见 [candidate13](../../../evidence/poc-semantic-only-20261109/candidate13-acceptance.json)。
Linux 608 passed / 1 ignored、真实 Assume 三臂、语义差分、完整 corpus、semver 和
生成式 200 函数／1,200 向量全部通过。结构测试证明要求与凭据不增加机器工作。
详细指标见 [当前状态](semantic-ir-status.md)：from_str 搬运和体积下降，但编译指令
及 req runtime 相对第一批回退，不能称为整体性能提升。candidate12 归档保持原样。

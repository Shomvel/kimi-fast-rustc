# kfrc 前端设计建议：减少工作 / 并行 / 缓存 / 数据布局

“上限”按 [time-breakdown.md](time-breakdown.md) 的前端占比（check 场景为主，release 另注）做 Amdahl 估算，是推断；“依据”列给实测或资料。对现有设计的具体改动见 [design-gaps.md](design-gaps.md)。

## A. 减少工作

| # | 做法 | 依据 | 收益上限（推断） | 设计现状 |
|---|---|---|---|---|
| A1 | MIR 级优化不放在接口之前（R 级在接口发布后、按需） | release 前端中 `fe_mir_opt` 15–22%；P2a：库元数据阶段 90–98% 可后移 | release：串行段 −15…−22%；check 只有 2–3% | 已有（[goals/compile-time.md](../../design/goals/compile-time.md) C4） |
| A2 | 不建独立 HIR/THIR：解析 + 展开产出唯一的“HIR 级 AST”，Body 由它 + 解析表 + TypeckResults 直接构建 | cargo release `hir_crate` 3.03 s + `thir_body` 1.17 s ≈ 前端 3.8%；rustc 借用检查还深拷贝一次 MIR | 前端约 4–6% | THIR 已跳过（[frontend/body.md](../../design/frontend/body.md)）；T04 仍有“AST → HIR 降低”一步 |
| A3 | 接口指纹只覆盖接口（签名、公开类型、impl 集合、导出宏、接口体），不做覆盖全部函数体与 span 的 SVH | rustc 凡输出 metadata 都稳定哈希全部 HIR；release 改注释重做 28.7% | 冷构建：省掉函数体哈希（占比未测）；增量：下游截断 | [build/cache.md](../../design/build/cache.md) 有依赖清单，缺“接口指纹”的定义 |
| A4 | 借用检查和 typeck 全做；R 级以后只对可达/导出/可内联的 body 做；非 deny 级 lint 默认不跑 | 链接丢弃 22%（rg）的机器码；`fe_lint` 2–3% | 前端：lint ≤2–3%；后端见 C6 | C6 已有；lint 未规定 |
| A5 | 宏转录不做 AST→token→AST 往返：片段匹配结果以已解析节点代入，保留不可见分组语义 | rustc 转录时 `TokenStream::from_ast` 后再解析（[why-slow-passes.md](why-slow-passes.md)） | `fe_parse_expand` 6–10% 中的一部分 | 未规定（T23） |
| A6 | metadata 接口段按 `DefKey` 索引、惰性解码；同一构建内 sysroot/依赖的解码结果共享 | `md_decode` 占前端 7–9%；每个下游进程重解码 std（`impl_trait_header` 320 万次）；Go 惰性导入 −12…−20% 墙钟 | 前端约 5–8% | T29 只列内容，不列访问方式 |

## B. 并行

| # | 做法 | 依据 | 收益上限 | 设计现状 |
|---|---|---|---|---|
| B1 | 名字解析后冻结快照，按 typeck root 并行 typeck/trait/借用检查/Body 构建，固定 worker + 确定性划分 | 可并行部分占 check 前端约 65%；rustc `-Zthreads=8` 整构建 Cargo −6.2%、rg −15.1%，关键路径前端段 −42% | 单 crate 8 worker ≤2.3×；整构建个位数到十几个百分点 | 规则有（typeck root 为单位），T11 写“前端在 M1 串行”，之后无任务 |
| B2 | 文件按发现顺序并行解析；`macro_rules` 展开与模块骨架按文件并行，名字解析不动点保持确定性定序 | rustc parse/expand/resolve/lowering 无任何 `par_*`；串行比例 35% → 25% 时 8 worker 上限 2.3× → 2.9× | 再 +25%（单 crate） | T05 只规定 DefKey 定序 |
| B3 | 签名优先发布：接口就绪即发，接口体（opaque、async、const fn、static、CTFE 依赖）先检查 | 签名就绪在 rmeta 的 10–17%；P2b 模型 rg −28.5%，8 vCPU 实测 −3.3%；headstart 16 核 −42% | 依核数：8 核个位数，16 核以上 20–40% | 已有（C4、接口体）；接口体清单少于 P2b 实测的 pending 类 |
| B4 | crate 内 worker 与 Cargo jobserver 共用 token；等待时归还 | P2b：8 vCPU 下提前开工只换来 CPU 争用 | 防退化 | 已有（[build/pipeline.md](../../design/build/pipeline.md)） |

## C. 缓存

| # | 做法 | 依据 | 收益上限 | 设计现状 |
|---|---|---|---|---|
| C1 | trait 求解缓存只在一次构建内共享（跨 crate 进程可共享，但不落盘、不优先） | 跨 crate 重复只占 trait 查询 self 时间 12.5%，执行次数 9.8% | 冷构建 ≤ 前端 1% | 未规定；建议明确“不做持久 trait 缓存” |
| C2 | 增量下缓存单次宏调用的展开结果，键 = 宏定义哈希 + 输入 token + 调用处 ctxt/edition + 读过的 cfg | rustc 增量每次全量展开：Cargo 增量前端 14%（0.64 s），rg 8.5% | 增量前端 8–14%；冷构建 0 | [build/cache.md](../../design/build/cache.md) 写“不存” |
| C3 | 增量用产物级依赖清单 + 接口早截断，不用查询级依赖图 | rustc 增量前端里依赖图 26–33%，真正重查 ≤6% | 增量前端约 1/3 | 方向已有；缺“验证成本”指标 |
| C4 | sysroot 与 registry crate 按 durability 一次验证 | rust-analyzer durability 前每次编辑重验 std 约 300 ms | 增量固定成本 | prior-art 已建议，未写进 cache.md |

## D. 数据布局

| # | 做法 | 依据 | 收益上限 | 设计现状 |
|---|---|---|---|---|
| D1 | token / AST / Body 用 u32 索引 + struct-of-arrays，少见字段外置，span 放旁表 | Zig 解析 −22%、ZIR −39% 且内存 −40%；Carbon token 8 B、节点 8 B；K2 合并树与旁表后分析阶段 2.5–3 倍 | 解析展开 + 名字解析（前端 17–18%）× 1.3–2 | T04 只列结构，未列布局约束 |
| D2 | 推导期类型放 body 局部 arena，writeback 时才进全局驻留表；全局表按线程分片 | rustc 每个 `mk_*` 哈希 + 探测全局表；Zig InternPool 按线程分片 | 未测；并行时减少全局表写入 | [frontend/types.md](../../design/frontend/types.md) 已规定 Infer 不出 body；[ids.md](../../design/ids.md) 已改为分片 + 每任务本地缓存；“推导期临时类型不进全局表”未写 |
| D3 | 借用检查的 region 用“region 槽 → RegionVid”旁表，不克隆 Body；优化重点放 MIR 类型检查与 region 求解 | 借用检查中数据流 4.5%，未拆分的类型检查/求解/访问检查 66.6%；rustc 每个 body 深拷贝 MIR | 借用检查（前端 9–16%）的一部分 | body.md 写“替换为自己的 RegionVid”，未说不拷贝 |
| D4 | 推导设确定性燃料上限 | Swift 按选择次数而非时间限界 | 防最坏情况 | 预算规则已有，未覆盖 typeck/trait |

## E. 合计估算（推断，check 场景、单个大 crate）

| 步骤 | 依据 | 前端 CPU |
|---|---|---:|
| rustc 1.94.1 单线程 | 基线 | 1.00 |
| A2 + A4 + A6 | 4–6% + 2–3% + 5–8% | 0.83–0.89 |
| D1（只作用于 17–18% 的解析段） | × 1.3–2 | 0.74–0.85 |
| typeck/trait/借用检查实现效率 | 假设与 rustc 持平（rustc 这部分已多年调优） | 不变 |
| B1 + B2，8 worker | 串行 25–35% | 墙钟 ÷ 2.3–2.9 |

结果：单 crate 前端墙钟约快 2.7–3.9×，CPU 约省 15–26%。整构建还受 crate 图和构建脚本限制：Cargo check 关键路径 28.4 s 中构建脚本 12.95 s，前端无限快也不到 2.2×。这与前期“前端是地板、冷构建 1.5–3×”的结论一致（前期 rustc-10x-rewrite）。冷构建的大倍数只能来自缓存与不重复（依赖命中缓存），增量的大倍数来自去掉 O(crate) 固定成本（C2、C3）。

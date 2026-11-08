# 对照现有设计：缺口与改法

只列建议，不改这些文件。路径相对 `docs/`。证据编号对应本目录各文件；“推断”见 [recommendations.md](recommendations.md)。

## 1. 建议清单

| # | 问题 | 证据 | 改法 | 目标文件 |
|---|---|---|---|---|
| G1 | 前端库内并行没有落到任何任务：T11 写“前端在 M1 仍串行”，之后的 T37–T40 只讲跨 crate 与实例级 | check 前端占全部工作 89–92%，可按 typeck root 并行的约 65%；rustc 关键路径前端段 `-Zthreads=4` −42%（[parallel-frontend.md](parallel-frontend.md)） | 加一项任务（建议并入 T40 或单列）：名字解析后冻结快照，typeck root 级 worker，固定数目 + 确定性划分；完成标志含 jobs=1/8 诊断与产物一致、关键路径大 crate 的 1/4/8 worker 曲线 | `plan/m6-quality-performance.md`（T40）、`plan/m1/t11-tasks-budget.md`（注明后续任务） |
| G2 | 冻结点没有定义：哪些表在函数体任务开始前必须完整 | rustc typeck 中途按需拉签名，并行时靠查询等待与 deadlock handler；K2/Sorbet 先完成签名再并行（[prior-art-layout.md](prior-art-layout.md)） | 写明阶段：收集定义 → 签名与谓词 → impl 索引与 coherence → **冻结** → 函数体；例外只有接口体（G3），按任务图边处理 | `design/frontend/overview.md`（循环与等待） |
| G3 | 接口体清单少于 P2b 实测的 pending 类 | P2b M0：闭包/协程体、inline const、表达式内 AnonConst、RPITIT 关联类型与 static 的 `type_of`、`trait_impl_trait_tys`、启用特化时的 `impl_parent`、static 初值、CTFE MIR（`mir_for_ctfe`/promoted/trivial const） | 按 P2b 的类别补全“接口体”列表，并规定接口发布期间不得触发函数体检查（调试断言，对应 P2b 的 `sig_probe`） | `design/goals/compile-time.md`（关键路径·接口体）、`design/build/pipeline.md`（产物依赖） |
| G4 | 没有“接口指纹”的定义；下游失效粒度不明 | rustc 凡输出 metadata 都对全部 HIR 稳定哈希，SVH 含函数体与 span；release 改注释重做 28.7%（[why-slow-infra.md](why-slow-infra.md)） | 定义接口指纹 = 签名、公开类型与布局依赖、impl 集合（含否定事实）、导出宏与卫生表、接口体结果；不含私有函数体、span。下游接口依赖只读它 | `design/build/cache.md`（依赖清单）、`design/ids.md`（`Fingerprint` 种类） |
| G5 | T29 只列 metadata 内容，不列访问方式 | `md_decode` 占前端 7–9%；cargo release 中 `impl_trait_header` 解码 3,207,143 次；Go 惰性导入 −12…−20% 墙钟；Clang PCH 只读 0.024% 声明 | T29 加：接口段与 R 图段分开发布；按 `DefKey` 的定宽零拷贝索引；按需解码；同一构建内共享 sysroot/依赖的解码结果；禁止打开 crate 时预建全 crate 映射；指标：解码条目数与字节 | `plan/m4-cargo-sysroot.md`（T29）、`design/build/cargo-sysroot.md` |
| G6 | 宏展开结果“不存” | 增量重建里 rustc 每次全量展开：前端 8.5%（rg）、14%（Cargo，0.64 s）；`-Zcache-proc-macros` 默认关 | 冷构建维持不存；增量下按单次调用缓存，键 = 宏定义哈希 + 输入 token + 调用处 ctxt/edition + 读过的 cfg；先做内建 derive 与 `macro_rules`，过程宏需声明纯 | `design/build/cache.md`（存什么） |
| G7 | 宏转录方式未规定 | rustc 转录把已解析片段 `TokenStream::from_ast` 转回 token 再解析 | T23 规定片段以已解析节点代入，保留不可见分组语义；加“宏内表达式优先级/不可见分组”回归 | `plan/m3-semantics.md`（T23） |
| G8 | T04 仍有“AST → HIR 降低”一步，可能建两棵树 | cargo release `hir_crate` 3.03 s、`drop_ast` 0.38 s；HIR+THIR 约占前端 3.8% | 脱糖在展开收尾时原地完成，只保留一棵 HIR 级 AST；HIR 级节点 ID 在此时分配 | `plan/m1/t04-syntax.md`、`design/frontend/overview.md` |
| G9 | 借用检查是否复制 Body 未规定 | rustc 每个 body 深拷贝 MIR；借用检查内数据流只占 4.5%，类型检查/求解/访问检查占 66.6%（[why-slow-passes.md](why-slow-passes.md)） | region 槽 → `RegionVid` 用旁表，不复制 Body；T13 的性能关注点写成“约束生成与 SCC 求解”，不是 bitset 数据流 | `design/frontend/body.md`、`plan/m1/t13-ownership-borrow-experiment.md` |
| G10 | 复用 next-solver（决策 D2）没有性能闸门 | rustc 1.94.1 `-Znext-solver=globally` 让 rg check 的 rustc 指令 ×1.148（本机 ABBA，`next-solver-ab.sh`）；trait 类占 check 前端 8.6–10.3% | T12/T21 加闸门：每 crate trait 求解指令与 rustc 默认对比；记录全局缓存命中率；超出时先查缓存与 fast_reject 是否接上 | `plan/m1/t12-trait-experiment.md`、`plan/m3-semantics.md`（T21） |
| G11 | 没写“trait 结果不跨构建持久化” | 跨 crate 重复只占 trait 查询 self 时间 12.5%（≤前端 1%） | 存什么表加一行：trait 求解结果只在构建内共享，不落盘 | `design/build/cache.md`（存什么） |
| G12 | 编译时间指标没有前端分项 | 本目录各表 | 加：与 `phase-lib.jq` 同口径的前端分项占比；接口就绪 / 分析结束 / R 图就绪时刻（与 P2b 签名就绪表同口径）；每 crate 1/4/8 worker 墙钟与峰值内存；metadata 解码条目数；各阶段 ns/行 | `design/goals/compile-time.md`（指标与闸门） |
| G13 | T04 只列结构，没有布局约束 | Zig 解析 −22%、ZIR −39%；Carbon token/节点 8 B；SoN 节点布局已在 [data-structures](../data-structures/README.md) 规定 | T04 加：token ≤8 B、AST 节点定长 + `extra` 外置、u32 索引、span 旁表；Body 同理（与 data-structures 的 Body 建议对齐） | `plan/m1/t04-syntax.md` |

## 2. 不需要改的

| 现有设计 | 本目录的支持证据 |
|---|---|
| R 级优化在接口之后（C4） | release 前端 15–22% 是 MIR 优化；P2a 元数据阶段 90–98% 可后移 |
| 跳过 THIR，Body 由 HIR 级 AST + TypeckResults 构建 | rustc 多一层 THIR，`thir_body` 1.17 s |
| 推导变量只在 body 局部表 | rustc 推导期类型进全局驻留表；与 [ids.md](../../design/ids.md) 的分片 + 本地缓存驻留一致 |
| 不用查询系统驱动前端 | rustc 非增量下查询开销小，增量下依赖图占前端 26–33%；Salsa 冷分析不快（rust-analyzer ripgrep 9.1 s） |
| durability 三级 | rust-analyzer 300 ms 的先例；已写进 [build/cache.md](../../design/build/cache.md) |
| 借用检查对所有函数体都做 | Zig 惰性分析改变报错语义（[prior-art-layout.md](prior-art-layout.md)） |

## 3. 落地顺序

按“晚加要返工”的程度排，不按收益排。

| 阶段 | 条目 | 理由 |
|---|---|---|
| M1（T04–T06、T13） | G2、G8、G9、G13 | 冻结点、单棵树、region 旁表、节点布局都是数据模型；M1 定错后 M3 要改遍调用点 |
| M3（T21、T23） | G7、G10 | 宏转录方式与求解器闸门在功能落地时一起定 |
| M4（T29） | G4、G5 | metadata 格式与接口指纹一起冻结 schema |
| M5–M6（T37–T40） | G1、G3、G6、G11、G12 | 需要真实 crate 的关键路径数据；G6 只在增量场景实测展开占比后做 |

G1 的完成标志要允许“不采用”：与 T40 一样，若 8 核整构建没有稳定收益，保留串行前端并记录数据。

# 用途、代价与首版范围

## 用途

| 用途 | 做法 | 需要哪一级的语义 | 收益 |
|---|---|---|---|
| **逐 pass oracle** | 每个 pass 前后在同一组输入上运行解释器，比较可观察行为；不一致时配合 `--opt-bisect-limit` 定位到 pass 和规则序号 | R、S、L | 把“最终二进制错了”缩短到“哪个 pass、哪条规则”；不需要 SMT |
| **检查模式** | 解释器同时断言分区归属、token 新鲜度、`Refine{Proven}`、答案读取（[memory.md](memory.md)、[refinement.md](refinement.md)） | R、S、L | 抓到“结果碰巧对、但事实已经错”的图，这类 bug 在别的输入上才表现为误编译 |
| **常量求值** | 不用 R 图；CTFE 解释 Body，与 IR 解释器共享核心 | Body | 见 [refinement.md](refinement.md#ctfe-能不能解释-r-图) |
| **规则验证** | 规则表里的标量规则生成 SMT（位向量 + poison 标志 + UB 标志）；聚合/变体/`Refine` 规则用小宽度穷举对解释器 | 标量部分的 SMT 语义 + 解释器 | 与 [ct-testing.md](../prior-art/ct-testing.md) 的规则表方案对接 |
| **翻译验证** | 只做差分执行（上面第一行），不做 Alive2 式 SMT 整图验证 | 同上 | 成本线性；漏报靠 fuzz 输入覆盖 |
| **verifier 的来源** | 每条 verifier 检查是某条语义规则的静态近似（[verifier-mapping.md](verifier-mapping.md)） | 全部 | 新增节点时先写语义，再推出检查 |
| **fuzz** | Rustlantis 程序与自写生成器：Miri → Body 解释器 → R/S/L 解释器 → 本机执行，四方对拍 | 全部 | 定位到层 |
| **缓存键审计** | 解释器记录读过的孔位，验证共享答案键的实例读到相同答案 | R | F01/F14 一类错误命中的动态检测 |

## 规则验证

标量语义只写**一次**，同时生成两样东西（推断，依 SpecTec 的“单一事实来源”与 Crocus 的教训）：

| 产物 | 用途 |
|---|---|
| 解释器里的 `eval_op(op, mode, args) -> Value \| Poison` | 执行 |
| SMT-LIB 编码 `(value, is_poison)` + UB 条件 | 规则证明：目标的 `(value, poison, ub)` refine 源（Alive2 定义） |

一致性检查：≤8 位宽度上对每个 op 穷举，比较两者。Crocus 的问题在于 CLIF 没有语义，规格写在规则旁边，规格之间的一致性靠人；kfrc 从第一天就有解释器，可以避免。

规则表每条规则需要的语义标志：`poison_producing`（结果可能 poison）、`introduces_ub_point`（在新位置使用值作分支条件/地址/除数/实参）、`speculates`（把节点移出原控制）、`float_bits`（是否依赖 NaN 选择）。后两项为真的规则需要额外前提（`noundef`、dereferenceable、硬件 NaN 一致）。

## 工作量估计（推断）

参照点：CompCert 每级 IR 的语义 300–600 行 Coq（CACM 2009）；kfrc 现有 `interpreter.rs`（单块 i64 SoN）597 行、`machine_eval.rs` 882 行；Graal 形式化的数据/控制/跨方法三层在 Isabelle 中可执行。

| 组件 | 行数 | 人周 |
|---|---|---|
| 共享核心：`Value`、`AbstractByte`、分配与 provenance、`encode`/`decode`、标量运算 | 2,500 | 4 |
| Body 解释器（Place、终结指令、drop、unwind） | 2,000 | 3 |
| R/S 图解释器：控制骨架、按需求值、phi、调用、答案 oracle | 2,500 | 4 |
| L 扩展：`MachineTy`、`ValueSlot`、tag/niche 算术、与 S 锁步 | 1,000 | 1.5 |
| 检查模式：分区归属、token 新鲜度、`Refine`、答案审计 | 1,500 | 3 |
| 标量 SMT 编码 + 穷举一致性 | 1,000 | 2 |
| 语义文档 `design/ir/semantics.md` | 约 300 行 | 1.5 |
| **合计** | **约 1.05 万行** | **约 19 人周（4–5 人月）** |

对比：CompCert 约 3 人年、CakeML 约 10 万行 HOL4 证明，代价主要在证明；kfrc 只做可执行语义，不做证明。

## 首版范围（与 M2 的 T17 同期）

M2 的 T17 同时引入聚合值、`lower_layout` 和只有 `Unknown` 的分区字段（[m2](../../plan/m2-types-memory-abi.md#t17)），首版语义随之覆盖三级，但只到 M2 的节点集合：

| 包含 | 不包含（后续阶段） |
|---|---|
| 整数、`bool`、`char`、聚合、enum 变体操作、瘦/宽指针、union 字节值 | 浮点 NaN 集合语义（首版按位比较，含 NaN 的用例标记跳过） |
| `Unknown`/`Const` 分区，token 新鲜度断言 | `Local`/`Heap` 归属检查（M3，与分区同期）；`Frozen`/`Unique`/内联 `Reborrow`（M6，与开关同期） |
| 固定 `Load`/`Store`、`Load.spec`、`Add.nsw` 的 poison 与 UB 点 | 原子、fence、并发 |
| 直接/间接调用、`Check` → panic（终止即可观察结果） | unwind、`Landing`/`Resume`（M3，与 `CallUnwind` 同期） |
| R 与 S 级、答案 oracle、R→S 相等检查；S→L 锁步 | SMT 规则证明（规则表定型后） |
| 每 pass 差分 + bisect（debug 构建） | Body 解释器的 drop/借用部分（M3） |

首版约 4,000–5,000 行、7–9 人周（推断）。现有 `interpreter.rs` 只能作起点：它是“单块 i64”，值域与控制模型都要换。

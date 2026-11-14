# 第一批要求迁移：验证准备

2026-11-09。唯一语义契约是 `docs/design/ir/requirements-and-facts.md`；本文件只登记验证入口和证据要求，不另定契约。实现 ownership 与冻结顺序见 `docs/plan/poc/requirements-migration.md`。

旧协议 candidate11 已完成回执。所有 owner ready 后才运行集成编译；新候选必须独立冻结，不能覆盖 candidate11。

## 执行结果分类

| 源执行 | 发布 L 的比较要求 |
| --- | --- |
| 正常且有定义 | 返回值、外部调用轨迹、内存结果、终止行为全部匹配 |
| 有明确依据的 `PoisonObserved` 或 `UndefinedBehaviour` | 按 S→L refinement 可接受目标正常或 UB；记录源错误类别与触发操作 |
| `Malformed`，包括 Proven 失败 | 编译器错误；必须失败，不能算源 UB |
| `UnknownCallee` / `OutOfFuel` | 证据不足；必须失败，不能算源 UB |
| `Unreachable` | 当前 outcome 同时表示 UB 或 divergence，不自动归入源 UB；默认严格比较，仅有明确 fixture 依据才细分 |

R→S 纯代入以及承诺诊断保持的 R/S 优化继续严格对拍，不使用发布 L 的宽容关系。不得仅凭两个错误携带相同 NodeId 判断语义来源相同。分类辅助由 root 维护；观察布局测试由 layout owner 维护。

## 第一批回归入口

| 范围 | 已有入口／owner | 应保留的断言 |
| --- | --- | --- |
| 源签名与 Ignore | `semantic_build.rs` / builder | Start 参数、direct/indirect Call `.2`、Return 保留原 Rust 值；Never 无正常 continuation，但调用效果保留 |
| 纯特化 | `semantic_specialize.rs` / generic | R/S Ignore enum、ZST、poison 的源码值和诊断一致；不能因 ABI Ignore 重新构造 variant |
| 内联与 identity | `semantic_inline.rs`、`semantic_identity.rs` / optimizations | 删除调用边界时保留 RequireDefined 的源执行顺序；原业务 SSA 不替换成要求输出 |
| 活动与非活动 payload | `semantic_observation_layout.rs` / layout | 有定义输入强对拍；活动 poison 留在源诊断；inactive poison 不成为 whole-value UB |
| 原始存储 | `semantic_maybe_uninit*.rs` 及 features fixture | MU 未初始化 mask、padding、raw invalid bool 合法；typed decode 按既有规则判断 |
| owned snapshot | `semantic_inline_memory.rs`、`semantic_memory_optimize.rs` | 别名写入后仍使用先前快照；不能因擦除要求将真实读取延后 |
| 真实控制与效果 | 观察布局／inline tests | 真正分支、panic、调用与必要存储不能随要求一起删除；循环下一次 poison 不复用前次证明 |
| 结构要求 | 观察布局 / layout | 冗余 RequireDefined 不增加发布分支、load/store/搬运；证明独占引用不要求 ABI payload 物化 |

第二批 Witness/Assume/placement 尚未实现，不能把第一批通过写成第二批验收。旧 Refine 相关负例仍执行，Proven 失败仍作为编译器错误。

## 集中验证和证据

1. 全 owner ready 后先统一编译和 workspace tests；首次同类错误停止重跑，交对应 owner 静态排查同类实例并批量修复。
2. 冻结新源码 tar、manifest、SHA；Linux E2B 构建后执行 feature 三臂、semantic 三臂、CLI、原 semver 34 tests/arm 和 workload、完整旧语料。
3. 原 W01–10 与 regex 保持原输入。已知 f64/NaN、atomic 和 caller-location 缺口单独保留，不能用缩小源码的替代结果验收。
4. 正确性通过后按已有同机同 sysroot 配置进行 5 轮交替编译、预热后 7 轮运行；计数另跑 3 轮，保留 enabled/running 和机器码透明性检查。
5. 记录 from_str S/L、If/Load/Store、实际 mov、text、完整 codegen 指令和 runtime。RequireDefined 消失不是性能结论；candidate11 parse 回退证据保留。

E2B 原始执行的 defined 输入结果仍强断言一致；仅修改具体源 UB 的解释器发布比较，不全局放宽 native fixture 输出或退出状态。

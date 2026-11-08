# 风险：依赖 `ra-ap-*`、照搬 RA 设计、许可与来源

## 依赖 `ra-ap-rustc_*`

| 风险 | 事实（实测/源码） | 对策 |
|---|---|---|
| 发布节奏快 | `ra-ap-rustc_type_ir` 过去 12 个月发了 45 个版本；2026-11-01 到 11-08 就有 0.180–0.184 五个 | 不跟发布；钉一个精确版本（`=0.148.0`），只在 kfrc 升级参考 rustc 版本时一起换 |
| 版本号与 rustc 无关 | 发布包没有 `.cargo_vcs_info.json`，无法从包反查 commit | 用源码 diff 定位：把候选版本的 `src/` 与官方 rustc 源码树逐文件比较，diff 为 0 的即对应版本（本次结果见 [components.md](components.md#实测哪个-ra-ap-版本等于-rustc-1941)），记录进来源清单 |
| API 不稳定 | `Interner` 方法 1.94.1 → 0.184.0 增删 51 处；`rustc_next_trait_solver` diff 5,591 行；RA 自己停在 0.166.0 | 适配层集中在一个模块；升级时以“适配层改动行数”作为成本指标；不承诺跟随 |
| 钉死版本不再有修复 | 旧版本不会补丁发布；RA 已不用 0.148 | 参考 rustc 1.94.1 本身也冻结；求解器的已知问题按 1.94.1 行为对拍即可，不需要修复 |
| 与 rustc 1.94.1 行为不同 | 0.148.0 源码相同，差异只可能来自：①关闭的特性（`rustc_abi` 无 `ty` 模块、`rustc_pattern_analysis` 无 `rustc` 模块）；②kfrc 自写的适配层（`Interner`、`PatCx`、`LayoutCalculator` 的输入）；③rustc 1.94.1 默认用旧求解器 | ②是主要风险，靠 oracle 对拍；③按 `solver-divergence` 单列（[T12](../../plan/m1/t12-trait-experiment.md)） |
| 依赖闭包 | next-solver 闭包 28 个 crate，含 4 个过程宏 crate（`rustc_type_ir_macros`、`rustc_index_macros`、`derive-where`、`tracing-attributes`）与 `syn` 2/3 | kfrc 自身用 stock rustc 构建，不影响目标；若将来要自举，过程宏是前提（推断） |
| 发布方不是 rust-lang 发版流程 | 由 RA 团队的 rustc-auto-publish 脚本发布 | 每次换版本都做一次源码 diff，确认与官方 tag 一致 |

## 照搬 RA 设计的风险

RA 为“编辑中的残缺代码 + 只算被问到的东西”设计；编译器要“对完整输入给出完整、精确的接受/拒绝”。下面这些在 RA 里是正确的工程选择，在 kfrc 里是错误：

| RA 做法 | 位置 | kfrc 的规则 |
|---|---|---|
| 可恢复断言：`never!`/`always!` 在 release 下只返回条件值，“终止比继续更糟时用” | [stdx/src/assert.rs](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/stdx/src/assert.rs#L1) | 内部不变量失败是一类错误，必须停止并报告（[ids.md](../../design/ids.md#错误分类)） |
| 不动点超限后静默退出；未解析属性宏当作不存在 | [collector.rs L420–L500](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres/collector.rs#L420) | force 模式报错；红线 1 |
| 不报歧义、重复定义 | `DefDiagnosticKind` | 按 rustc 报 E0659/E0428/E0252 |
| 别的函数体里的 impl 不可见 | [method_resolution.rs L600](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/method_resolution.rs#L600) | 全局可见，进 coherence |
| 内建 derive 不展开 | [builtin_derive.rs](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/builtin_derive.rs#L1) | 生成与 rustc 展开一致的函数体 |
| 环给回退值、类型错误降成 error 类型继续 | [infer.rs L187](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/infer.rs#L187) | 报具体错误；错误恢复值不进入 Body 之后的阶段 |
| 不做借用检查；常量求值是自有解释器 | `hir-ty/src/mir/` | 借用检查与 CTFE 以 rustc 为准 |
| trait 一律走 next-solver | `next_solver/` | 默认 oracle 是旧求解器 |
| 按需计算：只算被打开文件需要的查询 | Salsa 拉取模型 | 每个定义都要检查，诊断完整 |

**内存与身份模型。** RA 的 ID 是 Salsa 驻留号，类型带 `'db` 生命周期并按需 transmute，派生数据靠 LRU 丢弃、随时重算。kfrc 的会话 ID 只在一次编译内有效，跨任务、跨进程一律用持久键（[ids.md](../../design/ids.md#规则)），产物发布后不可变。借 RA 的数据切分时，要把每个“会话内驻留身份”换成“可哈希的持久键”，不能把 RA 的身份层原样搬来。

**阅读时的误导。** RA 很多文件的头注释写“port of rustc …”，但移植基线是旧 commit（例如 `infer.rs` 链接的是 `5503df87`，`mbe` 匹配器链接的是 `70b18bc2`），之后各自演化。读 RA 只能得到结构，规则要回到 rustc 1.94.1 核对。

## 许可与来源记录

- **RA：** workspace 许可 `MIT OR Apache-2.0`。部分文件本身是派生的：`stdx/src/assert.rs` 来自 matklad/always-assert，`mbe` 匹配器、`hir-ty` 的推导与 `next_solver/infer` 来自 rustc。复制 RA 代码等于引入双重来源。本计划的原则是**不复制 RA 代码**，只作设计参考。
- **`ra-ap-rustc_*`：** crates.io 元数据为 `MIT / Apache-2.0`；源码就是 rustc `compiler/` 下对应目录。若直接依赖，按 rustc 的 REUSE 注释逐文件确认（参考树许可规则见 [T00](../../plan/m0/t00-freeze-inputs.md)）。
- **记录方式（建议）：** 在 T00 的 `docs/plan/inputs/provenance/` 中：
  - 依赖 crate：名称、精确版本、对应的 rustc commit 与“src/ diff 为 0”的证据、启用的特性、适配层文件。
  - 设计参考：来源仓库与 commit（`rust-lang/rust-analyzer@03ae6be5`）、读过的文件、采用方式“仅设计参考”、借用的结构名（如“按调用位置驻留的展开身份”）、明确未采用的行为。
  - 红线 7（移植 rustc 代码时记录来源）建议扩到“依赖 rustc crate 时同样记录”。

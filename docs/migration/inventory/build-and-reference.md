# 构建、缓存、证据与 LLVM 参考路线

全部落到 `src/build/`。“历史隔离”表示归入 reference 职责，native 生产路径不调用。

## 编排与调度

| 文件 | 处置 | 切换条件 |
|---|---|---|
| [native_cli.rs](../../../src/native_cli.rs) | 替换编排 | CLI 只负责参数和诊断；生产链使用 CheckedBody/实例任务；旧 `.kfr` 输入显式区分 |
| [native_driver.rs](../../../src/native_driver.rs) | 保留链接/工具调用并扩展 | 工具身份、参数、目标固定；汇编调试输出与直接 ELF 路径分别记录 |
| [native_batch.rs](../../../src/native_batch.rs) | 替换任务输入，保留文件批处理兼容入口 | 实例任务与完整程序任务分开；每阶段 worker 数不叠加突破全局预算 |
| [parallel_native.rs](../../../src/parallel_native.rs) | 保留执行机制并适配 | 从 InstanceKey 接任务，受统一预算限制，错误和产物顺序稳定 |
| [pipelined_native.rs](../../../src/pipelined_native.rs) | 替换阶段输入，复用有界执行 | 旧的 源码 → Graph 流水线不能原样套到新语义流水线 |
| [pipeline.rs](../../../src/pipeline.rs) | 保留有界队列/回压/取消并扩展 | 同时限制任务数和驻留图内存；失败释放资源 |
| [scheduler.rs](../../../src/scheduler.rs) | 保留并扩展 | 固定 DAG → 显式依赖任务；语言循环按语义规则解决 |
| [system.rs](../../../src/system.rs) | 保留并扩展 | host/target、有效 CPU 配额、内存、工具链身份分开，不只看逻辑核数 |

## 缓存

| 文件 | 处置 | 切换条件 |
|---|---|---|
| [native_cache.rs](../../../src/native_cache.rs) | 替换键/格式，保留可验证 I/O | 新 namespace/schema；实例、类型/布局、callee、配置、target 失效契约先完成 |
| [cache_admin.rs](../../../src/cache_admin.rs) | 保留管理安全行为并扩展 | 统计/清理识别 namespace 与版本；保留“不跟随符号链接、不删除根目录” |
| [cache.rs](../../../src/cache.rs) | 历史隔离 | 现有 exact rustc invocation key 不作 native 语义键 |

## 证据与测量

| 文件 | 处置 | 切换条件 |
|---|---|---|
| [artifact.rs](../../../src/artifact.rs) | 保留证据采集并扩展 | 内容身份、生产者、配置、加载节/文件大小分别记录；旧 hash 不当语义键 |
| [audit.rs](../../../src/audit.rs) | 抽取共用记录；rustc 参数审计历史隔离 | 任务/产物来源记录与旧 subprocess argv 记录分型 |
| [benchmark.rs](../../../src/benchmark.rs) | 保留运行器并扩展 | 实施新的 [性能协议](../validation/performance.md) |
| [repro.rs](../../../src/repro.rs) | 替换清单 schema，保留可复现输出目标 | 记录 源 → 实例 → 对象 身份；明确当前单函数 ELF 假设；新旧清单分版本 |

## rustc/LLVM 参考路线（历史隔离）

| 文件 | 处置 | 切换条件 |
|---|---|---|
| [cargo.rs](../../../src/cargo.rs) | 历史隔离；自有 Cargo 协议重写 | 双 wrapper 行为不迁入新路径，见 [cargo-sysroot](../architecture/cargo-sysroot.md) |
| [rustc.rs](../../../src/rustc.rs) | 历史隔离，作 oracle | native 生产调用图不访问；选 oracle 与生成目标对象分开记录 |
| [llvm.rs](../../../src/llvm.rs) | 历史隔离 | LLVM 参数不作自有优化策略；旧 profile 不覆盖 native 的 panic/target 语义 |
| [pgo.rs](../../../src/pgo.rs) | 历史隔离 | LLVM profile 工具仅用于对照；未来 native PGO 用独立格式 |
| [pgo_driver.rs](../../../src/pgo_driver.rs) | 历史隔离 | 旧 instrumentation/training/use 路径不进 native driver |
| [tuning.rs](../../../src/tuning.rs) | 隔离 LLVM 候选模型；可提取测量选择逻辑 | 不把 LLVM CGU/LTO 参数当自有优化空间 |
| [tuning_runner.rs](../../../src/tuning_runner.rs) | 历史隔离；共用进程/证据机制可提取 | 参考构建与 native 构建各有产物清单 |

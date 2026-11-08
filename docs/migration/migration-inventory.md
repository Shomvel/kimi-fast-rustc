# 当前代码的迁移清单

状态：`planned`。清点基线为 `44c9d80b02ab140d7edf5348b6946d8122d842b6` 的 **46 个 `src/*.rs` 文件**。本 PR 没有移动或改写这些文件。拟建目录以 [架构契约](architecture.md) 为准；表中代码样式路径表示规划，不表示目录已经存在。

本清单用于每次派工确定文件所有权和调用者切换条件，不是批量 rename 指令。每个行为闭包须经过 [验证协议](validation.md) 后才能更新为 `verified`；新目录和旧文件短期共存不等于迁移完成。

## 处置类别与共用门禁

- **保留并扩展**：复用已验证实现，补齐新表示/目标能力；不能因为旧测试通过就跳过新契约验证。
- **替换生产路径**：新实现承担完整职责，旧入口在明确标签下保留到调用者迁移完毕。
- **历史隔离**：只服务参考编译器或旧实验，不得被 native 生产路径隐式调用。
- **合并后退役**：消除重复实现前，先证明新实现覆盖旧行为并迁完所有调用者和测试。

所有计划路径都属于职责目录，不预先决定 crate 划分、公开接口或最终文件名。涉及共享类型/ID、对象格式、预算和缓存的改动先更新架构契约，再改调用者。不要由各工作流分别创建含义不同的 `TypeId`、实例编号或 `ArtifactKey`。

## 全部源码文件映射

### 入口、语义与诊断辅助

| 当前文件 | 处置与规划落点 | 迁移前置和切换条件 |
|---|---|---|
| [lib.rs](../../src/lib.rs) | 保留并调整导出；各职责模块的入口 | 新旧能力显式分开；先检查仓库内和已知外部调用者，删除公开导出时记录兼容性变化 |
| [main.rs](../../src/main.rs) | 替换生产编排，委托 `src/build/`；保留 CLI 入口 | driver 契约和选择方式固定后逐命令切换，不让 legacy 失败触发另一后端 |
| [frontend.rs](../../src/frontend.rs) | 替换生产路径到 `src/syntax/`、`src/resolve/`、`src/semantics/`、`src/body/`；旧子集最终退役 | 新 AST/CheckedBody→SoN 全链完成；旧行为测试覆盖和显式入口迁移后再删除 |
| [mir.rs](../../src/mir.rs) | 隔离为旧文本测试输入；最终退役或保留专用测试入口 | 不能改名成 `src/body/` 就宣布完整 MIR 能力；测试改用新 Body 构造器后才可删除 |
| [config.rs](../../src/config.rs) | 替换配置模型到 `src/build/`，目标语义与其他模块共用契约 | 去除生产默认 LLVM 假设；panic/overflow/target 来自显式配置，不由 profile 偷改 |
| [interpreter.rs](../../src/interpreter.rs) | 保留旧图诊断用途；需要时扩展到 `src/son/` 内部测试支持 | 单 block i64 evaluator 不能充当新前端、borrowck 或实际机器执行的 oracle |
| [machine_eval.rs](../../src/machine_eval.rs) | 保留并适配到 `src/machine/` 内部诊断支持 | 扩展指令模型时与真实执行对照；不把两个共享 lowering 的模型一致视为独立正确性证据 |

### 图、优化、机器码与对象

| 当前文件 | 处置与规划落点 | 迁移前置和切换条件 |
|---|---|---|
| [son.rs](../../src/son.rs) | 保留设计并重构到 `src/son/` | 类型、效果、图内 ID、实例引用和 verifier 先固定；旧构造器显式适配 |
| [cfg.rs](../../src/cfg.rs) | 保留并适配到 `src/son/`，必要分析服务 `src/machine/` | 区分 Body CFG、图 placement CFG 与 Machine CFG；共享算法不等于共享可变表示 |
| [placement.rs](../../src/placement.rs) | 保留并扩展到 `src/son/` | 加入效果、异常边和新节点合法移动条件；完整前驱及支配验证通过再切换 |
| [optimizer_pipeline.rs](../../src/optimizer_pipeline.rs) | 替换编排、逐规则迁移到 `src/son/` | 每个 pass 有输入/输出不变量、预算和终止条件；不批量启用未经新类型验证的旧 pass |
| [son_optimize.rs](../../src/son_optimize.rs) | 保留算法并扩展到 `src/son/` | 按类型、溢出、trap/effect 前提迁移；启用/禁用均跑真实执行对拍 |
| [value_numbering.rs](../../src/value_numbering.rs) | 保留算法并扩展到 `src/son/` | key 包含类型及语义，代表节点满足支配与效果约束；纯值规则不推广为 load CSE |
| [loop_analysis.rs](../../src/loop_analysis.rs) | 保留并扩展到 `src/son/` | 多退出、异常和内存依赖验证齐备；保守 LICM 可用不代表任意循环优化安全 |
| [native.rs](../../src/native.rs) | 保留有效 lowering，拆入 `src/machine/` | 输入改为 VerifiedGraph 和独立 ABI；移除后端猜类型/符号身份的职责 |
| [x86.rs](../../src/x86.rs) | 保留并扩展到 `src/machine/` | 明确寄存器类别、宽度、通用地址、flags/call clobber；spill Load/Store 不当作用户内存支持 |
| [regalloc.rs](../../src/regalloc.rs) | 保留线性扫描并扩展到 `src/machine/` | 新寄存器类别、固定寄存器、调用保存与栈约束验收；不增加静态 ILP 调度 |
| [x86_encode.rs](../../src/x86_encode.rs) | 保留编码并扩展到 `src/machine/` | 新 opcode/宽度/重定位需编码和真实执行证据；分支缩短不破坏符号和 call offset |
| [elf.rs](../../src/elf.rs) | 合并到 `src/object/` 后退役重复写出器 | 先比较单函数对象和符号；统一 writer 覆盖旧输出要求后删旧分支 |
| [elf_reloc.rs](../../src/elf_reloc.rs) | 保留并扩展到 `src/object/` | 增加数据/TLS 等节区和重定位；重复/未定义符号、节对齐与链接均校验 |
| [call_reloc.rs](../../src/call_reloc.rs) | 合并到 `src/machine/` 调用降低及 `src/object/` 重定位后退役 | 零参数原型的样例迁为新 ABI 回归；不能与完整调用路径长期维护两套约定 |
| [multi_function.rs](../../src/multi_function.rs) | 合并到 `src/object/` 后退役窄原型路径 | 当前窄调用限制样例迁移；由统一实例/符号模型取代原型函数拼接 |
| [native_object.rs](../../src/native_object.rs) | 合并到 `src/object/` 后退役重复单函数路径 | 迁移单函数缓存、对象输出和调用者；移除旧限制前确认新 writer 的完整行为 |
| [native_program.rs](../../src/native_program.rs) | 扩展程序对象职责到 `src/object/`，身份来源接 `src/instantiate/` | 符号不依赖裸名字/声明顺序；对象、静态数据、调用与递归统一组装 |

### 构建、缓存、运行证据与历史路线

| 当前文件 | 处置与规划落点 | 迁移前置和切换条件 |
|---|---|---|
| [native_cli.rs](../../src/native_cli.rs) | 替换生产编排到 `src/build/` | CLI 负责参数和诊断，生产链使用 CheckedBody/实例任务；旧 `.kfr` 输入显式区分 |
| [native_driver.rs](../../src/native_driver.rs) | 保留链接/工具调用能力并扩展到 `src/build/` | 工具身份、参数和目标固定；汇编调试输出与直接 ELF 产品路径分别记录 |
| [native_batch.rs](../../src/native_batch.rs) | 替换任务输入到 `src/build/`，保留文件批处理兼容入口 | 实例任务和完整程序任务分清；避免旧每阶段 worker 数叠加突破全局预算 |
| [parallel_native.rs](../../src/parallel_native.rs) | 保留执行机制，适配到 `src/build/` | 从 InstanceKey 接任务，受统一 jobserver/内存预算限制，稳定错误及产物顺序 |
| [pipelined_native.rs](../../src/pipelined_native.rs) | 替换阶段输入到 `src/build/`，复用有界执行 | 源码→Graph 的旧流水线不能作为新语义流水线原样套用；阶段产物验证后发布 |
| [pipeline.rs](../../src/pipeline.rs) | 保留有界队列/回压/取消，扩展到 `src/build/` | 同时限制任务数与驻留图内存，避免每个 stage 独立扩张总 CPU；失败释放资源 |
| [scheduler.rs](../../src/scheduler.rs) | 保留并扩展到 `src/build/` | 固定 DAG 扩展为显式依赖任务；语言循环由语义规则解决，不让 worker 等自身 |
| [native_cache.rs](../../src/native_cache.rs) | 替换键/格式到 `src/build/`，保留可验证 I/O 机制 | 新 namespace/schema；完整实例、类型/布局、callee、配置和 target 失效契约先完成 |
| [cache_admin.rs](../../src/cache_admin.rs) | 保留管理安全行为并扩展到 `src/build/` | 统计/清理识别 namespace 与版本；保留不跟随符号链接、不删除根目录的行为 |
| [cache.rs](../../src/cache.rs) | 历史隔离于 `src/build/` 的 reference 职责 | 当前 exact rustc invocation key 不复用为 native 语义键；调用者仅限历史工具 |
| [artifact.rs](../../src/artifact.rs) | 保留证据采集，扩展到 `src/build/` | 内容身份、生产者、配置和加载节/文件大小分别记录；旧 hash 不自动当安全语义键 |
| [audit.rs](../../src/audit.rs) | 抽取共用记录到 `src/build/`，rustc 参数审计隔离为 reference | 任务/产物来源记录与旧 subprocess argv 记录分型；保持可重放而不混淆后端 |
| [benchmark.rs](../../src/benchmark.rs) | 保留运行器机制，扩展到 `src/build/` 的测量支持 | 实施新验证协议：功能等价、交错/随机采样、CPU/内存预算、原始记录 |
| [repro.rs](../../src/repro.rs) | 替换清单 schema 到 `src/build/`，保留可复现输出目标 | 记录源→实例→对象身份及版本，明确当前单函数 ELF 假设；新旧清单分版本 |
| [system.rs](../../src/system.rs) | 保留并扩展到 `src/build/` | host/target、有效 CPU 配额、内存及工具链身份分开，不能只用机器逻辑核数 |
| [cargo.rs](../../src/cargo.rs) | 当前 wrapper helper 历史隔离；自有 Cargo 协议在 `src/build/` 重写 | 新 driver/metadata/host-target 契约完成；双 wrapper 行为不能迁入新路径 |
| [rustc.rs](../../src/rustc.rs) | 历史隔离于 `src/build/` 的 reference/oracle 职责 | native 生产调用图不访问它；选择 oracle 与生成目标对象分开记录 |
| [llvm.rs](../../src/llvm.rs) | 历史隔离于 `src/build/` 的 reference 职责 | LLVM 参数不作为自有优化策略；旧 profile 不覆盖 native 的 panic/target 语义 |
| [pgo.rs](../../src/pgo.rs) | 历史隔离于 `src/build/` 的 reference 职责 | LLVM profile 工具仅用于对照；未来 native PGO 必须有独立格式及验证，不改名复用 |
| [pgo_driver.rs](../../src/pgo_driver.rs) | 历史隔离于 `src/build/` 的 reference 职责 | 旧 rustc instrumentation/training/use 路径不进入 native driver；证据标明生产者 |
| [tuning.rs](../../src/tuning.rs) | 隔离 LLVM candidate 模型；可提取测量选择逻辑到 `src/build/` | 不把 LLVM CGU/LTO 参数列表当自有优化空间；质量阈值采用统一协议 |
| [tuning_runner.rs](../../src/tuning_runner.rs) | 历史隔离；共用进程/证据机制可提取到 `src/build/` | 参考构建与 native 构建各有独立产物清单；共享工具不等于允许静默 fallback |

`reference` 表示 `src/build/` 内的明确历史职责，不要求现在建立固定子目录。隔离后不必立即删除历史代码；先移除 native 生产依赖，待没有可复现实验或调用者需要时再决定退役。

## 测试资产如何迁移

现有单元测试分布在源文件内部，integration tests 在 [tests](../../tests)。源码迁移时需同时登记内部测试；只移动 `tests/*.rs` 会漏掉大量编码、缓存和调度验证。以下按行为分组，所列文件是代表样本，不是允许删除其余测试的清单。

| 测试组与代表文件 | 迁移目的地/方式 | 不能省略的验收 |
|---|---|---|
| 语法/类型：[integer_syntax.rs](../../tests/integer_syntax.rs)、[casts.rs](../../tests/casts.rs)、[const_items.rs](../../tests/const_items.rs)、[function_visibility.rs](../../tests/function_visibility.rs) | 转为新入口源码接受/拒绝对拍；保留 legacy 结果单列 | edition、宽度、常量错误、作用域和可见性；“解析成功”不算类型支持 |
| 有状态控制流：[shadowing.rs](../../tests/shadowing.rs)、[loop_values.rs](../../tests/loop_values.rs)、[statement_if.rs](../../tests/statement_if.rs)、[match_ranges.rs](../../tests/match_ranges.rs)、[early_return.rs](../../tests/early_return.rs) | 新 Body→SoN 的永久语义回归；补赋值/调用/drop 组合 | stock rustc 与真实 x86 执行；false/零迭代等不执行分支也要覆盖 |
| 调用/ABI：[function_calls.rs](../../tests/function_calls.rs)、[forward_calls.rs](../../tests/forward_calls.rs)、[stack_arg_encoding.rs](../../tests/stack_arg_encoding.rs)、[multi_function.rs](../../tests/multi_function.rs) | 统一新实例、符号与 ABI 测试，旧窄原型样例保留来源 | 参数/返回、递归、callee-save、跨 crate/FFI；旧零参数样例不能证明完整 ABI |
| 图/优化：[bool_optimizer_regression.rs](../../tests/bool_optimizer_regression.rs)、[div_differential.rs](../../tests/div_differential.rs)、[loop_invariant.rs](../../tests/loop_invariant.rs)、[control_optimization.rs](../../tests/control_optimization.rs) | 按新类型/效果前提逐 pass 迁移；保留最小反例与图 dump | 启用/禁用优化均正确；trap、overflow、内存与控制效果不能被纯值简化抹除 |
| 编码/寄存器：[immediate_encoding.rs](../../tests/immediate_encoding.rs)、[short_jumps.rs](../../tests/short_jumps.rs)、[phi_moves.rs](../../tests/phi_moves.rs)、[regalloc_preference.rs](../../tests/regalloc_preference.rs) | 保留精确编码/约束测试，适配 Machine IR | 精确字节加真实执行；新类别、spill、固定寄存器和 flags 组合 |
| 产物/并行：[direct_control_object.rs](../../tests/direct_control_object.rs)、[function_parallel.rs](../../tests/function_parallel.rs)、[native_pipeline.rs](../../tests/native_pipeline.rs)、[repro.rs](../../tests/repro.rs) | 扩展到新任务/对象发布和缓存命名空间 | jobs=1/多 jobs、失败/取消/重复输入、原子发布、确定性和失效 |
| 模型与 CLI：[machine_differential.rs](../../tests/machine_differential.rs)、[cli_help.rs](../../tests/cli_help.rs) | 模型保留定位作用；CLI 明确新旧入口和能力标签 | 不把模型一致当最终正确性；未知特性报错并保持无 fallback |
| 编译耗时：[function_graph_performance.rs](../../tests/function_graph_performance.rs) 及源内局部时序 | 保留可解释微基准，和整项目测量分开 | 局部 pass 速度不算完整编译速度；正式计时使用冻结环境和原始样本 |

## 脚本、示例与文档

- [scripts/native_microbench.py](../../scripts/native_microbench.py)：保留当前子集微基准标签；新链覆盖同语义输入后增加显式 frontend/driver 选择、版本和缓存模式记录，不能原地覆盖历史结果后称全 Rust benchmark。
- [scripts/bench-yicloud.sh](../../scripts/bench-yicloud.sh)：保持 LLVM reference 说明；正式 native 项目测量采用新的冻结配置和验证协议。历史主机/工具路径不是新环境的默认事实。
- [examples](../../examples)：`.kfr` 与 `fn main() -> i64` 等测试约定保留显式标识；新稳定 Rust 例子必须可由 oracle 编译。迁移样例先保留输入来源和预期结果，再更新文档入口，不能用修改样例语义掩盖不兼容。
- [README.md](../../README.md)：每阶段只公布已验证能力，新入口在显式选择下稳定后才切默认；历史 wrapper 和 legacy native 的命令明确分类。
- [docs/ARCHITECTURE.md](../ARCHITECTURE.md)、[docs/VALIDATION.md](../VALIDATION.md)：当前事实和历史证据保持可追溯；新架构实施后按版本补充，不追认旧数据为新链结果。
- [docs/research](../research)：IR、codegen、benchmark 调研保留来源和适用范围；本 [迁移目录](README.md) 的状态随实施证据更新，不能由计划文字替换验证记录。

## 调用者切换、缓存隔离与退役顺序

1. **登记调用者**：按入口、库导出、内部函数、测试和脚本搜索引用；记录迁移范围及兼容性影响。共享旧函数同时服务多条链时先拆开身份/配置，不直接删除。
2. **增加显式新路径**：使用冻结的选择方式，不在本清单另造 CLI 开关名。新路径不支持输入时报告能力缺口；legacy 或 oracle 只由用户/测试显式选择。
3. **隔离产物**：旧 Graph cache、新语义产物 cache、LLVM reference 产物分别使用 namespace、schema 和输出目录。不能仅改目录名后读取旧键；要验证生产者、类型/布局、callee、target、配置和实现版本。
4. **迁移行为与证据**：从源码到真实执行对照，旧入口与新入口的测量单列；测试重写后仍保存原始错误目的。对象字节改变可以合法，但行为、ABI 和声明的确定性要求必须成立。
5. **切换默认调用者**：满足对应阶段门禁后逐调用者切换；取消、错误、缓存 miss 和 cold build 与成功路径一并验证。不要在整仓未完成时把 reference 产物计为 native 覆盖。
6. **移除旧实现**：无生产依赖、必要外部使用者有迁移说明、旧测试语义均有新归属、参考实验仍可重放，才可删除旧文件或公开导出；临时兼容入口须有明确退出条件。
7. **保留回退能力**：回退到上一版已验证的同语义实现或禁用有问题的优化；不能通过改变 panic/overflow/features 或自动调用 rustc 通过验收。缓存有疑问时重新编译，不相信不完整命中。

迁移完成记录应包含旧文件/符号、新职责、任务 ID、调用者清单、测试去向、原始验证证据、来源许可、退役条件和当前状态。整张表不会因为目录重排一次性变为 `verified`。

## 清单的执行责任

- 集成人负责入口、公开导出及跨职责契约；不能把共享文件同时交给两个未协调的实现任务。
- 前端语义流负责 parser/Body 的切换与接受/拒绝矩阵；后端流负责图、ABI、机器码和对象行为。
- 构建流负责任务预算、取消、发布及缓存 schema；验证流负责保存可重放的原始证据。
- 一个旧文件分散到多个目标职责时，先由一个任务拆出行为接口，再把互不重叠的实现交给其他任务。
- 每次新增或退役源码文件都重新运行清单覆盖检查；遗漏项先记为 `planned`，不能从覆盖统计中静默删除。

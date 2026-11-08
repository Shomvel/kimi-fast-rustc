# 前端与语义迁移工作流

状态：`planned`，尚未迁移代码。此工作流把 rustc 中成熟的语义规则和算法改造成自有实现，输出自有 Typed Body，再交给 SoN。目标不是把 rustc 私有查询系统嵌入产品。总体契约见 [架构](../architecture.md)，来源与许可要求见 [rustc 来源映射](../rustc-source-map.md)。

## 当前实现与迁移起点

当前 [frontend.rs](../../../src/frontend.rs) 的 `parse` 直接返回 `Graph`；同一个 `Parser` 持有词法 token、局部绑定、`ScalarType`、常量求值状态、函数签名和 SoN 控制状态。`ScalarType` 只有 `I64/U64/Bool/Unit`。该路径已经支持不少标量语法，但无法作为完整 Rust 语义层逐步叠加。

[mir.rs](../../../src/mir.rs) 是单 basic block、i64、赋值一次的文本输入解析器，不是 rustc MIR 适配器，也不是本计划中的 Typed Body。迁移时保留其测试用途，不把同名当作已有语义能力。

现有 `if`、`match`、短路与循环各自维护绑定状态。新路径必须把变量赋值和控制流先表示在 Body 中，在独立 SoN builder 内统一构造 SSA；不能继续让每个语法分支各自拼 Phi。

## 产物和发布契约

以下名称是逻辑产物，不是本 PR 已实现的 Rust 类型。实现名称与字段由核心接口工作包冻结，避免不同 agent 创建相互竞争的 `TypeId` 或 `DefId`。

| 产物 | 必需输入 | 对下游的保证 | 尚不保证 |
|---|---|---|---|
| ParsedModule | 源文件、edition、源身份 | token/AST、精确来源范围、解析错误 | 名字含义、类型、宏完成展开 |
| ExpandedModule | token tree、宏定义、hygiene、cfg、展开依赖 | 本轮可解析的定义和展开来源 | 未解决导入/宏依赖已稳定 |
| ResolvedInterface | 定义表、命名空间、可见性、稳定展开结果 | 路径映射到定义身份，接口依赖可枚举 | 所有函数体已检查、泛型体已可用 |
| TypedBody | 已解析接口、类型约束、trait obligations | 每个表达式/Place 有类型，控制流与检查点显式 | 所有权合法、drop 路径已展开 |
| CheckedBody | TypedBody、move/loan/region 分析、drop 需求 | 受支持语义的 move/borrow 约束成立，初始化和退出路径明确 | 对未支持语言特性作隐式接受 |
| InstantiatedBody | CheckedBody、具体泛型参数、实例环境 | 类型和调用目标在约定范围内具体化，可达依赖已登记 | 全部被调用函数机器码已完成 |

所有产物带状态（成功、错误、等待依赖）、依赖指纹、目标/edition 配置和来源。只有成功且依赖已满足的产物可以供缓存命中使用。诊断位置不是定义身份；宏 hygiene 又不能随诊断 span 一起丢掉。

调用和递归实例先登记声明与类型，再构造函数体。任务可返回“依赖未就绪”，但不可占着全部 worker 同步等待另一组相互依赖任务。遇到定义循环、常量循环、trait 循环必须执行各自语义规则；不能用一个通用 SCC 算法替代所有语言规则。

## 逐批语义闭包

每一批都交付输入程序、拒绝程序和可执行结果；仅 AST 能解析或函数体能生成不算完成。M1–M5 与总 roadmap 的阶段含义一致。

| 批次 | 范围及前置条件 | 实现工作 | 出口证据 |
|---|---|---|---|
| F0 / M0 | 冻结 oracle、edition、x86_64 目标、受支持特性表 | 确定 ID/type/Body 词汇；登记现有控制流回归；选上游源码 pin | 同一源码和配置可重复运行；支持/拒绝/待实现三态表可追溯 |
| F1 / M1 | F0；不依赖宏和泛型 | 成熟 lexer/token tree/表达式优先级适配；AST；函数、词法作用域、简单多模块与显式路径；标量类型检查；Body 构造 | 普通 `.rs` 经新路径执行；前向调用、遮蔽、分支赋值、循环退出与 rustc 一致；未定义/重复/非法赋值被拒绝 |
| F2 / M2 | F1 的稳定 Place/type 模型 | 整数宽度、浮点、tuple/struct/enum、数组/slice、引用/指针、投影、模式绑定；布局与 ABI 契约；基础初始化/move 与受限借用检查 | 只对已检查安全子集开放执行；聚合传参、读写别名、对齐、enum 分支、FFI 对拍；不得等 M3 才检查已经接受的引用程序 |
| F3a / M3 | F2；可回滚推导事务和统一类型关系 | 泛型参数与 substitutions、trait obligations、关联类型、方法查找、coherence、对象安全/动态分发逐项开放 | 有约束泛型与拒绝例通过；递归 goal、有歧义候选、关联类型规范化不会错判成功 |
| F3b / M3 | F2 的控制流、Place、move path；与 F3a 协同 | loan/region、重借用与生命周期、分支初始化、部分 move、drop flags、析构与 unwind 清理路径；所需 closure/捕获规则 | accepted/rejected 与冻结 rustc 一致；析构顺序、早退、循环、panic 下可观测行为一致 |
| F3c / M3 | F1 的 token tree/hygiene 基础；与名字解析迭代 | `macro_rules!` 匹配/转录、重复与 fragment；宏可见性；属性/cfg；声明式宏依赖闭包 | 同名局部/宏捕获、嵌套展开、跨模块导入、edition 差异、递归限制都有正反例 |
| F3d / M3 | type/layout/Body/初始化有效性；按需增量展开 | const-eval 解释器、受支持内建项、常量内存/指针、求值有效性；常量泛型需求 | 常量与运行时等价的有效例对拍；溢出、越界、循环依赖和无效值按 oracle 拒绝 |
| F4 / M4 | 前述语义达到冻结 sysroot 需求 | 过程宏 host 协议、编译器内建属性/lang items、core/alloc/std 所需 closure、async/coroutine 等缺口；实例和自有 metadata | 必需特性逐个销项；目标依赖闭包生产者清单完整；不能把 `Vec` 样例成功写成 `std` 已支持 |
| F5 / M5 | M4 及真实项目冻结依赖图 | 依实际需求补齐 target-feature、intrinsics、SIMD/atomics/TLS、panic/unwind、unsafe 检查等；每个缺口有负责工作包 | ripgrep/Cargo 不修改业务源、不减少 features，通过功能和行为对拍；所有未支持项显式诊断 |

F3a–F3d 是可协调的职责流，不是四个互不相干的实现：const 泛型、模式、drop 和 trait 结果共享类型与目标布局。过程宏可以早期做 host 协议实验，但不能在 hygiene/扩展语义尚未定义时宣布完成。

## Typed Body 与 SoN 交接

Body 至少表达：基本块、局部变量、`Place` 及 field/index/deref 投影、copy/move 操作数、赋值、借用、调用、分支、return、drop、assert、失败/清理边及作用域退出。保存每个操作对应的来源和已解析类型。

初版先以明确 CFG 保存局部读写，避免同时在 Body 和 parser 中实现两套 SSA。SoN builder 接收已验证 Body，为所有前驱统一构造值和效果合并；它不能重新决定名字解析、隐式转换、借用是否合法或析构何时发生。循环的前驱/回边以及不可达退出必须在图封口前确认。

类型检查阶段记录隐式借用、解引用、数值转换和方法调用选择；降低阶段执行这些决定，不再次猜测。跨阶段数据必须区分“未推导”与“错误类型”；错误恢复值不允许进入正式 codegen。

泛型 Body 可在参数环境下完成类型和借用分析，但实例化仍需具体 substitution、布局、调用解析、drop glue/vtable 需求；不要求把所有泛型定义复制为语法树重新检查。实例缓存键必须包含实际依赖，不能只用函数名和类型字符串。

## 首轮两个高风险移植实验

### 有约束泛型与关联类型

输入闭包：一个 trait、一个关联类型、两个候选 impl、一个 `where` 约束泛型函数及跨函数调用。先记录 rustc 对有效、无 impl、歧义/不满足约束三类程序的结果；设计 canonical goal 和推导变量驻留/回滚，再移植求解步骤。

交付：自有类型/约束结构、候选筛选与确定结果的最小实现、递归/缓存策略说明、依赖清单和运行/拒绝样例。不以依赖 `rustc_private` 的实验程序作为产品实现，也不把这个闭包等同完整 trait solver。高阶约束、opaque type、specialization 等未覆盖项必须明示。

### move、借用与分支 drop

输入闭包：结构体字段读取、可变借用和重借用、分支部分 move、早退与一次析构副作用。先建立 Body 的 Place/move path、初始化状态及 loan 事实，再做分析和 drop 显式化。

首轮分析实验交付：有效/无效程序判定、显式析构 CFG、对异常/不支持路径的处理契约及移植成本清单。分析原型的完成不代表生产语义已验证；接入生产路径前必须补齐析构次数和顺序的真实执行证据，见 T22/T27/T26 组合出口。若为了通过样例而直接在 parser 中插析构或修改绑定，视为实验未通过。

这两项实验用于验证移植粒度、接口和成本，不设“几天完成完整语义”的承诺。若移植持续要求把 rustc 全局 `TyCtxt`、arena 和 query providers 带入产品，先停在依赖分析结果，缩小闭包并修改自有模型。

## 现有文件与测试迁移

| 现有文件/测试 | 迁移处理 | 删除或切换条件 |
|---|---|---|
| [frontend.rs](../../../src/frontend.rs) | 冻结为 legacy 子集入口；将已验证词法和语义样例转成新路径测试；生产实现放入拟议 syntax/resolve/semantics/body 模块，类型驻留属于 semantics | 新路径覆盖其已声明行为并有端到端证据后切默认；legacy 暂留显式入口，不能在失败时自动 fallback |
| [mir.rs](../../../src/mir.rs) | 保留小型文本测试入口，文档注明职责；新 Body 使用独立名称和结构 | 使用者迁移完成才决定删除；不依赖解析 rustc 文本 MIR |
| [son.rs](../../../src/son.rs) | NodeId 限定为图内身份；定义/类型/实例 ID 独立；FunctionMetadata 按新契约演进 | 所有旧图输入显式适配完成，优化规则验证通过 |
| [multi_function.rs](../../../src/multi_function.rs)、[parallel_native.rs](../../../src/parallel_native.rs) | 从新实例产物接入，不再通过源码声明位置推导跨模块语义 | 新旧测试各自明确入口，输出来源可追溯 |
| [const_expressions.rs](../../../tests/const_expressions.rs)、[const_items.rs](../../../tests/const_items.rs)、[casts.rs](../../../tests/casts.rs) | 迁移为源码级对拍，补目标宽度和错误边界；保留单元测试辅助定位 | 同一测试不能只检查相同实现的 interpreter 与 native evaluator |
| [shadowing.rs](../../../tests/shadowing.rs)、[loop_values.rs](../../../tests/loop_values.rs)、[early_return.rs](../../../tests/early_return.rs) | 组合分支赋值、短路、循环退出和调用；作为新 Body→SoN 验收用例 | 新路径真实机器执行结果与 stock rustc 一致 |

## 语义工作流的完成定义

每个迁移 PR 说明采用的 rustc commit/文件/规则、自己的实现接口、改变的语义范围、正例和拒绝例、端到端验证命令及原始结果。源码接受、类型检查、所有权检查、代码生成和执行分别计覆盖，不能合为一个“支持 Rust 特性”勾选项。

首次差分出现某类错误后停止重复压测，静态追踪该类操作在解析、Body、SoN 和机器码的所有路径，成组修复后再继续。不会用放宽类型检查、忽略 drop 或替换 panic 策略消除失败。工作流完成以真实程序正确和无效程序正确拒绝为依据，不以新增文件数或测试数量为依据。

# M3：Rust 语义闭包

**目标：** 泛型与 trait、生命周期与借用、drop、声明式宏、const 求值、单态化，以及 panic/unwind 等 runtime 机制。

**依赖：** M1 的语义模型和 M2 的类型/内存/布局能力；部分算法原型可提前。

**出口：**

- 冻结的一组泛型集合、迭代器、trait 调用、常量布局和析构用例正确。
- 借用/类型/初始化反例被拒绝；提前退出和失败路径的副作用一致。
- 先用自包含小用例证明能力；库级覆盖由 M4 的真实 alloc/std 证明。

**通过后仍不代表：** sysroot 或 Cargo 已兼容。

**交付物：** 需求 → 语义能力 → 回归证据矩阵；未覆盖的 trait/宏/const 分支显式标出。

T21–T24 共享类型和目标布局（const 泛型、模式、drop、trait 结果互相依赖），需要协同而不是各自独立实现。

---

<a id="t21"></a>
## T21 泛型、trait 与实例约束

`M3 · B · 依赖：T12, T20`

- **做什么：** substitution、可回滚推导事务、obligations、关联类型规范化、方法查找、coherence、对象安全与动态分发，按能力逐项开放。canonical goal、循环和歧义规则沿用移植算法。
- **完成标志：** 约束成功/失败、关联类型规范化、冲突 impl、对象安全、递归边界与 oracle 一致；递归 goal 不会被一律判成功。

<a id="t22"></a>
## T22 生命周期、move/borrow 与 drop 显式化

`M3 · B · 依赖：T13, T20`

- **做什么：** loan/region、重借用、部分 move、分支初始化、drop flags、closure 捕获、异常 cleanup，逐项子包。产出 CheckedBody，正常/早退/失败路径的析构都显式表示在 Body 上。
- **完成标志：** 有效/无效程序与 oracle 一致；每条退出路径的析构可检查。真实运行由 T27/T26 验收。

<a id="t23"></a>
## T23 声明式宏、hygiene 与展开-解析迭代

`M3 · A · 依赖：T20`

- **做什么：** token tree 匹配/转录、fragment 与重复、`macro_rules!`、宏可见性、cfg/属性、edition 差异、递归限制；展开与名称解析迭代进行。产物带 hygiene 信息。
- **完成标志：** 同名捕获/遮蔽、跨模块导入、嵌套与重复、edition 差异的正反例；hygiene 身份不随诊断 span 丢失。

<a id="t24"></a>
## T24 常量求值、内存与有效性

`M3 · B · 依赖：T15, T20, T21`

- **做什么：** 自有 const 解释器，分三步：纯整数 → 目标内存/指针 → 所需内建项；有效性检查、必要的 const 泛型、常量依赖循环和资源限制诊断。
- **完成标志：** 有效常量与运行时结果一致；溢出、越界、无效值、循环依赖按 oracle 拒绝。

<a id="t25"></a>
## T25 单态实例、可达性、vtable 与 drop glue

`M3 · B · 依赖：T21, T22, T24`

- **做什么：** `src/instantiate/`：InstanceKey、可达实例图、递归先声明后定义、静态 allocation 引用、vtable 与 drop glue。泛型实例身份同时用于符号和缓存。
- **完成标志：** 多实例、递归、动态派发、drop glue 引用确定；换并发顺序不改变实例集合。

<a id="t26"></a>
## T26 M3 出口

`M3 · I · 依赖：T21, T22, T23, T24, T25, T27`

- **做什么：** 自包含的泛型集合/迭代器、trait、所有权、常量和析构组合测试；支持范围表和 sysroot 阻塞表。
- **完成标志：** 满足本阶段出口；失败/析构路径真实运行；缺失的 async、内建机制等如实列出。

<a id="t27"></a>
## T27 runtime、异常与并发机器能力

`M3–M4 · D · 依赖：T19, T22, T25`

- **做什么：** 析构调用、不可返回调用、panic 路径；`panic=abort` 和 unwind 两种配置；异常表、personality；atomics/volatile/fence、TLS、所需 intrinsic 和 target feature。按冻结 sysroot 的实际需求拆项。
- **完成标志：** 与 C/独立 driver 链接后观察到正确的析构次数与顺序（正常/早退/失败）、catch_unwind 支持范围、跨调用展开、线程同步 litmus。早期 abort-only 要显式列为限制并拒绝需要 unwind 的输入；最终同配置验收不能强制 abort。

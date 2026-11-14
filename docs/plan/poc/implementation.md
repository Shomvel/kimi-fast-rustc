# PoC 实现边界

链路为 **rustc 前端 + kfrc 中后端 + rustc std**。`rustc_*` 依赖只在
`rustc_codegen_kfrc` 外壳；Body、图、优化器和 x86 后端通过独立契约衔接。

本文记录当前代码，目标范围以 [scope.md](scope.md) 为准。当前转换器逐具体
Instance 单态化；旧链构图时展开布局，新链保留语义值再显式降低。尚无完整的
定义级 R→S→L 链路。新链支持范围及验收见 [semantic-ir-status.md](semantic-ir-status.md)。
[semantic-ir-migration.md](semantic-ir-migration.md) 规定纠正顺序与全部待交付项。
下文的 TLS、SIMD、union 和 dyn 历史实现不自动成为 PoC 必做范围。

## MIR 转换

`crates/rustc_codegen_kfrc/src/convert.rs` 只负责实例入口、MIR local 到
Body local 的对应关系和块构建。子模块各自负责一种转换：

| 模块 | 职责 |
|---|---|
| `types` | 类型形状、rustc 实际字段偏移、ScalarPair ABI 顺序、enum/niche 布局 |
| `places` | place 投影、operand、常量数据的物化 |
| `rvalues`、`casts` | 表达式和指针转换；保持 MIR 语义，不运行优化 pass |
| `unsize`、`trait_object` | unsizing 与真实 vtable 数据、虚调用和动态布局；受支持范围以端到端结果为准 |
| `statements` | 赋值、存储生命周期和非发散 intrinsic 语句 |
| `calls` | 直接/间接调用、rust-call 元组展开、隐式 caller location |
| `terminators` | 控制流、drop glue、assert 的 panic 失败块 |
| `intrinsics` | intrinsic 分派；整数展开、内存与辅助方法放在其子模块 |

独立的 `constants.rs` 复制 CTFE allocation 的原始字节并记录重定位。
panic `Location` 也走此路径，不在编译器中猜它的 Rust 内存布局。
`abi.rs` 查询真实 FnAbi；`enum_layout.rs` 查询真实 tag 编码。
`abi/cast.rs` 单独记录每次调用的 Cast ABI lane、部分字节和扩展方式；
不把一次调用的 ABI 缓存在类型上。
union 构造显式保留 MIR 的 active field；复制使用完整原始字节，避免读取
inactive 字段。静态数据的写权限取最终 interned allocation 的可变性，
包含 `UnsafeCell`，而不是按符号名或初始化过程推断。
数据符号另带显式绑定：匿名 allocation 与 vtable 为对象内 Local；真实 static
保留 rustc 的 linkage/visibility。候选 19 的多对象回归及 regex 16 CGU 链接
通过，不用常量名字前缀代替 ELF 绑定。

## 图与优化

`kfrc-son/src/build.rs` 及子模块负责一次 SSA 构建和效果顺序。旧 `build_graph` 中，
普通标量 local 使用 SSA；需要取地址的 local 和聚合值使用栈槽。
图中的聚合值表示其存储地址，调用 ABI 降低再按 rustc 记录的 lane 读取。
聚合赋值先读取全部 RHS，再写入目标，避免重叠字段覆盖尚未读取的源。

以上是待替换的旧表示，默认标签已纠正为 LegacyLayout。新增
`build_specialized_graph` 复用 SSA 算法，在支持的纯值范围保留聚合、enum 和
宽指针值；`lower_layout` 负责布局、值槽和物理 ABI 物化。新链有 S/L 阶段检查，
完整定义级 R 尚未实现。借用、动态 enum ABI、语义内存等边界明确拒绝。

`kfrc-son/src/pipeline.rs` 组织当前优化，`transforms/scalar/instcombine.rs`
独立执行整数常量折叠和保留 poison 的恒等式化简；`optimize.rs` 保留兼容导出。
它不改控制流、内存效果、调用或可陷入的除法/取余，不把 `x*0`、`x^x`
改成常量。每次优化后重新执行 verifier。

这是首版优化 pass，尚未实现设计中的构建期 GVN、规则 DSL 生成器及 SMT
证明缓存。验证结果不能代替这些计划交付物。

新链的 `optimize_specialized_graph` 进一步接入语义聚合/变体折叠、精确纯值
GVN、常量分支裁剪及不可达清理；Region 与所有 Phi 同步裁剪，活路径效果与
陷阱保留。此处 GVN 范围为相同操作、类型、输入的纯浮动表达式，尚不含内存
GVN、分区 load 转发或全部设计规则。

`black_box` 使用带效果的 Opaque 节点，x86 端只传递值；优化器不穿过此节点。
`KFRC_OPTIMIZE=0` 可关闭优化 pass，用同一输入进行前后差分。

## x86 与入口

`kfrc-x86/src/lower` 处理调度、ABI、指令选择和内存发射，
`analysis/liveness.rs` 独立计算机器 CFG 活跃区间，`regalloc/memory.rs` 承担
内存路径的分配候选。标量路径保留原有寄存器分配器。candidate19 的历史验收
仍对应保守全栈内存路径；本次 candidate02 已独立通过 Linux workspace、
新语义四组对拍及旧链 semver，见 [semantic-ir-status.md](semantic-ir-status.md)。
这些是当前快照的正确性结果，没有新的性能结论。

当前模块、LLVM 对应关系、后续接口及唯一文件 owner 见
[optimization-modules.md](optimization-modules.md)。
`object_file.rs` 写代码、只读数据和重定位。

可执行文件的 unit-main 外壳仍调用真实 `std::rt::lang_start_internal`，保留
运行时初始化和清理。编译器生成的 unit `lang_start` 闭包适配器由 shell 实现，
vtable 与符号来自 rustc；这不表示一般 `dyn Trait` 已完整支持。
候选 09 已差分通过普通 concrete-to-dyn、同 principal 的 auto-trait 删除、
虚调用、动态 size/align 及动态 drop（包括空析构槽）。
候选 15 patch1 又通过 Box FnOnce 的 by-value dyn receiver、ZST/Drop 捕获和
间接返回；它从原 place 获取地址与 metadata，避免读取 unsized payload。
supertrait upcast 和 Rc/Arc 等 unsized-tail receiver 的完整支持尚未交付。

TLS 使用真实 Linux x86-64 local-exec ELF 模型：固定效果节点取得当前线程
地址，机器层读取 FS:0 并通过 TPOFF32 重定位加入偏移；模板使用 .tdata/.tbss
和 STT_TLS。初始化及退出析构继续运行原 Rust/std 状态机。candidate16 patch1
的 Body 隔离和真实 pthread、编码对象/GNU 汇编两条路径已通过。该模型面向
链接到可执行文件的对象/rlib，不能据此声称支持装入共享库的 TLS 代码。
candidate17 又通过完整 Rust TLS 初始化、线程隔离与退出析构，以及自有
符号的 ELF TLS 来源检查。

整数 SIMD 的物理 lane 布局单独记录，图中继续用地址表示，操作在转换模块
按实际 lane 展开。实际 Rust FnAbi 的间接传递保持原样；只有 rustc 判定的
16 字节向量 Direct/Cast 使用独立 XMM 参数/返回路径，不按聚合大小猜 ABI。
candidate17 的 C 双向、间接调用与栈溢出回归已通过；更宽 direct vector、
浮点 SIMD 和优化 SIMD 机器指令尚未实现。

## 验证

入口和复现命令见 `scripts/poc-validation/README.md`。所有结论必须对应固定
源码快照、工具链、命令退出状态和产物；真实 crate 还须检查对象的 kfrc 来源。
通过库的外部 LLVM 测试 harness，不等于该 crate 的所有内部测试也由 kfrc 编译。
正确性差分通过也不等于运行时间、编译工作量、体积或缓存指标已经达标。

rustc 原版测试的固定来源、批次清单、官方 compiletest 双后端执行和阻塞
统计见 [upstream-tests.md](upstream-tests.md)。测试源码不做后端专用改写。

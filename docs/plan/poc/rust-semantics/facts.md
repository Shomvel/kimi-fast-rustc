# 事实账本与公共载体

每条 Rust 语义事实登记五项：**来源**（MIR 构造或 rustc 查询）、**载体**（在 kfrc 中存放的位置）、
**作用域**（控制范围、动态实例与内存依赖）、**消费者**（读取它的 pass）、**擦除点**（最后一个使用它的阶段），
以及对应的**验证**手段。字段含义沿用
[requirements-and-facts](../../../design/ir/requirements-and-facts.md#凭据作用域与维护)。
一条事实在消费者合入并有验收证据后才算落地（红线 R2）。

## 账本

| 编号 | 事实 | 来源 | 载体 | 作用域 | 消费者 | 擦除点 | 验证 |
|---|---|---|---|---|---|---|---|
| RF1 | 位计数、字节/位反转、漏斗移位、饱和运算的语义 | intrinsic 调用 | `BitCount`/`ByteSwap`/`BitReverse`/`FunnelShift`/`Saturating*` 节点 | 值 | instcombine、GVN、x86 选择 | x86 选择 | 解释器求值；≤ 8 位穷举 |
| RF2 | 零输入为 poison | `ctlz_nonzero`/`cttz_nonzero` | `ZERO_POISON` 位 | 值 | x86（`bsf`/`tzcnt`） | x86 选择 | N1.1 |
| RF3 | 函数的有效 CPU 特性 | `-Ctarget-cpu`/`-Ctarget-feature`、`#[target_feature]` | lowering 入参 | 函数 | x86 选择 | x86 选择 | N1.4（objdump） |
| RF4 | 无符号/有符号不溢出 | `BinOp::*Unchecked` | `NUW`/`NSW` 位 | 值 | division、instcombine、x86 规范化 | x86 选择 | 解释器 poison；N3.1 |
| RF5 | 精确除法 | `exact_div`、`ptr_offset_from(_unsigned)` | `EXACT` 位 | 值 | division | x86 选择 | N3.2 |
| RF6 | 按位或两侧不相交 | `disjoint_bitor` | `DISJOINT` 位 | 值 | instcombine；LP4 的 `lea` | x86 选择 | 解释器 poison |
| RF7 | 移位量小于位宽 | `ShlUnchecked`/`ShrUnchecked`、`unchecked_funnel_*` | `AMOUNT_IN_RANGE` 位 | 值 | x86 省掩码 | x86 选择 | 解释器 poison |
| RF8 | 标量有效范围 | layout `Scalar::Initialized` 的 `valid_range` | `RecordedTypes::scalar_valid_range` + `analysis/ranges.rs` | 由类型化操作产生的值 | 比较折叠、`BitCount`、bool 掩码、LP5 switch | 分析在 x86 选择前结束 | RS3.2a 解释器检查；N3.3 |
| RF9 | 路径谓词 | `If` 出口、`Assume`、归纳变量 | `PredicateFacts`（已有） | 被证明边支配且为同一动态实例 | `conditions.rs`、LP6 | S→L 擦除前按需导出 | verifier 复查；N3.4–N3.6 |
| RF10 | 字段需求 | S 的使用关系 | LP3 需求 worklist | 函数内值 | `lower_layout` 展开 | L 结束 | Omitted 外泄断言 |
| RF11 | 活跃变体 | `MakeVariant`、`VariantSwitch` 出口、按边事实 | `VariantWitness`、`variant_on_edge` | 边、同一动态实例 | 需求、`fill_phi`、变体穿线 | L 结束 | RS2.6 负例 |
| RF12 | 不可居留变体 | rustc inhabited 谓词 | `RecordedTypes::semantic_inhabited_variants`（已有） | 类型；值须 defined | 需求、`VariantSwitch` 简化 | L 结束 | `MaybeUninit<!>`、指向不可居留类型的指针负例 |
| RF13 | Indirect 实参归被调者独占 | `rustc_target` Indirect 属性；MIR 参数所有权 | `FnAbi` PassMode | 本次调用 | `Repr::Borrowed`、实参直接传槽 | L 结束 | N2.9 |
| RF14 | move 后源未初始化 | `Operand::Move` | S 中源写入 `Undefined` | 至源再初始化 | 等价类末次使用；删除移出写 | L 结束 | N2.2 |
| RF15 | 字段／偏移不相交 | 地址算术 | Location 路径 | 同根 | clobber walker | L 用偏移重算 | RS4.2 单测 |
| RF16 | local 未逃逸（流敏感） | 数据流 | 逃逸位向量 | 捕获点之前 | forward、DSE | 每次 pass 重算 | 动态 alias 断言；N4.1–N4.3 |
| RF17 | 调用效果摘要 | 本 crate 函数体分析 | `CallSummary` | 调用点；定义在链接时不可替换 | walker、DSE、RS2.7、RS5 | 进入调用者机器码键 | N4.14、N4.15 |
| RF18 | rustc 声明级被调者属性 | `NEVER_UNWIND`、`FFI_PURE`/`FFI_CONST`、allocator 标志、`COLD` | `CalleeFacts` | 调用点 | 摘要、Heap 根、RS7.5 | 进入机器码键 | converter 单测 |
| RF19 | copy 源与目标不重叠 | `CopyNonOverlapping` 操作契约 | 访问模型 | 该操作点 | walker、copy 前递 | L 的 `CopyBytes` | 解释器重叠即 UB |
| RF20 | `&T: Freeze` 参数在调用期间只读、无别名 | rustc-O 契约（`arg_attrs_for_rust_scalar`） | `ArgPermission::Frozen` | 实例函数体；内联后为侧表作用域 | Q1/Q2 | S→L | N5.14 守卫；Miri SB/TB |
| RF21 | `&mut T: Unpin` 参数无别名 | 同上 | `ArgPermission::Unique` | 同上，并受逃逸约束 | Q4 | S→L | 同上 |
| RF22 | 调用点只读实参 | 被调者的 Frozen 权限 | `callee_permissions` | 本次调用 | Q3 | S→L | N5.7 |
| RF23 | 返回别名、返回字段、常量返回 | 被调者函数体 | `ReturnSummary` | 调用点 | identity 折叠、RS4 | 进入机器码键 | N5.8 |
| RF24 | vtable 槽内容 | rustc `vtable_allocation` | 只读 `DataObject` + relocation + 初始化掩码 | 程序全程 | E1/E2 去虚化 | S | N5.13 |
| RF25 | 内联收益 | 调用者实参形态 + 被调者图 | 决策日志 | 调用点 | 收益驱动内联 | 内联完成 | 决策日志哈希一致 |
| RF26 | 孔位答案 | rustc 查询 | `Hole` 答案（已有） | 实例 | 特化 | S | `semantic_specialize` 测试 |
| RF27 | 特化观测读集 | S/L 对 oracle 的读取 | `ObservedFacts` | 实例 | 身份键 | 进入缓存键 | N6.8 检测器自检 |
| RF28 | 被调者身份 | `Instance::try_resolve` | `Resolve` 答案 | 实例 | 特化、身份键 | 进入缓存键 | N6.1 |
| RF29 | 存储寿命 | `StorageLive`/`StorageDead` | `SlotLive`/`SlotDead` | 每次动态寿命 | 帧着色 | 帧布局 | N7.1–N7.3 |
| RF30 | drop 身份与 NeedsDrop | `Drop` 终结符、`resolve_drop_in_place` | `DropInPlace`、`Hole::NeedsDrop` | 调用点 | RS6.1、RS4 摘要 | x86 选择（变为调用） | N7.6、N7.7 |
| RF31 | 冷 | `#[cold]`、`cold_path`、Assert 失败块、cleanup | `Likelihood` 旁表 | 块与调用点 | 内联、LP7 块布局 | 块布局 | N7.8–N7.10 |
| RF32 | 不 unwind | `NEVER_UNWIND`、`fn_can_unwind` | `CalleeFacts` | 调用点 | 摘要、RS7.6 | 进入机器码键 | converter 单测 |
| RF33 | panic 策略 | 会话配置 | 孔位或会话常量 | crate | cleanup 裁剪 | S | RS7.6 |

## 公共载体

以下载体由多个方向共用，各由一个步骤首先交付，其余步骤只扩展字段。

| 编号 | 载体 | 首个交付步骤 | 使用方 |
|---|---|---|---|
| RS0.1 | `CalleeFacts`：`kfrc-body` 的 `Callee::Function` 增加 `cold`、`nounwind`、allocator 种类、`ffi_pure`/`ffi_const`、`track_caller`、drop glue 种类、实例身份；SoN `CallTarget` 同步 | RS0.1（converter `convert/calls.rs:35-42` 与 `lib.rs:138-143` 读取 `codegen_fn_attrs`） | RS4.6、RS5、RS6.2、RS7.4、RS7.5 |
| RS0.2 | 纯值节点的语义标志位：`payload` 位集，verifier 按 opcode 检查，GVN 按标志区分 | RS3.1 | RS1、RS3 |
| RS0.3 | 解释器检查：标量有效范围解码（RS3.2a）、标志位违例产生 poison、动态 alias 断言模式（RS4.2）、`SlotDead` 后访问为 UB（已有） | 各自步骤 | 全部方向的差分验证 |
| RS0.4 | 度量：`KFRC_PROFILE` 新窗口（`collect_mono`、`convert_mono`、`convert_r(_rejected)`、`optimize_s_seeded/full`、`memory_s`）；`OptimizeStats`/`LayoutStats`/`InlineStats` 新计数；JSONL 输出与生成表格的脚本 | RS6.0（与 LP8 共用阶段口径） | 全部方向 |
| RS0.5 | 类型表扩展：`scalar_valid_range`、ScalarPair 车道范围 | RS3.2b | RS3、RS6.2 |
| RS0.6 | 事实重映射契约：CFG 重写、内联、特化、值替换与压实对事实的处理为“重映射或删除”，删除后消费者回到保守形式 | 每个新增载体在其首个步骤中声明 | 全部方向 |

RS0.6 对新载体的具体要求：侧表（`Likelihood`、`PermissionScope`、折叠记录）在 `compact` 时随 `NodeId`
重编号；内联 `splice` 复制或删除登记；`specialize` 的复制路径同步复制；verifier 检查旁表引用的节点存活。

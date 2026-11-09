# RS6 泛型特化的语义复用

对应 issue #5 方向 6。事实账本条目 RF26–RF28（[facts.md](facts.md)）。
主张 5（每定义 R 级优化恰好 1 次；去重比不低于 `-Zmono-dedup`）见 [metrics](../metrics.md)。

## 目标

1. 扩大定义级 R 转换覆盖，按定义与按实例统计回退原因及其代价。
2. 记录 S/L 实际读取的类型语义、被调者身份、drop、有效值、ABI 等观测，作为特化身份键。
3. 先以影子模式验证“同键同码”，再共享特化结果与机器码；身份键覆盖全部观测，大小与对齐只是其中两项。
4. 分别计量上游单态收集、R 准备、实例特化、布局与机器码生成。

## 基线现状（`37cea66`）

**路径选择。** `build_instance_graph`（`rustc_codegen_kfrc/src/lib.rs:425-482`）：
`Definitions::specialize` 返回 `Some` 走 R→S 后只处理 dirty 种子；返回 `None` 走逐实例
`convert_instance` 与完整 `optimize_specialized_graph`；返回 `Err` 为 fatal。

**回退原因**（`convert/definition.rs:34-90`、`convert/definition/emit.rs`、`convert/definition/types.rs`）：

| 类别 | 位置 |
|---|---|
| 非泛型定义；全部编译器生成的 shim | `definition.rs:38-42`；`definitions.rs:30-36` |
| `#[track_caller]` 定义与被调者 | `definition.rs:45-49`；`emit.rs:412-414` |
| cleanup 块或 spread 参数（含不可达块） | `definition.rs:53-57` |
| 类型：Float、Str、Slice、Dynamic、FnDef/FnPtr、Closure、Alias、泛型数组长度、u128 | `definition/types.rs:9-60,106` |
| `CopyNonOverlapping`、SetDiscriminant、Deinit | `emit.rs:36,46` |
| Index/ConstantIndex/Subslice 投影；泛型常量与非标量常量 | `emit.rs:99,116-138` |
| Div、Rem、Offset、Cmp、带溢出检查运算、unchecked 运算 | `emit.rs:225-228` |
| 除 `assume`/`size_of`/`align_of`/`needs_drop` 外的 intrinsic；间接调用 | `emit.rs:377-387` |
| **Drop 与 Assert 终结符**、TailCall、InlineAsm | `emit.rs:547` |

std 中常用泛型大多含 Drop、Assert 或 track_caller 被调者（`Option::unwrap`），
R 的实例覆盖率预计很低，以 RS6.0 实测为准。

**计数与窗口。** `mono_reasons: BTreeMap<String, usize>`（`definitions.rs:21,61-63`）按实例计数，
原因字符串带具体类型，只在 `KFRC_SPECIALIZATION_STATS`/`KFRC_DUMP_GRAPH` 下打印。
`collect_and_partition_mono_items`（`lib.rs:119`）与 `convert_instance`（`lib.rs:442`）不在任何
`KFRC_PROFILE` 窗口内；`prepare_r` 含被拒定义的白做转换；`optimize_s` 不区分 seeded/full。

**孔位与依赖。** 已实现 `SizeAlign`、`NeedsDrop`、`Resolve`、`VariantEncoding` 四种
（`graph.rs:466-474`）；`VariantEncoding` 无生产者（`definition/oracle.rs:117-119`）；
`Resolve` 的答案身份为符号（`oracle.rs:105-115`）。`specialize/dependencies.rs:6-70` 记录“依赖类型
参数的节点”，用作 S 的 dirty 种子（`specialize/clone.rs:35-37,65-67`）。S/L 还经 `instantiate_type`
与 target oracle 读取完整具体类型、字段偏移、enum 编码与 ABI（`specialize/mod.rs:362-379`；
`lower_layout/semantic_memory.rs:25-38`），这些读取尚未记录，因此 `answers` 只覆盖部分观测。
`specialize/mod.rs:48-50` 与测试 `semantic_specialize.rs:319` 已固定“相同大小不构成复用键”。

**共享。** 无 `SpecializationKey`；对象写出器不支持别名符号（`kfrc-x86/src/object_file.rs:25-28`）；
出现在多个 CGU 的符号在每个 CGU 各编译一次（`lib.rs:252-265`）；R 图未发布到 metadata，
“每定义 R 一次”在单个 crate 会话内成立。

## 身份键覆盖的观测

| 观测 | 理由 |
|---|---|
| R 图内容哈希、孔位答案 | 特化输入 |
| 每次 oracle 读取：`size_align`、字段、语义聚合、变体编码、有效范围、inhabited、shape | S/L 实际消费 |
| 本实例 `FnAbi`；每个 Call 的被调者身份、ABI、`cold`/`nounwind` | 调用约定与块布局、内联决策 |
| 内联时读取的被调者 S 图身份与被调者摘要（RS4.6） | 跨函数优化依赖 |
| NeedsDrop（ZST 可有带副作用的 Drop）、PanicStrategy | 语义与 cleanup |
| 常量与数据对象内容哈希；vtable、unsize、fn item、TLS、TypeId 按身份 | 数据依赖 |
| `#[track_caller]` 的 Location | 机器码不同 |

经验来源：mirx `-Zmono-dedup` 的浅指针键与已知漏项（`&[T]` 长度上的 `range`、`repr(packed)`
尾部对齐、导出的 `#[inline(never)]` 泛型）。

## 分步计划

| 步骤 | 内容 | 主要文件 | 退出条件 | 依赖 |
|---|---|---|---|---|
| RS6.0 度量 | `FallbackReason` 枚举（`NonGeneric`、`Shim(kind)`、`TrackCaller`、`Cleanup`、`Terminator(kind)`、`Rvalue(kind)`、`Type(kind)`、`Intrinsic(name)`、`Constant`、`Projection(kind)`），具体类型进可选样本；按定义与按实例计数，JSONL 输出；新增窗口 `collect_mono`、`convert_mono`、`convert_r`/`convert_r_rejected`、`optimize_s_seeded`/`optimize_s_full`；统计“S 优化但未发射”的实例 | `definitions.rs`、`lib.rs`、`profile.rs` | semver 与 W02/W06 的覆盖率、原因直方图、分阶段指令由脚本生成，并与 `KFRC_PROFILE_TOTAL` 闭合（差额单列） | 与 LP8 共用阶段口径 |
| RS6.1 扩大 R 子集 | 按直方图顺序：Drop（`DropInPlace` 加 `NeedsDrop` 与 drop glue `Resolve`，与 RS7.4 同一实现）→ Assert（冷失败子图，与 RS7.5 共用；溢出运算的 `CheckedBinary` 规则）→ track_caller 被调者（Location 作为 S 期常量实参）→ Div/Rem/Cmp、Index 投影、非标量常量、`ConstValue` 孔位 → 非泛型定义（空孔位表）；特化期 fatal（ABI adapter、Virtual/Intrinsic 解析）前移为定义级可用性判断 | `convert/definition*.rs`、`specialize/*`、`graph.rs`（孔位） | 每扩一项：R→S 与逐实例结果在解释器与真实 E2E 下一致；“每定义 R 一次”计数通过；无新增 fatal；semver 实例覆盖率达到 RS6.0 后设定的目标 | RS6.0；RS7.4、RS7.5 |
| RS6.2 观测记录 | `TypeOracle` 读接口外包 `ObservedFacts` 记录器，加上表其余项；调试构建中未登记的读取报错；`SpecializationKey = (R 图哈希, answers, 特化期观测)`，机器码键再加 L 读集与 L/x86 配置 | `specialize/oracle` 接口、`lower_layout` oracle 调用点、`lib.rs` | 审计脚本证明读集覆盖 S/L 访问的全部 oracle 接口；键计算开销 ≤ 后端指令的 0.5% | RS6.1 的主要项 |
| RS6.3 影子模式 | 只计算等价类，照常各自编译；比较归一化后的机器码（重定位目标按被调者类编号，剔除符号名） | 新增比较脚本与 `lib.rs` 输出 | 误合并（同类不同码）为 0，覆盖 semver、W06 与生成式语料；报告同码不同类数与去重比，并与同 share-generics 设置的 `-Zmono-dedup` 对照 | RS6.2；LP2 输出逐字节确定 |
| RS6.4 共享 | 对象写出器支持别名符号；同键只做一次 S/L/x86；SCC 内先分配临时身份再做划分细化；导出的 `#[inline(never)]` 泛型不跨 crate 合并；修复同一符号跨 CGU 重复编译 | `kfrc-x86/src/object_file.rs`、`lib.rs` | 主张 5 达标；`.text` 与编译指令改善；runtime 不回退（E2B 同沙箱） | RS6.3 |
| RS6.5 R 图发布 | R 图与孔位表进入 metadata，“每定义 R 一次”跨 crate 成立 | metadata 编码 | 下游 crate 不再重建上游定义的 R | RS6.4 |

## Fixture 与负例

正例：W06 `largest<T>`（u32、i32、char、u64、f64）与 `push_one<T>`（u32、i32、`[u8; 4]`、char）；
`fn f<T>(x: u32) -> u32`（T 未使用）；`size_of::<T>() == 0` 剪枝；`needs_drop::<T>()` 分支；
递归泛型与互递归 SCC；已有 `kfrc-son/tests/semantic_specialize.rs` 的 :275、:319、:557、:1161、:1247。

| 编号 | 形状 | 期望 |
|---|---|---|
| N6.1 | 大小对齐相同、trait impl 不同（u32/i32 的 `lt`；`[u8; 4]`/u32 的 `Clone`） | 不同类 |
| N6.2 | 两个 ZST，一个带写 static 的 Drop | 不同类，drop 调用保留 |
| N6.3 | `Option<NonZeroU32>` 与 `Option<u32>`；`bool` 与 `u8` | 不同类 |
| N6.4 | 被调实例一个 `#[cold]` 或 nounwind，另一个不是 | 不同类 |
| N6.5 | 不同 `#[track_caller]` Location | 不同类 |
| N6.6 | 导出的 `#[inline(never)]` 泛型（rlib/dylib） | 不跨 crate 合并 |
| N6.7 | `repr(packed)` 与普通 struct 字段相同 | 不同类 |
| N6.8 | 从读集中删去 ABI 一项 | 误合并检测器报错（检测器自检） |

## 指标

R 转换覆盖率（按实例、按定义、按 S 节点加权）；回退原因直方图及回退实例的
`convert_mono + build + optimize_s_full` 指令；每定义 R 优化次数；`instances_specialized`、
`holes_answered`/`branches_pruned`/`nodes_copied` 的均值与 P95；键数与去重比；别名符号数与节省的 `.text`；
误合并数；分阶段编译指令；S 优化但未发射的实例数。

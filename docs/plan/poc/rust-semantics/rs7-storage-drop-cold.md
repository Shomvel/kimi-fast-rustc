# RS7 存储寿命、Drop 与冷路径

对应 issue #5 方向 7。事实账本条目 RF29–RF33（[facts.md](facts.md)）。

## 目标

1. `SlotLive`/`SlotDead` 的范围传到帧布局，存储寿命不重叠的槽复用同一区域；
   L 生成的临时槽也有寿命；ZST 槽不占帧字节。栈槽寿命与寄存器活跃区间分别处理。
2. Drop 以 `DropInPlace` 保留身份、`NeedsDrop` 与效果事实，保持析构顺序与所支持的 unwind 行为。
3. `#[cold]`、`cold_path`、Assert 失败块、cleanup 等冷源贯通到内联决策、块布局与机器层。

## 基线现状（`37cea66`）

**存储寿命。** 标记链路 MIR（`convert/statements.rs:18-23`）→ SoN（`build.rs:429-450`，
只对 `memory_locals`）→ S（`specialize/clone.rs:117-126`）→ L（`lower_layout/semantic_memory.rs:270-273`）
→ x86 调度（`kfrc-x86/src/lower/schedule.rs:84-85,103-104,275-276`）已贯通，在指令选择时丢弃
（`kfrc-x86/src/lower.rs:587-589`）。帧布局给每个槽独立区域，ZST 占 1 字节
（`kfrc-x86/src/lower/memory.rs:19-58`；LP2 改写后的 `lower/allocated/emit.rs` 算法相同）。
LP2 的 spill 槽着色只作用于寄存器 spill 片段。L 新建的 ABI 物化、transmute、MaybeUninit 暂存槽
无寿命标记（`lower_layout/memory.rs:244-262`、`representation.rs:165-185`、`maybe_uninit.rs:16-30`）。

MIR 规定：在未分配的存储上做任何访问都是 UB（`rustc_middle/src/mir/syntax.rs:367-388`）。
因此按标记着色在地址已逃逸时同样成立，前提是 kfrc 的变换把同槽标记视为该槽的 clobber，
访问不跨越标记移动。

**Drop。** MIR `Drop` 转为普通 `Call(drop_in_place::<T>)`（`convert/terminators.rs:46-103`）；
SoN 无 Drop 节点；被 drop 的 local 因取地址进入内存（`build.rs:597-606`）。
`resolve_drop_in_place` 返回 `DropGlue(_, None)` 时已改为 Goto（`terminators.rs:62-65`）。

**unwind。** 后端只接受 `-Cpanic=abort`（`lib.rs:86-89`）；Body 保留 `UnwindAction` 与 `is_cleanup`，
SoN 构图忽略 unwind（`build.rs:532-541`）；R 遇到 cleanup 判为不可用（`definition.rs:53-57`）。

**冷路径。** `cold_path` 编译为空（`convert/intrinsics.rs:84`）；1.94.1 的 `likely`/`unlikely` 是
`#[inline(always)]` 普通函数，内部调用 `#[cold]` 的 `cold_path`，`intrinsics.rs:76-79` 的分支在
该版本不可达；`codegen_fn_attrs` 已在 `lib.rs:138-143` 查询，只读取 `inline`；Assert 失败块为
`SwitchInt` 的 otherwise 分支（`terminators.rs:151-174`）；块按逆后序发射，只删除跳到下一块的 `jmp`
（`schedule.rs:159,303-321`；`lower/fallthrough.rs:5-20`）。`from_str` 有 142 条 `jmp`，LLVM 为 20 条（LP7）。

rustc 对照：`find_cold_blocks`（`rustc_codegen_ssa/src/mir/mod.rs:563-608`）把以调用 cold 函数结尾的块、
`Unreachable` 块、后继全冷的块标为冷；两路 `SwitchInt` 用 `cond_br_with_expect`（`mir/block.rs:402-424`）；
`lifetime.start/end` 来自 StorageLive/Dead（`mir/statement.rs:53-66`），大小为 0 与 opt-level 0 时不生成
（`rustc_codegen_llvm/src/builder.rs:1765-1777`；`rustc_session/src/session.rs:511-517`）。

## 冷信息的契约

冷是策略输入（[constraints-and-effects](../../../design/ir/constraints-and-effects.md#opaque-与其他优化策略)）。
冷源限定为：调用 `#[cold]` 函数的调用点、`cold_path` 所在块、Assert 失败块、cleanup 与
`UnwindTerminate` 块，以及后继全冷的块。`-> !` 与 `#[inline(never)]` 本身不产生冷标记。
冷标记改变代码放置与内联决策，语义输出保持不变。

## 分步计划

| 步骤 | 内容 | 主要文件 | 退出条件 | 依赖 |
|---|---|---|---|---|
| RS7.0 帧统计 | 每函数输出槽数、按来源（MIR local、ABI 物化、transmute、opaque 暂存、spill）的字节、有标记的槽、理想着色下界、最终帧字节 | x86 lowering，`KFRC_FRAME_STATS` | semver、W01、W04、W09 的“当前帧 / 着色下界 / LLVM 帧”三列由脚本生成 | — |
| RS7.1 MIR 槽着色 | L 之后、帧布局之前做“可能已分配”前向数据流（`SlotLive` gen、`SlotDead` kill，已活槽再 `SlotLive` 先 kill 后 gen，无标记的槽全函数活）；不同时可能分配的槽共享区域，组对齐取最大值；`MachineFunction` 增加 `stack_slot_groups`，帧布局按组分配；spill 槽区域独立；verifier 检查访问与同槽标记的相对次序 | LP2 合入后的 `lower/allocated/emit.rs`、x86 `lower.rs`、`verify.rs` | W09 `pipe<256>` 帧约 528 → 约 272 B，`pipe<4096>` 约 8208 → 约 4112 B；全部测试、差分与 E2E 通过；`.text` 不增 | LP2 合入 |
| RS7.2 L 临时槽寿命 | L 创建 ABI 物化、transmute、暂存槽时插入 `SlotLive`，在最后消费者之后插入 `SlotDead`；ABI 物化槽的最后消费者为该调用 | `lower_layout/memory.rs`、`representation.rs`、`maybe_uninit.rs`、LP3 的 `owned_transport.rs` | semver ABI 物化槽字节下降；L 解释器正负例通过 | RS7.1；LP3 合入 |
| RS7.3 ZST 槽 | 所有 ZST 槽使用同一按最大对齐取的地址，帧字节为 0；drop 调用与次序保持 | 帧布局 | 带副作用 Drop 的 ZST fixture 输出与 LLVM 一致，帧为 0 | RS7.1 |
| RS7.4 `DropInPlace` | Body→SoN 把 `drop_in_place` 调用标为 `DropInPlace{glue, ty}`（`CallTarget` 的 kind，与 RS6 共用扩展）；R 中以 `NeedsDrop` 与 drop glue `Resolve` 表示；`DropInPlace` 的位置与次序固定，越过 Unknown 效果视为改变语义；“不捕获指针”只来自 drop glue 的 RS4.6 摘要 | `convert/terminators.rs`、`build/calls.rs`、`graph.rs`、`specialize/*` | W04 `work` 的 drop 序列与 rustc 一致；有摘要时被 drop 的 local 不阻止其他槽转发，统计报告 | RS4.6（摘要部分） |
| RS7.5 冷信息贯通 | Body：Call 终结符带 `cold`（读 `CodegenFnAttrFlags::COLD`）；`cold_path` 把所在块标冷；Assert 失败块、cleanup、`UnwindTerminate` 块标冷。SoN：`If` 分支与 Call 带 `Likelihood{Hot, Neutral, Cold}` 旁表，inline/splice、seam 折叠、promotion 改写控制时保持；verifier 检查旁表一致。R：`Resolve` 答案带被调实例的 `cold`/`nounwind`（进入 RS6.2 身份键）。消费者：冷调用点不内联（`#[inline(always)]` 除外）；LP7 把冷块放到函数末尾并合并相同 panic 尾块 | `convert/intrinsics.rs`、`convert/terminators.rs`、`lib.rs:138-143`、kfrc-body、`graph.rs`、`transforms/ipo/*`、x86 `lower/schedule.rs` | W01 `?` 错误路径、越界检查、`if unlikely(x)` 三个 fixture 的热路径上无向前无条件跳转，panic 尾块在函数末尾；`.text` 不增；解释器输出不变 | Body/SoN 部分无依赖；布局部分随 LP7 |
| RS7.6 unwind | R 保留 cleanup 边，S 按 `PanicStrategy` 裁剪；RS7.1 数据流计入 cleanup 边；需要 `.eh_frame` 与 landing pad | `build.rs`、x86、对象写出器 | 单独立项 | PoC 支持 unwind |

## Fixture 与负例

正例：W09 `pipe<N>`（N = 256、4096）；循环内 `let mut buf = [0u8; 512]; fill(&mut buf, i)`；
互斥作用域的两个大数组 `if c { let a = [0u64; 64]; .. } else { let b = [1u64; 64]; .. }`；
多个同时存活且取地址的 ZST `Guard`（Drop 写 static 计数）；W04 `work`；`slice[i]` 越界检查；
`checked_add().unwrap()`；`if core::intrinsics::unlikely(x) { cold() }`；`#[cold] fn slow()` 调用点。

| 编号 | 形状 | 期望 |
|---|---|---|
| N7.1 | `SlotDead` 之后访问 | L 解释器报 UB；变换不把它移到 `SlotDead` 之前 |
| N7.2 | 已活槽再 `SlotLive` | 旧内容视为未初始化，不转发旧值 |
| N7.3 | 循环内槽与跨回边槽 | 区间重叠，不合并；数据流走到回边不动点 |
| N7.4 | 无标记的槽（参数、`_0`） | 只与 ZST 共享 |
| N7.5 | 对齐 > 16 的槽与普通槽同组 | 偏移满足最大对齐 |
| N7.6 | 带 Drop 的 ZST | drop 调用次数不变 |
| N7.7 | `DropInPlace` 上移或越过未知调用 | 拒绝 |
| N7.8 | `fn run() -> ! { loop { .. } }` 的调用前缀 | 不标冷，不移到末尾 |
| N7.9 | `#[inline(never)]` 被调者 | 不标冷 |
| N7.10 | 冷标记前后的解释器输出与 Check/RequireDefined | 完全相同 |
| N7.11 | 互斥作用域的地址比较（只比较不解引用） | 不崩溃；比较结果与 LLVM 一样不作要求 |

## 指标

帧字节（总和、P50/P95、最大值，与 LLVM 对照）；按来源的栈字节与着色前后对比；
着色率（合并槽数 ÷ 有标记槽数，节省字节 ÷ 有标记槽字节）；静态 `jmp` 数与热路径向前 `jmp` 数；
冷块位于函数尾部的比例、合并的 panic 尾块数、冷调用点内联次数；`.text`；
新增分析在 `layout`/`x86` 窗口的指令增量。runtime 只用于不回退检查。

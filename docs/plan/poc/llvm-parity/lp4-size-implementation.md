# LP4 体积优先的下一候选

日期：2026-11-09。只读设计，未修改 x86 生产实现、未编译或重新计时。
顺序遵循 RS1 → ABI → LP4 → LP5/7；当前 RS1 的 x86 文件由独立 owner 修改，实施 LP4 前须重新冻结基线。

## 当前对象证据

源码包 SHA-256：`8b211306a26ae3e18aea2f93005e9e405a9f495ce6cdf39c7123e0394103e241`。
收据为 `evidence/poc-rs3-20261109/noif/source.json`。以下路径相对该 evidence 根：

- `results/noif/attribution-formal/kfrc/machine-00.json`、`disassembly.txt`、`graphs/stderr.gz`：真实 semver O3 对象。
- `results/noif/rs3-shapes/kfrc-on/bc.asm`：真实 O3/MIR0 bounds probe。
- `results/noif/attribution-formal/kfrc/text.json`：252 个文本符号记录，总 symbol bytes 287103；不是去重后链接体积。

`Version::from_str` 精确 ELF 符号区间为 19142 bytes、3687 条指令；2477 条精确 mov，
1408 条 mov 含 rbp 内存操作。**rbp 是 frame-memory proxy，不是已证明的 spill**：
`allocated/emit.rs::Emitter::new` 先放 allocation spill slots，再放 layout 对象槽；
之后还有固定寄存器保存、scratch/cycle、callee-save/outgoing 等帧区域。
单凭汇编基址不能把这些流量全部归因于寄存器分配。

| 最终 from_str 证据 | 数量 | 解释 |
|---|---:|---|
| L 级 If | 98 | 最终 Layout 图的真实控制消费者 |
| 单一消费者 Compare / Not(Compare) → If 链 | 96 | 可直接分支的静态候选；不是拿全部 cmp 数代替 |
| cmp | 108 / 324 bytes | 含返回值或其他物化消费者 |
| setcc | 108 / 403 bytes | sete84、setbe12、setne12 |
| movzx | 147 / 598 bytes | 同时含比较结果与窄 load，不能全删 |
| test | 98 / 294 bytes | 当前 If 全部重新 test Boolean |
| 相邻 mov immediate; add | 199 / 1990 bytes | 数值或地址运算混合，不是承诺可全部移除的字节 |
| L Load/Store | 231 | 全部地址为顶层 PtrOffset(base,const) |
| 上述非零字段偏移 | 75 | +8 为37，+4为29，+16/+24/+32各3；其余156为0 |

按符号起点+ELF size截断反汇编，避免把尾部 padding 当作第3688条指令。L 图按确切符号取最后 Layout，
使用次数含全部节点输入；96 条链每个 Compare/Not 只有后续链或 If 一处使用。
这些是命中依据，不是性能收益或最终压缩率。

`bc` 长126 bytes。两个 +8 字段地址各先 mov 8 到寄存器再 add；分支为
`cmp; setb; movzx; mov 1; xor; …; test; jne`；索引还出现 `mov 1; imul`；
byte load 的 `movzx` 后又 `mov 255; and`。ABI先解决的16字节slice对象槽也在这里，
LP4仅简化剩余操作数与地址，不能混淆两层收益。

## 第一独立 LP4 候选：直接比较分支 + 常量地址位移

可在一个候选内做两条独立选择规则，但计数和形状验收分开。不增加 inline budget，不碰 load 合并，
不推断新的别名、指针合法性或算术 NUW。

### 直接比较分支

当前 `lower.rs::run` 在 terminator 前遍历 `schedule.placed`，`emit_pure(Compare)` 生成
VirtualInstr::Compare；`allocated/emit.rs` 发 Cmp+SetCc；`emit_terminator(If)` 再生成 Test+Jcc+Jmp。
因此只改 If 发射会留下已提前物化的死 Compare。

建议新增小的 lowering 选择 helper，先识别同块、单一使用的 Boolean Not 链至整数 Compare，
记录可被覆盖的纯节点；`run` 的 placed emission 对这些节点延后，If 直接读取 Compare 两操作数，
用奇偶 Not 次数翻转 Condition。结果仍被返回、存储、Phi使用、另一个分支使用，或被固定观察消费时
保持现有物化路径；首版不跨 block、call 或 load 重新计算值。

现有 `VirtualInstr::Test { lhs, rhs }` 已实现 rhs=0 时 test lhs,lhs，其他rhs为cmp；
可复用它再紧接现有 Jcc，条件由 ComparePredicate映射，而不是一律NotEq。rhs为0时 test与
cmp lhs,0 对当前整数条件码等价。不需要新增可跨块存活的 EFLAGS 虚拟寄存器。

分配器契约：Compare两操作数是 Early inputs，无结果寄存器；Jcc仍是 Branch flow。
`allocated.rs` 可能在两操作间插入 edge/copy，`checker.rs::moves` 当前只允许
Mov/MovImm/Load/Store，这些不改 flags。必须保留该约束，并让最终 zero-idiom优化继续按 flags
活跃性决定是否用xor替换mov0。不能在 cmp和jcc之间插入重新计算地址、算术、call或test。
若以后合并为单个 CompareBranch operation，必须先完成所有保存/重载，再发cmp+jcc；
恢复代码不能只落在未跳转路径。这不是首版必需的改造。

具体文件：`lower.rs` 与可新建 `lower/branch.rs`（覆盖识别/条件映射），
`lower/allocated/constraints.rs`、`emit.rs`、`checker.rs`仅核验既有契约，
`lower/fallthrough.rs`仍负责已有的无条件fallthrough删除。首版不顺便做条件反转块布局。

### 常量地址位移

首版只收 `PtrOffset(base, const)` 的真实 Load/Store 地址，const按64位地址加法解释后必须可表示
有符号disp32；累积链用检查运算，超限回退原地址物化。0偏移不新增指令。
不改变Load/Store控制与effect顺序；只让同一访存使用 `base+disp`，不把load外提或折进比较。

新增虚拟地址结构可从 `BaseDisp { base: VirtualValue, disp:i32 }` 起步；
`LowerInstr::Load/Store`携带它。选择后基址作为明确 Early input，Store value也作为Early input。
实际地址只访问一次的PtrOffset纯链可覆盖，不再提前emit Add；其他地址消费者仍保留原值。
机器层 LoadIndirect/StoreIndirect增加disp并由编码器选择disp0/disp8/disp32。
现对象编码对零位移总留一个disp8，普通base还可省该字节；rbp/r13必须保留零disp，rsp/r12必须SIB。

具体文件：`lower.rs`（Load/Store选址与covered纯节点）、`lower/abi.rs`（load_field与对象copy的
常量偏移入口，按同一函数复用）、`lower/allocated/constraints.rs`（base输入）、`emit.rs`、
`machine/x86.rs`（类型与文本发射）、`x86_encode.rs`（ModRM/SIB）、`machine/eval.rs`。
不要把allocation home的StackSlot与业务base+disp混成同一权限含义。

下一小步才扩展 `base+index*{1,2,4,8}+disp`：base和index都是allocator可见inputs，不能藏在payload；
index不能分配为rsp，r12作为SIB index由REX.X编码，不要误当无index。Store的value与地址inputs
不得被scratch/materialization相互覆盖。MIR/L仍保留合法的wrap地址计算语义，不能凭缩放就贴NUW。
`index*1`可优先做选择期恒等式；若整棵地址被覆盖可省乘法，否则多消费者的乘法还需独立值。

## 立即数、内存操作数与宽度的后续顺序

Machine Operand 已有 Imm(i32)/Stack，但当前 constrained selector把二元与比较输入全分到Reg，
Emitter全用 Operand::Reg；所以不能说只缺编码器。立即数选择需要输入位置能够是合法立即数而非
每个input对应一个Reg，checker同时验证Imm值。64位add/cmp的imm32会符号扩展，
`0x80000000..0xffffffff`通常不能当作正数直接塞入64位imm32；交换比较还必须反转有序谓词。
这项可在第一候选稳定后做，地址disp规则不依赖通用立即数接口改造。

通用 `add reg,[mem]` / `cmp [mem],imm` 后做：除编码和allocator地址inputs外，需要固定load的
单消费者、effect顺序、别名写与trap位置证明。首版只折地址，不移动/合并load，避免把LP4变成RS4。

窄load已在LoadIndirect按bytes生成movzx；`lower.rs`随后仍调用canonicalize，产生重复mask。
可以建立少量机器值事实：ZeroExtended(bits)、SignExtended(bits)、Unknown，首先消除同宽
movzx之后的mask，再让32位整数算术用x86隐式高32位清零。全宽信息必须经过copy/phi join、
call ABI、8/16位部分写更新；signed i32需要sign extension时不能仅用32位零扩展替代。
不能全局删 `and255`：它可能是截断wrap结果或未知call高位的真实规范化。

## 验证与拒绝边界

第一候选交统一 validator，以新的RS1/ABI验收基线对照；8b2113这里只作固定历史归因。

- 比较：全部有符号/无符号条件，0、MAX、signed MIN/MAX；Not奇偶；比较同时返回/跨Phi/被观察
  的负例保持物化；跨call和高寄存器压力确保flags与值不被边复制破坏。
- 地址：disp0、±128边界、disp32两端与超界fallback；rbp/r13、rsp/r12；base/value同寄存器
  及spilled base；load/store1/2/4/8B；共享地址、多消费者与控制顺序保持。
- LLVM/kfrc原候选差分，真实semver全部用例；bc至少不再出现setb/movzx/xor/test分支链，
  两个字段地址的mov8/add变为访存disp。from_str记录direct-branch命中、setcc/test数量、对象text，
  不能承诺静态96候选在后续RS1/ABI基线上仍完全相同。
- 独立记录spill_slots、layout_object_bytes、preserve/scratch/cycle、callee-save/outgoing，
  rbp mov仍只报proxy；不要用它直接证明spill消除。
- 生成object后objdump核编码与symbol size，machine interpreter及Linux真实执行均通过；
  correctness与shape后再决定一次正式测量，不连续重跑无变化benchmark。

LP5/7公共helper只记录依赖：已内联helper是否还需独立发射，由实际符号可达性、address-taken、
export/linkage和跨codegen-unit使用决定；不能因在某个caller中内联就全局删helper。后续select/switch
与块布局可以减少比较/跳转及分支编码长度，但不作为提高inline budget的理由。

## LLVM 固定来源

固定 `llvmorg-21.1.8`，本次只读源码下载到 `/tmp/kfrc-lp4-llvm21/`：

- [X86ISelLowering.cpp:25652](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86ISelLowering.cpp#L25652)：
  LowerBRCOND识别SETCC，整数路径直接emitFlagsForSetcc，再用EFLAGS构造BRCOND；不强迫先物化bool。
  SHA-256 `2aaf4794ca304b34de813363d1c974ccafd276e20f3f5341cf50d3d49f714c1e`。
- [X86ISelDAGToDAG.cpp:3001](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86ISelDAGToDAG.cpp#L3001)：
  selectAddr提取Base/Scale/Index/Disp/Segment；1990附近matchAddress还按编码体积调整寻址形式。
  SHA-256 `afcd6ace89ad4d52ce5c737a4e2291a891e67d7f58649fd3e600452b92799194`。

复用的是显式flags依赖、合法寻址形式和回退原则，不引入LLVM整套SelectionDAG。


## 接手实施状态

比较直接分支候选已接续 `/tmp/kfrc-lp4a` 草稿，应用于 `lower/branch.rs` 和
`lower.rs` 的选择接缝；同块、单消费者的整数Compare及Bool Not链在If终结点生成
flags和Jcc，共享Bool与跨块Compare保留物化。独立测试覆盖全部十种谓词、零比较、
Not奇偶、共享/跨块负例、寄存器压力，以及Linux真实机器码与外部调用的flags破坏。
本机714项 / Linux746项和完整真实E2B已通过；最终尺寸在[体积执行记录](../rust-semantics/size-execution.md)更新。
本轮尚未实现base+disp/SIB、通用立即数和位宽事实，它们继续作为后续项。

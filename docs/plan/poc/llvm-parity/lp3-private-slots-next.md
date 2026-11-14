# 下一候选：ABI 展开后的私有槽精确消除

2026-11-09。下文保留原静态方案；接手后的私有槽候选已实现并通过独立真实验收，最终尺寸与验收入口见[体积执行记录](../rust-semantics/size-execution.md)。原研究基线为已冻结的
`8b211306a26ae3e18aea2f93005e9e405a9f495ce6cdf39c7123e0394103e241`，semver text
288,941 B。RS1 新机器指令候选先实施；本候选须等 RS1 冻结后再叠加，并以新的冻结包
重新记录比较基线，不能把两轮收益混在一起。

建议只做一个接缝：**完整 ABI 展开以后、寄存器分配以前，消除不逃逸私有槽中同基本块的
精确 store/load 往返；整个槽再无读取时删除其 stores、地址计算和帧空间。**
它直接覆盖 slice 参数的 pointer/len 落栈，以及 `nz` 中 ABI spill 和 transmute 临时槽。
本轮不增加跨块 mem2reg、不修改 sret 目的地、不重做 enum 活跃字段表示。

## 真实机器与 L 图基线

证据根：`evidence/poc-rs3-20261109/results/noif/`。小函数使用
`rs3-shapes/kfrc-on/{stderr.log,functions.json,nz.asm,bc.asm}`，semver 使用
`attribution-formal/kfrc/{graphs/stderr.gz,text.json,disassembly.txt}`。

| 函数 | 原 text | L 图 | 实际机器中的槽往返 |
|---|---:|---|---|
| `nz` | 45 B | final S 已 `Return true`，仍有 fixed transmute；L 2 Load、1 Store | Direct aggregate 参数另产生一次 spill；最终 2 Store、1 Load，16 B 帧 |
| `bc` | 126 B | slice 参数 2 Load，实际元素 1 Load；L 没有 Store | Pair 参数先写 pointer/len 两 lane，再读回来：2 Store、2 Load，16 B 帧；元素读取必须保留 |
| semver `<[u8] as PartialEq>::eq` | 222 B | 两个 slice 参数共 4 Load，无显式 Store | 两个 Pair 参数共 4 Store、4 Load；后续比较与 `memcmp` 保留 |

`nz` 的目标是去掉两个私有槽的全部访问；返回常量成立是上一阶段已有结果。本候选只
消除无须存在的物理存储，不能删除 S 解释器中的 transmute 有效性/definedness 观察。

`bc` 的目标是入口直接用捕获的 pointer/len 值，保留边界判断、元素 Load 和越界返回。
semver slice equality 中传给 `memcmp` 的是**槽里保存的 slice 数据指针**，不是槽自身
地址；不能把“从槽读出的 pointer 值被调用使用”误判为“私有槽地址逃逸”。

现有代码明确解释了为什么只改 S/L 不够：

- `kfrc-son/src/lower_layout/memory.rs::decode_boundary` 将复合参数变成 `LayoutValue`
  地址，并插入字段 Load；这里没有 ABI 参数寄存器的 Store。
- `kfrc-x86/src/lower/abi.rs::emit_parameters` 的 Direct aggregate / Pair 分支才创建
  `StackAddress` 并把 ABI 寄存器值写入槽。Pair 不管实际需求是否只读一 lane，仍写两 lane。
- `lower_layout/representation.rs::lower_transmute` 另外创建私有槽，先 encode 再 decode。
  `nz` 的目标值未使用，x86 已不发最后一次 Load，但仍保留此前私有 Store。
- `kfrc-x86/src/lower.rs::lower_function_inner` 的 `lowering.run()` 结束后，参数、调用、
  返回和图上的内存操作都已成为统一的 `LowerInstr`。在交 `allocated::compile` 前处理，
  能同时看见 ABI 隐式与 L 显式往返。

## semver 覆盖面，按重复 CGU 记录计数

静态遍历全部 252 个 L 图记录。仅沿 `PtrOffset(root, Const)` 追踪地址，不按符号名去重，
不把动态地址猜成私有槽。

| L 访问类别 | 记录数 | 含义 |
|---|---:|---|
| 全部 Load | 2,198 | 含真实外部内存访问，不能整体删除 |
| 参数 `LayoutValue` 根的 Load | 224 | 可能含 Indirect 参数，不能全部当私有 ABI spill |
| Call 结果 `LayoutValue` 根的 Load | 848 | 包含 sret 和 enum，首版不依赖这部分收益 |
| 显式 StackSlot 根的 Load | 575 | 后续按实际不逃逸/同块规则筛选 |
| 显式 StackSlot 根的 Store | 2,118 | 不代表 2,118 个死 Store |

其中可以单独锁定的 slice/str 参数：**54 个函数记录、59 个参数根、114 次 L 字段读取**。
类型为 `&str`、`&[u8]`、`&[Comparator]`；Pair 展开对应 118 次入口 lane Store。
这些是候选静态覆盖面，不是已经证明能删除的精确计数：实际 pass 仍须验证每个地址的
全部用途、offset、width、控制块和读取来源。未据此预测 text 百分比。

代表覆盖包括 `Version::parse`、`VersionReq::parse`、`Version::from_str`、`str::chars`、
`slice::iter`、slice equality 和 `parse::identifier` 的入口。`Version::from_str` 当前
19,142 B，`parse::identifier` 当前 5,670 B；只删除入口往返不会把其余成本一并归为收益。

## 首版证明边界

分析对象是 `StackAddress { slot, dst }` 产生的**存储地址**。只沿明确的地址 Mov、常量
offset Add 传播 `(slot, offset)`；先完整扫描该 root 的所有用途，再决定是否候选。

1. 全函数地址不逃逸。Call/AbiCall/CallIndirect 参数、返回值、存入其他内存、非精确
   pointer 运算、未知 opcode、ParallelCopy/Phi 合并及 vector 地址用途都拒绝此 root。
   根不是 `StackAddress` 的入参指针、sret 指针、global 和实际堆指针不属于候选。
2. 首版该槽所有实际内存访问必须在同一个控制段内。Label、Jump、条件跳转、Return
   划分控制段；调用保守清空可转发事实。这样不引入前驱合并、循环固定点或跨块
   “最后一次写入”推断。若跨块存在任何读取，整个槽的死存储删除必须退出。
3. 只转发完全相同 `(slot, offset, bytes)` 的访问，bytes 限 1/2/4/8；读区间必须在槽内。
   写入使所有重叠已有 lane 失效。宽读窄写、拼接 lane、padding、未初始化字节均不推断。
4. Store 保存的寄存器值必须在 Load 位置仍是同一定义。`LowerInstr` 不是无条件 SSA，
   Phi 的 ParallelCopy 会重复赋值；使用定义版本检查，源寄存器改写后拒绝转发。
   不能把旧 snapshot 替成重新计算后的当前值。
5. 1/2/4 字节 Load 原本零扩展；转发必须先保留写入截断及读取零扩展，再由已有
   canonicalize 做符号扩展/类型规范化。不能直接把含脏高位的 Store src 当 Load 结果。
6. 先规划全部读替换；只有该槽全部 Load 都已得到精确来源，或本来没有读取，才删除其
   全部 Stores。若有一处残留读取，首版保留该槽 Stores。不能仅因“本块最后一个 Store”
   就删掉可能被后继读取的内容。
7. 只移除已证明多余访问的纯地址/值计算。未使用的调用、除法/余数、trap、外部 Store、
   真正内存 Load 不因此删除。纯指令清理由显式读写集合和保守白名单驱动；多定义寄存器
   不能套单定义 DCE。不要把新 RS1 原子指令当作可随意拆开的文本序列。
8. 删除槽后要删除全部 `StackAddress`，并压缩/重映射存活 `stack_layouts` 的 slot index。
   只删访问、不删帧布局，`bc` 仍会保留多余 stack frame，不能算完整验收。

所有枚举 owned snapshot、间接传参和 sret，只要地址被 ABI 消费，就不满足上述
不逃逸证明，保持原来的存储生命周期。前端 S 语义、RequireDefined 和 transmute UB
契约不变；机器层优化只面向已验证图中的已定义执行。

## 实施所有权与验收

等 RS1 冻结后，建议本 agent 独占新增 `crates/kfrc-x86/src/lower/private_slots.rs` 及其
私有测试模块；在 `lower.rs` 仅增加模块和 `lowering.run()` 后的调用接缝。若寄存器
读写枚举需要共享 helper，先与 RS1 的 `LowerInstr` owner 协调，默认不动
`allocated/constraints.rs`、`emit.rs`、`lower_layout/values.rs` 或任何 opcode carrier。
新增真实机器测试放独立 `crates/kfrc-x86/tests/private_slot_transport.rs`。

RS1 当前在 `lower.rs` 增加 Rotate、ByteSwap、BitScanZero 三个原子 LowerInstr，以及
`lower/intrinsics.rs`；新 pass 必须认识其输入/输出，未知形式保守拒绝，不能覆盖该轮改动。

最小行为回归覆盖：slice Pair 入口、嵌套透明 newtype→scalar transmute、重复和覆盖写、
有脏高位的窄 lane、相邻不重叠 lane、部分覆盖/未初始化拒绝、源寄存器重定义、同块调用
边界、跨块/循环读取拒绝、slot 地址传给未知调用、slot 地址被返回/存入其他内存。
对比输出、内存结果与调用 trace，并在生成机器指令中确认匹配的访问与帧槽确实消失。

统一验证 agent 冻结新包，先本机测试，再真实 E2B LLVM/off/on 和 semver；首个错误类
先完整静态排查再恢复。结构验收至少记录 `nz`、`bc`、slice equality 的 text、frame、
私有 Load/Store 数量，semver 按全部对象实例累计 text/body；正式性能在正确性和结构
通过后决定，不用更多计时代替这轮明确的槽消除验证。

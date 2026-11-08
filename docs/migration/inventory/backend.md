# 图、优化、机器码与对象

## 可优先复用的资产

[cfg.rs](../../../src/cfg.rs) 的 CFG/liveness、[placement.rs](../../../src/placement.rs)、[regalloc.rs](../../../src/regalloc.rs) 的线性扫描、[x86_encode.rs](../../../src/x86_encode.rs) 的已验证编码、[elf_reloc.rs](../../../src/elf_reloc.rs) 的对象/重定位逻辑，以及对应回归。复用前核对新的类型与控制语义。

注意两个容易误读的点：[son.rs](../../../src/son.rs) 的 `memory` 只是预留输入，不是已实现的内存语义；[x86.rs](../../../src/x86.rs) 的 Load/Store 只用于栈 spill，不能证明支持用户指针读写。函数身份要改由 `InstanceKey` 驱动，不再用声明序号或名称。

## 图与优化 → `src/son/`

| 文件 | 处置 | 切换条件 |
|---|---|---|
| [son.rs](../../../src/son.rs) | 保留设计并重构 | 先固定类型、效果、图内 ID、实例引用和 verifier；旧构造器显式适配 |
| [cfg.rs](../../../src/cfg.rs) | 保留并适配；部分分析服务 `src/machine/` | 区分 Body CFG、placement CFG、Machine CFG；共享算法不共享可变表示 |
| [placement.rs](../../../src/placement.rs) | 保留并扩展 | 加入效果、异常边和新节点的移动条件；前驱与支配验证通过再切换 |
| [optimizer_pipeline.rs](../../../src/optimizer_pipeline.rs) | 替换编排，逐规则迁移 | 每个 pass 有不变量、预算和终止条件；不批量启用未经新类型验证的旧 pass |
| [son_optimize.rs](../../../src/son_optimize.rs) | 保留算法并扩展 | 按类型、溢出、trap/effect 前提迁移；启用/禁用都跑真实执行 |
| [value_numbering.rs](../../../src/value_numbering.rs) | 保留算法并扩展 | key 含类型与语义；代表节点满足支配与效果约束；纯值规则不推广为 load CSE |
| [loop_analysis.rs](../../../src/loop_analysis.rs) | 保留并扩展 | 多出口、异常、内存依赖验证齐备后再扩大 LICM |

## 机器码 → `src/machine/`

| 文件 | 处置 | 切换条件 |
|---|---|---|
| [native.rs](../../../src/native.rs) | 保留有效 lowering，拆分迁入 | 输入改为 VerifiedGraph + 独立 ABI；不再由后端猜类型和符号 |
| [x86.rs](../../../src/x86.rs) | 保留并扩展 | 寄存器类别、宽度、通用地址、flags/call clobber |
| [regalloc.rs](../../../src/regalloc.rs) | 保留线性扫描并扩展 | 新寄存器类别、固定寄存器、调用保存、栈约束；不加 ILP 调度 |
| [x86_encode.rs](../../../src/x86_encode.rs) | 保留并扩展 | 新 opcode/宽度/重定位要有编码和真实执行证据；分支缩短不破坏符号和 call offset |
| [call_reloc.rs](../../../src/call_reloc.rs) | 合并进调用降低和 `src/object/` 后退役 | 零参数原型样例迁为新 ABI 回归 |

## 对象 → `src/object/`

| 文件 | 处置 | 切换条件 |
|---|---|---|
| [elf.rs](../../../src/elf.rs) | 合并后退役重复 writer | 先比较单函数对象和符号；统一 writer 覆盖旧输出后删除 |
| [elf_reloc.rs](../../../src/elf_reloc.rs) | 保留并扩展 | 数据/TLS 节和重定位；重复/未定义符号、节对齐、链接校验 |
| [multi_function.rs](../../../src/multi_function.rs) | 合并后退役窄原型 | 窄调用限制样例迁移；由统一实例/符号模型取代函数拼接 |
| [native_object.rs](../../../src/native_object.rs) | 合并后退役单函数路径 | 迁移单函数缓存、对象输出和调用者 |
| [native_program.rs](../../../src/native_program.rs) | 扩展；身份来源接 `src/instantiate/` | 符号不依赖裸名字/声明顺序；对象、静态数据、调用、递归统一组装 |

# 机器层的位置与契约

机器层把一个已验证的 L 级 `FunctionGraph` 变成一段已编码的机器码。它的输入、输出各有一个与指令选择器无关的契约，
所以替换机器层只涉及这两个契约之间的代码。

```mermaid
flowchart LR
    L["L 级图（已验证）"] --> S["排程<br/>kfrc_son::schedule"]
    S --> E["导出为 Cranelift IR<br/>kfrc-cranelift"]
    E --> C["Cranelift：中端改写、指令选择、<br/>寄存器分配、编码"]
    C --> F["EncodedFunction<br/>kfrc_body::code"]
    F --> O["目标文件<br/>kfrc-object"]
```

## 输入：排程

`kfrc_son::schedule::Schedule` 给出基本块、每块的固定节点链、后继边、Region 的 Phi、每个纯节点所在的块（Click 的全局代码移动：
最早位置到使用点最近公共支配者之间循环最浅、位置最晚的块）、直接支配者和自然循环头。任何机器层都消费同一份排程，
块的身份与纯节点的位置由图决定。

## 输出：已编码函数

`kfrc_body::EncodedFunction`：符号名、最终字节、起始对齐、重定位表。重定位 `CodeRelocation` 带字段偏移、目标符号、
写入目标文件的 addend 和种类（`Call`、`PcRelative`、`GotPcRelative`、`Absolute64`、`TlsLocalExec`、`TlsGeneralDynamic`）。
外壳用重定位表做可达性遍历（一个函数引用了哪些符号），目标文件写出器把它们落成 ELF 重定位。

## 机器层的选择

| `KFRC_MACHINE` | 机器层 | 用途 |
|---|---|---|
| `cranelift`（未设置时） | `kfrc-cranelift` | 产物路线（2026-11-13 决定） |
| `x86` | `kfrc-x86` | 差分基线：同一份 L 图的另一条独立实现 |

运行时入口、分配器转发等手写 shim 仍用 `kfrc_x86::MachineFunction` 描述，经 `kfrc_x86::encode_function` 得到同一种
`EncodedFunction`。

## 各部分的归属

| 工作 | 归属 |
|---|---|
| Rust 语义优化、内联、布局降低 | `kfrc-son`（R/S/L 三级） |
| 块与纯节点位置 | `kfrc_son::schedule` |
| 存储 lane、ABI lane、调用边界 | `kfrc-cranelift` 导出器（[storage-lanes.md](storage-lanes.md)、[export.md](export.md)） |
| 值编号、常量折叠、循环不变量外提、冗余 load 消除 | Cranelift 中端 |
| 指令选择、寄存器分配、栈帧、编码 | Cranelift（版本与补丁见 [cranelift.md](cranelift.md)） |
| 节、符号、重定位 | `kfrc-object` |

分配器插入的复制还要经过跨块清理。块内复制消除会漏掉“在前驱读栈，在汇合块写回相同栈槽”的逆操作；
冷路径内联改变活跃区间时，这些操作可能进入热循环。清理依据物理位置当前内容的等价关系，汇合处取所有前驱的交集，
指令定义和 clobber 使受影响的事实失效；反向活跃分析再删除没有机器使用者的复制。汇合前缀只在部分边冗余时，可把必要复制放到
单后继前驱的无操作数跳转之前；总复制数不增加，每个汇合点只放置一次，原有机器指令不移动。它不把重复执行的虚拟寄存器定义当作常量。
重叠 spill 范围按分配器的寄存器类宽度处理；带调试值位置或固定栈寄存器别名的输出暂不参与清理。

`KFRC_CRANELIFT_VERIFY=1` 同时验证 CLIF 和清理后的寄存器分配结果。检查器消费最终发射使用的同一份分配。

## 验收

机器层的每次改动经同一道门：只改机器层时 x86 目标文件与改动前逐字节相同；表示降低改动用两条路线比较输出与最终状态；
项目测试套件与四库语料输出一致；
再比较执行指令数、`.text` 字节与编译指令数。命令与表格生成方式见
[plan/poc/llvm-parity](../../plan/poc/llvm-parity/)。

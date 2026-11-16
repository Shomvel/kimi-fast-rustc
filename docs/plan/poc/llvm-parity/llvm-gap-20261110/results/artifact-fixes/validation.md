# 产物缺口修复与验证（2026-11-10）

本轮基于 `bc718cca4a4c0e26af6e3695f13d1878e61d3bd1`，继续同一 PR #8。
对照为同一沙箱内保存的旧后端、LLVM 和候选后端。所有数字仅覆盖指定四库与负载。

## 实现

- ELF 函数与数据各自独立成节，保留绑定、对齐、重定位和对象身份；不增加 merge 标志。链接器可以按重定位闭包剔除代码与数据。
- S 层折叠同类型 transmute，保留 RequireDefined 的 poison 观察；简化无副作用的布尔 diamond，拒绝见证、锚定运算及其他边缘使用。
- L 层只对完整不逃逸的栈槽做精确字节区间转发。所有读取必须有同类型同宽写入，合流路径必须指向相同值；重叠写、生命周期边界、未知偏移和循环搜索失败均保守退出。每次查询最多 256 步。
- 两轮 L 层标量与 CFG 清理：KnownBits 范围只替换分支条件，保留原路径上的固定操作；复用已执行谓词证据及动态实例校验。随后再折叠 diamond 引入的双重取反。
- 两轮不可变小函数预处理，每 crate 最多消费 100,000 源节点，单候选 256 节点、4 个收费调用、2 层深度。只接纳缩小的函数，或不超过 64 节点且减少调用的函数。仅在这一有界阶段允许复制无身份、只读、非 TLS 的匿名常量；一般调用者的内联预算不变。
- 完整枚举字段快照扩展至按值调用、返回、存储和 ABI 参数；不对 inactive variant 做 typed load，不把循环内重复执行的槽当成不可变快照，不覆盖 MaybeUninit/union 子树。

实现参考 `/Users/shom/Downloads/llvm-study/topics/03-sroa-mem2reg.md`、`05-sccp-cfg-dce.md`、`09-ipo-lto.md`、`16-phase-ordering-budget.md` 和 `21-demanded-known-bits.md`。这是受现有 IR 契约约束的对应机制，不是逐行移植 LLVM，也未引入 solver。

## 验证口径

Linux x86_64，8 vCPU / 16 GiB，Rust 1.94.1 / LLVM 21.1.8，共用 MIR0 sysroot，库 O3 / CGU16。LLVM 编译公共驱动；某些跨 crate 泛型和标准库仍由 LLVM 生成。计时串行进行，避免与构建重叠。

本机 workspace：832 passed / 1 ignored。Linux workspace、原生边界快照差分、泛型/MaybeUninit/assume/符号保留三臂对照均通过。四库 corpus 与全部 13 workload checksums 与 LLVM 相同。semver 34、base64 13、Unicode 8 项集成测试通过。

完整单测仍有已知支持缺口：base64 `fabsf64`，Unicode `atomic_xsub`，humantime `atomic_load`。selection 全套被普通 union 值支持阻断，本轮已在旧后端重新复现同一错误，不能将其计为通过。验证器两处 dump 解析现同时接受 Sealed/Verified：发布构建的 R 图已封闭但未运行 debug verifier，行为断言保持不变。

## 已确认的链接结果

| 指标 | 旧 KFRC | 候选 KFRC | LLVM |
|---|---:|---:|---:|
| 单根可重定位 GC 保留代码 | 13,654 B | 31 B | 27 B |
| 单根 GC 保留只读数据 | 5,059 B | 0 B | 0 B |
| 真实小消费者剥离后文件 | 376,904 B | 350,160 B | 350,192 B |
| 真实小消费者代码段 | 273,387 B | 257,563 B | 257,563 B |

消费者使用 base64 的 decoded-length estimate API，LLVM O0 编译调用者，三臂均实际执行断言。可重定位探针与完整可执行文件分开报告。独立节增加对象节头和符号名开销，不能把链接后收益等同于 `.o` / `.rlib` 缩小。

## 仍未关闭的差距

- `Identifier::ptr_eq` / `is_empty` 的冗余分支、私有常量栈槽已消失，但仍为 6 / 6 条指令，LLVM 为 4 / 3；剩余是内存比较折叠和布尔 ABI 的多余扩展。
- base64 `read_u64` 从 108 条/7 次 call 降到 57 条/2 次 call；`internal_encode` 仍有大量切片范围与索引检查。循环关系和调用点已知值的专门化尚未完备。
- `parse_unit` 从 23,799 B / 5,674 条降至 5,094 B / 1,198 条，LLVM 为 1,093 B / 293 条。入口栈预留仍为 4,496 B，说明 representation copy、ABI 临时槽和栈分配仍需后续处理。
- Unicode reverse 保留 `next_code_point_reverse`、`from_u32_unchecked` 等调用链；LLVM 同名函数自身更大，不能仅以静态函数大小判胜负。

本轮没有证明四库已接近 LLVM，也没有证明编译耗时降低 50%。


## 后续固定测量与系统审阅

最终transport配对测量：13项运行时间等权几何平均相对baseline为0.933782（下降6.62%），相对LLVM为3.11910；四库编译wall几何平均为1.020948（增加2.09%）。semver/base64/Unicode编译分别增加10.8%/11.9%/5.1%，humantime减少16.6%。timestamp format相对baseline稳定回退3.51%，尚待归因。

此后ranges新增KnownBits Phi与差值事实，对base64 internal_encode的静态产物有进一步改善，但尚无同口径性能复测；不能把transport结果当作ranges结果。完整版本、样本区间、静态账本、剩余根因与执行计划见[系统审阅](../systematic-review/README.md)。

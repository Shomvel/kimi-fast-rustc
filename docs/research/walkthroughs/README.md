# Walkthrough：编译计划模拟

用具体程序逐级模拟 kfrc 新链（Body → R → S → L → 机器码），对照 rustc 1.94.1 + LLVM 实际做了什么，找出 [Rust 语义 SoN](../../design/README.md) 设计里不清楚、不够或错误的地方。每个 walkthrough 末尾的“设计细化”汇总到 [findings.md](findings.md)，再改回设计文件。

## 列表

| # | 程序 | 主要检验 |
|---|---|---|
| [W01](w01-try-error-path.md) | `?` 链 + `From` 转换的错误路径 | 聚合是值、move 链、enum 构造后投影、大值放置 |
| [W02](w02-iterator-closures.md) | `iter().filter().map().sum()` | 闭包孔位、S 级内联、`slice::Iter`、溢出检查 |
| [W03](w03-niche-option-derive.md) | `Option<&V>` 查找 + `derive(PartialEq)` enum 比较 | 变体操作、niche 编码推迟、`Refine` |
| [W04](w04-drop-unwind.md) | `String`/`Vec` 局部、条件 move、`?` 早退 | drop flag、cleanup、nounwind 摘要、panic=abort/unwind |
| [W05](w05-mut-alias-vec-push.md) | `&mut Vec<u8>` 循环 push 另一个 `&[u8]` | `Unique`/`Frozen` 分区、内联 `Reborrow`、扩容冷路径 |
| [W06](w06-generic-dedup.md) | `largest<T>`、`Vec<T>::push` 多种 `T` | 孔位表、答案键、递归去重、mentioned items |
| [W07](w07-dyn-trait.md) | `Box<dyn Write>`、vtable、去虚化 | `CallVirtual`、vtable 答案、drop via vtable |
| [W08](w08-incremental-cache.md) | 一组编辑场景 | 缓存键、early cutoff、源位置隔离 |
| [W09](w09-large-values.md) | 大结构体返回、`Box::new([0u8; 4096])`、大 enum | value slot、原地构造、sret |
| [W10](w10-interior-mutability.md) | `Cell`/`RefCell`/原子 | `Frozen` 的边界、`Unknown` 分区、fence |

## 每个 walkthrough 的结构

1. **程序**：≤30 行，可用 rustc 1.94.1 编译运行。
2. **rustc 现在怎么做**：实测数据——关键 MIR 片段、未优化 LLVM IR 的 alloca/memcpy/lifetime/assume/call/函数数、O3 后的形状；可能时列出改动它的 LLVM pass。命令在 [scripts/](scripts) 中。
3. **kfrc 编译计划**：Body 关键片段 → R 图（[dump 格式](../../design/ir/nodes.md#dump-格式)）→ R 级优化每一步用的规则和 Rust 事实 → S 级答案与答案键 → L 级降低 → 期望机器码（与 rustc O3 汇编对照）。
4. **代价核算**：编译工作（节点数、访问数的估计）、体积、运行时间、缓存键与失效。
5. **设计细化**：本例暴露的问题，每条写明改哪个设计文件、怎么改。

## 测量口径

- MIR 和 LLVM IR 用 rustc 1.94.1（aarch64-apple-darwin 主机；IR 形状与目标基本无关，个别 ABI 相关处注明）。x86_64 汇编用 rustc 1.99.1 `--target x86_64-unknown-linux-musl`，注明版本差异。
- 未优化 IR：`-Copt-level=3 -Ccodegen-units=1 -Cno-prepopulate-passes --emit=llvm-ir`（已过 MIR 优化，未过 LLVM）。显式写 `-Ccodegen-units`，避免 `--emit=llvm-ir` 隐式改成单 CGU 带来误解。
- 数字由脚本从产物计算后原样粘贴，不手抄。

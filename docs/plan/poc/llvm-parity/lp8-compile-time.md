# LP8 编译工作量

主张 2 要求中后端指令数比 LLVM 少 3×以上。semver 上 LLVM 是 1.67 G，目标即 ≤ 0.56 G；
kfrc 现为 5.56 G（总窗口口径），需要降约 10×。先做 LP0 的分阶段测量，下面是已能看到的疑点。

## 疑点

1. **生成的机器码太多**：LP1–LP3 让 `.text` 降一个数量级以前，x86 降低、分配和编码的工作量都按 L 节点数与指令数成比例放大。这一部分随 LP1–LP3 自然下降，先测再定。
2. **每个 pass 全图重扫**：semver 每个函数的 `OptimizeStats` 显示，几乎没有可做的工作时也会全图索引。例如 `from_str`：
   - `observation_nodes_indexed` = 节点数；
   - `memory_node_visits` 2,320，约为节点数的 5 倍；
   - 没有除法也有 `division_node_visits` 972。

   [optimization-modules.md](../optimization-modules.md) 已写“完整 S 特化的变化种子与跨 pass worklist 仍待实现”。按它交付：一个共享 worklist，pass 只访问变化的节点及其使用者；没有对应 opcode 的 pass 直接跳过（opcode 计数表）。
3. **生产路径上的 verifier**：`rustc_codegen_kfrc/src/lib.rs` 对每个函数跑 `verify_s_input`、`verify_s_output`、`verify_l`，`pipeline.rs` 里还有多处。
   - 正确性臂保持开启；
   - 正式计时臂按 rustc `-Zverify-llvm-ir` 的惯例关闭；
   - 两种配置都在证据里写明，比值报告关闭后的数字，并说明 verifier 占比。
4. **优化了不会发射的函数**：semver 有 598 个函数做了 S 优化，发射 250 个，LLVM 的目标对象只有 81 个 text 符号。
   - 被所有调用者内联、且不导出的实例，不必走 L 和 x86；
   - 只为内联而存在的被调者，只需 S 图，不必生成机器码；
   - 与定义级 R 一次（主张 5）和缓存（主张 6）一起设计，不另造一套可达性。
5. **内联深度**：`from_str` 在 kfrc 中仍有 31 个 call，LLVM 是 9 个。内联不足让运行时变慢、函数数量变多。
   - 内联预算与 [研究建议 R10](../../../research/llvm-mir/recommendations.md) 一起评估：用优化后的大小，而不是未优化 S 图的估计；
   - 记录每个保留 call 的原因（预算、递归、未知被调者）。
6. **c12→c13 的 +10.8%**：LP0 分项后，先定位到具体阶段再继续叠加改动。

## LLVM 参考

- [NewPassManager.rst](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/NewPassManager.rst)：分析失效与保留集合（kfrc 不必实现框架，但 pass 应声明改动范围）；
- [InstCombine worklist](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/InstCombine/InstructionCombining.cpp)：只把改动节点的使用者加回 worklist；
- [InlineCost.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Analysis/InlineCost.cpp)：按内联后可化简的部分折算成本。

## 验收

- 阶段分项表：每次候选都报告，与 LLVM 族占比并列。
- worklist 化以后，同一输入的输出对象字节不变（`object_bytes_unchanged`），只比较指令数。
- 主张 2 以 metrics.md 口径报告，旧口径并列。

# 差分执行协议

## 步骤

1. **固定输入：** 测试来源、seed、edition/cfg、target、溢出/panic 语义、程序输入；记录源码摘要和能力标签。
2. **两边编译：** stock rustc 与 kfrc 分别编译同一程序。测试 driver 需要的打印/断言若尚未支持，可用隔离的 C ABI driver，但被测函数必须由 kfrc 生成。
3. **真实执行：** 在 x86_64 上运行，比较退出码、stdout/stderr、输出文件和必要的内存/FFI 观测。交叉编译主机只负责生成对象。
4. **保存不匹配：** 原始输入、工具版本、产物、日志、各阶段 IR。最小化后用原错误条件复核，防止 reducer 把错误变成 UB。
5. **无效程序：** 比较拒绝类别——类型冲突、借用冲突、move 后使用、未初始化、约束不满足等。合法但未支持的程序明确拒绝，记为覆盖缺口。

新增语义的测试从完整源码经公共编译入口执行；内部 verifier 用来缩短定位路径。

## 判定细则（来自 rustc 精读）

- **接受/拒绝包含默认 Deny 的 lint。** 1.94.1 中 `overflowing_literals`、`arithmetic_overflow`、`unconditional_panic`、`dependency_on_unit_never_type_fallback` 默认 Deny。kfrc 要么实现，要么把依赖它们的用例标为 `lint-dependent` 剔出对拍集。
- **两种 trait 求解模式。** 默认模式和 `-Znext-solver=globally` 都跑；两者不一致的用例记为 `solver-divergence`，kfrc 与任一模式一致即可，记录与哪一个一致（[traits/which-solver.md](../rustc/traits/which-solver.md)）。
- **借用检查 oracle 是 NLL**（1.94 默认不开 Polonius）。比较接受/拒绝、错误码、主 span，不比较诊断文本。
- **panic 文本。** 无消息 `assert!` 的 panic 文本和 `stringify!` 结果由 rustc 的 pretty printer 生成；要逐字比较失败路径的 stderr，需要兼容的打印器。在实现之前，这类用例只比较退出码和 panic 位置。
- **复刻而不是“修正”rustc 的已知行为**：#47949（内层 drop panic 时已写入目标的值泄漏）、or-pattern 按第一分支绑定顺序 drop（#142163）、参数晚于函数体内局部析构。

## 随机程序生成

只生成预先定义的、无 UB、观测结果确定的程序：

- 避免数据竞争、未初始化读取、非法指针、生命周期外使用、未约束的浮点/随机/环境行为。
- 输出顺序不确定的场景，先定义可接受的结果集合。
- 行为差分里的布局相关测试只用有契约的 `repr`，不读 padding 或未初始化值。`repr(Rust)` 的布局与 ABI 另有[一致性测试](layout-abi-parity.md)。

## 借用上游测试

rustc 的 compiletest/UI 测试含版本、target、feature、auxiliary crate、预期错误等指令。迁移时保存原始来源和语义目的，适配 harness，不照抄易变的诊断文本。有效程序比较行为，无效程序比较拒绝类别。来源记录见 [sources/provenance.md](../sources/provenance.md)。

使用 nightly 内建项、`no_core`、特殊 lang item 的测试单列环境——它们证明局部 lowering，不证明稳定 Rust 或 sysroot 可用。

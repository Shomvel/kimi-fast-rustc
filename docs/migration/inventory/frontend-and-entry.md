# 入口、前端与诊断辅助

规划目录见 [architecture/code-layout.md](../architecture/code-layout.md)。

| 文件 | 处置 → 落点 | 切换条件 |
|---|---|---|
| [lib.rs](../../../src/lib.rs) | 保留并调整导出 | 新旧能力分开导出；删除公开导出前检查仓库内外调用者并记录兼容性变化 |
| [main.rs](../../../src/main.rs) | 替换编排 → `src/build/`；保留 CLI 入口 | driver 契约和选择方式固定后逐命令切换 |
| [frontend.rs](../../../src/frontend.rs) | 替换 → `src/syntax/`、`resolve/`、`semantics/`、`body/`；旧子集最终退役 | 新 AST → CheckedBody → SoN 全链完成，旧行为测试迁移后删除。期间冻结为 legacy 入口，已验证的词法/语义样例转成新路径测试 |
| [mir.rs](../../../src/mir.rs) | 隔离为旧文本测试输入；最终退役或作专用测试入口 | 测试改用新 Body 构造器后才删除；新 Body 用独立名称和结构 |
| [config.rs](../../../src/config.rs) | 替换配置模型 → `src/build/` | 去掉生产默认的 LLVM 假设；panic/overflow/target 来自显式配置 |
| [interpreter.rs](../../../src/interpreter.rs) | 保留旧图诊断用途；需要时作 `src/son/` 测试支持 | 单 block i64 evaluator 不作新前端、borrowck 或机器执行的 oracle |
| [machine_eval.rs](../../../src/machine_eval.rs) | 保留并适配 → `src/machine/` 诊断支持 | 扩展指令模型时与真实执行对照；两个共享 lowering 的模型一致不算独立证据 |

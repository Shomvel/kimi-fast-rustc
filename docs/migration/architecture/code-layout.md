# 规划目录与切换方式

## 规划目录

这些是职责目录，不预先拆成多个 crate；名称在接口冻结时可调整。逐文件去向见 [inventory/](../inventory/README.md)。

| 目录 | 来自 | 完成判据 |
|---|---|---|
| `src/syntax/`、`src/resolve/` | `frontend.rs` 的职责 + rustc 参考 | 新 AST/定义模型贯通后停止扩展旧入口 |
| `src/semantics/`、`src/body/` | 新模块，移植成熟语义算法 | 统一类型与 Body；正反例证明规则，不只是能解析 |
| `src/instantiate/` | 当前函数元数据 + rustc 单态化参考 | 实例身份稳定，不依赖声明顺序 |
| `src/son/` | `son.rs`、优化与分析模块 | 先扩展契约与 verifier，再迁移 pass；不做纯目录搬迁 PR |
| `src/machine/` | `native.rs`、`x86.rs`、`regalloc.rs`、编码 | 保留有效算法；接收有类型图和独立 ABI 描述 |
| `src/object/` | `elf.rs`、`elf_reloc.rs`、`native_object.rs` | 对象/数据/符号统一；真实链接验收 |
| `src/build/` | driver、scheduler、pipeline、cache | 语义产物任务图、预算和发布 |

## 新旧链并存

1. 新增一条**显式选择**的新链；旧 `.kfr` 入口保留用于回归定位。两条链的能力标签和产物标识可区分，缓存使用不同 namespace。
2. 以一个完整功能为单位切换：源码 → 语义检查 → 降低 → 运行 → 证据全部完成后，再迁移调用者。
3. 新链在显式选择下稳定后才切成默认。
4. 旧实现满足以下全部条件才删除：没有生产调用者、对应测试已迁移、新实现通过阶段出口。

回退方式是恢复上一个已验收的实现或关闭新优化/调度/缓存开关，不改变程序语义。

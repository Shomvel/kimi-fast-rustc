# T07 SoN 表示与 verifier — 细化

`M1 · C · 依赖：T02, T03` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t07) · 契约见 [son.md](../../architecture/contracts/son.md)、[son-verifier.md](../../architecture/contracts/son-verifier.md)

## 步骤

1. **新模块骨架**：`src/son/{mod,graph,types,verify,dump}.rs`。旧 `src/son.rs` 改名为 `src/son/legacy.rs` 并保留 `pub use`，现有调用者不动。
2. **FunctionGraph + ValueTy**：按契约实现节点、Region/Phi 结构、`GraphState`；Builder API 只允许在 `Building` 状态下修改。
3. **legacy adapter**：`legacy::Graph` + `FunctionMetadata` → 每个函数一个 `FunctionGraph`；`Call(usize)` 通过声明表映射为 `InstanceId`；bool 结果（Compare、短路 Phi）标 `Int{bits:1}`，其余 `Int{bits:64}`。
4. **verifier**：先 structural + typed，再 full（支配、循环、数据环、效果链、trap 位置）。支配用放置结果计算，复用 `cfg.rs` 的支配树。
5. **dump**：稳定文本格式，加类型列；`KFRC_DUMP=son` 时 driver 写到证据目录。
6. **接入现有测试**：在测试配置下对所有现有 integration test 的图跑 adapter + full verifier。

## 测试

| 类型 | 用例 |
|---|---|
| 正例 | 现有 63 个 integration test 文件中的全部图经 adapter 后通过 full verifier（T01 已知的错编译形状除外，单列为 known-fail） |
| 负例 | T01 的 4 个支配违规形状 → `DominanceViolation`；Phi 元数错、跨图引用、`If` 条件为 64 位、Store 缺效果、除法被提到守卫前、纯值环 |
| 稳定性 | 同一源码 dump 两次逐字节一致；`--jobs 1/4` 下 dump 一致 |

第 6 步很可能发现更多既有的支配违规——按[验证规则](../../validation/README.md#发现错误时)先批量归类，再统一在 T09 修。

## 完成标志

- 新 verifier 拒绝全部负例，接受全部正例（known-fail 除外）。
- `legacy::Graph` 的生产路径行为不变（现有测试全过）。
- verifier 和 dump 不依赖 frontend 内部状态。

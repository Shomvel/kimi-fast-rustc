# T06 类型检查与 Typed Body — 细化

`M1 · B · 依赖：T05` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t06) · 契约：[types.md](../../architecture/contracts/types.md)、[body.md](../../architecture/contracts/body.md) · rustc 笔记：[typeck](../../rustc/typeck/README.md)、[ownership](../../rustc/ownership/README.md)

分成两个子任务，顺序执行：**T06.a 产出 TypeckResults，T06.b 用它构建 Body**。不能边检查边建 Body：分支合并会回头修改前面分支的 adjustment，fallback 和 cast 检查在函数末尾，运算符是否内建在 writeback 才确定。

## T06.a 类型检查

范围（M1 标量）：

- 类型：`bool`、全部整数宽度（**必须含 `i32`**——无约束字面量 fallback 到 `i32`；现在的 kfrc 只有 `i64/u64`，`let x = 1;` 的溢出点会与 rustc 不同）、`()`、`!`。
- 推导表、快照、挂起约束队列从一开始就实现（[types.md](../../architecture/contracts/types.md#推导表)）。
- Expectation 传播；分支合并（`CoerceMany` 的 LUB，M1 只有 `NeverToAny` 一种 adjustment）。
- 阶段顺序照搬 rustc，它本身是语义：
  - cast 在 fallback **之后**检查：`let cap = 512*512; cap as u8` 中 `cap` 是 `i32`。
  - 字面量采纳 cast 目标类型：`256 as u8` 中字面量就是 `u8`（随后 `overflowing_literals` 报错）。
  - 内建二元运算的结果类型取左操作数：`let _: u8 = 0 + 0` 通过。
- never fallback 按 crate edition（2024 起 fallback 到 `!`）。
- 默认 Deny 的 lint 计入“拒绝”：`overflowing_literals`、`arithmetic_overflow`、`unconditional_panic`。实现前三者，或把依赖它们的 rustc 用例标为“lint 依赖”从对拍集中剔除。

产物：TypeckResults（每个节点的类型、adjustment、运算符内建标记、cast 种类、调用目标 DefId、`tainted`）。`Infer` 变量全部解析后才发布。

## T06.b 构建 Body

- **作用域栈 + 四类 DropTree**（break/continue、else、return、unwind）。标量阶段没有需要 drop 的值，但 `StorageDead`、各类出口边、溢出/越界/除零 `Assert` 的 unwind 边都已存在。
- **cleanup 边总是建出来**，`panic=abort` 也一样；裁剪在 SoN 之前。
- **region scope tree**：移植 rustc 的 `rustc_hir_analysis/check/region.rs`（约 1250 行，只依赖语法形状和 span edition），Body builder 查询它放置 `StorageDead`/`Drop`。
- 所有控制流（if/match/loop/break/return/短路）都汇合到 CFG，局部变量只通过 Place 读写——SSA 留给 T09。
- Body verifier：每个块有终结指令、cleanup 块只跳到 cleanup 块、`Call` 的目标块里返回值已初始化、所有 Place 类型一致。

## 测试

- 推导：`tests/ui` 中 typeck 笔记标为“M1 首轮可对拍”的用例（[typeck/test-cases.md](../../rustc/typeck/test-cases.md)）。
- 阶段顺序三类回归（上面的三个例子）。
- 拒绝：类型不匹配（E0308）、未初始化使用（E0381）、对不可变绑定赋值（E0384）、`-x` 作用于无符号（E0600）、溢出字面量。
- Body：每个 T01 形状、每种出口（break 带值、continue、return、块尾）都能 dump，并通过 Body verifier。

## 完成标志

T06.a 的 TypeckResults 无推导变量泄漏；T06.b 的 Body 通过 verifier；正反例与 rustc 的接受/拒绝和错误码一致。
